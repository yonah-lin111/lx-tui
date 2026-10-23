//! 本地命令卡片：标题、命令、输出预览与退出信息。
//!
//! 布局与绘制共用 [`lines`]：滚动条、置顶行与点击/复制几何不会与真实绘制漂移。
//! 预览最多 6 个物理行，保留首尾并显式标出省略行数；展开不在此处实现。

use ratatui::layout::Rect;
use ratatui::style::Style;
#[cfg(test)]
use unicode_width::UnicodeWidthChar;

use crate::app::state::UserShellView;
use crate::ui::{style, text};

use super::{Grid, STEP_GUTTER, visible_row, wrap};

/// 输出预览最多占用的物理行（含省略提示行）。
pub const PREVIEW_LINES: usize = 6;

/// 卡片一段（已按宽度换行）：文本 + 样式。
struct Line {
    text: String,
    style: Style,
}

/// 按宽度生成卡片的全部物理行（布局与绘制唯一来源）。
fn lines(
    shell: &UserShellView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
) -> Vec<Line> {
    let body_width = width.saturating_sub(STEP_GUTTER).max(1);
    let running = shell.is_running();
    let mut out = Vec::new();
    let (mark, mark_style) = header_mark(shell, loading_frame, animating);
    let label = if running {
        text::LX_SHELL_RUNNING
    } else {
        text::LX_SHELL_YOU_RAN
    };
    out.push(Line {
        text: format!("{mark} {label}"),
        style: mark_style,
    });
    // 命令：保留用户脚本的字面值，按内容宽度换行。
    for line in wrap(&format!("$ {}", shell.command), body_width) {
        out.push(Line {
            text: format!("  {line}"),
            style: style::text(),
        });
    }
    // 输出预览：折叠态首尾保留、中间省略；展开态显示全部受控输出。
    let preview = if expanded {
        all_output_lines(&shell.output, body_width)
    } else {
        preview_lines(&shell.output, body_width)
    };
    if preview.is_empty() {
        out.push(Line {
            text: format!(
                "  {}",
                if running {
                    text::LX_SHELL_WAITING_OUTPUT
                } else {
                    text::LX_SHELL_NO_OUTPUT
                }
            ),
            style: style::muted(),
        });
    } else {
        for line in preview {
            out.push(Line {
                text: format!("  {line}"),
                style: style::muted(),
            });
        }
    }
    if shell.output_truncated() {
        out.push(Line {
            text: format!("  … {} earlier bytes dropped", shell.dropped_bytes),
            style: style::agent_warn(),
        });
    }
    if !running {
        out.push(Line {
            text: format!("  {}", footer(shell)),
            style: style::muted(),
        });
    }
    out
}

/// 卡片占用的渲染行数。
pub fn rows(
    shell: &UserShellView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
) -> usize {
    lines(shell, width, loading_frame, animating, expanded).len()
}

/// 折叠态是否有可展开内容（没有更多输出时点击不会改变任何东西）。
pub fn has_expandable_content(shell: &UserShellView, width: u16) -> bool {
    let body = width.saturating_sub(STEP_GUTTER).max(1);
    all_output_lines(&shell.output, body).len() > PREVIEW_LINES
}

/// 绘制卡片并推进 `row_index`。
pub fn draw(
    grid: &mut Grid,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
    shell: &UserShellView,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
) {
    for line in lines(shell, content.width, loading_frame, animating, expanded) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line.text, line.style, content.width);
        }
        *row_index += 1;
    }
}

/// 标题方块与颜色：运行中橙/呼吸，失败红，成功中性/绿。
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

/// 输出预览：最多 [`PREVIEW_LINES`] 行；超出时保留前 3 行 + 省略提示 + 后 2 行。
fn preview_lines(output: &str, width: u16) -> Vec<String> {
    let rows = all_output_lines(output, width);
    if rows.len() <= PREVIEW_LINES {
        return rows;
    }
    let omitted = rows.len().saturating_sub(5);
    let mut preview: Vec<String> = rows[..3].to_vec();
    preview.push(format!("… {omitted} lines omitted"));
    preview.extend(rows[rows.len() - 2..].iter().cloned());
    preview
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

    #[test]
    fn draw_advances_exactly_the_layout_row_count() {
        let shell = shell(
            "line 1\nline 2\n",
            ProcessStateView::Exited { code: Some(0) },
        );
        let area = Rect::new(0, 0, 40, 20);
        let expected = rows(&shell, area.width, 0, false, false);
        let mut grid = Grid::new(area);
        let mut index = 0;
        draw(&mut grid, area, 0, &mut index, &shell, 0, false, false);
        assert_eq!(index, expected, "绘制与布局行数必须一致");
    }

    #[test]
    fn preview_keeps_head_tail_and_marks_omitted_lines() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        let text: Vec<String> = lines(&shell, 40, 0, false, false)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert!(text.iter().any(|line| line.contains("line 0")), "保留首部");
        assert!(text.iter().any(|line| line.contains("line 19")), "保留尾部");
        assert!(text.iter().any(|line| line.contains("lines omitted")));
        assert_eq!(
            text.iter()
                .filter(|line| line.trim_start().starts_with("line "))
                .count(),
            5,
            "最多保留 5 行真实输出"
        );
    }

    #[test]
    fn running_and_finished_use_distinct_labels() {
        let running = shell("", ProcessStateView::Running);
        let done = shell("", ProcessStateView::Exited { code: Some(0) });
        assert!(
            lines(&running, 40, 0, false, false)[0]
                .text
                .contains("Running")
        );
        assert!(
            lines(&done, 40, 0, false, false)[0]
                .text
                .contains("You ran")
        );
        assert!(
            lines(&running, 40, 0, false, false)
                .iter()
                .any(|line| line.text.contains("waiting for output"))
        );
        assert!(
            lines(&done, 40, 0, false, false)
                .iter()
                .any(|line| line.text.contains("(no output)"))
        );
    }

    #[test]
    fn long_lines_wrap_inside_width() {
        let shell = shell(&"x".repeat(200), ProcessStateView::Exited { code: Some(0) });
        for line in lines(&shell, 20, 0, false, false) {
            assert!(
                display_width(&line.text) <= 20,
                "行不得超宽: {}",
                display_width(&line.text)
            );
        }
    }

    #[test]
    fn expanded_shows_full_saved_output_and_rows_grow() {
        let output: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let shell = shell(&output, ProcessStateView::Exited { code: Some(0) });
        assert!(has_expandable_content(&shell, 40));
        let collapsed = lines(&shell, 40, 0, false, false);
        let expanded = lines(&shell, 40, 0, false, true);
        assert!(expanded.len() > collapsed.len());
        assert!(
            !expanded
                .iter()
                .any(|line| line.text.contains("lines omitted"))
        );
        assert!(
            expanded
                .iter()
                .filter(|line| line.text.trim_start().starts_with("line "))
                .count()
                == 20
        );
    }

    #[test]
    fn failed_command_is_marked_failed() {
        let shell = shell("", ProcessStateView::Failed);
        let first = &lines(&shell, 40, 0, false, false)[0];
        assert_eq!(first.style, style::agent_fail());
        assert!(first.text.contains("You ran"));
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
