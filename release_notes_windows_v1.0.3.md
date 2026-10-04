# 1.0.3 - Reduce editor, history and risk overview overhead

Windows client, sync launcher and sync service share the independent version 1.0.3.

Sticky-note editor frames borrow stored notes, versions and recovery records instead of repeatedly copying complete histories. Property undo snapshots are created when a property changes. Historical read-only views, pin changes, undo, redo and recovery actions retain their original behavior.

Knowledge history retains one comparison and a lightweight revision list until the page, source data or editor draft changes. Block matching uses an ID index. Locking a page or closing its encryption session clears retained plaintext. Recent-page cards and backlinks copy only the fields they display.

Risk overview preparation parses the profile once, computes the raw report and trend, then sanitizes once for health and risk. Differential tests compare the result with the existing independent calculations. Trend labels no longer copy the ledger on every frame.

Background save receipts retain the timer summary only when source generations and every timer-session field match. Changed sessions invalidate the summary; its original day remains in the key so midnight still triggers recalculation.

Synthetic frame and operation measurements, state tests and mutation checks are recorded in `release_artifacts/verification/v1.0.3-performance`. Formal installation status is defined by `release_artifacts/current/release_manifest.json`.
