# 1.0.2.2 - Reduce list, database and timer overhead during daily use

Windows client, sync launcher and sync service use the independent version 1.0.2.2. Android remains on its own release sequence.

Knowledge and sticky-note lists now cache display fields rather than copying whole documents and recovery histories. Preview text and search snippets are prepared when their source changes. Selecting a row resolves the latest complete document by ID and avoids an extra full-document copy. Locked pages remain hidden; an unlocked page remains readable until it is locked again.

Database views filter their row IDs once and share an indexed page lookup during the frame. Ordinary record properties no longer prepare an entire query engine, and collapsed properties defer query work until opened. Existing input focus and save behavior remain intact.

Paused timers reuse their current projection; running timers still advance from the exact current time. Board rendering borrows timer views and reads summary counters without copying recent sessions. Empty searches avoid normalizing every timer note. Timer phase persistence checks read only timer slots before the existing full mutation and integrity checks.

Synthetic comparisons from one debug test executable are recorded under `release_artifacts/verification/v1.0.2.2-performance`. They measure specific CPU/allocation paths, not complete native window latency or total process memory. The formal publication result is recorded separately by the release manifest and Windows verification report.
