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

/// 聊天步骤的配色必须：① 只取「Native 浅粉 218 + 金黄 220 + 灰 245」这套；
/// ② 不压暗（dim 会与正文 / 边框标题混同）；③ 与正文默认前景（Reset）、边框青（Cyan）
/// 都不同色；④ **不得出现任何蓝色槽位**（用户明确要求不回退旧配色）。
#[test]
fn chat_step_palette_is_native_pink_with_yellow_params() {
    // Native 入口标签渐变左侧的浅粉 218 是唯一的结构/动作色；可变参数沿用金黄 220；
    // 行范围沿用灰 245。蓝色槽位（117 天蓝、195 冰蓝）**不得出现**。
    let palette = [218u8, 220, 245];
    let forbidden_blue = [117u8, 195, 168];
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
            "{label} 用了体系之外的槽位 Indexed({index})"
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

    // 方块 / 文案 / 动词统一 Native 浅粉；运行中用字重 + 呼吸区分于完成态。
    for style in [
        chat_step_mark_running(),
        chat_step_mark_done(),
        chat_step_label_running(),
        chat_step_label_done(),
        chat_tool_verb(),
    ] {
        assert_eq!(style.fg, Some(Color::Indexed(218)), "统一 Native 浅粉");
    }
    assert!(
        chat_step_mark_running()
            .add_modifier
            .contains(Modifier::BOLD),
        "运行中方块加粗（呼吸 + 加粗是唯一的进行中信号）"
    );
    assert!(
        !chat_step_mark_done().add_modifier.contains(Modifier::BOLD),
        "完成方块不加粗，与运行中区分"
    );

    // 可变参数恢复为既有金黄 220 + 下划线（“每次调用会变的值”）。
    assert_eq!(
        chat_tool_param().fg,
        agent_param().fg,
        "可变参数回到之前的金黄（与 [UI] 样例同源）"
    );
    assert!(
        chat_tool_param()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    // 行范围沿用既有灰 245、无下划线，与参数区分。
    assert_eq!(chat_tool_range().fg, agent_range().fg, "行范围沿用既有灰");
    assert!(
        !chat_tool_range()
            .add_modifier
            .contains(Modifier::UNDERLINED)
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
