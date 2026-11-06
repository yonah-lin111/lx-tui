//! `lx-tui exec`：终端无头执行一轮 agent。
//!
//! 模块边界（design §9）：只依赖 `chat` / `config` / `db` / `mcp` / `git` / `platform`，
//! 绝不导入 `app` / `ui` / `tui` / `event`——无头路径不初始化终端，也不经过事件循环。
//! 本文件只做顶层编排；协议、输出与宿主状态机分别由 `events` / `processor` / `runner` 负责。

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::chat::agent_loop::{ModelContext, TurnOutcome};
use crate::chat::events::RequestPurpose;
use crate::chat::{DraftImageAttachment, ModelImage, ModelItem, context};
use crate::db::Database;
use crate::db::chat::{ModelSnapshot, ModelSwitchCommit};

pub mod cli;
pub mod events;
mod mutation;
pub mod processor;
pub mod runner;
mod subagent;

#[cfg(test)]
mod tests;

pub use cli::{
    CliAction, ColorMode, ExecCli, ExecCommand, ExecModelTarget, HelpTopic, ResumeArgs, help_text,
    no_color_requested, parse_cli, prompt_with_stdin_context, resolve_model_target,
    resolve_prompt_input,
};
pub use events::{
    ExecUsage, FileChangeEntry, ItemStatus, ThreadError, ThreadEvent, ThreadItem,
    ThreadItemDetails, TodoEntry,
};
pub use processor::{
    ExecEventProcessor, HumanOutputProcessor, JsonlOutputProcessor, SessionHeader,
};
pub use runner::{
    BoxStreamFuture, ExecStreamFactory, ExecTurnConfig, RealStreamFactory, run_exec_turn,
};

/// 正常完成。
pub const EXIT_OK: i32 = 0;
/// 参数错误、配置缺失、网络/协议错误或轮次失败。
pub const EXIT_ERR: i32 = 1;
/// 用户按下 `Ctrl+C`（SIGINT）中断。
pub const EXIT_INTERRUPTED: i32 = 130;

/// 单条用户消息超出模型上下文预算的稳定错误码（不截断用户输入）。
pub const DRAFT_EXCEEDS_CONTEXT_CODE: &str = "message exceeds model context";

/// 恢复会话时既没有 `SESSION_ID` 也没有 `--last` 的稳定错误文案。
const RESUME_ADDRESSING_ERROR: &str =
    "Specify a session ID or pass '--last' to resume the most recent session.";

/// 定位要恢复的会话：显式 `SESSION_ID` 必须存在于该工作区，`--last` 取最近一条。
pub fn resolve_resume_session(
    db: &Database,
    workspace: &Path,
    args: &ResumeArgs,
) -> Result<String, String> {
    if let Some(id) = args.session_id.as_deref() {
        let found = db
            .load_chat_session(workspace, id)
            .map_err(|error| format!("database error: {error}"))?;
        if found.is_none() {
            return Err(format!(
                "Session '{id}' not found in workspace {}.",
                workspace.display()
            ));
        }
        return Ok(id.to_string());
    }
    if args.last {
        let sessions = db
            .list_chat_sessions(workspace, 0, 1)
            .map_err(|error| format!("database error: {error}"))?;
        let Some(latest) = sessions.into_iter().next() else {
            return Err(format!(
                "No existing sessions found in workspace {}.",
                workspace.display()
            ));
        };
        return Ok(latest.external_id);
    }
    Err(RESUME_ADDRESSING_ERROR.to_string())
}

/// 装配一轮可执行的无头请求，并已在库中开启该轮（user + streaming assistant）。
///
/// 恢复会话时先读取既有历史（模型切换判定需要最后一条 assistant 的模型），
/// 再记录模型切换与开轮：切换记录必须排在新的 user 消息之前，才能按时间线注入。
pub fn assemble_exec_turn(
    db: &Database,
    workspace: &Path,
    cwd: &Path,
    existing_session: Option<&str>,
    raw_prompt: &str,
    image_paths: &[PathBuf],
    model: &ExecModelTarget,
) -> Result<ExecTurnConfig, String> {
    let mcp_snapshot = mcp_snapshot_for_turn(db, workspace);
    assemble_exec_turn_with_snapshot(
        db,
        workspace,
        cwd,
        existing_session,
        raw_prompt,
        image_paths,
        model,
        mcp_snapshot,
    )
}

/// 与 [`assemble_exec_turn`] 相同，但 MCP 快照由调用方注入（测试与同进程复用）。
pub(crate) fn assemble_exec_turn_with_snapshot(
    db: &Database,
    workspace: &Path,
    cwd: &Path,
    existing_session: Option<&str>,
    raw_prompt: &str,
    image_paths: &[PathBuf],
    model: &ExecModelTarget,
    mcp_snapshot: Arc<crate::mcp::McpToolSnapshot>,
) -> Result<ExecTurnConfig, String> {
    let draft_images = load_draft_images(cwd, image_paths, model)?;
    let prepared = crate::chat::prepare_user_message(cwd, raw_prompt, &draft_images)
        .map_err(|error| format!("failed to prepare prompt: {}", error.code()))?;
    let skills = skills_catalog_prompt(workspace, false);
    let skill_blocks = skill_reference_blocks(workspace, raw_prompt);
    let mut messages = Vec::new();
    if let Some(session_id) = existing_session {
        messages = db
            .load_chat_session(workspace, session_id)
            .map_err(|error| format!("database error: {error}"))?
            .ok_or_else(|| {
                format!(
                    "Session '{session_id}' not found in workspace {}.",
                    workspace.display()
                )
            })?;
    }
    let instructions = load_instructions(workspace, cwd);
    if let Some(session_id) = existing_session {
        record_model_switch_if_changed(
            db,
            workspace,
            cwd,
            session_id,
            model,
            &messages,
            instructions.is_some(),
        )?;
    }
    let model_switches = match existing_session {
        Some(session_id) => db
            .model_switches(workspace, session_id)
            .map_err(|error| format!("database error: {error}"))?,
        None => Vec::new(),
    };
    // 恢复会话时沿用仍生效的压缩摘要；否则历史会重新灌满上下文窗口。
    let compact = match existing_session {
        Some(session_id) => db
            .active_checkpoint(workspace, session_id)
            .map_err(|error| format!("database error: {error}"))?
            .map(|checkpoint| context::CompactContext {
                end_sequence: checkpoint.end_sequence,
                summary: checkpoint.summary_text,
                model_label: checkpoint.model_label,
            }),
        None => None,
    };

    let committed = db
        .begin_turn_with_attachments(
            workspace,
            existing_session,
            &prepared.raw_text,
            &prepared.attachments,
            &model.snapshot(),
        )
        .map_err(|error| format!("failed to start turn: {error}"))?;

    let environment = crate::chat::prompt::environment_snapshot(
        &crate::platform::EnvironmentFacts::capture(),
        Some(workspace),
        Some(cwd),
        &[],
    );
    let model_identity = model_identity(model).render_prompt_section();
    let mcp_prompt = mcp_snapshot.instructions_prompt();
    let web_search = web_search_keys();

    let assembly = context::assemble(&crate::chat::AssemblyInput {
        messages: &messages,
        draft: &prepared.raw_text,
        draft_attachments: &committed.attachments,
        model: &model.snapshot(),
        context_limit: model.context_limit,
        output_limit: model.output_limit,
        variant_params: model.variant_params.clone(),
        compact: compact.as_ref(),
        model_identity: Some(&model_identity),
        environment: Some(&environment),
        instructions: instructions.as_deref(),
        skills: skills.as_deref(),
        mcp: mcp_prompt.as_deref(),
        model_switches: &model_switches,
        skill_blocks: &skill_blocks,
        supports_image_input: model.supports_image_input,
    });
    if assembly.draft_exceeds {
        // 已开轮（streaming assistant 已落库）：先结算为失败，避免留下半轮记录。
        let _ = db.finish(
            workspace,
            &committed.assistant_message_id,
            &[],
            &crate::db::chat::TurnOutcome::Failed {
                error_code: DRAFT_EXCEEDS_CONTEXT_CODE.to_string(),
            },
            crate::db::chat::Usage::default(),
        );
        return Err(DRAFT_EXCEEDS_CONTEXT_CODE.to_string());
    }

    let mut request = assembly.request;
    hydrate_turn_images(
        db,
        workspace,
        &committed.session_id,
        model.supports_image_input,
        &assembly.item_image_attachments,
        &prepared.model_images,
        &mut request.items,
    );
    let subagents_config = crate::config::subagents::merge_project_roles_for_paths(
        &crate::config::subagents::load_subagents_or_default(),
        Some(workspace),
        Some(cwd),
        true,
    );
    request.purpose = RequestPurpose::ExecTurn;
    request.system_prompt = Some(assembly.system_prompt);
    request.tools = Arc::new(crate::chat::tools::turn_tools_with_subagents(
        crate::chat::agent::ToolSurface::ExecMain,
        &mcp_snapshot,
        Some(&subagents_config),
    ));
    let mut runtime_summary: Option<String> = None;
    if let Some(guidance) = crate::chat::agent::delegation_mode_guidance(
        subagents_config.delegation_mode,
        model.variant.as_deref(),
    ) {
        runtime_summary = Some(guidance.to_string());
    }
    let subagent_lines = subagent::known_subagent_lines(db, workspace, &committed.session_id);
    if !subagent_lines.is_empty() {
        let block = subagent_lines.join("\n");
        match &mut runtime_summary {
            Some(existing) => {
                existing.push_str("\n\n");
                existing.push_str(&block);
            }
            None => runtime_summary = Some(block),
        }
    }
    let context = ModelContext::new(request, cwd.to_path_buf())
        .with_context_limit(model.context_limit)
        .with_runtime_summary(runtime_summary);

    Ok(ExecTurnConfig {
        workspace: workspace.to_path_buf(),
        cwd: cwd.to_path_buf(),
        session_id: committed.session_id,
        user_message_id: committed.user_message_id,
        assistant_id: committed.assistant_message_id,
        prompt_display: prepared.raw_text,
        model: model.clone(),
        context,
        exa_api_key: web_search.exa_api_key,
        tavily_api_key: web_search.tavily_api_key,
        mcp_snapshot,
    })
}

/// 无头入口：解析 cwd / 工作区 / 提示词 / 模型，开库开轮后交给运行器执行。
pub fn run_main(
    cli: ExecCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let cwd = resolve_cwd(cli.cwd.as_deref())?;
    let workspace = crate::git::list_worktrees(&cwd)
        .map(|list| list.repo_root)
        .unwrap_or_else(|| cwd.clone());

    let (prompt_arg, images) = match &cli.command {
        Some(ExecCommand::Resume(args)) => (args.prompt.as_deref(), args.images.as_slice()),
        None => (cli.prompt.as_deref(), cli.images.as_slice()),
    };
    let prompt = resolve_prompt_input(prompt_arg, std::io::stdin().is_terminal(), read_stdin)?;

    // MCP 冷启动必须先有 Tokio 运行时：同步 prewarm 在短命 CLI 进程里永远只能拿到空快照。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // OAuth 凭证刷新需要 runtime：先刷新再解析模型配置。
    let settings = runtime.block_on(load_ai_settings())?;
    let effective_worktree = linked_worktree.map(Path::to_path_buf).or_else(|| {
        crate::db::is_dev_environment()
            .then(|| crate::git::linked_worktree_root(&cwd))
            .flatten()
    });
    let db = if cli.ephemeral {
        Database::open_in_memory()?
    } else {
        Database::open_default(effective_worktree.as_deref())?
    };

    let resumed = match &cli.command {
        Some(ExecCommand::Resume(args)) => Some(resolve_resume_session(&db, &workspace, args)?),
        None => None,
    };
    // 已有会话的锁必须在开轮前取得：另一个进程正在写同一会话时立即失败。
    let resumed_lock = resumed.as_deref().map(lock_session).transpose()?;

    let session_model = resumed
        .as_deref()
        .map(|session_id| last_assistant_model(&db, &workspace, session_id))
        .transpose()?
        .flatten();
    let model = resolve_model_target(
        &settings,
        cli.model.as_deref(),
        session_model.as_ref().map(|snapshot| {
            (
                snapshot.provider.as_str(),
                snapshot.model.as_str(),
                snapshot.variant.as_deref(),
            )
        }),
    )?;
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let disabled_mcp: std::collections::BTreeSet<String> =
        cli.disabled_mcp_servers.iter().cloned().collect();
    let snapshot = mcp_snapshot_for_turn_async(&db, &workspace, &disabled_mcp, cancel_rx.clone());
    let mcp_snapshot = runtime.block_on(snapshot);

    let turn_config = assemble_exec_turn_with_snapshot(
        &db,
        &workspace,
        &cwd,
        resumed.as_deref(),
        &prompt,
        images,
        &model,
        mcp_snapshot,
    )?;
    // 新会话的 id 只有开轮后才知道：此时补上会话锁；失败时结算刚开的轮次。
    let _session_lock = match resumed_lock {
        Some(guard) => guard,
        None => lock_session(&turn_config.session_id).inspect_err(|_| {
            let _ = db.finish(
                &workspace,
                &turn_config.assistant_id,
                &[],
                &crate::db::chat::TurnOutcome::Failed {
                    error_code: "session lock failed".to_string(),
                },
                crate::db::chat::Usage::default(),
            );
        })?,
    };

    let stdout_is_terminal = std::io::stdout().is_terminal();
    let use_color = cli
        .color
        .should_color_with_env(std::io::stderr().is_terminal(), no_color_requested());
    let mut processor: Box<dyn ExecEventProcessor> = if cli.json {
        Box::new(JsonlOutputProcessor::<std::io::Stdout>::stdio(
            cli.last_message_file.clone(),
        ))
    } else {
        Box::new(
            HumanOutputProcessor::<std::io::Stdout, std::io::Stderr>::stdio(
                use_color,
                stdout_is_terminal,
                cli.last_message_file.clone(),
            ),
        )
    };
    let summary = runtime.block_on(async {
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = cancel_tx.send(true);
            }
        });
        run_exec_turn(
            &db,
            turn_config,
            processor.as_mut(),
            Arc::new(RealStreamFactory),
            cancel_rx,
        )
        .await
    });
    Ok(match summary.outcome {
        TurnOutcome::Complete => EXIT_OK,
        TurnOutcome::Interrupted => EXIT_INTERRUPTED,
        TurnOutcome::Failed { .. } => EXIT_ERR,
    })
}

/// 本轮 cwd：`-C` 相对当前进程目录解析；可规范化时规范（工具路径基准必须稳定）。
fn resolve_cwd(raw: Option<&Path>) -> Result<PathBuf, String> {
    let cwd = match raw {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => std::env::current_dir()
            .map_err(|error| format!("failed to read the current directory: {error}"))?
            .join(path),
        None => std::env::current_dir()
            .map_err(|error| format!("failed to read the current directory: {error}"))?,
    };
    if !cwd.is_dir() {
        return Err(format!(
            "working directory does not exist: {}",
            cwd.display()
        ));
    }
    Ok(std::fs::canonicalize(&cwd).unwrap_or(cwd))
}

/// 一次读尽 stdin（管道提示词或 `<stdin>` 上下文）。
fn read_stdin() -> std::io::Result<String> {
    use std::io::Read;
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
}

/// 读取模型配置；文件缺失时返回空设置，由模型解析给出可读错误。
async fn load_ai_settings() -> Result<crate::config::ai::AiSettings, Box<dyn std::error::Error>> {
    let path = crate::config::ai::ai_config_path()?;
    crate::config::ai::oauth::ensure_connect_oauth_fresh(&path).await;
    Ok(crate::config::ai::connect::load_runtime_ai(&path)?
        .unwrap_or_else(crate::config::ai::AiSettings::empty))
}

/// 联网搜索密钥：与 TUI 同源读取 `ai.json`；文件缺失时使用内置默认值。
fn web_search_keys() -> crate::config::ai::WebSearchConfig {
    crate::config::ai::ai_config_path()
        .ok()
        .and_then(|path| crate::config::ai::load_ai(&path).ok().flatten())
        .map(|settings| settings.web_search.normalized())
        .unwrap_or_default()
}

/// 会话级跨进程执行权；被另一个 lx 进程占用时给出可读错误。
fn lock_session(session_id: &str) -> Result<crate::platform::FileGuard, String> {
    crate::platform::try_lock_session(session_id)
        .map_err(|error| format!("failed to lock session '{session_id}': {error}"))?
        .ok_or_else(|| format!("session '{session_id}' is currently active in another lx process"))
}

/// 会话最后一条 assistant 消息的模型快照（恢复会话时作为 `-m` 的次级优先级）。
fn last_assistant_model(
    db: &Database,
    workspace: &Path,
    session_id: &str,
) -> Result<Option<ModelSnapshot>, String> {
    let messages = db
        .load_chat_session(workspace, session_id)
        .map_err(|error| format!("database error: {error}"))?;
    Ok(messages.and_then(|messages| {
        messages
            .into_iter()
            .rev()
            .find(|message| message.role == "assistant")
            .and_then(|message| {
                Some(ModelSnapshot {
                    provider: message.model_provider?,
                    model: message.model_id?,
                    variant: message.variant,
                })
            })
    }))
}

/// 恢复会话时模型发生变化则写入切换时间线（先于新 user 消息落库）。
#[allow(clippy::too_many_arguments)]
fn record_model_switch_if_changed(
    db: &Database,
    workspace: &Path,
    cwd: &Path,
    session_id: &str,
    model: &ExecModelTarget,
    messages: &[crate::db::chat::ChatMessage],
    has_instructions: bool,
) -> Result<(), String> {
    let Some(from) = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .and_then(previous_snapshot)
    else {
        return Ok(());
    };
    let to = model.snapshot();
    if from == to {
        return Ok(());
    }
    let from_label = snapshot_label(&from);
    let identity = model_identity(model);
    let updated_sources =
        crate::chat::agent::compute_updated_sources(Some(&from), &to, has_instructions);
    let injected_prompt = crate::chat::agent::render_model_switch_update(
        Some(&from_label),
        &identity,
        &updated_sources,
        &[],
    );
    db.record_model_switch(
        workspace,
        &ModelSwitchCommit {
            session_id: session_id.to_string(),
            model: to,
            from_label: Some(from_label),
            to_label: identity.label_with_variant(),
            updated_sources,
            injected_prompt,
            cwd: Some(cwd.to_path_buf()),
            to_identity: identity,
        },
    )
    .map_err(|error| format!("database error: {error}"))?;
    Ok(())
}

fn previous_snapshot(message: &crate::db::chat::ChatMessage) -> Option<ModelSnapshot> {
    Some(ModelSnapshot {
        provider: message.model_provider.clone()?,
        model: message.model_id.clone()?,
        variant: message.variant.clone(),
    })
}

/// 历史模型没有展示名：用 `provider/model (variant)` 作为稳定标签。
fn snapshot_label(snapshot: &ModelSnapshot) -> String {
    let base = format!("{}/{}", snapshot.provider, snapshot.model);
    match snapshot.variant.as_deref().filter(|v| !v.is_empty()) {
        Some(variant) => format!("{base} ({variant})"),
        None => base,
    }
}

fn model_identity(model: &ExecModelTarget) -> crate::chat::agent::ModelIdentity {
    crate::chat::agent::ModelIdentity {
        display_name: model.display_name.clone(),
        provider_id: model.provider.clone(),
        model_id: model.model.clone(),
        variant: model.variant.clone(),
        supports_image_input: model.supports_image_input,
    }
}

/// 本 turn 冻结的 `AGENTS.md` 指令段；exec 无交互信任门，按已授权处理。
fn load_instructions(workspace: &Path, cwd: &Path) -> Option<String> {
    let user_root = crate::config::user_data_root().ok();
    crate::chat::agent::load_instructions(user_root.as_deref(), Some(workspace), Some(cwd), true)
        .render_prompt_section()
}

/// 本 turn 冻结的 skills 目录段；exec 无交互信任门，项目 roots 恒生效。
pub(crate) fn skills_catalog_prompt(workspace: &Path, subagent: bool) -> Option<String> {
    let catalog = discover_workspace_skills(workspace);
    crate::skills::render_catalog_with_omitted(&catalog.entries, subagent).0
}

/// 用户草稿里 `$skill-name` 点名的正文块（只注入当轮，不落库）。
fn skill_reference_blocks(workspace: &Path, raw_prompt: &str) -> Vec<String> {
    let catalog = discover_workspace_skills(workspace);
    let mut blocks = Vec::new();
    for token in raw_prompt.split_whitespace() {
        let Some(name) = token.strip_prefix('$').filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        }) else {
            continue;
        };
        let Some(entry) = catalog.entries.iter().find(|entry| {
            entry.enabled && entry.shadowed_by.is_none() && entry.name.eq_ignore_ascii_case(name)
        }) else {
            continue;
        };
        blocks.push(match crate::skills::load_skill_body(&entry.path) {
            Ok(body) => {
                crate::skills::render_injection(
                    &entry.name,
                    &entry.path,
                    &body,
                    crate::skills::MAX_SKILL_INJECTION_BYTES,
                )
                .0
            }
            Err(error) => format!("[skill {} could not be loaded: {error}]", entry.name),
        });
    }
    blocks
}

fn discover_workspace_skills(workspace: &Path) -> crate::skills::SkillCatalog {
    use crate::config::skills as cfg;
    let (disabled, enabled) = cfg::skills_config_path()
        .ok()
        .and_then(|path| cfg::load_skills(&path).ok().flatten())
        .map(|file| (file.disabled_set(), file.enabled_set()))
        .unwrap_or_default();
    let disabled: std::collections::HashSet<PathBuf> = disabled.into_iter().collect();
    let enabled: std::collections::HashSet<PathBuf> = enabled.into_iter().collect();
    let (Ok(user_root), Ok(user_agents)) =
        (cfg::user_skills_root(), cfg::user_agents_skills_root())
    else {
        return crate::skills::SkillCatalog::default();
    };
    let project = cfg::project_skills_root(workspace)
        .ok()
        .map(|lx| crate::skills::ProjectSkillRoots::for_project(workspace, lx));
    crate::skills::discover(
        &user_root,
        &user_agents,
        project.as_ref(),
        &disabled,
        &enabled,
    )
}

fn mcp_snapshot_for_turn(_db: &Database, workspace: &Path) -> Arc<crate::mcp::McpToolSnapshot> {
    let manager = crate::mcp::global_manager();
    manager.prewarm(workspace.to_path_buf(), effective_mcp_servers(workspace));
    manager.cached_snapshot(workspace)
}

async fn mcp_snapshot_for_turn_async(
    _db: &Database,
    workspace: &Path,
    disabled_mcp: &std::collections::BTreeSet<String>,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Arc<crate::mcp::McpToolSnapshot> {
    let manager = crate::mcp::global_manager();
    let servers: Vec<_> = effective_mcp_servers(workspace)
        .into_iter()
        .filter(|server| !disabled_mcp.contains(&server.name))
        .collect();
    let snapshot = if servers.iter().any(|server| server.config.enabled) {
        Arc::new(manager.snapshot_for(workspace, &servers, cancel).await)
    } else {
        manager.cached_snapshot(workspace)
    };
    snapshot.without_servers(disabled_mcp)
}

fn effective_mcp_servers(workspace: &Path) -> Vec<crate::config::mcp::EffectiveServer> {
    use crate::config::mcp::{effective_servers, load_mcp, project_mcp_path, user_mcp_path};
    let load = |path: Option<PathBuf>| path.and_then(|path| load_mcp(&path).ok().flatten());
    let user = load(user_mcp_path().ok());
    let project = load(project_mcp_path(workspace).ok());
    effective_servers(user.as_ref(), project.as_ref(), true, Some(workspace))
}

fn load_draft_images(
    cwd: &Path,
    image_paths: &[PathBuf],
    model: &ExecModelTarget,
) -> Result<Vec<DraftImageAttachment>, String> {
    if image_paths.is_empty() {
        return Ok(Vec::new());
    }
    if !model.supports_image_input {
        return Err("Current model does not support image input.".to_string());
    }
    image_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            // `join` 对绝对路径直接取后者，相对路径以本轮 `-C` 为基准。
            let resolved = cwd.join(path);
            crate::platform::load_and_process_image_file(&resolved)
                .map(|image| DraftImageAttachment {
                    id: format!("exec-image-{index}"),
                    image,
                })
                .map_err(|error| {
                    format!(
                        "failed to load image {}: {}",
                        resolved.display(),
                        error.code()
                    )
                })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn hydrate_turn_images(
    db: &Database,
    workspace: &Path,
    session_id: &str,
    supports_image_input: bool,
    attachments: &[(usize, Vec<context::ItemImageAttachment>)],
    model_images: &[ModelImage],
    items: &mut [ModelItem],
) {
    if attachments.is_empty() {
        if !model_images.is_empty()
            && let Some(ModelItem::UserContent { images, .. }) = items
                .iter_mut()
                .rev()
                .find(|item| matches!(item, ModelItem::UserContent { .. }))
        {
            images.extend_from_slice(model_images);
        }
        return;
    }
    let total_items = items.len();
    for (item_idx, atts) in attachments {
        let Some(item) = items.get_mut(*item_idx) else {
            continue;
        };
        if *item_idx + 1 == total_items {
            if let ModelItem::UserContent { images, .. } = item {
                images.extend_from_slice(model_images);
            }
            continue;
        }
        if !supports_image_input {
            if let ModelItem::UserContent { text, .. } = item {
                let mut replaced = std::mem::take(text);
                let notes: Vec<String> = atts
                    .iter()
                    .map(|att| {
                        format!(
                            "ERROR: Cannot read \"{}\" (this model does not support image input). Inform the user.",
                            att.label
                        )
                    })
                    .collect();
                if !replaced.is_empty() {
                    replaced.push('\n');
                }
                replaced.push_str(&notes.join("\n"));
                *item = ModelItem::UserText(replaced);
            }
            continue;
        }
        let mut loaded: Vec<ModelImage> = Vec::new();
        for att in atts {
            if let Ok(mut map) = db.load_image_payloads(workspace, session_id, &[att.id.as_str()])
                && let Some(bytes) = map.remove(&att.id)
                && !loaded
                    .iter()
                    .any(|existing| existing.bytes.as_slice() == bytes.as_slice())
            {
                loaded.push(ModelImage {
                    mime: att.mime.clone(),
                    bytes: Arc::new(bytes),
                });
            }
        }
        if let ModelItem::UserContent { text, images } = item {
            if loaded.is_empty() {
                *item = ModelItem::UserText(std::mem::take(text));
            } else {
                *images = loaded;
            }
        }
    }
}
