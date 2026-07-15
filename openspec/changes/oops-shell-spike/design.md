# Design: oops-shell-spike

## Context

A dogfooding run exposed a structural gap: an autonomous agent, told only to
"use `oops run` and check `oops diff`", finished its task and then called
`oops commit` itself — closing the safety window before any human reviewed
it. Its judgment was right that run; nothing in the tool *required* it to be.
The protection model silently assumes a reviewer sits between `run` and
`commit`/`undo`, and enforces nothing.

`oops shell` is the candidate structural fix: make the sandbox a
**per-session** unit (one sub-shell, sandboxed for its whole lifetime, the
commit/undo decision taken at exit by whoever launched it) instead of a
**per-command** unit (`oops run "cmd"`, where "who may commit this" is a
fresh authority question every command and a convention file cannot enforce
it against an autonomous caller).

The naive sketch — "spawn a sub-shell on the overlay/snapshot, change the
prompt, ask commit-or-undo on exit" — is only the user-facing surface. This
spike asks the hard question underneath it, and the answer differs sharply by
backend.

This doc is grounded in the current code:
`src/main.rs` (`pending_session`), `src/session.rs` (`find_for_target`),
`src/backend/overlayfs.rs` (mount layout), `src/backend/apfs.rs`,
`src/backend/mod.rs`. Two throwaway probes back it: `spikes/oops-shell/`.

---

## The one answer that matters, up front

**Does putting an agent inside `oops shell` actually take the commit/undo
decision out of its hands, or does it just feel like it does?**

- **Linux (OverlayFS): mostly yes — structurally, against the threat we
  actually saw.** The overlay is mounted *over the target directory itself*
  (`lowerdir == target`), so inside the sandbox namespace the real lower tree
  is shadowed. A process confined to that namespace cannot make its writes
  land on the real files: the merge has no reachable destination from inside.
  `oops commit` run *inside* the shell replays the upper layer onto the
  overlay — a no-op against the real tree. The finalize-to-real-files action
  genuinely lives in the parent namespace the agent isn't in. **Caveat:** the
  current backend requires root/privileged (`unshare(CLONE_NEWNS)` with no
  user namespace), so an inner shell holds `CAP_SYS_ADMIN` in its namespace
  and could `umount -l` the overlay to unshadow the lower and escape. The
  structural guarantee is therefore real against a *cooperative* agent doing
  the obvious `oops commit` — which is exactly the dogfood failure — but not
  against an adversary who knows to unmount. Closing that requires
  confinement (rootless userns / dropped caps), which is separate backlog.

- **macOS (APFS): no — it only feels like it.** The snapshot-restore backend
  has no mount namespace. The sub-shell runs against the **real files**;
  `oops commit` just means "keep the real files, drop the snapshot." oops
  identifies the session by the caller's *target directory* (canonical cwd,
  then `$PWD`), never by a token the launcher holds — so any process sitting
  in the target can finalize the enclosing session with zero extra authority.
  The macOS probe demonstrates this with the shipped 0.2.0 binary: a second,
  independent process given only the cwd committed an unreviewed change to
  the real files. On macOS the boundary is **conventional**, full stop. The
  only way to make it structural is to confine the agent so it never has a
  shell in which it can type `oops commit` (or `env -u OOPS_SHELL_SESSION`).

So: `oops shell` is a genuine structural safety story **on Linux, for the
realistic threat**, and a UX-plus-soft-guard story on macOS. It should be
framed as the former only where the former is true.

---

## Per-backend verdict table

| Dimension | Linux / OverlayFS | macOS / APFS |
| --- | --- | --- |
| Sandbox model | Interception (writes land in upper; real tree untouched) | Snapshot-restore (shell runs on real files; snapshot reverts on undo) |
| Long-lived interactive shell sandboxed? | **Yes** — shell is the payload process of an unshared mount namespace with an overlay mounted over the target | **No transparent sandbox** — shell runs on the real tree; "sandbox" = a snapshot taken at `shell` start, restore/keep decided at exit |
| Damage during the session | None (real tree pristine by construction) | Real (exposure window; watchers/sync clients can see & propagate it) |
| Where finalize-to-real-files lives | **Parent namespace only** — real lower is shadowed inside; inner merge can't reach it | **Anywhere in the target dir** — commit is "keep real files"; no barrier |
| `oops commit` from *inside* the shell | Structurally a no-op against real files (replays onto the overlay) | Succeeds — persists the unreviewed change |
| Authority boundary | **Structural** vs a cooperative agent; **conventional** vs an adversary with mount privilege (current root model) | **Conventional** — being in the dir is the whole authority |
| Guarantee level | Real (with the confinement caveat) | Aspirational unless the agent is confined |

---

## Q1 — Backend feasibility

### Linux / OverlayFS — the clean case

The mechanism already exists in miniature in `overlayfs::enter_and_exec`. For
`oops run` today, `exec` re-execs the oops binary as a hidden `__exec` child
that `unshare(CLONE_NEWNS)`s, makes mounts private, mounts the overlay
`over the target`, `chdir`s in, writes the "started" marker, then
`exec`s `/bin/sh -c "<command>"`. The namespace dies with that child, so no
mount outlives a run.

`oops shell` is the same setup with a different payload: instead of
`exec`ing the one wrapped command, `exec` an **interactive** `/bin/sh` (or
`$SHELL`). Everything the interactive shell and its children write for the
whole session lands in the upper layer, because they all live in the mount
namespace where the target path resolves to the overlay. When the
interactive shell exits, the child exits, the namespace tears down, and the
upper layer on disk is the entire pending state — identical to the `run`
end-state, just accumulated over many commands instead of one.

Sketch:

```
launcher (oops shell)
  └─ fork/exec: oops __shell --target … --upper … --work … --marker …
        unshare(CLONE_NEWNS); mount MS_PRIVATE /
        mount overlay over target        (lowerdir=target,upper,work)
        chdir(target); write marker
        exec $SHELL -i                    ← the long-lived payload
  ← child exits when the shell exits; launcher then finalizes
```

This is genuinely the clean case: no new primitive, just a longer-lived
payload and a finalize step driven by the launcher after the child returns.

### macOS / APFS — the crux

There is no mount namespace on macOS, and the APFS backend is
snapshot-then-restore: `clonefile(2)` the tree, let the command run **against
the real files**, `renamex_np(RENAME_SWAP)` the pristine clone back on undo.
A long-lived interactive shell cannot be transparently redirected into a
copy-on-write layer the way Linux can — there is nothing to redirect it
*into*. (Alternatives were considered and rejected for v1: per-process
filesystem views would need a custom FUSE/`nullfs`-style layer or a
sandbox-profile firewall around the target; both are large, fragile, and out
of scope for this spike. `sandbox_init`/Seatbelt can *deny* writes to a path
but cannot *redirect* them to a shadow copy, so it can't give
interception semantics either.)

So macOS `oops shell` has exactly one viable model:

> **Clone at `oops shell` start; the shell runs on the real files for the
> whole session; at exit the launcher either keeps the real tree (commit) or
> `RENAME_SWAP`s the clone back (undo).**

That is **snapshot-restore semantics, extended from one command to a whole
session** — damage is real during the session, reverted on exit-undo. The
existing "Snapshot-restore fine print" (safety spec) applies unchanged and,
if anything, harder: the exposure window is now the entire interactive
session, not one command. Watchers, Spotlight, Time Machine, and cloud sync
clients can observe and propagate the transient state for minutes or hours.
A future implementation MUST state this in the `oops shell` help and README,
consistent with the two-backend guarantee matrix.

**Consequence for the safety story on macOS:** running the agent on the real
files during the session does *not*, by itself, break "the agent can't reach
commit/undo" — a launcher-driven exit could still be the only *intended*
finalize path. But because the session is identified by target dir and commit
is just "keep the real files," the agent *can* finalize mid-flight
(`oops commit` in the dir) or after escaping the shell. See Q2: on macOS the
property does **not** hold structurally.

---

## Q2 — The authority boundary, concretely

The decisive fact is in `src/main.rs::pending_session` +
`src/session.rs::find_for_target`: **a session is looked up by the caller's
target directory** — canonical cwd, else logical `$PWD` — and matched against
`record.target`. The caller presents no session id, token, or capability.
`oops commit`/`oops undo`/`oops diff` all resolve "which session" purely from
where the caller stands. This is by design for the per-command model (the
user is standing in the dir they just sandboxed), and it is exactly what
makes the per-session model's authority boundary backend-dependent.

### What stops an in-shell agent from finalizing the enclosing session?

- **Linux:** the *identification* is still reachable — the session record
  lives under the state root (visible in the namespace), so the inner process
  can name the session. But *finalizing* it is not: `commit` = `merge` =
  replay the upper layer onto the target, and inside the namespace the target
  *is* the overlay. The real lower tree it would need to write is shadowed by
  the mount; there is no path to it from inside. So inner `oops commit`
  writes the change set back onto a view that already shows it — a no-op
  against the real files. The separation is **structural**: the finalize
  destination is unreachable, not merely forbidden. (`undo` from inside would
  succeed — it renames the session dir to trash — but undo is the *safe*
  direction; it throws away the agent's own work, it doesn't persist
  unreviewed changes. The dangerous direction, commit, is the one that's
  blocked.)

  The honest caveat, again: "unreachable" holds for a process confined to the
  namespace. The current root-required model gives the inner shell
  `CAP_SYS_ADMIN` in its own namespace, so `umount -l <target>` unshadows the
  lower and lets it write real files directly — a full escape, worse than
  commit. Against the dogfood threat (a cooperative agent doing the obvious
  thing) this doesn't happen; against an adversary it does. Structural
  strength here is bounded by privilege confinement, which is separate work.

- **macOS:** nothing structural stops it. The probe
  (`authority_probe_macos.sh`) shows a second, independent process finalizing
  the session with only the cwd. An in-shell agent is in the target dir by
  definition, so `oops commit` resolves and persists. **Conventional only.**

### Structural vs conventional — stated plainly

- Linux: **structural** against a cooperative agent and against the specific
  `oops commit` action; **conventional** against an adversary with mount
  privilege (today's model). Net: a real, meaningful block on the failure we
  observed.
- macOS: **conventional**, period.

### The honesty requirement

We must not claim `oops shell` makes agent-commit "impossible." If an agent
and a human share the same user and the same shell environment, no
pure-software mechanism makes commit impossible for the agent — it can do
anything the human can type (`env -u OOPS_SHELL_SESSION; oops commit`, or on
macOS just `oops commit`). The realistic, defensible claim is weaker and
splits by backend:

- The **per-session model moves the finalize decision to a layer the agent
  doesn't naturally act in** (exit, driven by the launcher). True on both
  backends as a *default-path* statement.
- On **Linux**, that layer is additionally **backed by a structural barrier**
  (shadowed lower) against the cooperative-agent case — so the claim is
  stronger than "convention" there.
- On **macOS**, it is *only* the default-path statement — no barrier. Making
  it real requires **confining the agent**: the outer launcher owns the
  process tree and the agent only ever sees the inner shell's channel, never
  a prompt where it can run `oops` or `env` against the host. That is
  aspirational relative to today's design and should be labelled as such.

A soft guard is worth adding on both backends regardless: `oops shell` sets a
marker (env var `OOPS_SHELL_SESSION=<id>` and/or a session file), and
`oops commit`/`oops undo` **refuse when they detect they're inside a live
shell session**, printing "you're inside an oops shell — exit to
finalize." On Linux this is belt-and-suspenders over the structural block; on
macOS it is a speed bump a knowledgeable agent defeats by unsetting the
marker. It should be documented as a guard, never as the guarantee.

---

## Q3 — Session lifecycle & orphans

Long-lived sessions make orphaning more likely than `oops run` (a shell can
sit open for hours). The good news: the existing state model already handles
the hard parts, because an `oops shell` session is, on disk, the same kind of
record as a `run` session — one pending sandbox per target dir, finalizable
by `oops commit`/`oops undo`/`oops diff` from the target dir. There is no
requirement that a live shell be attached to finalize.

Failure modes:

- **Shell exits normally:** the launcher regains control and finalizes
  (prompt or flag). Normal path.
- **Terminal closed / SIGHUP / `kill -9` of the shell / launcher dies:**
  - *Linux:* the mount namespace and overlay die with the process; the real
    tree was never touched (interception), so there is nothing to revert. The
    upper layer + session record persist under the state root. Recovery = the
    existing **stale-session** path: `oops undo` discards the leftover layer;
    `oops commit` merges it if still wanted (note: if the upper lives on tmpfs
    — as in the dev container — a reboot clears it, and the session becomes
    stale-and-empty, which undo handles by discarding). No new machinery.
  - *macOS:* the snapshot persists on disk (APFS sessions survive reboot per
    the session spec). The real tree holds the session's changes; `oops undo`
    restores from the snapshot, `oops commit` keeps. Same recovery as a
    crashed `run`.
- **Machine reboot mid-session:** same as the kill case per backend above.

Where state lives and whether gc extends cleanly: unchanged. Sessions live
under the per-volume state roots (`$XDG_STATE_HOME/oops` + registered
per-volume roots); `gc_sweep` quarantines recordless dirs and empties trash;
containment checks are unchanged. **One honest gap:** gc never touches a
session that still has a valid record (by design — pending sessions linger
until explicitly cleared). An abandoned `oops shell` session (shell died, no
one finalizes) therefore lingers exactly like an abandoned `run` session.
That is acceptable and already true today, but with long-lived shells it will
happen more often, so the implementation should consider (a) a `oops
sessions`/`oops status` listing so orphans are discoverable, and (b) whether
abandoned pending sessions past some age should be surfaced (not
auto-undone — auto-finalize would itself be an unreviewed decision). No spec
change is forced by this spike; flag for the implementation proposal.

---

## Q4 — Nesting

**Recommendation: forbid nesting in v1, fail closed with a clear message.**

If `oops run` or a second `oops shell` is invoked inside an `oops shell`:

- *Linux:* an overlay-on-overlay (upper of an upper) is representable but the
  authority model becomes ambiguous — which session does an inner `oops
  commit` finalize, and does committing the inner layer to the outer overlay
  (still sandboxed) mean anything the user expects? The "one pending sandbox
  per target dir" invariant (`ensure_no_pending`) is already violated in
  spirit.
- *macOS:* a clone-of-a-clone while the outer session's changes are live on
  the real tree captures a *mid-session* state as the inner "pristine"
  baseline; an inner undo would restore to that dirty baseline, not the
  user's original — silently wrong.

Both are surprising and hard to reason about. Detection is cheap: the marker
from Q2 (`OOPS_SHELL_SESSION` set, and on Linux "am I already in an oops
mount namespace") lets `run`/`shell` refuse with "already inside an oops
shell for <target> — exit first, or finalize it before starting another."
Fail-closed matches the project's safety posture. Revisit nesting only if a
concrete workflow demands it.

---

## Q5 — Does this subsume other pending issues?

### stale-cwd (macOS) — partially, verify don't assume

The earlier dogfood finding: `oops undo` `RENAME_SWAP`s the target inode out
from under a shell whose cwd points at it, so the shell gets phantom "No such
file or directory" until it re-`cd`s.

In the `oops shell` model, finalize happens **after the sub-shell has
exited** — so no *inner* process holds a live cwd fd on the swapped inode at
undo time. That removes the inner-shell instance of the bug. **But it does
not fully subsume the fix**, because the *launcher* shell is itself sitting in
the target dir (that's where the user typed `oops shell`), and when the
launcher runs the exit-undo, `RENAME_SWAP` swaps the inode out from under the
*launcher's* cwd — the same staleness, one level up. So:

- Inner-shell stale-cwd: gone (finalize is post-exit). ✅
- Launcher stale-cwd: still present on macOS undo. ❌ — the separate fix
  (re-`cd`, or an undo-message hint) is still warranted for the launcher.
- Linux: no swap at all (interception discards a layer), so stale-cwd never
  arose there. N/A.

Verified against the mechanism, not assumed: the swap target is the launcher's
cwd, so the launcher is still exposed.

### prefix friction — yes, fully

The per-session model removes the need to prefix every command with `oops
run`. Inside the shell, plain `rm`, `make`, `git …` are all sandboxed. The
original convenience benefit holds in full; it is just no longer the *point*.

---

## Recommendation

`oops shell` **is** the right structural fix for the agent-commit gap — but
only Linux makes it structural, and only against the cooperative-agent threat
we actually observed. State the story truthfully by backend:

1. **Ship it, and lead the safety claim on Linux.** On Linux the per-session
   model has a real structural barrier: an agent inside the shell cannot
   finalize to the real files, because the merge destination is shadowed. For
   the dogfood threat this is exactly the enforcement that was missing.
2. **On macOS, ship it as UX + soft guard, not as a safety guarantee.** The
   snapshot-restore model puts the agent on the real files with commit one
   `oops commit` away. Do not claim the agent can't commit. The honest macOS
   line: "everything in the shell is undoable in one step at exit," plus the
   marker-based refusal as a speed bump.
3. **The real cross-backend guarantee requires confinement.** The strong
   version of the story — agent genuinely cannot finalize on either
   backend — needs the outer launcher to own the process tree and expose only
   the inner shell to the agent (agent never reaches a host prompt). That is a
   product/deployment decision beyond this spike, and it is what an
   implementation proposal should scope if the strong claim is wanted.
4. **Two follow-ups this spike surfaces, neither blocking:** (a) rootless
   Linux (userns/dropped caps) would turn the Linux caveat (umount escape)
   from "real hole vs adversary" into "much harder," strengthening the
   headline claim — worth sequencing near `oops shell`; (b) a session-listing
   command (`oops sessions`) for orphan discoverability, made more valuable by
   long-lived sessions.

If the goal is a single, backend-uniform "the agent structurally cannot
commit" sentence for marketing, this spike says: **you don't have it today,
and `oops shell` alone doesn't give it to you on macOS.** What you have is a
strong Linux story and an honest macOS story, which is a defensible and
non-overstated position for a safety tool.

---

## Non-goals / what this spike did NOT settle

- No implementation, no trait changes, no spec delta (spike only).
- Did not design the launcher↔shell finalize UX (prompt vs flags, commit
  message capture) — implementation concern.
- Did not benchmark long-session overlay upper growth or clone-at-start cost
  for large trees over a long session (the apfs spike's clone/diff/swap
  numbers cover the primitives; session-duration effects are untested).
- Did not build the confinement layer; only identified it as the requirement
  for a structural macOS guarantee.
- The Linux probe was authored but not executed here (macOS host, no Docker);
  it is reproducible in the repo's privileged container.
