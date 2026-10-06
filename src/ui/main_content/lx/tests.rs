//! lx 页单元测试：艺术数据一致性、相位合成、调色板与降级。

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
    assert_eq!(width as u16, art_width(art));
}

/// 像素图宽度。
fn art_width(art: &Art) -> u16 {
    u16::try_from(art.base[0].chars().count()).expect("art width fits")
}

#[test]
fn art_layers_are_consistent() {
    assert_art("FOX", &FOX);
    assert_art("FOX_HEAD", &FOX_HEAD);
    assert_eq!(art_width(&FOX), FOX_WIDTH);
    assert_eq!(art_width(&FOX_HEAD), HEAD_WIDTH);
    assert_eq!(FOX.base.len(), usize::from(FOX_ROWS) * 2);
    assert_eq!(FOX_HEAD.base.len(), usize::from(HEAD_ROWS) * 2);
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
fn compact_head_blinks_too() {
    let open = compose(&FOX_HEAD, 0);
    let blink = compose(&FOX_HEAD, 6);
    assert_ne!(open[3], blink[3]);
    assert_eq!(blink[3][3], 'k');
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
}

#[test]
fn plan_degrades_with_available_space() {
    let hint = text::lx_hint().chars().count() as u16;
    assert_eq!(
        plan(Rect::new(0, 0, 60, 12), hint),
        Page::Fox { hint: true }
    );
    assert_eq!(
        plan(Rect::new(0, 0, 60, 10), hint),
        Page::Fox { hint: false }
    );
    assert_eq!(
        plan(Rect::new(0, 0, 19, 10), hint),
        Page::Head { wordmark: true }
    );
    assert_eq!(
        plan(Rect::new(0, 0, 14, 5), hint),
        Page::Head { wordmark: false }
    );
    assert_eq!(plan(Rect::new(0, 0, 8, 3), hint), Page::Wordmark);
}

#[test]
fn render_draws_wordmark_and_hint_when_roomy() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    render(Rect::new(0, 0, 60, 12), &mut buf, 0);
    let text = buffer_text(&buf);
    assert!(text.contains("lx"));
    assert!(text.contains("click [>_] to open terminal"));
    assert!(text.contains('▀') || text.contains('█'));
}

#[test]
fn render_animates_between_phases() {
    let area = Rect::new(0, 0, 60, 12);
    let mut open = Buffer::empty(area);
    let mut blink = Buffer::empty(area);
    render(area, &mut open, 0);
    render(area, &mut blink, 6);
    assert_ne!(buffer_text(&open), buffer_text(&blink));
}

#[test]
fn render_falls_back_to_wordmark_when_tiny() {
    let area = Rect::new(0, 0, 8, 3);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, 0);
    assert!(buffer_text(&buf).contains("lx"));
}

#[test]
fn render_compact_head_without_hint() {
    let area = Rect::new(0, 0, 14, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, 0);
    let text = buffer_text(&buf);
    assert!(text.contains("lx"));
    assert!(!text.contains("click"));
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
