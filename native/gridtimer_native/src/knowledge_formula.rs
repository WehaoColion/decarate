// v2.22.52 - Bounded, typed database expressions with lazy conditionals.
use super::{date_day, day_date, CellValue};

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Field(String),
    Name(String),
    Symbol(String),
    End,
}
#[derive(Clone, Debug)]
enum Expr {
    Value(CellValue),
    Field(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

fn tokens(source: &str) -> Result<Vec<Token>, String> {
    if source.len() > 32768 {
        return Err("公式过长".into());
    }
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if out.len() >= 4096 {
            return Err("公式过于复杂".into());
        }
        if c == '"' || c == '\'' || c == '[' {
            let end = if c == '[' { ']' } else { c };
            i += 1;
            let mut value = String::new();
            let mut closed = false;
            while i < chars.len() {
                let c = chars[i];
                i += 1;
                if c == end {
                    closed = true;
                    break;
                }
                if c == '\\' && i < chars.len() {
                    let next = chars[i];
                    i += 1;
                    value.push(match next {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                } else {
                    value.push(c);
                }
            }
            if !closed {
                return Err("字符串或字段引用未闭合".into());
            }
            out.push(if c == '[' {
                Token::Field(value)
            } else {
                Token::Text(value)
            });
            continue;
        }
        if c.is_ascii_digit() || c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                i += 1;
                if i < chars.len() && matches!(chars[i], '+' | '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let n: f64 = chars[start..i]
                .iter()
                .collect::<String>()
                .parse()
                .map_err(|_| "数字格式错误")?;
            if !n.is_finite() {
                return Err("数字超出范围".into());
            }
            out.push(Token::Number(n));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Name(chars[start..i].iter().collect()));
            continue;
        }
        if i + 1 < chars.len() {
            let pair: String = chars[i..i + 2].iter().collect();
            if ["==", "!=", "<=", ">=", "&&", "||"].contains(&pair.as_str()) {
                out.push(Token::Symbol(pair));
                i += 2;
                continue;
            }
        }
        if "+-*/%(),!<>".contains(c) {
            out.push(Token::Symbol(c.to_string()));
            i += 1;
            continue;
        }
        return Err(format!("无法识别的字符：{c}"));
    }
    out.push(Token::End);
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}
impl Parser {
    fn symbol(&mut self, s: &str) -> bool {
        if self.tokens.get(self.pos) == Some(&Token::Symbol(s.into())) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expression(&mut self, min: u8, depth: usize) -> Result<Expr, String> {
        if depth > 48 {
            return Err("公式嵌套过深".into());
        }
        let token = self.tokens.get(self.pos).cloned().unwrap_or(Token::End);
        self.pos += 1;
        let mut left = match token {
            Token::Number(n) => Expr::Value(CellValue::Number(n)),
            Token::Text(s) => Expr::Value(CellValue::Text(s)),
            Token::Field(s) => Expr::Field(s),
            Token::Name(s) if s.eq_ignore_ascii_case("true") => {
                Expr::Value(CellValue::Checkbox(true))
            }
            Token::Name(s) if s.eq_ignore_ascii_case("false") => {
                Expr::Value(CellValue::Checkbox(false))
            }
            Token::Name(s) if s.eq_ignore_ascii_case("null") => Expr::Value(CellValue::Empty),
            Token::Name(s) => {
                if !self.symbol("(") {
                    return Err(format!("函数 {s} 缺少括号；字段请用 [字段名]"));
                }
                let mut args = Vec::new();
                if !self.symbol(")") {
                    loop {
                        args.push(self.expression(0, depth + 1)?);
                        if self.symbol(")") {
                            break;
                        }
                        if !self.symbol(",") {
                            return Err("函数参数缺少逗号".into());
                        }
                        if args.len() > 256 {
                            return Err("参数过多".into());
                        }
                    }
                }
                Expr::Call(s.to_ascii_lowercase(), args)
            }
            Token::Symbol(s) if s == "(" => {
                let value = self.expression(0, depth + 1)?;
                if !self.symbol(")") {
                    return Err("缺少右括号".into());
                }
                value
            }
            Token::Symbol(s) if ["!", "-", "+"].contains(&s.as_str()) => {
                Expr::Unary(s, Box::new(self.expression(7, depth + 1)?))
            }
            _ => return Err("公式缺少数值或字段".into()),
        };
        loop {
            let Some(Token::Symbol(op)) = self.tokens.get(self.pos) else {
                break;
            };
            let priority = match op.as_str() {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" => 3,
                "<" | ">" | "<=" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                _ => break,
            };
            if priority < min {
                break;
            }
            let op = op.clone();
            self.pos += 1;
            let right = self.expression(priority + 1, depth + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
}

fn numeric(v: &CellValue) -> Result<f64, String> {
    v.number().ok_or_else(|| "需要数字类型".into())
}
fn truth(v: &CellValue) -> bool {
    match v {
        CellValue::Checkbox(v) => *v,
        CellValue::Number(n) => *n != 0.0,
        other => !other.empty(),
    }
}
fn finite(n: f64) -> Result<CellValue, String> {
    if n.is_finite() {
        Ok(CellValue::Number(n))
    } else {
        Err("计算结果超出范围".into())
    }
}

fn evaluate(
    expr: &Expr,
    resolve: &mut impl FnMut(&str) -> Result<CellValue, String>,
    today: i64,
    steps: &mut usize,
    depth: usize,
) -> Result<CellValue, String> {
    if *steps == 0 || depth > 96 {
        return Err("公式计算量超出限制".into());
    }
    *steps -= 1;
    let mut ev = |v: &Expr| evaluate(v, resolve, today, steps, depth + 1);
    match expr {
        Expr::Value(v) => Ok(v.clone()),
        Expr::Field(s) => resolve(s),
        Expr::Unary(op, value) => {
            let v = ev(value)?;
            match op.as_str() {
                "!" => Ok(CellValue::Checkbox(!truth(&v))),
                "-" => finite(-numeric(&v)?),
                _ => finite(numeric(&v)?),
            }
        }
        Expr::Binary(op, a, b) => {
            let a = ev(a)?;
            if op == "&&" && !truth(&a) {
                return Ok(CellValue::Checkbox(false));
            }
            if op == "||" && truth(&a) {
                return Ok(CellValue::Checkbox(true));
            }
            let b = ev(b)?;
            match op.as_str() {
                "+" if matches!(a, CellValue::Text(_)) || matches!(b, CellValue::Text(_)) => {
                    let text = format!("{}{}", a.text(), b.text());
                    if text.len() > 1_000_000 {
                        Err("结果文本过长".into())
                    } else {
                        Ok(CellValue::Text(text))
                    }
                }
                "+" => finite(numeric(&a)? + numeric(&b)?),
                "-" => finite(numeric(&a)? - numeric(&b)?),
                "*" => finite(numeric(&a)? * numeric(&b)?),
                "/" | "%" => {
                    let n = numeric(&b)?;
                    if n == 0.0 {
                        return Err("除数不能为零".into());
                    }
                    finite(if op == "/" {
                        numeric(&a)? / n
                    } else {
                        numeric(&a)? % n
                    })
                }
                "&&" | "||" => Ok(CellValue::Checkbox(truth(&b))),
                _ => {
                    let cmp = match (a.number(), b.number()) {
                        (Some(a), Some(b)) => a.total_cmp(&b),
                        _ => a.text().cmp(&b.text()),
                    };
                    Ok(CellValue::Checkbox(match op.as_str() {
                        "==" => cmp.is_eq(),
                        "!=" => !cmp.is_eq(),
                        "<" => cmp.is_lt(),
                        ">" => cmp.is_gt(),
                        "<=" => !cmp.is_gt(),
                        ">=" => !cmp.is_lt(),
                        _ => false,
                    }))
                }
            }
        }
        Expr::Call(name, args) => {
            if name == "if" {
                if args.len() != 3 {
                    return Err("if 需要条件、成立值、不成立值".into());
                }
                let test = ev(&args[0])?;
                return ev(&args[if truth(&test) { 1 } else { 2 }]);
            }
            if name == "prop" {
                if args.len() != 1 {
                    return Err("prop 需要一个字段名".into());
                }
                let field = ev(&args[0])?.text();
                return resolve(&field);
            }
            let values = args.iter().map(&mut ev).collect::<Result<Vec<_>, _>>()?;
            let v = |i: usize| values.get(i).ok_or_else(|| "函数参数不足".to_string());
            match name.as_str() {
                "today" if values.is_empty() => Ok(CellValue::Text(day_date(today))),
                "empty" if values.len() == 1 => Ok(CellValue::Checkbox(v(0)?.empty())),
                "length" if values.len() == 1 => finite(match v(0)? {
                    CellValue::Relation(v) | CellValue::MultiSelect(v) => v.len() as f64,
                    other => other.text().chars().count() as f64,
                }),
                "contains" if values.len() == 2 => {
                    Ok(CellValue::Checkbox(v(0)?.text().contains(&v(1)?.text())))
                }
                "lower" | "upper" if values.len() == 1 => Ok(CellValue::Text(if name == "lower" {
                    v(0)?.text().to_lowercase()
                } else {
                    v(0)?.text().to_uppercase()
                })),
                "concat" => {
                    let text = values
                        .iter()
                        .map(CellValue::text)
                        .collect::<Vec<_>>()
                        .join("");
                    if text.len() > 1_000_000 {
                        Err("结果文本过长".into())
                    } else {
                        Ok(CellValue::Text(text))
                    }
                }
                "tonumber" if values.len() == 1 => {
                    finite(v(0)?.text().parse::<f64>().map_err(|_| "不能转换为数字")?)
                }
                "format" if values.len() == 1 => Ok(CellValue::Text(v(0)?.text())),
                "abs" if values.len() == 1 => finite(numeric(v(0)?)?.abs()),
                "round" if (1..=2).contains(&values.len()) => {
                    let decimals = values.get(1).map(numeric).transpose()?.unwrap_or(0.0);
                    if decimals.fract() != 0.0 || !(-8.0..=8.0).contains(&decimals) {
                        return Err("小数位需要是 -8 到 8 的整数".into());
                    }
                    let scale = 10_f64.powi(decimals as i32);
                    finite((numeric(v(0)?)? * scale).round() / scale)
                }
                "sum" | "min" | "max" | "average" if !values.is_empty() => {
                    let nums = values.iter().map(numeric).collect::<Result<Vec<_>, _>>()?;
                    finite(match name.as_str() {
                        "min" => nums.iter().copied().fold(f64::INFINITY, f64::min),
                        "max" => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                        "average" => nums.iter().sum::<f64>() / nums.len() as f64,
                        _ => nums.iter().sum(),
                    })
                }
                "datebetween" if values.len() == 2 => {
                    let day = |i| {
                        let text = match v(i)? {
                            CellValue::Date(d) => d.start.clone(),
                            other => other.text(),
                        };
                        date_day(&text).ok_or_else(|| "需要 YYYY-MM-DD 日期".to_string())
                    };
                    finite((day(0)? - day(1)?) as f64)
                }
                "dateadd" if values.len() == 2 => {
                    let text = match v(0)? {
                        CellValue::Date(d) => d.start.clone(),
                        other => other.text(),
                    };
                    let start = date_day(&text).ok_or("日期无效")?;
                    let days = numeric(v(1)?)?;
                    if days.fract() != 0.0 || days.abs() > 3_650_000.0 {
                        return Err("天数无效".into());
                    }
                    let result = day_date(start + days as i64);
                    if date_day(&result).is_none() {
                        return Err("日期超出范围".into());
                    }
                    Ok(CellValue::Text(result))
                }
                _ => Err(format!("函数 {name} 不存在或参数数量错误")),
            }
        }
    }
}

pub fn evaluate_formula(
    source: &str,
    mut resolve: impl FnMut(&str) -> Result<CellValue, String>,
    today: i64,
) -> Result<CellValue, String> {
    let mut parser = Parser {
        tokens: tokens(source)?,
        pos: 0,
    };
    let expression = parser.expression(0, 0)?;
    if parser.tokens.get(parser.pos) != Some(&Token::End) {
        return Err("公式末尾有多余内容".into());
    }
    evaluate(&expression, &mut resolve, today, &mut 8192, 0)
}
