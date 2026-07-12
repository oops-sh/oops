//! APFS snapshot-restore backend (macOS only, rootless).
//!
//! `exec` takes a `clonefile(2)` copy-on-write snapshot of the target tree
//! into the session directory, then runs the command **against the real
//! tree**. Undo re-verifies the parent-directory identity recorded at run
//! time and restores via one of the three branches mandated by the safety
//! spec: atomic swap (target exists, not a symlink), rename-into-parent
//! (target deleted), refuse (target is a symlink). Commit is O(1): the
//! tree already has the changes; the snapshot goes to trash.
//!
//! Fine print (spec'd): between run and undo/commit the real tree holds
//! the command's changes; cloning is not atomic against concurrent
//! writers; modification detection is size + nanosecond mtime.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use super::{Change, ChangeKind, Layers, Sandbox, SnapshotBackend};

pub struct Apfs;

/// Written right after a successful restore so a crash between the swap
/// and session cleanup can never lead a second undo to swap the damage
/// back in.
const RESTORED_MARKER: &str = "restored";

fn snapshot_of(sandbox: &Sandbox) -> Result<(&PathBuf, u64, u64)> {
    match &sandbox.layers {
        Layers::Snapshot {
            snapshot,
            parent_dev,
            parent_ino,
        } => Ok((snapshot, *parent_dev, *parent_ino)),
        _ => bail!("apfs backend given a non-snapshot session"),
    }
}

fn cpath(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("path {} contains a NUL byte", path.display()))
}

fn clonefile(src: &Path, dst: &Path) -> Result<()> {
    let (csrc, cdst) = (cpath(src)?, cpath(dst)?);
    if unsafe { libc::clonefile(csrc.as_ptr(), cdst.as_ptr(), 0) } != 0 {
        let err = std::io::Error::last_os_error();
        bail!(
            "clonefile({} -> {}) failed: {err}\n\
             Note: the target and the oops state root must be on the same APFS volume.",
            src.display(),
            dst.display()
        );
    }
    Ok(())
}

fn rename_swap(a: &Path, b: &Path) -> Result<()> {
    let (ca, cb) = (cpath(a)?, cpath(b)?);
    if unsafe { libc::renamex_np(ca.as_ptr(), cb.as_ptr(), libc::RENAME_SWAP) } != 0 {
        let err = std::io::Error::last_os_error();
        bail!(
            "atomic swap of {} and {} failed: {err}",
            a.display(),
            b.display()
        );
    }
    Ok(())
}

impl SnapshotBackend for Apfs {
    fn exec(&self, sandbox: &Sandbox, command: &str) -> Result<ExitStatus> {
        let (snapshot, _, _) = snapshot_of(sandbox)?;

        // Spec: setup exceeding ~1s must give feedback rather than stall.
        let done = Arc::new(AtomicBool::new(false));
        let timer = {
            let done = done.clone();
            let target = sandbox.target.display().to_string();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if !done.load(Ordering::Relaxed) {
                    eprintln!("oops: snapshotting {target} (large tree, still working)...");
                }
            })
        };
        let clone_result = clonefile(&sandbox.target, snapshot);
        done.store(true, Ordering::Relaxed);
        let _ = timer; // detached; fires at most once
                       // Fail closed: any clone error means the command never runs.
        clone_result?;

        // Snapshot-restore: the command runs against the real tree.
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .current_dir(&sandbox.target)
            .status()
            .context("failed to spawn /bin/sh for the command")?;
        Ok(status)
    }

    fn changes(&self, sandbox: &Sandbox) -> Result<Vec<Change>> {
        let (snapshot, _, _) = snapshot_of(sandbox)?;
        let mut out = Vec::new();
        walk_deletions_and_mods(snapshot, &sandbox.target, &PathBuf::new(), &mut out)?;
        walk_additions(&sandbox.target, Some(snapshot), &PathBuf::new(), &mut out)?;
        super::sort_changes(&mut out);
        Ok(out)
    }

    fn restore(&self, sandbox: &Sandbox) -> Result<()> {
        let (snapshot, parent_dev, parent_ino) = snapshot_of(sandbox)?;
        let restored_marker = sandbox.session_dir.join(RESTORED_MARKER);
        if restored_marker.exists() {
            // A previous undo already swapped; never swap the damage back.
            return Ok(());
        }
        if !snapshot.is_dir() {
            bail!(
                "the snapshot for {} is gone (stale session) — nothing to restore from.\n\
                 If you are sure, remove the session under the oops state directory manually.",
                sandbox.target.display()
            );
        }

        // Identity anchor: the PARENT directory must be the one recorded at
        // run time. A replaced target is just another change; a replaced
        // parent means we are not where we think we are — refuse.
        let parent = sandbox
            .target
            .parent()
            .context("protected target has no parent directory")?;
        let pmeta = parent
            .symlink_metadata()
            .with_context(|| format!("cannot stat parent {}", parent.display()))?;
        if pmeta.dev() != parent_dev || pmeta.ino() != parent_ino {
            bail!(
                "refusing to restore: the parent directory {} is not the one recorded at run \
                 time (identity mismatch — was it moved or recreated?)",
                parent.display()
            );
        }

        // Three branches, per the safety spec.
        match sandbox.target.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!(
                    "refusing to restore: {} is now a symlink, which could redirect the \
                     restore outside the protected scope",
                    sandbox.target.display()
                );
            }
            Ok(_) => {
                // Target exists (original, replaced, or even a file): the
                // atomic swap restores the snapshot and captures the
                // displaced state inside the session directory.
                rename_swap(snapshot, &sandbox.target)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Target deleted by the command: plain rename into the
                // verified parent at the recorded name.
                std::fs::rename(snapshot, &sandbox.target).with_context(|| {
                    format!("cannot restore snapshot to {}", sandbox.target.display())
                })?;
            }
            Err(e) => return Err(e).context("cannot stat the protected target"),
        }
        std::fs::write(&restored_marker, b"")
            .context("restore succeeded but the restored marker could not be written")?;
        Ok(())
    }

    fn merge(&self, _sandbox: &Sandbox) -> Result<()> {
        // Snapshot-restore commit: the real tree already has the changes.
        // The caller trashes the snapshot; nothing to do here.
        Ok(())
    }

    fn is_stale(&self, sandbox: &Sandbox) -> bool {
        match snapshot_of(sandbox) {
            Ok((snapshot, _, _)) => !snapshot.is_dir(),
            Err(_) => true,
        }
    }

    fn kind(&self) -> crate::session::BackendKind {
        crate::session::BackendKind::Apfs
    }
}

fn meta_differs(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.size() != b.size() || a.mtime() != b.mtime() || a.mtime_nsec() != b.mtime_nsec()
}

/// Walk the snapshot against the live tree: missing in live ⇒ Deleted
/// (pruned — descendants of a deleted directory are never listed);
/// metadata drift on files ⇒ Modified; dirs on both sides recurse.
fn walk_deletions_and_mods(
    snap_dir: &Path,
    live_dir: &Path,
    rel: &Path,
    out: &mut Vec<Change>,
) -> Result<()> {
    for entry in std::fs::read_dir(snap_dir)
        .with_context(|| format!("cannot read snapshot {}", snap_dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let rel_path = rel.join(&name);
        let snap_path = entry.path();
        let live_path = live_dir.join(&name);
        let smeta = snap_path.symlink_metadata()?;

        match live_path.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push(Change {
                    kind: ChangeKind::Deleted,
                    path: rel_path,
                    is_dir: smeta.is_dir(),
                });
            }
            Err(e) => return Err(e.into()),
            Ok(lmeta) => {
                let (sdir, ldir) = (smeta.is_dir(), lmeta.is_dir());
                if sdir && ldir {
                    walk_deletions_and_mods(&snap_path, &live_path, &rel_path, out)?;
                } else if sdir != ldir {
                    // Type change: the old entry is gone, the new one is an
                    // addition (the additions walk emits it).
                    out.push(Change {
                        kind: ChangeKind::Deleted,
                        path: rel_path,
                        is_dir: sdir,
                    });
                } else if meta_differs(&smeta, &lmeta) {
                    out.push(Change {
                        kind: ChangeKind::Modified,
                        path: rel_path,
                        is_dir: false,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Walk the live tree against the snapshot: missing in snapshot ⇒ Added.
/// Added directories are recursed so their contents are listed too
/// (matching the overlayfs backend's output for created trees).
fn walk_additions(
    live_dir: &Path,
    snap_dir: Option<&Path>,
    rel: &Path,
    out: &mut Vec<Change>,
) -> Result<()> {
    for entry in std::fs::read_dir(live_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let rel_path = rel.join(&name);
        let live_path = entry.path();
        let lmeta = live_path.symlink_metadata()?;
        let snap_path = snap_dir.map(|d| d.join(&name));
        let in_snap = snap_path
            .as_ref()
            .map(|p| p.symlink_metadata())
            .map(|m| m.is_ok())
            .unwrap_or(false);

        if !in_snap || snap_path.as_ref().is_some_and(|p| type_changed(p, &lmeta)) {
            out.push(Change {
                kind: ChangeKind::Added,
                path: rel_path.clone(),
                is_dir: lmeta.is_dir(),
            });
            if lmeta.is_dir() {
                walk_additions(&live_path, None, &rel_path, out)?;
            }
        } else if lmeta.is_dir() {
            walk_additions(&live_path, snap_path.as_deref(), &rel_path, out)?;
        }
    }
    Ok(())
}

fn type_changed(snap_path: &Path, lmeta: &std::fs::Metadata) -> bool {
    snap_path
        .symlink_metadata()
        .map(|smeta| smeta.is_dir() != lmeta.is_dir())
        .unwrap_or(false)
}
