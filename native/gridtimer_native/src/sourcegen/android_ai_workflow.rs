// v2.23.2.24 - Bind document sources to a fresh readable target and isolate projection failures.
// v2.23.2.23 - Bind review mode and verified draft state to the save boundary.
// v2.23.2.20 - Show every sticky note and preserve complete legacy-note previews.
// v2.23.2.19 - Open selected Agent sticky-note sources and retain clear source labels.
// v2.23.2.17 - Add a bounded task Agent with explicit scope and durable draft handoff.
// v2.23.2.6 - Display AI answers with offline Markdown and formula layout.
// v2.23.2.5 - Separate direct AI questions from source-grounded knowledge answers.
// v2.23.2.4 - Restore question keyboard only after the editor and dialog are ready.
// v2.23.2.3 - Make Android AI actions discoverable and bind answers to their request.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeAiRequestBoundary.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeAiRequestBoundaryTest.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

internal data class KnowledgeAiOutcome(val ok: Boolean, val text: String, val message: String)

internal data class KnowledgeAgentDraft(
    val title: String,
    val content: String,
    val actionItems: List<String>,
    val sourceIds: List<String>
)

internal data class KnowledgeAgentSource(
    val id: String,
    val title: String,
    val folder: String,
    val content: String
)

internal data class KnowledgeAgentDraftContext(
    val workspaceKey: String,
    val identity: String,
    val configuration: AiConfiguration,
    val question: String,
    val documents: List<KnowledgeAgentSource>,
    val deepReview: Boolean = false
)

internal class KnowledgeAgentRunHandle {
    var taskId: String? = null
        private set
    fun started(id: String) { taskId = id }
    fun finished(id: String) { if (taskId == id) taskId = null }
}

internal object KnowledgeAgentReviewPolicy {
    fun canSave(verified: Boolean, completeDraft: Boolean): Boolean = verified && completeDraft
}


internal data class KnowledgeAgentProgress(
    val taskId: String, val sequence: Long, val stage: String, val activity: String,
    val requests: Int, val toolCalls: Int, val documentsRead: Int,
    val requestInFlight: Boolean, val cancelRequested: Boolean, val terminal: String, val elapsedSeconds: Long
)
internal fun acceptKnowledgeAgentProgress(
    previous: KnowledgeAgentProgress?, incoming: KnowledgeAgentProgress?, ownedTaskId: String, identityCurrent: Boolean
): KnowledgeAgentProgress? {
    if (incoming == null || !identityCurrent || incoming.taskId != ownedTaskId ||
        incoming.sequence < 0 || incoming.requests < 0 || incoming.toolCalls < 0 ||
        incoming.documentsRead < 0 || incoming.elapsedSeconds < 0 ||
        incoming.stage !in setOf("first_pass", "review", "verifying") ||
        incoming.activity !in setOf("preparing", "model", "tool", "verifying", "finished") ||
        incoming.terminal !in setOf("running", "ready", "review_incomplete", "failed", "cancelled")) return previous
    if (previous != null && previous.taskId == ownedTaskId &&
        (incoming.sequence < previous.sequence || incoming.elapsedSeconds < previous.elapsedSeconds ||
         incoming.requests < previous.requests || incoming.toolCalls < previous.toolCalls ||
         (previous.terminal != "running" && incoming.terminal == "running"))) return previous
    return incoming
}
internal enum class KnowledgeAgentLaunchAction { WAIT, PREVIEW, START }
internal fun knowledgeAgentLaunchAction(
    busy: Boolean, saving: Boolean, inputsReady: Boolean, previewVisible: Boolean, authorized: Boolean
): KnowledgeAgentLaunchAction = when {
    busy || saving || !inputsReady -> KnowledgeAgentLaunchAction.WAIT
    previewVisible && authorized -> KnowledgeAgentLaunchAction.START
    else -> KnowledgeAgentLaunchAction.PREVIEW
}
internal fun toggleKnowledgeAgentSelection(selectedIds: Set<String>, candidateIds: List<String>): Set<String> {
    val bounded = candidateIds.filter(String::isNotBlank).distinct().take(30).toSet()
    return if (selectedIds == bounded) emptySet() else bounded
}
internal enum class KnowledgeAgentSaveState {
    READY, RUNNING, SAVING, SAVED, CANCELLED, FAILED, REVIEW_INCOMPLETE, CONTEXT_CHANGED, INCOMPLETE, UNVERIFIED
}
internal fun knowledgeAgentSaveState(
    running: Boolean, saving: Boolean, saved: Boolean, terminal: String,
    identityCurrent: Boolean, contextCurrent: Boolean, verified: Boolean, completeDraft: Boolean
): KnowledgeAgentSaveState {
    if (saved) return KnowledgeAgentSaveState.SAVED
    if (saving) return KnowledgeAgentSaveState.SAVING
    if (running) return KnowledgeAgentSaveState.RUNNING
    if (!identityCurrent || !contextCurrent) return KnowledgeAgentSaveState.CONTEXT_CHANGED
    if (terminal == "cancelled") return KnowledgeAgentSaveState.CANCELLED
    if (terminal == "failed") return KnowledgeAgentSaveState.FAILED
    if (terminal == "review_incomplete") return KnowledgeAgentSaveState.REVIEW_INCOMPLETE
    if (!completeDraft) return KnowledgeAgentSaveState.INCOMPLETE
    if (!verified || terminal != "ready") return KnowledgeAgentSaveState.UNVERIFIED
    return KnowledgeAgentSaveState.READY
}

internal enum class KnowledgeAiMode(val wireValue: String) { DIRECT("direct"), KNOWLEDGE("knowledge") }

internal fun <T> knowledgeAiSourcesForMode(mode: KnowledgeAiMode, loadSources: () -> List<T>): List<T> =
    if (mode == KnowledgeAiMode.DIRECT) emptyList() else loadSources()

internal fun knowledgeAiInitialMode(priorityNoteId: String?): KnowledgeAiMode =
    if (priorityNoteId == null) KnowledgeAiMode.DIRECT else KnowledgeAiMode.KNOWLEDGE

internal fun <T> selectReadableKnowledgeSources(
    candidates: List<T>,
    priorityNoteId: String?,
    id: (T) -> String,
    isReadable: (T) -> Boolean
): List<T> = candidates.filter {
    (priorityNoteId == null || id(it) == priorityNoteId) && isReadable(it)
}

internal fun <T, R : Any> projectKnowledgeSources(candidates: List<T>, project: (T) -> R?): List<R> =
    candidates.mapNotNull { candidate ->
        try { project(candidate) } catch (_: Exception) { null }
    }

internal fun <T> selectKnowledgeAgentSources(
    candidates: List<T>,
    isSupportedSource: (T) -> Boolean,
    isDeleted: (T) -> Boolean,
    isEncrypted: (T) -> Boolean,
    inScope: (T) -> Boolean
): List<T> = candidates.filter { isSupportedSource(it) && !isDeleted(it) && !isEncrypted(it) && inScope(it) }

internal class KnowledgeAiRequestBoundary(initialMode: KnowledgeAiMode = KnowledgeAiMode.KNOWLEDGE) {
    private var generation = 0L
    private var pending: Long? = null
    @Volatile private var closed = false
    private var mode = initialMode

    @Synchronized fun begin(authorized: Boolean, configured: Boolean, question: String, sourceCount: Int): Long? {
        if (closed || pending != null || !authorized || !configured || question.isBlank() ||
            (mode == KnowledgeAiMode.KNOWLEDGE && sourceCount <= 0) ||
            (mode == KnowledgeAiMode.DIRECT && sourceCount != 0)) return null
        return (++generation).also { pending = it }
    }

    @Synchronized fun changeMode(next: KnowledgeAiMode) {
        if (mode != next) { mode = next; generation++ }
    }
    @Synchronized fun isCurrent(ticket: Long): Boolean = !closed && pending == ticket && generation == ticket
    @Synchronized fun invalidate() { generation++ }
    @Synchronized fun close() { closed = true; generation++ }

    @Synchronized fun finish(ticket: Long, identityCurrent: Boolean, ok: Boolean, output: String, message: String): KnowledgeAiOutcome? {
        if (pending != ticket) return null
        pending = null
        if (closed || generation != ticket || !identityCurrent) return null
        val text = output.trim()
        return if (ok && text.isNotBlank()) KnowledgeAiOutcome(true, text, "")
        else KnowledgeAiOutcome(false, "", if (ok) "模型没有返回可读回答，请重试。" else message.ifBlank { "AI 请求未完成，请检查连接后重试。" })
    }
}

internal class KnowledgeKeyboardRequest {
    private var generation = 0L
    private var pending: Long? = null
    private var closed = false

    fun request(): Long = (++generation).also { if (!closed) pending = it }
    fun cancel() { generation++; pending = null }
    fun close() { closed = true; cancel() }

    fun takeReady(ticket: Long, fieldFocused: Boolean, windowFocused: Boolean, resumed: Boolean): Boolean {
        if (closed || pending != ticket || generation != ticket || !fieldFocused || !windowFocused || !resumed) return false
        pending = null
        return true
    }
}

internal class KnowledgeAgentRequestBoundary {
    private var generation = 0L
    private var pending: Long? = null
    private var saving = false
    @Volatile private var closed = false

    @Synchronized fun begin(authorized: Boolean, configured: Boolean, question: String, documentCount: Int): Long? {
        if (closed || pending != null || !authorized || !configured || question.isBlank() || documentCount !in 1..30) return null
        return (++generation).also { pending = it }
    }
    @Synchronized fun isCurrent(ticket: Long): Boolean = !closed && pending == ticket && generation == ticket
    @Synchronized fun invalidate() { generation++ }
    @Synchronized fun finish(ticket: Long, identityCurrent: Boolean, complete: Boolean): Boolean {
        if (pending != ticket) return false
        pending = null
        return !closed && generation == ticket && identityCurrent && complete
    }
    @Synchronized fun canSave(identityCurrent: Boolean, contextCurrent: Boolean, hasDraft: Boolean, userConfirmed: Boolean): Boolean =
        !closed && identityCurrent && contextCurrent && hasDraft && userConfirmed
    @Synchronized fun beginSave(identityCurrent: Boolean, contextCurrent: Boolean, hasDraft: Boolean, userConfirmed: Boolean): Boolean {
        if (saving || !canSave(identityCurrent, contextCurrent, hasDraft, userConfirmed)) return false
        saving = true
        return true
    }
    @Synchronized fun finishSave(saved: Boolean) {
        if (!saving) return
        saving = false
        if (saved) closed = true
    }
    @Synchronized fun close() { closed = true; generation++ }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui
import org.junit.Assert.*
import org.junit.Test

class KnowledgeAiRequestBoundaryTest {
    @Test fun noAuthorizationOrInputCannotSend() {
        for (input in listOf(Triple(false, true, 1), Triple(true, false, 1), Triple(true, true, 0))) {
            assertNull(KnowledgeAiRequestBoundary().begin(input.first, input.second, "问题", input.third))
        }
        assertNull(KnowledgeAiRequestBoundary().begin(true, true, "  ", 1))
    }
    @Test fun duplicateClickCannotStartAnotherRequest() {
        val state = KnowledgeAiRequestBoundary()
        assertNotNull(state.begin(true, true, "总结", 2))
        assertNull(state.begin(true, true, "总结", 2))
    }
    @Test fun emptyOrFailedOutputCannotBecomeAnswer() {
        for ((ok, output) in listOf(true to " ", false to "残留文字")) {
            val state = KnowledgeAiRequestBoundary()
            val ticket = state.begin(true, true, "问题", 1)!!
            val result = state.finish(ticket, true, ok, output, "失败")!!
            assertFalse(result.ok); assertEquals("", result.text)
        }
    }
    @Test fun editedQuestionAndCancelledRequestCannotPublish() {
        val state = KnowledgeAiRequestBoundary()
        val ticket = state.begin(true, true, "旧问题", 1)!!
        state.invalidate()
        assertNull(state.begin(true, true, "新问题", 1))
        assertNull(state.finish(ticket, true, true, "旧回答", ""))
        val next = state.begin(true, true, "新问题", 1)!!
        assertEquals("新回答", state.finish(next, true, true, "新回答", "")!!.text)
    }
    @Test fun cancelledQueueCannotStartTransport() {
        val state = KnowledgeAiRequestBoundary()
        val ticket = state.begin(true, true, "问题", 1)!!
        assertTrue(state.isCurrent(ticket))
        state.invalidate()
        var calls = 0
        if (state.isCurrent(ticket)) calls++
        assertEquals(0, calls)
        state.close()
        assertFalse(state.isCurrent(ticket))
    }
    @Test fun closedOrChangedAccountCannotPublish() {
        val closed = KnowledgeAiRequestBoundary()
        val a = closed.begin(true, true, "问题", 1)!!
        closed.close()
        assertNull(closed.finish(a, true, true, "回答", ""))
        val changed = KnowledgeAiRequestBoundary()
        val b = changed.begin(true, true, "问题", 1)!!
        assertNull(changed.finish(b, false, true, "回答", ""))
    }
    @Test fun directModeAllowsNoKnowledgeButStillRequiresAuthorizationAndInput() {
        assertNotNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT).begin(true, true, "一加一等于几", 0))
        for ((authorized, configured, question) in listOf(Triple(false, true, "问题"), Triple(true, false, "问题"), Triple(true, true, "  "))) {
            assertNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT).begin(authorized, configured, question, 0))
        }
        assertNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT).begin(true, true, "问题", 1))
    }
    @Test fun directModeDoesNotReadOrIncludeKnowledgeSources() {
        var reads = 0
        val direct = knowledgeAiSourcesForMode(KnowledgeAiMode.DIRECT) { reads++; listOf("私人笔记") }
        assertTrue(direct.isEmpty()); assertEquals(0, reads)
        val knowledge = knowledgeAiSourcesForMode(KnowledgeAiMode.KNOWLEDGE) { reads++; listOf("已选节选") }
        assertEquals(listOf("已选节选"), knowledge); assertEquals(1, reads)
    }
    @Test fun modeSwitchInvalidatesQueuedTransportAndStaleAnswer() {
        val state = KnowledgeAiRequestBoundary(KnowledgeAiMode.KNOWLEDGE)
        val ticket = state.begin(true, true, "问题", 1)!!
        state.changeMode(KnowledgeAiMode.DIRECT)
        assertFalse(state.isCurrent(ticket))
        assertNull(state.begin(true, true, "问题", 0))
        assertNull(state.finish(ticket, true, true, "旧知识库回答", ""))
        val next = state.begin(true, true, "问题", 0)!!
        assertEquals("直接回答", state.finish(next, true, true, "直接回答", "")!!.text)
    }
    @Test fun switchingBackDoesNotReviveOldModeOrBypassKnowledgeSources() {
        val state = KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT)
        val ticket = state.begin(true, true, "问题", 0)!!
        state.changeMode(KnowledgeAiMode.KNOWLEDGE)
        state.changeMode(KnowledgeAiMode.DIRECT)
        assertFalse(state.isCurrent(ticket))
        assertNull(state.finish(ticket, true, true, "过期回答", ""))
        state.changeMode(KnowledgeAiMode.KNOWLEDGE)
        assertNull(state.begin(true, true, "问题", 0))
        assertNotNull(state.begin(true, true, "问题", 1))
    }
    @Test fun keyboardWaitsForFocusedResumedWindow() {
        val state = KnowledgeKeyboardRequest()
        val ticket = state.request()
        assertFalse(state.takeReady(ticket, true, false, true))
        assertFalse(state.takeReady(ticket, false, true, true))
        assertFalse(state.takeReady(ticket, true, true, false))
        assertTrue(state.takeReady(ticket, true, true, true))
    }
    @Test fun consumedKeyboardRequestDoesNotReopenAfterBackOrRecomposition() {
        val state = KnowledgeKeyboardRequest()
        val ticket = state.request()
        assertTrue(state.takeReady(ticket, true, true, true))
        assertFalse(state.takeReady(ticket, true, false, true))
        assertFalse(state.takeReady(ticket, true, true, true))
    }
    @Test fun tappingFocusedEditorCanRequestKeyboardAgain() {
        val state = KnowledgeKeyboardRequest()
        val first = state.request()
        assertTrue(state.takeReady(first, true, true, true))
        val second = state.request()
        assertFalse(state.takeReady(first, true, true, true))
        assertTrue(state.takeReady(second, true, true, true))
    }
    @Test fun lostFocusStoppedOrDisposedEditorRejectsDeferredKeyboard() {
        val state = KnowledgeKeyboardRequest()
        val old = state.request()
        state.cancel()
        assertFalse(state.takeReady(old, true, true, true))
        val next = state.request()
        assertTrue(state.takeReady(next, true, true, true))
        val disposed = state.request()
        state.close()
        assertFalse(state.takeReady(disposed, true, true, true))
        assertFalse(state.takeReady(state.request(), true, true, true))
    }
    @Test fun agentRequiresExplicitAuthorizationConfiguredModelAndReadableDocuments() {
        val state = KnowledgeAgentRequestBoundary()
        assertNull(state.begin(false, true, "整理项目", 1))
        assertNull(state.begin(true, false, "整理项目", 1))
        assertNull(state.begin(true, true, "整理项目", 0))
        assertNull(state.begin(true, true, " ", 1))
        assertNull(state.begin(true, true, "整理项目", 31))
        assertNotNull(state.begin(true, true, "整理项目", 1))
    }
    @Test fun agentDuplicateOrStaleWorkspaceCannotContinueOrSave() {
        val state = KnowledgeAgentRequestBoundary()
        val ticket = state.begin(true, true, "整理项目", 2)!!
        assertNull(state.begin(true, true, "整理项目", 2))
        state.invalidate()
        assertFalse(state.isCurrent(ticket))
        assertFalse(state.finish(ticket, true, true))
        assertFalse(state.canSave(identityCurrent = false, contextCurrent = true, hasDraft = true, userConfirmed = true))
    }
    @Test fun agentOnlySavesACompletedDraftAfterUserConfirmation() {
        val state = KnowledgeAgentRequestBoundary()
        val ticket = state.begin(true, true, "整理项目", 1)!!
        assertTrue(state.finish(ticket, identityCurrent = true, complete = true))
        assertFalse(state.canSave(identityCurrent = true, contextCurrent = true, hasDraft = false, userConfirmed = true))
        assertFalse(state.canSave(identityCurrent = true, contextCurrent = true, hasDraft = true, userConfirmed = false))
        assertFalse(state.canSave(identityCurrent = true, contextCurrent = false, hasDraft = true, userConfirmed = true))
        assertTrue(state.canSave(identityCurrent = true, contextCurrent = true, hasDraft = true, userConfirmed = true))
        assertTrue(state.beginSave(identityCurrent = true, contextCurrent = true, hasDraft = true, userConfirmed = true))
        assertFalse(state.beginSave(identityCurrent = true, contextCurrent = true, hasDraft = true, userConfirmed = true))
        state.finishSave(saved = false)
        assertTrue(state.beginSave(identityCurrent = true, contextCurrent = true, hasDraft = true, userConfirmed = true))
        state.finishSave(saved = true)
        assertFalse(state.beginSave(identityCurrent = true, contextCurrent = true, hasDraft = true, userConfirmed = true))
    }
    @Test fun agentDraftContextIncludesWorkspaceQuestionConfigurationAndSelectedPageContent() {
        val config = AiConfiguration("key", "https://api.example.test/v1", "model")
        val page = KnowledgeAgentSource("page-1", "工作记录", "项目", "当前正文")
        val authorized = KnowledgeAgentDraftContext("workspace-a", "identity-a", config, "整理项目", listOf(page))
        assertEquals(authorized, authorized.copy())
        assertNotEquals(authorized, authorized.copy(deepReview = true))
        assertNotEquals(authorized, authorized.copy(workspaceKey = "workspace-b"))
        assertNotEquals(authorized, authorized.copy(identity = "identity-b"))
        assertNotEquals(authorized, authorized.copy(configuration = config.copy(model = "other")))
        assertNotEquals(authorized, authorized.copy(question = "另一项任务"))
        assertNotEquals(authorized, authorized.copy(documents = listOf(page.copy(content = "更新后的正文"))))
    }
    @Test fun oldWorkspaceRunHandleRetainsOnlyItsOwnCancellationId() {
        val old = KnowledgeAgentRunHandle()
        old.started("old-task")
        val replacement = KnowledgeAgentRunHandle()
        replacement.started("new-task")
        assertEquals("old-task", old.taskId)
        old.finished("another-task")
        assertEquals("old-task", old.taskId)
        old.finished("old-task")
        assertNull(old.taskId)
        assertEquals("new-task", replacement.taskId)
    }
    @Test fun agentCannotSaveDraftUntilVerificationPasses() {
        val state = KnowledgeAgentRequestBoundary()
        val ticket = state.begin(true, true, "整理项目", 1)!!
        assertTrue(state.finish(ticket, true, true))
        assertFalse(state.beginSave(true, true, KnowledgeAgentReviewPolicy.canSave(false, true), true))
        assertFalse(state.beginSave(true, true, KnowledgeAgentReviewPolicy.canSave(true, false), true))
        assertTrue(state.beginSave(true, true, KnowledgeAgentReviewPolicy.canSave(true, true), true))
    }
    @Test fun documentEntryStartsKnowledgeAndOrdinaryEntryStartsDirect() {
        assertEquals(KnowledgeAiMode.KNOWLEDGE, knowledgeAiInitialMode("target"))
        assertEquals(KnowledgeAiMode.DIRECT, knowledgeAiInitialMode(null))
    }
    @Test fun wholeDocumentUsesFreshReadableTargetWithoutOtherSources() {
        data class Page(val id: String, val text: String)
        val pages = listOf(Page("other", "unrelated"), Page("target", "current revision"))
        val selected = selectReadableKnowledgeSources(pages, "target", Page::id) { true }
        assertEquals(listOf(Page("target", "current revision")), selected)
        assertTrue(selectReadableKnowledgeSources(pages, "missing", Page::id) { true }.isEmpty())
    }
    @Test fun unreadableWholeDocumentCannotSendEvenWhenOtherPagesExist() {
        data class Page(val id: String, val deleted: Boolean = false, val encrypted: Boolean = false, val document: Boolean = true)
        val targets = listOf(Page("target", deleted = true), Page("target", encrypted = true), Page("target", document = false))
        for (target in targets) {
            val selected = selectReadableKnowledgeSources(listOf(Page("other"), target), "target", Page::id) {
                it.document && !it.deleted && !it.encrypted
            }
            assertTrue(selected.isEmpty())
            assertNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.KNOWLEDGE).begin(true, true, "question", selected.size))
        }
    }
    @Test fun failedSourceProjectionIsIsolatedAndTargetFailureCannotSend() {
        val pages = listOf("bad", "good", "empty")
        fun project(page: String): String? = when (page) {
            "bad" -> throw IllegalStateException("Unreadable legacy record")
            "empty" -> null
            else -> page
        }
        assertEquals(listOf("good"), projectKnowledgeSources(pages, ::project))
        for (id in listOf("bad", "empty")) {
            val selected = selectReadableKnowledgeSources(pages, id, { it }) { true }
            val projected = projectKnowledgeSources(selected, ::project)
            assertTrue(projected.isEmpty())
            assertNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.KNOWLEDGE).begin(true, true, "question", projected.size))
        }
    }

    @Test fun progressRejectsOtherTaskOrWorkspaceAndKeepsCancellationVisible() {
        val current = KnowledgeAgentProgress("owned", 2, "first_pass", "model", 1, 0, 0, true, false, "running", 4)
        val cancelled = current.copy(sequence = 3, cancelRequested = true, elapsedSeconds = 5)
        assertEquals(cancelled, acceptKnowledgeAgentProgress(current, cancelled, "owned", true))
        assertEquals(current, acceptKnowledgeAgentProgress(current, cancelled.copy(taskId = "other"), "owned", true))
        assertEquals(current, acceptKnowledgeAgentProgress(current, cancelled, "owned", false))
        assertNull(acceptKnowledgeAgentProgress(null, current, "other", true))
    }
    @Test fun progressCannotRegressCountersElapsedOrCompletedRun() {
        val ready = KnowledgeAgentProgress("owned", 7, "verifying", "finished", 4, 5, 0, false, false, "ready", 20)
        for (incoming in listOf(ready.copy(sequence = 6), ready.copy(elapsedSeconds = 19),
            ready.copy(requests = 3), ready.copy(toolCalls = 4), ready.copy(terminal = "running"),
            ready.copy(documentsRead = -1), ready.copy(stage = "unknown"))) {
            assertEquals(ready, acceptKnowledgeAgentProgress(ready, incoming, "owned", true))
        }
        assertEquals(ready.copy(sequence = 8, elapsedSeconds = 21),
            acceptKnowledgeAgentProgress(ready, ready.copy(sequence = 8, elapsedSeconds = 21), "owned", true))
    }
    @Test fun launchRequiresSeparatePreviewAndAuthorization() {
        assertEquals(KnowledgeAgentLaunchAction.PREVIEW, knowledgeAgentLaunchAction(false, false, true, false, false))
        assertEquals(KnowledgeAgentLaunchAction.PREVIEW, knowledgeAgentLaunchAction(false, false, true, true, false))
        assertEquals(KnowledgeAgentLaunchAction.PREVIEW, knowledgeAgentLaunchAction(false, false, true, false, true))
        assertEquals(KnowledgeAgentLaunchAction.START, knowledgeAgentLaunchAction(false, false, true, true, true))
        assertEquals(KnowledgeAgentLaunchAction.WAIT, knowledgeAgentLaunchAction(true, false, true, true, true))
        assertEquals(KnowledgeAgentLaunchAction.WAIT, knowledgeAgentLaunchAction(false, true, true, true, true))
        assertEquals(KnowledgeAgentLaunchAction.WAIT, knowledgeAgentLaunchAction(false, false, false, true, true))
    }
    @Test fun boundedSelectionCanAlwaysToggleBackToEmpty() {
        val ids = (1..40).map { "page-$it" }
        val first = toggleKnowledgeAgentSelection(emptySet(), ids)
        assertEquals(30, first.size)
        assertEquals(ids.take(30).toSet(), first)
        assertTrue(toggleKnowledgeAgentSelection(first, ids).isEmpty())
        assertEquals(setOf("page-1"), toggleKnowledgeAgentSelection(emptySet(), listOf("", "page-1", "page-1")))
    }
    @Test fun saveStateExplainsCriticalGatesAndCompletedSaveCannotRepeat() {
        fun state(running: Boolean = false, saving: Boolean = false, saved: Boolean = false,
            terminal: String = "ready", identity: Boolean = true, context: Boolean = true,
            verified: Boolean = true, draft: Boolean = true) =
            knowledgeAgentSaveState(running, saving, saved, terminal, identity, context, verified, draft)
        assertEquals(KnowledgeAgentSaveState.READY, state())
        assertEquals(KnowledgeAgentSaveState.RUNNING, state(running = true))
        assertEquals(KnowledgeAgentSaveState.SAVING, state(saving = true))
        assertEquals(KnowledgeAgentSaveState.SAVED, state(saved = true))
        assertEquals(KnowledgeAgentSaveState.CANCELLED, state(terminal = "cancelled"))
        assertEquals(KnowledgeAgentSaveState.FAILED, state(terminal = "failed"))
        assertEquals(KnowledgeAgentSaveState.REVIEW_INCOMPLETE, state(terminal = "review_incomplete"))
        assertEquals(KnowledgeAgentSaveState.CONTEXT_CHANGED, state(identity = false))
        assertEquals(KnowledgeAgentSaveState.CONTEXT_CHANGED, state(context = false))
        assertEquals(KnowledgeAgentSaveState.UNVERIFIED, state(verified = false))
        assertEquals(KnowledgeAgentSaveState.INCOMPLETE, state(draft = false))
        val boundary = KnowledgeAgentRequestBoundary()
        assertTrue(boundary.beginSave(true, true, true, true))
        boundary.finishSave(true)
        assertFalse(boundary.beginSave(true, true, true, true))
    }

    @Test fun agentScopeIncludesStickyNotesButExcludesDeletedEncryptedAndOutOfFolderSources() {
        data class Page(val id: String, val kind: String = "DOCUMENT", val deleted: Boolean = false, val encrypted: Boolean = false, val folder: String = "A")
        val pages = listOf(Page("allowed"), Page("legacy-sticky", kind = "STICKY"), Page("deleted", deleted = true), Page("encrypted", encrypted = true), Page("other-folder", folder = "B"), Page("unsupported", kind = "ATTACHMENT"))
        val selected = selectKnowledgeAgentSources(
            pages,
            isSupportedSource = { it.kind == "DOCUMENT" || it.kind == "STICKY" },
            isDeleted = Page::deleted,
            isEncrypted = Page::encrypted,
            inScope = { it.folder == "A" }
        )
        assertEquals(listOf("allowed", "legacy-sticky"), selected.map(Page::id))
    }
}
"####;

fn replace(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!("AI workflow anchor is not unique: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

fn render_legacy_sticky_source_fixes(source: &mut String) -> Result<(), String> {
    replace(
        source,
        "import com.ofairyo.gridtimer.data.plainContent\n",
        "import com.ofairyo.gridtimer.data.plainContent\nimport com.ofairyo.gridtimer.data.plainText\nimport com.ofairyo.gridtimer.data.resolvedDocument\n",
    )?;
    replace(
        source,
        "appData.sortedActiveStickyNotes().take(12)",
        "appData.sortedActiveStickyNotes()",
    )?;
    replace(
        source,
        "text = \"这里能看便签内容，也能把它收进文档页面。\",",
        "text = \"共 ${stickySources.size} 条，横向滑动可查看全部；点按可看内容或收进文档页面。\",",
    )?;
    replace(
        source,
        "text = note.plainContent().ifBlank { \"这张便签还没有内容。\" },",
        "text = when {\n                    note.isEncryptionLocked() -> \"这张便签已加密，请先在便签页解锁后再查看原文。\"\n                    else -> note.resolvedDocument().plainText(preserveStructure = true).ifBlank { \"这张便签还没有内容。\" }\n                },",
    )?;
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    let mut source = source.to_owned();
    match path {
        "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" => {
            replace(&mut source,
                "    var selectedDestination by rememberSaveable { mutableStateOf(HomeNavigationDestination.BOARD) }",
                "    var selectedDestination by rememberSaveable { mutableStateOf(HomeNavigationDestination.BOARD) }\n    var pendingKnowledgeAi by rememberSaveable(activeWorkspaceKey) { mutableStateOf(false) }\n    var pendingLegalAi by rememberSaveable(activeWorkspaceKey) { mutableStateOf(false) }\n    var openAiSettings by rememberSaveable(activeWorkspaceKey) { mutableStateOf(false) }")?;
            replace(&mut source,
                "                                NoteStudioSheet(\n                                    initialNoteId = pendingLegalNoteId,",
                "                                NoteStudioSheet(\n                                    openAiOnEntry = pendingKnowledgeAi,\n                                    onAiEntryConsumed = { pendingKnowledgeAi = false },\n                                    onOpenAiSettings = { openAiSettings = true; selectedDestination = HomeNavigationDestination.MY },\n                                    initialNoteId = pendingLegalNoteId,")?;
            replace(&mut source,
                "                            FinanceSheet(\n                                appData = appData,",
                "                            FinanceSheet(\n                                openLegalOnEntry = pendingLegalAi,\n                                onLegalEntryConsumed = { pendingLegalAi = false },\n                                onOpenAiSettings = { openAiSettings = true; selectedDestination = HomeNavigationDestination.MY },\n                                appData = appData,")?;
            replace(&mut source,
                "                                onAiConfigurationSave = { apiKey, baseUrl, model ->",
                "                                initialAiExpanded = openAiSettings,\n                                onOpenKnowledgeAi = { pendingKnowledgeAi = true; selectedDestination = HomeNavigationDestination.NOTEBOOK },\n                                onOpenLegalRisk = { pendingLegalAi = true; selectedDestination = HomeNavigationDestination.FINANCE },\n                                onAiConfigurationSave = { apiKey, baseUrl, model ->")?;
            replace(&mut source,
                "private fun FinanceSheet(\n    appData: AppData,",
                "private fun FinanceSheet(\n    openLegalOnEntry: Boolean = false,\n    onLegalEntryConsumed: () -> Unit = {},\n    onOpenAiSettings: () -> Unit = {},\n    appData: AppData,")?;
            replace(&mut source,
                "    var showLegalRisk by rememberSaveable(workspaceKey) { mutableStateOf(false) }",
                "    var showLegalRisk by rememberSaveable(workspaceKey) { mutableStateOf(false) }\n    LaunchedEffect(openLegalOnEntry, workspaceKey) {\n        if (openLegalOnEntry) { showLegalRisk = true; onLegalEntryConsumed() }\n    }")?;
            replace(&mut source,
                "            onDismiss = { showLegalRisk = false },\n            onOpenEvidence =",
                "            onDismiss = { showLegalRisk = false },\n            onOpenAiSettings = onOpenAiSettings,\n            onOpenEvidence =")?;
            replace(&mut source,
                "                Text(\"账目浏览周期\", style = MaterialTheme.typography.labelMedium)",
                r####"                Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("AI 法律风险分析", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                        Text("扫描任务、笔记、知识页和账目，逐条给出待核查线索与原始记录。", style = MaterialTheme.typography.bodyMedium)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextButton(onClick = { showLegalRisk = true }) { Text("开始 AI 分析") }
                            TextButton(onClick = onOpenAiSettings) { Text("AI 设置") }
                        }
                        Text(if (syncSession.aiApiKey.isBlank()) "先配置 AI，扫描预览可以在本机查看。" else "${syncSession.aiModel} · 发送前查看范围并确认", style = MaterialTheme.typography.bodySmall)
                    }
                }
                Text("账目浏览周期", style = MaterialTheme.typography.labelMedium)"####)?;
        }
        "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt" => {
            for import in [
                "androidx.compose.foundation.clickable",
                "androidx.compose.foundation.interaction.PressInteraction",
                "androidx.compose.foundation.layout.ime",
                "androidx.compose.foundation.layout.imePadding",
                "androidx.compose.foundation.relocation.BringIntoViewRequester",
                "androidx.compose.foundation.relocation.bringIntoViewRequester",
                "androidx.compose.runtime.withFrameNanos",
                "androidx.compose.ui.focus.FocusRequester",
                "androidx.compose.ui.focus.focusRequester",
                "androidx.compose.ui.platform.LocalDensity",
                "androidx.compose.ui.platform.LocalSoftwareKeyboardController",
                "androidx.compose.ui.platform.LocalWindowInfo",
                "androidx.compose.ui.window.DialogProperties",
                "kotlinx.coroutines.flow.collect",
                "java.util.UUID",
                "org.json.JSONArray",
                "org.json.JSONObject",
                "androidx.compose.material3.Checkbox",
                "androidx.compose.material3.OutlinedTextField",
            ] {
                let statement = format!("import {import}\n");
                if !source.contains(&statement) {
                    replace(
                        &mut source,
                        "package com.ofairyo.gridtimer.ui\n",
                        &format!("package com.ofairyo.gridtimer.ui\n{statement}"),
                    )?;
                }
            }
            if !source.contains("import androidx.compose.material3.TextButton\n") {
                replace(&mut source, "package com.ofairyo.gridtimer.ui\n", "package com.ofairyo.gridtimer.ui\nimport androidx.compose.material3.TextButton\n")?;
            }
            replace(&mut source,
                "internal fun NoteStudioSheet(\n    initialNoteId: String? = null,",
                "internal fun NoteStudioSheet(\n    openAiOnEntry: Boolean = false,\n    onAiEntryConsumed: () -> Unit = {},\n    onOpenAiSettings: () -> Unit = {},\n    initialNoteId: String? = null,")?;
            replace(&mut source,
                "    var knowledgePriorityNoteId by remember(workspaceKey) { mutableStateOf<String?>(null) }",
                "    var knowledgePriorityNoteId by remember(workspaceKey) { mutableStateOf<String?>(null) }\n    LaunchedEffect(openAiOnEntry, workspaceKey) {\n        if (openAiOnEntry) { knowledgeDialogVisible = true; knowledgePriorityNoteId = null; onAiEntryConsumed() }\n    }")?;
            replace(&mut source,
                "            priorityNoteId = knowledgePriorityNoteId,\n            viewModel = viewModel,",
                "            priorityNoteId = knowledgePriorityNoteId,\n            onOpenAiSettings = onOpenAiSettings,\n            viewModel = viewModel,")?;
            replace(&mut source,
                "            onSaveAnswer = { question, answer, sources ->\n                val freshNote = buildKnowledgeAnswerNote(question, answer, sources)\n                pendingCreatedNote = freshNote\n                selectedNoteId = freshNote.id\n                searchLocateQuery = null\n                knowledgeDialogVisible = false\n                knowledgePriorityNoteId = null\n                viewModel.upsertNote(freshNote)\n            }",
                "            onSaveAnswer = { question, answer, sources ->\n                val freshNote = buildKnowledgeAnswerNote(question, answer, sources)\n                pendingCreatedNote = freshNote\n                selectedNoteId = freshNote.id\n                searchLocateQuery = null\n                knowledgeDialogVisible = false\n                knowledgePriorityNoteId = null\n                viewModel.upsertNote(freshNote)\n            },\n            onSaveAgentDraft = { title, question, body, sources ->\n                val expectedWorkspaceKey = workspaceKey\n                val freshNote = buildKnowledgeAgentDraftNote(title, question, body, sources)\n                if (viewModel.currentWorkspaceKey() != expectedWorkspaceKey) {\n                    Toast.makeText(context, \"工作区已切换，本次草稿没有保存。\", Toast.LENGTH_LONG).show()\n                } else {\n                    viewModel.upsertNoteAndFlushResult(\n                        note = freshNote,\n                        expectedWorkspaceKey = expectedWorkspaceKey,\n                        reason = \"android_ai_agent_draft\"\n                    ) { result ->\n                        if (result.committed && viewModel.currentWorkspaceKey() == expectedWorkspaceKey) {\n                            pendingCreatedNote = freshNote\n                            selectedNoteId = freshNote.id\n                            searchLocateQuery = null\n                            knowledgeDialogVisible = false\n                            knowledgePriorityNoteId = null\n                            Toast.makeText(context, \"已保存为新的知识页\", Toast.LENGTH_SHORT).show()\n                        } else {\n                            Toast.makeText(context, \"知识页尚未确认保存：\" + (result.failure?.name ?: result.detail), Toast.LENGTH_LONG).show()\n                        }\n                    }\n                }\n            }")?;
            replace(
                &mut source,
                "onSaveAgentDraft = { title, question, body, sources ->",
                "onSaveAgentDraft = { title, question, body, sources, onComplete ->",
            )?;
            replace(&mut source,
                "Toast.makeText(context, \"工作区已切换，本次草稿没有保存。\", Toast.LENGTH_LONG).show()\n                } else {",
                "Toast.makeText(context, \"工作区已切换，本次草稿没有保存。\", Toast.LENGTH_LONG).show()\n                    onComplete(false, \"工作区已切换，本次草稿没有保存。\")\n                } else {")?;
            replace(&mut source,
                "Toast.makeText(context, \"已保存为新的知识页\", Toast.LENGTH_SHORT).show()",
                "Toast.makeText(context, \"已保存为新的知识页\", Toast.LENGTH_SHORT).show()\n                            onComplete(true, \"\")")?;
            replace(&mut source,
                "Toast.makeText(context, \"知识页尚未确认保存：\" + (result.failure?.name ?: result.detail), Toast.LENGTH_LONG).show()",
                "val message = \"知识页尚未确认保存：\" + (result.failure?.name ?: result.detail)\n                            Toast.makeText(context, message, Toast.LENGTH_LONG).show()\n                            onComplete(false, message)")?;
            let start = source
                .find("@Composable\nprivate fun KnowledgeAiDialog(")
                .ok_or("missing knowledge AI dialog")?;
            let end = source[start..]
                .find("@Composable\nprivate fun KnowledgeSourceCard(")
                .map(|offset| start + offset)
                .ok_or("missing knowledge source card")?;
            source.replace_range(start..end, KNOWLEDGE_DIALOG);
            // Provider-neutral errors also apply to the note editor.
            source = source.replace(
                "请先在“我的”里填写 OpenAI API Key",
                "请先在“我的 → AI”中配置密钥、接口和模型。",
            );
            render_legacy_sticky_source_fixes(&mut source)?;
        }
        "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt" => {
            source = source.replace(
                "请先在“我的”里填写 OpenAI API Key",
                "请先在“我的 → AI”中配置密钥、接口和模型。",
            );
        }
        _ => {}
    }
    Ok(source)
}

#[cfg(test)]
mod sticky_source_render_tests {
    use super::render_legacy_sticky_source_fixes;

    #[test]
    fn legacy_sticky_references_are_complete_and_preview_keeps_full_text() {
        let mut source = [
            "import com.ofairyo.gridtimer.data.plainContent",
            "val stickySources = appData.sortedActiveStickyNotes().take(12)",
            "text = \"这里能看便签内容，也能把它收进文档页面。\",",
            "text = note.plainContent().ifBlank { \"这张便签还没有内容。\" },",
        ]
        .join("\n");

        render_legacy_sticky_source_fixes(&mut source).unwrap();

        assert!(source.contains("appData.sortedActiveStickyNotes()"));
        assert!(!source.contains(".take(12)"));
        assert!(source.contains("横向滑动可查看全部"));
        assert!(source.contains("${stickySources.size}"));
        assert!(source.contains("note.isEncryptionLocked()"));
        assert!(source.contains("import com.ofairyo.gridtimer.data.plainText"));
        assert!(source.contains("import com.ofairyo.gridtimer.data.resolvedDocument"));
        assert!(source.contains("note.resolvedDocument().plainText(preserveStructure = true)"));
        assert!(!source.contains("text = note.plainContent()"));
    }
}

const KNOWLEDGE_DIALOG: &str = r####"private fun buildKnowledgeAgentDraftNote(
    title: String,
    question: String,
    body: String,
    sources: List<KnowledgeSourceCandidate>,
    now: Long = System.currentTimeMillis()
): NoteEntry {
    val references = sources.joinToString("\n") { source ->
        val folder = source.folderName.takeIf(String::isNotBlank)?.let { " · $it" }.orEmpty()
        "- ${source.note.displayTitle()}$folder · 来源编号 ${source.note.id}"
    }
    val noteBody = buildString {
        append("## 任务目标\n").append(question.trim()).append("\n\n")
        append(body.trim())
        if (references.isNotBlank()) append("\n\n## 原始来源\n").append(references)
    }
    return NoteEntry(
        title = title.trim().ifBlank { "AI Agent 草稿" },
        content = noteBody,
        kind = NoteEntryKind.DOCUMENT,
        document = NoteDocument(markdownEnabled = true, blocks = listOf(NoteBlock(type = NoteBlockType.TEXT, text = noteBody))),
        accentSeed = "blue",
        folderId = sources.firstOrNull()?.note?.folderId,
        createdAtEpochMillis = now,
        updatedAtEpochMillis = now
    )
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun KnowledgeAiDialog(
    appData: AppData,
    syncSession: SyncAccountSession,
    selectedFolderId: String?,
    priorityNoteId: String?,
    onOpenAiSettings: () -> Unit,
    viewModel: TimerViewModel,
    onDismiss: () -> Unit,
    onOpenSource: (KnowledgeSourceCandidate) -> Unit,
    onSaveAnswer: (String, String, List<KnowledgeSourceCandidate>) -> Unit,
    onSaveAgentDraft: (String, String, String, List<KnowledgeSourceCandidate>, (Boolean, String) -> Unit) -> Unit
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val lifecycleOwner = LocalLifecycleOwner.current
    val workspaceKey = LocalNoteMediaWorkspaceKey.current
    val identity = listOf(syncSession.token, syncSession.userId, syncSession.serverInstanceId, syncSession.accountNamespace).joinToString("\u0000")
    val configuration = AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel)
    val latestIdentity by rememberUpdatedState(identity)
    val latestConfiguration by rememberUpdatedState(configuration)
    var queryMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(KnowledgeAiMode.DIRECT) }
    var agentMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    val latestMode by rememberUpdatedState(queryMode)
    val boundary = remember(workspaceKey, identity, priorityNoteId) { KnowledgeAiRequestBoundary(queryMode) }
    val latestBoundary by rememberUpdatedState(boundary)
    val agentBoundary = remember(workspaceKey, identity, priorityNoteId) { KnowledgeAgentRequestBoundary() }
    val latestAgentBoundary by rememberUpdatedState(agentBoundary)
    var question by rememberSaveable(workspaceKey, priorityNoteId) { mutableStateOf("") }
    var searchScope by rememberSaveable(workspaceKey, priorityNoteId, selectedFolderId) {
        mutableStateOf(if (selectedFolderId == null) KnowledgeSearchScope.ALL else KnowledgeSearchScope.CURRENT_FOLDER)
    }
    var answer by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var answerQuestion by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var answerSources by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<KnowledgeSourceCandidate>>(emptyList()) }
    var statusText by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var busy by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var cancelling by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var agentBusy by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var agentCancelling by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var agentTaskId by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<String?>(null) }
    val agentRunHandle = remember(agentBoundary) { KnowledgeAgentRunHandle() }
    var agentDraft by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<KnowledgeAgentDraft?>(null) }
    var agentDraftContext by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<KnowledgeAgentDraftContext?>(null) }
    var agentDraftTitle by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var agentDraftBody by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var agentSaving by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var agentPreviewAcknowledged by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var agentResultSources by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<KnowledgeSourceCandidate>>(emptyList()) }
    var agentTrace by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<String>>(emptyList()) }
    val agentSelectedIds = rememberSaveable(workspaceKey, identity, selectedFolderId) { mutableStateListOf<String>() }
    var showSendingContent by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    val selectedFolderName = appData.findNoteFolder(selectedFolderId)?.name
    val sourceCandidates = if (agentMode) emptyList() else remember(queryMode, appData.notes, appData.noteFolders, question, searchScope, selectedFolderId, priorityNoteId) {
        knowledgeAiSourcesForMode(queryMode) {
            buildKnowledgeSourceCandidates(appData, question, selectedFolderId, searchScope, priorityNoteId)
        }
    }
    val agentAvailableDocuments = remember(appData.notes, appData.noteFolders, selectedFolderId, searchScope) {
        selectKnowledgeAgentSources(
            candidates = appData.notes,
            isSupportedSource = { it.kind == NoteEntryKind.DOCUMENT || it.kind == NoteEntryKind.STICKY },
            isDeleted = NoteEntry::isDeleted,
            isEncrypted = { it.encryption != null },
            inScope = { note -> searchScope == KnowledgeSearchScope.ALL || selectedFolderId == null || note.folderId == selectedFolderId }
        )
    }
    val selectedAgentDocuments = remember(agentAvailableDocuments, agentSelectedIds.toSet()) {
        val selectedIds = agentSelectedIds.toSet()
        agentAvailableDocuments.filter { it.id in selectedIds }.map { note ->
            KnowledgeAgentSource(
                id = note.id,
                title = "${if (note.kind == NoteEntryKind.STICKY) "便签" else "知识页"} · ${note.displayTitle()}",
                folder = note.folderId?.let { appData.findNoteFolder(it)?.name }.orEmpty(),
                content = note.plainContent()
            )
        }.filter { it.content.isNotBlank() }
    }
    val sourceSignature = sourceCandidates.map { listOf(it.note.id, it.note.displayTitle(), it.folderName, it.excerpt) }
    val latestSourceSignature by rememberUpdatedState(sourceSignature)
    val latestQuestion by rememberUpdatedState(question)
    val latestSelectedAgentDocuments by rememberUpdatedState(selectedAgentDocuments)
    val latestAgentMode by rememberUpdatedState(agentMode)
    val latestAgentTaskId by rememberUpdatedState(agentTaskId)
    var observedAgentDocuments by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(selectedAgentDocuments) }
    var observedAgentConfiguration by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(configuration) }
    var observedSources by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(sourceSignature) }
    var observedConfiguration by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(configuration) }

    fun invalidateAnswer() {
        boundary.invalidate()
        answer = ""; answerQuestion = ""; answerSources = emptyList()
        statusText = if (busy) "问题或资料已改变，本次回答将不再采用。" else ""
        cancelling = busy
    }
    fun stopAgent(reason: String) {
        agentBoundary.invalidate()
        agentRunHandle.taskId?.let { taskId -> runCatching { NativeOptimizerBridge.cancelAndroidKnowledgeAgent(taskId) } }
        if (agentMode) {
            showSendingContent = false
            agentPreviewAcknowledged = false
        }
        if (agentBusy) {
            agentCancelling = true
            statusText = "$reason；当前模型请求结束后会停止后续步骤。"
        } else if (agentDraft != null || agentDraftContext != null) {
            agentDraft = null; agentDraftContext = null; agentDraftTitle = ""; agentDraftBody = ""
            agentResultSources = emptyList(); agentTrace = emptyList()
            statusText = "$reason，旧草稿已失效，请重新运行 Agent。"
        }
    }
    fun setAgentMode(enabled: Boolean) {
        if (agentMode == enabled) return
        if (agentBusy) stopAgent("已取消 Agent")
        boundary.invalidate()
        invalidateAnswer()
        agentBoundary.invalidate()
        agentMode = enabled
        showSendingContent = false
        agentPreviewAcknowledged = false
        agentDraft = null; agentDraftContext = null; agentDraftTitle = ""; agentDraftBody = ""; agentResultSources = emptyList(); agentTrace = emptyList()
        if (enabled) statusText = "先选择资料并查看发送范围，再点击授权并执行。"
    }
    fun changeMode(next: KnowledgeAiMode) {
        if (queryMode == next) return
        if (agentMode) setAgentMode(false)
        boundary.changeMode(next)
        invalidateAnswer()
        queryMode = next
        showSendingContent = false
        if (busy) statusText = "模式已切换，旧请求结束前不能重复发送。"
    }
    DisposableEffect(boundary, agentBoundary, lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) {
                boundary.invalidate()
                if (busy) { cancelling = true; statusText = "已停止采用本次回答，返回后可重新发送。" }
                if (agentBusy) stopAgent("应用切到后台")
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            boundary.close(); agentBoundary.close()
            agentRunHandle.taskId?.let { taskId -> runCatching { NativeOptimizerBridge.cancelAndroidKnowledgeAgent(taskId) } }
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }
    LaunchedEffect(sourceSignature, configuration) {
        if (observedSources != sourceSignature || observedConfiguration != configuration) {
            invalidateAnswer()
            observedSources = sourceSignature
            observedConfiguration = configuration
        }
    }
    LaunchedEffect(selectedAgentDocuments, configuration) {
        if (observedAgentDocuments != selectedAgentDocuments || observedAgentConfiguration != configuration) {
            stopAgent("资料或 AI 配置已改变")
            observedAgentDocuments = selectedAgentDocuments
            observedAgentConfiguration = configuration
        }
    }
    LaunchedEffect(agentAvailableDocuments.map(NoteEntry::id)) {
        val availableIds = agentAvailableDocuments.mapTo(HashSet(), NoteEntry::id)
        agentSelectedIds.retainAll(availableIds)
    }

    fun requestAnswer() {
        if (agentMode || agentBusy) return
        val safeQuestion = question.trim()
        val sources = sourceCandidates.toList()
        observedSources = sourceSignature
        observedConfiguration = configuration
        val ticket = boundary.begin(true, configuration.testable, safeQuestion, sources.size) ?: return
        val requestMode = queryMode
        val requestIdentity = identity
        val requestConfiguration = configuration
        val requestSignature = sourceSignature
        answer = ""; answerQuestion = ""; answerSources = emptyList()
        busy = true; cancelling = false; statusText = ""
        scope.launch {
            try {
                val result = withContext(Dispatchers.IO) {
                    if (!boundary.isCurrent(ticket) || latestBoundary !== boundary || viewModel.currentWorkspaceKey() != workspaceKey ||
                        latestIdentity != requestIdentity || latestConfiguration != requestConfiguration ||
                        latestQuestion.trim() != safeQuestion || latestMode != requestMode || latestSourceSignature != requestSignature) null
                    else NativeOptimizerBridge.completeAndroidAiQuery(
                        apiKey = requestConfiguration.apiKey.trim(),
                        baseUrl = requestConfiguration.baseUrl.trim(),
                        model = requestConfiguration.model.trim(),
                        mode = requestMode.wireValue,
                        question = safeQuestion,
                        sourceTitles = sources.map { it.note.displayTitle() }.toTypedArray(),
                        sourceFolders = sources.map(KnowledgeSourceCandidate::folderName).toTypedArray(),
                        sourceExcerpts = sources.map(KnowledgeSourceCandidate::excerpt).toTypedArray()
                    )
                }
                val current = latestBoundary === boundary && latestIdentity == requestIdentity && latestConfiguration == requestConfiguration &&
                    viewModel.currentWorkspaceKey() == workspaceKey && latestQuestion.trim() == safeQuestion &&
                    latestMode == requestMode && latestSourceSignature == requestSignature
                val outcome = boundary.finish(ticket, current, result?.getOrNull(0)?.toBooleanStrictOrNull() == true,
                    result?.getOrNull(2).orEmpty(), result?.getOrNull(1).orEmpty())
                if (outcome != null && latestBoundary === boundary) {
                    if (outcome.ok) {
                        answer = outcome.text; answerQuestion = safeQuestion; answerSources = sources
                        statusText = if (requestMode == KnowledgeAiMode.DIRECT) "AI 回答完成" else "回答完成 · ${sources.size} 个来源"
                    } else statusText = outcome.message
                }
            } finally {
                if (latestBoundary === boundary) {
                    val interrupted = cancelling
                    busy = false; cancelling = false
                    if (interrupted) statusText = "请求已结束，可以重新发送。"
                }
            }
        }
    }

    fun requestAgent() {
        if (agentBusy || agentSaving) return
        val safeQuestion = question.trim()
        val documents = selectedAgentDocuments.toList()
        val ticket = agentBoundary.begin(
            authorized = showSendingContent && agentPreviewAcknowledged,
            configured = configuration.testable,
            question = safeQuestion,
            documentCount = documents.size
        ) ?: run {
            statusText = when {
                !showSendingContent || !agentPreviewAcknowledged -> "先展开并核对完整发送预览，再勾选授权。"
                !configuration.testable -> "请先完成 AI 配置并测试连接。"
                safeQuestion.isBlank() -> "请先写明需要 Agent 处理的目标。"
                documents.isEmpty() -> "请至少选择一条未加密便签或知识页。"
                else -> "Agent 正在处理，请等待当前任务结束。"
            }
            return
        }
        val runId = UUID.randomUUID().toString()
        val runIdentity = identity
        val runConfiguration = configuration
        val runContext = KnowledgeAgentDraftContext(workspaceKey, runIdentity, runConfiguration, safeQuestion, documents)
        val scopeJson = JSONObject().put("question", safeQuestion).put("documents", JSONArray().apply {
            documents.forEach { document ->
                put(JSONObject().put("id", document.id).put("title", document.title)
                    .put("folder", document.folder).put("content", document.content))
            }
        }).toString()
        agentRunHandle.started(runId)
        agentTaskId = runId
        agentBusy = true; agentCancelling = false; agentDraft = null; agentDraftContext = null; agentDraftTitle = ""; agentDraftBody = ""; agentSaving = false; agentPreviewAcknowledged = false
        agentResultSources = emptyList(); agentTrace = emptyList(); statusText = "正在连接 ${runConfiguration.recipientHost}，最多执行 6 次模型请求。"
        answer = ""; answerSources = emptyList()
        scope.launch {
            try {
                val raw = withContext(Dispatchers.IO) {
                    val current = agentBoundary.isCurrent(ticket) && latestAgentBoundary === agentBoundary && latestAgentMode &&
                        viewModel.currentWorkspaceKey() == workspaceKey && latestIdentity == runIdentity &&
                        latestConfiguration == runConfiguration && latestQuestion.trim() == safeQuestion &&
                        latestSelectedAgentDocuments == documents
                    if (!current) null else NativeOptimizerBridge.runAndroidKnowledgeAgent(
                        apiKey = runConfiguration.apiKey.trim(), baseUrl = runConfiguration.baseUrl.trim(),
                        model = runConfiguration.model.trim(), scopeJson = scopeJson, taskId = runId
                    )
                }
                val parsed = raw?.let { runCatching { JSONObject(it) }.getOrNull() }
                val wireOk = parsed?.optBoolean("ok", false) == true
                val draftJson = parsed?.optJSONObject("draft")
                val sourceIds = draftJson?.optJSONArray("sourceIds")?.let { array ->
                    (0 until array.length()).mapNotNull { index -> array.optString(index).takeIf(String::isNotBlank) }
                }.orEmpty()
                val validSourceIds = sourceIds.isNotEmpty() && sourceIds.distinct().size == sourceIds.size && sourceIds.all { id -> documents.any { it.id == id } }
                val wireDraft = if (wireOk && draftJson != null && validSourceIds) KnowledgeAgentDraft(
                    title = draftJson.optString("title").trim(),
                    content = draftJson.optString("content").trim(),
                    actionItems = draftJson.optJSONArray("actionItems")?.let { array -> (0 until array.length()).map { index -> array.optString(index) } }.orEmpty(),
                    sourceIds = sourceIds
                ).takeIf { it.title.isNotBlank() && it.content.isNotBlank() } else null
                val current = latestAgentBoundary === agentBoundary && latestIdentity == runIdentity && latestConfiguration == runConfiguration &&
                    viewModel.currentWorkspaceKey() == workspaceKey && latestQuestion.trim() == safeQuestion && latestAgentMode &&
                    latestSelectedAgentDocuments == documents
                val completed = agentBoundary.finish(ticket, current, wireOk && wireDraft != null)
                if (completed && latestAgentBoundary === agentBoundary) {
                    val draft = wireDraft!!
                    agentDraft = draft
                    agentDraftContext = runContext
                    agentDraftTitle = draft.title
                    agentDraftBody = buildString {
                        append(draft.content)
                        if (draft.actionItems.isNotEmpty()) {
                            append("\n\n## 待办草稿\n")
                            draft.actionItems.forEach { append("- [ ] ").append(it).append('\n') }
                        }
                    }.trim()
                    agentResultSources = documents.filter { it.id in draft.sourceIds }.mapNotNull { document ->
                        val note = agentAvailableDocuments.firstOrNull { it.id == document.id } ?: return@mapNotNull null
                        KnowledgeSourceCandidate(note, document.folder, document.content.take(760))
                    }
                    agentTrace = parsed?.optJSONArray("toolTrace")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optJSONObject(index)?.let { "${it.optString("summary")} · ${it.optInt("resultCount")} 项" } }
                    }.orEmpty()
                    answer = parsed?.optString("answer").orEmpty()
                    statusText = "Agent 已生成草稿，尚未保存到知识库。"
                } else if (current) {
                    statusText = parsed?.optString("message")?.takeIf(String::isNotBlank) ?: "Agent 未能完整完成本次任务。"
                }
            } finally {
                runCatching { NativeOptimizerBridge.finishAndroidKnowledgeAgent(runId) }
                agentRunHandle.finished(runId)
                if (latestAgentBoundary === agentBoundary) {
                    val wasCancelling = agentCancelling
                    agentBusy = false; agentCancelling = false
                    if (agentTaskId == runId) agentTaskId = null
                    if (!wasCancelling && agentDraft == null && statusText.isBlank()) statusText = "Agent 请求已结束。"
                }
            }
        }
    }

    fun currentAgentDraftContext() = KnowledgeAgentDraftContext(
        workspaceKey = viewModel.currentWorkspaceKey(),
        identity = latestIdentity,
        configuration = latestConfiguration,
        question = latestQuestion.trim(),
        documents = latestSelectedAgentDocuments
    )

    fun saveAgentDraft() {
        if (agentSaving) return
        val authorizedContext = agentDraftContext
        val current = viewModel.currentWorkspaceKey() == workspaceKey && latestIdentity == identity
        val hasDraft = agentDraft != null && agentDraftTitle.isNotBlank() && agentDraftBody.isNotBlank() && agentResultSources.isNotEmpty()
        if (!agentBoundary.beginSave(
                identityCurrent = current,
                contextCurrent = authorizedContext != null && authorizedContext == currentAgentDraftContext(),
                hasDraft = hasDraft,
                userConfirmed = true
            )) {
            statusText = "草稿或工作区已变化，请重新运行 Agent 后再保存。"
            return
        }
        agentSaving = true
        onSaveAgentDraft(agentDraftTitle.trim(), authorizedContext!!.question, agentDraftBody.trim(), agentResultSources) { saved, message ->
            agentBoundary.finishSave(saved)
            agentSaving = false
            if (saved) {
                agentDraftContext = null
                statusText = "已保存为新的知识页。"
            } else {
                statusText = message.ifBlank { "知识页尚未确认保存，请检查后重试。" }
            }
        }
    }

    Dialog(onDismissRequest = { if (agentBusy) stopAgent("已关闭 Agent 面板"); onDismiss() }, properties = DialogProperties(decorFitsSystemWindows = false)) {
        Box(modifier = Modifier.fillMaxWidth().imePadding().navigationBarsPadding()
            .windowInsetsPadding(WindowInsets.statusBars.only(WindowInsetsSides.Top))) {
        Surface(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(22.dp), color = flowusPanelBackground(), border = BorderStroke(1.dp, flowusBorderColor())) {
            LazyColumn(modifier = Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                item {
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                        Text(if (agentMode) "AI Agent" else "AI 问答", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                        CapsuleAction(text = "关闭", icon = Icons.Rounded.Close, onClick = { if (agentBusy) stopAgent("已关闭 Agent 面板"); onDismiss() })
                    }
                }
                item {
                    Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                        Column(modifier = Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(if (configuration.testable) "${configuration.model} · ${configuration.recipientHost}" else "AI 尚未配置完整", fontWeight = FontWeight.SemiBold)
                            Text(when {
                                agentMode -> "使用本机工具检索你选择的便签和知识页，生成可编辑的新页草稿；不会自动修改或保存。"
                                queryMode == KnowledgeAiMode.DIRECT -> "直接向 AI 提问，仅发送你的问题。"
                                else -> "依据所选知识页节选回答，并列出来源。"
                            }, style = MaterialTheme.typography.bodySmall)
                            TextButton(onClick = { boundary.invalidate(); onDismiss(); onOpenAiSettings() }) { Text(if (configuration.testable) "更改 AI 设置" else "配置 DeepSeek / 其他 AI") }
                        }
                    }
                }
                item {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        ChoicePill(text = "直接问 AI", selected = !agentMode && queryMode == KnowledgeAiMode.DIRECT, accent = notesAccentColor,
                            onClick = { setAgentMode(false); changeMode(KnowledgeAiMode.DIRECT) })
                        ChoicePill(text = "问知识库", selected = !agentMode && queryMode == KnowledgeAiMode.KNOWLEDGE, accent = accentFor("blue"),
                            onClick = { setAgentMode(false); changeMode(KnowledgeAiMode.KNOWLEDGE) })
                        ChoicePill(text = "任务 Agent", selected = agentMode, accent = accentFor("green"),
                            onClick = { setAgentMode(true) })
                    }
                }
                if (agentMode || queryMode == KnowledgeAiMode.KNOWLEDGE) item {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (selectedFolderId != null) ChoicePill(text = selectedFolderName ?: "当前文档库", selected = searchScope == KnowledgeSearchScope.CURRENT_FOLDER, accent = notesAccentColor,
                            onClick = { stopAgent("资料范围已改变"); invalidateAnswer(); agentSelectedIds.clear(); searchScope = KnowledgeSearchScope.CURRENT_FOLDER })
                        ChoicePill(text = if (agentMode) "全部未加密资料" else "全部知识页", selected = searchScope == KnowledgeSearchScope.ALL, accent = accentFor("blue"),
                            onClick = { stopAgent("资料范围已改变"); invalidateAnswer(); agentSelectedIds.clear(); searchScope = KnowledgeSearchScope.ALL })
                    }
                }
                if (agentMode) item {
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                        Text("授权资料 ${selectedAgentDocuments.size}/30", fontWeight = FontWeight.SemiBold)
                        TextButton(onClick = {
                            stopAgent("资料选择已改变")
                            if (selectedAgentDocuments.size == agentAvailableDocuments.size) agentSelectedIds.clear()
                            else { agentSelectedIds.clear(); agentAvailableDocuments.take(30).forEach { agentSelectedIds.add(it.id) } }
                        }) { Text(if (selectedAgentDocuments.size == agentAvailableDocuments.size) "清空选择" else "全选当前范围") }
                    }
                    Text("可选当前未删除、未加密的便签和知识页；每条资料需单独勾选。附件、财务、计时、归档记录、密钥和诊断资料不在本次范围。", style = MaterialTheme.typography.bodySmall)
                }
                if (agentMode) items(agentAvailableDocuments, key = { "agent-doc-${it.id}" }) { note ->
                    Row(modifier = Modifier.fillMaxWidth().clickable {
                        stopAgent("资料选择已改变")
                        if (note.id in agentSelectedIds) agentSelectedIds.remove(note.id)
                        else if (agentSelectedIds.size < 30) agentSelectedIds.add(note.id)
                    }, verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = note.id in agentSelectedIds, onCheckedChange = { checked ->
                            stopAgent("资料选择已改变")
                            if (checked && note.id !in agentSelectedIds && agentSelectedIds.size < 30) agentSelectedIds.add(note.id)
                            if (!checked) agentSelectedIds.remove(note.id)
                        })
                        Column(modifier = Modifier.weight(1f)) {
                            Text("${if (note.kind == NoteEntryKind.STICKY) "便签" else "知识页"} · ${note.displayTitle()}", maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = FontWeight.Medium)
                            Text(note.folderId?.let { appData.findNoteFolder(it)?.name }.orEmpty().ifBlank { "未归类" }, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
                item(key = "knowledge-question") {
                    KnowledgeQuestionField(question = question,
                        hint = when { agentMode -> "例如：梳理项目进展，整理未完成事项并生成新页面"; queryMode == KnowledgeAiMode.DIRECT -> "例如：一加一等于几"; else -> "例如：总结这些记录里需要我处理的事情" },
                        onQuestionChange = { stopAgent("任务目标已改变"); invalidateAnswer(); question = it })
                }
                if (question.isBlank()) item {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        for (example in when { agentMode -> listOf("整理项目进展并列出待办", "归纳这组知识页的共同结论", "草拟一份复盘页面"); queryMode == KnowledgeAiMode.DIRECT -> listOf("一加一等于几", "解释复利是怎么计算的"); else -> listOf("总结最近的记录", "找出待办和未解决的问题", "整理主要观点") }) {
                            TextButton(onClick = { stopAgent("任务目标已改变"); invalidateAnswer(); question = example }) { Text(example) }
                        }
                    }
                }
                item {
                    val selectedChars = selectedAgentDocuments.sumOf { it.content.length }
                    Text(when {
                        agentMode -> "接收方：${configuration.recipientHost.ifBlank { "未配置" }} · 模型：${configuration.model.ifBlank { "未配置" }} · 已选 ${selectedAgentDocuments.size} 条资料，正文共 $selectedChars 字符；先发问题和工具说明，再按需发送读取片段，最多 6 次请求。"
                        queryMode == KnowledgeAiMode.DIRECT -> "将发送：仅你的问题"
                        else -> "将发送：你的问题 + ${sourceCandidates.size} 个知识页节选"
                    }, style = MaterialTheme.typography.bodySmall)
                    if (agentMode) Text("每轮会携带此前的工具记录，资料读取累计上限 32,000 字符；请求设置 store:false，但自定义服务的留存规则由服务商决定。", style = MaterialTheme.typography.bodySmall)
                    if (agentMode) Text("请展开完整预览并核对接收方、模型和所选资料；预览末尾勾选授权后才能启动。草稿仍需检查、编辑并再次确认才会保存。", style = MaterialTheme.typography.bodySmall)
                    if (!configuration.testable) Text("先点上方配置入口，填写密钥、接口根地址和模型。", style = MaterialTheme.typography.bodySmall)
                    else if (question.isBlank()) Text("输入问题，或点选上面的示例。", style = MaterialTheme.typography.bodySmall)
                    else if (agentMode && selectedAgentDocuments.isEmpty()) Text("请至少勾选一条未加密便签或知识页。", style = MaterialTheme.typography.bodySmall)
                    else if (!agentMode && queryMode == KnowledgeAiMode.KNOWLEDGE && sourceCandidates.isEmpty()) Text("当前范围没有可读知识页。可切到全部知识页，或改用直接问 AI。", style = MaterialTheme.typography.bodySmall)
                    TextButton(onClick = {
                        showSendingContent = !showSendingContent
                        agentPreviewAcknowledged = false
                    }) { Text(if (showSendingContent) "收起发送内容" else "查看将发送的资料") }
                    PhysicalButton(label = when { agentMode && agentBusy -> if (agentCancelling) "正在停止后续步骤" else "Agent 正在执行"; agentMode -> "授权本次范围并运行 Agent"; busy -> if (cancelling) "等待本次请求结束" else "AI 正在回答"; else -> "发送问题" }, icon = Icons.Rounded.Title,
                        accent = if (agentMode) accentFor("green") else notesAccentColor, filled = true,
                        enabled = if (agentMode) !agentBusy && !agentSaving && showSendingContent && agentPreviewAcknowledged && configuration.testable && question.isNotBlank() && selectedAgentDocuments.isNotEmpty()
                            else !busy && !agentBusy && configuration.testable && question.isNotBlank() && (queryMode == KnowledgeAiMode.DIRECT || sourceCandidates.isNotEmpty()),
                        modifier = Modifier.fillMaxWidth(), onClick = if (agentMode) ::requestAgent else ::requestAnswer)
                }
                if (busy) item {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                        TextButton(onClick = { boundary.invalidate(); cancelling = true; statusText = "已取消，结束前不能发起重复请求。" }, enabled = !cancelling) { Text("取消本次回答") }
                    }
                }
                if (agentBusy) item {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                        TextButton(onClick = { stopAgent("已取消 Agent") }, enabled = !agentCancelling) { Text("取消并停止后续请求") }
                    }
                }
                if (statusText.isNotBlank()) item {
                    Text(statusText, style = MaterialTheme.typography.bodyMedium, color = if (answer.isBlank() && !busy) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)
                }
                if (answer.isNotBlank()) {
                    item {
                        FlowusPanel(accent = notesAccentColor) { AndroidRenderedMarkdown(answer, modifier = Modifier.padding(14.dp)) }
                        TextButton(onClick = {
                            val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as? android.content.ClipboardManager
                            clipboard?.setPrimaryClip(android.content.ClipData.newPlainText("AI 回答", answer))
                            Toast.makeText(context, "已复制回答原文", Toast.LENGTH_SHORT).show()
                        }) { Text("复制回答原文") }
                        if (!agentMode) {
                            PhysicalButton(label = "保存回答为知识页", icon = Icons.Rounded.SaveAlt, accent = accentFor("green"), filled = false, enabled = !busy && !agentBusy,
                                modifier = Modifier.fillMaxWidth(), onClick = { onSaveAnswer(answerQuestion, answer, answerSources) })
                        }
                    }
                    if (answerSources.isNotEmpty()) item { Text("回答依据 ${answerSources.size}", fontWeight = FontWeight.SemiBold) }
                    items(answerSources, key = { "answer-${it.note.id}" }) { source -> KnowledgeSourceCard(source, answerSources.indexOf(source) + 1) { onOpenSource(source) } }
                }
                if (agentMode && agentDraft != null) {
                    item {
                        Text("新知识页草稿 · 尚未保存", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                        OutlinedTextField(value = agentDraftTitle, onValueChange = { agentDraftTitle = it }, label = { Text("新知识页标题") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
                        OutlinedTextField(value = agentDraftBody, onValueChange = { agentDraftBody = it }, label = { Text("可编辑草稿内容与待办") }, modifier = Modifier.fillMaxWidth(), minLines = 6, maxLines = 18)
                        if (agentTrace.isNotEmpty()) {
                            Text("本次工具记录", fontWeight = FontWeight.SemiBold)
                            agentTrace.forEach { traceLine -> Text("· $traceLine", style = MaterialTheme.typography.bodySmall) }
                        }
                        Text("来源 ${agentResultSources.size} 条；原文未修改。点击来源可查看原始资料。", style = MaterialTheme.typography.bodySmall)
                        PhysicalButton(label = if (agentSaving) "正在核验保存" else "确认保存为新知识页", icon = Icons.Rounded.SaveAlt, accent = accentFor("green"), filled = true,
                            enabled = !agentBusy && !agentSaving && agentBoundary.canSave(
                                identityCurrent = viewModel.currentWorkspaceKey() == workspaceKey && latestIdentity == identity,
                                contextCurrent = agentDraftContext != null && agentDraftContext == currentAgentDraftContext(),
                                hasDraft = agentDraftBody.isNotBlank() && agentDraftTitle.isNotBlank() && agentResultSources.isNotEmpty(),
                                userConfirmed = true
                            ), modifier = Modifier.fillMaxWidth(), onClick = ::saveAgentDraft)
                    }
                    items(agentResultSources, key = { "agent-source-${it.note.id}" }) { source -> KnowledgeSourceCard(source, agentResultSources.indexOf(source) + 1) { onOpenSource(source) } }
                }
                if (showSendingContent) {
                    item { Text(if (agentMode) "本次授权范围与正文预览" else "待发送的问题", fontWeight = FontWeight.SemiBold); Text(question.ifBlank { "尚未输入问题" }) }
                    if (agentMode) item { Text("授权 ${selectedAgentDocuments.size} 条资料 · 总计 ${selectedAgentDocuments.sumOf { it.content.length }} 字符，模型按需检索和读取", fontWeight = FontWeight.Medium) }
                    if (!agentMode && queryMode == KnowledgeAiMode.KNOWLEDGE) item { Text("待发送的来源节选 ${sourceCandidates.size}", fontWeight = FontWeight.SemiBold) }
                    if (agentMode) items(selectedAgentDocuments, key = { "agent-preview-${it.id}" }) { document ->
                        FlowusPanel(accent = accentFor("blue")) {
                            Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                Text(document.title, fontWeight = FontWeight.SemiBold)
                                Text("${document.folder.ifBlank { "未归类" }} · ${document.content.length} 字符", style = MaterialTheme.typography.bodySmall)
                                AndroidRenderedMarkdown(document.content, modifier = Modifier.fillMaxWidth())
                            }
                        }
                    }
                    if (!agentMode) items(sourceCandidates, key = { "preview-${it.note.id}" }) { source -> KnowledgeSourceCard(source, sourceCandidates.indexOf(source) + 1) { onOpenSource(source) } }
                    if (agentMode) item(key = "agent-preview-authorization") {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Checkbox(
                                checked = agentPreviewAcknowledged,
                                enabled = !agentBusy,
                                onCheckedChange = { checked -> agentPreviewAcknowledged = checked && showSendingContent }
                            )
                            Text(
                                "我已核对上方完整资料、接收方和模型，并授权本次发送。",
                                modifier = Modifier.clickable { agentPreviewAcknowledged = showSendingContent },
                                style = MaterialTheme.typography.bodySmall
                            )
                        }
                    }
                }
            }
        }
        }
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)
@Composable
private fun KnowledgeQuestionField(question: String, hint: String, onQuestionChange: (String) -> Unit) {
    // These locals must belong to the Dialog, not the Activity underneath it.
    val keyboard = LocalSoftwareKeyboardController.current
    val windowInfo = LocalWindowInfo.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val focusRequester = remember { FocusRequester() }
    val bringIntoView = remember { BringIntoViewRequester() }
    val interactions = remember { MutableInteractionSource() }
    val request = remember(lifecycleOwner) { KnowledgeKeyboardRequest() }
    var fieldFocused by remember { mutableStateOf(false) }
    var ticket by remember(request) { mutableStateOf(0L) }
    var resumed by remember(lifecycleOwner) {
        mutableStateOf(lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED))
    }
    val currentFocused by rememberUpdatedState(fieldFocused)
    val windowFocused = windowInfo.isWindowFocused
    val imeBottom = WindowInsets.ime.getBottom(LocalDensity.current)

    DisposableEffect(request, lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) resumed = true
            if (event == Lifecycle.Event.ON_PAUSE || event == Lifecycle.Event.ON_STOP) resumed = false
            if (event == Lifecycle.Event.ON_STOP) request.cancel()
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { request.close(); lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(interactions, request) {
        interactions.interactions.collect { interaction ->
            if (interaction is PressInteraction.Release) {
                focusRequester.requestFocus()
                ticket = request.request()
            }
        }
    }
    LaunchedEffect(request, ticket, fieldFocused, windowFocused, resumed) {
        if (!fieldFocused || !windowFocused || !resumed) return@LaunchedEffect
        // Let the dialog editor establish its input connection before showing IME.
        withFrameNanos { }
        if (request.takeReady(ticket, currentFocused, windowInfo.isWindowFocused,
                lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED))) {
            keyboard?.show()
            bringIntoView.bringIntoView()
        }
    }
    LaunchedEffect(imeBottom, fieldFocused) {
        if (imeBottom > 0 && fieldFocused) bringIntoView.bringIntoView()
    }
    SmartisanTextField(value = question, onValueChange = onQuestionChange, label = "你的问题",
        hint = hint, maxLength = 1000, minLines = 2, maxLines = 5,
        modifier = Modifier.fillMaxWidth(),
        fieldModifier = Modifier.focusRequester(focusRequester).bringIntoViewRequester(bringIntoView),
        interactionSource = interactions,
        onFocusChanged = { focused ->
            fieldFocused = focused
            if (focused) ticket = request.request() else request.cancel()
        })
}

"####;
