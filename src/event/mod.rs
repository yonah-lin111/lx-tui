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

use crate::app::actions::Action;
use crate::app::state::PaneKind;
use crate::app::{state::AppState, update};
use crate::config::Config;
use crate::input::{self, InputState, Routed};
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
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let mut sessions = spawn_sessions(state, &sender);
    let mut events = EventStream::new();
    let mut keyboard = InputState::default();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    let mut last_rects: Vec<(PaneId, Rect)> = Vec::new();

    while !state.should_quit {
        let (rects, area) = current_geometry(tui, state, config)?;
        if rects != last_rects {
            update::resize_panes(state, &rects);
            resize_sessions(&mut sessions, &rects, state.active_tab().layout.collapsed());
            last_rects.clone_from(&rects);
            dirty = true;
        }

        if dirty && last_draw.elapsed() >= FRAME_INTERVAL {
            tui.terminal()
                .draw(|frame| ui::render(frame, state, config))?;
            dirty = false;
            last_draw = Instant::now();
        }

        let wait = if dirty {
            FRAME_INTERVAL.saturating_sub(last_draw.elapsed())
        } else {
            IDLE_WAIT
        };

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => {
                    handle_terminal_event(event, state, &mut keyboard, &mut sessions, &rects, area, config, &mut dirty);
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

/// 为全部窗格启动 PTY；单个失败不阻断其余窗格。
fn spawn_sessions(
    state: &AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> HashMap<PaneId, PtySession> {
    let mut sessions = HashMap::new();
    for id in state.all_pane_ids() {
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
    sessions
}

/// 当前终端尺寸下活动标签的窗格几何与全屏区域。
fn current_geometry(
    tui: &mut Tui,
    state: &AppState,
    config: &Config,
) -> io::Result<(Vec<(PaneId, Rect)>, Rect)> {
    let size = tui.terminal().size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    let view = ui::layout::compute(area, config, state.sidebar_collapsed);
    Ok((
        layout::pane_rects(&state.active_tab().layout, view.panes),
        area,
    ))
}

/// 随几何变化同步 PTY 窗口尺寸；折叠窗格保持原尺寸。
fn resize_sessions(
    sessions: &mut HashMap<PaneId, PtySession>,
    rects: &[(PaneId, Rect)],
    collapsed: Option<PaneId>,
) {
    for (id, rect) in rects {
        if Some(*id) == collapsed {
            continue;
        }
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
    keyboard: &mut InputState,
    sessions: &mut HashMap<PaneId, PtySession>,
    rects: &[(PaneId, Rect)],
    area: Rect,
    config: &Config,
    dirty: &mut bool,
) {
    match event {
        TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
            let mode = state
                .active_pane()
                .map(|pane| pane.terminal.mode())
                .unwrap_or_else(TermMode::empty);
            let Some(routed) = input::route(key, keyboard, mode) else {
                return;
            };
            match routed {
                Routed::Action(action) => {
                    update::apply(action, state, rects);
                    *dirty = true;
                }
                Routed::Pane(bytes) => {
                    let id = state.active_tab().layout.focus();
                    write_to_pane(sessions, id, &bytes);
                    *dirty = true;
                }
                Routed::Consumed => {}
            }
        }
        TerminalEvent::Paste(text) => {
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
                if let Some(target) =
                    ui::collapse_button_at(state, config, area, mouse.column, mouse.row)
                {
                    let action = match target {
                        ui::CollapseTarget::Sidebar => Action::ToggleSidebar,
                        ui::CollapseTarget::Prompt => Action::TogglePrompt,
                    };
                    update::apply(action, state, rects);
                    *dirty = true;
                } else if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
                    let selectable = matches!(
                        state.pane_anywhere(pane).map(|pane| pane.kind),
                        Some(PaneKind::Terminal | PaneKind::Prompt)
                    );
                    if selectable {
                        update::begin_selection(
                            state,
                            pane,
                            mouse.row - inner.y,
                            mouse.column - inner.x,
                        );
                        *dirty = true;
                    } else {
                        update::clear_selection(state);
                    }
                } else {
                    update::clear_selection(state);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
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
                *dirty = true;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(text) = update::finish_selection(state)
                    && !crate::platform::write_clipboard(&text)
                {
                    tracing::warn!("clipboard write failed");
                }
                update::clear_selection(state);
                *dirty = true;
            }
            _ => {}
        },
        TerminalEvent::Resize(_, _) => *dirty = true,
        _ => {}
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
