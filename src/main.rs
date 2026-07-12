// Parts of the backend surface are only exercised on Linux.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod backend;
mod diff;
mod session;
mod state;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use backend::Sandbox;

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
    /// List paths the pending sandbox created (A), modified (M), or deleted (D)
    Diff,
    /// Discard the pending sandbox; the real files are untouched
    Undo,
    /// Apply the pending sandbox to the real files
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
        Cmd::Diff => diff_cmd(),
        Cmd::Undo => undo(),
        Cmd::Commit => commit(),
        Cmd::Gc => {
            session::gc_sweep(&state::state_dir()?)?;
            Ok(0)
        }
        Cmd::Exec {
            target,
            upper,
            work,
            marker,
            command,
        } => {
            #[cfg(target_os = "linux")]
            {
                // Only returns on error, always before the command has run.
                backend::overlayfs::enter_and_exec(&target, &upper, &work, &marker, &command)?;
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

fn pending_session(state: &Path) -> Result<session::Session> {
    let target = target_dir()?;
    session::find_for_target(state, &target)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no pending sandbox for {} — run something first: oops run \"<command>\"",
            target.display()
        )
    })
}

fn sandbox_of(record: &session::SessionRecord) -> Sandbox {
    Sandbox {
        target: record.target.clone(),
        upper: record.upper.clone(),
        work: record.work.clone(),
    }
}

fn run(command: &str) -> Result<i32> {
    // Select the backend before anything else: on an unsupported platform
    // this refuses up front (fail closed) — the command never runs.
    let backend = backend::select()?;

    let state = state::state_dir()?;
    std::fs::create_dir_all(state::sessions_dir(&state))?;
    std::fs::create_dir_all(state::trash_dir(&state))?;
    if let Err(e) = session::gc_sweep(&state) {
        eprintln!("oops: warning: gc sweep failed: {e:#}");
    }

    let target = target_dir()?;
    if target == Path::new("/") {
        bail!("refusing to sandbox the filesystem root");
    }
    if target.starts_with(&state) || state.starts_with(&target) {
        bail!("refusing to sandbox a directory that contains or lives in the oops state directory");
    }
    session::ensure_no_pending(&state, &target)?;

    let sess = session::create(&state, &target, command)?;
    let sandbox = sandbox_of(&sess.record);
    let marker = backend::marker_path(&sandbox);

    let status = match backend.exec(&sandbox, command) {
        Ok(status) => status,
        Err(e) => {
            discard_session(&state, &sess.dir);
            return Err(e);
        }
    };

    if !marker.exists() {
        // The child died before exec'ing the command: sandbox setup failed.
        // The command never ran; discard the empty session. Fail closed.
        discard_session(&state, &sess.dir);
        bail!("sandbox setup failed (see the message above); the command was NOT executed");
    }

    let code = exit_code(status);
    let mut record = sess.record;
    record.exit_code = Some(code);
    session::save(&sess.dir, &record)?;

    eprintln!(
        "oops: sandboxed (exit {code}). `oops diff` to inspect, `oops undo` to discard, `oops commit` to keep."
    );
    Ok(code)
}

fn diff_cmd() -> Result<i32> {
    let backend = backend::select()?;
    let state = state::state_dir()?;
    let sess = pending_session(&state)?;
    ensure_not_stale(&sess)?;
    let changes = backend.changes(&sandbox_of(&sess.record))?;
    print!("{}", diff::render(&changes));
    Ok(0)
}

fn undo() -> Result<i32> {
    let state = state::state_dir()?;
    let sess = pending_session(&state)?;
    // O(1) critical section: one rename into trash. Deletion is async.
    session::move_to_trash(&state, &sess.dir)?;
    session::spawn_background_gc();
    eprintln!(
        "oops: undone 💨 (discarded the sandbox from `oops run \"{}\"`)",
        sess.record.command
    );
    Ok(0)
}

fn commit() -> Result<i32> {
    let backend = backend::select()?;
    let state = state::state_dir()?;
    let sess = pending_session(&state)?;
    ensure_not_stale(&sess)
        .context("refusing to commit a stale sandbox (`oops undo` cleans it up)")?;
    backend.merge(&sandbox_of(&sess.record))?;
    session::move_to_trash(&state, &sess.dir)?;
    session::spawn_background_gc();
    eprintln!(
        "oops: committed `oops run \"{}\"` to the real files.",
        sess.record.command
    );
    Ok(0)
}

/// A session whose upper layer is gone (e.g. state on tmpfs across a
/// reboot) can still be undone, but must not be diffed or committed.
fn ensure_not_stale(sess: &session::Session) -> Result<()> {
    if !sess.record.upper.is_dir() {
        bail!(
            "the sandbox layers for {} are gone (stale session, e.g. after a reboot)",
            sess.record.target.display()
        );
    }
    Ok(())
}

fn discard_session(state: &Path, dir: &Path) {
    if session::move_to_trash(state, dir).is_ok() {
        session::spawn_background_gc();
    }
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
