# confinement spike — Linux rootless-userns probe

Throwaway probe for the `confinement-spike`. Tests whether an agent shell can
be structurally confined on Linux **without host root** — closing the
`umount -l` escape the `oops-shell` spike found — and enumerates the escape
vectors. Full analysis and the macOS story live in
`openspec/changes/confinement-spike/design.md`.

## `userns_escape_probe_linux.sh`

Design under test: **userns A** (`unshare -U -m -r`) owns the mount ns and
mounts the overlay **over the target** (real lower shadowed in place);
**userns B** (nested `unshare -U`, no uid map → `uid=nobody`) hosts the agent
shell and holds no `CAP_SYS_ADMIN` over A's mount ns.

Run it (from the repo root) — **without `--privileged`**, as a non-root uid,
with Docker's added seccomp/AppArmor lifted (a bare-metal user doesn't have
that layer):

```console
$ docker run --rm \
    --security-opt seccomp=unconfined --security-opt apparmor=unconfined \
    --tmpfs /scratch:exec,mode=1777 --user 1000:1000 \
    -v "$PWD/spikes/confinement":/c:ro rust:1.97-slim-trixie \
    unshare -U -m -r bash /c/userns_escape_probe_linux.sh
```

(`--tmpfs /scratch` stands in for a real, non-overlay fs for the upper/work
layers — Docker's `/` is overlayfs and can't host an upperdir. A bare-metal
user's `$HOME` already qualifies.)

## Proven results (kernel 6.10.14 LinuxKit, non-root, unconfined policy, no privileged)

| Finding | Result |
|---|---|
| unprivileged userns + mount ns | ✅ works |
| unprivileged overlay mount | ✅ works — **but `redirect_dir=off` is REJECTED (EPERM)**; use `metacopy=off,userxattr` |
| agent (child userns B) `umount -l` / `umount` the overlay | ❌ **BLOCKED** (EINVAL) |
| agent `nsenter` into pid1's mount ns | ❌ **BLOCKED** (EPERM) |
| agent write to the target | lands in **upper**; real lower **byte-identical** after unmount |

**Verdict:** the `oops-shell` `umount` escape is **closed** by nesting the
agent in a child userns; mount-over-target keeps writes in the upper with the
real files pristine — all rootless. Commit/undo authority stays with the
launcher in A. Linux reaches **tier 3** for commit-authority.

**The one required backend change:** oops mounts with `redirect_dir=off`
today, which unprivileged overlay forbids. Dropping it means the kernel may
write `redirect` xattrs in `user.overlay.*`, which the current
`changes/validate_upper/replay` in `src/backend/overlayfs.rs` deliberately
refuse — so the rootless port includes reworking that xattr handling. This is
the stage-2 long pole in the design doc.

## Honesty notes

- Run on Docker Desktop's LinuxKit 6.10 VM, not bare-metal Debian/Ubuntu/
  Fedora. The kernel mechanism is mainline; the **distro policy matrix**
  (esp. Ubuntu's `apparmor_restrict_unprivileged_userns`) is reasoned from
  documented behavior — verify on real hosts in implementation.
- `umount` and `nsenter` blocks are **executed**. The `ptrace` /
  `/proc/<pid>` / fd-passing vectors are **reasoned** from Linux
  `ptrace_may_access` rules, not each scripted — stated as such in the design
  doc's vector table.
