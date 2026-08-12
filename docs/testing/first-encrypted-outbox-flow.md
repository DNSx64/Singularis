# First encrypted outbox test

## Purpose

This integration test proves the first crash-safe client-to-relay message path. It does not require a running server or desktop UI; the Axum router runs in the test process.

## Run

Use the VS Code task `Singularis: Test first encrypted outbox flow` or run:

```sh
flatpak-spawn --host cargo test -p singularis-server queued_mls_event_survives_restart_and_relay_retry -- --nocapture
```

## Proven flow

1. Alice and Bob establish an MLS group.
2. Alice encrypts a plaintext canary.
3. Alice's successor MLS snapshot and canonical relay request commit atomically to SQLCipher.
4. Alice is dropped and the vault is locked to simulate a process crash before submission.
5. A fresh client unlocks the vault and recovers exactly one pending request.
6. The same request is posted twice to the in-process Axum relay.
7. Both posts return the same receipt and the relay lists one event.
8. Bob decrypts the original canary.
9. Alice acknowledges the request locally and a second restart confirms an empty outbox.

The test passes only when every assertion above succeeds and Cargo reports `1 passed; 0 failed`.

## Native desktop worker

The Tauri sender boundary has a separate focused suite:

```sh
flatpak-spawn --host cargo test -p singularis-desktop outbox::tests
```

It verifies that the worker posts a recovered canonical request to an actual Axum HTTP listener, validates the receipt, and acknowledges the SQLCipher entry. It also proves that network I/O runs without the vault mutex, Quick-Lock cancels an in-flight request without losing it, insecure remote HTTP origins are rejected, and oversized chunked receipts are bounded before parsing.

At runtime, successful initialize or unlock resumes the worker. Quick-Lock pauses it before the database closes. The composer status reports paused, sending, deferred, or synchronized state and offers a manual retry when encrypted entries remain pending. The browser preview remains local-only and does not claim relay delivery or MLS encryption.

## Native composer flow

Run the focused service-to-relay test with:

```sh
flatpak-spawn --host cargo test -p singularis-desktop native_message_queue_is_relayed_and_acknowledged -- --nocapture
```

The test starts with an empty schema-v4 vault. The native messaging service creates one persistent local device ID and a prototype MLS channel, then commits the local message, FTS entry, successor MLS snapshot, and opaque `SubmitEvent` atomically. The production HTTP worker submits that exact request to an Axum listener, validates its receipt, removes the acknowledged outbox row, and leaves the local archive intact.

The current desktop bootstrap creates a one-member MLS group for each known prototype channel. This proves native encryption, ratchet persistence, crash-safe queueing, and relay opacity; it does not yet prove community membership. Key-package publication, Welcome delivery, member addition, and incoming synchronization remain required before the UI can claim multi-user E2EE.