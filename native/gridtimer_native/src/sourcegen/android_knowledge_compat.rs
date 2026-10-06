// v2.22.47 - Read advanced Windows pages without passing them through a flat editor.
// Mobile navigation is presentation-only; preserve the structured-data write boundary.
#[path = "android_structured_reader.rs"]
mod mobile_reader;

pub const PATH: &str = "com/ofairyo/gridtimer/ui/StructuredKnowledgeReader.kt";
pub const CONTENTS: &str = mobile_reader::CONTENTS;

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path.ends_with("/NoteEditorUi.kt") || source.contains("internal fun NoteEditorContent(") {
        let needle = ") {\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }";
        if source.matches(needle).count() != 1 {
            return Err("structured document reader insertion point missing or duplicated".into());
        }
        return Ok(source.replacen(needle, ") {\n    if (note.hasStructuredKnowledge()) {\n        EncryptedNoteSecureWindowEffect(note.encryption != null)\n        StructuredKnowledgeReader(note, onBack, onRequestPreviewAttachment)\n        return\n    }\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }", 1));
    }
    if source.contains("internal fun SmartisanNoteEditorContent(") {
        let needle = ") {\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current";
        if source.matches(needle).count() != 1 {
            return Err("structured sticky reader insertion point missing or duplicated".into());
        }
        return Ok(source.replacen(needle, ") {\n    if (note.hasStructuredKnowledge()) {\n        EncryptedNoteSecureWindowEffect(note.encryption != null)\n        StructuredKnowledgeReader(note, onBack) { id -> onRequestPreviewAttachment(null, id) }\n        return\n    }\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current", 1));
    }
    Ok(source.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOCUMENT: &str = "internal fun NoteEditorContent(note: NoteEntry) {\n    val context = LocalContext.current\n    val focusManager = LocalFocusManager.current\n    val titleFocusRequester = remember { FocusRequester() }\n}";
    const STICKY: &str = "internal fun SmartisanNoteEditorContent(note: NoteEntry) {\n    val context = androidx.compose.ui.platform.LocalContext.current\n    val workspaceKey = LocalNoteMediaWorkspaceKey.current\n}";

    #[test]
    fn ordinary_sources_are_untouched() {
        assert_eq!(render("Other.kt", "plain text").unwrap(), "plain text");
    }
    #[test]
    fn document_route_retains_secure_window_and_existing_callbacks() {
        let output = render("NoteDocumentEditor.kt", DOCUMENT).unwrap();
        assert!(output.contains("if (note.hasStructuredKnowledge())"));
        assert!(output.contains("EncryptedNoteSecureWindowEffect(note.encryption != null)"));
        assert!(output.contains("StructuredKnowledgeReader(note, onBack, onRequestPreviewAttachment)"));
    }
    #[test]
    fn sticky_route_retains_attachment_adapter() {
        let output = render("Sticky.kt", STICKY).unwrap();
        assert!(output.contains("onRequestPreviewAttachment(null, id)"));
    }
    #[test]
    fn missing_duplicate_and_second_application_are_rejected() {
        assert!(render("NoteEditorUi.kt", "no anchor").is_err());
        assert!(render("NoteEditorUi.kt", &format!("{DOCUMENT}{DOCUMENT}")).is_err());
        assert!(render("Sticky.kt", &format!("{STICKY}{STICKY}")).is_err());
        assert!(render("NoteEditorUi.kt", &render("NoteEditorUi.kt", DOCUMENT).unwrap()).is_err());
        assert!(render("Sticky.kt", &render("Sticky.kt", STICKY).unwrap()).is_err());
    }
    #[test]
    fn generated_reader_remains_presentation_only() {
        for forbidden in ["upsertNote(", "upsertNoteAndFlush", "deleteNote(", "runLegalScan(", "rememberSaveable"] {
            assert!(!CONTENTS.contains(forbidden), "unexpected side effect: {forbidden}");
        }
        assert!(CONTENTS.contains("if (note.isEncryptionLocked())"));
        assert!(CONTENTS.contains("key(note.id, workspace, document)"));
        assert!(CONTENTS.contains("onCheckedChange = null"));
    }
}
