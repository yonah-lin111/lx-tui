//! 唯一事件循环：tokio select 收敛键盘、PTY 输出与渲染调度。

pub(crate) mod chat;
pub(crate) mod commands;
mod git;
pub(crate) mod input;
pub(crate) mod mouse;
pub(crate) mod overlay;
mod paste;
pub(crate) mod session;
mod trust;

pub use trust::startup_trust;

#[cfg(test)]
pub(crate) use overlay::handle_overlay_click;
#[cfg(test)]
pub(crate) use session::Geometry;
#[cfg(test)]
pub(crate) use session::poll_active_agents;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event as TerminalEvent, EventStream};
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::app::markdown::MentionEntry;
use crate::app::state::{AppState, WorkspaceGit};
use crate::app::update;
use crate::config::Config;
use crate::db::service::Persistence;
use crate::layout::PaneId;
use crate::pty::PtySession;
use crate::settings::service::{SettingsEvent, SettingsService};
use crate::tui::Tui;
use crate::ui;

/// 应用级事件：后台任务（PTY 读线程、提及扫描与 git 查询）经此汇入主循环。
#[derive(Debug)]
pub enum AppEvent {
    PaneOutput(PaneId, Vec<u8>),
    PaneExit(PaneId),
    MentionScanned {
        /// 发起扫描的 prompt 编辑器标识；结果按此路由到显示中或挂起的草稿。
        prompt: PaneId,
        generation: u64,
        entries: Vec<MentionEntry>,
    },
    /// lx 输入框 `@` 面板的目录扫描结果；按窗格、代号与根路由。
    LxMentionScanned {
        pane: PaneId,
        generation: u64,
        root: PathBuf,
        entries: Vec<MentionEntry>,
    },
    /// 工作区 git 元数据查询结果。
    GitRefreshed {
        cwd: PathBuf,
        checkout: Option<WorkspaceGit>,
    },
    /// 信任检查的仓库根解析结果；主循环以其重新入队信任检查。
    TrustRootResolved {
        path: PathBuf,
        root: PathBuf,
        trigger: crate::app::trust::TrustTrigger,
    },
    /// worktree 对话框列表查询结果。
    WorktreeListed {
        repo_root: PathBuf,
        entries: Vec<crate::git::WorktreeEntry>,
        failed: bool,
    },
    /// prompt 模板块绑定用的 worktree 候选查询结果。
    PromptWorktreeListed {
        repo_root: PathBuf,
        candidates: Vec<crate::app::markdown::WorktreeCandidate>,
        failed: bool,
    },
    /// 引用文件夹内容扫描结果。
    ReferencedScanned {
        id: u64,
        outcome: Result<crate::app::referenced::ReferencedScanOutcome, ()>,
    },
    /// 引用文件夹新增路径后台校验结果：规范化绝对路径或失败原因（含重复）。
    ReferencedValidated {
        id: u64,
        outcome: Result<PathBuf, crate::app::referenced::PathValidation>,
    },
    /// lx 聊天网络流事件（delta / finished）；按 request key 过滤。
    ChatNet(chat::ChatNetEvent),
    /// lx agent loop 的工具调用握手（pending/running/结果落库 ack）。
    ///
    /// 事件层在 **DB 与状态都就绪后**才回执，因此 loop 拿到 ticket 才执行文件 IO。
    AgentHost(chat::AgentHostEvent),
}

/// 最小帧间隔；PTY 洪峰经此合并。
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// 空闲等待：只由输入或 PTY 输出唤醒。
const IDLE_WAIT: Duration = Duration::from_secs(3600);

/// lx loading 转轮每帧间隔（对齐 codex motion.rs 的 100ms）。
const LOADING_FRAME_INTERVAL: Duration = Duration::from_millis(100);

/// 同步入口：构造 tokio 运行时并阻塞到主循环结束；持久化服务由入口构造并移交。
pub fn run(
    tui: &mut Tui,
    state: &mut AppState,
    config: &Config,
    mut persistence: Persistence,
) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_loop(tui, state, config, &mut persistence))
}

async fn run_loop(
    tui: &mut Tui,
    state: &mut AppState,
    config: &Config,
    persistence: &mut Persistence,
) -> io::Result<()> {
    // 运行时配置副本：设置页保存后经事件更新，渲染每帧读取。
    let mut config = config.clone();
    // 本地设置服务：仅回环监听；失败只影响设置页，不阻断 TUI。
    let (settings_sender, mut settings_receiver) = mpsc::unbounded_channel::<SettingsEvent>();
    let settings_service = match SettingsService::start(settings_sender).await {
        Ok(service) => Some(service),
        Err(error) => {
            tracing::warn!(%error, "settings web service failed to start");
            None
        }
    };
    state.settings_available = settings_service.is_some();
    // 模型配置：启动加载（首次已由 main 导入）；加载后刷新状态行。
    load_models_snapshot(state);
    // 自定义命令重载触发：启动、切换显示工作区与设置页写入后。
    let mut commands_reload = true;
    let mut last_prompt_workspace = state.prompt_workspace();
    initialize_layout_widths(tui, state, &config)?;
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let mut sessions: HashMap<PaneId, PtySession> = HashMap::new();
    let mut events = EventStream::new();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    // lx loading 转轮的下一帧时刻；无流中窗格时为 None（空闲不轮询）。
    let mut spinner_at: Option<Instant> = None;
    let mut last_rects: Vec<(PaneId, Rect)> = Vec::new();
    // PTY 输出后的 cwd 去抖检查时刻；空闲时不设置、不轮询。
    let mut cwd_check: Option<Instant> = None;
    // PTY 输出后的 Agent 去抖扫描时刻；与 cwd 检查同窗口合并。
    let mut agent_check: Option<Instant> = None;
    // 对话框列表查询在途的仓库根；结果返回前不重复发起。
    let mut git_inflight: HashSet<PathBuf> = HashSet::new();
    // lx 聊天在途流的取消句柄。
    let mut chat_streams = chat::ChatStreams::default();
    commands::seed_git_requests(state);

    while !state.should_quit {
        // 会话按状态对齐：新建工作区的窗格在此启动，被移除工作区的会话在此终止。
        session::reconcile_sessions(state, &mut sessions, &sender);
        // 设置页事件：保存的配置与命令变更在事件循环内应用。
        while let Ok(event) = settings_receiver.try_recv() {
            match event {
                SettingsEvent::ConfigChanged(updated) => config = updated,
                SettingsEvent::CommandsChanged => commands_reload = true,
                SettingsEvent::ModelsChanged(settings) => {
                    // 模型配置已落盘：更新快照；在途请求继续持有旧快照到结束，
                    // 下一轮发送才用新配置。
                    update::apply_models_changed(state, &settings);
                }
            }
            dirty = true;
        }
        // 设置图标点击：请求打开浏览器（后台执行，不阻塞事件循环）。
        if state.settings_open_requested {
            state.settings_open_requested = false;
            if let Some(url) = settings_service.as_ref().map(SettingsService::url) {
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = crate::platform::open_browser(&url) {
                        tracing::warn!(%error, "opening the settings page failed");
                    }
                });
            } else {
                tracing::warn!("settings web service is unavailable");
            }
        }
        // 显示工作区变化或设置页写入后重载自定义命令；git 元数据到达后同步仓库根。
        let prompt_workspace = state.prompt_workspace();
        // `/resume` 编辑模态归属失效（切工作区/切焦点窗格）即作废，丢弃未保存标题。
        if update::discard_stale_resume_edit(state) {
            dirty = true;
        }
        if prompt_workspace != last_prompt_workspace {
            last_prompt_workspace = prompt_workspace;
            commands_reload = true;
        }
        let repo_changed = commands::sync_prompt_repo(state);
        if commands::reload_custom_commands(state, commands_reload) || repo_changed {
            dirty = true;
            // 命令集变化时刷新设置页快照（避免每帧克隆全量命令）。
            if let Some(service) = settings_service.as_ref() {
                service.set_commands_snapshot(&commands::command_views(state));
            }
        }
        commands_reload = false;
        // 执行 app 层效应出箱（信任检查/授予、提示词 CRUD 与剪贴板）并载入提示词树。
        if persistence.pump(state) {
            dirty = true;
        }
        // 聊天网络效应：取走并在 Tokio 任务执行（结果按 key 回投）。
        if chat::pump_chat_streams(state, &mut chat_streams, &sender) {
            dirty = true;
        }
        if cwd_check.is_some_and(|at| Instant::now() >= at) {
            cwd_check = None;
            if session::poll_process_cwds(state, &sessions) {
                dirty = true;
            }
        }
        if agent_check.is_some_and(|at| Instant::now() >= at) {
            agent_check = None;
            if session::poll_active_agents(state, &sessions) {
                dirty = true;
            }
        }
        // lx `@` 面板有效根随窗格 cwd、pane 生命周期与工作区切换同步；变化即重算面板。
        if update::sync_all_lx_mention_roots(state) {
            dirty = true;
        }
        pump_lx_mention_scan(state, &sender);
        git::pump_git_queries(state, &sender, &mut git_inflight);
        let geometry = session::current_geometry(tui, state, &config)?;
        // 工具调用 loading 门槛按「窗格本帧是否可见」决定是否等一帧；每帧都刷新，
        // 避免首帧（几何未变化）误判为不可见而跳过渲染门槛。
        chat_streams.set_visible_panes(&geometry.rects);
        if let Some(rows) = ui::agent_list_rows(&geometry.view, state)
            && update::clamp_agent_scroll(state, rows)
        {
            dirty = true;
        }
        if geometry.rects != last_rects {
            update::resize_panes(state, &geometry.rects);
            // 几何变化使自动滚动登记的坐标失效，停止拖拽滚动。
            update::stop_selection_autoscroll(state);
            session::resize_sessions(&mut sessions, &geometry.rects);
            last_rects.clone_from(&geometry.rects);
            if let Some(rows) = ui::workspace_list_rows(&geometry.view, state) {
                update::clamp_workspace_scroll(state, rows);
            }
            let tab_bar =
                ui::tab_bar::layout(&geometry.view, state.active_workspace(), state.tab_scroll);
            update::set_tab_scroll(state, state.tab_scroll, tab_bar.max_scroll);
            dirty = true;
        }
        if update::tick(state, Instant::now()) {
            dirty = true;
        }
        // lx loading 转轮：流中每 `LX_LOADING_FRAME_MS` 推进一帧（无流时不计时）。
        let streaming = state.has_streaming_lx();
        if streaming {
            let at = spinner_at.get_or_insert_with(|| Instant::now() + LOADING_FRAME_INTERVAL);
            if Instant::now() >= *at {
                spinner_at = Some(*at + LOADING_FRAME_INTERVAL);
                if update::advance_lx_loading(state) {
                    dirty = true;
                }
            }
        } else {
            spinner_at = None;
        }

        if dirty && last_draw.elapsed() >= FRAME_INTERVAL {
            // 置顶用户消息行结论：渲染前落状态（渲染层只读，不写状态）。
            update::resolve_lx_sticky_prompt(state, &geometry.rects);
            tui.draw(|frame| ui::render(frame, state, &config))?;
            dirty = false;
            last_draw = Instant::now();
            // 工具调用 loading 已绘制：推进 pending → running 并回执 ticket，
            // loop 此刻才被允许执行文件 IO（不靠 sleep 保证可见性）。
            if chat::flush_pending_tool_begins(state, &mut chat_streams, persistence) {
                dirty = true;
            }
        }

        // 待执行效应（如自动保存落库）必须尽快进入下一轮 pump，不随空闲睡死。
        let mut wait = if dirty || !state.effects.is_empty() {
            FRAME_INTERVAL.saturating_sub(last_draw.elapsed())
        } else {
            IDLE_WAIT
        };
        if let Some(deadline) = update::next_deadline(state) {
            wait = wait.min(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(at) = spinner_at {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }
        if let Some(at) = cwd_check {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }
        if let Some(at) = agent_check {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => {
                    handle_terminal_event(event, state, &mut sessions, &geometry, &config, &mut dirty);
                    pump_mention_scan(state, &sender);
                    pump_lx_mention_scan(state, &sender);
                    pump_prompt_worktree(state, &sender, &mut git_inflight);
                    git::pump_git_queries(state, &sender, &mut git_inflight);
                    pump_referenced_requests(state, &sender);
                }
                Some(Err(error)) => return Err(error),
                None => break,
            },
            message = receiver.recv() => {
                if let Some(message) = message {
                    handle_app_event(
                        message,
                        state,
                        &mut sessions,
                        &mut git_inflight,
                        &mut chat_streams,
                        persistence,
                    );
                    while let Ok(message) = receiver.try_recv() {
                        handle_app_event(
                            message,
                            state,
                            &mut sessions,
                            &mut git_inflight,
                            &mut chat_streams,
                            persistence,
                        );
                    }
                    // 输出后安排一次去抖检查；窗口内合并，空闲不轮询。
                    cwd_check.get_or_insert_with(|| Instant::now() + session::CWD_CHECK_DELAY);
                    agent_check.get_or_insert_with(|| Instant::now() + session::AGENT_CHECK_DELAY);
                    dirty = true;
                }
            }
            () = tokio::time::sleep(wait) => {}
        }
    }

    for session in sessions.values_mut() {
        session.kill();
    }
    // 设置服务随 TUI 退出关闭，避免回环端口悬挂。
    if let Some(service) = settings_service.as_ref() {
        service.shutdown();
    }
    // 退出前收尾：提交编辑中的提示词内容并落库。
    persistence.shutdown(state);
    Ok(())
}

/// 启动时确定栏宽：右栏取主区可用宽度的 2/5（主区更宽），左栏取配置值。
fn initialize_layout_widths(
    tui: &mut Tui,
    state: &mut AppState,
    config: &Config,
) -> io::Result<()> {
    let size = tui.terminal().size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    state.sidebar_width = config.sidebar_width;
    state.prompt_width = ui::layout::default_side_width(area, config, state.sidebar_collapsed);
    Ok(())
}

/// 启动加载模型配置快照；缺失或损坏时留空配置并刷新状态行。
fn load_models_snapshot(state: &mut AppState) {
    let path = match crate::config::ai::ai_config_path() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "ai config path unavailable");
            return;
        }
    };
    match crate::config::ai::load_ai(&path) {
        Ok(Some(settings)) => state.models.settings = settings,
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "ai config load failed"),
    }
    update::refresh_all_lx_status(state);
}

fn handle_terminal_event(
    event: TerminalEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &session::Geometry,
    config: &Config,
    dirty: &mut bool,
) {
    match event {
        TerminalEvent::Key(key) => {
            input::handle_key_event(key, state, sessions, geometry.screen, &geometry.view, dirty);
        }
        TerminalEvent::Paste(text) => {
            input::handle_paste_event(text, state, sessions, dirty);
        }
        TerminalEvent::Mouse(mouse) => {
            mouse::handle_mouse_event(mouse, state, sessions, geometry, config, dirty);
        }
        TerminalEvent::FocusGained | TerminalEvent::FocusLost | TerminalEvent::Resize(..) => {}
    }
}

fn handle_app_event(
    message: AppEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    git_inflight: &mut HashSet<PathBuf>,
    chat_streams: &mut chat::ChatStreams,
    persistence: &Persistence,
) {
    match message {
        AppEvent::PaneOutput(id, bytes) => {
            let responses = update::feed_pane(state, id, &bytes);
            if !responses.is_empty() {
                session::write_to_pane(sessions, id, &responses);
            }
        }
        AppEvent::PaneExit(id) => update::mark_pane_exited(state, id),
        AppEvent::MentionScanned {
            prompt,
            generation,
            entries,
        } => update::apply_mention_entries(state, prompt, generation, entries),
        AppEvent::LxMentionScanned {
            pane,
            generation,
            root,
            entries,
        } => update::apply_lx_mention_entries(state, pane, generation, &root, entries),
        AppEvent::GitRefreshed { cwd, checkout } => {
            update::apply_git_refresh(state, &cwd, checkout)
        }
        AppEvent::TrustRootResolved {
            path,
            root,
            trigger,
        } => state.effects.push(crate::app::effects::Effect::CheckTrust {
            path,
            root: Some(root),
            trigger,
        }),
        AppEvent::WorktreeListed {
            repo_root,
            entries,
            failed,
        } => {
            git_inflight.remove(&repo_root);
            update::apply_worktree_list(state, &repo_root, entries, failed);
        }
        AppEvent::ReferencedScanned { id, outcome } => match outcome {
            Ok(outcome) => {
                update::apply_referenced_scan(state, id, outcome);
            }
            Err(()) => {
                update::apply_referenced_scan_failure(state, id);
            }
        },
        AppEvent::ReferencedValidated { id, outcome } => match outcome {
            Ok(canonical) => {
                update::apply_referenced_validation(state, id, canonical);
            }
            Err(error) => {
                update::apply_referenced_validation_error(state, id, error);
            }
        },
        AppEvent::ChatNet(event) => {
            // 网络事件按 key 过滤后转 update；调用方随后统一置脏。
            chat::handle_net_event(state, chat_streams, event);
        }
        AppEvent::AgentHost(event) => {
            // 工具调用握手：DB 落库门槛 → 状态 → 回执 ticket（顺序不可交换）。
            chat::handle_agent_host_event(state, chat_streams, persistence, event);
        }
        AppEvent::PromptWorktreeListed {
            repo_root,
            candidates,
            failed,
        } => {
            git_inflight.remove(&repo_root);
            let targeted = state
                .prompt_owner()
                .git
                .as_ref()
                .is_some_and(|git| git.repo_root == repo_root);
            if targeted {
                if failed {
                    state.prompt.worktree_failed();
                } else if state.prompt.apply_worktree_entries(candidates) {
                    // 列表变化时重建选择器并把高亮对齐当前绑定；未变化保留用户浏览位置。
                    state.prompt.rebuild_worktree_entries();
                }
            }
        }
    }
}

/// @ 提及面板需要缓存且无在途扫描时，发起后台扫描并把结果投递回主循环。
///
/// prompt 未聚焦时不扫描；扫描失败按空结果处理（面板保持隐藏）。
fn pump_mention_scan(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) {
    if !state.prompt_focused {
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
fn pump_lx_mention_scan(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) {
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
fn pump_prompt_worktree(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    inflight: &mut HashSet<PathBuf>,
) {
    if state.prompt_collapsed {
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
fn pump_referenced_requests(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) {
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

#[cfg(test)]
mod tests;
