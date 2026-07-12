//! Pending-sandbox session records and the gc sweep.
//!
//! One pending sandbox per target directory. A session is a directory
//! `<root>/sessions/<id>/` containing `session.json` plus backend layers
//! (`upper/`+`work/` for overlayfs, `snapshot/` for apfs). Undo renames the
//! whole session directory into that root's `trash/` (O(1)); deletion
//! happens asynchronously. gc sweeps every registered, mounted state root.
//! See openspec/specs/session.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::{self, StateRoots};

pub const RECORD_FILE: &str = "session.json";

/// Sessions younger than this are exempt from gc quarantine, so a sweep
/// never races a `run` that is still writing its record.
const GC_MIN_AGE_SECS: u64 = 60;

fn default_backend() -> String {
    // Records written before the backend field existed could only have come
    // from the overlayfs backend.
    "overlayfs".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    /// Canonicalized directory the sandbox covers.
    pub target: PathBuf,
    #[serde(default = "default_backend")]
    pub backend: String,
    /// OverlayFS layers.
    #[serde(default)]
    pub upper: Option<PathBuf>,
    #[serde(default)]
    pub work: Option<PathBuf>,
    /// APFS snapshot.
    #[serde(default)]
    pub snapshot: Option<PathBuf>,
    /// Identity anchor for snapshot-restore undo: the target's PARENT
    /// directory at run time. A replaced target is a command change; a
    /// replaced parent means we must refuse to restore.
    #[serde(default)]
    pub parent_dev: Option<u64>,
    #[serde(default)]
    pub parent_ino: Option<u64>,
    pub command: String,
    pub created_unix: u64,
    /// Exit status of the wrapped command; None while it is still running.
    pub exit_code: Option<i32>,
}

pub struct Session {
    /// The state root this session lives under.
    pub root: PathBuf,
    pub dir: PathBuf,
    pub record: SessionRecord,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn new_session_id() -> String {
    format!("{}-{}", now_unix(), std::process::id())
}

#[allow(dead_code)] // each variant is constructed on its own platform only
pub enum BackendKind {
    Overlayfs,
    Apfs,
}

/// Create the session directory skeleton and persist the record immediately
/// (exit_code: None), so gc never mistakes an in-flight session for an orphan.
pub fn create(root: &Path, target: &Path, command: &str, kind: BackendKind) -> Result<Session> {
    let id = new_session_id();
    let dir = state::sessions_dir(root).join(&id);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create session directory {}", dir.display()))?;

    use std::os::unix::fs::MetadataExt;
    let parent = target
        .parent()
        .with_context(|| format!("target {} has no parent directory", target.display()))?;
    let pmeta = parent
        .symlink_metadata()
        .with_context(|| format!("cannot stat parent {}", parent.display()))?;

    let mut record = SessionRecord {
        id,
        target: target.to_path_buf(),
        backend: String::new(),
        upper: None,
        work: None,
        snapshot: None,
        parent_dev: Some(pmeta.dev()),
        parent_ino: Some(pmeta.ino()),
        command: command.to_string(),
        created_unix: now_unix(),
        exit_code: None,
    };
    match kind {
        BackendKind::Overlayfs => {
            let upper = dir.join("upper");
            let work = dir.join("work");
            std::fs::create_dir_all(&upper)?;
            std::fs::create_dir_all(&work)?;
            record.backend = "overlayfs".into();
            record.upper = Some(upper);
            record.work = Some(work);
        }
        BackendKind::Apfs => {
            // The snapshot path must NOT exist yet: clonefile creates it.
            record.backend = "apfs".into();
            record.snapshot = Some(dir.join("snapshot"));
        }
    }
    save(&dir, &record)?;
    Ok(Session {
        root: root.to_path_buf(),
        dir,
        record,
    })
}

pub fn save(dir: &Path, record: &SessionRecord) -> Result<()> {
    let json = serde_json::to_string_pretty(record)?;
    std::fs::write(dir.join(RECORD_FILE), json)
        .with_context(|| format!("cannot write session record in {}", dir.display()))?;
    Ok(())
}

fn load(dir: &Path) -> Result<SessionRecord> {
    let raw = std::fs::read_to_string(dir.join(RECORD_FILE))?;
    Ok(serde_json::from_str(&raw)?)
}

/// Find the pending session for `target` across every mounted state root.
pub fn find_for_target(roots: &StateRoots, target: &Path) -> Result<Option<Session>> {
    for root in roots.mounted() {
        let sessions = state::sessions_dir(&root);
        if !sessions.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&sessions)? {
            let dir = entry?.path();
            if !dir.is_dir() {
                continue;
            }
            if let Ok(record) = load(&dir) {
                if record.target == target {
                    return Ok(Some(Session { root, dir, record }));
                }
            }
        }
    }
    Ok(None)
}

/// Undo/commit cleanup: atomically move the session directory into its
/// root's trash. O(1) regardless of how large the layers are.
pub fn move_to_trash(roots: &StateRoots, root: &Path, session_dir: &Path) -> Result<PathBuf> {
    roots.ensure_contains(session_dir)?;
    let trash = state::trash_dir(root);
    std::fs::create_dir_all(&trash)?;
    let name = session_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".into());
    let dest = trash.join(format!("{}.{}.{}", name, now_unix(), std::process::id()));
    std::fs::rename(session_dir, &dest)
        .with_context(|| format!("cannot move session into trash at {}", dest.display()))?;
    Ok(dest)
}

/// The gc sweep over every mounted root: delete trash entries, quarantine
/// recordless session dirs. Every deletion is containment-checked. Never
/// touches a session with a parseable record or one younger than
/// GC_MIN_AGE_SECS.
pub fn gc_sweep(roots: &StateRoots) -> Result<()> {
    for root in roots.mounted() {
        let trash = state::trash_dir(&root);
        if trash.is_dir() {
            for entry in std::fs::read_dir(&trash)? {
                let path = entry?.path();
                if roots.ensure_contains(&path).is_ok() {
                    let _ = remove_all(&path);
                }
            }
        }

        let sessions = state::sessions_dir(&root);
        if sessions.is_dir() {
            for entry in std::fs::read_dir(&sessions)? {
                let dir = entry?.path();
                if !dir.is_dir() || load(&dir).is_ok() {
                    continue;
                }
                let age = std::fs::metadata(&dir)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .map(|e| e.as_secs())
                    .unwrap_or(0);
                if age >= GC_MIN_AGE_SECS {
                    let _ = move_to_trash(roots, &root, &dir);
                }
            }
        }
    }
    Ok(())
}

fn remove_all(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Spawn a detached background `oops __gc` so undo can return immediately.
/// Failure to spawn is non-fatal: the next run's sweep will finish the job.
pub fn spawn_background_gc() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg("__gc")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// Guard for `run`: refuse a second pending sandbox for the same target.
pub fn ensure_no_pending(roots: &StateRoots, target: &Path) -> Result<()> {
    if let Some(existing) = find_for_target(roots, target)? {
        bail!(
            "a sandbox is already pending for {} (from `oops run \"{}\"`).\n\
             Inspect it with `oops diff`, then `oops undo` or `oops commit` first.",
            target.display(),
            existing.record.command
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(tmp: &tempfile::TempDir) -> StateRoots {
        let primary = tmp.path().join("state");
        std::fs::create_dir_all(&primary).unwrap();
        StateRoots {
            primary,
            registered: Vec::new(),
        }
    }

    #[test]
    fn create_find_trash_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = roots(&tmp);
        let target = tmp.path().join("proj");
        std::fs::create_dir(&target).unwrap();

        let s = create(&roots.primary, &target, "echo hi", BackendKind::Overlayfs).unwrap();
        assert!(s.record.exit_code.is_none());
        assert_eq!(s.record.backend, "overlayfs");
        assert!(s.record.parent_dev.is_some() && s.record.parent_ino.is_some());
        let found = find_for_target(&roots, &target).unwrap().unwrap();
        assert_eq!(found.record.command, "echo hi");
        assert!(ensure_no_pending(&roots, &target).is_err());

        move_to_trash(&roots, &found.root, &found.dir).unwrap();
        assert!(find_for_target(&roots, &target).unwrap().is_none());

        gc_sweep(&roots).unwrap();
        assert_eq!(
            std::fs::read_dir(roots.primary.join("trash"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn old_records_without_backend_field_load_as_overlayfs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(RECORD_FILE),
            r#"{"id":"x","target":"/t","upper":"/s/upper","work":"/s/work",
                "command":"true","created_unix":1,"exit_code":0}"#,
        )
        .unwrap();
        let rec = load(&dir).unwrap();
        assert_eq!(rec.backend, "overlayfs");
        assert_eq!(rec.upper.as_deref(), Some(Path::new("/s/upper")));
        assert!(rec.snapshot.is_none());
    }

    #[test]
    fn gc_sweeps_all_mounted_roots_and_spares_valid_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut roots = roots(&tmp);
        let volume = tmp.path().join("vol/.oops/state");
        std::fs::create_dir_all(state::trash_dir(&volume)).unwrap();
        std::fs::write(state::trash_dir(&volume).join("junk"), "x").unwrap();
        roots.registered.push(volume.clone());

        let target = tmp.path().join("proj");
        std::fs::create_dir(&target).unwrap();
        let valid = create(&roots.primary, &target, "true", BackendKind::Apfs).unwrap();
        let fresh_orphan = roots.primary.join("sessions/fresh-orphan");
        std::fs::create_dir_all(&fresh_orphan).unwrap();

        gc_sweep(&roots).unwrap();
        assert!(valid.dir.is_dir(), "valid session must survive gc");
        assert!(
            fresh_orphan.is_dir(),
            "fresh orphan must not be quarantined (race guard)"
        );
        assert_eq!(
            std::fs::read_dir(state::trash_dir(&volume))
                .unwrap()
                .count(),
            0,
            "registered volume root trash must be swept"
        );
    }
}
