//! oops state roots: the primary well-known directory plus registered
//! per-volume roots (snapshot-restore backends need same-volume state),
//! and the containment check that every deletion (undo, gc) must pass.
//! See openspec/specs/safety and openspec/specs/session.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub const REGISTRY_FILE: &str = "volumes.json";

/// The primary state root plus every registered per-volume root.
pub struct StateRoots {
    pub primary: PathBuf,
    pub registered: Vec<PathBuf>,
}

/// Resolve the primary state root: `$XDG_STATE_HOME/oops`, defaulting to
/// `~/.local/state/oops`. Does not create it.
pub fn primary_root() -> Result<PathBuf> {
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

impl StateRoots {
    pub fn load() -> Result<Self> {
        let primary = primary_root()?;
        let registered = read_registry(&primary).unwrap_or_default();
        Ok(StateRoots {
            primary,
            registered,
        })
    }

    /// Every root, primary first. Roots whose directory is absent (e.g. an
    /// unmounted volume) are skipped by callers that iterate `mounted()`.
    pub fn all(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.primary).chain(self.registered.iter())
    }

    /// Roots that currently exist on disk. Never creates anything.
    pub fn mounted(&self) -> Vec<PathBuf> {
        self.all().filter(|r| r.is_dir()).cloned().collect()
    }

    /// Containment check over the registered set: `path` must live inside
    /// one of the roots. Canonicalized, so symlinks cannot smuggle a
    /// deletion outside. A root that is on disk but not registered is
    /// foreign — it never grants deletion rights.
    pub fn ensure_contains(&self, path: &Path) -> Result<()> {
        let path = path
            .canonicalize()
            .with_context(|| format!("cannot canonicalize {}", path.display()))?;
        for root in self.all() {
            if let Ok(root) = root.canonicalize() {
                if path.starts_with(&root) {
                    return Ok(());
                }
            }
        }
        bail!(
            "refusing to touch {}: outside every registered oops state root (corrupted state?)",
            path.display()
        );
    }

    /// The state root for a target directory. Same volume as the primary
    /// root → primary. On macOS, a target on another volume gets
    /// `<volume-mount>/.oops/state`, created and registered (atomically)
    /// on first use; failure to create it fails closed.
    pub fn root_for_target(&mut self, target: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.primary)
            .with_context(|| format!("cannot create state root {}", self.primary.display()))?;

        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::MetadataExt;
            let primary_dev = self.primary.metadata()?.dev();
            let target_dev = target.metadata()?.dev();
            if target_dev != primary_dev {
                let mount = mount_point_of(target)?;
                let root = mount.join(".oops/state");
                std::fs::create_dir_all(&root).with_context(|| {
                    format!(
                        "cannot create per-volume state root {} (read-only volume?); \
                         refusing to run",
                        root.display()
                    )
                })?;
                if !self.registered.contains(&root) {
                    self.registered.push(root.clone());
                    write_registry(&self.primary, &self.registered)?;
                }
                return Ok(root);
            }
        }
        let _ = target; // linux: overlay layers have no same-volume constraint
        Ok(self.primary.clone())
    }
}

fn read_registry(primary: &Path) -> Option<Vec<PathBuf>> {
    let raw = std::fs::read_to_string(primary.join(REGISTRY_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(
        v.get("roots")?
            .as_array()?
            .iter()
            .filter_map(|s| s.as_str().map(PathBuf::from))
            .collect(),
    )
}

/// Atomic registry write: temp file + rename, so a crash can never leave a
/// truncated volumes.json.
fn write_registry(primary: &Path, roots: &[PathBuf]) -> Result<()> {
    let json = serde_json::json!({ "roots": roots }).to_string();
    let tmp = primary.join(format!("{REGISTRY_FILE}.tmp.{}", std::process::id()));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, primary.join(REGISTRY_FILE))
        .context("cannot commit volumes.json registry update")?;
    Ok(())
}

/// The mount point of the volume holding `path` (macOS: statfs f_mntonname).
#[cfg(target_os = "macos")]
fn mount_point_of(path: &Path) -> Result<PathBuf> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(path.as_os_str().as_bytes())?;
    let mut sfs: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(cpath.as_ptr(), &mut sfs) } != 0 {
        bail!(
            "statfs({}) failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    let mnt = unsafe { CStr::from_ptr(sfs.f_mntonname.as_ptr()) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(mnt.to_bytes())))
}

pub fn sessions_dir(root: &Path) -> PathBuf {
    root.join("sessions")
}

pub fn trash_dir(root: &Path) -> PathBuf {
    root.join("trash")
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
    fn containment_rejects_outside_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let r = roots(&tmp);
        std::fs::create_dir_all(r.primary.join("sessions/x")).unwrap();
        let outside = tmp.path().join("victim");
        std::fs::create_dir(&outside).unwrap();

        assert!(r.ensure_contains(&r.primary.join("sessions/x")).is_ok());
        assert!(r.ensure_contains(&outside).is_err());
    }

    #[test]
    fn containment_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let r = roots(&tmp);
        let outside = tmp.path().join("victim");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, r.primary.join("sneaky")).unwrap();
        assert!(r.ensure_contains(&r.primary.join("sneaky")).is_err());
    }

    #[test]
    fn registered_roots_grant_containment_but_foreign_dirs_do_not() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = roots(&tmp);
        let volume_root = tmp.path().join("volume/.oops/state");
        std::fs::create_dir_all(volume_root.join("trash/x")).unwrap();
        let foreign = tmp.path().join("foreign/.oops/state");
        std::fs::create_dir_all(foreign.join("trash/y")).unwrap();

        assert!(
            r.ensure_contains(&volume_root.join("trash/x")).is_err(),
            "unregistered = foreign"
        );
        r.registered.push(volume_root.clone());
        assert!(r.ensure_contains(&volume_root.join("trash/x")).is_ok());
        assert!(
            r.ensure_contains(&foreign.join("trash/y")).is_err(),
            "still foreign"
        );
    }

    #[test]
    fn registry_roundtrip_is_atomic_style() {
        let tmp = tempfile::tempdir().unwrap();
        let r = roots(&tmp);
        let roots_list = vec![tmp.path().join("v1"), tmp.path().join("v2")];
        write_registry(&r.primary, &roots_list).unwrap();
        assert_eq!(read_registry(&r.primary).unwrap(), roots_list);
        // No temp file left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&r.primary)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp.")
            })
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn mounted_skips_absent_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = roots(&tmp);
        r.registered
            .push(tmp.path().join("not-mounted/.oops/state"));
        let mounted = r.mounted();
        assert_eq!(mounted, vec![r.primary.clone()]);
    }
}
