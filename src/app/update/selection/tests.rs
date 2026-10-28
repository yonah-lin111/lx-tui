//! 屏幕选区、粘贴目标快照与剪切事务测试。

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::overlay::{Overlay, Rename, RenameTarget, TextInput};
use crate::app::selection::{ScreenPoint, SelectionSource};
use crate::app::state::AppState;
use crate::app::update::clipboard::{
    finish_cut, paste_target, paste_target_matches, request_cut, request_paste,
};
use crate::ui::screen::ScreenText;

fn screen(lines: &[&str], width: u16) -> ScreenText {
    let area = Rect::new(0, 0, width, lines.len() as u16);
    let mut buffer = Buffer::empty(area);
    for (y, line) in lines.iter().enumerate() {
        buffer.set_string(0, y as u16, line, ratatui::style::Style::default());
    }
    ScreenText::from_buffer(&buffer)
}

#[test]
fn screen_selection_keeps_highlight_after_release() {
    let mut state = AppState::demo();
    let screen = screen(&["hello", "world"], 10);
    let now = Instant::now();
    begin_screen_selection(&mut state, &screen, ScreenPoint::new(0, 0), now);
    drag_screen_selection(&mut state, &screen, ScreenPoint::new(1, 4));
    finish_screen_selection(&mut state);
    assert!(state.screen_selection.is_some(), "松开不复制也不清选区");
    assert_eq!(
        screen_selection_text(&state).as_deref(),
        Some("hello\nworld")
    );
    assert_eq!(
        state.screen_selection.as_ref().and_then(|s| s.range()),
        Some((ScreenPoint::new(0, 0), ScreenPoint::new(1, 4)))
    );
}

#[test]
fn screen_selection_double_click_selects_word_and_triple_selects_row() {
    let mut state = AppState::demo();
    let screen = screen(&["hello-world", "tail"], 20);
    let now = Instant::now();
    // 第一次单击：字符。
    begin_screen_selection(&mut state, &screen, ScreenPoint::new(0, 2), now);
    finish_screen_selection(&mut state);
    assert_eq!(screen_selection_text(&state), None, "单击不形成非空选区");

    // 第二次：Unicode 词。
    begin_screen_selection(
        &mut state,
        &screen,
        ScreenPoint::new(0, 2),
        now + Duration::from_millis(100),
    );
    finish_screen_selection(&mut state);
    assert_eq!(screen_selection_text(&state).as_deref(), Some("hello"));

    // 第三次：整行。
    begin_screen_selection(
        &mut state,
        &screen,
        ScreenPoint::new(0, 2),
        now + Duration::from_millis(200),
    );
    finish_screen_selection(&mut state);
    assert_eq!(
        screen_selection_text(&state).as_deref(),
        Some("hello-world")
    );
}

#[test]
fn screen_selection_reanchors_after_scroll_and_hides_when_gone() {
    let mut state = AppState::demo();
    let before = screen(&["head", "alpha", "beta"], 8);
    begin_screen_selection(&mut state, &before, ScreenPoint::new(1, 0), Instant::now());
    drag_screen_selection(&mut state, &before, ScreenPoint::new(2, 3));
    finish_screen_selection(&mut state);

    let after = screen(&["new", "head", "alpha", "beta"], 8);
    assert!(refresh_screen_selection(&mut state, &after));
    let selection = state.screen_selection.as_ref().expect("selection");
    assert_eq!(
        selection.range(),
        Some((ScreenPoint::new(2, 0), ScreenPoint::new(3, 3)))
    );
    assert!(selection.visible());

    let gone = screen(&["something", "else"], 8);
    assert!(refresh_screen_selection(&mut state, &gone));
    assert!(
        !state
            .screen_selection
            .as_ref()
            .expect("selection")
            .visible()
    );
    assert!(
        state.screen_selection.is_some(),
        "内容滚出视口时保留选区，只隐藏高亮"
    );
}

#[test]
fn copyable_selection_reads_frozen_screen_text() {
    let mut state = AppState::demo();
    let screen = screen(&["abc"], 6);
    begin_screen_selection(&mut state, &screen, ScreenPoint::new(0, 0), Instant::now());
    drag_screen_selection(&mut state, &screen, ScreenPoint::new(0, 2));
    finish_screen_selection(&mut state);
    let mut cache = crate::ui::RenderCache::default();
    let copied = copyable_selection(&state, &[], &mut cache).expect("screen selection");
    assert_eq!(copied.text, "abc");
    assert_eq!(copied.source, SelectionSource::Screen);
}

#[test]
fn cut_transaction_deletes_only_after_confirmed_write_and_unchanged_revision() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("hello world"),
    }));
    {
        let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
            panic!("rename overlay");
        };
        rename.input.select_all();
    }
    assert!(request_cut(&mut state));
    let pending = state.pending_cut.clone().expect("pending cut");
    assert!(matches!(
        state.clipboard_requests.as_slice(),
        [crate::app::state::ClipboardRequest::Write { .. }]
    ));

    // 写入完成前修改编辑器：迟到回执不得删除任何文字。
    {
        let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
            panic!("rename overlay");
        };
        rename.input.clear_selection();
        rename.input.insert_char('!');
    }
    assert_eq!(finish_cut(&mut state, pending.id), None);
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        panic!("rename overlay");
    };
    assert!(rename.input.text().contains("hello world"));

    // 新事务在修订号未变时删除选区。
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("hello world"),
    }));
    {
        let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
            panic!("rename overlay");
        };
        rename.input.select_all();
    }
    assert!(request_cut(&mut state));
    let id = state.pending_cut.as_ref().expect("pending").id;
    assert_eq!(finish_cut(&mut state, id), Some(None));
    let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
        panic!("rename overlay");
    };
    assert_eq!(rename.input.text(), "");
    assert!(rename.input.undo(), "剪切保留撤销记录");
    assert_eq!(rename.input.text(), "hello world");
}

#[test]
fn paste_target_changes_with_focus_and_revision() {
    let mut state = AppState::demo();
    crate::app::update::focus_prompt(&mut state);
    let target = paste_target(&state);
    assert!(paste_target_matches(&state, &target));

    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('x'),
    );
    assert!(
        !paste_target_matches(&state, &target),
        "修订号变化后迟到粘贴必须丢弃"
    );
}

#[test]
fn clipboard_read_request_keeps_target_snapshot() {
    let mut state = AppState::demo();
    crate::app::update::focus_prompt(&mut state);
    assert!(request_paste(&mut state));
    let [crate::app::state::ClipboardRequest::Read { target, .. }] =
        state.clipboard_requests.as_slice()
    else {
        panic!("read request");
    };
    assert_eq!(target.focus, state.focus());
    assert!(!target.page_rename);
}

#[test]
fn triple_click_line_selection_stops_at_box_borders() {
    let mut state = AppState::demo();
    let screen = screen(&["┌────┬────┐", "│left│right│", "└────┴────┘"], 12);
    let now = Instant::now();
    let point = ScreenPoint::new(1, 2);
    // 同格 250 ms 内三击：字符 → 词 → 行。
    begin_screen_selection(&mut state, &screen, point, now);
    begin_screen_selection(&mut state, &screen, point, now + Duration::from_millis(80));
    begin_screen_selection(&mut state, &screen, point, now + Duration::from_millis(160));
    finish_screen_selection(&mut state);
    assert_eq!(screen_selection_text(&state).as_deref(), Some("left"));
}

#[test]
fn single_character_word_selection_is_not_empty() {
    let mut state = AppState::demo();
    let screen = screen(&["a"], 5);
    let point = ScreenPoint::new(0, 0);
    begin_screen_selection_unit(
        &mut state,
        &screen,
        point,
        crate::app::selection::SelectionUnit::Word,
    );
    finish_screen_selection(&mut state);
    assert_eq!(screen_selection_text(&state).as_deref(), Some("a"));
    assert!(
        state
            .screen_selection
            .as_ref()
            .and_then(|selection| selection.range())
            .is_some(),
        "单字符词仍应保留折叠范围"
    );
}

#[test]
fn screen_selection_region_excludes_input_rows() {
    let mut state = AppState::demo();
    let screen = screen(&["history one", "history two", "input text"], 20);
    let region = Rect::new(0, 0, 20, 2);
    begin_screen_selection_unit_in(
        &mut state,
        &screen,
        ScreenPoint::new(0, 0),
        crate::app::selection::SelectionUnit::Character,
        Some(region),
    );
    drag_screen_selection(&mut state, &screen, ScreenPoint::new(2, 10));
    finish_screen_selection(&mut state);
    assert_eq!(
        screen_selection_text(&state).as_deref(),
        Some("history one\nhistory two"),
        "输入框行不得进入历史选区"
    );
}

#[test]
fn pending_screen_selection_uses_the_next_frame() {
    let mut state = AppState::demo();
    let before = screen(&["aaa", "bbb"], 5);
    begin_screen_selection_unit(
        &mut state,
        &before,
        ScreenPoint::new(0, 0),
        crate::app::selection::SelectionUnit::Character,
    );
    drag_screen_selection(&mut state, &before, ScreenPoint::new(0, 2));
    state
        .screen_selection
        .as_mut()
        .expect("selection")
        .shift_rows(-1);
    state.screen_selection_refresh = Some((1, 2));
    let after = screen(&["aaa", "bbb"], 5);
    assert!(apply_pending_screen_selection(&mut state, &after));
    assert!(state.screen_selection_refresh.is_none());
    assert_eq!(
        screen_selection_text(&state).as_deref(),
        Some("aaa\nbbb"),
        "刷新必须使用滚动后的新画面"
    );
}

#[test]
fn prompt_word_selection_uses_local_editing_selection() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    state.prompt.set_text("hello world");
    let screen = screen(&["hello world"], 20);
    let point = ScreenPoint::new(0, 6);
    assert!(begin_prompt_screen_selection(
        &mut state,
        &screen,
        point,
        crate::app::selection::SelectionUnit::Word,
        Rect::new(0, 0, 20, 1),
    ));
    assert!(
        state.screen_selection.is_none(),
        "prompt 不保留只读屏幕选区"
    );
    assert_eq!(prompt_copy_text(&state).as_deref(), Some("world"));
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('X'),
    );
    assert_eq!(state.prompt.text(), "hello X", "输入应替换选中的词");
}

#[test]
fn prompt_triple_click_selects_full_wrapped_logical_line() {
    let mut state = AppState::demo();
    state.prompt.resize(5, 5);
    state.prompt.set_text("hello world");
    let screen = screen(&["hello", "world"], 5);
    assert!(begin_prompt_screen_selection(
        &mut state,
        &screen,
        ScreenPoint::new(0, 1),
        crate::app::selection::SelectionUnit::Line,
        Rect::new(0, 0, 5, 2),
    ));
    assert_eq!(
        prompt_copy_text(&state).as_deref(),
        Some("hello world"),
        "三击应覆盖同一逻辑行的全部软折行"
    );
}

#[test]
fn prompt_single_character_line_selection_is_editable() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 3);
    state.prompt.set_text("a");
    let screen = screen(&["a"], 5);
    assert!(begin_prompt_screen_selection(
        &mut state,
        &screen,
        ScreenPoint::new(0, 0),
        crate::app::selection::SelectionUnit::Line,
        Rect::new(0, 0, 5, 1),
    ));
    assert_eq!(prompt_copy_text(&state).as_deref(), Some("a"));
}

#[test]
fn prompt_cursor_movement_clears_select_all() {
    let mut state = AppState::demo();
    crate::app::update::focus_prompt(&mut state);
    state.prompt.insert_str("hello");
    state.select_all_prompt();
    assert!(state.selection.is_some());
    crate::app::update::apply_editor(&mut state, crate::app::actions::EditorCommand::Left);
    assert!(state.selection.is_none(), "光标移动取消全选");
}
