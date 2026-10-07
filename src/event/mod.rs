//! 唯一事件循环：tokio select 收敛键盘、PTY 输出与渲染调度。

mod git;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use crossterm::event::{
    Event as TerminalEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Direction, Rect};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::app::actions::{Action, EditorCommand, OverlayKey};
use crate::app::markdown::MentionEntry;
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneKind, PaneView, WorkspaceGit, home_dir, workspace_label};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::config::Config;
use crate::input::{self, Routed};
use crate::layout::{self, PaneId};
use crate::pty::{PtyEvent, PtySession};
use crate::terminal::WheelRouting;
use crate::tui::Tui;
use crate::ui;

/// 应用级事件：后台任务（PTY 读线程、提及扫描与 git 查询）经此汇入主循环。
#[derive(Debug)]
pub enum AppEvent {
    PaneOutput(PaneId, Vec<u8>),
    PaneExit(PaneId),
    MentionScanned {
        generation: u64,
        entries: Vec<MentionEntry>,
    },
    /// 工作区 git 元数据查询结果。
    GitRefreshed {
        cwd: PathBuf,
        checkout: Option<WorkspaceGit>,
    },
    /// worktree 对话框列表查询结果。
    WorktreeListed {
        repo_root: PathBuf,
        entries: Vec<crate::git::WorktreeEntry>,
        failed: bool,
    },
}

/// 最小帧间隔；PTY 洪峰经此合并。
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// 空闲等待：只由输入或 PTY 输出唤醒。
const IDLE_WAIT: Duration = Duration::from_secs(3600);

/// 同步入口：构造 tokio 运行时并阻塞到主循环结束。
pub fn run(tui: &mut Tui, state: &mut AppState, config: &Config) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_loop(tui, state, config))
}

async fn run_loop(tui: &mut Tui, state: &mut AppState, config: &Config) -> io::Result<()> {
    initialize_layout_widths(tui, state, config)?;
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let mut sessions: HashMap<PaneId, PtySession> = HashMap::new();
    let mut events = EventStream::new();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    let mut last_rects: Vec<(PaneId, Rect)> = Vec::new();
    // PTY 输出后的 cwd 去抖检查时刻；空闲时不设置、不轮询。
    let mut cwd_check: Option<Instant> = None;
    // 对话框列表查询在途的仓库根；结果返回前不重复发起。
    let mut git_inflight: HashSet<PathBuf> = HashSet::new();
    seed_git_requests(state);

    while !state.should_quit {
        // 会话按状态对齐：新建工作区的窗格在此启动，被移除工作区的会话在此终止。
        reconcile_sessions(state, &mut sessions, &sender);
        if cwd_check.is_some_and(|at| Instant::now() >= at) {
            cwd_check = None;
            if poll_process_cwds(state, &sessions) {
                dirty = true;
            }
        }
        git::pump_git_queries(state, &sender, &mut git_inflight);
        let geometry = current_geometry(tui, state, config)?;
        if geometry.rects != last_rects {
            update::resize_panes(state, &geometry.rects);
            // 几何变化使自动滚动登记的坐标失效，停止拖拽滚动。
            update::stop_selection_autoscroll(state);
            resize_sessions(&mut sessions, &geometry.rects);
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

        if dirty && last_draw.elapsed() >= FRAME_INTERVAL {
            tui.draw(|frame| ui::render(frame, state, config))?;
            dirty = false;
            last_draw = Instant::now();
        }

        let mut wait = if dirty {
            FRAME_INTERVAL.saturating_sub(last_draw.elapsed())
        } else {
            IDLE_WAIT
        };
        if let Some(deadline) = update::next_deadline(state) {
            wait = wait.min(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(at) = cwd_check {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => {
                    handle_terminal_event(event, state, &mut sessions, &geometry, config, &mut dirty);
                    pump_mention_scan(state, &sender);
                    git::pump_git_queries(state, &sender, &mut git_inflight);
                }
                Some(Err(error)) => return Err(error),
                None => break,
            },
            message = receiver.recv() => {
                if let Some(message) = message {
                    handle_app_event(message, state, &mut sessions, &mut git_inflight);
                    while let Ok(message) = receiver.try_recv() {
                        handle_app_event(message, state, &mut sessions, &mut git_inflight);
                    }
                    // 输出后安排一次去抖检查；窗口内合并，空闲不轮询。
                    cwd_check.get_or_insert_with(|| Instant::now() + CWD_CHECK_DELAY);
                    dirty = true;
                }
            }
            () = tokio::time::sleep(wait) => {}
        }
    }

    for session in sessions.values_mut() {
        session.kill();
    }
    Ok(())
}

/// PTY 输出后的 cwd 去抖检查延迟。
const CWD_CHECK_DELAY: Duration = Duration::from_millis(300);

/// 启动时为已有工作区登记一次 git 元数据查询。
fn seed_git_requests(state: &mut AppState) {
    let cwds: Vec<PathBuf> = state
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.cwd.clone())
        .collect();
    for cwd in cwds {
        update::request_git_refresh(state, &cwd);
    }
}

/// 轮询窗格 shell 进程的 cwd：全部窗格更新 cwd 与标题标签，工作区跟随根窗格 cwd
/// （自动命名同步改名，手动命名只更新目录并重查 git 归属）；返回是否有变化。
///
/// 窗格退出或读不到 cwd 时保持原值。
fn poll_process_cwds(state: &mut AppState, sessions: &HashMap<PaneId, PtySession>) -> bool {
    let home = home_dir();
    let mut cwds: HashMap<PaneId, std::path::PathBuf> = HashMap::new();
    for id in state.all_pane_ids() {
        let Some(pid) = sessions.get(&id).and_then(PtySession::process_id) else {
            continue;
        };
        if let Some(cwd) = crate::platform::process_cwd(pid) {
            cwds.insert(id, cwd);
        }
    }
    let mut changed = false;
    for (id, cwd) in &cwds {
        let label = workspace_label(cwd, home.as_deref());
        if update::update_pane_cwd(state, *id, cwd, label) {
            changed = true;
        }
    }
    let tracked: Vec<(usize, PaneId)> = state
        .workspaces
        .iter()
        .enumerate()
        .filter_map(|(index, workspace)| workspace.root_pane().map(|pane| (index, pane)))
        .collect();
    for (index, pane) in tracked {
        if let Some(cwd) = cwds.get(&pane)
            && update::update_workspace_cwd(state, index, cwd)
        {
            changed = true;
            // 缓存 checkout 仍包含新 cwd 时只是进了子目录；否则重查 git 元数据。
            let stale = !state
                .workspaces
                .get(index)
                .and_then(|workspace| workspace.git.as_ref())
                .is_some_and(|git| cwd.starts_with(&git.checkout_path));
            if stale {
                update::request_git_refresh(state, cwd);
            }
        }
    }
    changed
}

/// 按状态对齐 PTY 会话：新增终端窗格启动会话，被移除或非终端的窗格终止会话；幂等。
fn reconcile_sessions(
    state: &AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    sender: &mpsc::UnboundedSender<AppEvent>,
) {
    sessions.retain(|id, session| {
        let alive = state
            .pane_anywhere(*id)
            .is_some_and(|pane| pane.kind == PaneKind::Terminal);
        if !alive {
            session.kill();
        }
        alive
    });
    for id in state.all_pane_ids() {
        if sessions.contains_key(&id) {
            continue;
        }
        let Some(pane) = state.pane_anywhere(id) else {
            continue;
        };
        if pane.kind != PaneKind::Terminal {
            continue;
        }
        let size = pane.terminal.size();
        let sink = sender.clone();
        let cwd = state.workspace_cwd_for_pane(id);
        let result = PtySession::spawn(id, size.cols, size.rows, cwd, move |event| {
            let message = match event {
                PtyEvent::Output(bytes) => AppEvent::PaneOutput(id, bytes),
                PtyEvent::Exited => AppEvent::PaneExit(id),
            };
            let _ = sink.send(message);
        });
        match result {
            Ok(session) => {
                sessions.insert(id, session);
            }
            Err(error) => tracing::error!(pane = id.raw(), %error, "pty spawn failed"),
        }
    }
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
    state.prompt_width = ui::layout::default_prompt_width(area, config, state.sidebar_collapsed);
    Ok(())
}

/// 当前帧几何：整屏区域、窗格矩形与区域划分；渲染、命中测试与 PTY 尺寸同步共用。
struct Geometry {
    screen: Rect,
    rects: Vec<(PaneId, Rect)>,
    view: ui::layout::ViewLayout,
}

/// 当前终端尺寸下活动标签的窗格几何（含 prompt 右栏）与屏幕区域划分。
///
/// 右栏折叠时不参与命中、选择与尺寸同步，展开时按当前几何恢复。
fn current_geometry(tui: &mut Tui, state: &AppState, config: &Config) -> io::Result<Geometry> {
    let size = tui.terminal().size()?;
    let screen = Rect::new(0, 0, size.width, size.height);
    let view = ui::layout::compute(
        screen,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    if !state.prompt_collapsed {
        rects.push((state.prompt.id(), view.prompt));
    }
    Ok(Geometry {
        screen,
        rects,
        view,
    })
}

/// 随几何变化同步 PTY 窗口尺寸。
fn resize_sessions(sessions: &mut HashMap<PaneId, PtySession>, rects: &[(PaneId, Rect)]) {
    for (id, rect) in rects {
        let (cols, rows) = layout::pane_inner_size(*rect);
        if let Some(session) = sessions.get_mut(id)
            && let Err(error) = session.resize(cols, rows)
        {
            tracing::warn!(pane = id.raw(), %error, "pty resize failed");
        }
    }
}

fn handle_terminal_event(
    event: TerminalEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    config: &Config,
    dirty: &mut bool,
) {
    let Geometry {
        screen,
        rects,
        view,
    } = geometry;
    match event {
        TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
            let mode = state
                .active_pane()
                .map(|pane| pane.terminal.mode())
                .unwrap_or_else(TermMode::empty);
            let overlay = state.overlay.as_ref().map(Overlay::kind);
            let pane_lx = state
                .active_pane()
                .is_some_and(|pane| pane.view == PaneView::Lx);
            let Some(routed) = input::route(key, mode, state.prompt_focused, overlay, pane_lx)
            else {
                return;
            };
            match routed {
                Routed::Action(action) => {
                    update::apply(action, state);
                    *dirty = true;
                }
                Routed::Editor(command) => {
                    update::apply_editor(state, command);
                    *dirty = true;
                }
                Routed::Overlay(key) => {
                    let was_confirm = matches!(state.overlay, Some(Overlay::ConfirmClose(_)));
                    let was_menu = matches!(state.overlay, Some(Overlay::Menu(_)));
                    let was_worktree = matches!(state.overlay, Some(Overlay::WorktreeOpen(_)));
                    update::apply_overlay_key(state, key);
                    if was_confirm || was_worktree {
                        ensure_workspace_visible(state, view);
                    }
                    if was_confirm || was_menu || was_worktree {
                        reveal_active_tab(state, view);
                    }
                    *dirty = true;
                }
                Routed::Copy => {
                    if let Some(text) = update::prompt_selection_text(state) {
                        let anchor = Some(state.prompt.id());
                        let now = Instant::now();
                        if crate::platform::write_clipboard(&text) {
                            update::show_toast(
                                state,
                                Toast::new(ToastKind::Info, ui::text::TOAST_COPIED, anchor, now)
                                    .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
                            );
                        } else {
                            tracing::warn!("clipboard write failed");
                            update::show_toast(
                                state,
                                Toast::new(
                                    ToastKind::Error,
                                    ui::text::TOAST_COPY_FAILED,
                                    anchor,
                                    now,
                                )
                                .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
                            );
                        }
                        *dirty = true;
                    }
                }
                Routed::Pane(bytes) => {
                    let id = state.active_tab().layout.focus();
                    write_to_pane(sessions, id, &bytes);
                    update::reset_pane_scroll(state, id);
                    *dirty = true;
                }
            }
        }
        TerminalEvent::Paste(text) => {
            // 浮层打开时粘贴吞掉，不穿透到底层终端（与键盘过滤一致）。
            if state.overlay.is_some() {
                return;
            }
            if state.prompt_focused {
                update::apply_editor(state, EditorCommand::InsertText(text));
                *dirty = true;
                return;
            }
            // lx 视图不接收输入：粘贴同样吞掉，不写入隐藏终端。
            if state
                .active_pane()
                .is_some_and(|pane| pane.view == PaneView::Lx)
            {
                return;
            }
            let id = state.active_tab().layout.focus();
            let bracketed = state
                .active_pane()
                .is_some_and(|pane| pane.terminal.mode().contains(TermMode::BRACKETED_PASTE));
            let payload = if bracketed {
                format!("\x1b[200~{text}\x1b[201~").into_bytes()
            } else {
                text.into_bytes()
            };
            write_to_pane(sessions, id, &payload);
            update::reset_pane_scroll(state, id);
            *dirty = true;
        }
        TerminalEvent::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if state.overlay.is_some() {
                    let was_confirm = matches!(state.overlay, Some(Overlay::ConfirmClose(_)));
                    let was_worktree = matches!(state.overlay, Some(Overlay::WorktreeOpen(_)));
                    handle_overlay_click(state, *screen, mouse.column, mouse.row);
                    if was_confirm || was_worktree {
                        ensure_workspace_visible(state, view);
                    }
                    reveal_active_tab(state, view);
                    *dirty = true;
                    return;
                }
                if let Some(toast) = ui::toast::rect(state, view, rects, *screen, config)
                    && toast.contains((mouse.column, mouse.row).into())
                {
                    update::dismiss_toast(state);
                    *dirty = true;
                    return;
                }
                if ui::exit_button_at(view, mouse.column, mouse.row) {
                    update::apply(Action::Quit, state);
                    *dirty = true;
                    return;
                }
                let tab_bar = ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
                if tab_bar
                    .scroll_left
                    .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                {
                    if update::scroll_tab_bar(state, -1, tab_bar.max_scroll) {
                        *dirty = true;
                    }
                    return;
                }
                if tab_bar
                    .scroll_right
                    .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                {
                    if update::scroll_tab_bar(state, 1, tab_bar.max_scroll) {
                        *dirty = true;
                    }
                    return;
                }
                if let Some(index) = ui::tab_bar::tab_at(&tab_bar, mouse.column, mouse.row) {
                    update::switch_tab(state, index);
                    reveal_active_tab(state, view);
                    *dirty = true;
                    return;
                }
                if tab_bar
                    .add
                    .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                {
                    update::create_tab(state);
                    reveal_active_tab(state, view);
                    *dirty = true;
                    return;
                }
                if let Some(target) =
                    ui::collapse_button_at(view, state.agents_collapsed, mouse.column, mouse.row)
                {
                    let action = match target {
                        ui::CollapseTarget::Sidebar => Action::ToggleSidebar,
                        ui::CollapseTarget::Prompt => Action::TogglePrompt,
                        ui::CollapseTarget::Agents => Action::ToggleAgents,
                    };
                    update::apply(action, state);
                    let mut pointer_reset = update::set_prompt_hover(state, false);
                    pointer_reset |= update::set_sidebar_hover(state, false);
                    if pointer_reset {
                        reset_pointer_shape();
                    }
                    *dirty = true;
                } else if ui::sidebar_boundary_at(view, mouse.column, mouse.row) {
                    update::begin_sidebar_resize(state);
                    *dirty = true;
                } else if handle_workspace_scrollbar_press(state, view, mouse.column, mouse.row)
                    || handle_prompt_scrollbar_press(state, view, mouse.column, mouse.row)
                {
                    *dirty = true;
                } else if let Some(index) =
                    ui::workspace_group_toggle_at(view, state, mouse.column, mouse.row)
                {
                    update::toggle_workspace_group(state, index);
                    if let Some(rows) = ui::workspace_list_rows(view, state) {
                        update::clamp_workspace_scroll(state, rows);
                    }
                    ensure_workspace_visible(state, view);
                    *dirty = true;
                } else if let Some(index) =
                    ui::workspace_item_at(view, state, mouse.column, mouse.row)
                {
                    update::switch_workspace(state, index);
                    ensure_workspace_visible(state, view);
                    reveal_active_tab(state, view);
                    update::begin_workspace_drag(state, index);
                    *dirty = true;
                } else if ui::add_workspace_button(view)
                    .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                {
                    update::create_workspace(state);
                    ensure_workspace_visible(state, view);
                    reveal_active_tab(state, view);
                    *dirty = true;
                } else if layout::resize_boundary_at(rects, mouse.column, mouse.row)
                    .is_some_and(|hit| hit.second == state.prompt.id())
                {
                    update::begin_prompt_resize(state);
                    *dirty = true;
                } else if let Some(index) = mention_item_at(state, view, mouse.column, mouse.row) {
                    update::select_mention(state, index);
                    *dirty = true;
                } else if let Some(index) = panel_item_at(state, view, mouse.column, mouse.row) {
                    update::select_panel(state, index);
                    *dirty = true;
                } else if let Some(pane) =
                    pane_toggle_button_at(state, rects, mouse.column, mouse.row)
                {
                    // 按钮是控件：只切换视图，不改变焦点。
                    update::toggle_pane_view(state, pane);
                    *dirty = true;
                } else if let Some(hit) = layout::resize_boundary_at(rects, mouse.column, mouse.row)
                    .filter(|hit| pane_boundary_draggable(state, *hit))
                {
                    update::begin_pane_resize(state, hit);
                    *dirty = true;
                } else if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
                    let row = mouse.row - inner.y;
                    let col = mouse.column - inner.x;
                    if pane == state.prompt.id() {
                        update::focus_prompt(state);
                        update::place_prompt_cursor(state, row, col);
                        update::begin_selection(state, pane, row, col);
                        *dirty = true;
                    } else if matches!(
                        state.pane_anywhere(pane).map(|pane| pane.kind),
                        Some(PaneKind::Terminal)
                    ) {
                        if state
                            .pane_anywhere(pane)
                            .is_some_and(|target| target.view == PaneView::Lx)
                        {
                            // lx 页不接受终端交互：仅聚焦并清掉既有选区。
                            update::focus_pane(state, pane);
                            update::clear_selection(state);
                            *dirty = true;
                        } else if handle_terminal_scrollbar_press(
                            state,
                            pane,
                            inner,
                            mouse.column,
                            mouse.row,
                        ) {
                            // 滚动条按下先于焦点与选区：thumb 拖拽，轨道点击跳转。
                            *dirty = true;
                        } else {
                            update::focus_pane(state, pane);
                            // 应用在鼠标上报模式时按键归它自己（对齐 herdr）：转发并按它的
                            // 选区逻辑处理，不启动本地选区。
                            if forward_pane_mouse_event(state, sessions, pane, inner, &mouse) {
                                update::clear_selection(state);
                            } else {
                                update::begin_terminal_selection(state, pane, row, col);
                            }
                            *dirty = true;
                        }
                    } else {
                        update::clear_selection(state);
                    }
                } else {
                    update::clear_selection(state);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // 菜单打开时右键不穿透：标签、工作区项或窗格上改挂菜单，其余位置关闭。
                if matches!(state.overlay, Some(Overlay::Menu(_))) {
                    if !open_right_click_menu(state, rects, view, mouse.column, mouse.row) {
                        update::close_overlay(state);
                    }
                    *dirty = true;
                    return;
                }
                // 重命名与关闭确认期间右键吞掉，不替换模态。
                if state.overlay.is_some() {
                    return;
                }
                if open_right_click_menu(state, rects, view, mouse.column, mouse.row) {
                    *dirty = true;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if state.overlay.is_some() {
                    return;
                }
                if let Some(grab) = state.workspace_scroll_drag {
                    let Some(bar) = ui::workspace_scrollbar(view, state) else {
                        return;
                    };
                    let rows = ui::workspace_list_rows(view, state).unwrap_or(0);
                    let offset =
                        ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
                    if update::set_workspace_scroll(state, offset, rows) {
                        *dirty = true;
                    }
                    return;
                }
                if let Some(grab) = state.prompt_scroll_drag {
                    let Some(bar) = ui::prompt_scrollbar(view, state) else {
                        return;
                    };
                    let offset =
                        ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
                    if update::set_prompt_scroll(state, offset) {
                        *dirty = true;
                    }
                    return;
                }
                if let Some((pane, grab)) = state.terminal_scroll_drag {
                    let bar = rects
                        .iter()
                        .find(|(id, _)| *id == pane)
                        .and_then(|(_, rect)| {
                            let inner = layout::pane_inner_rect(*rect);
                            state
                                .pane_anywhere(pane)
                                .and_then(|target| ui::main_content::scrollbar(inner, target))
                        });
                    let Some(bar) = bar else {
                        return;
                    };
                    let offset =
                        ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
                    if update::set_terminal_scroll(state, pane, offset) {
                        *dirty = true;
                    }
                    return;
                }
                if let Some(hit) = state.resizing_pane {
                    let pos = match hit.direction {
                        Direction::Horizontal => mouse.column,
                        Direction::Vertical => mouse.row,
                    };
                    if update::drag_pane_boundary(
                        state,
                        hit,
                        view.panes,
                        pos,
                        config.min_pane_width,
                        config.min_pane_height,
                    ) {
                        *dirty = true;
                    }
                    return;
                }
                if state.resizing_sidebar {
                    let width = ui::layout::sidebar_width_at(view, config, mouse.column);
                    update::drag_sidebar(state, width);
                    *dirty = true;
                    return;
                }
                if state.resizing_prompt {
                    let width =
                        ui::layout::prompt_width_at(view, mouse.column, config.min_pane_width);
                    update::drag_prompt(state, width);
                    *dirty = true;
                    return;
                }
                if state.workspace_drag.is_some() {
                    if let Some(target) = ui::workspace_drop_index(view, state, mouse.row)
                        && update::drag_workspace_to(state, target)
                    {
                        *dirty = true;
                    }
                    return;
                }
                // 没有本地选区时，鼠标上报模式的应用自己处理拖拽（对齐 herdr）。
                if state.terminal_selection.is_none()
                    && state.selection.is_none()
                    && let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row)
                    && forward_pane_mouse_event(state, sessions, pane, inner, &mouse)
                {
                    *dirty = true;
                    return;
                }
                // 终端拖拽选区：把终点更新到鼠标位置的内容；prompt 选区同时移动编辑器光标。
                let pane = if let Some(pane) = state.terminal_selection {
                    pane
                } else if let Some(selection) = state.selection {
                    selection.pane()
                } else {
                    return;
                };
                let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
                    return;
                };
                let inner = layout::pane_inner_rect(*rect);
                if inner.width == 0 || inner.height == 0 {
                    return;
                }
                let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
                let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
                if state.terminal_selection == Some(pane) {
                    update::drag_terminal_selection(state, pane, row, col);
                } else {
                    update::drag_selection(state, pane, row, col);
                }
                if pane == state.prompt.id() {
                    update::place_prompt_cursor(state, row, col);
                }
                update::arm_selection_autoscroll(
                    state,
                    pane,
                    inner,
                    (mouse.column, mouse.row),
                    Instant::now(),
                );
                *dirty = true;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if state.overlay.is_some() {
                    return;
                }
                if state.workspace_scroll_drag.take().is_some() {
                    return;
                }
                if state.prompt_scroll_drag.take().is_some() {
                    return;
                }
                if state.terminal_scroll_drag.take().is_some() {
                    return;
                }
                if state.resizing_pane.is_some() {
                    update::end_pane_resize(state);
                    *dirty = true;
                    return;
                }
                if state.workspace_drag.is_some() {
                    update::end_workspace_drag(state);
                    *dirty = true;
                    return;
                }
                if state.resizing_sidebar {
                    update::end_sidebar_resize(state);
                    *dirty = true;
                    return;
                }
                if state.resizing_prompt {
                    update::end_prompt_resize(state);
                    *dirty = true;
                    return;
                }
                // 没有本地选区时，鼠标上报模式的应用自己处理释放（对齐 herdr）。
                if state.terminal_selection.is_none()
                    && state.selection.is_none()
                    && let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row)
                    && forward_pane_mouse_event(state, sessions, pane, inner, &mouse)
                {
                    *dirty = true;
                    return;
                }
                if state
                    .selection
                    .is_some_and(|selection| selection.pane() == state.prompt.id())
                {
                    update::end_selection_drag(state);
                    *dirty = true;
                    return;
                }
                if let Some(pane) = state.terminal_selection
                    && let Some(text) = update::finish_terminal_selection(state, pane)
                {
                    let now = Instant::now();
                    if crate::platform::write_clipboard(&text) {
                        update::show_toast(
                            state,
                            Toast::new(ToastKind::Info, ui::text::TOAST_COPIED, Some(pane), now)
                                .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
                        );
                    } else {
                        tracing::warn!("clipboard write failed");
                        update::show_toast(
                            state,
                            Toast::new(
                                ToastKind::Error,
                                ui::text::TOAST_COPY_FAILED,
                                Some(pane),
                                now,
                            )
                            .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
                        );
                    }
                }
                update::clear_selection(state);
                *dirty = true;
            }
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {
                if state.overlay.is_some() {
                    return;
                }
                if state.resizing_prompt || state.resizing_pane.is_some() {
                    return;
                }
                // @ 提及面板打开且指针在其上：滚轮滚面板，不穿透到 prompt 与窗格。
                if let Some(direction) = vertical_wheel_direction(mouse.kind)
                    && mention_panel_rect(state, view)
                        .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()))
                {
                    update::scroll_mention(state, direction);
                    *dirty = true;
                    return;
                }
                // 块命令面板同上：滚轮移动高亮到底/顶后钳制。
                if let Some(direction) = vertical_wheel_direction(mouse.kind)
                    && panel_rect(state, view)
                        .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()))
                {
                    update::scroll_panel(state, direction);
                    *dirty = true;
                    return;
                }
                // prompt 拖拽选区：滚轮滚动视口，锚点钉在文本上，终点跟到鼠标。
                if let Some(selection) = state.selection
                    && selection.is_dragging()
                {
                    if let Some(direction) = vertical_wheel_direction(mouse.kind)
                        && let Some((_, rect)) =
                            rects.iter().find(|(id, _)| *id == selection.pane())
                    {
                        let inner = layout::pane_inner_rect(*rect);
                        if inner.width > 0 && inner.height > 0 {
                            let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
                            let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
                            if update::wheel_prompt_selection(state, direction, row, col) {
                                *dirty = true;
                            }
                        }
                    }
                    return;
                }
                // 终端拖拽选区优先于鼠标上报：滚动选区所在窗格并延伸选区。
                if let Some(pane) = state.terminal_selection
                    && scroll_selection_wheel(state, pane, rects, &mouse)
                {
                    *dirty = true;
                    return;
                }
                if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
                    if pane == state.prompt.id() {
                        update::clear_selection(state);
                        if let Some(direction) = vertical_wheel_direction(mouse.kind) {
                            update::scroll_prompt(state, direction);
                            *dirty = true;
                        }
                    } else if state.pane_anywhere(pane).is_some_and(|target| {
                        target.kind == PaneKind::Terminal && target.view == PaneView::Terminal
                    }) && handle_pane_wheel(state, sessions, pane, inner, &mouse)
                    {
                        *dirty = true;
                    }
                } else if ui::workspace_section_at(view, state, mouse.column, mouse.row)
                    && let Some(rows) = ui::workspace_list_rows(view, state)
                    && let Some(direction) = vertical_wheel_direction(mouse.kind)
                    && update::scroll_workspace_list(state, direction, rows)
                {
                    // 列表滚动后指针下的工作区可能已换行：按新偏移重算悬停。
                    let hover = ui::workspace_item_at(view, state, mouse.column, mouse.row);
                    update::set_workspace_hover(state, hover);
                    *dirty = true;
                }
            }
            MouseEventKind::Moved => {
                // 浮层打开时悬停只服务菜单高亮，不触碰底层 hover 状态。
                if state.overlay.is_some() {
                    if let Some(Overlay::Menu(menu)) = state.overlay.as_ref() {
                        let layout = ui::overlay::menu_layout(state, *screen, menu);
                        if let Some(index) =
                            ui::widgets::menu::item_at(&layout, mouse.column, mouse.row)
                            && update::set_menu_selection(state, index)
                        {
                            *dirty = true;
                        }
                    }
                    return;
                }
                // 侧栏工作区行与标签栏的悬停高亮先于浮层命中计算：指针离开条目区即清空。
                let workspace_hover = if state.sidebar_collapsed || state.workspace_drag.is_some() {
                    None
                } else {
                    ui::workspace_item_at(view, state, mouse.column, mouse.row)
                };
                let tab_layout =
                    ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
                let tab_hover = ui::tab_bar::tab_at(&tab_layout, mouse.column, mouse.row);
                let mut item_hover_changed = update::set_workspace_hover(state, workspace_hover);
                item_hover_changed |= update::set_tab_hover(state, tab_hover);
                if item_hover_changed {
                    *dirty = true;
                }
                if let Some(index) = mention_item_at(state, view, mouse.column, mouse.row) {
                    if update::hover_mention(state, index) {
                        *dirty = true;
                    }
                    return;
                }
                if let Some(index) = panel_item_at(state, view, mouse.column, mouse.row) {
                    if update::hover_panel(state, index) {
                        *dirty = true;
                    }
                    return;
                }
                let hovering_toast = ui::toast::rect(state, view, rects, *screen, config)
                    .is_some_and(|toast| toast.contains((mouse.column, mouse.row).into()));
                if update::set_toast_hover(state, hovering_toast) {
                    *dirty = true;
                }
                let boundary = layout::resize_boundary_at(rects, mouse.column, mouse.row);
                let sidebar_hover =
                    !hovering_toast && ui::sidebar_boundary_at(view, mouse.column, mouse.row);
                let prompt_hover = !hovering_toast
                    && !sidebar_hover
                    && boundary.is_some_and(|hit| hit.second == state.prompt.id());
                let pane_hover = if hovering_toast || sidebar_hover || prompt_hover {
                    None
                } else {
                    boundary.filter(|hit| pane_boundary_draggable(state, *hit))
                };
                let mut hover_changed = update::set_sidebar_hover(state, sidebar_hover);
                hover_changed |= update::set_prompt_hover(state, prompt_hover);
                hover_changed |= update::set_pane_hover(state, pane_hover);
                if hover_changed {
                    let shape = if sidebar_hover || prompt_hover {
                        crate::platform::PointerShape::EwResize
                    } else {
                        match pane_hover.map(|hit| hit.direction) {
                            Some(Direction::Horizontal) => crate::platform::PointerShape::EwResize,
                            Some(Direction::Vertical) => crate::platform::PointerShape::NsResize,
                            None => crate::platform::PointerShape::Default,
                        }
                    };
                    set_pointer_shape(shape);
                    *dirty = true;
                }
                if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
                    forward_pane_mouse_event(state, sessions, pane, inner, &mouse);
                }
            }
            _ => {}
        },
        TerminalEvent::Resize(_, _) => *dirty = true,
        _ => {}
    }
}

/// @ 提及面板需要缓存且无在途扫描时，发起后台扫描并把结果投递回主循环。
///
/// prompt 未聚焦时不扫描；扫描失败按空结果处理（面板保持隐藏）。
fn pump_mention_scan(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) {
    if !state.prompt_focused {
        return;
    }
    let Some((generation, root)) = state.prompt.take_mention_scan_request() else {
        return;
    };
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let entries = crate::files::scan(&root);
        let _ = sender.send(AppEvent::MentionScanned {
            generation,
            entries,
        });
    });
}

/// 纵向滚轮方向（负数向上）；横向滚轮返回 None。
fn vertical_wheel_direction(kind: MouseEventKind) -> Option<isize> {
    match kind {
        MouseEventKind::ScrollUp => Some(-1),
        MouseEventKind::ScrollDown => Some(1),
        _ => None,
    }
}

/// 拖拽选择中滚轮：滚动选区所在窗格，并把选区光标延伸到鼠标位置（钳在窗格内）。
///
/// 纵向滚轮消费事件（对齐 herdr：选区进行中优先于鼠标上报）；横向返回 false 交回常规路径。
fn scroll_selection_wheel(
    state: &mut AppState,
    pane: PaneId,
    rects: &[(PaneId, Rect)],
    mouse: &MouseEvent,
) -> bool {
    let Some(direction) = vertical_wheel_direction(mouse.kind) else {
        return false;
    };
    let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
        return true;
    };
    let inner = layout::pane_inner_rect(*rect);
    if inner.width == 0 || inner.height == 0 {
        return true;
    }
    update::scroll_pane(state, pane, direction);
    let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
    let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
    update::drag_terminal_selection(state, pane, row, col);
    true
}

/// 按终端模式把滚轮交给窗格：转发鼠标上报、备用屏方向键或本地回滚。
///
/// 前两种由应用接管滚动，本地视口吸回底部；返回事件是否被消费（需要重绘）。
fn handle_pane_wheel(
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    pane: PaneId,
    inner: Rect,
    mouse: &MouseEvent,
) -> bool {
    let Some((routing, mode)) = state
        .pane_anywhere(pane)
        .map(|target| (target.terminal.wheel_routing(), target.terminal.mode()))
    else {
        return false;
    };
    match routing {
        WheelRouting::HostScroll => {
            let Some(direction) = vertical_wheel_direction(mouse.kind) else {
                return false;
            };
            update::scroll_pane(state, pane, direction)
        }
        WheelRouting::MouseReport => {
            let column = mouse.column.saturating_sub(inner.x);
            let row = mouse.row.saturating_sub(inner.y);
            let Some(bytes) =
                input::encode::encode_mouse_wheel(mouse.kind, column, row, mouse.modifiers, mode)
            else {
                return false;
            };
            update::reset_pane_scroll(state, pane);
            write_to_pane(sessions, pane, &bytes);
            true
        }
        WheelRouting::AlternateScroll => {
            let code = match mouse.kind {
                MouseEventKind::ScrollUp => KeyCode::Up,
                MouseEventKind::ScrollDown => KeyCode::Down,
                _ => return false,
            };
            let Some(bytes) =
                input::encode::encode_key(KeyEvent::new(code, KeyModifiers::NONE), mode)
            else {
                return false;
            };
            update::reset_pane_scroll(state, pane);
            write_to_pane(sessions, pane, &bytes);
            true
        }
    }
}

/// 把鼠标按键、拖拽或移动按应用声明的协议转发给窗格；非鼠标上报模式返回 false 交回本地选区。
///
/// 对齐 herdr：应用接管鼠标时 lx-tui 不做本地选区，滚动与选择都由应用自己实现；
/// 拖拽与无按键移动还需应用分别开启 1002/1003 才上报，避免多发未订阅的事件。
fn forward_pane_mouse_event(
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    pane: PaneId,
    inner: Rect,
    mouse: &MouseEvent,
) -> bool {
    // lx 视图不向隐藏终端转发任何鼠标事件。
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == PaneView::Lx)
    {
        return false;
    }
    let Some((routing, mode)) = state
        .pane_anywhere(pane)
        .map(|target| (target.terminal.wheel_routing(), target.terminal.mode()))
    else {
        return false;
    };
    let reportable = routing == WheelRouting::MouseReport
        && match mouse.kind {
            MouseEventKind::Drag(_) => {
                mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
            }
            MouseEventKind::Moved => mode.contains(TermMode::MOUSE_MOTION),
            _ => true,
        };
    if !reportable {
        return false;
    }
    let column = mouse.column.saturating_sub(inner.x);
    let row = mouse.row.saturating_sub(inner.y);
    let Some(bytes) =
        input::encode::encode_mouse_button(mouse.kind, column, row, mouse.modifiers, mode)
    else {
        return false;
    };
    if !matches!(mouse.kind, MouseEventKind::Moved) {
        update::reset_pane_scroll(state, pane);
    }
    write_to_pane(sessions, pane, &bytes);
    true
}

/// 浮层左键点击：菜单项与按钮执行命令，其余位置取消。
fn handle_overlay_click(state: &mut AppState, screen: Rect, column: u16, row: u16) {
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => {
            let layout = ui::overlay::menu_layout(state, screen, menu);
            match ui::widgets::menu::item_at(&layout, column, row) {
                Some(index) => {
                    update::set_menu_selection(state, index);
                    update::activate_menu(state);
                }
                None => update::close_overlay(state),
            }
        }
        Some(Overlay::Rename(_)) => {
            let button = ui::overlay::rename_shell(screen)
                .and_then(|shell| ui::overlay::rename_button_at(&shell, column, row));
            match button {
                Some(ui::overlay::RenameButton::Save) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::RenameButton::Clear) => {
                    update::apply_overlay_key(state, OverlayKey::Clear)
                }
                Some(ui::overlay::RenameButton::Cancel) | None => update::close_overlay(state),
            }
        }
        Some(Overlay::ConfirmClose(_)) => {
            let button = ui::overlay::confirm_shell(screen)
                .and_then(|shell| ui::overlay::confirm_button_at(&shell, column, row));
            match button {
                Some(ui::overlay::ConfirmButton::Confirm) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::ConfirmButton::Cancel) | None => update::close_overlay(state),
            }
        }
        Some(Overlay::WorktreeOpen(dialog)) => {
            let Some(shell) = ui::overlay::worktree_dialog_shell(screen, dialog.entries.len())
            else {
                update::close_overlay(state);
                return;
            };
            if let Some(index) = ui::overlay::worktree_dialog_entry_at(&shell, dialog, column, row)
            {
                update::set_worktree_open_selection(state, index);
                return;
            }
            match ui::overlay::worktree_dialog_button_at(&shell, column, row) {
                Some(ui::overlay::WorktreeDialogButton::Open) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::WorktreeDialogButton::Cancel) | None => {
                    update::close_overlay(state)
                }
            }
        }
        None => {}
    }
}

/// 命中标签、工作区项或主区窗格时打开对应右键菜单；返回是否打开。
///
/// prompt 右栏、边框与切换按钮不参与：`pane_at` 只用窗格内容区，
/// 且目标必须在当前标签的窗格载荷中。
fn open_right_click_menu(
    state: &mut AppState,
    rects: &[(PaneId, Rect)],
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let tab_bar = ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
    if let Some(index) = ui::tab_bar::tab_at(&tab_bar, column, row) {
        update::open_tab_menu(state, index, (column, row));
        return true;
    }
    if let Some(index) = ui::workspace_item_at(view, state, column, row) {
        update::open_workspace_menu(state, index, (column, row));
        return true;
    }
    if let Some((pane, _)) = pane_at(rects, column, row)
        && state.active_tab().pane(pane).is_some()
    {
        update::open_pane_menu(state, pane, (column, row));
        return true;
    }
    false
}

/// 工作区滚动条按下：thumb 开始拖拽，轨道点击跳转；返回是否命中。
fn handle_workspace_scrollbar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = ui::workspace_scrollbar(view, state) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    let rows = ui::workspace_list_rows(view, state).unwrap_or(0);
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.workspace_scroll_drag = Some(grab),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_workspace_scroll(state, offset, rows);
        }
    }
    true
}

/// prompt 滚动条按下：thumb 开始拖拽，轨道点击跳转；不改变焦点与光标；返回是否命中。
fn handle_prompt_scrollbar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = ui::prompt_scrollbar(view, state) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.prompt_scroll_drag = Some(grab),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_prompt_scroll(state, offset);
        }
    }
    true
}

/// 终端窗格滚动条按下：thumb 开始拖拽，轨道点击跳转；不改变焦点；返回是否命中。
fn handle_terminal_scrollbar_press(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::scrollbar(inner, target))
    else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.terminal_scroll_drag = Some((pane, grab)),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_terminal_scroll(state, pane, offset);
        }
    }
    true
}

/// 保证当前工作区在列表可见范围内（创建/切换/关闭后调用）。
fn ensure_workspace_visible(state: &mut AppState, view: &ui::layout::ViewLayout) {
    if let Some(rows) = ui::workspace_list_rows(view, state) {
        update::ensure_workspace_visible(state, rows);
    }
}

/// 保证激活标签在标签栏可见范围内（切换/创建/关闭后调用）。
fn reveal_active_tab(state: &mut AppState, view: &ui::layout::ViewLayout) {
    let workspace = state.active_workspace();
    let tab_bar = ui::tab_bar::layout(view, workspace, state.tab_scroll);
    let offset = ui::tab_bar::reveal_scroll(&tab_bar, workspace);
    let max = tab_bar.max_scroll;
    update::set_tab_scroll(state, offset, max);
}

fn handle_app_event(
    message: AppEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    git_inflight: &mut HashSet<PathBuf>,
) {
    match message {
        AppEvent::PaneOutput(id, bytes) => {
            let responses = update::feed_pane(state, id, &bytes);
            if !responses.is_empty() {
                write_to_pane(sessions, id, &responses);
            }
        }
        AppEvent::PaneExit(id) => update::mark_pane_exited(state, id),
        AppEvent::MentionScanned {
            generation,
            entries,
        } => update::apply_mention_entries(state, generation, entries),
        AppEvent::GitRefreshed { cwd, checkout } => {
            update::apply_git_refresh(state, &cwd, checkout)
        }
        AppEvent::WorktreeListed {
            repo_root,
            entries,
            failed,
        } => {
            git_inflight.remove(&repo_root);
            update::apply_worktree_list(state, &repo_root, entries, failed);
        }
    }
}

/// 写入鼠标指针形状；不支持的终端静默忽略，失败只影响悬停提示。
fn set_pointer_shape(shape: crate::platform::PointerShape) {
    if let Err(error) = crate::platform::set_pointer_shape(shape) {
        tracing::debug!(%error, "pointer shape write failed");
    }
}

/// 指针形状恢复终端默认。
fn reset_pointer_shape() {
    set_pointer_shape(crate::platform::PointerShape::Default);
}

fn write_to_pane(sessions: &mut HashMap<PaneId, PtySession>, id: PaneId, bytes: &[u8]) {
    if let Some(session) = sessions.get_mut(&id)
        && let Err(error) = session.write(bytes)
    {
        tracing::warn!(pane = id.raw(), %error, "pty write failed");
    }
}

/// 提及面板命中：返回条目索引；面板未打开或未命中返回 None。
fn mention_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = layout::pane_inner_rect(view.prompt);
    ui::prompt::mention_item_at(&state.prompt, area, column, row)
}

/// 提及面板矩形；用于滚轮命中。
fn mention_panel_rect(state: &AppState, view: &ui::layout::ViewLayout) -> Option<Rect> {
    let area = layout::pane_inner_rect(view.prompt);
    ui::prompt::mention_panel_rect(&state.prompt, area)
}

/// 块命令面板命中：返回条目索引；面板未打开或未命中返回 None。
fn panel_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = layout::pane_inner_rect(view.prompt);
    ui::prompt::panel_item_at(&state.prompt, area, column, row)
}

/// 块命令面板矩形；用于滚轮命中。
fn panel_rect(state: &AppState, view: &ui::layout::ViewLayout) -> Option<Rect> {
    let area = layout::pane_inner_rect(view.prompt);
    ui::prompt::panel_rect(&state.prompt, area)
}

/// 命中窗格内容区：返回窗格标识与其内容区矩形。
fn pane_at(rects: &[(PaneId, Rect)], column: u16, row: u16) -> Option<(PaneId, Rect)> {
    rects.iter().find_map(|(id, rect)| {
        let inner = layout::pane_inner_rect(*rect);
        if inner.width > 0 && inner.height > 0 && inner.contains((column, row).into()) {
            Some((*id, inner))
        } else {
            None
        }
    })
}

/// 主区窗格边框可拖：两侧均为活动标签窗格且都未折叠；prompt 右栏边界归 prompt 拖拽。
fn pane_boundary_draggable(state: &AppState, hit: layout::BoundaryHit) -> bool {
    let tab = state.active_tab();
    tab.pane(hit.first).is_some()
        && tab.pane(hit.second).is_some()
        && Some(hit.first) != tab.layout.collapsed()
        && Some(hit.second) != tab.layout.collapsed()
}

/// 命中窗格顶边框右端的视图切换按钮：仅 Terminal 窗格有按钮（prompt 栏与占位窗格无）。
fn pane_toggle_button_at(
    state: &AppState,
    rects: &[(PaneId, Rect)],
    column: u16,
    row: u16,
) -> Option<PaneId> {
    ui::main_content::toggle_button_at(rects, column, row).filter(|pane| {
        state
            .pane_anywhere(*pane)
            .is_some_and(|target| target.kind == PaneKind::Terminal)
    })
}

#[cfg(test)]
mod tests;
