//! 唯一事件循环：tokio select 收敛键盘、PTY 输出与渲染调度。

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use crossterm::event::{Event as TerminalEvent, EventStream, KeyEventKind};
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::app::state::Mode;
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
        let rects = current_rects(tui, state, config)?;
        if rects != last_rects {
            update::resize_panes(state, &rects);
            resize_sessions(&mut sessions, &rects);
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
                    handle_terminal_event(event, state, &mut keyboard, &mut sessions, &rects, &mut dirty);
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
        let size = state
            .pane_anywhere(id)
            .map(|pane| pane.terminal.size())
            .unwrap_or(crate::terminal::GridSize { cols: 80, rows: 24 });
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

/// 当前终端尺寸下活动标签的窗格几何。
fn current_rects(
    tui: &mut Tui,
    state: &AppState,
    config: &Config,
) -> io::Result<Vec<(PaneId, Rect)>> {
    let size = tui.terminal().size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    let view = ui::layout::compute(area, config, state.sidebar_collapsed);
    Ok(layout::pane_rects(&state.active_tab().layout, view.panes))
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
    keyboard: &mut InputState,
    sessions: &mut HashMap<PaneId, PtySession>,
    rects: &[(PaneId, Rect)],
    dirty: &mut bool,
) {
    match event {
        TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
            let overlay = state.mode == Mode::Help;
            let mode = state
                .active_pane()
                .map(|pane| pane.terminal.mode())
                .unwrap_or_else(TermMode::empty);
            let Some(routed) = input::route(key, keyboard, mode, overlay) else {
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
