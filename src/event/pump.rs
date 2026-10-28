//! 后台请求泵：提及扫描、worktree 查询、剪贴板与引用文件夹扫描。

use std::collections::HashSet;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::state::AppState;
use crate::app::update;

use super::AppEvent;

pub(crate) fn pump_mention_scan(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) {
    if !state.prompt_focused() {
        return;
    }
    let prompt = state.prompt.id();
    let Some((generation, root)) = state.prompt.take_mention_scan_request() else {
        return;
    };
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let entries = crate::files::scan(&root);
        let _ = sender.send(AppEvent::MentionScanned {
            prompt,
            generation,
            entries,
        });
    });
}

/// lx `@` 面板需要扫描的窗格：取走请求并在后台按根扫描，结果携窗格、代号与根回投。
///
/// 只为当前处于 lx 视图、有有效 `@` 触发且无缓存/在途的窗格发起；同一根在 pane 内缓存。
pub(crate) fn pump_lx_mention_scan(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) {
    for (pane, generation, root) in update::take_lx_mention_scan_requests(state) {
        let sender = sender.clone();
        let scan_root = root.clone();
        tokio::task::spawn_blocking(move || {
            let entries = crate::files::scan(&scan_root);
            let _ = sender.send(AppEvent::LxMentionScanned {
                pane,
                generation,
                root,
                entries,
            });
        });
    }
}

/// prompt 模板块的 worktree 绑定：选择器打开或块内绑定未解析时发起后台查询。
/// `inflight` 记录在途仓库根，避免同一仓库重复发起。
pub(crate) fn pump_prompt_worktree(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    inflight: &mut HashSet<PathBuf>,
) {
    if !update::prompt_panel_active(state) {
        return;
    }
    let chooser = state.prompt.worktree_chooser_open();
    let repo = if chooser {
        state.prompt.take_worktree_refresh()
    } else {
        state.prompt.take_worktree_request()
    };
    let Some(repo) = repo else {
        return;
    };
    if !inflight.insert(repo.clone()) {
        return;
    }
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let (candidates, failed) = match crate::git::list_worktrees(&repo) {
            Some(list) => (update::worktree_candidates(list.entries), false),
            None => (Vec::new(), true),
        };
        let _ = sender.send(AppEvent::PromptWorktreeListed {
            repo_root: repo,
            candidates,
            failed,
        });
    });
}

/// 剪贴板请求：读取剪贴板或确认原生写入；全部在阻塞线程执行，回投按序号/目标校验。
pub(crate) fn pump_clipboard_requests(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
) {
    if state.clipboard_requests.is_empty() {
        return;
    }
    let requests = std::mem::take(&mut state.clipboard_requests);
    for request in requests {
        let sender = sender.clone();
        match request {
            crate::app::state::ClipboardRequest::Read {
                id,
                target,
                generation,
            } => {
                tokio::task::spawn_blocking(move || {
                    let result = crate::platform::read_clipboard();
                    let _ = sender.send(AppEvent::ClipboardRead {
                        id,
                        target,
                        generation,
                        result,
                    });
                });
            }
            crate::app::state::ClipboardRequest::Write { id, text } => {
                tokio::task::spawn_blocking(move || {
                    let success = crate::platform::write_clipboard_confirmed(&text);
                    let _ = sender.send(AppEvent::ClipboardWriteFinished { id, success });
                });
            }
        }
    }
}

/// 引用文件夹后台请求：扫描目录内容或校验新增路径；结果按请求代号投递回主循环。
///
/// 请求由 `app` 层登记（打开弹窗/进入目录/提交输入），此处取走并在阻塞线程执行，
/// 绝不在事件循环或渲染路径同步遍历目录。路径校验需要判重，故读取当时的项目快照。
pub(crate) fn pump_referenced_requests(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
) {
    if state.referenced_requests.is_empty() {
        return;
    }
    let requests = std::mem::take(&mut state.referenced_requests);
    let known: Vec<PathBuf> = state
        .referenced
        .folders
        .iter()
        .map(|folder| folder.path.clone())
        .collect();
    for request in requests {
        let sender = sender.clone();
        let known = known.clone();
        match request {
            crate::app::referenced::ReferencedRequest::Scan { id, root, .. } => {
                tokio::task::spawn_blocking(move || {
                    let outcome = crate::files::scan_referenced(&root).map_err(|_| ());
                    let _ = sender.send(AppEvent::ReferencedScanned { id, outcome });
                });
            }
            crate::app::referenced::ReferencedRequest::ValidateAdd { id, input } => {
                tokio::task::spawn_blocking(move || {
                    let outcome = validate_referenced_path(&input, &known);
                    let _ = sender.send(AppEvent::ReferencedValidated { id, outcome });
                });
            }
        }
    }
}

/// 后台校验新增路径：绝对路径、存在、目录类型、引用可表达性与规范化去重。
///
/// 返回规范化路径或明确的失败原因；规范化后与快照重复时以 `Duplicate` 报告
/// （调用方回投时携带 duplicate 标记，避免在同一响应里既给成功又给重复）。
pub(crate) fn validate_referenced_path(
    input: &str,
    known: &[PathBuf],
) -> Result<PathBuf, crate::app::referenced::PathValidation> {
    use crate::app::referenced::PathValidation;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(PathValidation::Empty);
    }
    let path = std::path::Path::new(trimmed);
    if !path.is_absolute() {
        return Err(PathValidation::NotAbsolute);
    }
    if !path.exists() {
        return Err(PathValidation::NotFound);
    }
    if !path.is_dir() {
        return Err(PathValidation::NotDirectory);
    }
    if path.to_str().is_none() {
        return Err(PathValidation::NotRepresentable);
    }
    let Some(canonical) = std::fs::canonicalize(path).ok() else {
        return Err(PathValidation::NotFound);
    };
    if !crate::app::markdown::reference_representable(&canonical.to_string_lossy()) {
        return Err(PathValidation::NotRepresentable);
    }
    if known.contains(&canonical) {
        return Err(PathValidation::Duplicate);
    }
    Ok(canonical)
}
