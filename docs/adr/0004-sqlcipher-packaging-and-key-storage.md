# ADR 0004: SQLCipher packaging and vault key storage

- Status: Accepted for the Phase 1 prototype
- Date: 2026-08-09
- Scope: Desktop clients

## Context

Singularis must persist messages, drafts, outbox entries, and the local search index without placing plaintext in SQLite, browser storage, or normal files. The desktop prototype must build reproducibly on Void Linux while preserving a path to supported Windows and macOS packages.

Using the passphrase directly as the database key would make passphrase changes expensive and would couple SQLCipher settings to authentication policy. Depending on a system SQLCipher installation would also make development and release artifacts differ across distributions.

## Decision

1. The shared Rust vault uses `rusqlite` with `bundled-sqlcipher-vendored-openssl` during Phase 1. Release packaging must review platform licensing, update cadence, and binary provenance before public distribution.
2. Each vault receives a random 256-bit data key. SQLCipher receives this value as a raw key; the user passphrase is never used as the database key.
3. A versioned key header stores only a random salt, bounded Argon2 parameters, a random XChaCha20 nonce, and the authenticated ciphertext of the data key.
4. Argon2id derives a 256-bit wrapping key with a Phase 1 minimum of 64 MiB memory, three iterations, and one lane. Parameters are authenticated as associated data and validated before allocation.
5. XChaCha20-Poly1305 wraps the data key. Authentication failure and an incorrect passphrase produce the same public error.
6. The derived wrapping key is zeroized immediately. The raw data key is zeroized after opening SQLCipher. Locking closes every database connection and clears vault-backed UI state.
7. The vault directory uses user-only permissions where the platform supports them. SQLCipher memory security, secure deletion, in-memory temporary storage, foreign keys, and WAL checkpointing are enabled.
8. FTS5 lives inside the encrypted database. Search terms never leave the client.
9. The Vite browser preview remains memory-only. It must not emulate the vault with Local Storage, IndexedDB, or Cache Storage.
10. Operating-system keyring integration is deferred. The passphrase path remains mandatory until a separate keyring threat model and recovery policy are accepted.
11. Vault schema version 2 adds one bounded MLS client snapshot per device. The complete snapshot is replaced by a single SQLite statement, so readers observe either the previous state or the complete successor state. Opening a version 1 vault creates this table and advances `user_version` in one migration transaction.
12. Vault schema version 3 adds a bounded MLS outbox. An outgoing application `SubmitEvent` and its successor MLS snapshot commit in one SQLite transaction. Exact retries are idempotent; reusing an event ID with a different device, payload, or snapshot fails and rolls back the whole transaction. Entries remain queued across restarts until the client processes a matching successful relay receipt.
13. Vault schema version 4 adds one random local desktop device ID inside SQLCipher. The ID is created lazily, remains stable across lock and restart, and selects the matching persistent MLS snapshot. A native composer send commits the local archive row, its FTS update, the successor MLS snapshot, and the canonical opaque outbox request in one transaction. Failure in any part rolls back every part.
14. The native desktop owns one device-agnostic outbox worker. A successful initialize or unlock resumes it; Quick-Lock pauses it and cancels an in-flight request before closing SQLCipher. The worker copies bounded canonical requests while holding the vault mutex, releases the mutex for network I/O, and acknowledges only a matching accepted receipt. It retries periodically or on explicit user request.
15. `SINGULARIS_API_URL` must be an origin without credentials, query, fragment, or path. Remote relays require HTTPS; plaintext HTTP is restricted to loopback development. Redirects are disabled, requests have connect and total timeouts, and relay receipts are read incrementally with a 64 KiB limit.

## Consequences

- The desktop build is larger and compiles vendored OpenSSL and SQLCipher from source.
- Passphrases can later be changed by rewrapping the random data key without re-encrypting the database.
- A damaged or missing key header makes the database unavailable even when the database file survives; backup/export work must treat both as one atomic vault.
- SQLCipher protects data at rest but does not protect an unlocked client from a compromised process, kernel, accessibility service, or screen capture.

## Rejected alternatives

- Plain SQLite plus application-level encryption: too easy to leak indexes, metadata, journals, and temporary query material.
- Direct passphrase-to-SQLCipher keying: prevents cheap passphrase rotation and mixes authentication policy with data encryption.
- JavaScript browser storage: violates the desktop trust boundary and cannot provide the required lock semantics.
- System-only SQLCipher packages: not reproducible enough for the Phase 1 cross-distribution prototype.

## Verification requirements

- SQLCipher must report a non-empty `cipher_version`.
- Opening the database without its raw key must fail.
- A plaintext canary stored and retrieved through the unlocked vault must not occur in the database, WAL, SHM, or key-header bytes.
- Wrong passphrases, modified key headers, and modified wrapped keys must fail without returning partial data.
- Locking must make reads, writes, and searches fail until a successful unlock.
- A fresh process must unlock the same vault and recover persisted messages and FTS results.
- A fresh process must recover MLS signing keys, groups, ratchets, sender chains, and replay state without exposing their snapshot bytes in the database, WAL, SHM, or key header.
- Failed MLS processing and failed snapshot checkpoints must leave the last committed state loadable. Empty, oversized, malformed, duplicate, and wrong-device snapshots must be rejected.
- A queued MLS application event and its successor ratchet state must become visible atomically. Restarting before network submission must preserve the canonical request; retrying it must produce one relay event; local acknowledgement must survive another restart.
- The local device ID must be generated once inside SQLCipher and remain unchanged after restart. A composer write failure must leave the local archive, FTS index, MLS snapshot, and outbox at their prior state.
- The native worker must not hold the vault mutex during relay I/O. Quick-Lock must cancel an in-flight submission without acknowledging it, and the queued request must remain available after the next unlock.
- Oversized receipts must fail before the worker accumulates more than 64 KiB, including when the relay uses chunked transfer encoding without `Content-Length`.

## Follow-up decisions

- OS-specific keyring storage and high-security mode behavior.
- Locked-memory guarantees, core-dump policy, and crash handling per platform.
- Vault export container, recovery KDF, streaming AEAD, and test vectors in ADR 0011.
- Provisioned community membership, key-package publication, Welcome delivery, and replacement of the current one-member desktop bootstrap groups.