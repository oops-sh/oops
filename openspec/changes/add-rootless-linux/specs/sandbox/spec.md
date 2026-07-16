# sandbox Specification — delta for add-rootless-linux

## MODIFIED Requirements

### Requirement: Writes are redirected to the upper layer
For the **OverlayFS (interception) backend**, `run` MUST execute the
command in a context where the target directory is an OverlayFS mount whose
lower layer is the real directory. On a supporting kernel this context is an
**unprivileged user namespace + mount namespace** created without root; the
wrapped command MUST run in a **nested child user namespace** that holds no
capability over the mount namespace's mounts (see "Rootless execution and the
nested-userns authority boundary"). All creates, modifications, and deletions
performed by the command MUST land in the upper layer; the lower layer MUST
remain byte-identical to its pre-run state. (The snapshot-restore backend has
no equivalent guarantee — see "APFS snapshot-restore backend".)

#### Scenario: Command creates a file
- **WHEN** `oops run "echo hi > new.txt"` completes in directory D on the overlayfs backend
- **THEN** `new.txt` exists in the upper layer and does not exist in the real D

#### Scenario: Command deletes a tree
- **WHEN** `oops run "rm -rf sub/"` completes in directory D containing `sub/` on the overlayfs backend
- **THEN** the upper layer contains a whiteout for `sub` and the real `D/sub` still exists with identical contents

### Requirement: Sandbox scope
The sandbox covers exactly one directory tree: the working directory where
`oops run` was invoked. The overlay MUST be mounted rootless with
`metacopy=off` and `userxattr` (so overlay metadata lives in the
`user.overlay.*` namespace an unprivileged mount can write). `redirect_dir`
MUST NOT be forced off — unprivileged overlay rejects that option — so the
upper layer's special encodings are exactly: whiteouts (character device
0:0), opaque-directory xattrs (`user.overlay.opaque`), and directory-rename
redirect xattrs (`user.overlay.redirect`). `metacopy` being off guarantees no
partial-copy-up entries: a file present in the upper layer is always its full
new content. Diff and commit-replay MUST handle exactly this encoding set and
no other (see "Commit partial-failure semantics").

#### Scenario: Rename inside the sandbox is encoded as a redirect
- **WHEN** `oops run "mv a b"` completes in a sandboxed directory where `a` is a directory
- **THEN** the upper layer encodes the rename with a `user.overlay.redirect` xattr on `b` and a whiteout for `a`, and `oops commit` replays it so the real directory afterwards contains `b` with `a`'s original contents and no `a`

### Requirement: Commit partial-failure semantics
For the **OverlayFS backend**, `commit` is a fail-stop, idempotent replay.
Before modifying the real tree, commit MUST run a read-only classification
pass over the whole upper layer and abort — touching no real path — if it
finds any overlay metadata outside the recognized set {whiteout,
`user.overlay.opaque`, `user.overlay.redirect`}; any other `user.overlay.*`
key, any `trusted.overlay.*` xattr, or unknown metadata MUST cause this
abort. On a recognized layer, commit then applies the mutation pass; on the
first error it MUST stop, report the failing path and applied/remaining
counts, exit non-zero, and preserve the session record and upper layer so
that re-running `commit` completes the merge. (The APFS backend's commit
performs no replay — see "Snapshot-restore commit".)

#### Scenario: Commit fails midway
- **WHEN** `oops commit` fails partway through the replay (e.g. a permission error on one path)
- **THEN** oops exits non-zero naming the failing path, the session and upper layer remain intact, and a subsequent `oops commit` (after fixing the cause) completes the merge with the same end state as a single successful commit

#### Scenario: Recognized redirect metadata is replayed
- **WHEN** the upper layer contains a `user.overlay.redirect` produced by an in-sandbox directory rename and `oops commit` runs
- **THEN** commit canonicalizes and validates the redirect's resolved source and destination as inside the protected tree, then replays it to the correct real end-state (the lower source becomes the renamed destination with contents intact) rather than aborting

#### Scenario: Forged out-of-tree redirect is refused
- **WHEN** the upper layer contains a `user.overlay.redirect` (adversary-writable, since `user.*` xattrs are owner-settable) whose value resolves outside the protected tree and `oops commit` runs
- **THEN** commit rejects it in the read-only classification pass, modifies no real path, and exits non-zero — a forged redirect can at most abort the commit, never redirect a write outside the protected tree (see safety's "Commit real-file modification boundary")

#### Scenario: Unrecognized overlay metadata
- **WHEN** the upper layer contains an overlay xattr outside the recognized set
- **THEN** commit aborts during the read-only classification pass before modifying any real path and exits non-zero explaining what it found

## ADDED Requirements

### Requirement: Rootless execution and the nested-userns authority boundary
On a supporting Linux kernel, `oops run` MUST require no root privileges. The
launcher MUST create the overlay mount inside an unprivileged user namespace
("A") that it owns, and MUST execute the wrapped command inside a **nested
child user namespace** ("B") that is a descendant of A and therefore holds no
`CAP_SYS_ADMIN` over A's mount namespace. As a result, a process in B MUST
NOT be able to unmount the sandbox (`umount`/`umount -l` fail) nor enter any
ancestor mount namespace — neither pid 1's nor the launcher's (`nsenter`/
`setns` fail). The command's writes **within the sandboxed target subtree**
MUST land in the upper layer with the real lower tree left byte-identical;
writes outside that subtree are not part of this boundary and are governed by
the honest guarantee boundary (see safety's "Honest guarantee boundary" — the
undo guarantee covers only the target tree). The finalize actions (`undo`,
`commit`) MUST run in the launcher's context, never inside B; B MUST NOT
possess the means to finalize the session. This is the Linux tier-3 authority
boundary defined in the confinement spike.

#### Scenario: Command in the nested userns cannot dismantle the sandbox
- **WHEN** the wrapped command (running in userns B) attempts `umount -l` of the target, or `nsenter`/`setns` into pid 1's mount namespace or the launcher's mount namespace
- **THEN** all fail, the overlay stays mounted, and the command's writes within the target subtree remain in the upper layer

#### Scenario: Rootless run needs no root
- **WHEN** `oops run "touch x"` is invoked by an unprivileged user on a kernel with unprivileged user namespaces and unprivileged overlayfs available
- **THEN** the sandbox is created and the command runs without any root privilege or `--privileged` container

### Requirement: Privileged fallback is explicit and fail-closed
When unprivileged user namespaces or unprivileged overlayfs are unavailable
(kernel older than the supported minimum, or a distro restriction such as a
disabled `unprivileged_userns_clone` sysctl or an AppArmor
`apparmor_restrict_unprivileged_userns` profile), `oops run` MUST fail closed
with a message that names the detected blocker and the specific knob to
change, and MUST NOT run the command unsandboxed nor silently require root. A
privileged mode MAY be offered only as an **explicit** opt-in (a flag or
environment variable); when used it runs without the nested userns and MUST
be documented as providing only the cooperative-agent (tier-1/2) guarantee,
not the tier-3 boundary.

#### Scenario: Unprivileged namespaces unavailable
- **WHEN** `oops run` is invoked where unprivileged user namespaces are restricted
- **THEN** the command is not executed, oops exits non-zero naming the blocker and the knob to change, and mentions the explicit privileged opt-in — it does not silently run unsandboxed or silently require root

#### Scenario: Explicit privileged opt-in is honestly scoped
- **WHEN** the user invokes the explicit privileged mode
- **THEN** the run proceeds without the nested userns and oops documents that this mode provides only the tier-1/2 guarantee
