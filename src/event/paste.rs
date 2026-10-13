//! prompt 粘贴入口：终端粘贴的路径文本提案与候选校验。
//!
//! 图片与文件都只以路径表达：终端送来整段路径（或 `file://` 列表）时先写占位文本再
//! 打开引用/路径面板；其余文本按普通文本插入。应用自身不读取系统剪贴板。

use std::path::PathBuf;

use crate::app::actions::EditorCommand;
use crate::app::prompt::{PasteAnchor, PasteCandidate, PasteCandidateKind};
use crate::app::state::AppState;
use crate::app::update;
use crate::layout;
use crate::ui;

/// 终端 `Paste(text)`：整段是存在的路径集合时打开提案，否则按普通文本插入。
///
/// 新粘贴先取消旧提案（Choosing → Reading）；终端自身粘贴只承载位图时送来空内容，
/// 没有正文可插入，直接忽略。
pub(super) fn handle_terminal_paste(state: &mut AppState, text: String) {
    state.prompt.paste_cancel();
    // 树视图没有正文提案入口：文本按搜索框语义处理。
    if update::prompt_tree_mode(state) {
        update::apply_editor(state, EditorCommand::InsertText(text));
        return;
    }
    if text.is_empty() {
        return;
    }
    if let Some(paths) = crate::platform::parse_path_text(&text)
        && let Some(candidates) = candidates_from_paths(&paths)
    {
        let anchor = paste_anchor(state);
        let request = state.prompt.begin_paste_read(anchor);
        state.prompt.paste_stage(request, candidates);
        return;
    }
    update::apply_editor(state, EditorCommand::InsertText(text));
}

/// 路径集合 → 已存在候选；任一项不存在返回 None（调用方回退普通文本）。
fn candidates_from_paths(paths: &[PathBuf]) -> Option<Vec<PasteCandidate>> {
    let mut candidates: Vec<PasteCandidate> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for path in paths {
        let metadata = std::fs::metadata(path).ok()?;
        let key = crate::platform::path_key(path);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let kind = if metadata.is_dir() {
            PasteCandidateKind::Folder
        } else if crate::platform::sniff_image_file(path) {
            PasteCandidateKind::Image
        } else {
            PasteCandidateKind::File
        };
        candidates.push(PasteCandidate::external(path.clone(), kind));
    }
    Some(candidates)
}

/// 当前编辑器的粘贴锚点：选区优先，回退光标。
pub(super) fn paste_anchor(state: &AppState) -> PasteAnchor {
    let cursor = state.prompt.cursor();
    let selection = state
        .selection
        .filter(|selection| selection.pane() == state.prompt.id())
        .and_then(|selection| selection.range())
        .and_then(|(start, end)| {
            let row = |row: i32| u16::try_from(row).ok();
            Some(((row(start.0)?, start.1), (row(end.0)?, end.1)))
        })
        .and_then(|(start, end)| state.prompt.selection_bounds(start, end));
    PasteAnchor { selection, cursor }
}

/// 粘贴二级面板命中。
pub(super) fn paste_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = prompt_text_area(view);
    ui::prompt::paste_item_at(&state.prompt, area, column, row)
}

/// 字母快捷面板命中。
pub(super) fn refer_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = prompt_text_area(view);
    ui::prompt::refer_item_at(&state.prompt, area, column, row)
}

/// prompt 文本区。
pub(super) fn prompt_text_area(view: &ui::layout::ViewLayout) -> ratatui::layout::Rect {
    layout::prompt_content_rect(view.prompt)
}
