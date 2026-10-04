// Android money operations are generated from this Rust source.
pub const PATH: &str = "com/ofairyo/gridtimer/data/FinanceMoney.kt";
pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import java.math.BigDecimal
import java.text.DecimalFormat
import java.text.DecimalFormatSymbols
import java.util.Locale

internal fun financeMinorAmount(whole: Long, minor: Long?): Long {
    if (minor != null) {
        require(minor >= 0L && minor / 100L == whole) { "Inconsistent finance precision" }
        return minor
    }
    return Math.multiplyExact(whole, 100L)
}

internal fun parseFinanceMinorDraft(value: String): Long? {
    NativeOptimizerBridge.parseFinanceMinorDraft(value, 9, 999_999_999L)?.let { return it }
    val text = value.trim()
    if (text.isEmpty()) return 0L
    val parts = text.split('.')
    if (parts.size > 2) return null
    val whole = parts[0]
    val fraction = parts.getOrElse(1) { "" }
    if (whole.length > 9 || fraction.length > 2 ||
        whole.any { it !in '0'..'9' } || fraction.any { it !in '0'..'9' }) return null
    val major = if (whole.isEmpty()) 0L else whole.toLongOrNull() ?: return null
    if (major > 999_999_999L) return null
    val cents = fraction.padEnd(2, '0').toLongOrNull() ?: return null
    return Math.addExact(Math.multiplyExact(major, 100L), cents)
}

internal fun formatFinanceMinor(amount: Long, prefix: String = "", signed: Boolean = false, grouped: Boolean = true): String {
    NativeOptimizerBridge.formatFinanceMinor(amount, prefix, signed, grouped)?.let { return it }
    val value = BigDecimal.valueOf(amount, 2)
    val formatter = DecimalFormat(if (grouped) "#,##0.00" else "0.00", DecimalFormatSymbols(Locale.ROOT))
    val sign = if (amount < 0L) "-" else if (signed && amount > 0L) "+" else ""
    return sign + prefix + formatter.format(value.abs())
}

internal fun FinanceIncomeEntry.preciseAmount(): Long = financeMinorAmount(amount, amountMinor)
internal fun FinanceExpenseEntry.preciseAmount(): Long = financeMinorAmount(amount, amountMinor)
internal fun FinanceNamedAmountEntry.preciseAmount(): Long = financeMinorAmount(amount, amountMinor)

internal fun FinanceProfile.preciseLegacyAmount(field: String, whole: Long): Long {
    val minor = legacyAmountMinor[field]
    if (minor != null && whole < 0L && minor == Math.multiplyExact(whole, 100L)) return minor
    return financeMinorAmount(whole, minor)
}

internal fun FinanceProfile.requireValidFinancePrecision(): FinanceProfile {
    val fields = mapOf(
        "activeIncomeMonthly" to activeIncomeMonthly, "assetIncomeMonthly" to assetIncomeMonthly,
        "livingExpenseMonthly" to livingExpenseMonthly, "liabilityPaymentMonthly" to liabilityPaymentMonthly,
        "cashReserve" to cashReserve, "productiveAssetValue" to productiveAssetValue,
        "liabilityBalance" to liabilityBalance
    )
    for ((field, minor) in legacyAmountMinor) {
        val whole = fields[field] ?: throw IllegalArgumentException("Unknown finance money field")
        if (whole < 0L && minor == Math.multiplyExact(whole, 100L)) continue
        financeMinorAmount(whole, minor)
    }
    dailyLedgers.values.forEach { ledger ->
        ledger.incomes.forEach { row -> if (row.amountMinor != null) row.preciseAmount() }
        ledger.expenses.forEach { row -> if (row.amountMinor != null) row.preciseAmount() }
    }
    monthlySnapshots.values.forEach { snapshot ->
        snapshot.assets.forEach { row -> if (row.amountMinor != null) row.preciseAmount() }
        snapshot.liabilities.forEach { row -> if (row.amountMinor != null) row.preciseAmount() }
    }
    return this
}

internal fun FinanceProfile.withLegacyAmount(field: String, minor: Long): FinanceProfile {
    require(minor in 0L..99_999_999_999L)
    val precision = legacyAmountMinor + (field to minor)
    val whole = minor / 100L
    return when (field) {
        "activeIncomeMonthly" -> copy(activeIncomeMonthly = whole, legacyAmountMinor = precision)
        "assetIncomeMonthly" -> copy(assetIncomeMonthly = whole, legacyAmountMinor = precision)
        "livingExpenseMonthly" -> copy(livingExpenseMonthly = whole, legacyAmountMinor = precision)
        "liabilityPaymentMonthly" -> copy(liabilityPaymentMonthly = whole, legacyAmountMinor = precision)
        "cashReserve" -> copy(cashReserve = whole, legacyAmountMinor = precision)
        "productiveAssetValue" -> copy(productiveAssetValue = whole, legacyAmountMinor = precision)
        "liabilityBalance" -> copy(liabilityBalance = whole, legacyAmountMinor = precision)
        else -> error("Unknown finance money field")
    }
}
"####;

pub const TEST_PATH: &str = "com/ofairyo/gridtimer/data/FinanceMoneyTest.kt";
pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import com.ofairyo.gridtimer.ui.toFinanceAmountOrNull

class FinanceMoneyTest {
    @Test fun decimalDraftPreservesIncompleteInputAndRejectsThirdDecimal() {
        assertEquals(5800L, parseFinanceMinorDraft("58"))
        assertEquals(5800L, parseFinanceMinorDraft("58."))
        assertEquals(5810L, parseFinanceMinorDraft("58.1"))
        assertEquals(5812L, parseFinanceMinorDraft("58.12"))
        assertEquals(1L, parseFinanceMinorDraft(".01"))
        assertEquals(0L, parseFinanceMinorDraft("."))
        assertEquals(99_999_999_999L, parseFinanceMinorDraft("999999999.99"))
        for (value in listOf("58.123", "1..2", "1e2", "-1", "1000000000", "９")) assertNull(parseFinanceMinorDraft(value))
    }

    @Test fun incompleteUiInputCannotBecomeAZeroSave() {
        assertNull("".toFinanceAmountOrNull())
        assertNull(".".toFinanceAmountOrNull())
        assertNull("58.123".toFinanceAmountOrNull())
        assertEquals(0L, "0.00".toFinanceAmountOrNull())
        assertEquals(5812L, "58.12".toFinanceAmountOrNull())
    }

    @Test fun smallIncomeExpenseAssetAndLiabilityKeepCents() {
        val ledger = FinanceDayLedger(
            incomes = listOf(FinanceIncomeEntry(amount = 0, amountMinor = 1), FinanceIncomeEntry(amount = 0, amountMinor = 1)),
            expenses = listOf(FinanceExpenseEntry(amount = 58, amountMinor = 5812))
        )
        assertEquals(2L, ledger.toAggregate().incomeTotal)
        assertEquals(5812L, ledger.toAggregate().expenseTotal)
        assertEquals("¥0.02", formatFinanceMinor(ledger.toAggregate().incomeTotal, "¥"))
        val snapshot = FinanceMonthSnapshot(
            assets = listOf(FinanceNamedAmountEntry(amount = 0, amountMinor = 2)),
            liabilities = listOf(FinanceNamedAmountEntry(amount = 0, amountMinor = 1))
        ).toSummary()
        assertEquals(2L, snapshot.assetTotal)
        assertEquals(1L, snapshot.liabilityTotal)
        assertEquals(1L, snapshot.netWorth)
        assertEquals(5800L, financeMinorAmount(58, null))
        assertThrows(IllegalArgumentException::class.java) { financeMinorAmount(58, 5912) }
        assertThrows(IllegalArgumentException::class.java) { financeMinorAmount(0, -1) }
    }

    @Test fun calculationViewsAndInvalidPrecisionNeverEnterBackupRestore() {
        for (raw in listOf(
            """{"cashReserve":123,"calculationMoneyUnit":"rmb-cent-v1"}""",
            """{"financeProfile":{"cashReserve":123,"calculationMoneyUnit":null},"schemaVersion":1}""",
            """{"dailyLedgers":{"2026-10-02":{"expenses":[{"amount":58,"amountMinor":5912}]}}}""",
            """{"cashReserve":1,"legacyAmountMinor":{"unknownField":101}}"""
        )) assertThrows(IllegalArgumentException::class.java) { decodeFinanceBackup(raw) }
        assertEquals(-1L, decodeFinanceBackup("""{"cashReserve":-1}""").cashReserve)
        assertEquals(Long.MAX_VALUE, decodeFinanceBackup("""{"cashReserve":9223372036854775807}""").cashReserve)
    }

    @Test fun financeBackupRestorePreservesWireYuanAndExplicitCents() {
        val profile = FinanceProfile(
            dailyLedgers = mapOf("2026-10-02" to FinanceDayLedger(expenses = listOf(FinanceExpenseEntry(id = "cent", name = "会员", amount = 58, amountMinor = 5812)))),
            monthlySnapshots = mapOf("2026-10" to FinanceMonthSnapshot(assets = listOf(FinanceNamedAmountEntry(id = "cash", amount = 0, amountMinor = 1))))
        ).withLegacyAmount("cashReserve", 123L)
        val restored = decodeFinanceBackup(encodeFinanceBackup(profile, "test", 1L))
        assertEquals(58L, restored.dailyLedgers.getValue("2026-10-02").expenses.single().amount)
        assertEquals(5812L, restored.dailyLedgers.getValue("2026-10-02").expenses.single().preciseAmount())
        assertEquals(1L, restored.monthlySnapshots.getValue("2026-10").assets.single().preciseAmount())
        assertEquals(123L, restored.preciseLegacyAmount("cashReserve", restored.cashReserve))
    }
}
"####;
