//! `/fork` 面板渲染：Full session、消息标签与底边提示。

use super::*;
use crate::app::state::lx::slash::ForkEntry;
use crate::app::state::{ForkPanel, PanelWindow, SlashPanel};

fn buffer_lines(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

fn fork_lx() -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Fork(Box::new(ForkPanel {
        entries: vec![
            ForkEntry {
                message_id: None,
                label: String::new(),
                created_at_ms: None,
            },
            ForkEntry {
                message_id: Some("u2".into()),
                label: "deploy the release".into(),
                created_at_ms: Some(2_000),
            },
            ForkEntry {
                message_id: Some("u1".into()),
                label: "write tests".into(),
                created_at_ms: Some(1_000),
            },
        ],
        window: PanelWindow::new(1),
        filter: String::new(),
        filtered: vec![0, 1, 2],
        now_ms: 2_000 + 5 * 60_000,
    })));
    lx
}

#[test]
fn fork_panel_items_render_full_session_and_message_labels() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = fork_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());
    let lines = buffer_lines(&buf, area);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::LX_FORK_FULL_SESSION)),
        "Full session 必须渲染: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("deploy the release")),
        "消息文本必须渲染: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("5m ago")),
        "消息时间列必须渲染: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::LX_FORK_PANEL_TITLE)),
        "面板标题必须渲染: {lines:?}"
    );
}

#[test]
fn fork_panel_empty_filter_keeps_all_entries() {
    let mut panel = ForkPanel {
        entries: vec![
            ForkEntry {
                message_id: None,
                label: String::new(),
                created_at_ms: None,
            },
            ForkEntry {
                message_id: Some("u1".into()),
                label: "one".into(),
                created_at_ms: Some(1_000),
            },
            ForkEntry {
                message_id: Some("u2".into()),
                label: "two".into(),
                created_at_ms: Some(2_000),
            },
        ],
        window: PanelWindow::default(),
        filter: String::new(),
        filtered: vec![0, 1, 2],
        now_ms: 2_000,
    };
    panel.refilter("");
    assert_eq!(panel.filtered, vec![0, 1, 2]);
    assert_eq!(panel.visible().len(), 3);
}
