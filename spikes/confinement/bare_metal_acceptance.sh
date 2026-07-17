#!/usr/bin/env bash
# Group 6 — bare-metal rootless acceptance for the Linux backend.
#
# Run this INSIDE a real distro VM as a NORMAL (non-root) user. It proves the
# things Docker-on-Mac cannot: that `oops run` is rootless on this distro's
# real kernel + security policy, that the nested-userns command cannot escape,
# that redirect replay and its adversarial refusals hold, and (with sudo) that
# oops fails closed when unprivileged userns is restricted.
#
# Usage (as the VM's normal user, from a checkout of the repo):
#   bash spikes/confinement/bare_metal_acceptance.sh            # non-root suite
#   bash spikes/confinement/bare_metal_acceptance.sh --failclosed  # + sudo knob test
#
# Env:
#   OOPS_BIN   path to the oops binary (default: build with `cargo build --release`)
#
# Exit code is non-zero if any check fails. Nothing outside this script's own
# temp dirs is modified (the failclosed section toggles one sysctl and restores
# it).
set -uo pipefail

FAILCLOSED=0
[ "${1:-}" = "--failclosed" ] && FAILCLOSED=1

pass=0 fail=0
ok()   { printf '  \033[32mPASS\033[0m %s\n' "$1"; pass=$((pass+1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; fail=$((fail+1)); }
hr()   { printf '\n== %s ==\n' "$*"; }

# --------------------------------------------------------------------------
hr "environment"
echo "  user:   $(id -un) (uid $(id -u))"
echo "  kernel: $(uname -r)"
if [ "$(id -u)" = "0" ]; then
  echo "  WARNING: running as root — the 'no root needed' claim is only proven as a normal user."
fi
for k in kernel.unprivileged_userns_clone user.max_user_namespaces \
         kernel.apparmor_restrict_unprivileged_userns; do
  v=$(sysctl -n "$k" 2>/dev/null) && echo "  $k = $v" || echo "  $k = (absent)"
done
command -v setfattr >/dev/null || echo "  NOTE: setfattr (attr pkg) missing — adversarial checks will be skipped."

# --------------------------------------------------------------------------
hr "build / locate oops"
if [ -n "${OOPS_BIN:-}" ] && [ -x "$OOPS_BIN" ]; then
  BIN="$OOPS_BIN"
else
  cargo build --release >/dev/null 2>&1 || { echo "cargo build failed"; exit 2; }
  BIN="$(pwd)/target/release/oops"
fi
echo "  oops: $BIN"
"$BIN" --version || { echo "oops --version failed"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/oops-accept.XXXXXX")"
export XDG_STATE_HOME="$WORK/state"
mkdir -p "$XDG_STATE_HOME"
trap 'rm -rf "$WORK"' EXIT
upper_of() { ls -d "$XDG_STATE_HOME"/oops/sessions/*/upper 2>/dev/null | head -1; }

# --------------------------------------------------------------------------
hr "1. rootless run leaves the real tree untouched (interception)"
d="$WORK/t1"; mkdir -p "$d"; ( cd "$d"; echo original > important.txt
  "$BIN" run 'echo hi > new.txt; rm important.txt' >/dev/null 2>&1 )
if [ -f "$d/important.txt" ] && [ ! -e "$d/new.txt" ]; then ok "real tree untouched during run"; else bad "real tree changed during run"; fi
diff_out=$( cd "$d"; "$BIN" diff --porcelain 2>/dev/null )
[ "$diff_out" = "D important.txt
A new.txt" ] && ok "diff --porcelain correct" || bad "diff wrong: [$diff_out]"
( cd "$d"; "$BIN" undo >/dev/null 2>&1 )
{ [ -f "$d/important.txt" ] && [ ! -e "$d/new.txt" ]; } && ok "undo restored the tree" || bad "undo did not restore"

hr "2. commit — simple create"
d="$WORK/t2"; mkdir -p "$d"; ( cd "$d"; "$BIN" run 'echo c > c.txt' >/dev/null 2>&1; "$BIN" commit >/dev/null 2>&1 )
[ "$(cat "$d/c.txt" 2>/dev/null)" = "c" ] && ok "created file committed" || bad "commit-create failed"

hr "3. commit — directory rename (redirect replay)"
d="$WORK/t3"; mkdir -p "$d/olddir"; echo hello > "$d/olddir/f.txt"
( cd "$d"; "$BIN" run 'mv olddir newdir && echo add > newdir/g.txt' >/dev/null 2>&1; "$BIN" commit >/dev/null 2>&1 )
if [ ! -e "$d/olddir" ] && [ "$(cat "$d/newdir/f.txt" 2>/dev/null)" = "hello" ] \
   && [ "$(cat "$d/newdir/g.txt" 2>/dev/null)" = "add" ]; then ok "redirect rename replayed"; else bad "redirect replay wrong"; fi

hr "4. escape vectors — command in userns B cannot umount / nsenter / reach launcher"
d="$WORK/t4"; mkdir -p "$d"; echo real > "$d/keep.txt"
( cd "$d"; "$BIN" run 'umount -l . 2>umount.err || true; \
   nsenter --mount=/proc/1/ns/mnt true 2>ns1.err || true; \
   nsenter --mount=/proc/$PPID/ns/mnt true 2>nsL.err || true; \
   echo hacked > keep.txt' >/dev/null 2>&1 )
U="$(upper_of)"
[ -s "$U/umount.err" ] && ok "umount blocked" || bad "umount NOT blocked"
[ -s "$U/ns1.err" ]    && ok "nsenter pid1 blocked" || bad "nsenter pid1 NOT blocked"
[ -s "$U/nsL.err" ]    && ok "nsenter launcher blocked" || bad "nsenter launcher NOT blocked"
[ "$(cat "$d/keep.txt")" = "real" ] && ok "real file untouched (write stayed in upper)" || bad "ESCAPE: real file changed"
( cd "$d"; "$BIN" undo >/dev/null 2>&1 )

if command -v setfattr >/dev/null; then
  hr "5. adversarial — forged redirect escaping via .. is refused"
  d="$WORK/t5"; mkdir -p "$d"; sec="$WORK/secret5"; mkdir -p "$sec"; echo SECRET > "$sec/loot"; shaB=$(sha1sum "$sec/loot")
  ( cd "$d"; "$BIN" run 'mkdir dd' >/dev/null 2>&1 )
  setfattr -n user.overlay.redirect -v "../..$sec" "$(upper_of)/dd" 2>/dev/null
  out=$( cd "$d"; "$BIN" commit 2>&1 ); rc=$?
  { [ $rc -ne 0 ] && [ "$(sha1sum "$sec/loot")" = "$shaB" ]; } && ok "..-escape refused, sentinel intact" || bad "..-escape NOT refused ($out)"
  ( cd "$d"; "$BIN" undo >/dev/null 2>&1 )

  hr "6. adversarial — redirect through a symlink is refused at mutate time"
  d="$WORK/t6"; mkdir -p "$d"; sec="$WORK/secret6"; mkdir -p "$sec"; echo SECRET > "$sec/loot"; shaB=$(sha1sum "$sec/loot")
  ln -s "$sec" "$d/evil"
  ( cd "$d"; "$BIN" run 'mkdir dd' >/dev/null 2>&1 )
  setfattr -n user.overlay.redirect -v "evil/loot" "$(upper_of)/dd" 2>/dev/null
  out=$( cd "$d"; "$BIN" commit 2>&1 ); rc=$?
  { [ $rc -ne 0 ] && [ "$(sha1sum "$sec/loot")" = "$shaB" ]; } && ok "symlink-relay refused, sentinel intact" || bad "symlink-relay NOT refused ($out)"
  ( cd "$d"; "$BIN" undo >/dev/null 2>&1 )
fi

# --------------------------------------------------------------------------
if [ "$FAILCLOSED" = "1" ]; then
  hr "7. fail-closed — restricted unprivileged userns refuses (needs sudo)"
  knob=kernel.apparmor_restrict_unprivileged_userns
  if ! sysctl -n "$knob" >/dev/null 2>&1; then
    echo "  $knob absent on this distro — skipping (this is the Debian/Fedora case: no AppArmor userns restriction)."
  else
    orig=$(sysctl -n "$knob")
    sudo sysctl -w "$knob=1" >/dev/null
    d="$WORK/t7"; mkdir -p "$d"
    out=$( cd "$d"; "$BIN" run 'touch fc_evidence' 2>&1 ); rc=$?
    if [ $rc -ne 0 ] && [ ! -e "$d/fc_evidence" ]; then
      ok "refused when userns restricted (command never ran)"
      echo "$out" | grep -qiE 'user namespace|apparmor|OOPS_PRIVILEGED' && ok "message names the knob / opt-in" || bad "message not actionable: $out"
    else
      bad "did NOT fail closed under restriction (rc=$rc)"
    fi
    sudo sysctl -w "$knob=$orig" >/dev/null
    echo "  restored $knob=$orig"
  fi
fi

# --------------------------------------------------------------------------
hr "summary"
echo "  PASS: $pass   FAIL: $fail   ($(uname -r), $(id -un))"
[ "$fail" -eq 0 ] || exit 1
