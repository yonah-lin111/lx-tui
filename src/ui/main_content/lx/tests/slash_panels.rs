//! `/model`、`/resume` 面板渲染测试：当前项 `❯` 标记、同底色高亮、时间列与去轨道滚动条。

use super::*;
use crate::app::state::lx::slash::{HistoryEntry, HistoryPanel};
use crate::app::state::{ModelCandidate, ModelPickerStage, SlashPanel};

/// 把整页缓冲区拼成文本行。
fn buffer_lines(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// 三个候选的 `/model` 第一级面板，指定索引为"当前模型"。
fn model_panel_lx(current: Option<usize>) -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: (0..3)
            .map(|index| ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "p".into(),
                    model: format!("model-{index}"),
                    variant: None,
                },
                provider_name: "Provider".into(),
                model_name: format!("model-{index}"),
            })
            .collect(),
        current,
        current_variant: None,
        window: PanelWindow::new(current.unwrap_or(0)),
        filter: String::new(),
        filtered: (0..3).collect(),
    })));
    lx
}

/// 两条历史的 `/resume` 面板，第 2 条为"当前会话"（5 分钟前更新）。
fn history_panel_lx() -> Lx {
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::History(Box::new(HistoryPanel {
        entries: vec![
            HistoryEntry {
                session_id: "sess-a".into(),
                title: "Alpha".into(),
                status: "active".into(),
                updated_at_ms: 1_000,
            },
            HistoryEntry {
                session_id: "sess-b".into(),
                title: "Beta".into(),
                status: "active".into(),
                updated_at_ms: 2_000,
            },
        ],
        current: Some(1),
        now_ms: 2_000 + 5 * 60_000,
        filter: String::new(),
        filtered: vec![0, 1],
        window: PanelWindow::new(1),
    })));
    lx
}

#[test]
fn model_panel_marks_current_model_with_marker() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(Some(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let lines = buffer_lines(&buf, area);
    let marked: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(text::CURRENT_ITEM_MARKER))
        .collect();
    assert_eq!(marked.len(), 1, "只有当前模型带标记: {lines:?}");
    assert!(marked[0].contains("model-1"), "{:?}", marked[0]);
}

#[test]
fn model_panel_without_current_model_has_no_marker() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(None);
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let lines = buffer_lines(&buf, area);
    assert!(
        lines
            .iter()
            .all(|line| !line.contains(text::CURRENT_ITEM_MARKER)),
        "无当前项时不画标记: {lines:?}"
    );
}

#[test]
fn model_panel_marker_cell_carries_selection_background() {
    // 当前项与键盘高亮同一行：底色存在、标记也在，互不覆盖。
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(Some(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let mut found = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if buf[(x, y)].symbol() == text::CURRENT_ITEM_MARKER {
                // ratatui 的 Cell 用 `Color::Reset` 表示未设背景。
                let expected = style::overlay_selection()
                    .bg
                    .unwrap_or(ratatui::style::Color::Reset);
                assert_eq!(buf[(x, y)].bg, expected, "标记格带选中底色");
                found = true;
            }
        }
    }
    assert!(found, "当前行应带 `❯` 标记");
}

#[test]
fn model_panel_names_stay_aligned_with_marker_column() {
    // 按单元格验证：标记行 `❯ 名字`，未标记行 `  名字`，名字左缘同列。
    let area = Rect::new(0, 0, 80, 24);
    let lx = model_panel_lx(Some(1));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let line_at = |y: u16| -> String { (0..area.width).map(|x| buf[(x, y)].symbol()).collect() };
    let row_of = |needle: &str| -> u16 {
        (0..area.height)
            .find(|y| line_at(*y).contains(needle))
            .expect("row")
    };
    let marked_row = row_of("model-1");
    let plain_row = row_of("model-0");
    assert!(
        line_at(marked_row).contains(&format!("{} model-1", text::CURRENT_ITEM_MARKER)),
        "标记行是 `❯ 名字`: {:?}",
        line_at(marked_row)
    );
    // 名字首字符的单元格列：标记列只占宽度，不挤动名字左缘。
    let first_name_cell = |line: &str, name: &str| -> u16 {
        let chars: Vec<char> = line.chars().collect();
        let needle: Vec<char> = name.chars().collect();
        let width = chars.len();
        let name_width = needle.len();
        (0..width.saturating_sub(name_width))
            .find(|start| chars[*start..*start + name_width] == needle[..])
            .map(|start| u16::try_from(start).unwrap_or(0))
            .unwrap_or(0)
    };
    assert_eq!(
        first_name_cell(&line_at(marked_row), "model-1"),
        first_name_cell(&line_at(plain_row), "model-0"),
        "标记项与未标记项名字同列"
    );
}

#[test]
fn history_panel_shows_relative_time_instead_of_session_id() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = history_panel_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let lines = buffer_lines(&buf, area);
    let all = lines.join("\n");
    assert!(all.contains("5m ago"), "显示相对时间: {all}");
    assert!(!all.contains("sess-a"), "不再显示会话 id: {all}");
    assert!(
        all.contains(text::CURRENT_ITEM_MARKER),
        "当前会话带标记: {all}"
    );
}

#[test]
fn history_panel_marks_only_bound_session() {
    let area = Rect::new(0, 0, 80, 24);
    let lx = history_panel_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let lines = buffer_lines(&buf, area);
    let marked: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(text::CURRENT_ITEM_MARKER))
        .collect();
    assert_eq!(marked.len(), 1, "只有已绑定会话带标记: {lines:?}");
    assert!(marked[0].contains("Beta"), "{:?}", marked[0]);
}

#[test]
fn slash_panel_scrollbar_has_no_track_border() {
    // 候选多于可见行时出现滚动条：只有 thumb，没有轨道竖线。
    let area = Rect::new(0, 0, 80, 24);
    let mut lx = Lx::new();
    lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: (0..30)
            .map(|index| ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "p".into(),
                    model: format!("model-{index}"),
                    variant: None,
                },
                provider_name: "Provider".into(),
                model_name: format!("model-{index}"),
            })
            .collect(),
        current: Some(0),
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: (0..30).collect(),
    })));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let layout = slash_layout(area, &lx).expect("panel");
    let bar = layout.scrollbar.expect("overflow");
    let lines = buffer_lines(&buf, area);
    assert!(lines.iter().any(|line| line.contains('▐')), "thumb 可见");
    assert!(
        lines.iter().all(|line| !line.contains('▕')),
        "轨道不再画竖线: {lines:?}"
    );
    // 轨道列在 thumb 之外是空格（保留占位，不画符号）。
    for y in bar.track.y..bar.track.bottom() {
        if y >= bar.thumb.y && y < bar.thumb.bottom() {
            continue;
        }
        let row: Vec<char> = lines[usize::from(y)].chars().collect();
        assert_eq!(row[usize::from(bar.track.x)], ' ', "轨道列留空");
    }
}

#[test]
fn history_panel_footer_documents_shift_enter_edit() {
    // `/resume` 底边必须出现 `⇧↵`（与 @ 面板同符号），否则用户无从发现编辑模态；
    // 也必须出现 `^c`，否则用户无从发现复制会话 id 的快捷键。
    let area = Rect::new(0, 0, 100, 24);
    let lx = history_panel_lx();
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let all = buffer_lines(&buf, area).join("\n");
    assert!(
        all.contains(crate::ui::text::LX_HISTORY_PANEL_FOOTER),
        "底边显示完整提示: {all}"
    );
    assert!(all.contains("⇧↵"), "使用 ⇧↵ 符号: {all}");
    assert!(all.contains("[copy ^c]"), "底边提示复制快捷键: {all}");
}

#[test]
fn history_panel_footer_symbols_match_mention_panel() {
    // 与 @ 面板同一套符号：`⇧↵` 与 `^z` 记法，不出现 shift/ctrl 全拼。
    let footer = crate::ui::text::LX_HISTORY_PANEL_FOOTER;
    assert!(footer.contains(crate::ui::text::key::SHIFT));
    assert!(footer.contains(crate::ui::text::key::ENTER));
    assert!(!footer.contains("shift"), "不用全拼 shift: {footer}");
    assert!(!footer.contains("ctrl"), "不用全拼 ctrl: {footer}");
    // 与 @ 面板尾部的 3 行对比：两个面板的 `⇧↵` 写法完全一致。
    assert!(crate::ui::text::MENTION_PANEL_FOOTER.contains("⇧↵"));
}

#[test]
fn model_panel_footer_stays_without_edit_hint() {
    // `/model` 没有编辑模态，底边保持通用提示（不放 `⇧↵`）。
    let area = Rect::new(0, 0, 100, 24);
    let lx = model_panel_lx(Some(0));
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None);
    let all = buffer_lines(&buf, area).join("\n");
    assert!(
        all.contains(crate::ui::text::LX_SLASH_PANEL_FOOTER),
        "{all}"
    );
    assert!(!all.contains("⇧↵"), "model 面板不放编辑提示: {all}");
}
