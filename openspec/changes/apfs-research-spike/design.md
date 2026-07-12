# Design: apfs-research-spike

## Context

Candidate primitive set for the APFS `SnapshotBackend`:

- **snapshot**: `clonefile(2)` — copy-on-write clone of a whole directory
  tree in one unprivileged syscall; shares data extents, new inodes,
  preserves modes/mtimes/xattrs.
- **undo**: `renamex_np(2)` with `RENAME_SWAP` — atomic exchange of two
  directory entries on the same volume.
- **diff**: two-way metadata walk between the clone and the live tree.

All three were benchmarked on this machine (M-series, APFS, macOS 26) with
`spikes/apfs/bench.c`:

| tree size | clonefile (tree) | diff (2-way walk) | RENAME_SWAP |
| --- | --- | --- | --- |
| 1k files | 8 ms | 14 ms | 0.15 ms |
| 10k files | 98–156 ms | 137–160 ms | 0.2–0.3 ms |
| 50k files | 594 ms | 787 ms | 0.4 ms |

Clone and diff scale ~linearly with tree size (O(tree)); swap is O(1) and
was verified to restore a byte-identical tree (deleted subtree back,
additions gone, modifications reverted). **No root or entitlement is
required for any of the three** — the APFS backend is inherently rootless.

## Q1 — Security model: snapshot-restore vs interception

**OverlayFS (interception):** the command's writes land in a layer; the
real tree is never touched during the run. "Undo" discards writes that
never happened. A crash mid-run leaves the real tree pristine by
construction.

**clonefile (snapshot-restore):** the clone is taken first, then the
command runs **against the real tree**. The damage really happens; undo
swaps the pristine clone back. The safety guarantee weakens from "your
files were never touched" to "your files can always be put back". That
difference forces new fine print, which the implementation change MUST put
in the README and safety spec:

1. **Exposure window.** Between `run` and `undo`/`commit`, the real tree
   holds the damaged state. A crash or power loss inside the window does
   not lose data — the clone persists under oops state and can be restored
   on the next invocation — but restoration is not automatic-by-construction
   the way OverlayFS is.
2. **Observers see the damage.** FSEvents watchers, Spotlight, Time
   Machine, and cloud sync clients (iCloud Drive, Dropbox) can observe and
   even **propagate** the transient state — a synced folder may replicate a
   deletion to other devices before undo. The fine print must warn against
   pointing the APFS backend at cloud-synced directories, or at minimum
   document the propagation risk. Editors/IDEs with the tree open will see
   mutated files mid-window.
3. **Torn snapshots.** `clonefile` on a directory is not atomic with
   respect to concurrent writers; cloning a tree while another process
   writes it can capture a torn state. Acceptable under a single-actor
   assumption, must be documented.
4. **Fail-closed is preserved.** If the clone fails (cross-volume EXDEV,
   ENOSPC, unsupported fs), oops refuses to run the command — the invariant
   carries over unchanged.
5. **Forensic remanence.** Written data hits real disk blocks; undo
   restores the namespace, it does not scrub freed extents. (Same class of
   caveat as any snapshot system; worth one line in fine print.)

Neither model covers network side effects or out-of-tree writes — the
existing guarantee boundary is unchanged.

## Q2 — Diff strategy and the spec revision

**Chosen strategy: two-way metadata walk with pruning** (benchmarked):

- Walk the clone; a path missing in live is **deleted** — emit one `D path/`
  entry and prune (never descend), which preserves the porcelain
  single-entry contract for directory deletions.
- A file present in both with differing `(size, mtimespec)` is
  **modified** (nanosecond mtime on APFS; clonefile preserves mtimes, and
  any write updates them).
- Walk live; a path missing in the clone is **added** — emit and prune.

Cost is O(tree) `lstat` calls (2× tree size worst case): ~140 ms at 10k
files, ~790 ms at 50k — acceptable for an interactive `oops diff`.

**Alternatives rejected for v1:** FSEvents recording during the run
(approximate: events coalesce and can drop; usable later as an accelerator
hint that narrows the walk, never as the source of truth);
`fs_snapshot_create(2)` (requires root or a restricted entitlement — kills
the rootless win); content hashing (O(bytes), way too slow as a default).

**Known heuristic gap:** a command that rewrites a file and then forges
back its exact size and mtime escapes `M` detection (rsync's quick-check
class of caveat). Document it; a future `oops diff --verify` can do content
comparison on demand.

**Spec revisions the implementation change must make:**

- `diff` porcelain contract: the phrase "oops never enumerates the lower
  layer" is an OverlayFS implementation claim, not an output contract.
  Rewrite to: "a deleted directory is exactly one `D path/` line and
  descendants are never **listed**"; move enumeration/cost language into a
  new requirement: diff MAY cost O(tree) metadata operations and MUST NOT
  read file contents by default. **Porcelain output bytes are unchanged**,
  so the frozen format survives intact.
- `sandbox` "Undo performance" (O(changes), < 100 ms): still satisfied —
  swap-based undo is O(1). No revision needed for undo; add that the
  < 100 ms benchmark must also run for the APFS backend (host-side).
- New backend-conformance note: `run` startup pays the clone cost
  (~100 ms @ 10k, ~600 ms @ 50k files). Set expectations in the sandbox
  spec: sandbox setup MAY be O(tree) but MUST be reported to the user if it
  exceeds ~1s (progress line), rather than silently hanging.

## Q3 — Undo atomicity and the explicit protection scope

**Mechanism (validated):** `renamex_np(clone, target, RENAME_SWAP)`
atomically exchanges the two directory entries — 0.2–0.4 ms independent of
tree size, works on directories, byte-identical restore verified. There is
no intermediate state in which the target is missing or partially restored.
After the swap, the damaged tree (now sitting at the clone's path inside
oops state) is renamed into trash and deleted asynchronously — the exact
rename-then-async-delete pattern the session spec already mandates.

**Constraints validated/derived:**

- Both paths must exist and be on the **same volume**. Therefore clones
  must live in a per-volume state root, not unconditionally under
  `~/.local/state/oops`. v1: refuse targets on a different volume than the
  state root (fail closed), with a clear message; per-volume state roots
  are a follow-up.
- **Identity check before swap:** the session record must store the
  target's `(st_dev, st_ino)` at clone time; undo must verify they still
  match the path before swapping, so a target directory that was replaced
  (path reuse) is never swapped with an unrelated tree.

**Explicit protection scope (to be added to the safety spec verbatim,
adapted):**

1. The protected scope of a session is exactly the target directory
   subtree, identified by canonical path + device + inode at `run` time.
2. Undo restores that subtree to its snapshot state **in its entirety**:
   changes made inside it by *any* process during the window — not just the
   wrapped command — are reverted (collateral undo). This is a documented
   difference from interception, where bystander writes to the real tree
   survive an undo.
3. Undo containment must be restated: undo MUST modify only (a) the
   protected target subtree, by atomically restoring it, and (b) oops state
   directories. Everything else on the system remains untouchable. (The
   current invariant says "only state directories", which snapshot-restore
   cannot satisfy literally.)
4. Processes holding open file descriptors into the pre-swap tree keep
   their handles (they now reference the trashed tree until closed);
   harmless to the restored tree, documented.

## Risks / Trade-offs

- [Clone latency at run start grows with tree size] → linear, ~600 ms at
  50k files; acceptable v1, progress reporting above 1s; `fs_snapshot` is a
  possible future privileged fast path.
- [Cloud-sync propagation of transient damage] → strongest fine-print item;
  consider a v1 warning when the target is inside `~/Library/Mobile
  Documents` (iCloud) or contains `.dropbox`.
- [mtime+size heuristic misses forged modifications] → documented;
  `--verify` escape hatch later.

## Recommendation

Proceed to an `add-apfs-backend` implementation proposal built on
clonefile + two-way walk + RENAME_SWAP. The primitives are fast (undo is
400× under the 100 ms budget at 50k files), rootless, and atomic where it
matters. The bulk of the implementation work is not the syscalls — it is
the spec revisions (Q2, Q3) and the honest fine print (Q1).
