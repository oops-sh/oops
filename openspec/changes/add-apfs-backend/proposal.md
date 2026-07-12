# Proposal: add-apfs-backend

## Why

The APFS research spike (archived change `apfs-research-spike`, PR #4) validated the primitive set — `clonefile(2)` snapshot, two-way stat-walk diff, `renamex_np(RENAME_SWAP)` undo — with benchmarks: undo is O(1) (0.4 ms at 50k files), everything is rootless, and restores are byte-identical. This change implements the second `SnapshotBackend`, making oops native on macOS, and enacts the spec revisions the spike identified.

## What Changes

- **`apfs` backend** (macOS only): `run` clones the target tree into oops state, then runs the command **against the real tree** (snapshot-restore model); `diff` is a pruned two-way metadata walk; `undo` is an atomic directory swap (after a dev/ino identity check) followed by the existing rename-then-async-delete pattern; `commit` is O(1) — trash the clone.
- **Backend selection**: macOS + APFS target → `apfs`; Linux → `overlayfs`; anything else fails closed as today.
- **Run-startup cost clause** (new spec requirement): sandbox setup MAY cost O(tree) (the clone: ~100 ms @ 10k, ~600 ms @ 50k files); setup taking longer than 1 s MUST emit progress feedback; each backend's setup cost MUST be documented and benchmarked. Optimization work is explicitly backlogged, not in scope (see Backlog).
- **Per-volume state directories**: swap and clonefile require same-volume state. Design: the primary state root stays `$XDG_STATE_HOME/oops`; a target on a different volume uses `<volume-mount>/.oops/state/` with the identical layout, and every created volume root is registered in `volumes.json` under the primary root. gc sweeps all registered, currently-mounted roots; **every containment check is performed against the specific root set** (a deletion is legal only inside one of the registered roots). Unmounted volumes are skipped, never created.
- **Containment invariant restatement** — safety-spec delta **flagged for verbatim review** (see `specs/safety/spec.md`, requirement "Undo containment", and the quoted block in design.md §Containment). Snapshot-restore cannot satisfy the literal "undo deletes only inside state directories": the swap modifies the protected target. New wording: undo modifies exactly (a) the session's protected target subtree, by atomically restoring it, and (b) registered oops state roots — nothing else, ever.
- **Destructive-test policy delta** (second safety-spec change, also flagged): APFS tests must run on a macOS host — the container-only rule is restated as "destructive tests touch only self-created temp trees; the OverlayFS suite additionally runs only inside the container".
- **Diff spec revision** (porcelain **output bytes unchanged**): "never enumerates the lower layer" becomes an output contract ("descendants of a deleted directory are never listed"), plus a new cost clause: diff MAY cost O(tree) metadata operations and MUST NOT read file contents by default; the mtime+size modification heuristic and its forged-metadata gap are documented.
- **README dual-backend guarantee matrix**: a table making the model difference impossible to miss — interception vs snapshot-restore, real-files-during-run, crash behavior, bystander writes, cloud-sync risk, root requirement, diff cost.
- Session semantics: sessions now survive reboots (real disk, not tmpfs); stale = clone missing. A pending APFS session after a crash is recoverable with `oops undo` — documented.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `sandbox`: apfs backend requirements (clone, swap, identity check), backend selection, run-startup cost clause.
- `safety`: **containment restatement (verbatim-review flagged)**; destructive-test policy for macOS host suites; snapshot-restore fine print added to the guarantee boundary.
- `session`: per-volume state roots + registry, gc across roots, same-volume refusal, reboot-surviving sessions.
- `diff`: porcelain contract rewording (bytes unchanged) + cost clause.

## Impact

- Code: `src/backend/apfs.rs` (new), `src/backend/mod.rs` (selection), `src/state.rs` (volume roots, registry, root-set containment), `src/session.rs` (gc across roots), README.
- No new dependencies: `libc` (already a dep on Linux) becomes a macOS dependency too for `clonefile`/`renamex_np`; everything else is std.
- Tests: new macOS host suite (tempdir-confined destructive tests + `make test-apfs` / `make bench-apfs`); Linux suite unchanged.
- The macOS `fail_closed_host` test changes meaning: macOS now *has* a backend — that test moves to a stubbed "no backend" platform guard or is replaced by APFS-specific fail-closed tests (clone failure → command never runs).

## Backlog (explicitly out of scope, tracked here)

1. **Run-startup optimization**: `fs_snapshot_create` privileged fast path; FSEvents-assisted diff acceleration; clone progress UI polish.
2. **iCloud/Dropbox detection**: warn (or require `--force`) when the target is under `~/Library/Mobile Documents` or contains sync-client markers — mitigates transient-damage propagation.
3. `oops diff --verify` (content comparison closing the forged-mtime gap).
4. Per-volume state root auto-cleanup for volumes that disappear permanently.
5. Rootless Linux (userns) — separately prioritized after this.

## Non-goals

- Everything in the Backlog above; cross-volume targets beyond the refusal path; Windows; any CLI surface change (the four verbs and `diff --porcelain` are untouched).
