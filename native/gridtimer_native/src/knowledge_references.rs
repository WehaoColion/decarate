// v2.22.52 - Resolve complete wiki references and preserve them when copying pages.
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
pub struct WikiReference<'a> {
    pub start: usize,
    pub end: usize,
    pub page: &'a str,
    pub block: &'a str,
    pub label: &'a str,
}

pub fn wiki_references(text: &str) -> Vec<WikiReference<'_>> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("[[") {
        let start = cursor + offset;
        cursor = start + 2;
        if text[..start]
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count()
            % 2
            == 1
        {
            continue;
        }
        let Some(close) = text[cursor..].find("]]") else {
            break;
        };
        let end = cursor + close + 2;
        let body = &text[cursor..end - 2];
        if body.len() > 4096 || body.contains(['[', ']', '\n', '\r']) {
            continue;
        }
        let (target, label) = body.split_once('|').unwrap_or((body, ""));
        let (page, block) = target.trim().split_once('#').unwrap_or((target.trim(), ""));
        if page.is_empty()
            || page.chars().any(char::is_whitespace)
            || block.chars().any(char::is_whitespace)
        {
            cursor = end;
            continue;
        }
        result.push(WikiReference {
            start,
            end,
            page,
            block,
            label: label.trim(),
        });
        cursor = end;
    }
    result
}

pub fn remap_wiki_references(
    text: &str,
    pages: &HashMap<String, String>,
    blocks: &HashMap<(String, String), String>,
) -> String {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for reference in wiki_references(text) {
        result.push_str(&text[cursor..reference.start]);
        if let Some(page) = pages.get(reference.page) {
            result.push_str("[[");
            result.push_str(page);
            if !reference.block.is_empty() {
                result.push('#');
                result.push_str(
                    blocks
                        .get(&(reference.page.into(), reference.block.into()))
                        .map(String::as_str)
                        .unwrap_or(reference.block),
                );
            }
            if !reference.label.is_empty() {
                result.push('|');
                result.push_str(reference.label);
            }
            result.push_str("]]");
        } else {
            result.push_str(&text[reference.start..reference.end]);
        }
        cursor = reference.end;
    }
    result.push_str(&text[cursor..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn knowledge_references_match_whole_ids_and_remap_only_internal_targets() {
        let source = r"[[page-long]] [[page#block|引文]] [[outside]] \[[page]] [[page";
        let links = wiki_references(source);
        assert_eq!(links.iter().filter(|r| r.page == "page").count(), 1);
        assert_eq!(links[1].block, "block");
        let result = remap_wiki_references(
            source,
            &HashMap::from([("page".into(), "copy".into())]),
            &HashMap::from([(("page".into(), "block".into()), "copy-block".into())]),
        );
        assert_eq!(
            result,
            r"[[page-long]] [[copy#copy-block|引文]] [[outside]] \[[page]] [[page"
        );
    }
}
