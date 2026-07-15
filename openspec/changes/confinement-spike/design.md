# Design: confinement-spike

## The one answer, up front (two sentences per backend)

**Linux:** with a rootless-userns redesign we can reach **tier 3
(prompt-injected agent)** — proven here: an agent in a nested child user
namespace cannot unmount the sandbox, cannot `nsenter` out, and its writes
never reach the real files; the smallest work that gets us there is
(a) switch the backend to unprivileged userns + mount-over-target with
rootless-compatible overlay options, and (b) run the agent shell in a nested
child userns — roughly **1.5–3 weeks**.

**macOS:** the native APFS backend cannot reach tier 3 at all (no
namespaces; the shell runs on the real files and any process in the target
dir can finalize — the shell spike proved this empirically), so tier-3
agent confinement on a Mac means **running the agent in a Linux guest** and
keeping the human on the host; the APFS backend stays a genuine **tier-1/2
tool for humans undoing their own mistakes**, not an agent-confinement
mechanism, and the smallest honest macOS story is "Linux confinement inside
a VM (weeks, mostly UX/plumbing) OR ship APFS as human-undo only and say so."

Everything below is the evidence and the staging for those two sentences.

---

## Context

Two prior spikes set this up. `oops-shell-spike` established that the
per-session model moves the finalize decision to the launcher, and that the
Linux boundary is *structural against a cooperative agent* (real lower masked
inside the namespace) but escapable via `umount -l` because today's backend
requires root and hands the inner shell `CAP_SYS_ADMIN`; and that macOS is
*conventional only* — proven with the shipped 0.2.0 binary, where a second
process holding nothing but the cwd committed an unreviewed change. The
decision was made to build confinement before launch. This spike defines the
finish line, tests the Linux mechanism empirically, and faces the macOS
reality.

Grounding in code: `src/backend/overlayfs.rs` (mount layout + options),
`src/main.rs` / `src/session.rs` (session identified by target dir, no
token), `src/backend/mod.rs`. Probe: `spikes/confinement/`.

---

## Q1 — Define "done": the threat-model tier table (answer FIRST)

| Tier | Adversary | Concrete example | oops **today** | with **confinement** |
|---|---|---|---|---|
| 0 | Cooperative, good judgment | follows CLAUDE.md, commits appropriately | Linux ✅ · macOS ✅ | ✅ / ✅ |
| 1 | Cooperative, **bad judgment** (the dogfood case) | commits its own unreviewed work; deletes the wrong "dead" file | Linux ⚠️ structural-but-escapable · macOS ❌ conventional | Linux ✅ · macOS ✅ *(guest)* / ⚠️ *(APFS: undo exists, but agent can still finalize)* |
| 2 | **Careless** | accidentally writes outside the tree (`/tmp`, `~/.config`, absolute paths), spawns a daemon | Linux ❌ *(overlay covers only the target subtree)* · macOS ❌ | Linux ✅ *(needs a full mount-ns rootfs, not just target overlay)* · macOS ✅ *(guest)* |
| 3 | **Prompt-injected** | acts on hostile instructions; actively tries documented escapes (`umount`, `nsenter`, finalize mid-flight) | Linux ❌ *(umount escape)* · macOS ❌ | **Linux ✅ (proven below)** · macOS ✅ *(guest only)* |
| 4 | **Actively malicious code** | kernel/hypervisor exploit, deliberate escape engineering | ❌ (out of scope) | ❌ — VM/hardware territory, explicitly **not** a v0.3 prerequisite |

### Recommended ship gate: **tier 3**, and tier 4 is explicitly excluded.

I agree with the prior. **Tier 3 is the right gate** because the whole reason
this spike exists is the dogfood failure (tier 1) and its adversarial cousin:
an agent that has read a hostile file and *actively* tries to finalize or
escape (tier 3). A safety story that only survives a cooperative agent is the
thing we already have and already found wanting. **Tier 4 must not be a
prerequisite** — defending against deliberate kernel/hypervisor exploitation
is VM-and-hardware work with no natural finish line, and folding it into v0.3
is exactly the scope-creep this question exists to prevent. Say it in the
spec when the implementation lands: *oops confinement targets tiers 0–3; a
tier-4 adversary (malicious code engineering an escape) is out of scope and
requires a hardware-isolated VM the user controls.*

One important nuance the table encodes: **tier 2 (careless, writes outside
the tree) is a different mechanism from tier 3 (finalize/escape)**. The
overlay only covers the target subtree, so even a fully "confined" agent can
still `echo x > /tmp/y` or `~/.bashrc`. Closing tier 2 requires the agent to
run in a mount namespace whose *whole* rootfs is controlled (only the target
writable, everything else read-only or private) — see Q5. Confinement is not
one switch; the tiers need naming so "done" can't drift.

---

## Q2 — Linux: rootless confinement (empirically tested)

All results below are from `spikes/confinement/userns_escape_probe_linux.sh`,
run on **kernel 6.10.14 (Docker Desktop's LinuxKit VM)** as a **non-root uid
(1000)** with **seccomp and AppArmor unconfined** and **no `--privileged`, no
`cap-add`** — i.e. emulating an ordinary unprivileged user on a distro that
permits unprivileged userns. (Docker's default seccomp profile *blocks*
`unshare(CLONE_NEWUSER)`; that is Docker policy, not a kernel limit, so it is
lifted to test the kernel mechanism a bare-metal user would actually have.)

### Can the backend run unprivileged? Yes — with one required option change.

- **Unprivileged userns + mount ns**: `unshare -U -m -r` works; inside, the
  user is uid 0 mapped to host uid 1000, owning its own mount namespace. ✅
- **Unprivileged overlay mount**: works ✅ **but only with
  rootless-compatible options.** The probe confirms **`redirect_dir=off` is
  rejected with EPERM** for unprivileged mounts. oops mounts today with
  `redirect_dir=off,metacopy=off` (`overlayfs.rs`), so **the current option
  set does not port to rootless.** The rootless option set is
  `metacopy=off,userxattr` (drop `redirect_dir=off`, add `userxattr` so
  overlay xattrs live in the `user.` namespace an unprivileged mount can
  write).
  - **Consequence for the backend, concretely:** `overlayfs.rs`'s
    `RECOGNIZED_OVERLAY_XATTRS` deliberately **excludes `redirect`**, and the
    merge/diff walk assumes `redirect_dir=off` ("seeing them means an upper
    layer we cannot replay"). Under rootless mounts the kernel may write
    `redirect` xattrs (renamed dirs), and they'll be in `user.overlay.*` not
    `trusted.overlay.*`. So the rootless port is **not** just a mount-flag
    change: `changes()`, `validate_upper()`, and `replay()` must learn to
    read `user.overlay.*` and to handle (or explicitly, safely refuse) the
    `redirect` xattr. This is the single biggest backend work item and the
    doc flags it loudly.

### Distro variance (the real matrix)

- **Kernel**: unprivileged overlay-in-userns needs **≥ 5.11**. Below that,
  no rootless overlay — fail closed.
- **Fedora / Arch / openSUSE**: unprivileged userns enabled by default →
  works.
- **Debian ≥ 11 / Ubuntu**: ship `kernel.unprivileged_userns_clone` and,
  since Ubuntu 23.10, an **AppArmor restriction** (`kernel.apparmor_restrict
  _unprivileged_userns=1`) that blocks unprivileged userns for unconfined
  binaries unless an AppArmor profile grants `userns create`. So on stock
  modern Ubuntu, oops would need either the sysctl relaxed or a shipped
  AppArmor profile. This is a real friction point to document, not hand-wave.
- **RHEL/CentOS Stream**: userns on; overlay-in-userns on recent kernels.
- **Fallback when userns/overlay is unavailable**: **fail closed** with an
  actionable message — e.g. "rootless sandboxing needs unprivileged user
  namespaces (kernel ≥ 5.11; on Ubuntu, `sysctl -w
  kernel.apparmor_restrict_unprivileged_userns=0` or install the oops
  AppArmor profile). Refusing to run rather than run unconfined." The
  fail-closed invariant already exists (`safety` spec); this extends it.

### The escape hole — enumerated and verdicted (the point of the question)

The confinement design: **userns A** (`unshare -U -m -r`) owns the mount ns
and mounts the overlay **over the target** (so `lowerdir == mountpoint`,
shadowing the real lower in place); **userns B** (`unshare -U`, nested, no
uid map → `uid=nobody`) hosts the agent shell. B is a *descendant* userns, so
it holds no `CAP_SYS_ADMIN` over the mount ns owned by A. Probe verdicts:

| Vector | Result | Why |
|---|---|---|
| `umount -l <target>` | **BLOCKED** (EINVAL) | umount checks caps against the userns owning the *mount ns* (A); B is a child userns and lacks CAP_SYS_ADMIN there |
| plain `umount <target>` | **BLOCKED** (EINVAL) | same |
| `nsenter --mount=/proc/1/ns/mnt` | **BLOCKED** (EPERM, cannot open) | `nobody` fails `ptrace_may_access` against pid1; can't enter its ns |
| write to the target path | **captured in UPPER** | target *is* the overlay; the real lower is shadowed → real files byte-identical after unmount (verified by sha) |
| direct write to the real lower *path* | **N/A under mount-over-target** | there is no separate reachable lower path — it's the mountpoint, occupied by the overlay. (A naive *separate-mountpoint* layout leaves the lower reachable and writable — the probe's earlier iteration confirmed that failure mode, which is why mount-**over**-target is required.) |
| `ptrace` the launcher (A) to hijack its creds | **BLOCKED** (reasoned, not probed) | `nobody` can't ptrace a process owned by uid 1000/0-in-A; `ptrace_may_access` denies cross-uid without CAP_SYS_PTRACE in the owning userns |
| `/proc/<launcher-pid>/root` or `/cwd` | **BLOCKED** (reasoned) | same `ptrace_may_access` gate on `/proc/<pid>/{root,cwd}` magic-symlink traversal for a non-owner |
| pass an fd over a unix socket | **needs a confederate** (reasoned) | not a self-escape: requires a cooperating process already outside the sandbox to send the fd; the agent inside cannot manufacture one |
| **symlink / absolute write outside the tree** (`/tmp`, `$HOME`) | **NOT blocked by the overlay** | tier-2 concern: the overlay covers only the target subtree. Closing this needs a full controlled rootfs (Q5), not the target overlay |
| remount `/proc` | irrelevant to finalize authority | doesn't reach the real lower or the launcher's creds |

**Bottom line, proven:** the shell-spike `umount` escape is **closed** by
nesting the agent in a child userns, and mount-over-target keeps the agent's
writes in the upper layer with the real files pristine — all without host
root. The finalize action (commit = replay upper; undo = discard upper) lives
in A / the launcher, which B structurally cannot reach. **Tier 3 is reached
for commit-authority on Linux.** Tier 2 (writes outside the tree) is a
separate, additional mechanism (Q5).

### Probe honesty notes

- Run on LinuxKit 6.10 via Docker, not on bare-metal Debian/Ubuntu/Fedora;
  the *kernel mechanism* is mainline and version-appropriate, but the
  *distro policy matrix* above (esp. Ubuntu AppArmor) was reasoned from
  documented behavior, not each distro booted. Flagged for the
  implementation to verify on real Ubuntu/Fedora hosts.
- `ptrace`/`/proc/<pid>` and fd-passing vectors were reasoned from the Linux
  `ptrace_may_access` rules, not each individually scripted; the umount and
  nsenter blocks *were* executed. Stated as such in the table.

---

## Q3 — macOS: face the OS reality

There is no mount namespace, no user namespace, no native lightweight
process-filesystem isolation on macOS. The APFS backend is snapshot-restore:
the shell runs on the **real files**; commit = keep them, undo =
`RENAME_SWAP` the clone back. The shell spike proved the authority boundary
is conventional — any process in the target dir finalizes. **There is no
native macOS mechanism that changes this**; `sandbox_init`/Seatbelt can
*deny* writes to a path but cannot *redirect* them into a shadow layer or
create a per-process mount view, so it cannot give either interception
semantics or a launcher-only finalize boundary. I evaluated and **kill the
idea of a native macOS confinement scheme.**

### The realistic route: agent in a Linux guest, human on the host

Confirmed as the only path to tier 3 on a Mac. The isolation happens at the
Linux layer (where Q2 shows it is real and rootless); the human stays on
macOS and holds commit/undo from outside the guest.

**Guest-layer options:**

| Option | Maturity | Install friction | Licensing | Notes |
|---|---|---|---|---|
| **Apple `Containerization` / `container`** | New (2025, macOS 15+/26); Linux-VM-per-container via Virtualization.framework | Bundled-ish on recent macOS; still early | Apache-2.0 | Native, promising, but young — betting the launch on it is risky in 2026 |
| **Lima** | Mature, CLI-first, scriptable | `brew install lima`; a `limactl start` | Apache-2.0 | Best fit for a CLI tool: reproducible YAML, virtiofs/9p mounts, no GUI |
| **OrbStack** | Mature, fast, polished | GUI app install | **Proprietary**, paid for commercial | Great UX, but licensing + closed-source make it a poor *default* dependency |
| **Docker Desktop** | Mature | GUI app; heavier | Proprietary (paid for large orgs) | Ubiquitous but a heavy, licensed default |

Recommendation for a confinement MVP: **Lima** (open, scriptable, CLI-native,
already how this very spike's Linux probes could be run), with Apple
`container` tracked as the eventual native successor.

### The make-or-break detail: the file-sharing hole

If the user's project is virtiofs/9p-shared from the Mac into the guest,
**agent writes land on the Mac's real files** and the entire structural
guarantee evaporates — the overlay in the guest would sit on top of a share
that is itself the real host tree. So **the protected tree and its overlay
must live INSIDE the guest**, on the guest's own filesystem, not on a host
share. Concretely what that means for the workflow:

- **Where the code lives:** the working copy the agent touches is a **clone
  inside the guest** (e.g. the guest does `git clone` of the user's repo, or
  a one-time copy-in). The agent runs entirely against the guest-internal
  tree, under the Q2 rootless overlay confinement.
- **How the human edits on the Mac:** they edit their *host* copy normally;
  the agent's guest copy is a separate checkout. This is the honest cost —
  it is **not** "the agent edits the files open in your Mac editor." It is
  closer to "the agent works on a branch in a sandbox VM."
- **How committed work returns to the host:** via the version-control
  boundary — the guest, after the human approves at the launcher, pushes a
  branch / emits a patch / the host pulls. `oops commit` inside the guest
  finalizes the guest overlay; getting it to the host is a **git push/pull or
  patch**, deliberately, because that is the reviewable boundary. (A
  host-share "sync back" would reintroduce the hole.)

This is real friction and must be stated as such — the macOS confinement
story is "agent works in a Linux VM on a checkout, results come back as
reviewable VCS changes," not a transparent local sandbox.

### Honest verdict for the APFS backend

The native APFS backend has **no path to tier 3**. It remains valuable as a
**tier-1/2 tool for humans**: "undo for your own terminal mistakes" —
`oops run rm -rf x`, look, `oops undo`. That is a real, shippable product for
a human at their own keyboard. But it is **not** an agent-confinement
mechanism, and the docs must not imply it is. So the macOS split is:

- **Human, own mistakes:** APFS backend, tiers 1–2, ship as-is. ✅
- **Agent confinement (tier 3):** Linux guest only. The Mac is the host where
  the human reviews; the agent never runs against host files.

---

## Q4 — Session identity: the missing token

The shell spike found sessions are keyed by the caller's target dir
(`pending_session` → `find_for_target`, matching `record.target`), never by a
launcher-held token — which is why any process in the dir finalizes. A
**launcher-held session token** (a random handle minted at `oops shell`/run
start, required by `commit`/`undo`, held by the launcher, never exported into
the inner environment) is worth evaluating independently of the OS work.

**What it buys, per backend — precisely:**

- **Linux (with Q2 confinement):** hardens an already-structural boundary
  (defense in depth). The child userns already blocks the agent from reaching
  the real files; the token additionally means that even if a future bug
  weakened the namespace story, finalize still needs a secret the agent was
  never given. Cheap insurance. Does not by itself create the guarantee — the
  namespace does — but strengthens it.
- **macOS (native APFS, no guest):** does **not** create a structural
  guarantee. A determined inner process on the **same user** can read the
  launcher's environment (`/proc` equivalent via `ps -E` is restricted, but
  same-uid processes can often inspect each other; a token file is readable
  if same-uid) or the token file. **But** it moves macOS from *"zero
  authorization needed"* (today: just be in the dir) to *"must actively steal
  a handle"* — a real **tier-2 → tier-3-ish** distinction: it stops the
  careless/cooperative-bad-judgment agent (tier 1) cleanly, and forces a
  prompt-injected agent to take a deliberate, describable extra step (locate
  and exfiltrate the token) rather than just calling `oops commit`. State
  plainly: this reaches **tier 1 solidly and raises the bar for tier 2–3 on
  macOS, but is NOT a tier-3 structural guarantee** — same-user, same-machine
  defeats it in principle.

**Worth doing regardless?** **Yes.** It is cheap (a random token in the
session record + an env/handle the launcher holds + a check in
`commit`/`undo`), it closes the "any process in the dir finalizes" gap that
looks bad in a demo, it is the honest minimum for the native macOS backend,
and on Linux it is defense-in-depth. It should be built **first** (Q5 stage
1) because it is backend-independent and low-risk.

---

## Q5 — Architecture sketch & staging

### What the product looks like after confinement

- **Linux, rootless:** `oops shell` (or `oops run`) becomes: launcher mints a
  session + token → `unshare -U -m -r` (userns A) → mount overlay over the
  target with rootless options → `unshare -U` (child userns B) → exec the
  agent's shell in B. The agent works normally; every write lands in the
  upper. On exit, the **launcher** (in A / the host process) prompts the
  human and runs commit (replay upper) or undo (discard upper). The agent in
  B can neither unmount, nsenter out, reach the real files, nor present the
  token. **This is the core safety story, and it is real.**
- **macOS, agent:** `oops` orchestrates a **Lima guest**: provision (once),
  clone/copy the project into the guest, run the Linux flow above *inside the
  guest*, surface diff to the host, human approves on the host, result comes
  back as a git branch/patch. Command surface could be `oops shell --guest`
  or an `oops env` that abstracts "where the agent runs."
- **macOS, human:** unchanged `oops run`/`undo` on APFS, documented as tier
  1–2 human-undo.

### Staging — most guarantee per unit of work, minimum set for tier 3

| Stage | Work | Tier delta | Effort |
|---|---|---|---|
| **1. Session token** | random token in the session record; launcher holds it; `commit`/`undo` require it; never exported inward | macOS tier 0→1 (and raises 2–3 bar); Linux defense-in-depth | **~2–4 days**, backend-independent, low risk |
| **2. Rootless Linux backend** | unprivileged userns + mount-over-target + rootless overlay options; **rework `changes/validate_upper/replay` for `user.overlay.*` + `redirect` xattr**; fail-closed matrix (kernel/AppArmor) with actionable message | Linux tier 1⚠️→ solid tier 1–2 (no more root; но agent still in same ns) | **~1–1.5 weeks**, the xattr rework is the risk |
| **3. Nested-userns agent shell** | run the agent in child userns B; launcher-driven finalize after B exits; block umount/nsenter (proven) | **Linux → tier 3 for commit-authority** | **~3–5 days** on top of stage 2 |
| **4. Controlled rootfs (tier 2 outside-tree)** | mount ns where only the target is writable, rest read-only/private; optionally pivot_root; seccomp to deny mount/ptrace as belt-and-suspenders | Linux tier 3 for *whole-system* writes, not just the tree | **~1 week** |
| **5. macOS Linux-guest orchestration** | Lima provisioning, project clone-in, diff surfacing to host, VCS return path | macOS → tier 3 (agent), via guest | **~2–3 weeks**, mostly UX/plumbing, not novel isolation |

**Minimum set that meets the tier-3 ship gate:**

- **On Linux:** stages **1 + 2 + 3** (token + rootless backend + nested
  userns). That is the real launch-blocking work: **~2–3 weeks**, dominated
  by the overlay-xattr rework in stage 2. Stage 4 (tier-2 outside-tree) is
  strongly recommended for the "prompt-injected agent" story to be complete
  but can be sequenced immediately after if launch pressure demands — call it
  out explicitly rather than silently dropping it, because a tier-3 agent
  that can still `rm -rf ~/.ssh` outside the tree is a weak tier 3.
- **On macOS:** stage **1** ships immediately with the native backend (honest
  tier-1 human tool). Full agent tier 3 on Mac is stage **5**, and it is
  **weeks** — so the honest near-term macOS position is: *human-undo now,
  agent-confinement via Linux guest later.* Do not claim macOS agent
  confinement until stage 5 lands.

### Honest effort summary

- Linux tier-3 (commit-authority): **~2–3 weeks** (stages 1–3).
- Linux tier-3 (whole-system, incl. outside-tree): **+~1 week** (stage 4).
- macOS agent tier-3: **~2–3 weeks more** (stage 5), and it changes the UX to
  "agent works in a VM on a checkout" — a product decision, not just eng.

---

## Recommendation & the two-sentence answer restated

- **Ship gate = tier 3. Tier 4 explicitly excluded** (write it into the spec
  so it can't drift).
- **Linux gets there and it's proven:** rootless userns + mount-over-target +
  nested-userns agent closes the umount escape and keeps writes off the real
  files with no host root. Launch-blocking work ≈ 2–3 weeks, the overlay
  `redirect`/`user.overlay.*` xattr rework being the long pole.
- **macOS native APFS cannot get there** and should be shipped honestly as a
  tier-1/2 human-undo tool; tier-3 agent confinement on a Mac means a Linux
  guest (Lima), which is a weeks-long, UX-heavy stage that reframes the
  product to "agent works in a VM."
- **Build the session token first** (stage 1): cheap, backend-independent,
  fixes the "any process in the dir finalizes" gap, and is the honest minimum
  for macOS.

## Non-goals / not settled here

- Spike only: no implementation, no trait/CLI change, no spec delta.
- Did not boot real Debian/Ubuntu/Fedora hosts — the distro/AppArmor matrix is
  reasoned + the kernel mechanism is probed on LinuxKit 6.10; verify on bare
  metal in implementation.
- Did not script `ptrace`/`/proc/<pid>`/fd-passing (reasoned from
  `ptrace_may_access`); umount + nsenter blocks are executed.
- Did not build the Lima orchestration or measure guest provisioning time.
- Did not design the `redirect`-xattr handling for rootless overlay merge —
  identified as the stage-2 long pole, left to the implementation proposal.

## Backlog surfaced by this spike

- **`oops sessions`** — list pending/orphaned sessions. The 2026-07-12 orphan
  (target `/Users/jesse/Documents/Test`, `touch i_was_here.txt`) survived,
  invisible, until manually found — empirically confirming the shell spike's
  Q3 gap. A safety tool where the user cannot see their own outstanding
  sessions is a blind spot. Deleted as housekeeping for this spike.
- Ubuntu AppArmor `userns create` profile (shipping artifact for stage 2).
- Apple `container` framework as the eventual native successor to Lima.
