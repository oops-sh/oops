# Proposal: rootless-linux-followups

## Why

The `add-rootless-linux` change shipped tier-3 rootless `oops run`. Bare-metal
Group 6 validation on real VMs (Lima, aarch64; Ubuntu 24.04 kernel 6.8,
Debian 12 kernel 6.1, Fedora 44 kernel 6.19) confirmed the guarantee — escape
vectors and adversarial redirects all held across kernels 6.1–6.19 on ext4
and btrfs — but surfaced two release blockers that Docker-on-Mac could not
see:

1. **Rootless trash is not reclaimable.** A rootless overlay mount leaves a
   `work/work` directory owned by the mapped uid with mode `000`. The plain
   unprivileged user cannot delete it (cannot enter a mode-000 dir it owns
   without first chmod-ing), so `gc` fails to reclaim `trash/`, the state root
   grows unboundedly, and the user cannot even `rm -rf` it by hand.
   Reproduced on all three distros. (Podman hits the same class of problem
   and solves it by deleting inside a matching userns.)

2. **The fail-closed message is non-actionable, and the acceptance script
   hid it.** Ubuntu 24.04's default AppArmor policy does not make `unshare`
   fail; `unshare` succeeds but the new userns is stripped of the capability
   to write its own id map, so setup fails later at
   `write /proc/self/uid_map` with EPERM — a code path whose message named
   neither the sysctl nor `OOPS_PRIVILEGED`. The acceptance script's
   assertion used an OR match that the inadequate message still satisfied — a
   test that greens on wrong behavior, worse than none.

## What Changes

- **Trash reclamation runs in an identity-mapped user namespace** (session
  spec): the `__gc` process enters a userns mapping the invoking uid to
  userns root, gaining `CAP_DAC_OVERRIDE` to reclaim the mode-000 leftovers.
  Best-effort and fail-safe. A container regression test plus a non-root
  bare-metal assertion prove reclamation.
- **Rootless setup diagnostics** (sandbox spec): both the `unshare` and the
  id-map-write failure sites route through one message that names the full
  sysctl `kernel.apparmor_restrict_unprivileged_userns` (with persistence)
  and the explicit `OOPS_PRIVILEGED=1` fallback and its honest tier-1/2
  downgrade.
- **Acceptance script hardening**: the fail-closed assertion now strictly
  requires both the full sysctl name and `OOPS_PRIVILEGED` (self-verified:
  the old message FAILs, the new one PASSes); a new check asserts the state
  root is fully reclaimed and removable by the plain user.
- **README guarantee matrix** records the confirmed baseline honestly:
  rootless verified on kernels 6.1–6.19 / ext4 + btrfs; Debian 12 and Fedora
  44 out of the box; Ubuntu 24.04 needs the one sysctl (or the tier-1/2
  fallback).
- **Version bump to 0.3.0** (APFS + rootless Linux are a major step).

## Impact

- Specs: `session` (ADDED: rootless trash reclamation), `sandbox` (ADDED:
  rootless setup diagnostics).
- Code: `src/session.rs` (`enter_gc_userns`), `src/main.rs` (`__gc` enters it),
  `src/backend/overlayfs.rs` (`rootless_userns_error` on both failure sites).
- Tests: `tests/linux.rs` (`rootless_trash_is_reclaimed_by_gc`),
  `spikes/confinement/bare_metal_acceptance.sh` (strict §7 + reclaim check).
- Docs: README matrix. `Cargo.toml` 0.2.0 → 0.3.0.
- APFS backend untouched.

## Non-goals

- The session token (still deferred to `oops shell`), tier-2 outside-tree
  writes, macOS Linux-guest — all remain in the confinement backlog.
