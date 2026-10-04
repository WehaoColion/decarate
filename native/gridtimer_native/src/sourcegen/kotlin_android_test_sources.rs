// Rust-owned Android instrumentation-test templates emitted into Gradle's generated
// androidTest source set. Keep executable Android behavior tests here so production
// and test Kotlin remain generated from the Rust source tree.

pub struct KotlinAndroidTestSource {
    pub path: &'static str,
    pub contents: &'static str,
}

pub const SOURCES: &[KotlinAndroidTestSource] = &[
    KotlinAndroidTestSource {
        path: "com/ofairyo/gridtimer/data/WorkspaceIdentityRebindInstrumentedTest.kt",
        contents: r####"
package com.ofairyo.gridtimer.data

import android.content.Context
import android.content.ContextWrapper
import android.system.Os
import android.system.OsConstants
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.security.MessageDigest
import java.util.UUID
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
@OptIn(ExperimentalSerializationApi::class)
class WorkspaceIdentityRebindInstrumentedTest {
    private val json = Json {
        prettyPrint = false
        ignoreUnknownKeys = false
        encodeDefaults = true
        coerceInputValues = false
        explicitNulls = false
    }

    @Test
    fun generationZeroRebindPersistsAcrossRestartAndSecondSync() {
        val targetContext = ApplicationProvider.getApplicationContext<Context>()
        val fixture = fixture("success")
        val sessionContext = isolatedFilesContext(targetContext, fixture.nonce)
        try {
            assertTrue(SecureSyncSessionStore.persist(sessionContext, json, fixture.oldSession))
            writeSnapshot(targetContext, fixture.workspaceStorageKey, fixture.localSnapshot, 100L)

            val baselineResult = fixture.baselineResult()
            val reboundBinding = WorkspaceIdentityRebindContract.bindAuthenticatedResponse(
                working = fixture.oldSession,
                result = baselineResult,
                activeWorkspaceKey = fixture.workspaceStorageKey,
                allowNamespaceBootstrap = false,
                allowWorkspaceIdentityRebind = true
            )
            assertNotEquals(fixture.oldServerInstanceId, reboundBinding.serverInstanceId)
            assertEquals(fixture.workspaceStorageKey, reboundBinding.workspaceStorageKey)

            val remoteSnapshot = json.decodeFromString<AppData>(
                requireNotNull(baselineResult.appDataJson)
            )
            val mergedSnapshot =
                TimerRepository.AccountSnapshotMerge.mergeAccountSnapshotWithCurrent(
                    fixture.localSnapshot,
                    remoteSnapshot
                )
            writeSnapshot(targetContext, fixture.workspaceStorageKey, mergedSnapshot, 200L)
            val reboundSession = reboundBinding.copy(
                workspaceId = fixture.workspaceId,
                workspaceProof = "",
                acknowledgedGeneration = 0L,
                restoreReceipt = fixture.restoreReceipt,
                lastMessage = "Identity rebound baseline stored"
            )
            assertTrue(SecureSyncSessionStore.persist(sessionContext, json, reboundSession))

            val restartedSession = requireNotNull(
                SecureSyncSessionStore.load(sessionContext, json)
            )
            val restartedSnapshot = readSnapshot(targetContext, fixture.workspaceStorageKey)
            assertEquals(fixture.newServerInstanceId, restartedSession.serverInstanceId)
            assertEquals(fixture.newAccountNamespace, restartedSession.accountNamespace)
            assertEquals(fixture.workspaceStorageKey, restartedSession.workspaceStorageKey)
            assertEquals(fixture.restoreReceipt, restartedSession.restoreReceipt)
            assertContainsNote(restartedSnapshot, fixture.localNoteId)
            assertContainsNote(restartedSnapshot, fixture.remoteNoteId)

            val confirmationResult = fixture.confirmationResult(mergedSnapshot)
            val confirmedBinding = WorkspaceIdentityRebindContract.bindAuthenticatedResponse(
                working = restartedSession,
                result = confirmationResult,
                activeWorkspaceKey = fixture.workspaceStorageKey,
                allowNamespaceBootstrap = false,
                allowWorkspaceIdentityRebind = true
            )
            val confirmedSession = confirmedBinding.copy(
                workspaceId = fixture.workspaceId,
                workspaceProof = fixture.newWorkspaceProof,
                acknowledgedGeneration = 0L,
                restoreReceipt = "",
                lastMessage = "Identity rebound confirmed"
            )
            assertTrue(SecureSyncSessionStore.persist(sessionContext, json, confirmedSession))

            val secondRestart = requireNotNull(
                SecureSyncSessionStore.load(sessionContext, json)
            )
            val secondRestartSnapshot = readSnapshot(targetContext, fixture.workspaceStorageKey)
            assertEquals(fixture.newServerInstanceId, secondRestart.serverInstanceId)
            assertEquals(fixture.newAccountNamespace, secondRestart.accountNamespace)
            assertEquals(fixture.newWorkspaceProof, secondRestart.workspaceProof)
            assertEquals("", secondRestart.restoreReceipt)
            assertContainsNote(secondRestartSnapshot, fixture.localNoteId)
            assertContainsNote(secondRestartSnapshot, fixture.remoteNoteId)
        } finally {
            sessionContext.root.deleteRecursively()
        }
    }

    @Test
    fun sessionPersistenceFailureKeepsOldIdentityAndLocalData() {
        val targetContext = ApplicationProvider.getApplicationContext<Context>()
        val fixture = fixture("failure")
        val sessionContext = isolatedFilesContext(targetContext, fixture.nonce)
        try {
            assertTrue(SecureSyncSessionStore.persist(sessionContext, json, fixture.oldSession))
            writeSnapshot(targetContext, fixture.workspaceStorageKey, fixture.localSnapshot, 300L)
            val originalCiphertext = sessionContext.secureSessionFile().readBytes()

            val baselineResult = fixture.baselineResult()
            val reboundBinding = WorkspaceIdentityRebindContract.bindAuthenticatedResponse(
                working = fixture.oldSession,
                result = baselineResult,
                activeWorkspaceKey = fixture.workspaceStorageKey,
                allowNamespaceBootstrap = false,
                allowWorkspaceIdentityRebind = true
            )
            val remoteSnapshot = json.decodeFromString<AppData>(
                requireNotNull(baselineResult.appDataJson)
            )
            val mergedSnapshot =
                TimerRepository.AccountSnapshotMerge.mergeAccountSnapshotWithCurrent(
                    fixture.localSnapshot,
                    remoteSnapshot
                )
            // Match production ordering: the merged snapshot becomes durable before the
            // encrypted identity is published. A later identity-write failure must still
            // leave the previous durable identity and all local data recoverable.
            writeSnapshot(targetContext, fixture.workspaceStorageKey, mergedSnapshot, 400L)
            val reboundSession = reboundBinding.copy(
                workspaceId = fixture.workspaceId,
                workspaceProof = "",
                acknowledgedGeneration = 0L,
                restoreReceipt = fixture.restoreReceipt
            )

            val filesDirectory = sessionContext.filesDir
            Os.chmod(
                filesDirectory.absolutePath,
                OsConstants.S_IRUSR or OsConstants.S_IXUSR
            )
            val persisted = try {
                SecureSyncSessionStore.persist(sessionContext, json, reboundSession)
            } finally {
                Os.chmod(
                    filesDirectory.absolutePath,
                    OsConstants.S_IRUSR or OsConstants.S_IWUSR or OsConstants.S_IXUSR
                )
            }
            assertFalse(persisted)
            assertTrue(originalCiphertext.contentEquals(sessionContext.secureSessionFile().readBytes()))

            val restartedSession = requireNotNull(
                SecureSyncSessionStore.load(sessionContext, json)
            )
            val restartedSnapshot = readSnapshot(targetContext, fixture.workspaceStorageKey)
            assertEquals(fixture.oldServerInstanceId, restartedSession.serverInstanceId)
            assertEquals(fixture.oldAccountNamespace, restartedSession.accountNamespace)
            assertEquals(fixture.oldWorkspaceProof, restartedSession.workspaceProof)
            assertEquals("", restartedSession.restoreReceipt)
            assertContainsNote(restartedSnapshot, fixture.localNoteId)
        } finally {
            runCatching {
                Os.chmod(
                    sessionContext.filesDir.absolutePath,
                    OsConstants.S_IRUSR or OsConstants.S_IWUSR or OsConstants.S_IXUSR
                )
            }
            sessionContext.root.deleteRecursively()
        }
    }

    @Test
    fun forgedAndUnsafeRebindVariantsFailClosedWithoutDataMutation() {
        val targetContext = ApplicationProvider.getApplicationContext<Context>()
        val fixture = fixture("reject-matrix")
        val sessionContext = isolatedFilesContext(targetContext, fixture.nonce)
        try {
            assertTrue(SecureSyncSessionStore.persist(sessionContext, json, fixture.oldSession))
            writeSnapshot(targetContext, fixture.workspaceStorageKey, fixture.localSnapshot, 500L)
            val originalCiphertext = sessionContext.secureSessionFile().readBytes()
            val valid = fixture.baselineResult()
            val newIdentityStorageKey = accountWorkspaceStorageKey(
                fixture.newServerInstanceId,
                fixture.newAccountNamespace,
                fixture.userId
            )
            val cases = listOf(
                RebindRejectCase(
                    "retryable response",
                    fixture.oldSession,
                    valid.copy(retryable = true),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "missing committed flag",
                    fixture.oldSession,
                    valid.copy(currentCommitted = false),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "missing restore flag",
                    fixture.oldSession,
                    valid.copy(restoreRequired = false),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "missing baseline flag",
                    fixture.oldSession,
                    valid.copy(baselineMergeRequired = false),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "wrong mode",
                    fixture.oldSession,
                    valid.copy(mode = "merged"),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "nonzero generation",
                    fixture.oldSession,
                    valid.copy(currentGeneration = 1L),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "missing snapshot",
                    fixture.oldSession,
                    valid.copy(appDataJson = null),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "malformed receipt",
                    fixture.oldSession,
                    valid.copy(restoreReceipt = "not-a-valid-receipt"),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "changed workspace",
                    fixture.oldSession,
                    valid.copy(workspaceId = sha256("changed-workspace-${fixture.nonce}")),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "premature proof",
                    fixture.oldSession,
                    valid.copy(workspaceProof = fixture.newWorkspaceProof),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "different user",
                    fixture.oldSession,
                    valid.copy(userId = "other-${fixture.userId}"),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "different token",
                    fixture.oldSession,
                    valid.copy(tokenId = sha256("other-token-${fixture.nonce}").take(32)),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "namespace derivation mismatch",
                    fixture.oldSession,
                    valid.copy(accountNamespace = fixture.oldAccountNamespace),
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "nonzero local acknowledgement",
                    fixture.oldSession.copy(acknowledgedGeneration = 1L),
                    valid,
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "pending local receipt",
                    fixture.oldSession.copy(
                        restoreReceipt = sha256("pending-${fixture.nonce}")
                    ),
                    valid,
                    fixture.workspaceStorageKey
                ),
                RebindRejectCase(
                    "stale active workspace",
                    fixture.oldSession,
                    valid,
                    newIdentityStorageKey
                ),
                RebindRejectCase(
                    "forced-operation gate",
                    fixture.oldSession,
                    valid,
                    fixture.workspaceStorageKey,
                    allowIdentityRebind = false
                ),
                RebindRejectCase(
                    "stable identity forged rebind claim",
                    fixture.oldSession,
                    valid.copy(
                        serverInstanceId = fixture.oldServerInstanceId,
                        accountNamespace = fixture.oldAccountNamespace
                    ),
                    fixture.workspaceStorageKey
                )
            )

            cases.forEach { case ->
                val rejected = runCatching {
                    WorkspaceIdentityRebindContract.bindAuthenticatedResponse(
                        working = case.working,
                        result = case.result,
                        activeWorkspaceKey = case.activeWorkspaceKey,
                        allowNamespaceBootstrap = false,
                        allowWorkspaceIdentityRebind = case.allowIdentityRebind
                    )
                }
                assertTrue("${case.label} must fail closed", rejected.isFailure)
            }

            assertTrue(originalCiphertext.contentEquals(sessionContext.secureSessionFile().readBytes()))
            val restartedSession = requireNotNull(
                SecureSyncSessionStore.load(sessionContext, json)
            )
            val restartedSnapshot = readSnapshot(targetContext, fixture.workspaceStorageKey)
            assertEquals(fixture.oldServerInstanceId, restartedSession.serverInstanceId)
            assertEquals(fixture.oldAccountNamespace, restartedSession.accountNamespace)
            assertEquals(fixture.oldWorkspaceProof, restartedSession.workspaceProof)
            assertContainsNote(restartedSnapshot, fixture.localNoteId)
            assertFalse(restartedSnapshot.notes.any { note -> note.id == fixture.remoteNoteId })
        } finally {
            sessionContext.root.deleteRecursively()
        }
    }

    @Test
    fun unsafeIdentityBoundaryHasOneActionableMessage() {
        val expected =
            "服务器身份已变化，自动迁移仅适用于第0代安全工作区。本机数据已保留，请退出后重新登录，再使用“下载账户数据”。"
        assertEquals(
            expected,
            workspaceIdentityBoundaryStatusMessage("workspace_binding_mismatch")
        )
        assertEquals(
            expected,
            workspaceIdentityBoundaryStatusMessage("workspace_identity_rebind_invalid")
        )
        assertEquals(null, workspaceIdentityBoundaryStatusMessage("unchanged"))
    }

    private fun fixture(label: String): RebindFixture {
        val nonce = "$label-${UUID.randomUUID()}"
        val userId = "review-user-$nonce"
        val token = "review-token-$nonce"
        val tokenId = sha256(token).take(32)
        val oldServerInstanceId = sha256("old-server-$nonce")
        val newServerInstanceId = sha256("new-server-$nonce")
        val oldAccountNamespace = accountNamespaceIdentifier(oldServerInstanceId, userId)
        val newAccountNamespace = accountNamespaceIdentifier(newServerInstanceId, userId)
        val workspaceId = sha256("workspace-$nonce")
        val oldWorkspaceProof = sha256("old-proof-$nonce")
        val newWorkspaceProof = sha256("new-proof-$nonce")
        val restoreReceipt = sha256("receipt-$nonce")
        val workspaceStorageKey = accountWorkspaceStorageKey(
            oldServerInstanceId,
            oldAccountNamespace,
            userId
        )
        val localNoteId = "local-$nonce"
        val remoteNoteId = "remote-$nonce"
        val localSnapshot = AppData.default().copy(
            notes = listOf(
                NoteEntry(
                    id = localNoteId,
                    title = "Local offline edit",
                    content = "local-marker-$nonce",
                    createdAtEpochMillis = 100L,
                    updatedAtEpochMillis = 100L
                )
            )
        )
        val remoteSnapshot = AppData.default().copy(
            notes = listOf(
                NoteEntry(
                    id = remoteNoteId,
                    title = "Server baseline edit",
                    content = "remote-marker-$nonce",
                    createdAtEpochMillis = 200L,
                    updatedAtEpochMillis = 200L
                )
            )
        )
        return RebindFixture(
            nonce = nonce,
            userId = userId,
            token = token,
            tokenId = tokenId,
            oldServerInstanceId = oldServerInstanceId,
            newServerInstanceId = newServerInstanceId,
            oldAccountNamespace = oldAccountNamespace,
            newAccountNamespace = newAccountNamespace,
            workspaceId = workspaceId,
            oldWorkspaceProof = oldWorkspaceProof,
            newWorkspaceProof = newWorkspaceProof,
            restoreReceipt = restoreReceipt,
            workspaceStorageKey = workspaceStorageKey,
            localNoteId = localNoteId,
            remoteNoteId = remoteNoteId,
            localSnapshot = localSnapshot,
            remoteSnapshot = remoteSnapshot
        )
    }

    private fun writeSnapshot(
        context: Context,
        workspaceKey: String,
        data: AppData,
        revision: Long
    ) {
        val encoded = json.encodeToString(data)
        val database = AppStateDatabase(context)
        try {
            database.writeSnapshot(
                workspaceKey = workspaceKey,
                appDataJson = encoded,
                schemaVersion = APP_DATA_SCHEMA_VERSION,
                revision = revision,
                itemCount = data.categories.size +
                    data.slots.size +
                    data.sessions.size +
                    data.archivedTasks.size +
                    data.notes.size,
                now = revision
            )
        } finally {
            database.close()
        }
    }

    private fun readSnapshot(context: Context, workspaceKey: String): AppData {
        val database = AppStateDatabase(context)
        return try {
            val stored = database.readSnapshots(workspaceKey)
                .first { snapshot -> snapshot.source == "sqlite-current" }
            json.decodeFromString(stored.appDataJson)
        } finally {
            database.close()
        }
    }

    private fun assertContainsNote(data: AppData, noteId: String) {
        assertTrue(data.notes.any { note -> note.id == noteId })
    }

    private fun isolatedFilesContext(
        base: Context,
        nonce: String
    ): IsolatedFilesContext {
        val root = File(base.cacheDir, "identity_rebind_android_test/$nonce")
        root.deleteRecursively()
        return IsolatedFilesContext(base, root)
    }

    private fun sha256(value: String): String {
        return MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray(Charsets.UTF_8))
            .joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }
    }

    private data class RebindFixture(
        val nonce: String,
        val userId: String,
        val token: String,
        val tokenId: String,
        val oldServerInstanceId: String,
        val newServerInstanceId: String,
        val oldAccountNamespace: String,
        val newAccountNamespace: String,
        val workspaceId: String,
        val oldWorkspaceProof: String,
        val newWorkspaceProof: String,
        val restoreReceipt: String,
        val workspaceStorageKey: String,
        val localNoteId: String,
        val remoteNoteId: String,
        val localSnapshot: AppData,
        val remoteSnapshot: AppData
    ) {
        val oldSession: SyncAccountSession
            get() = SyncAccountSession(
                email = "$userId@example.test",
                userId = userId,
                serverInstanceId = oldServerInstanceId,
                accountNamespace = oldAccountNamespace,
                token = token,
                tokenId = tokenId,
                acknowledgedGeneration = 0L,
                restoreReceipt = "",
                workspaceId = workspaceId,
                workspaceProof = oldWorkspaceProof,
                workspaceStorageKey = workspaceStorageKey
            )

        fun baselineResult(): SyncNetworkResult = SyncNetworkResult(
            ok = true,
            message = "Workspace baseline required.",
            userId = userId,
            serverInstanceId = newServerInstanceId,
            accountNamespace = newAccountNamespace,
            tokenId = tokenId,
            appDataJson = Json {
                prettyPrint = false
                ignoreUnknownKeys = false
                encodeDefaults = true
                coerceInputValues = false
                explicitNulls = false
            }.encodeToString(remoteSnapshot),
            currentGeneration = 0L,
            restoreRequired = true,
            baselineMergeRequired = true,
            restoreReceipt = restoreReceipt,
            currentCommitted = true,
            workspaceId = workspaceId,
            workspaceProof = "",
            mode = "baseline_required",
            workspaceIdentityRebound = true
        )

        fun confirmationResult(snapshot: AppData): SyncNetworkResult = SyncNetworkResult(
            ok = true,
            message = "Account data is current.",
            userId = userId,
            serverInstanceId = newServerInstanceId,
            accountNamespace = newAccountNamespace,
            tokenId = tokenId,
            appDataJson = Json {
                prettyPrint = false
                ignoreUnknownKeys = false
                encodeDefaults = true
                coerceInputValues = false
                explicitNulls = false
            }.encodeToString(snapshot),
            currentGeneration = 0L,
            restoreRequired = false,
            baselineMergeRequired = false,
            restoreReceipt = "",
            currentCommitted = true,
            workspaceId = workspaceId,
            workspaceProof = newWorkspaceProof,
            mode = "unchanged",
            workspaceIdentityRebound = false
        )
    }

    private data class RebindRejectCase(
        val label: String,
        val working: SyncAccountSession,
        val result: SyncNetworkResult,
        val activeWorkspaceKey: String,
        val allowIdentityRebind: Boolean = true
    )

    private class IsolatedFilesContext(
        base: Context,
        val root: File
    ) : ContextWrapper(base) {
        override fun getApplicationContext(): Context = this

        override fun getFilesDir(): File {
            return File(root, "files").apply {
                check(exists() || mkdirs()) { "Could not create isolated files directory." }
            }
        }

        fun secureSessionFile(): File = File(filesDir, "sync_account.secure")
    }
}
"####,
    },
    KotlinAndroidTestSource {
        path: "com/ofairyo/gridtimer/core/NoteTextFormatterInstrumentedTest.kt",
        contents: r####"
package com.ofairyo.gridtimer.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NoteTextFormatterInstrumentedTest {
    @Test
    fun todoCyclesBetweenUncheckedAndCompletedWithoutLosingUnicodeOrCursor() {
        val source = "中文🙂任务"
        val unchecked = NoteTextFormatter.apply(
            action = NoteTextAction.TODO,
            content = source,
            selectionStart = source.length,
            selectionEnd = source.length
        )
        assertEquals("- [ ] $source", unchecked.content)
        assertEquals("- [ ] ".length + source.length, unchecked.selectionStart)
        assertEquals(unchecked.selectionStart, unchecked.selectionEnd)

        val completed = NoteTextFormatter.apply(
            action = NoteTextAction.TODO,
            content = unchecked.content,
            selectionStart = unchecked.selectionStart,
            selectionEnd = unchecked.selectionEnd
        )
        assertEquals("- [x] $source", completed.content)
        assertEquals(unchecked.selectionStart, completed.selectionStart)

        val reopened = NoteTextFormatter.apply(
            action = NoteTextAction.TODO,
            content = completed.content,
            selectionStart = completed.selectionStart,
            selectionEnd = completed.selectionEnd
        )
        assertEquals(unchecked.content, reopened.content)
        assertEquals(unchecked.selectionStart, reopened.selectionStart)
    }

    @Test
    fun todoTransformsEverySelectedLineAndKeepsTheWholeBlockSelected() {
        val source = "甲🙂\n- [ ] 乙\n- [X] 丙"
        val edit = NoteTextFormatter.apply(
            action = NoteTextAction.TODO,
            content = source,
            selectionStart = 0,
            selectionEnd = source.length
        )
        assertEquals("- [ ] 甲🙂\n- [x] 乙\n- [ ] 丙", edit.content)
        assertEquals(0, edit.selectionStart)
        assertEquals(edit.content.length, edit.selectionEnd)
    }
}
"####,
    },
    KotlinAndroidTestSource {
        path: "com/ofairyo/gridtimer/ui/NoteVersionStackInstrumentedTest.kt",
        contents: r####"
package com.ofairyo.gridtimer.ui

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.ofairyo.gridtimer.data.NoteAttachment
import com.ofairyo.gridtimer.data.NoteBlock
import com.ofairyo.gridtimer.data.NoteBlockType
import com.ofairyo.gridtimer.data.NoteDocument
import com.ofairyo.gridtimer.data.NoteEntry
import com.ofairyo.gridtimer.data.NoteEntryKind
import com.ofairyo.gridtimer.data.NoteVersionSnapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NoteVersionStackInstrumentedTest {
    @Test
    fun unchangedVisitPreservesTimestampBlockIdsAttachmentsAndCollectionOrderSignal() {
        val attachment = NoteAttachment(
            id = "image-a",
            fileName = "a.jpg",
            displayName = "图片 A",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val base = NoteEntry(
            id = "note-a",
            title = "原题",
            content = "正文",
            kind = NoteEntryKind.STICKY,
            document = NoteDocument(
                blocks = listOf(
                    NoteBlock(id = "text-stable", type = NoteBlockType.TEXT, text = "正文"),
                    NoteBlock(
                        id = "image-stable",
                        type = NoteBlockType.IMAGE,
                        attachmentId = attachment.id,
                        caption = attachment.displayName
                    )
                )
            ),
            accentSeed = "amber",
            attachments = listOf(attachment),
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 20L
        )

        val unchanged = base.toSmartisanDraftNote(
            title = "原题",
            plainBody = "正文",
            richBodyHtml = "",
            richTextEnabled = false,
            accentSeed = "amber",
            folderId = null,
            pinned = false,
            attachments = listOf(attachment),
            now = 999L
        )

        assertSame(base, unchanged)
        assertEquals(20L, unchanged.updatedAtEpochMillis)
        assertEquals(listOf("text-stable", "image-stable"), unchanged.document.blocks.map { it.id })
        assertFalse(isSmartisanDraftDirty(base, unchanged))
        assertEquals(SmartisanNoteExitAction.Noop, resolveSmartisanNoteExitAction(base, unchanged))

        val edited = base.toSmartisanDraftNote(
            title = "原题",
            plainBody = "正文已修改",
            richBodyHtml = "",
            richTextEnabled = false,
            accentSeed = "amber",
            folderId = null,
            pinned = false,
            attachments = listOf(attachment),
            now = 999L
        )
        assertEquals(999L, edited.updatedAtEpochMillis)
        assertEquals(listOf("text-stable", "image-stable"), edited.document.blocks.map { it.id })
        assertEquals(listOf(attachment), edited.attachments)
        assertTrue(isSmartisanDraftDirty(base, edited))
        assertEquals(SmartisanNoteExitAction.SaveAndFlush, resolveSmartisanNoteExitAction(base, edited))
    }

    @Test
    fun historicalProjectionUsesExactVersionAttachmentsAndRichMarkup() {
        val historicalAttachment = NoteAttachment(id = "image-old", fileName = "old.jpg")
        val latestAttachment = NoteAttachment(id = "image-new", fileName = "new.jpg")
        val historical = NoteVersionSnapshot(
            id = "version-1",
            noteId = "note-stack",
            sequence = 1L,
            title = "旧版",
            content = "旧版正文",
            document = NoteDocument(
                richTextEnabled = true,
                richTextPlainText = "旧版正文",
                blocks = listOf(
                    NoteBlock(
                        id = "rich-old",
                        type = NoteBlockType.TEXT,
                        text = "<h2>旧版</h2><p><strong>保留格式</strong></p>"
                    )
                )
            ),
            attachmentIds = listOf(historicalAttachment.id),
            attachments = listOf(historicalAttachment),
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val latest = NoteVersionSnapshot(
            id = "version-2",
            noteId = "note-stack",
            sequence = 2L,
            title = "新版",
            content = "新版正文",
            document = NoteDocument(
                blocks = listOf(NoteBlock(id = "latest-text", type = NoteBlockType.TEXT, text = "新版正文"))
            ),
            attachmentIds = listOf(latestAttachment.id),
            attachments = listOf(latestAttachment),
            createdAtEpochMillis = 20L,
            updatedAtEpochMillis = 20L,
            isLatest = true
        )
        val owner = NoteEntry(
            id = "note-stack",
            title = latest.title,
            content = latest.content,
            document = latest.document,
            attachments = latest.attachments,
            versions = listOf(historical, latest),
            latestVersionId = latest.id,
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 20L
        )

        val projected = owner.attachmentPreviewProjection(historical.id)
        assertEquals(listOf(historicalAttachment), projected?.attachments)
        assertEquals(historical.title, projected?.title)
        assertNull(owner.attachmentPreviewProjection("missing-version"))
        assertEquals(owner, owner.attachmentPreviewProjection(null))
        assertEquals(
            "<h2>旧版</h2><p><strong>保留格式</strong></p>",
            historical.historicalDisplayHtml()
        )

        val commonTitleOwner = owner.copy(
            title = "共同标题",
            versions = listOf(
                historical.copy(title = "共同标题"),
                latest.copy(title = "共同标题")
            )
        )
        assertEquals(latest.id, commonTitleOwner.versionIdForSearchQuery("共同标题"))
        assertEquals(historical.id, owner.versionIdForSearchQuery("保留格式"))
        assertEquals(latest.id, owner.versionIdForSearchQuery(""))
    }
}
"####,
    },
    KotlinAndroidTestSource {
        path: "com/ofairyo/gridtimer/ui/PersistenceLifecycleRecoveryInstrumentedTest.kt",
        contents: r####"
package com.ofairyo.gridtimer.ui

import android.content.Context
import android.content.ContextWrapper
import android.content.SharedPreferences
import android.database.DatabaseErrorHandler
import android.database.sqlite.SQLiteDatabase
import android.system.Os
import android.util.Base64
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.ofairyo.gridtimer.core.NativeNoteUpsertResult
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import com.ofairyo.gridtimer.data.AppData
import com.ofairyo.gridtimer.data.AppStateDatabase
import com.ofairyo.gridtimer.data.NoteDraftJournal
import com.ofairyo.gridtimer.data.NoteDraftRecoveryPolicy
import com.ofairyo.gridtimer.data.NoteDraftRecoveryPriority
import com.ofairyo.gridtimer.data.NoteEncryptionEnvelope
import com.ofairyo.gridtimer.data.NoteEntry
import com.ofairyo.gridtimer.data.NoteSaveFailure
import com.ofairyo.gridtimer.data.NoteSaveResult
import com.ofairyo.gridtimer.data.TimerRepository
import com.ofairyo.gridtimer.data.createNextProductVersion
import com.ofairyo.gridtimer.data.effectiveProtectionStateRevision
import com.ofairyo.gridtimer.data.hasAeadCiphertextRecoveryEnvelope
import com.ofairyo.gridtimer.data.repairVersionStack
import java.io.File
import java.util.Collections
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PersistenceLifecycleRecoveryInstrumentedTest {
    @Test
    fun lifecycleShutdownReturnsOnMainThreadThenDrainsFlushesAndClosesInOrder() = runBlocking {
        val events = Collections.synchronizedList(mutableListOf<String>())
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val controller = NoteMutationDrainController(scope) { throwable -> throw throwable }
        val firstStarted = CompletableDeferred<Unit>()
        val releaseFirst = CompletableDeferred<Unit>()
        val lateMutationRan = AtomicBoolean(false)
        try {
            val first = controller.enqueue {
                firstStarted.complete(Unit)
                releaseFirst.await()
                events += "mutation-1"
                NoteSaveResult.committed()
            }
            val second = controller.enqueue {
                events += "mutation-2"
                NoteSaveResult.committed()
            }
            withTimeout(5_000L) { firstStarted.await() }

            lateinit var shutdown: CompletableDeferred<NoteSaveResult>
            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                val startedAt = System.nanoTime()
                shutdown = controller.shutdownAfterDrain(
                    afterClosed = { events += "sessions-closed" },
                    flushOperation = {
                        events += "flush"
                        NoteSaveResult.committed()
                    }
                )
                assertTrue(System.nanoTime() - startedAt < 1_000_000_000L)
            }

            assertFalse(controller.isAcceptingMutations())
            val rejected = controller.enqueue {
                lateMutationRan.set(true)
                NoteSaveResult.committed()
            }
            assertEquals(NoteSaveFailure.QUEUE_UNAVAILABLE, rejected.await().failure)
            releaseFirst.complete(Unit)

            assertTrue(withTimeout(5_000L) { first.await() }.committed)
            assertTrue(withTimeout(5_000L) { second.await() }.committed)
            assertTrue(withTimeout(5_000L) { shutdown.await() }.committed)
            assertEquals(
                listOf("mutation-1", "mutation-2", "flush", "sessions-closed"),
                events.toList()
            )
            assertFalse(lateMutationRan.get())
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun typedReadOnlyAndIoFailuresDoNotSkipLaterAcceptedWritesOrFinalFlush() = runBlocking {
        val events = Collections.synchronizedList(mutableListOf<String>())
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val controller = NoteMutationDrainController(scope) { throwable -> throw throwable }
        try {
            val readOnly = controller.enqueue {
                events += "read-only"
                NoteSaveResult.failed(NoteSaveFailure.READ_ONLY_PROTECTION)
            }
            val ioFailure = controller.enqueue {
                events += "io-failure"
                NoteSaveResult.failed(NoteSaveFailure.DATABASE_WRITE_FAILED)
            }
            val accepted = controller.enqueue {
                events += "accepted"
                NoteSaveResult.committed()
            }
            val shutdown = controller.shutdownAfterDrain {
                events += "flush-failure"
                NoteSaveResult.failed(NoteSaveFailure.FLUSH_FAILED)
            }

            assertEquals(NoteSaveFailure.READ_ONLY_PROTECTION, readOnly.await().failure)
            assertEquals(NoteSaveFailure.DATABASE_WRITE_FAILED, ioFailure.await().failure)
            assertTrue(accepted.await().committed)
            assertEquals(NoteSaveFailure.FLUSH_FAILED, withTimeout(5_000L) { shutdown.await() }.failure)
            assertEquals(
                listOf("read-only", "io-failure", "accepted", "flush-failure"),
                events.toList()
            )
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun passwordChangeCandidateCanAbortRetryCommitAndRevokeTheOldSession() {
        val codec = Json {
            encodeDefaults = true
            ignoreUnknownKeys = false
            coerceInputValues = false
        }
        val oldPassword = "old-password-22134"
        val newPassword = "new-password-22134"
        val originalJson = codec.encodeToString(
            NoteEntry(
                id = "password-transaction-note",
                title = "protected title",
                content = "protected body",
                createdAtEpochMillis = 10L,
                updatedAtEpochMillis = 10L
            )
        )
        val encrypted = requireNotNull(
            NativeOptimizerBridge.encryptNote(originalJson, oldPassword)
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(encrypted[1]))

        val unlocked = requireNotNull(
            NativeOptimizerBridge.unlockNote(encrypted[0], oldPassword)
        )
        val oldSessionToken = unlocked[1]
        val firstCandidate = requireNotNull(
            NativeOptimizerBridge.changeEncryptedNotePassword(
                unlocked[0],
                oldSessionToken,
                newPassword
            )
        )
        assertTrue(
            NativeOptimizerBridge.abortEncryptedNotePasswordChange(
                firstCandidate,
                oldSessionToken
            )
        )
        assertNotNull(
            NativeOptimizerBridge.sealEncryptedNote(unlocked[0], oldSessionToken)
        )

        val committedCandidate = requireNotNull(
            NativeOptimizerBridge.changeEncryptedNotePassword(
                unlocked[0],
                oldSessionToken,
                newPassword
            )
        )
        assertTrue(
            NativeOptimizerBridge.commitEncryptedNotePasswordChange(
                committedCandidate,
                oldSessionToken
            )
        )
        assertNull(NativeOptimizerBridge.sealEncryptedNote(unlocked[0], oldSessionToken))
        assertNull(NativeOptimizerBridge.unlockNote(committedCandidate, oldPassword))
        val reopened = requireNotNull(
            NativeOptimizerBridge.unlockNote(committedCandidate, newPassword)
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(reopened[1]))
    }

    @Test
    fun nativeProtectionRejectionIsTypedAndNeverFallsBack() = runBlocking {
        val (context, _) = repositoryFixture("typed-native-rejection")
        val repository = TimerRepository(context)
        repository.awaitInitialized()
        val workspaceKey = repository.currentWorkspaceKey()
        val baseline = NoteEntry(
            id = "typed-native-rejection-note",
            title = "durable baseline",
            content = "must remain unchanged",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        assertTrue(repository.upsertNoteDurably(baseline, workspaceKey))
        val persistedBaseline = requireNotNull(
            repository.appData.value.notes.firstOrNull { note -> note.id == baseline.id }
        )
        val codec = strictNoteJson()
        val encrypted = requireNotNull(
            NativeOptimizerBridge.encryptNote(codec.encodeToString(persistedBaseline), "typed-rejection-password")
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(encrypted[1]))
        val sealed = codec.decodeFromString<NoteEntry>(encrypted[0])
        val sealedEncryption = requireNotNull(sealed.encryption)
        val malformed = sealed.copy(
            protectionStateRevision = sealedEncryption.protectionRevision + 1L
        )

        val nativeResult = NativeOptimizerBridge.upsertNoteAppDataJson(
            appDataJson = codec.encodeToString(repository.appData.value),
            noteJson = codec.encodeToString(malformed),
            now = 30L
        )
        assertTrue(nativeResult is NativeNoteUpsertResult.Rejected)

        val skippedRevision = sealed.copy(
            protectionStateRevision = 2L,
            encryption = sealedEncryption.copy(protectionRevision = 2L)
        )
        assertTrue(
            NativeOptimizerBridge.upsertNoteAppDataJson(
                appDataJson = codec.encodeToString(repository.appData.value),
                noteJson = codec.encodeToString(skippedRevision),
                now = 31L
            ) is NativeNoteUpsertResult.Rejected
        )

        val firstEncryptedData = requireNotNull(
            NativeOptimizerBridge.upsertNoteAppDataJson(
                appDataJson = codec.encodeToString(AppData.default()),
                noteJson = codec.encodeToString(sealed),
                now = 32L
            ) as? NativeNoteUpsertResult.Applied
        ).appDataJson
        val rollbackPlaintext = persistedBaseline.copy(protectionStateRevision = 0L)
        assertTrue(
            NativeOptimizerBridge.upsertNoteAppDataJson(
                appDataJson = firstEncryptedData,
                noteJson = codec.encodeToString(rollbackPlaintext),
                now = 33L
            ) is NativeNoteUpsertResult.Rejected
        )

        val forkEncryption = requireNotNull(
            NativeOptimizerBridge.encryptNote(
                codec.encodeToString(persistedBaseline),
                "typed-rejection-fork-password"
            )
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(forkEncryption[1]))
        val keyFork = codec.decodeFromString<NoteEntry>(forkEncryption[0])
        assertTrue(
            NativeOptimizerBridge.upsertNoteAppDataJson(
                appDataJson = firstEncryptedData,
                noteJson = codec.encodeToString(keyFork),
                now = 34L
            ) is NativeNoteUpsertResult.Rejected
        )

        val sameGenerationPlaintextData = requireNotNull(
            NativeOptimizerBridge.upsertNoteAppDataJson(
                appDataJson = codec.encodeToString(AppData.default()),
                noteJson = codec.encodeToString(
                    persistedBaseline.copy(protectionStateRevision = 1L)
                ),
                now = 35L
            ) as? NativeNoteUpsertResult.Applied
        ).appDataJson
        assertTrue(
            NativeOptimizerBridge.upsertNoteAppDataJson(
                appDataJson = sameGenerationPlaintextData,
                noteJson = codec.encodeToString(sealed),
                now = 36L
            ) is NativeNoteUpsertResult.Rejected
        )

        val before = repository.appData.value
        val save = repository.upsertNoteDurablyResult(
            note = malformed,
            expectedWorkspaceKey = workspaceKey,
            reason = "instrumented_typed_native_rejection"
        )
        assertFalse(save.committed)
        assertEquals(NoteSaveFailure.PROTECTION_STATE_CONFLICT, save.failure)
        assertEquals(before, repository.appData.value)

        delay(2_000L)
        assertEquals(before, repository.appData.value)
        val restarted = TimerRepository(context)
        restarted.awaitInitialized()
        val restartedNote = requireNotNull(
            restarted.appData.value.notes.firstOrNull { note -> note.id == baseline.id }
        )
        assertEquals("durable baseline", restartedNote.title)
        assertNull(restartedNote.encryption)
    }

    @Test
    fun failedProtectionCommitIsNeverPublishedOrRetried() = runBlocking {
        val (context, _) = repositoryFixture("durable-first-failure")
        val repository = TimerRepository(context)
        repository.awaitInitialized()
        val workspaceKey = repository.currentWorkspaceKey()
        val codec = strictNoteJson()
        val oldPassword = "durable-first-old-password"
        val newPassword = "durable-first-new-password"
        val plaintext = NoteEntry(
            id = "durable-first-note",
            title = "old durable value",
            content = "old durable body",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val initialEncryption = requireNotNull(
            NativeOptimizerBridge.encryptNote(codec.encodeToString(plaintext), oldPassword)
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(initialEncryption[1]))
        val initialSealed = codec.decodeFromString<NoteEntry>(initialEncryption[0])
        assertTrue(repository.upsertNoteDurably(initialSealed, workspaceKey))
        val before = repository.appData.value
        val persistedInitial = requireNotNull(
            before.notes.firstOrNull { note -> note.id == plaintext.id }
        )
        val unlocked = requireNotNull(
            NativeOptimizerBridge.unlockNote(codec.encodeToString(persistedInitial), oldPassword)
        )
        val oldSessionToken = unlocked[1]
        val candidateJson = requireNotNull(
            NativeOptimizerBridge.changeEncryptedNotePassword(
                noteJson = unlocked[0],
                sessionToken = oldSessionToken,
                newPassword = newPassword
            )
        )
        val candidate = codec.decodeFromString<NoteEntry>(candidateJson)
        assertTrue(candidate.protectionStateRevision > persistedInitial.protectionStateRevision)
        val database = repositoryDatabase(repository)
        val databaseFile = context.getDatabasePath(AppStateDatabase.DATABASE_NAME)
        val databaseDirectory = requireNotNull(databaseFile.parentFile)
        database.close()
        setDatabaseWritable(databaseDirectory, writable = false)
        val result = try {
            repository.upsertNoteDurablyResult(
                note = candidate,
                expectedWorkspaceKey = workspaceKey,
                reason = "instrumented_storage_failure"
            )
        } finally {
            setDatabaseWritable(databaseDirectory, writable = true)
        }

        assertFalse(result.committed)
        assertTrue(
            result.failure == NoteSaveFailure.DATABASE_WRITE_FAILED ||
                result.failure == NoteSaveFailure.FLUSH_FAILED
        )
        assertEquals(before, repository.appData.value)
        assertTrue(
            NativeOptimizerBridge.abortEncryptedNotePasswordChange(
                candidateJson,
                oldSessionToken
            )
        )
        assertNotNull(NativeOptimizerBridge.sealEncryptedNote(unlocked[0], oldSessionToken))
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(oldSessionToken))

        // A failed durable-first candidate must never be handed to the generic dirty retry.
        delay(3_000L)
        assertEquals(before, repository.appData.value)
        val restarted = TimerRepository(context)
        restarted.awaitInitialized()
        val restartedNote = requireNotNull(
            restarted.appData.value.notes.firstOrNull { note -> note.id == plaintext.id }
        )
        assertEquals(persistedInitial.protectionStateRevision, restartedNote.protectionStateRevision)
        val reopenedOld = requireNotNull(
            NativeOptimizerBridge.unlockNote(codec.encodeToString(restartedNote), oldPassword)
        )
        assertNull(NativeOptimizerBridge.unlockNote(codec.encodeToString(restartedNote), newPassword))

        val legalRetryJson = requireNotNull(
            NativeOptimizerBridge.changeEncryptedNotePassword(
                noteJson = reopenedOld[0],
                sessionToken = reopenedOld[1],
                newPassword = newPassword
            )
        )
        val legalRetry = codec.decodeFromString<NoteEntry>(legalRetryJson)
        val legalSave = restarted.upsertNoteDurablyResult(
            note = legalRetry,
            expectedWorkspaceKey = workspaceKey,
            reason = "instrumented_storage_recovered"
        )
        assertTrue(legalSave.committed)
        assertTrue(
            NativeOptimizerBridge.commitEncryptedNotePasswordChange(
                legalRetryJson,
                reopenedOld[1]
            )
        )
        assertNull(NativeOptimizerBridge.sealEncryptedNote(reopenedOld[0], reopenedOld[1]))

        val finalRestart = TimerRepository(context)
        finalRestart.awaitInitialized()
        val finalNote = requireNotNull(
            finalRestart.appData.value.notes.firstOrNull { note -> note.id == plaintext.id }
        )
        assertNull(NativeOptimizerBridge.unlockNote(codec.encodeToString(finalNote), oldPassword))
        val reopenedNew = requireNotNull(
            NativeOptimizerBridge.unlockNote(codec.encodeToString(finalNote), newPassword)
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(reopenedNew[1]))
    }

    @Test
    fun redactionFailureIsCommittedAndRetried() = runBlocking {
        val (context, root) = repositoryFixture("post-commit-redaction")
        val repository = TimerRepository(context)
        repository.awaitInitialized()
        val workspaceKey = repository.currentWorkspaceKey()
        val codec = strictNoteJson()
        val oldPassword = "redaction-old-password"
        val newPassword = "redaction-new-password"
        val plaintext = NoteEntry(
            id = "redaction-retry-note",
            title = "plaintext baseline",
            content = "plaintext recovery payload",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val initialEncryption = requireNotNull(
            NativeOptimizerBridge.encryptNote(codec.encodeToString(plaintext), oldPassword)
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(initialEncryption[1]))
        val initialSealed = codec.decodeFromString<NoteEntry>(initialEncryption[0])
        assertTrue(repository.upsertNoteDurably(initialSealed, workspaceKey))
        val persisted = requireNotNull(
            repository.appData.value.notes.firstOrNull { note -> note.id == plaintext.id }
        )
        val unlocked = requireNotNull(
            NativeOptimizerBridge.unlockNote(codec.encodeToString(persisted), oldPassword)
        )
        val candidateJson = requireNotNull(
            NativeOptimizerBridge.changeEncryptedNotePassword(
                noteJson = unlocked[0],
                sessionToken = unlocked[1],
                newPassword = newPassword
            )
        )
        val candidate = codec.decodeFromString<NoteEntry>(candidateJson)
        val checkpoint = requireNotNull(
            repository.journalNoteDraft(
                workspaceKey = workspaceKey,
                baseNote = persisted,
                note = candidate,
                editorSessionId = "redaction-editor",
                sequence = 1L
            )
        )
        val journalFile = requireNotNull(
            root.walkTopDown().firstOrNull { file ->
                file.isFile && file.extension == "json" &&
                    file.path.contains("note_draft_journal_v2")
            }
        )
        val journalDirectory = requireNotNull(journalFile.parentFile)

        Os.chmod(journalDirectory.absolutePath, DIRECTORY_READ_ONLY_MODE)
        val result = try {
            val attempted = repository.upsertNoteDurablyResult(
                note = candidate,
                expectedWorkspaceKey = workspaceKey,
                reason = "instrumented_redaction_failure",
                draftCheckpoint = checkpoint
            )
            assertTrue(attempted.committed)
            assertTrue(attempted.postCommitCleanupPending)
            assertTrue(journalFile.exists())
            assertNotNull(repository.appData.value.notes.firstOrNull { note ->
                note.id == plaintext.id &&
                    note.protectionStateRevision == candidate.protectionStateRevision
            })
            assertTrue(
                NativeOptimizerBridge.commitEncryptedNotePasswordChange(
                    candidateJson,
                    unlocked[1]
                )
            )
            assertNull(NativeOptimizerBridge.sealEncryptedNote(unlocked[0], unlocked[1]))

            // The authoritative commit must remain visible after process-style reconstruction,
            // even while the exact recovery-copy cleanup is still unable to run.
            val restarted = TimerRepository(context)
            restarted.awaitInitialized()
            val restartedNote = requireNotNull(
                restarted.appData.value.notes.firstOrNull { note -> note.id == plaintext.id }
            )
            assertEquals(candidate.protectionStateRevision, restartedNote.protectionStateRevision)
            assertNull(NativeOptimizerBridge.unlockNote(codec.encodeToString(restartedNote), oldPassword))
            val reopened = requireNotNull(
                NativeOptimizerBridge.unlockNote(codec.encodeToString(restartedNote), newPassword)
            )
            assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(reopened[1]))
            assertTrue(journalFile.exists())
            attempted
        } finally {
            Os.chmod(journalDirectory.absolutePath, DIRECTORY_WRITABLE_MODE)
        }

        assertTrue(result.committed)
        assertTrue(result.postCommitCleanupPending)

        withTimeout(15_000L) {
            while (journalFile.exists()) {
                delay(100L)
            }
        }
        assertFalse(journalFile.exists())
    }

    @Test
    fun encryptedVersionSameGenerationSameKeyCommitsAcrossRestart() = runBlocking {
        val (context, _) = repositoryFixture("encrypted-version-generation")
        val codec = strictNoteJson()
        val password = "same-generation-version-password"
        val original = NoteEntry(
            id = "encrypted-version-note",
            title = "version one",
            content = "encrypted version body",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        ).repairVersionStack(now = 10L)
        val encrypted = requireNotNull(
            NativeOptimizerBridge.encryptNote(codec.encodeToString(original), password)
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(encrypted[1]))
        val initialSealed = codec.decodeFromString<NoteEntry>(encrypted[0])

        val firstRepository = TimerRepository(context)
        firstRepository.awaitInitialized()
        val workspaceKey = firstRepository.currentWorkspaceKey()
        assertTrue(firstRepository.upsertNoteDurably(initialSealed, workspaceKey))

        val restartedRepository = TimerRepository(context)
        restartedRepository.awaitInitialized()
        val afterRestart = requireNotNull(
            restartedRepository.appData.value.notes.firstOrNull { note -> note.id == original.id }
        )
        val unlocked = requireNotNull(
            NativeOptimizerBridge.unlockNote(codec.encodeToString(afterRestart), password)
        )
        val sessionToken = unlocked[1]
        val unlockedNote = codec.decodeFromString<NoteEntry>(unlocked[0])
            .copy(encryptionUnlocked = true)
            .repairVersionStack(now = 20L)
        val sourceVersionId = unlockedNote.latestVersionId
        val newVersionId = "${original.id}:version:instrumented-next"
        val nextVersion = requireNotNull(
            unlockedNote.createNextProductVersion(
                sourceVersionId = sourceVersionId,
                expectedLatestVersionId = unlockedNote.latestVersionId,
                newVersionId = newVersionId,
                now = 30L
            )
        )
        val nextSealedJson = requireNotNull(
            NativeOptimizerBridge.sealEncryptedNote(codec.encodeToString(nextVersion), sessionToken)
        )
        val nextSealed = codec.decodeFromString<NoteEntry>(nextSealedJson)
        assertEquals(afterRestart.protectionStateRevision, nextSealed.protectionStateRevision)
        assertEquals(afterRestart.encryption!!.keyId, nextSealed.encryption!!.keyId)
        assertTrue(
            restartedRepository.createNextNoteVersionDurably(
                note = nextSealed,
                expectedWorkspaceKey = workspaceKey,
                sourceVersionId = sourceVersionId,
                expectedLatestVersionId = unlockedNote.latestVersionId,
                newVersionId = newVersionId
            )
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(sessionToken))

        val secondRestart = TimerRepository(context)
        secondRestart.awaitInitialized()
        val durableNext = requireNotNull(
            secondRestart.appData.value.notes.firstOrNull { note -> note.id == original.id }
        )
        val reopened = requireNotNull(
            NativeOptimizerBridge.unlockNote(codec.encodeToString(durableNext), password)
        )
        val reopenedNote = codec.decodeFromString<NoteEntry>(reopened[0])
        assertEquals(newVersionId, reopenedNote.latestVersionId)
        assertTrue(reopenedNote.versions.any { version -> version.id == newVersionId })
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(reopened[1]))

        val forkEncryption = requireNotNull(
            NativeOptimizerBridge.encryptNote(codec.encodeToString(original), "different-key-password")
        )
        assertTrue(NativeOptimizerBridge.closeEncryptedNoteSession(forkEncryption[1]))
        val keyFork = codec.decodeFromString<NoteEntry>(forkEncryption[0])
        val durableNextEncryption = requireNotNull(durableNext.encryption)
        assertEquals(durableNext.protectionStateRevision, keyFork.protectionStateRevision)
        assertNotEquals(durableNextEncryption.keyId, requireNotNull(keyFork.encryption).keyId)
        assertFalse(
            secondRestart.createNextNoteVersionDurably(
                note = keyFork,
                expectedWorkspaceKey = workspaceKey,
                sourceVersionId = sourceVersionId,
                expectedLatestVersionId = newVersionId,
                newVersionId = "${original.id}:version:key-fork"
            )
        )
        assertEquals(
            durableNextEncryption.keyId,
            secondRestart.appData.value.notes.first { note -> note.id == original.id }.encryption!!.keyId
        )
    }

    @Test
    fun cleanupCheckpointANeverDeletesNewerJournalB() {
        val baseContext = ApplicationProvider.getApplicationContext<Context>()
        val root = File(baseContext.cacheDir, "checkpoint_exact_cleanup_test/${UUID.randomUUID()}")
        val context = IsolatedFilesContext(baseContext, root)
        val workspaceKey = "checkpoint-workspace"
        val base = NoteEntry(
            id = "checkpoint-note",
            title = "base",
            content = "base",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val draftA = base.copy(content = "draft A", updatedAtEpochMillis = 20L)
        val draftB = base.copy(content = "draft B", updatedAtEpochMillis = 30L)
        try {
            val checkpointA = requireNotNull(
                NoteDraftJournal.write(
                    context,
                    workspaceKey,
                    base,
                    draftA,
                    editorSessionId = "same-editor",
                    sequence = 1L
                )
            )
            val checkpointB = requireNotNull(
                NoteDraftJournal.write(
                    context,
                    workspaceKey,
                    draftA,
                    draftB,
                    editorSessionId = "same-editor",
                    sequence = 2L
                )
            )

            assertTrue(NoteDraftJournal.removeExactIfCurrent(context, checkpointA))

            val surviving = NoteDraftJournal.read(context, workspaceKey).single()
            assertEquals(2L, surviving.sequence)
            assertEquals("draft B", surviving.note.content)
            assertTrue(NoteDraftJournal.removeExactIfCurrent(context, checkpointB))
            assertTrue(NoteDraftJournal.read(context, workspaceKey).isEmpty())
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun durableDraftJournalSurvivesRepositoryRecreationLikeAForcedProcessKill() {
        val baseContext = ApplicationProvider.getApplicationContext<Context>()
        val root = File(baseContext.cacheDir, "note_recovery_android_test/${UUID.randomUUID()}")
        val firstProcess = IsolatedFilesContext(baseContext, root)
        val workspaceKey = "guest"
        val base = NoteEntry(
            id = "plain-note",
            title = "before",
            content = "before-body",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val draft = base.copy(title = "after", content = "after-body", updatedAtEpochMillis = 20L)
        try {
            val checkpoint = NoteDraftJournal.write(
                firstProcess,
                workspaceKey,
                base,
                draft,
                editorSessionId = "editor-1",
                sequence = 1L
            )
            assertNotNull(checkpoint)

            val restartedProcess = IsolatedFilesContext(baseContext, root)
            val recovered = NoteDraftJournal.read(restartedProcess, workspaceKey).single()
            assertEquals("after-body", recovered.note.content)
            assertEquals(
                NoteDraftRecoveryPriority.PLAINTEXT,
                NoteDraftRecoveryPolicy.priority(recovered, base, hasDeletionEvidence = false)
            )
        } finally {
            root.deleteRecursively()
        }
    }

    // These two opt-in phases are selected individually by the host harness,
    // with a real `am force-stop` between invocations. A normal suite run does
    // not claim to have performed the external process kill.
    @Test
    fun prepareDurableDraftForExternalForceStop() {
        val requestedPhase =
            InstrumentationRegistry.getArguments().getString("forceStopPhase") ?: return
        assertEquals("prepare", requestedPhase)
        val context = ApplicationProvider.getApplicationContext<Context>()
        val workspaceKey = "instrumented-force-stop"
        val base = NoteEntry(
            id = "force-stop-note",
            title = "before force-stop",
            content = "durable ancestor",
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 10L
        )
        val draft = base.copy(
            title = "after force-stop",
            content = "journal survived a real package force-stop",
            updatedAtEpochMillis = 20L
        )
        assertTrue(NoteDraftJournal.removeAllForNote(context, workspaceKey, base.id))
        assertNotNull(
            NoteDraftJournal.write(
                context,
                workspaceKey,
                base,
                draft,
                editorSessionId = "external-force-stop",
                sequence = 1L
            )
        )
    }

    @Test
    fun verifyDurableDraftAfterExternalForceStop() {
        val requestedPhase =
            InstrumentationRegistry.getArguments().getString("forceStopPhase") ?: return
        assertEquals("verify", requestedPhase)
        val context = ApplicationProvider.getApplicationContext<Context>()
        val workspaceKey = "instrumented-force-stop"
        val record = NoteDraftJournal.read(context, workspaceKey).single { candidate ->
            candidate.noteId == "force-stop-note"
        }
        assertEquals("journal survived a real package force-stop", record.note.content)
        assertEquals(
            NoteDraftRecoveryPriority.PLAINTEXT,
            NoteDraftRecoveryPolicy.priority(
                record = record,
                existing = NoteEntry(
                    id = record.noteId,
                    title = "before force-stop",
                    content = "durable ancestor",
                    createdAtEpochMillis = 10L,
                    updatedAtEpochMillis = 10L
                ),
                hasDeletionEvidence = false
            )
        )
        assertTrue(
            NoteDraftJournal.removeIfCommitted(
                context,
                NoteDraftJournal.checkpoint(record),
                record.note
            )
        )
    }

    @Test
    fun encryptedRecoveryAcceptsOnlyRedactedAeadCiphertextAndRejectsStateRollback() {
        val baseContext = ApplicationProvider.getApplicationContext<Context>()
        val root = File(baseContext.cacheDir, "encrypted_note_recovery_test/${UUID.randomUUID()}")
        val context = IsolatedFilesContext(baseContext, root)
        val workspaceKey = "guest"
        val base = encryptedNote("encrypted-note", revision = 1L, ciphertextByte = 7)
        val draft = encryptedNote("encrypted-note", revision = 1L, ciphertextByte = 9)
        val secret = "plaintext-must-never-enter-encrypted-journal"
        try {
            assertNull(
                NoteDraftJournal.write(
                    context,
                    workspaceKey,
                    base,
                    draft.copy(title = secret, content = secret, encryptionUnlocked = true),
                    editorSessionId = "unsafe-editor",
                    sequence = 1L
                )
            )
            assertNotNull(
                NoteDraftJournal.write(
                    context,
                    workspaceKey,
                    base,
                    draft,
                    editorSessionId = "safe-editor",
                    sequence = 2L
                )
            )
            val diskText = context.filesDir.walkTopDown()
                .filter(File::isFile)
                .joinToString(separator = "") { file -> file.readText() }
            assertFalse(diskText.contains(secret))

            val recovered = NoteDraftJournal.read(context, workspaceKey).single()
            assertTrue(recovered.note.hasAeadCiphertextRecoveryEnvelope())
            assertEquals(1L, recovered.note.effectiveProtectionStateRevision())
            assertEquals(
                NoteDraftRecoveryPriority.AUTHENTICATED_CIPHERTEXT,
                NoteDraftRecoveryPolicy.priority(recovered, base, hasDeletionEvidence = false)
            )

            val disabledAtNewerState = NoteEntry(
                id = base.id,
                protectionStateRevision = 2L,
                createdAtEpochMillis = 10L,
                updatedAtEpochMillis = 30L
            )
            val rollbackRecord = recovered.copy(
                baseFingerprint = NoteDraftJournal.fingerprint(disabledAtNewerState),
                ancestorFingerprints = listOf(NoteDraftJournal.fingerprint(disabledAtNewerState))
            )
            assertEquals(
                NoteDraftRecoveryPriority.REJECT,
                NoteDraftRecoveryPolicy.priority(
                    rollbackRecord,
                    disabledAtNewerState,
                    hasDeletionEvidence = false
                )
            )
        } finally {
            root.deleteRecursively()
        }
    }

    private fun encryptedNote(noteId: String, revision: Long, ciphertextByte: Byte): NoteEntry {
        return NoteEntry(
            id = noteId,
            protectionStateRevision = revision,
            encryption = NoteEncryptionEnvelope(
                keyId = encoded(16, 1, urlSafe = true),
                protectionRevision = revision,
                memoryKiB = 65_536,
                iterations = 3,
                parallelism = 1,
                saltBase64 = encoded(16, 2),
                keyNonceBase64 = encoded(12, 3),
                wrappedKeyBase64 = encoded(48, 4),
                contentNonceBase64 = encoded(12, 5),
                ciphertextBase64 = encoded(64, ciphertextByte.toInt())
            ),
            createdAtEpochMillis = 10L,
            updatedAtEpochMillis = 20L
        )
    }

    private fun encoded(size: Int, value: Int, urlSafe: Boolean = false): String {
        val flags = Base64.NO_WRAP or Base64.NO_PADDING or
            (if (urlSafe) Base64.URL_SAFE else 0)
        return Base64.encodeToString(ByteArray(size) { value.toByte() }, flags)
    }

    private fun strictNoteJson(): Json = Json {
        encodeDefaults = true
        ignoreUnknownKeys = false
        coerceInputValues = false
    }

    private fun repositoryFixture(label: String): Pair<RepositoryFilesContext, File> {
        val baseContext = ApplicationProvider.getApplicationContext<Context>()
        val root = File(
            baseContext.cacheDir,
            "protection_transaction_android_test/$label-${UUID.randomUUID()}"
        )
        return RepositoryFilesContext(baseContext, root) to root
    }

    private fun repositoryDatabase(repository: TimerRepository): AppStateDatabase {
        val field = TimerRepository::class.java.getDeclaredField("stateDatabase")
        field.isAccessible = true
        return field.get(repository) as AppStateDatabase
    }

    private fun setDatabaseWritable(directory: File, writable: Boolean) {
        check(directory.isDirectory)
        if (writable) {
            Os.chmod(directory.absolutePath, DIRECTORY_WRITABLE_MODE)
            directory.listFiles().orEmpty().forEach { file ->
                Os.chmod(file.absolutePath, FILE_WRITABLE_MODE)
            }
        } else {
            directory.listFiles().orEmpty().forEach { file ->
                Os.chmod(file.absolutePath, FILE_READ_ONLY_MODE)
            }
            Os.chmod(directory.absolutePath, DIRECTORY_READ_ONLY_MODE)
        }
    }

    private class RepositoryFilesContext(base: Context, private val root: File) : ContextWrapper(base) {
        override fun getApplicationContext(): Context = this

        override fun getFilesDir(): File = directory("files")

        override fun getCacheDir(): File = directory("cache")

        override fun getNoBackupFilesDir(): File = directory("no_backup")

        override fun getDatabasePath(name: String): File {
            return File(directory("databases"), name)
        }

        override fun openOrCreateDatabase(
            name: String,
            mode: Int,
            factory: SQLiteDatabase.CursorFactory?
        ): SQLiteDatabase {
            return SQLiteDatabase.openDatabase(
                getDatabasePath(name).absolutePath,
                factory,
                sqliteOpenFlags(mode)
            )
        }

        override fun openOrCreateDatabase(
            name: String,
            mode: Int,
            factory: SQLiteDatabase.CursorFactory?,
            errorHandler: DatabaseErrorHandler?
        ): SQLiteDatabase {
            return SQLiteDatabase.openDatabase(
                getDatabasePath(name).absolutePath,
                factory,
                sqliteOpenFlags(mode),
                errorHandler
            )
        }

        override fun deleteDatabase(name: String): Boolean {
            return SQLiteDatabase.deleteDatabase(getDatabasePath(name))
        }

        override fun getExternalFilesDir(type: String?): File {
            return directory(if (type.isNullOrBlank()) "external" else "external/$type")
        }

        override fun getSharedPreferences(name: String, mode: Int): SharedPreferences {
            return baseContext.getSharedPreferences("${root.name}-$name", mode)
        }

        private fun directory(relativePath: String): File = File(root, relativePath).apply {
            check(exists() || mkdirs()) { "Could not create isolated $relativePath directory." }
        }

        private fun sqliteOpenFlags(mode: Int): Int {
            var flags = SQLiteDatabase.CREATE_IF_NECESSARY
            if (mode and Context.MODE_ENABLE_WRITE_AHEAD_LOGGING != 0) {
                flags = flags or SQLiteDatabase.ENABLE_WRITE_AHEAD_LOGGING
            }
            if (mode and Context.MODE_NO_LOCALIZED_COLLATORS != 0) {
                flags = flags or SQLiteDatabase.NO_LOCALIZED_COLLATORS
            }
            return flags
        }

        private val baseContext: Context = base
    }

    private class IsolatedFilesContext(base: Context, private val root: File) : ContextWrapper(base) {
        override fun getApplicationContext(): Context = this

        override fun getFilesDir(): File = File(root, "files").apply {
            check(exists() || mkdirs()) { "Could not create isolated files directory." }
        }
    }

    private companion object {
        const val FILE_READ_ONLY_MODE = 0x100
        const val FILE_WRITABLE_MODE = 0x180
        const val DIRECTORY_READ_ONLY_MODE = 0x140
        const val DIRECTORY_WRITABLE_MODE = 0x1C0
    }
}
"####,
    },
    KotlinAndroidTestSource {
        path: "com/ofairyo/gridtimer/ui/ThemeNavigationInstrumentedTest.kt",
        contents: r####"
package com.ofairyo.gridtimer.ui

import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.ofairyo.gridtimer.MainActivity
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ThemeNavigationInstrumentedTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun darkAndOledMainPagesRemainReachable() {
        openTab("Me")
        selectTheme("深色")
        verifyAllTabs()

        openTab("Me")
        selectTheme("OLED 纯黑")
        verifyAllTabs()
    }

    private fun verifyAllTabs() {
        listOf(
            "Timer" to "十倍率 v2.22-黑夜模式",
            "Quick notes" to "最近便签",
            "Knowledge" to "文档",
            "Risk Control" to "风控",
            "Me" to "外观"
        ).forEach { (tabLabel, pageMarker) ->
            openTab(tabLabel)
            composeRule.onNodeWithText(
                pageMarker,
                substring = false,
                useUnmergedTree = true
            ).assertExists()
        }
    }

    private fun openTab(label: String) {
        composeRule.onNodeWithText(label, useUnmergedTree = true).performClick()
        composeRule.waitForIdle()
    }

    private fun selectTheme(label: String) {
        composeRule.onNodeWithText(label, useUnmergedTree = true).performClick()
        composeRule.waitForIdle()
    }
}
"####,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrumentation_test_uses_production_contract_and_real_durable_stores() {
        let source = SOURCES
            .iter()
            .find(|source| {
                source.path
                    == "com/ofairyo/gridtimer/data/WorkspaceIdentityRebindInstrumentedTest.kt"
            })
            .expect("identity-rebind instrumentation source");
        assert!(source
            .contents
            .contains("WorkspaceIdentityRebindContract.bindAuthenticatedResponse("));
        assert!(source
            .contents
            .contains("TimerRepository.AccountSnapshotMerge.mergeAccountSnapshotWithCurrent("));
        assert!(source.contents.contains("AppStateDatabase(context)"));
        assert!(source
            .contents
            .contains("SecureSyncSessionStore.persist(sessionContext, json, reboundSession)"));
        assert!(source
            .contents
            .contains("SecureSyncSessionStore.load(sessionContext, json)"));
        assert!(source.contents.contains("Os.chmod("));
        assert!(source.contents.contains("assertFalse(persisted)"));
        assert!(source
            .contents
            .contains("forgedAndUnsafeRebindVariantsFailClosedWithoutDataMutation"));
        assert!(source.contents.contains("valid.copy(retryable = true)"));
        assert!(source
            .contents
            .contains("valid.copy(restoreReceipt = \"not-a-valid-receipt\")"));
        assert!(source.contents.contains("allowIdentityRebind = false"));
        assert!(source
            .contents
            .contains("assertContainsNote(restartedSnapshot, fixture.localNoteId)"));
        assert!(source
            .contents
            .contains("confirmationResult(mergedSnapshot)"));
    }

    #[test]
    fn instrumentation_test_covers_actionable_identity_boundary_message() {
        let source = SOURCES[0].contents;
        assert!(source.contains("workspace_binding_mismatch"));
        assert!(source.contains("workspace_identity_rebind_invalid"));
        assert!(source.contains("下载账户数据"));
    }

    #[test]
    fn note_formatter_instrumentation_exercises_the_generated_kotlin_fallback() {
        let source = SOURCES
            .iter()
            .find(|source| {
                source.path == "com/ofairyo/gridtimer/core/NoteTextFormatterInstrumentedTest.kt"
            })
            .expect("note formatter instrumentation source");
        assert!(source.contents.contains("NoteTextFormatter.apply("));
        assert!(source.contents.contains("NoteTextAction.TODO"));
        assert!(source.contents.contains("- [ ] $source"));
        assert!(source.contents.contains("- [x] $source"));
        assert!(source.contents.contains("- [X] 丙"));
        assert!(source
            .contents
            .contains("edit.content.length, edit.selectionEnd"));
    }

    #[test]
    fn lifecycle_and_draft_recovery_instrumentation_exercises_production_boundaries() {
        let source = SOURCES
            .iter()
            .find(|source| {
                source.path
                    == "com/ofairyo/gridtimer/ui/PersistenceLifecycleRecoveryInstrumentedTest.kt"
            })
            .expect("persistence lifecycle instrumentation source");
        assert!(source
            .contents
            .contains("lifecycleShutdownReturnsOnMainThreadThenDrainsFlushesAndClosesInOrder"));
        assert!(source
            .contents
            .contains("typedReadOnlyAndIoFailuresDoNotSkipLaterAcceptedWritesOrFinalFlush"));
        assert!(source
            .contents
            .contains("passwordChangeCandidateCanAbortRetryCommitAndRevokeTheOldSession"));
        assert!(source
            .contents
            .contains("NativeOptimizerBridge.abortEncryptedNotePasswordChange("));
        assert!(source
            .contents
            .contains("NativeOptimizerBridge.commitEncryptedNotePasswordChange("));
        assert!(source
            .contents
            .contains("durableDraftJournalSurvivesRepositoryRecreationLikeAForcedProcessKill"));
        assert!(source
            .contents
            .contains("prepareDurableDraftForExternalForceStop"));
        assert!(source
            .contents
            .contains("verifyDurableDraftAfterExternalForceStop"));
        assert!(source.contents.contains("forceStopPhase"));
        assert!(!source.contents.contains("assumeTrue("));
        assert!(source
            .contents
            .contains("encryptedRecoveryAcceptsOnlyRedactedAeadCiphertextAndRejectsStateRollback"));
        assert!(source
            .contents
            .contains("nativeProtectionRejectionIsTypedAndNeverFallsBack"));
        assert!(source
            .contents
            .contains("failedProtectionCommitIsNeverPublishedOrRetried"));
        assert!(source
            .contents
            .contains("redactionFailureIsCommittedAndRetried"));
        assert!(source
            .contents
            .contains("encryptedVersionSameGenerationSameKeyCommitsAcrossRestart"));
        assert!(source
            .contents
            .contains("cleanupCheckpointANeverDeletesNewerJournalB"));
        assert!(source
            .contents
            .contains("nativeResult is NativeNoteUpsertResult.Rejected"));
        assert!(source
            .contents
            .contains("NoteSaveFailure.PROTECTION_STATE_CONFLICT"));
        assert!(source.contents.contains("result.postCommitCleanupPending"));
        assert!(source
            .contents
            .contains("assertEquals(2L, surviving.sequence)"));
        assert!(source
            .contents
            .contains("NoteDraftJournal.removeExactIfCurrent(context, checkpointA)"));
        assert!(source
            .contents
            .contains("override fun openOrCreateDatabase("));
        assert!(source.contents.contains("SQLiteDatabase.openDatabase("));
        assert!(source.contents.contains("NoteMutationDrainController("));
        assert!(source.contents.contains("NoteDraftJournal.write("));
        assert!(source
            .contents
            .contains("NoteDraftRecoveryPriority.AUTHENTICATED_CIPHERTEXT"));
    }

    #[test]
    fn note_version_stack_instrumentation_covers_non_mutating_visits_and_history_projection() {
        let source = SOURCES
            .iter()
            .find(|source| {
                source.path == "com/ofairyo/gridtimer/ui/NoteVersionStackInstrumentedTest.kt"
            })
            .expect("note version stack instrumentation source");
        assert!(source.contents.contains("assertSame(base, unchanged)"));
        assert!(source.contents.contains("SmartisanNoteExitAction.Noop"));
        assert!(source
            .contents
            .contains("attachmentPreviewProjection(historical.id)"));
        assert!(source
            .contents
            .contains("attachmentPreviewProjection(\"missing-version\")"));
        assert!(source
            .contents
            .contains("historical.historicalDisplayHtml()"));
        assert!(source
            .contents
            .contains("commonTitleOwner.versionIdForSearchQuery(\"共同标题\")"));
    }
}
