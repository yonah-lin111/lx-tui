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

/// 聊天步骤的配色必须：① 结构色用翡翠绿 `Indexed 42`、可变参数用金黄 220；
/// ② 不压暗（dim 只用于只读元信息）；③ 与正文默认前景（Reset）、边框青（Cyan）
/// 都不同色；④ 与 markdown 的 GitHub Dark token 色域不相交。
#[test]
fn chat_step_palette_uses_green_structure_and_gold_params() {
    // 用户带截图反馈「方块与正文颜色特别相近」后改定的结构色（理由见 style.rs 注释）。
    let step_green = Color::Indexed(42);
    let palette = [42u8, 220];
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
            panic!("{label} 必须取品牌槽位，实际 {:?}", style.fg);
        };
        assert!(
            palette.contains(&index),
            "{label} 用了体系之外的槽位 Indexed({index})"
        );
        assert!(
            !style.add_modifier.contains(Modifier::DIM),
            "{label} 不得压暗（会与正文/边框标题混同）"
        );
        assert!(!chrome.contains(&style.fg), "{label} 与正文/边框青撞色");
    }

    // 方块 / 文案 / 动词统一翡翠绿；运行中用字重 + 呼吸区分于完成态。
    for style in [
        chat_step_mark_running(),
        chat_step_mark_done(),
        chat_step_label_running(),
        chat_step_label_done(),
        chat_tool_verb(),
    ] {
        assert_eq!(style.fg, Some(step_green), "统一翡翠绿结构色");
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
        "可变参数沿用既有金黄（与 [UI] 样例同源）"
    );
    assert!(
        chat_tool_param()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    // 只读元信息退回 dim：不占用第三个色相档。
    assert_eq!(chat_tool_range(), muted(), "行范围退回 dim");
    assert!(
        !chat_tool_range()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}

/// markdown 高亮必须是 **GitHub Dark 的官方 token 色**（用户要求“用 github 的黑暗模式”）。
///
/// 断言钉死每个语义 token 的 RGB：谁把它改回 ANSI 槽位、或换成别的主题色，都立刻失败。
#[test]
fn markdown_palette_is_github_dark_truecolor() {
    // GitHub Dark：--color-prettylights-syntax-* 与 --color-accent-fg。
    let keyword = Some(Color::Rgb(0xff, 0x7b, 0x72)); // #ff7b72
    let string = Some(Color::Rgb(0xa5, 0xd6, 0xff)); // #a5d6ff
    let constant = Some(Color::Rgb(0x79, 0xc0, 0xff)); // #79c0ff
    let entity = Some(Color::Rgb(0xd2, 0xa8, 0xff)); // #d2a8ff
    let comment = Some(Color::Rgb(0x8b, 0x94, 0x9e)); // #8b949e
    let muted = Some(Color::Rgb(0x7d, 0x85, 0x90)); // #7d8590
    let link = Some(Color::Rgb(0x44, 0x93, 0xf8)); // #4493f8

    // 语法高亮：一一对应 prettylights token。
    assert_eq!(code_keyword().fg, keyword, "关键字 = #ff7b72");
    assert_eq!(code_string().fg, string, "字符串 = #a5d6ff");
    assert_eq!(code_number().fg, constant, "数字 = #79c0ff");
    assert_eq!(code_type().fg, entity, "类型名 = #d2a8ff");
    assert_eq!(code_comment().fg, comment, "注释 = #8b949e");
    assert_eq!(
        markdown_inline_code().fg,
        constant,
        "行内代码 = markup.inline.raw = #79c0ff"
    );

    // 文档结构：链接用 accent 蓝；引用/标记/URL/表格用 fg-muted。
    assert_eq!(markdown_link_text().fg, link, "链接 = #4493f8");
    assert!(
        markdown_link_text()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    assert_eq!(markdown_url().fg, link, "裸 URL 同链接色");
    assert_eq!(markdown_quote().fg, muted, "引用 = fg-muted");
    assert_eq!(markdown_marker().fg, muted, "语法标记 = fg-muted");
    assert_eq!(markdown_rule().fg, muted, "水平线/表格边框 = fg-muted");
    assert_eq!(markdown_code_lang().fg, muted, "语言标记 = fg-muted");
    assert_eq!(markdown_image().fg, muted, "图片说明 = fg-muted");
    assert_eq!(markdown_task_done().fg, muted);
    assert_eq!(markdown_task_open().fg, muted);

    // 提及/引用与链接同源（GitHub 里都是可跳转引用）。
    for style in [
        markdown_file_mention(),
        markdown_refer_file(),
        markdown_refer_folder(),
        markdown_refer_image(),
    ] {
        assert_eq!(style.fg, link, "提及/引用用链接蓝");
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    // GitHub Dark 的结构性强调**不上色**，只用修饰符。
    for style in [
        markdown_heading(),
        markdown_strong(),
        markdown_strong_emphasis(),
        markdown_emphasis(),
        markdown_bullet(),
        markdown_ordered(),
        markdown_code_block(),
    ] {
        assert_eq!(style.fg, None, "GitHub 的结构强调不换色");
    }
    assert!(markdown_heading().add_modifier.contains(Modifier::BOLD));
    assert!(markdown_emphasis().add_modifier.contains(Modifier::ITALIC));
    assert!(
        markdown_strikethrough()
            .add_modifier
            .contains(Modifier::CROSSED_OUT)
    );

    // 层级靠下划线：GitHub 的 h1/h2 带 border-bottom。
    assert!(
        markdown_heading_level(1)
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    assert!(
        markdown_heading_level(2)
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}

/// 步骤侧结构色与 markdown 的 GitHub token **色域不相交**：方块/动词的翡翠绿
/// （`Indexed 42`）不得出现在任何 markdown token 上，反之亦然。
///
/// 这是「工具/思考/步骤方块要区别于 markdown 高亮」的机器可验形式：GitHub Dark
/// 的代码 token 是珊瑚红 / 浅蓝 / 蓝 / 紫 / 灰（全部 `Color::Rgb`），步骤结构色是
/// 单一 `Indexed 42` + 金黄 220，两者永不相交。
#[test]
fn markdown_palette_is_disjoint_from_chat_steps() {
    let step_colors = [
        chat_step_mark_running().fg,
        chat_step_mark_done().fg,
        chat_step_label_running().fg,
        chat_step_label_done().fg,
        chat_tool_verb().fg,
        chat_tool_param().fg,
    ];
    assert!(
        step_colors
            .iter()
            .all(|color| matches!(color, Some(Color::Indexed(_)))),
        "步骤结构色必须是 256 色槽位：{step_colors:?}"
    );

    let markdown_tokens = [
        ("行内代码", markdown_inline_code()),
        ("关键字", code_keyword()),
        ("字符串", code_string()),
        ("数字", code_number()),
        ("类型名", code_type()),
        ("注释", code_comment()),
        ("链接文字", markdown_link_text()),
        ("URL", markdown_url()),
        ("引用", markdown_quote()),
        ("提及", markdown_file_mention()),
    ];
    for (label, style) in markdown_tokens {
        assert!(
            !step_colors.contains(&style.fg),
            "markdown 的 {label} 不得复用步骤结构色：{:?}",
            style.fg
        );
    }
}

/// 三类步骤（正文 / 思考 / 工具）的配色必须**两两可辨**。
///
/// 用户要求：「设置正文内容的小方块为默认字体的颜色即可；thinking 的小方块和思考的
/// 字体颜色使用其他的颜色」。因此：
/// - 正文 = 终端默认前景（`fg == None`），方块与正文本就同色；
/// - 思考 = 品红 218；
/// - 工具 = 翡翠绿 42。
#[test]
fn step_roles_use_distinct_colors() {
    // 正文：不设前景（= 默认字体颜色），运行/完成只差字重。
    assert_eq!(
        chat_step_mark_running_body().fg,
        None,
        "正文方块的运行态不设前景（默认字体色）"
    );
    assert_eq!(chat_step_mark_done_body().fg, None, "正文方块的完成态同上");
    assert_eq!(chat_step_label_running_body().fg, None);
    assert_eq!(chat_step_label_done_body().fg, None);
    assert!(
        chat_step_mark_running_body()
            .add_modifier
            .contains(Modifier::BOLD),
        "正文方块运行中加粗"
    );
    assert!(
        !chat_step_mark_done_body()
            .add_modifier
            .contains(Modifier::BOLD),
        "正文方块完成去加粗"
    );

    // 思考：品红 218；工具：翡翠绿 42。
    assert_eq!(
        chat_step_mark_running_thinking().fg,
        Some(Color::Indexed(218)),
        "思考方块用品红"
    );
    assert_eq!(
        chat_step_label_running_thinking().fg,
        Some(Color::Indexed(218))
    );
    assert_eq!(
        chat_step_label_done_thinking().fg,
        Some(Color::Indexed(218))
    );
    assert_eq!(
        chat_step_mark_running().fg,
        Some(Color::Indexed(42)),
        "工具方块用翡翠绿"
    );

    // 两个有色角色互不同色，且都不与正文的默认前景重合。
    let thinking = chat_step_mark_done_thinking().fg;
    let tool = chat_step_mark_done().fg;
    let body = chat_step_mark_done_body().fg;
    assert_ne!(thinking, tool, "思考与工具的方块必须不同色");
    assert_ne!(body, thinking, "正文与思考的方块必须不同色");
    assert_ne!(body, tool, "正文与工具的方块必须不同色");
}

/// loading 卡片的循环色必须**严格来自 `Native Agent` 标签渐变**，不能自己另起一套。
///
/// 用户要求「小卡片的颜色使用 native agent 的字体颜色循环显示」。这条测试把
/// 「loading 的四档色」与「标签逐字符取色」直接对齐：以后谁调 `lx_entry_gradient`
/// 或换循环表，两边必须一起动。
#[test]
fn loading_cycle_matches_native_agent_label_gradient() {
    let cycle = crate::ui::text::agent::native_label_cycle();
    assert!(!cycle.is_empty(), "循环表不能为空");

    // 标签上真正出现过的颜色集合（去重、保持出现顺序）。
    let label = crate::ui::text::LX_ENTRY_NATIVE;
    let length = label.chars().count();
    let mut seen: Vec<u8> = Vec::new();
    for position in 0..length {
        let Some(Color::Indexed(index)) =
            lx_entry_label_style(LxEntry::Native, position, length, false).fg
        else {
            panic!("标签取色必须是 256 色槽位");
        };
        if !seen.contains(&index) {
            seen.push(index);
        }
    }
    assert_eq!(cycle, seen.as_slice(), "循环表 == 标签上实际出现的颜色序列");

    // 每一档都能被 `chat_loading_phase` 取到，且加粗。
    for (phase, expected) in cycle.iter().enumerate() {
        let style = chat_loading_phase(phase);
        assert_eq!(style.fg, Some(Color::Indexed(*expected)), "第 {phase} 档");
        assert!(style.add_modifier.contains(Modifier::BOLD), "加载色加粗");
    }
    // 循环取模：走完一轮回到起点。
    assert_eq!(
        chat_loading_phase(cycle.len()),
        chat_loading_phase(0),
        "循环取模"
    );
}
