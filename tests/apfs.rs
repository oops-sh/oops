//! Destructive integration tests for the APFS snapshot-restore backend.
//!
//! Safety spec triple gate — all three required, otherwise every test
//! skips:
//!   1. explicit state-root override: each test points XDG_STATE_HOME at
//!      its own temp directory (the developer's real state is never used);
//!   2. self-created temp trees for every target;
//!   3. OOPS_TEST_DESTRUCTIVE=1, set by `make test-apfs`, never by default.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

macro_rules! destructive_only {
    () => {
        if std::env::var("OOPS_TEST_DESTRUCTIVE").as_deref() != Ok("1") {
            eprintln!("skipped: destructive APFS test without OOPS_TEST_DESTRUCTIVE=1");
            return;
        }
    };
}

/// One test's world: its own state root (gate 1) and target tree (gate 2).
struct World {
    _keep: tempfile::TempDir,
    state_home: PathBuf,
    target: PathBuf,
}

fn world() -> World {
    let keep = tempfile::Builder::new()
        .prefix("oops-apfs-test-")
        .tempdir()
        .unwrap();
    let state_home = keep.path().join("state-home");
    let target = keep.path().join("target");
    std::fs::create_dir_all(&state_home).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    World {
        _keep: keep,
        state_home,
        target,
    }
}

impl World {
    fn oops(&self, args: &[&str]) -> Output {
        self.oops_in(&self.target, args)
    }

    fn oops_in(&self, dir: &Path, args: &[&str]) -> Output {
        // Set $PWD like a real shell would; the harness's own PWD (the
        // repo) must not leak into session lookup.
        Command::new(env!("CARGO_BIN_EXE_oops"))
            .args(args)
            .current_dir(dir)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("PWD", dir)
            .output()
            .expect("failed to spawn oops")
    }

    /// Like a shell whose cwd was deleted/replaced: run oops with cwd at
    /// `physical_cwd` but `$PWD` still naming the recorded target.
    fn oops_with_pwd(&self, physical_cwd: &Path, pwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_oops"))
            .args(args)
            .current_dir(physical_cwd)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("PWD", pwd)
            .output()
            .expect("failed to spawn oops")
    }

    fn run_ok(&self, cmd: &str) {
        let out = self.oops(&["run", cmd]);
        assert!(
            out.status.success(),
            "oops run `{cmd}` failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Deterministic manifest: path, type, mode, content of every entry, sorted.
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

fn seed_tree(target: &Path) {
    std::fs::create_dir_all(target.join("testdir/nested")).unwrap();
    std::fs::write(target.join("testdir/a.txt"), "alpha").unwrap();
    std::fs::write(target.join("testdir/nested/b.txt"), "beta").unwrap();
    std::fs::write(target.join("keep.txt"), "outside").unwrap();
}

#[test]
fn flagship_demo_rm_rf_then_undo() {
    destructive_only!();
    let w = world();
    seed_tree(&w.target);
    let before = manifest(&w.target);

    w.run_ok("rm -rf testdir");
    // Snapshot-restore: the damage is real...
    assert!(
        !w.target.join("testdir").exists(),
        "the real tree holds the command's changes"
    );

    let out = w.oops(&["diff", "--porcelain"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "D testdir/\n", "single non-expanded entry");

    // ...and fully reversible.
    let out = w.oops(&["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        manifest(&w.target),
        before,
        "undo must restore a byte-identical tree"
    );

    let out = w.oops(&["undo"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no pending sandbox"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn diff_classifies_mixed_changes_sorted() {
    destructive_only!();
    let w = world();
    std::fs::write(w.target.join("m"), "old").unwrap();
    std::fs::write(w.target.join("d"), "doomed").unwrap();

    w.run_ok("echo x > n && echo more >> m && rm d");

    for _ in 0..2 {
        let out = w.oops(&["diff", "--porcelain"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(
            stdout(&out),
            "D d\nM m\nA n\n",
            "byte-order sorted, A/M/D classified"
        );
    }
    let human = w.oops(&["diff"]);
    assert_eq!(
        stdout(&human),
        "Created (1)\n  n\n\nModified (1)\n  m\n\nDeleted (1)\n  d\n\n\
         1 created, 1 modified, 1 deleted\n"
    );
    w.oops(&["undo"]);
}

#[test]
fn commit_keeps_the_mutated_tree() {
    destructive_only!();
    let w = world();
    std::fs::write(w.target.join("m"), "old").unwrap();

    w.run_ok("printf mod > m && mkdir sub && printf new > sub/n");
    let start = Instant::now();
    let out = w.oops(&["commit"]);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(elapsed.as_millis() < 100, "O(1) commit took {elapsed:?}");

    assert_eq!(std::fs::read_to_string(w.target.join("m")).unwrap(), "mod");
    assert_eq!(
        std::fs::read_to_string(w.target.join("sub/n")).unwrap(),
        "new"
    );
    let out = w.oops(&["diff"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no pending sandbox"));
}

#[test]
fn wrapped_exit_status_is_propagated_and_second_run_refused() {
    destructive_only!();
    let w = world();

    let out = w.oops(&["run", "exit 7"]);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));

    let out = w.oops(&["run", "true"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already pending"), "{}", stderr(&out));
    w.oops(&["undo"]);
}

#[test]
fn no_pending_sandbox_errors() {
    destructive_only!();
    let w = world();
    for verb in ["diff", "undo", "commit"] {
        let out = w.oops(&[verb]);
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
fn restore_branch_target_replaced_by_command() {
    destructive_only!();
    let w = world();
    seed_tree(&w.target);
    let before = manifest(&w.target);

    // The command replaces the target directory's own content wholesale
    // (delete + recreate of entries). A changed target is a command change.
    w.run_ok("rm -rf testdir keep.txt && mkdir impostor && echo x > impostor/f");
    let out = w.oops(&["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(manifest(&w.target), before);
}

#[test]
fn restore_branch_target_deleted_rename_into_parent() {
    destructive_only!();
    let w = world();
    seed_tree(&w.target);
    let before = manifest(&w.target);

    // The command deletes the target directory itself.
    w.run_ok("cd / && rm -rf \"$OLDPWD\"");
    assert!(!w.target.exists(), "target itself should be gone");

    // A shell sitting in the deleted directory still has $PWD naming it;
    // the session lookup falls back to that, and restore takes the
    // rename-into-parent branch.
    let parent = w.target.parent().unwrap().to_path_buf();
    let out = w.oops_with_pwd(&parent, &w.target, &["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(manifest(&w.target), before);
}

#[test]
fn restore_branch_symlink_refused() {
    destructive_only!();
    let w = world();
    seed_tree(&w.target);

    w.run_ok("touch marker");
    // Simulate a hostile/accidental symlink at the target path after run.
    let parent = w.target.parent().unwrap().to_path_buf();
    let elsewhere = parent.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let displaced = parent.join("displaced");
    std::fs::rename(&w.target, &displaced).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &w.target).unwrap();

    // $PWD names the recorded target (now a symlink): the session is
    // found, and restore must refuse the symlink branch outright.
    let out = w.oops_with_pwd(&parent, &w.target, &["undo"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("symlink"), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing may be restored through the symlink"
    );
    assert!(
        displaced.join("marker").exists(),
        "displaced tree untouched"
    );
}

#[test]
fn restore_refused_on_parent_identity_mismatch() {
    destructive_only!();
    let w = world();
    // Layout: keep/parent/target — the parent is replaced after run.
    let parent = w.target.join("parent");
    let inner = parent.join("proj");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("f"), "x").unwrap();
    let inner = inner.canonicalize().unwrap();

    let out = w.oops_in(&inner, &["run", "rm -f f"]);
    assert!(out.status.success(), "{}", stderr(&out));

    // Replace the parent (new inode), recreate the same paths.
    std::fs::rename(&parent, w.target.join("parent-moved")).unwrap();
    std::fs::create_dir_all(&inner).unwrap();

    let out = w.oops_in(&inner, &["undo"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("identity mismatch"),
        "{}",
        stderr(&out)
    );
    assert!(
        w.target.join("parent-moved/proj").is_dir(),
        "nothing was moved or deleted"
    );
}

#[test]
fn stale_snapshot_refuses_undo_and_commit() {
    destructive_only!();
    let w = world();
    std::fs::write(w.target.join("f"), "x").unwrap();
    w.run_ok("rm f");

    // Destroy the snapshot behind oops's back.
    let sessions = w.state_home.join("oops/sessions");
    let sess_dir = std::fs::read_dir(&sessions)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::remove_dir_all(sess_dir.join("snapshot")).unwrap();

    let undo = w.oops(&["undo"]);
    assert!(!undo.status.success());
    assert!(stderr(&undo).contains("stale"), "{}", stderr(&undo));
    let commit = w.oops(&["commit"]);
    assert!(!commit.status.success());
    assert!(stderr(&commit).contains("stale"), "{}", stderr(&commit));
}

#[test]
fn fail_closed_when_state_root_unusable() {
    destructive_only!();
    let w = world();
    // A read-only XDG_STATE_HOME makes state-root creation fail: the
    // command must never run.
    use std::os::unix::fs::PermissionsExt;
    let ro = w.target.parent().unwrap().join("ro-state");
    std::fs::create_dir(&ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_oops"))
        .args(["run", "touch evidence"])
        .current_dir(&w.target)
        .env("XDG_STATE_HOME", &ro)
        .output()
        .unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(!out.status.success());
    assert!(
        !w.target.join("evidence").exists(),
        "the command must never have run"
    );
}

#[test]
fn displaced_tree_from_undo_is_reclaimed_by_gc() {
    destructive_only!();
    let w = world();
    seed_tree(&w.target);
    let before = manifest(&w.target);

    w.run_ok("rm -rf testdir && echo damage > evidence.txt");
    let out = w.oops(&["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(manifest(&w.target), before, "damage undone");

    // The swap displaced the damaged tree into the session directory,
    // and undo renamed that whole directory into trash: nothing pending.
    let oops_root = w.state_home.join("oops");
    assert_eq!(
        std::fs::read_dir(oops_root.join("sessions"))
            .unwrap()
            .count(),
        0,
        "the session (and displaced tree) must leave sessions/"
    );

    // Async reclamation: undo's background gc or a later sweep must
    // delete the trash entry. Poll with explicit sweeps so the test is
    // deterministic even if the background gc lost the race.
    let trash = oops_root.join("trash");
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let leftover = std::fs::read_dir(&trash).map(|d| d.count()).unwrap_or(0);
        if leftover == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "displaced tree still in trash after 5s of gc sweeps"
        );
        w.oops(&["__gc"]);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        !w.target.join("evidence.txt").exists(),
        "displaced content must not resurface in the target"
    );
}

#[test]
fn undo_after_new_process_like_a_reboot() {
    destructive_only!();
    let w = world();
    seed_tree(&w.target);
    let before = manifest(&w.target);

    w.run_ok("rm -rf testdir");
    // Every oops invocation is a fresh process reading state from disk —
    // the same recovery path a post-reboot/post-crash undo takes.
    let out = w.oops(&["undo"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(manifest(&w.target), before);
}

#[test]
#[ignore = "benchmark: run via `make bench-apfs`"]
fn bench_undo_apfs_under_100ms_on_repo_sized_tree() {
    destructive_only!();
    let w = world();
    for d in 0..100 {
        let dir = w.target.join(format!("sub/dir{d:03}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            std::fs::write(dir.join(format!("f{f:03}.txt")), "content\n").unwrap();
        }
    }
    let before = manifest(&w.target);

    let start = Instant::now();
    w.run_ok("rm -rf sub");
    let setup_and_cmd = start.elapsed();

    let start = Instant::now();
    let out = w.oops(&["undo"]);
    let undo_time = start.elapsed();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(manifest(&w.target), before);

    eprintln!(
        "apfs: run (clone+cmd) took {setup_and_cmd:?}; undo took {undo_time:?} (target < 100ms)"
    );
    assert!(
        undo_time.as_millis() < 100,
        "undo took {undo_time:?}, target is < 100ms"
    );
}
