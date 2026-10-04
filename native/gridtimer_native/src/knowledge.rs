// v2.22.52 - Versioned knowledge pages, blocks, databases and saved views.
//! Shared knowledge data. These values live inside the document, so document
//! versions, encryption, atomic persistence and account sync protect them too.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[path = "knowledge_formula.rs"]
mod formula;
#[path = "knowledge_query.rs"]
mod query;
pub use formula::*;
pub use query::*;
#[path = "knowledge_exchange.rs"]
mod exchange;
pub use exchange::*;
#[path = "knowledge_sync.rs"]
mod sync;
pub use sync::resolve_parent_cycles;
#[path = "knowledge_references.rs"]
mod references;
pub use references::*;

pub const KNOWLEDGE_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgePage {
    pub version: u32,
    pub parent_id: Option<String>,
    pub icon: String,
    pub cover: String,
    pub full_width: bool,
    pub small_text: bool,
    pub locked: bool,
    pub template: bool,
    pub order: i64,
    pub tags: Vec<String>,
    pub properties: BTreeMap<String, CellValue>,
    pub database: Option<KnowledgeDatabase>,
    pub comments: Vec<PageComment>,
}

impl Default for KnowledgePage {
    fn default() -> Self {
        Self {
            version: KNOWLEDGE_FORMAT_VERSION,
            parent_id: None,
            icon: String::new(),
            cover: String::new(),
            full_width: false,
            small_text: false,
            locked: false,
            template: false,
            order: 0,
            tags: Vec::new(),
            properties: BTreeMap::new(),
            database: None,
            comments: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PageComment {
    pub id: String,
    pub block_id: Option<String>,
    pub body: String,
    pub author: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub resolved: bool,
    pub deleted_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    #[default]
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    BulletedList,
    NumberedList,
    Todo,
    Quote,
    Callout,
    Toggle,
    Code,
    Equation,
    Divider,
    Table,
    Columns,
    PageLink,
    BlockLink,
    TableOfContents,
    Bookmark,
    Embed,
    File,
    Audio,
    Video,
}

impl BlockKind {
    pub const ALL: [Self; 23] = [
        Self::Paragraph,
        Self::Heading1,
        Self::Heading2,
        Self::Heading3,
        Self::BulletedList,
        Self::NumberedList,
        Self::Todo,
        Self::Quote,
        Self::Callout,
        Self::Toggle,
        Self::Code,
        Self::Equation,
        Self::Divider,
        Self::Table,
        Self::Columns,
        Self::PageLink,
        Self::BlockLink,
        Self::TableOfContents,
        Self::Bookmark,
        Self::Embed,
        Self::File,
        Self::Audio,
        Self::Video,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Paragraph => "正文",
            Self::Heading1 => "一级标题",
            Self::Heading2 => "二级标题",
            Self::Heading3 => "三级标题",
            Self::BulletedList => "无序列表",
            Self::NumberedList => "有序列表",
            Self::Todo => "待办",
            Self::Quote => "引用",
            Self::Callout => "提示块",
            Self::Toggle => "折叠块",
            Self::Code => "代码",
            Self::Equation => "公式",
            Self::Divider => "分隔线",
            Self::Table => "简单表格",
            Self::Columns => "分栏",
            Self::PageLink => "页面引用",
            Self::BlockLink => "块引用",
            Self::TableOfContents => "页面目录",
            Self::Bookmark => "网页书签",
            Self::Embed => "嵌入网页",
            Self::File => "文件",
            Self::Audio => "音频",
            Self::Video => "视频",
        }
    }

    pub fn search_terms(self) -> &'static str {
        match self {
            Self::Paragraph => "text paragraph wenben zw",
            Self::Heading1 => "h1 heading title biaoti bt1",
            Self::Heading2 => "h2 heading biaoti bt2",
            Self::Heading3 => "h3 heading biaoti bt3",
            Self::BulletedList => "bullet list liebiao lb",
            Self::NumberedList => "number list youxu yx",
            Self::Todo => "todo checkbox task daiban db",
            Self::Quote => "quote yinyong yy",
            Self::Callout => "callout tip tishi ts",
            Self::Toggle => "toggle fold zhedie zd",
            Self::Code => "code daima dm",
            Self::Equation => "math equation latex gongshi gs",
            Self::Divider => "divider line fengexian fgx",
            Self::Table => "table biaoge bg",
            Self::Columns => "columns fenlan fl",
            Self::PageLink => "page link yemian ym",
            Self::BlockLink => "block link kuaiyy",
            Self::TableOfContents => "toc mulu ml",
            Self::Bookmark => "bookmark shuqian sq",
            Self::Embed => "embed qianru qr",
            Self::File => "file wenjian wj",
            Self::Audio => "audio yinpin yp",
            Self::Video => "video shipin sp",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeBlock {
    pub version: u32,
    pub kind: BlockKind,
    pub parent_id: Option<String>,
    pub column: u8,
    pub columns: u8,
    pub checked: bool,
    pub collapsed: bool,
    pub color: String,
    pub background: String,
    pub language: String,
    pub target_page_id: String,
    pub target_block_id: String,
    pub url: String,
    pub table: Vec<Vec<String>>,
    pub table_header: bool,
}

impl Default for KnowledgeBlock {
    fn default() -> Self {
        Self {
            version: KNOWLEDGE_FORMAT_VERSION,
            kind: BlockKind::Paragraph,
            parent_id: None,
            column: 0,
            columns: 2,
            checked: false,
            collapsed: false,
            color: String::new(),
            background: String::new(),
            language: String::new(),
            target_page_id: String::new(),
            target_block_id: String::new(),
            url: String::new(),
            table: Vec::new(),
            table_header: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CellValue {
    #[default]
    Empty,
    Text(String),
    Number(f64),
    Select(String),
    MultiSelect(Vec<String>),
    Date(DateRange),
    Checkbox(bool),
    Url(String),
    Relation(Vec<String>),
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Text,
    Number,
    Select,
    MultiSelect,
    Status,
    Date,
    Checkbox,
    Url,
    Email,
    Phone,
    Relation,
    Formula,
    Rollup,
    CreatedTime,
    UpdatedTime,
}

impl FieldKind {
    pub const ALL: [Self; 15] = [
        Self::Text,
        Self::Number,
        Self::Select,
        Self::MultiSelect,
        Self::Status,
        Self::Date,
        Self::Checkbox,
        Self::Url,
        Self::Email,
        Self::Phone,
        Self::Relation,
        Self::Formula,
        Self::Rollup,
        Self::CreatedTime,
        Self::UpdatedTime,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Number => "数字",
            Self::Select => "单选",
            Self::MultiSelect => "多选",
            Self::Status => "状态",
            Self::Date => "日期",
            Self::Checkbox => "复选",
            Self::Url => "网址",
            Self::Email => "邮箱",
            Self::Phone => "电话",
            Self::Relation => "关联",
            Self::Formula => "公式",
            Self::Rollup => "汇总",
            Self::CreatedTime => "创建时间",
            Self::UpdatedTime => "更新时间",
        }
    }
    pub fn computed(self) -> bool {
        matches!(
            self,
            Self::Formula | Self::Rollup | Self::CreatedTime | Self::UpdatedTime
        )
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseField {
    pub id: String,
    pub name: String,
    pub kind: FieldKind,
    pub options: Vec<String>,
    pub formula: String,
    pub relation_database_id: Option<String>,
    pub rollup_relation_field: String,
    pub rollup_target_field: String,
    pub aggregate: AggregateKind,
    pub required: bool,
    pub deleted: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AggregateKind {
    #[default]
    Count,
    Sum,
    Average,
    Minimum,
    Maximum,
    Unique,
    CheckedPercent,
}

impl AggregateKind {
    pub const ALL: [Self; 7] = [
        Self::Count,
        Self::Sum,
        Self::Average,
        Self::Minimum,
        Self::Maximum,
        Self::Unique,
        Self::CheckedPercent,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Count => "数量",
            Self::Sum => "求和",
            Self::Average => "平均值",
            Self::Minimum => "最小值",
            Self::Maximum => "最大值",
            Self::Unique => "去重计数",
            Self::CheckedPercent => "勾选比例",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    #[default]
    Table,
    Board,
    List,
    Gallery,
    Calendar,
    Timeline,
    Form,
}

impl ViewKind {
    pub const ALL: [Self; 7] = [
        Self::Table,
        Self::Board,
        Self::List,
        Self::Gallery,
        Self::Calendar,
        Self::Timeline,
        Self::Form,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "表格",
            Self::Board => "看板",
            Self::List => "列表",
            Self::Gallery => "画廊",
            Self::Calendar => "日历",
            Self::Timeline => "时间轴",
            Self::Form => "表单",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseView {
    pub id: String,
    pub name: String,
    pub kind: ViewKind,
    pub group_by: String,
    pub date_field: String,
    pub visible_fields: Vec<String>,
    pub filter: Option<Filter>,
    pub sorts: Vec<SortRule>,
    pub column_widths: BTreeMap<String, f32>,
    pub aggregates: BTreeMap<String, AggregateKind>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SortRule {
    pub field: String,
    pub descending: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Filter {
    All {
        filters: Vec<Filter>,
    },
    Any {
        filters: Vec<Filter>,
    },
    Rule {
        field: String,
        operator: FilterOperator,
        value: CellValue,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    #[default]
    Equals,
    NotEquals,
    Contains,
    NotContains,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Empty,
    NotEmpty,
    StartsWith,
}

impl FilterOperator {
    pub const ALL: [Self; 11] = [
        Self::Equals,
        Self::NotEquals,
        Self::Contains,
        Self::NotContains,
        Self::Greater,
        Self::GreaterOrEqual,
        Self::Less,
        Self::LessOrEqual,
        Self::Empty,
        Self::NotEmpty,
        Self::StartsWith,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Equals => "等于",
            Self::NotEquals => "不等于",
            Self::Contains => "包含",
            Self::NotContains => "不包含",
            Self::Greater => "大于",
            Self::GreaterOrEqual => "大于或等于",
            Self::Less => "小于",
            Self::LessOrEqual => "小于或等于",
            Self::Empty => "为空",
            Self::NotEmpty => "不为空",
            Self::StartsWith => "开头是",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeDatabase {
    pub fields: Vec<DatabaseField>,
    pub views: Vec<DatabaseView>,
    pub default_view_id: String,
}

/// A query projection, never a second persistent copy of a note.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageRecord {
    pub id: String,
    pub title: String,
    pub content: String,
    pub meta: KnowledgePage,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted: bool,
    pub encrypted: bool,
}

impl CellValue {
    pub fn text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(s) | Self::Select(s) | Self::Url(s) => s.clone(),
            Self::Number(n) => {
                if *n == 0.0 {
                    "0".into()
                } else {
                    n.to_string()
                }
            }
            Self::MultiSelect(values) | Self::Relation(values) => values.join(", "),
            Self::Checkbox(v) => if *v { "是" } else { "否" }.to_string(),
            Self::Date(value) => {
                if value.end.is_empty() || value.end == value.start {
                    value.start.clone()
                } else {
                    format!("{} → {}", value.start, value.end)
                }
            }
        }
    }
    pub fn empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Text(s) | Self::Select(s) | Self::Url(s) => s.trim().is_empty(),
            Self::MultiSelect(v) | Self::Relation(v) => v.is_empty(),
            Self::Date(v) => v.start.is_empty(),
            Self::Number(_) | Self::Checkbox(_) => false,
        }
    }
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Number(v) if v.is_finite() => Some(*v),
            Self::Checkbox(v) => Some(if *v { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Number(v) => v.is_finite(),
            Self::Text(v) | Self::Select(v) | Self::Url(v) => v.len() <= 1_000_000,
            Self::MultiSelect(v) | Self::Relation(v) => {
                v.len() <= 10_000 && v.iter().all(|s| s.len() <= 10_000)
            }
            Self::Date(v) => {
                (v.start.is_empty() && v.end.is_empty())
                    || (date_day(&v.start).is_some()
                        && (v.end.is_empty()
                            || date_day(&v.end)
                                .is_some_and(|end| end >= date_day(&v.start).unwrap())))
            }
            _ => true,
        }
    }
}

impl KnowledgePage {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != KNOWLEDGE_FORMAT_VERSION {
            return Err("知识页面版本不受支持".into());
        }
        if self.icon.chars().count() > 32
            || self.cover.len() > 8_192
            || self.tags.len() > 256
            || self.properties.len() > 1_024
            || self.comments.len() > 10_000
        {
            return Err("知识页面超出容量限制".into());
        }
        if !self.properties.values().all(CellValue::valid) {
            return Err("页面属性包含无效值".into());
        }
        if self
            .comments
            .iter()
            .any(|c| c.id.is_empty() || c.body.len() > 1_000_000)
            || !unique(self.comments.iter().map(|c| c.id.as_str()))
        {
            return Err("页面批注编号无效".into());
        }
        if let Some(database) = &self.database {
            database.validate()?;
        }
        Ok(())
    }
}

impl KnowledgeBlock {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != KNOWLEDGE_FORMAT_VERSION {
            return Err("知识块版本不受支持".into());
        }
        if !(2..=4).contains(&self.columns)
            || self.column > 3
            || self.language.len() > 64
            || self.url.len() > 8_192
            || self.table.len() > 10_000
        {
            return Err("知识块属性无效".into());
        }
        let width = self.table.first().map_or(0, Vec::len);
        if width > 128
            || self
                .table
                .iter()
                .any(|row| row.len() != width || row.iter().any(|cell| cell.len() > 1_000_000))
        {
            return Err("表格行列不一致或超出容量限制".into());
        }
        Ok(())
    }
}

impl KnowledgeDatabase {
    pub fn validate(&self) -> Result<(), String> {
        if self.fields.len() > 256 || self.views.is_empty() || self.views.len() > 64 {
            return Err("数据库字段或视图数量无效".into());
        }
        if !unique(self.fields.iter().map(|f| f.id.as_str()))
            || !unique(self.views.iter().map(|v| v.id.as_str()))
            || self.fields.iter().any(|f| {
                f.id.is_empty()
                    || f.id == "title"
                    || f.name.trim().is_empty()
                    || f.options.len() > 1_000
                    || !unique(f.options.iter().map(String::as_str))
                    || f.options.iter().any(|option| option.trim().is_empty())
                    || f.formula.len() > 32_768
            })
            || self
                .views
                .iter()
                .any(|v| v.id.is_empty() || v.name.trim().is_empty())
        {
            return Err("数据库编号或名称无效".into());
        }
        if !self.views.iter().any(|v| v.id == self.default_view_id) {
            return Err("默认视图不存在".into());
        }
        let known_field = |id: &str| id == "title" || self.fields.iter().any(|f| f.id == id);
        for view in &self.views {
            if view
                .visible_fields
                .iter()
                .chain(view.column_widths.keys())
                .chain(view.aggregates.keys())
                .any(|id| !known_field(id))
                || view.sorts.iter().any(|sort| !known_field(&sort.field))
                || (!view.group_by.is_empty() && !known_field(&view.group_by))
                || (!view.date_field.is_empty()
                    && !self
                        .fields
                        .iter()
                        .any(|f| f.id == view.date_field && f.kind == FieldKind::Date))
            {
                return Err("视图引用了不存在或类型不匹配的属性".into());
            }
            if view.sorts.len() > 16
                || view.visible_fields.len() > 256
                || view
                    .column_widths
                    .values()
                    .any(|v| !v.is_finite() || !(48.0..=2000.0).contains(v))
            {
                return Err("视图设置无效".into());
            }
            if let Some(filter) = &view.filter {
                if !valid_filter(filter, 0) || !filter_fields_valid(filter, &known_field) {
                    return Err("筛选条件无效或嵌套过深".into());
                }
            }
        }
        Ok(())
    }
    pub fn task_database() -> Self {
        let fields = vec![
            DatabaseField {
                id: "status".into(),
                name: "状态".into(),
                kind: FieldKind::Status,
                options: vec!["未开始".into(), "进行中".into(), "已完成".into()],
                ..Default::default()
            },
            DatabaseField {
                id: "date".into(),
                name: "日期".into(),
                kind: FieldKind::Date,
                ..Default::default()
            },
            DatabaseField {
                id: "priority".into(),
                name: "优先级".into(),
                kind: FieldKind::Select,
                options: vec!["高".into(), "中".into(), "低".into()],
                ..Default::default()
            },
        ];
        let views = ViewKind::ALL
            .iter()
            .enumerate()
            .map(|(i, kind)| DatabaseView {
                id: format!("view-{i}"),
                name: kind.label().into(),
                kind: *kind,
                group_by: "status".into(),
                date_field: "date".into(),
                visible_fields: fields.iter().map(|f| f.id.clone()).collect(),
                ..Default::default()
            })
            .collect();
        Self {
            fields,
            views,
            default_view_id: "view-0".into(),
        }
    }
}

fn unique<'a>(values: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = HashSet::new();
    values.into_iter().all(|value| seen.insert(value))
}

fn filter_fields_valid(filter: &Filter, known: &impl Fn(&str) -> bool) -> bool {
    match filter {
        Filter::All { filters } | Filter::Any { filters } => {
            filters.iter().all(|f| filter_fields_valid(f, known))
        }
        Filter::Rule { field, .. } => known(field),
    }
}

fn valid_filter(filter: &Filter, depth: usize) -> bool {
    if depth > 12 {
        return false;
    }
    match filter {
        Filter::All { filters } | Filter::Any { filters } => {
            filters.len() <= 64 && filters.iter().all(|f| valid_filter(f, depth + 1))
        }
        Filter::Rule { field, value, .. } => !field.is_empty() && value.valid(),
    }
}

/// Strict Gregorian day keys; invalid dates never silently roll into next month.
pub fn date_day(value: &str) -> Option<i64> {
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
        || !parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let year: i64 = parts[0].parse().ok()?;
    let month: i64 = parts[1].parse().ok()?;
    let day: i64 = parts[2].parse().ok()?;
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=max).contains(&day) {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yo = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    Some(era * 146097 + yo * 365 + yo / 4 - yo / 100 + doy - 719468)
}

pub fn day_date(day: i64) -> String {
    let z = day + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn can_move_page(records: &[PageRecord], page_id: &str, parent_id: Option<&str>) -> bool {
    if !records
        .iter()
        .any(|p| p.id == page_id && !p.deleted && !p.encrypted && !p.meta.locked)
    {
        return false;
    }
    let mut parent = parent_id;
    let mut seen = HashSet::new();
    while let Some(id) = parent {
        if id == page_id || !seen.insert(id) {
            return false;
        }
        let Some(record) = records
            .iter()
            .find(|p| p.id == id && !p.deleted && !p.encrypted)
        else {
            return false;
        };
        if record.meta.locked {
            return false;
        }
        parent = record.meta.parent_id.as_deref();
    }
    true
}

pub fn ancestors(records: &[PageRecord], page_id: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::from([page_id]);
    let mut id = records
        .iter()
        .find(|p| p.id == page_id)
        .and_then(|p| p.meta.parent_id.as_deref());
    while let Some(parent) = id {
        if !seen.insert(parent) {
            break;
        }
        let Some(record) = records
            .iter()
            .find(|p| p.id == parent && !p.deleted && !p.encrypted)
        else {
            break;
        };
        result.push(parent.to_string());
        id = record.meta.parent_id.as_deref();
    }
    result.reverse();
    result
}

pub fn safe_web_url(value: &str) -> bool {
    !value.chars().any(char::is_control)
        && url::Url::parse(value).is_ok_and(|u| {
            matches!(u.scheme(), "https" | "http")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        })
}

#[cfg(test)]
#[path = "knowledge_model_tests.rs"]
mod tests;

/// Search and plain-text exports share the same structural meaning. Paragraph
/// text stays byte-for-byte intact while the user is typing.
pub fn block_plaintext(meta: Option<&KnowledgeBlock>, text: &str) -> String {
    let Some(meta) = meta else {
        return text.into();
    };
    match meta.kind {
        BlockKind::Todo => format!("- [{}] {}", if meta.checked { "x" } else { " " }, text),
        BlockKind::Heading1 => format!("# {text}"),
        BlockKind::Heading2 => format!("## {text}"),
        BlockKind::Heading3 => format!("### {text}"),
        BlockKind::BulletedList => format!("- {text}"),
        BlockKind::NumberedList => format!("1. {text}"),
        BlockKind::Quote => text
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        BlockKind::Table => meta
            .table
            .iter()
            .map(|row| row.join(" | "))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => text.into(),
    }
}
