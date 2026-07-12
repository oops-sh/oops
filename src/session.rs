//! Pending-sandbox session records and the gc sweep.
//!
//! One pending sandbox per target directory. A session is a directory
//! `<state>/sessions/<id>/` containing `session.json`, `upper/`, and
//! `work/`. Undo renames the whole session directory into `<state>/trash/`
//! (O(1)) and deletion happens asynchronously. See openspec/specs/session.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::state;

pub const RECORD_FILE: &str = "session.json";

/// Sessions younger than this are exempt from gc quarantine, so a sweep
/// never races a `run` that is still writing its record.
const GC_MIN_AGE_SECS: u64 = 60;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    /// Canonicalized directory the sandbox covers.
    pub target: PathBuf,
    pub upper: PathBuf,
    pub work: PathBuf,
    pub command: String,
    pub created_unix: u64,
    /// Exit status of the wrapped command; None while it is still running.
    pub exit_code: Option<i32>,
}

pub struct Session {
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

/// Create the session directory skeleton and persist the record immediately
/// (exit_code: None), so gc never mistakes an in-flight session for an orphan.
pub fn create(state: &Path, target: &Path, command: &str) -> Result<Session> {
    let id = new_session_id();
    let dir = state::sessions_dir(state).join(&id);
    let upper = dir.join("upper");
    let work = dir.join("work");
    std::fs::create_dir_all(&upper)
        .with_context(|| format!("cannot create sandbox layer at {}", upper.display()))?;
    std::fs::create_dir_all(&work)?;
    let record = SessionRecord {
        id,
        target: target.to_path_buf(),
        upper,
        work,
        command: command.to_string(),
        created_unix: now_unix(),
        exit_code: None,
    };
    save(&dir, &record)?;
    Ok(Session { dir, record })
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

/// Find the pending session for `target`, if any.
pub fn find_for_target(state: &Path, target: &Path) -> Result<Option<Session>> {
    let sessions = state::sessions_dir(state);
    if !sessions.is_dir() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(&sessions)? {
        let dir = entry?.path();
        if !dir.is_dir() {
            continue;
        }
        if let Ok(record) = load(&dir) {
            if record.target == target {
                return Ok(Some(Session { dir, record }));
            }
        }
    }
    Ok(None)
}

/// Undo/commit-success cleanup: atomically move the session directory into
/// trash. O(1) regardless of how large the upper layer is.
pub fn move_to_trash(state: &Path, session_dir: &Path) -> Result<PathBuf> {
    state::ensure_in_state_dir(state, session_dir)?;
    let trash = state::trash_dir(state);
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

/// The gc sweep: delete trash entries, quarantine recordless session dirs.
/// Every deletion is containment-checked. Never touches a session with a
/// parseable record or one younger than GC_MIN_AGE_SECS.
pub fn gc_sweep(state: &Path) -> Result<()> {
    let trash = state::trash_dir(state);
    if trash.is_dir() {
        for entry in std::fs::read_dir(&trash)? {
            let path = entry?.path();
            if state::ensure_in_state_dir(state, &path).is_ok() {
                let _ = remove_all(&path);
            }
        }
    }

    let sessions = state::sessions_dir(state);
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
                let _ = move_to_trash(state, &dir);
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
pub fn ensure_no_pending(state: &Path, target: &Path) -> Result<()> {
    if let Some(existing) = find_for_target(state, target)? {
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

    fn state(tmp: &tempfile::TempDir) -> PathBuf {
        let s = tmp.path().join("state");
        std::fs::create_dir_all(&s).unwrap();
        s
    }

    #[test]
    fn create_find_trash_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state(&tmp);
        let target = tmp.path().join("proj");
        std::fs::create_dir(&target).unwrap();

        let s = create(&state, &target, "echo hi").unwrap();
        assert!(s.record.exit_code.is_none());
        let found = find_for_target(&state, &target).unwrap().unwrap();
        assert_eq!(found.record.command, "echo hi");
        assert!(ensure_no_pending(&state, &target).is_err());

        move_to_trash(&state, &found.dir).unwrap();
        assert!(find_for_target(&state, &target).unwrap().is_none());
        assert!(ensure_no_pending(&state, &target).is_ok());

        // The trash entry exists until a sweep removes it.
        assert_eq!(std::fs::read_dir(state.join("trash")).unwrap().count(), 1);
        gc_sweep(&state).unwrap();
        assert_eq!(std::fs::read_dir(state.join("trash")).unwrap().count(), 0);
    }

    #[test]
    fn gc_spares_fresh_recordless_dirs_and_valid_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state(&tmp);
        let target = tmp.path().join("proj");
        std::fs::create_dir(&target).unwrap();

        let valid = create(&state, &target, "true").unwrap();
        let fresh_orphan = state.join("sessions/fresh-orphan");
        std::fs::create_dir_all(&fresh_orphan).unwrap();

        gc_sweep(&state).unwrap();
        assert!(valid.dir.is_dir(), "valid session must survive gc");
        assert!(
            fresh_orphan.is_dir(),
            "fresh orphan must not be quarantined (race guard)"
        );
    }
}
