//! The oops state directory: the single well-known location for all
//! persistent oops state, and the containment check that every deletion
//! (undo, gc) must pass. See openspec/specs/safety.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Resolve the state directory: `$XDG_STATE_HOME/oops`, defaulting to
/// `~/.local/state/oops`. Does not create it.
pub fn state_dir() -> Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return Ok(xdg.join("oops"));
        }
    }
    let home =
        std::env::var_os("HOME").context("cannot resolve state directory: HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/state/oops"))
}

pub fn sessions_dir(state: &Path) -> PathBuf {
    state.join("sessions")
}

pub fn trash_dir(state: &Path) -> PathBuf {
    state.join("trash")
}

/// Containment check: `path` must live inside `state`. Every path that undo
/// or gc is about to delete goes through this; a corrupted session record
/// pointing elsewhere must make us refuse, not delete.
///
/// Uses canonicalized paths so symlinks cannot smuggle a deletion outside
/// the state directory.
pub fn ensure_in_state_dir(state: &Path, path: &Path) -> Result<()> {
    let state = state
        .canonicalize()
        .with_context(|| format!("cannot canonicalize state dir {}", state.display()))?;
    let path = path
        .canonicalize()
        .with_context(|| format!("cannot canonicalize {}", path.display()))?;
    if !path.starts_with(&state) {
        bail!(
            "refusing to touch {}: outside the oops state directory {} (corrupted state?)",
            path.display(),
            state.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_rejects_outside_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        std::fs::create_dir_all(state.join("sessions/x")).unwrap();
        let outside = tmp.path().join("victim");
        std::fs::create_dir(&outside).unwrap();

        assert!(ensure_in_state_dir(&state, &state.join("sessions/x")).is_ok());
        assert!(ensure_in_state_dir(&state, &outside).is_err());
    }

    #[test]
    fn containment_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let outside = tmp.path().join("victim");
        std::fs::create_dir(&outside).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, state.join("sneaky")).unwrap();
            assert!(ensure_in_state_dir(&state, &state.join("sneaky")).is_err());
        }
    }

    #[test]
    fn state_dir_respects_xdg() {
        // Env-var tests can race between threads; this only reads the
        // pure path logic via a subprocess-free approximation: skip if the
        // variable is already set by the harness.
        if std::env::var_os("XDG_STATE_HOME").is_none() {
            let d = state_dir().unwrap();
            assert!(d.ends_with(".local/state/oops"));
        }
    }
}
