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

/// 聊天步骤的配色必须：① 只用「浅粉 218 + 金黄 220 + dim」三层；
/// ② 结构与参数不压暗（dim 只用于只读元信息）；③ 与正文默认前景（Reset）、
/// 边框青（Cyan）都不同色；④ **不得出现任何蓝色槽位**（用户明确要求不回退旧配色）。
#[test]
fn chat_step_palette_is_native_pink_with_yellow_params() {
    // 结构与状态统一 Native 浅粉 218；可变参数沿用金黄 220；只读元信息退回 dim。
    let palette = [218u8, 220];
    let forbidden_blue = [117u8, 195, 168];
    let chrome = [None, Some(Color::Cyan)];

    for (label, style) in [
        ("方块-运行中", chat_step_mark_running()),
        ("方块-已完成", chat_step_mark_done()),
        ("文案-运行中", chat_step_label_running()),
        ("文案-已完成", chat_step_label_done()),
        ("工具动词", chat_tool_verb()),
        ("工具参数", chat_tool_param()),
    ] {
        let Some(Color::Indexed(index)) = style.fg else {
            panic!("{label} 必须取暖色槽位，实际 {:?}", style.fg);
        };
        assert!(
            palette.contains(&index),
            "{label} 用了暖色体系之外的槽位 Indexed({index})"
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

    // 可变参数沿用金黄 220 + 下划线（“每次调用会变的值”）。
    assert_eq!(
        chat_tool_param().fg,
        agent_param().fg,
        "可变参数沿用之前的金黄（与 [UI] 样例同源）"
    );
    assert!(
        chat_tool_param()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    // 只读元信息退回 dim：不再占用第三个色相档。
    assert_eq!(chat_tool_range(), muted(), "行范围退回 dim");
    assert!(
        !chat_tool_range()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}

/// markdown 正文高亮必须是**无色相**的：正文用终端默认前景，强调只靠修饰符，
/// 全链路最多一档低饱和灰绿（108）。这是用户“markdown 过于花里胡哨”的整改契约。
#[test]
fn markdown_palette_is_achromatic() {
    // 允许出现的显式前景：正文默认（None）与唯一的灰绿档。
    let allowed = [None, Some(Color::Indexed(108))];
    for (label, style) in [
        ("标题", markdown_heading()),
        ("h1", markdown_heading_level(1)),
        ("h2", markdown_heading_level(2)),
        ("h3", markdown_heading_level(3)),
        ("h4", markdown_heading_level(4)),
        ("h6", markdown_heading_level(6)),
        ("粗体", markdown_strong()),
        ("粗斜体", markdown_strong_emphasis()),
        ("斜体", markdown_emphasis()),
        ("删除线", markdown_strikethrough()),
        ("行内代码", markdown_inline_code()),
        ("代码块", markdown_code_block()),
        ("引用", markdown_quote()),
        ("链接文字", markdown_link_text()),
        ("URL", markdown_url()),
        ("项目符号", markdown_bullet()),
        ("有序序号", markdown_ordered()),
        ("任务已完成", markdown_task_done()),
        ("任务未完成", markdown_task_open()),
        ("水平线", markdown_rule()),
        ("代码语言标记", markdown_code_lang()),
        ("图片说明", markdown_image()),
        ("语法标记", markdown_marker()),
        ("@提及", markdown_file_mention()),
        ("refer-file", markdown_refer_file()),
        ("refer-folder", markdown_refer_folder()),
        ("refer-image", markdown_refer_image()),
        ("关键字", code_keyword()),
        ("字符串", code_string()),
        ("数字", code_number()),
        ("类型名", code_type()),
        ("注释", code_comment()),
    ] {
        assert!(
            allowed.contains(&style.fg),
            "{label} 的前景必须是默认或唯一灰绿档 108，实际 {:?}",
            style.fg
        );
    }

    // 强调必须落到修饰符上：标题/粗体加粗、斜体、下划线、删除线各有归属，
    // 不能出现“靠换色来表达强调”的空修饰符。
    assert!(markdown_heading().add_modifier.contains(Modifier::BOLD));
    assert!(markdown_strong().add_modifier.contains(Modifier::BOLD));
    assert!(markdown_emphasis().add_modifier.contains(Modifier::ITALIC));
    assert!(
        markdown_strikethrough()
            .add_modifier
            .contains(Modifier::CROSSED_OUT)
    );
    assert!(
        markdown_link_text()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    assert!(markdown_ordered().add_modifier.contains(Modifier::DIM));
}

/// 步骤侧暖色与正文无色相**色域不相交**：任何 `chat_*` token 都不得使用
/// markdown 那唯一的彩色档（108），结构/参数也不得回落到正文默认前景。
#[test]
fn markdown_palette_is_disjoint_from_chat_steps() {
    // markdown 侧实际会出现的**有色**前景（正文默认前景 None 不算色域）。
    let markdown_colors: Vec<Option<Color>> = [
        markdown_inline_code(),
        code_keyword(),
        code_string(),
        code_number(),
        code_type(),
        markdown_file_mention(),
        markdown_refer_file(),
        markdown_refer_folder(),
        markdown_refer_image(),
    ]
    .iter()
    .map(|style| style.fg)
    .collect();
    assert!(
        markdown_colors.iter().all(|color| color.is_some()),
        "markdown 的彩色档应当都带显式前景"
    );

    // 结构与参数必须是暖色（Some），且不得与 markdown 的有色档重合。
    for (label, style) in [
        ("方块-运行中", chat_step_mark_running()),
        ("方块-已完成", chat_step_mark_done()),
        ("文案-运行中", chat_step_label_running()),
        ("文案-已完成", chat_step_label_done()),
        ("工具动词", chat_tool_verb()),
        ("工具参数", chat_tool_param()),
    ] {
        assert!(
            style.fg.is_some(),
            "{label} 必须有显式暖色前景（不能与正文默认前景同色）"
        );
        assert!(
            !markdown_colors.contains(&style.fg),
            "{label} 不得复用 markdown 的彩色档：{:?}",
            style.fg
        );
    }
}
