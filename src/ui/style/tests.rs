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

/// 边框标签前缀必须是"淡青 + dim、且显式去掉 BOLD"：
/// 叠加到强调色边框后仍不加粗，聚焦不再变亮变粗。
#[test]
fn border_label_prefix_is_soft_cyan_without_bold() {
    let style = border_label_prefix();
    assert_eq!(style.fg, Some(Color::Cyan), "前缀保持青色");
    assert!(style.add_modifier.contains(Modifier::DIM), "前缀带 dim");
    assert!(
        style.sub_modifier.contains(Modifier::BOLD),
        "显式移除边框继承的 BOLD"
    );
    let mut cell = ratatui::buffer::Cell::default();
    cell.set_style(accent());
    cell.set_style(style);
    assert_eq!(cell.fg, Color::Cyan);
    assert!(!cell.modifier.contains(Modifier::BOLD));
    assert!(cell.modifier.contains(Modifier::DIM));
}

/// 叠加在强调色边框上的灰字同样不能被染色、也不能继承粗体。
#[test]
fn muted_on_border_strips_inherited_bold() {
    let style = muted_on_border();
    assert_eq!(style.fg, Some(Color::Reset));
    assert!(style.sub_modifier.contains(Modifier::BOLD));
    let mut cell = ratatui::buffer::Cell::default();
    cell.set_style(accent());
    cell.set_style(style);
    assert_eq!(cell.fg, Color::Reset);
    assert!(!cell.modifier.contains(Modifier::BOLD));
    assert!(cell.modifier.contains(Modifier::DIM));
}

/// 边框标签值必须是"显式默认前景 + dim、且显式去掉 BOLD"：
/// 该样式叠加在边框单元格上，缺任一项都会把聚焦边框的 Cyan + BOLD 继承出来。
#[test]
fn border_label_value_resets_inherited_accent() {
    let style = border_label_value();
    assert_eq!(style.fg, Some(Color::Reset), "显式默认前景，切断边框青色");
    assert!(style.add_modifier.contains(Modifier::DIM), "保留 dim 一档");
    assert!(
        style.sub_modifier.contains(Modifier::BOLD),
        "显式移除边框继承的 BOLD"
    );
    // 应用到已带强调色的单元格后，取值必须等于"默认灰"，与焦点无关。
    let mut cell = ratatui::buffer::Cell::default();
    cell.set_style(accent());
    cell.set_style(style);
    assert_eq!(cell.fg, Color::Reset);
    assert!(!cell.modifier.contains(Modifier::BOLD));
    assert!(cell.modifier.contains(Modifier::DIM));
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

/// 后台进程"存活"橙色必须与既有色域全部拉得开，且**不复用**其他语义色。
///
/// 橙色是"任务还在后台跑"的专属信号：进程停止即回翡翠绿，所以它必须和绿（结构色）、
/// 金黄（可变参数）、品红（思考）、天蓝（连接符）都明显不同，否则用户分不出
/// "还在跑"与"已经停了"。同时不得压暗（dim 只留给只读元信息）。
#[test]
fn alive_process_yellow_is_distinct_from_every_other_step_color() {
    let orange = chat_step_mark_alive().fg.expect("存活色必须有前景");
    let Color::Indexed(index) = orange else {
        panic!("存活色必须取 256 色槽位，实际 {orange:?}");
    };
    assert_eq!(
        index, 208,
        "运行中橙黄固定为 Indexed 208（选色依据见 style.rs）"
    );

    // 动词/状态文字与方块同色系（同一档深黄），只靠字重区分。
    assert_eq!(
        chat_step_label_alive().fg,
        Some(orange),
        "状态文字与方块同色"
    );
    assert!(
        chat_step_mark_alive().add_modifier.contains(Modifier::BOLD),
        "存活方块加粗（与深绿完成态靠字重区分）"
    );
    assert!(
        !chat_step_label_alive()
            .add_modifier
            .contains(Modifier::BOLD),
        "状态文字不加粗，让方块衬出来"
    );
    for (label, style) in [
        ("方块", chat_step_mark_alive()),
        ("文案", chat_step_label_alive()),
    ] {
        assert!(
            !style.add_modifier.contains(Modifier::DIM),
            "{label} 不得压暗"
        );
    }

    // 不能与既有语义色撞车（撞了就分不出"活着"与"做完了/参数/思考"）。
    for (label, other) in [
        ("工具绿", chat_step_mark_done().fg),
        ("工具动词绿", chat_tool_verb().fg),
        ("思考品红", Some(Color::Indexed(218))),
        ("失败红", agent_fail().fg),
        ("正文默认", None),
    ] {
        assert_ne!(Some(orange), other, "存活深黄不得与{label}同色");
    }

    // 工具参数用次要信息色（用户要求「改为输入框下方上下文的字体颜色」）：
    // 不占第二个暖色槽位，存活橙黄仍是唯一的暖色结构色。
    assert_eq!(
        chat_tool_param().fg,
        muted().fg,
        "参数不上色，存活橙黄({index})是唯一的暖色结构色"
    );
}

/// 工具步骤的配色规则：**运行中橙黄 `Indexed 208`、完成才翡翠绿 `Indexed 42`**，
/// 不再附带任何行内参数色（参数用终端默认前景），三者都不压暗、不与正文默认前景/边框青撞色。
///
/// 用户要求「所有正在运行中的 tool 的方块和 title 统一为黄色，完成才显示绿色」：
/// 因此不再按工具种类分色（进程卡片 / 文件工具 / 计划卡片共用同一档），
/// 运行态由 `chat_step_mark_alive` / `chat_step_label_alive` / `chat_tool_verb_running`
/// 三个 token 表达，完成态仍走原来的绿。
#[test]
fn chat_step_palette_uses_amber_for_running_and_green_when_done() {
    let running_amber = Color::Indexed(208);
    let step_green = Color::Indexed(42);
    let palette = [208u8, 42];
    let chrome = [None, Some(Color::Cyan)];

    for (label, style) in [
        ("方块-运行中", chat_step_mark_alive()),
        ("方块-已完成", chat_step_mark_done()),
        ("文案-运行中", chat_step_label_running()),
        ("文案-已完成", chat_step_label_done()),
        ("动词-运行中", chat_tool_verb_running()),
        ("动词-已完成", chat_tool_verb()),
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

    // 运行态统一橙黄；完成态统一翡翠绿。
    for style in [
        chat_step_mark_alive(),
        chat_step_label_running(),
        chat_tool_verb_running(),
    ] {
        assert_eq!(style.fg, Some(running_amber), "运行态统一橙黄");
    }
    for style in [
        chat_step_mark_done(),
        chat_step_label_done(),
        chat_tool_verb(),
    ] {
        assert_eq!(style.fg, Some(step_green), "完成态统一翡翠绿");
    }
    assert_ne!(
        chat_step_mark_alive().fg,
        chat_step_mark_done().fg,
        "运行与完成必须不同色（这是「完成才显示绿色」的前提）"
    );
    assert!(
        chat_step_mark_alive().add_modifier.contains(Modifier::BOLD),
        "运行中方块加粗（呼吸 + 加粗是进行中信号）"
    );
    assert!(
        !chat_step_mark_done().add_modifier.contains(Modifier::BOLD),
        "完成方块不加粗，与运行中区分"
    );

    // 可变参数用次要信息色：不占第三个色相档，也不与运行/完成两档抢注意力。
    assert_eq!(
        chat_tool_param().fg,
        muted().fg,
        "工具参数用 dim（与上下文用量同档）"
    );
    assert!(
        !chat_tool_param()
            .add_modifier
            .contains(Modifier::UNDERLINED),
        "工具参数不加下划线"
    );
    // 只读元信息退回 dim：不占用第三个色相档。
    assert_eq!(chat_tool_range(), muted(), "行范围退回 dim");
    assert!(
        !chat_tool_range()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}

/// 工具行可变参数：与左下角上下文用量同档的**次要信息色**；悬停升为默认前景。
///
/// 用户要求「工具参数颜色改为输入框下方上下文的字体颜色」：那行用量是 `muted()`，
/// 参数跟着走；折叠参数悬停提亮为默认前景；**没有**展开块底色（只有 diff 增删行有底色）。
#[test]
fn folded_param_lifts_on_hover_and_no_block_background() {
    assert_eq!(
        chat_tool_param_hover(),
        Style::default(),
        "悬停提亮为默认前景"
    );
    assert_eq!(chat_tool_param().bg, None, "参数行不铺底色");
}

#[test]
fn chat_tool_param_matches_context_usage() {
    let style = chat_tool_param();
    assert_eq!(style, muted(), "参数与上下文用量同色（dim）");
    assert!(
        !style.add_modifier.contains(Modifier::UNDERLINED),
        "只有 dim，不加下划线（连续多行下划线会盖过翡翠绿结构色）"
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
        md_read_inline_code().fg,
        constant,
        "行内代码 = markup.inline.raw = #79c0ff"
    );

    // 文档结构：链接用 accent 蓝；引用/标记/URL/表格用 fg-muted。
    assert_eq!(md_read_link_text().fg, link, "链接 = #4493f8");
    assert!(
        md_read_link_text()
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    assert_eq!(md_read_url().fg, link, "裸 URL 同链接色");
    assert_eq!(md_read_quote().fg, muted, "引用 = fg-muted");
    assert_eq!(md_read_marker().fg, muted, "语法标记 = fg-muted");
    assert_eq!(md_read_rule().fg, muted, "水平线/表格边框 = fg-muted");
    assert_eq!(md_read_image().fg, muted, "图片说明 = fg-muted");
    // 任务复选框按状态分色（只染复选框，正文保持原样）。
    assert_eq!(md_read_task_done().fg, Some(Color::Green), "已完成 = 绿");
    assert_eq!(md_read_task_open().fg, Some(Color::Yellow), "未完成 = 黄");
    assert!(
        !md_read_task_done()
            .add_modifier
            .contains(Modifier::CROSSED_OUT),
        "已完成项不再整行加删除线"
    );

    // 行内代码 `xxx` 恢复为纯前景色（不铺背景）。
    assert_eq!(
        md_read_inline_code().bg,
        None,
        "行内代码不铺背景（按用户要求恢复原格式）"
    );

    // 列表符号是**结构强调**：用链接蓝只染符号，正文保持原色。
    assert_eq!(md_read_bullet().fg, link, "无序列表符号 = 链接蓝");
    assert_eq!(md_read_ordered().fg, link, "有序列表序号 = 链接蓝");

    // 其余结构性强调仍**不上色**，只用修饰符。
    for style in [
        md_read_strong(),
        md_read_strong_emphasis(),
        md_read_emphasis(),
        md_read_code_block(),
    ] {
        assert_eq!(style.fg, None, "GitHub 的结构强调不换色");
    }
    assert!(md_read_strong().add_modifier.contains(Modifier::BOLD));
    assert!(md_read_emphasis().add_modifier.contains(Modifier::ITALIC));
    assert!(
        md_read_strikethrough()
            .add_modifier
            .contains(Modifier::CROSSED_OUT)
    );

    // 层级靠下划线：GitHub 的 h1/h2 带 border-bottom。
    assert!(
        md_read_heading_level(1)
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
    assert!(
        md_read_heading_level(2)
            .add_modifier
            .contains(Modifier::UNDERLINED)
    );
}

/// 编辑器高亮（prompt 编辑器 / lx 输入框）是**独立于阅读渲染**的 ANSI 语义色板。
///
/// 用户要求两者不得混用：阅读侧（`md_read_*` / `code_*`）是 GitHub Dark 固定 RGB，
/// 编辑侧（`markdown_*`）是打字时的语义分色。这里的断言把编辑侧钉死在 ANSI 槽位，
/// 并显式检查它与阅读侧不同——谁把两边再合并一次都会立刻失败。
#[test]
fn editor_palette_stays_ansi_and_independent_from_reading() {
    // 编辑侧：标题/粗体/斜体 Yellow、行内代码与删除线 LightRed、引用 LightMagenta 斜体、
    // 链接 LightBlue 下划线、URL Cyan、标记 dim、提及黄/引用三色。
    assert_eq!(markdown_heading().fg, Some(Color::Yellow));
    assert!(markdown_heading().add_modifier.contains(Modifier::BOLD));
    assert_eq!(markdown_strong().fg, Some(Color::Yellow));
    assert_eq!(markdown_emphasis().fg, Some(Color::Yellow));
    assert!(markdown_emphasis().add_modifier.contains(Modifier::ITALIC));
    assert_eq!(markdown_inline_code().fg, Some(Color::LightRed));
    assert_eq!(markdown_strikethrough().fg, Some(Color::LightRed));
    assert_eq!(markdown_quote().fg, Some(Color::LightMagenta));
    assert!(markdown_quote().add_modifier.contains(Modifier::ITALIC));
    assert_eq!(markdown_link_text().fg, Some(Color::LightBlue));
    assert_eq!(markdown_url().fg, Some(Color::Cyan));
    assert_eq!(markdown_marker().fg, muted().fg);
    assert!(markdown_marker().add_modifier.contains(Modifier::DIM));

    // 提及与三类 refer 各自一色（与 prompt 设计文档一致）。
    assert_eq!(markdown_file_mention().fg, Some(Color::Yellow));
    assert_eq!(markdown_refer_file().fg, Some(Color::LightGreen));
    assert_eq!(markdown_refer_folder().fg, Some(Color::Indexed(208)));
    assert_eq!(markdown_refer_image().fg, Some(Color::LightMagenta));
    assert_eq!(markdown_short_ref().fg, Some(Color::Indexed(141)));

    // 两边独立：编辑侧不得出现 GitHub Dark 的任意 truecolor；阅读侧不得回到 ANSI 槽位。
    let editor = [
        markdown_heading(),
        markdown_strong(),
        markdown_emphasis(),
        markdown_inline_code(),
        markdown_strikethrough(),
        markdown_quote(),
        markdown_link_text(),
        markdown_url(),
        markdown_file_mention(),
        markdown_refer_file(),
        markdown_refer_folder(),
        markdown_refer_image(),
        markdown_short_ref(),
        markdown_marker(),
    ];
    for style in editor {
        assert!(
            !matches!(style.fg, Some(Color::Rgb(..))),
            "编辑器高亮不得使用阅读侧的 GitHub truecolor：{:?}",
            style.fg
        );
    }
    let reading = [
        md_read_marker(),
        md_read_strong(),
        md_read_emphasis(),
        md_read_inline_code(),
        md_read_quote(),
        md_read_link_text(),
        md_read_url(),
    ];
    for style in reading {
        assert!(
            matches!(style.fg, None | Some(Color::Rgb(..))),
            "阅读渲染不得回到 ANSI 槽位：{:?}",
            style.fg
        );
    }
}

/// 步骤侧结构色与 markdown 的 GitHub token **色域不相交**：方块/动词的翡翠绿
/// （`Indexed 42`）不得出现在任何 markdown token 上，反之亦然。
///
/// 这是「工具/思考/步骤方块要区别于 markdown 高亮」的机器可验形式：GitHub Dark
/// 的代码 token 是珊瑚红 / 浅蓝 / 蓝 / 紫 / 灰（全部 `Color::Rgb`），步骤结构色是
/// 单一 `Indexed 42`（+ 运行橙 208），两者永不相交。
#[test]
fn markdown_palette_is_disjoint_from_chat_steps() {
    let step_colors = [
        chat_step_mark_running().fg,
        chat_step_mark_done().fg,
        chat_step_label_running().fg,
        chat_step_label_done().fg,
        chat_tool_verb().fg,
    ];
    assert!(
        step_colors
            .iter()
            .all(|color| matches!(color, Some(Color::Indexed(_)))),
        "步骤结构色必须是 256 色槽位：{step_colors:?}"
    );

    let markdown_tokens = [
        ("行内代码", md_read_inline_code()),
        ("关键字", code_keyword()),
        ("字符串", code_string()),
        ("数字", code_number()),
        ("类型名", code_type()),
        ("注释", code_comment()),
        ("链接文字", md_read_link_text()),
        ("URL", md_read_url()),
        ("引用", md_read_quote()),
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

    // 思考：常态压暗为次要信息色，**悬停恢复品红 218**（用户要求恢复原来的 hover
    // 交互）；工具：翡翠绿 42。
    for (label, normal, hovered) in [
        (
            "思考方块",
            chat_step_mark_running_thinking(false),
            chat_step_mark_running_thinking(true),
        ),
        (
            "思考方块完成",
            chat_step_mark_done_thinking(false),
            chat_step_mark_done_thinking(true),
        ),
        (
            "思考标题运行中",
            chat_step_label_running_thinking(false),
            chat_step_label_running_thinking(true),
        ),
        (
            "思考标题完成",
            chat_step_label_done_thinking(false),
            chat_step_label_done_thinking(true),
        ),
    ] {
        assert_eq!(normal.fg, muted().fg, "{label}常态压暗");
        assert_eq!(hovered.fg, Some(Color::Indexed(218)), "{label}悬停恢复品红");
    }
    assert_eq!(
        chat_step_mark_running().fg,
        Some(Color::Indexed(42)),
        "工具方块用翡翠绿"
    );

    // 两个有色角色互不同色，且都不与正文的默认前景重合。
    let thinking = chat_step_mark_done_thinking(true).fg;
    let tool = chat_step_mark_done().fg;
    let body = chat_step_mark_done_body().fg;
    assert_ne!(thinking, tool, "思考与工具的方块必须不同色");
    assert_ne!(body, thinking, "正文与思考的方块必须不同色");
    assert_ne!(body, tool, "正文与工具的方块必须不同色");
}

/// loading 的扫描高亮色必须按 `Native Agent` 标签的 4 档渐变循环、且加粗。
///
/// 对话框上方已无方块，但「颜色取自 Native 入口标签」这条要求仍在——把循环表与
/// 标签的**实际取色**直接对齐，谁改了标签渐变两边会一起失败。
#[test]
fn loading_scan_cycles_native_agent_label_colors() {
    let cycle = crate::ui::text::agent::native_label_cycle();
    assert_eq!(cycle, &[218u8, 182, 153, 117], "4 档循环");

    // 与标签逐字符取色对账（去重、保持出现顺序）。
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
    assert_eq!(cycle, seen.as_slice(), "循环表 == 标签实际取色序列");

    for (phase, expected) in cycle.iter().enumerate() {
        let style = chat_loading_scan(phase);
        assert_eq!(style.fg, Some(Color::Indexed(*expected)), "第 {phase} 档");
        assert!(style.add_modifier.contains(Modifier::BOLD), "扫描高亮加粗");
    }
    assert_eq!(
        chat_loading_scan(cycle.len()),
        chat_loading_scan(0),
        "循环取模"
    );
}
