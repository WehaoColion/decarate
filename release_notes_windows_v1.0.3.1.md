# 1.0.3.1 - Recover appearance saves and isolate encrypted note sessions

Published Windows release. Formal installation status is defined by `release_artifacts/current/release_manifest.json`.

An appearance change can be saved after a previous appearance-only write failed and the user makes a new choice. A delayed failure receipt preserves a newer selection. Failed content saves retain their explicit retry requirement. Switching to another workspace after a completed save barrier clears only the previous workspace's failure state.

Encrypted note sessions are isolated by verified account and local workspace. Changing the password of a note no longer conflicts with or revokes a same-ID note in another workspace. Existing encryption formats and synchronization data versions are retained.

Windows build receipts bind the Windows manifest fields and binaries independently of a later retained Android APK publication. Legacy verification receipts remain bound to their original complete release. Windows formatting checks cover Windows and shared Rust source, including included test modules; Android generators are outside this Windows-only gate.

The Windows client, sync service, sync supervisor and stable desktop entry use 1.0.3.1. This increments only the fourth Windows version segment. This update does not build, test or publish Android. Existing APK files are preserved and checked by hash and modification time.

Validation evidence is recorded in `release_artifacts/verification/v1.0.3.1-windows-audit`. Physical phone synchronization, a fresh Windows logon and interactive native-window acceptance require separate runtime evidence and are not implied by automated regression results.
