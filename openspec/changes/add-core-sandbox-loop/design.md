# Design: add-core-sandbox-loop

## Context

Empty Rust crate; nothing implemented. Dev host is macOS, so everything that
touches OverlayFS runs inside the Docker container (`make test-linux`,
`--privileged` for `mount(2)` and `unshare(2)`). The safety spec is
authoritative: fail closed, undo containment, single state directory,
container-only destructive tests.

## Goals / Non-Goals

**Goals:**
- Prove the run → diff → undo/commit loop on OverlayFS, end-to-end, in a container.
- Establish the `SnapshotBackend` trait boundary so an APFS backend can be added without touching the CLI, session, or diff code.
- Ship the flagship-demo integration test and the < 100ms undo benchmark.

**Non-Goals:**
- Multiple/named sessions, pretty diff output, edge-case fidelity (symlinks, hardlinks, xattrs), APFS, anything terminal-related.

## Decisions

### D1: Sandbox mechanism — mount namespace + OverlayFS bind trick

`oops run` must make the wrapped command see the target directory as an
overlay without changing what other processes see. Approach:

1. Parent (oops) prepares state dirs: `~/.local/state/oops/sessions/<id>/{upper,work}`.
2. `unshare(CLONE_NEWNS)` in the child (plus making mounts private via
   `mount --make-rprivate /`), so mounts are invisible outside.
3. Inside the namespace: `mount -t overlay overlay -o lowerdir=<target>,upperdir=<upper>,workdir=<work> <target>` — mounting the overlay *directly over the target path* so the command's cwd and any absolute paths into the target hit the overlay.
4. `chdir` to the mapped cwd, exec `/bin/sh -c "<cmd>"`.
5. When the child exits, the namespace dies and the mount vanishes automatically — but the upper layer persists on disk. This makes `undo` trivial (delete upper/work dirs) and means no long-lived mounts to leak.

Rationale: namespace-scoped mounts give us cleanup-by-construction (no stale
mounts after a crash — satisfies the session staleness spec cheaply) and
containment (other processes never see the overlay). Alternative considered:
a persistent mount managed by oops between run and commit — rejected because
crash-safety then requires mount tracking and the spec's stale-session
handling becomes much hairier.

Consequence: only the wrapped command sees the sandbox. `diff` and `commit`
operate on the upper layer directly (no live mount needed), which is exactly
what the specs describe.

Scope decision that falls out of this: Phase 0 sandboxes **one target
directory** (the cwd where `oops run` is invoked). Writes outside that tree
(e.g. `/tmp`) are not captured — documented honestly, revisited in Phase 1.

### D2: Namespace entry — re-exec self with a hidden subcommand

`unshare` + mount + exec needs to happen in the child process. Rust has no
safe fork story with threads, so: `oops run` spawns its own binary again as
`oops __exec --session <id>` (hidden subcommand) via `Command::new(current_exe)`,
and that child does unshare → mount → exec sh. Alternative considered:
`nix::sched::unshare` after plain `fork` — rejected as unsafe-heavy and
fiddly; re-exec is the boring, debuggable route.

### D3: `commit` = replay the upper layer, not `mount -o remount` games

Walk the upper layer: whiteout (char dev 0:0) → delete lower path; opaque
dir (xattr `trusted.overlay.opaque=y`) → replace lower dir; regular
file/dir → copy over with mode preserved. Alternative considered: rsync-like
external tool — rejected (new runtime dependency, less control over whiteout
semantics).

### D4: Backend trait shape

```rust
trait SnapshotBackend {
    fn is_supported(&self) -> Result<(), UnsupportedReason>;
    fn prepare(&self, target: &Path, session_dir: &Path) -> Result<Sandbox>;
    fn exec(&self, sandbox: &Sandbox, cmd: &str) -> Result<ExitStatus>;
    fn changes(&self, sandbox: &Sandbox) -> Result<Vec<Change>>; // A/M/D
    fn discard(&self, sandbox: &Sandbox) -> Result<()>;
    fn merge(&self, sandbox: &Sandbox) -> Result<()>;
}
```

`changes` lives on the backend (not a generic walker) because whiteout
representation is backend-specific; APFS snapshots will diff differently.

### D5: Dependencies

`clap` (derive), `anyhow`, `serde`+`serde_json`, `nix` (mount, unshare,
mknod detection), `tempfile` (dev-dep). `nix` is the one addition beyond the
pre-approved list — it is the standard safe-ish wrapper for the syscalls we
need; the alternative (raw `libc`) is strictly worse.

## Risks / Trade-offs

- [OverlayFS rejects an upperdir that itself lives on overlay — verified in the test container, whose root is overlay2] → in tests, the oops state dir is a tmpfs mount (`--tmpfs /root/.local/state/oops` in the Makefile). In real use the state dir sits on a normal filesystem, but `is_supported()`/`prepare()` must surface this kernel error clearly (it is one of the fail-closed paths).
- [OverlayFS over the same dir as lowerdir may misbehave on some kernels/filesystems] → the integration tests run on the container's filesystem first; if `lowerdir=<target>` mounted onto `<target>` proves flaky, fall back to mounting at a state-dir mountpoint and bind-mounting it over the target — same UX, slightly more moving parts.
- [Docker container needs `--privileged`] → acceptable for dev/test; documented in the Makefile. CI can use the same flag.
- [Writes outside the target tree escape the sandbox] → Phase 0 documented limitation; the demo and tests only exercise the target tree.
- [`undo` < 100ms] → undo is `rm -rf` of upper+work; O(changes). Benchmark will confirm; if directory removal of a large deleted-tree whiteout set is slow, that is still O(changes), not O(tree).
- [Root required inside container] → container runs as root; fine for Phase 0. Rootless (userns) is a Phase 1+ question.

## Open Questions

- None blocking; D1's "mount over the target path" vs "bind-mount fallback" is resolved empirically by the first integration test.
