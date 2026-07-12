# Tasks: apfs-research-spike

## 1. Benchmarks

- [x] 1.1 `spikes/apfs/bench.c`: build a repo-sized tree, benchmark whole-tree `clonefile(2)`, two-way stat-walk diff (with pruning, verifying A/M/D counts), and `renamex_np(RENAME_SWAP)` undo with byte-identical restore verification
- [x] 1.2 Run at 1k / 10k / 50k files on the macOS host (APFS), record results in `spikes/apfs/README.md`

## 2. Design answers

- [x] 2.1 Q1: security model comparison (snapshot-restore vs interception) with the forced fine print (exposure window, sync-client propagation, torn snapshots, remanence)
- [x] 2.2 Q2: diff strategy (two-way walk, pruning, mtime+size heuristic and its gap) and the concrete diff/sandbox spec revisions — porcelain output bytes unchanged
- [x] 2.3 Q3: undo atomicity via RENAME_SWAP (same-volume + dev/ino identity check) and the explicit protection-scope statement, including the undo-containment invariant restatement

## 3. Wrap-up

- [ ] 3.1 Spike reviewed by the user; findings feed the `add-apfs-backend` implementation proposal
