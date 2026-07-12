# Proposal: add-core-sandbox-loop

## Why

oops does not exist yet as a working program. Its entire value proposition — any command can be undone — rests on one primitive: running a command inside a copy-on-write filesystem layer that can be discarded (`undo`), inspected (`diff`), or merged down (`commit`). Until that loop is proven end-to-end, nothing else (sessions, pretty output, terminal integration) matters. This change is Phase 0: the smallest implementation that demonstrates the flagship demo inside a Linux container.

## What Changes

- New Rust binary `oops` with exactly four subcommands: `run <cmd>`, `diff`, `undo`, `commit`.
- A `SnapshotBackend` trait abstracting the copy-on-write layer, with one implementation: **OverlayFS** (Linux only). An `apfs` backend is planned but explicitly out of scope.
- `oops run "<cmd>"` mounts an OverlayFS over the working tree, executes the command inside a mount namespace so all filesystem writes land in the upper layer, and records the pending sandbox under `~/.local/state/oops/`.
- `oops diff` inspects the upper layer and lists created / modified / deleted paths (deletions detected via OverlayFS whiteout files).
- `oops undo` unmounts and discards the upper layer; the lower (real) filesystem is untouched.
- `oops commit` merges the upper layer into the real filesystem, then cleans up.
- Safety invariants written as the first spec: fail-closed sandbox setup, undo never deletes outside oops state, all state under one well-known directory.
- Integration test proving the flagship demo (create files → `oops run "rm -rf testdir"` → `oops undo` → files byte-identical), running only inside the Docker container via `make test-linux`.
- Benchmark showing `undo` completes in < 100ms on a repo-sized tree.

## Capabilities

### New Capabilities

- `safety`: Non-negotiable invariants — fail-closed sandboxing, undo containment, state directory containment, container-only destructive tests. Written first; every other capability defers to it.
- `sandbox`: The `SnapshotBackend` trait and the OverlayFS implementation — mount lifecycle, mount-namespace command execution, upper-layer/whiteout semantics.
- `cli`: The four-verb command surface (`run` / `diff` / `undo` / `commit`), exit codes, and error reporting.
- `session`: Minimal state tracking for a single pending sandbox between `run` and `undo`/`commit` (what is pending, where its layers live, staleness detection). Multiple/named sessions are Phase 1.
- `diff`: World-diff computation from the upper layer — the created/modified/deleted classification. Plain-text output only; readable/colored rendering is Phase 1.

### Modified Capabilities

_None — this is the first change; no specs exist yet._

## Impact

- New code: entire `src/` tree (CLI parsing, backend trait, overlayfs impl, session state, diff walker).
- Dependencies to add: `clap` (CLI), `anyhow` (errors), `serde`/`serde_json` (session state file), `nix` (mount/unshare syscalls), `tempfile` (dev-dependency for tests). `nix` is the only one beyond the pre-approved list — flagged here for review.
- Runs only on Linux; on macOS every subcommand must fail with a clear "use the container" message rather than doing anything (safety invariant: fail closed).
- Test/dev workflow: `make test-linux` (Docker, `--privileged` for `mount(2)`), already scaffolded.
- No network, no daemon, no PTY, no TUI.

## Non-goals

- Session management beyond one pending sandbox (named/multiple sessions → Phase 1).
- Readable/colored/grouped diff output (→ Phase 1).
- Edge cases: daemons spawned by commands, network side effects, symlink/hardlink fidelity guarantees, permission-bit preservation audits (→ Phase 1; documented honestly as filesystem-only).
- APFS backend (→ Phase 1 research spike, proposal only).
- Anything terminal/PTY related (→ Phase 2).
