//! prompt 粘贴入口：剪贴板读取、路径文本提案、结果校验与 managed 截图落盘。

use std::path::PathBuf;
use std::time::Instant;

use tokio::sync::mpsc;

use crate::app::actions::EditorCommand;
use crate::app::markdown::ReferKind;
use crate::app::prompt::{PasteAnchor, PasteCandidate, PasteCandidateKind};
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::db::service::Persistence;
use crate::layout;
use crate::layout::PaneId;
use crate::platform::{ClipboardContent, ClipboardError, ImageFormat};
use crate::ui;
use crate::ui::text;

use super::AppEvent;

/// 剪贴板读取结果载体；位图已在后台落盘为 managed 候选。
#[derive(Debug)]
pub enum ClipboardPayload {
    Paths(Vec<PathBuf>),
    Text(String),
    Image { asset_id: String, path: PathBuf },
    Error(ClipboardError),
}

/// `Ctrl+V`：记录请求代号并发起后台剪贴板读取。
pub(super) fn request_clipboard_read(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
) {
    let anchor = paste_anchor(state);
    let request = state.prompt.begin_paste_read(anchor);
    let prompt = state.prompt.id();
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let payload = read_clipboard_payload(crate::platform::read_clipboard());
        let _ = sender.send(AppEvent::ClipboardRead {
            prompt,
            request,
            payload,
        });
    });
}

/// 终端 `Paste(text)`：整段是存在的路径集合时打开提案，否则按普通文本插入。
///
/// 新粘贴先取消旧提案（Choosing → Reading），普通文本仍是同步编辑行为。
pub(super) fn handle_terminal_paste(state: &mut AppState, text: String) {
    state.prompt.paste_cancel();
    // 树视图没有正文提案入口：文本按搜索框语义处理。
    if update::prompt_tree_mode(state) {
        update::apply_editor(state, EditorCommand::InsertText(text));
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

/// 读取结果路由：目标、请求与修订任一不匹配即丢弃并清理新生成的截图。
///
/// 位图直接以 `@[refer-image]` 插入正文（不进面板）；路径/路径文本走占位插入 + 面板。
pub(super) fn apply_clipboard_read(
    state: &mut AppState,
    persistence: &Persistence,
    prompt: PaneId,
    request: u64,
    payload: ClipboardPayload,
) {
    if state.prompt.id() != prompt
        || !state.prompt.paste_request_matches(request)
        || update::prompt_tree_mode(state)
    {
        discard_payload(payload);
        return;
    }
    match payload {
        ClipboardPayload::Error(error) => {
            state.prompt.paste_discard();
            let message = match error {
                ClipboardError::Empty => text::TOAST_CLIPBOARD_EMPTY,
                ClipboardError::Unavailable(_) => text::TOAST_CLIPBOARD_READ_FAILED,
            };
            update::show_toast(state, toast(ToastKind::Error, message));
        }
        ClipboardPayload::Paths(paths) => {
            let Some(candidates) =
                candidates_from_paths(&paths).filter(|candidates| !candidates.is_empty())
            else {
                state.prompt.paste_discard();
                update::show_toast(
                    state,
                    toast(ToastKind::Error, text::TOAST_CLIPBOARD_READ_FAILED),
                );
                return;
            };
            if !state.prompt.paste_stage(request, candidates) {
                state.prompt.paste_discard();
                update::show_toast(
                    state,
                    toast(ToastKind::Error, text::TOAST_CLIPBOARD_READ_FAILED),
                );
            }
        }
        ClipboardPayload::Text(content) => {
            // 纯文本先尝试整段路径解析；失败走普通文本插入。
            if let Some(paths) = crate::platform::parse_path_text(&content)
                && let Some(candidates) = candidates_from_paths(&paths)
                && state.prompt.paste_stage(request, candidates)
            {
                return;
            }
            state.prompt.paste_discard();
            update::apply_editor(state, EditorCommand::InsertText(content));
        }
        ClipboardPayload::Image { asset_id, path } => {
            // 位图不进面板：登记 managed 资产后直接把正文写成 @[refer-image]（对齐 lx-agent）。
            let Some(anchor) = state.prompt.paste_settle(request) else {
                if let Err(error) = std::fs::remove_file(&path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(%error, path = %path.display(), "stale screenshot cleanup failed");
                }
                return;
            };
            if !persistence.register_managed_asset(&asset_id, &path) {
                let _ = std::fs::remove_file(&path);
                update::show_toast(state, toast(ToastKind::Error, text::TOAST_IMAGE_FAILED));
                return;
            }
            if !state
                .prompt
                .insert_reference(&anchor, ReferKind::Image, &path)
            {
                persistence.unregister_managed_asset(&asset_id);
                let _ = std::fs::remove_file(&path);
                update::show_toast(state, toast(ToastKind::Error, text::TOAST_IMAGE_FAILED));
            }
        }
    }
}

/// 过期/错误结果清理：managed 截图（未登记）直接删除文件。
fn discard_payload(payload: ClipboardPayload) {
    if let ClipboardPayload::Image { path, .. } = payload
        && let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), "stale screenshot cleanup failed");
    }
}

/// 后台读取并按内容生成结果载体；位图立即原子落盘为 managed 候选。
fn read_clipboard_payload(result: Result<ClipboardContent, ClipboardError>) -> ClipboardPayload {
    match result {
        Ok(ClipboardContent::Files(files)) => ClipboardPayload::Paths(files),
        Ok(ClipboardContent::Text(text)) => ClipboardPayload::Text(text),
        Ok(ClipboardContent::Image { bytes, format }) => stage_screenshot(&bytes, format)
            .map_or_else(
                |reason| {
                    tracing::warn!(%reason, "screenshot staging failed");
                    ClipboardPayload::Error(ClipboardError::Unavailable(reason))
                },
                |(asset_id, path)| ClipboardPayload::Image { asset_id, path },
            ),
        Err(error) => ClipboardPayload::Error(error),
    }
}

/// 位图原子落盘：先写 `.tmp-*` 再改名；文件名与资产 ID 一致。
fn stage_screenshot(bytes: &[u8], format: ImageFormat) -> Result<(String, PathBuf), String> {
    let dir =
        crate::platform::attachments_dir().ok_or_else(|| "data dir unavailable".to_string())?;
    stage_screenshot_in(&dir, bytes, format)
}

/// 指定目录的位图原子落盘（目录可注入，测试隔离用）。
pub(super) fn stage_screenshot_in(
    dir: &std::path::Path,
    bytes: &[u8],
    format: ImageFormat,
) -> Result<(String, PathBuf), String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let asset_id = crate::platform::secure_token();
    let name = format!("{asset_id}.{}", format.extension());
    let final_path = dir.join(&name);
    let temp_path = dir.join(format!(".tmp-{name}"));
    let mut file = std::fs::File::create(&temp_path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    std::fs::rename(&temp_path, &final_path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        error.to_string()
    })?;
    Ok((asset_id, final_path))
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

/// 错误/toast 快捷构造。
fn toast(kind: ToastKind, message: &'static str) -> Toast {
    Toast::new(kind, message, None, Instant::now())
}
