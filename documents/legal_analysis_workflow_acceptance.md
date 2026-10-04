# Android legal analysis workflow acceptance

## Baseline and scope

This follow-up is based on PR #2's candidate branch, `fix/android-legal-scan-recovery-20261004`, at `e7fa95a650c6f9c675e8bafbf7e2c29d3cb3bed0`. That branch contains the locally built Android 2.23.2.8 candidate and its lifecycle fix. PR #2 is still open and unmerged; these changes are NOT already on main. This follow-up PR targets that candidate branch so its diff contains only the new workflow repair. Do not overwrite newer local work or blindly reapply PR #2.

The overview's existing start action navigates to the legal screen. The actual model request already exists: `LegalRiskScreen.send()` -> `NativeOptimizerBridge.runLegalScan()` -> `legal_scan::run_scan()`. The user report does not establish a unique device-side cause. The source does establish that preparation was below the complete encrypted-note list, sending was below coverage and omissions, and some disabled-send conditions lacked actionable feedback.

## Changes

The overview has a full-width start button. The workflow opens in a full-screen secure dialog, separate from the finance pager and floating home navigation. Preparation, sending, cancellation, input/configuration blockers and retry controls appear above the scrolling content instead of after potentially long lists.

Sending opens an actual confirmation dialog showing recipient, model, evidence count, upload estimate and capture time, with scrollable disclosures. Logged-in users are also told about final-report synchronization. Only confirmation invokes the existing send function. Consent is bound to the scan, full endpoint, model and key in memory and can be consumed only once. Opening the page and local preparation do not send model requests. Changed input/configuration cannot reuse consent. Native report failures are surfaced near the main action, and the complete report remains below.

The checked transform runs inside `write_source`, after older transforms and for the separately emitted legal screen and its existing JVM test file. Missing or duplicated anchors fail source generation rather than silently removing an action.

Existing data invalidation, account/workspace isolation, encrypted-note handling, native request ownership, evidence validation and report storage are retained. This PR does not alter financial calculations, the model protocol, Windows services, signing materials, release versions or the formal release manifest.

## Verification status

Before the integration edit, the reconstructed source generator was verified byte-for-byte against Git blob `cba821eb7dc31a7c73dc57138e39cbc361bf7005`. The generator change adds two module-registration lines and three final-transform lines.

During preparation of this change, 12 new pure Kotlin workflow/consent tests were compiled and executed with the installed Kotlin compiler and kotlin.test assertions; all 12 passed. The helper and test bodies came from the authored template. Only JUnit annotations/imports were adapted for the standalone runner. This tests decision and consent behavior, not rendered Compose UI or a live provider.

The generator appends these 12 cases to the existing legal test source, preserving its 13 previous cases. Four Rust source-contract tests are added for production-template action wiring, explicit confirmation, entry isolation, drift failures and final-hook registration.

Not executed in this editing environment: Rust compilation/tests/rustfmt, the complete source-generation chain, Gradle/Compose compilation, device UI tests, real DeepSeek requests or signed APK packaging. No claim of phone-side success should be made from the standalone tests alone.

## Local Codex acceptance and APK delivery

Preserve local changes and original signing/configuration files. Fetch this follow-up branch, inspect the incremental commit, and integrate it with the current local candidate. Because PR #2 is unmerged, do not reset the project to main and thereby lose the candidate fixes.

Use the repository's supported build flow. Include the new generator tests:

    cargo test --manifest-path native/gridtimer_native/Cargo.toml --bin gridtimer_sourcegen android_legal_workflow::tests

Run formatting, the generated LegalSendReadyTest and LegalAnalysisWorkflowTest, the complete source-generation chain and the Android release checks. Existing app/build.gradle policy prohibits debug/test APKs and requires formal release signing. Do not disable that policy just to package an instrumentation test.

Inspect actual generated Kotlin, not just templates: GridTimerScreen.kt must contain legal_open_analysis; LegalRiskScreen.kt must contain legal_primary_action, legal_confirm_send, and the guarded native request. The visible labels include "整理资料并继续" and "确认发送并分析". Check that the newly built APK contains this change rather than returning an old APK.

Use a non-sensitive test workspace for device/provider acceptance. Cover an empty workspace, incomplete configuration, long encrypted-note/omission lists, readable text input, an invalid API key, duplicate clicks, cancellation, data changes during work, account switching, closing/reopening and a successful report. Primary controls must remain visible without scrolling through those lists. Before confirmation, there must be zero model requests. After confirmation, verify an actual provider response and saved report, not merely a spinner or a connection probe. Unsupported structured output or images must report the actual failure rather than a fabricated success.

For delivery, keep the original package name and signing certificate. Increment the Android version's fourth component and versionCode according to AGENTS.md and the actual current local version. Build a complete signed APK. Verify its signature compatibility, version, size, SHA-256 and corresponding source commit. Follow the user's established APK workflow: keep identical versioned APK copies in the original project's root, APK directory and app/build/outputs/apk/release directory; archive older convenient copies in old_apks, retaining the accepted formal current release until candidate acceptance. Do not create a new desktop folder. Select the APK-directory copy in Explorer for WeChat transfer. Do not substitute an AAB, unsigned APK, debug signature or stale build. If no phone is connected, deliver a clearly identified candidate and list the unverified phone/provider checks.

Perform the AGENTS.md sync-service delivery checks locally without changing unrelated VPNs or tunnels. Verify the existing autostart task, formal service identity, and local/current public health metadata against the formal manifest. Do not equate a running process or HTTP 200 with actual account-sync, reboot or sleep/resume acceptance.

Keep both PRs unmerged until the required local acceptance is complete. The follow-up can first be integrated into the candidate branch; merging that reviewed branch into main remains a separate local Codex step.
