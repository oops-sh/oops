# safety Specification — delta for add-rootless-linux

## ADDED Requirements

### Requirement: Commit real-file modification boundary
`oops commit` on the OverlayFS backend is the only operation that applies
sandboxed changes to real files, and it MUST modify real paths only as the
faithful replay of a recognized upper-layer encoding. The recognized set is
exactly: whiteouts (a deletion of the corresponding lower path),
`user.overlay.opaque` (a wholesale directory replacement), and
`user.overlay.redirect` (a directory rename, replayed by moving the recorded
lower source to the destination name and applying the destination's own
changes on top). Commit MUST determine the entire set of modifications in a
read-only classification pass over the upper layer **before** modifying any
real path, and MUST abort — modifying nothing — if the upper layer contains
any metadata outside the recognized set (any other `user.overlay.*` key, any
`trusted.overlay.*` xattr, or unknown metadata). Regardless of upper-layer
corruption or unexpected metadata, commit MUST NOT create, delete, truncate,
rewrite, or rename any real path except as the replay of a recognized
encoding, and an aborted or interrupted commit MUST be safely re-runnable to
the same end state (idempotent fail-stop). This is the merge-side companion
to undo containment: undo restores the target and touches only state roots;
commit modifies the target only per the recognized encodings and aborts
before touching real files otherwise.

Under the rootless model the upper layer's `user.overlay.*` xattrs are
**untrusted, adversary-controlled input**: user-namespace overlay stores
metadata in the `user.*` xattr namespace, which the file's owner — i.e. any
process inside the sandbox, including a prompt-injected (tier-3) agent — can
set to arbitrary values on upper-layer files. This is a change in trust from
the old privileged model, where `trusted.overlay.*` could only be written by
root and the metadata was trustworthy. Commit therefore MUST treat every
`user.overlay.redirect` value as hostile: before any replay, it MUST resolve
the value (absolute values relative to the overlay root, relative values
relative to the redirected directory's parent), **canonicalize** the
resulting source and destination paths, and verify that both remain inside
the protected target tree (with the state roots as the only other permitted
location). A redirect whose resolved path escapes the protected tree via `..`
traversal, resolves to an out-of-tree absolute path, or passes through a
symlink at any component MUST cause commit to abort before touching any real
path, under the same fail-closed, idempotent-retry semantics as unrecognized
metadata. A forged redirect can therefore never make commit write outside the
protected tree — it can at most abort the commit.

#### Scenario: Recognized encodings are replayed, everything else aborts before touching real files
- **WHEN** `oops commit` runs against an upper layer containing whiteouts, `user.overlay.opaque`, and `user.overlay.redirect` entries
- **THEN** commit replays exactly those to the real tree; and **WHEN** any other overlay metadata is present, commit aborts during the read-only classification pass having modified no real path, and exits non-zero explaining what it found

#### Scenario: Forged redirect pointing outside the protected tree is refused
- **WHEN** a process inside the sandbox sets a `user.overlay.redirect` whose value resolves outside the protected target tree (an out-of-tree absolute path, a `..` traversal such as `../../../home/<user>/.ssh`, or a value passing through a symlink) and `oops commit` runs
- **THEN** commit rejects it during the read-only classification pass, modifies no real path (the out-of-tree target is byte-identical afterward), exits non-zero explaining the containment violation, and a subsequent commit after the offending xattr is removed completes normally

#### Scenario: Interrupted commit is safely re-runnable
- **WHEN** a commit replay is interrupted partway (error or kill) after modifying some real paths
- **THEN** re-running `oops commit` completes the merge with the same end state as a single successful commit, and no real path is left in a state outside the recognized replay

### Requirement: Linux confinement threat tier
oops's confinement guarantees MUST be stated against an explicit adversary
model. On the OverlayFS backend running rootless (unprivileged user
namespaces), oops targets **tier 3 (a prompt-injected agent that actively
attempts documented escapes)**: an agent running as the wrapped command in
the nested child user namespace cannot dismantle the sandbox, escape the
mount namespace, reach the real files, or finalize the session — the commit
and undo authority remain with the launcher. A **tier-4 adversary** (running
code that engineers a kernel or hypervisor escape, ptraces the launcher, or
smuggles file descriptors out of band) is **explicitly out of scope** and
requires a hardware-isolated virtual machine the user controls; oops MUST NOT
claim protection against it. User-facing documentation MUST state both the
tier-3 target and the tier-4 exclusion honestly.

#### Scenario: Guarantee is stated with its boundary
- **WHEN** a user reads the Linux backend's guarantee documentation
- **THEN** it states that rootless oops confines a prompt-injected agent (tier 3) and that a tier-4 adversary engineering a kernel/hypervisor escape is out of scope
