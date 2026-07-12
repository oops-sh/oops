//! The snapshot backend abstraction. OverlayFS (Linux) is the Phase 0
//! implementation; an APFS backend is planned. See openspec/specs/sandbox.

#[cfg(target_os = "linux")]
pub mod overlayfs;

use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use anyhow::Result;

/// Paths of one pending sandbox, as recorded in the session.
pub struct Sandbox {
    pub target: PathBuf,
    pub upper: PathBuf,
    pub work: PathBuf,
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
    /// Run `command` (via `sh -c`) with all filesystem writes to
    /// `sandbox.target` redirected into the upper layer. Must not execute
    /// the command at all if sandbox setup fails (safety: fail closed).
    fn exec(&self, sandbox: &Sandbox, command: &str) -> Result<ExitStatus>;

    /// Classify the upper layer into created/modified/deleted paths.
    /// Read-only: must not mutate any layer.
    fn changes(&self, sandbox: &Sandbox) -> Result<Vec<Change>>;

    /// Apply the upper layer to the real tree. Fail-stop and idempotent:
    /// on error, stop and leave both layers so a retry can complete.
    fn merge(&self, sandbox: &Sandbox) -> Result<()>;
}

/// Select the backend for this platform, failing closed when there is none.
pub fn select() -> Result<Box<dyn SnapshotBackend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(overlayfs::OverlayFs))
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!(
            "no snapshot backend supports this platform yet (OverlayFS is Linux-only; \
             an APFS backend is planned).\n\
             Refusing to run the command unsandboxed. On macOS, use the Linux dev \
             container: `make shell-linux`."
        )
    }
}

/// True when running inside the oops Linux test container (set by
/// docker/Dockerfile). Destructive integration tests refuse to run without it.
pub fn in_test_container() -> bool {
    std::env::var_os("OOPS_TEST_CONTAINER").is_some()
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

/// Path of the "command actually started" marker for a sandbox. The exec
/// child writes it after sandbox setup succeeds, immediately before the
/// command runs; its absence after a failed child means the command never
/// executed (fail closed). Lives in the session directory, outside any layer.
pub fn marker_path(sandbox: &Sandbox) -> PathBuf {
    sandbox
        .upper
        .parent()
        .unwrap_or(&sandbox.upper)
        .join("started")
}

/// Overlay mount option strings cannot represent these characters portably;
/// refuse rather than risk mounting the wrong paths (safety: fail closed).
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
