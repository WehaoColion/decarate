Version 2.21.24 - mark the 2.21.23 acceptance contract as a historical template.

Status: historical contract for the unreleased 2.21.23 target. Its example evidence fields intentionally remain unfilled and are not the current release gate. The current candidate record is `release_acceptance_v2.21.34.md`.

# Sync Identity Rebind Acceptance and Verification Contract

## 1. Purpose and Release Target

This document defines the release gate for recovering an authenticated sync workspace after the server instance identity changes. It supplements, but does not replace, `development_spec_v2.19.2.md`.

The target post-fix release is `2.21.23-sync-identity-rebind-review-fix`. Until the release artifacts are built, the exact version, hashes, test counts, timestamps, and runtime observations in Section 9 remain explicitly marked `PENDING`. A pending evidence field is not a pass.

The contract has four goals:

1. Recover a generation-zero workspace without losing local or server data.
2. Reject identity drift outside the narrow recovery case.
3. Make every persistence failure leave the previously committed workspace active and recoverable.
4. Produce executable Android and Windows evidence rather than relying only on generated-source string checks.

## 2. Component Scope

| Component | Required responsibility |
| --- | --- |
| Android client | Detect the eligible mismatch during normal sync, validate the rebind response, merge the server baseline with current local data, preserve the active workspace storage key, durably persist the merged snapshot and new session, survive restart, and acknowledge the restore receipt only after the local transition is durable. |
| Windows client | Detect the eligible mismatch during normal sync, validate the rebind response against the request identity, prepare the new account-identity state scope, durably persist the merged snapshot and sync session, switch the active scope only after both commits succeed, survive restart, and acknowledge the restore receipt only after the transition is durable. |
| Sync server | Return an identity mismatch without account data for an ordinary request bound to an old server identity; accept a rebind request only under the generation-zero rules in this document; return a baseline snapshot, restore receipt, current account identity, and unchanged workspace ID; reject ineligible, forced, stale, or malformed requests without consuming a receipt or changing committed account data. |
| Sync launcher | Start or reuse the intended post-fix server build, expose a health result whose build ID matches the release, preserve the configured public/local route, and never rewrite client workspace identity or application data itself. |

This change does not authorize an automatic migration for a nonzero restore generation, a previously acknowledged restore barrier, forced upload, forced download, account switching, token switching, or an unauthenticated session.

## 3. Terms and Invariants

- **Previous identity** means the `serverInstanceId` and derived `accountNamespace` stored when the normal sync request starts.
- **Current identity** means the server identity and derived account namespace returned after the server instance changes.
- **Workspace identity** means the stable `workspaceId` and its capability proof.
- **Generation zero** means that both the client-acknowledged generation and the server current generation are exactly `0`.
- **Baseline response** means an `ok` response in `baseline_required` mode with a server snapshot, a nonempty restore receipt, `restoreRequired`, `baselineMergeRequired`, `currentCommitted`, and `workspaceIdentityRebound` all true.
- **Active workspace** means the session, state path or storage key, and application snapshot that the client will load after restart.

The following invariants are mandatory:

1. The authenticated user and bearer-token identity must not change during rebind.
2. The current account namespace must be deterministically derived from the current server instance ID and the authenticated user ID.
3. The previous account namespace must be deterministically derived from the previous server instance ID and the same user ID.
4. The previous and current server identities must both be structurally valid and must differ.
5. The workspace ID must remain unchanged across rebind.
6. The baseline response must not return a workspace proof. A new proof is issued only through the later receipt-acknowledgement flow.
7. The local data present when sync starts and the server baseline data must both be represented in the committed merged snapshot.
8. A response may be applied only if the still-active local session and workspace checkpoint match the request expectation that started the operation.
9. A failed write must not activate the new identity, discard the previous identity, acknowledge the receipt, or replace the previously committed local data.

## 4. Automatic Rebind Allow Matrix

Every row in this table is required. Failure of any one row converts the operation to the rejected path in Section 5.

| Area | Required allow condition |
| --- | --- |
| Authentication | The client is logged in; the response user ID equals the request user ID; the response token ID equals the identifier derived from the request bearer token; any returned bearer token resolves to the same token ID. |
| Operation | The operation is ordinary merge sync. It is not registration, login, health, forced local upload, forced account download, account deletion, or another mutation. |
| Initial mismatch | The first response is not successful, is nonretryable `workspace_binding_mismatch`, contains no application snapshot, receipt, workspace ID, or workspace proof, does not claim a committed current state, restore, baseline merge, or identity rebind, and reports server generation `0`. |
| Previous identity | The stored previous server instance ID and account namespace are structurally valid; the namespace is derived from that server ID and the authenticated user; both values match the request expectation. |
| Current identity | The mismatch response contains a structurally valid current server instance ID and account namespace; the namespace is derived from that server ID and the same authenticated user; both current identity values differ from the previous identity. |
| Workspace checkpoint | The stored workspace ID and previous proof are present, structurally valid, and match the request expectation. The workspace ID remains unchanged in the rebind result. |
| Generation barrier | The request acknowledged generation is exactly `0`; the server current generation is exactly `0`; the server token restore barrier has not previously been acknowledged. |
| Receipt state | The local session has no pending restore receipt before rebind. The initial mismatch contains no receipt. |
| Server activation state | The authenticated server account is not pending activation. |
| Rebind request | The second request targets the current server identity, carries the unchanged workspace ID, uses an empty workspace proof and empty restore receipt, sets the explicit identity-rebind flag, supplies the previous identity, and does not set forced upload or forced download. |
| Baseline response | The second response is successful and nonretryable; its mode is `baseline_required`; `currentCommitted`, `restoreRequired`, `baselineMergeRequired`, and `workspaceIdentityRebound` are true; generation is `0`; application data and a structurally valid nonempty receipt are present. |
| Baseline identity | The baseline response user ID, token ID, current server instance ID, and current account namespace exactly match the validated mismatch and request identities. |
| Baseline workspace | The baseline response workspace ID equals the previous workspace ID and its workspace proof is empty. |
| Request race guard | Immediately before applying the result, the active user, token ID, previous server identity, previous namespace, generation, workspace ID, and previous proof still equal the expectation captured when the request started. |
| Target-scope guard | A pre-existing target identity scope must not contain a conflicting workspace checkpoint. If compatible target data exists, it must be merged without dropping either side. |

## 5. Automatic Rebind Reject Matrix

The client and server must reject automatic rebind for each condition below. Rejection must leave the active local snapshot and identity unchanged.

| Reject condition | Required result |
| --- | --- |
| Logged out, different user, different token ID, or returned token mismatch | Reject as an authentication or response-identity error. Do not apply response data. |
| Registration, login, health, forced upload, forced download, deletion, or any operation other than normal merge sync | Do not enter automatic rebind. |
| Previous or current server identity is missing, malformed, not derived from the authenticated user, or unchanged | Reject the response or request. |
| Initial mismatch is retryable, successful, carries account data, carries workspace capability data, or claims another transition | Do not issue the rebind request. |
| Client acknowledged generation is negative or nonzero | Reject automatic rebind. |
| Server generation is nonzero | Reject automatic rebind. |
| The token restore barrier was already acknowledged | Reject automatic rebind. |
| A restore receipt is already pending locally or appears in the initial mismatch | Reject automatic rebind. |
| Workspace ID or previous proof is absent, malformed, or no longer matches the request checkpoint | Reject automatic rebind. |
| Baseline response omits application data or a valid receipt | Reject the response. |
| Baseline response lacks any required flag, uses a mode other than `baseline_required`, returns nonzero generation, or claims retryability | Reject the response. |
| Baseline response changes the workspace ID or returns a workspace proof before acknowledgement | Reject the response. |
| Active local session or checkpoint changes while the request is in flight | Reject as stale. The newer local session remains active. |
| A target identity scope contains an incompatible workspace checkpoint | Reject the transition and require explicit recovery. |
| Snapshot, state-store, session, secret-store, or journal persistence fails | Roll back the activation attempt. Keep the previous committed session, state path or storage key, receipt state, and application data active. |
| A response claims `workspaceIdentityRebound` without satisfying every allow condition | Treat it as forged or malformed and reject it. |

## 6. Generation-Zero-Only Safety Boundary

Automatic identity rebind is deliberately limited to generation zero. This is a safety boundary, not an incomplete migration promise.

At generation zero, no later restore generation has been acknowledged for the token. The server can require a baseline merge and receipt acknowledgement before issuing the next workspace proof. Once a generation is nonzero, or a restore barrier has already been acknowledged, silently changing account identity could cross an established rollback boundary or attach local state to the wrong recovery history. That case requires an explicit user-driven recovery.

The client must not weaken this boundary by retrying with generation reset to zero, clearing an existing receipt, converting the action into forced upload or forced download, or signing the user out automatically.

## 7. Durable Transition and No-Data-Loss Semantics

The transition is accepted only when all of the following stages complete in order:

1. Capture the request identity and workspace checkpoint.
2. Validate the mismatch and the baseline response against the allow matrix.
3. Merge the server baseline with the current local application snapshot in memory.
4. Sanitize and validate the merged snapshot.
5. Durably write the merged snapshot to the intended workspace storage.
6. Durably write the new current identity, unchanged workspace ID, empty proof, and pending receipt to the session and secret stores.
7. Activate the new identity scope in memory.
8. On a later normal sync, acknowledge the pending receipt and accept the new workspace proof.

The following rules apply to failures:

- Failure before Stage 7 leaves the previous identity and previous local snapshot active in memory and after restart.
- The pending receipt must not be acknowledged before Stages 5 and 6 are durable.
- A partially prepared target file or database row must never be selected merely because it exists. It may be ignored and cleaned later after its ownership is proven.
- The server must not discard or overwrite the committed account baseline when an ineligible or malformed rebind request is rejected.
- Successful rebind must preserve local-only records and server-only records. Equality of only summary counts is insufficient; tests must verify unique record markers from both sides.
- Restart is part of the transaction test. In-memory success without a successful reload from durable storage is a failure.

## 8. Actionable Recovery for Ineligible Workspaces

When automatic rebind is rejected because the generation is nonzero, a receipt is already pending, the restore barrier was acknowledged, or the target scope conflicts, the client must show a concise explanation equivalent to:

> The server identity changed, and this workspace cannot be migrated automatically. Your local data is still retained. Sign out, sign in again, then use Download Account Data to recover the account workspace.

The recovery flow is:

1. Stop automatic retries for the rejected transition.
2. Keep the current local data, workspace scope, and diagnostic details intact.
3. Let the user sign out explicitly.
4. Sign in to the intended account against the current server.
5. Use the explicit **Download Account Data** action to establish the current account namespace.
6. Confirm the downloaded workspace opens correctly before removing or archiving any previous local scope.

The interface must not report success, silently force-upload the old local snapshot, or imply that local data was deleted. If explicit download cannot complete, the old local data remains available for export and diagnosis.

## 9. Verification Matrix

Each test must record its command or harness, isolated data root, result, and evidence path. Android behavior tests must execute on an emulator or device and exercise production persistence code generated from the Rust source tree.

| ID | Platform | Scenario | Required assertions |
| --- | --- | --- | --- |
| IR-SRV-01 | Server and shared sync core | Eligible normal sync receives mismatch, then requests generation-zero rebind | First response exposes no account data; second response satisfies every baseline flag and identity invariant. |
| IR-SRV-02 | Server and shared sync core | Rebind followed by receipt acknowledgement | Receipt is issued once, acknowledgement returns the new proof, and the token restore barrier becomes acknowledged only after confirmation. |
| IR-SRV-03 | Server and shared sync core | Reject matrix parameter test | Forced operations, nonzero generations, pending activation, existing receipt, malformed identity, malformed workspace, and stale barrier are rejected without committed data mutation. |
| IR-WIN-01 | Windows client | Successful old-identity-to-new-identity transition | Local-only and server-only markers are merged; workspace ID is unchanged; identity changes; proof is empty until acknowledgement; receipt is durable. |
| IR-WIN-02 | Windows client | Restart after successful rebind | Reload selects the new identity scope and merged data, with the pending receipt intact. |
| IR-WIN-03 | Windows client | Forged or malformed rebind response | Validation fails and previous identity, state path, checkpoint, and data remain active. |
| IR-WIN-04 | Windows client | State-store or snapshot write failure | Apply fails; the previous session and previous durable snapshot reload unchanged. |
| IR-WIN-05 | Windows client | Session or secret-store write failure after target preparation | Apply fails; the old session remains authoritative after restart; any partial target state is not activated. |
| IR-WIN-06 | Windows client | Nonzero generation, force upload, and force download | Automatic rebind is not attempted and local data remains unchanged. |
| IR-AND-01 | Android instrumentation | Successful identity rebind with offline local modification and distinct server baseline | Production binding, merge, database persistence, and secure session persistence retain unique markers from both sides. |
| IR-AND-02 | Android instrumentation | Application restart after rebind | Reload from the production database and secure session store selects the current identity while retaining the pinned workspace storage and merged snapshot. |
| IR-AND-03 | Android instrumentation | Injected snapshot or session persistence failure | The old identity and old local snapshot remain loadable; the new identity is not activated; receipt is not acknowledged. |
| IR-AND-04 | Android instrumentation | Forged response and reject-matrix cases | Each invalid identity, generation, receipt, workspace, flag, or operation fails closed without data mutation. |
| IR-E2E-01 | Launcher and server | Launch the packaged post-fix server in an isolated data root | Health returns HTTP 200, exact post-fix build ID, and healthy backup/database status. |
| IR-E2E-02 | Android or Windows client with isolated server | Complete mismatch, rebind, restart, and second-sync acknowledgement | The full transition survives restart; second sync stores the new proof and clears the receipt; both local and server markers remain. |
| IR-E2E-03 | Public route | Query the configured public health route without changing VPN or proxy state | Public health reaches the same server build as local health. |

## 10. Current Post-Fix Evidence Record

The main repair thread must replace every `PENDING` field below with current evidence from the exact release candidate. Historical logs from earlier versions do not satisfy this gate.

| Evidence | Required value |
| --- | --- |
| Release version | `PENDING` - expected `2.21.23-sync-identity-rebind-review-fix` unless the main repair thread records a later version consistently across all components. |
| Git revision or worktree fingerprint | `PENDING` |
| Verification timestamp and timezone | `PENDING` |
| Rust library tests | `PENDING` - record passed, failed, ignored, and filtered counts. |
| Rust binary tests | `PENDING` - record passed, failed, ignored, and filtered counts. |
| Windows client tests | `PENDING` - include IR-WIN-01 through IR-WIN-06. |
| Android host tests | `PENDING` |
| Android instrumentation device or AVD | `PENDING` - record exact model or AVD, API level, ABI, and clean install or upgrade path. |
| Android instrumentation result | `PENDING` - include IR-AND-01 through IR-AND-04. |
| Source generation and source audit | `PENDING` |
| Formatting, diff whitespace, and static checks | `PENDING` |
| Formal APK filename | `PENDING` |
| APK version code and version name | `PENDING` |
| APK SHA-256 | `PENDING` |
| APK signing certificate digest | `PENDING` |
| APK zip alignment | `PENDING` |
| Windows client filename and SHA-256 | `PENDING` |
| Server filename and SHA-256 | `PENDING` |
| Launcher filename and SHA-256 | `PENDING` |
| Isolated local health result | `PENDING` |
| Isolated database integrity result | `PENDING` |
| Public health result | `PENDING` |
| Full rebind runtime evidence | `PENDING` - reference IR-E2E-02 output. |
| Independent reviewer verdict | `PENDING` |
| Evidence directory | `PENDING` |

## 11. Release Gates

The release passes only when all gates below are satisfied:

1. **Requirements completeness:** Android, Windows, server, and launcher behavior matches this document, including explicit recovery for deliberately unsupported cases.
2. **Logic correctness:** Every allow condition is enforced, every reject condition fails closed, the response is bound to the request expectation, and receipt acknowledgement occurs only after durable transition.
3. **Boundary coverage:** Nonzero generation, pending receipt, acknowledged barrier, forced operations, malformed or forged responses, stale in-flight requests, target-scope conflict, and persistence failures have executable tests.
4. **Code quality:** Formatting and whitespace checks pass; new warnings are removed or explicitly justified; generated Android test sources originate from Rust; production behavior is not duplicated only inside tests.
5. **Test coverage:** Current Rust, Windows, Android host, and Android instrumentation suites pass. Generated-source string assertions alone do not satisfy Android behavior coverage.
6. **Actual runtime:** An isolated packaged server and launcher report the exact release build; at least one client completes rebind, restart, and receipt acknowledgement; local and public health checks reach the same build without disabling or changing VPN or proxy state.
7. **Artifact integrity:** The formal APK is versioned, signed, zip-aligned, and hash-recorded. Release directories contain no debug APK, Xiaomi debug APK, unversioned `app-release.apk`, or AAB file. Older formal APKs are moved to `old_apks/`.
8. **Version consistency:** APK, server, launcher, Windows client, health output, evidence record, and filenames identify one post-fix release version.
9. **Independent review:** The independent review thread rechecks all six review dimensions against current code, tests, artifacts, and runtime evidence and returns an unqualified pass.

If a gate cannot be completed, the release remains blocked. The evidence record must identify the exact failing test, component, error, last confirmed safe state, and next required action rather than substituting an earlier successful result.
