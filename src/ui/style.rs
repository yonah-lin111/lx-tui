//! 语义样式 Token：沿用 codex 的终端原生 ANSI 配色。
//! 默认不铺自绘背景；仅侧栏/标签栏的选中行使用 ANSI-16 背景（`selected_item`），
//! 悬停行沿用终端原生反显（`selection`）；浮层面板使用自包含深底浅字（`overlay_panel`）。
//! 组件禁止直接写死颜色与修饰符。

use ratatui::style::{Color, Modifier, Style};

/// 正文。
pub fn text() -> Style {
    Style::default()
}

/// 次要信息：使用终端的 dim 修饰，自动适配明暗终端。
pub fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// 强调/激活：codex 在明暗未知终端上的 accent（Cyan 加粗）。
pub fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 分区边框；焦点态使用 accent。
pub fn border(focused: bool) -> Style {
    if focused { accent() } else { muted() }
}

/// 选中项：主题强调色填充（Cyan 底、黑字、加粗）；侧栏激活项与激活标签共用。
pub fn selected_item() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 列表/菜单当前项：终端原生反显；侧栏/标签栏的鼠标悬停使用；浮层内改用 `overlay_selection`。
pub fn selection() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// 浮层内选中/悬停项：先重置为终端默认前景/背景再反显，不受浮层底色影响。
///
/// 与 `selection` 的区别仅在于显式重置面板底色，效果与侧栏/标签栏的悬停一致。
pub fn overlay_selection() -> Style {
    Style::default()
        .fg(Color::Reset)
        .bg(Color::Reset)
        .add_modifier(Modifier::REVERSED)
}

/// 状态：失败（Red）。
pub fn error() -> Style {
    Style::default().fg(Color::Red)
}

/// 浮层标题：正文加粗。
pub fn strong() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// 浮层面板：自包含深底浅字（256 色），保证明暗终端下的对比度。
///
/// 用于 toast、右键菜单、命令面板与模态容器的整块着色：底色 Indexed 236、
/// 文字前景 Indexed 252；边框与强调/错误色在各自绘制时覆盖前景。
pub fn overlay_panel() -> Style {
    Style::default()
        .fg(Color::Indexed(252))
        .bg(Color::Indexed(236))
}

/// markdown 语法标记（`#`、`**`、`` ` `` 等）：次要信息。
pub fn markdown_marker() -> Style {
    muted()
}

/// markdown 标题文字。
pub fn markdown_heading() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 粗体文字。
pub fn markdown_strong() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 斜体文字。
pub fn markdown_emphasis() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::ITALIC)
}

/// markdown 删除线文字。
pub fn markdown_strikethrough() -> Style {
    Style::default()
        .fg(Color::LightRed)
        .add_modifier(Modifier::CROSSED_OUT)
}

/// markdown 行内代码内容。
pub fn markdown_inline_code() -> Style {
    Style::default().fg(Color::LightRed)
}

/// markdown 围栏代码块内容：不引入语法着色，保持正文色。
pub fn markdown_code_block() -> Style {
    text()
}

/// markdown 引用内容。
pub fn markdown_quote() -> Style {
    Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::ITALIC)
}

/// markdown 链接文字。
pub fn markdown_link_text() -> Style {
    Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::UNDERLINED)
}

/// markdown URL。
pub fn markdown_url() -> Style {
    Style::default().fg(Color::Cyan)
}

/// lx 页吉祥物像素色：按像素字符映射 256 色（仅 lx 页使用，不影响其余区域的 ANSI 槽位规则）。
///
/// `k` 描边藏青、`p` 主体粉、`n` 深粉、`b` 天空蓝、`w` 高光白；其余字符视为透明。
pub fn mascot_pixel(ch: char) -> Option<Color> {
    match ch {
        'k' => Some(Color::Indexed(17)),
        'p' => Some(Color::Indexed(218)),
        'n' => Some(Color::Indexed(168)),
        'b' => Some(Color::Indexed(117)),
        'w' => Some(Color::Indexed(231)),
        _ => None,
    }
}

/// lx 页占位元素（白）：未来内容面板的边框与文案。
pub fn lx_placeholder() -> Style {
    Style::default().fg(Color::Indexed(231))
}

#[cfg(test)]
mod tests;
