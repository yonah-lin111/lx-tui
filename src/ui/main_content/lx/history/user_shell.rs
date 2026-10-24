//! 本地命令卡片：标题、命令、输出预览与退出信息。
//!
//! 与工具步骤共用同一套直角树体语法（`  │  ` / `  └─ `，[`TREE_COLS`] 列）：
//!
//! ```text
//! ■ You run command · exit 2 · 1.2s ← 标题：状态色 + 退出码 + 耗时
//!   │  $ cargo test                ← 参数区：折叠 2 行，展开更多（可独立折叠）
//!   │  error[E0308]: mismatched…   ← 输出区：折叠 5 行，展开完整输出（可独立折叠）
//!   └─ … 18 more lines             ← 输出区自己的省略提示
//! ```
//!
//! 布局与绘制共用 [`lines`]：滚动条、置顶行与点击/复制几何不会与真实绘制漂移。
//!
//! **两个可独立折叠的区域**：命令参数与命令输出各有展开状态，点其中一块不会连带
//! 另一块；折叠态一律 dim，展开升为终端默认前景（没有整行灰底，也不随笔悬停变色）。
//! **运行中（长任务）只给一行引导**：命令 + `  └─ open task panel · live output`，
//! 消息列表不铺实时输出——完整输出在后台任务面板里，点卡片打开面板并定位该进程。

use ratatui::layout::Rect;
use ratatui::style::Style;
#[cfg(test)]
use unicode_width::UnicodeWidthChar;

use crate::app::state::{CardSection, UserShellView};
use crate::ui::{style, text};

use super::tool::{Folds, TREE_COLS, output_style, param_lines};
use super::{Grid, truncate_with_ellipsis, visible_row, wrap};

/// 折叠态输出预览最多占用的物理行（含末尾的省略提示行）。
const PREVIEW_LINES: usize = 6;
/// 折叠态保留的真实输出行数：首 3 + 尾 2（提示行另占一行，合计 [`PREVIEW_LINES`]）。
const PREVIEW_HEAD: usize = 3;
const PREVIEW_TAIL: usize = 2;

/// 树体的一行：前缀（空 = 标题行）+ 内容 + 样式。
///
/// 连接符由 [`draw`] 统一补色：前缀与内容不同色，只有「哪一行收口」这一处判断。
struct Line {
    prefix: &'static str,
    text: String,
    style: Style,
}

/// 树体两块的行数（标题之外）：命令参数与输出。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Blocks {
    pub params: usize,
    pub output: usize,
}

/// 按宽度生成卡片的全部物理行（布局与绘制唯一来源）。
fn lines(
    shell: &UserShellView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    folds: Folds,
) -> Vec<Line> {
    // 极窄：连接符自身都放不下，退回整宽画内容（与通知/文件详情同一降级口径），
    // 否则树体前缀会把可用列吃光、正文一个字都看不见。
    let tree = keeps_tree(width);
    let content_width = if tree {
        body_width(width)
    } else {
        width.max(1)
    };
    let (params, output) = sections(shell, content_width, folds);
    // `└─` 只包裹参数部分（与文件工具的 `└─ 详情` 同口径）：输出内容只用等宽空白
    // 对齐到内容列，不再被 `│` / `└─` 围住。
    let params_last = params.len().saturating_sub(1);
    let mut out = Vec::with_capacity(params.len() + output.len() + 1);
    out.push(title(shell, width, loading_frame, animating));
    for (index, mut line) in params.into_iter().enumerate() {
        if tree {
            line.prefix = if index == params_last {
                text::agent::TREE_LEAF
            } else {
                text::agent::TREE_BAR
            };
        }
        out.push(line);
    }
    for mut line in output {
        if tree {
            line.prefix = text::agent::TREE_PAD;
        }
        out.push(line);
    }
    out
}

/// 树体两块：命令参数与输出（运行中输出区只剩一行引导）。
fn sections(shell: &UserShellView, width: u16, folds: Folds) -> (Vec<Line>, Vec<Line>) {
    // 保留用户脚本的字面值，按内容宽度换行（折叠预算与 agent 命令卡片同一口径）。
    let params = param_lines(&format!("$ {}", shell.command), width, folds.body)
        .into_iter()
        .map(|line| Line {
            prefix: "",
            text: line.text,
            style: line.style,
        })
        .collect();
    let output = if shell.is_running() {
        // 长任务：不铺实时输出，只留一句「去面板看」。
        vec![Line {
            prefix: "",
            text: text::agent::TASK_OPEN_GUIDE.to_string(),
            style: output_style(false),
        }]
    } else {
        finished_output(shell, width, folds.output)
    };
    (clamp(params, width), clamp(output, width))
}

/// 终态输出区：折叠首 3 + 尾 2 行 + 省略提示，展开全部已保存输出。
fn finished_output(shell: &UserShellView, width: u16, expanded: bool) -> Vec<Line> {
    let style = output_style(expanded);
    let (preview, omitted) = if expanded {
        (all_output_lines(&shell.output, width), 0)
    } else {
        preview(&shell.output, width)
    };
    let mut output = Vec::with_capacity(preview.len() + 2);
    if preview.is_empty() {
        output.push(Line {
            prefix: "",
            text: text::LX_SHELL_NO_OUTPUT.to_string(),
            style,
        });
    } else {
        output.extend(preview.into_iter().map(|line| Line {
            prefix: "",
            text: line,
            style,
        }));
    }
    if shell.output_truncated() {
        output.push(Line {
            prefix: "",
            text: format!("… {} earlier bytes dropped", shell.dropped_bytes),
            style: style::agent_warn(),
        });
    }
    // 省略提示是输出区的最后一行：与折叠输出同档。
    if omitted > 0 {
        output.push(Line {
            prefix: "",
            text: text::agent::file_diff_more_lines(omitted),
            style: output_style(false),
        });
    }
    output
}

/// 每行截到内容列宽（极窄时树体前缀让位、折行算法也会退化，这里兜底不越界）。
fn clamp(mut lines: Vec<Line>, width: u16) -> Vec<Line> {
    for line in &mut lines {
        line.text = truncate_with_ellipsis(&line.text, usize::from(width));
    }
    lines
}

/// 宽度是否还容得下树体前缀（放不下就退回整宽，不画连接符）。
fn keeps_tree(width: u16) -> bool {
    width > TREE_COLS as u16
}

/// 标题行：`■ Running` / `■ You run command · exit 0 · 1.2s`。
///
/// 标题占整宽（树体之外），窄屏尾部省略——不挤掉行首方块。
fn title(shell: &UserShellView, width: u16, loading_frame: usize, animating: bool) -> Line {
    let (mark, mark_style) = header_mark(shell, loading_frame, animating);
    let label = if shell.is_running() {
        text::LX_SHELL_RUNNING
    } else {
        text::LX_SHELL_COMMAND
    };
    let mut text = format!("{mark} {label}");
    if !shell.is_running() {
        let tail = footer(shell);
        if !tail.is_empty() {
            text.push_str(text::LX_CHAT_TOOL_STATUS_SEP);
            text.push_str(&tail);
        }
    }
    let text = if width < 2 {
        // 一列宽连省略号都放不下：只保留方块，状态信号永远可见。
        text.chars().take(1).collect()
    } else {
        truncate_with_ellipsis(&text, usize::from(width))
    };
    Line {
        prefix: "",
        text,
        style: mark_style,
    }
}

/// 卡片占用的渲染行数。
pub fn rows(
    shell: &UserShellView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    folds: Folds,
) -> usize {
    lines(shell, width, loading_frame, animating, folds).len()
}

/// 树体两块各自的高度（布局/命中共用；运行中输出区恒为 1 行引导）。
pub fn blocks(shell: &UserShellView, width: u16, folds: Folds) -> Blocks {
    let content_width = if keeps_tree(width) {
        body_width(width)
    } else {
        width.max(1)
    };
    let (params, output) = sections(shell, content_width, folds);
    Blocks {
        params: params.len(),
        output: output.len(),
    }
}

/// 卡片内的一个交互区域（行偏移相对卡片首行，标题行是偏移 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub section: CardSection,
    pub offset: usize,
    pub rows: usize,
    pub clickable: bool,
}

/// 卡片的可交互区域：运行中所有区域都打开面板；终态参数/输出各自独立折叠。
pub fn regions(shell: &UserShellView, width: u16, folds: Folds) -> Vec<Region> {
    let blocks = blocks(shell, width, folds);
    let params_end = 1 + blocks.params;
    if shell.is_running() {
        return vec![
            Region {
                section: CardSection::Body,
                offset: 0,
                rows: params_end,
                clickable: true,
            },
            Region {
                section: CardSection::Output,
                offset: params_end,
                rows: blocks.output,
                clickable: true,
            },
        ];
    }
    let mut out = vec![Region {
        section: CardSection::Body,
        offset: 1,
        rows: blocks.params,
        clickable: params_expandable(shell, width),
    }];
    if blocks.output > 0 {
        // 只有**真被折叠**的输出才登记点击面（短输出点了也不会变）。
        out.push(Region {
            section: CardSection::Output,
            offset: params_end,
            rows: blocks.output,
            clickable: output_expandable(shell, width),
        });
    }
    out
}

/// 命令参数区展开后是否**真的多出内容**。
pub fn params_expandable(shell: &UserShellView, width: u16) -> bool {
    let content_width = if keeps_tree(width) {
        body_width(width)
    } else {
        width.max(1)
    };
    let command = format!("$ {}", shell.command);
    param_lines(&command, content_width, true).len()
        > param_lines(&command, content_width, false).len()
}

/// 输出区展开后是否**真的多出内容**。
pub fn output_expandable(shell: &UserShellView, width: u16) -> bool {
    if shell.is_running() {
        return false;
    }
    let content_width = if keeps_tree(width) {
        body_width(width)
    } else {
        width.max(1)
    };
    all_output_lines(&shell.output, content_width).len() > PREVIEW_LINES
}

/// 绘制卡片并推进 `row_index`。
#[allow(clippy::too_many_arguments)]
pub fn draw(
    grid: &mut Grid,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
    shell: &UserShellView,
    loading_frame: usize,
    animating: bool,
    folds: Folds,
) {
    for line in lines(shell, content.width, loading_frame, animating, folds) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            // 连接符用强调灰色、内容用自己的语义色：与工具卡片同一套读法。
            let indent = if line.prefix.is_empty() {
                0
            } else {
                TREE_COLS as u16
            };
            if indent > 0 {
                grid.put_clipped(
                    content.x,
                    y,
                    line.prefix,
                    style::chat_notice_connector(),
                    content.width,
                );
            }
            grid.put_clipped(
                content.x.saturating_add(indent),
                y,
                &line.text,
                line.style,
                content.width.saturating_sub(indent),
            );
        }
        *row_index += 1;
    }
}

/// 树体内容可用列数（标题与内容共用整宽，树体让出连接符列）。
fn body_width(width: u16) -> u16 {
    width.saturating_sub(TREE_COLS as u16).max(1)
}

/// 标题方块与颜色：运行中橙/呼吸，失败红，主动停止警告色，正常结束绿。
fn header_mark(shell: &UserShellView, loading_frame: usize, animating: bool) -> (char, Style) {
    if shell.is_running() {
        let filled =
            !animating || (loading_frame / text::agent::STEP_BREATH_TICKS).is_multiple_of(2);
        let glyph = if filled {
            text::agent::STEP_MARK_FILLED
        } else {
            text::agent::STEP_MARK_HOLLOW
        };
        return (glyph, style::chat_step_mark_alive());
    }
    if shell.is_failed() {
        return (text::agent::STEP_MARK_FILLED, style::agent_fail());
    }
    // 用户主动停止不是成功：绿色勾会让标题与底部 `stopped` 自相矛盾。
    if matches!(shell.state, crate::app::state::ProcessStateView::Stopped) {
        return (text::agent::STEP_MARK_FILLED, style::agent_warn());
    }
    (text::agent::STEP_MARK_FILLED, style::agent_ok())
}

/// 折叠态预览：保留前 [`PREVIEW_HEAD`] 行 + 后 [`PREVIEW_TAIL`] 行，返回省略行数。
fn preview(output: &str, width: u16) -> (Vec<String>, usize) {
    let rows = all_output_lines(output, width);
    if rows.len() <= PREVIEW_LINES {
        return (rows, 0);
    }
    let omitted = rows.len().saturating_sub(PREVIEW_HEAD + PREVIEW_TAIL);
    let mut preview: Vec<String> = rows[..PREVIEW_HEAD].to_vec();
    preview.extend(rows[rows.len() - PREVIEW_TAIL..].iter().cloned());
    (preview, omitted)
}

/// 全部输出按宽度换行（展开态；末尾空行不占位）。
fn all_output_lines(output: &str, width: u16) -> Vec<String> {
    if output.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    for line in output.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            rows.push(String::new());
            continue;
        }
        rows.extend(wrap(line, width));
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// 底部信息：退出码与耗时（失败原因由状态表达）。
fn footer(shell: &UserShellView) -> String {
    let mut parts: Vec<String> = Vec::new();
    match shell.state {
        crate::app::state::ProcessStateView::Exited { code } => parts.push(format!(
            "exit {}",
            code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
        )),
        crate::app::state::ProcessStateView::Stopped => parts.push("stopped".into()),
        crate::app::state::ProcessStateView::TimedOut => parts.push("timed out".into()),
        crate::app::state::ProcessStateView::Interrupted => parts.push("interrupted".into()),
        crate::app::state::ProcessStateView::Failed => parts.push("failed".into()),
        crate::app::state::ProcessStateView::Starting
        | crate::app::state::ProcessStateView::Running => {}
    }
    if let Some(duration_ms) = shell.duration_ms {
        parts.push(format_duration(duration_ms));
    }
    parts.join(" · ")
}

/// 毫秒 → `1.2s` / `2m 3s`。
fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1_000.0;
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let total = ms / 1_000;
        format!("{}m {}s", total / 60, total % 60)
    }
}

/// 输出行的显示宽度（测试用）。
#[cfg(test)]
pub fn display_width(value: &str) -> usize {
    value.chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::ProcessStateView;

    fn shell(output: &str, state: ProcessStateView) -> UserShellView {
        UserShellView {
            command: "printf hello".into(),
            cwd: "/repo".into(),
            shell: "/bin/zsh".into(),
            state,
            exit_code: state.exit_code(),
            duration_ms: Some(1_200),
            output: output.into(),
            output_start: 0,
            output_end: output.len() as u64,
            dropped_bytes: 0,
            truncated: false,
            runtime_epoch: None,
            process_id: None,
            revision: 1,
        }
    }

    fn folds(body: bool, output: bool) -> Folds {
        Folds { body, output }
    }

    /// 拼接后的可见文本（含树体前缀）。
    fn text_of(shell: &UserShellView, width: u16, folds: Folds) -> Vec<String> {
        lines(shell, width, 0, false, folds)
            .into_iter()
            .map(|line| format!("{}{}", line.prefix, line.text))
            .collect()
    }

    #[test]
    fn draw_advances_exactly_the_layout_row_count() {
        let shell = shell(
            "line 1\nline 2\n",
            ProcessStateView::Exited { code: Some(0) },
        );
        let area = Rect::new(0, 0, 40, 20);
        let expected = rows(&shell, area.width, 0, false, folds(false, false));
        let mut grid = Grid::new(area);
        let mut index = 0;
        draw(
            &mut grid,
            area,
            0,
            &mut index,
            &shell,
            0,
            false,
            folds(false, false),
        );
        assert_eq!(index, expected, "绘制与布局行数必须一致");
    }

    #[test]
    fn body_rows_use_the_shared_tree_prefixes() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        let text = text_of(&shell, 40, folds(false, false));
        assert!(
            text[0].starts_with("■ You run command"),
            "标题行不带连接符: {text:?}"
        );
        assert!(text[0].contains("exit 0"), "退出码并入标题行: {text:?}");
        // `└─` 只包裹参数：命令只有一行，它自己就是收口行。
        assert!(text[1].starts_with("  └─ $ printf hello"), "{text:?}");
        // 输出区不画树，只用等宽空白对齐。
        let last = text.last().expect("末行");
        assert!(
            last.starts_with("     ") && !last.contains('│') && !last.contains('└'),
            "输出不被连接符包裹: {text:?}"
        );
        assert!(last.contains("lines"), "末行是省略提示: {text:?}");
    }

    #[test]
    fn preview_keeps_head_tail_and_marks_omitted_lines() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        let text = text_of(&shell, 40, folds(false, false));
        assert!(text.iter().any(|line| line.contains("line 0")), "保留首部");
        assert!(text.iter().any(|line| line.contains("line 19")), "保留尾部");
        assert!(
            text.iter()
                .any(|line| line.contains("more lines") || line.contains("lines omitted")),
            "省略必须显式标注: {text:?}"
        );
        let output_rows = text
            .iter()
            .filter(|line| {
                line.trim_start_matches([' ', '│', '└', '─'])
                    .starts_with("line ")
            })
            .count();
        assert_eq!(
            output_rows,
            PREVIEW_HEAD + PREVIEW_TAIL,
            "折叠态保留 5 行真实输出: {text:?}"
        );
    }

    #[test]
    fn running_and_finished_use_distinct_labels() {
        let running = shell("", ProcessStateView::Running);
        let done = shell("", ProcessStateView::Exited { code: Some(0) });
        let running_text = text_of(&running, 40, folds(false, false));
        let done_text = text_of(&done, 40, folds(false, false));
        assert!(running_text[0].contains("Running"));
        assert!(done_text[0].contains("You run command"));
        // 运行中不铺输出，只给一行「去面板看」；终态才显示输出占位。
        assert!(
            running_text
                .iter()
                .any(|line| line.contains(text::agent::TASK_OPEN_GUIDE)),
            "{running_text:?}"
        );
        assert!(!running_text.iter().any(|line| line.contains("line 0")));
        assert!(done_text.iter().any(|line| line.contains("(no output)")));
        assert!(!done_text[0].contains("Running"), "终态不得残留运行文案");
    }

    #[test]
    fn long_lines_wrap_inside_width() {
        let shell = shell(&"x".repeat(200), ProcessStateView::Exited { code: Some(0) });
        for line in lines(&shell, 20, 0, false, folds(false, false)) {
            let width = display_width(line.prefix) + display_width(&line.text);
            assert!(width <= 20, "行不得超宽: {width}");
        }
    }

    /// 极窄宽度：标题尾部省略，树体让出连接符列后仍不越界。
    #[test]
    fn narrow_width_truncates_title_and_keeps_tree_within_bounds() {
        let shell = shell("output", ProcessStateView::Exited { code: Some(2) });
        for width in [1u16, 4, 8, 12] {
            for line in lines(&shell, width, 0, false, folds(false, false)) {
                let cols = display_width(line.prefix) + display_width(&line.text);
                assert!(cols <= usize::from(width), "宽度 {width} 溢出: {cols}");
            }
            let title = text_of(&shell, width, folds(false, false)).remove(0);
            assert!(
                title.starts_with('■'),
                "宽度 {width} 下标题仍须保留方块: {title:?}"
            );
        }
    }

    #[test]
    fn expanded_shows_full_saved_output_and_rows_grow() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        assert!(output_expandable(&shell, 40));
        let collapsed = lines(&shell, 40, 0, false, folds(false, false));
        let expanded = lines(&shell, 40, 0, false, folds(false, true));
        assert!(expanded.len() > collapsed.len());
        assert!(
            !expanded
                .iter()
                .any(|line| line.text.contains("more lines") || line.text.contains("omitted"))
        );
        assert!(
            expanded
                .iter()
                .filter(|line| line.text.trim_start().starts_with("line "))
                .count()
                == 20
        );
    }

    /// 参数区与输出区各自独立折叠：展开参数不动输出，展开输出不动参数。
    #[test]
    fn params_and_output_fold_independently() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let mut view = shell(&output, ProcessStateView::Exited { code: Some(0) });
        view.command = format!("echo {}", "y".repeat(200));
        assert!(params_expandable(&view, 40));
        let collapsed = lines(&view, 40, 0, false, folds(false, false));
        let params_only = lines(&view, 40, 0, false, folds(true, false));
        let output_only = lines(&view, 40, 0, false, folds(false, true));
        // 展开参数只增加命令行的物理行，输出预览行数不变。
        assert!(count_output_rows(&params_only) == count_output_rows(&collapsed));
        assert!(count_output_rows(&output_only) > count_output_rows(&collapsed));
        assert!(params_only.len() > collapsed.len());
        assert_eq!(
            count_command_rows(&output_only),
            count_command_rows(&collapsed),
            "展开输出不得连带展开参数"
        );
    }

    fn count_output_rows(lines: &[Line]) -> usize {
        lines
            .iter()
            .filter(|line| line.text.trim_start().starts_with("line "))
            .count()
    }

    fn count_command_rows(lines: &[Line]) -> usize {
        lines
            .iter()
            .filter(|line| line.text.contains("yyyyyyyy"))
            .count()
    }

    /// 短输出（没达到折叠额度）不登记点击面。
    #[test]
    fn short_output_has_no_click_face() {
        let shell = shell("one\ntwo\n", ProcessStateView::Exited { code: Some(0) });
        let regions = regions(&shell, 40, folds(false, false));
        let output = regions
            .iter()
            .find(|region| region.section == CardSection::Output)
            .expect("输出区");
        assert!(!output.clickable, "短输出没有可展开内容");
    }

    /// 参数恒为浅黄；输出折叠恒 dim、展开恒默认前景（都不随鼠标变化）。
    #[test]
    fn params_stay_light_yellow_and_output_follows_expanded_only() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        let dim = lines(&shell, 40, 0, false, folds(false, false));
        let param_expanded = lines(&shell, 40, 0, false, folds(true, false));
        let output_expanded = lines(&shell, 40, 0, false, folds(false, true));
        for rows in [&dim, &param_expanded] {
            assert_eq!(
                find_line(rows, "$ ").style,
                style::chat_tool_param(),
                "参数恒为浅黄"
            );
        }
        assert_eq!(
            find_line(&dim, "line 0").style,
            style::muted(),
            "折叠输出恒为 dim"
        );
        assert_eq!(
            find_line(&output_expanded, "line 0").style,
            style::text(),
            "展开的输出是默认前景"
        );
        assert_eq!(
            find_line(&output_expanded, "$ ").style,
            style::chat_tool_param(),
            "展开输出不得连带展开参数的颜色"
        );
    }

    /// 按文本前缀找一行。
    fn find_line<'a>(lines: &'a [Line], needle: &str) -> &'a Line {
        lines
            .iter()
            .find(|line| line.text.starts_with(needle) || line.text.contains(needle))
            .unwrap_or_else(|| panic!("找不到包含 {needle:?} 的行"))
    }

    #[test]
    fn failed_command_is_marked_failed() {
        let shell = shell("", ProcessStateView::Failed);
        let first = &lines(&shell, 40, 0, false, folds(false, false))[0];
        assert_eq!(first.style, style::agent_fail());
        assert!(first.text.contains("You run command"));
    }

    #[test]
    fn stopped_command_does_not_use_success_mark() {
        let ok = shell("", ProcessStateView::Exited { code: Some(0) });
        let stopped = shell("", ProcessStateView::Stopped);
        let (_, ok_style) = header_mark(&ok, 0, false);
        let (_, stopped_style) = header_mark(&stopped, 0, false);
        assert_ne!(ok_style, stopped_style, "主动停止不是成功");
        assert_eq!(stopped_style, style::agent_warn());
    }
}
