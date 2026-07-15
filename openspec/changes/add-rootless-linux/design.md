# Design: add-rootless-linux

## Context

Grounded in the two spikes (`openspec/changes/archive/.../oops-shell-spike`,
`confinement-spike`) and the current code: `src/backend/overlayfs.rs`,
`src/main.rs` (`__exec`), `tests/linux.rs`. The confinement probe
(`spikes/confinement/userns_escape_probe_linux.sh`, executed on kernel 6.10)
is the empirical basis. Ship gate: **tier 3** (prompt-injected agent). Tier 4
is out of scope by decision.

---

## 1. The `redirect_dir` migration — the core, treated first

### Why it is forced

`overlayfs.rs` mounts today with `redirect_dir=off,metacopy=off`. The probe
established that **`redirect_dir=off` is rejected with EPERM for unprivileged
overlay mounts** (`metacopy=off` is accepted). An unprivileged mount also
stores overlay xattrs in the **`user.overlay.*`** namespace, not
`trusted.overlay.*` (that is what the `userxattr` mount option selects, and
it is required — unprivileged mounts cannot write `trusted.*`). So two things
change at once:

1. the kernel may now write a **`redirect`** xattr when a directory is
   renamed inside the sandbox (instead of degrading to copy-up + whiteout);
2. every overlay xattr the backend reads or validates moves from
   `trusted.overlay.*` to `user.overlay.*`.

The commit replay path is the only code that touches real files, so it must
now understand this metadata exactly. This section specifies it
condition-by-condition; it is the delta flagged for word-by-word review.

### The exact metadata set under the new mount options

Mount options become `metacopy=off,userxattr` (no `redirect_dir` override →
kernel default `redirect_dir=on` for userxattr mounts). Under those options
the upper layer can contain:

| Encoding | Form | Meaning | Replay semantics |
|---|---|---|---|
| **Whiteout** | char device `0:0` at the name | the lower path was deleted | `remove_lower(path)` — unchanged from today |
| **Opaque dir** | xattr `user.overlay.opaque = "y"` on an upper dir | the lower dir was replaced wholesale (deleted + recreated) | delete the lower dir, then create fresh and replay children — the existing opaque path, xattr name changed |
| **Redirect** | xattr `user.overlay.redirect = "<path>"` on an upper dir | this upper dir is the rename *destination*; its contents come from the lower dir named `<path>` | **NEW**: the merge must move/copy the lower source `<path>` to this name, then apply the upper dir's own changes on top. See below. |
| **Regular create/modify/dir/symlink** | plain upper entry | as today | as today (`changes`/`replay` unchanged except xattr namespace) |

`metacopy` stays off, so there are **no** `metacopy` xattrs and no
partial-copy-up entries — a file present in the upper is always the full new
content, preserving the current `replay` file-copy logic.

### Redirect replay semantics, precisely

`user.overlay.redirect = "<value>"` on upper dir `D` means: the directory the
user sees at `D` is the lower directory identified by `<value>`, renamed.
Overlayfs redirect values are either **absolute** (relative to the overlay
root, leading `/`) or **relative** (a bare name, relative to `D`'s parent).
Replay of `D` with a redirect value `R`:

1. Resolve `R` to a lower path `L` (absolute → `<target>/<R>`; relative →
   `<parent-of-D>/<R>`).
2. The real deletion of `L`'s old location is encoded **separately** in the
   upper as a whiteout at `L` (overlayfs writes both the redirect on the
   destination and a whiteout at the source). So replay ordering must:
   a. **first** materialize the destination by moving the real lower `L` to
   `D`'s real path, **then** b. apply the upper `D`'s own child changes on
   top, and c. let the source whiteout at `L` be a no-op if `L` was already
   moved (idempotent: if `L` no longer exists, the whiteout removal is
   satisfied).
3. If `L` does not exist in the real lower (e.g. the rename source was itself
   created in-sandbox), the redirect degrades to a plain directory create —
   materialize `D` from the upper only.

**Ordering constraint:** redirects must be replayed in an order where a
source is moved before its own whiteout is processed, and nested
redirects resolve outermost-first. The replay walk therefore does a
**metadata-classification pass** (collect redirects + whiteouts + opaques)
before the mutation pass, rather than the current single-pass recursion. This
is the substantive code change and the main risk.

### Redirect values are untrusted, adversary-controlled input

Critical trust change from the old model. Privileged overlay wrote metadata
to `trusted.overlay.*`, writable only by root, so a redirect value was
trustworthy. Rootless overlay uses `user.overlay.*`, and **`user.*` xattrs
are settable by the file's owner** — i.e. by any process inside the sandbox,
including a tier-3 prompt-injected agent. Such an agent can set an arbitrary
`user.overlay.redirect` on an upper-layer file, forging a value that points
**outside** the protected tree (`../../../home/<user>/.ssh`, an absolute
out-of-tree path, or a value routed through a symlink). If commit's replay
followed it naively, the write would land outside the target — our own commit
code would punch through the containment invariant.

So every redirect value is treated as hostile. In the classification pass,
before any mutation, replay MUST: (1) resolve the value (absolute → overlay
root; relative → the redirected dir's parent); (2) **canonicalize** the
resolved source and destination; (3) verify both are inside the protected
target tree (state roots the only other permitted location); (4) reject —
abort-before-touch, idempotent retry — any redirect that escapes via `..`,
resolves to an out-of-tree absolute path, or passes through a symlink at any
component. A forged redirect can then at most abort the commit; it can never
redirect a real-file write. This is written into the safety-spec invariant
(the flagged delta) and covered by an adversarial test alongside the existing
`trusted.overlay` injection test (not replacing it).

### The fail-closed boundary — unchanged in spirit, restated in form

Today: commit refuses **any** overlay metadata beyond whiteouts and
`trusted.overlay.opaque`. New: commit **replays** the enumerated set —
whiteout, `user.overlay.opaque`, `user.overlay.redirect` — and still
**aborts before touching any real file** on anything outside it (any other
`user.overlay.*` suffix: `metacopy`, `origin`, `nlink`, `uuid`, an unknown
key, or any lingering `trusted.overlay.*`). Idempotent fail-stop retry is
preserved: the classification pass runs to completion (read-only) and aborts
the whole commit before the mutation pass begins, so a rejected layer never
leaves a half-applied tree. The invariant is: *replay exactly the enumerated
encodings; refuse everything else before modifying real files; a refused or
interrupted commit is safely re-runnable.*

### Test migration (deliberate, not an assertion flip)

- `tests/linux.rs::commit_aborts_on_unrecognized_overlay_xattr_and_retry_completes`
  currently injects `trusted.overlay.redirect` and asserts rejection, with a
  comment "as if redirect_dir had [been on]". Under this change a
  `user.overlay.redirect` is a **handled** case. Migrate by: (a) changing the
  *rejection* test to inject a genuinely unrecognized key (e.g.
  `user.overlay.metacopy` or `user.overlay.bogus`) and keep asserting
  abort-before-touch + retry-after-removal; (b) adding a **new** test that a
  real in-sandbox `mv olddir newdir` produces a `user.overlay.redirect` and
  that commit replays it to the correct real end-state (lower `olddir`
  becomes real `newdir` with contents intact).
- The sandbox-scope scenario "Rename inside the sandbox degrades to copy-up
  plus whiteout" (asserting **no** redirect xattr) is inverted by this
  change and is migrated in the spec delta to "rename is encoded as a
  redirect".

---

## 2. Namespace architecture

### Setup sequence (rootless `oops run`)

1. **Launcher** (the `oops run` process, unprivileged): mint the session
   (and optional token, §4). Re-exec a hidden child that will build the
   namespaces — mirror today's `__exec` re-exec pattern.
2. **userns A + mount ns** (`CLONE_NEWUSER | CLONE_NEWNS`): the child creates
   a user namespace and a mount namespace together. Inside A it is uid 0
   (mapped, see below) and holds `CAP_SYS_ADMIN` **over A's own mount ns**.
   Make `/` `MS_REC|MS_PRIVATE`, then mount the overlay over the target with
   `lowerdir=<target>,upperdir=<upper>,workdir=<work>,metacopy=off,userxattr`.
3. **userns B** (`CLONE_NEWUSER`, nested, **no new mount ns**): the child
   forks/execs the wrapped command inside a second, nested user namespace
   that is a *descendant* of A. B holds no `CAP_SYS_ADMIN` over A's mount ns,
   so `umount`/`nsenter` are denied (probed: EINVAL/EPERM). `chdir` into the
   target (now the overlay view) before entering B, so B's cwd is the
   sandboxed tree.
4. The command runs in B. All writes hit the overlay → upper. On exit, the
   child returns the status to the launcher.
5. **Finalize stays with the launcher** (outside B): `oops undo` discards the
   upper (rename session dir to trash, as today); `oops commit` runs the
   §1 replay in the launcher's context. B never had the capability or the
   token to finalize.

The marker-file "point of no return" contract from today's `__exec`
(`marker` written immediately before `exec`, so the parent can tell
setup-failure from command-failure) is preserved across the two-stage setup:
the marker is written after userns B is entered and before the command
execs.

### uid/gid mapping strategy — pick the simplest that needs no /etc/subuid

Use a **single-uid identity map** for userns A: map the invoking uid → uid 0
inside A (`uid_map: 0 <invoking-uid> 1`), same for gid, and write
`setgroups: deny` before the gid map (required for unprivileged gid mapping).
This needs **no `/etc/subuid`/`/etc/subgid` ranges** and no `newuidmap`
setuid helper — it works for any unprivileged user out of the box, which is
the whole point. The tradeoff: only one uid exists inside the sandbox, so a
command that itself tries to `setuid` to another uid fails — acceptable for
`oops run "<cmd>"` (single-user command execution), and documented. userns B
is nested with a trivial map (0 → 0 within A, i.e. the same identity), which
is enough to drop B's authority over A's mounts; B does **not** need its own
uid range either.

subuid ranges (`newuidmap`) are rejected: they require system configuration
per user, reintroduce a setuid helper, and buy nothing `oops run` needs.

### Trait fit — APFS untouched

The `SnapshotBackend` trait is unchanged. The new setup lives entirely inside
the overlayfs backend's `exec` (and its re-exec helpers in `main.rs`). The
APFS backend, the trait signatures, `sandbox_of`, and the session model are
not modified. `changes`/`merge`/`is_stale` signatures are unchanged; only
their overlayfs bodies learn the `user.overlay.*` set.

### `oops run` now vs. groundwork for `oops shell` later

This proposal ships **`oops run`** rootless: the wrapped command is the
single payload in userns B, finalize after it exits. `oops shell` (a
long-lived interactive shell in B, finalize on shell exit) is **not** in
scope, but the architecture is deliberately shaped so it drops in: B already
hosts an arbitrary process, and finalize already lives in the launcher. The
only `oops shell` addition later is keeping B alive interactively and a
launcher-side prompt — no rework of the namespace or redirect design. The
design must not, and does not, assume a single short-lived command in a way
that blocks the interactive case (e.g. no reliance on the command's stdout
being captured, no fixed timeout).

---

## 3. Compatibility matrix & fail-closed fallback

### Requirements

- **Kernel ≥ 5.11** for unprivileged overlay-in-userns. Below → no rootless
  overlay.
- **Unprivileged userns enabled**: Fedora/Arch/openSUSE default yes;
  Debian/Ubuntu gate via `kernel.unprivileged_userns_clone` and, since
  Ubuntu 23.10, `kernel.apparmor_restrict_unprivileged_userns` + an AppArmor
  profile requiring `userns create`.

### Fallback policy (fail closed, never silent)

When userns or unprivileged overlay is unavailable, `run` refuses and prints
an actionable message that (a) names the specific blocker detected (kernel
version; `unprivileged_userns_clone=0`; AppArmor restriction), (b) gives the
exact knob (`sysctl kernel.apparmor_restrict_unprivileged_userns=0`, or
install the shipped oops AppArmor profile), and (c) offers the **explicit
privileged opt-in**. Decision: **keep a privileged path as an explicit
opt-in** (`oops run --privileged` or `OOPS_PRIVILEGED=1`), not the default,
not automatic. Justification: dropping it entirely would strand users on
locked-down kernels (managed Ubuntu fleets) with no path at all, violating
"never silently require root" only by making root un-offerable; keeping it
*explicit* preserves "the user must see which mode they're in." The
privileged path reuses today's root `unshare(CLONE_NEWNS)` code (now behind
the flag) and runs the command **without** userns B — so it is honestly
documented as **tier-1/2 only** (cooperative agent), because a privileged
run hands back `CAP_SYS_ADMIN`. The default rootless path is the tier-3 one.

### What must be verified on real hardware (not Docker-for-Mac)

The probe ran on LinuxKit 6.10 with Docker's seccomp/AppArmor lifted. Bare
metal must confirm: Ubuntu 24.04 with `apparmor_restrict_unprivileged_userns`
**on** (expect fail-closed message) and **off** (expect rootless success);
Debian 12; Fedora latest. This is a task the user or CI must run on real
boxes — see tasks §5.

---

## 4. Session token (spike stage 1) — included as defense-in-depth

Add an optional random `token` to the session record, held by the launcher,
required by `commit`/`undo`, never exported into userns B's environment. On
Linux the namespace already blocks B from finalizing; the token is
belt-and-suspenders (a future namespace regression still needs a secret B
never had) and closes the shell-spike "any process in the dir finalizes"
demo gap. It is cheap and backend-independent. If review prefers it as a
separate `add-session-token` change, it lifts out without touching the
namespace work. Precise guarantee, restated from the spike: on Linux it
hardens a structural boundary; it does **not** by itself make anything
structural — the userns does.

---

## 5. Escape-vector regression suite (the definition of done)

Container CI (privileged container is fine as the *test harness*; the test
asserts the *unprivileged* code path inside it), guarded like the existing
destructive suite. Each is a tier-3 acceptance test:

1. command in userns B: `umount <target>` and `umount -l <target>` both fail
   (EINVAL); the overlay is still mounted afterward.
2. command in userns B: `nsenter --mount=/proc/1/ns/mnt` fails (EPERM).
3. a write by the command lands in the upper layer; the real lower is
   byte-identical (sha) after the overlay is torn down.
4. `oops run` succeeds with **no root** (the harness drops privileges / uses
   an unprivileged uid for the oops invocation even if the container is
   privileged).
5. redirect replay: `mv olddir newdir` in-sandbox → commit yields real
   `newdir` with contents, no `olddir`.
6. fail-closed: an unrecognized `user.overlay.*` key aborts commit before any
   real write, and retry after removal completes.

"Finished" = this suite green on a real supporting kernel, plus the §3 matrix
confirmed on the three distros.

---

## Risks & the long pole

- **Redirect replay (§1)** is the substantive risk: the single-pass replay
  becomes a classify-then-mutate two-pass, and rename/whiteout ordering must
  be exactly right or commit corrupts the real tree. It is isolated into its
  own task group and gets word-by-word spec review.
- **Distro variance (§3)** can only be closed on real hardware.
- Everything else (namespace setup, uid map, token) is mechanical and
  probe-backed.

## Out of scope (backlog, from the confinement spike)

Tier-2 outside-the-tree writes (controlled rootfs); `oops shell`; macOS
Linux-guest; tier-4 (ptrace/fd-passing/kernel exploits). Named here so the
proposal cannot silently absorb them.
