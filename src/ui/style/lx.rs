//! lx 页专属样式：品牌渐变、AI Web 站点品牌渐变、聊天步骤/工具行配色与会话标题胶囊配色。

use ratatui::style::{Color, Modifier, Style};

use crate::app::state::{AiWebSite, LxEntry};

use super::muted;

// ===== 聊天步骤 / 工具行的配色（翡翠绿结构色 + 默认色参数）=====
//
// lx 聊天历史里同时存在两类着色信息，必须一眼分得开：
//   A. **步骤结构**（方块、思考头、工具行）——本组 `chat_*` token，分三层：
//      ① 结构 / 状态（方块、头部文案、工具动词）：**翡翠绿 `Indexed 42`**（`#00d787`）；
//      ② 可变参数（路径 / 搜索范围）：**终端默认前景**（不额外占色相）；
//      ③ 其余元信息与行内动作（行范围、计数、状态文字、`open` / `collapse`）：
//         `muted()`（终端 dim，无色相），动作文字悬停升为默认前景。
//   B. **正文高亮**（markdown / 代码）——GitHub Dark 官方色板，颜色只出现在链接（蓝）
//      与代码 token（珊瑚红 / 浅蓝 / 蓝 / 紫 / 灰）上，标题 / 粗体 / 斜体 / 引用全部是正文色 + 修饰符。

/// 工具结构色：翡翠绿 `Indexed 42`（`#00d787`）。
const CHAT_STEP_GREEN: u8 = 42;
/// 思考结构色：品红 `Indexed 218`（`#ffafd7`）——展开态与悬停态使用。
const CHAT_STEP_MAGENTA: u8 = 218;
/// 思考折叠态色：浅粉 `Indexed 182`（`#d7afd7`）。
const CHAT_STEP_PINK_SOFT: u8 = 182;
/// 工具运行中色：橙黄 `Indexed 208`（`#ff8700`）。
const CHAT_STEP_AMBER: u8 = 208;

/// 正文块的方块：运行中（呼吸）——终端默认前景 + 加粗。
pub fn chat_step_mark_running_body() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// 正文块的方块：已完成——终端默认前景、不加粗。
pub fn chat_step_mark_done_body() -> Style {
    Style::default()
}

/// 正文块的头部文案：运行中——默认前景、不加粗。
pub fn chat_step_label_running_body() -> Style {
    Style::default()
}

/// 正文块的头部文案：已完成——默认前景、不加粗。
pub fn chat_step_label_done_body() -> Style {
    Style::default()
}

/// 思考块的方块：运行中（呼吸）——折叠态浅粉、展开亮粉加粗。
pub fn chat_step_mark_running_thinking(expanded: bool) -> Style {
    if expanded {
        Style::default()
            .fg(Color::Indexed(CHAT_STEP_MAGENTA))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Indexed(CHAT_STEP_PINK_SOFT))
            .add_modifier(Modifier::BOLD)
    }
}

/// 思考块的方块：已完成——折叠态浅粉、展开亮粉（不加粗）。
pub fn chat_step_mark_done_thinking(expanded: bool) -> Style {
    if expanded {
        Style::default().fg(Color::Indexed(CHAT_STEP_MAGENTA))
    } else {
        Style::default().fg(Color::Indexed(CHAT_STEP_PINK_SOFT))
    }
}

/// 思考块的头部文案：运行中——折叠态浅粉、展开亮粉（不加粗）。
pub fn chat_step_label_running_thinking(expanded: bool) -> Style {
    if expanded {
        Style::default().fg(Color::Indexed(CHAT_STEP_MAGENTA))
    } else {
        Style::default().fg(Color::Indexed(CHAT_STEP_PINK_SOFT))
    }
}

/// 思考块的头部文案：已完成——折叠态浅粉、展开亮粉（不加粗）。
pub fn chat_step_label_done_thinking(expanded: bool) -> Style {
    if expanded {
        Style::default().fg(Color::Indexed(CHAT_STEP_MAGENTA))
    } else {
        Style::default().fg(Color::Indexed(CHAT_STEP_PINK_SOFT))
    }
}

/// 工具行的方块：运行中（呼吸）——翡翠绿加粗。
pub fn chat_step_mark_running() -> Style {
    Style::default()
        .fg(Color::Indexed(CHAT_STEP_GREEN))
        .add_modifier(Modifier::BOLD)
}

/// 工具行的方块：已完成——同款翡翠绿、不加粗。
pub fn chat_step_mark_done() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_GREEN))
}

/// 工具运行中的方块：橙黄加粗。
pub fn chat_step_mark_alive() -> Style {
    Style::default()
        .fg(Color::Indexed(CHAT_STEP_AMBER))
        .add_modifier(Modifier::BOLD)
}

/// 工具运行中的动词与状态文字：同档橙黄、不加粗。
pub fn chat_step_label_alive() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_AMBER))
}

/// 工具行的头部文案：运行中——橙黄（不加粗）。
pub fn chat_step_label_running() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_AMBER))
}

/// 工具行的头部文案：已完成——翡翠绿（不加粗）。
pub fn chat_step_label_done() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_GREEN))
}

/// 首页 `AGENTS.md` 资源计数：品牌冰蓝（与 `mcp` 翡翠绿、`skills` 浅黄区分）。
pub fn chat_agents_md() -> Style {
    Style::default().fg(LX_SKY_BLUE)
}

/// 工具行的固定动词（`Read` / `Search` / `Edit` / `Write`）：已完成——翡翠绿、不加粗。
pub fn chat_tool_verb() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_GREEN))
}

/// 工具行的固定动词：运行中——与方块同档橙黄。
pub fn chat_tool_verb_running() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_AMBER))
}

/// 工具行的可变参数（路径 / 搜索模式 / 命令 / 搜索范围）：浅黄、常驻。
pub fn chat_tool_param() -> Style {
    Style::default().fg(Color::Indexed(CHAT_TOOL_PARAM_YELLOW))
}

/// 工具参数浅黄：比运行态琥珀 `208` 淡一档，常驻、不压暗。
const CHAT_TOOL_PARAM_YELLOW: u8 = 222;

/// 工具展开块的整行背景（参考 Claude Code 的深色卡片底）。
pub fn chat_expanded_bg() -> Style {
    Style::default().bg(Color::Indexed(CHAT_EXPANDED_BG))
}

/// 展开块的整行底色。
const CHAT_EXPANDED_BG: u8 = 236;

/// 工具行的只读元信息（`read` 的行范围）：退回 `muted()`。
pub fn chat_tool_range() -> Style {
    muted()
}

/// lx 输入框上方 loading 行文案：次要信息。
pub fn chat_loading_label() -> Style {
    muted()
}

/// loading 行右端「回到底部」按钮：常态次要信息、悬停亮白。
pub fn chat_jump_bottom(hovered: bool) -> Style {
    if hovered {
        Style::default().fg(Color::Indexed(252))
    } else {
        muted()
    }
}

/// 补充块命令名：固定品红加粗（不随父 `&&&` 状态变化）。
pub fn supple_command() -> Style {
    Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::BOLD)
}

/// 补充块边框：固定品红（覆盖父块状态色）。
pub fn supple_border() -> Style {
    Style::default().fg(Color::LightMagenta)
}

/// 块操作按钮（补充块）：常态次要信息。
pub fn block_button() -> Style {
    muted()
}

/// lx 页品牌主粉：logo `p` 像素与 Native 入口渐变起点（`Indexed 218`）。
pub const LX_PINK: Color = Color::Indexed(218);

/// lx 页品牌天空蓝：logo `b` 像素、Native 入口渐变终点与首页 `AGENTS.md` 计数（`Indexed 117`）。
pub const LX_SKY_BLUE: Color = Color::Indexed(117);

/// lx 页品牌 logo 像素色：按像素字符映射 256 色（仅 lx 页使用，不影响其余区域的 ANSI 槽位规则）。
///
/// `k` 描边藏青、`p` 主体粉、`n` 深粉、`b` 天空蓝、`c` 冰蓝高光、`w` 高光白；其余字符视为透明。
pub fn logo_pixel(ch: char) -> Option<Color> {
    match ch {
        'k' => Some(Color::Indexed(17)),
        'p' => Some(LX_PINK),
        'n' => Some(Color::Indexed(168)),
        'b' => Some(LX_SKY_BLUE),
        'c' => Some(Color::Indexed(195)),
        'w' => Some(Color::Indexed(231)),
        _ => None,
    }
}

/// lx 页入口标签的渐变端点色（粉 → 蓝），取该入口 logo 的两主色（字标 L 侧粉、X 侧蓝）。
pub fn lx_entry_gradient(entry: LxEntry) -> (Color, Color) {
    match entry {
        LxEntry::Native => (LX_PINK, LX_SKY_BLUE),
        LxEntry::Hybrid => (Color::Indexed(168), Color::Indexed(195)),
        LxEntry::Automation => (Color::Indexed(195), Color::Indexed(168)),
        LxEntry::AiWeb => (LX_SKY_BLUE, LX_PINK),
    }
}

/// lx 页入口标签第 `position` 个字符（共 `length` 个）的样式：粉→蓝渐变取色，选中加粗。
pub fn lx_entry_label_style(
    entry: LxEntry,
    position: usize,
    length: usize,
    selected: bool,
) -> Style {
    let (from, to) = lx_entry_gradient(entry);
    let t = if length <= 1 {
        0.0
    } else {
        position as f32 / (length - 1) as f32
    };
    let base = Style::default().fg(lerp_indexed(from, to, t));
    if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

/// AI Web 站点标签的品牌渐变端点色（256 色 cube 段）。
///
/// - `Deepseek`：深海钴蓝（`Indexed 63`, `#5f5fff`）→ 电光青蓝（`Indexed 39`, `#00afff`）
/// - `Gemini`  ：星光蓝（`Indexed 69`, `#5f87ff`）→ 紫罗兰 → 珊瑚粉（`Indexed 204`, `#ff5f87`）
pub fn ai_web_site_gradient(site: AiWebSite) -> (Color, Color) {
    match site {
        AiWebSite::Deepseek => (Color::Indexed(63), Color::Indexed(39)),
        AiWebSite::Gemini => (Color::Indexed(69), Color::Indexed(204)),
    }
}

/// AI Web 站点标签第 `position` 个字符（共 `length` 个）的样式：品牌渐变取色，选中加粗。
pub fn ai_web_site_label_style(
    site: AiWebSite,
    position: usize,
    length: usize,
    selected: bool,
) -> Style {
    let (from, to) = ai_web_site_gradient(site);
    let t = if length <= 1 {
        0.0
    } else {
        position as f32 / (length - 1) as f32
    };
    let base = Style::default().fg(lerp_indexed(from, to, t));
    if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

/// 256 色 Indexed cube 段（16..=231）转 RGB；其它槽位返回 None。
fn cube_rgb(index: u8) -> Option<(u8, u8, u8)> {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let value = index.checked_sub(16)?;
    (value <= 215).then(|| {
        (
            LEVELS[usize::from(value / 36)],
            LEVELS[usize::from((value % 36) / 6)],
            LEVELS[usize::from(value % 6)],
        )
    })
}

/// RGB 归入最近的 256 色 cube 段（逐通道取最近色阶，平手取较暗者）。
fn cube_index((r, g, b): (u8, u8, u8)) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |value: u8| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, level)| level.abs_diff(value))
            .map_or(0, |(index, _)| index as u8)
    };
    16 + nearest(r) * 36 + nearest(g) * 6 + nearest(b)
}

/// 两个 256 色按 `t`∈[0,1] 线性插值，结果归入最近的 256 色。
fn lerp_indexed(from: Color, to: Color, t: f32) -> Color {
    let (Color::Indexed(a), Color::Indexed(b)) = (from, to) else {
        return if t < 0.5 { from } else { to };
    };
    let (Some(rgb_a), Some(rgb_b)) = (cube_rgb(a), cube_rgb(b)) else {
        return if t < 0.5 { from } else { to };
    };
    let mix = |x: u8, y: u8| {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::Indexed(cube_index((
        mix(rgb_a.0, rgb_b.0),
        mix(rgb_a.1, rgb_b.1),
        mix(rgb_a.2, rgb_b.2),
    )))
}

/// 会话标题胶囊配色槽位总数。
pub const SESSION_TITLE_PALETTE_COUNT: u8 = 8;

/// 会话标题胶囊柔和暗色配色池（结合 Claude Code 暗色胶囊风格与 LX 品牌 Logo 色调）。
pub const SESSION_TITLE_PALETTE: [(Color, Color); 8] = [
    // 0: LX 品牌柔樱粉：暗暮粉紫底 (#2e1e29) + 柔樱粉字 (#e8a6c4)
    (Color::Rgb(0x2e, 0x1e, 0x29), Color::Rgb(0xe8, 0xa6, 0xc4)),
    // 1: LX 品牌柔天蓝：暗海蓝底 (#1b2a38) + 柔天蓝字 (#8fc7e8)
    (Color::Rgb(0x1b, 0x2a, 0x38), Color::Rgb(0x8f, 0xc7, 0xe8)),
    // 2: LX 品牌柔冰青：暗松石青底 (#1a2e30) + 柔冰青字 (#88d4d6)
    (Color::Rgb(0x1a, 0x2e, 0x30), Color::Rgb(0x88, 0xd4, 0xd6)),
    // 3: LX 品牌柔薰衣草紫：暗夜紫底 (#272138) + 柔薰衣草紫字 (#baaee8)
    (Color::Rgb(0x27, 0x21, 0x38), Color::Rgb(0xba, 0xae, 0xe8)),
    // 4: Claude 柔薄荷绿：暗苔绿底 (#1d2f24) + 柔薄荷绿字 (#8fcfa2)
    (Color::Rgb(0x1d, 0x2f, 0x24), Color::Rgb(0x8f, 0xcf, 0xa2)),
    // 5: Claude 柔暖琥珀：暗琥珀棕底 (#32261a) + 柔暖金字 (#e2b882)
    (Color::Rgb(0x32, 0x26, 0x1a), Color::Rgb(0xe2, 0xb8, 0x82)),
    // 6: Claude 柔珊瑚粉：暗绯红底 (#342022) + 柔珊瑚粉字 (#e4a29e)
    (Color::Rgb(0x34, 0x20, 0x22), Color::Rgb(0xe4, 0xa2, 0x9e)),
    // 7: Claude 柔雾蓝紫：暗靛蓝底 (#202538) + 柔雾蓝字 (#9eb4e8)
    (Color::Rgb(0x20, 0x25, 0x38), Color::Rgb(0x9e, 0xb4, 0xe8)),
];

/// 按会话配色槽位生成标题胶囊样式：使用柔和暗底 + 柔和亮字。
pub fn session_title_badge(palette_index: u8) -> Style {
    let (bg, fg) = SESSION_TITLE_PALETTE[usize::from(palette_index) % SESSION_TITLE_PALETTE.len()];
    Style::default()
        .fg(fg)
        .bg(bg)
        .remove_modifier(Modifier::BOLD | Modifier::DIM | Modifier::REVERSED | Modifier::HIDDEN)
}
