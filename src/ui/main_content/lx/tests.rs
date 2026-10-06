//! lx 页单元测试：艺术数据一致性、相位合成、调色板与布局降级。

use super::*;
use ratatui::style::Color;

/// 校验像素图：行宽一致、行数为偶（半块渲染按 2 行一组）、只含调色板字符。
fn assert_art(name: &str, art: &Art) {
    let width = art.base[0].chars().count();
    assert_eq!(art.base.len() % 2, 0, "{name} 行数必须为偶");
    let check = |label: &str, rows: &[&str]| {
        assert_eq!(rows.len(), art.base.len(), "{name} {label} 行数一致");
        for row in rows {
            assert_eq!(row.chars().count(), width, "{name} {label} 行宽一致");
            for ch in row.chars() {
                assert!(
                    matches!(ch, '.' | 'k' | 'p' | 'n' | 'b' | 'w'),
                    "{name} {label} 未知像素字符 {ch:?}"
                );
            }
        }
    };
    check("base", art.base);
    if let Some(ears) = art.ears {
        check("ears0", ears[0]);
        check("ears1", ears[1]);
    }
    if let Some(tail) = art.tail {
        check("tail0", tail[0]);
        check("tail1", tail[1]);
    }
    if let Some(blink) = art.blink {
        check("blink", blink);
    }
    assert_eq!(width, usize::from(FOX_WIDTH));
}

#[test]
fn art_layers_are_consistent() {
    assert_art("FOX", &FOX);
    assert_eq!(FOX.base.len(), usize::from(FOX_ROWS) * 2);
}

#[test]
fn tail_alternates_every_two_phases_and_ears_twitch_periodically() {
    let phase0 = compose(&FOX, 0);
    let phase2 = compose(&FOX, 2);
    assert_ne!(phase0[10], phase2[10], "摆尾帧应切换");
    let twitch = compose(&FOX, 10);
    assert_ne!(phase0[0], twitch[0], "抖耳帧应切换");
    assert_eq!(compose(&FOX, 12), compose(&FOX, 0), "24 帧周期回环");
}

#[test]
fn blink_closes_eyes_at_expected_phases() {
    let open = compose(&FOX, 0);
    assert_eq!(open[6][5], 'w');
    let blink = compose(&FOX, 6);
    assert_eq!(blink[6][5], 'k', "眼白行闭合成描边");
    assert_eq!(blink[7][5], 'p', "瞳孔行还原毛色");
    assert_eq!(compose(&FOX, 7), blink, "闭眼持续 2 帧");
    assert_eq!(compose(&FOX, 8), open, "之后恢复睁眼");
}

#[test]
fn mascot_palette_maps_logo_colors() {
    assert_eq!(style::mascot_pixel('k'), Some(Color::Indexed(17)));
    assert_eq!(style::mascot_pixel('p'), Some(Color::Indexed(218)));
    assert_eq!(style::mascot_pixel('n'), Some(Color::Indexed(168)));
    assert_eq!(style::mascot_pixel('b'), Some(Color::Indexed(117)));
    assert_eq!(style::mascot_pixel('w'), Some(Color::Indexed(231)));
    assert_eq!(style::mascot_pixel('.'), None);
    assert_eq!(style::mascot_pixel('x'), None);
    assert_eq!(
        style::lx_placeholder().fg,
        Some(Color::Indexed(231)),
        "占位元素为白"
    );
}

#[test]
fn input_box_sits_at_bottom_with_margins() {
    // 宽屏：左右各 2 列边距。
    assert_eq!(
        input_box(Rect::new(0, 0, 60, 12)),
        Some(Rect::new(2, 9, 56, 3))
    );
    // 窄屏：无边距。
    assert_eq!(
        input_box(Rect::new(0, 0, 20, 5)),
        Some(Rect::new(0, 2, 20, 3))
    );
    // 高度不足或过窄：无输入框。
    assert_eq!(input_box(Rect::new(0, 0, 60, 2)), None);
    assert_eq!(input_box(Rect::new(0, 0, 6, 5)), None);
}

#[test]
fn roomy_page_draws_header_panel_hint_and_input() {
    let area = Rect::new(0, 0, 60, 20);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, 0);
    let text = buffer_text(&buf);
    assert!(text.contains("placeholder"), "白色占位面板可见");
    assert!(text.contains("Ask anything…"), "输入框占位文案可见");
    assert!(text.contains("click [>_] to open terminal"), "切换提示可见");
    assert!(text.contains('▀') || text.contains('█'), "狐狸像素画可见");
    // 输入框贴内容区底部。
    let input = input_box(area).expect("input box");
    assert_eq!(input.bottom(), area.bottom());
}

#[test]
fn header_is_skipped_when_the_fox_does_not_fit() {
    let area = Rect::new(0, 0, 16, 12);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, 0);
    let text = buffer_text(&buf);
    assert!(
        !text.contains('▀') && !text.contains('█'),
        "宽度不足不画残缺狐狸"
    );
    assert!(text.contains("placeholder"), "占位面板仍在");
    assert!(text.contains("> Ask"), "输入框占位仍可见（按宽度截断）");
}

#[test]
fn narrow_page_hides_the_hint() {
    let area = Rect::new(0, 0, 20, 12);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, 0);
    let text = buffer_text(&buf);
    assert!(!text.contains("click"), "提示放不下时省略");
    assert!(text.contains("Ask anything…"));
}

#[test]
fn tiny_page_falls_back_to_wordmark() {
    let area = Rect::new(0, 0, 8, 2);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, 0);
    assert!(buffer_text(&buf).contains("lx"));
}

#[test]
fn render_animates_between_phases() {
    let area = Rect::new(0, 0, 60, 20);
    let mut open = Buffer::empty(area);
    let mut blink = Buffer::empty(area);
    render(area, &mut open, 0);
    render(area, &mut blink, 6);
    assert_ne!(buffer_text(&open), buffer_text(&blink));
}

/// 把缓冲区按行拼成文本，便于断言内容。
fn buffer_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
