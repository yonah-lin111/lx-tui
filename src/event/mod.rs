//! 唯一事件循环：tokio select 收敛键盘、PTY 输出与渲染调度。

pub(crate) mod chat;
pub(crate) mod commands;
mod git;
pub(crate) mod input;
mod inventory;
pub(crate) mod mouse;
pub(crate) mod overlay;
mod paste;
pub(crate) mod session;
mod trust;

pub use trust::startup_trust;

#[cfg(test)]
pub(crate) use overlay::handle_overlay_click;
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
use crate::chat::process::ProcessUiEvent;
use crate::config::Config;
use crate::db::service::Persistence;
use crate::layout::PaneId;
use crate::mcp::McpEvent;
use crate::pty::PtySession;
use crate::settings::service::{McpStatusView, SettingsEvent, SettingsService};
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
    /// 首页 skill 计数：后台预扫描完成。
    SkillsLoaded {
        root: Option<PathBuf>,
        total: usize,
        errors: usize,
        entries: Vec<crate::skills::SkillEntry>,
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
    /// 子 agent 轮次终态（需要 DB，回投事件循环结算）。
    SubagentFinished {
        key: crate::app::state::RequestKey,
        reason: crate::chat::events::StopReason,
        /// 多步累计 Token 预算耗尽（子轮专用）；与普通取消区分。
        budget_exceeded: bool,
        usage: crate::db::chat::Usage,
        billing: crate::db::chat::Usage,
    },
    /// 子 agent 运行时自动压缩完成：把摘要作为 compact 检查点写入 DB 并挂到子 agent 面板。
    SubagentAutoCompact {
        key: crate::app::state::RequestKey,
        notice: crate::chat::compact::AutoCompactNotice,
        reply: Option<tokio::sync::oneshot::Sender<Result<(), crate::chat::agent_loop::HostError>>>,
    },
    /// 子 agent `wait_agent` 到期：事件层读库结算并回执。
    SubagentWaitReady {
        key: crate::app::state::RequestKey,
        targets: Vec<String>,
        timed_out: bool,
        all: bool,
        mode: crate::chat::agent::WaitMode,
        timeout_ms: u64,
        auto_continue: bool,
        reply: tokio::sync::oneshot::Sender<
            Result<crate::chat::agent::SubagentOutcome, crate::chat::agent_loop::HostError>,
        >,
    },
    /// 会话运行实例被回收（切会话 / `/new` / 关窗格）后，其进程的终态快照。
    ///
    /// 停止发生在后台任务里，而 `Persistence` 不跨线程：终态必须回投事件层落库，
    /// 否则库里永远留着 `running` 行（设计 §8「清理进程并记录终态」）。
    ProcessesReaped {
        workspace: PathBuf,
        epoch: String,
        snapshots: Vec<crate::chat::process::ProcessSnapshot>,
    },
    /// `/compact` 的后台执行结果（摘要请求 + 检查点事务）。
    CompactFinished {
        pane: PaneId,
        session_id: String,
        /// 会话 DB 工作区键（回执重载只认它）。
        workspace: PathBuf,
        /// 发起时的 operation id：迟到回执不得覆盖更新的一次执行。
        operation_id: String,
        outcome: chat::compact::CompactOutcome,
    },
    /// compact 撤销的后台执行结果（只切检查点状态与指针）。
    CompactReverted {
        pane: PaneId,
        session_id: String,
        /// 发起时冻结的会话 DB 工作区键；回执不得按当前窗格重新解析。
        workspace: PathBuf,
        /// 发起时的 operation id：迟到回执不得覆盖更新的一次执行。
        operation_id: String,
        outcome: Result<(), String>,
    },
    /// 最新轮次撤销的后台执行结果（文件恢复 + 最终事务）。
    RewindFinished {
        pane: PaneId,
        session_id: String,
        /// 发起时冻结的会话 DB 工作区键（回执重载只认它）。
        workspace: std::path::PathBuf,
        /// 发起时的 operation id：迟到回执不得覆盖更新的一次执行。
        operation_id: String,
        outcome: chat::RewindOutcome,
    },
    /// 本地命令 spawn 成功：登记进程 ID，按修订号决定是否清空草稿。
    UserShellSpawned {
        pane: PaneId,
        generation: u64,
        message_id: String,
        process_id: u64,
    },
    /// 本地命令 spawn 失败：记录失败原因，保留草稿可重试。
    UserShellSpawnFailed {
        pane: PaneId,
        generation: u64,
        message_id: String,
        code: String,
    },
    /// `lx` 会话自动标题或 Prompt 模板块 `/title` 后台生成完成。
    TitleGenerated(chat::TitleGeneratedEvent),
    /// `/connect` 快捷连接的后台结果（凭证保存、OAuth 回调）。
    Connect(chat::connect::ConnectEvent),
    /// AI Web 后台会话事件（会话绑定、历史列表、历史加载与错误提示）。
    AiWeb(chat::ai_web::AiWebEvent),
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
    // 启动恢复：在进入事件循环前按 session guard 结算遗留 journal 与旧流/工具/进程。
    // 必须在任何新轮次之前，且只处理拿得到 guard 的会话。
    if let Some(database) = persistence.database() {
        chat::rewind::recover_startup(database);
    }
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
    // MCP 状态事件：设置页状态点、lx 页 toast 与启动进度。
    let (mcp_sender, mut mcp_receiver) = mpsc::unbounded_channel::<McpEvent>();
    crate::mcp::global_manager().set_events(mcp_sender);
    // 按 (项目, server) 汇总的状态；toast 去重用同一键 + 错误文本。
    let mut mcp_statuses: std::collections::BTreeMap<(PathBuf, String), McpStatusView> =
        std::collections::BTreeMap::new();
    let mut mcp_toasted: HashSet<(PathBuf, String, String)> = HashSet::new();
    // 启动时把设置页项目表推给服务（同样逻辑在每轮循环里做增量比较）。
    if let Some(service) = settings_service.as_ref() {
        service.set_projects(chat::mcp::project_views(state, persistence));
    }
    // 模型配置：启动加载（首次已由 main 导入）；加载后刷新状态行。
    chat::connect::load_models_snapshot(state).await;
    // 预设环境变量快照：`/connect` 的 `$ENV` 状态与断开判定统一读这份内存快照。
    state.connect_env = crate::config::ai::collect_preset_env_snapshot();
    // 启动后台静默刷新云端模型目录（不阻塞首帧，也不弹 Toast）。
    state
        .effects
        .push(crate::app::effects::Effect::RefreshModelsCatalog {
            pane: None,
            provider_id: None,
        });
    state.subagents_config = crate::config::subagents::load_subagents_or_default();
    // 自定义命令重载触发：启动、切换显示工作区与设置页写入后。
    let mut commands_reload = true;
    // skill 目录缓存刷新触发：切换显示工作区与设置页写入后；启动时单独预扫描。
    let mut skills_reload = false;
    let mut last_prompt_workspace = state.prompt_workspace();
    session::initialize_layout_widths(tui, state, &config)?;
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let mut sessions: HashMap<PaneId, PtySession> = HashMap::new();
    let mut events = EventStream::new();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    // lx loading 转轮的下一帧时刻；无流中窗格时为 None（空闲不轮询）。
    let mut spinner_at: Option<Instant> = None;
    let mut last_rects: Vec<(PaneId, Rect)> = Vec::new();
    // UI 渲染缓存：跨帧复用历史几何快照；按窗格隔离，窗格集合变化时清理。
    let mut ui_cache = ui::RenderCache::default();
    // PTY 输出后的 cwd 去抖检查时刻；空闲时不设置、不轮询。
    let mut cwd_check: Option<Instant> = None;
    // PTY 输出后的 Agent 去抖扫描时刻；与 cwd 检查同窗口合并。
    let mut agent_check: Option<Instant> = None;
    // 对话框列表查询在途的仓库根；结果返回前不重复发起。
    let mut git_inflight: HashSet<PathBuf> = HashSet::new();
    // lx 聊天在途流的取消句柄 + 会话进程运行实例。
    let mut chat_streams = chat::ChatStreams::default();
    let connect_tasks = chat::connect::ConnectTasks::default();
    // 进程 UI 事件走独立通道：它们不携带请求代次，只按进程会话 ID 过滤归属。
    let (process_sender, mut process_receiver) = mpsc::unbounded_channel::<ProcessUiEvent>();
    chat_streams.set_process_sender(process_sender);
    chat_streams.set_event_sender(sender.clone());
    commands::seed_git_requests(state);
    // 进入应用即装载资源：AGENTS.md 计数、MCP 后台预热、skill 目录后台扫描。
    let mut mcp_roots = chat::mcp::project_roots(state);
    let mut skill_roots = inventory::skill_roots(state);
    let mut agents_scopes = inventory::agents_md_scopes(state, persistence);
    inventory::refresh_agents_md(state, persistence);
    inventory::initialize_mcp(state, persistence, &mcp_roots);
    inventory::spawn_skills_preload(state, persistence, &sender);

    while !state.should_quit {
        // 会话按状态对齐：新建工作区的窗格在此启动，被移除工作区的会话在此终止。
        session::reconcile_sessions(state, &mut sessions, &sender);
        // 新打开的工作区或 GitRefreshed 解析出的新仓库根也要立即预热 MCP 与 Skills。
        let roots = chat::mcp::project_roots(state);
        if roots != mcp_roots {
            mcp_roots = roots.clone();
            inventory::initialize_mcp(state, persistence, &mcp_roots);
        }
        let current_skill_roots = inventory::skill_roots(state);
        if current_skill_roots != skill_roots {
            skill_roots = current_skill_roots;
            skills_reload = true;
        }
        // 设置页项目表：工作区打开/关闭/信任变化都可能改变 scope 映射。
        if let Some(service) = settings_service.as_ref() {
            service.set_projects(chat::mcp::project_views(state, persistence));
        }
        if inventory::drain_settings_events(
            &mut settings_receiver,
            state,
            persistence,
            &mut config,
            &mut commands_reload,
            &mut skills_reload,
            &mut mcp_statuses,
        ) {
            dirty = true;
        }
        if inventory::drain_mcp_events(
            &mut mcp_receiver,
            state,
            settings_service.as_ref(),
            &mut mcp_statuses,
            &mut mcp_toasted,
        ) {
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
            skills_reload = true;
            inventory::sync_prompt_mcp_mentions(state, persistence);
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
        if skills_reload {
            skills_reload = false;
            agents_scopes = inventory::agents_md_scopes(state, persistence);
            inventory::refresh_agents_md(state, persistence);
            inventory::spawn_skills_preload(state, persistence, &sender);
        }
        // 撤销、压缩、续轮 session guard、`/connect`、`/mcp` 与持久化泵效应按序执行。
        dirty |= chat::apply_rewind_effects(state, &sender, persistence);
        dirty |= chat::apply_compact_effects(state, &sender, persistence);
        dirty |= chat::guard_begin_turns(state, &mut chat_streams, persistence);
        dirty |= chat::connect::apply_effects(state, &connect_tasks, &sender);
        dirty |= inventory::apply_mcp_effects(state, persistence, &mut mcp_statuses);
        dirty |= persistence.pump(state);
        // 工作区集合、cwd（含 linked worktree / 子目录）或信任状态变化时精准刷新 AGENTS.md 与项目级 MCP。
        let current_agents_scopes = inventory::agents_md_scopes(state, persistence);
        if current_agents_scopes != agents_scopes {
            agents_scopes = current_agents_scopes;
            inventory::refresh_agents_md(state, persistence);
            let roots = chat::mcp::project_roots(state);
            inventory::prune_mcp_statuses(persistence, &roots, &mut mcp_statuses);
            inventory::refresh_mcp(state, persistence, &roots, &mcp_statuses);
            dirty = true;
        }
        // 双 Esc 待命超时：3 秒窗口外自动失效（不产生历史记录）。
        if update::clear_expired_arms(state) {
            dirty = true;
        }
        // 聊天网络效应：取走并在 Tokio 任务执行（结果按 key 回投）。
        if chat::pump_chat_streams(state, &mut chat_streams, &sender, persistence) {
            dirty = true;
        }
        // 后台标题生成效应：轻量无工具流转入后台任务。
        if chat::apply_title_effects(state, &sender) {
            dirty = true;
        }
        if chat::prune_pending_guards(state, &mut chat_streams) {
            dirty = true;
        }
        // 本地命令：DB 预登记成功后才 spawn；与模型轮次并行。
        if chat::pump_user_shells(state, &mut chat_streams, &sender, persistence) {
            dirty = true;
        }
        // 后台任务页的列表与停止：进程句柄只在运行实例里，由事件层执行。
        if chat::apply_process_effects(state, &mut chat_streams) {
            dirty = true;
        }
        // 子 agent 面板的列表、懒加载与停止。
        if chat::apply_subagent_effects(state, &mut chat_streams, persistence) {
            dirty = true;
        }
        // 底部状态行的后台任务计数：与页面是否打开无关，每轮同步。
        if chat::sync_background_task_counts(state, &chat_streams) {
            dirty = true;
        }
        // 面板打开期间每轮刷新进程列表：打开之后才启动的进程也要能出现。
        //
        // 这里**不置脏**：返回值的语义是「已压入请求」，不是「画面需要变」。
        // 早先把它当变化用，导致面板一打开就每帧 dirty、空闲也按 60fps 重绘
        // 并反复克隆全部进程快照（用户观感：停在后台任务页风扇就转）。
        // 真正的重绘由 `apply_process_effects` / `tick_task_pages` 的实际变化驱动。
        chat::refresh_open_task_pages(state);
        // 后台任务面板的运行时长节拍：`now_ms` 只在列表回投时更新会冻住时长。
        if chat::tick_task_pages(state) {
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
        // 资源计数按项目集中维护：每帧同步到所有窗格，新分割/新打开的工作区
        // 不会显示成 0。
        if state.sync_lx_inventory() {
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
        // 历史列表的滚动与选中项：折叠 Agents / 终端变高后夹取偏移，
        // 并在进入/新建/重载后把选中项滚入可见区（渲染层只读，不能自行修正）。
        if let Some(rows) = ui::workspace_history_visible_rows(&geometry.view, state) {
            let mut changed = state.workspace_history.clamp(rows);
            if state.workspace_history.reveal_selected {
                state.workspace_history.scroll_selected_into_view(rows);
                state.workspace_history.reveal_selected = false;
                changed = true;
            }
            if changed {
                dirty = true;
            }
        }
        if geometry.rects != last_rects || state.prompt_resize_pending {
            update::resize_panes(state, &geometry.rects);
            state.prompt_resize_pending = false;
            // 窗格集合变化：清掉已关闭窗格的历史缓存，避免泄漏。
            let alive: Vec<PaneId> = geometry.rects.iter().map(|(id, _)| *id).collect();
            ui_cache.retain_panes(&alive);
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
        // lx loading 转轮：流中/压缩中每 `LX_LOADING_FRAME_MS` 推进一帧
        // （前者推扫描帧，后者还刷新本次压缩耗时；无流、无压缩时不计时）。
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
            // 双击 Esc 选中的最新用户消息：先按几何滚到视口居中，再算置顶/顶部行。
            // 本帧紧接着绘制，无需再次置脏。
            update::resolve_lx_rewind_center(state, &geometry.rects, &mut ui_cache);
            // 视口顶部行与置顶抑制记忆：一次几何计算同时落两处状态（渲染层只读）。
            update::resolve_lx_history(state, &geometry.rects, &mut ui_cache);
            // 后台任务面板的输出可见行数：同上，供翻页与滚动条几何钳制。
            update::resolve_task_output_visible(state, &geometry.rects);
            // Agents 详情区可见行数：与上一条同款。
            update::resolve_task_agents_visible(state, &geometry.rects);
            // ask 统一列表窗口：同上，供 PageUp/PageDown 钳制滚动。
            update::resolve_ask_list_window(state, &geometry.rects);
            // Todo 列表可见行数：本帧紧接着绘制，钳制结果直接生效，无需再置脏。
            update::resolve_todo_viewport(state, geometry.view.prompt);
            ui_cache.begin_frame();
            tui.draw(|frame| ui::render_cached(frame, state, &config, &mut ui_cache))?;
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
                    handle_terminal_event(
                        event,
                        state,
                        &mut sessions,
                        &geometry,
                        &config,
                        &mut ui_cache,
                        &mut dirty,
                    );
                    pump_mention_scan(state, &sender);
                    pump_lx_mention_scan(state, &sender);
                    commands::pump_prompt_worktree(state, &sender, &mut git_inflight);
                    git::pump_git_queries(state, &sender, &mut git_inflight);
                    commands::pump_referenced_requests(state, &sender);
                }
                Some(Err(error)) => return Err(error),
                None => break,
            },
            process_event = process_receiver.recv() => {
                if let Some(event) = process_event {
                    chat::handle_process_event(state, &mut chat_streams, persistence, event);
                    // 后台输出/退出一定意味着需要重绘；批量排空同一帧的剩余事件。
                    dirty = true;
                    while let Ok(event) = process_receiver.try_recv() {
                        chat::handle_process_event(state, &mut chat_streams, persistence, event);
                    }
                }
            }
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
    // 退出 lx：停止并 reap 全部会话进程（有界等待，未确认的保留 Interrupted 记录）。
    let runtimes: Vec<String> = chat_streams.runtimes.sessions();
    for session in runtimes {
        if let Some(runtime) = chat_streams.runtimes.take(&session) {
            let manager = runtime.manager().clone();
            let workspace = manager.owner().workspace.clone();
            let epoch = runtime.epoch.clone();
            let shutdown = tokio::spawn(async move {
                manager.shutdown().await;
                manager.snapshots()
            });
            // 单批最长 3 秒：不为持久化失败无限挂住退出。
            if let Ok(Ok(snapshots)) =
                tokio::time::timeout(std::time::Duration::from_secs(3), shutdown).await
            {
                // 停止后落库终态：否则下次启动这些行仍是 running，
                // 卡片会显示 Running 但没有任何进程可停（设计 §8）。
                chat::persist_terminal_snapshots(persistence, &workspace, &epoch, &snapshots);
                // 用户命令记录同样收尾（保留已产生输出，标记 interrupted/stopped）。
                chat::user_shell::persist_reaped(
                    persistence,
                    &mut chat_streams.user_shell,
                    &snapshots,
                );
            }
        }
    }
    // MCP 子进程随退出关闭：先取消，2 秒兜底后整组 SIGKILL（见 manager::shutdown）。
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        crate::mcp::global_manager().shutdown(),
    )
    .await;
    // 设置服务随 TUI 退出关闭，避免回环端口悬挂。
    if let Some(service) = settings_service.as_ref() {
        service.shutdown();
    }
    // 退出前收尾：提交编辑中的提示词内容并落库。
    persistence.shutdown(state);
    Ok(())
}

fn handle_terminal_event(
    event: TerminalEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &session::Geometry,
    config: &Config,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    match event {
        TerminalEvent::Key(key) => {
            input::handle_key_event(key, state, sessions, geometry, cache, dirty);
        }
        TerminalEvent::Paste(text) => {
            input::handle_paste_event(text, state, sessions, dirty);
        }
        TerminalEvent::Mouse(mouse) => {
            mouse::handle_mouse_event(mouse, state, sessions, geometry, config, cache, dirty);
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
        AppEvent::SkillsLoaded {
            root,
            total,
            errors,
            entries,
        } => inventory::apply_skills_loaded(state, root, total, errors, &entries),
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
            chat::handle_net_event(state, chat_streams, persistence, event);
        }
        AppEvent::AgentHost(event) => {
            // 工具调用握手：DB 落库门槛 → 状态 → 回执 ticket（顺序不可交换）。
            chat::handle_agent_host_event(state, chat_streams, persistence, event);
        }
        AppEvent::SubagentFinished {
            key,
            reason,
            budget_exceeded,
            usage,
            billing,
        } => {
            chat::finish_subagent_with_billing(
                state,
                chat_streams,
                persistence,
                key,
                reason,
                budget_exceeded,
                usage,
                billing,
            );
        }
        AppEvent::SubagentAutoCompact { key, notice, reply } => {
            chat::handle_subagent_auto_compact(
                state,
                chat_streams,
                persistence,
                &key,
                &notice,
                reply,
            );
        }
        AppEvent::SubagentWaitReady {
            key,
            targets,
            timed_out,
            all,
            mode,
            timeout_ms,
            auto_continue,
            reply,
        } => {
            chat::handle_subagent_wait_ready(
                chat_streams,
                persistence,
                key,
                targets,
                timed_out,
                all,
                mode,
                timeout_ms,
                auto_continue,
                reply,
            );
        }
        AppEvent::CompactFinished {
            pane,
            session_id,
            workspace,
            operation_id,
            outcome,
        } => {
            chat::handle_compact_finished(
                state,
                persistence,
                pane,
                &session_id,
                &workspace,
                &operation_id,
                outcome,
            );
        }
        AppEvent::CompactReverted {
            pane,
            session_id,
            workspace,
            operation_id,
            outcome,
        } => {
            chat::handle_compact_reverted(
                state,
                persistence,
                pane,
                &session_id,
                &workspace,
                &operation_id,
                outcome,
            );
        }
        AppEvent::TitleGenerated(event) => {
            chat::handle_title_generated(state, persistence, event);
        }
        AppEvent::RewindFinished {
            pane,
            session_id,
            workspace,
            operation_id,
            outcome,
        } => {
            chat::handle_rewind_finished(
                state,
                persistence,
                pane,
                &session_id,
                &workspace,
                &operation_id,
                outcome,
            );
        }
        AppEvent::Connect(event) => {
            chat::connect::handle_event(state, event);
        }
        AppEvent::AiWeb(event) => {
            chat::ai_web::handle_event(state, event);
        }
        AppEvent::ProcessesReaped {
            workspace,
            epoch,
            snapshots,
        } => {
            // 会话运行实例回收后的终态落库：只写终态，活进程快照一律跳过。
            chat::persist_terminal_snapshots(persistence, &workspace, &epoch, &snapshots);
            chat::user_shell::persist_reaped(persistence, &mut chat_streams.user_shell, &snapshots);
        }
        AppEvent::UserShellSpawned {
            pane,
            generation,
            message_id,
            process_id,
        } => {
            chat::user_shell::handle_spawned(
                state,
                chat_streams,
                persistence,
                pane,
                generation,
                &message_id,
                process_id,
            );
        }
        AppEvent::UserShellSpawnFailed {
            pane,
            generation,
            message_id,
            code,
        } => {
            chat::user_shell::handle_spawn_failed(
                state,
                chat_streams,
                persistence,
                pane,
                generation,
                &message_id,
                &code,
            );
        }
        AppEvent::PromptWorktreeListed {
            repo_root,
            candidates,
            failed,
        } => {
            git_inflight.remove(&repo_root);
            // 面板不可交互时丢弃结果：`apply_worktree_entries` 结尾会无条件
            // `mention.refresh`，而 `clear()` 只 bump generation、不清 cache，
            // 于是 Todo 期间到达的回调会把刚收起的提及面板重新置 Some——
            // 一个看不见、却仍能吞滚轮的面板。
            if !update::prompt_panel_active(state) {
                return;
            }
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

#[cfg(test)]
mod tests;
