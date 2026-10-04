// v2.23.2.7 Android - Accept TeX whitespace while preserving dollar currency boundaries.
// This module is never used by the Windows rich-text or knowledge renderer.
use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};
use rand::RngCore;
use std::ops::Range;

const KATEX_CSS: &str = include_str!("desktop/assets/katex_v0.18.7_embedded.css");
const KATEX_JS: &str = include_str!("desktop/assets/katex_v0.18.7.min.js");

/// Only this trusted shell is executable. Answer text is parsed into safe HTML;
/// mathematical input reaches KaTeX as an escaped attribute, never JavaScript.
pub fn render_android_ai_answer_html(content: &str, dark_theme: bool) -> String {
    let body = render_answer_body(content);
    let mut nonce_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce: String = nonce_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let theme = if dark_theme { "dark" } else { "light" };
    format!(
        r#"<!doctype html><html lang="zh-CN" data-theme="{theme}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'; font-src data:; img-src 'none'; media-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"><title>AI 回答</title><style>{KATEX_CSS}
html,body{{margin:0;padding:0;background:transparent;color:#24272a;font:16px/1.75 system-ui,-apple-system,'Noto Sans CJK SC',sans-serif;overflow-wrap:anywhere}}html[data-theme=dark] body{{color:#e5e8ec}}main{{padding:2px 0 8px;min-width:0}}p{{margin:0 0 14px}}p:last-child{{margin-bottom:0}}h1,h2,h3,h4,h5,h6{{line-height:1.4;margin:20px 0 12px;font-size:1.15em}}h1{{font-size:1.4em}}h2{{font-size:1.25em}}ul,ol{{margin:10px 0 16px;padding-left:26px}}li{{margin:5px 0}}blockquote{{border-left:3px solid #8798a8;padding:2px 12px;margin:12px 0;opacity:.92}}pre{{max-width:100%;overflow:auto;white-space:pre;background:#8798a815;border:1px solid #8798a833;border-radius:8px;padding:12px;font:14px/1.6 ui-monospace,monospace}}code{{font:14px/1.6 ui-monospace,monospace;background:#8798a815;border-radius:3px;padding:1px 3px}}pre code{{padding:0;background:none}}table{{display:block;max-width:100%;overflow-x:auto;border-collapse:collapse;margin:14px 0}}th,td{{border:1px solid #8798a855;padding:7px 10px;text-align:left}}hr{{border:0;border-top:1px solid #8798a855;margin:18px 0}}.math-display{{display:block;max-width:100%;overflow-x:auto;overflow-y:hidden;padding:6px 0;margin:8px 0}}.math-inline{{display:inline;white-space:nowrap}}.math-source[data-math-error]{{white-space:pre-wrap;overflow-wrap:anywhere;font-family:ui-monospace,monospace}}.katex{{font-size:1.05em}}.katex-display{{margin:.35em 0}}.image-alt,.link-text{{color:inherit}}
</style></head><body><main id="android-ai-answer">{body}</main><script nonce="{nonce}">{KATEX_JS}</script><script nonce="{nonce}">
(()=>{{let errors=0;const formulas=document.querySelectorAll('.math-source[data-latex]');for(const element of formulas){{const source=element.textContent;try{{katex.render(element.dataset.latex,element,{{displayMode:element.dataset.display==='true',throwOnError:true,trust:false,maxSize:20,maxExpand:1000,strict:'ignore',output:'htmlAndMathml'}});element.dataset.mathRendered='true';}}catch(error){{errors++;element.textContent=source;element.dataset.mathError='true';element.title='公式无法解析，已保留原文';}}}}document.documentElement.dataset.mathCount=String(formulas.length);document.documentElement.dataset.mathErrors=String(errors);document.documentElement.dataset.mathReady='true';document.dispatchEvent(new Event('android-ai-answer-ready'));}})();
</script></body></html>"#
    )
}

#[derive(Debug)]
struct Formula {
    latex: String,
    source: String,
    display: bool,
}

fn markdown_options() -> Options {
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS
}

/// Math delimiters inside Markdown code remain literal. HTML encountered later
/// is escaped as visible text; it never becomes an executable answer fragment.
fn protected_markdown_ranges(content: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut code_start = None;
    for (event, range) in Parser::new_ext(content, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                ranges.push(code_start.take().unwrap_or(range.start)..range.end)
            }
            Event::Code(_) => ranges.push(range),
            _ => {}
        }
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| range.start <= previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn escaped_at(bytes: &[u8], position: usize) -> bool {
    let mut start = position;
    while start > 0 && bytes[start - 1] == b'\\' {
        start -= 1;
    }
    (position - start) % 2 == 1
}

fn formula_at(content: &str, position: usize, limit: usize) -> Option<(usize, Formula)> {
    let remaining = &content[position..limit];
    let (opening, closing, display) = if remaining.starts_with("\\[") {
        ("\\[", "\\]", true)
    } else if remaining.starts_with("\\(") {
        ("\\(", "\\)", false)
    } else if remaining.starts_with("$$") {
        ("$$", "$$", true)
    } else if remaining.starts_with('$') {
        ("$", "$", false)
    } else {
        return None;
    };
    if escaped_at(content.as_bytes(), position) {
        return None;
    }
    let start = position + opening.len();
    if opening == "$" && content[start..limit].chars().next()?.is_whitespace() {
        return None;
    }
    let mut search = start;
    while let Some(relative) = content[search..limit].find(closing) {
        let end = search + relative;
        if opening == "$" && content[start..end].contains('\n') {
            return None;
        }
        if escaped_at(content.as_bytes(), end) {
            search = end + closing.len();
            continue;
        }
        let latex = &content[start..end];
        let after = end + closing.len();
        // A dollar followed by a number is normally the next currency amount.
        // Incomplete or empty formulas stay as readable original text.
        if latex.trim().is_empty()
            || (opening == "$" && latex.chars().last().is_some_and(char::is_whitespace))
            || (opening == "$"
                && content[after..limit]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_digit()))
        {
            search = after;
            continue;
        }
        return Some((
            after,
            Formula {
                latex: latex.to_owned(),
                source: content[position..after].to_owned(),
                display,
            },
        ));
    }
    None
}

fn tokenize_math(content: &str) -> (String, String, Vec<Formula>) {
    let protected = protected_markdown_ranges(content);
    let mut prefix = "GTANDROIDMATHTOKEN".to_owned();
    while content.contains(&prefix) {
        prefix.push('X');
    }
    let mut output = String::with_capacity(content.len());
    let mut formulas = Vec::new();
    let mut position = 0;
    let mut range_index = 0;
    while position < content.len() {
        while range_index < protected.len() && protected[range_index].end <= position {
            range_index += 1;
        }
        if let Some(range) = protected
            .get(range_index)
            .filter(|range| range.start <= position)
        {
            output.push_str(&content[position..range.end]);
            position = range.end;
            continue;
        }
        let limit = protected
            .get(range_index)
            .map_or(content.len(), |range| range.start);
        if let Some((end, formula)) = formula_at(content, position, limit) {
            output.push_str(&format!("{prefix}{}Z", formulas.len()));
            formulas.push(formula);
            position = end;
        } else {
            let character = content[position..].chars().next().unwrap();
            output.push(character);
            position += character.len_utf8();
        }
    }
    (output, prefix, formulas)
}

fn formula_html(formula: &Formula) -> String {
    let display = if formula.display { "true" } else { "false" };
    let class = if formula.display {
        "math-display"
    } else {
        "math-inline"
    };
    format!(
        "<span class=\"math-source {class}\" data-display=\"{display}\" data-latex=\"{}\">{}</span>",
        escape_html(&formula.latex),
        escape_html(&formula.source)
    )
}

fn text_with_formulas<'a>(text: &str, prefix: &str, formulas: &[Formula]) -> Vec<Event<'a>> {
    let mut result = Vec::new();
    let mut remainder = text;
    while let Some(start) = remainder.find(prefix) {
        if start > 0 {
            result.push(Event::Text(CowStr::from(remainder[..start].to_owned())));
        }
        let tail = &remainder[start + prefix.len()..];
        let Some(end) = tail.find('Z') else {
            result.push(Event::Text(CowStr::from(remainder[start..].to_owned())));
            return result;
        };
        let Some(formula) = tail[..end]
            .parse::<usize>()
            .ok()
            .and_then(|index| formulas.get(index))
        else {
            result.push(Event::Text(CowStr::from(remainder[start..].to_owned())));
            return result;
        };
        result.push(Event::Html(CowStr::from(formula_html(formula))));
        remainder = &tail[end + 1..];
    }
    if !remainder.is_empty() {
        result.push(Event::Text(CowStr::from(remainder.to_owned())));
    }
    result
}

fn render_answer_body(content: &str) -> String {
    let (markdown, prefix, formulas) = tokenize_math(content);
    let events = Parser::new_ext(&markdown, markdown_options()).flat_map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => {
            text_with_formulas(&text, &prefix, &formulas)
        }
        Event::Start(Tag::Link { .. }) => vec![Event::Html("<span class=\"link-text\">".into())],
        Event::End(TagEnd::Link) => vec![Event::Html("</span>".into())],
        Event::Start(Tag::Image { .. }) => vec![Event::Html("<span class=\"image-alt\">".into())],
        Event::End(TagEnd::Image) => vec![Event::Html("</span>".into())],
        Event::TaskListMarker(done) => vec![Event::Text(if done { "☑ " } else { "☐ " }.into())],
        Event::Text(text) => text_with_formulas(&text, &prefix, &formulas),
        other => vec![other],
    });
    let mut body = String::new();
    html::push_html(&mut body, events);
    body
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_math_render_supports_all_delimiters_and_markdown() {
        let body = render_answer_body("## 复利\n\n**基本公式**\n\n\\[\nA = P \\times (1 + r)^n\n\\]\n\n- \\(A\\)：最终金额\n- $P$：本金\n\n$$\\frac{1}{2} + \\sqrt{x}$$");
        assert!(body.contains("<h2>复利</h2>"));
        assert!(body.contains("<strong>基本公式</strong>"));
        assert_eq!(body.matches("class=\"math-source ").count(), 4);
        assert_eq!(body.matches("data-display=\"true\"").count(), 2);
        assert!(body.contains("A = P \\times (1 + r)^n"));
        assert!(body.contains("<ul>"));
    }

    #[test]
    fn android_math_render_preserves_code_escapes_currency_and_incomplete_source() {
        let source = "`\\(x\\)` and `$q$`\n\n```latex\n\\[x^2\\]\n$$y$$\n```\n\n\\$20 and $30\n\n\\[not closed";
        let body = render_answer_body(source);
        assert!(!body.contains("class=\"math-source "));
        assert!(body.contains("<code>\\(x\\)</code>"));
        assert!(body.contains("\\[x^2\\]"));
        assert!(body.contains("$$y$$"));
        assert!(body.contains("$20 and $30"));
        let (_, _, formulas) = tokenize_math("$100 and $200; \\(a_b^2\\)");
        assert_eq!(formulas.len(), 1);
        assert_eq!(formulas[0].latex, "a_b^2");
    }

    #[test]
    fn android_math_render_accepts_parenthesized_whitespace_without_treating_currency_as_math() {
        let source = "正文 \\( x_i^2 \\) 和 \\(\n r = 0.05\n\\)。金额 $100 and $200；美元分隔的跨行 $x\n+y$ 保留。空公式 \\( \n \\) 保留。";
        let (tokenized, _, formulas) = tokenize_math(source);
        assert_eq!(formulas.len(), 2);
        assert_eq!(formulas[0].latex.trim(), "x_i^2");
        assert_eq!(formulas[0].source, "\\( x_i^2 \\)");
        assert_eq!(formulas[1].latex.trim(), "r = 0.05");
        assert_eq!(formulas[1].source, "\\(\n r = 0.05\n\\)");
        assert!(tokenized.contains("$100 and $200"));
        assert!(tokenized.contains("$x\n+y$"));
        assert!(tokenized.contains("\\( \n \\)"));
        assert_eq!(
            render_answer_body(source)
                .matches("class=\"math-source ")
                .count(),
            2
        );
    }

    #[test]
    fn android_math_render_rejects_answer_html_and_external_resources() {
        let source = "<script nonce=\"x\">alert(1)</script>\n\n<img src=\"https://example.invalid/a\" onerror=\"alert(2)\">\n\n![外部图片](https://example.invalid/b) [链接](javascript:alert(3))\n\n\\(\\href{javascript:alert(4)}{x}\\)";
        let body = render_answer_body(source);
        assert!(!body.contains("<script"));
        assert!(!body.contains("<img"));
        assert!(!body.contains("href="));
        assert!(body.contains("&lt;script"));
        assert!(body.contains("外部图片"));
        assert!(body.contains("data-latex=\"\\href{javascript:alert(4)}{x}\""));
    }

    #[test]
    fn android_math_render_cannot_escape_formula_attributes_and_does_not_mutate_source() {
        let source = "\\[\\text{\"</span><script>alert(1)</script> & 原文}\\]".to_owned();
        let original = source.clone();
        let body = render_answer_body(&source);
        assert!(!body.contains("<script>"));
        assert!(body.contains("&quot;&lt;/span&gt;&lt;script&gt;"));
        assert_eq!(source, original);
        let (_, _, formulas) = tokenize_math(&source);
        assert_eq!(formulas[0].source, source);
    }

    #[test]
    fn android_math_render_keeps_a_malformed_formula_readable_without_losing_other_content() {
        let body = render_answer_body(
            "前文 \\(\\unknowncmd{x}\\) 后文\n\nGTANDROIDMATHTOKEN0Z\n\n\\(x+1\\)",
        );
        assert_eq!(body.matches("class=\"math-source ").count(), 2);
        assert!(body.contains("\\(\\unknowncmd{x}\\)"));
        assert!(body.contains("GTANDROIDMATHTOKEN0Z"));
        assert!(body.contains("前文"));
        assert!(body.contains("后文"));
    }
}
