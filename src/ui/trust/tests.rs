//! 信任页渲染单元测试：布局、logo、边框、选项高亮与命中、窄屏降级。

use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::*;

const PATH: &str = "/Users/example/workspace";

fn draw(width: u16, height: u16, selected: usize) -> (Buffer, TrustLayout) {
    let screen = Rect::new(0, 0, width, height);
    let layout = layout(screen, &PathBuf::from(PATH));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal
        .draw(|frame| render(frame, &layout, selected))
        .expect("draw succeeds");
    (terminal.backend().buffer().clone(), layout)
}

fn row_text(buf: &Buffer, rect: Rect) -> String {
    (rect.x..rect.right())
        .map(|x| buf[(x, rect.y)].symbol().to_string())
        .collect()
}

#[test]
fn renders_logo_bordered_dialog_and_options() {
    let (buf, layout) = draw(100, 30, 0);
    let logo = layout.logo.expect("logo fits on large screen");
    let logo_row = row_text(&buf, Rect::new(logo.x, logo.y, logo.width, 1));
    assert!(
        logo_row.chars().any(|ch| matches!(ch, '▀' | '▄' | '█')),
        "logo 像素已绘制: {logo_row}"
    );

    assert_eq!(buf[(layout.dialog.x, layout.dialog.y)].symbol(), "╭");
    let title_row = row_text(
        &buf,
        Rect::new(layout.dialog.x, layout.dialog.y, layout.dialog.width, 1),
    );
    assert!(title_row.contains(text::TRUST_CONSOLE_TITLE), "{title_row}");

    let question = row_text(
        &buf,
        Rect::new(
            layout.dialog.x + 1,
            layout.dialog.y + 1,
            layout.dialog.width - 2,
            1,
        ),
    );
    assert!(
        question.contains("Do you trust the files in this folder?"),
        "{question}"
    );

    assert!(row_text(&buf, layout.options[0]).contains(text::TRUST_OPTION_YES));
    assert!(row_text(&buf, layout.options[1]).contains(text::TRUST_OPTION_NO));

    // 路径行：值用 accent 高亮。
    let mut path_highlighted = false;
    for y in layout.dialog.y..layout.dialog.bottom() {
        let row = row_text(&buf, Rect::new(layout.dialog.x, y, layout.dialog.width, 1));
        if let Some(position) = row.find(text::TRUST_CONSOLE_PATH_PREFIX) {
            let x = layout.dialog.x
                + position as u16
                + text::TRUST_CONSOLE_PATH_PREFIX.len() as u16
                + 2;
            assert_eq!(buf[(x, y)].fg, Color::Cyan, "路径值强调色: {row}");
            path_highlighted = true;
            break;
        }
    }
    assert!(path_highlighted, "路径行存在");
}

#[test]
fn selected_option_is_highlighted_with_marker() {
    let (buf, layout) = draw(100, 30, 0);
    assert_eq!(
        buf[(layout.options[0].x, layout.options[0].y)].symbol(),
        "❯"
    );
    assert_eq!(
        buf[(layout.options[0].x, layout.options[0].y)].bg,
        Color::Indexed(240),
        "选中项整行高亮"
    );
    assert_eq!(
        buf[(layout.options[1].x, layout.options[1].y)].symbol(),
        " "
    );

    let (buf, layout) = draw(100, 30, 1);
    assert_eq!(
        buf[(layout.options[1].x, layout.options[1].y)].bg,
        Color::Indexed(240),
        "切换选中"
    );
    assert_eq!(
        buf[(layout.options[1].x, layout.options[1].y)].symbol(),
        "❯"
    );
}

#[test]
fn option_at_hits_option_rows_only() {
    let (_, layout) = draw(100, 30, 0);
    let yes = layout.options[0];
    let no = layout.options[1];
    assert_eq!(option_at(&layout, yes.x + 1, yes.y), Some(0));
    assert_eq!(option_at(&layout, no.x + 1, no.y), Some(1));
    assert_eq!(option_at(&layout, layout.dialog.x, layout.dialog.y), None);
    assert_eq!(option_at(&layout, yes.x, no.y + 1), None);
}

#[test]
fn narrow_screen_degrades_and_stays_inside_screen() {
    let (buf, layout) = draw(40, 10, 0);
    assert!(layout.logo.is_none(), "屏幕高度不足时省略 logo");
    assert!(layout.dialog.right() <= 40 && layout.dialog.bottom() <= 10);
    assert!(layout.options[1].bottom() <= 10, "选项在屏幕内可点击");
    let mut text_joined = String::new();
    for y in layout.dialog.y..layout.dialog.bottom() {
        text_joined.push_str(&row_text(
            &buf,
            Rect::new(layout.dialog.x, y, layout.dialog.width, 1),
        ));
        text_joined.push(' ');
    }
    // 窄屏下问题换行，断言两段子串都出现。
    assert!(
        text_joined.contains("Do you trust the files"),
        "{text_joined}"
    );
    assert!(text_joined.contains("folder?"), "{text_joined}");
    assert!(text_joined.contains(text::TRUST_OPTION_NO), "{text_joined}");
}
