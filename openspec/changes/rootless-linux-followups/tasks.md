# Tasks: rootless-linux-followups

## 1. Trash reclamation (blocker 1)

- [x] 1.1 `session::enter_gc_userns` (Linux): unshare `CLONE_NEWUSER` + write
  the identity id map; best-effort.
- [x] 1.2 `__gc` enters it before sweeping, so mode-000 `work/work` is
  reclaimable via `CAP_DAC_OVERRIDE`.
- [x] 1.3 Container regression test `rootless_trash_is_reclaimed_by_gc`
  (root can already delete mode 000, so it guards against gc regressions);
  real non-root reclamation asserted by the bare-metal script.
- [x] 1.4 Reproduced the root cause as a non-root user (mode-000 `work/work`,
  owner = invoking uid, `rm` → Permission denied) and verified `__gc`
  reclaims it.
- [x] 1.5 **Fd-anchored gc deletion** (review): trash reclamation walks each
  `trash/` from its registered root with `O_NOFOLLOW`, deletes via `unlinkat`,
  and never traverses a symlink — same standard as commit replay. Containment
  anchor established BEFORE elevation; the userns only punches through
  mode-000 dirs, every op relative to the anchored fds.
- [x] 1.6 Adversarial test `gc_does_not_follow_trash_symlink_out_of_tree`
  (container) + a matching check in the acceptance script (`6c`): plant an
  out-of-tree symlink in the upper → undo → gc → sentinel byte-identical.
- [x] 1.7 `spikes/confinement/vm_matrix.sh`: Lima orchestrator (build 3 VMs →
  run acceptance → collect logs → dry-run stop, never publishes).

## 2. Fail-closed diagnostics (blocker 2)

- [x] 2.1 `rootless_userns_error` shared message; both `unshare` and the
  id-map write route through it (the Ubuntu EPERM path now names the sysctl
  and `OOPS_PRIVILEGED`).
- [x] 2.2 Acceptance script §7 assertion made strict (full sysctl AND
  `OOPS_PRIVILEGED`), self-verified: old message FAILs, new PASSes.
- [x] 2.3 Acceptance script adds the state-root-reclaimed + user-removable
  checks.

## 3. Matrix + release

- [x] 3.1 README guarantee matrix: confirmed kernels 6.1–6.19, ext4/btrfs;
  Debian 12 & Fedora 44 out of the box; Ubuntu 24.04 sysctl.
- [x] 3.2 `Cargo.toml` 0.2.0 → 0.3.0.

## 4. Wrap-up

- [ ] 4.1 User re-runs the bare-metal script on the three VMs (all checks,
  incl. reclaim + strict fail-closed) and confirms green.
- [ ] 4.2 Tag `v0.3.0`, publish crates.io, GitHub release.
