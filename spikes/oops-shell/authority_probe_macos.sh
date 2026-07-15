#!/usr/bin/env bash
# oops-shell spike — macOS authority-boundary probe.
#
# Question: on the snapshot-restore backend, is the commit/undo decision
# reachable by a process that merely SITS IN the sandboxed target, or does
# finalizing require a capability that process lacks?
#
# oops identifies the pending session by the caller's target directory
# (canonical cwd, then $PWD) — never by a token the launcher holds. This
# probe demonstrates the consequence with the SHIPPED binary: a *second*,
# independent process whose cwd is the target can finalize the session the
# first one created, with no authority check beyond "I am in this dir".
#
# That is exactly the position an agent running inside an `oops shell` would
# be in on macOS. If this probe commits successfully from the second
# process, the macOS authority boundary is conventional, not structural.
#
# Safety: self-created temp target + temp XDG_STATE_HOME (never the real
# state root). Requires an APFS volume (the macOS default) and `oops` 0.2.0+
# on PATH.
set -euo pipefail

OOPS="${OOPS:-oops}"
work="$(mktemp -d "${TMPDIR:-/tmp}/oops-shell-probe.XXXXXX")"
export XDG_STATE_HOME="$work/state"
target="$work/project"
mkdir -p "$XDG_STATE_HOME" "$target"
echo "original, human-authored content" >"$target/important.txt"

echo "== setup =="
echo "target:         $target"
echo "state root:     $XDG_STATE_HOME"
echo "seed file sha:  $(shasum "$target/important.txt" | awk '{print $1}')"
echo

echo "== process #1 (the 'agent inside the shell'): run a destructive command =="
( cd "$target" && "$OOPS" run 'rm -f important.txt && echo "agent output, unreviewed" >generated.txt' ) || true
echo "  real tree now holds the change (snapshot-restore runs on real files):"
ls -1 "$target"
echo

echo "== process #2 (independent: no handle from process #1, only the cwd) =="
echo "  It was never told a session id. It just runs 'oops commit' in the dir."
( cd "$target" && "$OOPS" commit )
echo

echo "== result =="
if [ -f "$target/important.txt" ]; then
  echo "RESTORED — commit did not persist (unexpected on snapshot-restore)."
else
  echo "COMMITTED by process #2 — the unreviewed change is now permanent."
  echo "  important.txt is gone; generated.txt persists:"
  ls -1 "$target"
fi
echo
echo "Interpretation: finalize authority == being in the target dir. No"
echo "capability, token, or namespace gates it. On macOS an in-shell agent"
echo "holds exactly this position, so the boundary is CONVENTIONAL."

rm -rf "$work"
