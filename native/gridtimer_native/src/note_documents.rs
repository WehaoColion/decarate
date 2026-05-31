#[derive(Clone, Debug, PartialEq)]
pub struct NoteDocumentTextInput {
    pub rich_text_enabled: bool,
    pub rich_text_plain_text: String,
    pub blocks: Vec<NoteBlockTextInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoteBlockTextInput {
    pub type_code: i32,
    pub text: String,
    pub caption: String,
    pub contact_name: String,
    pub contact_organization: String,
    pub first_contact_phone_number: String,
    pub contact_phone_search_text: String,
    pub call_contact_name: String,
    pub call_phone_number: String,
    pub call_direction_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoteDocumentTextDigest {
    pub suggested_title: Option<String>,
    pub suggested_preview: Option<String>,
    pub plain_text: String,
    pub searchable_text: String,
}

const BLOCK_TYPE_TEXT: i32 = 0;
const BLOCK_TYPE_IMAGE: i32 = 1;
const BLOCK_TYPE_CONTACT: i32 = 2;
const BLOCK_TYPE_CALL: i32 = 3;
const HEADING_PREFIX: &str = "# ";
const LIST_PREFIX: &str = "- ";
const QUOTE_PREFIX: &str = "> ";
const TODO_PREFIX: &str = "- [ ] ";
const DONE_PREFIX: &str = "- [x] ";
const LEGACY_CENTER_PREFIX: &str = "【居中】";

#[cfg(test)]
impl NoteBlockTextInput {
    pub fn empty_with_type(type_code: i32) -> Self {
        Self {
            type_code,
            text: String::new(),
            caption: String::new(),
            contact_name: String::new(),
            contact_organization: String::new(),
            first_contact_phone_number: String::new(),
            contact_phone_search_text: String::new(),
            call_contact_name: String::new(),
            call_phone_number: String::new(),
            call_direction_name: String::new(),
        }
    }
}

pub fn build_note_document_text_digest(
    input: &NoteDocumentTextInput,
    preserve_structure: bool,
    skip_leading_title_line: bool,
    excluded_text: &str,
) -> NoteDocumentTextDigest {
    let excluded = non_empty_trimmed(excluded_text);
    let suggested_title = input
        .text_preview_lines(false, false)
        .into_iter()
        .next()
        .or_else(|| input.non_text_summaries(false).into_iter().next());
    let suggested_preview = input
        .text_preview_lines(true, skip_leading_title_line)
        .into_iter()
        .find(|line| excluded.map(|value| value != line.as_str()).unwrap_or(true))
        .or_else(|| {
            input
                .non_text_summaries(true)
                .into_iter()
                .find(|line| excluded.map(|value| value != line.as_str()).unwrap_or(true))
        });
    let plain_text = input.plain_text(preserve_structure);
    let searchable_text = input.searchable_text();
    NoteDocumentTextDigest {
        suggested_title,
        suggested_preview,
        plain_text,
        searchable_text,
    }
}

impl NoteDocumentTextInput {
    fn rich_text_plain_text_or_null(&self) -> Option<String> {
        if !self.rich_text_enabled {
            return None;
        }
        let normalized = normalize_newlines(&self.rich_text_plain_text);
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn text_preview_lines(
        &self,
        preserve_structure: bool,
        skip_leading_title_line: bool,
    ) -> Vec<String> {
        if let Some(rich_text) = self.rich_text_plain_text_or_null() {
            return rich_text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }

        let mut skipped_leading = !skip_leading_title_line;
        let mut lines = Vec::new();
        for block in self
            .blocks
            .iter()
            .filter(|block| block.type_code == BLOCK_TYPE_TEXT)
        {
            for raw_line in normalize_newlines(&block.text).lines() {
                if !skipped_leading && !display_note_line(raw_line, false).trim().is_empty() {
                    skipped_leading = true;
                    continue;
                }
                let display = display_note_line(raw_line, preserve_structure)
                    .trim()
                    .to_string();
                if !display.is_empty() {
                    lines.push(display);
                }
            }
        }
        lines
    }

    fn non_text_summaries(&self, preserve_structure: bool) -> Vec<String> {
        self.blocks
            .iter()
            .filter(|block| block.type_code != BLOCK_TYPE_TEXT)
            .filter_map(|block| block.summary_text(preserve_structure))
            .map(|summary| summary.trim().to_string())
            .filter(|summary| !summary.is_empty())
            .collect()
    }

    fn plain_text(&self, preserve_structure: bool) -> String {
        let text_sections = if let Some(rich_text) = self.rich_text_plain_text_or_null() {
            let section = rich_text
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
            if section.is_empty() {
                Vec::new()
            } else {
                vec![section]
            }
        } else {
            self.blocks
                .iter()
                .filter(|block| block.type_code == BLOCK_TYPE_TEXT)
                .filter_map(|block| block.summary_text(preserve_structure))
                .map(|section| section.trim().to_string())
                .filter(|section| !section.is_empty())
                .collect()
        };
        let mut sections = text_sections;
        sections.extend(self.non_text_summaries(preserve_structure));
        collapse_triple_newlines(&sections.join("\n\n"))
            .trim()
            .to_string()
    }

    fn searchable_text(&self) -> String {
        let mut output = String::new();
        if let Some(rich_text) = self.rich_text_plain_text_or_null() {
            append_line(&mut output, &rich_text);
        }
        for block in &self.blocks {
            append_line(&mut output, &block.text);
            append_line(&mut output, &block.caption);
            append_line(&mut output, &block.contact_name);
            append_line(&mut output, &block.contact_organization);
            if !block.contact_phone_search_text.is_empty() {
                for phone_line in block.contact_phone_search_text.lines() {
                    append_line(&mut output, phone_line);
                }
            }
            append_line(&mut output, &block.call_contact_name);
            append_line(&mut output, &block.call_phone_number);
            append_line(&mut output, &block.call_direction_name);
        }
        output.trim().to_string()
    }
}

impl NoteBlockTextInput {
    fn summary_text(&self, preserve_structure: bool) -> Option<String> {
        match self.type_code {
            BLOCK_TYPE_TEXT => Some(
                normalize_newlines(&self.text)
                    .lines()
                    .map(|line| display_note_line(line, preserve_structure))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string(),
            ),
            BLOCK_TYPE_IMAGE => {
                let caption = self.caption.trim();
                if caption.is_empty() {
                    if preserve_structure {
                        Some("图片".to_string())
                    } else {
                        Some(String::new())
                    }
                } else if preserve_structure {
                    Some(format!("图片 {caption}"))
                } else {
                    Some(caption.to_string())
                }
            }
            BLOCK_TYPE_CONTACT => {
                let mut parts = Vec::new();
                parts.push("联系人".to_string());
                parts.push(
                    non_empty_trimmed(&self.contact_name)
                        .unwrap_or("未命名联系人")
                        .to_string(),
                );
                push_trimmed(&mut parts, &self.first_contact_phone_number);
                push_trimmed(&mut parts, &self.contact_organization);
                push_trimmed(&mut parts, &self.caption);
                Some(parts.join(" ").trim().to_string())
            }
            BLOCK_TYPE_CALL => {
                let target = non_empty_trimmed(&self.call_contact_name)
                    .or_else(|| non_empty_trimmed(&self.call_phone_number))
                    .unwrap_or("未记录号码");
                let mut parts = vec!["通话速记".to_string(), target.to_string()];
                push_trimmed(&mut parts, &self.text);
                Some(parts.join(" ").trim().to_string())
            }
            _ => None,
        }
    }
}

fn display_note_line(line: &str, preserve_structure: bool) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let plain_bold = trimmed.replace("**", "").replace("__", "");
    if let Some(centered) = centered_payload(&plain_bold) {
        return centered.to_string();
    }
    if let Some(payload) = plain_bold.strip_prefix(TODO_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("☐ {payload}")
        } else {
            payload.to_string()
        };
    }
    if plain_bold
        .get(..DONE_PREFIX.len())
        .map(|prefix| prefix.eq_ignore_ascii_case(DONE_PREFIX))
        .unwrap_or(false)
    {
        let payload = plain_bold[DONE_PREFIX.len()..].trim_start();
        return if preserve_structure {
            format!("☑ {payload}")
        } else {
            payload.to_string()
        };
    }
    if let Some(payload) = plain_bold.strip_prefix(HEADING_PREFIX) {
        return payload.trim_start().to_string();
    }
    if let Some(payload) = plain_bold.strip_prefix(QUOTE_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("“{payload}”")
        } else {
            payload.to_string()
        };
    }
    if let Some(payload) = plain_bold.strip_prefix(LIST_PREFIX) {
        let payload = payload.trim_start();
        return if preserve_structure {
            format!("• {payload}")
        } else {
            payload.to_string()
        };
    }
    plain_bold
}

fn centered_payload(value: &str) -> Option<&str> {
    if let Some(payload) = value.strip_prefix(LEGACY_CENTER_PREFIX) {
        let trimmed = payload.trim_start();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if value.starts_with('[') && value.ends_with(']') && value.len() > 2 {
        let payload = value[1..value.len() - 1].trim();
        if !payload.is_empty() {
            return Some(payload);
        }
    }
    None
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn push_trimmed(parts: &mut Vec<String>, value: &str) {
    if let Some(trimmed) = non_empty_trimmed(value) {
        parts.push(trimmed.to_string());
    }
}

fn append_line(output: &mut String, value: &str) {
    output.push_str(value);
    output.push('\n');
}

fn collapse_triple_newlines(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut newline_count = 0usize;
    for ch in value.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                output.push(ch);
            }
        } else {
            newline_count = 0;
            output.push(ch);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        build_note_document_text_digest, display_note_line, NoteBlockTextInput,
        NoteDocumentTextInput, BLOCK_TYPE_CALL, BLOCK_TYPE_CONTACT, BLOCK_TYPE_IMAGE,
        BLOCK_TYPE_TEXT,
    };

    #[test]
    fn display_note_line_matches_kotlin_markers() {
        assert_eq!("Title", display_note_line("# Title", false));
        assert_eq!("☐ Task", display_note_line("- [ ] Task", true));
        assert_eq!("☑ Done", display_note_line("- [x] Done", true));
        assert_eq!("“Quote”", display_note_line("> Quote", true));
        assert_eq!("• Item", display_note_line("- Item", true));
        assert_eq!("Centered", display_note_line("[**Centered**]", true));
        assert_eq!("☐ 第一件事", display_note_line("☐ 第一件事", true));
        assert_eq!("💡 先记下来", display_note_line("💡 先记下来", true));
    }

    #[test]
    fn document_digest_derives_title_preview_plain_text_and_search_text() {
        let input = NoteDocumentTextInput {
            rich_text_enabled: false,
            rich_text_plain_text: String::new(),
            blocks: vec![
                NoteBlockTextInput {
                    type_code: BLOCK_TYPE_TEXT,
                    text: "# Release plan\n**Focus**\n- [ ] Update apk".to_string(),
                    ..NoteBlockTextInput::empty_with_type(BLOCK_TYPE_TEXT)
                },
                NoteBlockTextInput {
                    type_code: BLOCK_TYPE_IMAGE,
                    caption: "screen".to_string(),
                    ..NoteBlockTextInput::empty_with_type(BLOCK_TYPE_IMAGE)
                },
                NoteBlockTextInput {
                    type_code: BLOCK_TYPE_CONTACT,
                    caption: "review".to_string(),
                    contact_name: "Alice".to_string(),
                    contact_organization: "Ops".to_string(),
                    first_contact_phone_number: "123".to_string(),
                    contact_phone_search_text: "work 123".to_string(),
                    ..NoteBlockTextInput::empty_with_type(BLOCK_TYPE_CONTACT)
                },
                NoteBlockTextInput {
                    type_code: BLOCK_TYPE_CALL,
                    text: "follow up".to_string(),
                    call_phone_number: "456".to_string(),
                    call_direction_name: "OUTGOING".to_string(),
                    ..NoteBlockTextInput::empty_with_type(BLOCK_TYPE_CALL)
                },
            ],
        };

        let digest = build_note_document_text_digest(&input, true, true, "Focus");

        assert_eq!(Some("Release plan".to_string()), digest.suggested_title);
        assert_eq!(Some("☐ Update apk".to_string()), digest.suggested_preview);
        assert!(digest
            .plain_text
            .contains("Release plan\nFocus\n☐ Update apk"));
        assert!(digest.plain_text.contains("图片 screen"));
        assert!(digest.searchable_text.contains("work 123"));
        assert!(digest.searchable_text.contains("OUTGOING"));
    }

    #[test]
    fn rich_text_preview_keeps_existing_skip_behavior() {
        let input = NoteDocumentTextInput {
            rich_text_enabled: true,
            rich_text_plain_text: "\r\nTitle\nBody\n".to_string(),
            blocks: vec![NoteBlockTextInput {
                type_code: BLOCK_TYPE_TEXT,
                text: "Ignored".to_string(),
                ..NoteBlockTextInput::empty_with_type(BLOCK_TYPE_TEXT)
            }],
        };

        let digest = build_note_document_text_digest(&input, true, true, "");

        assert_eq!(Some("Title".to_string()), digest.suggested_title);
        assert_eq!(Some("Title".to_string()), digest.suggested_preview);
        assert_eq!("Title\nBody", digest.plain_text);
    }
}
