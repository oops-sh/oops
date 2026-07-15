# Proposal: confinement-spike

## Why

The `oops-shell` spike proved the current safety boundary is escapable: on
Linux the sandbox is structural against a cooperative agent but a
prompt-injected one can `umount -l` its way out (the backend requires root
and hands the inner shell CAP_SYS_ADMIN); on macOS the boundary is merely
conventional — a second process holding only the cwd committed an unreviewed
change with the shipped 0.2.0 binary. The origin is a dogfood failure: an
autonomous agent called `oops commit` on its own unreviewed work. The
decision is to build confinement — structural isolation — before launch. This
spike must define "done" (it has no natural finish line), test the Linux
mechanism with real data, and face the macOS reality honestly, so the launch
gate is a real answer and cannot silently expand to unwinnable scope.

## What Changes

Research spike: no product code, no spec delta. Deliverables:

- `design.md` answering: (Q1) a threat-model tier table with a recommended
  ship gate — **tier 3 (prompt-injected agent); tier 4 explicitly excluded**;
  (Q2) Linux rootless confinement, empirically tested — unprivileged userns +
  mount-over-target + nested-userns agent, with every escape vector
  verdicted; (Q3) macOS: native confinement killed, Linux-guest route
  evaluated (Lima/Apple container/OrbStack/Docker) with the file-sharing hole
  faced concretely; (Q4) a launcher-held session token and exactly which tier
  it buys per backend; (Q5) architecture sketch + staged plan with honest
  effort estimates and the minimum set for the tier-3 gate.
- `spikes/confinement/userns_escape_probe_linux.sh` + README: an executed
  probe proving the agent (nested child userns) cannot unmount the sandbox,
  cannot `nsenter` out, and its writes never reach the real files — rootless,
  no host root — and establishing that `redirect_dir=off` is rejected for
  unprivileged overlay (a required backend change).

## Capabilities

### New Capabilities

_None — spike only. Spec revisions are proposed in design.md and enacted by a
future implementation change, if the user approves one._

### Modified Capabilities

_None._

## Impact

- New files under `spikes/confinement/` and this change directory. No
  dependencies, no runtime changes, no changes to shipped behavior.
- Output feeds the decision on an `add-confinement` implementation proposal
  and the launch gate. Identifies the overlay `redirect`/`user.overlay.*`
  xattr rework as the long pole.

## Non-goals

- Any implementation of confinement, backend/CLI changes, or spec edits.
- Booting real Debian/Ubuntu/Fedora hosts for the distro matrix (reasoned +
  LinuxKit-probed; verify in implementation).
- Building the Lima guest orchestration or the tier-2 controlled-rootfs
  layer; both are scoped and staged in design.md, not built.
- Tier-4 (malicious code engineering a kernel/hypervisor escape) — explicitly
  out of scope for the ship gate.

## Housekeeping

- Deleted the orphaned 2026-07-12 session in the real state root (target
  `/Users/jesse/Documents/Test`, `touch i_was_here.txt`) — leftover manual
  testing whose survival confirmed the shell spike's Q3 gap. Backlogged
  `oops sessions` (list pending/orphaned sessions).
