//! Canonical allow-list sanitizer for rich note markup.
//!
//! Rich note HTML is synced and imported across trust boundaries.  This
//! module deliberately accepts only the small vocabulary emitted by the note
//! editor and reserializes it before the markup reaches a WebView or export.

use serde_json::Value;

const MAX_ATTACHMENT_ID_LENGTH: usize = 128;
const MAX_ALT_TEXT_LENGTH: usize = 512;
// The editor bridge accepts at most 1,000,000 UTF-16 code units. Four million
// UTF-8 bytes safely cover that contract while still bounding native work.
const MAX_RICH_TEXT_INPUT_BYTES: usize = 4_000_000;
const MAX_RICH_TEXT_NESTING_DEPTH: usize = 128;
const MAX_RICH_TEXT_ELEMENTS: usize = 20_000;
const MAX_TAG_BYTES: usize = 8 * 1024;
const MAX_ATTRIBUTES_PER_TAG: usize = 32;
const OVERSIZED_RICH_TEXT_PLACEHOLDER: &str =
    "<p data-gridtimer-security-blocked=\"oversized\">富文本内容超过安全上限，当前未加载。</p>";

#[derive(Debug)]
struct ParsedTag {
    closing: bool,
    name: String,
    attributes: Vec<(String, String)>,
    self_closing: bool,
}

pub(crate) fn sanitize_rich_text_html(input: &str) -> String {
    if input.len() > MAX_RICH_TEXT_INPUT_BYTES {
        return OVERSIZED_RICH_TEXT_PLACEHOLDER.to_string();
    }
    if input.trim().is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(input.len());
    let mut open_tags = Vec::<String>::new();
    let mut emitted_elements = 0usize;
    let mut cursor = 0usize;
    let mut blocked_until_close: Option<String> = None;

    while cursor < input.len() {
        let Some(relative_open) = input[cursor..].find('<') else {
            if blocked_until_close.is_none() {
                output.push_str(&input[cursor..]);
            }
            break;
        };
        let open = cursor + relative_open;
        if blocked_until_close.is_none() {
            output.push_str(&input[cursor..open]);
        }

        if input[open..].starts_with("<!--") {
            cursor = input[open + 4..]
                .find("-->")
                .map(|offset| open + 4 + offset + 3)
                .unwrap_or(input.len());
            continue;
        }

        let Some(close) = find_tag_end(input, open + 1) else {
            if blocked_until_close.is_none() {
                push_text_with_escaped_less_than(&mut output, &input[open..]);
            }
            break;
        };
        let raw_tag = &input[open + 1..close];
        cursor = close + 1;
        if raw_tag.len() > MAX_TAG_BYTES {
            continue;
        }
        let Some(parsed) = parse_tag(raw_tag) else {
            continue;
        };

        if let Some(blocked_name) = blocked_until_close.as_deref() {
            if parsed.closing && parsed.name == blocked_name {
                blocked_until_close = None;
            }
            continue;
        }

        if is_blocked_container(&parsed.name) {
            if !parsed.closing && !parsed.self_closing && !is_void_tag(&parsed.name) {
                blocked_until_close = Some(parsed.name);
            }
            continue;
        }

        let Some(canonical_name) = canonical_allowed_tag(&parsed.name) else {
            continue;
        };

        if parsed.closing {
            close_through_tag(&mut output, &mut open_tags, canonical_name);
            continue;
        }

        if canonical_name == "img" {
            if let Some(image_tag) = sanitize_image_tag(&parsed.attributes) {
                emitted_elements += 1;
                if emitted_elements > MAX_RICH_TEXT_ELEMENTS {
                    return OVERSIZED_RICH_TEXT_PLACEHOLDER.to_string();
                }
                output.push_str(&image_tag);
            }
            continue;
        }

        if canonical_name != "br"
            && !parsed.self_closing
            && open_tags.len() >= MAX_RICH_TEXT_NESTING_DEPTH
        {
            continue;
        }

        emitted_elements += 1;
        if emitted_elements > MAX_RICH_TEXT_ELEMENTS {
            return OVERSIZED_RICH_TEXT_PLACEHOLDER.to_string();
        }

        output.push('<');
        output.push_str(canonical_name);
        append_allowed_attributes(&mut output, canonical_name, &parsed.attributes);
        output.push('>');

        if canonical_name == "br" {
            continue;
        }
        if parsed.self_closing {
            output.push_str("</");
            output.push_str(canonical_name);
            output.push('>');
        } else {
            open_tags.push(canonical_name.to_string());
        }
    }

    while let Some(tag) = open_tags.pop() {
        output.push_str("</");
        output.push_str(&tag);
        output.push('>');
    }
    if output.len() > MAX_RICH_TEXT_INPUT_BYTES {
        OVERSIZED_RICH_TEXT_PLACEHOLDER.to_string()
    } else {
        output
    }
}

pub(crate) fn sanitize_rich_text_html_for_storage(input: &str) -> Option<String> {
    let sanitized = sanitize_rich_text_html(input);
    (sanitized != OVERSIZED_RICH_TEXT_PLACEHOLDER).then_some(sanitized)
}

pub(crate) fn sanitize_note_document_value(document: &mut Value) {
    let Some(object) = document.as_object_mut() else {
        return;
    };
    if object.get("richTextEnabled").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(blocks) = object.get_mut("blocks").and_then(Value::as_array_mut) else {
        return;
    };
    for block in blocks {
        let Some(block_object) = block.as_object_mut() else {
            continue;
        };
        if block_object.get("type").and_then(Value::as_str) != Some("TEXT") {
            continue;
        }
        let Some(text) = block_object.get_mut("text") else {
            continue;
        };
        if let Some(html) = text.as_str() {
            if let Some(sanitized) = sanitize_rich_text_html_for_storage(html) {
                *text = Value::String(sanitized);
            }
        }
    }
}

fn find_tag_end(input: &str, mut cursor: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut quote = None::<u8>;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' if quote.is_none() => quote = Some(bytes[cursor]),
            value if quote == Some(value) => quote = None,
            b'>' if quote.is_none() => return Some(cursor),
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn push_text_with_escaped_less_than(output: &mut String, value: &str) {
    for character in value.chars() {
        if character == '<' {
            output.push_str("&lt;");
        } else {
            output.push(character);
        }
    }
}

fn parse_tag(raw: &str) -> Option<ParsedTag> {
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;
    skip_ascii_whitespace(bytes, &mut cursor);
    if cursor >= bytes.len() || matches!(bytes[cursor], b'!' | b'?') {
        return None;
    }

    let closing = bytes[cursor] == b'/';
    if closing {
        cursor += 1;
        skip_ascii_whitespace(bytes, &mut cursor);
    }
    let name_start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
        cursor += 1;
    }
    if cursor == name_start {
        return None;
    }
    let name = raw[name_start..cursor].to_ascii_lowercase();
    if closing {
        return Some(ParsedTag {
            closing: true,
            name,
            attributes: Vec::new(),
            self_closing: false,
        });
    }

    let mut attributes = Vec::<(String, String)>::new();
    let mut self_closing = false;
    while cursor < bytes.len() {
        skip_ascii_whitespace(bytes, &mut cursor);
        if cursor >= bytes.len() {
            break;
        }
        if bytes[cursor] == b'/' {
            self_closing = true;
            cursor += 1;
            continue;
        }

        let attribute_start = cursor;
        while cursor < bytes.len() && is_attribute_name_byte(bytes[cursor]) {
            cursor += 1;
        }
        if cursor == attribute_start {
            cursor += 1;
            continue;
        }
        let attribute_name = raw[attribute_start..cursor].to_ascii_lowercase();
        skip_ascii_whitespace(bytes, &mut cursor);
        let mut value = String::new();
        if cursor < bytes.len() && bytes[cursor] == b'=' {
            cursor += 1;
            skip_ascii_whitespace(bytes, &mut cursor);
            if cursor < bytes.len() && matches!(bytes[cursor], b'\'' | b'"') {
                let quote = bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                value = raw[value_start..cursor].to_string();
                if cursor < bytes.len() {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'/'
                {
                    cursor += 1;
                }
                value = raw[value_start..cursor].to_string();
            }
        }
        attributes.push((attribute_name, value));
        if attributes.len() >= MAX_ATTRIBUTES_PER_TAG {
            break;
        }
    }

    Some(ParsedTag {
        closing: false,
        name,
        attributes,
        self_closing,
    })
}

fn skip_ascii_whitespace(bytes: &[u8], cursor: &mut usize) {
    while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
}

fn is_attribute_name_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b':')
}

fn canonical_allowed_tag(name: &str) -> Option<&'static str> {
    match name {
        "p" => Some("p"),
        "div" => Some("div"),
        "br" => Some("br"),
        "strong" | "b" => Some("strong"),
        "em" | "i" => Some("em"),
        "u" => Some("u"),
        "s" | "strike" | "del" => Some("s"),
        "ul" => Some("ul"),
        "ol" => Some("ol"),
        "li" => Some("li"),
        "blockquote" => Some("blockquote"),
        "h1" => Some("h1"),
        "h2" => Some("h2"),
        "h3" => Some("h3"),
        "figure" => Some("figure"),
        "figcaption" => Some("figcaption"),
        "img" => Some("img"),
        _ => None,
    }
}

fn is_blocked_container(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "iframe"
            | "frame"
            | "frameset"
            | "object"
            | "embed"
            | "applet"
            | "svg"
            | "math"
            | "template"
            | "noscript"
            | "form"
            | "input"
            | "button"
            | "textarea"
            | "select"
            | "option"
            | "video"
            | "audio"
            | "source"
            | "track"
            | "canvas"
            | "link"
            | "meta"
            | "base"
    )
}

fn is_void_tag(name: &str) -> bool {
    matches!(
        name,
        "input" | "embed" | "source" | "track" | "link" | "meta" | "base"
    )
}

fn close_through_tag(output: &mut String, open_tags: &mut Vec<String>, closing: &str) {
    let Some(index) = open_tags.iter().rposition(|tag| tag == closing) else {
        return;
    };
    while open_tags.len() > index {
        let tag = open_tags.pop().expect("length checked");
        output.push_str("</");
        output.push_str(&tag);
        output.push('>');
    }
}

fn append_allowed_attributes(output: &mut String, tag: &str, attributes: &[(String, String)]) {
    if supports_alignment(tag) && attribute_equals(attributes, "data-align", "center") {
        output.push_str(" data-align=\"center\"");
    }
    if tag == "ul" && attribute_equals(attributes, "data-note-todo", "true") {
        output.push_str(" data-note-todo=\"true\"");
    }
    if tag == "li" {
        if attribute_equals(attributes, "data-done", "true") {
            output.push_str(" data-done=\"true\"");
        } else if attribute_equals(attributes, "data-done", "false") {
            output.push_str(" data-done=\"false\"");
        }
    }
    if tag == "figure" {
        if let Some(id) =
            attribute_value(attributes, "data-note-image").filter(|id| safe_attachment_id(id))
        {
            output.push_str(" data-note-image=\"");
            push_escaped_attribute(output, id);
            output.push_str("\" contenteditable=\"false\"");
        }
    }
}

fn sanitize_image_tag(attributes: &[(String, String)]) -> Option<String> {
    let source = attribute_value(attributes, "src")?;
    let attachment_id = note_image_attachment_id(source)?;
    let mut output = String::from("<img src=\"note-image://");
    push_escaped_attribute(&mut output, attachment_id);
    output.push('"');
    if let Some(alt) = attribute_value(attributes, "alt") {
        output.push_str(" alt=\"");
        let normalized_alt = truncate_chars(&decode_basic_html_entities(alt), MAX_ALT_TEXT_LENGTH);
        push_escaped_attribute(&mut output, &normalized_alt);
        output.push('"');
    }
    output.push('>');
    Some(output)
}

fn supports_alignment(tag: &str) -> bool {
    matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "blockquote" | "li")
}

fn attribute_value<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find_map(|(attribute_name, value)| (attribute_name == name).then_some(value.as_str()))
}

fn attribute_equals(attributes: &[(String, String)], name: &str, expected: &str) -> bool {
    attribute_value(attributes, name).is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn note_image_attachment_id(source: &str) -> Option<&str> {
    const PREFIX: &str = "note-image://";
    let bytes = source.as_bytes();
    if bytes.len() <= PREFIX.len() || !bytes[..PREFIX.len()].eq_ignore_ascii_case(PREFIX.as_bytes())
    {
        return None;
    }
    let id = &source[PREFIX.len()..];
    safe_attachment_id(id).then_some(id)
}

fn safe_attachment_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ATTACHMENT_ID_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn decode_basic_html_entities(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while cursor < value.len() {
        let Some(relative_ampersand) = value[cursor..].find('&') else {
            output.push_str(&value[cursor..]);
            break;
        };
        let ampersand = cursor + relative_ampersand;
        output.push_str(&value[cursor..ampersand]);
        let entity_start = ampersand + 1;
        let entity_search_end = (entity_start + 16).min(value.len());
        let Some(relative_semicolon) = value.as_bytes()[entity_start..entity_search_end]
            .iter()
            .position(|byte| *byte == b';')
        else {
            output.push('&');
            cursor = ampersand + 1;
            continue;
        };
        let semicolon = entity_start + relative_semicolon;
        let entity = &value[ampersand + 1..semicolon];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        if let Some(character) = decoded.filter(|character| !character.is_control()) {
            output.push(character);
        } else {
            output.push_str(&value[ampersand..=semicolon]);
        }
        cursor = semicolon + 1;
    }
    output
}

fn push_escaped_attribute(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        sanitize_rich_text_html, sanitize_rich_text_html_for_storage, MAX_RICH_TEXT_ELEMENTS,
        MAX_RICH_TEXT_INPUT_BYTES, MAX_RICH_TEXT_NESTING_DEPTH, OVERSIZED_RICH_TEXT_PLACEHOLDER,
    };

    #[test]
    fn strips_scripts_event_handlers_and_external_images() {
        let unsafe_html = concat!(
            "<p onclick=\"steal()\">safe</p>",
            "<script>fetch('https://example.invalid')</script>",
            "<img src=x onerror=\"steal()\">",
            "<iframe src=\"https://example.invalid\"><p>hidden</p></iframe>"
        );
        assert_eq!(sanitize_rich_text_html(unsafe_html), "<p>safe</p>");
    }

    #[test]
    fn preserves_editor_markup_and_only_controlled_images() {
        let input = concat!(
            "<h2 data-align=\"center\" style=\"color:red\">Title</h2>",
            "<ul data-note-todo=\"true\"><li data-done=\"false\"><b>Task</b></li></ul>",
            "<figure data-note-image=\"a-b_1.2\" contenteditable=\"true\">",
            "<img src=\"NOTE-IMAGE://a-b_1.2\" alt=\"A &amp; B\" onload=\"bad()\">",
            "<figcaption>Caption</figcaption></figure>"
        );
        assert_eq!(
            sanitize_rich_text_html(input),
            concat!(
                "<h2 data-align=\"center\">Title</h2>",
                "<ul data-note-todo=\"true\"><li data-done=\"false\"><strong>Task</strong></li></ul>",
                "<figure data-note-image=\"a-b_1.2\" contenteditable=\"false\">",
                "<img src=\"note-image://a-b_1.2\" alt=\"A &amp; B\">",
                "<figcaption>Caption</figcaption></figure>"
            )
        );
    }

    #[test]
    fn many_ampersands_in_alt_text_remain_bounded_and_safe() {
        let alt = format!("{};", "&".repeat(511));
        let input = format!("<img src=\"note-image://safe\" alt=\"{alt}\">");
        let output = sanitize_rich_text_html(&input);
        assert_eq!(output.matches("&amp;").count(), 511);
        assert!(!output.contains(" on"));
        assert_eq!(sanitize_rich_text_html(&output), output);
    }

    #[test]
    fn rejects_entity_obfuscation_and_balances_allowed_tags() {
        let input = "<p><strong>ok<img src=\"note-image&#58;//bad\"></p><svg><img src=\"note-image://hidden\"></svg>";
        assert_eq!(sanitize_rich_text_html(input), "<p><strong>ok</strong></p>");
    }

    #[test]
    fn unwraps_unknown_formatting_without_reintroducing_markup() {
        let input = "<marquee><span><a href=\"javascript:bad()\">plain</a></span></marquee>";
        assert_eq!(sanitize_rich_text_html(input), "plain");
    }

    #[test]
    fn sanitizer_is_idempotent() {
        let input =
            "<p data-align=\"center\">A</p><img src=\"note-image://abc\" alt=\"A &amp; B\">";
        let once = sanitize_rich_text_html(input);
        assert_eq!(sanitize_rich_text_html(&once), once);
    }

    #[test]
    fn caps_nesting_and_balances_output() {
        let attempted_depth = MAX_RICH_TEXT_NESTING_DEPTH + 64;
        let input = format!(
            "{}x{}",
            "<div>".repeat(attempted_depth),
            "</div>".repeat(attempted_depth)
        );
        let output = sanitize_rich_text_html(&input);
        assert_eq!(output.matches("<div>").count(), MAX_RICH_TEXT_NESTING_DEPTH);
        assert_eq!(
            output.matches("</div>").count(),
            MAX_RICH_TEXT_NESTING_DEPTH
        );
        assert!(output.contains('x'));
        assert_eq!(sanitize_rich_text_html(&output), output);
    }

    #[test]
    fn rejects_oversized_utf8_input_with_a_visible_safe_placeholder() {
        let input = format!(
            "<p>{}<script>steal()</script><img src=x onerror=steal()>",
            "安".repeat(MAX_RICH_TEXT_INPUT_BYTES / 3 + 256)
        );
        let output = sanitize_rich_text_html(&input);
        assert_eq!(output, OVERSIZED_RICH_TEXT_PLACEHOLDER);
        assert!(sanitize_rich_text_html_for_storage(&input).is_none());
        assert_eq!(
            sanitize_rich_text_html(&output),
            "<p>富文本内容超过安全上限，当前未加载。</p>"
        );
    }

    #[test]
    fn preserves_large_cjk_content_within_the_editor_contract() {
        let body = "安".repeat(400_000);
        let input = format!("<p>{body}</p>");
        assert_eq!(sanitize_rich_text_html(&input), input);
    }

    #[test]
    fn mismatched_closes_remain_bounded_and_idempotent() {
        let input = format!(
            "{}safe{}",
            "<div>".repeat(MAX_RICH_TEXT_NESTING_DEPTH + 64),
            "</p>".repeat(10_000)
        );
        let output = sanitize_rich_text_html(&input);
        assert_eq!(output.matches("<div>").count(), MAX_RICH_TEXT_NESTING_DEPTH);
        assert_eq!(
            output.matches("</div>").count(),
            MAX_RICH_TEXT_NESTING_DEPTH
        );
        assert_eq!(sanitize_rich_text_html(&output), output);
    }

    #[test]
    fn many_unterminated_openers_are_linear_and_safe() {
        let input = "<".repeat(100_000);
        let output = sanitize_rich_text_html(&input);
        assert_eq!(output, "&lt;".repeat(100_000));
        assert_eq!(sanitize_rich_text_html(&output), output);
    }

    #[test]
    fn caps_dom_element_count_before_webview_rendering() {
        let at_limit = "<br>".repeat(MAX_RICH_TEXT_ELEMENTS);
        assert_eq!(sanitize_rich_text_html(&at_limit), at_limit);

        let over_limit = "<br>".repeat(MAX_RICH_TEXT_ELEMENTS + 1);
        assert_eq!(
            sanitize_rich_text_html(&over_limit),
            OVERSIZED_RICH_TEXT_PLACEHOLDER
        );
        assert!(sanitize_rich_text_html_for_storage(&over_limit).is_none());
    }
}
