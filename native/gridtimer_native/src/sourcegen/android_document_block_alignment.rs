//! Vertically center text inside the document editor's existing minimum-height text block.
//! This transform runs after caret/Markdown transforms so it preserves their hooks.

const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    let count = source.matches(before).count();
    if count != 1 {
        return Err(format!(
            "document block alignment anchor count {count}: {before}"
        ));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }

    let mut source = source.to_owned();
    replace_once(
        &mut source,
        r#"                                Box(modifier = Modifier.fillMaxWidth().bringIntoViewRequester(caretVisibility.requester)) {
                                    if (textFieldState.text.isBlank()) {"#,
        r#"                                Box(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .heightIn(min = 52.dp)
                                        .bringIntoViewRequester(caretVisibility.requester),
                                    contentAlignment = Alignment.CenterStart
                                ) {
                                    if (textFieldState.text.isBlank()) {"#,
    )?;

    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_editor() -> &'static str {
        super::super::kotlin_sources::SOURCES
            .iter()
            .find(|source| source.path == SCREEN)
            .expect("actual document editor template")
            .contents
    }

    fn editor_after_caret() -> String {
        super::super::android_document_caret::render(SCREEN, base_editor())
            .expect("caret transform")
    }

    #[test]
    fn text_field_content_is_vertically_centered_without_growing_normal_blocks() {
        let rendered = render(SCREEN, &editor_after_caret()).expect("alignment transform");
        assert!(rendered.contains("contentAlignment = Alignment.CenterStart"));
        assert!(rendered.contains(".heightIn(min = 52.dp)"));
        assert!(rendered.contains(".bringIntoViewRequester(caretVisibility.requester)"));
        assert!(rendered.contains("onTextLayout = { caretVisibility.layout = it },"));
    }

    #[test]
    fn alignment_is_limited_to_the_text_decoration_box() {
        let rendered = render(SCREEN, &editor_after_caret()).expect("alignment transform");
        let text_branch = rendered
            .split("NoteBlockType.TEXT -> {")
            .nth(1)
            .expect("text branch")
            .split("NoteBlockType.IMAGE -> {")
            .next()
            .expect("text branch end");
        assert_eq!(text_branch.matches("contentAlignment = Alignment.CenterStart").count(), 1);
        assert!(text_branch.contains("textFieldState.text.isBlank()"));
        assert!(rendered.contains(".padding(top = 2.dp)"));
        assert!(rendered.contains(".size(width = 34.dp, height = 44.dp)"));
    }

    #[test]
    fn multiline_input_and_existing_minimum_height_contract_remain_intact() {
        let rendered = render(SCREEN, &editor_after_caret()).expect("alignment transform");
        assert!(rendered.contains(".heightIn(min = 52.dp)"));
        assert!(rendered.contains("value = textFieldState,"));
        assert!(rendered.contains("onValueChange = onTextChanged,"));
        assert!(rendered.contains("textStyle = fieldStyle.copy(color = MaterialTheme.colorScheme.onSurface)"));
    }

    #[test]
    fn drift_duplicate_or_second_application_fails_loudly() {
        let caret = editor_after_caret();
        let missing = caret.replace(
            "Box(modifier = Modifier.fillMaxWidth().bringIntoViewRequester(caretVisibility.requester))",
            "Box(modifier = Modifier.fillMaxWidth())",
        );
        assert!(render(SCREEN, &missing).is_err());

        let duplicated = format!(
            "{caret}\nBox(modifier = Modifier.fillMaxWidth().bringIntoViewRequester(caretVisibility.requester)) {{\n    if (textFieldState.text.isBlank()) {{"
        );
        assert!(render(SCREEN, &duplicated).is_err());

        let once = render(SCREEN, &caret).expect("first application");
        assert!(render(SCREEN, &once).is_err());
    }

    #[test]
    fn unrelated_files_are_unchanged() {
        assert_eq!(render("other.kt", "hello").unwrap(), "hello");
    }
}
