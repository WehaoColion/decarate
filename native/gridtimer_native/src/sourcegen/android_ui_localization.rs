// v2.22.49.10 - Localize Android interface text from bundled language catalogs.
pub const KOTLIN_PATH: &str = "com/ofairyo/gridtimer/i18n/LocalizedText.kt";

pub const KOTLIN_CONTENTS: &str = r####"package com.ofairyo.gridtimer.i18n

import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text as MaterialText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import com.ofairyo.gridtimer.R
import org.json.JSONObject
import java.util.Collections
import java.util.IdentityHashMap
import java.util.Locale

private object UiTextCatalog {
    @Volatile private var loadedLanguage: String? = null
    @Volatile private var loadedStrings: Map<String, String> = emptyMap()
    @Volatile private var loadedLiteralReferences: Set<String> = emptySet()

    fun translate(context: Context, language: String, original: String, sourceLiteralOnly: Boolean): String {
        if (language == "zh" || original.isBlank()) return original
        if (loadedLanguage != language) {
            synchronized(this) {
                if (loadedLanguage != language) {
                    val (strings, literals) = load(context, language)
                    loadedStrings = strings
                    loadedLiteralReferences = literals
                    loadedLanguage = language
                }
            }
        }
        // A note or canvas title can equal a UI label. Translate only source-code
        // literals in the Text wrapper; keep text loaded from user data verbatim.
        if (sourceLiteralOnly && !loadedLiteralReferences.contains(original)) return original
        return loadedStrings[original] ?: original
    }

    private fun load(context: Context, language: String): Pair<Map<String, String>, Set<String>> {
        val resourceId = when (language) {
            "en" -> R.raw.ui_en
            "hi" -> R.raw.ui_hi
            "es" -> R.raw.ui_es
            "ar" -> R.raw.ui_ar
            "fr" -> R.raw.ui_fr
            "bn" -> R.raw.ui_bn
            "pt" -> R.raw.ui_pt
            "id", "in" -> R.raw.ui_id
            "ur" -> R.raw.ui_ur
            "ja" -> R.raw.ui_ja
            else -> return emptyMap<String, String>() to emptySet()
        }
        return runCatching<Pair<Map<String, String>, Set<String>>> {
            val source = context.resources.openRawResource(resourceId)
                .bufferedReader(Charsets.UTF_8).use { it.readText() }
            val json = JSONObject(source)
            val result = HashMap<String, String>(json.length())
            val literalReferences = Collections.newSetFromMap(IdentityHashMap<String, Boolean>())
            val keys = json.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                val value = json.optString(key)
                if (value.isNotBlank()) {
                    result[key] = value
                    literalReferences.add(key.intern())
                }
            }
            result to literalReferences
        }.getOrDefault(emptyMap<String, String>() to emptySet<String>())
    }
}

fun localizedToast(context: Context, message: CharSequence, duration: Int): Toast {
    val language = context.resources.configuration.locales[0]
        ?.language?.lowercase(Locale.ROOT)
        ?: Locale.getDefault().language.lowercase(Locale.ROOT)
    val localized = UiTextCatalog.translate(context, language, message.toString(), sourceLiteralOnly = true)
    return Toast.makeText(context, localized, duration)
}

fun localizedToast(context: Context, messageResource: Int, duration: Int): Toast =
    Toast.makeText(context, messageResource, duration)

@Composable
fun localizedUiText(text: String, sourceLiteralOnly: Boolean = false): String {
    val language = LocalConfiguration.current.locales[0]
        ?.language?.lowercase(Locale.ROOT)
        ?: Locale.getDefault().language.lowercase(Locale.ROOT)
    return UiTextCatalog.translate(LocalContext.current, language, text, sourceLiteralOnly)
}

@Composable
fun Text(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    fontStyle: FontStyle? = null,
    fontWeight: FontWeight? = null,
    fontFamily: FontFamily? = null,
    letterSpacing: TextUnit = TextUnit.Unspecified,
    textDecoration: TextDecoration? = null,
    textAlign: TextAlign? = null,
    lineHeight: TextUnit = TextUnit.Unspecified,
    overflow: TextOverflow = TextOverflow.Clip,
    softWrap: Boolean = true,
    maxLines: Int = Int.MAX_VALUE,
    minLines: Int = 1,
    onTextLayout: (TextLayoutResult) -> Unit = {},
    style: TextStyle = LocalTextStyle.current
) {
    MaterialText(
        text = localizedUiText(text, sourceLiteralOnly = true), modifier = modifier, color = color, fontSize = fontSize,
        fontStyle = fontStyle, fontWeight = fontWeight, fontFamily = fontFamily,
        letterSpacing = letterSpacing, textDecoration = textDecoration, textAlign = textAlign,
        lineHeight = lineHeight, overflow = overflow, softWrap = softWrap, maxLines = maxLines,
        minLines = minLines, onTextLayout = onTextLayout, style = style
    )
}

@Composable
fun Text(
    text: AnnotatedString,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    fontStyle: FontStyle? = null,
    fontWeight: FontWeight? = null,
    fontFamily: FontFamily? = null,
    letterSpacing: TextUnit = TextUnit.Unspecified,
    textDecoration: TextDecoration? = null,
    textAlign: TextAlign? = null,
    lineHeight: TextUnit = TextUnit.Unspecified,
    overflow: TextOverflow = TextOverflow.Clip,
    softWrap: Boolean = true,
    maxLines: Int = Int.MAX_VALUE,
    minLines: Int = 1,
    inlineContent: Map<String, InlineTextContent> = mapOf(),
    onTextLayout: (TextLayoutResult) -> Unit = {},
    style: TextStyle = LocalTextStyle.current
) {
    MaterialText(
        text = text, modifier = modifier, color = color, fontSize = fontSize,
        fontStyle = fontStyle, fontWeight = fontWeight, fontFamily = fontFamily,
        letterSpacing = letterSpacing, textDecoration = textDecoration, textAlign = textAlign,
        lineHeight = lineHeight, overflow = overflow, softWrap = softWrap, maxLines = maxLines,
        minLines = minLines, inlineContent = inlineContent, onTextLayout = onTextLayout,
        style = style
    )
}
"####;

pub const RESOURCES: &[(&str, &str)] = &[
    ("res/raw/ui_en.json", include_str!("ui_locales/ui_en.json")),
    ("res/raw/ui_hi.json", include_str!("ui_locales/ui_hi.json")),
    ("res/raw/ui_es.json", include_str!("ui_locales/ui_es.json")),
    ("res/raw/ui_ar.json", include_str!("ui_locales/ui_ar.json")),
    ("res/raw/ui_fr.json", include_str!("ui_locales/ui_fr.json")),
    ("res/raw/ui_bn.json", include_str!("ui_locales/ui_bn.json")),
    ("res/raw/ui_pt.json", include_str!("ui_locales/ui_pt.json")),
    ("res/raw/ui_id.json", include_str!("ui_locales/ui_id.json")),
    ("res/raw/ui_ur.json", include_str!("ui_locales/ui_ur.json")),
    ("res/raw/ui_ja.json", include_str!("ui_locales/ui_ja.json")),
];

pub fn render(path: &str, source: &str) -> String {
    let should_localize = matches!(
        path,
        "com/ofairyo/gridtimer/ui/GridTimerScreen.kt"
            | "com/ofairyo/gridtimer/ui/FinanceLedgerSection.kt"
            | "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt"
            | "com/ofairyo/gridtimer/ui/NoteMediaUi.kt"
            | "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt"
            | "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt"
            | "com/ofairyo/gridtimer/ui/DiagnosticsExportUi.kt"
            | "com/ofairyo/gridtimer/ui/KnowledgeCanvasScreen.kt"
            | "com/ofairyo/gridtimer/ui/StructuredKnowledgeReader.kt"
    );
    if !should_localize {
        return source.to_owned();
    }
    let text_import = "import androidx.compose.material3.Text\n";
    let localized_import = "import com.ofairyo.gridtimer.i18n.Text\n";
    let localized = if source.contains(text_import) {
        source.replacen(text_import, localized_import, 1)
    } else {
        source.replacen(
            "package com.ofairyo.gridtimer.ui\n",
            "package com.ofairyo.gridtimer.ui\n\nimport com.ofairyo.gridtimer.i18n.Text\n",
            1,
        )
    };
    let localized = if localized.contains("Toast.makeText(") {
        localized
            .replacen(
                localized_import,
                "import com.ofairyo.gridtimer.i18n.Text\nimport com.ofairyo.gridtimer.i18n.localizedToast\n",
                1,
            )
            .replace("Toast.makeText(", "localizedToast(")
    } else {
        localized
    };
    if path == "com/ofairyo/gridtimer/ui/KnowledgeCanvasScreen.kt" {
        return localized
            .replacen(
                localized_import,
                "import com.ofairyo.gridtimer.i18n.Text\nimport com.ofairyo.gridtimer.i18n.localizedUiText\n",
                1,
            )
            .replace(
                "\"全部画布（${boards.size}）\"",
                "\"${localizedUiText(\"全部画布\")} (${boards.size})\"",
            )
            .replace(
                "\"卡片 ${canvas?.nodes?.size ?: 0}\"",
                "\"${localizedUiText(\"卡片\")} ${canvas?.nodes?.size ?: 0}\"",
            );
    }
    if path == "com/ofairyo/gridtimer/ui/StructuredKnowledgeReader.kt" {
        return localized
            .replacen(
                localized_import,
                "import com.ofairyo.gridtimer.i18n.Text\nimport com.ofairyo.gridtimer.i18n.localizedUiText\n",
                1,
            )
            .replace(
                "\"${file.sizeBytes} 字节\"",
                "\"${file.sizeBytes} ${localizedUiText(\"字节\")}\"",
            )
            .replace(
                "\"已解决 · \"",
                "\"${localizedUiText(\"已解决\")} · \"",
            );
    }
    if path == "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt" {
        return localized
            .replacen(
                localized_import,
                "import com.ofairyo.gridtimer.i18n.Text\nimport com.ofairyo.gridtimer.i18n.localizedUiText\n",
                1,
            )
            .replace(
                "\"创建于 ${safeFormat(dateTimeFormatter, version?.createdAtEpochMillis ?: 0L, \"--\")}\"",
                "\"${localizedUiText(\"创建于\")} ${safeFormat(dateTimeFormatter, version?.createdAtEpochMillis ?: 0L, \"--\")}\"",
            );
    }
    if path == "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" {
        let localized = localized
            .replacen(
                localized_import,
                "import com.ofairyo.gridtimer.i18n.Text\nimport com.ofairyo.gridtimer.i18n.localizedUiText\n",
                1,
            )
            .replace(
                "text = \"格子 ${displayIndex.toString().padStart(2, '0')}\"",
                "text = \"${localizedUiText(\"格子\")} ${displayIndex.toString().padStart(2, '0')}\"",
            )
            .replace(
                "text = \"格子 ${slot.id.toString().padStart(2, '0')} 快捷操作\"",
                "text = \"${localizedUiText(\"格子\")} ${slot.id.toString().padStart(2, '0')} ${localizedUiText(\"快捷操作\")}\"",
            );
        let start = localized.find("private fun DockNavigationButton(");
        let end = localized.find("private fun HeaderBlock(");
        if let (Some(start), Some(end)) = (start, end) {
            let mut button = localized[start..end].to_owned();
            button = button.replacen(".size(28.dp)", ".size(24.dp)", 1);
            button = button.replacen("size = 25.dp", "size = 23.dp", 1);
            button = button.replacen(
                "Spacer(modifier = Modifier.height(2.dp))",
                "Spacer(modifier = Modifier.height(1.dp))",
                1,
            );
            button = button.replacen(
                "maxLines = 1,\n                softWrap = false,",
                "maxLines = 2,\n                softWrap = true,",
                1,
            );
            button = button.replacen(
                "style = MaterialTheme.typography.labelMedium.copy(\n                    color = contentColor,\n                    fontWeight = if (selected) FontWeight.Bold else FontWeight.SemiBold,\n                    lineHeight = 14.sp",
                "style = MaterialTheme.typography.labelSmall.copy(\n                    color = contentColor,\n                    fontWeight = if (selected) FontWeight.Bold else FontWeight.SemiBold,\n                    fontSize = when {\n                        label.length > 10 -> 10.sp\n                        label.length > 6 -> 11.sp\n                        else -> 12.sp\n                    },\n                    lineHeight = 12.sp",
                1,
            );
            return format!("{}{}{}", &localized[..start], button, &localized[end..]);
        }
        return localized;
    }
    localized
}
