//! `[UI]` 查看器 `agent` 分组的右侧预览：agent 步骤调用样式的四套候选。
//!
//! 每个 agent 步骤 item 在右区**纵向叠放四套候选样式**（`a` codex、`b` claude、
//! `c` opencode、`d` lx），每套上方一行 muted 标签（`a/b/c/d` + 风格短名），
//! 套间空 1 行。
//!
//! 四套风格均**无边框、走简洁风**，且**每一步都带 token 页脚**（对齐 lx-agent 的
//! flow item：`IN 1.2k · OUT 300 · CACHE 500`）。所有宽度以显示列计算，
//! 越界即截断，窄屏不 panic。本模块纯渲染，不读 `AppState` 领域数据。
//!
//! 设计：先把一套样例展开成 `Vec<SampleLine>`（缩进 + 多色分段），再由同一份
//! 结果驱动渲染与高度计算，避免「算的高度」与「画的内容」漂移。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::ui_gallery::{AgentStepId, UiGalleryComponentId};
use crate::ui::{style, text};

mod markdown;

/// 单个步骤样例的 token 用量（input、output、cache read）。
///
/// 与 lx-agent 一致用**数值 + [`format_tokens`]** 生成字符串；同一组数值会同时喂给
/// 该步的 token 页脚与 `tokens` 聚合 item 的汇总条，显示天然一致。
type Tokens = (u32, u32, u32);

/// 一套样例中的一行：左缩进列 + 多色分段。
struct SampleLine {
    indent: u16,
    spans: Vec<(String, Style)>,
}

impl SampleLine {
    /// 单色单段行。
    fn plain(indent: u16, value: impl Into<String>, style: Style) -> Self {
        Self {
            indent,
            spans: vec![(value.into(), style)],
        }
    }

    /// 多段行。
    fn spans(indent: u16, spans: Vec<(String, Style)>) -> Self {
        Self { indent, spans }
    }
}

/// 各套候选样式的标题标签（`a`/`b`/`c`/`d` + 风格短名）。
const VARIANT_LABELS: [(&str, &str); 4] = [
    ("a", text::agent::VARIANT_A),
    ("b", text::agent::VARIANT_B),
    ("c", text::agent::VARIANT_C),
    ("d", text::agent::VARIANT_D),
];

/// 组件 id 是否为 agent 步骤；是则返回步骤种类。
pub fn agent_step_of(id: UiGalleryComponentId) -> Option<AgentStepId> {
    match id {
        UiGalleryComponentId::Agent(step) => Some(step),
        _ => None,
    }
}

/// `agent` 步骤预览的自然尺寸：宽度贴合最长候选，高度按四套样例实际行数计算。
pub fn agent_appearance_size(step: AgentStepId, available: Rect) -> (u16, u16) {
    let width = 52u16.min(available.width);
    let height = (0..4)
        .map(|variant| variant_line_count(step, variant) as u16)
        .sum::<u16>()
        .min(available.height);
    (width, height)
}

/// 一套样例（标签行 + 内容行 + 套间空行）占用的行数。
fn variant_line_count(step: AgentStepId, variant: usize) -> usize {
    // 标签 1 行 + 内容 n 行 + 套间空 1 行。
    2 + variant_lines(step, variant).len()
}

/// 把第 `variant` 套（0=codex / 1=claude / 2=opencode / 3=lx）展开成若干行。
fn variant_lines(step: AgentStepId, variant: usize) -> Vec<SampleLine> {
    if step == AgentStepId::Tokens {
        return tokens_variant_lines(variant);
    }
    if step == AgentStepId::Markdown {
        return markdown::markdown_lines(variant, step_tokens(step));
    }
    let tokens = step_tokens(step);
    let mut lines = vec![header_line(step, variant)];
    // 子行（todo 用三态复选框；edit 用分色 diff 计数；其余用各风格连接符 + 文案）。
    if step == AgentStepId::Todo {
        lines.extend(todo_lines(variant));
    } else if step == AgentStepId::Edit {
        lines.extend(edit_diff_line(variant));
    } else {
        for body in body_lines(step) {
            lines.push(detail_line(variant, body));
        }
    }
    // subagent：只显示“进行中”的那一步（一个直角 icon）；token 仅底部一行总计。
    if step == AgentStepId::Subagent {
        lines.push(detail_line(variant, text::agent::SUBAGENT_IN_PROGRESS));
    }
    // 每一步都带 token 页脚（subagent 即该子任务的总用量）。
    lines.push(SampleLine::plain(
        2,
        format_tokens_line(tokens),
        style::agent_tokens(),
    ));
    lines
}

/// 头部行：四套各自的风格化头部；error 的标题整行红 + 加粗。
fn header_line(step: AgentStepId, variant: usize) -> SampleLine {
    let marker_style = status_style(step);
    // error：标题（动词与参数）红 + 加粗；其余用该套的常规标题样式。
    let title_style = if step == AgentStepId::Error {
        style::agent_fail()
    } else {
        match variant {
            0 | 1 => style::agent_step_detail(),
            _ => style::agent_step_name(),
        }
    };
    let detail = detail(step);
    let mut spans: Vec<(String, Style)> = Vec::new();
    match variant {
        0 => {
            // codex：`• Verb detail`
            spans.push((format!("{} ", text::agent::MARKER_BULLET), marker_style));
            spans.push((format!("{} {detail}", verb(step)), title_style));
        }
        1 => {
            // claude：`⏺ Name(detail)`
            spans.push((format!("{} ", text::agent::MARKER_CLAUDE_DOT), marker_style));
            spans.push((format!("{}({detail})", claude_name(step)), title_style));
        }
        2 => {
            // opencode：`<icon> Verb detail`
            spans.push((format!("{} ", opencode_icon(step)), marker_style));
            spans.push((format!("{} {detail}", verb(step)), title_style));
        }
        _ => {
            // lx：`<icon> Verb detail`
            spans.push((format!("{} ", lx_icon(step)), lx_icon_style(step)));
            spans.push((format!("{} {detail}", verb(step)), title_style));
        }
    }
    SampleLine::spans(0, spans)
}

/// 子行：各风格连接符 + 文案。
fn detail_line(variant: usize, body: &str) -> SampleLine {
    let (marker, body_style) = match variant {
        0 | 1 => (text::agent::MARKER_TREE_LEAF, style::agent_step_detail()),
        2 => (text::agent::MARKER_DETAIL, style::agent_step_detail()),
        _ => (text::agent::MARKER_TREE_LEAF, style::text()),
    };
    SampleLine::plain(0, format!("  {marker} {body}"), body_style)
}

/// edit 的子行：连接符 + 分色的 `+N` / `-M` 计数 + 路径，再一行 diff 正文。
fn edit_diff_line(variant: usize) -> Vec<SampleLine> {
    let marker = if variant == 2 {
        text::agent::MARKER_DETAIL
    } else {
        text::agent::MARKER_TREE_LEAF
    };
    vec![
        SampleLine::spans(
            0,
            vec![
                (format!("  {marker} "), style::agent_step_detail()),
                (text::agent::EDIT_DIFF_ADD.to_string(), style::agent_added()),
                (" ".to_string(), style::agent_step_detail()),
                (
                    text::agent::EDIT_DIFF_DEL.to_string(),
                    style::agent_removed(),
                ),
                ("  ".to_string(), style::agent_step_detail()),
                (
                    text::agent::EDIT_DIFF_PATH.to_string(),
                    style::agent_step_detail(),
                ),
            ],
        ),
        SampleLine::plain(4, text::agent::EDIT_BODY[0], style::text()),
    ]
}

/// todo 的三态复选框（各风格原生标记；done 用绿色 + 删除线、in-progress 用强调色、pending 用 muted）。
fn todo_lines(variant: usize) -> Vec<SampleLine> {
    let todo = |marker: &str, body: &str, marker_style: Style| -> SampleLine {
        SampleLine::spans(
            2,
            vec![
                (format!("{marker} "), marker_style),
                (body.to_string(), style::agent_step_detail()),
            ],
        )
    };
    let done_style = style::agent_ok().add_modifier(ratatui::style::Modifier::CROSSED_OUT);
    match variant {
        // codex：`✔` 完成、`□` 进行中（强调）、`□` 待办（muted）。
        0 => vec![
            todo(text::agent::TODO_DONE_CODEX, "add agent folder", done_style),
            todo(
                text::agent::TODO_PENDING_CODEX,
                "design variants",
                style::accent(),
            ),
            todo(
                text::agent::TODO_PENDING_CODEX,
                "write tests",
                style::agent_step_detail(),
            ),
        ],
        // claude：`✔` / `◼` / `◻`。
        1 => vec![
            todo(
                text::agent::TODO_DONE_CLAUDE,
                "add agent folder",
                done_style,
            ),
            todo(
                text::agent::TODO_INPROG_CLAUDE,
                "design variants",
                style::accent(),
            ),
            todo(
                text::agent::TODO_PENDING_CLAUDE,
                "write tests",
                style::agent_step_detail(),
            ),
        ],
        // opencode / lx：`[✓]` / `[•]` / `[ ]`。
        _ => vec![
            todo(
                text::agent::TODO_DONE_BRACKET,
                "add agent folder",
                done_style,
            ),
            todo(
                text::agent::TODO_INPROG_BRACKET,
                "design variants",
                style::accent(),
            ),
            todo(
                text::agent::TODO_PENDING_BRACKET,
                "write tests",
                style::agent_step_detail(),
            ),
        ],
    }
}

/// `tokens` 聚合 item 的第 `variant` 套：回合汇总条 + 上下文 pill。
fn tokens_variant_lines(variant: usize) -> Vec<SampleLine> {
    let tokens = step_tokens(AgentStepId::Tokens);
    let mut lines = Vec::new();
    match variant {
        0 => {
            // codex：`• Ran 3 tools · 12k tokens` + `IN · OUT · CACHE` 明细。
            lines.push(SampleLine::spans(
                0,
                vec![
                    (
                        format!("{} ", text::agent::MARKER_BULLET),
                        style::agent_ok(),
                    ),
                    (
                        text::agent::TOKENS_BAR_A.to_string(),
                        style::agent_step_name(),
                    ),
                ],
            ));
            lines.push(SampleLine::plain(
                2,
                format_tokens_line(tokens),
                style::agent_tokens(),
            ));
        }
        1 => {
            // claude：`⏺ 3 tool uses · 12.4k tokens` + `(↓ · 1.0k tokens) · took 4.5s`
            lines.push(SampleLine::spans(
                0,
                vec![
                    (
                        format!("{} ", text::agent::MARKER_CLAUDE_DOT),
                        style::agent_ok(),
                    ),
                    (
                        text::agent::TOKENS_BAR_B.to_string(),
                        style::agent_step_name(),
                    ),
                ],
            ));
            lines.push(SampleLine::plain(
                2,
                text::agent::TOKENS_BAR_B_SUB,
                style::agent_step_detail(),
            ));
        }
        2 => {
            // opencode：`⚙ 3 calls · 12,400 tokens · 42% used` + `context · 42% used · $0.12`
            lines.push(SampleLine::spans(
                0,
                vec![
                    (
                        format!("{} ", text::agent::OPEN_ICON_TODO),
                        style::agent_step_name(),
                    ),
                    (
                        text::agent::TOKENS_BAR_C.to_string(),
                        style::agent_step_name(),
                    ),
                ],
            ));
            lines.push(SampleLine::plain(
                2,
                text::agent::TOKENS_BAR_C_SUB,
                style::agent_step_detail(),
            ));
        }
        _ => {
            // lx：`3 calls · IN 12k · OUT 312 · CACHE 8.1k` + 三枚上下文 pill。
            lines.push(SampleLine::plain(
                0,
                format!(
                    "{}{}",
                    text::agent::TOKENS_BAR_D_LEAD,
                    format_tokens_line(tokens)
                ),
                style::agent_step_name(),
            ));
            for percent in [
                text::agent::TOKENS_PILL_GREEN,
                text::agent::TOKENS_PILL_YELLOW,
                text::agent::TOKENS_PILL_RED,
            ] {
                let pill = format!("{} {percent}%", text::agent::TOKENS_PILL_LABEL);
                lines.push(SampleLine::spans(
                    2,
                    vec![
                        (
                            format!("{} ", text::agent::MARKER_DOT),
                            style::agent_context_dot(percent),
                        ),
                        (pill, style::agent_step_detail()),
                    ],
                ));
            }
        }
    }
    lines
}

/// 在 `rect` 内渲染 agent 步骤的四套候选样式（无边框、简洁风）。
pub fn render_agent_appearance(buf: &mut Buffer, rect: Rect, step: AgentStepId) {
    if rect.width < 6 || rect.height < 4 {
        return;
    }
    let mut y = rect.y;
    for (variant, (letter, label)) in VARIANT_LABELS.iter().enumerate() {
        if y >= rect.bottom() {
            break;
        }
        put_spans(
            buf,
            rect.x,
            y,
            rect.right(),
            &[(format!("{letter} {label}"), style::agent_step_detail())],
        );
        y = y.saturating_add(1);
        for line in variant_lines(step, variant) {
            if y >= rect.bottom() {
                break;
            }
            put_spans(
                buf,
                rect.x.saturating_add(line.indent),
                y,
                rect.right(),
                &line.spans,
            );
            y = y.saturating_add(1);
        }
        // 套间空 1 行。
        y = y.saturating_add(1);
    }
}

/// 步骤样例的固定 token 数值；`tokens` item 用回合聚合值。
fn step_tokens(step: AgentStepId) -> Tokens {
    match step {
        AgentStepId::Exec => (940, 120, 0),
        AgentStepId::Read => (1_240, 80, 620),
        AgentStepId::Edit => (3_400, 180, 2_100),
        AgentStepId::Write => (2_050, 90, 1_200),
        AgentStepId::Search => (1_680, 60, 900),
        AgentStepId::Web => (2_900, 240, 1_400),
        AgentStepId::Thinking => (820, 310, 0),
        AgentStepId::Subagent => (3_100, 220, 2_000),
        AgentStepId::Todo => (760, 70, 0),
        AgentStepId::Mcp => (1_120, 140, 480),
        AgentStepId::Compact => (12_400, 0, 8_100),
        AgentStepId::Mode => (0, 0, 0),
        AgentStepId::Final => (5_200, 640, 3_300),
        AgentStepId::Markdown => (12_400, 1_200, 8_100),
        AgentStepId::Error => (1_460, 40, 700),
        AgentStepId::Question => (900, 60, 300),
        AgentStepId::Tokens => (12_400, 312, 8_100),
    }
}

/// token 页脚字符串（对齐 lx-agent）：`IN <in> · OUT <out> · CACHE <cache>`。
///
/// cache 为 0 时省略 CACHE 段。
pub fn format_tokens_line(tokens: Tokens) -> String {
    let (input, output, cache) = tokens;
    let mut line = format!("{} {}", text::agent::TOKEN_IN, format_tokens(input));
    line.push_str(text::agent::SEPARATOR);
    line.push_str(&format!(
        "{} {}",
        text::agent::TOKEN_OUT,
        format_tokens(output)
    ));
    if cache > 0 {
        line.push_str(text::agent::SEPARATOR);
        line.push_str(&format!(
            "{} {}",
            text::agent::TOKEN_CACHE,
            format_tokens(cache)
        ));
    }
    line
}

/// token 数字短格式（对齐 lx-agent）：`<1000` 原样、`<10000` 一位小数（`4.2k`）、
/// `<1_000_000` 取整（`12k`）、否则一位小数的 `M`（`1.2M`）。
pub fn format_tokens(count: u32) -> String {
    if count < 1_000 {
        count.to_string()
    } else if count < 10_000 {
        format!("{:.1}k", f64::from(count) / 1_000.0)
    } else if count < 1_000_000 {
        format!("{}k", (count + 500) / 1_000)
    } else {
        format!("{:.1}M", f64::from(count) / 1_000_000.0)
    }
}

/// 步骤状态色：失败/中断红、其余成功绿（样例只展示这两种终态）。
fn status_style(step: AgentStepId) -> Style {
    match step {
        AgentStepId::Error => style::agent_fail(),
        _ => style::agent_ok(),
    }
}

/// 步骤动词（codex/opencode/lx 头部用）。
fn verb(step: AgentStepId) -> &'static str {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::VERB_EXEC,
        AgentStepId::Read => agent::VERB_READ,
        AgentStepId::Edit => agent::VERB_EDIT,
        AgentStepId::Write => agent::VERB_WRITE,
        AgentStepId::Search => agent::VERB_SEARCH,
        AgentStepId::Web => agent::VERB_WEB,
        AgentStepId::Thinking => agent::VERB_THINKING,
        AgentStepId::Subagent => agent::VERB_SUBAGENT,
        AgentStepId::Todo => agent::VERB_TODO,
        AgentStepId::Mcp => agent::VERB_MCP,
        AgentStepId::Compact => agent::VERB_COMPACT,
        AgentStepId::Mode => agent::VERB_MODE,
        AgentStepId::Final => agent::VERB_FINAL,
        AgentStepId::Markdown => agent::VERB_FINAL,
        AgentStepId::Error => agent::VERB_ERROR,
        AgentStepId::Question => agent::VERB_QUESTION,
        AgentStepId::Tokens => agent::VERB_FINAL,
    }
}

/// 步骤主参数。
fn detail(step: AgentStepId) -> &'static str {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::DETAIL_EXEC,
        AgentStepId::Read => agent::DETAIL_READ,
        AgentStepId::Edit => agent::DETAIL_EDIT,
        AgentStepId::Write => agent::DETAIL_WRITE,
        AgentStepId::Search => agent::DETAIL_SEARCH,
        AgentStepId::Web => agent::DETAIL_WEB,
        AgentStepId::Thinking => agent::DETAIL_THINKING,
        AgentStepId::Subagent => agent::DETAIL_SUBAGENT,
        AgentStepId::Todo => agent::DETAIL_TODO,
        AgentStepId::Mcp => agent::DETAIL_MCP,
        AgentStepId::Compact => agent::DETAIL_COMPACT,
        AgentStepId::Mode => agent::DETAIL_MODE,
        AgentStepId::Final => agent::DETAIL_FINAL,
        AgentStepId::Markdown => agent::DETAIL_FINAL,
        AgentStepId::Error => agent::DETAIL_ERROR,
        AgentStepId::Question => agent::DETAIL_QUESTION,
        AgentStepId::Tokens => agent::DETAIL_FINAL,
    }
}

/// claude 套工具名。
fn claude_name(step: AgentStepId) -> &'static str {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::NAME_EXEC,
        AgentStepId::Read => agent::NAME_READ,
        AgentStepId::Edit => agent::NAME_EDIT,
        AgentStepId::Write => agent::NAME_WRITE,
        AgentStepId::Search => agent::NAME_SEARCH,
        AgentStepId::Web => agent::NAME_WEB,
        AgentStepId::Thinking => agent::NAME_THINKING,
        AgentStepId::Subagent => agent::NAME_SUBAGENT,
        AgentStepId::Todo => agent::NAME_TODO,
        AgentStepId::Mcp => agent::NAME_MCP,
        AgentStepId::Compact => agent::NAME_COMPACT,
        AgentStepId::Mode => agent::NAME_MODE,
        AgentStepId::Final => agent::NAME_FINAL,
        AgentStepId::Markdown => agent::NAME_FINAL,
        AgentStepId::Error => agent::NAME_ERROR,
        AgentStepId::Question => agent::NAME_QUESTION,
        AgentStepId::Tokens => agent::NAME_FINAL,
    }
}

/// `lx` 套步骤图标。
fn lx_icon(step: AgentStepId) -> &'static str {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::ICON_EXEC,
        AgentStepId::Read => agent::ICON_READ,
        AgentStepId::Edit => agent::ICON_EDIT,
        AgentStepId::Write => agent::ICON_WRITE,
        AgentStepId::Search => agent::ICON_SEARCH,
        AgentStepId::Web => agent::ICON_WEB,
        AgentStepId::Thinking => agent::ICON_THINKING,
        AgentStepId::Subagent => agent::ICON_SUBAGENT,
        AgentStepId::Todo => agent::ICON_TODO,
        AgentStepId::Mcp => agent::ICON_MCP,
        AgentStepId::Compact => agent::ICON_COMPACT,
        AgentStepId::Mode => agent::ICON_MODE,
        AgentStepId::Final => agent::ICON_FINAL,
        AgentStepId::Markdown => agent::ICON_FINAL,
        AgentStepId::Error => agent::ICON_ERROR,
        AgentStepId::Question => agent::ICON_QUESTION,
        AgentStepId::Tokens => agent::ICON_TOKENS,
    }
}

/// `lx` 套图标颜色：失败红、终态绿、其余强调色。
fn lx_icon_style(step: AgentStepId) -> Style {
    match step {
        AgentStepId::Error => style::agent_fail(),
        AgentStepId::Final => style::agent_ok(),
        _ => style::agent_step_name(),
    }
}

/// opencode 套原生图标。
fn opencode_icon(step: AgentStepId) -> &'static str {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::OPEN_ICON_EXEC,
        AgentStepId::Read => agent::OPEN_ICON_READ,
        AgentStepId::Write => agent::OPEN_ICON_WRITE,
        AgentStepId::Edit => agent::OPEN_ICON_EDIT,
        AgentStepId::Search => agent::OPEN_ICON_SEARCH,
        AgentStepId::Web => agent::OPEN_ICON_WEB,
        AgentStepId::Thinking => agent::OPEN_ICON_THINKING,
        AgentStepId::Subagent => agent::OPEN_ICON_SUBAGENT,
        AgentStepId::Todo => agent::OPEN_ICON_TODO,
        AgentStepId::Mcp => agent::OPEN_ICON_MCP,
        AgentStepId::Compact => agent::OPEN_ICON_COMPACT,
        AgentStepId::Mode => agent::OPEN_ICON_MODE,
        AgentStepId::Final => agent::OPEN_ICON_FINAL,
        AgentStepId::Markdown => agent::OPEN_ICON_FINAL,
        AgentStepId::Error => agent::OPEN_ICON_ERROR,
        AgentStepId::Question => agent::OPEN_ICON_QUESTION,
        AgentStepId::Tokens => agent::OPEN_ICON_FINAL,
    }
}

/// 步骤子行文案（各套共用）；`todo`/`tokens` 用自定义渲染。
fn body_lines(step: AgentStepId) -> &'static [&'static str] {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::BODY_EXEC,
        AgentStepId::Read => agent::BODY_READ,
        AgentStepId::Edit => agent::EDIT_BODY,
        AgentStepId::Write => agent::BODY_WRITE,
        AgentStepId::Search => agent::BODY_SEARCH,
        AgentStepId::Web => agent::BODY_WEB,
        AgentStepId::Thinking => agent::BODY_THINKING,
        AgentStepId::Mcp => agent::BODY_MCP,
        AgentStepId::Final => agent::BODY_FINAL,
        AgentStepId::Error => agent::BODY_ERROR,
        AgentStepId::Question => agent::BODY_QUESTION,
        // 这些步骤在 `variant_lines` 里单独展开：
        // subagent 的子任务各带 token 行；todo/tokens 用自定义渲染；markdown 走渲染器。
        AgentStepId::Subagent
        | AgentStepId::Todo
        | AgentStepId::Compact
        | AgentStepId::Mode
        | AgentStepId::Markdown
        | AgentStepId::Tokens => &[],
    }
}

/// 逐段写入多色文本，返回结束列；越过 `right` 边界停止，宽字符按显示宽度计。
fn put_spans(buf: &mut Buffer, x: u16, y: u16, right: u16, spans: &[(String, Style)]) -> u16 {
    let mut column = x;
    for (value, style) in spans {
        for symbol in value.chars() {
            let width = char_width(symbol);
            if column.saturating_add(width) > right {
                return column;
            }
            if let Some(cell) = buf.cell_mut((column, y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(*style);
            }
            column = column.saturating_add(width);
        }
    }
    column
}

/// 单个字符的显示宽度（0/1/2 列）。
fn char_width(symbol: char) -> u16 {
    use unicode_width::UnicodeWidthChar;
    symbol.width().unwrap_or(1).min(2) as u16
}

#[cfg(test)]
mod tests;
