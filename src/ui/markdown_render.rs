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
mod inline;
mod table;

pub use code_block::{CodeLineKind, DEFAULT_CODE_LANG, extract_code_block};
/// 轻量词法高亮一行代码（文件差异卡片复用同一套分色，避免第二份实现）。
pub(crate) use highlight::highlight_line;
/// 流式 Markdown 增量渲染状态（稳定前缀 + 活跃尾部）。
pub(crate) use incremental::IncrementalMarkdownState;
pub use inline::MdLinkSpan;
#[cfg(test)]
use inline::inline_spans;
use inline::{heading_spans_and_links, marker_spans_and_links, spans_and_links};
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
    /// 行内 `http(s)://` 超链接在当前行内的列区间（相对 `indent` 之后的正文起点）。
    pub links: Vec<MdLinkSpan>,
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
            links: Vec::new(),
            table: None,
            code: None,
        }
    }

    /// 多段行。
    pub(crate) fn spans(indent: u16, spans: Vec<(String, Style)>) -> Self {
        Self::spans_with_links(indent, spans, Vec::new())
    }

    /// 携带超链接列区间的多段行。
    pub(crate) fn spans_with_links(
        indent: u16,
        spans: Vec<(String, Style)>,
        links: Vec<MdLinkSpan>,
    ) -> Self {
        Self {
            indent,
            hang: 0,
            spans,
            links,
            table: None,
            code: None,
        }
    }

    /// 携带表格结构引用与超链接列区间的多段行。
    pub(crate) fn with_table_and_links(
        indent: u16,
        spans: Vec<(String, Style)>,
        links: Vec<MdLinkSpan>,
        table: TableLineRef,
    ) -> Self {
        Self {
            indent,
            hang: 0,
            spans,
            links,
            table: Some(table),
            code: None,
        }
    }

    /// 多段行 + 悬挂缩进 + 超链接列区间（列表项/引用：续行对齐正文起始列）。
    pub(crate) fn hanging_with_links(
        indent: u16,
        hang: u16,
        spans: Vec<(String, Style)>,
        links: Vec<MdLinkSpan>,
    ) -> Self {
        Self {
            indent,
            hang,
            spans,
            links,
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

/// 列表项种类：用于区分无序、有序与任务列表，同级切换列表类型时视作新列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Ordered,
    Task,
}

/// 上一个已产出块的种类（供 `code_chrome == true` 的块间距状态机判定是否插入前导空行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevBlock {
    /// 普通段落或图片行；`in_list` 表示该行是否作为列表项内的缩进续段，`depth` 为其所属列表层级，
    /// `loose` 表示该列表内续段是否由空行（`\n\n`）引导。
    Paragraph {
        in_list: bool,
        depth: usize,
        loose: bool,
    },
    /// 标题行（`#`..`######`）。
    Heading,
    /// 列表项（记录换算后的层级 `depth` 与列表种类 `kind`）。
    ListItem { depth: usize, kind: ListKind },
    /// 围栏代码块或 4 空格缩进代码块。
    CodeBlock,
    /// 表格块。
    Table,
    /// 引用块（`>`）。
    Quote,
    /// 水平分割线（`---` / `***` / `___`）。
    Rule,
}

/// 块级解析的跨行状态：列表缩进档位栈 + 未闭合围栏语言 + 代码块序号 + 块间距前驱状态。
///
/// 增量渲染的「稳定前缀 / 活跃尾部」两段共用它：尾部从中断处的状态继续解析，
/// 保证与一次性全量渲染逐行一致。
#[derive(Debug, Clone, Default)]
struct ParserState {
    /// 列表缩进档位栈：把源码缩进列数换算成层级，跨行维护，非列表行清空（空行不清，
    /// 允许松散列表）。这样 2 空格（`- ` 对齐）与 3 空格（`1. ` 对齐）混用也能得到均匀层间距。
    list_indents: Vec<usize>,
    /// 与 `list_indents` 平行的列表项内容起始列栈（最内层为 last）。
    /// 用于区分“列表续段”和真正的 4 空格缩进代码块。
    list_content_indents: Vec<usize>,
    /// 与 `list_indents` 平行的列表项标记悬挂宽度栈（无序 `2`、有序 `N. ` 宽度、任务 `4`）。
    list_hangs: Vec<u16>,
    /// `Some(lang)` 表示位于未闭合围栏代码块内（空串 = 围栏未写语言）。
    open_fence_lang: Option<String>,
    /// 当前未闭合围栏 opener 的反引号数量；关闭行必须不少于该数量。
    open_fence_ticks: usize,
    /// 下一个代码块的 0-based 序号（围栏与 4 空格缩进代码块统一递增）。
    next_code_block_index: usize,
    /// 上一个已产出的块种类（跨增量分块保留；`None` 表示尚未产出任何块）。
    prev_block: Option<PrevBlock>,
    /// 自上一个块产出后，源码中是否至少出现过一条空行（跨增量分块保留）。
    saw_blank_line: bool,
}

/// 按行切分源码：末尾 `\n` 是行终止符，不额外产生一条空行；真正的空行
/// （`\n\n`）仍保留。空源码返回零行。
fn split_source_lines(source: &str) -> Vec<&str> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = source.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// 在产出下一个非空块 `curr` 之前，按块间距规则推入至多 1 行空行并推进 `state`。
fn transition_block(
    state: &mut ParserState,
    curr: PrevBlock,
    code_chrome: bool,
    lines: &mut Vec<MdLine>,
) {
    if code_chrome && should_insert_blank_line(state.prev_block, state.saw_blank_line, curr) {
        lines.push(MdLine::spans(0, Vec::new()));
    }
    state.prev_block = Some(curr);
    state.saw_blank_line = false;
}

/// 判定前驱块 `prev` 与当前块 `curr` 之间是否需要插入 1 行空行。
fn should_insert_blank_line(
    prev: Option<PrevBlock>,
    saw_blank_line: bool,
    curr: PrevBlock,
) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    match curr {
        // 代码块自带 `▂` 顶边（上半截 7/8 留白），仅在连续两个代码块之间补 1 行空行。
        PrevBlock::CodeBlock => prev == PrevBlock::CodeBlock,
        // 标题、表格、分割线作为独立顶层块，上方恒隔 1 行。
        PrevBlock::Heading | PrevBlock::Table | PrevBlock::Rule => true,
        // 引用块：连续 `>` 行（无空行分隔）同属一个引用块，其余情况上方隔 1 行。
        PrevBlock::Quote => prev != PrevBlock::Quote || saw_blank_line,
        // 列表项：区分进入列表、父进子、子退父、同级异类切换与松紧列表。
        PrevBlock::ListItem { depth, kind } => match prev {
            PrevBlock::ListItem {
                depth: prev_depth,
                kind: prev_kind,
            } => {
                if depth > prev_depth {
                    false
                } else if saw_blank_line || depth < prev_depth {
                    true
                } else {
                    prev_kind != kind
                }
            }
            PrevBlock::Paragraph {
                in_list: true,
                depth: prev_depth,
                loose,
            } => {
                if depth > prev_depth {
                    false
                } else {
                    saw_blank_line || loose || depth < prev_depth
                }
            }
            _ => true,
        },
        // 段落或图片行：区分顶层段落、列表内紧凑续行、列表内松散续段与退出列表。
        PrevBlock::Paragraph { in_list, depth, .. } => match prev {
            PrevBlock::Heading
            | PrevBlock::CodeBlock
            | PrevBlock::Table
            | PrevBlock::Quote
            | PrevBlock::Rule => true,
            PrevBlock::ListItem { .. } => saw_blank_line || !in_list,
            PrevBlock::Paragraph {
                in_list: prev_in_list,
                depth: prev_depth,
                ..
            } => saw_blank_line || (prev_in_list && (!in_list || depth != prev_depth)),
        },
    }
}

/// 清空列表缩进栈（进入标题、分割线、表格、引用块等独立顶层块时调用）。
fn clear_list_context(state: &mut ParserState) {
    state.list_indents.clear();
    state.list_content_indents.clear();
    state.list_hangs.clear();
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
    let source_lines = split_source_lines(source);
    let mut index = 0;
    while index < source_lines.len() {
        let line = source_lines[index];
        // 未闭合围栏内部：每行都是独立代码行（闭合行只推进状态）。
        if let Some(lang) = state.open_fence_lang.take() {
            if fence_ticks(line).is_some_and(|ticks| ticks >= state.open_fence_ticks) {
                state.prev_block = Some(PrevBlock::CodeBlock);
                state.saw_blank_line = false;
                state.open_fence_ticks = 0;
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
            if state.prev_block.is_some() {
                state.saw_blank_line = true;
            }
            index += 1;
            continue;
        }
        let item = list_item(line);
        update_list_context(state, item.as_ref(), line);
        let min_indent = list_code_min_indent(state);
        if let Some(ticks) = fence_ticks(line) {
            transition_block(state, PrevBlock::CodeBlock, code_chrome, &mut lines);
            let lang = line.trim().trim_start_matches('`').trim();
            let block_index = state.next_code_block_index;
            state.next_code_block_index += 1;
            if code_chrome {
                lines.push(code_block::top_border_line(width));
                lines.push(code_block::header_line(lang, block_index, width));
            }
            state.open_fence_lang = Some(lang.to_string());
            state.open_fence_ticks = ticks;
            index += 1;
            continue;
        }
        // 缩进列表项优先于缩进代码块：模型常用 4 空格嵌套（`- xxx` 下接 `    - aaa`），
        // 若按 CommonMark 的 4 空格规则当成代码块，整段会被渲染成一坨 `▎ - aaa` 贴最左，
        // 列表看起来就“没有缩进”。真代码块请用围栏（```）。
        if let Some(item) = item {
            let depth = list_depth(&mut state.list_indents, item.indent());
            transition_block(
                state,
                PrevBlock::ListItem {
                    depth,
                    kind: item.kind(),
                },
                code_chrome,
                &mut lines,
            );
            lines.push(item.render(depth));
            index += 1;
            continue;
        }
        if is_indented_code(line, min_indent) {
            transition_block(state, PrevBlock::CodeBlock, code_chrome, &mut lines);
            let block_index = state.next_code_block_index;
            state.next_code_block_index += 1;
            index = indented_code(
                &source_lines,
                index,
                width,
                block_index,
                code_chrome,
                min_indent,
                &mut lines,
            );
            continue;
        }
        if let Some((level, body)) = heading(trimmed) {
            clear_list_context(state);
            transition_block(state, PrevBlock::Heading, code_chrome, &mut lines);
            lines.push(heading_line(level, body));
            index += 1;
            continue;
        }
        if is_rule(trimmed) {
            clear_list_context(state);
            transition_block(state, PrevBlock::Rule, code_chrome, &mut lines);
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
            clear_list_context(state);
            transition_block(state, PrevBlock::Table, code_chrome, &mut lines);
            index = table::table(&source_lines, index, &mut lines, width);
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            clear_list_context(state);
            transition_block(state, PrevBlock::Quote, code_chrome, &mut lines);
            lines.push(blockquote_line(trimmed));
            index += 1;
            continue;
        }
        let in_list = !state.list_indents.is_empty();
        let depth = state.list_indents.len().saturating_sub(1);
        let loose = in_list
            && (state.saw_blank_line
                || matches!(
                    state.prev_block,
                    Some(PrevBlock::Paragraph {
                        in_list: true,
                        loose: true,
                        ..
                    })
                ));
        transition_block(
            state,
            PrevBlock::Paragraph {
                in_list,
                depth,
                loose,
            },
            code_chrome,
            &mut lines,
        );
        // 普通段落或图片行：行内渲染（若仍处于列表上下文内，续段按当前列表层级的正文列缩进）。
        let cont_indent = if !in_list {
            0
        } else {
            let hang = state.list_hangs.get(depth).copied().unwrap_or(2);
            indent(depth).saturating_add(hang)
        };
        let body = if cont_indent > 0 {
            trimmed.trim_start()
        } else {
            trimmed
        };
        if let Some((alt, url)) = image(body) {
            lines.push(MdLine::spans(
                cont_indent,
                vec![
                    (alt.to_string(), style::text()),
                    (" ".to_string(), style::text()),
                    (format!("({url})"), style::md_read_image()),
                ],
            ));
            index += 1;
            continue;
        }
        let (spans, links) = spans_and_links(body);
        lines.push(MdLine::spans_with_links(cont_indent, spans, links));
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
    let line = line.trim_start_matches(' ');
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    let spaces = rest.bytes().take_while(|b| *b == b' ').count();
    (spaces > 0).then(|| (hashes as u8, rest[spaces..].trim_end()))
}

/// 渲染一行标题：隐藏 `#` 标记，正文用分级样式（含网页超链接时保留链接热区与下划线）。
fn heading_line(level: u8, body: &str) -> MdLine {
    let (spans, links) = heading_spans_and_links(body, style::md_read_heading_level(level));
    MdLine::spans_with_links(0, spans, links)
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
fn is_indented_code(line: &str, min_indent: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    // 列表项不算缩进代码：块级分派已优先按列表处理，这里同时兜住 `indented_code` 的
    // 续行判断——紧跟在代码行后的 `- 子项` 形态的行会提前收尾代码块，不再漏成 `▎ - 子项`。
    indent >= min_indent && !line.trim().is_empty() && list_item(line).is_none()
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
    min_indent: usize,
    lines: &mut Vec<MdLine>,
) -> usize {
    let mut index = start;
    let mut body = Vec::new();
    while index < source.len()
        && (is_indented_code(source[index], min_indent) || source[index].trim().is_empty())
    {
        if source[index].trim().is_empty() {
            // 段内空行保留，但行尾空行不追加。
            if index + 1 < source.len() && is_indented_code(source[index + 1], min_indent) {
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

/// 引用行：用 `>` 作前缀（不用左边框），正文支持行内 markdown 折叠与着色。
fn blockquote_line(line: &str) -> MdLine {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('>').unwrap_or(trimmed);
    let body = rest.strip_prefix(' ').unwrap_or(rest);
    let marker = format!("{QUOTE_MARKER} ");
    let quote_style = style::md_read_quote();
    let (spans, links) = marker_spans_and_links(&marker, quote_style, body, Some(quote_style));
    MdLine::hanging_with_links(0, marker_width(&marker), spans, links)
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
    /// 列表项种类。
    fn kind(&self) -> ListKind {
        match *self {
            Self::Bullet { .. } => ListKind::Bullet,
            Self::Ordered { .. } => ListKind::Ordered,
            Self::Task { .. } => ListKind::Task,
        }
    }

    /// 源码缩进列数。
    fn indent(&self) -> usize {
        match *self {
            Self::Bullet { indent, .. }
            | Self::Ordered { indent, .. }
            | Self::Task { indent, .. } => indent,
        }
    }

    /// 列表项正文起始列（标记宽度 + 一个空格）。
    fn content_indent(&self) -> usize {
        match *self {
            Self::Bullet { indent, .. } => indent + 2,
            Self::Ordered { indent, number, .. } => indent + number.len() + 2,
            Self::Task { indent, .. } => indent + 6,
        }
    }

    /// 渲染后的列表项标记悬挂宽度（用于续段与换行对齐正文列）。
    fn hang_width(&self) -> u16 {
        match *self {
            Self::Bullet { .. } => marker_width("• "),
            Self::Ordered { number, .. } => marker_width(&format!("{number}. ")),
            Self::Task { .. } => marker_width(&format!("{TASK_OPEN} ")),
        }
    }

    /// 渲染一行列表项（lx：`•` 项目符号；任务用 claude 一组 `[x]`/`[ ]`）；
    /// `depth` 为换算后的层级（0 = 顶层）。
    fn render(&self, depth: usize) -> MdLine {
        let (marker, marker_style, body) = match *self {
            Self::Bullet { text: body, .. } => ("• ".to_string(), style::md_read_bullet(), body),
            Self::Ordered {
                number, text: body, ..
            } => (format!("{number}. "), style::md_read_ordered(), body),
            Self::Task {
                done, text: body, ..
            } => {
                // 复选框按状态分色，**正文保持正常样式**：已完成项不再整行加删除线
                // 或压暗——任务状态只由复选框表达，文字仍要能正常阅读。
                if done {
                    (format!("{TASK_DONE} "), style::md_read_task_done(), body)
                } else {
                    (format!("{TASK_OPEN} "), style::md_read_task_open(), body)
                }
            }
        };
        let (spans, links) = marker_spans_and_links(&marker, marker_style, body, None);
        MdLine::hanging_with_links(indent(depth), marker_width(&marker), spans, links)
    }
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

/// 维护列表上下文：列表项更新内容起始列栈；非列表行按缩进逐层回退，
/// 直到找到满足 `indent >= content_indent` 的外层列表（列表续段/列表内代码块保留层级）。
fn update_list_context(state: &mut ParserState, item: Option<&ListItem<'_>>, line: &str) {
    if let Some(item) = item {
        let depth = list_depth(&mut state.list_indents, item.indent());
        state.list_content_indents.truncate(depth + 1);
        state.list_hangs.truncate(depth + 1);
        if state.list_content_indents.len() <= depth {
            state.list_content_indents.push(item.content_indent());
            state.list_hangs.push(item.hang_width());
        } else {
            state.list_content_indents[depth] = item.content_indent();
            state.list_hangs[depth] = item.hang_width();
        }
        return;
    }
    let indent = line.len() - line.trim_start_matches(' ').len();
    while state
        .list_content_indents
        .last()
        .is_some_and(|&content| indent < content)
    {
        state.list_indents.pop();
        state.list_content_indents.pop();
        state.list_hangs.pop();
    }
}

/// 围栏 opener/closer 的反引号数量；不足 3 个返回 `None`。
fn fence_ticks(line: &str) -> Option<usize> {
    let rest = line.trim_start();
    let ticks = rest.chars().take_while(|ch| *ch == '`').count();
    (ticks >= 3).then_some(ticks)
}

/// 当前列表上下文下，缩进代码块所需的最小列数；不在列表内为 4。
fn list_code_min_indent(state: &ParserState) -> usize {
    state
        .list_content_indents
        .last()
        .map_or(4, |content| content.saturating_add(4))
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
        // 宽度 1：只能显示左栏，禁止按字符拆成大量 `▎` 行。
        if width <= 1 {
            out.push(MdLine {
                indent: 0,
                hang: 0,
                spans: code_block::prepend_body_bar(Vec::new(), width),
                links: Vec::new(),
                table: None,
                code: Some(CodeLineKind::Body),
            });
            return;
        }
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
    let cells = inline::cell_links_for_line(&line.spans, &line.links);
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
        let (spans, links) = inline::slice_spans_and_links(&cells[index..break_at]);
        let indent_u16 = u16::try_from(indent).unwrap_or(u16::MAX);
        if first
            && break_at == cells.len()
            && let Some(ref table) = line.table
        {
            out.push(MdLine::with_table_and_links(
                indent_u16,
                spans,
                links,
                table.clone(),
            ));
        } else {
            out.push(MdLine::spans_with_links(indent_u16, spans, links));
        }
        index = break_at;
        first = false;
        // 跳过下一物理行的行首空白。
        while index < cells.len() && cells[index].0 == ' ' {
            index += 1;
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
