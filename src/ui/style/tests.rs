//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn styles_stay_palette_native() {
    for style in [
        text(),
        muted(),
        accent(),
        border(true),
        border(false),
        error(),
        strong(),
    ] {
        assert!(matches!(
            style.fg,
            None | Some(Color::Cyan) | Some(Color::Red)
        ));
    }
}

#[test]
fn selected_item_fills_accent_with_contrast_foreground() {
    let style = selected_item();
    assert_eq!(style.fg, Some(Color::Black));
    assert_eq!(style.bg, Some(Color::Cyan));
    assert!(style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn overlay_panel_paints_indexed_surface_with_readable_foreground() {
    let style = overlay_panel();
    assert_eq!(style.fg, Some(Color::Indexed(252)));
    assert_eq!(style.bg, Some(Color::Indexed(236)));
}

#[test]
fn selection_uses_explicit_colors_without_reversed() {
    let style = selection();
    assert_eq!(style.fg, Some(Color::Indexed(255)));
    assert_eq!(style.bg, Some(Color::Indexed(240)));
    assert!(!style.add_modifier.contains(Modifier::REVERSED));
}

#[test]
fn overlay_selection_uses_contrast_surface_without_reversed() {
    let style = overlay_selection();
    assert_eq!(style.fg, Some(Color::Indexed(255)));
    assert_eq!(style.bg, Some(Color::Indexed(240)));
    assert!(!style.add_modifier.contains(Modifier::REVERSED));
}

#[test]
fn model_label_style_runs_the_entry_gradient_and_is_bold() {
    let first = model_label_style(0, 5);
    let last = model_label_style(4, 5);
    assert_eq!(first.fg, Some(Color::Indexed(218)), "起点 = 品牌粉");
    assert_eq!(last.fg, Some(Color::Indexed(117)), "终点 = 品牌蓝");
    assert!(first.add_modifier.contains(Modifier::BOLD), "加粗");
    assert_eq!(
        model_label_style(2, 5),
        lx_entry_label_style(LxEntry::Native, 2, 5, true),
        "与 Native 入口标签（加粗态）完全同款"
    );
    // 单字符标签取起点色，不除零。
    assert_eq!(model_label_style(0, 1).fg, Some(Color::Indexed(218)));
}

/// 聊天步骤的品牌色必须：① 只取 lx logo 的**粉色系**槽位（用户明确要求“只用粉色、
/// 不要变回原来的蓝色”）；② 不压暗（dim 会与正文 / 边框标题混同）；③ 与正文默认
/// 前景（Reset）、边框青（Cyan）都不同色。
#[test]
fn chat_step_palette_is_pink_saturated_and_distinct_from_chrome() {
    // lx logo 的粉色系：218 品牌粉、205 艳粉、211 浅粉、175 玫瑰灰、168 深粉。
    // 蓝色槽位（117 天蓝、195 冰蓝、245 灰蓝）**不得出现**——这正是被否掉的旧配色。
    let palette = [218u8, 205, 211, 175, 168];
    let forbidden_blue = [117u8, 195, 245];
    let chrome = [None, Some(Color::Cyan)];

    for (label, style) in [
        ("方块-运行中", chat_step_mark_running()),
        ("方块-已完成", chat_step_mark_done()),
        ("文案-运行中", chat_step_label_running()),
        ("文案-已完成", chat_step_label_done()),
        ("工具动词", chat_tool_verb()),
        ("工具参数", chat_tool_param()),
        ("工具行范围", chat_tool_range()),
    ] {
        let Some(Color::Indexed(index)) = style.fg else {
            panic!("{label} 必须取 256 色品牌槽位，实际 {:?}", style.fg);
        };
        assert!(
            palette.contains(&index),
            "{label} 用了粉色系之外的槽位 Indexed({index})"
        );
        assert!(
            !forbidden_blue.contains(&index),
            "{label} 又出现了蓝色槽位 Indexed({index})"
        );
        assert!(
            !style.add_modifier.contains(Modifier::DIM),
            "{label} 不得压暗（会与正文/边框标题混同）"
        );
        assert!(!chrome.contains(&style.fg), "{label} 与正文/边框青撞色");
    }

    // 语义成对：运行中 = 品牌粉、已完成 = 深粉（热 → 冷但不出蓝）。
    assert_eq!(chat_step_mark_running().fg, Some(Color::Indexed(218)));
    assert_eq!(chat_step_mark_done().fg, Some(Color::Indexed(168)));
    // 方块与头文案同色系但字重不同，方块更跳。
    assert_eq!(
        chat_step_mark_running().fg,
        chat_step_label_running().fg,
        "运行中方块与文案同色系"
    );
    assert!(
        chat_step_mark_running()
            .add_modifier
            .contains(Modifier::BOLD)
            && !chat_step_label_running()
                .add_modifier
                .contains(Modifier::BOLD),
        "同色系下落差靠字重"
    );
    // 工具行三档粉互不同色（动词最艳、路径浅一档、行范围最沉），
    // 一眼分出「动作 / 实体 / 元信息」。
    let tool_colors = [
        chat_tool_verb().fg,
        chat_tool_param().fg,
        chat_tool_range().fg,
    ];
    for (i, a) in tool_colors.iter().enumerate() {
        for b in &tool_colors[i + 1..] {
            assert_ne!(a, b, "工具行三段颜色不得重复：{tool_colors:?}");
        }
    }
    assert!(
        chat_tool_verb().add_modifier.contains(Modifier::BOLD)
            && !chat_tool_param().add_modifier.contains(Modifier::BOLD),
        "动词加粗、参数不加粗"
    );
    // 参数保留下划线（“每次调用会变的值”），行范围不加。
    assert!(
        chat_tool_param()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    assert!(
        !chat_tool_range()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}
