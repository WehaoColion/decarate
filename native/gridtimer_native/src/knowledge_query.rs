// v2.22.52 - Typed records, nested filters, stable sorting and relational calculations.
use super::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn parse_cell(field: &DatabaseField, input: &str) -> Result<CellValue, String> {
    let input = input.trim();
    if input.is_empty() {
        return if field.required {
            Err("此属性必填".into())
        } else {
            Ok(CellValue::Empty)
        };
    }
    let value = match field.kind {
        FieldKind::Text | FieldKind::Email | FieldKind::Phone => CellValue::Text(input.into()),
        FieldKind::Number => CellValue::Number(input.parse().map_err(|_| "请输入有效数字")?),
        FieldKind::Select | FieldKind::Status => CellValue::Select(input.into()),
        FieldKind::MultiSelect => CellValue::MultiSelect(
            input
                .split([',', '，'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        FieldKind::Date => {
            let dates = input.split_once(" → ").or_else(|| input.split_once(".."));
            let (start, end) = dates.unwrap_or((input, ""));
            CellValue::Date(DateRange {
                start: start.trim().into(),
                end: end.trim().into(),
            })
        }
        FieldKind::Checkbox => CellValue::Checkbox(match input.to_lowercase().as_str() {
            "true" | "1" | "是" | "完成" => true,
            "false" | "0" | "否" | "未完成" => false,
            _ => return Err("复选框需要是或否".into()),
        }),
        FieldKind::Url => CellValue::Url(input.into()),
        FieldKind::Relation => CellValue::Relation(
            input
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        _ => return Err("计算属性不能直接填写".into()),
    };
    validate_cell(field, &value)?;
    Ok(value)
}

pub fn validate_cell(field: &DatabaseField, value: &CellValue) -> Result<(), String> {
    if !value.valid() {
        return Err(format!("{} 的值无效", field.name));
    }
    if value.empty() {
        return if field.required {
            Err(format!("{} 必填", field.name))
        } else {
            Ok(())
        };
    }
    let valid = match field.kind {
        FieldKind::Text | FieldKind::Phone => matches!(value, CellValue::Text(_)),
        FieldKind::Email => {
            matches!(value,CellValue::Text(v) if v.split_once('@').is_some_and(|(a,b)| !a.is_empty() && b.contains('.') && !b.chars().any(char::is_whitespace)))
        }
        FieldKind::Number => matches!(value, CellValue::Number(_)),
        FieldKind::Select | FieldKind::Status => {
            matches!(value,CellValue::Select(v) if field.options.contains(v))
        }
        FieldKind::MultiSelect => {
            matches!(value,CellValue::MultiSelect(v) if v.iter().all(|s|field.options.contains(s)) && v.iter().collect::<HashSet<_>>().len()==v.len())
        }
        FieldKind::Date => matches!(value, CellValue::Date(_)),
        FieldKind::Checkbox => matches!(value, CellValue::Checkbox(_)),
        FieldKind::Url => matches!(value,CellValue::Url(v) if safe_web_url(v)),
        FieldKind::Relation => {
            matches!(value,CellValue::Relation(v) if v.iter().collect::<HashSet<_>>().len()==v.len())
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("{} 与属性类型或选项不匹配", field.name))
    }
}

pub fn validate_record(
    database: &KnowledgeDatabase,
    properties: &BTreeMap<String, CellValue>,
) -> Result<(), String> {
    database.validate()?;
    for field in database
        .fields
        .iter()
        .filter(|f| !f.deleted && !f.kind.computed())
    {
        validate_cell(
            field,
            properties.get(&field.id).unwrap_or(&CellValue::Empty),
        )?;
    }
    Ok(())
}

pub fn aggregate(values: &[CellValue], kind: AggregateKind) -> CellValue {
    let filled = values.iter().filter(|v| !v.empty()).collect::<Vec<_>>();
    let nums = filled.iter().filter_map(|v| v.number()).collect::<Vec<_>>();
    let n = match kind {
        AggregateKind::Count => filled.len() as f64,
        AggregateKind::Unique => filled
            .iter()
            .map(|v| v.text())
            .collect::<HashSet<_>>()
            .len() as f64,
        AggregateKind::Sum => nums.iter().sum(),
        AggregateKind::Average => {
            if nums.is_empty() {
                return CellValue::Empty;
            }
            nums.iter().sum::<f64>() / nums.len() as f64
        }
        AggregateKind::Minimum => {
            if nums.is_empty() {
                return CellValue::Empty;
            }
            nums.iter().copied().fold(f64::INFINITY, f64::min)
        }
        AggregateKind::Maximum => {
            if nums.is_empty() {
                return CellValue::Empty;
            }
            nums.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        }
        AggregateKind::CheckedPercent => {
            let checks = filled
                .iter()
                .filter_map(|v| match v {
                    CellValue::Checkbox(b) => Some(*b),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if checks.is_empty() {
                return CellValue::Empty;
            }
            100.0 * checks.iter().filter(|v| **v).count() as f64 / checks.len() as f64
        }
    };
    if n.is_finite() {
        CellValue::Number(n)
    } else {
        CellValue::Empty
    }
}

pub struct QueryEngine<'a> {
    records: &'a [PageRecord],
    index: HashMap<&'a str, usize>,
    cache: HashMap<(String, String), Result<CellValue, String>>,
    active: HashSet<(String, String)>,
    budget: usize,
    pub today: i64,
}
impl<'a> QueryEngine<'a> {
    pub fn new(records: &'a [PageRecord], today: i64) -> Self {
        Self {
            records,
            index: records
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.as_str(), i))
                .collect(),
            cache: HashMap::new(),
            active: HashSet::new(),
            budget: 250_000,
            today,
        }
    }
    pub fn value(&mut self, page_id: &str, field_id: &str) -> Result<CellValue, String> {
        let key = (page_id.to_string(), field_id.to_string());
        if let Some(value) = self.cache.get(&key) {
            return value.clone();
        }
        if self.budget == 0 || self.active.len() > 32 {
            return Err("关联计算量超出限制".into());
        }
        self.budget -= 1;
        if !self.active.insert(key.clone()) {
            return Err("公式或关联存在循环引用".into());
        }
        let result = self.compute(page_id, field_id);
        self.active.remove(&key);
        self.cache.insert(key, result.clone());
        result
    }
    fn compute(&mut self, page_id: &str, field_id: &str) -> Result<CellValue, String> {
        let Some(&i) = self.index.get(page_id) else {
            return Err("引用的页面不存在".into());
        };
        let page = &self.records[i];
        if page.encrypted || page.deleted {
            return Err("引用的页面已删除或加密".into());
        }
        if field_id == "title" {
            return Ok(CellValue::Text(page.title.clone()));
        }
        let database = page
            .meta
            .parent_id
            .as_deref()
            .and_then(|id| self.index.get(id))
            .map(|i| &self.records[*i])
            .filter(|p| !p.deleted && !p.encrypted)
            .and_then(|p| p.meta.database.as_ref());
        let Some(field) = database.and_then(|db| {
            db.fields
                .iter()
                .find(|f| !f.deleted && (f.id == field_id || f.name == field_id))
        }) else {
            return Err(format!("属性不存在：{field_id}"));
        };
        match field.kind {
            FieldKind::Formula => {
                let expression = field.formula.clone();
                let today = self.today;
                evaluate_formula(&expression, |name| self.value(page_id, name), today)
            }
            FieldKind::Rollup => {
                let relation = field.rollup_relation_field.clone();
                let target = field.rollup_target_field.clone();
                let kind = field.aggregate;
                let links = self.value(page_id, &relation)?;
                let ids = match links {
                    CellValue::Relation(ids) => ids,
                    CellValue::Empty => Vec::new(),
                    _ => return Err("汇总需要关联属性".into()),
                };
                let mut values = Vec::new();
                for id in ids {
                    if self
                        .index
                        .get(id.as_str())
                        .is_some_and(|i| self.records[*i].deleted || self.records[*i].encrypted)
                    {
                        continue;
                    }
                    values.push(self.value(&id, &target)?);
                }
                Ok(aggregate(&values, kind))
            }
            FieldKind::CreatedTime => Ok(CellValue::Number(page.created_at as f64)),
            FieldKind::UpdatedTime => Ok(CellValue::Number(page.updated_at as f64)),
            _ => Ok(page
                .meta
                .properties
                .get(&field.id)
                .cloned()
                .unwrap_or_default()),
        }
    }
    pub fn matches(&mut self, id: &str, filter: &Filter) -> bool {
        match filter {
            Filter::All { filters } => filters.iter().all(|f| self.matches(id, f)),
            Filter::Any { filters } => filters.iter().any(|f| self.matches(id, f)),
            Filter::Rule {
                field,
                operator,
                value,
            } => self
                .value(id, field)
                .is_ok_and(|cell| cell_matches(&cell, *operator, value)),
        }
    }
    pub fn rows(&mut self, database_id: &str, view: &DatabaseView) -> Vec<String> {
        let mut rows = self
            .records
            .iter()
            .filter(|p| {
                !p.deleted
                    && !p.encrypted
                    && !p.meta.template
                    && p.meta.parent_id.as_deref() == Some(database_id)
            })
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        if let Some(filter) = &view.filter {
            rows.retain(|id| self.matches(id, filter));
        }
        let sorts = view.sorts.clone();
        for id in &rows {
            for sort in &sorts {
                let _ = self.value(id, &sort.field);
            }
        }
        rows.sort_by(|a, b| {
            for sort in &sorts {
                let av = self
                    .cache
                    .get(&(a.clone(), sort.field.clone()))
                    .and_then(|v| v.as_ref().ok())
                    .cloned()
                    .unwrap_or_default();
                let bv = self
                    .cache
                    .get(&(b.clone(), sort.field.clone()))
                    .and_then(|v| v.as_ref().ok())
                    .cloned()
                    .unwrap_or_default();
                let order = compare_cells(&av, &bv);
                if !order.is_eq() {
                    return if sort.descending {
                        order.reverse()
                    } else {
                        order
                    };
                }
            }
            let pa = &self.records[self.index[a.as_str()]];
            let pb = &self.records[self.index[b.as_str()]];
            pa.meta.order.cmp(&pb.meta.order).then_with(|| a.cmp(b))
        });
        rows
    }
}

pub fn compare_cells(a: &CellValue, b: &CellValue) -> Ordering {
    match (a.empty(), b.empty()) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    match (a, b) {
        (CellValue::Number(a), CellValue::Number(b)) => a.total_cmp(b),
        (CellValue::Checkbox(a), CellValue::Checkbox(b)) => a.cmp(b),
        (CellValue::Date(a), CellValue::Date(b)) => a.start.cmp(&b.start),
        _ => a.text().to_lowercase().cmp(&b.text().to_lowercase()),
    }
}
pub fn cell_matches(cell: &CellValue, op: FilterOperator, value: &CellValue) -> bool {
    if op == FilterOperator::Empty {
        return cell.empty();
    }
    if op == FilterOperator::NotEmpty {
        return !cell.empty();
    }
    let equal = match (cell, value) {
        (CellValue::MultiSelect(a), CellValue::Select(b) | CellValue::Text(b))
        | (CellValue::Relation(a), CellValue::Text(b)) => a.contains(b),
        _ => compare_cells(cell, value).is_eq(),
    };
    let contains = match cell {
        CellValue::MultiSelect(a) | CellValue::Relation(a) => a
            .iter()
            .any(|s| s.to_lowercase() == value.text().to_lowercase()),
        _ => cell
            .text()
            .to_lowercase()
            .contains(&value.text().to_lowercase()),
    };
    match op {
        FilterOperator::Equals => equal,
        FilterOperator::NotEquals => !equal,
        FilterOperator::Contains => contains,
        FilterOperator::NotContains => !contains,
        FilterOperator::StartsWith => cell
            .text()
            .to_lowercase()
            .starts_with(&value.text().to_lowercase()),
        _ if cell.empty() || value.empty() => false,
        FilterOperator::Greater => compare_cells(cell, value).is_gt(),
        FilterOperator::GreaterOrEqual => !compare_cells(cell, value).is_lt(),
        FilterOperator::Less => compare_cells(cell, value).is_lt(),
        FilterOperator::LessOrEqual => !compare_cells(cell, value).is_gt(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "knowledge_query_tests.rs"]
mod tests;
