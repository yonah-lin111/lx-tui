use super::*;
use crate::app::paste::{PasteAnchor, TextPasteAction, encode_pasted_text, folded_tokens};

#[test]
fn renders_collapsed_chip_with_yellow_bold_on_indexed_236_and_expanded_only_tints_pasted_text() {
    let area = Rect::new(0, 0, 40, 6);
    let mut p = Prompt::new(PaneId::from_raw_for_test(1));
    p.resize(area.width, area.height);
    let tok = encode_pasted_text("hello **world**\nsecond line\nthird line");
    p.set_text(&format!("prefix {tok}"));

    // 1. Collapsed state: "prefix " has default bg, chip starts at col 7 with `style::pasted_chip()`.
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &p, None);
    assert_eq!(buf[(0, 0)].symbol(), "p");
    assert_eq!(buf[(0, 0)].bg, Color::Reset);
    assert_eq!(buf[(7, 0)].symbol(), "[");
    assert_eq!(buf[(7, 0)].fg, Color::Yellow);
    assert_eq!(buf[(7, 0)].bg, Color::Indexed(236));
    assert!(buf[(7, 0)].modifier.contains(Modifier::BOLD));

    // 2. Expanded state: "prefix " at col 0..7 keeps default bg (no black/gray block on its left!),
    // while "hello **world**" at col 7.. and "second line" on row 1 have `Indexed(236)` bg.
    let id = folded_tokens(p.text())[0].id;
    p.toggle_paste(id, 0);
    let mut buf_exp = Buffer::empty(area);
    render(area, &mut buf_exp, &p, None);

    assert_eq!(buf_exp[(0, 0)].symbol(), "p");
    assert_eq!(
        buf_exp[(0, 0)].bg,
        Color::Reset,
        "Plain prefix text must not have dark pasted background"
    );
    assert_eq!(buf_exp[(6, 0)].symbol(), " ");
    assert_eq!(buf_exp[(6, 0)].bg, Color::Reset);
    assert_eq!(buf_exp[(7, 0)].symbol(), "h");
    assert_eq!(buf_exp[(7, 0)].bg, Color::Indexed(236));
    assert_ne!(
        buf_exp[(15, 0)].fg,
        Color::Yellow,
        "Expanded preview must not apply markdown strong styling"
    );

    // Row 1 starts "second line" at column 0 with `Indexed(236)` bg.
    assert_eq!(buf_exp[(0, 1)].symbol(), "s");
    assert_eq!(buf_exp[(0, 1)].bg, Color::Indexed(236));
}

#[test]
fn renders_text_paste_panel_and_hits_items() {
    let area = Rect::new(0, 0, 50, 10);
    let mut p = Prompt::new(PaneId::from_raw_for_test(1));
    p.resize(area.width, area.height);
    assert!(p.begin_text_paste(
        "1\n2\n3".into(),
        "1\n2\n3".into(),
        PasteAnchor::at_cursor(0),
    ));

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &p, None);

    let layout = paste_layout(&p, area).expect("text paste layout");
    let item0_y = layout.content.y;
    let item1_y = layout.content.y + 1;
    assert_eq!(
        paste_item_at(&p, area, layout.content.x + 2, item0_y),
        Some(0)
    );
    assert_eq!(
        paste_item_at(&p, area, layout.content.x + 2, item1_y),
        Some(1)
    );
    assert!(p.text_paste_confirm(TextPasteAction::Collapse));
}
