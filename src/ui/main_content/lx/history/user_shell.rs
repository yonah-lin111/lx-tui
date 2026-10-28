//! 本地命令卡片：标题状态、命令与输出预览。
//!
//! 与工具步骤共用同一套树体语法（[`tree`]，`├─` / `│` / `└─`）：
//!
//! ```text
//! ■ You run command · exit 2 · 1.2s
//!   ├─ $ cargo test
//!   └─ error[E0308]: mismatched…
//!      … 18 more lines
//! ```
//!
//! - 退出状态与耗时在**标题右侧**，不再占树体行；
//! - **两个可独立折叠的区域**：命令参数与命令输出各有展开状态；
//! - 输出正文是一块多行节点：首行 `└─`，其余行左对齐；
//! - 折叠态一律 dim，展开升为终端默认前景（没有整行灰底，也不随鼠标悬停变色）；
//! - **运行中（长任务）只给一行引导**：命令 + `open task panel · live output`；
//! - 输出被丢弃（`dropped_bytes`）是执行事实，附在输出块末尾，不混进 UI 省略行数。

use ratatui::layout::Rect;
use ratatui::style::Style;
#[cfg(test)]
use unicode_width::UnicodeWidthChar;

use crate::app::state::{CardSection, UserShellView};
use crate::ui::{style, text};

use super::tool::tree::{self, Node, Preview, Section};
use super::tool::{Folds, Segment, output_style, put_segments, segments_cols};
use super::{Grid, truncate_with_ellipsis, visible_row, wrap};

/// 折叠态保留的真实输出行数：首 3 + 尾 2（提示行另占一行）。
const PREVIEW_HEAD: usize = 3;
const PREVIEW_TAIL: usize = 2;
/// 命令参数折叠态保留的物理行数。
const PARAM_PREVIEW: usize = 2;

/// 树体两块的行数（标题之外）：命令参数与输出。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Blocks {
    pub params: usize,
    pub output: usize,
}

/// 标题右侧状态：`Running` / `exit 2 · 1.2s`。
fn title_status(shell: &UserShellView) -> String {
    if shell.is_running() {
        return text::agent::STATE_RUNNING.to_string();
    }
    let mut parts = vec![text::agent::proc_exit_label(shell.state)];
    if let Some(duration) = shell.duration_ms {
        parts.push(text::agent::duration_label(duration));
    }
    parts.join(text::LX_CHAT_TOOL_STATUS_SEP)
}

/// 命令参数块：`$ 命令`；运行中把引导语作为同一节点的续行。
fn param_section(shell: &UserShellView, folded: bool) -> Section {
    let mut lines = vec![vec![Segment::new(
        format!("$ {}", shell.command),
        style::chat_tool_param(),
    )]];
    if shell.is_running() {
        lines.push(vec![Segment::new(
            text::agent::TASK_OPEN_GUIDE,
            output_style(false),
        )]);
    }
    Section::new(
        vec![Node::block(lines)],
        folded.then(|| Preview::head(PARAM_PREVIEW)),
    )
}

/// 输出块：整块一个多行节点；折叠首尾预览；丢弃字节作为块内续行。
fn output_section(shell: &UserShellView, width: u16, folded: bool) -> Option<Section> {
    if shell.is_running() {
        return None;
    }
    let body_width = tree::content_width(width);
    let rows = all_output_lines(&shell.output, u16::try_from(body_width).unwrap_or(u16::MAX));
    let mut lines: Vec<Vec<Segment>> = if rows.is_empty() {
        vec![vec![Segment::new(
            text::LX_SHELL_NO_OUTPUT,
            output_style(!folded),
        )]]
    } else {
        rows.into_iter()
            .map(|line| vec![Segment::new(line, output_style(!folded))])
            .collect()
    };
    if shell.output_truncated() {
        lines.push(vec![Segment::new(
            text::agent::dropped_bytes_label(shell.dropped_bytes),
            style::agent_warn(),
        )]);
    }
    Some(Section::new(
        vec![Node::block(lines)],
        folded.then(|| Preview::head_tail(PREVIEW_HEAD, PREVIEW_TAIL)),
    ))
}

/// 卡片的两块（参数 / 输出）。
fn sections(shell: &UserShellView, width: u16, folds: Folds) -> (Vec<Section>, Vec<Section>) {
    let params = vec![param_section(shell, !folds.body)];
    let output = output_section(shell, width, !folds.output)
        .into_iter()
        .collect();
    (params, output)
}

/// 树体布局（不含标题行）。
fn body_layout(shell: &UserShellView, width: u16, folds: Folds) -> tree::Layout {
    let (params, output) = sections(shell, width, folds);
    let mut all = params;
    all.extend(output);
    tree::layout(&all, width)
}

/// 卡片占用的渲染行数。
pub fn rows(
    shell: &UserShellView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    folds: Folds,
) -> usize {
    let _ = (loading_frame, animating);
    if width == 0 {
        return 0;
    }
    1 + body_layout(shell, width, folds).rows.len()
}

/// 树体两块各自的高度（布局/命中共用）。
pub fn blocks(shell: &UserShellView, width: u16, folds: Folds) -> Blocks {
    if width == 0 {
        return Blocks::default();
    }
    let (params, output) = sections(shell, width, folds);
    Blocks {
        params: tree::layout(&params, width).rows.len(),
        output: tree::layout(&output, width).rows.len(),
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
        let mut out = vec![Region {
            section: CardSection::Body,
            offset: 0,
            rows: params_end,
            clickable: true,
        }];
        if blocks.output > 0 {
            out.push(Region {
                section: CardSection::Output,
                offset: params_end,
                rows: blocks.output,
                clickable: true,
            });
        }
        return out;
    }
    let mut out = vec![Region {
        section: CardSection::Body,
        offset: 1,
        rows: blocks.params,
        clickable: params_expandable(shell, width),
    }];
    if blocks.output > 0 {
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
    width > 0 && tree::layout(&[param_section(shell, true)], width).hidden > 0
}

/// 输出区展开后是否**真的多出内容**。
pub fn output_expandable(shell: &UserShellView, width: u16) -> bool {
    if shell.is_running() || width == 0 {
        return false;
    }
    output_section(shell, width, true)
        .is_some_and(|section| tree::layout(&[section], width).hidden > 0)
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
    if content.width == 0 {
        return;
    }
    let (mark, mark_style) = header_mark(shell, loading_frame, animating);
    let label = if shell.is_running() {
        text::LX_SHELL_RUNNING
    } else {
        text::LX_SHELL_COMMAND
    };
    if let Some(y) = visible_row(content, scroll, *row_index) {
        let title = format!("{mark} {label}");
        let mut segments = vec![Segment::new(title.clone(), mark_style)];
        let tail = Segment::new(
            format!("{}{}", text::LX_CHAT_TOOL_STATUS_SEP, title_status(shell)),
            style::agent_step_detail(),
        );
        if segments_cols(&segments) + tail.cols() <= usize::from(content.width) {
            segments.push(tail);
        }
        if segments_cols(&segments) > usize::from(content.width) {
            let trimmed = if content.width < 2 {
                title.chars().take(1).collect()
            } else {
                truncate_with_ellipsis(&title, usize::from(content.width))
            };
            segments = vec![Segment::new(trimmed, mark_style)];
        }
        put_segments(grid, content.x, y, content.width, &segments);
    }
    *row_index += 1;
    tree::draw(
        grid,
        content,
        scroll,
        row_index,
        &body_layout(shell, content.width, folds).rows,
    );
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

/// 全部输出按宽度换行（末尾空行不占位）。
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
        let line = super::tool::sample_output_line(line);
        rows.extend(wrap(&line, width));
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
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

    /// 把卡片绘制进网格并按行取文本。
    fn text_of(shell: &UserShellView, width: u16, folds: Folds) -> Vec<String> {
        let height = rows(shell, width, 0, false, folds).max(1) as u16;
        let area = Rect::new(0, 0, width, height);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, area, 0, &mut row, shell, 0, false, folds);
        assert_eq!(row, rows(shell, width, 0, false, folds));
        (0..height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..width {
                    let glyph = grid.cell(x, y);
                    if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                        line.push(glyph.ch);
                    }
                }
                line.trim_end().to_string()
            })
            .collect()
    }

    #[test]
    fn title_shows_exit_status_and_duration() {
        let shell = shell("", ProcessStateView::Exited { code: Some(2) });
        let text = text_of(&shell, 40, folds(false, false));
        assert!(text[0].contains("You run command"), "{text:?}");
        assert!(text[0].contains("exit 2"), "状态在标题右侧：{text:?}");
        assert!(text[0].contains("1.2s"), "耗时在标题右侧：{text:?}");
        assert!(
            text.iter().any(|line| line.contains("$ printf hello")),
            "{text:?}"
        );
    }

    #[test]
    fn preview_keeps_head_tail_and_marks_omitted_lines() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        let text = text_of(&shell, 40, folds(false, false));
        assert!(text.iter().any(|line| line.contains("line 0")), "{text:?}");
        assert!(text.iter().any(|line| line.contains("line 19")), "{text:?}");
        assert!(
            text.iter()
                .any(|line| line.contains("more lines") || line.contains("lines omitted")),
            "{text:?}"
        );
        let output_rows = text
            .iter()
            .filter(|line| {
                line.trim_start_matches([' ', '│', '└', '├', '─'])
                    .starts_with("line ")
            })
            .count();
        assert_eq!(output_rows, PREVIEW_HEAD + PREVIEW_TAIL, "{text:?}");
    }

    #[test]
    fn running_and_finished_use_distinct_labels() {
        let running = shell("", ProcessStateView::Running);
        let done = shell("", ProcessStateView::Exited { code: Some(0) });
        let running_text = text_of(&running, 40, folds(false, false));
        let done_text = text_of(&done, 40, folds(false, false));
        assert!(running_text[0].contains("Running"));
        assert!(done_text[0].contains("You run command"));
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
        for line in text_of(&shell, 20, folds(false, false)) {
            assert!(display_width(&line) <= 20, "行不得超宽: {line:?}");
        }
    }

    /// 极窄宽度：标题尾部省略，树体让出连接符列后仍不越界。
    #[test]
    fn narrow_width_truncates_title_and_keeps_tree_within_bounds() {
        let shell = shell("output", ProcessStateView::Exited { code: Some(2) });
        for width in [1u16, 4, 8, 12] {
            let text = text_of(&shell, width, folds(false, false));
            assert!(
                text[0].starts_with('■'),
                "宽度 {width} 下标题仍须保留方块: {text:?}"
            );
            for line in &text {
                assert!(display_width(line) <= usize::from(width), "{line:?}");
            }
        }
    }

    #[test]
    fn expanded_shows_full_saved_output_and_rows_grow() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        assert!(output_expandable(&shell, 40));
        let collapsed = rows(&shell, 40, 0, false, folds(false, false));
        let expanded = rows(&shell, 40, 0, false, folds(false, true));
        assert!(expanded > collapsed);
        let text = text_of(&shell, 40, folds(false, true));
        assert!(!text.iter().any(|line| line.contains("more lines")));
        assert_eq!(
            text.iter().filter(|line| line.contains("line ")).count(),
            20
        );
    }

    /// 参数区与输出区各自独立折叠：展开参数不动输出，展开输出不动参数。
    #[test]
    fn params_and_output_fold_independently() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let mut view = shell(&output, ProcessStateView::Exited { code: Some(0) });
        view.command = format!("echo {}", "y".repeat(200));
        assert!(params_expandable(&view, 40));
        let collapsed = rows(&view, 40, 0, false, folds(false, false));
        let params_only = rows(&view, 40, 0, false, folds(true, false));
        let output_only = rows(&view, 40, 0, false, folds(false, true));
        assert!(params_only > collapsed);
        assert!(output_only > collapsed);
        assert_ne!(params_only, output_only);
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

    /// 参数恒为浅黄；输出折叠恒 dim、展开恒默认前景。
    #[test]
    fn params_stay_light_yellow_and_output_follows_expanded_only() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        let dim = body_layout(&shell, 40, folds(false, false));
        let output_expanded = body_layout(&shell, 40, folds(false, true));
        let param_style = |layout: &tree::Layout| {
            layout
                .rows
                .iter()
                .flat_map(|row| row.segments.iter())
                .find(|segment| segment.text.contains("$ "))
                .map(|segment| segment.style)
                .expect("参数段")
        };
        assert_eq!(param_style(&dim), style::chat_tool_param());
        assert_eq!(param_style(&output_expanded), style::chat_tool_param());
        let out_style = |layout: &tree::Layout| {
            layout
                .rows
                .iter()
                .flat_map(|row| row.segments.iter())
                .find(|segment| segment.text.contains("line 0"))
                .map(|segment| segment.style)
                .expect("输出段")
        };
        assert_eq!(out_style(&dim), style::muted(), "折叠输出恒为 dim");
        assert_eq!(
            out_style(&output_expanded),
            style::text(),
            "展开的输出是默认前景"
        );
    }

    #[test]
    fn failed_command_is_marked_failed() {
        let shell = shell("", ProcessStateView::Failed);
        let text = text_of(&shell, 40, folds(false, false));
        assert!(text[0].contains("You run command"));
        let area = Rect::new(0, 0, 40, 10);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(
            &mut grid,
            area,
            0,
            &mut row,
            &shell,
            0,
            false,
            folds(false, false),
        );
        assert_eq!(grid.cell(0, 0).style.fg, style::agent_fail().fg);
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

    #[test]
    fn dropped_bytes_is_a_separate_fact_node() {
        let mut shell = shell("out\n", ProcessStateView::Exited { code: Some(0) });
        shell.truncated = true;
        shell.dropped_bytes = 512;
        let text = text_of(&shell, 50, folds(false, false));
        assert!(
            text.iter()
                .any(|line| line.contains("512 earlier bytes dropped")),
            "{text:?}"
        );
        assert!(
            !text.iter().any(|line| line.contains("more lines")),
            "{text:?}"
        );
    }
}
