use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;
use crate::ui::main_content::lx::history::PaneCache;
use crate::ui::main_content::lx::{layout, render};

fn color_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("expected Color::Rgb, got {other:?}"),
    }
}

fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    fn channel(c: u8) -> f64 {
        let s = f64::from(c) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// 计算 WCAG 2.1 相对亮度对比度。
fn contrast_ratio(a: Color, b: Color) -> f64 {
    let l1 = relative_luminance(color_rgb(a));
    let l2 = relative_luminance(color_rgb(b));
    let (bright, dark) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (bright + 0.05) / (dark + 0.05)
}

#[test]
fn session_title_palette_uses_soft_dark_bg_and_clear_pastel_fg() {
    for (idx, &(bg, fg)) in SESSION_TITLE_PALETTE.iter().enumerate() {
        assert_ne!(bg, fg, "slot {idx}: bg and fg must differ");
        let bg_lum = relative_luminance(color_rgb(bg));
        assert!(
            bg_lum <= 0.04,
            "slot {idx}: background must stay dark and non-glaring (lum <= 0.04), got {bg_lum:.4} for {bg:?}"
        );
        let ratio = contrast_ratio(bg, fg);
        assert!(
            (5.5..=10.0).contains(&ratio),
            "slot {idx}: contrast must be clear yet soft (5.5..=10.0), got {ratio:.2} for bg={bg:?}, fg={fg:?}"
        );
        let style = session_title_badge_style(idx as u8);
        assert_eq!(style.bg, Some(bg));
        assert_eq!(style.fg, Some(fg));
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

    // 验证胶囊区域的单元格均携带预期的柔和暗底与柔亮前景，且不含 DIM
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
