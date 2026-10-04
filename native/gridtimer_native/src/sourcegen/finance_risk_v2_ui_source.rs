// Android finance review UI. Generated Kotlin is derived from this Rust source.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/FinanceRiskV2Panel.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import com.ofairyo.gridtimer.data.FinanceReviewStore
import com.ofairyo.gridtimer.data.formatFinanceMinor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.time.LocalDate
import java.time.YearMonth
import java.util.Locale
import org.json.JSONArray
import org.json.JSONObject

/** Finance review remains local to this workspace; only the source profile is synced. */
@Composable
fun FinanceRiskV2Panel(
    profileJson: String?,
    workspaceKey: String,
    referenceDate: LocalDate,
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current
    val appContext = remember(context) { context.applicationContext }
    val scope = rememberCoroutineScope()
    val storeMutex = remember(workspaceKey) { Mutex() }
    val activeWorkspaceKey by rememberUpdatedState(workspaceKey)
    val today = LocalDate.now()
    val currentMonth = YearMonth.from(today)
    var selectedMonth by remember(workspaceKey, referenceDate) {
        mutableStateOf(YearMonth.from(referenceDate).let { month ->
            if (month.isAfter(currentMonth)) currentMonth else month
        })
    }
    var receiptsJson by remember(workspaceKey) { mutableStateOf("[]") }
    var receiptsLoaded by remember(workspaceKey) { mutableStateOf(false) }
    var receiptsLoadFailed by remember(workspaceKey) { mutableStateOf(false) }
    var loadAttempt by remember(workspaceKey) { mutableLongStateOf(0L) }
    var requestVersion by remember(workspaceKey) { mutableLongStateOf(0L) }
    var savingReceipt by remember(workspaceKey) { mutableStateOf(false) }
    var errorText by remember(workspaceKey) { mutableStateOf<String?>(null) }
    LaunchedEffect(appContext, workspaceKey, loadAttempt) {
        val requestedWorkspace = workspaceKey
        val issuedVersion = ++requestVersion
        val result = withContext(Dispatchers.IO) {
            runCatching {
                storeMutex.withLock {
                    FinanceReviewStore.loadReceipts(appContext, requestedWorkspace)
                }
            }
        }
        if (activeWorkspaceKey != requestedWorkspace || requestVersion != issuedVersion) {
            return@LaunchedEffect
        }
        result.onSuccess {
            receiptsJson = it
            receiptsLoaded = true
            receiptsLoadFailed = false
            errorText = null
        }.onFailure {
            receiptsLoaded = false
            receiptsLoadFailed = true
            errorText = "本机核对记录读取失败，请重试"
        }
    }
    val monthKey = selectedMonth.toString()
    val selectedDay = if (selectedMonth == currentMonth) {
        if (YearMonth.from(referenceDate) == currentMonth) {
            referenceDate.dayOfMonth.coerceAtMost(today.dayOfMonth)
        } else {
            today.dayOfMonth
        }
    } else {
        selectedMonth.lengthOfMonth()
    }
    val fingerprints = remember(profileJson, monthKey) {
        profileJson?.let { profile ->
            NativeOptimizerBridge.buildAndroidFinanceReviewFingerprintsJson(profile, monthKey)
        }?.let { raw -> runCatching { JSONObject(raw) }.getOrNull() }
    }
    val risk = remember(profileJson, receiptsJson, selectedMonth, selectedDay) {
        profileJson?.let { profile ->
            NativeOptimizerBridge.buildAndroidFinanceRiskV2Json(
                profile,
                receiptsJson,
                selectedMonth.year,
                selectedMonth.monthValue,
                selectedDay
            )
        }?.let { raw -> runCatching { JSONObject(raw) }.getOrNull() }
    }
    fun submitReceiptWrite(failureText: String, write: suspend () -> String) {
        if (!receiptsLoaded || savingReceipt) return
        val submittedWorkspace = workspaceKey
        val issuedVersion = ++requestVersion
        savingReceipt = true
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching { storeMutex.withLock { write() } }
            }
            if (activeWorkspaceKey != submittedWorkspace || requestVersion != issuedVersion) {
                return@launch
            }
            savingReceipt = false
            result.onSuccess {
                receiptsJson = it
                errorText = null
            }.onFailure {
                errorText = failureText
            }
        }
    }
    fun confirm(category: String, zeroConfirmed: Boolean) {
        val rawFingerprint = fingerprints?.optString("${category}Fingerprint").orEmpty()
        if (rawFingerprint.isBlank()) {
            errorText = "无法核对：本月数据指纹未生成"
            return
        }
        val fingerprint = if (category == "incomeExpense") {
            if (selectedMonth.isBefore(currentMonth)) {
                if (zeroConfirmed) "zero-full:$rawFingerprint" else "full:$rawFingerprint"
            } else {
                "through:$selectedDay:$rawFingerprint"
            }
        } else {
            rawFingerprint
        }
        val submittedWorkspace = workspaceKey
        val submittedMonth = monthKey
        submitReceiptWrite("核对记录未保存，请重试") {
            FinanceReviewStore.confirm(
                appContext,
                submittedWorkspace,
                submittedMonth,
                category,
                fingerprint,
                zeroConfirmed
            )
        }
    }
    fun setRecurring(key: String, confirmed: Boolean) {
        val submittedWorkspace = workspaceKey
        val submittedMonth = monthKey
        submitReceiptWrite("周期项确认未保存，请重试") {
            val latest = FinanceReviewStore.loadReceipts(appContext, submittedWorkspace)
            val keys = financeConfirmedKeys(latest, submittedMonth).toMutableSet()
            if (confirmed) keys.remove(key) else keys.add(key)
            FinanceReviewStore.setConfirmedRecurringKeys(
                appContext, submittedWorkspace, submittedMonth, keys.toList()
            )
        }
    }

    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text("财务风控", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(monthKey, style = MaterialTheme.typography.labelLarge)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { selectedMonth = selectedMonth.minusMonths(1) },
                enabled = selectedMonth.isAfter(currentMonth.minusMonths(6))
            ) { Text("上个月") }
            OutlinedButton(
                onClick = { selectedMonth = selectedMonth.plusMonths(1) },
                enabled = selectedMonth.isBefore(currentMonth)
            ) { Text("下个月") }
        }
        if (!receiptsLoaded) {
            Text(errorText ?: "正在读取本机核对记录", style = MaterialTheme.typography.bodySmall)
            if (receiptsLoadFailed) {
                OutlinedButton(onClick = { loadAttempt += 1L }) { Text("重试读取") }
            }
            return@Column
        }
        if (profileJson == null || risk == null || fingerprints == null) {
            Text("财务资料暂时无法读取", color = MaterialTheme.colorScheme.error)
            return@Column
        }
        if (errorText != null) {
            Text(errorText.orEmpty(), color = MaterialTheme.colorScheme.error)
        }
        if (savingReceipt) {
            Text("正在保存核对记录", style = MaterialTheme.typography.bodySmall)
        }

        FinanceRiskV2Section("已录事实") {
            val hasRecordedDay = risk.optLong("recordedDays") > 0L
            FinanceRiskV2Value("收入", if (hasRecordedDay) financeYuan(risk.optLong("recordedIncome")) else "未记录")
            FinanceRiskV2Value("流出", if (hasRecordedDay) financeYuan(risk.optLong("recordedOutflow")) else "未记录")
            FinanceRiskV2Value("净现金流", if (hasRecordedDay) financeYuan(risk.optLong("recordedNetCashflow")) else "未记录")
            FinanceRiskV2Value(
                "记账天数",
                if (risk.optLong("recordedDays") == 0L) "未记录" else "${risk.optLong("recordedDays")} 天"
            )
            FinanceRiskV2Value("现金余额", financeKnownYuan(risk, "recordedCash"))
            FinanceRiskV2Value("资产", financeKnownYuan(risk, "recordedAssets"))
            FinanceRiskV2Value("负债", financeKnownYuan(risk, "recordedLiabilities"))
        }

        FinanceRiskV2Section("经核对的判断") {
            val state = risk.optString("riskState")
            FinanceRiskV2Value("健康结论", when (state) {
                "STABLE" -> "平稳"
                "WATCH" -> "留意"
                "TIGHT" -> "资金偏紧"
                "CRITICAL" -> "风险较高"
                else -> "待核对"
            })
            val reason = when (risk.optString("reasonCode")) {
                "THREE_REVIEWED_MONTHS_REQUIRED" -> "先核对最近 6 个月中的 3 个完整月份"
                "CURRENT_REVIEW_INCOMPLETE" -> "本月收支、现金、资产和负债尚未核对齐全"
                "VERIFIED_NEGATIVE_NET_WORTH" -> "已核对负债超过资产"
                "VERIFIED_CASH_GAP" -> "已核对现金覆盖出现缺口"
                "VERIFIED_90_DAY_GAP" -> "按已核对基线推算，90 天内可能有缺口"
                else -> "依据当前已核对记录"
            }
            Text(reason, style = MaterialTheme.typography.bodySmall)
            val safe = financeOptionalLong(risk, "safeToSpend")
            FinanceRiskV2Value(
                "安全可支配",
                when {
                    safe == null -> "待核对"
                    safe < 0L -> "缺口 ${financeYuan(-safe)}"
                    else -> financeYuan(safe)
                }
            )
            FinanceRiskV2Value("30 天余额", financeKnownYuan(risk, "projectedBalance30Days"))
            FinanceRiskV2Value("90 天余额", financeKnownYuan(risk, "projectedBalance90Days"))
            FinanceRiskV2Value("待付必要项", financeKnownYuan(risk, "pendingEssentialReserve"))
            FinanceRiskV2Value("保护配置", financeKnownYuan(risk, "protectedAllocation"))
            val months = risk.optJSONArray("baselineMonths") ?: JSONArray()
            Text(
                if (months.length() >= 3) {
                    "预测基线：${(0 until months.length()).joinToString("、") { months.optString(it) }}"
                } else {
                    "预测基线：还需核对 ${3 - months.length()} 个完整月份"
                },
                style = MaterialTheme.typography.bodySmall
            )
            if (risk.optBoolean("forecastAvailable")) {
                FinanceRiskV2Value("月收入基线", financeKnownYuan(risk, "monthlyIncomeBaseline"))
                FinanceRiskV2Value("月流出基线", financeKnownYuan(risk, "monthlyOutflowBaseline"))
            }
        }

        FinanceRiskV2Section("核对 ${monthKey} 数据") {
            Text("只对所选月份生效。修改记录后，相关核对会自动失效。", style = MaterialTheme.typography.bodySmall)
            val zeroRecordedDays = risk.optLong("recordedDays") == 0L
            val completedMonth = selectedMonth.isBefore(currentMonth)
            Text(
                if (completedMonth) {
                    "整月收支确认后才会进入预测基线；未记录月份须单独确认整月为 0。"
                } else {
                    "当月仅确认截至所选日期；月末后还需核对整月，才会进入预测基线。"
                },
                style = MaterialTheme.typography.bodySmall
            )
            FinanceRiskV2ReviewAction(
                label = "收支",
                verified = risk.optBoolean("incomeExpenseVerified"),
                hasRecordedValue = !zeroRecordedDays,
                enabled = !savingReceipt,
                statusOverride = if (zeroRecordedDays) {
                    if (risk.optBoolean("incomeExpenseVerified")) "未记录，已核对" else "未记录"
                } else null,
                reviewButtonText = if (completedMonth) "确认整月收支已记全" else "确认截至所选日已记全",
                zeroButtonText = if (completedMonth) "确认整月无收支（0）" else "确认截至所选日无收支",
                onReview = { confirm("incomeExpense", false) },
                onConfirmZero = { confirm("incomeExpense", true) }
            )
            FinanceRiskV2ReviewAction(
                label = "现金余额",
                verified = risk.optBoolean("cashVerified") && financeOptionalLong(risk, "verifiedCash") != null,
                hasRecordedValue = financeOptionalLong(risk, "recordedCash") != null,
                enabled = !savingReceipt,
                onReview = { confirm("cash", false) },
                onConfirmZero = { confirm("cash", true) }
            )
            FinanceRiskV2ReviewAction(
                label = "资产",
                verified = risk.optBoolean("assetsVerified") && financeOptionalLong(risk, "verifiedAssets") != null,
                hasRecordedValue = financeOptionalLong(risk, "recordedAssets") != null,
                enabled = !savingReceipt,
                onReview = { confirm("assets", false) },
                onConfirmZero = { confirm("assets", true) }
            )
            FinanceRiskV2ReviewAction(
                label = "负债",
                verified = risk.optBoolean("liabilitiesVerified") && financeOptionalLong(risk, "verifiedLiabilities") != null,
                hasRecordedValue = financeOptionalLong(risk, "recordedLiabilities") != null,
                enabled = !savingReceipt,
                onReview = { confirm("liabilities", false) },
                onConfirmZero = { confirm("liabilities", true) }
            )
        }

        val recurring = risk.optJSONArray("recurringExpenses") ?: JSONArray()
        FinanceRiskV2Section("周期支出") {
            if (recurring.length() == 0) {
                Text("暂无跨月稳定记录", style = MaterialTheme.typography.bodySmall)
            } else {
                for (index in 0 until recurring.length()) {
                    val item = recurring.optJSONObject(index) ?: continue
                    val key = item.optString("key")
                    val confirmed = item.optBoolean("confirmed")
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(item.optString("name"), style = MaterialTheme.typography.bodyMedium)
                            Text(
                                "每月约 ${financeYuan(item.optLong("monthlyAmount"))} · " +
                                    if (item.optBoolean("paidThisMonth")) "本月已记" else "本月未记",
                                style = MaterialTheme.typography.bodySmall
                            )
                        }
                        OutlinedButton(
                            onClick = { setRecurring(key, confirmed) },
                            enabled = risk.optBoolean("incomeExpenseVerified") && !savingReceipt
                        ) { Text(if (confirmed) "取消确认" else "确认周期") }
                    }
                }
            }
        }

        val duplicates = risk.optJSONArray("duplicatePayments") ?: JSONArray()
        FinanceRiskV2Section("疑似重复付款") {
            if (duplicates.length() == 0) {
                Text("暂无同日、同金额、同对象的重复记录", style = MaterialTheme.typography.bodySmall)
            } else {
                for (index in 0 until duplicates.length()) {
                    val item = duplicates.optJSONObject(index) ?: continue
                    Text(
                        "${item.optString("dayKey")} · ${item.optString("name")} · " +
                            "${financeYuan(item.optLong("amount"))}，请核对两条原始记录",
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }
        }
        if (financeOptionalLong(risk, "anomalyAmount") != null) {
            FinanceRiskV2Section("异常提醒") {
                Text(
                    "${risk.optString("anomalyDayKey")} 的非固定支出 " +
                        financeKnownYuan(risk, "anomalyAmount") + " 高于已核对基线，请检查记录",
                    style = MaterialTheme.typography.bodyMedium
                )
            }
        }
        Spacer(Modifier.height(4.dp))
    }
}

@Composable
private fun FinanceRiskV2Section(title: String, content: @Composable () -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
            content()
        }
    }
}

@Composable
private fun FinanceRiskV2Value(label: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = MaterialTheme.typography.bodyMedium)
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun FinanceRiskV2ReviewAction(
    label: String,
    verified: Boolean,
    hasRecordedValue: Boolean,
    enabled: Boolean,
    statusOverride: String? = null,
    reviewButtonText: String = "核对记录",
    zeroButtonText: String? = null,
    onReview: () -> Unit,
    onConfirmZero: (() -> Unit)?
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text("$label · ${statusOverride ?: if (verified) "已核对" else "待核对"}", modifier = Modifier.weight(1f))
        if (hasRecordedValue) {
            OutlinedButton(onClick = onReview, enabled = enabled) { Text(reviewButtonText) }
        } else if (onConfirmZero != null) {
            Button(onClick = onConfirmZero, enabled = enabled) {
                Text(zeroButtonText ?: "确认 $label 为 0")
            }
        }
    }
}

private fun financeOptionalLong(value: JSONObject, key: String): Long? =
    if (value.has(key) && !value.isNull(key)) value.optLong(key) else null

private fun financeKnownYuan(value: JSONObject, key: String): String =
    financeOptionalLong(value, key)?.let(::financeYuan) ?: "待核对"

private fun financeYuan(value: Long): String =
    formatFinanceMinor(value, "¥")

private fun financeConfirmedKeys(receiptsJson: String, monthKey: String): Set<String> {
    val receipts = runCatching { JSONArray(receiptsJson) }.getOrNull() ?: return emptySet()
    for (index in receipts.length() - 1 downTo 0) {
        val receipt = receipts.optJSONObject(index) ?: continue
        if (receipt.optString("monthKey") != monthKey) continue
        val keys = receipt.optJSONArray("confirmedRecurringKeys") ?: return emptySet()
        return (0 until keys.length()).mapNotNull { item ->
            keys.optString(item).takeIf { it.isNotBlank() }
        }.toSet()
    }
    return emptySet()
}
"####;
