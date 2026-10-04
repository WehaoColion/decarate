// v2.23.2.7 - Export ordinary answers and whitespace math loading fixtures offline.
use gridtimer_native::android_answer_render::render_android_ai_answer_html;
use std::{env, fs, path::PathBuf};

fn main() -> std::io::Result<()> {
    let output = PathBuf::from(env::args_os().nth(1).expect("fixture output directory"));
    fs::create_dir_all(&output)?;
    let sample = r#"复利就是“利息也会生利息”。每一期的利息会加入本金，成为下一期计算利息的基础。

**基本公式**

\[
A = P \times (1 + r)^n
\]

- \(A\)：最终金额
- \(P\)：本金
- \(r\)：每期利率，比如年利率 5% 写成 0.05
- \(n\)：期数

**举个例子**

本金 10000 元，年利率 5%，存 3 年，每年复利一次：

\[
A = 10000 \times (1 + 0.05)^3 = 11576.25
\]

## 更多公式

行内 $x_i^2 + y^2$，分数与根号：

$$\frac{a+b}{c} + \sqrt{x^2+1}$$

$$\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$$

$$\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}$$

代码中的 `\(x^2\)` 保持原样；美元金额 $100 和 $200 保持原样。

```latex
\[A=P(1+r)^n\]
```

| 项目 | 公式 |
| --- | --- |
| 平方 | $x^2$ |

## 无效公式局部降级

前文 \(\unknowncmd{x}\) 后文仍能阅读。

## 不可信内容

<script>window.answerInjected=true</script>

![外部图片](https://example.invalid/image.png)

\(\href{https://example.invalid/}{x}\)
"#;
    fs::write(output.join("source.md"), sample)?;
    write_preview(&output, "light.html", sample, false)?;
    write_preview(&output, "dark.html", sample, true)?;

    // Synthetic fixtures exercise the same trusted HTML used on Android. They
    // contain no account content, API key, API request, or saved model answer.
    let fixtures = [
        (
            "compound_interest",
            r#"复利就是“利息也会生利息”。每一期的利息会加入本金，成为下一期计算利息的基础。

**基本公式**

\[
A = P \times (1 + r)^n
\]

- \(A\)：最终金额
- \(P\)：本金
- \(r\)：每期利率，比如年利率 5% 写成 0.05
- \(n\)：期数，比如存 3 年就是 3

**举个例子**

本金 10000 元，年利率 5%，存 3 年，每年复利一次：

\[
A = 10000 \times (1 + 0.05)^3 = 11576.25
\]

因此，三年后的最终金额是 **11576.25 元**。
"#,
            6,
            0,
        ),
        (
            "one_plus_one_proof",
            r#"## 用自然数的定义证明一加一等于二

**1. 定义数字**

后继函数记作 \(S\)。定义 \(1=S(0)\)，\(2=S(S(0))\)。

**2. 定义加法**

加法的规则为 \(a+0=a\) 和 \(a+S(b)=S(a+b)\)。

**3. 按规则展开**

\[
\begin{aligned}
1+1 &= S(0)+S(0) \\
    &= S(S(0)+0) \\
    &= S(S(0)) \\
    &= 2
\end{aligned}
\]

所以：

\[
\boxed{1+1=2}
\]
"#,
            7,
            0,
        ),
        (
            "plain_iron_man",
            r#"## 钢铁侠是谁

**钢铁侠**是漫威作品中的超级英雄，名字是托尼·斯塔克。

他是一名工程师，依靠自己设计的装甲行动。装甲通常具备飞行、防护和工具系统，角色的核心特点是把工程知识用于解决问题。

### 可以从这些方面理解这个人物

- 他擅长发明和制造。
- 他会犯错，也需要承担自己的选择带来的后果。
- 不同漫画和电影中的设定有所不同。

这是一段普通回答，没有数学公式。最后一句也应完整显示。
"#,
            0,
            0,
        ),
        (
            "parenthesized_whitespace",
            r#"## 行内公式中的合法空白

带首尾空格的行内公式：\( x_i^2 \)。

模型把行内公式跨行输出时仍应显示：\(
r = 0.05
\)。

金额 $100 和 $200 应保持原文，代码 `\( x \)` 也应保持原文。
"#,
            2,
            0,
        ),
        (
            "mixed_markdown",
            r#"# 混合内容

普通段落、**加粗**和~~删除线~~。行内变量 \(x_i\)，平方 $x^2$。

1. 第一项
2. 第二项

> 引用内容中的公式仍可阅读。

\[
\sum_{i=1}^{n}i=\frac{n(n+1)}{2}
\]

$$\sqrt{x^2+1}$$

| 项目 | 内容 |
| --- | --- |
| 变量 | $y$ |
| 普通文字 | 保留中文 |

代码 `\(literal\)` 不应变成公式。

```latex
\[A=P(1+r)^n\]
```

美元金额 $100 和 $200 保持原样。

错误公式仅保留这一条原文：\(\unknowncmd{x}\)。后面的文字继续显示。

<script>window.answerInjected=true</script>

![外部图片](https://example.invalid/fixture.png)

[普通链接](https://example.invalid/)
"#,
            6,
            1,
        ),
    ];
    let mut cases = Vec::new();
    for (slug, source, expected_math_count, expected_math_errors) in fixtures {
        fs::write(output.join(format!("{slug}.md")), source)?;
        for dark in [false, true] {
            let theme = if dark { "dark" } else { "light" };
            let filename = format!("{slug}_{theme}.html");
            write_preview(&output, &filename, source, dark)?;
            cases.push(serde_json::json!({
                "file": filename,
                "source": format!("{slug}.md"),
                "theme": theme,
                "expectedMathCount": expected_math_count,
                "expectedMathErrors": expected_math_errors,
                "synthetic": true,
                "networkRequests": 0
            }));
        }
    }
    fs::write(
        output.join("fixture_manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({"cases": cases}))?,
    )?;
    Ok(())
}

fn write_preview(
    output: &std::path::Path,
    name: &str,
    source: &str,
    dark: bool,
) -> std::io::Result<()> {
    let background = if dark { "#16191c" } else { "#f7f8fa" };
    let shell = format!("<style>html{{background:{background}}}body{{max-width:396px;margin:0 auto;padding:18px;box-sizing:border-box}}</style></head>");
    let html = render_android_ai_answer_html(source, dark).replace("</head>", &shell);
    fs::write(output.join(name), html)?;
    Ok(())
}
