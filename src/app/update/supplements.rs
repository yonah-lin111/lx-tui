//! 补充块附件模态的纯状态更新：打开、选择、编辑、保存、取消与删除。
//!
//! 所有判断走领域数据与父 Prompt 的派生索引；不读几何、不做 IO。
//! 保存与删除前复核 owner/revision/原文快照，目标变化时保留草稿并拒绝覆盖。

use crate::app::actions::OverlayKey;
use crate::app::overlay::Overlay;
use crate::app::prompt::presentation::supplement_index;
use crate::app::prompt::{PromptEditing, PromptFocus};
use crate::app::state::AppState;
use crate::app::supplements::{
    SaveOutcome, SupplementDraft, SupplementView, Supplements, apply_draft, new_block_text,
};
use crate::app::supplements::{SupplementOwner, SupplementTarget};
use crate::ui::text;

use super::menu::close_overlay;

/// 斜杠来源的补充块：先建本地草稿，保存时才在冻结的触发行插入完整块。
///
/// 打开草稿不删命令行；取消后用户已输入的命令文本原样保留。
pub fn start_slash_supplement(state: &mut AppState) -> bool {
    if state.overlay.is_some() {
        return false;
    }
    let Some((command, title, body)) = state.prompt.slash_supple_draft() else {
        return false;
    };
    let Some(trigger) = state.prompt.slash_trigger_range() else {
        return false;
    };
    let cursor = state.prompt.cursor();
    let Some(parent_start) =
        crate::app::markdown::template_block_start_at(state.prompt.text(), cursor)
    else {
        return false;
    };
    let Some(span) = state.prompt.template_block(parent_start) else {
        return false;
    };
    // 闭合父任务插在结束标记前；未闭合父任务追加到文末。
    let insert_before = span.end;
    let text = state.prompt.text().to_string();
    let session = Supplements::new(
        owner_for(state),
        state.prompt.revision(),
        parent_start,
        text,
        Vec::new(),
    );
    state.prompt.clear_panel();
    state.overlay = Some(Overlay::Supplements(Box::new(session)));
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        let mut draft =
            SupplementDraft::blank(title.as_deref().unwrap_or_default(), &body, insert_before);
        draft.command = command;
        draft.trigger = Some(trigger);
        session.draft = Some(draft);
        session.view = SupplementView::Edit;
    }
    true
}

/// 打开当前光标（或入口焦点）所在父任务的附件列表；无安全附件时返回 false。
///
/// 冻结 owner（身份/页/root）、修订号与附件快照，后续保存逐项复核。
pub fn open_supplements(state: &mut AppState) -> bool {
    if state.overlay.is_some() || state.prompt.paste_panel().is_some() {
        return false;
    }
    let Some(parent_start) = current_parent(state) else {
        return false;
    };
    open_supplements_for(state, parent_start)
}

/// 打开指定父任务的附件列表；供鼠标入口与键盘入口共用。
pub fn open_supplements_for(state: &mut AppState, parent_start: usize) -> bool {
    let text = state.prompt.text().to_string();
    // 目标必须是 `&&&` 任务块；附件数可以为 0（空态仍提供新增入口）。
    if state.prompt.template_block(parent_start).is_none() {
        return false;
    }
    let targets: Vec<SupplementTarget> = supplement_index(&text)
        .entries_for(parent_start)
        .iter()
        .filter_map(|entry| SupplementTarget::from_entry(&text, entry))
        .collect();
    let session = Supplements::new(
        owner_for(state),
        state.prompt.revision(),
        parent_start,
        text,
        targets,
    );
    state.prompt.clear_panel();
    state
        .prompt
        .set_focus(PromptFocus::Supplements { parent_start });
    state.overlay = Some(Overlay::Supplements(Box::new(session)));
    true
}

/// 当前焦点归属的父任务：入口焦点直接取它，文本光标取所在任务块。
fn current_parent(state: &AppState) -> Option<usize> {
    if let PromptFocus::Supplements { parent_start } = state.prompt.focus() {
        return Some(parent_start);
    }
    let cursor = state.prompt.cursor();
    let start = crate::app::markdown::template_block_start_at(state.prompt.text(), cursor)?;
    state
        .prompt
        .supplement_index()
        .contains_parent(start)
        .then_some(start)
}

/// 冻结当前显示中 Prompt 的归属身份。
fn owner_for(state: &AppState) -> SupplementOwner {
    let workspace = state.workspaces.get(state.prompt_workspace());
    let panel = &state.prompt_owner().prompt_panel;
    let page = panel.editing.and_then(|target| match target {
        PromptEditing::Item(id) => panel
            .tree
            .item(id)
            .and_then(|item| item.pages.get(item.active_index()).map(|page| page.id)),
        PromptEditing::Temp(id) => panel.temp(id).and_then(|temp| {
            temp.pages
                .get(
                    temp.pages
                        .iter()
                        .position(|page| page.id == temp.active_page)
                        .unwrap_or(0),
                )
                .map(|page| page.id)
        }),
    });
    SupplementOwner {
        pane: state.prompt.id(),
        workspace: workspace
            .and_then(|workspace| workspace.repo_key())
            .map(Into::into),
        target: panel.editing,
        page,
        root: state.prompt_root.clone(),
    }
}

/// 附件模态按键分派；返回是否产生了变化（调用方据此置脏）。
pub fn apply_supplements_key(state: &mut AppState, key: OverlayKey) {
    let Some((view, editable)) = state.overlay.as_ref().and_then(|overlay| match overlay {
        Overlay::Supplements(session) => Some((session.view, session.editable)),
        _ => None,
    }) else {
        return;
    };
    // 尺寸不足以编辑时只放行退出：不处理写操作、不丢草稿。
    if !editable && key != OverlayKey::Esc {
        return;
    }
    match view {
        SupplementView::List => list_key(state, key),
        SupplementView::Edit => edit_key(state, key),
        SupplementView::Unsaved => unsaved_key(state, key),
        SupplementView::ConfirmDelete => confirm_key(state, key),
    }
}

/// 列表态：上下/Home/End 选择、回车打开、Delete 进入删除确认、
/// Ctrl+C 复制当前正文、Esc 关闭整个模态。
fn list_key(state: &mut AppState, key: OverlayKey) {
    let Some(Overlay::Supplements(mut session)) = state.overlay.take() else {
        return;
    };
    let visible = session.list_rows.max(1) as isize;
    match key {
        OverlayKey::Esc => {
            state.overlay = Some(Overlay::Supplements(session));
            close_supplements(state);
            return;
        }
        OverlayKey::Up => {
            session.move_selection(-1);
        }
        OverlayKey::Down => {
            session.move_selection(1);
        }
        OverlayKey::Home => {
            session.select_edge(false);
        }
        OverlayKey::End => {
            session.select_edge(true);
        }
        OverlayKey::Enter => {
            match session.selected_entry().cloned() {
                Some(target) => {
                    let text = session.text_snapshot.clone();
                    session.draft = Some(SupplementDraft::existing(&text, target));
                    session.view = SupplementView::Edit;
                    session.hint = None;
                }
                // 新增动作行：进入空草稿（保存时才插入父 Prompt）。
                None => {
                    session.draft = Some(SupplementDraft::blank("", "", session.parent_start));
                    session.view = SupplementView::Edit;
                    session.hint = None;
                }
            }
        }
        OverlayKey::Delete | OverlayKey::Remove if session.selected_entry().is_some() => {
            session.enter_confirm_delete();
        }
        OverlayKey::PageUp => {
            session.scroll_viewport(-visible, visible as usize);
        }
        OverlayKey::PageDown => {
            session.scroll_viewport(visible, visible as usize);
        }
        _ => {}
    }
    let copy = key == OverlayKey::Copy;
    state.overlay = Some(Overlay::Supplements(session));
    if copy {
        copy_selected(state);
    }
}

/// 编辑态：编辑键进草稿（标题行或正文），Ctrl/Cmd+S 保存，Esc 返回列表；
/// 有修改时进入三选一确认。内部候选面板（草稿的 @ / 斜杠）优先消费按键。
fn edit_key(state: &mut AppState, key: OverlayKey) {
    let Some(Overlay::Supplements(mut session)) = state.overlay.take() else {
        return;
    };
    let Some(draft) = session.draft.as_mut() else {
        session.view = SupplementView::List;
        state.overlay = Some(Overlay::Supplements(session));
        return;
    };
    match key {
        OverlayKey::Esc => {
            if draft.body.panel_open() {
                draft.body.panel_escape_key();
            } else if draft.dirty() {
                session.enter_unsaved();
            } else {
                session.draft = None;
                session.view = SupplementView::List;
            }
        }
        OverlayKey::Save => {
            state.overlay = Some(Overlay::Supplements(session));
            save_draft(state);
            return;
        }
        OverlayKey::NextField | OverlayKey::Tab => {
            draft.title_focused = !draft.title_focused;
        }
        OverlayKey::PreviousField | OverlayKey::BackTab => {
            draft.title_focused = !draft.title_focused;
        }
        OverlayKey::Undo => draft.body.undo(),
        OverlayKey::Redo => draft.body.redo(),
        _ if draft.title_focused => match key {
            OverlayKey::Enter => {
                draft.title_focused = false;
                let end = draft.body.text().len();
                draft.body.set_cursor(end);
            }
            OverlayKey::Char(ch) => draft.title.insert_char(ch),
            OverlayKey::Clear => draft.title.clear(),
            OverlayKey::Backspace => draft.title.backspace(),
            OverlayKey::Delete => draft.title.delete(),
            OverlayKey::Left => draft.title.move_left(),
            OverlayKey::Right => draft.title.move_right(),
            OverlayKey::Home => draft.title.move_home(),
            OverlayKey::End => draft.title.move_end(),
            _ => {}
        },
        _ => draft_editor_key(draft, key),
    }
    state.overlay = Some(Overlay::Supplements(session));
}

/// 正文草稿编辑键。
fn draft_editor_key(draft: &mut SupplementDraft, key: OverlayKey) {
    match key {
        OverlayKey::Enter | OverlayKey::EnterFolder => draft.body.newline(),
        OverlayKey::Char(ch) => draft.body.insert_char(ch),
        OverlayKey::Backspace => draft.body.backspace(),
        OverlayKey::Delete => draft.body.delete(),
        OverlayKey::Left => draft.body.move_left(),
        OverlayKey::Right => draft.body.move_right(),
        OverlayKey::Up => draft.body.move_up(),
        OverlayKey::Down => draft.body.move_down(),
        OverlayKey::Home => draft.body.move_home(),
        OverlayKey::End => draft.body.move_end(),
        _ => {}
    }
}

/// 未保存确认：左右/Tab 选择按钮，Enter 激活焦点项，Esc 等价 keep editing。
fn unsaved_key(state: &mut AppState, key: OverlayKey) {
    let Some(Overlay::Supplements(mut session)) = state.overlay.take() else {
        return;
    };
    match key {
        // Esc 等价 keep editing（默认焦点）。
        OverlayKey::Esc => {
            session.view = SupplementView::Edit;
            state.overlay = Some(Overlay::Supplements(session));
        }
        OverlayKey::Left | OverlayKey::BackTab => {
            session.move_button_focus(-1);
            state.overlay = Some(Overlay::Supplements(session));
        }
        OverlayKey::Right | OverlayKey::Tab => {
            session.move_button_focus(1);
            state.overlay = Some(Overlay::Supplements(session));
        }
        OverlayKey::Enter | OverlayKey::Save => {
            let focus = session.button_focus;
            state.overlay = Some(Overlay::Supplements(session));
            match focus {
                crate::app::supplements::UNSAVED_SAVE_INDEX => save_draft(state),
                crate::app::supplements::UNSAVED_DISCARD_INDEX => discard_draft(state),
                _ => keep_editing(state),
            }
        }
        _ => {
            session.view = SupplementView::Edit;
            state.overlay = Some(Overlay::Supplements(session));
        }
    }
}

/// 鼠标点击确认态按钮：先设置焦点再按 Enter 激活该按钮。
pub fn click_supplement_button(state: &mut AppState, index: usize) {
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut()
        && index < session.button_count()
    {
        session.button_focus = index;
    }
    apply_supplements_key(state, OverlayKey::Enter);
}

/// 放弃草稿：只丢本地修改，父 Prompt 与历史不变。
pub fn discard_draft(state: &mut AppState) {
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.draft = None;
        session.view = SupplementView::List;
        session.hint = None;
    }
}

/// 继续编辑：回到详情，草稿原样保留。
pub fn keep_editing(state: &mut AppState) {
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.view = SupplementView::Edit;
    }
}

/// 删除确认：左右/Tab 选择按钮，Enter 激活焦点项（默认 cancel），Esc 无条件取消。
fn confirm_key(state: &mut AppState, key: OverlayKey) {
    let Some(Overlay::Supplements(mut session)) = state.overlay.take() else {
        return;
    };
    match key {
        OverlayKey::Esc => {
            session.view = SupplementView::List;
            state.overlay = Some(Overlay::Supplements(session));
        }
        OverlayKey::Left | OverlayKey::BackTab => {
            session.move_button_focus(-1);
            state.overlay = Some(Overlay::Supplements(session));
        }
        OverlayKey::Right | OverlayKey::Tab => {
            session.move_button_focus(1);
            state.overlay = Some(Overlay::Supplements(session));
        }
        OverlayKey::Enter => {
            let focus = session.button_focus;
            state.overlay = Some(Overlay::Supplements(session));
            if focus == crate::app::supplements::CONFIRM_DELETE_INDEX {
                delete_selected(state);
            } else if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
                session.view = SupplementView::List;
            }
        }
        _ => {
            state.overlay = Some(Overlay::Supplements(session));
        }
    }
}

/// 保存当前草稿：归属、修订与原文快照全部匹配才写回父 Prompt。
///
/// 成功返回列表并保持该项可见；失败留在详情，草稿与错误提示保留。
pub fn save_draft(state: &mut AppState) {
    let Some(Overlay::Supplements(session)) = state.overlay.as_ref() else {
        return;
    };
    let Some(draft) = session.draft.as_ref() else {
        return;
    };
    if draft.new {
        insert_new(state);
        return;
    }
    if !owner_matches(state, session) {
        set_hint(state, text::SUPPLEMENTS_TARGET_CHANGED);
        return;
    }
    let outcome = if snapshot_matches(state, session, draft) {
        apply_draft(&mut state.prompt, draft)
    } else {
        SaveOutcome::TargetChanged
    };
    match outcome {
        SaveOutcome::Changed | SaveOutcome::Unchanged => {
            let parent_start = session.parent_start;
            state
                .prompt
                .set_focus(PromptFocus::Supplements { parent_start });
            refresh_entries(state);
            if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
                session.view = SupplementView::List;
                session.draft = None;
                session.hint = None;
            }
        }
        SaveOutcome::Rejected => set_hint(state, text::SUPPLEMENTS_SAVE_REJECTED),
        SaveOutcome::TargetChanged => set_hint(state, text::SUPPLEMENTS_TARGET_CHANGED),
    }
}

/// 草稿编辑的块快照是否仍与父 Prompt 一致（打开后目标未被改写）。
fn snapshot_matches(state: &AppState, session: &Supplements, draft: &SupplementDraft) -> bool {
    if draft.new {
        return true;
    }
    let Some(target) = draft.target.as_ref() else {
        return false;
    };
    state.prompt.text().get(target.range.clone()) == Some(target.snapshot.as_str())
        && session.revision <= state.prompt.revision()
}

/// 新建草稿保存：列表来源插到父任务结束行之前，斜杠来源替换冻结的触发行。
fn insert_new(state: &mut AppState) {
    let Some(Overlay::Supplements(session)) = state.overlay.as_ref() else {
        return;
    };
    let Some(draft) = session.draft.as_ref() else {
        return;
    };
    let Some(span) = state.prompt.template_block(session.parent_start) else {
        set_hint(state, text::SUPPLEMENTS_TARGET_CHANGED);
        return;
    };
    if !owner_matches(state, session) {
        set_hint(state, text::SUPPLEMENTS_TARGET_CHANGED);
        return;
    }
    let block = new_block_text(draft, text::SUPPLEMENTS_DEFAULT_COMMAND);
    let inserted = match draft.trigger.clone() {
        Some(trigger) => state.prompt.replace_supplement_trigger(&trigger, &block),
        None if span.closed => state.prompt.insert_supplement_before(span.end, &block),
        None => state.prompt.append_supplement(&block),
    };
    if !inserted {
        set_hint(state, text::SUPPLEMENTS_SAVE_REJECTED);
        return;
    }
    refresh_entries(state);
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.view = SupplementView::List;
        session.draft = None;
        session.hint = None;
    }
}

/// 删除选中附件：复核归属与原文快照后删除，保留列表并钳制选中。
fn delete_selected(state: &mut AppState) {
    let Some(Overlay::Supplements(session)) = state.overlay.as_ref() else {
        return;
    };
    let Some(target) = session.selected_entry().cloned() else {
        return;
    };
    if !owner_matches(state, session) || !snapshot_matches_target(state, &target) {
        set_hint(state, text::SUPPLEMENTS_TARGET_CHANGED);
        if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
            session.view = SupplementView::List;
        }
        return;
    }
    if !state.prompt.delete_supple_block(target.start) {
        set_hint(state, text::SUPPLEMENTS_SAVE_REJECTED);
        return;
    }
    refresh_entries(state);
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.clamp_after_removal();
        session.view = SupplementView::List;
        session.hint = None;
    }
}

/// 目标快照是否仍与当前原文一致。
fn snapshot_matches_target(state: &AppState, target: &SupplementTarget) -> bool {
    state.prompt.text().get(target.range.clone()) == Some(target.snapshot.as_str())
}

/// 用父编辑器当前文本重建列表快照与选中；父任务已无安全附件时关闭模态。
pub fn refresh_entries(state: &mut AppState) {
    let Some(Overlay::Supplements(session)) = state.overlay.as_ref() else {
        return;
    };
    let parent_start = session.parent_start;
    let text = state.prompt.text().to_string();
    let entries: Vec<SupplementTarget> = supplement_index(&text)
        .entries_for(parent_start)
        .iter()
        .filter_map(|entry| SupplementTarget::from_entry(&text, entry))
        .collect();
    let revision = state.prompt.revision();
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.entries = entries;
        session.text_snapshot = text;
        session.revision = revision;
        session.clamp_after_removal();
    }
}

/// 会话归属是否仍然匹配；活动工作区索引变化不算失效（钉住与重排都可能改变它）。
fn owner_matches(state: &AppState, session: &Supplements) -> bool {
    session.owner == owner_for(state)
}

/// 设置模态内错误提示。
fn set_hint(state: &mut AppState, hint: &'static str) {
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.hint = Some(hint);
    }
}

/// 新建附件草稿：沿用内置补充块结构。
pub fn start_new_supplement(state: &mut AppState) -> bool {
    let Some(Overlay::Supplements(session)) = state.overlay.as_mut() else {
        return false;
    };
    session.draft = Some(SupplementDraft::blank("", "", session.parent_start));
    session.view = SupplementView::Edit;
    session.hint = None;
    true
}

/// 列表选中项的正文复制：复用补充块复制语义，经既有剪贴板效应执行。
pub fn copy_selected(state: &mut AppState) -> bool {
    let Some(Overlay::Supplements(session)) = state.overlay.as_ref() else {
        return false;
    };
    let Some(target) = session.selected_entry() else {
        return false;
    };
    let Some(body) = session
        .text_snapshot
        .get(target.body.clone())
        .map(str::to_string)
    else {
        return false;
    };
    let copy = crate::app::markdown::copy_template_content(&body);
    if copy.trim().is_empty() {
        return false;
    }
    state
        .effects
        .push(crate::app::effects::Effect::CopyToClipboard {
            text: copy,
            anchor: Some(state.prompt.id()),
        });
    true
}

/// 关闭整个附件模态：草稿 dirty 时先进入未保存确认。
pub fn close_supplements(state: &mut AppState) {
    let Some(Overlay::Supplements(session)) = state.overlay.as_ref() else {
        return;
    };
    if session.draft.as_ref().is_some_and(SupplementDraft::dirty) {
        if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
            session.view = SupplementView::Unsaved;
        }
        return;
    }
    close_overlay(state);
}

/// 按当前屏幕几何刷新附件模态：列表窗口、可编辑性与草稿正文尺寸。
///
/// 事件层在按键/鼠标前后调用；渲染层仍只读，不在绘制中改状态。
pub fn sync_supplement_geometry(state: &mut AppState, screen: ratatui::layout::Rect) {
    if !matches!(state.overlay, Some(Overlay::Supplements(_))) {
        return;
    }
    let Some(layout) = crate::ui::overlay::supplements_layout(screen) else {
        return;
    };
    resolve_supplement_rows(state, &layout);
}

/// 把列表几何同步进会话（事件层按当前模态布局调用）。
pub fn resolve_supplement_rows(
    state: &mut AppState,
    layout: &crate::ui::overlay::SupplementLayout,
) {
    let (rows, editable) = match state.overlay.as_ref() {
        Some(Overlay::Supplements(session)) => (
            crate::ui::overlay::list_rows(layout, session),
            crate::ui::overlay::supplements_editable(&layout.shell),
        ),
        _ => return,
    };
    let body = crate::ui::overlay::detail_body_rect(layout);
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut() {
        session.list_rows = rows.max(1);
        session.editable = editable;
        // 草稿正文编辑器按详情正文区同步尺寸：换行与光标几何必须与绘制一致。
        if let Some(draft) = session.draft.as_mut() {
            draft.body.resize(body.width.max(1), body.height.max(1));
        }
    }
}

/// 列表滚轮：只滚动列表视口（不改变选中项）；返回是否移动。
pub fn scroll_supplement_list(state: &mut AppState, delta: isize, visible: usize) -> bool {
    match state.overlay.as_mut() {
        Some(Overlay::Supplements(session)) => session.scroll_viewport(delta, visible),
        _ => false,
    }
}

/// 列表悬停行：只改 hover 底色；返回是否变化。
pub fn hover_supplement_row(state: &mut AppState, hover: Option<usize>) -> bool {
    match state.overlay.as_mut() {
        Some(Overlay::Supplements(session)) => {
            if session.hover == hover {
                return false;
            }
            session.hover = hover;
            true
        }
        _ => false,
    }
}

/// 附件列表双击判定窗口。
const SUPPLEMENT_DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// 单击列表行：只选中；双击（同项、窗口内）才打开草稿。
pub fn click_supplement_row(state: &mut AppState, index: usize, now: std::time::Instant) {
    let double = state.supplement_click.is_some_and(|(last, at)| {
        last == index && now.saturating_duration_since(at) <= SUPPLEMENT_DOUBLE_CLICK
    });
    state.supplement_click = (!double).then_some((index, now));
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut()
        && index < session.row_count()
    {
        session.selected = index;
        session.viewport = None;
    }
    if double {
        open_selected_supplement(state);
    }
}

/// 打开列表当前行进入草稿编辑（Enter 与双击共用）；
/// 停在新增动作行时建立空草稿。
pub fn open_selected_supplement(state: &mut AppState) -> bool {
    let Some(Overlay::Supplements(session)) = state.overlay.as_mut() else {
        return false;
    };
    match session.selected_entry().cloned() {
        Some(target) => {
            let text = session.text_snapshot.clone();
            session.draft = Some(SupplementDraft::existing(&text, target));
        }
        None if session.on_add_row() => {
            session.draft = Some(SupplementDraft::blank("", "", session.parent_start));
        }
        None => return false,
    }
    session.view = SupplementView::Edit;
    session.hint = None;
    true
}

/// 详情焦点切换：`title` 为真聚焦标题行，否则聚焦正文。
pub fn focus_supplement_title(state: &mut AppState, title: bool) {
    if let Some(Overlay::Supplements(session)) = state.overlay.as_mut()
        && let Some(draft) = session.draft.as_mut()
    {
        draft.title_focused = title;
    }
}

/// 清理当前草稿：只清理本地草稿的空占位项，作为本地一步撤销；不触碰父 Prompt。
pub fn clean_supplement_draft(state: &mut AppState) {
    let Some(Overlay::Supplements(session)) = state.overlay.as_mut() else {
        return;
    };
    let Some(draft) = session.draft.as_mut() else {
        return;
    };
    let cleaned = crate::app::markdown::clean_template_content(draft.body.text());
    let cleaned = crate::app::markdown::ensure_bottom_gap(cleaned);
    if cleaned == draft.body.text() {
        return;
    }
    draft.body.set_text(&cleaned);
}

#[cfg(test)]
mod tests;
