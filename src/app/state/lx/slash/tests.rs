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
        filter: String::new(),
        filtered: vec![0],
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
    // 换级后过滤视图按新阶段重建（真实路径由 apply_slash_enter 完成）。
    picker.filtered = vec![0, 1];
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
        filter: String::new(),
        filtered: vec![0, 1],
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

/// 过滤：空查询恢复全量恒等视图（顺序不变）。
#[test]
fn history_refilter_empty_query_restores_full_list() {
    let mut panel = HistoryPanel {
        entries: vec![entry("b", "Beta", 2), entry("a", "Alpha", 1)],
        current: None,
        now_ms: 10,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: Vec::new(),
    };
    panel.refilter("");
    assert_eq!(panel.filtered, vec![0, 1], "空查询是恒等视图");
    assert!(!panel.is_filter_empty(), "空查询不算过滤为空");
    panel.refilter("   ");
    assert_eq!(panel.filtered, vec![0, 1], "全空白同样不过滤");
}

/// 过滤：按标题匹配，前缀命中排在子序列命中之前。
#[test]
fn history_refilter_matches_title_with_prefix_first() {
    let mut panel = HistoryPanel {
        entries: vec![
            entry("s1", "重构登录模块", 3),
            entry("s2", "登录态过期修复", 2),
            entry("s3", "渲染性能", 1),
        ],
        current: None,
        now_ms: 10,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: Vec::new(),
    };
    panel.refilter("登录");
    // s2 标题以 `登录` 开头（前缀 800）→ 排前；s1 是子串命中（600）→ 排后。
    assert_eq!(panel.filtered, vec![1, 0], "前缀优先于子串");
    assert_eq!(panel.visible().len(), 2, "只留命中的两条");
    assert_eq!(panel.visible()[0].session_id, "s2", "首位是前缀命中");
}

/// 过滤：session id 也参与匹配（用户能直接搜 id）。
#[test]
fn history_refilter_matches_session_id() {
    let mut panel = HistoryPanel {
        entries: vec![
            entry("9a3f5c21e0b7", "Alpha", 2),
            entry("6f1c8d40aa11", "Beta", 1),
        ],
        current: None,
        now_ms: 10,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: Vec::new(),
    };
    panel.refilter("9a3f");
    assert_eq!(panel.filtered, vec![0], "按 id 前缀命中");
    assert_eq!(panel.visible()[0].session_id, "9a3f5c21e0b7");
}

/// 过滤：大小写不敏感。
#[test]
fn history_refilter_is_case_insensitive() {
    let mut panel = HistoryPanel {
        entries: vec![entry("s1", "Login Fix", 1)],
        current: None,
        now_ms: 10,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: Vec::new(),
    };
    panel.refilter("LOGIN");
    assert_eq!(panel.filtered, vec![0]);
}

/// 过滤：无命中时 `is_filter_empty` 为真（与「真的没有会话」区分）。
#[test]
fn history_refilter_marks_filter_empty_only_when_no_match() {
    let mut panel = HistoryPanel {
        entries: vec![entry("s1", "Alpha", 1)],
        current: None,
        now_ms: 10,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: Vec::new(),
    };
    panel.refilter("zzzz");
    assert!(panel.filtered.is_empty());
    assert!(panel.is_filter_empty(), "有过滤词且无命中");

    let mut empty = HistoryPanel::default();
    empty.refilter("");
    assert!(empty.filtered.is_empty());
    assert!(!empty.is_filter_empty(), "真无数据不算过滤为空");
}

/// 过滤：高亮越界后钳回范围内；退格恢复全量视图。
#[test]
fn history_refilter_clamps_highlight_and_recovers_on_backspace() {
    let mut panel = HistoryPanel {
        entries: vec![entry("s1", "Alpha", 1), entry("s2", "Beta", 2)],
        current: None,
        now_ms: 10,
        window: PanelWindow::new(1),
        filter: String::new(),
        filtered: Vec::new(),
    };
    panel.refilter("alp");
    assert_eq!(panel.filtered, vec![0]);
    assert_eq!(panel.window.active, 0, "高亮钳到唯一命中项");

    panel.refilter("");
    assert_eq!(panel.filtered, vec![0, 1], "退格恢复全量");
    assert_eq!(panel.window.active, 0, "高亮保持在范围内");
}

/// 过滤：已生效项被过滤掉时不再画标记（不把 `❯` 错贴到别的行）。
#[test]
fn history_filtered_current_index_is_none_when_filtered_out() {
    let mut panel = HistoryPanel {
        entries: vec![entry("s1", "Alpha", 1), entry("s2", "Beta", 2)],
        current: Some(1),
        now_ms: 10,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: vec![0, 1],
    };
    assert_eq!(
        SlashPanel::History(Box::new(panel.clone())).current_index(),
        Some(1),
        "未过滤时标记在第 2 行"
    );
    panel.refilter("alpha");
    assert_eq!(
        SlashPanel::History(Box::new(panel)).current_index(),
        None,
        "已生效项被过滤掉后不画标记"
    );
}

/// `/model` 第一级：模型名与 provider 名都参与匹配。
#[test]
fn model_picker_refilter_matches_model_and_provider() {
    let candidates = vec![
        ModelCandidate {
            selection: crate::config::ai::ModelSelection {
                provider: "p1".into(),
                model: "gpt-5".into(),
                variant: None,
            },
            provider_name: "9router".into(),
            model_name: "gpt-5".into(),
        },
        ModelCandidate {
            selection: crate::config::ai::ModelSelection {
                provider: "p2".into(),
                model: "deepseek".into(),
                variant: None,
            },
            provider_name: "penguin".into(),
            model_name: "deepseek".into(),
        },
    ];
    let mut picker = ModelPicker {
        stage: ModelPickerStage::Models,
        candidates,
        current: None,
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: vec![0, 1],
    };
    picker.refilter("gpt");
    assert_eq!(picker.filtered, vec![0], "按模型名命中");
    picker.refilter("penguin");
    assert_eq!(picker.filtered, vec![1], "按 provider 名命中");
    picker.refilter("router");
    assert_eq!(picker.filtered, vec![0], "provider 名子串也命中");
}

/// `/model` 第二级：按 variant 名匹配。
#[test]
fn model_picker_refilter_matches_variant_names() {
    let mut picker = ModelPicker {
        stage: ModelPickerStage::Variants {
            model: crate::config::ai::ModelSelection {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
            variants: vec!["low".into(), "high".into(), "max".into()],
        },
        candidates: Vec::new(),
        current: None,
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: vec![0, 1, 2],
    };
    picker.refilter("hi");
    assert_eq!(picker.filtered, vec![1], "按 variant 前缀命中");
    assert_eq!(picker.active_stage_index(), Some(1));
}

/// 一级高亮项经视图换算到原始候选（采纳时不能拿视图索引当数据索引）。
#[test]
fn model_picker_active_candidate_maps_view_index_to_real_index() {
    let mut picker = ModelPicker {
        stage: ModelPickerStage::Models,
        candidates: vec![
            ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "p".into(),
                    model: "gpt-5".into(),
                    variant: None,
                },
                provider_name: "9router".into(),
                model_name: "gpt-5".into(),
            },
            ModelCandidate {
                selection: crate::config::ai::ModelSelection {
                    provider: "p".into(),
                    model: "deepseek".into(),
                    variant: None,
                },
                provider_name: "9router".into(),
                model_name: "deepseek".into(),
            },
        ],
        current: None,
        current_variant: None,
        window: PanelWindow::new(0),
        filter: String::new(),
        filtered: vec![0, 1],
    };
    picker.refilter("deep");
    // 视图里只有 1 项，高亮位置 0 对应原始索引 1。
    assert_eq!(picker.window.active, 0);
    assert_eq!(picker.active_stage_index(), Some(1));
    assert_eq!(
        picker.active_candidate().map(|c| c.model_name.as_str()),
        Some("deepseek")
    );
}
