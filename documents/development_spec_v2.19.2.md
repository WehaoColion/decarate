- v2.19.2-english_development_spec

# DecaRate Development Specification

## 1. Document Purpose

This document is reverse-engineered from the current application source code, release artifacts, and existing project notes. It is intended as a baseline document for future development, maintenance, testing, and handoff. It is not marketing copy and it is not a user manual. It answers three practical questions: what the app currently is, what capabilities it already has, and which boundaries must be preserved during further development.

The current baseline is Android `2.19-knowledge_ai_link`. The repository also contains a Windows client, a Windows sync server, and a sync launcher. The Android entry source is generated from Rust-side templates. A large part of the core business logic is provided through Rust and JNI, while the interface is built with Kotlin and Jetpack Compose.

## 2. Product Definition

DecaRate is a personal time, note, knowledge base, finance, and sync tool. The timer is the entry point, but the actual product goal is to keep daily work rhythm, short notes, long documents, money records, and review signals inside one local-first system.

Core principles:

- Local first. Timer, note, document, and finance data are written to the local device first.
- Timer driven. Users enter a working state by starting a slot. History, notes, archives, and review are organized around timer records.
- Layered recording. Temporary thoughts belong in quick notes, structured content belongs in the document library, and financial data belongs in a separate ledger.
- Optional sync. Account sync depends on the desktop sync service and does not force a cloud service.
- AI is used only for organization and question answering. It must not replace the local data structure.

## 3. Current Module Map

The bottom navigation has five main entries:

- Timer: the main board, showing timer slots, today's total, running slots, recent records, and a recommended item to continue.
- Notes: the quick note area for short text, images, to-do items, and temporary materials.
- Knowledge: the document library for long documents, image and text blocks, contacts, call records, and knowledge base Q&A.
- Risk: the finance and analysis area, covering daily ledgers, monthly snapshots, quarterly review, and yearly review.
- Mine: account sync, device information, and OpenAI configuration.

History is not a standalone bottom tab. It opens as a sheet from the timer board. Diagnostics and data export are available near the bottom of the timer board.

## 4. Functional Specification

### 4.1 Timer Board

Current behavior:

- The app provides 14 timer slots by default. Slot IDs are fixed from 1 to 14.
- Each slot supports a title, note, category, accumulated duration, and running state.
- Multiple slots can run at the same time.
- Slots can be reordered by drag and drop.
- Slot cards show current state, today's duration, record count, recent record data, and related details.
- The home screen shows the running count, today's total, archive count, recent records, longest record, and today's busiest slot.
- The home screen provides a recommended item to continue. It prioritizes currently running slots or the best slot to resume.

Main operations:

- Tap a slot to open its detail screen.
- Tap the play or pause button to start or pause timing.
- Long press and drag to change board order.
- Open the quick action sheet to view details, view history, reset, or archive.

Acceptance points:

- After any slot starts, the home screen, detail screen, and notification duration must increase consistently.
- When multiple slots are running, the home running count and notification content must be correct.
- After reordering slots, the order must persist after leaving and reopening the app.
- Empty slots, titled slots, and slots with accumulated time must use distinct display copy.

### 4.2 Micro-Break Mechanism

Current behavior:

- The focus phase target duration is 3 to 5 minutes, with small variations based on slot and cycle.
- After a focus phase ends, the app enters a 15-second micro-break.
- After the break ends, the app automatically starts the next focus phase.
- Break duration is not counted as focused duration.
- Phase changes play a bell and attempt to send a reminder notification.

Acceptance points:

- When the focus target is reached, a timer record must be generated and the slot must switch to break.
- After the 15-second break ends, focus must continue without creating an extra focus record.
- Pausing during a break must not write break time into focus records.
- Notifications must keep refreshing while the app is running in the background.

### 4.3 Timer Detail

Current behavior:

- The detail screen shows the slot title, category, note, accumulated duration, today's total, longest single record, and recent records.
- Title, note, and category can be edited.
- New categories can be created and assigned to the current slot.
- The current slot can be started, paused, reset, and archived.

Acceptance points:

- Title length, note length, and category name length must be limited and cleaned automatically.
- A running slot cannot be archived.
- After archiving, the original slot must return to an idle state. The archived task can be restored from history.

### 4.4 History

Current behavior:

- A timer record is generated when a focus phase is paused.
- The history sheet supports filtering by category, slot, and keyword.
- It shows today's total and the total from the last seven days.
- Timer records can be deleted.
- Archived tasks can be viewed, deleted, and restored.

Acceptance points:

- Search must cover title, note, category name, and slot ID.
- After deleting a history record, statistics must refresh immediately.
- When restoring an archived task, the app must prefer the original slot. If the original slot is unavailable, it must choose an available slot or show that no slot is available.

### 4.5 Quick Notes

Current behavior:

- Notes are divided into quick notes and document library pages. The quick note type is `STICKY`.
- The app supports creation, editing, autosave, pinning, folder organization, and search.
- Trash, restore, permanent delete, and empty trash are supported.
- Plain text, rich text, Markdown, and to-do lists are supported.
- Images can be inserted and are stored locally.
- Local audio and video transcription is supported. The current primary language is Chinese.
- OpenAI can be used to organize the current note body.
- Copy, text sharing, long-image sharing, and web page sharing are supported.
- Manual version saving and version restore are supported.

Acceptance points:

- If a new blank note is opened and the user returns immediately, the blank draft must be cleaned automatically.
- If a note has content, returning from it must save it to disk.
- After an image is deleted, local attachment references must be cleaned accordingly.
- Switching between rich text and plain text must not lose the body content.
- If no OpenAI API key is configured, the organize action must clearly point to the configuration entry.

### 4.6 Document Library and Knowledge Pages

Current behavior:

- The document library type is `DOCUMENT`. It is used for pages that are more complete than quick notes.
- A document is made of multiple blocks. Block types include text, image, contact, and call.
- Markdown mode, block-level editing, block moving, and block deletion are supported.
- Undo, redo, to-do, numbered list, title, quote, bold, and related text operations are supported.
- Images can be selected from the system album or inserted from the camera.
- Contacts can be selected and converted into contact blocks.
- Recent call records can be read and converted into call blocks.
- Local audio and video transcription can be inserted as text blocks.
- Outline, folders, color, pinning, trash, search, and filtering are supported.
- Quick notes can be converted into knowledge pages.
- Text sharing, long-image sharing, web page preview, and web page sharing are supported.

Acceptance points:

- After document block order changes, the document must autosave.
- If contact or call permissions are denied, the interface must show understandable feedback.
- Document search must cover title, body, image caption, contact data, and call numbers.
- When restoring a document version, title, body, folder, color, pinned state, and attachment references must restore together.

### 4.7 AI Knowledge Base Q&A

Current behavior:

- The document library provides an "AI Ask Knowledge Base" entry.
- The Q&A scope can be the current folder or all knowledge pages.
- The system retrieves up to 5 local document sources based on the question.
- Requests are sent through the OpenAI Responses API. The default model is `gpt-5.2`.
- Answers must be based on local sources. If sources are insufficient, the answer must state that evidence is missing.
- An answer can be saved as a new knowledge page. The body includes the question, answer, and source summaries.

Acceptance points:

- Requests cannot be sent without an API key.
- Requests cannot be sent for an empty question or with no sources.
- Source cards must be able to open the original document.
- After saving an answer, it must appear in the document library and keep the source content.

### 4.8 Finance and Risk Control

Current behavior:

- Finance data is stored in `FinanceProfile`.
- Basic monthly input is supported: active income, asset income, living expenses, debt expenses, cash reserve, productive assets, and liability balance.
- Daily ledgers are supported: income, expense, and notes.
- Income types include active income, asset income, and other income.
- Expense buckets include debt reduction, daily consumption, long-term allocation, fixed expenses, growth investment, and flexible reserve.
- Custom expense bucket labels and target ratios are supported.
- Monthly snapshots are supported: asset items, liability items, and notes.
- Day, month, quarter, and year views are supported.
- The app automatically summarizes income, expense, net cash flow, net worth, defense buffer, passive coverage, salary dependence, debt pressure, and asset return.
- It generates a finance health score, trend alerts, and target drift alerts.
- Finance data can be exported and restored independently without affecting timer, note, or sync settings.

Acceptance points:

- After daily ledger changes, monthly, quarterly, and yearly summaries must change automatically.
- After monthly snapshot changes, net worth and trends must change automatically.
- Finance restore must only overwrite the finance module. It must not overwrite timer or note data.
- Exported finance backup filenames must include the current version number.

### 4.9 Account Sync

Current behavior:

- Email registration, login, and logout are supported.
- Passwords must contain at least 6 characters.
- The default sync service address is automatic mode, `auto://sync`.
- The mobile client tries to reuse the last available public address, probe local network services, and read the public desktop entry.
- The app supports normal sync and forced upload of local data to the account.
- The Windows sync service listens on `0.0.0.0:8917` by default. The local health address is `127.0.0.1:8917`.
- The sync service stores users, password hashes, tokens, and app data snapshots locally.
- The desktop launcher checks whether the local service is already running. If it is not running, it starts the latest sync server automatically.

Acceptance points:

- Sync cannot run while the user is logged out.
- After login succeeds, the token and device name must be saved.
- When the phone and computer are on the same local network, automatic mode must find the service.
- When a mobile network tries to access an intranet address, the app must explain the need for public access, tunneling, or port forwarding.
- Forced local upload must overwrite account data with local data.

### 4.10 Notifications and Xiaomi HyperOS

Current behavior:

- Running timers generate a persistent live notification.
- The notification shows the primary timer item, accumulated duration, parallel running count, and summaries for up to 4 running slots.
- The notification provides quick actions to pause the current slot or pause all slots.
- Micro-break transitions generate high-priority reminder notifications.
- Experimental paths exist for Xiaomi Focus notification and Super Island payloads.
- The Xiaomi path checks protocol version, Super Island capability, permissions, App ID configuration, and build type, then writes diagnostic logs.

Acceptance points:

- On Android 13 and above, if notification permission is not granted, the app must not crash. It must cancel live notifications instead.
- Tapping a notification must open the corresponding slot detail screen.
- Notification pause actions must pause the target slot and refresh the notification.
- On Xiaomi devices, the app must output diagnostic snapshots to help locate why Super Island is unavailable.

### 4.11 Mine, Theme, and Language

Current behavior:

- The Mine page shows login state, sync state, slot count, record count, and note count.
- Sync email, device name, and password can be configured.
- OpenAI API key, API base URL, and model name can be configured.
- Theme modes include system, light, and dark.
- Language entries include system, Simplified Chinese, English, and Japanese.

Acceptance points:

- Theme changes must persist.
- Language changes must take effect through the system Locale mechanism.
- OpenAI configuration must persist with the sync account session, but AI configuration must remain after logout.

### 4.12 Diagnostics and Export

Current behavior:

- Diagnostic logs and crash capture are installed when the app starts.
- A diagnostic recording can be started, then logs can be exported after reproducing an issue.
- Logs include app events, crashes, performance summaries, process exit reasons, logcat, and current app data.
- Full data package export is supported. The package includes current state, backup state, event logs, crash files, and related data.
- The timer board provides diagnostics access and Xiaomi Super Island integration status near the bottom.

Acceptance points:

- After diagnostic recording starts, even if the app crashes, the recording must be exportable after restart.
- Exported data packages must be sent through the system share sheet.
- Log files must not contain signing passwords, API keys in plain text, or other sensitive values.

## 5. Data Model

### 5.1 App State

The core state object is `AppData`. The current schema version is 10. Main fields:

- `categories`: timer categories.
- `slots`: 14 timer slots.
- `slotOrder`: board display order.
- `sessions`: completed timer records.
- `archivedTasks`: archived tasks.
- `noteFolders`: note and document folders.
- `notes`: quick notes and document library pages.
- `notePreferences`: note sorting and current folder.
- `financeProfile`: finance data.
- `themeMode`: theme mode.

### 5.2 Timer Data

`TimerSlot`:

- `id`: 1 to 14.
- `title`: title.
- `categoryId`: category.
- `note`: note.
- `accumulatedMillis`: accumulated focus duration.
- `runningSinceEpochMillis`: running start time. Null means the slot is not running.
- `microBreakPhase`: current focus or break phase.
- `microBreakCycleIndex`: micro-break cycle index.
- `microBreakPhaseProgressMillis`: progress in the current phase.
- `updatedAt`: update time.

`TimerSession`:

- Records one completed focus segment.
- Includes slot ID, title, category, start time, end time, and duration.

`ArchivedTask`:

- Stores the archived slot title, note, category, accumulated duration, and original slot ID.

### 5.3 Notes and Documents

`NoteEntry`:

- `kind` is either `STICKY` or `DOCUMENT`.
- Supports title, legacy plain text body, structured document, color, pinned state, folder, attachments, version snapshots, creation and update times, and deletion time.

`NoteDocument`:

- Supports Markdown mode, rich text mode, rich text plain text index, and a block list.

`NoteBlock`:

- `TEXT`: plain text block.
- `IMAGE`: image block referencing an attachment ID.
- `CONTACT`: contact block.
- `CALL`: call block.

`NoteRevisionSnapshot`:

- Stores title, body, document structure, color, pinned state, folder, and attachment references for version restore.

### 5.4 Finance

`FinanceProfile`:

- Basic monthly income and expense fields.
- Current focused assets and liabilities.
- Expense category settings.
- Daily ledgers, `dailyLedgers`.
- Monthly snapshots, `monthlySnapshots`.

`FinanceDayLedger`:

- Income items, expense items, and notes.

`FinanceMonthSnapshot`:

- Asset items, liability items, and notes.

### 5.5 Sync Session

`SyncAccountSession`:

- Sync service address, last resolved address, email, user ID, token, device name, last sync time, and last status.
- AI configuration: API key, base URL, model name, and latest AI status.

## 6. Persistence and Files

Main Android local files:

- `filesDir/timer_state.json`: main state.
- `filesDir/timer_state_backup.json`: backup state.
- `filesDir/timer_state.tmp`: temporary write file.
- `filesDir/sync_account.json`: sync account and AI configuration.
- `filesDir/note_media/`: note images and other attachments.
- `filesDir/diagnostic_events.log`: diagnostic events.
- `filesDir/last_crash.txt`: latest crash.
- `cacheDir/shared_exports/`: shared export packages, finance backups, web pages, and related files.
- `cacheDir/note_transcription/`: temporary media for transcription.

Persistence requirements:

- State writes must write to a temporary file first, then replace the main file, and keep a usable backup.
- The app must flush state when entering the background, when the ViewModel is cleared, and after key operations.
- When reading data, the app must clean both main and backup files and prefer usable data with a newer revision.

## 7. Technical Architecture

### 7.1 Android

- The Gradle module is in `app/`.
- The package name is `com.ofairyo.gridtimer`.
- The UI uses Jetpack Compose.
- `MainActivity` only handles window setup, Intent routing to a target slot, and mounting `GridTimerRoot`.
- `TimerViewModel` passes UI operations to `TimerRepository`.
- `TimerRepository` handles state loading, cleaning, disk writes, sync, timing, notes, finance, and notification integration.

### 7.2 Rust

The Rust crate is in `native/gridtimer_native/`. Responsibilities include:

- Generating Android Kotlin sources and resource entries.
- Providing the JNI optimization bridge, `NativeOptimizerBridge`.
- Core logic for timing, micro-breaks, search, finance summaries, text processing, sync, AI requests, and related operations.
- Windows client, sync server, and sync launcher.
- Packaging, source audit, and build helper tools.

### 7.3 Generated Source Strategy

Android code is not primarily maintained as hand-written Kotlin files. It is emitted from Rust templates into the Gradle generated source set. When changing Android behavior, update these first:

- `native/gridtimer_native/src/sourcegen/kotlin_sources.rs`
- `native/gridtimer_native/src/sourcegen/android_sources.rs`
- Related Rust business modules

Do not only edit the Gradle generated directory, because generated outputs are overwritten during rebuilds.

### 7.4 JNI Fallback

Most native optimizations have Kotlin fallbacks. Rules:

- Use Rust results first when JNI is available.
- If JNI is unavailable or throws, the UI must not crash.
- Sync and AI requests depend more heavily on Rust and require separate testing after changes.

## 8. Permissions

The Android Manifest currently uses:

- `INTERNET`: sync and OpenAI requests.
- `ACCESS_NETWORK_STATE`, `CHANGE_NETWORK_STATE`: network status and sync route decisions.
- `POST_NOTIFICATIONS`: live timer and micro-break notifications.
- `RECORD_AUDIO`: local audio and video transcription.
- `READ_CONTACTS`: contact blocks.
- `READ_CALL_LOG`: recent call blocks.
- `WRITE_EXTERNAL_STORAGE`, Android 9 and below only: historical compatibility.

Permission strategy:

- Sensitive permissions should be requested only when a feature triggers them.
- If a permission is denied, the feature entry should remain visible but provide clear feedback.
- Missing permissions must not make the editor or main timer workflow unusable.

## 9. Sync Protocol

The sync service provides registration, login, sync, forced upload, and related abilities. The server stores user records locally. Passwords use salt plus hash and are never stored as plain text. During sync, the client sends local app state and revision time. The server compares freshness and decides whether to return server data or accept client data.

Key rules:

- The token is the account session credential.
- Device name is used to distinguish sources.
- Automatic address resolution must prioritize a clear explanation of the current network problem.
- Forced upload is a dangerous operation. UI copy must clearly state that it overwrites account data with local data.

## 10. AI Integration

AI is currently used for two capabilities:

- Organizing the current note body.
- Answering questions based on local knowledge base sources.

Request constraints:

- The default base URL is `https://api.openai.com/v1`.
- Remote endpoints must use HTTPS. Local debugging may use HTTP for `127.0.0.1`, `localhost`, or `[::1]`.
- Knowledge base Q&A uses at most 5 to 6 source summaries.
- Answers must cite source numbers. If sources are insufficient, the app must not invent content.
- AI output is written only to the note selected by the user or to a new knowledge page.

## 11. Release and Build

Current Android build requirements:

- JDK 17.
- Android SDK API 34.
- Android NDK 27.1.12297006.
- Rust stable.
- `cargo-ndk`.
- Local signing configuration in `release_signing.properties`.

Gradle tasks run:

- Rust source generator.
- Rust native library build.
- Source audit.
- Android release APK output.

Release rules:

- The current official APK is stored in `release_artifacts/current/`.
- Old APKs and old Windows artifacts are moved into `old_apks/`.
- Official delivery keeps only the official APK. It must not keep debug APKs, Xiaomi debug APKs, `app-release` APKs, or AAB files.
- APK filenames must include the version number.

## 12. Test Checklist

Each major change must cover at least:

- Cold start can load the main state.
- The app can restore from backup when main state is damaged.
- Timer start, pause, continue, reset, archive, and restore.
- Parallel timing across multiple slots.
- Micro-break automatic transition and reminder notification.
- History search, deletion, and statistics refresh.
- Quick note creation, save, delete, restore, image, and share.
- Document library block editing, image, contact, call, transcription, and version restore.
- Missing AI configuration prompt, note organization, and knowledge base Q&A.
- Finance daily ledger, monthly snapshot, period switching, export, and restore.
- Registration, login, sync, forced upload, and logout.
- Notification permission denied state and notification refresh after permission is granted.
- Super Island status output on Xiaomi devices or simulated diagnostic paths.
- Light, dark, and system theme modes.
- Simplified Chinese, English, and Japanese entries do not crash.

## 13. Future Development Priority

P0:

- Prevent data loss in timing, saving, restore, and sync.
- Keep the official APK packaging flow stable.
- Fix any issue that causes crashes, app exits, or main state corruption.

P1:

- Improve knowledge base Q&A source ranking and source citation display.
- Strengthen note attachment cleanup to prevent orphan file growth.
- Improve validation feedback for finance export and restore.
- Complete sync conflict prompts to reduce accidental overwrites.

P2:

- Add clearer first-use guidance.
- Add more finance charts and long-term trend views.
- Extend local transcription languages.
- Improve feature parity between the Windows client and Android.

## 14. Maintenance Notes

- When changing Android UI, update Rust source generation templates first. Do not edit generated directories as the source of truth.
- When changing data models, update Kotlin and Rust cleaning logic together.
- Any `AppData` schema change must include a migration path for old data.
- Notification changes must test both normal Android notifications and the Xiaomi diagnostic path.
- Sync changes must test local network, fixed address, and mobile network access to intranet address scenarios.
- AI changes must test no key, invalid key, network failure, and empty output.
- File export changes must confirm that shared files can be opened by other apps.
- Do not commit signing files, passwords, API keys, or personal local paths.
