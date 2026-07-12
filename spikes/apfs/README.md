# APFS backend spike — clonefile / tree-diff / RENAME_SWAP benchmarks

Measures the three primitives an APFS `SnapshotBackend` would be built on.
Runs directly on a macOS host (APFS volume required, no root needed):

```console
$ cc -O2 -o bench bench.c
$ ./bench /tmp/apfs-bench            # 100 dirs x 100 files (10k)
$ ./bench /tmp/apfs-bench 500 100    # 50k files
```

Results on the dev machine (M-series, macOS 26, 2026-07-12):

| tree size | clonefile (whole tree) | diff (2-way stat walk) | renamex_np RENAME_SWAP |
| --- | --- | --- | --- |
| 1k files | 8 ms | 14 ms | 0.15 ms |
| 10k files | 98–156 ms | 137–160 ms | 0.2–0.3 ms |
| 50k files | 594 ms | 787 ms | 0.4 ms |

The bench also verifies correctness after the swap: a deleted subtree is
restored, added files are gone, modified files carry their original
content. Conclusions and the resulting design answers live in
`openspec/changes/apfs-research-spike/design.md`.
