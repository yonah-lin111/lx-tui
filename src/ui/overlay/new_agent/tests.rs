//! New agent 弹窗的几何、命中与渲染测试。

use super::*;

use crate::detect::{AgentKind, InstalledAgent};
use crate::ui::widgets;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

fn agent(kind: AgentKind, command: &str) -> InstalledAgent {
    InstalledAgent {
        kind,
        command: command.to_string(),
    }
}

fn agents(count: usize) -> Vec<InstalledAgent> {
    crate::detect::ALL_AGENT_KINDS
        .iter()
        .take(count)
        .map(|kind| agent(*kind, text::agent_label(*kind)))
        .collect()
}

fn make_dialog(agents: Vec<InstalledAgent>, selected: usize) -> NewAgentDialog {
    NewAgentDialog {
        workspace: 0,
        agents,
        selected,
    }
}

fn draw(dialog: &NewAgentDialog, width: u16, height: u16) -> (Vec<String>, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal
        .draw(|frame| {
            render_new_agent(frame, frame.area(), dialog);
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    (lines, buffer)
}

#[test]
fn shell_height_follows_row_count_and_clamps() {
    let height = |count| {
        new_agent_shell(SCREEN, count)
            .expect("shell fits")
            .area
            .height
    };
    assert_eq!(height(0), 6);
    assert_eq!(height(1), 6);
    assert_eq!(height(2), 7);
    assert_eq!(height(8), 13);
    assert_eq!(height(100), 13, "超过 8 条按 8 条计算");

    let narrow = new_agent_shell(Rect::new(0, 0, 20, 8), 8).expect("narrow shell");
    assert_eq!(narrow.area.width, 20);
    assert_eq!(narrow.area.height, 8);
}

#[test]
fn max_rows_reserves_button_row_and_visible_start_follows_selection() {
    let shell = new_agent_shell(SCREEN, 3).expect("shell fits");
    assert_eq!(new_agent_max_rows(&shell), 3);

    let mut dialog = make_dialog(agents(8), 5);
    assert_eq!(new_agent_visible_start(&dialog, 8), 0);
    assert_eq!(new_agent_visible_start(&dialog, 3), 3, "高亮行滚入窗口");
    dialog.selected = 0;
    assert_eq!(new_agent_visible_start(&dialog, 3), 0);
    assert_eq!(new_agent_visible_start(&dialog, 0), 0);
}

#[test]
fn row_hit_maps_visible_offset_into_dialog_index() {
    let dialog = make_dialog(agents(8), 0);
    let shell = new_agent_shell(Rect::new(0, 0, 80, 8), dialog.agents.len()).expect("shell");
    assert_eq!(new_agent_max_rows(&shell), 3);
    let x = shell.inner.x;
    assert_eq!(new_agent_row_at(&shell, &dialog, x, shell.inner.y), Some(0));
    assert_eq!(
        new_agent_row_at(&shell, &dialog, x, shell.inner.y + 2),
        Some(2)
    );
    assert_eq!(
        new_agent_row_at(&shell, &dialog, x, shell.inner.y + 3),
        None,
        "按钮行不是候选行"
    );
    assert_eq!(
        new_agent_row_at(&shell, &dialog, shell.inner.x - 1, shell.inner.y),
        None,
        "内框外不命中"
    );

    let scrolled = make_dialog(agents(8), 5);
    assert_eq!(
        new_agent_row_at(&shell, &scrolled, x, shell.inner.y),
        Some(3),
        "窗口起点跟随高亮"
    );
}

#[test]
fn button_hit_targets_create_and_cancel() {
    let shell = new_agent_shell(SCREEN, 1).expect("shell");
    let rects = widgets::modal::button_row(
        shell.inner,
        &NEW_AGENT_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    let center = |rect: Rect| (rect.x + rect.width / 2, rect.y);
    let (create_x, create_y) = center(rects[0]);
    let (cancel_x, cancel_y) = center(rects[1]);
    assert_eq!(
        new_agent_button_at(&shell, create_x, create_y),
        Some(NewAgentButton::Create)
    );
    assert_eq!(
        new_agent_button_at(&shell, cancel_x, cancel_y),
        Some(NewAgentButton::Cancel)
    );
    assert_eq!(
        new_agent_button_at(&shell, shell.inner.x, shell.inner.y),
        None
    );
}

#[test]
fn render_lists_agents_and_highlights_selected_row() {
    let dialog = make_dialog(
        vec![
            agent(AgentKind::Claude, "claude"),
            agent(AgentKind::OpenCode, "opencode2"),
            agent(AgentKind::Kimi, "kimi"),
        ],
        1,
    );
    let (lines, buffer) = draw(&dialog, 80, 24);

    assert!(lines.iter().any(|line| line.contains("New agent")));
    assert!(lines.iter().any(|line| line.contains("claude")));
    assert!(lines.iter().any(|line| line.contains("kimi")));
    let opencode = lines
        .iter()
        .find(|line| line.contains("opencode2"))
        .expect("selected row shows hit command");
    assert!(opencode.contains("opencode"), "左侧仍是稳定 Agent 标签");

    let shell = new_agent_shell(SCREEN, 3).expect("shell");
    let selected_y = shell.inner.y + 1;
    let selected_x = shell.inner.x;
    assert_eq!(
        buffer[(selected_x, selected_y)].bg,
        Color::Indexed(240),
        "选中行整行铺选中底色"
    );
    assert_eq!(
        buffer[(selected_x, shell.inner.y)].bg,
        Color::Reset,
        "未选中行不上底色"
    );
}

#[test]
fn render_empty_state_and_muted_create_button() {
    let dialog = make_dialog(Vec::new(), 0);
    let (lines, buffer) = draw(&dialog, 80, 24);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No coding agents installed"))
    );
    assert!(lines.iter().any(|line| line.contains("create")));

    let shell = new_agent_shell(SCREEN, 0).expect("shell");
    let rects = widgets::modal::button_row(
        shell.inner,
        &NEW_AGENT_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    let create = rects[0];
    // 空列表禁用主按钮：不给实心强调色底。
    assert_ne!(buffer[(create.x, create.y)].bg, Color::Cyan);
}

#[test]
fn hidden_row_below_window_is_not_hit() {
    let dialog = make_dialog(agents(8), 0);
    let shell = new_agent_shell(Rect::new(0, 0, 80, 7), 8).expect("shell");
    assert_eq!(new_agent_max_rows(&shell), 2);
    assert_eq!(
        new_agent_row_at(&shell, &dialog, shell.inner.x, shell.inner.y + 2),
        None,
        "窗口外的行不命中"
    );
}
