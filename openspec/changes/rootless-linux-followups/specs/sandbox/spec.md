# sandbox Specification — delta for rootless-linux-followups

## ADDED Requirements

### Requirement: Rootless setup diagnostics
When rootless sandbox setup fails, the error MUST be a single actionable
diagnostic regardless of which stage failed. A restricted unprivileged user
namespace (notably Ubuntu 23.10+'s default AppArmor policy) does NOT make the
`unshare` fail — `unshare` succeeds but the new namespace is stripped of the
capability to write its own id map, so the failure surfaces later as EPERM
writing `/proc/self/uid_map`. Both the `unshare` failure and the id-map-write
failure MUST therefore produce the same message, and that message MUST name:
(a) the full sysctl `kernel.apparmor_restrict_unprivileged_userns` to set to
`0` and how to persist it, and (b) the explicit `OOPS_PRIVILEGED=1` fallback
together with its honest downgrade (root required; tier-1/2 only; no nested
namespace). A message that names neither the sysctl nor the fallback is
non-conforming (it strands the user with no next step). Whichever stage
fails, the command MUST NOT have executed (fail closed).

#### Scenario: Restricted userns yields an actionable message
- **WHEN** `oops run` is invoked as a normal user on a host where unprivileged user namespaces are restricted (e.g. stock Ubuntu 24.04), so the id-map write fails with EPERM
- **THEN** the command never runs, and the error names the full `kernel.apparmor_restrict_unprivileged_userns` sysctl (with how to persist it) and the `OOPS_PRIVILEGED=1` fallback with its tier-1/2 caveat

#### Scenario: Confirmed rootless baseline is documented
- **WHEN** a user reads the Linux guarantee matrix
- **THEN** it states rootless is verified on kernels 6.1 and newer (ext4 and btrfs), that Debian 12 and Fedora 44 need no root out of the box, and that Ubuntu 24.04 requires the one sysctl (or the tier-1/2 privileged fallback)
