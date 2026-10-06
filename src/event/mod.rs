//! 唯一事件循环：tokio select 收敛键盘、PTY 输出与渲染调度。

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use crossterm::event::{
    Event as TerminalEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::app::actions::{Action, EditorCommand, OverlayKey};
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneKind, home_dir, workspace_label};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::config::Config;
use crate::input::{self, Routed};
use crate::layout::{self, PaneId};
use crate::pty::{PtyEvent, PtySession};
use crate::terminal::WheelRouting;
use crate::tui::Tui;
use crate::ui;

/// 应用级事件：后台任务（目前是 PTY 读线程）经此汇入主循环。
#[derive(Debug)]
pub enum AppEvent {
    PaneOutput(PaneId, Vec<u8>),
    PaneExit(PaneId),
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

    while !state.should_quit {
        // 会话按状态对齐：新建工作区的窗格在此启动，被移除工作区的会话在此终止。
        reconcile_sessions(state, &mut sessions, &sender);
        if cwd_check.is_some_and(|at| Instant::now() >= at) {
            cwd_check = None;
            if poll_process_cwds(state, &sessions) {
                dirty = true;
            }
        }
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
            tui.terminal()
                .draw(|frame| ui::render(frame, state, config))?;
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
                }
                Some(Err(error)) => return Err(error),
                None => break,
            },
            message = receiver.recv() => {
                if let Some(message) = message {
                    handle_app_event(message, state, &mut sessions);
                    while let Ok(message) = receiver.try_recv() {
                        handle_app_event(message, state, &mut sessions);
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

/// 轮询窗格 shell 进程的 cwd：全部窗格更新标题标签，自动命名工作区同步名字；
/// 返回是否有变化。
///
/// 手动命名工作区不参与改名；窗格退出或读不到 cwd 时保持原值。
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
        if update::update_pane_cwd(state, *id, label) {
            changed = true;
        }
    }
    let tracked: Vec<(usize, PaneId)> = state
        .workspaces
        .iter()
        .enumerate()
        .filter(|(_, workspace)| !workspace.name_is_manual)
        .filter_map(|(index, workspace)| workspace.root_pane().map(|pane| (index, pane)))
        .collect();
    for (index, pane) in tracked {
        if let Some(cwd) = cwds.get(&pane)
            && update::update_workspace_cwd(state, index, cwd)
        {
            changed = true;
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
        let result = PtySession::spawn(id, size.cols, size.rows, move |event| {
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

/// 启动时确定栏宽：右栏取主区可用宽度的一半（复刻旧 50% 分割），左栏取配置值。
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
            let Some(routed) = input::route(key, mode, state.prompt_focused, overlay) else {
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
                    update::apply_overlay_key(state, key);
                    if was_confirm {
                        ensure_workspace_visible(state, view);
                    }
                    if was_confirm || was_menu {
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
                                Toast::new(ToastKind::Info, ui::text::TOAST_COPIED, anchor, now),
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
                                ),
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
            if state.prompt_focused {
                update::apply_editor(state, EditorCommand::InsertText(text));
                *dirty = true;
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
                    handle_overlay_click(state, *screen, mouse.column, mouse.row);
                    if was_confirm {
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
                    .is_some_and(|(_, right)| right == state.prompt.id())
                {
                    update::begin_prompt_resize(state);
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
                        update::focus_pane(state, pane);
                        // 应用在鼠标上报模式时按键归它自己（对齐 herdr）：转发并按它的
                        // 选区逻辑处理，不启动本地选区。
                        if forward_pane_mouse_event(state, sessions, pane, inner, &mouse) {
                            update::clear_selection(state);
                        } else {
                            update::begin_terminal_selection(state, pane, row, col);
                        }
                        *dirty = true;
                    } else {
                        update::clear_selection(state);
                    }
                } else {
                    update::clear_selection(state);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // 菜单打开时右键不穿透：标签或工作区项上重开，其余位置关闭。
                if matches!(state.overlay, Some(Overlay::Menu(_))) {
                    let tab_bar =
                        ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
                    if let Some(index) = ui::tab_bar::tab_at(&tab_bar, mouse.column, mouse.row) {
                        update::open_tab_menu(state, index, (mouse.column, mouse.row));
                    } else if let Some(index) =
                        ui::workspace_item_at(view, state, mouse.column, mouse.row)
                    {
                        update::open_workspace_menu(state, index, (mouse.column, mouse.row));
                    } else {
                        update::close_overlay(state);
                    }
                    *dirty = true;
                    return;
                }
                // 重命名与关闭确认期间右键吞掉，不替换模态。
                if state.overlay.is_some() {
                    return;
                }
                let tab_bar = ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
                if let Some(index) = ui::tab_bar::tab_at(&tab_bar, mouse.column, mouse.row) {
                    update::open_tab_menu(state, index, (mouse.column, mouse.row));
                    *dirty = true;
                } else if let Some(index) =
                    ui::workspace_item_at(view, state, mouse.column, mouse.row)
                {
                    update::open_workspace_menu(state, index, (mouse.column, mouse.row));
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
                            Toast::new(ToastKind::Info, ui::text::TOAST_COPIED, Some(pane), now),
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
                            ),
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
                if state.resizing_prompt {
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
                    } else if state
                        .pane_anywhere(pane)
                        .is_some_and(|target| target.kind == PaneKind::Terminal)
                        && handle_pane_wheel(state, sessions, pane, inner, &mouse)
                    {
                        *dirty = true;
                    }
                } else if ui::workspace_section_at(view, state, mouse.column, mouse.row)
                    && let Some(rows) = ui::workspace_list_rows(view, state)
                    && let Some(direction) = vertical_wheel_direction(mouse.kind)
                    && update::scroll_workspace_list(state, direction, rows)
                {
                    *dirty = true;
                }
            }
            MouseEventKind::Moved => {
                // 浮层打开时悬停只服务菜单高亮，不触碰底层 hover 状态。
                if state.overlay.is_some() {
                    if let Some(Overlay::Menu(menu)) = state.overlay.as_ref() {
                        let layout = ui::overlay::menu_layout(*screen, menu);
                        if let Some(index) =
                            ui::widgets::menu::item_at(&layout, mouse.column, mouse.row)
                            && update::set_menu_selection(state, index)
                        {
                            *dirty = true;
                        }
                    }
                    return;
                }
                let hovering_toast = ui::toast::rect(state, view, rects, *screen, config)
                    .is_some_and(|toast| toast.contains((mouse.column, mouse.row).into()));
                if update::set_toast_hover(state, hovering_toast) {
                    *dirty = true;
                }
                let sidebar_hover =
                    !hovering_toast && ui::sidebar_boundary_at(view, mouse.column, mouse.row);
                let prompt_hover = !hovering_toast
                    && !sidebar_hover
                    && layout::resize_boundary_at(rects, mouse.column, mouse.row)
                        .is_some_and(|(_, right)| right == state.prompt.id());
                let mut hover_changed = update::set_sidebar_hover(state, sidebar_hover);
                hover_changed |= update::set_prompt_hover(state, prompt_hover);
                if hover_changed {
                    let shape = if sidebar_hover || prompt_hover {
                        crate::platform::PointerShape::EwResize
                    } else {
                        crate::platform::PointerShape::Default
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
            let layout = ui::overlay::menu_layout(screen, menu);
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
        None => {}
    }
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
) {
    match message {
        AppEvent::PaneOutput(id, bytes) => {
            let responses = update::feed_pane(state, id, &bytes);
            if !responses.is_empty() {
                write_to_pane(sessions, id, &responses);
            }
        }
        AppEvent::PaneExit(id) => update::mark_pane_exited(state, id),
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

#[cfg(test)]
mod tests;
