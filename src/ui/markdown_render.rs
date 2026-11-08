//! 面向阅读的 markdown 渲染器：把 markdown 源码渲染成带样式的行（吃掉标记）。
//!
//! 这是**面向阅读的渲染**（隐藏 `#`、`**`、``` 等标记），与编辑器用的
//! `ui/markdown.rs` 高亮（保留标记上色）是两回事，故独立实现。
//! UI 查看器的 agent 回答样例与 lx 聊天历史共用本模块，保证 markdown 观感一致。
//!
//! 支持全部常见 markdown 格式：六级标题、段落、无序/有序/嵌套列表、任务列表、
//! 引用、围栏/缩进代码块、表格（对齐）、水平线、图片，以及行内
//! 粗体/斜体/粗斜体/删除线/行内代码/链接/自动链接/裸 URL。
//!
//! 采用 lx 直角风格：标题隐藏 `#`、列表用 `•`、引用用 `>`（不用左边框）、
//! 代码块做轻量语法高亮；表格用方框网格；任务列表复用 claude 一组复选框
//! （`[x]`/`[ ]`）。

use ratatui::style::Style;

use crate::ui::style;

mod code_block;
mod highlight;
mod incremental;
mod table;

pub use code_block::{CodeLineKind, DEFAULT_CODE_LANG, extract_code_block};
/// 轻量词法高亮一行代码（文件差异卡片复用同一套分色，避免第二份实现）。
pub(crate) use highlight::highlight_line;
/// 流式 Markdown 增量渲染状态（稳定前缀 + 活跃尾部）。
pub(crate) use incremental::IncrementalMarkdownState;
pub use table::{
    TableCellSlice, TableColumnSpan, TableLayoutMeta, TableLineKind, TableLineRef, TableLogicalRow,
};

/// 渲染出的一行：左缩进列 + 多色分段（拼接即该行纯文本）。
#[derive(Debug, Clone, PartialEq)]
pub struct MdLine {
    pub indent: u16,
    /// 悬挂缩进：软换行时**续行**相对首行额外缩进的列数（列表/引用的标记宽度）。
    /// 首行含 `• ` / `1. ` / `> ` 等标记，续行要跳过标记、对齐正文起始列。
    pub hang: u16,
    pub spans: Vec<(String, Style)>,
    /// 若该行属于 Markdown 表格，记录其表格结构与行内偏移。
    pub table: Option<TableLineRef>,
    /// 若该行属于启用了边框与复制按钮的代码块，记录其代码块行角色。
    pub code: Option<CodeLineKind>,
}

impl MdLine {
    /// 单色单段行。
    pub(crate) fn plain(indent: u16, value: impl Into<String>, style: Style) -> Self {
        Self {
            indent,
            hang: 0,
            spans: vec![(value.into(), style)],
            table: None,
            code: None,
        }
    }

    /// 多段行。
    pub(crate) fn spans(indent: u16, spans: Vec<(String, Style)>) -> Self {
        Self {
            indent,
            hang: 0,
            spans,
            table: None,
            code: None,
        }
    }

    /// 携带表格结构引用的多段行。
    pub(crate) fn with_table(
        indent: u16,
        spans: Vec<(String, Style)>,
        table: TableLineRef,
    ) -> Self {
        Self {
            indent,
            hang: 0,
            spans,
            table: Some(table),
            code: None,
        }
    }

    /// 多段行 + 悬挂缩进（列表项/引用：续行对齐正文起始列）。
    pub(crate) fn hanging(indent: u16, hang: u16, spans: Vec<(String, Style)>) -> Self {
        Self {
            indent,
            hang,
            spans,
            table: None,
            code: None,
        }
    }

    /// 该行的显示宽度（缩进 + 各分段宽度）。
    pub fn width(&self) -> usize {
        usize::from(self.indent)
            + self
                .spans
                .iter()
                .map(|(value, _)| display_width(value))
                .sum::<usize>()
    }

    /// 该行的纯文本（不含缩进）。
    pub fn text(&self) -> String {
        self.spans.iter().map(|(value, _)| value.as_str()).collect()
    }
}

/// 把 markdown 源码渲染成带样式的行（任意来源；不附加 token 页脚）。
///
/// 调用方负责换行与宽度裁剪；本函数只做块级结构还原与行内标记折叠。
pub fn render(source: &str) -> Vec<MdLine> {
    render_blocks(source, None, true, false)
}

/// 把 markdown 源码渲染成**恰好不超过 `width` 列**的行（块级：表格自适应宽度、其余软换行）。
///
/// 与 [`render`] 共用块级解析，但表格按可用宽度收缩列宽并在单元格内换行
/// （对齐 opencode 的 markdown 表格），避免窄宽度下把方框网格硬折断成「变形」；
/// 其余块（段落/列表/引用/代码）按词优先软换行，超长不可断的内容按字符切分。
pub fn render_wrapped(source: &str, width: u16) -> Vec<MdLine> {
    render_wrapped_with_options(source, width, true, false)
}

/// 与 [`render_wrapped`] 相同，但为代码块启用内部第一行 `<lang> ... [❐]` 头部与左侧 `▎` 单栏。
///
/// 供 `lx` 聊天历史的 AI 助手正文块（主对话与子 Agent 详情页）使用；
/// 思考块与工具/汇报卡片内的压暗 Markdown 仍用 [`render_wrapped`] 或 [`render_wrapped_literal_rules`]。
pub fn render_wrapped_code_blocks(source: &str, width: u16) -> Vec<MdLine> {
    render_wrapped_with_options(source, width, true, true)
}

/// 与 [`render_wrapped`] 相同，但水平分割线（`---` / `***` / `___`）保持原始文本而不拉满整行 `─` 横线。
///
/// 供工具卡片展开内容、子 agent 汇报与 compact 摘要使用：输出中的 `---` 保持原样，不画成整行边框线；
/// 思考块仍用 [`render_wrapped`]，主回答正文用 [`render_wrapped_code_blocks`]。
pub fn render_wrapped_literal_rules(source: &str, width: u16) -> Vec<MdLine> {
    render_wrapped_with_options(source, width, false, false)
}

fn render_wrapped_with_options(
    source: &str,
    width: u16,
    expand_rules: bool,
    code_chrome: bool,
) -> Vec<MdLine> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for line in render_blocks(source, Some(width), expand_rules, code_chrome) {
        wrap_md_line(&line, width, &mut out);
    }
    out
}

/// 块级解析：`width` 为 Some 时表格按可用宽度收缩（其余块仍按自然宽度产出，由调用方换行）。
fn render_blocks(
    source: &str,
    width: Option<usize>,
    expand_rules: bool,
    code_chrome: bool,
) -> Vec<MdLine> {
    render_blocks_stateful(
        source,
        width,
        expand_rules,
        code_chrome,
        &mut ParserState::default(),
    )
}

/// 块级解析的跨行状态：列表缩进档位栈 + 未闭合围栏语言 + 代码块序号。
///
/// 增量渲染的「稳定前缀 / 活跃尾部」两段共用它：尾部从中断处的状态继续解析，
/// 保证与一次性全量渲染逐行一致。
#[derive(Debug, Clone, Default)]
struct ParserState {
    /// 列表缩进档位栈：把源码缩进列数换算成层级，跨行维护，非列表行清空（空行不清，
    /// 允许松散列表）。这样 2 空格（`- ` 对齐）与 3 空格（`1. ` 对齐）混用也能得到均匀层间距。
    list_indents: Vec<usize>,
    /// `Some(lang)` 表示位于未闭合围栏代码块内（空串 = 围栏未写语言）。
    open_fence_lang: Option<String>,
    /// 下一个代码块的 0-based 序号（围栏与 4 空格缩进代码块统一递增）。
    next_code_block_index: usize,
    /// 上一个块是代码块：下一个块前需补一行空白间隔（跨增量分块保留）。
    after_code_block: bool,
}

/// 从给定跨行状态继续块级解析，返回新产出的行。
fn render_blocks_stateful(
    source: &str,
    width: Option<usize>,
    expand_rules: bool,
    code_chrome: bool,
    state: &mut ParserState,
) -> Vec<MdLine> {
    let mut lines = Vec::new();
    let source_lines: Vec<&str> = source.split('\n').collect();
    let mut index = 0;
    while index < source_lines.len() {
        let line = source_lines[index];
        // 未闭合围栏内部：每行都是独立代码行（闭合行只推进状态）。
        if let Some(lang) = state.open_fence_lang.take() {
            if line.trim_start().starts_with("```") {
                if code_chrome {
                    state.after_code_block = true;
                }
                index += 1;
                continue;
            }
            code_block::render_code_line(
                line,
                (!lang.is_empty()).then_some(lang.as_str()),
                code_chrome,
                &mut lines,
            );
            state.open_fence_lang = Some(lang);
            index += 1;
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        // 代码块之后与下一个块之间固定空一行（跨增量分块由 `after_code_block` 保留）。
        if code_chrome && state.after_code_block {
            lines.push(MdLine::spans(0, Vec::new()));
            state.after_code_block = false;
        }
        let item = list_item(line);
        if item.is_none() {
            state.list_indents.clear();
        }
        if trimmed.trim_start().starts_with("```") {
            let lang = line.trim().trim_start_matches('`').trim();
            let block_index = state.next_code_block_index;
            state.next_code_block_index += 1;
            if code_chrome {
                lines.push(code_block::top_border_line(width));
                lines.push(code_block::header_line(lang, block_index, width));
            }
            state.open_fence_lang = Some(lang.to_string());
            index += 1;
            continue;
        }
        // 缩进列表项优先于缩进代码块：模型常用 4 空格嵌套（`- xxx` 下接 `    - aaa`），
        // 若按 CommonMark 的 4 空格规则当成代码块，整段会被渲染成一坨 `▎ - aaa` 贴最左，
        // 列表看起来就“没有缩进”。真代码块请用围栏（```）。
        if let Some(item) = item {
            let depth = list_depth(&mut state.list_indents, item.indent());
            lines.push(item.render(depth));
            index += 1;
            continue;
        }
        if is_indented_code(line) {
            let block_index = state.next_code_block_index;
            state.next_code_block_index += 1;
            index = indented_code(
                &source_lines,
                index,
                width,
                block_index,
                code_chrome,
                &mut lines,
            );
            if code_chrome {
                state.after_code_block = true;
            }
            continue;
        }
        if let Some((level, body)) = heading(trimmed) {
            lines.push(heading_line(level, body));
            index += 1;
            continue;
        }
        if is_rule(trimmed) {
            if expand_rules {
                let rule_width = width.unwrap_or(40).max(1);
                lines.push(MdLine::plain(
                    0,
                    RULE.repeat(rule_width),
                    style::md_read_rule(),
                ));
            } else {
                lines.push(MdLine::plain(0, trimmed, Style::default()));
            }
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with('|') {
            index = table::table(&source_lines, index, &mut lines, width);
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            index = blockquote(&source_lines, index, &mut lines);
            continue;
        }
        if let Some((alt, url)) = image(trimmed) {
            lines.push(MdLine::spans(
                0,
                vec![
                    (alt.to_string(), style::text()),
                    (" ".to_string(), style::text()),
                    (format!("({url})"), style::md_read_image()),
                ],
            ));
            index += 1;
            continue;
        }
        // 普通段落行：行内渲染。
        lines.push(MdLine::spans(0, inline_spans(trimmed)));
        index += 1;
    }
    lines
}

/// 水平线字符。
const RULE: &str = "─";
/// 引用块前缀标记。
const QUOTE_MARKER: &str = ">";
/// 任务列表复选框（与 claude 一组一致；已完成 / 未完成）。
const TASK_DONE: &str = "[x]";
const TASK_OPEN: &str = "[ ]";

/// ATX 标题：`#{1,6}` 加后随空白；返回（级别、正文）。
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    let spaces = rest.bytes().take_while(|b| *b == b' ').count();
    (spaces > 0).then(|| (hashes as u8, rest[spaces..].trim_end()))
}

/// 渲染一行标题：隐藏 `#` 标记，正文用分级样式。
fn heading_line(level: u8, body: &str) -> MdLine {
    MdLine::plain(0, body, style::md_read_heading_level(level))
}

/// 水平线：仅由同一种 `-`/`*`/`_` 组成且至少 3 个。
fn is_rule(line: &str) -> bool {
    let mut symbol = None;
    let mut count = 0;
    for ch in line.chars() {
        match ch {
            ' ' | '\t' => {}
            '-' | '*' | '_' => {
                if symbol.is_some_and(|current| current != ch) {
                    return false;
                }
                symbol = Some(ch);
                count += 1;
            }
            _ => return false,
        }
    }
    count >= 3
}

/// 缩进代码块行：4 个以上前导空格且非空（列表项已在调用前按列表处理，走不到这里）。
fn is_indented_code(line: &str) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    // 列表项不算缩进代码：块级分派已优先按列表处理，这里同时兜住 `indented_code` 的
    // 续行判断——紧跟在代码行后的 `- 子项` 形态的行会提前收尾代码块，不再漏成 `▎ - 子项`。
    indent >= 4 && !line.trim().is_empty() && list_item(line).is_none()
}

/// 图片 `![alt](url)`。
fn image(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("![")?;
    let alt_close = rest.find(']')?;
    let after = rest.get(alt_close + 1..)?;
    let after = after.strip_prefix('(')?;
    let url_close = after.find(')')?;
    Some((&rest[..alt_close], &after[..url_close]))
}

/// 缩进代码块：收集连续 4 空格缩进行。
fn indented_code(
    source: &[&str],
    start: usize,
    width: Option<usize>,
    block_index: usize,
    code_chrome: bool,
    lines: &mut Vec<MdLine>,
) -> usize {
    let mut index = start;
    let mut body = Vec::new();
    while index < source.len()
        && (is_indented_code(source[index]) || source[index].trim().is_empty())
    {
        if source[index].trim().is_empty() {
            // 段内空行保留，但行尾空行不追加。
            if index + 1 < source.len() && is_indented_code(source[index + 1]) {
                body.push("");
                index += 1;
                continue;
            }
            break;
        }
        // 去掉 4 空格缩进。
        body.push(source[index].strip_prefix("    ").unwrap_or(source[index]));
        index += 1;
    }
    if code_chrome {
        lines.push(code_block::top_border_line(width));
        lines.push(code_block::header_line("", block_index, width));
    }
    for line in &body {
        code_block::render_code_line(line, None, code_chrome, lines);
    }
    index
}

/// 引用块：收集连续 `>` 行，用 `>` 作前缀（不用左边框），正文支持行内 markdown 折叠与着色。
fn blockquote(source: &[&str], start: usize, lines: &mut Vec<MdLine>) -> usize {
    let mut index = start;
    while index < source.len() {
        let trimmed = source[index].trim_start();
        let Some(rest) = trimmed.strip_prefix('>') else {
            break;
        };
        let body = rest.strip_prefix(' ').unwrap_or(rest);
        let marker = format!("{QUOTE_MARKER} ");
        let quote_style = style::md_read_quote();
        let mut spans = vec![(marker.clone(), quote_style)];
        for (value, span_style) in inline_spans(body) {
            spans.push((value, quote_style.patch(span_style)));
        }
        lines.push(MdLine::hanging(0, marker_width(&marker), spans));
        index += 1;
    }
    index
}

/// 列表项（无序 / 有序 / 任务）；`indent` 是源码里的**原始缩进列数**，
/// 层级由 [`list_depth`] 按实际出现的缩进档位换算。
#[derive(Debug, Clone, Copy)]
enum ListItem<'a> {
    Bullet {
        indent: usize,
        text: &'a str,
    },
    Ordered {
        indent: usize,
        number: &'a str,
        text: &'a str,
    },
    Task {
        indent: usize,
        done: bool,
        text: &'a str,
    },
}

impl ListItem<'_> {
    /// 源码缩进列数。
    fn indent(&self) -> usize {
        match *self {
            Self::Bullet { indent, .. }
            | Self::Ordered { indent, .. }
            | Self::Task { indent, .. } => indent,
        }
    }

    /// 渲染一行列表项（lx：`•` 项目符号；任务用 claude 一组 `[x]`/`[ ]`）；
    /// `depth` 为换算后的层级（0 = 顶层）。
    fn render(&self, depth: usize) -> MdLine {
        match *self {
            Self::Bullet { text: body, .. } => {
                let marker = "• ";
                MdLine::hanging(
                    indent(depth),
                    marker_width(marker),
                    marker_spans(marker, style::md_read_bullet(), body),
                )
            }
            Self::Ordered {
                number, text: body, ..
            } => {
                let marker = format!("{number}. ");
                MdLine::hanging(
                    indent(depth),
                    marker_width(&marker),
                    marker_spans(&marker, style::md_read_ordered(), body),
                )
            }
            Self::Task {
                done, text: body, ..
            } => {
                // 复选框按状态分色，**正文保持正常样式**：已完成项不再整行加删除线
                // 或压暗——任务状态只由复选框表达，文字仍要能正常阅读。
                let marker = if done {
                    (format!("{TASK_DONE} "), style::md_read_task_done())
                } else {
                    (format!("{TASK_OPEN} "), style::md_read_task_open())
                };
                let mut spans = vec![(marker.0.clone(), marker.1)];
                spans.extend(inline_spans(body));
                MdLine::hanging(indent(depth), marker_width(&marker.0), spans)
            }
        }
    }
}

/// 列表项内容：标记分段 + 行内渲染的正文（粗体/斜体/代码等在列表内同样折叠）。
fn marker_spans(marker: &str, marker_style: Style, body: &str) -> Vec<(String, Style)> {
    let mut spans = vec![(marker.to_string(), marker_style)];
    spans.extend(inline_spans(body));
    spans
}

/// 列表项缩进（每级 2 列）。
fn indent(depth: usize) -> u16 {
    (depth * 2) as u16
}

/// 把源码缩进列数换算成层级：维护“已出现缩进档位”的栈——
/// 相同档位 = 同级；更深一档 = 下一级；更浅则回退到匹配的档位。
fn list_depth(stack: &mut Vec<usize>, indent: usize) -> usize {
    while let Some(last) = stack.last().copied() {
        if last == indent {
            break;
        }
        if last < indent {
            stack.push(indent);
            break;
        }
        stack.pop();
    }
    if stack.is_empty() {
        stack.push(indent);
    }
    stack.len() - 1
}

/// 标记的显示宽度（悬挂缩进列数）。
fn marker_width(marker: &str) -> u16 {
    u16::try_from(display_width(marker)).unwrap_or(u16::MAX)
}

/// 解析列表项；非列表返回 None。
fn list_item(line: &str) -> Option<ListItem<'_>> {
    let spaces = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[spaces..];
    // 任务列表：`- [ ] ` / `- [x] `。
    if let Some(after) = rest.strip_prefix("- [") {
        let done = after.starts_with('x') || after.starts_with('X');
        let marker = after.get(1..)?.strip_prefix("] ")?;
        return Some(ListItem::Task {
            indent: spaces,
            done,
            text: marker.trim(),
        });
    }
    // 无序列表。
    if let Some(after) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        return Some(ListItem::Bullet {
            indent: spaces,
            text: after.trim(),
        });
    }
    // 有序列表 `N. `。
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && rest.as_bytes().get(digits) == Some(&b'.') {
        let after = rest.get(digits + 1..)?.strip_prefix(' ')?;
        return Some(ListItem::Ordered {
            indent: spaces,
            number: &rest[..digits],
            text: after.trim(),
        });
    }
    None
}

/// 行内渲染：吃掉粗体/斜体/粗斜体/删除线/行内代码标记；链接显示 `文字 (url)`。
fn inline_spans(text: &str) -> Vec<(String, Style)> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let prev = text[..index].chars().next_back();
        let rest = &text[index..];
        // 行内代码：`code`（内部不再解析）。
        if let Some(inner) = delimited(rest, "`", "`") {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::md_read_inline_code()));
            index += inner.len() + 2;
            continue;
        }
        // 粗斜体 `***…***`。
        if let Some(inner) = delimited_emphasis(rest, "***", prev) {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::md_read_strong_emphasis()));
            index += inner.len() + 6;
            continue;
        }
        // 粗体 `**…**` / `__…__`。
        if let Some(inner) =
            delimited_emphasis(rest, "**", prev).or_else(|| delimited_emphasis(rest, "__", prev))
        {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::md_read_strong()));
            index += inner.len() + 4;
            continue;
        }
        // 删除线 `~~…~~`。
        if let Some(inner) = delimited_emphasis(rest, "~~", prev) {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::md_read_strikethrough()));
            index += inner.len() + 4;
            continue;
        }
        // 斜体 `*…*` / `_…_`（排除双写与词内下划线）。
        if let Some(inner) = emphasis(rest, prev) {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::md_read_emphasis()));
            index += inner.len() + 2;
            continue;
        }
        // 链接 `[文字](url)`。
        if rest.starts_with('[')
            && let Some((label, url, consumed)) = link(rest)
        {
            flush(&mut plain, &mut spans);
            spans.push((label.to_string(), style::md_read_link_text()));
            spans.push((format!(" ({url})"), style::md_read_url()));
            index += consumed;
            continue;
        }
        // 自动链接 `<url>`。
        if rest.starts_with('<')
            && let Some(close) = rest.find('>')
        {
            let url = &rest[1..close];
            if url.starts_with("http://") || url.starts_with("https://") {
                flush(&mut plain, &mut spans);
                spans.push((url.to_string(), style::md_read_url()));
                index += close + 1;
                continue;
            }
        }
        // 裸 URL。
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            flush(&mut plain, &mut spans);
            spans.push((rest[..end].to_string(), style::md_read_url()));
            index += end;
            continue;
        }
        let ch = rest.chars().next().unwrap_or('\0');
        plain.push(ch);
        index += ch.len_utf8();
    }
    flush(&mut plain, &mut spans);
    spans
}

/// 把累积的普通文本刷入 spans（正文色）。
fn flush(plain: &mut String, spans: &mut Vec<(String, Style)>) {
    if !plain.is_empty() {
        spans.push((std::mem::take(plain), style::text()));
    }
}

/// 匹配 `open … close` 并返回内部文本（需非空）。
fn delimited<'a>(rest: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let after = rest.strip_prefix(open)?;
    let end = after.find(close)?;
    (end > 0).then(|| &after[..end])
}

/// 匹配强调类对称标记（`***` / `**` / `__` / `~~` / `*` / `_`）：
/// - 遵循 CommonMark 规则：开标记后与闭标记前均不得紧邻空白（防止 `2 * 3 * 4` 或 `*.rs *.toml` 误配对）；
/// - 下划线标记（`_` / `__`）不得在 ASCII 字母或数字词内开闭（防止 `exec_command / write_stdin` 吞掉 `_`）。
fn delimited_emphasis<'a>(rest: &'a str, delim: &str, prev: Option<char>) -> Option<&'a str> {
    let delim_char = delim.chars().next()?;
    let is_underscore = delim_char == '_';
    if is_underscore && prev.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return None;
    }
    let after = rest.strip_prefix(delim)?;
    let first = after.chars().next()?;
    if first == delim_char || first.is_whitespace() {
        return None;
    }
    let mut search_from = 0;
    while let Some(pos) = after[search_from..].find(delim) {
        let end = search_from + pos;
        let inner = &after[..end];
        let tail = &after[end + delim.len()..];
        let before_close = inner.chars().next_back();
        let after_close = tail.chars().next();
        let valid_before = before_close.is_some_and(|ch| !ch.is_whitespace() && ch != delim_char);
        let valid_after = after_close
            .is_none_or(|ch| ch != delim_char && (!is_underscore || !ch.is_ascii_alphanumeric()));
        if valid_before && valid_after {
            return Some(inner);
        }
        search_from = end + delim.len();
    }
    None
}

/// 斜体 `*…*` / `_…_`；排除双写前缀与词内下划线。
fn emphasis(rest: &str, prev: Option<char>) -> Option<&str> {
    let ch = rest.chars().next()?;
    if ch != '*' && ch != '_' {
        return None;
    }
    delimited_emphasis(rest, if ch == '*' { "*" } else { "_" }, prev)
}

/// 链接 `[文字](url)`；返回（文字、URL、消耗字节数）。
fn link(rest: &str) -> Option<(&str, &str, usize)> {
    let after = rest.strip_prefix('[')?;
    let label_close = after.find(']')?;
    let url_start = after.get(label_close + 1..)?.strip_prefix('(')?;
    let url_close = url_start.find(')')?;
    let consumed = 1 + label_close + 1 + 1 + url_close + 1;
    Some((&after[..label_close], &url_start[..url_close], consumed))
}

/// 字符串的显示宽度（按字符宽度求和）。
pub fn display_width(value: &str) -> usize {
    value.chars().map(|ch| char_width(ch) as usize).sum()
}

/// 渲染行的近似字节数（二级缓存 LRU 记账用）。
pub(crate) fn md_bytes(lines: &[MdLine]) -> usize {
    lines
        .iter()
        .map(|line| {
            std::mem::size_of::<MdLine>()
                + line
                    .spans
                    .iter()
                    .map(|(text, _)| text.len() + 24)
                    .sum::<usize>()
        })
        .sum()
}

/// 把一条渲染行按可用宽度软换行（保留缩进；续行缩进到内容起始列）。
///
/// 优先在空白/汉字边界断行（对齐 opencode/主流 agent 的 word wrap），
/// 单个超宽字符单独成行；供表格外的段落/列表/引用/代码使用。
fn wrap_md_line(line: &MdLine, width: usize, out: &mut Vec<MdLine>) {
    if matches!(
        line.code,
        Some(CodeLineKind::TopBorder | CodeLineKind::Header { .. })
    ) {
        out.push(line.clone());
        return;
    }
    if line.code == Some(CodeLineKind::Body) {
        // 扣掉左侧 `▎` 的 1 列。
        let body_width = width.saturating_sub(1).max(1);
        let start_len = out.len();
        wrap_md_line_inner(line, body_width, out);
        for wrapped in &mut out[start_len..] {
            let spans = std::mem::take(&mut wrapped.spans);
            wrapped.spans = code_block::prepend_body_bar(spans, width);
            wrapped.code = Some(CodeLineKind::Body);
        }
        return;
    }
    wrap_md_line_inner(line, width, out);
}

fn wrap_md_line_inner(line: &MdLine, width: usize, out: &mut Vec<MdLine>) {
    let first_indent = usize::from(line.indent);
    let hang = usize::from(line.hang);
    let cont_indent = first_indent.saturating_add(hang);
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|(value, span_style)| value.chars().map(|ch| (ch, *span_style)))
        .collect();
    if cells.is_empty() {
        // 代码块里的空行也是内容：软换行不能把它吞掉（否则代码上下文断开）。
        out.push(MdLine::spans(line.indent, Vec::new()));
        return;
    }
    let mut index = 0usize;
    let mut first = true;
    while index < cells.len() {
        // 首行含标记（`• ` / `1. ` / `> `），续行用悬挂缩进对齐正文起始列。
        let indent = if first { first_indent } else { cont_indent };
        let avail = width.saturating_sub(indent).max(1);
        // 适应 `avail` 列的最大结束下标。
        let mut col = 0usize;
        let mut end = index;
        while end < cells.len() {
            let cell = char_width(cells[end].0) as usize;
            if col + cell > avail {
                break;
            }
            col += cell;
            end += 1;
        }
        if end == index {
            end = index + 1; // 单字符超宽：单独成行。
        }
        // 未到行尾时，回退到最近一个空白之后断行（词优先）。
        let mut break_at = end;
        if end < cells.len() {
            for probe in (index..end).rev() {
                if cells[probe].0.is_whitespace() {
                    break_at = probe + 1;
                    break;
                }
            }
        }
        // 首行不能断在标记内部（否则出现 `•` 独占一行的孤儿标记）：
        // 断点没越过标记宽度时改为按列硬切。
        if first && break_at <= index.saturating_add(hang) {
            break_at = end;
        }
        let mut spans: Vec<(String, Style)> = Vec::new();
        for (ch, span_style) in &cells[index..break_at] {
            if let Some(last) = spans.last_mut()
                && last.1 == *span_style
            {
                last.0.push(*ch);
                continue;
            }
            spans.push((ch.to_string(), *span_style));
        }
        trim_trailing_whitespace(&mut spans);
        let indent_u16 = u16::try_from(indent).unwrap_or(u16::MAX);
        if first
            && break_at == cells.len()
            && let Some(ref table) = line.table
        {
            out.push(MdLine::with_table(indent_u16, spans, table.clone()));
        } else {
            out.push(MdLine::spans(indent_u16, spans));
        }
        index = break_at;
        first = false;
        // 跳过下一物理行的行首空白。
        while index < cells.len() && cells[index].0 == ' ' {
            index += 1;
        }
    }
}

/// 去掉行尾空白分段（软换行产生的悬挂空格）。
fn trim_trailing_whitespace(spans: &mut Vec<(String, Style)>) {
    while let Some((value, _)) = spans.last() {
        let trimmed = value.trim_end();
        if trimmed.len() == value.len() {
            break;
        }
        if trimmed.is_empty() {
            spans.pop();
        } else {
            let trimmed = trimmed.to_string();
            if let Some(last) = spans.last_mut() {
                last.0 = trimmed;
            }
            break;
        }
    }
}

/// 单个字符的显示宽度。
fn char_width(symbol: char) -> u16 {
    use unicode_width::UnicodeWidthChar;
    symbol.width().unwrap_or(1).min(2) as u16
}

#[cfg(test)]
mod tests;
