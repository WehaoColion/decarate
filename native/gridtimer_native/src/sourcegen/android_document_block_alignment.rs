//! Center the document text block while keeping caret requests in text-local coordinates.
//! Run after the caret/Markdown transforms and reject changed or duplicated editor anchors.

const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt";
const CANVAS_START: &str = "private fun DocumentCanvasBlock(";
const CANVAS_END: &str = "private fun DocumentInlineField(";
const TEXT_START: &str = "                    NoteBlockType.TEXT -> {";
const TEXT_END: &str = "                    NoteBlockType.IMAGE -> {";

const BEFORE_BOX: &str = r#"                            decorationBox = { innerField ->
                                Box(modifier = Modifier.fillMaxWidth().bringIntoViewRequester(caretVisibility.requester)) {
                                    if (textFieldState.text.isBlank()) {"#;
const AFTER_BOX: &str = r#"                            decorationBox = { innerField ->
                                Box(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .heightIn(min = 52.dp),
                                    contentAlignment = Alignment.CenterStart
                                ) {
                                    if (textFieldState.text.isBlank()) {"#;
const BEFORE_INNER: &str = r#"                                    innerField()
                                }
                            }"#;
const AFTER_INNER: &str = r#"                                    Box(modifier = Modifier.bringIntoViewRequester(caretVisibility.requester)) {
                                        innerField()
                                    }
                                }
                            }"#;

fn anchor_offset(source: &str, anchor: &str) -> Result<usize, String> {
    let count = source.matches(anchor).count();
    if count != 1 {
        return Err(format!(
            "document block alignment anchor count {count}: {anchor}"
        ));
    }
    Ok(source.find(anchor).expect("one matching anchor"))
}

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    anchor_offset(source, before)?;
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }

    let canvas_start = anchor_offset(source, CANVAS_START)?;
    let canvas_end = anchor_offset(source, CANVAS_END)?;
    if canvas_start >= canvas_end {
        return Err("document block alignment canvas boundaries are reversed".to_owned());
    }
    let canvas = &source[canvas_start..canvas_end];
    let text_start = anchor_offset(canvas, TEXT_START)?;
    let text_end = anchor_offset(canvas, TEXT_END)?;
    if text_start >= text_end {
        return Err("document block alignment text boundaries are reversed".to_owned());
    }

    let mut text_branch = canvas[text_start..text_end].to_owned();
    replace_once(&mut text_branch, BEFORE_BOX, AFTER_BOX)?;
    // This wrapper has the input node's size and origin, including RTL placement.
    // The outer minimum-height box may center it without shifting caret-local rectangles.
    replace_once(&mut text_branch, BEFORE_INNER, AFTER_INNER)?;
    let mut source = source.to_owned();
    source.replace_range(
        canvas_start + text_start..canvas_start + text_end,
        &text_branch,
    );
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_after_caret() -> String {
        let editor = super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == SCREEN)
            .expect("actual document editor template")
            .contents;
        super::super::android_document_caret::render(SCREEN, editor).expect("caret transform")
    }

    #[test]
    fn missing_required_anchors_reject_generation() {
        let editor = editor_after_caret();
        for anchor in [
            CANVAS_START,
            CANVAS_END,
            TEXT_START,
            TEXT_END,
            BEFORE_BOX,
            BEFORE_INNER,
        ] {
            let changed = editor.replacen(anchor, "changed editor anchor", 1);
            assert!(
                render(SCREEN, &changed).is_err(),
                "accepted missing {anchor}"
            );
        }
    }

    #[test]
    fn duplicate_exact_anchors_reject_generation() {
        let editor = editor_after_caret();
        for anchor in [
            CANVAS_START,
            CANVAS_END,
            TEXT_START,
            TEXT_END,
            BEFORE_BOX,
            BEFORE_INNER,
        ] {
            let duplicated = editor.replacen(anchor, &format!("{anchor}\n{anchor}"), 1);
            assert!(
                render(SCREEN, &duplicated).is_err(),
                "accepted duplicate {anchor}"
            );
        }
    }

    #[test]
    fn second_application_rejects_generation() {
        let rendered = render(SCREEN, &editor_after_caret()).expect("first application");
        assert!(render(SCREEN, &rendered).is_err());
    }

    #[test]
    fn unrelated_files_are_passed_through_without_editor_validation() {
        let unrelated = "unrelated source without document-editor anchors";
        assert_eq!(render("other.kt", unrelated).unwrap(), unrelated);
    }
}
