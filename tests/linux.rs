//! Destructive integration tests for the OverlayFS sandbox loop.
//!
//! Safety spec: these tests mangle filesystems, so they run ONLY inside the
//! Linux dev container (`make test-linux`), guarded by OOPS_TEST_CONTAINER.
//! Outside the container every test is a skip, not a failure.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

/// Skip (successfully) unless we are inside the dedicated test container.
macro_rules! container_only {
    () => {
        if std::env::var_os("OOPS_TEST_CONTAINER").is_none() {
            eprintln!("skipped: destructive test outside the oops test container");
            return;
        }
    };
}

fn oops_bin() -> &'static str {
    env!("CARGO_BIN_EXE_oops")
}

fn oops_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(oops_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn oops")
}

fn run_ok(dir: &Path, cmd: &str) {
    let out = oops_in(dir, &["run", cmd]);
    assert!(
        out.status.success(),
        "oops run `{cmd}` failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Deterministic manifest of a tree: path, type, mode, and content of every
/// entry, sorted. Two identical manifests == byte-identical trees.
fn manifest(root: &Path) -> String {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let meta = path.symlink_metadata().unwrap();
            use std::os::unix::fs::MetadataExt;
            if meta.is_dir() {
                out.push(format!("d {rel} {:o}", meta.mode()));
                walk(root, &path, out);
            } else if meta.file_type().is_symlink() {
                out.push(format!(
                    "l {rel} -> {}",
                    std::fs::read_link(&path).unwrap().display()
                ));
            } else {
                let content = std::fs::read(&path).unwrap();
                out.push(format!("f {rel} {:o} {}", meta.mode(), content.len()));
                out.push(format!("  {}", String::from_utf8_lossy(&content)));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out.join("\n")
}

fn make_target() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oops-test-")
        .tempdir()
        .expect("tempdir")
}

/// Locate the pending session record for a target by scanning the state dir.
fn session_record_for(target: &Path) -> Option<(PathBuf, serde_json::Value)> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/state"))
        .join("oops");
    let sessions = state.join("sessions");
    for entry in std::fs::read_dir(sessions).ok()? {
        let dir = entry.ok()?.path();
        let Ok(raw) = std::fs::read_to_string(dir.join("session.json")) else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
        if v["target"].as_str() == Some(&target.to_string_lossy() as &str) {
            return Some((dir, v));
        }
    }
    None
}

#[test]
fn run_redirects_writes_to_upper_layer() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();
    std::fs::write(t.join("existing"), "keep").unwrap();
    let before = manifest(&t);

    run_ok(&t, "echo hi > new.txt");

    assert_eq!(
        manifest(&t),
        before,
        "the real tree must be untouched by a sandboxed run"
    );
    let (dir, v) = session_record_for(&t).expect("session record exists");
    assert_eq!(v["exit_code"], 0);
    let upper = dir.join("upper");
    assert_eq!(
        std::fs::read_to_string(upper.join("new.txt")).unwrap(),
        "hi\n"
    );

    let out = oops_in(&t, &["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn flagship_demo_rm_rf_then_undo() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();
    std::fs::create_dir_all(t.join("testdir/nested")).unwrap();
    std::fs::write(t.join("testdir/a.txt"), "alpha").unwrap();
    std::fs::write(t.join("testdir/nested/b.txt"), "beta").unwrap();
    std::fs::write(t.join("keep.txt"), "outside").unwrap();
    let before = manifest(&t);

    run_ok(&t, "rm -rf testdir");
    assert_eq!(manifest(&t), before, "rm -rf must not touch the real tree");

    // The deletion is visible in the diff (whiteout in the upper layer).
    let out = oops_in(&t, &["diff"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "D testdir/\n");

    // The upper layer encodes the deletion as a char 0:0 whiteout.
    let (dir, _) = session_record_for(&t).expect("session pending");
    let meta = dir.join("upper/testdir").symlink_metadata().unwrap();
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    assert!(
        meta.file_type().is_char_device() && meta.rdev() == 0,
        "expected whiteout"
    );

    let out = oops_in(&t, &["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        manifest(&t),
        before,
        "undo must restore a byte-identical tree"
    );

    // Nothing pending anymore.
    let out = oops_in(&t, &["undo"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no pending sandbox"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn diff_classifies_mixed_changes_sorted() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();
    std::fs::write(t.join("m"), "old").unwrap();
    std::fs::write(t.join("d"), "doomed").unwrap();

    run_ok(&t, "echo x > n && echo more >> m && rm d");

    let out = oops_in(&t, &["diff"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "D d\nM m\nA n\n",
        "sorted by path, A/M/D classified"
    );

    // diff is read-only and repeatable.
    let again = oops_in(&t, &["diff"]);
    assert_eq!(stdout(&again), "D d\nM m\nA n\n");

    oops_in(&t, &["undo"]);
}

#[test]
fn commit_applies_creations_modifications_deletions() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();
    std::fs::write(t.join("m"), "old").unwrap();
    std::fs::create_dir(t.join("d")).unwrap();
    std::fs::write(t.join("d/inner"), "x").unwrap();

    run_ok(
        &t,
        "printf mod > m && rm -rf d && mkdir sub && printf new > sub/n && chmod 700 sub",
    );

    let out = oops_in(&t, &["commit"]);
    assert!(out.status.success(), "{}", stderr(&out));

    assert_eq!(std::fs::read_to_string(t.join("m")).unwrap(), "mod");
    assert!(
        !t.join("d").exists(),
        "whiteout must become a real deletion"
    );
    assert_eq!(std::fs::read_to_string(t.join("sub/n")).unwrap(), "new");
    use std::os::unix::fs::MetadataExt;
    assert_eq!(
        t.join("sub").metadata().unwrap().mode() & 0o777,
        0o700,
        "dir mode preserved"
    );

    // Session consumed.
    let out = oops_in(&t, &["diff"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no pending sandbox"));
}

#[test]
fn commit_aborts_on_unrecognized_overlay_xattr_and_retry_completes() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();

    run_ok(&t, "mkdir newdir && echo f > newdir/f");
    let (dir, _) = session_record_for(&t).expect("session pending");
    let upper_newdir = dir.join("upper/newdir");

    // Simulate upper-layer metadata we cannot replay (as if redirect_dir had
    // been active). Commit must abort before touching the real tree and
    // preserve the session, and a retry after fixing must complete — the
    // fail-stop + idempotent-retry contract.
    set_xattr(&upper_newdir, "trusted.overlay.redirect", b"/elsewhere");

    let out = oops_in(&t, &["commit"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("unrecognized overlay metadata"),
        "{}",
        stderr(&out)
    );
    assert!(
        !t.join("newdir").exists(),
        "commit must abort before modifying the real tree"
    );
    assert!(
        session_record_for(&t).is_some(),
        "session must be preserved on failure"
    );

    remove_xattr(&upper_newdir, "trusted.overlay.redirect");
    let out = oops_in(&t, &["commit"]);
    assert!(
        out.status.success(),
        "retry must complete: {}",
        stderr(&out)
    );
    assert_eq!(std::fs::read_to_string(t.join("newdir/f")).unwrap(), "f\n");
}

#[test]
fn wrapped_exit_status_is_propagated() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();

    let out = oops_in(&t, &["run", "exit 7"]);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    // The failing command's sandbox is preserved for inspection.
    assert!(session_record_for(&t).is_some());
    oops_in(&t, &["undo"]);
}

#[test]
fn second_run_is_refused_while_pending() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();

    run_ok(&t, "true");
    let out = oops_in(&t, &["run", "true"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already pending"), "{}", stderr(&out));
    oops_in(&t, &["undo"]);
}

#[test]
fn no_pending_sandbox_errors() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();
    for verb in ["diff", "undo", "commit"] {
        let out = oops_in(&t, &[verb]);
        assert!(
            !out.status.success(),
            "{verb} must fail with nothing pending"
        );
        assert!(
            stderr(&out).contains("no pending sandbox"),
            "{verb}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn sandbox_setup_failure_never_runs_the_command() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();
    // A state dir on overlayfs (the container root) makes the overlay mount
    // fail: upperdir-on-overlay is rejected by the kernel. Fail closed.
    let bad_state = tempfile::Builder::new()
        .prefix("oops-badstate-")
        .tempdir()
        .unwrap();

    let out = Command::new(oops_bin())
        .args(["run", "touch evidence"])
        .current_dir(&t)
        .env("XDG_STATE_HOME", bad_state.path())
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert!(stderr(&out).contains("NOT executed"), "{}", stderr(&out));
    assert!(
        !t.join("evidence").exists(),
        "the command must never have run"
    );
}

#[test]
fn gc_quarantines_old_orphans() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();

    let state = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/state/oops");
    let orphan = state.join("sessions/orphan-test-12345");
    std::fs::create_dir_all(&orphan).unwrap();
    // Age it past the gc quarantine threshold.
    let ok = Command::new("touch")
        .args(["-d", "2020-01-01T00:00:00", orphan.to_str().unwrap()])
        .status()
        .unwrap()
        .success();
    assert!(ok);

    run_ok(&t, "true");
    assert!(!orphan.exists(), "old recordless session dir must be swept");
    oops_in(&t, &["undo"]);
}

#[test]
#[ignore = "benchmark: run via `make bench-linux`"]
fn bench_undo_under_100ms_on_repo_sized_tree() {
    container_only!();
    let target = make_target();
    let t = target.path().canonicalize().unwrap();

    // Repo-sized tree: 100 dirs x 100 files = 10_000 files.
    for d in 0..100 {
        let dir = t.join(format!("sub/dir{d:03}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            std::fs::write(dir.join(format!("f{f:03}.txt")), "content\n").unwrap();
        }
    }
    let before = manifest(&t);

    run_ok(&t, "rm -rf sub");

    let start = Instant::now();
    let out = oops_in(&t, &["undo"]);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(manifest(&t), before);

    eprintln!("undo took {elapsed:?} (target < 100ms)");
    assert!(
        elapsed.as_millis() < 100,
        "undo took {elapsed:?}, target is < 100ms"
    );
}

fn set_xattr(path: &Path, name: &str, value: &[u8]) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let p = CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = CString::new(name).unwrap();
    let rc = unsafe {
        libc::lsetxattr(
            p.as_ptr(),
            n.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    assert_eq!(
        rc,
        0,
        "lsetxattr {name} failed: {}",
        std::io::Error::last_os_error()
    );
}

fn remove_xattr(path: &Path, name: &str) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let p = CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = CString::new(name).unwrap();
    let rc = unsafe { libc::lremovexattr(p.as_ptr(), n.as_ptr()) };
    assert_eq!(
        rc,
        0,
        "lremovexattr {name} failed: {}",
        std::io::Error::last_os_error()
    );
}
