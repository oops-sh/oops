# Tasks: add-rootless-linux

Task groups are ordered so the **redirect_dir migration (group 2) gates
everything that touches commit**. Groups 1 and 2 can proceed in parallel;
group 3 (nested userns) depends on 1; group 5 (acceptance) depends on all.

## 1. Rootless namespace setup (no commit-path changes)

- [ ] 1.1 Replace the overlayfs mount options with `metacopy=off,userxattr`
  (drop `redirect_dir=off`); confirm the mount succeeds unprivileged.
- [ ] 1.2 Build userns A + mount ns in the `__exec` child: `CLONE_NEWUSER |
  CLONE_NEWNS`, single-uid identity map (`uid_map: 0 <uid> 1`,
  `setgroups: deny`, `gid_map: 0 <gid> 1`), `MS_REC|MS_PRIVATE /`, then the
  overlay mount over the target. No `/etc/subuid` dependency.
- [ ] 1.3 Preserve the fail-closed setup contract and the marker "point of no
  return" across the new two-stage setup.
- [ ] 1.4 Fallback detection: probe userns + unprivileged-overlay
  availability; on absence, refuse with the actionable message (name the
  kernel/sysctl/AppArmor blocker) and the explicit `--privileged`/
  `OOPS_PRIVILEGED` opt-in. Never silently degrade; never silently require
  root.
- [ ] 1.5 Keep the old root `unshare(CLONE_NEWNS)` path behind the explicit
  privileged opt-in only; document it as tier-1/2 (no userns B).

## 2. The redirect_dir migration — commit replay (LONG POLE, gates commit)

- [ ] 2.1 Switch overlay xattr reads from `trusted.overlay.*` to
  `user.overlay.*` in `changes()`, `validate_upper()`, `replay()`
  (opaque, and the new redirect; keep whiteout char-device detection).
- [ ] 2.2 Restructure `replay` into a **classify pass** (collect whiteouts,
  `user.overlay.opaque`, `user.overlay.redirect`) then a **mutate pass**, so
  the whole layer is validated read-only before any real write.
- [ ] 2.3 Implement redirect replay per design §1: resolve absolute/relative
  redirect values; move the real lower source into the destination name;
  apply the destination's own child changes; make the paired source-whiteout
  idempotent; handle the "source created in-sandbox → plain create" degrade;
  enforce outermost-first / source-before-whiteout ordering.
- [ ] 2.4 Extend the recognized set to exactly {whiteout,
  `user.overlay.opaque`, `user.overlay.redirect`}; abort fail-closed on any
  other `user.overlay.*` key, any `trusted.overlay.*`, or unknown metadata —
  before the mutate pass begins. Preserve idempotent fail-stop retry.
- [ ] 2.5 Update `changes()` diff classification for redirect (a renamed dir
  shows as a deletion at the old path + addition at the new, matching the
  porcelain contract) — verify `diff --porcelain` bytes are unchanged for
  equivalent user-visible changes.

## 3. Nested-userns command execution (delivers tier 3)

- [ ] 3.1 After the overlay mount in A, enter a nested child userns B
  (`CLONE_NEWUSER`, no new mount ns) and exec the wrapped command there;
  `chdir` into the overlay target first.
- [ ] 3.2 Confirm finalize (undo/commit) runs in the launcher, never in B;
  wire the marker/status handoff from B back to the launcher.

## 4. Session token (defense-in-depth; liftable to its own change)

- [ ] 4.1 Add an optional random `token` to the session record; launcher
  holds it; `commit`/`undo` require it; never export it into B's env.
- [ ] 4.2 Test: a process in the target dir without the token cannot
  finalize; the launcher can.

## 5. Escape-vector regression suite (definition of done)

- [ ] 5.1 umount / umount -l from userns B blocked (EINVAL), overlay still
  mounted.
- [ ] 5.2 nsenter into pid1 mount ns from B blocked (EPERM).
- [ ] 5.3 write in B lands in upper; real lower byte-identical (sha) after
  teardown.
- [ ] 5.4 `oops run` succeeds with no root on a supporting kernel.
- [ ] 5.5 redirect replay: in-sandbox `mv olddir newdir` → commit yields real
  `newdir` (contents intact), no `olddir`.
- [ ] 5.6 fail-closed: unrecognized `user.overlay.*` key aborts commit before
  any real write; retry after removal completes.
- [ ] 5.7 **Migrate** `commit_aborts_on_unrecognized_overlay_xattr_and_retry
  _completes`: inject a genuinely-unknown key (not `redirect`, which is now
  handled); keep the abort+retry assertions.

## 6. Bare-metal compatibility verification (USER/CI must run on real hosts)

- [ ] 6.1 Ubuntu 24.04, `apparmor_restrict_unprivileged_userns=1`: expect
  fail-closed message. **Needs a real Ubuntu box/VM — not Docker-for-Mac.**
- [ ] 6.2 Ubuntu 24.04 with the restriction off (or the shipped AppArmor
  profile installed): expect rootless success + suite §5 green.
- [ ] 6.3 Debian 12 and Fedora latest: rootless success + suite §5 green.
- [ ] 6.4 Record the confirmed minimum kernel and the exact distro knobs in
  the README guarantee matrix.

## 7. Spec + docs

- [ ] 7.1 Land the `sandbox` spec delta (rootless architecture, nested-userns
  boundary, overlay-encoding set, privileged-fallback policy).
- [ ] 7.2 Land the `safety` spec delta (redirect-metadata invariant rewrite —
  **word-by-word review**; Linux tier-3 statement; tier-4 out of scope).
- [ ] 7.3 README guarantee matrix: "no root required (kernel ≥ 5.11; see
  fallback)", tier-3 on Linux, tier-4 explicitly out of scope.

## 8. Wrap-up

- [ ] 8.1 Full suite green: macOS unit + apfs, Linux container incl. the new
  escape-vector suite; `openspec validate --strict`.
- [ ] 8.2 Bare-metal matrix (§6) confirmed by the user/CI.
