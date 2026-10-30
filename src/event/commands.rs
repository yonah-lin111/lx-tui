//! 自定义命令重载、作用域根确定与命令视图转换。

use std::collections::HashSet;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::state::AppState;
use crate::app::update;
use crate::settings::commands::{self, CommandKind, CommandRecord, CommandScope, MdScope};

use super::AppEvent;

/// 重载自定义命令：User 与 Project 合并后注入 prompt；命令集变化时返回 true。
///
/// 每个显示工作区变化、设置页写入与仓库根变化后调用；`set_custom_commands`
/// 内部按内容比较，未变化时面板保持原样。
pub(crate) fn reload_custom_commands(state: &mut AppState, force: bool) -> bool {
    if !force {
        return false;
    }
    let (items, _) = custom_commands_for(state);
    let changed = items != state.prompt.custom_commands().to_vec();
    state.prompt.set_custom_commands(items);
    changed
}

/// 依据显示中 prompt 的工作区装载有效自定义命令与设置页视图快照。
pub(crate) fn custom_commands_for(
    state: &AppState,
) -> (
    Vec<crate::app::markdown::SlashItem>,
    Vec<crate::settings::service::CommandView>,
) {
    let user_root = match crate::config::user_data_root() {
        Ok(root) => root,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let (user, _user_errors) = commands::list(CommandScope::User, &user_root);
    let project_root = project_command_root(state);
    let (project, _project_errors) = match project_root.as_deref() {
        Some(root) => commands::list(CommandScope::Project, root),
        None => (Vec::new(), Vec::new()),
    };
    let mut views: Vec<crate::settings::service::CommandView> = Vec::new();
    let mut items = Vec::new();
    for record in &user {
        views.push(crate::settings::service::CommandView::of(
            record,
            CommandScope::User,
        ));
    }
    for record in &project {
        views.push(crate::settings::service::CommandView::of(
            record,
            CommandScope::Project,
        ));
    }
    for record in &user {
        items.push(slash_item_of(
            record,
            crate::app::markdown::CommandSource::User,
        ));
    }
    for record in &project {
        items.push(slash_item_of(
            record,
            crate::app::markdown::CommandSource::Project,
        ));
    }
    (items, views)
}

/// Project 作用域根：git 仓库取主 checkout；新工作区元数据未到达（cwd 已就绪）
/// 时用工作区路径；完全无路径时 None（不创建无关目录）。
fn project_command_root(state: &AppState) -> Option<PathBuf> {
    let workspace = state.prompt_owner();
    if let Some(git) = workspace.git.as_ref() {
        return Some(git.repo_root.clone());
    }
    workspace.cwd.clone().filter(|cwd| cwd.is_dir())
}

/// 命令记录 → 斜杠候选；`source` 由调用方按作用域给定。
fn slash_item_of(
    record: &CommandRecord,
    source: crate::app::markdown::CommandSource,
) -> crate::app::markdown::SlashItem {
    use crate::app::markdown::{SlashItem, SlashScope};
    match record.kind {
        CommandKind::AgentMd => SlashItem::custom_markdown(
            &record.name,
            source,
            match record.scope {
                MdScope::Global => SlashScope::Global,
                MdScope::Template => SlashScope::Template,
            },
            &record.body,
            &record.description,
        ),
        CommandKind::AgentBlock => match record.block {
            crate::settings::commands::BlockType::Template => SlashItem::custom_block(
                &record.name,
                source,
                record.title.as_deref(),
                &record.body,
                &record.description,
            ),
            crate::settings::commands::BlockType::Supple => SlashItem::custom_supple(
                &record.name,
                source,
                record.title.as_deref(),
                &record.body,
                &record.description,
            ),
        },
    }
}

/// 设置页命令快照（User 与 Project 合并后的视图）。
pub(crate) fn command_views(state: &AppState) -> Vec<crate::settings::service::CommandView> {
    custom_commands_for(state).1
}

/// 同步显示中工作区的 git 仓库根到 prompt（影响 `/gitWorkTree` 与绑定解析）；
/// 返回是否发生变化。
pub(crate) fn sync_prompt_repo(state: &mut AppState) -> bool {
    let repo = state
        .prompt_owner()
        .git
        .as_ref()
        .map(|git| git.repo_root.clone());
    state.prompt.set_worktree_repo(repo)
}

/// 启动时为已有工作区登记一次 git 元数据查询。
pub(crate) fn seed_git_requests(state: &mut AppState) {
    let cwds: Vec<PathBuf> = state
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.cwd.clone())
        .collect();
    for cwd in cwds {
        crate::app::update::request_git_refresh(state, &cwd);
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
fn validate_referenced_path(
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
