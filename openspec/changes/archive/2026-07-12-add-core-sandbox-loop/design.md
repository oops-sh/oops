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

### D5: Commit failure semantics — non-atomic replay, fail-stop, idempotent retry

Commit walks the upper layer in a deterministic order (parents before
children; within a directory: whiteouts/opaque first, then copies). On the
first error it stops, prints applied/remaining counts and the failing path,
exits non-zero, and leaves the session record + upper layer untouched.
Because replay steps are idempotent (deleting an already-deleted path and
re-copying the same content are both no-ops), `commit` can simply be re-run.
The overlay is mounted with `redirect_dir=off,metacopy=off` (plus `xino=off`)
so renames degrade to copy-up + whiteout and the upper layer never contains
`trusted.overlay.redirect` or metacopy stubs; commit verifies this and
aborts before touching the real tree if it sees an overlay xattr it does not
recognize. Alternative considered: staging the merge in a temp dir and
atomically swapping — rejected for Phase 0 (directory-tree swaps are not
atomic on Linux anyway; honesty + idempotence is simpler and safer).

### D6: Undo — rename-then-async-delete

`undo` must be O(1), not O(changes): its critical section is one
`rename(sessions/<id>, trash/<id>.<nonce>)` within the state directory
(same filesystem, atomic, fast). It then spawns a detached
`oops __gc` child to delete trash contents in the background and returns
immediately. If the background deletion dies, the entry just stays in
`trash/` until the next gc sweep — never a correctness problem, only disk
space. This makes the < 100ms benchmark independent of change-set size.

### D7: Orphaned-state gc — opportunistic sweep on `run`

At the start of every `oops run` (before creating its own session), oops
sweeps the state directory: delete everything under `trash/`, and move any
session directory that has no parseable `session.json` into `trash/`.
Validly pending sessions are never touched. The hidden `__gc` subcommand
performs the same sweep (used both by undo's background deletion and
manually). All gc deletions go through the same containment check as undo:
canonicalized paths must be inside the state directory. Alternative
considered: a persistent daemon or cron — rejected; opportunistic sweep is
zero-infrastructure and the state dir is small.

### D8: Dependencies

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
