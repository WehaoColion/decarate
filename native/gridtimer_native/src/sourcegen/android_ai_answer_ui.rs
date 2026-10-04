// v2.23.2.7 - Load only the current offline document and finish from its real render state.
// v2.23.2.6 - Render Android Markdown and formulas offline without changing stored answers.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/AndroidRenderedMarkdown.kt";
pub const BOUNDARY_PATH: &str = "com/ofairyo/gridtimer/ui/AndroidMarkdownRenderBoundary.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/AndroidMarkdownRenderBoundaryTest.kt";

pub const BOUNDARY_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

internal const val ANDROID_MARKDOWN_DOCUMENT_PREFIX = "https://appassets.androidplatform.net/ai-answer/"

internal fun androidMarkdownDocumentTrusted(url: String?): Boolean = url != null &&
    Regex("^https://appassets\\.androidplatform\\.net/ai-answer/[a-f0-9]{32}/index\\.html$").matches(url)

internal fun androidMarkdownDocumentAllowed(url: String?, documentUrl: String?): Boolean =
    androidMarkdownDocumentTrusted(documentUrl) && url == documentUrl

internal fun androidMarkdownResourceAllowed(url: String?, documentUrl: String? = null, mainFrame: Boolean = false): Boolean =
    url?.startsWith("data:font/woff2;base64,") == true ||
        (mainFrame && androidMarkdownDocumentAllowed(url, documentUrl))

internal enum class AndroidMarkdownRenderState { LOADING, READY, FAILED_SOURCE, FAILED_DOM, FAILED_SCRIPT, FAILED_LOAD, CLOSED }

internal class AndroidMarkdownRenderBoundary {
    private var generation = 0L
    private var closed = false
    private var documentUrl: String? = null
    var state = AndroidMarkdownRenderState.LOADING
        private set

    fun begin(url: String): Long {
        documentUrl = url
        state = if (closed) AndroidMarkdownRenderState.CLOSED else AndroidMarkdownRenderState.LOADING
        return ++generation
    }
    fun isCurrent(ticket: Long): Boolean = !closed && generation == ticket &&
        state in setOf(AndroidMarkdownRenderState.LOADING, AndroidMarkdownRenderState.READY)
    fun close() { closed = true; generation++; state = AndroidMarkdownRenderState.CLOSED }

    fun complete(ticket: Long, url: String?, rootPresent: Boolean, formulaCount: Int,
        scriptReady: Boolean, renderedCount: Int, errorCount: Int): AndroidMarkdownRenderState? {
        if (!isCurrent(ticket)) return null
        state = when {
            !androidMarkdownDocumentAllowed(url, documentUrl) -> AndroidMarkdownRenderState.FAILED_SOURCE
            !rootPresent || formulaCount < 0 || renderedCount < 0 || errorCount < 0 -> AndroidMarkdownRenderState.FAILED_DOM
            formulaCount == 0 -> AndroidMarkdownRenderState.READY
            scriptReady && renderedCount.toLong() + errorCount.toLong() == formulaCount.toLong() -> AndroidMarkdownRenderState.READY
            else -> AndroidMarkdownRenderState.FAILED_SCRIPT
        }
        return state
    }
    fun fail(ticket: Long): Boolean {
        if (!isCurrent(ticket)) return false
        state = AndroidMarkdownRenderState.FAILED_LOAD
        return true
    }

    fun acceptHeight(ticket: Long, measuredWidth: Int, currentWidth: Int, cssHeight: Double): Float? {
        if (!isCurrent(ticket) || state != AndroidMarkdownRenderState.READY || measuredWidth <= 0 || measuredWidth != currentWidth ||
            !cssHeight.isFinite() || cssHeight <= 0.0 || cssHeight > 300_000.0) return null
        return cssHeight.toFloat() + 2f
    }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui
import org.junit.Assert.*
import org.junit.Test

class AndroidMarkdownRenderBoundaryTest {
    private val documentUrl = ANDROID_MARKDOWN_DOCUMENT_PREFIX + "0123456789abcdef0123456789abcdef/index.html"
    private fun ready(state: AndroidMarkdownRenderBoundary): Long = state.begin(documentUrl).also {
        assertEquals(AndroidMarkdownRenderState.READY, state.complete(it, documentUrl, true, 0, false, 0, 0))
    }
    @Test fun networkAndActiveDocumentsCannotLoad() {
        for (url in listOf(null, "https://api.deepseek.com", "http://127.0.0.1/",
            "file:///sdcard/document.html", "content://private/file", "javascript:alert(1)",
            "data:text/html;base64,AA==", "data:application/javascript;base64,AA==",
            "DATA:font/woff2;base64,AA==")) assertFalse(androidMarkdownResourceAllowed(url))
        assertTrue(androidMarkdownResourceAllowed("data:font/woff2;base64,AA=="))
    }
    @Test fun onlyExactCurrentMainDocumentCanLoadOffline() {
        assertTrue(androidMarkdownResourceAllowed(documentUrl, documentUrl, true))
        assertFalse(androidMarkdownResourceAllowed(documentUrl, documentUrl, false))
        for (url in listOf("https://api.deepseek.com", documentUrl + "?x=1", documentUrl + "#a",
            documentUrl.replace("01234567", "11111111"), "about:blank", "data:text/html,body"))
            assertFalse(androidMarkdownResourceAllowed(url, documentUrl, true))
        assertFalse(androidMarkdownResourceAllowed("https://api.deepseek.com", "https://api.deepseek.com", true))
        assertFalse(androidMarkdownDocumentTrusted(documentUrl.replace("https://", "http://")))
    }
    @Test fun plainMarkdownNeedsItsDocumentAndBodyWithoutMathBootstrap() {
        val state = AndroidMarkdownRenderBoundary()
        val ticket = state.begin(documentUrl)
        assertEquals(AndroidMarkdownRenderState.READY, state.complete(ticket, documentUrl, true, 0, false, 0, 0))
        assertNotNull(state.acceptHeight(ticket, 360, 360, 120.0))
    }
    @Test fun incompleteFormulaOrForeignDocumentCannotBecomeReady() {
        val state = AndroidMarkdownRenderBoundary()
        var ticket = state.begin(documentUrl)
        assertEquals(AndroidMarkdownRenderState.FAILED_SCRIPT, state.complete(ticket, documentUrl, true, 2, false, 0, 0))
        assertNull(state.acceptHeight(ticket, 360, 360, 120.0))
        ticket = state.begin(documentUrl)
        assertEquals(AndroidMarkdownRenderState.FAILED_SCRIPT, state.complete(ticket, documentUrl, true, 2, true, 1, 0))
        ticket = state.begin(documentUrl)
        assertEquals(AndroidMarkdownRenderState.FAILED_SOURCE, state.complete(ticket, "about:blank", true, 0, true, 0, 0))
        ticket = state.begin(documentUrl)
        assertEquals(AndroidMarkdownRenderState.FAILED_DOM, state.complete(ticket, documentUrl, false, 0, true, 0, 0))
        ticket = state.begin(documentUrl)
        assertEquals(AndroidMarkdownRenderState.READY, state.complete(ticket, documentUrl, true, 2, true, 1, 1))
    }
    @Test fun explicitFailureCanRetryWithoutAcceptingOldCompletion() {
        val state = AndroidMarkdownRenderBoundary()
        val old = state.begin(documentUrl)
        assertTrue(state.fail(old))
        assertNull(state.complete(old, documentUrl, true, 0, true, 0, 0))
        val next = state.begin(documentUrl)
        assertNull(state.complete(old, documentUrl, true, 0, true, 0, 0))
        assertEquals(AndroidMarkdownRenderState.READY, state.complete(next, documentUrl, true, 0, false, 0, 0))
    }
    @Test fun disposedOrSupersededPageCannotResizeCurrentAnswer() {
        val state = AndroidMarkdownRenderBoundary()
        val old = ready(state)
        val current = ready(state)
        assertNull(state.acceptHeight(old, 360, 360, 100.0))
        assertEquals(102f, state.acceptHeight(current, 360, 360, 100.0)!!, 0f)
        state.close()
        assertNull(state.acceptHeight(current, 360, 360, 100.0))
        assertFalse(state.isCurrent(state.begin(documentUrl)))
    }
    @Test fun staleWidthAndInvalidHeightCannotHideAnswer() {
        val state = AndroidMarkdownRenderBoundary()
        val ticket = ready(state)
        assertNull(state.acceptHeight(ticket, 360, 400, 100.0))
        assertNull(state.acceptHeight(ticket, 0, 0, 100.0))
        for (height in listOf(0.0, -1.0, Double.NaN, Double.POSITIVE_INFINITY, 300_001.0))
            assertNull(state.acceptHeight(ticket, 360, 360, height))
        assertEquals(20002f, state.acceptHeight(ticket, 360, 360, 20000.0)!!, 0f)
    }
}
"####;

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.annotation.SuppressLint
import android.graphics.Color
import android.util.Log
import android.view.MotionEvent
import android.view.View
import android.view.ViewParent
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.webkit.ConsoleMessage
import android.webkit.WebChromeClient
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import java.io.ByteArrayInputStream
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import kotlin.math.abs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray

private class AndroidMarkdownViewHolder {
    val boundary = AndroidMarkdownRenderBoundary()
    val documentToken = UUID.randomUUID().toString().replace("-", "")
    val documentUrl = ANDROID_MARKDOWN_DOCUMENT_PREFIX + documentToken + "/index.html"
    var webView: WebView? = null
    var loadTicket = 0L
    var pageFinished = false
    val callbacks = mutableListOf<Runnable>()
    val blockedResources = AtomicInteger(0)

    fun cancelMeasurements() {
        callbacks.forEach { webView?.removeCallbacks(it) }
        callbacks.clear()
    }
    fun dispose() {
        boundary.close()
        cancelMeasurements()
        webView?.let { view -> view.stopLoading(); view.webViewClient = WebViewClient(); view.webChromeClient = null; view.destroy() }
        webView = null
    }
}

private const val MARKDOWN_RENDER_LOG = "AndroidMarkdownRender"
private const val MARKDOWN_RENDER_SNAPSHOT = """
(()=>{const root=document.getElementById('android-ai-answer');const all=root?root.querySelectorAll('.math-source[data-latex]'):[];return [String(location.href),!!root,root?root.dataset.renderSession:'',all.length,document.documentElement.dataset.mathReady==='true'&&typeof katex==='object'&&typeof katex.render==='function',root?root.querySelectorAll('[data-math-rendered=true]').length:0,root?root.querySelectorAll('[data-math-error=true]').length:0,document.readyState,typeof katex==='object'];})()
"""

@SuppressLint("SetJavaScriptEnabled", "ClickableViewAccessibility")
@Composable
internal fun AndroidRenderedMarkdown(content: String, modifier: Modifier = Modifier) {
    val darkTheme = MaterialTheme.colorScheme.surface.luminance() < 0.45f
    val fontScale = LocalDensity.current.fontScale
    var attempt by remember(content, darkTheme) { mutableStateOf(0) }
    var html by remember(content, darkTheme, attempt) { mutableStateOf<String?>(null) }
    var renderingFailure by remember(content, darkTheme, attempt) { mutableStateOf<String?>(null) }

    LaunchedEffect(content, darkTheme, attempt) {
        Log.i(MARKDOWN_RENDER_LOG, "native_begin chars=${content.length} attempt=$attempt")
        val rendered = withContext(Dispatchers.IO) {
            NativeOptimizerBridge.renderAndroidAiAnswerHtml(content, darkTheme)
        }
        if (rendered.isNullOrBlank()) {
            Log.w(MARKDOWN_RENDER_LOG, "native_render_empty")
            renderingFailure = "native_render_empty"
        } else {
            Log.i(MARKDOWN_RENDER_LOG, "native_ready chars=${rendered.length}")
            html = rendered
        }
    }

    if (renderingFailure != null) {
        Column(modifier = modifier.fillMaxWidth()) {
            Text("排版未完成，已保留原文。", style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error)
            TextButton(onClick = { attempt++ }) { Text("重试排版") }
            SelectionContainer { Text(content, style = MaterialTheme.typography.bodyMedium) }
        }
        return
    }
    val document = html
    if (document == null) {
        Text("正在排版…", modifier = modifier, style = MaterialTheme.typography.bodySmall)
        return
    }

    key(content, darkTheme, attempt) {
        val holder = remember { AndroidMarkdownViewHolder() }
        val trustedDocument = remember(document, holder) {
            document.replaceFirst("<main id=\"android-ai-answer\">",
                "<main id=\"android-ai-answer\" data-render-session=\"${holder.documentToken}\">")
        }
        val documentBytes = remember(trustedDocument) { trustedDocument.toByteArray(Charsets.UTF_8) }
        var height by remember { mutableStateOf(48f) }
        var formulaErrors by remember { mutableStateOf(0) }
        DisposableEffect(holder) { onDispose { holder.dispose() } }

        fun fail(reason: String) {
            Log.w(MARKDOWN_RENDER_LOG, "failed reason=$reason")
            renderingFailure = reason
        }

        fun measure(view: WebView, fontChecks: Int = 20) {
            holder.cancelMeasurements()
            val ticket = holder.loadTicket
            val width = view.width
            if (width <= 0 || !holder.pageFinished || !holder.boundary.isCurrent(ticket) ||
                holder.boundary.state != AndroidMarkdownRenderState.READY) return
            // Fonts can finish after the DOM. This only refreshes geometry; it never declares a load failure.
            view.evaluateJavascript("[Math.ceil(document.body.getBoundingClientRect().height),!document.fonts||document.fonts.status==='loaded']") { output ->
                if (!holder.boundary.isCurrent(ticket) || holder.webView !== view) return@evaluateJavascript
                val result = runCatching { JSONArray(output) }.getOrNull() ?: return@evaluateJavascript
                val measured = holder.boundary.acceptHeight(ticket, width, view.width, result.optDouble(0))
                if (measured != null && abs(measured - height) > 1f) height = measured
                if (!result.optBoolean(1) && fontChecks > 0 && width == view.width) {
                val callback = Runnable {
                    if (!holder.boundary.isCurrent(ticket) || holder.webView !== view) return@Runnable
                    measure(view, fontChecks - 1)
                }
                holder.callbacks += callback
                view.postDelayed(callback, 100L)
                }
            }
        }

        fun finish(view: WebView) {
            val ticket = holder.loadTicket
            if (!holder.boundary.isCurrent(ticket) || holder.webView !== view) return
            view.evaluateJavascript(MARKDOWN_RENDER_SNAPSHOT) { output ->
                if (!holder.boundary.isCurrent(ticket) || holder.webView !== view) return@evaluateJavascript
                val result = runCatching { JSONArray(output) }.getOrNull()
                if (result == null) { holder.boundary.fail(ticket); fail("snapshot_unreadable"); return@evaluateJavascript }
                val currentUrl = result.optString(0)
                val rootPresent = result.optBoolean(1) && result.optString(2) == holder.documentToken &&
                    result.optString(7) == "complete"
                val count = result.optInt(3, -1)
                val scriptReady = result.optBoolean(4)
                val rendered = result.optInt(5, -1)
                val errors = result.optInt(6, -1)
                val state = holder.boundary.complete(ticket, currentUrl, rootPresent, count, scriptReady, rendered, errors) ?: return@evaluateJavascript
                Log.i(MARKDOWN_RENDER_LOG, "dom state=$state root=$rootPresent formulas=$count rendered=$rendered errors=$errors script=$scriptReady readyState=${result.optString(7)}")
                if (state != AndroidMarkdownRenderState.READY) { fail(state.name); return@evaluateJavascript }
                formulaErrors = errors
                if (count == 0 && !scriptReady) Log.w(MARKDOWN_RENDER_LOG, "plain_document_ready math_bootstrap_absent")
                view.postVisualStateCallback(ticket, object : WebView.VisualStateCallback() {
                    override fun onComplete(requestId: Long) {
                        if (!holder.boundary.isCurrent(ticket) || holder.webView !== view) return
                        Log.i(MARKDOWN_RENDER_LOG, "visual_ready formulas=$count rendered=$rendered errors=$errors")
                        measure(view)
                    }
                })
            }
        }

        Column(modifier = modifier.fillMaxWidth()) {
        if (formulaErrors > 0) Text("部分公式无法解析，已保留对应原文。", style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error)
        AndroidView(
            factory = { context ->
                object : WebView(context) {
                    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
                        super.onSizeChanged(w, h, oldw, oldh)
                        if (w != oldw && holder.webView === this) {
                            measure(this)
                        }
                    }
                }.apply {
                    holder.webView = this
                    holder.loadTicket = holder.boundary.begin(holder.documentUrl)
                    setBackgroundColor(Color.TRANSPARENT)
                    overScrollMode = View.OVER_SCROLL_NEVER
                    isVerticalScrollBarEnabled = false
                    isHorizontalScrollBarEnabled = false
                    settings.javaScriptEnabled = true
                    settings.javaScriptCanOpenWindowsAutomatically = false
                    settings.domStorageEnabled = false
                    settings.cacheMode = WebSettings.LOAD_NO_CACHE
                    settings.allowFileAccess = false
                    settings.allowContentAccess = false
                    settings.allowFileAccessFromFileURLs = false
                    settings.allowUniversalAccessFromFileURLs = false
                    settings.blockNetworkLoads = true
                    settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
                    settings.safeBrowsingEnabled = true
                    settings.setSupportMultipleWindows(false)
                    settings.useWideViewPort = false
                    settings.loadWithOverviewMode = false
                    settings.builtInZoomControls = false
                    settings.displayZoomControls = false
                    settings.setSupportZoom(false)
                    settings.textZoom = (fontScale * 100).toInt().coerceIn(50, 300)
                    webChromeClient = object : WebChromeClient() {
                        override fun onConsoleMessage(message: ConsoleMessage): Boolean {
                            // Console text may contain equation source; log only level and source line.
                            Log.w(MARKDOWN_RENDER_LOG, "script_console level=${message.messageLevel()} line=${message.lineNumber()}")
                            return true
                        }
                    }
                    var downX = 0f
                    var downY = 0f
                    setOnTouchListener { view, event ->
                        when (event.actionMasked) {
                            MotionEvent.ACTION_DOWN -> { downX = event.x; downY = event.y }
                            MotionEvent.ACTION_MOVE -> markdownParentIntercept(view,
                                abs(event.x - downX) > abs(event.y - downY))
                            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> markdownParentIntercept(view, false)
                        }
                        false
                    }
                    webViewClient = object : WebViewClient() {
                        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                            markdownResourceResponse(request.url.toString(), request.isForMainFrame, holder, documentBytes)
                        @Suppress("DEPRECATION")
                        override fun shouldInterceptRequest(view: WebView, url: String?): WebResourceResponse? =
                            markdownResourceResponse(url, false, holder, documentBytes)
                        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                            !(request.isForMainFrame && androidMarkdownDocumentAllowed(request.url.toString(), holder.documentUrl))
                        @Suppress("DEPRECATION")
                        override fun shouldOverrideUrlLoading(view: WebView, url: String?): Boolean = !androidMarkdownDocumentAllowed(url, holder.documentUrl)
                        override fun onPageStarted(view: WebView, url: String?, favicon: android.graphics.Bitmap?) {
                            Log.i(MARKDOWN_RENDER_LOG, "page_started current=${androidMarkdownDocumentAllowed(url, holder.documentUrl)}")
                        }
                        override fun onPageFinished(view: WebView, url: String?) {
                            if (holder.webView === view && holder.boundary.isCurrent(holder.loadTicket)) {
                                if (!androidMarkdownDocumentAllowed(url, holder.documentUrl)) {
                                    Log.w(MARKDOWN_RENDER_LOG, "page_finished_foreign")
                                    return
                                }
                                Log.i(MARKDOWN_RENDER_LOG, "page_finished current=true")
                                holder.pageFinished = true
                                finish(view)
                            }
                        }
                        override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                            if (request.isForMainFrame && holder.boundary.fail(holder.loadTicket)) fail("main_frame_${error.errorCode}")
                        }
                        override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                            if (holder.webView === view) fail("renderer_process_gone")
                            holder.dispose()
                            return true
                        }
                    }
                    Log.i(MARKDOWN_RENDER_LOG, "load_local_document chars=${trustedDocument.length}")
                    // Resolve this exact URL from the memory response, avoiding an internal data: main frame.
                    loadUrl(holder.documentUrl)
                }
            },
            modifier = Modifier.fillMaxWidth().height(height.dp),
            update = { view ->
                val zoom = (fontScale * 100).toInt().coerceIn(50, 300)
                if (view.settings.textZoom != zoom) { view.settings.textZoom = zoom; measure(view) }
            }
        )
        }
    }
}

private fun markdownResourceResponse(url: String?, mainFrame: Boolean, holder: AndroidMarkdownViewHolder,
    documentBytes: ByteArray): WebResourceResponse? {
    if (androidMarkdownResourceAllowed(url, holder.documentUrl, mainFrame)) {
        if (mainFrame && androidMarkdownDocumentAllowed(url, holder.documentUrl)) {
            Log.i(MARKDOWN_RENDER_LOG, "main_document_memory_response")
            return WebResourceResponse("text/html", "utf-8", 200, "OK",
                mapOf("Cache-Control" to "no-store", "X-Content-Type-Options" to "nosniff"), ByteArrayInputStream(documentBytes))
        }
        return null
    }
    if (holder.blockedResources.incrementAndGet() <= 4) Log.w(MARKDOWN_RENDER_LOG, "resource_blocked main=$mainFrame")
    return WebResourceResponse("text/plain", "utf-8", 403, "Blocked", emptyMap(), ByteArrayInputStream(ByteArray(0)))
}

private fun markdownParentIntercept(view: View, disallow: Boolean) {
    var parent: ViewParent? = view.parent
    while (parent != null) { parent.requestDisallowInterceptTouchEvent(disallow); parent = parent.parent }
}
"####;
