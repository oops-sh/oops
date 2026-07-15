#!/usr/bin/env bash
# confinement spike — Linux rootless-userns confinement probe (definitive).
#
# The oops-shell spike found the Linux boundary is structural against a
# COOPERATIVE agent but escapable by one that runs `umount -l` (today's
# root-required backend gives the inner shell CAP_SYS_ADMIN in its own
# namespace). This probe tests the confinement design and its escape
# vectors, WITHOUT host root:
#
#   userns A  (unshare -U -m -r): owns the mount ns; mounts the overlay
#             OVER the target so the real lower is shadowed in place.
#   userns B  (unshare -U, nested, NO uid map -> uid=nobody): the agent's
#             shell. Descendant of A, so it holds no CAP_SYS_ADMIN over A's
#             mount ns.
#
# Proven results on kernel 6.10 (Docker Desktop LinuxKit VM), run as a
# non-root uid with seccomp+apparmor unconfined and NO --privileged / no
# cap-add — i.e. emulating an ordinary unprivileged Linux user on a distro
# that permits unprivileged userns:
#
#   * unprivileged overlay mount: WORKS — but ONLY with rootless-compatible
#     options. `redirect_dir=off` (which oops uses today) is REJECTED with
#     EPERM for unprivileged mounts; drop it and use `userxattr`.
#   * B `umount -l` / `umount` of A's overlay: BLOCKED (EINVAL).
#   * B `nsenter` into pid1's mount ns: BLOCKED (EPERM).
#   * B write to the target: lands in UPPER (target is the overlay); the
#     real lower is byte-identical after unmount -> the agent never touched
#     the real files. Finalize (commit=replay upper / undo=discard) is the
#     launcher's, in A, unreachable from B.
#
# Reproduce (from the repo root):
#   docker run --rm \
#     --security-opt seccomp=unconfined --security-opt apparmor=unconfined \
#     --tmpfs /scratch:exec,mode=1777 --user 1000:1000 \
#     -v "$PWD/spikes/confinement":/c:ro rust:1.97-slim-trixie \
#     unshare -U -m -r bash /c/userns_escape_probe_linux.sh
#
# NOTE: `--tmpfs /scratch` stands in for a real (non-overlay) filesystem for
# the upper/work layers — Docker's / is overlayfs and cannot host an
# upperdir. A bare-metal user's $HOME already satisfies this.
set -uo pipefail
sec() { printf -- '---- %s ----\n' "$*"; }
SCRATCH="${SCRATCH:-/scratch}"

if [ "${STAGE:-A}" = "A" ]; then
  sec "A: identity (the launcher's privileged-in-ns context)"
  echo "id: $(id)"; echo "uid_map: $(tr -s ' ' </proc/self/uid_map)"

  b="$SCRATCH/confine.$$"; mkdir -p "$b/target" "$b/upper" "$b/work"; chmod 700 "$b"
  echo "human-authored, must survive" > "$b/target/important.txt"
  LSHA=$(sha1sum "$b/target/important.txt" | cut -d' ' -f1)
  export B_DIR="$b" LSHA

  sec "A: option-compat check — redirect_dir=off must be REJECTED unprivileged"
  if mount -t overlay ov -o \
      "lowerdir=$b/target,upperdir=$b/upper,workdir=$b/work,redirect_dir=off,metacopy=off" \
      "$b/target" 2>/dev/null; then
    echo "  UNEXPECTED: redirect_dir=off was accepted (kernel/policy differs here)."
    umount -l "$b/target" 2>/dev/null || true
  else
    echo "  confirmed: redirect_dir=off -> EPERM (oops's current option set is"
    echo "  incompatible with rootless overlay; merge/diff must stop relying on it)."
  fi

  sec "A: mount overlay OVER target with rootless-compatible options"
  if mount -t overlay ov -o \
      "lowerdir=$b/target,upperdir=$b/upper,workdir=$b/work,metacopy=off,userxattr" \
      "$b/target" 2>"$b/mnt.err"; then
    echo "  MOUNT OK (unprivileged). target now shows the overlay: $(ls "$b/target")"
  else
    echo "  MOUNT FAILED: $(cat "$b/mnt.err")"
    echo "  RESULT: no rootless overlay here -> confinement must fail closed."
    exit 3
  fi

  sec "A: run the agent shell in a nested child userns B"
  STAGE=B unshare -U bash "$0"

  sec "A: verify the real files after B ran"
  umount -l "$b/target" 2>/dev/null || true   # drop the overlay to see the real lower
  NSHA=$(sha1sum "$b/target/important.txt" | cut -d' ' -f1)
  echo "upper (the pending, un-committed change set): $(ls "$b/upper" | tr '\n' ' ')"
  if [ "$NSHA" = "$LSHA" ]; then
    echo "REAL LOWER PRISTINE: important.txt byte-identical — B's writes stayed in upper."
  else
    echo "REAL LOWER MODIFIED: escape confirmed."
  fi
  # shellcheck disable=SC1090
  [ -f "$b/b.result" ] && . "$b/b.result"
  echo
  sec "VERDICT (Linux rootless confinement)"
  if [ "${B_FAIL:-1}" = "0" ] && [ "$NSHA" = "$LSHA" ]; then
    echo "CONFINED: agent (child userns B) could not unmount, could not nsenter"
    echo "out, and its writes never reached the real files. Commit/undo authority"
    echo "stays with the launcher in A. The shell-spike umount escape is CLOSED."
  else
    echo "NOT FULLY CONFINED — see the vectors above."
  fi
  exit 0
fi

# ---- Stage B: the confined agent shell -----------------------------------
sec "B: identity (the agent — nested userns, no uid map)"
echo "id: $(id)"
b="$B_DIR"; fail=0

sec "B v1: umount -l the overlay (the shell-spike escape)"
if umount -l "$b/target" 2>"$b/b1.err"; then echo "  ESCAPED"; fail=1
else echo "  BLOCKED: $(cat "$b/b1.err")"; fi

sec "B v2: plain umount"
if umount "$b/target" 2>"$b/b2.err"; then echo "  ESCAPED"; fail=1
else echo "  BLOCKED: $(cat "$b/b2.err")"; fi

sec "B v3: nsenter into pid1's mount ns (host view)"
if nsenter --mount=/proc/1/ns/mnt true 2>"$b/b3.err"; then echo "  ESCAPED"; fail=1
else echo "  BLOCKED: $(cat "$b/b3.err")"; fi

sec "B v4: write via the target (should land in upper, not the real lower)"
rm -f "$b/target/important.txt" 2>/dev/null
echo "unreviewed agent output" > "$b/target/generated.txt" 2>/dev/null
echo "  wrote through the overlay; the real-lower check runs back in A."

echo "B_FAIL=$fail" > "$b/b.result"
exit 0
