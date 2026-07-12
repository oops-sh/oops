# Proposal: apfs-research-spike

## Why

The APFS backend is now prioritized ahead of rootless Linux and launch work. Before an implementation proposal can be written, three design questions must be answered with data, because the natural macOS primitive — `clonefile(2)` snapshots — implies a **different sandbox model** (snapshot-restore) than OverlayFS (interception), with different safety fine print, a different diff cost model, and a different undo mechanism.

## What Changes

This is a research spike: no product code. Deliverables:

- `design.md` answering, with measured data:
  1. the security-model difference between snapshot-restore and interception, and the honest fine print it forces;
  2. the tree-comparison diff strategy and the concrete revision needed to the O(changes)/"never enumerates" spec clauses;
  3. undo atomicity via `renamex_np(RENAME_SWAP)` and an explicit statement of the protection scope (including the undo-containment invariant revision).
- `spikes/apfs/bench.c` + README: reproducible benchmarks of clone / tree-diff / atomic swap at 1k, 10k, and 50k files, run on a real APFS volume (this dev machine).

## Capabilities

### New Capabilities

_None — spike only. Spec revisions are proposed in design.md but enacted by the future implementation change._

### Modified Capabilities

_None._

## Impact

- New files under `spikes/apfs/` and this change directory. No dependencies, no runtime changes.
- Output feeds directly into the next change: `add-apfs-backend` (implementation proposal, written after this spike is reviewed).

## Non-goals

- Any implementation of the backend, trait changes, or spec edits.
- FSEvents-based diff acceleration, `fs_snapshot_*` APIs (evaluated and rejected in design.md), cross-volume support.
