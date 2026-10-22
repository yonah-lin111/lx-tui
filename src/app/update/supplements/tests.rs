//! 附件模态更新测试：打开、草稿、未保存确认、删除与目标失效。

use super::*;
use crate::app::actions::EditorCommand;
use crate::app::overlay::Overlay;
use crate::app::prompt::PromptPresentation;
use crate::app::supplements::SupplementView;
use crate::app::update::{apply_editor, toggle_prompt_presentation};

const TEXT: &str = concat!(
    "&&& a --start\n",
    "{title: }\n",
    "body\n",
    "+++ supplement --start\n",
    "{title: First}\n",
    "\n",
    "one\n",
    "+++ supplement --end\n",
    "\n",
    "+++ supplement --start\n",
    "{title: Second}\n",
    "\n",
    "two\n",
    "+++ supplement --end\n",
    "&&& a --end",
);

fn state_with(text: &str) -> AppState {
    let mut state = AppState::demo();
    state.prompt.resize(80, 24);
    state.prompt.set_text(text);
    state.prompt_focused = true;
    state
}

fn session(state: &AppState) -> &Supplements {
    match state.overlay.as_ref() {
        Some(Overlay::Supplements(session)) => session,
        _ => panic!("supplements modal open"),
    }
}

fn session_mut(state: &mut AppState) -> &mut Supplements {
    match state.overlay.as_mut() {
        Some(Overlay::Supplements(session)) => session,
        _ => panic!("supplements modal open"),
    }
}

#[test]
fn open_freezes_entries_and_does_not_touch_text() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    let revision = state.prompt.revision();
    assert_eq!(session(&state).entries.len(), 2);
    assert_eq!(session(&state).entries[0].label, "First");
    assert_eq!(state.prompt.text(), TEXT);
    assert_eq!(state.prompt.revision(), revision, "打开不置脏");
    assert!(matches!(
        state.prompt.focus(),
        PromptFocus::Supplements { parent_start: 0 }
    ));
}

#[test]
fn opening_parent_without_supplements_shows_empty_state() {
    let mut state = state_with("&&& a --start\nbody\n&&& a --end");
    assert!(open_supplements_for(&mut state, 0));
    assert!(session(&state).entries.is_empty());
    assert_eq!(session(&state).view, SupplementView::List);
    // 非模板块行不能打开模态。
    state.overlay = None;
    assert!(!open_supplements_for(&mut state, 99));
    assert!(state.overlay.is_none());
}

#[test]
fn edit_and_save_returns_to_list_with_one_undo_step() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).view, SupplementView::Edit);
    let draft = session(&state).draft.as_ref().expect("draft");
    assert!(draft.title_focused);
    // 标题行输入后回车切到正文，再写两个字。
    apply_supplements_key(&mut state, OverlayKey::Char('X'));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('!'));
    assert!(session(&state).draft.as_ref().expect("draft").dirty());
    apply_supplements_key(&mut state, OverlayKey::Save);

    assert_eq!(session(&state).view, SupplementView::List);
    assert!(state.prompt.text().contains("{title: FirstX}"));
    assert!(state.prompt.text().contains("one!"));
    assert!(
        state.prompt.text().contains("{title: Second}"),
        "其他附件不变"
    );
    assert!(state.prompt.can_undo());
    state.prompt.undo();
    assert_eq!(state.prompt.text(), TEXT, "父编辑器一步撤销恢复");
}

#[test]
fn esc_with_dirty_draft_enters_unsaved_then_discard() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('Z'));
    apply_supplements_key(&mut state, OverlayKey::Esc);
    assert_eq!(session(&state).view, SupplementView::Unsaved);
    assert_eq!(
        session(&state).button_focus,
        crate::app::supplements::UNSAVED_KEEP_INDEX,
        "未保存确认默认焦点在 keep editing"
    );
    // 默认焦点下 Enter 只是继续编辑，不保存。
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).view, SupplementView::Edit);
    assert_eq!(state.prompt.text(), TEXT);
    // Esc → Esc → 左移到 discard → Enter：丢弃草稿，正文不变。
    apply_supplements_key(&mut state, OverlayKey::Esc);
    apply_supplements_key(&mut state, OverlayKey::Left);
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).view, SupplementView::List);
    assert_eq!(state.prompt.text(), TEXT);
    assert!(!state.prompt.can_undo());
}

#[test]
fn delete_needs_confirmation_and_is_undoable() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Delete);
    assert_eq!(session(&state).view, SupplementView::ConfirmDelete);
    assert_eq!(
        session(&state).button_focus,
        crate::app::supplements::CONFIRM_CANCEL_INDEX,
        "删除确认默认焦点在 cancel"
    );
    assert_eq!(state.prompt.text(), TEXT, "确认前不改原文");
    // 默认焦点下 Enter 只取消，不删除。
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).view, SupplementView::List);
    assert_eq!(state.prompt.text(), TEXT);
    // 再进确认，左移到 confirm 后 Enter 才删除。
    apply_supplements_key(&mut state, OverlayKey::Delete);
    apply_supplements_key(&mut state, OverlayKey::Left);
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).entries.len(), 1);
    assert!(!state.prompt.text().contains("First"));
    assert!(state.prompt.text().contains("Second"));
    state.prompt.undo();
    assert_eq!(state.prompt.text(), TEXT);
}

#[test]
fn deleting_last_entry_keeps_empty_state_with_add_action() {
    let text = "&&& a --start\n+++ supplement --start\n{title: Only}\n\nx\n+++ supplement --end\n&&& a --end";
    let mut state = state_with(text);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Delete);
    apply_supplements_key(&mut state, OverlayKey::Left);
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert!(
        matches!(state.overlay, Some(Overlay::Supplements(_))),
        "留在空态"
    );
    assert!(session(&state).entries.is_empty());
    assert!(!state.prompt.text().contains("+++ supplement"));
    assert!(start_new_supplement(&mut state), "空态仍可新增");
}

#[test]
fn new_supplement_cancel_does_not_change_parent() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    assert!(start_new_supplement(&mut state));
    apply_supplements_key(&mut state, OverlayKey::Char('N'));
    apply_supplements_key(&mut state, OverlayKey::Esc);
    assert_eq!(session(&state).view, SupplementView::Unsaved);
    apply_supplements_key(&mut state, OverlayKey::Left);
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.prompt.text(), TEXT);
    assert_eq!(session(&state).entries.len(), 2);
}

#[test]
fn new_supplement_save_inserts_one_block_and_undoes_once() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    assert!(start_new_supplement(&mut state));
    apply_supplements_key(&mut state, OverlayKey::Char('N'));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('b'));
    apply_supplements_key(&mut state, OverlayKey::Save);
    assert_eq!(session(&state).entries.len(), 3);
    assert!(state.prompt.text().contains("{title: N}"));
    state.prompt.undo();
    assert_eq!(state.prompt.text(), TEXT);
}

#[test]
fn target_change_rejects_save_and_keeps_draft() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('Q'));
    // 外部重新载入正文（revision 不增，但快照变了）。
    state.prompt.set_text(&TEXT.replace("one", "changed"));
    state.prompt.set_presentation(PromptPresentation::Compact);
    assert!(open_supplements_for(&mut state, 0));
    session_mut(&mut state).draft = None;
    // 直接复用旧草稿路径：重新打开后写入并篡改目标。
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('R'));
    state.prompt.set_text(&TEXT.replace("two", "swapped"));
    apply_supplements_key(&mut state, OverlayKey::Save);
    assert_eq!(session(&state).view, SupplementView::Edit, "失败留在详情");
    assert_eq!(
        session(&state).hint,
        Some(crate::ui::text::SUPPLEMENTS_TARGET_CHANGED)
    );
    assert!(session(&state).draft.as_ref().expect("draft").dirty());
    assert!(state.prompt.text().contains("swapped"));
}

#[test]
fn close_with_clean_draft_closes_modal() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    close_supplements(&mut state);
    assert!(state.overlay.is_none());
    assert_eq!(state.prompt.text(), TEXT);
}

#[test]
fn copy_selected_queues_clipboard_effect_without_hidden_markers() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    assert!(copy_selected(&mut state));
    match state.effects.last() {
        Some(crate::app::effects::Effect::CopyToClipboard { text, .. }) => {
            assert_eq!(text, "one");
        }
        other => panic!("expected clipboard effect, got {other:?}"),
    }
}

#[test]
fn selection_copy_in_compact_mode_excludes_hidden_supplements() {
    let text = "&&& a --start\nA\n+++ supplement --start\n{title: T}\n\nhidden\n+++ supplement --end\nB\n&&& a --end";
    let mut state = state_with(text);
    state.prompt.set_presentation(PromptPresentation::Compact);
    let copied = state.prompt.selection_text((0, 0), (u16::MAX, 0));
    // 行数越界会被钳到最后一显示行；这里只关心隐藏内容不出现。
    assert!(copied.is_none_or(|text| !text.contains("hidden")));
    // 只选入口行：没有可复制文字，不回落全文。
    let entry_row = state
        .prompt
        .display_rows()
        .iter()
        .position(|row| {
            matches!(
                row,
                crate::app::prompt::PromptDisplayRow::Supplements { .. }
            )
        })
        .expect("entry row") as u16;
    assert_eq!(
        state.prompt.selection_text((entry_row, 0), (entry_row, 0)),
        None
    );
}

#[test]
fn typing_across_hidden_supplement_is_rejected_and_text_unchanged() {
    let text = "&&& a --start\nA\n+++ supplement --start\n{title: T}\n\nhidden\n+++ supplement --end\nB\n&&& a --end";
    let mut state = state_with(text);
    state.prompt.set_presentation(PromptPresentation::Compact);
    // 把光标放在 B 前，向回删除到 A 会跨过隐藏块。
    let at = state.prompt.text().find('B').expect("B");
    state.prompt.set_cursor(at);
    apply_editor(&mut state, EditorCommand::DeleteWordBackward);
    assert_eq!(state.prompt.text(), text, "跨隐藏附件的删除整次拒绝");
    apply_editor(&mut state, EditorCommand::InsertChar('z'));
    assert_eq!(state.prompt.text(), text.replace("B", "zB"));
}

#[test]
fn entry_focus_blocks_text_writes_and_enter_opens_modal() {
    let text =
        "&&& a --start\n+++ supplement --start\n{title: T}\n\nx\n+++ supplement --end\n&&& a --end";
    let mut state = state_with(text);
    state.prompt.set_presentation(PromptPresentation::Compact);
    state.prompt.focus_supplement_entry(0);
    assert!(state.prompt.focus().is_supplements());
    apply_editor(&mut state, EditorCommand::InsertChar('q'));
    assert_eq!(state.prompt.text(), text, "入口焦点不写入");
    apply_editor(&mut state, EditorCommand::Newline);
    assert!(
        matches!(state.overlay, Some(Overlay::Supplements(_))),
        "Enter 打开列表"
    );
}

#[test]
fn f4_toggle_keeps_history_and_text() {
    let mut state = state_with(TEXT);
    apply_editor(&mut state, EditorCommand::InsertChar('a'));
    assert!(state.prompt.can_undo());
    let revision = state.prompt.revision();
    toggle_prompt_presentation(&mut state);
    assert_eq!(state.prompt.text(), format!("{TEXT}a"));
    assert!(state.prompt.can_undo(), "切换不清空撤销历史");
    assert_eq!(state.prompt.revision(), revision, "切换不置脏");
    toggle_prompt_presentation(&mut state);
    assert_eq!(state.prompt.presentation(), PromptPresentation::Compact);
}

#[test]
fn selection_copy_does_not_fall_back_to_full_text_for_entry_only() {
    let mut state = state_with(TEXT);
    let row = state
        .prompt
        .display_rows()
        .iter()
        .position(|row| {
            matches!(
                row,
                crate::app::prompt::PromptDisplayRow::Supplements { .. }
            )
        })
        .expect("entry") as i32;
    let pane = state.prompt.id();
    let mut selection = crate::app::selection::Selection::begin(pane, row, 0);
    selection.drag(row, 3);
    state.selection = Some(selection);
    assert!(
        crate::app::update::prompt_copy_text(&state).is_none(),
        "只选入口不回落全文"
    );
}

#[test]
fn list_scroll_keeps_selection_and_clamps_at_bounds() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    session_mut(&mut state).list_rows = 1;
    // 行数 = 2 个附件 + 新增动作行 = 3；视口高 1 时最大起点为 2。
    assert!(scroll_supplement_list(&mut state, 1, 1));
    assert_eq!(session(&state).selected, 0, "滚轮不改变选中项");
    assert_eq!(session(&state).window_start(1), 1);
    assert!(scroll_supplement_list(&mut state, 5, 1));
    assert_eq!(session(&state).window_start(1), 2, "到底后钳制");
    assert!(
        !scroll_supplement_list(&mut state, 5, 1),
        "已在底部不再移动"
    );
    assert!(scroll_supplement_list(&mut state, -9, 1));
    assert_eq!(session(&state).window_start(1), 0);
}

#[test]
fn add_row_is_keyboard_reachable_and_opens_blank_draft() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    let add_row = session(&state).row_count() - 1;
    assert!(session_mut(&mut state).move_selection(add_row as isize));
    assert!(session(&state).on_add_row());
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).view, SupplementView::Edit);
    assert!(session(&state).draft.as_ref().expect("draft").new);
}

#[test]
fn delete_keeps_list_open_and_selects_neighbour() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    session_mut(&mut state).selected = 1;
    apply_supplements_key(&mut state, OverlayKey::Delete);
    apply_supplements_key(&mut state, OverlayKey::Left);
    apply_supplements_key(&mut state, OverlayKey::Enter);
    assert_eq!(session(&state).entries.len(), 1);
    assert_eq!(session(&state).selected, 0, "删末项回退上一项");
    assert!(state.prompt.text().contains("First"));
}

#[test]
fn hover_row_only_paints_and_does_not_move_selection() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    assert!(hover_supplement_row(&mut state, Some(1)));
    assert_eq!(session(&state).hover, Some(1));
    assert_eq!(session(&state).selected, 0, "悬停不移动选中");
    assert!(!hover_supplement_row(&mut state, Some(1)), "同值不重复置脏");
}

#[test]
fn slash_supplement_opens_draft_and_cancel_keeps_command_line() {
    let mut state = state_with("");
    apply_editor(
        &mut state,
        EditorCommand::InsertText("&&& addTemplate --start {title: }\n".into()),
    );
    apply_editor(&mut state, EditorCommand::InsertText("/supplement".into()));
    assert!(state.prompt.slash_supple_draft().is_some(), "候选为补充块");
    let before = state.prompt.text().to_string();
    assert!(start_slash_supplement(&mut state));
    assert_eq!(session(&state).view, SupplementView::Edit);
    assert_eq!(state.prompt.text(), before, "打开草稿不删命令行");
    // 取消：命令行与附件数都不变。
    apply_supplements_key(&mut state, OverlayKey::Esc);
    assert!(matches!(state.overlay, Some(Overlay::Supplements(_))));
    assert_eq!(state.prompt.text(), before);
}

#[test]
fn slash_supplement_save_replaces_trigger_line_once() {
    let mut state = state_with("&&& addTemplate --start {title: }\n");
    let at = state.prompt.text().len();
    apply_editor(&mut state, EditorCommand::InsertText("/supplement".into()));
    apply_editor(
        &mut state,
        EditorCommand::InsertText("\n&&& addTemplate --end".into()),
    );
    state.prompt.set_cursor(at + "/supplement".len());
    // 回到命令行行尾（settle 会重算斜杠面板）。
    apply_editor(&mut state, EditorCommand::End);
    assert!(state.prompt.slash_supple_draft().is_some());
    assert!(start_slash_supplement(&mut state));
    apply_supplements_key(&mut state, OverlayKey::Char('T'));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('x'));
    apply_supplements_key(&mut state, OverlayKey::Save);
    assert!(
        !state.prompt.text().contains("/supplement"),
        "命令行被整行替换"
    );
    assert_eq!(
        crate::app::prompt::supplement_index(state.prompt.text())
            .entries
            .len(),
        1
    );
    state.prompt.undo();
    assert!(
        state.prompt.text().contains("/supplement"),
        "一步撤销恢复命令行"
    );
}

#[test]
fn empty_parent_still_opens_modal_with_add_action() {
    let mut state = state_with("&&& a --start\nbody\n&&& a --end");
    assert!(open_supplements_for(&mut state, 0));
    assert!(session(&state).entries.is_empty(), "空态仍可进入");
    assert!(start_new_supplement(&mut state));
    apply_supplements_key(&mut state, OverlayKey::Char('T'));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('x'));
    apply_supplements_key(&mut state, OverlayKey::Save);
    assert_eq!(session(&state).entries.len(), 1);
    assert!(state.prompt.text().contains("+++ supplement --start"));
}

#[test]
fn tiny_screen_pauses_editing_and_keeps_draft() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('K'));
    // 终端缩到不可编辑尺寸：写键被吞，草稿保留。
    sync_supplement_geometry(&mut state, ratatui::layout::Rect::new(0, 0, 20, 6));
    assert!(!session(&state).editable);
    apply_supplements_key(&mut state, OverlayKey::Char('!'));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    let draft = session(&state).draft.as_ref().expect("draft kept");
    assert_eq!(draft.title.text(), "FirstK");
    // Esc 仍可达：进入未保存确认。
    apply_supplements_key(&mut state, OverlayKey::Esc);
    assert_eq!(session(&state).view, SupplementView::Unsaved);
    // 放大后继续编辑。
    sync_supplement_geometry(&mut state, ratatui::layout::Rect::new(0, 0, 120, 40));
    assert!(session(&state).editable);
    apply_supplements_key(&mut state, OverlayKey::Esc);
    assert_eq!(session(&state).view, SupplementView::Edit);
}

#[test]
fn selection_replace_across_hidden_is_rejected_without_fallback_insert() {
    let text = "&&& a --start\nA\n+++ supplement --start\n{title: T}\n\nhidden\n+++ supplement --end\nB\n&&& a --end";
    let mut state = state_with(text);
    state.prompt.set_presentation(PromptPresentation::Compact);
    // 从第一行拖到 B 之后：选区跨过隐藏附件。
    let stop_row = state
        .prompt
        .display_rows()
        .iter()
        .position(|row| match row {
            crate::app::prompt::PromptDisplayRow::Text { start, .. } => {
                state.prompt.text()[*start..].starts_with('B')
            }
            _ => false,
        })
        .expect("B row") as u16;
    let mut selection = crate::app::selection::Selection::begin(state.prompt.id(), 0, 0);
    selection.drag(i32::from(stop_row), 1);
    state.selection = Some(selection);
    let before = state.prompt.text().to_string();
    apply_editor(&mut state, EditorCommand::InsertChar('z'));
    assert_eq!(
        state.prompt.text(),
        before,
        "跨隐藏附件整次拒绝，不退化插入"
    );
    assert!(state.selection.is_some(), "拒绝时保留选区");
}

#[test]
fn draft_supports_local_undo_redo() {
    let mut state = state_with(TEXT);
    assert!(open_supplements_for(&mut state, 0));
    apply_supplements_key(&mut state, OverlayKey::Enter);
    // 正文焦点：输入两个字符。
    apply_supplements_key(&mut state, OverlayKey::Enter);
    apply_supplements_key(&mut state, OverlayKey::Char('a'));
    apply_supplements_key(&mut state, OverlayKey::Char('b'));
    let draft = session(&state).draft.as_ref().expect("draft");
    assert!(draft.body.text().ends_with("ab"), "{}", draft.body.text());
    apply_supplements_key(&mut state, OverlayKey::Undo);
    apply_supplements_key(&mut state, OverlayKey::Undo);
    let draft = session(&state).draft.as_ref().expect("draft");
    assert!(!draft.body.text().ends_with("ab"));
    apply_supplements_key(&mut state, OverlayKey::Redo);
    assert_ne!(
        session(&state).draft.as_ref().expect("draft").body.text(),
        ""
    );
    // 本地撤销不改父 Prompt。
    assert_eq!(state.prompt.text(), TEXT);
}
