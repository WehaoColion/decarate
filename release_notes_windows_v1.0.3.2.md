# 1.0.3.2 - Isolate editor undo state and restore explicitly recovered attachments

Published Windows release. Formal installation status is defined by `release_artifacts/current/release_manifest.json`.

The Windows knowledge editor releases its framework text-edit state when the active document or plaintext session changes. This prevents another document from receiving earlier text through undo and limits retained editor history during repeated navigation. Undo in the same active document remains available.

The attachment upload preflight accepts an explicit current attachment restoration only when its revision is newer than every applicable deletion boundary. Historical references and a newer upload clock do not grant restoration authority. Download-only behavior and server restore barriers remain enforced.

The Windows client, synchronization service, supervisor and stable desktop entry use 1.0.3.2. Only the fourth Windows version component is incremented. This release does not build, test or publish Android. Existing Android release files are preserved.

Validation evidence belongs to `release_artifacts/verification/v1.0.3.2-windows-audit`. Historical encrypted and shared-media ownership, managed recovery completeness, native continuous editing, and actual fresh logon or lock-screen acceptance remain separate open audit items until verified.
