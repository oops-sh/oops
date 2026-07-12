//! Safety spec, "Unsupported platform" scenario: on a platform with no
//! snapshot backend (the macOS dev host), `oops run` must refuse to execute
//! the command rather than running it unsandboxed. This test is
//! non-destructive by design and runs on the host.
#![cfg(not(target_os = "linux"))]

use std::process::Command;

#[test]
fn run_refuses_without_a_backend_and_never_executes() {
    let target = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_oops"))
        .args(["run", "touch evidence"])
        .current_dir(target.path())
        .output()
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no snapshot backend"), "stderr: {stderr}");
    assert!(
        !target.path().join("evidence").exists(),
        "the command must never run unsandboxed"
    );
}
