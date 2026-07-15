//! The snapshot backend abstraction. Two models:
//! - **interception** (OverlayFS, Linux): the command's writes land in a
//!   layer and never touch the real tree;
//! - **snapshot-restore** (APFS, macOS): the real tree is mutated and can
//!   be atomically restored from a clonefile snapshot.
//! See openspec/specs/sandbox.

#[cfg(target_os = "macos")]
pub mod apfs;
#[cfg(target_os = "linux")]
pub mod overlayfs;

use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use anyhow::Result;

use crate::session::SessionRecord;

/// One pending sandbox, reconstructed from a session record.
pub struct Sandbox {
    pub target: PathBuf,
    pub session_dir: PathBuf,
    pub layers: Layers,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum Layers {
    Overlay {
        upper: PathBuf,
        work: PathBuf,
        /// The target's parent identity at run time (same anchor undo
        /// containment uses). Commit re-verifies it before replaying, so the
        /// tree root cannot be swapped between run and commit. Optional for
        /// backward compatibility with records written before it was
        /// threaded through.
        parent_dev: Option<u64>,
        parent_ino: Option<u64>,
    },
    Snapshot {
        snapshot: PathBuf,
        parent_dev: u64,
        parent_ino: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

impl ChangeKind {
    pub fn letter(self) -> char {
        match self {
            ChangeKind::Added => 'A',
            ChangeKind::Modified => 'M',
            ChangeKind::Deleted => 'D',
        }
    }
}

/// One changed path, relative to the sandbox target. `is_dir` controls the
/// trailing `/` in diff output.
#[derive(Debug)]
pub struct Change {
    pub kind: ChangeKind,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub trait SnapshotBackend {
    /// Run `command` (via `sh -c`) under this backend's protection model.
    /// Contract: an `Err` means the command was never executed (fail
    /// closed); `Ok(status)` means it ran and exited with `status`.
    fn exec(&self, sandbox: &Sandbox, command: &str) -> Result<ExitStatus>;

    /// Classify the pending changes into created/modified/deleted paths.
    /// Read-only: must not mutate the target, any layer, or the session.
    fn changes(&self, sandbox: &Sandbox) -> Result<Vec<Change>>;

    /// Undo's target-side work. Interception backends do nothing (the
    /// layer discard is the undo). Snapshot-restore backends restore the
    /// target per the safety spec's three branches.
    fn restore(&self, sandbox: &Sandbox) -> Result<()>;

    /// Commit's target-side work. OverlayFS replays the layer;
    /// snapshot-restore backends do nothing (the tree already has the
    /// changes).
    fn merge(&self, sandbox: &Sandbox) -> Result<()>;

    /// True if the sandbox's backing state is unusable (stale session).
    fn is_stale(&self, sandbox: &Sandbox) -> bool;

    fn kind(&self) -> crate::session::BackendKind;
}

/// Select the backend for this platform, failing closed when there is none.
pub fn select() -> Result<Box<dyn SnapshotBackend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(overlayfs::OverlayFs))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(apfs::Apfs))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!(
            "no snapshot backend supports this platform yet (OverlayFS on Linux, \
             APFS on macOS).\nRefusing to run the command unsandboxed."
        )
    }
}

/// The backend that created an existing session, by record name. Errors if
/// that backend is not available on this platform.
pub fn for_record(record: &SessionRecord) -> Result<Box<dyn SnapshotBackend>> {
    match record.backend.as_str() {
        #[cfg(target_os = "linux")]
        "overlayfs" => Ok(Box::new(overlayfs::OverlayFs)),
        #[cfg(target_os = "macos")]
        "apfs" => Ok(Box::new(apfs::Apfs)),
        other => anyhow::bail!(
            "session was created by the `{other}` backend, which is not available on this platform"
        ),
    }
}

/// Reconstruct a Sandbox from a session record.
pub fn sandbox_of(session_dir: &Path, record: &SessionRecord) -> Result<Sandbox> {
    let layers = match record.backend.as_str() {
        "overlayfs" => Layers::Overlay {
            upper: record
                .upper
                .clone()
                .ok_or_else(|| anyhow::anyhow!("overlayfs session record missing upper path"))?,
            work: record
                .work
                .clone()
                .ok_or_else(|| anyhow::anyhow!("overlayfs session record missing work path"))?,
            parent_dev: record.parent_dev,
            parent_ino: record.parent_ino,
        },
        "apfs" => Layers::Snapshot {
            snapshot: record
                .snapshot
                .clone()
                .ok_or_else(|| anyhow::anyhow!("apfs session record missing snapshot path"))?,
            parent_dev: record
                .parent_dev
                .ok_or_else(|| anyhow::anyhow!("apfs session record missing parent identity"))?,
            parent_ino: record
                .parent_ino
                .ok_or_else(|| anyhow::anyhow!("apfs session record missing parent identity"))?,
        },
        other => anyhow::bail!("unknown backend `{other}` in session record"),
    };
    Ok(Sandbox {
        target: record.target.clone(),
        session_dir: session_dir.to_path_buf(),
        layers,
    })
}

/// Porcelain contract: entries sort by the raw byte order of the path —
/// not locale collation, not path-component order.
pub fn sort_changes(changes: &mut [Change]) {
    changes.sort_by(|a, b| {
        a.path
            .as_os_str()
            .as_encoded_bytes()
            .cmp(b.path.as_os_str().as_encoded_bytes())
    });
}

/// Overlay mount option strings cannot represent these characters portably;
/// refuse rather than risk mounting the wrong paths (safety: fail closed).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn validate_mount_path(path: &Path) -> Result<()> {
    let s = path.to_string_lossy();
    if s.contains(':') || s.contains(',') || s.contains('\\') {
        anyhow::bail!(
            "path {} contains characters unsupported in overlay mount options (: , \\)",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_is_raw_byte_order_not_component_order() {
        // '-' (0x2d) < '/' (0x2f): "a-b" must sort before "a/c", which is
        // the opposite of PathBuf's component-wise ordering.
        let mut changes = vec![
            Change {
                kind: ChangeKind::Added,
                path: PathBuf::from("a/c"),
                is_dir: false,
            },
            Change {
                kind: ChangeKind::Added,
                path: PathBuf::from("a-b"),
                is_dir: false,
            },
        ];
        sort_changes(&mut changes);
        assert_eq!(changes[0].path, PathBuf::from("a-b"));
        assert_eq!(changes[1].path, PathBuf::from("a/c"));
    }
}
