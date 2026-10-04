// v2.22.39 - Recognize current and legacy backup names in missing-attachment warnings.
// v2.22.33 - Keep attachment completeness visible throughout export, save and share.

pub const PATH: &str = "com/ofairyo/gridtimer/ui/DiagnosticsExportUi.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" {
        return Ok(base.to_owned());
    }
    let start = "@Composable\nprivate fun DiagnosticsActionPanel(";
    let end = "@Composable\nprivate fun XiaomiIslandDiagnosticsPanel(";
    if base.matches(start).count() != 1 || base.matches(end).count() != 1 {
        return Err("diagnostic panel boundaries changed".into());
    }
    let from = base.find(start).unwrap();
    let to = base[from..].find(end).unwrap() + from;
    let mut source = base.to_owned();
    source.replace_range(from..to, PANEL);
    let anchor = "    TimerNotificationEffects(slots = appData.slots)";
    if source.matches(anchor).count() != 1 {
        return Err("diagnostic activity host anchor changed".into());
    }
    source = source.replacen(
        anchor,
        "    DiagnosticsExportHost()\n    TimerNotificationEffects(slots = appData.slots)",
        1,
    );
    let share_start = source
        .find("internal fun shareFileWithChooser(")
        .ok_or("missing file share helper")?;
    let share_end = source[share_start..]
        .find("internal fun shareTextWithChooser(")
        .ok_or("missing text share boundary")?
        + share_start;
    let old = &source[share_start..share_end];
    let new = old.replace("addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)",
        "clipData = shareIntent.clipData\n        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)\n        if (context !is Activity) addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)");
    if old == new {
        return Err("missing chooser URI grant flags".into());
    }
    source.replace_range(share_start..share_end, &new);
    Ok(source)
}

const PANEL: &str = r####"@Composable
private fun DiagnosticsActionPanel(
    appData: AppData,
    onPrepareDataExport: suspend () -> PreparedDataExport
) {
    ReliableDiagnosticsPanel(appData, onPrepareDataExport)
}

"####;

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.Unarchive
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import com.ofairyo.gridtimer.data.AppData
import com.ofairyo.gridtimer.data.PreparedDataExport
import com.ofairyo.gridtimer.diagnostics.DiagnosticLogStore
import com.ofairyo.gridtimer.diagnostics.DiagnosticRecordingSession
import com.ofairyo.gridtimer.diagnostics.DiagnosticRecordingSessionStore
import com.ofairyo.gridtimer.diagnostics.DiagnosticsExporter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

internal data class DiagnosticExportState(
    val busy: String? = null,
    val filePath: String? = null,
    val mimeType: String = "text/plain",
    val message: String? = null,
    val errorDetail: String? = null,
    val pickerPending: Boolean = false,
    val pickerLaunched: Boolean = false
)

// The activity owns this ViewModel. Scrolling the diagnostic card out of the
// lazy grid must not cancel a backup or strand a restored "exporting" flag.
internal class DiagnosticsExportViewModel(private val saved: SavedStateHandle) : ViewModel() {
    private val mutable = MutableStateFlow(DiagnosticExportState(
        filePath = saved["diagnostic_export_path"],
        mimeType = saved["diagnostic_export_mime"] ?: "text/plain",
        pickerPending = saved["diagnostic_picker_pending"] ?: false,
        pickerLaunched = saved["diagnostic_picker_launched"] ?: false
    ))
    val state = mutable.asStateFlow()

    fun startCollection(context: Context) = changeCollection(context, true)

    fun stopCollection(context: Context) = changeCollection(context, false)

    private fun changeCollection(context: Context, start: Boolean) {
        if (mutable.value.busy != null || mutable.value.pickerPending) return
        val appContext = context.applicationContext
        mutable.value = mutable.value.copy(busy = if (start) "正在开始收集" else "正在停止收集",
            message = null, errorDetail = null)
        viewModelScope.launch {
            try {
                val collection = withContext(Dispatchers.IO) {
                    if (start) DiagnosticRecordingSessionStore.start(appContext)
                    else DiagnosticRecordingSessionStore.finish(appContext)
                }
                check(collection.isActive == start) { collection.lastError ?: "收集状态未更新，请重试" }
                mutable.value = mutable.value.copy(message = if (start) "已开始收集，可离开此页复现问题" else "已停止收集，可以导出日志")
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                reportFailure(appContext, if (start) "开始收集失败" else "停止收集失败", failure)
            } finally {
                mutable.value = mutable.value.copy(busy = null)
            }
        }
    }

    fun exportLog(context: Context, data: AppData) {
        val appContext = context.applicationContext
        generate(appContext, "正在生成日志", "text/plain") {
            DiagnosticsExporter.recordDiagnosticLog(appContext, data)
        }
    }

    fun exportData(context: Context, prepare: suspend () -> PreparedDataExport) {
        val appContext = context.applicationContext
        generate(appContext, "正在生成数据包", "application/zip") {
            DiagnosticsExporter.exportAppData(appContext, prepare())
        }
    }

    private fun generate(context: Context, progress: String, mime: String, exporter: suspend () -> File) {
        if (mutable.value.busy != null || mutable.value.pickerPending) return
        mutable.value = mutable.value.copy(busy = progress, message = null, errorDetail = null)
        viewModelScope.launch {
            try {
                val file = withContext(Dispatchers.IO) {
                    exporter().also { requireExportFile(context, it) }
                }
                saved["diagnostic_export_path"] = file.absolutePath
                saved["diagnostic_export_mime"] = mime
                mutable.value = DiagnosticExportState(filePath = file.absolutePath, mimeType = mime,
                    message = "文件已生成，请保存或分享")
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                reportFailure(context, "生成失败", failure)
            } finally {
                mutable.value = mutable.value.copy(busy = null)
            }
        }
    }

    fun requestSave(context: Context) {
        if (mutable.value.busy != null || mutable.value.pickerPending) return
        try {
            requireExportFile(context, File(requireNotNull(mutable.value.filePath)))
            saved["diagnostic_picker_path"] = mutable.value.filePath
            saved["diagnostic_picker_pending"] = true
            saved["diagnostic_picker_launched"] = false
            mutable.value = mutable.value.copy(pickerPending = true, pickerLaunched = false,
                message = null, errorDetail = null)
        } catch (failure: Exception) {
            reportFailure(context, "无法保存", failure)
        }
    }

    fun markPickerLaunched() {
        saved["diagnostic_picker_launched"] = true
        mutable.value = mutable.value.copy(pickerLaunched = true)
    }

    fun pickerFailed(context: Context, failure: Exception) {
        clearPicker()
        reportFailure(context, "无法打开文件选择器，可改用分享", failure)
    }

    private fun clearPicker() {
        saved.remove<String>("diagnostic_picker_path")
        saved["diagnostic_picker_pending"] = false
        saved["diagnostic_picker_launched"] = false
        mutable.value = mutable.value.copy(pickerPending = false, pickerLaunched = false)
    }

    fun saveResult(context: Context, uri: Uri?) {
        if (!mutable.value.pickerPending) return
        val source = (saved.get<String>("diagnostic_picker_path") ?: mutable.value.filePath)?.let(::File)
        clearPicker()
        if (uri == null) {
            mutable.value = mutable.value.copy(message = "已取消保存，文件仍可分享")
            return
        }
        if (source == null || mutable.value.busy != null) return
        val appContext = context.applicationContext
        mutable.value = mutable.value.copy(busy = "正在保存", message = null, errorDetail = null)
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    requireExportFile(appContext, source)
                    val expectedSize = source.length()
                    val copied = appContext.contentResolver.openOutputStream(uri, "wt")?.use { output ->
                        source.inputStream().buffered().use { input -> input.copyTo(output, 64 * 1024) }
                    } ?: throw IOException("无法写入所选位置，请重新选择")
                    check(copied == expectedSize && source.length() == expectedSize) { "文件未完整写入，请重试" }
                }
                mutable.value = mutable.value.copy(message = "已保存到所选位置")
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                reportFailure(appContext, "保存失败，原文件仍可重试", failure)
            } finally {
                mutable.value = mutable.value.copy(busy = null)
            }
        }
    }

    fun cancelPickerWait() {
        if (!mutable.value.pickerPending || mutable.value.busy != null) return
        clearPicker()
        mutable.value = mutable.value.copy(message = "已停止等待，可重新保存或分享", errorDetail = null)
    }

    fun share(context: Context) {
        if (mutable.value.busy != null || mutable.value.pickerPending) return
        try {
            val current = mutable.value
            val file = File(requireNotNull(current.filePath))
            requireExportFile(context, file)
            shareFileWithChooser(context, file, current.mimeType, "分享导出文件")
            // Opening the chooser is not proof that another app accepted the file.
            mutable.value = mutable.value.copy(message = "已打开分享面板", errorDetail = null)
        } catch (failure: Exception) {
            reportFailure(context, "分享失败，文件已保留，可保存到文件", failure)
        }
    }

    private fun requireExportFile(context: Context, file: File) {
        val directory = File(context.cacheDir, "shared_exports").canonicalFile
        require(file.canonicalFile.parentFile == directory) { "导出文件位置无效，请重新生成" }
        require(file.isFile && file.length() > 0L) { "导出文件已被清理，请重新生成" }
        require(file.extension in setOf("txt", "zip")) { "导出文件格式无效" }
    }

    private fun reportFailure(context: Context, prefix: String, failure: Exception) {
        val detail = "${failure.javaClass.simpleName}: ${failure.message.orEmpty().take(240)}"
        mutable.value = mutable.value.copy(message = prefix, errorDetail = detail)
        runCatching { DiagnosticLogStore.record(context.applicationContext, "export.action", prefix, failure) }
    }
}

// Register result launchers outside the lazy card, so scrolling or reopening
// the page cannot lose the document-picker result.
@Composable
internal fun DiagnosticsExportHost(model: DiagnosticsExportViewModel = viewModel()) {
    val context = LocalContext.current
    val state by model.state.collectAsState()
    LaunchedEffect(context.applicationContext) {
        DiagnosticRecordingSessionStore.currentSession(context.applicationContext)
    }
    val saveLog = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("text/plain")) {
        model.saveResult(context, it)
    }
    val saveData = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/zip")) {
        model.saveResult(context, it)
    }
    LaunchedEffect(state.pickerPending, state.pickerLaunched) {
        if (state.pickerPending && !state.pickerLaunched) {
            model.markPickerLaunched()
            try {
                val name = File(requireNotNull(state.filePath)).name
                if (state.mimeType == "application/zip") saveData.launch(name) else saveLog.launch(name)
            } catch (failure: Exception) {
                model.pickerFailed(context, failure)
            }
        }
    }
}

@Composable
internal fun ReliableDiagnosticsPanel(
    appData: AppData,
    onPrepareDataExport: suspend () -> PreparedDataExport,
    model: DiagnosticsExportViewModel = viewModel()
) {
    val context = LocalContext.current
    val state by model.state.collectAsState()
    val collection by DiagnosticRecordingSessionStore.session.collectAsState()
    val enabled = state.busy == null && !state.pickerPending
    val accent = MaterialTheme.colorScheme.secondary
    Surface(shape = RoundedCornerShape(24.dp), tonalElevation = 2.dp,
        modifier = Modifier.fillMaxWidth().testTag("diagnostics_panel")) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("诊断与导出", style = MaterialTheme.typography.titleMedium)
            Text(collectionStatus(collection), style = MaterialTheme.typography.bodyMedium,
                color = if (collection.isActive) accent else MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("diagnostics_collection_status"))
            PhysicalButton(
                label = when {
                    collection.isLoading -> "正在读取收集状态"
                    collection.isActive -> "停止收集"
                    collection.hasCollection -> "重新收集"
                    else -> "开始收集"
                },
                icon = if (collection.isActive) Icons.Rounded.Stop else Icons.Rounded.PlayArrow,
                accent = accent, filled = true, enabled = enabled && !collection.isLoading,
                modifier = Modifier.fillMaxWidth().testTag("diagnostics_collection_toggle"),
                onClick = {
                    if (collection.isActive) model.stopCollection(context)
                    else model.startCollection(context)
                })
            if (collection.isActive) {
                Text("复现问题后，回到这里停止收集并导出日志", style = MaterialTheme.typography.bodySmall)
            }
            if (collection.limitReached) {
                Text("本次收集已达容量上限，已保留的记录可以导出", style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error)
            }
            collection.lastError?.let { detail ->
                Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
            PhysicalButton(label = "导出日志", icon = Icons.Rounded.History, accent = accent,
                filled = false, enabled = enabled,
                modifier = Modifier.fillMaxWidth().testTag("diagnostics_export_log"),
                onClick = { model.exportLog(context, appData) })
            HorizontalDivider()
            Text("数据备份", style = MaterialTheme.typography.titleSmall)
            PhysicalButton(label = "导出数据", icon = Icons.Rounded.Unarchive, accent = accent,
                filled = false, enabled = enabled,
                modifier = Modifier.fillMaxWidth().testTag("diagnostics_export_data"),
                onClick = { model.exportData(context, onPrepareDataExport) })
            state.busy?.let {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                Text(it, style = MaterialTheme.typography.bodySmall)
            }
            if (state.pickerPending) {
                Text("等待选择保存位置", style = MaterialTheme.typography.bodySmall)
                TextButton(onClick = model::cancelPickerWait) { Text("取消等待") }
            }
            state.message?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium,
                    color = if (state.errorDetail == null) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.error)
            }
            state.filePath?.let { path ->
                Regex("^(?:tenfold|grid_timer)_data_missing_([0-9]+)_").find(File(path).name)?.let { missing ->
                    Text("此数据包缺少 ${missing.groupValues[1]} 个附件，缺失清单已随包保存",
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.testTag("diagnostics_backup_incomplete"))
                }
                Text("已生成文件", style = MaterialTheme.typography.labelMedium)
                Text(File(path).name, style = MaterialTheme.typography.bodySmall)
                PhysicalButton(label = "保存到文件", icon = Icons.Rounded.Unarchive, accent = accent,
                    filled = false, enabled = enabled,
                    modifier = Modifier.fillMaxWidth().testTag("diagnostics_save_file"),
                    onClick = { model.requestSave(context) })
                PhysicalButton(label = "分享文件", icon = Icons.Rounded.Share, accent = accent,
                    filled = false, enabled = enabled,
                    modifier = Modifier.fillMaxWidth().testTag("diagnostics_share_file"),
                    onClick = { model.share(context) })
            }
            state.errorDetail?.let { detail ->
                Text(detail, style = MaterialTheme.typography.bodySmall)
                TextButton(onClick = { runCatching { copyTextToClipboard(context, "导出错误", detail) } }) {
                    Text("复制错误信息")
                }
            }
        }
    }
}

private val collectionTimeFormat = DateTimeFormatter.ofPattern("MM-dd HH:mm:ss").withZone(ZoneId.systemDefault())

private fun collectionStatus(session: DiagnosticRecordingSession): String {
    if (session.isLoading) return "正在读取收集状态"
    if (!session.hasCollection) return "未开始收集"
    val started = session.startedAtEpochMillis?.let { collectionTimeFormat.format(Instant.ofEpochMilli(it)) }.orEmpty()
    val prefix = if (session.isActive) "正在收集" else "已停止收集"
    val ended = session.stoppedAtEpochMillis?.let { " · 停止于 ${collectionTimeFormat.format(Instant.ofEpochMilli(it))}" }.orEmpty()
    return "$prefix · ${session.eventCount} 条记录\n开始于 $started$ended"
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_and_export_are_separate_and_survive_lazy_card_lifetime() {
        let source = crate::kotlin_sources::SOURCES
            .iter()
            .find(|s| s.path.ends_with("/GridTimerScreen.kt"))
            .unwrap();
        let rendered = render(source.path, source.contents).unwrap();
        let panel = rendered
            .split("private fun DiagnosticsActionPanel(")
            .nth(1)
            .unwrap()
            .split("private fun XiaomiIslandDiagnosticsPanel(")
            .next()
            .unwrap();
        assert!(panel.contains("ReliableDiagnosticsPanel"));
        assert!(!panel.contains("DiagnosticRecordingSessionStore.start"));
        assert!(rendered.contains("DiagnosticsExportHost()\n    TimerNotificationEffects"));
        assert!(CONTENTS.contains("viewModelScope.launch"));
        assert!(!CONTENTS.contains("rememberCoroutineScope"));
        assert!(CONTENTS.contains("DiagnosticsExporter.recordDiagnosticLog(appContext, data)"));
        assert!(CONTENTS.contains("DiagnosticRecordingSessionStore.session.collectAsState()"));
        assert!(CONTENTS.contains("if (start) DiagnosticRecordingSessionStore.start(appContext)"));
        assert!(CONTENTS.contains("else DiagnosticRecordingSessionStore.finish(appContext)"));
        assert!(CONTENTS.contains("collection.isActive -> \"停止收集\""));
        assert!(CONTENTS.contains("else -> \"开始收集\""));
        assert!(CONTENTS.contains("if (!session.hasCollection) return \"未开始收集\""));
        let export_log = CONTENTS
            .split("    fun exportLog(")
            .nth(1)
            .unwrap()
            .split("    fun exportData(")
            .next()
            .unwrap();
        assert!(!export_log.contains("DiagnosticRecordingSessionStore.start"));
        assert!(!export_log.contains("DiagnosticRecordingSessionStore.finish"));
    }

    #[test]
    fn failed_delivery_keeps_file_and_copy_checks_completion() {
        assert!(CONTENTS.contains("copied == expectedSize && source.length() == expectedSize"));
        assert!(CONTENTS.contains("openOutputStream(uri, \"wt\")"));
        assert!(CONTENTS.contains("file.canonicalFile.parentFile == directory"));
        assert!(CONTENTS.contains("saved[\"diagnostic_export_path\"] = file.absolutePath"));
        assert!(CONTENTS.contains(
            "finally {\n                mutable.value = mutable.value.copy(busy = null)"
        ));
        assert!(!CONTENTS.contains("filePath = null"));
        let share = CONTENTS
            .split("    fun share(")
            .nth(1)
            .unwrap()
            .split("    private fun requireExportFile(")
            .next()
            .unwrap();
        assert!(!share.contains("DiagnosticRecordingSessionStore.finish"));
        assert!(CONTENTS.contains("if (!mutable.value.pickerPending) return"));
        assert!(CONTENTS.contains("TextButton(onClick = model::cancelPickerWait)"));
        assert!(CONTENTS.contains("saved[\"diagnostic_picker_path\"] = mutable.value.filePath"));
    }

    #[test]
    fn stale_panel_template_fails_closed() {
        assert!(render("com/ofairyo/gridtimer/ui/GridTimerScreen.kt", "changed").is_err());
        assert_eq!(render("other.kt", "unchanged").unwrap(), "unchanged");
    }
}
