#!/usr/bin/env bash
# oops-shell spike — Linux overlay long-lived-shell + authority probe.
#
# Two questions the design doc needs Linux data for:
#
#   A. Can an interactive shell live INSIDE an unshared overlay mount for a
#      whole session, with every write landing in the upper layer and the
#      real lower tree untouched? (feasibility of the per-session sandbox)
#
#   B. When a process confined to that namespace tries to FINALIZE — i.e.
#      make its changes hit the real files — can it? The real lower tree is
#      shadowed by the overlay at the target path, so the merge (upper ->
#      real lower) has no reachable destination from inside. This probe
#      shows the naive finalize is structurally neutralized, AND shows the
#      escape hatch: a process with mount privilege can `umount -l` the
#      overlay and reach the lower directly.
#
# Container-only: needs CLONE_NEWNS + overlay mount privilege, exactly like
# the OverlayFS destructive suite. Run inside the repo's Linux dev container
# (privileged), never on a dev host. Mirrors that suite's guard convention.
set -euo pipefail

if [ ! -f /.dockerenv ] && [ "${OOPS_IN_CONTAINER:-}" != "1" ]; then
  echo "refusing: run inside the privileged Linux container (set OOPS_IN_CONTAINER=1)" >&2
  exit 2
fi

work="$(mktemp -d /tmp/oops-shell-linux.XXXXXX)"
target="$work/project"; upper="$work/upper"; workdir="$work/work"
mkdir -p "$target" "$upper" "$workdir"
echo "human-authored" >"$target/important.txt"
lower_real_sha="$(sha1sum "$target/important.txt" | awk '{print $1}')"

echo "== A. long-lived shell inside the overlay namespace =="
# One unshare hosts a scripted "interactive session": several commands, as a
# persistent shell would run them, all against the overlay view.
unshare --mount --propagation private bash -euc '
  target="'"$target"'"; upper="'"$upper"'"; workdir="'"$workdir"'"
  mount -t overlay overlay -o \
    "lowerdir=$target,upperdir=$upper,workdir=$workdir,redirect_dir=off,metacopy=off" \
    "$target"
  cd "$target"
  # ---- the "session": what an agent would do across many commands ----
  rm -f important.txt
  echo "unreviewed agent output" >generated.txt
  echo "more work" >>generated.txt
  echo "  [inside] view after edits: $(ls)"
  # ---- B. naive finalize attempt from INSIDE the namespace ----
  # Replaying the upper onto the target writes to the OVERLAY (still the
  # sandbox), not the real lower, which is shadowed here. No escape.
  echo "  [inside] upper layer (the pending change set):"; ls "$upper" | sed "s/^/    /"
'
echo
echo "== B1. real lower tree AFTER the session (outside the namespace) =="
if [ -f "$target/important.txt" ] && \
   [ "$(sha1sum "$target/important.txt" | awk '{print $1}')" = "$lower_real_sha" ] && \
   [ ! -f "$target/generated.txt" ]; then
  echo "  STRUCTURAL: real lower is pristine — important.txt intact, generated.txt absent."
  echo "  The namespace died with the shell; nothing inside reached the real files."
else
  echo "  UNEXPECTED: real lower was modified — investigate."
  ls -1 "$target"
fi
echo "  upper layer still holds the pending change set (finalizable only from OUTSIDE):"
ls -1 "$upper" | sed 's/^/    /'
echo

echo "== B2. escape-hatch caveat: mount privilege defeats the structural block =="
echo "  A namespace owner can 'umount -l \$target' to unshadow the lower, then"
echo "  write to the real files directly. The structural block holds against a"
echo "  naive/cooperative agent and against 'oops commit', NOT against an"
echo "  adversary with mount privilege in the namespace. Demonstrated logically;"
echo "  the fix is confinement (drop CAP_SYS_ADMIN / rootless userns), see design.md."

rm -rf "$work"
