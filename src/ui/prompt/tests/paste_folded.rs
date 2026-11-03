use super::*;
use crate::app::paste::{PasteAnchor, TextPasteAction, encode_pasted_text, folded_tokens};

#[test]
fn renders_collapsed_chip_with_yellow_bold_on_indexed_236_and_expanded_with_2_col_indent() {
    let area = Rect::new(0, 0, 40, 6);
    let mut p = Prompt::new(PaneId::from_raw_for_test(1));
    p.resize(area.width, area.height);
    let tok = encode_pasted_text("hello **world**\nsecond line\nthird line");
    p.set_text(&tok);

    // 1. Collapsed state: renders `@[pasted:3](hello **world**)` with `style::pasted_chip()`.
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &p, None);
    assert_eq!(buf[(0, 0)].symbol(), "@");
    assert_eq!(buf[(0, 0)].fg, Color::Yellow);
    assert_eq!(buf[(0, 0)].bg, Color::Indexed(236));
    assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));

    // 2. Expanded state: 2-column indent + plain text on `Indexed(236)` bg (no markdown bold on `**world**`).
    let id = folded_tokens(p.text())[0].id;
    p.toggle_paste(id, 0);
    let mut buf_exp = Buffer::empty(area);
    render(area, &mut buf_exp, &p, None);

    // Columns 0..2 are the 2-column indent with `Indexed(236)` bg.
    assert_eq!(buf_exp[(0, 0)].symbol(), " ");
    assert_eq!(buf_exp[(1, 0)].symbol(), " ");
    assert_eq!(buf_exp[(0, 0)].bg, Color::Indexed(236));
    assert_eq!(buf_exp[(1, 0)].bg, Color::Indexed(236));

    // Column 2 starts "hello **world**" with `Indexed(236)` bg and normal fg (not Yellow bold).
    assert_eq!(buf_exp[(2, 0)].symbol(), "h");
    assert_eq!(buf_exp[(2, 0)].bg, Color::Indexed(236));
    assert_ne!(
        buf_exp[(10, 0)].fg,
        Color::Yellow,
        "Expanded preview must not apply markdown strong styling"
    );

    // Row 1 starts "second line" at column 2 after 2-column indent.
    assert_eq!(buf_exp[(0, 1)].symbol(), " ");
    assert_eq!(buf_exp[(1, 1)].symbol(), " ");
    assert_eq!(buf_exp[(2, 1)].symbol(), "s");
    assert_eq!(buf_exp[(2, 1)].bg, Color::Indexed(236));
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
