//! OverlayFS snapshot backend (Linux only).
//!
//! `exec` re-runs the oops binary as a hidden `__exec` child. Rootless
//! (default): the child creates an unprivileged user namespace ("A") + mount
//! namespace, mounts an overlay over the target, then execs the command in a
//! NESTED child user namespace ("B") that holds no capability over A's
//! mounts — so the command cannot unmount the sandbox or escape the mount
//! namespace (the tier-3 boundary). The namespaces die with the child, so no
//! mount outlives a run; the upper layer on disk is the entire pending state.
//! An explicit `OOPS_PRIVILEGED=1` opt-in keeps the historical root path
//! (plain mount ns, no userns B; tier-1/2 only).
//!
//! The overlay is mounted `metacopy=off,userxattr` (rootless mounts reject
//! `redirect_dir=off`). The upper layer's special encodings are therefore
//! whiteouts (char device 0:0), opaque-directory xattrs
//! (`user.overlay.opaque`), and directory-rename redirects
//! (`user.overlay.redirect`) — the last is untrusted, adversary-writable
//! input, so `merge` validates it statically and enforces containment at
//! mutate time with an `O_NOFOLLOW` component walk. `changes` and `merge`
//! handle exactly this set and refuse anything else.

use std::ffi::{CString, OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};
use std::process::ExitStatus;

use anyhow::{bail, Context, Result};

use super::{validate_mount_path, Change, ChangeKind, Layers, Sandbox, SnapshotBackend};

pub struct OverlayFs;

fn layers(sandbox: &Sandbox) -> Result<(&PathBuf, &PathBuf)> {
    match &sandbox.layers {
        Layers::Overlay { upper, work, .. } => Ok((upper, work)),
        _ => anyhow::bail!("overlayfs backend given a non-overlay session"),
    }
}

/// The recorded target-parent identity (`st_dev`, `st_ino`) for an overlay
/// session, if present. Commit re-verifies it so the tree root cannot be
/// swapped between run and commit (the undo-containment identity anchor).
fn overlay_parent_identity(sandbox: &Sandbox) -> Option<(u64, u64)> {
    match &sandbox.layers {
        Layers::Overlay {
            parent_dev: Some(d),
            parent_ino: Some(i),
            ..
        } => Some((*d, *i)),
        _ => None,
    }
}

fn marker_path(sandbox: &Sandbox) -> PathBuf {
    // In the session directory: outside any layer, inside oops state.
    sandbox.session_dir.join("started")
}

impl SnapshotBackend for OverlayFs {
    fn exec(&self, sandbox: &Sandbox, command: &str) -> Result<ExitStatus> {
        let (upper, work) = layers(sandbox)?;
        for p in [&sandbox.target, upper, work] {
            validate_mount_path(p)?;
        }
        let marker = marker_path(sandbox);
        // Rootless by default; the historical root path is an explicit,
        // honestly-scoped opt-in (tier-1/2 only). See enter_and_exec.
        let privileged = std::env::var_os("OOPS_PRIVILEGED").is_some();
        let exe = std::env::current_exe().context("cannot locate the oops binary for re-exec")?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("__exec")
            .arg("--target")
            .arg(&sandbox.target)
            .arg("--upper")
            .arg(upper)
            .arg("--work")
            .arg(work)
            .arg("--marker")
            .arg(&marker);
        if privileged {
            cmd.arg("--privileged");
        }
        let status = cmd
            .arg("--")
            .arg(command)
            .status()
            .context("failed to spawn the sandbox child process")?;
        // Contract: Err ⇒ the command never executed. The child writes the
        // marker only after the sandbox is fully set up, immediately before
        // exec'ing the command — no marker means setup failed.
        if !marker.exists() {
            anyhow::bail!(
                "sandbox setup failed (see the message above); the command was NOT executed"
            );
        }
        Ok(status)
    }

    fn changes(&self, sandbox: &Sandbox) -> Result<Vec<Change>> {
        let (upper, _) = layers(sandbox)?;
        let mut out = Vec::new();
        walk_changes(upper, &sandbox.target, true, &PathBuf::new(), &mut out)?;
        super::sort_changes(&mut out);
        Ok(out)
    }

    fn restore(&self, _sandbox: &Sandbox) -> Result<()> {
        // Interception: the real tree was never touched; discarding the
        // layer (done by the caller via trash) is the whole undo.
        Ok(())
    }

    fn merge(&self, sandbox: &Sandbox) -> Result<()> {
        let (upper, _) = layers(sandbox)?;
        // Phase A (read-only classification): walk the upper ONCE, reject any
        // overlay metadata outside the recognized set and any redirect value
        // that escapes the tree, and record the ordered operation list.
        // Nothing real is touched here. This static check is necessary but
        // NOT sufficient (see Phase B).
        let mut ops = Vec::new();
        classify(upper, Path::new(""), &mut ops)?;
        let total = ops.len();
        // Verified fds anchor every path resolution in Phase B: the tree root
        // (write side) on the recorded parent identity so it cannot be swapped
        // between run and commit; the upper root (read side) opened nofollow.
        let root = open_verified_root(&sandbox.target, overlay_parent_identity(sandbox))?;
        let upper_root = open_dir_nofollow(upper)
            .with_context(|| format!("cannot open upper layer {} (nofollow)", upper.display()))?;
        // Phase B: fail-stop, idempotent replay. It acts ONLY on `ops` and
        // never re-reads either layer's directory structure; every mutation
        // (and every upper-side read) is performed relative to O_NOFOLLOW
        // component-walked fds, so a symlink planted between the passes cannot
        // redirect a read or a write out of the tree (the TOCTOU guarantee).
        let mut applied = 0usize;
        apply_ops(&ops, &root, &upper_root, &mut applied).map_err(|e| {
            e.context(format!(
                "commit stopped after applying {applied} of {total} operations; \
                 the session is preserved — fix the cause and re-run `oops commit`"
            ))
        })?;
        Ok(())
    }

    fn is_stale(&self, sandbox: &Sandbox) -> bool {
        layers(sandbox)
            .map(|(upper, _)| !upper.is_dir())
            .unwrap_or(true)
    }

    fn kind(&self) -> crate::session::BackendKind {
        crate::session::BackendKind::Overlayfs
    }
}

/// Overlay mount options. Uniform across the rootless and privileged paths:
/// `userxattr` puts overlay metadata in the `user.overlay.*` namespace (the
/// only namespace an unprivileged mount can write), and it is required for
/// rootless. `redirect_dir` is NOT forced off — unprivileged overlay rejects
/// that option — so directory renames are encoded as `user.overlay.redirect`
/// and handled by the commit-replay path. `metacopy=off` guarantees full
/// copy-ups (no partial-copy-up entries). See openspec/specs/sandbox.
fn overlay_mount_data(target: &Path, upper: &Path, work: &Path) -> String {
    format!(
        "lowerdir={},upperdir={},workdir={},metacopy=off,userxattr",
        target.display(),
        upper.display(),
        work.display()
    )
}

/// Write a single-identity uid/gid map for the current process's user
/// namespace: `0 <outer_id> 1`. `setgroups` is denied first, which
/// unprivileged gid mapping requires. This needs no `/etc/subuid` ranges and
/// no setuid helper — it works for any unprivileged user.
fn write_identity_maps(outer_uid: u32, outer_gid: u32) -> Result<()> {
    // Best-effort: on some kernels setgroups is already "deny" and the write
    // is rejected; ignore that specific failure and let the gid_map write be
    // the real signal.
    let _ = std::fs::write("/proc/self/setgroups", b"deny");
    std::fs::write("/proc/self/uid_map", format!("0 {outer_uid} 1"))
        .context("cannot write /proc/self/uid_map for the user namespace")?;
    std::fs::write("/proc/self/gid_map", format!("0 {outer_gid} 1"))
        .context("cannot write /proc/self/gid_map for the user namespace")?;
    Ok(())
}

/// The `__exec` child: set up the sandbox namespaces, mount the overlay over
/// the target, mark the sandbox as started, and become the command. Only
/// returns on error, and only before the command has executed.
///
/// Rootless (default): create an unprivileged user namespace A + mount
/// namespace, mount the overlay inside it, then run the command in a NESTED
/// child user namespace B that holds no capability over A's mounts — so the
/// command cannot unmount the sandbox or escape the mount namespace (the
/// tier-3 boundary; see openspec/specs/sandbox and the confinement spike).
///
/// Privileged (explicit opt-in): the historical root path — a plain mount
/// namespace, command runs with CAP_SYS_ADMIN, no nested userns. Tier-1/2
/// only (a cooperative agent can `umount` its way out).
pub fn enter_and_exec(
    target: &Path,
    upper: &Path,
    work: &Path,
    marker: &Path,
    command: &str,
    privileged: bool,
) -> Result<()> {
    use nix::mount::{mount, MsFlags};
    use nix::sched::{unshare, CloneFlags};
    use std::os::unix::process::CommandExt;

    // Capture the invoking identity before entering any user namespace.
    let outer_uid = unsafe { libc::geteuid() };
    let outer_gid = unsafe { libc::getegid() };

    if privileged {
        unshare(CloneFlags::CLONE_NEWNS).context(
            "unshare(CLONE_NEWNS) failed — privileged mode needs root (or a privileged container)",
        )?;
    } else {
        // User namespace A owns the mount namespace. A single unshare of both
        // puts us in a new userns (initially unmapped/nobody) with a new mount
        // ns; writing the identity map makes us uid 0 in A with full caps
        // there, enough to mount the overlay.
        unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS).map_err(|e| {
            anyhow::anyhow!(
                "unshare(CLONE_NEWUSER|CLONE_NEWNS) failed: {e}.\n\
                 Rootless oops needs unprivileged user namespaces (kernel >= 5.11).\n\
                 If your distro restricts them, enable one of:\n  \
                 sysctl kernel.unprivileged_userns_clone=1   (Debian/older)\n  \
                 sysctl kernel.apparmor_restrict_unprivileged_userns=0   (Ubuntu 23.10+)\n\
                 or re-run with OOPS_PRIVILEGED=1 (requires root; weaker, tier-1/2 guarantee)."
            )
        })?;
        write_identity_maps(outer_uid, outer_gid)?;
    }

    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("failed to make mounts private in the sandbox namespace")?;

    let data = overlay_mount_data(target, upper, work);
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

    if !privileged {
        // Nested child user namespace B: a descendant of A, so it holds no
        // CAP_SYS_ADMIN over A's mount namespace. The command runs here and
        // cannot umount the overlay or nsenter out. Map A's root (0) to B's
        // root (0) so the command keeps the invoking user's identity.
        unshare(CloneFlags::CLONE_NEWUSER)
            .context("unshare(CLONE_NEWUSER) for the nested command namespace failed")?;
        write_identity_maps(0, 0)?;
    }

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

/// The redirect xattr value on an upper directory (a directory rename's
/// source), if present. Rootless mounts store it in `user.overlay.*`; the
/// `trusted.*` form is also read defensively.
fn redirect_value(path: &Path) -> Option<OsString> {
    for name in ["user.overlay.redirect", "trusted.overlay.redirect"] {
        if let Some(v) = get_xattr(path, name) {
            return Some(OsString::from_vec(v));
        }
    }
    None
}

/// Overlay xattr suffixes commit knows how to handle or safely ignore.
/// `redirect` is now recognized (rootless mounts cannot force `redirect_dir`
/// off); its VALUE is untrusted and separately validated. `metacopy` stays
/// absent — we mount `metacopy=off`, so seeing it means an unreplayable
/// layer.
const RECOGNIZED_OVERLAY_XATTRS: &[&str] =
    &["opaque", "redirect", "origin", "impure", "nlink", "uuid"];

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

/// Normalize a redirect value to a tree-relative path, treating it as
/// untrusted input. Absolute values (leading `/`) resolve from the tree root;
/// relative values from `base_rel` (the tree-relative dir containing the
/// redirected entry). Any `..` that would pop above the root is an escape and
/// errors. This is the static (necessary-but-not-sufficient) containment
/// check; the mutate-time `O_NOFOLLOW` walk is what actually enforces it.
fn normalize_redirect(value: &OsStr, base_rel: &Path) -> Result<PathBuf> {
    let v = Path::new(value);
    let mut stack: Vec<OsString> = Vec::new();
    if !v.is_absolute() {
        for c in base_rel.components() {
            if let Component::Normal(n) = c {
                stack.push(n.to_os_string());
            }
        }
    }
    for c in v.components() {
        match c {
            Component::RootDir | Component::Prefix(_) => stack.clear(),
            Component::CurDir => {}
            Component::ParentDir => {
                if stack.pop().is_none() {
                    bail!("redirect value {value:?} escapes the protected tree via `..`");
                }
            }
            Component::Normal(n) => stack.push(n.to_os_string()),
        }
    }
    if stack.is_empty() {
        bail!("redirect value {value:?} resolves to the tree root");
    }
    Ok(stack.iter().collect())
}

/// One replay operation, fully resolved and validated at classification time.
/// Phase B consumes this list and never re-reads the upper layer's structure,
/// so a metadata or path-structure change between the two passes cannot steer
/// a real-file mutation. All `rel` paths are relative to the tree root.
enum Op {
    /// Delete the lower path (a whiteout).
    Whiteout { rel: PathBuf },
    /// Ensure a directory exists at `rel`; `opaque` first clears the lower.
    Dir {
        rel: PathBuf,
        mode: u32,
        opaque: bool,
    },
    /// Directory rename: move the validated in-tree `source` onto `rel`.
    Redirect {
        rel: PathBuf,
        mode: u32,
        source: PathBuf,
    },
    /// Create or replace a symlink at `rel` pointing to `target`.
    Symlink { rel: PathBuf, target: PathBuf },
    /// Copy a regular file: read the upper at `rel`, write the lower at `rel`.
    File { rel: PathBuf, mode: u32 },
}

/// Read-only classification pass: walk the upper ONCE, reject unrecognized or
/// tree-escaping metadata, and record the ordered operation list. Within a
/// level, non-whiteout ops (and the whole subtree below them) are emitted
/// before that level's whiteouts, so a directory rename moves its source
/// before any sibling whiteout would delete it.
fn classify(upper_dir: &Path, rel: &Path, ops: &mut Vec<Op>) -> Result<()> {
    check_overlay_xattrs(upper_dir)?;
    let mut whiteouts: Vec<OsString> = Vec::new();
    let mut subdirs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(upper_dir)? {
        let entry = entry?;
        let upath = entry.path();
        let name = entry.file_name();
        let meta = upath.symlink_metadata()?;
        if is_whiteout(&meta) {
            whiteouts.push(name);
            continue;
        }
        check_overlay_xattrs(&upath)?;
        let rel_path = rel.join(&name);
        if meta.is_dir() {
            if let Some(val) = redirect_value(&upath) {
                // Resolve + validate the untrusted redirect value to a
                // root-relative in-tree source (or abort). `rel` is the dir
                // containing the entry.
                let source = normalize_redirect(&val, rel)?;
                ops.push(Op::Redirect {
                    rel: rel_path.clone(),
                    mode: meta.mode(),
                    source,
                });
            } else {
                ops.push(Op::Dir {
                    rel: rel_path.clone(),
                    mode: meta.mode(),
                    opaque: is_opaque(&upath),
                });
            }
            subdirs.push((upath, rel_path));
        } else if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&upath)?;
            ops.push(Op::Symlink {
                rel: rel_path,
                target,
            });
        } else {
            ops.push(Op::File {
                rel: rel_path,
                mode: meta.mode(),
            });
        }
    }
    // Children after their parent dir op (so parents exist first).
    for (upath, rel_path) in subdirs {
        classify(&upath, &rel_path, ops)?;
    }
    // Whiteouts last within this level.
    for name in whiteouts {
        ops.push(Op::Whiteout {
            rel: rel.join(name),
        });
    }
    Ok(())
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
            if redirect_value(&path).is_some() {
                // A directory renamed into this path: it appears as an
                // addition here; the source's deletion is a separate whiteout
                // entry elsewhere in the upper layer.
                out.push(Change {
                    kind: ChangeKind::Added,
                    path: rel_path.clone(),
                    is_dir: true,
                });
                walk_changes(&path, &lower_path, false, &rel_path, out)?;
            } else if is_opaque(&path) && lower_is_dir {
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

// ---------------------------------------------------------------------------
// Mutate-time containment: every real-file mutation below — and every
// upper-side read — is performed relative to a directory fd obtained by
// walking the path one component at a time with `O_NOFOLLOW`, rooted at a
// verified root. A symlink at any component (from either layer, including one
// planted between the classify and apply passes) makes the open fail, so no
// read or write can be redirected out of its tree. This is the TOCTOU-safe
// half of the redirect containment; the static check in `classify` is the
// necessary-but-not-sufficient half.
// ---------------------------------------------------------------------------

fn cstr(s: &OsStr) -> Result<CString> {
    CString::new(s.as_bytes()).context("path component contains a NUL byte")
}

fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// Open the target tree root, refusing a symlinked root and verifying the
/// recorded parent identity (`st_dev`/`st_ino`) if present — so the root
/// cannot have been swapped between run and commit.
fn open_verified_root(target: &Path, parent_id: Option<(u64, u64)>) -> Result<OwnedFd> {
    let parent = target.parent().context("target has no parent directory")?;
    let fname = target
        .file_name()
        .context("target has no final component")?;

    let pc = CString::new(parent.as_os_str().as_bytes())?;
    let pfd = unsafe {
        libc::open(
            pc.as_ptr(),
            libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if pfd < 0 {
        return Err(errno())
            .with_context(|| format!("cannot open target parent {}", parent.display()));
    }
    let pfd = unsafe { OwnedFd::from_raw_fd(pfd) };

    if let Some((dev, ino)) = parent_id {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(pfd.as_raw_fd(), &mut st) } < 0 {
            return Err(errno()).context("cannot stat target parent");
        }
        if st.st_dev as u64 != dev || st.st_ino as u64 != ino {
            bail!(
                "target parent identity changed since run (dev/ino mismatch) — \
                 refusing to commit and modifying nothing"
            );
        }
    }
    open_dir_nofollow_at(pfd.as_raw_fd(), fname)
        .with_context(|| format!("cannot open target root {} (nofollow)", target.display()))
}

/// `openat(dirfd, name, O_DIRECTORY|O_NOFOLLOW|O_RDONLY)`.
fn open_dir_nofollow_at(dirfd: i32, name: &OsStr) -> Result<OwnedFd> {
    let c = cstr(name)?;
    let fd = unsafe {
        libc::openat(
            dirfd,
            c.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(errno()).with_context(|| format!("openat {name:?} (nofollow dir)"));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Walk `rel` (which MUST contain only normal components) from `base`,
/// opening each intermediate directory with `O_NOFOLLOW`. Returns the fd of
/// the final component's parent directory and the final component name.
fn resolve_parent_nofollow(base: &OwnedFd, rel: &Path) -> Result<(OwnedFd, OsString)> {
    let mut comps: Vec<OsString> = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(n) => comps.push(n.to_os_string()),
            _ => bail!("refusing to resolve unsafe path component in {rel:?}"),
        }
    }
    if comps.is_empty() {
        bail!("empty relative path");
    }
    let mut cur = base.try_clone().context("cannot dup directory fd")?;
    for name in &comps[..comps.len() - 1] {
        cur = open_dir_nofollow_at(cur.as_raw_fd(), name)?;
    }
    Ok((cur, comps.last().unwrap().clone()))
}

/// Directory entry names (excluding `.`/`..`) under a directory fd.
fn read_dir_fd(dirfd: &OwnedFd) -> Result<Vec<OsString>> {
    let dup = unsafe { libc::dup(dirfd.as_raw_fd()) };
    if dup < 0 {
        return Err(errno()).context("dup for readdir");
    }
    let dirp = unsafe { libc::fdopendir(dup) };
    if dirp.is_null() {
        unsafe { libc::close(dup) };
        return Err(errno()).context("fdopendir");
    }
    let mut names = Vec::new();
    loop {
        let ent = unsafe { libc::readdir(dirp) };
        if ent.is_null() {
            break;
        }
        let cname = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) };
        let bytes = cname.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        names.push(OsStr::from_bytes(bytes).to_os_string());
    }
    unsafe { libc::closedir(dirp) };
    Ok(names)
}

/// Remove `name` under `dirfd` (file, symlink, or recursively a directory).
/// Absent is success (idempotent).
fn remove_at(dirfd: i32, name: &OsStr) -> Result<()> {
    let c = cstr(name)?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(dirfd, c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } < 0 {
        let e = errno();
        if e.raw_os_error() == Some(libc::ENOENT) {
            return Ok(());
        }
        return Err(e).with_context(|| format!("fstatat {name:?}"));
    }
    if (st.st_mode & libc::S_IFMT) == libc::S_IFDIR {
        let child = open_dir_nofollow_at(dirfd, name)?;
        for ent in read_dir_fd(&child)? {
            remove_at(child.as_raw_fd(), &ent)?;
        }
        drop(child);
        if unsafe { libc::unlinkat(dirfd, c.as_ptr(), libc::AT_REMOVEDIR) } < 0 {
            return Err(errno()).with_context(|| format!("rmdir {name:?}"));
        }
    } else if unsafe { libc::unlinkat(dirfd, c.as_ptr(), 0) } < 0 {
        let e = errno();
        if e.raw_os_error() != Some(libc::ENOENT) {
            return Err(e).with_context(|| format!("unlink {name:?}"));
        }
    }
    Ok(())
}

/// Create `name` as a directory under `dirfd` if absent; set its mode.
fn ensure_dir_at(dirfd: i32, name: &OsStr, mode: u32) -> Result<()> {
    let c = cstr(name)?;
    // If a non-directory sits here, replace it.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(dirfd, c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } == 0
        && (st.st_mode & libc::S_IFMT) != libc::S_IFDIR
    {
        remove_at(dirfd, name)?;
    }
    if unsafe { libc::mkdirat(dirfd, c.as_ptr(), mode as libc::mode_t) } < 0 {
        let e = errno();
        if e.raw_os_error() != Some(libc::EEXIST) {
            return Err(e).with_context(|| format!("mkdirat {name:?}"));
        }
    }
    Ok(())
}

fn fchmod(fd: i32, mode: u32) -> Result<()> {
    if unsafe { libc::fchmod(fd, mode as libc::mode_t) } < 0 {
        return Err(errno()).context("fchmod");
    }
    Ok(())
}

/// Write a regular file `name` under `dirfd` with `data` and `mode`,
/// replacing whatever was there. `O_NOFOLLOW` prevents following a planted
/// symlink; `name` is removed first so creation is fresh.
fn write_file_at(dirfd: i32, name: &OsStr, data: &[u8], mode: u32) -> Result<()> {
    use std::io::Write;
    remove_at(dirfd, name)?;
    let c = cstr(name)?;
    let fd = unsafe {
        libc::openat(
            dirfd,
            c.as_ptr(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(errno()).with_context(|| format!("create {name:?}"));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(data)
        .with_context(|| format!("write {name:?}"))?;
    fchmod(file.as_raw_fd(), mode)?;
    Ok(())
}

fn symlink_at(dirfd: i32, name: &OsStr, target: &Path) -> Result<()> {
    remove_at(dirfd, name)?;
    let cn = cstr(name)?;
    let ct = CString::new(target.as_os_str().as_bytes())?;
    if unsafe { libc::symlinkat(ct.as_ptr(), dirfd, cn.as_ptr()) } < 0 {
        return Err(errno()).with_context(|| format!("symlinkat {name:?}"));
    }
    Ok(())
}

/// Open a directory by path, refusing to follow a final symlink.
fn open_dir_nofollow(path: &Path) -> Result<OwnedFd> {
    let c = CString::new(path.as_os_str().as_bytes())?;
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(errno()).with_context(|| format!("open {} (nofollow dir)", path.display()));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `renameat(src_parent/src_name -> dst_parent/dst_name)`.
fn renameat(
    src_parent: &OwnedFd,
    src_name: &OsStr,
    dst_parent: &OwnedFd,
    dst_name: &OsStr,
) -> Result<()> {
    let sc = cstr(src_name)?;
    let dc = cstr(dst_name)?;
    if unsafe {
        libc::renameat(
            src_parent.as_raw_fd(),
            sc.as_ptr(),
            dst_parent.as_raw_fd(),
            dc.as_ptr(),
        )
    } < 0
    {
        return Err(errno()).with_context(|| format!("rename into {dst_name:?}"));
    }
    Ok(())
}

/// Read a regular file `name` under `dirfd`, refusing a symlink at the final
/// component (`O_NOFOLLOW`) — so a swapped upper component cannot redirect the
/// read outside the upper layer.
fn read_file_nofollow(dirfd: i32, name: &OsStr) -> Result<Vec<u8>> {
    use std::io::Read;
    let c = cstr(name)?;
    let fd = unsafe {
        libc::openat(
            dirfd,
            c.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(errno()).with_context(|| format!("open upper {name:?} (nofollow)"));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .with_context(|| format!("read upper {name:?}"))?;
    Ok(buf)
}

/// Mutation pass: apply the classified operation list. It never re-reads
/// either layer's directory structure — every path is resolved by an
/// `O_NOFOLLOW` component walk from a verified root fd (the tree root for the
/// write side, the upper root for the read side), so a symlink planted
/// between the passes cannot redirect a read or a write out of its tree.
fn apply_ops(
    ops: &[Op],
    root_fd: &OwnedFd,
    upper_root_fd: &OwnedFd,
    applied: &mut usize,
) -> Result<()> {
    for op in ops {
        match op {
            Op::Whiteout { rel } => {
                let (parent, name) = resolve_parent_nofollow(root_fd, rel)?;
                remove_at(parent.as_raw_fd(), &name)?;
            }
            Op::Dir { rel, mode, opaque } => {
                let (parent, name) = resolve_parent_nofollow(root_fd, rel)?;
                if *opaque {
                    remove_at(parent.as_raw_fd(), &name)?;
                }
                ensure_dir_at(parent.as_raw_fd(), &name, *mode)?;
                let child = open_dir_nofollow_at(parent.as_raw_fd(), &name)?;
                fchmod(child.as_raw_fd(), *mode)?;
            }
            Op::Redirect { rel, mode, source } => {
                let (src_parent, src_name) = resolve_parent_nofollow(root_fd, source)?;
                let (dst_parent, dst_name) = resolve_parent_nofollow(root_fd, rel)?;
                remove_at(dst_parent.as_raw_fd(), &dst_name)?;
                renameat(&src_parent, &src_name, &dst_parent, &dst_name)?;
                let child = open_dir_nofollow_at(dst_parent.as_raw_fd(), &dst_name)?;
                fchmod(child.as_raw_fd(), *mode)?;
            }
            Op::Symlink { rel, target } => {
                let (parent, name) = resolve_parent_nofollow(root_fd, rel)?;
                symlink_at(parent.as_raw_fd(), &name, target)?;
            }
            Op::File { rel, mode } => {
                let (up_parent, up_name) = resolve_parent_nofollow(upper_root_fd, rel)?;
                let data = read_file_nofollow(up_parent.as_raw_fd(), &up_name)?;
                let (parent, name) = resolve_parent_nofollow(root_fd, rel)?;
                write_file_at(parent.as_raw_fd(), &name, &data, *mode)?;
            }
        }
        *applied += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The upper-side read path is fd-anchored just like the write side: a
    // symlink at any component of an upper path refuses the read, so a
    // component swapped to a symlink between the classify and apply passes
    // cannot redirect a read out of the upper layer (the (c) TOCTOU detail).
    #[test]
    fn read_and_resolve_refuse_symlink_components() {
        let tmp = tempfile::tempdir().unwrap();
        let base = open_dir_nofollow(tmp.path()).unwrap();

        std::fs::write(tmp.path().join("f"), b"real bytes").unwrap();
        std::os::unix::fs::symlink("/etc/hostname", tmp.path().join("link")).unwrap();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/x"), b"x").unwrap();
        std::os::unix::fs::symlink("d", tmp.path().join("sdir")).unwrap();

        // A real file reads back; a symlink at the final component is refused.
        assert_eq!(
            read_file_nofollow(base.as_raw_fd(), OsStr::new("f")).unwrap(),
            b"real bytes"
        );
        assert!(read_file_nofollow(base.as_raw_fd(), OsStr::new("link")).is_err());

        // An intermediate symlink component is refused; the real dir resolves.
        assert!(resolve_parent_nofollow(&base, Path::new("sdir/x")).is_err());
        assert!(resolve_parent_nofollow(&base, Path::new("d/x")).is_ok());
    }
}
