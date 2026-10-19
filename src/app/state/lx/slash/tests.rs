//! `/` 面板状态单元测试：相对时间、当前项与窗口定位。

use super::*;

/// 构造一条会话条目（`updated_at_ms` 为绝对毫秒）。
fn entry(session_id: &str, title: &str, updated_at_ms: i64) -> HistoryEntry {
    HistoryEntry {
        session_id: session_id.into(),
        title: title.into(),
        status: "active".into(),
        updated_at_ms,
    }
}

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

#[test]
fn time_label_is_relative_within_a_week() {
    let now = 1_800_000_000_000;
    let label = |elapsed: i64| entry("s", "t", now - elapsed).time_label(now);
    assert_eq!(label(0), "just now");
    assert_eq!(label(30_000), "just now");
    assert_eq!(label(MINUTE), "1m ago");
    assert_eq!(label(59 * MINUTE), "59m ago");
    assert_eq!(label(HOUR), "1h ago");
    assert_eq!(label(23 * HOUR), "23h ago");
    assert_eq!(label(DAY), "1d ago");
    assert_eq!(label(6 * DAY), "6d ago");
}

#[test]
fn time_label_switches_to_absolute_date_beyond_a_week() {
    let now = 1_800_000_000_000;
    // 8 天前：改绝对日期，不再显示 `8d ago`。
    let label = entry("s", "t", now - 8 * DAY).time_label(now);
    assert!(
        label.len() == 10 && label.matches('-').count() == 2,
        "应为 YYYY-MM-DD: {label}"
    );
    assert!(!label.contains("ago"), "{label}");
}

#[test]
fn time_label_treats_future_timestamps_as_just_now() {
    // 时钟回拨：不显示负数。
    let now = 1_800_000_000_000;
    assert_eq!(entry("s", "t", now + HOUR).time_label(now), "just now");
}

#[test]
fn absolute_date_formats_known_epochs() {
    // 1970-01-01 / 2000-03-01 / 2024-02-29（闰日）。
    assert_eq!(format_date(0), "1970-01-01");
    assert_eq!(format_date(951_868_800_000), "2000-03-01");
    assert_eq!(format_date(1_709_164_800_000), "2024-02-29");
}

#[test]
fn panel_window_new_puts_target_at_bottom_of_window() {
    // `new(active)` 把高亮当窗口底部锚点：候选很多时高亮贴底可见。
    let window = PanelWindow::new(30);
    assert_eq!(window.active, 30);
    assert_eq!(window.anchor, 30);
    assert_eq!(window.viewport, None, "未滚轮时不引入显式视口");
}

#[test]
fn model_picker_reports_current_index_per_stage() {
    let candidates = vec![ModelCandidate {
        selection: crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: Some("high".into()),
        },
        provider_name: "Provider".into(),
        model_name: "Model".into(),
    }];
    let mut picker = ModelPicker {
        stage: ModelPickerStage::Models,
        candidates,
        current: Some(0),
        current_variant: None,
        window: PanelWindow::new(0),
    };
    let panel = SlashPanel::Model(Box::new(picker.clone()));
    assert_eq!(panel.current_index(), Some(0), "第一级标当前模型");

    picker.stage = ModelPickerStage::Variants {
        model: crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: Some("high".into()),
        },
        variants: vec!["low".into(), "high".into()],
    };
    picker.current_variant = Some(1);
    let panel = SlashPanel::Model(Box::new(picker));
    assert_eq!(panel.current_index(), Some(1), "第二级标当前 variant");
}

#[test]
fn history_panel_reports_current_session() {
    let panel = HistoryPanel {
        entries: vec![entry("a", "A", 1), entry("b", "B", 2)],
        current: Some(1),
        now_ms: 10,
        window: PanelWindow::new(1),
    };
    assert_eq!(
        SlashPanel::History(Box::new(panel)).current_index(),
        Some(1)
    );
}

#[test]
fn commands_panel_has_no_current_item() {
    let panel = SlashPanel::Commands {
        items: vec![SlashCommand::Model, SlashCommand::Resume],
        window: PanelWindow::new(0),
    };
    assert_eq!(panel.current_index(), None, "命令候选没有已生效项");
}
