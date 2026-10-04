// v2.22.52 - Portable knowledge documents, strict CSV and remapped copies.
use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn parse_csv(raw: &str) -> Result<Vec<Vec<String>>, String> {
    if raw.len() > 64 * 1024 * 1024 {
        return Err("CSV 超过 64 MB".into());
    }
    let source = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                cell.push(c);
            }
            if cell.len() > 1_000_000 {
                return Err("CSV 单元格超过限制".into());
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() && !closed => quoted = true,
            '"' => return Err("CSV 的引号位置无效".into()),
            ',' => {
                row.push(std::mem::take(&mut cell));
                closed = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                closed = false;
            }
            _ if closed => return Err("CSV 引号结束后只能跟逗号或换行".into()),
            _ => cell.push(c),
        }
        if cell.len() > 1_000_000 || row.len() > 256 || rows.len() > 10_001 {
            return Err("CSV 超出行列或单元格限制".into());
        }
    }
    if quoted {
        return Err("CSV 有未闭合的引号".into());
    }
    if !cell.is_empty() || !row.is_empty() || closed {
        row.push(cell);
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("CSV 为空".into());
    }
    let width = rows[0].len();
    if width == 0 || width > 256 || rows.len() > 10_001 {
        return Err("CSV 超出行列限制".into());
    }
    if rows.iter().any(|row| row.len() != width) {
        return Err("CSV 每行列数必须一致".into());
    }
    if rows[0].iter().any(|s| s.trim().is_empty())
        || rows[0]
            .iter()
            .map(|s| s.trim().to_lowercase())
            .collect::<HashSet<_>>()
            .len()
            != width
    {
        return Err("CSV 表头不能为空或重名".into());
    }
    Ok(rows)
}

pub fn write_csv(rows: &[Vec<String>]) -> String {
    let escape = |value: &String| {
        if value.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.clone()
        }
    };
    rows.iter()
        .map(|row| row.iter().map(escape).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

pub fn csv_to_pages(
    source: &str,
    title: &str,
    mut id: impl FnMut(&str) -> String,
) -> Result<Vec<Value>, String> {
    let rows = parse_csv(source)?;
    let db_id = id("database");
    let mut fields = Vec::new();
    for (index, name) in rows[0].iter().enumerate().skip(1) {
        let values = rows
            .iter()
            .skip(1)
            .map(|row| row[index].trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let kind = if !values.is_empty()
            && values.iter().all(|s| {
                let unsigned = s.trim_start_matches(['+', '-']);
                let integer = unsigned.split(['.', 'e', 'E']).next().unwrap_or("");
                s.parse::<f64>().is_ok_and(f64::is_finite)
                    && !(integer.len() > 1 && integer.starts_with('0'))
            }) {
            FieldKind::Number
        } else if !values.is_empty() && values.iter().all(|s| date_day(s).is_some()) {
            FieldKind::Date
        } else {
            FieldKind::Text
        };
        fields.push(DatabaseField {
            id: id("property"),
            name: name.trim().into(),
            kind,
            ..Default::default()
        });
    }
    let view = DatabaseView {
        id: id("view"),
        name: "表格".into(),
        visible_fields: fields.iter().map(|f| f.id.clone()).collect(),
        ..Default::default()
    };
    let database = KnowledgeDatabase {
        fields: fields.clone(),
        default_view_id: view.id.clone(),
        views: vec![view],
    };
    database.validate()?;
    let mut pages = vec![
        json!({"id":db_id,"kind":"DOCUMENT","title":title,"document":{"blocks":[],"knowledge":KnowledgePage{database:Some(database),icon:"▦".into(),..Default::default()}}}),
    ];
    for (index, row) in rows.iter().enumerate().skip(1) {
        let mut properties = BTreeMap::new();
        for (i, field) in fields.iter().enumerate() {
            properties.insert(
                field.id.clone(),
                parse_cell(field, &row[i + 1]).map_err(|e| format!("第 {} 行：{e}", index + 1))?,
            );
        }
        pages.push(json!({"id":id("page"),"kind":"DOCUMENT","title":row[0],"document":{"blocks":[],"knowledge":KnowledgePage{parent_id:Some(db_id.clone()),properties,order:index as i64,..Default::default()}}}));
    }
    Ok(pages)
}

pub fn markdown_blocks(source: &str, mut id: impl FnMut() -> String) -> Vec<Value> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line.trim().is_empty() {
            continue;
        }
        let mut meta = KnowledgeBlock::default();
        let mut text = line.to_string();
        if line.starts_with("```") || line.starts_with("~~~") {
            let count = line
                .chars()
                .take_while(|c| Some(*c) == line.chars().next())
                .count();
            let fence = &line[..count];
            meta.kind = BlockKind::Code;
            meta.language = line[count..].trim().into();
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].starts_with(fence) {
                body.push(lines[index]);
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            text = body.join("\n");
        } else if let Some(value) = line.strip_prefix("### ") {
            meta.kind = BlockKind::Heading3;
            text = value.into();
        } else if let Some(value) = line.strip_prefix("## ") {
            meta.kind = BlockKind::Heading2;
            text = value.into();
        } else if let Some(value) = line.strip_prefix("# ") {
            meta.kind = BlockKind::Heading1;
            text = value.into();
        } else if let Some(value) = line.strip_prefix("- [ ] ") {
            meta.kind = BlockKind::Todo;
            text = value.into();
        } else if let Some(value) = line
            .strip_prefix("- [x] ")
            .or_else(|| line.strip_prefix("- [X] "))
        {
            meta.kind = BlockKind::Todo;
            meta.checked = true;
            text = value.into();
        } else if let Some(value) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            meta.kind = BlockKind::BulletedList;
            text = value.into();
        } else if let Some((number, value)) = line
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        {
            let _ = number;
            meta.kind = BlockKind::NumberedList;
            text = value.into();
        } else if let Some(value) = line.strip_prefix("> ") {
            meta.kind = BlockKind::Quote;
            text = value.into();
        } else if matches!(line.trim(), "---" | "***" | "___") {
            meta.kind = BlockKind::Divider;
            text.clear();
        } else if line.contains('|')
            && index < lines.len()
            && lines[index].contains('-')
            && lines[index]
                .trim()
                .chars()
                .all(|c| matches!(c, '|' | '-' | ':' | ' '))
        {
            meta.kind = BlockKind::Table;
            let cells = |line: &str| {
                line.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|s| s.trim().replace("<br>", "\n").replace("&#124;", "|"))
                    .collect::<Vec<_>>()
            };
            meta.table.push(cells(line));
            index += 1;
            while index < lines.len() && lines[index].contains('|') {
                meta.table.push(cells(lines[index]));
                index += 1;
            }
            let width = meta.table.iter().map(Vec::len).max().unwrap_or(1);
            for row in &mut meta.table {
                row.resize(width, String::new());
            }
            text = meta
                .table
                .iter()
                .map(|r| r.join(" | "))
                .collect::<Vec<_>>()
                .join("\n");
        } else {
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && !lines[index].starts_with(['#', '-', '>', '`', '~', '*'])
            {
                text.push('\n');
                text.push_str(lines[index]);
                index += 1;
            }
        }
        blocks.push(json!({"id":id(),"type":"TEXT","text":text,"knowledge":meta}));
    }
    if blocks.is_empty() {
        blocks
            .push(json!({"id":id(),"type":"TEXT","text":"","knowledge":KnowledgeBlock::default()}));
    }
    blocks
}

pub fn blocks_markdown(blocks: &[Value]) -> String {
    let mut output = Vec::new();
    let mut list = 0;
    for block in blocks {
        let text = block["text"].as_str().unwrap_or("");
        let meta = serde_json::from_value::<KnowledgeBlock>(block["knowledge"].clone())
            .unwrap_or_default();
        let body = match meta.kind {
            BlockKind::Heading1 => format!("# {text}"),
            BlockKind::Heading2 => format!("## {text}"),
            BlockKind::Heading3 => format!("### {text}"),
            BlockKind::BulletedList => format!("- {text}"),
            BlockKind::NumberedList => {
                list += 1;
                format!("{list}. {text}")
            }
            BlockKind::Todo => format!("- [{}] {text}", if meta.checked { "x" } else { " " }),
            BlockKind::Quote | BlockKind::Callout => text
                .lines()
                .map(|l| format!("> {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
            BlockKind::Divider => "---".into(),
            BlockKind::Code => {
                let count = text
                    .lines()
                    .map(|line| line.chars().take_while(|c| *c == '`').count())
                    .max()
                    .unwrap_or(0);
                let fence = "`".repeat((count + 1).max(3));
                format!("{fence}{}\n{text}\n{fence}", meta.language)
            }
            BlockKind::Equation => format!("$$\n{text}\n$$"),
            BlockKind::PageLink => format!("[{text}](page:{})", meta.target_page_id),
            BlockKind::BlockLink => format!(
                "[{text}](page:{}#{})",
                meta.target_page_id, meta.target_block_id
            ),
            BlockKind::Bookmark
            | BlockKind::Embed
            | BlockKind::Audio
            | BlockKind::Video
            | BlockKind::File => format!("[{text}]({})", meta.url),
            BlockKind::Table => {
                let mut lines = Vec::new();
                for (i, row) in meta.table.iter().enumerate() {
                    lines.push(format!(
                        "| {} |",
                        row.iter()
                            .map(|v| v.replace('|', "&#124;").replace('\n', "<br>"))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ));
                    if i == 0 {
                        lines.push(format!("| {} |", vec!["---"; row.len()].join(" | ")));
                    }
                }
                lines.join("\n")
            }
            BlockKind::TableOfContents => String::new(),
            _ => text.into(),
        };
        if meta.kind != BlockKind::NumberedList {
            list = 0;
        }
        if !body.is_empty() {
            output.push(body);
        }
    }
    output.join("\n\n")
}

pub fn remap_pages(
    pages: &[Value],
    mut id: impl FnMut(&str) -> String,
) -> Result<Vec<Value>, String> {
    if pages.is_empty() || pages.len() > 10_000 {
        return Err("页面数量无效".into());
    }
    let mut page_ids = HashMap::new();
    let mut block_ids = HashMap::new();
    for page in pages {
        let old = page["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("页面缺少 ID")?;
        if page_ids.insert(old.to_string(), id("page")).is_some() {
            return Err("页面 ID 重复".into());
        }
        if !page["encryption"].is_null() {
            return Err("请先解锁并单独导出加密页面".into());
        }
        if let Some(blocks) = page["document"]["blocks"].as_array() {
            for block in blocks {
                if let Some(old) = block["id"].as_str() {
                    block_ids.insert(
                        (page["id"].as_str().unwrap().to_string(), old.to_string()),
                        id("block"),
                    );
                }
            }
        }
    }
    let mut result = Vec::new();
    for original in pages {
        let old_id = original["id"].as_str().unwrap();
        let mut page = original.clone();
        page["id"] = json!(page_ids[old_id]);
        page["folderId"] = Value::Null;
        page["createdAtEpochMillis"] = json!(0);
        page["updatedAtEpochMillis"] = json!(0);
        page["revisions"] = json!([]);
        page["versions"] = json!([]);
        page["latestVersionId"] = json!("");
        page["deletedAtEpochMillis"] = Value::Null;
        page["protectionStateRevision"] = json!(0);
        page["pinned"] = json!(false);
        let mut meta = if page["document"]["knowledge"].is_null() {
            KnowledgePage::default()
        } else {
            serde_json::from_value::<KnowledgePage>(page["document"]["knowledge"].clone())
                .map_err(|e| e.to_string())?
        };
        meta.validate()?;
        meta.parent_id = meta
            .parent_id
            .as_ref()
            .and_then(|id| page_ids.get(id).cloned());
        meta.locked = false;
        meta.template = false;
        for value in meta.properties.values_mut() {
            if let CellValue::Relation(ids) = value {
                for target in ids {
                    if let Some(next) = page_ids.get(target) {
                        *target = next.clone();
                    }
                }
            }
        }
        if let Some(db) = &mut meta.database {
            for field in &mut db.fields {
                if let Some(next) = field
                    .relation_database_id
                    .as_ref()
                    .and_then(|id| page_ids.get(id))
                {
                    field.relation_database_id = Some(next.clone());
                }
            }
        }
        for comment in &mut meta.comments {
            comment.id = id("comment");
            if let Some(next) = comment
                .block_id
                .as_ref()
                .and_then(|block| block_ids.get(&(old_id.into(), block.clone())))
            {
                comment.block_id = Some(next.clone());
            }
        }
        page["document"]["knowledge"] = serde_json::to_value(meta).map_err(|e| e.to_string())?;
        if let Some(blocks) = page["document"]["blocks"].as_array_mut() {
            for block in blocks {
                if let Some(text) = block["text"].as_str() {
                    if !matches!(
                        block["knowledge"]["kind"].as_str(),
                        Some("code" | "equation")
                    ) {
                        block["text"] = json!(remap_wiki_references(text, &page_ids, &block_ids));
                    }
                }
                if let Some(old) = block["id"].as_str() {
                    if let Some(next) = block_ids.get(&(old_id.into(), old.into())) {
                        block["id"] = json!(next);
                    }
                }
                if !block["knowledge"].is_null() {
                    let mut meta: KnowledgeBlock =
                        serde_json::from_value(block["knowledge"].clone())
                            .map_err(|e| e.to_string())?;
                    meta.validate()?;
                    meta.parent_id = meta
                        .parent_id
                        .as_ref()
                        .and_then(|p| block_ids.get(&(old_id.into(), p.clone())).cloned());
                    if let Some(next) =
                        block_ids.get(&(meta.target_page_id.clone(), meta.target_block_id.clone()))
                    {
                        meta.target_block_id = next.clone();
                    }
                    if let Some(next) = page_ids.get(&meta.target_page_id) {
                        meta.target_page_id = next.clone();
                    }
                    block["knowledge"] = serde_json::to_value(meta).map_err(|e| e.to_string())?;
                }
            }
        }
        result.push(page);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "knowledge_exchange_tests.rs"]
mod tests;
