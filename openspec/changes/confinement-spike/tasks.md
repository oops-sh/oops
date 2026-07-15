# Tasks: confinement-spike

## 1. Probe

- [x] 1.1 `spikes/confinement/userns_escape_probe_linux.sh`: unprivileged
  userns A + mount-over-target overlay + nested child userns B (the agent).
  Executed on kernel 6.10 LinuxKit, non-root uid, seccomp/apparmor unconfined,
  no `--privileged`.
- [x] 1.2 Establish the option-compat finding: `redirect_dir=off` is rejected
  for unprivileged overlay; rootless set is `metacopy=off,userxattr`.
- [x] 1.3 Verdict every escape vector: umount -l / umount / nsenter BLOCKED
  (executed); real lower byte-identical after B's writes (sha-verified);
  ptrace / /proc / fd-passing reasoned from `ptrace_may_access`.

## 2. Design answers

- [x] 2.1 Q1: threat-model tier table (0–4) with per-backend today-vs-
  confinement columns; recommend **tier 3** ship gate; **exclude tier 4**.
- [x] 2.2 Q2: Linux rootless feasibility (kernel ≥5.11, distro/AppArmor
  matrix, fail-closed fallback), the mechanism, the executed escape verdicts,
  and the required `user.overlay.*`/`redirect` xattr backend rework.
- [x] 2.3 Q3: macOS — kill native confinement; evaluate Lima/Apple container/
  OrbStack/Docker; face the file-sharing hole (protected tree lives INSIDE
  the guest; VCS return path); verdict APFS as a tier-1/2 human tool.
- [x] 2.4 Q4: launcher-held session token; precise per-backend tier gain
  (Linux defense-in-depth; macOS tier-1 solid, not tier-3 structural); build
  first.
- [x] 2.5 Q5: architecture sketch + 5-stage plan with effort estimates and
  the minimum set (Linux stages 1–3 ≈ 2–3 weeks) for the tier-3 gate.
- [x] 2.6 Two-sentence per-backend answer up front (reachable tier + smallest
  work).

## 3. Housekeeping

- [x] 3.1 Delete the 2026-07-12 orphan session; backlog `oops sessions`.

## 4. Wrap-up

- [ ] 4.1 Spike reviewed by the user; findings decide whether to open an
  `add-confinement` implementation proposal and set the launch gate.
