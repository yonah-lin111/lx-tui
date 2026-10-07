//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn styles_stay_palette_native() {
    for style in [
        text(),
        muted(),
        accent(),
        border(true),
        border(false),
        error(),
        strong(),
    ] {
        assert!(matches!(
            style.fg,
            None | Some(Color::Cyan) | Some(Color::Red)
        ));
    }
}

#[test]
fn selected_item_fills_accent_with_contrast_foreground() {
    let style = selected_item();
    assert_eq!(style.fg, Some(Color::Black));
    assert_eq!(style.bg, Some(Color::Cyan));
    assert!(style.add_modifier.contains(Modifier::BOLD));
}
