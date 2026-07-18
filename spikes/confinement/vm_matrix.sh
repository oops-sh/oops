#!/usr/bin/env bash
# Group 6 orchestrator — build the VM matrix, run the acceptance suite in each,
# collect logs, and STOP at a dry-run (never publishes).
#
# Run on the macOS host with Lima installed (`brew install lima`). Works on
# aarch64 or x86_64. For each distro it: creates a fresh VM, installs rust
# (rustup) + attr + git, clones the repo, and runs
# spikes/confinement/bare_metal_acceptance.sh as the VM's normal user.
#
#   bash spikes/confinement/vm_matrix.sh
#
# Env: OOPS_REPO (default https://github.com/oops-sh/oops), OOPS_BRANCH (main).
#
# NOTE: authored on a host without Lima; `bash -n` clean but not executed here.
# It is idempotent (deletes and recreates each named VM).
set -uo pipefail

REPO="${OOPS_REPO:-https://github.com/oops-sh/oops}"
BRANCH="${OOPS_BRANCH:-main}"
OUT="$(mktemp -d)/oops-vm-logs"; mkdir -p "$OUT"
echo "logs -> $OUT"

command -v limactl >/dev/null || { echo "limactl not found — brew install lima"; exit 2; }

# distro name | Lima template | failclosed(1=Ubuntu: relax knob + test refusal)
MATRIX=(
  "oops-ubuntu|ubuntu-24.04|1"
  "oops-debian|debian-12|0"
  "oops-fedora|fedora|0"
)

# The provisioning + acceptance run, executed inside each guest. Args:
#   $1 repo  $2 branch  $3 failclosed
guest_script() {
cat <<'GUEST'
set -u
REPO="$1"; BRANCH="$2"; FC="$3"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y >/dev/null 2>&1
fi
. "$HOME/.cargo/env" 2>/dev/null || true
# C toolchain is required to link the binary (rustup ships no linker), plus
# attr (setfattr) and git. Do NOT swallow install errors.
if command -v apt-get >/dev/null 2>&1; then
  sudo apt-get -qq update
  sudo apt-get -qq install -y build-essential attr git || echo "WARN: apt install failed (see above)"
elif command -v dnf >/dev/null 2>&1; then
  sudo dnf -q install -y gcc attr git || echo "WARN: dnf install failed (see above)"
fi
command -v cc >/dev/null || command -v gcc >/dev/null || echo "WARN: still no C compiler after install"
rm -rf ~/oops
git clone -q --branch "$BRANCH" "$REPO" ~/oops 2>/dev/null || git clone -q "$REPO" ~/oops
cd ~/oops || { echo "clone failed"; exit 3; }
# Ubuntu: relax the AppArmor knob so the rootless suite can run; the
# --failclosed section then re-tests refusal and restores it.
FCFLAG=""
if [ "$FC" = "1" ]; then
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0 >/dev/null 2>&1
  FCFLAG="--failclosed"
fi
echo "=== $(. /etc/os-release 2>/dev/null; echo "$PRETTY_NAME") | kernel $(uname -r) | user $(id -un) ==="
bash spikes/confinement/bare_metal_acceptance.sh $FCFLAG
GUEST
}

overall=0
for row in "${MATRIX[@]}"; do
  IFS='|' read -r name template fc <<<"$row"
  echo; echo "########## $name ($template) ##########"
  # Reuse an existing VM (just ensure it is running); only create when absent.
  # Set OOPS_VM_RECREATE=1 to force a clean rebuild.
  if [ "${OOPS_VM_RECREATE:-0}" = "1" ]; then
    limactl delete -f "$name" >/dev/null 2>&1 || true
  fi
  if limactl list -q 2>/dev/null | grep -qx "$name"; then
    echo "reusing existing VM $name"
    limactl start "$name" >/dev/null 2>&1 || true
  elif ! limactl start --name="$name" --tty=false "template://$template" >/dev/null 2>&1; then
    echo "$name: VM start FAILED"; overall=1; continue
  fi
  guest_script | limactl shell "$name" bash -s -- "$REPO" "$BRANCH" "$fc" 2>&1 | tee "$OUT/$name.log"
  if grep -q "FAIL: 0" "$OUT/$name.log"; then
    echo "$name: GREEN"
  else
    echo "$name: NOT GREEN (see $OUT/$name.log)"; overall=1
  fi
done

echo; echo "================= matrix summary ================="
for row in "${MATRIX[@]}"; do
  IFS='|' read -r name _ _ <<<"$row"
  line=$(grep -E '^  PASS:' "$OUT/$name.log" 2>/dev/null | tail -1)
  printf '  %-14s %s\n' "$name" "${line:-<no summary — run failed>}"
done

echo
if [ "$overall" -eq 0 ]; then
  cat <<EOF
All three VMs GREEN. DRY-RUN — nothing published.
To release v0.3.0 yourself:
  git tag v0.3.0 && git push origin v0.3.0
  cargo publish            # crate oops-sh
  gh release create v0.3.0 --title v0.3.0 --notes "APFS + rootless Linux (tier-3)"
Leave the VMs up (this script recreates them) or: limactl delete -f oops-ubuntu oops-debian oops-fedora
EOF
else
  echo "Matrix NOT fully green — do not tag. Inspect $OUT/*.log"
  exit 1
fi
