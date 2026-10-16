//! `[UI]` 查看器 `agent` 分组的右侧预览：agent 步骤调用样式（lx 直角风）。
//!
//! 每个 agent 步骤 item 在右区渲染**一套** lx 直角风格样例：`<icon> Verb detail`
//! 头部，子行用 `└─` 直角连接，末尾一行 token 页脚
//! （`IN 1.2k · OUT 300 · CACHE 500`，对齐 lx-agent flow item）。
//!
//! 配色取 lx 品牌 logo 像素调色板（见 [`crate::ui::style`] 的 `logo_pixel`）：
//! 左侧图标用品牌粉 [`style::agent_icon`]、固定动词用品牌天蓝 [`style::agent_verb`]、
//! 可变参数用品牌冰蓝外加下划线 [`style::agent_param`]；失败步骤整行红加粗。
//! 全部避开边框青，避免与分区边框混色。
//!
//! 所有宽度以显示列计算，越界即截断，窄屏不 panic。本模块纯渲染，不读
//! `AppState` 领域数据。设计上先把一套样例展开成 `Vec<SampleLine>`（缩进 + 多色
//! 分段），再由同一份结果驱动渲染与高度计算，避免「算的高度」与「画的内容」漂移。

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
pub(super) struct SampleLine {
    indent: u16,
    pub(super) spans: Vec<(String, Style)>,
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

/// 组件 id 是否为 agent 步骤；是则返回步骤种类。
pub fn agent_step_of(id: UiGalleryComponentId) -> Option<AgentStepId> {
    match id {
        UiGalleryComponentId::Agent(step) => Some(step),
        _ => None,
    }
}

/// `agent` 步骤预览的自然尺寸：宽度贴合最长内容，高度按样例实际行数计算。
pub fn agent_appearance_size(step: AgentStepId, available: Rect) -> (u16, u16) {
    let width = 52u16.min(available.width);
    let height = (step_lines(step).len() as u16).min(available.height);
    (width, height)
}

/// 把某步骤展开成若干行（头部 + 子行 + token 页脚）。
fn step_lines(step: AgentStepId) -> Vec<SampleLine> {
    if step == AgentStepId::Tokens {
        return tokens_lines();
    }
    if step == AgentStepId::Markdown {
        return markdown::markdown_lines(step_tokens(step));
    }
    let tokens = step_tokens(step);
    let mut lines = vec![header_line(step)];
    // 子行（todo 用三态复选框；edit 用分色 diff 计数；其余用直角连接符 + 文案）。
    if step == AgentStepId::Todo {
        lines.extend(todo_lines());
    } else if step == AgentStepId::Edit {
        lines.push(edit_diff_line());
    } else {
        for body in body_lines(step) {
            lines.push(detail_line(body));
        }
    }
    // subagent：只显示“进行中”的那一步（一个直角连接符）；token 仅底部一行总计。
    if step == AgentStepId::Subagent {
        lines.push(detail_line(text::agent::SUBAGENT_IN_PROGRESS));
    }
    // 每一步都带 token 页脚（subagent 即该子任务的总用量）。
    lines.push(SampleLine::plain(
        2,
        format_tokens_line(tokens),
        style::agent_tokens(),
    ));
    lines
}

/// 头部行：`<icon> Verb detail`；失败步骤（error）整行红 + 加粗。
fn header_line(step: AgentStepId) -> SampleLine {
    let detail = detail(step);
    if step == AgentStepId::Error {
        // error：图标、动词、参数统一红 + 加粗。
        let fail = style::agent_fail();
        return SampleLine::spans(
            0,
            vec![
                (format!("{} ", lx_icon(step)), fail),
                (format!("{} ", verb(step)), fail),
                (detail.to_string(), fail),
            ],
        );
    }
    SampleLine::spans(
        0,
        vec![
            // 左侧图标：品牌粉高亮，让每步一眼可辨。
            (format!("{} ", lx_icon(step)), style::agent_icon()),
            // 固定动词：品牌天蓝。
            (format!("{} ", verb(step)), style::agent_verb()),
            // 可变参数：品牌冰蓝 + 下划线。
            (detail.to_string(), style::agent_param()),
        ],
    )
}

/// 子行：直角连接符 + 文案。
fn detail_line(body: &str) -> SampleLine {
    SampleLine::spans(
        0,
        vec![
            (
                format!("  {} ", text::agent::MARKER_TREE_LEAF),
                style::agent_connector(),
            ),
            (body.to_string(), style::text()),
        ],
    )
}

/// edit 的子行：直角连接符 + 分色的 `+N` / `-M` 计数 + 路径（不再展示 diff 正文）。
fn edit_diff_line() -> SampleLine {
    SampleLine::spans(
        0,
        vec![
            (
                format!("  {} ", text::agent::MARKER_TREE_LEAF),
                style::agent_connector(),
            ),
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
    )
}

/// todo 的三态复选框（取 claude 一组：`✔`/`◼`/`◻`；done 整行绿 + 删除线、进行中强调、
/// 待办为次要信息）。
fn todo_lines() -> Vec<SampleLine> {
    let todo = |marker: &str, body: &str, style: Style| -> SampleLine {
        SampleLine::spans(
            2,
            vec![(format!("{marker} "), style), (body.to_string(), style)],
        )
    };
    let done_style = style::agent_ok().add_modifier(ratatui::style::Modifier::CROSSED_OUT);
    vec![
        todo(text::agent::TODO_DONE, "add agent folder", done_style),
        todo(
            text::agent::TODO_IN_PROGRESS,
            "design variants",
            style::accent(),
        ),
        todo(
            text::agent::TODO_PENDING,
            "write tests",
            style::agent_step_detail(),
        ),
    ]
}

/// `tokens` 聚合 item：回合汇总条 + 上下文 pill。
fn tokens_lines() -> Vec<SampleLine> {
    let tokens = step_tokens(AgentStepId::Tokens);
    let mut lines = vec![SampleLine::plain(
        0,
        format!(
            "{}{}",
            text::agent::TOKENS_BAR_LEAD,
            format_tokens_line(tokens)
        ),
        style::agent_verb(),
    )];
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
    lines
}

/// 在 `rect` 内渲染 agent 步骤的 lx 直角风格样例。
///
/// 单套样例的高度随步骤而变（`mode`/`compact` 仅头部 + token 两行），故只要求
/// 有可用行（`height >= 1`）即可绘制，不能像四套叠放时那样要求最小高度，否则
/// 矮步骤会被整体吞掉。
pub fn render_agent_appearance(buf: &mut Buffer, rect: Rect, step: AgentStepId) {
    if rect.width < 6 || rect.height == 0 {
        return;
    }
    let mut y = rect.y;
    for line in step_lines(step) {
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

/// 步骤动词（固定部分，标题用品牌天蓝）。
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

/// 步骤主参数（可变部分，标题用品牌冰蓝 + 下划线）。
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

/// 步骤图标（单宽跨平台，避免 emoji 与东亚宽字符）。
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

/// 步骤子行文案；`todo`/`tokens` 用自定义渲染。
fn body_lines(step: AgentStepId) -> &'static [&'static str] {
    use text::agent;
    match step {
        AgentStepId::Exec => agent::BODY_EXEC,
        AgentStepId::Write => agent::BODY_WRITE,
        AgentStepId::Web => agent::BODY_WEB,
        AgentStepId::Mcp => agent::BODY_MCP,
        AgentStepId::Final => agent::BODY_FINAL,
        AgentStepId::Error => agent::BODY_ERROR,
        AgentStepId::Question => agent::BODY_QUESTION,
        // 这些步骤不展示子行内容（read 只给行范围、search 不给命中、thinking 只给
        // 标题、edit 用 diff 计数行），或在 `step_lines` 里单独展开
        // （subagent / todo / tokens 自定义，markdown 走渲染器）。
        AgentStepId::Read
        | AgentStepId::Edit
        | AgentStepId::Search
        | AgentStepId::Thinking
        | AgentStepId::Subagent
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
