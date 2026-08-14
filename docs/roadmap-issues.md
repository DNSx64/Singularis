# Singularis Issue Backlog from the Roadmap

Status: 2026-08-14

GitHub status: The entries were created in DNSx64/Singularis as issues #1 to #12.

## Issue 1: ADR 0002 - Define the device grant format and signature model

Labels: `adr`, `security`, `protocol`, `blocker`

Description:
- Define the format for signed device grants.
- Specify version, expiration, revocation, and audit fields.
- Describe how a client verifies validity and authenticity.

Acceptance criteria:
- The device grant is clearly versioned.
- Signature verification is testable and covered by negative tests.
- No silent downgrade or unnoticed revocation is possible.

## Issue 2: ADR 0005 - Define the binary event format and versioning strategy

Labels: `adr`, `protocol`, `blocker`

Description:
- Define the canonical event envelope.
- Specify header, schema version, sequence reference, and replay fields.
- Describe forward and backward compatibility.

Acceptance criteria:
- Parser and serializer are deterministic.
- Positive and negative test cases exist.
- Invalid or tampered events are rejected.

## Issue 3: ADR 0003 - Specify event spool, replication, and deletion window

Labels: `adr`, `storage`, `ttl`, `blocker`

Description:
- Specify accepted_at, expires_at, and replication rules.
- Define the measurable deletion window and alert thresholds.
- Define restart, worker, and clock drift behavior.

Acceptance criteria:
- TTL does not extend through restarts or failure paths.
- Deletion latency is measurable and alertable.
- Reproducible robustness tests exist.

## Issue 4: ADR 0011 - Define vault export format, KDF, and streaming AEAD

Labels: `adr`, `vault`, `security`, `blocker`

Description:
- Define the v1 export container format.
- Specify the KDF, manifest, and chunk authentication.
- Standardize error behavior for wrong passwords and tampering.

Acceptance criteria:
- Export and import are reproducibly testable.
- Wrong passwords and tampering return generic errors.
- Tampering cases are covered by tests.

## Issue 5: ADR 0007 - Define the secure update path and release key management

Labels: `adr`, `release`, `security`, `blocker`

Description:
- Define the signature chain, key roles, and rotation.
- Specify the secure update path and provenance verification.
- Describe the release gate process.

Acceptance criteria:
- Reproducible builds and signature verification are defined.
- Releases are blocked for unexplained binary differences.
- The update path is documented and testable.

## Issue 6: Decide the Linux reference release for SQLCipher and keyring

Labels: `desktop`, `vault`, `platform`, `release`

Description:
- Decide the reference path for SQLCipher packaging.
- Specify keyring storage for Linux.
- Document follow-up decisions for other platforms.

Acceptance criteria:
- The reference path is documented.
- Packaging and key storage are clearly separated.
- No open implementation gaps remain in the reference path.

## Issue 7: Implement the device pairing baseline flow

Labels: `feature`, `multi-device`, `security`

Description:
- Implement QR-based enrollment and compare-code verification.
- Bind signature chain verification and revocation to the flow.
- Ensure that only authorized devices can join.

Acceptance criteria:
- Two users with two devices each are stable in testing.
- Unauthorized devices are rejected.
- Revocation triggers an MLS epoch change.

## Issue 8: Implement the MVP baseline for roles and channel permissions

Labels: `feature`, `permissions`, `server`

Description:
- Implement Owner, Admin, Moderator, Member, and Guest.
- Enforce permissions on the server side.
- Make channel-level permissions testable.

Acceptance criteria:
- Role-violating events are rejected.
- Permissions are consistent server-side and client-side.
- A minimal roles API exists.

## Issue 9: Implement the invitation flow and recovery baseline

Labels: `feature`, `recovery`, `security`

Description:
- Implement invitations with expiration and single-use behavior.
- Implement recovery export/import as an end-to-end flow.
- Bind recovery to identity instead of the server.

Acceptance criteria:
- Recovery works without server key access.
- Invitations expire in a controlled manner.
- Failure cases are clear and testable.

## Issue 10: Implement the file lifecycle with RESERVED, COMMITTED, and EXPIRED

Labels: `feature`, `files`, `ttl`, `storage`

Description:
- Bind uploads to event ID and TTL.
- Implement early deletion with an opaque capability.
- Prevent object TTL from exceeding event TTL.

Acceptance criteria:
- Object TTL is never greater than event TTL.
- Upload state is clearly traceable.
- Orphaned uploads expire correctly.

## Issue 11: Implement rate limits and the reporting baseline flow

Labels: `feature`, `moderation`, `security`

Description:
- Introduce token buckets for login, message, upload, invite, and report.
- Implement the reporting flow with explicit content selection.
- Ensure only selected content is disclosed.

Acceptance criteria:
- Limits are visible and testable.
- Reports include only explicitly selected content.
- No hidden content upload path exists.

## Issue 12: Finish the canary, leak, and migration suite for Private Alpha

Labels: `testing`, `release`, `alpha`

Description:
- Add tests for DB, object storage, logs, traces, and backups.
- Test migration, replay, tampering, restart, and clock drift.
- Write self-hosting documentation and incident runbooks.

Acceptance criteria:
- Core acceptance criteria are reproducibly verifiable.
- No open P0/P1 security findings remain.
- A small test community runs stably.

## Issue order

Recommended order for solo execution:
1. Issue 1
2. Issue 2
3. Issue 3
4. Issue 4
5. Issue 5
6. Issue 6
7. Issue 7
8. Issue 8
9. Issue 9
10. Issue 10
11. Issue 11
12. Issue 12

## Next step

If you want, I can turn this into a GitHub-compatible import list next, for example as CSV or as individual Markdown files per issue.
