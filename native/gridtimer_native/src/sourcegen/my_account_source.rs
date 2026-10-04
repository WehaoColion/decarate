// v2.23.2.5 - Explain direct AI questions and knowledge sources without coupling account sync.
// v2.23.2.3 - Show saved connection state and direct AI feature entry points.
// v2.23.2.2 - Add DeepSeek setup and guarded synthetic connection tests.
// v2.23.1.3 - Open older update notes separately and show minute-level update times.
// v2.23.1.2 - Copy update summaries, paragraphs, and full version notes on tap.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/MyAccountScreen.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.content.Intent
import android.net.Uri
import android.widget.Toast
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.ofairyo.gridtimer.BuildConfig
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import com.ofairyo.gridtimer.data.AppData
import com.ofairyo.gridtimer.data.SyncAccountSession
import com.ofairyo.gridtimer.data.ThemeMode
import com.ofairyo.gridtimer.i18n.localizedUiText
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun MyAccountScreen(
    syncSession: SyncAccountSession,
    appData: AppData,
    workspaceKey: String,
    onServerUrlChange: (String) -> Unit,
    onEmailChange: (String) -> Unit,
    onDeviceNameChange: (String) -> Unit,
    onAiConfigurationSave: suspend (String, String, String) -> Boolean,
    onOpenKnowledgeAi: () -> Unit,
    onOpenLegalRisk: () -> Unit,
    onRegister: (String, String) -> Unit,
    onLogin: (String, String) -> Unit,
    onSyncNow: () -> Unit,
    onUploadLocalData: () -> Unit,
    onDownloadAccountData: () -> Unit,
    onLogout: () -> Unit,
    onThemeModeSelected: (ThemeMode) -> Unit,
    bottomContentPadding: Dp,
    initialAiExpanded: Boolean = false,
    modifier: Modifier = Modifier
) {
    var serverUrl by rememberSaveable { mutableStateOf(syncSession.serverUrl) }
    var email by rememberSaveable { mutableStateOf(syncSession.email) }
    var deviceName by rememberSaveable { mutableStateOf(syncSession.deviceName) }
    var password by rememberSaveable { mutableStateOf("") }
    var accountSettingsExpanded by rememberSaveable { mutableStateOf(false) }
    var languagePickerExpanded by rememberSaveable { mutableStateOf(false) }
    val systemLocale = rememberSystemLocale()
    val selectedLanguage = AppLanguage.currentSelection()
    val settingsListState = rememberLazyListState()
    LaunchedEffect(initialAiExpanded) {
        if (initialAiExpanded) settingsListState.scrollToItem(2)
    }

    LaunchedEffect(
        syncSession.serverUrl,
        syncSession.email,
        syncSession.deviceName
    ) {
        serverUrl = syncSession.serverUrl
        email = syncSession.email
        deviceName = syncSession.deviceName
    }
    LaunchedEffect(syncSession.loggedIn, syncSession.token) {
        if (syncSession.loggedIn) {
            password = ""
            accountSettingsExpanded = false
        }
    }

    LazyColumn(
        state = settingsListState,
        modifier = modifier
            .background(MaterialTheme.colorScheme.background)
            .windowInsetsPadding(WindowInsets.statusBars.only(WindowInsetsSides.Top)),
        contentPadding = PaddingValues(
            start = 20.dp,
            top = 18.dp,
            end = 20.dp,
            bottom = bottomContentPadding + 28.dp
        ),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 4.dp, vertical = 2.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                Text(
                    text = myPageText(MyPageText.TITLE),
                    style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.SemiBold)
                )
                Text(
                text = if (syncSession.loggedIn) syncSession.email else myPageText(MyPageText.ACCOUNT_FALLBACK),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodyMedium.copy(
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                )
            }
        }

        item {
            CompactSettingsCard {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = myPageText(MyPageText.ACCOUNT),
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
                    )
                    AccountStatusPill(
                        label = if (syncSession.loggedIn) myPageText(MyPageText.LOGGED_IN) else myPageText(MyPageText.LOGGED_OUT),
                        active = syncSession.loggedIn
                    )
                }

                if (syncSession.loggedIn) {
                    Text(
                        text = localizedUiText(
                            if (syncSession.lastMessage.startsWith("Authenticated response omitted its token owner."))
                                "上次账户同步未完成，请点立即同步重试。AI 可使用本机保存的配置。"
                            else syncSession.lastMessage.ifBlank { myPageText(MyPageText.WAITING_SYNC) }
                        ),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodyMedium.copy(
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    )
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        CompactActionButton(
                            label = if (syncSession.syncing) myPageText(MyPageText.SYNCING) else myPageText(MyPageText.SYNC_NOW),
                            accent = MaterialTheme.colorScheme.primary,
                            enabled = !syncSession.syncing,
                            onClick = onSyncNow
                        )
                        CompactActionButton(
                            label = if (accountSettingsExpanded) myPageText(MyPageText.HIDE_SETTINGS) else myPageText(MyPageText.ACCOUNT_SETTINGS),
                            accent = MaterialTheme.colorScheme.onSurfaceVariant,
                            enabled = !syncSession.syncing,
                            onClick = { accountSettingsExpanded = !accountSettingsExpanded }
                        )
                    }

                    AnimatedVisibility(visible = accountSettingsExpanded) {
                        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                            OutlinedTextField(
                                value = deviceName,
                                onValueChange = {
                                    deviceName = it
                                    onDeviceNameChange(it)
                                },
                                label = { Text(myPageText(MyPageText.DEVICE_NAME)) },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next)
                            )
                            OutlinedTextField(
                                value = serverUrl,
                                onValueChange = {
                                    serverUrl = it
                                    onServerUrlChange(it)
                                },
                                label = { Text(myPageText(MyPageText.SYNC_ADDRESS)) },
                                placeholder = { Text(myPageText(MyPageText.FIND_COMPUTER)) },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                                keyboardOptions = KeyboardOptions(
                                    keyboardType = KeyboardType.Uri,
                                    imeAction = ImeAction.Done
                                )
                            )
                            Text(
                                text = myPageText(MyPageText.MANUAL_TRANSFER),
                                style = MaterialTheme.typography.labelMedium.copy(
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    fontWeight = FontWeight.SemiBold
                                )
                            )
                            FlowRow(
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                                verticalArrangement = Arrangement.spacedBy(10.dp)
                            ) {
                                CompactActionButton(
                                    label = if (syncSession.syncing) myPageText(MyPageText.PROCESSING) else myPageText(MyPageText.ACCOUNT_TO_DEVICE),
                                    accent = MaterialTheme.colorScheme.secondary,
                                    enabled = !syncSession.syncing,
                                    onClick = onDownloadAccountData
                                )
                                CompactActionButton(
                                    label = if (syncSession.syncing) myPageText(MyPageText.PROCESSING) else myPageText(MyPageText.DEVICE_TO_ACCOUNT),
                                    accent = MaterialTheme.colorScheme.tertiary,
                                    enabled = !syncSession.syncing,
                                    onClick = onUploadLocalData
                                )
                                CompactActionButton(
                                    label = myPageText(MyPageText.SIGN_OUT),
                                    accent = MaterialTheme.colorScheme.onSurfaceVariant,
                                    enabled = !syncSession.syncing,
                                    onClick = onLogout
                                )
                            }
                        }
                    }
                } else {
                    OutlinedTextField(
                        value = email,
                        onValueChange = {
                            email = it
                            onEmailChange(it)
                        },
                        label = { Text(myPageText(MyPageText.EMAIL)) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = KeyboardOptions(
                            keyboardType = KeyboardType.Email,
                            imeAction = ImeAction.Next
                        )
                    )
                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        label = { Text(myPageText(MyPageText.PASSWORD)) },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = KeyboardOptions(
                            keyboardType = KeyboardType.Password,
                            imeAction = ImeAction.Done
                        )
                    )
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        CompactActionButton(
                            label = if (syncSession.syncing) myPageText(MyPageText.PROCESSING) else myPageText(MyPageText.SIGN_IN),
                            accent = MaterialTheme.colorScheme.primary,
                            enabled = !syncSession.syncing,
                            onClick = { onLogin(email, password) }
                        )
                        CompactActionButton(
                            label = if (syncSession.syncing) myPageText(MyPageText.PROCESSING) else myPageText(MyPageText.CREATE_ACCOUNT),
                            accent = MaterialTheme.colorScheme.secondary,
                            enabled = !syncSession.syncing,
                            onClick = { onRegister(email, password) }
                        )
                        CompactActionButton(
                            label = if (accountSettingsExpanded) myPageText(MyPageText.HIDE_SETTINGS) else myPageText(MyPageText.CONNECTION_SETTINGS),
                            accent = MaterialTheme.colorScheme.onSurfaceVariant,
                            enabled = !syncSession.syncing,
                            onClick = { accountSettingsExpanded = !accountSettingsExpanded }
                        )
                    }

                    AnimatedVisibility(visible = accountSettingsExpanded) {
                        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                            OutlinedTextField(
                                value = deviceName,
                                onValueChange = {
                                    deviceName = it
                                    onDeviceNameChange(it)
                                },
                                label = { Text(myPageText(MyPageText.DEVICE_NAME)) },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next)
                            )
                            OutlinedTextField(
                                value = serverUrl,
                                onValueChange = {
                                    serverUrl = it
                                    onServerUrlChange(it)
                                },
                                label = { Text(myPageText(MyPageText.SYNC_ADDRESS)) },
                                placeholder = { Text(myPageText(MyPageText.FIND_COMPUTER)) },
                                singleLine = true,
                                modifier = Modifier.fillMaxWidth(),
                                keyboardOptions = KeyboardOptions(
                                    keyboardType = KeyboardType.Uri,
                                    imeAction = ImeAction.Done
                                )
                            )
                        }
                    }
                }
            }
        }

        item(key = "ai_features") {
            AndroidAiSettingsCard(
                syncSession = syncSession,
                workspaceKey = workspaceKey,
                onSave = onAiConfigurationSave,
                onOpenKnowledgeAi = onOpenKnowledgeAi,
                onOpenLegalRisk = onOpenLegalRisk,
                initialAiExpanded = initialAiExpanded
            )
        }

        item {
            CompactSettingsCard {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(14.dp))
                        .semantics { role = Role.Button }
                        .clickable { languagePickerExpanded = true }
                        .padding(vertical = 2.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("🌐", style = MaterialTheme.typography.titleLarge)
                    Column(
                        modifier = Modifier.weight(1f),
                        verticalArrangement = Arrangement.spacedBy(2.dp)
                    ) {
                        Text(
                            text = myPageText(MyPageText.LANGUAGE),
                            style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
                        )
                        Text(
                            text = "${selectedLanguage.displayFlag(systemLocale)} " +
                                if (selectedLanguage == AppLanguage.SYSTEM) myPageText(MyPageText.SYSTEM_LANGUAGE)
                                else selectedLanguage.nativeName,
                            style = MaterialTheme.typography.bodySmall.copy(
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        )
                    }
                    Text(
                        text = myPageText(MyPageText.CHANGE),
                        style = MaterialTheme.typography.labelLarge.copy(
                            color = MaterialTheme.colorScheme.primary,
                            fontWeight = FontWeight.SemiBold
                        )
                    )
                }
            }

            if (languagePickerExpanded) {
                AlertDialog(
                    onDismissRequest = { languagePickerExpanded = false },
                    title = { Text(myPageText(MyPageText.SELECT_LANGUAGE)) },
                    text = {
                        LazyColumn(modifier = Modifier.heightIn(max = 440.dp)) {
                            items(AppLanguage.entries.toList()) { language ->
                                val isSelected = language == selectedLanguage
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clip(RoundedCornerShape(14.dp))
                                        .semantics {
                                            role = Role.RadioButton
                                            selected = isSelected
                                        }
                                        .clickable {
                                            languagePickerExpanded = false
                                            if (!isSelected) language.apply()
                                        }
                                        .padding(horizontal = 10.dp, vertical = 12.dp),
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    Text(
                                        text = language.displayFlag(systemLocale),
                                        style = MaterialTheme.typography.titleMedium
                                    )
                                    Text(
                                        text = if (language == AppLanguage.SYSTEM) {
                                            myPageText(MyPageText.SYSTEM_LANGUAGE)
                                        } else {
                                            language.nativeName
                                        },
                                        modifier = Modifier.weight(1f),
                                        style = MaterialTheme.typography.bodyLarge.copy(
                                            fontWeight = if (isSelected) FontWeight.SemiBold else FontWeight.Normal
                                        )
                                    )
                                    if (isSelected) {
                                        Text(
                                            text = myPageText(MyPageText.CURRENT),
                                            style = MaterialTheme.typography.labelMedium.copy(
                                                color = MaterialTheme.colorScheme.primary
                                            )
                                        )
                                    }
                                }
                            }
                        }
                    },
                    confirmButton = {
                        TextButton(onClick = { languagePickerExpanded = false }) {
                            Text(myPageText(MyPageText.CANCEL))
                        }
                    }
                )
            }
        }

        item {
            CompactSettingsCard {
                Text(
                    text = myPageText(MyPageText.APPEARANCE),
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
                )
                FlowRow(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    listOf(
                        ThemeMode.SYSTEM to myPageText(MyPageText.SYSTEM_THEME),
                        ThemeMode.LIGHT to myPageText(MyPageText.LIGHT_THEME),
                        ThemeMode.DARK to myPageText(MyPageText.DARK_THEME),
                        ThemeMode.OLED to myPageText(MyPageText.OLED_THEME)
                    ).forEach { (mode, label) ->
                        CompactThemeChoice(
                            label = label,
                            selected = appData.resolvedThemeMode == mode,
                            onClick = { onThemeModeSelected(mode) }
                        )
                    }
                }
            }
        }

        item {
            AndroidUpdateHistorySection()
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AndroidAiSettingsCard(
    syncSession: SyncAccountSession,
    workspaceKey: String,
    onSave: suspend (String, String, String) -> Boolean,
    onOpenKnowledgeAi: () -> Unit,
    onOpenLegalRisk: () -> Unit,
    initialAiExpanded: Boolean
) {
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()
    // Credentials live only in memory here; the repository owns encrypted persistence.
    val controller = remember(
        workspaceKey, syncSession.serverInstanceId, syncSession.accountNamespace,
        syncSession.userId, syncSession.token
    ) {
        AiConnectionController(AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel).setupDefaults())
    }
    var revision by remember(controller) { mutableStateOf(0) }
    var aiSettingsExpanded by rememberSaveable { mutableStateOf(initialAiExpanded || syncSession.aiApiKey.isBlank()) }
    var endpointExpanded by rememberSaveable { mutableStateOf(false) }
    var saving by remember(controller) { mutableStateOf(false) }
    var notice by remember(controller) { mutableStateOf("") }
    var savedConfiguration by remember(controller) {
        mutableStateOf(AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel).normalized())
    }
    val configuration = remember(controller, revision) { controller.configuration }
    val busy = remember(controller, revision) { controller.busy }
    val outcome = remember(controller, revision) { controller.outcome }
    val dirty = configuration != savedConfiguration
    val launchAction = configuration.featureAction(savedConfiguration)
    LaunchedEffect(initialAiExpanded) {
        if (initialAiExpanded) aiSettingsExpanded = true
    }
    DisposableEffect(controller) {
        onDispose { controller.close() }
    }
    DisposableEffect(controller, lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP && controller.busy) {
                controller.cancel()
                notice = "本次连接检测已取消，可以稍后重新检测"
                revision++
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(controller, syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel) {
        val stored = AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel).normalized()
        savedConfiguration = stored
        controller.update(stored.setupDefaults())
        revision++
    }
    fun edit(next: AiConfiguration) {
        controller.update(next)
        notice = ""
        revision++
    }
    fun cancelTest() {
        if (controller.busy) {
            controller.cancel()
            notice = "本次结果已取消；已发出的测试可能仍会计费"
            revision++
        }
    }
    fun openPlatform() {
        cancelTest()
        runCatching {
            context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://platform.deepseek.com")))
        }.onFailure { Toast.makeText(context, "未找到浏览器", Toast.LENGTH_SHORT).show() }
    }
    fun openFeature(open: () -> Unit) {
        if (saving || controller.busy || controller.closed) return
        when (controller.configuration.featureAction(savedConfiguration)) {
            AiFeatureAction.CONFIGURE -> {
                aiSettingsExpanded = true
                notice = if (controller.configuration.apiKey.isBlank()) "先填写并保存 API Key，再进入 AI 功能"
                else "请补全 HTTPS 接口根地址和模型名称"
            }
            AiFeatureAction.OPEN -> open()
            AiFeatureAction.SAVE_AND_OPEN -> {
                val draft = controller.configuration.normalized()
                saving = true
                scope.launch {
                    try {
                        val saved = onSave(draft.apiKey, draft.baseUrl, draft.model)
                        if (!controller.closed && controller.configuration == draft) {
                            if (saved) {
                                savedConfiguration = draft
                                open()
                            } else {
                                aiSettingsExpanded = true
                                notice = "配置未保存成功，请检查当前账号后重试"
                            }
                        }
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (_: Exception) {
                        if (!controller.closed) notice = "配置保存失败，请重试；尚未打开 AI 功能"
                    } finally { saving = false }
                }
            }
        }
    }
    val summary = when {
        saving -> "正在保存配置"
        busy -> "正在检测连接"
        configuration.apiKey.isBlank() -> "待填写 API Key"
        !configuration.endpointValid -> "接口信息待补全"
        dirty -> "配置有修改，尚未保存"
        outcome?.fullyAvailable == true -> "已连接 · 文字和图片可用"
        outcome?.ok == true -> "已连接 · 部分能力可用"
        outcome != null -> "配置已保存 · 连接未通过"
        else -> "配置已保存 · 可以进入 AI 功能"
    }
    val entryAction = when (launchAction) {
        AiFeatureAction.CONFIGURE -> "先配置"
        AiFeatureAction.SAVE_AND_OPEN -> "保存并进入"
        AiFeatureAction.OPEN -> "进入"
    }
    Surface(
        modifier = Modifier.fillMaxWidth().animateContentSize(),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Text("AI", style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.SemiBold))
                    Text(summary, style = MaterialTheme.typography.bodySmall,
                        color = if (outcome != null && !outcome.ok) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
                }
                TextButton(onClick = {
                    aiSettingsExpanded = !aiSettingsExpanded
                    if (!aiSettingsExpanded) cancelTest()
                }) {
                    Text(if (aiSettingsExpanded) "收起设置" else "连接设置")
                }
            }
            if (configuration.apiKey.isNotBlank() && !dirty && outcome == null) {
                Text("连接检测可选。AI 请求直接发往配置的接口，独立于账户同步。", style = MaterialTheme.typography.bodySmall)
            }
            AiFeatureEntry(
                title = "问 AI",
                detail = "直接提问，或依据知识页回答",
                action = entryAction,
                enabled = !saving && !busy,
                onClick = { openFeature(onOpenKnowledgeAi) }
            )
            AiFeatureEntry(
                title = "法律风险线索",
                detail = "先查看扫描范围和发送预览，再由你确认发送",
                action = entryAction,
                enabled = !saving && !busy,
                onClick = { openFeature(onOpenLegalRisk) }
            )
            if (outcome != null) {
                Text(
                    if (outcome.fullyAvailable) "本次检测通过：结构化输出、图片输入可用。现在可以进入上方功能。"
                    else outcome.message,
                    color = if (outcome.ok) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall
                )
                if (!outcome.fullyAvailable) {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { aiSettingsExpanded = true }) { Text("检查密钥与接口") }
                        if (configuration.isDeepSeek) TextButton(onClick = ::openPlatform) { Text("查看 DeepSeek 额度与权限") }
                    }
                }
            }
            if (notice.isNotBlank()) Text(notice, style = MaterialTheme.typography.bodySmall)
            AnimatedVisibility(visible = aiSettingsExpanded) {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(myPageText(MyPageText.AI_SETTINGS), style = MaterialTheme.typography.titleSmall)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(enabled = !saving, onClick = {
                            val previous = configuration
                            edit(configuration.deepSeekPreset())
                            endpointExpanded = false
                            notice = if (previous.apiKey.isNotBlank() && !previous.isDeepSeek) {
                                "已切换到 DeepSeek，请填写 DeepSeek 专用密钥"
                            } else "DeepSeek 地址和模型已填好，保存后可使用上方功能"
                        }) { Text(if (configuration.isDeepSeek) "✓ DeepSeek 官方" else "DeepSeek 官方") }
                        TextButton(enabled = !saving, onClick = {
                            if (configuration.isDeepSeek) edit(AiConfiguration("", "", ""))
                            endpointExpanded = true
                            notice = "填写服务商的接口根地址、模型和专用密钥"
                        }) { Text("自定义服务") }
                    }
                    OutlinedTextField(
                        value = configuration.apiKey,
                        onValueChange = { edit(configuration.copy(apiKey = it)) },
                        enabled = !saving,
                        label = { Text(if (configuration.isDeepSeek) "DeepSeek API Key" else "API Key") },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
                        modifier = Modifier.fillMaxWidth()
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(enabled = !saving, onClick = {
                            val pasted = clipboard.getText()?.text?.trim().orEmpty()
                            if (pasted.isNotBlank()) edit(configuration.copy(apiKey = pasted))
                            else notice = "剪贴板没有可粘贴的密钥"
                        }) { Text("粘贴密钥") }
                        if (configuration.isDeepSeek) TextButton(onClick = ::openPlatform) { Text("登录 / 充值 / 创建密钥") }
                    }
                    Text("接收：${configuration.recipientHost.ifBlank { "待填写" }} · 模型：${configuration.model.ifBlank { "待填写" }}",
                        style = MaterialTheme.typography.bodySmall)
                    TextButton(onClick = { endpointExpanded = !endpointExpanded }) {
                        Text(if (endpointExpanded) "收起接口与模型" else "修改接口与模型")
                    }
                    if (endpointExpanded || !configuration.endpointValid) {
                        OutlinedTextField(
                            value = configuration.baseUrl,
                            onValueChange = {
                                val next = configuration.withEndpoint(it)
                                edit(next)
                                if (configuration.apiKey.isNotBlank() && next.apiKey.isBlank()) {
                                    notice = "接收地址已改变，请填写该服务的专用密钥"
                                }
                            },
                            enabled = !saving,
                            label = { Text("接口根地址") },
                            supportingText = { Text("DeepSeek 填 https://api.deepseek.com，不加请求路径") },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
                            modifier = Modifier.fillMaxWidth()
                        )
                        OutlinedTextField(
                            value = configuration.model,
                            onValueChange = { edit(configuration.copy(model = it)) },
                            enabled = !saving,
                            label = { Text("模型") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth()
                        )
                    }
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(enabled = !saving && !busy && configuration.endpointValid, onClick = {
                            val draft = configuration.normalized()
                            saving = true
                            scope.launch {
                                try {
                                    val saved = onSave(draft.apiKey, draft.baseUrl, draft.model)
                                    if (!controller.closed && controller.configuration == draft) {
                                        if (saved) {
                                            savedConfiguration = draft
                                            if (draft.apiKey.isNotBlank()) aiSettingsExpanded = false
                                        }
                                        notice = if (saved && draft.apiKey.isNotBlank()) "配置已保存，点上方“问 AI”或“法律风险线索”开始使用"
                                        else if (saved) "设置已保存，请填写 API Key 后使用"
                                        else "保存失败或账号已切换，请重试"
                                    }
                                } catch (cancelled: CancellationException) {
                                    throw cancelled
                                } catch (_: Exception) {
                                    if (!controller.closed) notice = "保存失败，请重试"
                                } finally { saving = false }
                            }
                        }) { Text(if (saving) "正在保存" else "保存配置") }
                        TextButton(enabled = !saving && !busy && configuration.testable, onClick = {
                            val ticket = controller.begin() ?: return@TextButton
                            saving = true
                            notice = ""
                            revision++
                            scope.launch {
                                try {
                                    val draft = ticket.configuration
                                    val saved = onSave(draft.apiKey, draft.baseUrl, draft.model)
                                    saving = false
                                    if (saved && !controller.closed && controller.configuration == draft) savedConfiguration = draft
                                    val result = if (!saved || !controller.isCurrent(ticket)) {
                                        AiConnectionOutcome(false, "配置未保存或已改变，未发送测试", false, false)
                                    } else {
                                        withContext(Dispatchers.IO) {
                                            if (!controller.isCurrent(ticket)) return@withContext AiConnectionOutcome(false, "测试已取消", false, false)
                                            val raw = NativeOptimizerBridge.testAiConnection(draft.apiKey, draft.baseUrl, draft.model)
                                            runCatching {
                                                val parsed = JSONObject(raw ?: "{}")
                                                AiConnectionOutcome(parsed.optBoolean("ok"), parsed.optString("message", "连接测试不可用"),
                                                    parsed.optBoolean("structured"), parsed.optBoolean("vision"))
                                            }.getOrElse { AiConnectionOutcome(false, "检测响应无法读取，请重试或更换接口", false, false) }
                                        }
                                    }
                                    if (controller.finish(ticket, result)) {
                                        notice = ""
                                        if (result.fullyAvailable) aiSettingsExpanded = false
                                    }
                                } catch (cancelled: CancellationException) {
                                    controller.cancel()
                                    throw cancelled
                                } catch (_: Exception) {
                                    controller.finish(ticket, AiConnectionOutcome(false, "连接检测失败，请检查网络与接口后重试", false, false))
                                } finally {
                                    saving = false
                                    controller.release(ticket)
                                    revision++
                                }
                            }
                        }) { Text(if (busy) "正在检测" else if (dirty) "保存并检测连接" else "检测连接（可选）") }
                        if (busy) TextButton(onClick = ::cancelTest) { Text("取消检测") }
                    }
                    Text("连接检测最多发送 1 次固定测试文字和合成图片，会消耗少量额度，不传个人资料。",
                        style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

@Composable
private fun AiFeatureEntry(title: String, detail: String, action: String, enabled: Boolean, onClick: () -> Unit) {
    Surface(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp))
            .semantics { role = Role.Button }.clickable(enabled = enabled, onClick = onClick),
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = if (enabled) 0.62f else 0.24f)
    ) {
        Row(Modifier.padding(14.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(title, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold))
                Text(detail, style = MaterialTheme.typography.bodySmall)
            }
            Text(action, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
        }
    }
}

@Composable
internal fun AndroidUpdateHistorySection() {
    var historyExpanded by rememberSaveable { mutableStateOf(false) }
    var previousVersionsOpen by rememberSaveable { mutableStateOf(false) }
    val currentEntry = androidUpdateHistory.firstOrNull { it.version == BuildConfig.VERSION_NAME }
    val previousEntries = androidUpdateHistory.filter { it.version != BuildConfig.VERSION_NAME }

    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .animateContentSize(),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        contentColor = MaterialTheme.colorScheme.onSurface,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(20.dp))
                    .semantics {
                        role = Role.Button
                        stateDescription = if (historyExpanded) "已展开" else "已收起"
                    }
                    .clickable { historyExpanded = !historyExpanded }
                    .padding(16.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(3.dp)
                ) {
                    Text(
                        text = "更新记录",
                        style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
                    )
                    Text(
                        text = "当前版本 ${BuildConfig.VERSION_NAME}",
                        style = MaterialTheme.typography.bodySmall.copy(
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    )
                }
                Text(
                    text = if (historyExpanded) "收起" else "查看",
                    style = MaterialTheme.typography.labelLarge.copy(
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.SemiBold
                    )
                )
            }

            AnimatedVisibility(visible = historyExpanded) {
                Column(
                    modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    currentEntry?.let { entry ->
                        AndroidUpdateHistoryEntry(entry, initiallyExpanded = true)
                    }
                    if (previousEntries.isNotEmpty()) {
                        Surface(
                            shape = RoundedCornerShape(16.dp),
                            color = MaterialTheme.colorScheme.surfaceContainer,
                            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
                        ) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(16.dp))
                                    .clickable(role = Role.Button, onClickLabel = "查看历史版本") {
                                        previousVersionsOpen = true
                                    }
                                    .padding(horizontal = 14.dp, vertical = 13.dp),
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Column(
                                    modifier = Modifier.weight(1f),
                                    verticalArrangement = Arrangement.spacedBy(3.dp)
                                ) {
                                    Text(
                                        text = "历史版本",
                                        style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold)
                                    )
                                    Text(
                                        text = "${previousEntries.size} 个版本",
                                        style = MaterialTheme.typography.bodySmall.copy(
                                            color = MaterialTheme.colorScheme.onSurfaceVariant
                                        )
                                    )
                                }
                                Text(
                                    text = "查看",
                                    style = MaterialTheme.typography.labelMedium.copy(
                                        color = MaterialTheme.colorScheme.primary,
                                        fontWeight = FontWeight.SemiBold
                                    )
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    if (previousVersionsOpen && previousEntries.isNotEmpty()) {
        AlertDialog(
            onDismissRequest = { previousVersionsOpen = false },
            title = { Text("历史版本") },
            text = {
                LazyColumn(
                    modifier = Modifier.fillMaxWidth().heightIn(max = 480.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    items(previousEntries, key = { it.version }) { entry ->
                        AndroidUpdateHistoryEntry(entry, initiallyExpanded = false)
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { previousVersionsOpen = false }) {
                    Text("关闭")
                }
            }
        )
    }
}

@Composable
private fun AndroidUpdateHistoryEntry(
    entry: AndroidUpdateEntry,
    initiallyExpanded: Boolean
) {
    var expanded by rememberSaveable(entry.version) { mutableStateOf(initiallyExpanded) }
    val context = LocalContext.current
    val copyUpdateText: (String, String) -> Unit = { text, successMessage ->
        runCatching { copyTextToClipboard(context, "更新说明 ${entry.version}", text) }
            .onSuccess { Toast.makeText(context, successMessage, Toast.LENGTH_SHORT).show() }
            .onFailure { Toast.makeText(context, "复制失败，请重试", Toast.LENGTH_SHORT).show() }
    }

    Surface(
        modifier = Modifier.fillMaxWidth().animateContentSize(),
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainer,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .semantics {
                        role = Role.Button
                        stateDescription = if (expanded) "已展开" else "已收起"
                    }
                    .clickable { expanded = !expanded }
                    .padding(horizontal = 14.dp, vertical = 13.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    Text(
                        text = "版本 ${entry.version}",
                        style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold)
                    )
                    if (entry.updatedAt.isNotBlank()) {
                        Text(
                            text = "更新时间 ${entry.updatedAt}",
                            style = MaterialTheme.typography.labelSmall.copy(
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        )
                    }
                    Text(
                        text = entry.summary,
                        modifier = Modifier.clickable(role = Role.Button, onClickLabel = "复制摘要") {
                            copyUpdateText(entry.summary, "已复制摘要")
                        },
                        maxLines = if (expanded) Int.MAX_VALUE else 2,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall.copy(
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    )
                }
                Text(
                    text = if (expanded) "收起" else "展开",
                    style = MaterialTheme.typography.labelMedium.copy(
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.SemiBold
                    )
                )
            }

            AnimatedVisibility(visible = expanded) {
                Column(
                    modifier = Modifier.padding(start = 14.dp, end = 14.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = "点按文字可复制",
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodySmall.copy(
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        )
                        TextButton(onClick = {
                            copyUpdateText(entry.toClipboardText(), "已复制本版更新说明")
                        }) {
                            Text("复制本版")
                        }
                    }
                    entry.sections.forEach { section ->
                        Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
                            Text(
                                text = section.heading,
                                modifier = Modifier.clickable(role = Role.Button, onClickLabel = "复制分组标题") {
                                    copyUpdateText(section.heading, "已复制标题")
                                },
                                style = MaterialTheme.typography.labelLarge.copy(
                                    color = MaterialTheme.colorScheme.primary,
                                    fontWeight = FontWeight.SemiBold
                                )
                            )
                            section.details.forEach { detail ->
                                Text(
                                    text = detail,
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clip(RoundedCornerShape(6.dp))
                                        .clickable(role = Role.Button, onClickLabel = "复制这一段") {
                                            copyUpdateText(detail, "已复制这一段")
                                        }
                                        .padding(horizontal = 2.dp, vertical = 4.dp),
                                    style = MaterialTheme.typography.bodyMedium.copy(
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun CompactSettingsCard(content: @Composable () -> Unit) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .animateContentSize(),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        contentColor = MaterialTheme.colorScheme.onSurface,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            content()
        }
    }
}

@Composable
private fun AccountStatusPill(label: String, active: Boolean) {
    val accent = if (active) accentFor("green") else MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        shape = RoundedCornerShape(999.dp),
        color = accent.copy(alpha = 0.12f),
        border = BorderStroke(1.dp, accent.copy(alpha = 0.22f))
    ) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelMedium.copy(
                color = accent,
                fontWeight = FontWeight.SemiBold
            )
        )
    }
}

@Composable
private fun CompactThemeChoice(
    label: String,
    selected: Boolean,
    onClick: () -> Unit
) {
    val accent = if (selected) {
        MaterialTheme.colorScheme.primary
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }
    Surface(
        modifier = Modifier
            .clip(RoundedCornerShape(15.dp))
            .semantics {
                this.selected = selected
                role = Role.RadioButton
            }
            .clickable(onClick = onClick),
        shape = RoundedCornerShape(15.dp),
        color = if (selected) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            MaterialTheme.colorScheme.surfaceContainer
        },
        border = BorderStroke(1.dp, accent.copy(alpha = if (selected) 0.72f else 0.24f))
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 9.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Surface(
                modifier = Modifier.size(14.dp),
                shape = CircleShape,
                color = if (selected) MaterialTheme.colorScheme.primary else androidx.compose.ui.graphics.Color.Transparent,
                border = BorderStroke(1.dp, accent)
            ) {}
            Text(
                text = label,
                style = MaterialTheme.typography.labelLarge.copy(
                    color = if (selected) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,
                    fontWeight = FontWeight.SemiBold
                )
            )
        }
    }
}

@Composable
private fun CompactActionButton(
    label: String,
    accent: androidx.compose.ui.graphics.Color,
    enabled: Boolean,
    onClick: () -> Unit
) {
    Surface(
        modifier = Modifier
            .clip(RoundedCornerShape(16.dp))
            .semantics { role = Role.Button }
            .clickable(enabled = enabled, onClick = onClick),
        shape = RoundedCornerShape(16.dp),
        color = accent.copy(alpha = if (enabled) 0.14f else 0.06f),
        border = BorderStroke(1.dp, accent.copy(alpha = if (enabled) 0.24f else 0.10f))
    ) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 9.dp),
            style = MaterialTheme.typography.labelLarge.copy(
                color = accent.copy(alpha = if (enabled) 0.94f else 0.42f),
                fontWeight = FontWeight.SemiBold
            )
        )
    }
}
"####;

pub const AI_CONTROLLER_PATH: &str = "com/ofairyo/gridtimer/ui/AiConnectionController.kt";
pub const AI_CONTROLLER_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import java.net.URI

internal data class AiConfiguration(val apiKey: String, val baseUrl: String, val model: String) {
    fun normalized() = copy(apiKey = apiKey.trim(), baseUrl = baseUrl.trim().trimEnd('/'), model = model.trim())
    private val endpoint: URI? get() = runCatching { URI(baseUrl.trim()) }.getOrNull()
    val recipientHost: String get() = endpoint?.host.orEmpty().lowercase()
    val endpointValid: Boolean get() = endpoint?.let {
        it.scheme.equals("https", ignoreCase = true) && !it.host.isNullOrBlank() &&
            it.userInfo == null && it.fragment == null && it.query == null && model.isNotBlank() &&
            !it.path.orEmpty().trimEnd('/').endsWith("/responses") &&
            !it.path.orEmpty().trimEnd('/').endsWith("/chat/completions")
    } == true
    val testable: Boolean get() = endpointValid && apiKey.isNotBlank()
    private val origin: String? get() = endpoint?.takeIf {
        it.scheme.equals("https", ignoreCase = true) && !it.host.isNullOrBlank() && it.userInfo == null
    }?.let { "https://${it.host.lowercase()}:${if (it.port < 0) 443 else it.port}" }
    val isDeepSeek: Boolean get() = origin == "https://api.deepseek.com:443"
    fun setupDefaults(): AiConfiguration {
        val current = normalized()
        if (current.isDeepSeek && current.model.isBlank()) return current.copy(model = "deepseek-flash")
        if (current.apiKey.isBlank() && (current.baseUrl.isBlank() ||
            (current.baseUrl == "https://api.openai.com/v1" && current.model == "gpt-5.2"))) {
            return current.deepSeekPreset()
        }
        return current
    }
    fun featureAction(saved: AiConfiguration): AiFeatureAction = when {
        !testable -> AiFeatureAction.CONFIGURE
        normalized() != saved.normalized() -> AiFeatureAction.SAVE_AND_OPEN
        else -> AiFeatureAction.OPEN
    }
    fun deepSeekPreset() = AiConfiguration(if (isDeepSeek) apiKey else "", "https://api.deepseek.com", "deepseek-flash")
    fun withEndpoint(nextUrl: String): AiConfiguration {
        val next = copy(baseUrl = nextUrl)
        return next.copy(apiKey = if (origin != null && origin == next.origin) apiKey else "")
    }
    override fun toString() = "AiConfiguration(apiKey=<redacted>, endpointConfigured=${endpointValid})"
}

internal enum class AiFeatureAction { CONFIGURE, SAVE_AND_OPEN, OPEN }

internal data class AiConnectionOutcome(val ok: Boolean, val message: String, val structured: Boolean, val vision: Boolean) {
    val fullyAvailable: Boolean get() = ok && structured && vision
}

internal class AiConnectionTicket internal constructor(
    internal val id: Long,
    internal val generation: Long,
    val configuration: AiConfiguration
)

// A cancelled blocking JNI call retains its slot until it returns.
internal class AiConnectionController(initial: AiConfiguration) {
    @Volatile
    var configuration: AiConfiguration = initial.normalized()
        private set
    var outcome: AiConnectionOutcome? = null
        private set
    @Volatile
    var closed: Boolean = false
        private set
    private var generation = 0L
    private var nextId = 0L
    @Volatile
    private var pending: AiConnectionTicket? = null
    val busy: Boolean get() = pending != null
    @Synchronized
    fun update(next: AiConfiguration) {
        val normalized = next.normalized()
        if (configuration != normalized) {
            configuration = normalized
            cancel()
        }
    }
    @Synchronized
    fun begin(): AiConnectionTicket? {
        if (closed || busy || !configuration.testable) return null
        outcome = null
        return AiConnectionTicket(++nextId, generation, configuration).also { pending = it }
    }
    @Synchronized
    fun isCurrent(ticket: AiConnectionTicket): Boolean = !closed && pending === ticket &&
        ticket.generation == generation && ticket.configuration == configuration
    @Synchronized
    fun finish(ticket: AiConnectionTicket, result: AiConnectionOutcome): Boolean {
        val accepted = isCurrent(ticket)
        if (accepted) outcome = result
        release(ticket)
        return accepted
    }
    @Synchronized
    fun release(ticket: AiConnectionTicket) {
        if (pending === ticket) pending = null
    }
    @Synchronized
    fun cancel() {
        generation++
        outcome = null
    }
    @Synchronized
    fun close() {
        closed = true
        cancel()
    }
}
"####;

pub const AI_TEST_PATH: &str = "com/ofairyo/gridtimer/ui/AiConnectionControllerTest.kt";
pub const AI_TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.*
import org.junit.Test

class AiConnectionControllerTest {
    private fun configured() = AiConfiguration("test-key", "https://api.deepseek.com", "deepseek-flash")
    private fun success() = AiConnectionOutcome(true, "test result", true, true)

    @Test fun blankKeyAndInvalidEndpointCannotStart() {
        for (config in listOf(configured().copy(apiKey = " "), configured().copy(baseUrl = "http://example.com"),
            configured().copy(baseUrl = "https://user:password@example.com"),
            configured().copy(baseUrl = "https://api.deepseek.com/responses"),
            configured().copy(baseUrl = "https://api.deepseek.com/chat/completions"), configured().copy(model = ""))) {
            val state = AiConnectionController(config)
            assertNull(state.begin())
            assertFalse(state.busy)
        }
    }
    @Test fun repeatedTapCannotStartAnotherRequest() {
        val state = AiConnectionController(configured())
        val ticket = state.begin()!!
        assertNull(state.begin())
        assertTrue(state.finish(ticket, success()))
        assertFalse(state.busy)
    }
    @Test fun editedConfigurationRejectsOldSuccessEvenAfterChangingBack() {
        val state = AiConnectionController(configured())
        val ticket = state.begin()!!
        state.update(configured().copy(model = "another-model"))
        state.update(configured())
        assertFalse(state.finish(ticket, success()))
        assertNull(state.outcome)
    }
    @Test fun cancellationRejectsReceiptAndRetainsInFlightSlot() {
        val state = AiConnectionController(configured())
        val ticket = state.begin()!!
        state.cancel()
        assertNull(state.begin())
        assertFalse(state.finish(ticket, success()))
        assertNull(state.outcome)
        assertNotNull(state.begin())
    }
    @Test fun leavingWorkspaceOrScreenClosesReceiptBoundary() {
        val old = AiConnectionController(configured())
        val ticket = old.begin()!!
        old.close()
        val current = AiConnectionController(configured())
        assertFalse(old.finish(ticket, success()))
        assertNull(old.begin())
        assertNull(current.outcome)
    }
    @Test fun providerChangeCannotReuseUnrelatedCredential() {
        val other = AiConfiguration("other-service-key", "https://example.com/v1", "custom")
        assertEquals("", other.deepSeekPreset().apiKey)
        assertEquals("", configured().withEndpoint("https://different.example/v1").apiKey)
        assertEquals("", configured().withEndpoint("https://api.deepseek.com:8443").apiKey)
        assertEquals("test-key", configured().withEndpoint("https://api.deepseek.com/v1").apiKey)
        assertEquals("test-key", configured().deepSeekPreset().apiKey)
        assertFalse(other.toString().contains("other-service-key"))
    }
    @Test fun successRequiresBothCapabilitiesForCurrentConfiguration() {
        val state = AiConnectionController(configured())
        assertTrue(state.finish(state.begin()!!, AiConnectionOutcome(true, "ok", true, false)))
        assertFalse(state.outcome!!.fullyAvailable)
        assertTrue(state.finish(state.begin()!!, success()))
        assertTrue(state.outcome!!.fullyAvailable)
        state.update(configured().copy(apiKey = "replacement-key"))
        assertNull(state.outcome)
    }
    @Test fun freshSetupPrefillsDeepSeekWithoutNeedingAProbe() {
        val fresh = AiConfiguration("", "https://api.openai.com/v1", "gpt-5.2").setupDefaults()
        assertTrue(fresh.isDeepSeek)
        assertEquals("deepseek-flash", fresh.model)
        assertEquals("", fresh.apiKey)
        assertEquals(AiFeatureAction.CONFIGURE, fresh.featureAction(fresh))
    }
    @Test fun defaultsPreserveExistingCredentialsAndCustomRecipients() {
        val custom = AiConfiguration("custom-key", "https://example.com/v1", "custom-model")
        assertEquals(custom, custom.setupDefaults())
        val unknown = AiConfiguration("unknown-key", "", "")
        assertEquals(unknown, unknown.setupDefaults())
        val deepSeek = configured().copy(model = "").setupDefaults()
        assertEquals("test-key", deepSeek.apiKey)
        assertEquals("deepseek-flash", deepSeek.model)
    }
    @Test fun actualFeatureEntryRequiresSavedConfigButNotSuccessfulProbe() {
        val config = configured()
        val state = AiConnectionController(config)
        assertEquals(AiFeatureAction.OPEN, state.configuration.featureAction(config))
        state.finish(state.begin()!!, AiConnectionOutcome(false, "temporary failure", false, false))
        assertEquals(AiFeatureAction.OPEN, state.configuration.featureAction(config))
        assertEquals(AiFeatureAction.SAVE_AND_OPEN, config.copy(model = "another-model").featureAction(config))
        assertEquals(AiFeatureAction.CONFIGURE, config.copy(apiKey = "").featureAction(config))
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::CONTENTS;

    #[test]
    fn my_page_keeps_account_appearance_language_and_ai_sections() {
        assert!(CONTENTS.contains("myPageText(MyPageText.ACCOUNT)"));
        assert!(CONTENTS.contains("myPageText(MyPageText.APPEARANCE)"));
        assert!(CONTENTS.contains("myPageText(MyPageText.AI_SETTINGS)"));
        assert!(CONTENTS.contains("myPageText(MyPageText.LANGUAGE)"));
        assert!(CONTENTS.contains("language.apply()"));
        assert!(!CONTENTS.contains("MyStatPill"));
        assert!(!CONTENTS.contains("text = \"格子\""));
        assert!(CONTENTS.contains("AnimatedVisibility(visible = accountSettingsExpanded)"));
        assert!(CONTENTS.contains("AnimatedVisibility(visible = aiSettingsExpanded)"));
    }
}
