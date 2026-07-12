# Design: add-apfs-backend

## Context

Research and benchmarks: `openspec/changes/archive`-bound
`apfs-research-spike` (design.md there) and `spikes/apfs/`. This document
covers implementation shape only; the model rationale lives in the spike.

## Goals / Non-Goals

**Goals:** native, rootless oops on macOS; spec revisions enacted exactly
as flagged in the proposal; README that makes the two guarantee models
impossible to confuse.

**Non-Goals:** everything in the proposal's Backlog (fs_snapshot,
FSEvents, iCloud detection, `--verify`, cross-volume support).

## Decisions

### D1: `src/backend/apfs.rs` shape

`libc::clonefile` / `libc::renamex_np` via the existing `libc` dependency
(extended to macOS in Cargo.toml); no new crates. The backend implements
the same `SnapshotBackend` trait:

- `exec`: create session dirs → `clonefile(target, session/snapshot)` →
  (progress line if >1s, using a pre-clone `st_nlink`-free size estimate is
  unreliable, so simply print after a 1s timer if the clone hasn't
  returned) → record identity (dev/ino) in the session → spawn `sh -c`
  directly (no namespace, no re-exec, no marker: if the clone fails we
  never reach spawn — fail-closed is trivially sequential here).
- `changes`: two-way pruned stat walk (port of the spike's C walk to Rust),
  reusing `sort_changes` and the existing renderers untouched.
- `discard` (undo): verify dev/ino → `renamex_np(snapshot, target,
  RENAME_SWAP)` → rename displaced tree into the root's trash → background
  gc. On identity mismatch: refuse.
- `merge` (commit): move snapshot to trash; nothing else.

### D2: State roots and containment (multi-root)

`state.rs` grows a `StateRoots` value: primary root + entries loaded from
`volumes.json` (created lazily, atomic write via temp+rename). Root
resolution for a target: same `st_dev` as primary root → primary;
otherwise walk up from the target to its volume mount point (statfs
`f_mntonname`), root is `<mount>/.oops/state`. `ensure_in_state_dir`
becomes `ensure_in_state_roots(roots, path)` — same canonicalize + prefix
check, over the set. gc iterates roots, skipping entries whose directory
does not currently exist (unmounted volume: mount point absent or dev
mismatch — check both, never mkdir under a mount point).

Registry consistency: a per-volume root is registered **before** first
use; a root present on disk but missing from the registry is treated as
foreign (never swept) — deleting the registry entry plus the volume's
`.oops/` resets that volume.

### D3: Session record versioning

Records gain `backend`, `snapshot` (APFS) and `target_dev`/`target_ino`
fields. Old Phase-0 records lack them: loader treats a record without
`backend` as `overlayfs` (only thing that could have written it). No
migration step.

### D4: Test layout

New `tests/apfs.rs`, `#![cfg(target_os = "macos")]`, destructive but
tempdir-confined per the revised safety spec: every test sets
`XDG_STATE_HOME` to its own tempdir (test-scoped state root) and targets
its own tempdir tree. Mirrors the Linux suite scenario-for-scenario
(flagship demo, mixed diff, exit propagation, second-run refusal, stale
snapshot, identity mismatch, no-pending errors) plus volume-root logic via
unit tests (multi-root containment, registry, unmounted skip — simulated
with plain dirs). `make test-apfs` / `make bench-apfs` run them on the
host; `tests/fail_closed_host.rs` is deleted (macOS now has a backend —
its scenario is superseded by clone-failure fail-closed tests, e.g. a
non-APFS `exfat`-style tmp image is out of scope, so the test targets a
directory whose volume cannot host a state root instead).

### D5: README guarantee matrix (lands verbatim, wording reviewable in PR)

| | Linux · OverlayFS | macOS · APFS |
| --- | --- | --- |
| Model | interception | snapshot-restore |
| Real files during `run` | never touched | modified — restorable |
| Guarantee | "never happened" | "can always be put back" |
| `undo` | discard layer, O(1) | atomic swap, O(1) |
| `commit` | replay layer, O(changes) | keep tree, O(1) |
| Crash mid-window | tree already pristine | tree modified; `oops undo` after restart restores |
| Other processes' writes during run | survive undo | reverted by undo (collateral) |
| Cloud-synced folders | safe | transient damage may propagate — avoid |
| Root required | yes (Phase 0) | no |
| `diff` cost | O(changes) | O(tree) metadata |

## Risks / Trade-offs

- [statfs mount-point discovery differs across macOS versions] → use
  `libc::statfs` `f_mntonname` (stable ABI); integration test asserts the
  home-volume happy path only.
- [Swap displaces a tree other processes hold fds into] → documented fine
  print (spike Q3); no mitigation in v1.
- [Registry corruption] → registry is advisory for gc/containment
  expansion only; a corrupt registry degrades to primary-root-only
  behavior, never to broader deletion rights (fail closed direction).

## Open Questions

None — flagged decisions (naming, matrix wording, containment text) are in
the proposal/spec deltas for review.
