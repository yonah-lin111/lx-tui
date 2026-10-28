//! PTY 会话对齐、窗口尺寸调整与进程 cwd / Agent 扫描。

use std::collections::HashMap;
use std::io;
use std::time::Duration;

use ratatui::layout::Rect;
use tokio::sync::mpsc;

use crate::app::state::{AppState, PaneKind, home_dir, workspace_label};
use crate::app::update;
use crate::config::Config;
use crate::detect::{STATE_TAIL_LINES, pane_agent_snapshot};
use crate::layout::{self, PaneId};
use crate::pty::{PtyEvent, PtySession};
use crate::tui::Tui;
use crate::ui;

use super::AppEvent;

/// PTY 输出后的 cwd 去抖检查延迟。
pub(crate) const CWD_CHECK_DELAY: Duration = Duration::from_millis(300);

/// PTY 输出后的 Agent 去抖扫描延迟；与 cwd 检查共用窗口。
pub(crate) const AGENT_CHECK_DELAY: Duration = Duration::from_millis(300);

/// 当前帧几何：整屏区域、窗格矩形与区域划分；渲染、命中测试与 PTY 尺寸同步共用。
pub(crate) struct Geometry {
    pub(crate) screen: Rect,
    pub(crate) rects: Vec<(PaneId, Rect)>,
    pub(crate) view: ui::layout::ViewLayout,
    /// 最近一帧的最终可见文本；鼠标文字选择与复制读它。
    pub(crate) screen_text: crate::ui::screen::ScreenText,
}

/// 当前终端尺寸下活动标签的窗格几何（含 prompt 栏）与屏幕区域划分。
///
/// 折叠的右列面板不参与命中、选择与尺寸同步，展开时按当前几何恢复；
/// 主区折叠时窗格 PTY 不重排（网格保持折叠前尺寸）。
pub(crate) fn current_geometry(
    tui: &mut Tui,
    state: &AppState,
    config: &Config,
) -> io::Result<Geometry> {
    let size = tui.terminal().size()?;
    let screen = Rect::new(0, 0, size.width, size.height);
    let view = ui::layout::compute(
        screen,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    // 折叠的右列面板不参与命中与尺寸同步：主区折叠时窗格 PTY 保持原尺寸。
    let mut rects = if state.prompt_left && state.panes_collapsed {
        Vec::new()
    } else {
        layout::pane_rects(
            &state.active_tab().layout,
            view.panes,
            config.min_pane_width,
            config.min_pane_height,
        )
    };
    if !state.prompt_collapsed {
        rects.push((state.prompt.id(), view.prompt));
    }
    Ok(Geometry {
        screen,
        rects,
        view,
        screen_text: crate::ui::screen::ScreenText::default(),
    })
}

/// 轮询窗格 shell 进程的 cwd：更新各窗格的 cwd 与标题标签；返回是否有变化。
///
/// 工作区路径在生命周期内不可变，不随窗格 cwd 漂移；窗格退出或读不到 cwd 时保持原值。
pub(crate) fn poll_process_cwds(
    state: &mut AppState,
    sessions: &HashMap<PaneId, PtySession>,
) -> bool {
    let home = home_dir();
    let mut changed = false;
    for id in state.all_pane_ids() {
        let Some(pid) = sessions.get(&id).and_then(PtySession::process_id) else {
            continue;
        };
        let Some(cwd) = crate::platform::process_cwd(pid) else {
            continue;
        };
        let label = workspace_label(&cwd, home.as_deref());
        if update::update_pane_cwd(state, id, &cwd, label) {
            changed = true;
        }
    }
    changed
}

/// 扫描当前激活工作区各终端窗格：前台进程定 Agent 身份、终端尾部行定状态；
/// 未识别或非终端窗格清空快照；返回是否有变化。
pub(crate) fn poll_active_agents(
    state: &mut AppState,
    sessions: &HashMap<PaneId, PtySession>,
) -> bool {
    let mut changed = false;
    for id in state.active_pane_ids() {
        let Some(pane) = state.pane_anywhere(id) else {
            continue;
        };
        if pane.kind != PaneKind::Terminal || pane.exited {
            continue;
        }
        let snapshot = sessions
            .get(&id)
            .and_then(PtySession::process_id)
            .and_then(crate::platform::foreground_process_name)
            .and_then(|name| {
                let lines = pane.terminal.tail_lines(STATE_TAIL_LINES);
                let title = pane.terminal.title().map(str::to_string);
                pane_agent_snapshot(&name, &lines, title.as_deref())
            });
        if update::update_pane_agent(state, id, snapshot) {
            changed = true;
        }
    }
    changed
}

/// 按状态对齐 PTY 会话：新增终端窗格启动会话，被移除或非终端的窗格终止会话；幂等。
pub(crate) fn reconcile_sessions(
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

/// 随几何变化同步 PTY 窗口尺寸。
pub(crate) fn resize_sessions(
    sessions: &mut HashMap<PaneId, PtySession>,
    rects: &[(PaneId, Rect)],
) {
    for (id, rect) in rects {
        let (cols, rows) = layout::pane_inner_size(*rect);
        if let Some(session) = sessions.get_mut(id)
            && let Err(error) = session.resize(cols, rows)
        {
            tracing::warn!(pane = id.raw(), %error, "pty resize failed");
        }
    }
}

/// 向指定窗格对应的 PTY 会话写入字节。
pub(crate) fn write_to_pane(sessions: &mut HashMap<PaneId, PtySession>, id: PaneId, bytes: &[u8]) {
    if let Some(session) = sessions.get_mut(&id)
        && let Err(error) = session.write(bytes)
    {
        tracing::error!(pane = id.raw(), %error, "pty write failed");
    }
}

/// 保证当前工作区在列表可见范围内（创建/切换/关闭后调用）。
pub(crate) fn ensure_workspace_visible(state: &mut AppState, view: &ui::layout::ViewLayout) {
    if let Some(rows) = ui::workspace_list_rows(view, state) {
        update::ensure_workspace_visible(state, rows);
    }
}

/// 保证激活标签在标签栏可见范围内（切换/创建/关闭后调用）。
pub(crate) fn reveal_active_tab(state: &mut AppState, view: &ui::layout::ViewLayout) {
    let workspace = state.active_workspace();
    let tab_bar = ui::tab_bar::layout(view, workspace, state.tab_scroll);
    let offset = ui::tab_bar::reveal_scroll(&tab_bar, workspace);
    let max = tab_bar.max_scroll;
    update::set_tab_scroll(state, offset, max);
}
