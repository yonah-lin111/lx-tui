//! 语义样式 Token：沿用 codex 的终端原生 ANSI 配色，无自绘背景。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_stay_palette_native() {
        for style in [text(), muted(), accent(), border(true), border(false)] {
            assert!(matches!(style.fg, None | Some(Color::Cyan)));
        }
    }
}
