//! 唯一事件循环：tokio select 收敛键盘、PTY 输出与渲染调度。

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use crossterm::event::{
    Event as TerminalEvent, EventStream, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::app::actions::{Action, EditorCommand};
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneKind};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::config::Config;
use crate::input::{self, Routed};
use crate::layout::{self, PaneId};
use crate::pty::{PtyEvent, PtySession};
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
    initialize_prompt_width(tui, state, config)?;
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let mut sessions: HashMap<PaneId, PtySession> = HashMap::new();
    let mut events = EventStream::new();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    let mut last_rects: Vec<(PaneId, Rect)> = Vec::new();

    while !state.should_quit {
        // 会话按状态对齐：新建工作区的窗格在此启动，被移除工作区的会话在此终止。
        reconcile_sessions(state, &mut sessions, &sender);
        let geometry = current_geometry(tui, state, config)?;
        if geometry.rects != last_rects {
            update::resize_panes(state, &geometry.rects);
            resize_sessions(&mut sessions, &geometry.rects);
            last_rects.clone_from(&geometry.rects);
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

/// 启动时右栏宽度取主区可用宽度的一半，复刻旧 50% 分割的版面。
fn initialize_prompt_width(tui: &mut Tui, state: &mut AppState, config: &Config) -> io::Result<()> {
    let size = tui.terminal().size()?;
    let area = Rect::new(0, 0, size.width, size.height);
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
                    update::apply_overlay_key(state, key);
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
            *dirty = true;
        }
        TerminalEvent::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if state.overlay.is_some() {
                    handle_overlay_click(state, *screen, mouse.column, mouse.row);
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
                if let Some(target) =
                    ui::collapse_button_at(view, state.agents_collapsed, mouse.column, mouse.row)
                {
                    let action = match target {
                        ui::CollapseTarget::Sidebar => Action::ToggleSidebar,
                        ui::CollapseTarget::Prompt => Action::TogglePrompt,
                        ui::CollapseTarget::Agents => Action::ToggleAgents,
                    };
                    update::apply(action, state);
                    if update::set_prompt_hover(state, false) {
                        reset_pointer_shape();
                    }
                    *dirty = true;
                } else if let Some(index) =
                    ui::workspace_item_at(view, state, mouse.column, mouse.row)
                {
                    update::switch_workspace(state, index);
                    *dirty = true;
                } else if ui::add_workspace_button(view, state.agents_collapsed)
                    .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                {
                    update::create_workspace(state);
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
                        update::begin_selection(state, pane, row, col);
                        *dirty = true;
                    } else {
                        update::clear_selection(state);
                    }
                } else {
                    update::clear_selection(state);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // 菜单打开时右键不穿透：工作区项上重开，其余位置关闭。
                if matches!(state.overlay, Some(Overlay::Menu(_))) {
                    match ui::workspace_item_at(view, state, mouse.column, mouse.row) {
                        Some(index) => {
                            update::open_workspace_menu(state, index, (mouse.column, mouse.row))
                        }
                        None => update::close_overlay(state),
                    }
                    *dirty = true;
                    return;
                }
                // 重命名与关闭确认期间右键吞掉，不替换模态。
                if state.overlay.is_some() {
                    return;
                }
                if let Some(index) = ui::workspace_item_at(view, state, mouse.column, mouse.row) {
                    update::open_workspace_menu(state, index, (mouse.column, mouse.row));
                    *dirty = true;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if state.overlay.is_some() {
                    return;
                }
                if state.resizing_prompt {
                    let width =
                        ui::layout::prompt_width_at(view, mouse.column, config.min_pane_width);
                    update::drag_prompt(state, width);
                    *dirty = true;
                    return;
                }
                let Some(selection) = state.selection else {
                    return;
                };
                let pane = selection.pane();
                let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
                    return;
                };
                let inner = layout::pane_inner_rect(*rect);
                if inner.width == 0 || inner.height == 0 {
                    return;
                }
                let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
                let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
                update::drag_selection(state, pane, row, col);
                if pane == state.prompt.id() {
                    update::place_prompt_cursor(state, row, col);
                }
                *dirty = true;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if state.overlay.is_some() {
                    return;
                }
                if state.resizing_prompt {
                    update::end_prompt_resize(state);
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
                if let Some(text) = update::finish_selection(state) {
                    let anchor = state.selection.map(|selection| selection.pane());
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
                            Toast::new(ToastKind::Error, ui::text::TOAST_COPY_FAILED, anchor, now),
                        );
                    }
                }
                update::clear_selection(state);
                *dirty = true;
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if state.overlay.is_some() {
                    return;
                }
                if state.resizing_prompt
                    || state
                        .selection
                        .is_some_and(|selection| selection.is_dragging())
                {
                    return;
                }
                if let Some((pane, _)) = pane_at(rects, mouse.column, mouse.row)
                    && pane == state.prompt.id()
                {
                    update::clear_selection(state);
                    let direction = if mouse.kind == MouseEventKind::ScrollUp {
                        -1
                    } else {
                        1
                    };
                    update::scroll_prompt(state, direction);
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
                let hover = !hovering_toast
                    && layout::resize_boundary_at(rects, mouse.column, mouse.row)
                        .is_some_and(|(_, right)| right == state.prompt.id());
                if update::set_prompt_hover(state, hover) {
                    let shape = if hover {
                        crate::platform::PointerShape::EwResize
                    } else {
                        crate::platform::PointerShape::Default
                    };
                    set_pointer_shape(shape);
                    *dirty = true;
                }
            }
            _ => {}
        },
        TerminalEvent::Resize(_, _) => *dirty = true,
        _ => {}
    }
}

/// 浮层左键点击：菜单项执行命令，点击浮层外取消；点击浮层内空白不动作。
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
            let inside = ui::overlay::rename_shell(screen)
                .is_some_and(|shell| shell.area.contains((column, row).into()));
            if !inside {
                update::close_overlay(state);
            }
        }
        Some(Overlay::ConfirmClose(_)) => {
            let inside = ui::overlay::confirm_shell(screen)
                .is_some_and(|shell| shell.area.contains((column, row).into()));
            if !inside {
                update::close_overlay(state);
            }
        }
        None => {}
    }
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
