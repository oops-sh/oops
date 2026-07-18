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
    // Quarantine first: move recordless session dirs into trash (O(1) renames,
    // no elevation needed).
    for root in roots.mounted() {
        let sessions = state::sessions_dir(&root);
        if !sessions.is_dir() {
            continue;
        }
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

    // Then reclaim trash. On Linux this is fd-anchored and may elevate into an
    // identity-mapped userns to punch through rootless mode-000 leftovers;
    // containment is anchored on the registered roots BEFORE any elevation.
    #[cfg(target_os = "linux")]
    reap::reclaim_trash(roots);
    #[cfg(not(target_os = "linux"))]
    for root in roots.mounted() {
        let trash = state::trash_dir(&root);
        if trash.is_dir() {
            for entry in std::fs::read_dir(&trash)? {
                let path = entry?.path();
                if roots.ensure_contains(&path).is_ok() {
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
    }
    Ok(())
}

/// Enter an identity-mapped user namespace so gc can reclaim rootless-overlay
/// leftovers. A rootless overlay mount leaves a `work/work` directory owned by
/// the (single) mapped uid with mode 000: the plain unprivileged user cannot
/// delete it, so `trash/` would grow unboundedly. As root inside a user
/// namespace that maps that uid we hold `CAP_DAC_OVERRIDE` over it and can
/// remove it. Best-effort and only meaningful in a short-lived `__gc` process
/// (it changes the process's user namespace): on any failure gc proceeds
/// without it, reclaiming whatever is already deletable.
#[cfg(target_os = "linux")]
fn enter_gc_userns() {
    use nix::sched::{unshare, CloneFlags};
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    if unshare(CloneFlags::CLONE_NEWUSER).is_err() {
        return;
    }
    let _ = std::fs::write("/proc/self/setgroups", b"deny");
    let _ = std::fs::write("/proc/self/uid_map", format!("0 {uid} 1"));
    let _ = std::fs::write("/proc/self/gid_map", format!("0 {gid} 1"));
}

/// Fd-anchored trash reclamation (Linux). gc now runs with `CAP_DAC_OVERRIDE`
/// inside an identity-mapped userns, so the old "not enough permission" that
/// incidentally guarded against following an agent-planted symlink is GONE —
/// containment is now the only defense, and it is enforced exactly like commit
/// replay: every path is reached by an `O_NOFOLLOW` component walk from a
/// registered state root, deletion is `unlinkat`, and a symlink component is
/// unlinked, never traversed. The containment anchor (opening each `trash/`
/// from its registered root) is established BEFORE elevation; elevation only
/// punches through the mode-000 dirs and every op stays relative to the
/// anchored fds.
#[cfg(target_os = "linux")]
mod reap {
    use super::{enter_gc_userns, StateRoots};
    use std::ffi::{CString, OsStr, OsString};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn reclaim_trash(roots: &StateRoots) {
        // Phase 1 (UNPRIVILEGED): the containment decision. Open each trash
        // dir by an O_NOFOLLOW walk from its registered root, so the fd proves
        // the dir is inside a registered root and was reached without
        // traversing any symlink. Done before any elevation.
        let mut trash_fds = Vec::new();
        for root in roots.mounted() {
            let Ok(root_fd) = open_dir_nofollow(&root) else {
                continue;
            };
            if let Ok(tfd) = openat_dir_nofollow(root_fd.as_raw_fd(), OsStr::new("trash")) {
                trash_fds.push(tfd);
            }
        }
        if trash_fds.is_empty() {
            return;
        }
        // Phase 2: elevate ONLY to punch through mode-000 dirs, then delete
        // strictly relative to the anchored fds (no path is re-parsed).
        enter_gc_userns();
        for tfd in &trash_fds {
            if let Ok(names) = read_dir_fd(tfd.as_raw_fd()) {
                for name in names {
                    rm_recursive_at(tfd.as_raw_fd(), &name);
                }
            }
        }
    }

    fn open_dir_nofollow(path: &Path) -> std::io::Result<OwnedFd> {
        let c = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let fd = unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn openat_dir_nofollow(dirfd: i32, name: &OsStr) -> std::io::Result<OwnedFd> {
        let c = CString::new(name.as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let fd = unsafe {
            libc::openat(
                dirfd,
                c.as_ptr(),
                libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn read_dir_fd(dirfd: i32) -> std::io::Result<Vec<OsString>> {
        let dup = unsafe { libc::dup(dirfd) };
        if dup < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let dirp = unsafe { libc::fdopendir(dup) };
        if dirp.is_null() {
            unsafe { libc::close(dup) };
            return Err(std::io::Error::last_os_error());
        }
        let mut names = Vec::new();
        loop {
            let ent = unsafe { libc::readdir(dirp) };
            if ent.is_null() {
                break;
            }
            let cn = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) };
            let b = cn.to_bytes();
            if b == b"." || b == b".." {
                continue;
            }
            names.push(OsStr::from_bytes(b).to_os_string());
        }
        unsafe { libc::closedir(dirp) };
        Ok(names)
    }

    /// Recursively remove `name` under `dirfd`, fd-anchored and `O_NOFOLLOW`:
    /// a directory is entered via `openat(O_NOFOLLOW)` and emptied then
    /// `rmdir`'d; a symlink (or any non-directory) is `unlinkat`'d — never
    /// followed. So a symlink an agent planted in trash pointing out of the
    /// tree is deleted as a link, and a directory component swapped to a
    /// symlink between passes fails the `O_NOFOLLOW` open (its subtree is left
    /// for a later sweep rather than followed out of the tree).
    fn rm_recursive_at(dirfd: i32, name: &OsStr) {
        let Ok(c) = CString::new(name.as_bytes()) else {
            return;
        };
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(dirfd, c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } < 0 {
            return;
        }
        if (st.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            if let Ok(child) = openat_dir_nofollow(dirfd, name) {
                if let Ok(names) = read_dir_fd(child.as_raw_fd()) {
                    for n in names {
                        rm_recursive_at(child.as_raw_fd(), &n);
                    }
                }
                unsafe { libc::unlinkat(dirfd, c.as_ptr(), libc::AT_REMOVEDIR) };
            }
        } else {
            unsafe { libc::unlinkat(dirfd, c.as_ptr(), 0) };
        }
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
