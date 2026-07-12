# Tasks: add-apfs-backend

## 1. State roots

- [ ] 1.1 Multi-root state: `StateRoots` (primary + `volumes.json` registry, atomic writes), per-volume root resolution via statfs mount point, `ensure_in_state_roots` containment over the set; host-runnable unit tests (containment, registry, foreign-root exclusion, unmounted skip)
- [ ] 1.2 gc across mounted registered roots (skip absent volumes); session records gain `backend` + target identity fields with backward-compatible loading

## 2. APFS backend

- [ ] 2.1 `backend/apfs.rs`: clone-based `exec` (fail-closed sequencing, >1s progress line), backend selection on macOS, Cargo.toml libc on macOS
- [ ] 2.2 `changes`: pruned two-way stat walk feeding the existing renderers (porcelain bytes unchanged)
- [ ] 2.3 `undo`: dev/ino identity check + `RENAME_SWAP` + trash + background gc; `commit`: trash the snapshot (O(1)); stale-snapshot refusal for both
- [ ] 2.4 Remove `tests/fail_closed_host.rs`; replace with APFS clone-failure fail-closed coverage

## 3. Tests and benchmarks (macOS host, tempdir-confined)

- [ ] 3.1 `tests/apfs.rs` mirroring the Linux scenarios: flagship demo byte-identical restore, mixed A/M/D diff, exit propagation, second-run refusal, no-pending errors, reboot-equivalent persistence (new process, same state), identity-mismatch refusal
- [ ] 3.2 `make test-apfs` + `make bench-apfs`: undo < 100ms at 10k files, setup (clone) cost reported

## 4. Docs

- [ ] 4.1 README: dual-backend guarantee matrix (design D5), macOS install/run section update, snapshot-restore fine print incl. cloud-sync warning and mtime-heuristic gap; `run --help` scope note gains the macOS model sentence
