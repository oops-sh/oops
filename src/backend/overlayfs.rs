//! OverlayFS snapshot backend (Linux only).
//!
//! `exec` re-runs the oops binary as a hidden `__exec` child that unshares a
//! mount namespace, mounts an overlay directly over the target directory,
//! and execs the command. The namespace dies with the child, so no mount
//! outlives a run; the upper layer on disk is the entire pending state.
//!
//! The overlay is mounted with `redirect_dir=off,metacopy=off`, so the upper
//! layer's only special encodings are whiteouts (char device 0:0) and
//! opaque-directory xattrs — which `changes` and `merge` handle explicitly.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use anyhow::{bail, Context, Result};

use super::{validate_mount_path, Change, ChangeKind, Sandbox, SnapshotBackend};

pub struct OverlayFs;

impl SnapshotBackend for OverlayFs {
    fn exec(&self, sandbox: &Sandbox, command: &str) -> Result<ExitStatus> {
        for p in [&sandbox.target, &sandbox.upper, &sandbox.work] {
            validate_mount_path(p)?;
        }
        let exe = std::env::current_exe().context("cannot locate the oops binary for re-exec")?;
        let status = std::process::Command::new(exe)
            .arg("__exec")
            .arg("--target")
            .arg(&sandbox.target)
            .arg("--upper")
            .arg(&sandbox.upper)
            .arg("--work")
            .arg(&sandbox.work)
            .arg("--marker")
            .arg(super::marker_path(sandbox))
            .arg("--")
            .arg(command)
            .status()
            .context("failed to spawn the sandbox child process")?;
        Ok(status)
    }

    fn changes(&self, sandbox: &Sandbox) -> Result<Vec<Change>> {
        let mut out = Vec::new();
        walk_changes(
            &sandbox.upper,
            &sandbox.target,
            true,
            &PathBuf::new(),
            &mut out,
        )?;
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    fn merge(&self, sandbox: &Sandbox) -> Result<()> {
        // Phase A: validate the whole upper layer before touching the real
        // tree. Any overlay xattr we do not recognize aborts the commit.
        let total = validate_upper(&sandbox.upper)?;
        // Phase B: fail-stop, idempotent replay.
        let mut applied = 0usize;
        replay(&sandbox.upper, &sandbox.target, &mut applied).map_err(|e| {
            e.context(format!(
                "commit stopped after applying {applied} of {total} entries; \
                 the session is preserved — fix the cause and re-run `oops commit`"
            ))
        })?;
        Ok(())
    }
}

/// The `__exec` child: enter a private mount namespace, mount the overlay
/// over the target, mark the sandbox as started, and become the command.
/// Only returns on error, and only before the command has executed.
pub fn enter_and_exec(
    target: &Path,
    upper: &Path,
    work: &Path,
    marker: &Path,
    command: &str,
) -> Result<()> {
    use nix::mount::{mount, MsFlags};
    use nix::sched::{unshare, CloneFlags};
    use std::os::unix::process::CommandExt;

    unshare(CloneFlags::CLONE_NEWNS).context(
        "unshare(CLONE_NEWNS) failed — oops needs root (or a privileged container) in Phase 0",
    )?;
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("failed to make mounts private in the sandbox namespace")?;

    let data = format!(
        "lowerdir={},upperdir={},workdir={},redirect_dir=off,metacopy=off",
        target.display(),
        upper.display(),
        work.display()
    );
    mount(
        Some("overlay"),
        target,
        Some("overlay"),
        MsFlags::empty(),
        Some(data.as_str()),
    )
    .with_context(|| {
        format!(
            "failed to mount overlay over {} (options: {data}).\n\
                 Note: the oops state directory cannot itself be on overlayfs \
                 (in the dev container it must be tmpfs).",
            target.display()
        )
    })?;

    std::env::set_current_dir(target)
        .with_context(|| format!("cannot chdir into sandboxed {}", target.display()))?;

    // Point of no return: from here on, failures belong to the command.
    std::fs::write(marker, b"")
        .with_context(|| format!("cannot write started marker {}", marker.display()))?;

    let err = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .exec();
    // exec only returns on failure; undo the marker so the parent treats
    // this as "command never ran".
    let _ = std::fs::remove_file(marker);
    Err(err).context("failed to exec /bin/sh")
}

fn is_whiteout(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_char_device() && meta.rdev() == 0
}

fn get_xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let cpath = CString::new(path.as_os_str().as_bytes()).ok()?;
    let cname = CString::new(name).ok()?;
    let mut buf = [0u8; 256];
    let n = unsafe {
        libc::lgetxattr(
            cpath.as_ptr(),
            cname.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if n < 0 {
        None
    } else {
        Some(buf[..n as usize].to_vec())
    }
}

fn list_xattrs(path: &Path) -> Vec<String> {
    let Ok(cpath) = CString::new(path.as_os_str().as_bytes()) else {
        return Vec::new();
    };
    let mut buf = vec![0u8; 4096];
    let n = unsafe { libc::llistxattr(cpath.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n <= 0 {
        return Vec::new();
    }
    buf.truncate(n as usize);
    buf.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

fn is_opaque(path: &Path) -> bool {
    ["trusted.overlay.opaque", "user.overlay.opaque"]
        .iter()
        .any(|n| get_xattr(path, n).as_deref() == Some(b"y"))
}

/// Overlay xattr suffixes that are safe to ignore or handle during replay.
/// `redirect` and `metacopy` are deliberately absent: we mount with those
/// features off, and seeing them means an upper layer we cannot replay.
const RECOGNIZED_OVERLAY_XATTRS: &[&str] = &["opaque", "origin", "impure", "nlink", "uuid"];

fn check_overlay_xattrs(path: &Path) -> Result<()> {
    for name in list_xattrs(path) {
        let suffix = name
            .strip_prefix("trusted.overlay.")
            .or_else(|| name.strip_prefix("user.overlay."));
        if let Some(suffix) = suffix {
            if !RECOGNIZED_OVERLAY_XATTRS.contains(&suffix) {
                bail!(
                    "unrecognized overlay metadata `{name}` on {} — refusing to commit \
                     (the upper layer cannot be replayed reliably)",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

/// Pre-commit validation walk. Returns the number of replayable entries.
fn validate_upper(upper: &Path) -> Result<usize> {
    let mut count = 0usize;
    let mut stack = vec![upper.to_path_buf()];
    while let Some(dir) = stack.pop() {
        check_overlay_xattrs(&dir)?;
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            check_overlay_xattrs(&path)?;
            count += 1;
            if path.symlink_metadata()?.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(count)
}

fn lower_kind(lower: &Path) -> Option<std::fs::FileType> {
    lower.symlink_metadata().ok().map(|m| m.file_type())
}

fn walk_changes(
    upper_dir: &Path,
    lower_dir: &Path,
    lower_present: bool,
    rel: &Path,
    out: &mut Vec<Change>,
) -> Result<()> {
    for entry in std::fs::read_dir(upper_dir)
        .with_context(|| format!("cannot read upper layer {}", upper_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let rel_path = rel.join(&name);
        let lower_path = lower_dir.join(&name);
        let meta = path.symlink_metadata()?;
        let lkind = if lower_present {
            lower_kind(&lower_path)
        } else {
            None
        };

        if is_whiteout(&meta) {
            out.push(Change {
                kind: ChangeKind::Deleted,
                path: rel_path,
                is_dir: lkind.map(|k| k.is_dir()).unwrap_or(false),
            });
        } else if meta.is_dir() {
            let lower_is_dir = lkind.map(|k| k.is_dir()).unwrap_or(false);
            if is_opaque(&path) && lower_is_dir {
                // The directory was deleted and recreated: report the
                // deletion, then everything inside is an addition.
                out.push(Change {
                    kind: ChangeKind::Deleted,
                    path: rel_path.clone(),
                    is_dir: true,
                });
                out.push(Change {
                    kind: ChangeKind::Added,
                    path: rel_path.clone(),
                    is_dir: true,
                });
                walk_changes(&path, &lower_path, false, &rel_path, out)?;
            } else if lower_is_dir {
                // Present on both sides: just a traversal node, recurse.
                walk_changes(&path, &lower_path, true, &rel_path, out)?;
            } else {
                out.push(Change {
                    kind: ChangeKind::Added,
                    path: rel_path.clone(),
                    is_dir: true,
                });
                walk_changes(&path, &lower_path, false, &rel_path, out)?;
            }
        } else {
            let kind = if lkind.is_some() {
                ChangeKind::Modified
            } else {
                ChangeKind::Added
            };
            out.push(Change {
                kind,
                path: rel_path,
                is_dir: false,
            });
        }
    }
    Ok(())
}

/// Replay one directory level of the upper layer onto the real tree.
/// Every step is idempotent, so a failed commit can be re-run.
fn replay(upper_dir: &Path, lower_dir: &Path, applied: &mut usize) -> Result<()> {
    for entry in std::fs::read_dir(upper_dir)? {
        let entry = entry?;
        let path = entry.path();
        let lower_path = lower_dir.join(entry.file_name());
        let meta = path.symlink_metadata()?;

        if is_whiteout(&meta) {
            remove_lower(&lower_path)
                .with_context(|| format!("cannot delete {}", lower_path.display()))?;
            *applied += 1;
        } else if meta.is_dir() {
            if is_opaque(&path) {
                remove_lower(&lower_path)
                    .with_context(|| format!("cannot replace {}", lower_path.display()))?;
            }
            match lower_kind(&lower_path) {
                Some(k) if k.is_dir() => {}
                Some(_) => remove_lower(&lower_path)?,
                None => {}
            }
            if lower_kind(&lower_path).is_none() {
                std::fs::create_dir(&lower_path)
                    .with_context(|| format!("cannot create {}", lower_path.display()))?;
            }
            std::fs::set_permissions(&lower_path, std::fs::Permissions::from_mode(meta.mode()))
                .with_context(|| format!("cannot set mode on {}", lower_path.display()))?;
            *applied += 1;
            replay(&path, &lower_path, applied)?;
        } else if meta.file_type().is_symlink() {
            remove_lower(&lower_path)?;
            let dest = std::fs::read_link(&path)?;
            std::os::unix::fs::symlink(&dest, &lower_path)
                .with_context(|| format!("cannot create symlink {}", lower_path.display()))?;
            *applied += 1;
        } else {
            if lower_kind(&lower_path).map(|k| k.is_dir()).unwrap_or(false) {
                remove_lower(&lower_path)?;
            }
            std::fs::copy(&path, &lower_path)
                .with_context(|| format!("cannot write {}", lower_path.display()))?;
            *applied += 1;
        }
    }
    Ok(())
}

fn remove_lower(path: &Path) -> std::io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
