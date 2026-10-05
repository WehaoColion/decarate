//! Task-oriented finance UI. Preserve native results and all explicit review writes.
//! Applied after the legal workflow, to the actual generated Android entry points.

const GRID: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";
const PANEL: &str = "com/ofairyo/gridtimer/ui/FinanceRiskV2Panel.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/FinanceWorkspacePolicyTest.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!("finance workspace anchor missing or duplicated: {}", before.lines().next().unwrap_or("")));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

fn unique_index(source: &str, marker: &str) -> Result<usize, String> {
    if source.matches(marker).count() != 1 {
        return Err(format!("finance workspace section missing or duplicated: {marker}"));
    }
    source.find(marker).ok_or_else(|| format!("missing {marker}"))
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != GRID && path != PANEL {
        return Ok(source.to_owned());
    }
    let mut result = source.to_owned();
    if path == GRID {
        replace_once(&mut result, LEGAL_HERO, COMPACT_LEGAL_HERO)?;
        return Ok(result);
    }
    let markers = [
        "        FinanceRiskV2Section(\"已录事实\") {",
        "        FinanceRiskV2Section(\"经核对的判断\") {",
        "        FinanceRiskV2Section(\"核对 ${monthKey} 数据\") {",
        "        val recurring = risk.optJSONArray(\"recurringExpenses\") ?: JSONArray()",
        "        val duplicates = risk.optJSONArray(\"duplicatePayments\") ?: JSONArray()",
        "        Spacer(Modifier.height(4.dp))",
    ];
    let positions = markers.iter().map(|m| unique_index(&result, m)).collect::<Result<Vec<_>, _>>()?;
    if positions.windows(2).any(|p| p[0] >= p[1]) {
        return Err("finance workspace section order changed".into());
    }
    // Copy original section bodies verbatim. UI navigation never creates a receipt,
    // recalculates a risk classification, or turns an unknown balance into zero.
    let sections = positions.windows(2).map(|p| result[p[0]..p[1]].to_owned()).collect::<Vec<_>>();
    let mut content = String::from(WORKSPACE);
    content.push_str("\n        val openedDetail = route.detail\n        if (openedDetail != null) {\n            FinanceWorkspaceDetailDialog(openedDetail.title, onDismiss = { savedRoute = route.copy(detail = null) }) {\n                if (errorText != null) Text(errorText.orEmpty(), color = MaterialTheme.colorScheme.error)\n                if (savingReceipt) Text(\"正在保存核对记录\", style = MaterialTheme.typography.bodySmall)\n                when (openedDetail) {\n");
    for (name, index) in [("FACTS", 0), ("FORECAST", 1), ("VERIFY", 2), ("RECURRING", 3), ("ALERTS", 4)] {
        content.push_str(&format!("                    FinanceWorkspaceDetail.{name} -> {{\n"));
        content.push_str(&sections[index]);
        content.push_str("                    }\n");
    }
    content.push_str("                }\n            }\n        }\n");
    result.replace_range(positions[0]..positions[5], &content);
    replace_once(&mut result, MONTH_NAVIGATION, COMPACT_MONTH_NAVIGATION)?;
    for import in [
        "androidx.compose.foundation.layout.heightIn",
        "androidx.compose.foundation.layout.safeDrawing",
        "androidx.compose.foundation.layout.WindowInsets",
        "androidx.compose.foundation.layout.windowInsetsPadding",
        "androidx.compose.foundation.layout.fillMaxSize",
        "androidx.compose.foundation.rememberScrollState",
        "androidx.compose.foundation.verticalScroll",
        "androidx.compose.ui.platform.testTag",
        "androidx.compose.ui.semantics.semantics",
        "androidx.compose.ui.semantics.contentDescription",
    ] {
        let statement = format!("import {import}\n");
        if !result.contains(&statement) {
            replace_once(&mut result, "package com.ofairyo.gridtimer.ui\n", &format!("package com.ofairyo.gridtimer.ui\n{statement}"))?;
        }
    }
    replace_once(
        &mut result,
        "        Text(label, style = MaterialTheme.typography.bodyMedium)\n        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)",
        "        Text(label, modifier = Modifier.weight(1f).padding(end = 8.dp), style = MaterialTheme.typography.bodyMedium)\n        Text(value, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.End, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)",
    )?;
    result.push_str(POLICY);
    result.push_str(COMPONENTS);
    Ok(result)
}

const LEGAL_HERO: &str = r####"                Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("AI 法律风险分析", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                        Text("扫描任务、笔记、知识页和账目，逐条给出待核查线索与原始记录。", style = MaterialTheme.typography.bodyMedium)
                        androidx.compose.material3.Button(
                            onClick = { showLegalRisk = true },
                            modifier = Modifier.fillMaxWidth().testTag("legal_open_analysis")
                        ) { Text("开始 AI 分析") }
                        TextButton(onClick = onOpenAiSettings) { Text("AI 设置") }
                        Text(if (syncSession.aiApiKey.isBlank()) "先配置 AI，扫描预览可以在本机查看。" else "${syncSession.aiModel} · 点击上方按钮，整理资料后确认发送", style = MaterialTheme.typography.bodySmall)
                    }
                }"####;

const COMPACT_LEGAL_HERO: &str = r####"                Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Column(modifier = Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text("法律风险分析", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                                Text(if (syncSession.aiApiKey.isBlank()) "可先在本机整理资料" else "${syncSession.aiModel} · 确认后才发送",
                                    style = MaterialTheme.typography.bodySmall, maxLines = 1,
                                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                            }
                            TextButton(onClick = onOpenAiSettings) { Text("AI 设置") }
                        }
                        androidx.compose.material3.Button(
                            onClick = { showLegalRisk = true },
                            modifier = Modifier.fillMaxWidth().testTag("legal_open_analysis")
                        ) { Text("开始 AI 分析") }
                    }
                }"####;

const MONTH_NAVIGATION: &str = r####"        Row(
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
        }"####;

const COMPACT_MONTH_NAVIGATION: &str = r####"        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("财务风控", modifier = Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            androidx.compose.material3.IconButton(
                modifier = Modifier.testTag("risk_previous_month").semantics { contentDescription = "上个月" },
                onClick = { selectedMonth = selectedMonth.minusMonths(1) },
                enabled = selectedMonth.isAfter(currentMonth.minusMonths(6))
            ) { Text("‹", style = MaterialTheme.typography.headlineSmall) }
            Text(monthKey, style = MaterialTheme.typography.labelLarge)
            androidx.compose.material3.IconButton(
                modifier = Modifier.testTag("risk_next_month").semantics { contentDescription = "下个月" },
                onClick = { selectedMonth = selectedMonth.plusMonths(1) },
                enabled = selectedMonth.isBefore(currentMonth)
            ) { Text("›", style = MaterialTheme.typography.headlineSmall) }
        }"####;

pub const POLICY: &str = r####"

internal enum class FinanceWorkspacePage(val label: String) {
    OVERVIEW("总览"), REVIEW("核对"), FORECAST("预测")
}

internal enum class FinanceWorkspaceDetail(val title: String) {
    FACTS("已录事实"), VERIFY("核对本月数据"), RECURRING("周期支出"),
    ALERTS("需要核查的记录"), FORECAST("预测与判断依据")
}

// Navigation state only. No receipts, amounts, API configuration or personal records.
internal data class FinanceWorkspaceRoute(
    val workspaceKey: String,
    val monthKey: String,
    val page: FinanceWorkspacePage = FinanceWorkspacePage.OVERVIEW,
    val detail: FinanceWorkspaceDetail? = null
) {
    fun forScope(workspace: String, month: String): FinanceWorkspaceRoute =
        if (workspaceKey == workspace && monthKey == month) this else FinanceWorkspaceRoute(workspace, month)
    fun select(next: FinanceWorkspacePage): FinanceWorkspaceRoute = copy(page = next, detail = null)
}

internal fun financeWorkspaceReviewRemaining(income: Boolean, cash: Boolean, assets: Boolean, liabilities: Boolean): Int =
    listOf(income, cash, assets, liabilities).count { !it }

internal fun financeWorkspacePreviewCount(total: Int): Int = total.coerceIn(0, 2)

internal fun financeWorkspaceRiskLabel(state: String): String = when (state) {
    "STABLE" -> "平稳"
    "WATCH" -> "留意"
    "TIGHT" -> "资金偏紧"
    "CRITICAL" -> "风险较高"
    else -> "待核对"
}

internal fun financeWorkspaceRiskReason(reason: String): String = when (reason) {
    "THREE_REVIEWED_MONTHS_REQUIRED" -> "先核对最近 6 个月中的 3 个完整月份"
    "CURRENT_REVIEW_INCOMPLETE" -> "本月收支、现金、资产和负债尚未核对齐全"
    "VERIFIED_NEGATIVE_NET_WORTH" -> "已核对负债超过资产"
    "VERIFIED_CASH_GAP" -> "已核对现金覆盖出现缺口"
    "VERIFIED_90_DAY_GAP" -> "按已核对基线推算，90 天内可能有缺口"
    else -> "依据当前已核对记录"
}
"####;

const WORKSPACE: &str = r####"        var savedRoute by remember(workspaceKey, monthKey) {
            mutableStateOf(FinanceWorkspaceRoute(workspaceKey, monthKey))
        }
        val route = savedRoute.forScope(workspaceKey, monthKey)
        // Navigation callbacks update presentation state only.
        val remainingReviews = financeWorkspaceReviewRemaining(
            risk.optBoolean("incomeExpenseVerified"),
            risk.optBoolean("cashVerified") && financeOptionalLong(risk, "verifiedCash") != null,
            risk.optBoolean("assetsVerified") && financeOptionalLong(risk, "verifiedAssets") != null,
            risk.optBoolean("liabilitiesVerified") && financeOptionalLong(risk, "verifiedLiabilities") != null
        )
        val duplicateItems = risk.optJSONArray("duplicatePayments") ?: JSONArray()
        val recurringItems = risk.optJSONArray("recurringExpenses") ?: JSONArray()
        val hasAnomaly = financeOptionalLong(risk, "anomalyAmount") != null
        val hasRecordedDay = risk.optLong("recordedDays") > 0L
        val baselineCount = (risk.optJSONArray("baselineMonths") ?: JSONArray()).length()
        androidx.compose.material3.TabRow(selectedTabIndex = route.page.ordinal) {
            FinanceWorkspacePage.values().forEach { page ->
                androidx.compose.material3.Tab(selected = route.page == page,
                    onClick = { savedRoute = route.select(page) },
                    modifier = Modifier.heightIn(min = 48.dp).testTag("risk_tab_${page.name.lowercase(Locale.ROOT)}"),
                    text = { Text(if (page == FinanceWorkspacePage.REVIEW && remainingReviews > 0) "核对 $remainingReviews" else page.label) })
            }
        }
        when (route.page) {
            FinanceWorkspacePage.OVERVIEW -> {
                FinanceRiskV2Section("当前判断") {
                    val state = risk.optString("riskState")
                    Text(financeWorkspaceRiskLabel(state), style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.SemiBold,
                        color = if (state == "CRITICAL" || state == "TIGHT") MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)
                    Text(financeWorkspaceRiskReason(risk.optString("reasonCode")), style = MaterialTheme.typography.bodyMedium)
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        FinanceWorkspaceMetric("安全可支配", financeKnownYuan(risk, "safeToSpend"), Modifier.weight(1f))
                        FinanceWorkspaceMetric("已核对现金", if (risk.optBoolean("cashVerified")) financeKnownYuan(risk, "verifiedCash") else "待核对", Modifier.weight(1f))
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        FinanceWorkspaceMetric("已录净现金流", if (hasRecordedDay) financeYuan(risk.optLong("recordedNetCashflow")) else "未记录", Modifier.weight(1f))
                        FinanceWorkspaceMetric("30 天预计余额", financeKnownYuan(risk, "projectedBalance30Days"), Modifier.weight(1f))
                    }
                    Text("本月已核对 ${4 - remainingReviews}/4 项 · 基线已核对 $baselineCount 个完整月", style = MaterialTheme.typography.bodySmall)
                    Button(onClick = { savedRoute = route.select(FinanceWorkspacePage.REVIEW).copy(detail = FinanceWorkspaceDetail.VERIFY) },
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("risk_review_now")) {
                        Text(if (remainingReviews > 0) "核对剩余 $remainingReviews 项" else "查看本月核对")
                    }
                }
                if (duplicateItems.length() > 0 || hasAnomaly) {
                    FinanceRiskV2Section("需要处理") {
                        if (duplicateItems.length() > 0) Text("${duplicateItems.length()} 组疑似重复付款，尚未认定为重复扣款")
                        for (index in 0 until financeWorkspacePreviewCount(duplicateItems.length())) {
                            val item = duplicateItems.optJSONObject(index) ?: continue
                            Text("${item.optString("dayKey")} · ${item.optString("name")} · ${financeYuan(item.optLong("amount"))}",
                                maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                                style = MaterialTheme.typography.bodySmall)
                        }
                        if (hasAnomaly) Text("${risk.optString("anomalyDayKey")} 的非固定支出高于已核对基线，请检查记录")
                        OutlinedButton(onClick = { savedRoute = route.copy(detail = FinanceWorkspaceDetail.ALERTS) },
                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("risk_open_alerts")) { Text("查看全部待核查记录") }
                    }
                }
            }
            FinanceWorkspacePage.REVIEW -> {
                FinanceWorkspaceEntry("本月核对", if (remainingReviews == 0) "4 项已核对；修改原记录后需重新核对" else "还有 $remainingReviews 项待核对，不会自动把空白确认为 0",
                    "risk_detail_verify", { savedRoute = route.copy(detail = FinanceWorkspaceDetail.VERIFY) })
                FinanceWorkspaceEntry("已录事实", "${risk.optLong("recordedDays")} 天有记录 · 收支、现金、资产与负债",
                    "risk_detail_facts", { savedRoute = route.copy(detail = FinanceWorkspaceDetail.FACTS) })
                FinanceWorkspaceEntry("周期支出", "${recurringItems.length()} 项 · 逐项确认或取消",
                    "risk_detail_recurring", { savedRoute = route.copy(detail = FinanceWorkspaceDetail.RECURRING) })
                FinanceWorkspaceEntry("待核查记录", "疑似重复 ${duplicateItems.length()} 组 · 异常提醒 ${if (hasAnomaly) 1 else 0} 条",
                    "risk_detail_alerts", { savedRoute = route.copy(detail = FinanceWorkspaceDetail.ALERTS) })
            }
            FinanceWorkspacePage.FORECAST -> {
                FinanceRiskV2Section("现金预测") {
                    Text(if (risk.optBoolean("forecastAvailable")) "依据已核对基线推算，不是确定收益或余额。" else "资料未核对齐全时不补造预测结果。")
                    FinanceRiskV2Value("30 天余额", financeKnownYuan(risk, "projectedBalance30Days"))
                    FinanceRiskV2Value("90 天余额", financeKnownYuan(risk, "projectedBalance90Days"))
                    Text("基线已核对 $baselineCount 个完整月；至少需要 3 个。", style = MaterialTheme.typography.bodySmall)
                    OutlinedButton(onClick = { savedRoute = route.copy(detail = FinanceWorkspaceDetail.FORECAST) },
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("risk_detail_forecast")) { Text("查看完整判断与预测依据") }
                    if (!risk.optBoolean("forecastAvailable")) Button(
                        onClick = { savedRoute = route.select(FinanceWorkspacePage.REVIEW).copy(detail = FinanceWorkspaceDetail.VERIFY) },
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("去核对数据") }
                }
            }
        }
"####;

const COMPONENTS: &str = r####"

@Composable
private fun FinanceWorkspaceMetric(label: String, value: String, modifier: Modifier = Modifier) {
    Column(modifier = modifier.padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, style = MaterialTheme.typography.bodySmall)
        // Do not truncate monetary values, including negatives and large balances.
        Text(value, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun FinanceWorkspaceEntry(title: String, summary: String, tag: String, onClick: () -> Unit) {
    OutlinedButton(onClick = onClick, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp).testTag(tag)) {
        Column(modifier = Modifier.weight(1f).padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
            Text(summary, style = MaterialTheme.typography.bodySmall)
        }
        Text("›", style = MaterialTheme.typography.titleLarge)
    }
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun FinanceWorkspaceDetailDialog(title: String, onDismiss: () -> Unit, content: @Composable () -> Unit) {
    androidx.compose.ui.window.Dialog(onDismissRequest = onDismiss,
        properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false,
            securePolicy = androidx.compose.ui.window.SecureFlagPolicy.SecureOn)) {
        androidx.compose.material3.Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column(modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing)) {
                Row(modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(title, modifier = Modifier.weight(1f), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                    androidx.compose.material3.TextButton(onClick = onDismiss, modifier = Modifier.heightIn(min = 48.dp)) { Text("返回") }
                }
                // A separate, bounded detail viewport. Closing it does not move the home list.
                Column(modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)) { content() }
            }
        }
    }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui
import org.junit.Assert.*
import org.junit.Test

class FinanceWorkspacePolicyTest {
    @Test fun startsOnOverviewWithoutDetail() {
        val state = FinanceWorkspaceRoute("a", "2026-10")
        assertEquals(FinanceWorkspacePage.OVERVIEW, state.page)
        assertNull(state.detail)
    }
    @Test fun sameScopeKeepsNavigation() {
        val state = FinanceWorkspaceRoute("a", "2026-10", FinanceWorkspacePage.REVIEW, FinanceWorkspaceDetail.RECURRING)
        assertSame(state, state.forScope("a", "2026-10"))
    }
    @Test fun newMonthClosesOldDetailAndReturnsToOverview() {
        val state = FinanceWorkspaceRoute("a", "2026-10", FinanceWorkspacePage.REVIEW, FinanceWorkspaceDetail.VERIFY)
        assertEquals(FinanceWorkspaceRoute("a", "2026-09"), state.forScope("a", "2026-09"))
    }
    @Test fun newWorkspaceDoesNotInheritFinancialDetail() {
        val state = FinanceWorkspaceRoute("a", "2026-10", FinanceWorkspacePage.FORECAST, FinanceWorkspaceDetail.FACTS)
        assertEquals(FinanceWorkspaceRoute("b", "2026-10"), state.forScope("b", "2026-10"))
    }
    @Test fun switchingTaskClearsThePreviousDialog() {
        val state = FinanceWorkspaceRoute("a", "2026-10", detail = FinanceWorkspaceDetail.ALERTS)
        for (page in FinanceWorkspacePage.values()) {
            assertEquals(page, state.select(page).page)
            assertNull(state.select(page).detail)
        }
    }
    @Test fun closingDetailPreservesTheSelectedTask() {
        val state = FinanceWorkspaceRoute("a", "2026-10", FinanceWorkspacePage.REVIEW, FinanceWorkspaceDetail.VERIFY)
        assertEquals(FinanceWorkspacePage.REVIEW, state.copy(detail = null).page)
    }
    @Test fun reviewCountUsesAllFourExplicitFlags() {
        for (mask in 0..15) {
            val flags = (0..3).map { mask and (1 shl it) != 0 }
            assertEquals(4 - Integer.bitCount(mask), financeWorkspaceReviewRemaining(flags[0], flags[1], flags[2], flags[3]))
        }
    }
    @Test fun alertPreviewIsBoundedWithoutChangingTheTotal() {
        for (count in listOf(0, 1, 2, 3, 1000, Int.MAX_VALUE)) {
            assertEquals(minOf(count, 2), financeWorkspacePreviewCount(count))
        }
        assertEquals(0, financeWorkspacePreviewCount(-1))
    }
    @Test fun unknownRiskIsNotStable() {
        assertEquals("待核对", financeWorkspaceRiskLabel(""))
        assertEquals("待核对", financeWorkspaceRiskLabel("UNKNOWN"))
    }
    @Test fun allExistingRiskStatesKeepTheirMeaning() {
        assertEquals("平稳", financeWorkspaceRiskLabel("STABLE"))
        assertEquals("留意", financeWorkspaceRiskLabel("WATCH"))
        assertEquals("资金偏紧", financeWorkspaceRiskLabel("TIGHT"))
        assertEquals("风险较高", financeWorkspaceRiskLabel("CRITICAL"))
    }
    @Test fun insufficientBaselineStillRequestsReview() {
        assertTrue(financeWorkspaceRiskReason("THREE_REVIEWED_MONTHS_REQUIRED").contains("3 个完整月份"))
        assertTrue(financeWorkspaceRiskReason("CURRENT_REVIEW_INCOMPLETE").contains("尚未核对齐全"))
    }
    @Test fun cashAndDebtWarningsAreNotReplacedByEmptyState() {
        assertTrue(financeWorkspaceRiskReason("VERIFIED_CASH_GAP").contains("缺口"))
        assertTrue(financeWorkspaceRiskReason("VERIFIED_90_DAY_GAP").contains("90 天"))
        assertTrue(financeWorkspaceRiskReason("VERIFIED_NEGATIVE_NET_WORTH").contains("负债超过资产"))
    }
}
"####;

#[cfg(test)]
#[path = "finance_risk_v2_ui_source.rs"]
mod original;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_financial_sections_and_actions_are_retained_once() {
        let generated = render(PANEL, original::CONTENTS).unwrap();
        for key in ["recordedIncome", "recordedOutflow", "recordedAssets", "recordedLiabilities", "safeToSpend", "projectedBalance90Days", "protectedAllocation", "monthlyIncomeBaseline", "monthlyOutflowBaseline", "confirmedRecurringKeys"] {
            assert!(generated.contains(key), "lost {key}");
        }
        for action in ["confirm(\"incomeExpense\", false)", "confirm(\"incomeExpense\", true)", "confirm(\"cash\", false)", "confirm(\"assets\", true)", "confirm(\"liabilities\", true)", "setRecurring(key, confirmed)"] {
            assert_eq!(generated.matches(action).count(), original::CONTENTS.matches(action).count());
        }
        assert!(generated.contains("FinanceWorkspaceDetailDialog(openedDetail.title"));
        assert!(generated.contains("savedRoute = route.copy(detail = null)"));
    }
    #[test]
    fn mutation_and_storage_functions_are_byte_identical() {
        let generated = render(PANEL, original::CONTENTS).unwrap();
        let start = "    fun submitReceiptWrite(";
        let end = "    Column(modifier = modifier.fillMaxWidth()";
        let extract = |text: &str| text.split(start).nth(1).unwrap().split(end).next().unwrap().to_owned();
        assert_eq!(extract(&generated), extract(original::CONTENTS));
    }
    #[test]
    fn stale_or_duplicate_templates_fail_instead_of_silently_shipping_old_ui() {
        assert!(render(PANEL, &original::CONTENTS.replace("FinanceRiskV2Section(\"已录事实\")", "DifferentFactsSection()")).is_err());
        assert!(render(PANEL, &format!("{}{}", original::CONTENTS, original::CONTENTS)).is_err());
        assert!(render(GRID, "not the expected legal hero").is_err());
    }
    #[test]
    fn compact_legal_entry_keeps_real_callbacks_and_no_send_call() {
        let generated = render(GRID, LEGAL_HERO).unwrap();
        assert!(generated.contains("onClick = { showLegalRisk = true }"));
        assert!(generated.contains("onClick = onOpenAiSettings"));
        assert!(generated.contains("legal_open_analysis"));
        assert!(!generated.contains("runLegalScan"));
    }
    #[test]
    fn unrelated_editors_are_not_changed() {
        assert_eq!(render("com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt", "original").unwrap(), "original");
    }
    #[test]
    fn detail_screen_cannot_mutate_finances_by_navigation() {
        assert!(!POLICY.contains("FinanceReviewStore"));
        assert!(!WORKSPACE.contains("confirm("));
        assert!(!WORKSPACE.contains("setRecurring("));
        assert!(COMPONENTS.contains("WindowInsets.safeDrawing"));
    }
}
