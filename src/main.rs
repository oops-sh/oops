// Parts of the backend surface are only exercised per-platform.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
mod backend;
mod diff;
mod session;
mod state;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use state::StateRoots;

#[cfg(target_os = "macos")]
const SCOPE_NOTE: &str = "\
Guarantee boundary: the sandbox covers filesystem writes under the current \
directory tree only. Writes outside it (/tmp, $HOME, other mounts), network \
side effects, and spawned processes are NOT captured and cannot be undone.\n\
On macOS the model is snapshot-restore: the command runs against your real \
files and `oops undo` puts them back — between run and undo/commit the tree \
holds the command's changes, and cloud sync clients or file watchers can \
observe (and may propagate) that transient state.";

#[cfg(not(target_os = "macos"))]
const SCOPE_NOTE: &str = "\
Guarantee boundary: the sandbox covers filesystem writes under the current \
directory tree only. Writes outside it (/tmp, $HOME, other mounts), network \
side effects, and spawned processes are NOT captured and cannot be undone.";

#[derive(Parser)]
#[command(
    name = "oops",
    version,
    about = "Undo for your terminal: any command can be undone."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a command in an undoable sandbox
    #[command(after_help = SCOPE_NOTE)]
    Run {
        /// The command, passed to `sh -c`
        #[arg(value_name = "COMMAND")]
        command: String,
    },
    /// Show what the pending sandbox created, modified, or deleted
    Diff {
        /// Stable machine-readable output for scripts/agents: `A/M/D path`
        /// lines, byte-order sorted, never colored (frozen format)
        #[arg(long)]
        porcelain: bool,
    },
    /// Discard the pending sandbox, restoring your files
    Undo,
    /// Keep the pending sandbox's changes
    Commit,
    /// Internal: sandbox child (unshare + mount + exec)
    #[command(name = "__exec", hide = true)]
    Exec {
        #[arg(long)]
        target: PathBuf,
        #[arg(long)]
        upper: PathBuf,
        #[arg(long)]
        work: PathBuf,
        #[arg(long)]
        marker: PathBuf,
        /// Explicit privileged (root) fallback: plain mount ns, no nested
        /// userns. Tier-1/2 only.
        #[arg(long)]
        privileged: bool,
        #[arg(value_name = "COMMAND")]
        command: String,
    },
    /// Internal: sweep orphaned state (trash, recordless sessions)
    #[command(name = "__gc", hide = true)]
    Gc,
}

fn main() {
    let cli = Cli::parse();
    let code = match dispatch(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("oops: error: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

#[allow(unused_variables)]
fn dispatch(cmd: Cmd) -> Result<i32> {
    match cmd {
        Cmd::Run { command } => run(&command),
        Cmd::Diff { porcelain } => diff_cmd(porcelain),
        Cmd::Undo => undo(),
        Cmd::Commit => commit(),
        Cmd::Gc => {
            // Enter a matching identity-mapped user namespace first (Linux):
            // rootless overlay leaves `work/work` owned by the mapped uid with
            // mode 000, which the plain unprivileged user cannot delete. As
            // userns root here we hold CAP_DAC_OVERRIDE over the mapped uid and
            // can reclaim it. Best-effort — if it fails, gc still runs and
            // reclaims everything that is deletable without it.
            #[cfg(target_os = "linux")]
            session::enter_gc_userns();
            session::gc_sweep(&StateRoots::load()?)?;
            Ok(0)
        }
        Cmd::Exec {
            target,
            upper,
            work,
            marker,
            privileged,
            command,
        } => {
            #[cfg(target_os = "linux")]
            {
                // Only returns on error, always before the command has run.
                backend::overlayfs::enter_and_exec(
                    &target, &upper, &work, &marker, &command, privileged,
                )?;
                unreachable!("enter_and_exec returned Ok without exec")
            }
            #[cfg(not(target_os = "linux"))]
            bail!("__exec is only available on Linux")
        }
    }
}

/// The invocation directory, canonicalized — the sandbox target.
fn target_dir() -> Result<PathBuf> {
    std::env::current_dir()?
        .canonicalize()
        .context("cannot resolve the current directory")
}

/// Find the pending session for where the user is. Primary key: canonical
/// cwd. Fallback: the logical `$PWD` — after `oops run` deleted or replaced
/// the target directory itself, getcwd fails (or resolves elsewhere), but
/// the shell's $PWD still names the recorded target, which is exactly the
/// situation undo exists to fix.
fn pending_session(roots: &StateRoots) -> Result<session::Session> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = target_dir() {
        candidates.push(cwd);
    }
    if let Some(pwd) = std::env::var_os("PWD").map(PathBuf::from) {
        if pwd.is_absolute() && !candidates.contains(&pwd) {
            candidates.push(pwd);
        }
    }
    if candidates.is_empty() {
        bail!("cannot resolve the current directory (and $PWD is not set)");
    }
    for candidate in &candidates {
        if let Some(sess) = session::find_for_target(roots, candidate)? {
            return Ok(sess);
        }
    }
    bail!(
        "no pending sandbox for {} — run something first: oops run \"<command>\"",
        candidates[0].display()
    )
}

fn run(command: &str) -> Result<i32> {
    // Select the backend before anything else: on an unsupported platform
    // this refuses up front (fail closed) — the command never runs.
    let backend = backend::select()?;

    let mut roots = StateRoots::load()?;
    if let Err(e) = session::gc_sweep(&roots) {
        eprintln!("oops: warning: gc sweep failed: {e:#}");
    }

    let target = target_dir()?;
    if target == Path::new("/") {
        bail!("refusing to sandbox the filesystem root");
    }
    let root = roots.root_for_target(&target)?;
    if target.starts_with(&root) || root.starts_with(&target) {
        bail!("refusing to sandbox a directory that contains or lives in the oops state directory");
    }
    session::ensure_no_pending(&roots, &target)?;

    let sess = session::create(&root, &target, command, backend.kind())?;
    let sandbox = backend::sandbox_of(&sess.dir, &sess.record)?;

    let status = match backend.exec(&sandbox, command) {
        Ok(status) => status,
        Err(e) => {
            // Contract: exec Err ⇒ the command never executed. Fail closed
            // and discard the (unused) session.
            if session::move_to_trash(&roots, &sess.root, &sess.dir).is_ok() {
                session::spawn_background_gc();
            }
            return Err(e);
        }
    };

    let code = exit_code(status);
    let mut record = sess.record;
    record.exit_code = Some(code);
    session::save(&sess.dir, &record)?;

    eprintln!(
        "oops: sandboxed (exit {code}). `oops diff` to inspect, `oops undo` to discard, `oops commit` to keep."
    );
    Ok(code)
}

fn diff_cmd(porcelain: bool) -> Result<i32> {
    let roots = StateRoots::load()?;
    let sess = pending_session(&roots)?;
    let backend = backend::for_record(&sess.record)?;
    let sandbox = backend::sandbox_of(&sess.dir, &sess.record)?;
    ensure_not_stale(backend.as_ref(), &sandbox, &sess)?;
    let changes = backend.changes(&sandbox)?;
    if porcelain {
        print!("{}", diff::render_porcelain(&changes));
    } else {
        use std::io::IsTerminal;
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        print!("{}", diff::render_human(&changes, color));
    }
    Ok(0)
}

fn undo() -> Result<i32> {
    let roots = StateRoots::load()?;
    let sess = pending_session(&roots)?;
    let backend = backend::for_record(&sess.record)?;
    let sandbox = backend::sandbox_of(&sess.dir, &sess.record)?;
    // Target-side restore first (no-op for interception backends; the
    // identity-checked three-branch restore for snapshot-restore). Only
    // then is the session consumed — O(1) rename into trash + async gc.
    backend.restore(&sandbox)?;
    session::move_to_trash(&roots, &sess.root, &sess.dir)?;
    session::spawn_background_gc();
    eprintln!(
        "oops: undone 💨 (discarded the sandbox from `oops run \"{}\"`)",
        sess.record.command
    );
    Ok(0)
}

fn commit() -> Result<i32> {
    let roots = StateRoots::load()?;
    let sess = pending_session(&roots)?;
    let backend = backend::for_record(&sess.record)?;
    let sandbox = backend::sandbox_of(&sess.dir, &sess.record)?;
    ensure_not_stale(backend.as_ref(), &sandbox, &sess)
        .context("refusing to commit a stale sandbox")?;
    backend.merge(&sandbox)?;
    session::move_to_trash(&roots, &sess.root, &sess.dir)?;
    session::spawn_background_gc();
    eprintln!(
        "oops: committed `oops run \"{}\"` to the real files.",
        sess.record.command
    );
    Ok(0)
}

fn ensure_not_stale(
    backend: &dyn backend::SnapshotBackend,
    sandbox: &backend::Sandbox,
    sess: &session::Session,
) -> Result<()> {
    if backend.is_stale(sandbox) {
        bail!(
            "the sandbox state for {} is gone (stale session)",
            sess.record.target.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}

#[cfg(not(unix))]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
