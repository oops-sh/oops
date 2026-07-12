# Tasks: add-core-sandbox-loop

## 1. Skeleton and safety scaffolding

- [ ] 1.1 Add dependencies (clap, anyhow, serde, serde_json, nix; tempfile as dev-dep) and the module layout (`cli`, `backend`, `backend::overlayfs`, `session`, `diff`)
- [ ] 1.2 Define `SnapshotBackend` trait, `Change` enum (Added/Modified/Deleted), and backend selection that fails closed on unsupported platforms (macOS build compiles, every verb errors cleanly)
- [ ] 1.3 State-directory module: resolve `$XDG_STATE_HOME/oops` (default `~/.local/state/oops`), session dir layout, and a path-containment check used by undo (refuse paths outside the state dir)
- [ ] 1.4 Container-only guard for destructive tests (env marker set by the Docker image; tests skip without it) and verify `make test-linux` runs a trivial test in the container

## 2. run — sandboxed execution

- [ ] 2.1 Session records: create/load/delete JSON record per target directory; refuse a second `run` while one is pending
- [ ] 2.2 Hidden `__exec` subcommand: unshare mount namespace, make mounts private, mount overlay over the target, chdir, exec `sh -c`
- [ ] 2.3 `oops run`: prepare session dirs, spawn `__exec`, propagate stdout/stderr and exit status, persist session record (including command exit status); on any sandbox failure, abort before executing the command
- [ ] 2.4 Integration test (container): created file lands in upper layer only; lower tree byte-identical after run

## 3. diff / undo / commit

- [ ] 3.1 `changes()` for overlayfs: walk upper layer, detect whiteouts (char 0:0) and opaque dirs, classify A/M/D; `oops diff` prints sorted `A/M/D path` lines; read-only (verify repeat runs identical)
- [ ] 3.2 `oops undo`: containment check, delete upper/work/session record, handle stale sessions (mount gone → still discard); `oops commit` refuses stale sessions
- [ ] 3.3 `oops commit`: replay upper layer onto the real tree (creates, modifies, whiteout deletions, opaque dirs, preserve modes), then clean up session
- [ ] 3.4 Integration tests (container): the mixed A/M/D diff scenario; commit-applies-everything scenario; no-pending-session error paths for all three verbs

## 4. Flagship demo, benchmark, docs

- [ ] 4.1 Flagship integration test: generate a tree → `oops run "rm -rf testdir"` → verify tree intact → `oops undo` → verify byte-identical (hash the tree before/after)
- [ ] 4.2 Undo benchmark: ~10k-file generated tree, `run "rm -rf <subtree>"`, assert undo < 100ms in the container; wire as `make bench-linux`
- [ ] 4.3 Minimal README: what oops is, the demo, Phase 0 limitations (Linux-only, single directory, filesystem-only), `make test-linux` instructions
