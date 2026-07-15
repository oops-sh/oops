# Proposal: add-rootless-linux

## Why

The `oops-shell` and `confinement` spikes established the launch-blocking
gap. Today's Linux backend requires **root** (`unshare(CLONE_NEWNS)` with no
user namespace), and the wrapped command runs holding `CAP_SYS_ADMIN` in that
namespace — so a prompt-injected agent can `umount -l` the overlay and escape
onto the real files. The commit authority is only structural against a
*cooperative* agent. The confinement spike proved, with an executed probe on
kernel 6.10, that an unprivileged **user namespace** design closes this: the
command placed in a nested child userns cannot unmount the sandbox
(`umount`/`umount -l` → EINVAL), cannot `nsenter` into the parent mount ns
(→ EPERM), and its writes stay in the upper layer with the real lower tree
byte-identical. This proposal rebuilds the Linux backend on that design so
`oops run` is **rootless and tier-3** (prompt-injected agent) as fixed in the
confinement spike's threat table.

## What Changes

- **Rootless namespace architecture.** `oops run` on Linux creates an
  unprivileged user namespace + mount namespace (launcher, "userns A"),
  mounts the overlay over the target, then executes the wrapped command in a
  **nested child user namespace ("userns B")** that holds no mount capability
  over A's mounts. No root, no `--privileged` container required on a
  supporting kernel.
- **The `redirect_dir` migration (the core of this change).** Unprivileged
  overlay **rejects `redirect_dir=off`**, the option the backend relies on so
  the commit replay can refuse all unfamiliar metadata. Dropping it means the
  kernel may write `user.overlay.*` metadata (notably `redirect` on directory
  renames) into the upper layer, and the commit path — the only code that
  touches real files — must now **understand and correctly replay the
  enumerated set**, while still aborting fail-closed on anything outside it.
  The containment invariant changes from *"refuse all redirect metadata"* to
  *"replay the enumerated set, refuse everything else"* and is flagged for
  word-by-word review.
- **Fail-closed fallback.** When unprivileged userns or unprivileged
  overlayfs is unavailable (old kernel, Debian/Ubuntu AppArmor/sysctl
  restrictions), `run` refuses with an actionable message naming the knob,
  and offers an explicit privileged opt-in — never silently degrades to
  unsandboxed and never silently requires root.
- **Escape-vector regression suite.** The spike's verdict table becomes
  permanent container CI: umount blocked from userns B, nsenter blocked,
  writes land in upper, post-unmount lower byte-identical. These are the
  tier-3 acceptance tests — "done" is a green run of them.
- **Spec deltas** to `sandbox` (rootless architecture, nested-userns
  authority boundary, the overlay-encoding set, privileged-fallback policy)
  and `safety` (the redirect-metadata invariant rewrite + the Linux tier-3
  statement).

Out of scope, explicitly (backlog): tier-4 adversaries (ptrace of the
launcher, fd-passing, kernel/hypervisor exploits); tier-2 *outside-the-tree*
writes (a controlled rootfs where only the target is writable — the spike's
stage 4); `oops shell` (the interactive per-session surface — the namespace
design here must not preclude it, but it ships separately); macOS agent
confinement via Linux guest (spike stage 5).

## Scope note — stage numbering vs. the confinement spike

The confinement spike numbered the work stages 1 (session token), 2 (rootless
backend), 3 (nested-userns agent). This proposal delivers a **tier-3 rootless
`oops run`**, which necessarily folds the spike's **stage 3 into stage 2**:
for `oops run` there is no separate interactive shell, so the wrapped command
*is* the payload that must run in userns B — a rootless backend that ran the
command in userns A would be tier-1/2 only (agent still holds mount caps).
The spike's stage 1 (launcher-held session token) is **backend-independent
defense-in-depth** and is included here as a small component because it
closes the "any process in the target dir finalizes" gap the shell spike
found; on Linux the namespace is what delivers the guarantee, the token
hardens it. If a separate `add-session-token` change is preferred, that
component lifts out cleanly — flag it in review.

## Impact

- Affected specs: `sandbox` (MODIFIED requirements), `safety` (MODIFIED
  requirements).
- Affected code: `src/backend/overlayfs.rs` (mount options, namespace setup,
  `changes`/`validate_upper`/`replay` xattr handling), `src/main.rs`
  (`__exec` child re-work; possible new `__exec2` nested stage; fallback
  messaging), `src/session.rs` (optional session token field). The
  `SnapshotBackend` trait and the **APFS backend are untouched**.
- Affected tests: `tests/linux.rs` — the existing
  `commit_aborts_on_unrecognized_overlay_xattr_and_retry_completes` and the
  sandbox-scope rename scenario must be **migrated** (they currently assert
  rejection of `redirect`, which becomes a handled case), plus the new
  escape-vector suite.
- User-visible: Linux no longer needs root on supporting kernels; the README
  guarantee matrix updates to "no root required (kernel ≥ 5.11; see
  fallback)" and states tier 4 is out of scope.

## Verification I cannot do on Docker-for-Mac (need a real box)

The spike probe ran on Docker Desktop's LinuxKit 6.10 VM with the added
seccomp/AppArmor policy lifted. Bare-metal distro behavior — especially
Ubuntu's `apparmor_restrict_unprivileged_userns` — was reasoned, not booted.
This proposal's compatibility-matrix task **requires you (or CI) to run the
acceptance suite on real hosts**: stock Ubuntu 24.04 (AppArmor restriction on
*and* off), Debian 12, and Fedora. See tasks §5.

## Non-goals

- No `oops shell`, no tier-2 outside-tree rootfs, no macOS guest, no tier-4
  hardening. Each is named in the confinement spike backlog.
- No change to the four verbs, `diff --porcelain`, or the APFS backend.
