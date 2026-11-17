use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;
use crate::ui::main_content::lx::history::PaneCache;
use crate::ui::main_content::lx::{layout, render};
use crate::ui::style::{SESSION_TITLE_PALETTE, session_title_badge};

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
        let style = session_title_badge(idx as u8);
        assert_eq!(style.bg, Some(bg));
        assert_eq!(style.fg, Some(fg));
        assert!(style.sub_modifier.contains(Modifier::DIM));
        assert!(style.sub_modifier.contains(Modifier::REVERSED));
        assert!(style.sub_modifier.contains(Modifier::HIDDEN));
    }
}

#[test]
fn session_title_badge_with_zero_width_variation_selector_preserves_right_border() {
    let area = Rect::new(0, 0, 50, 14);
    let mut lx = Lx::new();
    lx.chat.session_id = Some("sess-zw".into());
    lx.chat.set_session_title(Some("Fix ❤️ bug".into()));

    let input = layout(area, &lx).input.expect("input rect");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());

    assert_eq!(
        buf[(input.right() - 1, input.y)].symbol(),
        "─",
        "含 VS16 (U+FE0F) 零宽字符时右上角最右 1 列仍须保留 `─`"
    );
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

#[test]
fn lx_input_renders_folded_chip_and_expanded_preview_with_selection() {
    let area = Rect::new(0, 0, 50, 14);
    let mut lx = Lx::new();
    let tok = crate::app::paste::encode_pasted_text("alpha\nbeta\ngamma");
    lx.insert_str(&format!("ask {tok}"));

    let input = layout(area, &lx).input.expect("input rect");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());
    let text_y = input.y + 1;
    let content_x = input.x + INPUT_PREFIX;

    // Plain prefix "ask " has default bg, collapsed chip starts at content_x + 4 with `[` in Yellow BOLD on Indexed(236).
    assert_eq!(buf[(content_x, text_y)].symbol(), "a");
    assert_eq!(buf[(content_x, text_y)].bg, Color::Reset);
    assert_eq!(buf[(content_x + 4, text_y)].symbol(), "[");
    assert_eq!(buf[(content_x + 4, text_y)].fg, Color::Yellow);
    assert_eq!(buf[(content_x + 4, text_y)].bg, Color::Indexed(236));
    assert!(
        buf[(content_x + 4, text_y)]
            .modifier
            .contains(Modifier::BOLD)
    );

    // Expand chip and select "beta" on line 1 (display offset 10..14 in "ask alpha\nbeta\ngamma").
    let id = crate::app::paste::folded_tokens(&lx.input)[0].id;
    lx.toggle_paste(id, 0);
    lx.set_preview_selection(Some((10, 14)));

    let input_exp = layout(area, &lx).input.expect("input rect");
    let mut buf_exp = Buffer::empty(area);
    let _ = render(
        area,
        &mut buf_exp,
        &lx,
        true,
        None,
        &mut PaneCache::default(),
    );
    let exp_y0 = input_exp.y + 1;
    let exp_y1 = input_exp.y + 2;

    // Row 0 keeps "ask " with Reset bg, then "alpha" with Indexed(236) bg.
    assert_eq!(buf_exp[(content_x, exp_y0)].symbol(), "a");
    assert_eq!(buf_exp[(content_x, exp_y0)].bg, Color::Reset);
    assert_eq!(buf_exp[(content_x + 4, exp_y0)].symbol(), "a");
    assert_eq!(buf_exp[(content_x + 4, exp_y0)].bg, Color::Indexed(236));

    // Row 1 ("beta") starts at content_x with Indexed(236) bg and REVERSED modifier from preview_selection.
    assert_eq!(buf_exp[(content_x, exp_y1)].symbol(), "b");
    assert_eq!(buf_exp[(content_x, exp_y1)].bg, Color::Indexed(236));
    assert!(
        buf_exp[(content_x, exp_y1)]
            .modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn lx_input_top_and_bottom_rules_use_explicit_gray_while_preserving_rule_badges() {
    let area = Rect::new(0, 0, 60, 14);
    let mut lx = Lx::new();
    lx.chat.session_id = Some("sess-rule".into());
    lx.chat.set_session_title(Some("Title".into()));
    lx.chat
        .queue
        .push(crate::app::state::lx::chat::QueuedSubmission::Chat {
            draft: "next".into(),
        });

    let input = layout(area, &lx).input.expect("input rect");
    let mut buf = Buffer::empty(area);
    let _ = render(area, &mut buf, &lx, true, None, &mut PaneCache::default());

    let expected_fg = crate::ui::style::lx_input_rule().fg.expect("explicit gray");
    let top_y = input.y;
    let bottom_y = input.bottom() - 1;

    // 顶/底分割线除右侧胶囊/`1 queued` 之外的每一列均为显式灰、透明底且无 DIM。
    let badge_w = display_width(" Title ") as u16;
    let badge_x = input.right() - 1 - badge_w;
    for x in input.x..input.right() {
        if x >= badge_x && x < badge_x + badge_w {
            continue;
        }
        let cell = &buf[(x, top_y)];
        assert_eq!(cell.symbol(), "─", "top rule at ({x},{top_y})");
        assert_eq!(cell.fg, expected_fg, "top rule fg at ({x},{top_y})");
        assert_eq!(cell.bg, Color::Reset, "top rule bg at ({x},{top_y})");
        assert!(
            !cell.modifier.contains(Modifier::DIM),
            "top rule no DIM at ({x},{top_y})"
        );
    }

    let queued_w = display_width("1 queued") as u16;
    let queued_x = input.right() - 1 - queued_w;
    for x in input.x..input.right() {
        if x >= queued_x && x < queued_x + queued_w {
            continue;
        }
        let cell = &buf[(x, bottom_y)];
        assert_eq!(cell.symbol(), "─", "bottom rule at ({x},{bottom_y})");
        assert_eq!(cell.fg, expected_fg, "bottom rule fg at ({x},{bottom_y})");
        assert_eq!(cell.bg, Color::Reset, "bottom rule bg at ({x},{bottom_y})");
        assert!(
            !cell.modifier.contains(Modifier::DIM),
            "bottom rule no DIM at ({x},{bottom_y})"
        );
    }

    // 顶分割线胶囊与底分割线 `1 queued` 保持各自原有样式。
    assert_ne!(buf[(badge_x, top_y)].fg, expected_fg);
    assert_eq!(buf[(queued_x, bottom_y)].symbol(), "1");
    assert!(buf[(queued_x, bottom_y)].modifier.contains(Modifier::DIM));

    // Shell mode 与排队同时存在：宽屏显示完整 `Shell mode · 1 queued`，窄屏降级保 `Shell mode`。
    lx.replace_input("!ls");
    let mut shell_buf = Buffer::empty(area);
    let _ = render(
        area,
        &mut shell_buf,
        &lx,
        true,
        None,
        &mut PaneCache::default(),
    );
    let shell_bottom: String = (input.x..input.right())
        .map(|x| shell_buf[(x, bottom_y)].symbol())
        .collect();
    assert!(
        shell_bottom.contains("Shell mode · 1 queued"),
        "shell+queue={shell_bottom:?}"
    );
    assert_eq!(
        shell_buf[(input.x, bottom_y)].fg,
        expected_fg,
        "shell 模式左端分割线仍保持显式灰"
    );

    let narrow = Rect::new(0, 0, 20, 14);
    let narrow_input = layout(narrow, &lx).input.expect("narrow input");
    let mut narrow_buf = Buffer::empty(narrow);
    let _ = render(
        narrow,
        &mut narrow_buf,
        &lx,
        true,
        None,
        &mut PaneCache::default(),
    );
    let narrow_bottom: String = (narrow_input.x..narrow_input.right())
        .map(|x| narrow_buf[(x, narrow_input.bottom() - 1)].symbol())
        .collect();
    assert!(
        narrow_bottom.contains("Shell mode") && !narrow_bottom.contains("queued"),
        "窄宽下优先保留 Shell mode 指示: {narrow_bottom:?}"
    );
}

#[test]
fn input_draw_rejects_degenerate_rects_without_panicking_or_overflowing() {
    let lx = Lx::new();
    for rect in [
        Rect::new(0, 0, 0, 0),
        Rect::new(0, 0, 1, 1),
        Rect::new(2, 2, 8, 2),
        Rect::new(2, 2, 1, 3),
    ] {
        let grid_area = Rect::new(0, 0, 20, 10);
        let mut grid = Grid::new(grid_area);
        let cursor = draw(&mut grid, rect, &lx, true, 0, 5);
        assert!(cursor.is_none(), "退化矩形 {rect:?} 不得返回光标");
        if rect.height < 3 || rect.width == 0 {
            for y in grid_area.y..grid_area.bottom() {
                for x in grid_area.x..grid_area.right() {
                    assert_eq!(
                        grid.cell(x, y).ch,
                        ' ',
                        "height < 3 或 width == 0 不得写入网格 at ({x},{y}) for {rect:?}"
                    );
                }
            }
        } else {
            // width == 1, height == 3：不越过 rect.right() 写入外边距列。
            assert_eq!(grid.cell(rect.right(), rect.y + 1).ch, ' ');
        }
    }

    // visible 超过 rect.height - 2 时必须钳制，绝不覆盖底分割线或下沿外侧行。
    let mut multi = Lx::new();
    multi.resize(20, 10);
    multi.insert_str("line1\nline2\nline3\nline4");
    let mut grid = Grid::new(Rect::new(0, 0, 30, 8));
    let box_rect = Rect::new(2, 1, 20, 4);
    let _ = draw(&mut grid, box_rect, &multi, true, 0, 10);
    assert_eq!(grid.cell(box_rect.x + 2, box_rect.bottom() - 1).ch, '─');
    assert_eq!(grid.cell(box_rect.x + 2, box_rect.bottom()).ch, ' ');
}
