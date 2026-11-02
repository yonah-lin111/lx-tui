use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;
use crate::ui::main_content::lx::history::PaneCache;
use crate::ui::main_content::lx::{layout, render};

/// 256 色 Cube 索引（16..=231）转换为 RGB。
fn indexed_rgb(color: Color) -> (u8, u8, u8) {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let Color::Indexed(idx) = color else {
        panic!("expected 256-color Indexed slot, got {color:?}");
    };
    let val = idx
        .checked_sub(16)
        .expect("palette slot must be in 6x6x6 cube (16..=231)");
    assert!(val <= 215, "palette slot must be in 6x6x6 cube (16..=231)");
    (
        LEVELS[usize::from(val / 36)],
        LEVELS[usize::from((val % 36) / 6)],
        LEVELS[usize::from(val % 6)],
    )
}

/// 计算 WCAG 2.1 相对亮度对比度。
fn contrast_ratio(a: Color, b: Color) -> f64 {
    fn channel(c: u8) -> f64 {
        let s = f64::from(c) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }
    let l1 = luminance(indexed_rgb(a));
    let l2 = luminance(indexed_rgb(b));
    let (bright, dark) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (bright + 0.05) / (dark + 0.05)
}

#[test]
fn session_title_palette_guarantees_high_contrast_and_never_dims_or_reverses() {
    for (idx, &(bg, fg)) in SESSION_TITLE_PALETTE.iter().enumerate() {
        assert_ne!(bg, fg, "slot {idx}: bg and fg must differ");
        let ratio = contrast_ratio(bg, fg);
        assert!(
            ratio >= 7.0,
            "slot {idx}: WCAG AAA contrast >= 7.0 required, got {ratio:.2} for bg={bg:?}, fg={fg:?}"
        );
        let style = session_title_badge_style(idx as u8);
        assert_eq!(style.bg, Some(bg));
        assert_eq!(style.fg, Some(fg));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.sub_modifier.contains(Modifier::DIM));
        assert!(style.sub_modifier.contains(Modifier::REVERSED));
        assert!(style.sub_modifier.contains(Modifier::HIDDEN));
    }
}

#[test]
fn session_title_badge_renders_inside_top_right_border_with_bg_and_fg() {
    let area = Rect::new(0, 0, 60, 16);
    let mut lx = Lx::new();
    lx.chat.session_id = Some("sess-1".into());
    lx.chat.set_session_title(Some("修复标题生成问题".into()));
    lx.chat.session_title_palette = 0;

    let input = layout(area, &lx).input.expect("input rect");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());

    let top_y = input.y;
    let mut top_line = String::new();
    let mut x = input.x;
    while x < input.right() {
        let sym = buf[(x, top_y)].symbol();
        top_line.push_str(sym);
        let w = sym
            .chars()
            .next()
            .and_then(|ch| ch.width())
            .unwrap_or(1)
            .max(1) as u16;
        x = x.saturating_add(w);
    }
    assert!(
        top_line.contains(" 修复标题生成问题 "),
        "顶边框内应包含带左右空格内边距的标题胶囊：{top_line}"
    );
    // 右上角最末端 1 列仍保留 `─` 边框线（确保胶囊在边框内部）
    assert_eq!(
        buf[(input.right() - 1, top_y)].symbol(),
        "─",
        "右上角最右 1 列应保留边框线 `─`"
    );
    assert_eq!(
        buf[(input.x, top_y)].symbol(),
        "─",
        "左上角应保留边框线 `─`"
    );

    // 验证胶囊区域的单元格均携带预期的高对比背景色与前景色，且不含 DIM
    let (expected_bg, expected_fg) = SESSION_TITLE_PALETTE[0];
    let badge_w = display_width(" 修复标题生成问题 ") as u16;
    let badge_start_x = input.right() - 1 - badge_w;
    for x in badge_start_x..(input.right() - 1) {
        let cell = &buf[(x, top_y)];
        assert_eq!(
            cell.bg, expected_bg,
            "列 {x} 应带胶囊背景色 {expected_bg:?}"
        );
        assert_eq!(
            cell.fg, expected_fg,
            "列 {x} 应带胶囊字体色 {expected_fg:?}"
        );
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "列 {x} 标题文字应加粗"
        );
        assert!(
            !cell.modifier.contains(Modifier::DIM),
            "列 {x} 严禁继承分割线的 DIM 修饰符"
        );
    }
}

#[test]
fn session_title_badge_is_hidden_when_none_and_truncates_when_narrow() {
    let area = Rect::new(0, 0, 30, 12);
    let mut lx = Lx::new();
    let input = layout(area, &lx).input.expect("input rect");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, false, None, &mut PaneCache::default());

    // 未设置标题时整条顶边框全是 `─`
    for x in input.x..input.right() {
        assert_eq!(buf[(x, input.y)].symbol(), "─");
    }

    // 超长标题在窄输入框下自动截断并保留两侧 `─`
    lx.chat.set_session_title(Some(
        "This is an extremely long session title that exceeds narrow input width".into(),
    ));
    let mut buf2 = Buffer::empty(area);
    let _ = render(area, &mut buf2, &lx, false, None, &mut PaneCache::default());
    assert_eq!(buf2[(input.x, input.y)].symbol(), "─");
    assert_eq!(buf2[(input.right() - 1, input.y)].symbol(), "─");
}
