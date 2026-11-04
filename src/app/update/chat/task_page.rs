//! 后台任务页（Task page）的交互与纯状态更新。
//!
//! 在所在窗格内整页显示，并请求事件层回投进程列表。
//! 页面挂在 `Lx::task` 上而非浮层：`pane.lx` 的会话、消息、草稿、滚动全部原样保留，
//! `Esc` 返回即回到离开时的 lx 页。

use crate::app::state::AppState;
use crate::layout::PaneId;

/// 打开**后台任务页**：在所在窗格内整页显示，并请求事件层回投进程列表。
///
/// 与 `/resume` 同款数据流：`app/` 不持有进程句柄，先建页面骨架、再发效应。
/// 页面挂在 `Lx::task` 上而非浮层：`pane.lx` 的会话、消息、草稿、滚动全部原样
/// 保留，`Esc` 返回即回到离开时的 lx 页——打开它不会清理任何 lx 页状态。
/// 打开后台任务面板（`↓` 落空时进入）；返回是否打开。
///
/// **没有绑定会话时拒绝打开**：面板列的是「本会话的后台进程」，而进程运行实例挂在
/// 会话上（`ChatStreams::runtimes`）。未绑定会话的空白草稿既没有可列的进程，也
/// 发不出有效的 `ListSessionProcesses`——开了只会显示一个永远空着的列表，
/// 让人误以为"后台什么都没跑"。此时直接返回 false，按键不产生任何副作用。
pub fn open_task_page(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 未绑定会话：不打开、不发效应。
    if target.lx.chat.session_id.is_none() {
        return false;
    }
    // 已在面板上：再次进入只刷新列表，不重置 tab 与高亮。
    let freshly_opened = target.lx.task.is_none();
    if freshly_opened {
        target.lx.task = Some(crate::app::state::TaskPage::default());
    }
    if freshly_opened {
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    state
        .effects
        .push(crate::app::effects::Effect::ListSessionProcesses { pane });
    true
}

/// 从卡片的 `[open]` 打开后台任务面板，并把高亮**定位到该进程**。
///
/// `session_id` 为空（卡片没有关联进程）时退化为普通打开（高亮保持当前值）。
/// 进程已经结束的降级由 [`crate::app::state::TaskPage::resolve_focus`] 负责：
/// 请求落地时若找不到该会话就回落列表首项，面板照常打开。
pub fn open_task_page_at(state: &mut AppState, pane: PaneId, session_id: Option<u64>) -> bool {
    let opened = open_task_page(state, pane);
    if let Some(session_id) = session_id
        && let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(page) = target.lx.task.as_mut()
    {
        // 先登记请求。列表**已经**在时（面板本来就开着）当场落地；列表还没到时
        // 必须留着，等 `on_process_listed` 回投后再落地——否则一次性请求会被
        // 「此刻列表为空」吃掉，高亮永远落在首项。
        page.focus = Some(session_id);
        page.resolve_focus();
    }
    opened
}

/// 关闭后台任务页，回到 lx 页（`Esc`）；返回是否确实关闭了。
pub fn close_task_page(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let closed = target.lx.task.take().is_some();
    if closed {
        // 清掉拖拽抓取标记：面板没了还留着 grab，会让后续**所有**拖拽事件
        // （窗格分割线、选区、prompt 滚动条）都被输出区滚动条那一支吞掉。
        state.task_scroll_drag = None;
        state.task_agents_scroll_drag = None;
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    closed
}

/// 鼠标点击面板 tab：直接切到该 tab（`position` 为 `TaskTab::ALL` 的索引）。
pub fn select_task_tab(state: &mut AppState, pane: PaneId, position: usize) -> bool {
    let Some(tab) = crate::app::state::TaskTab::ALL.get(position).copied() else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    let changed = page.select_tab(tab);
    if changed {
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    changed
}

/// 后台进程列表回投：填充后台任务页的进程 tab。
///
/// 只收**仍在运行**的进程：已退出的没有可停的东西，列出来只会让用户误以为
/// 还有服务在跑。空列表照样回投——页面保持打开并显示空态，用户能明确看到
/// "没有在跑的"。
pub fn on_process_listed(
    state: &mut AppState,
    pane: PaneId,
    processes: Vec<crate::app::state::ProcessEntry>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    let changed = page.set_processes(processes, crate::db::now_ms());
    // 列表到了才落地后台任务页的一次性定位请求（见 `open_task_page_at`）。
    page.resolve_focus();
    changed
}

/// 回投高亮任务的输出尾部（与进程列表同一次快照，保证上下同步）。
///
/// 内容没变返回 false（不置脏、不重绘）。内容变了**不**无条件贴尾：只在原本
/// 贴尾（跟随最新）时继续跟随，用户手动上翻过就保持阅读位置——否则面板每轮
/// 重拉一次输出就会把上翻的用户拽回底部（用户报过的 bug）。
pub fn on_task_output(state: &mut AppState, pane: PaneId, output: Vec<String>) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    page.set_output(output)
}

/// 滚动后台任务面板的 item 列表视口（鼠标滚轮）；返回视口是否移动。
pub fn scroll_task_list(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let agents_len = target.lx.agents.runs.len();
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    // 可见行数与渲染的 `list` 矩形一致：`LIST_MAX` 是固定高度。
    match page.tab {
        crate::app::state::TaskTab::Processes => {
            page.scroll_window(delta, crate::ui::main_content::lx::task::LIST_MAX)
        }
        crate::app::state::TaskTab::Agents => page.scroll_agents_window(
            delta,
            crate::ui::main_content::lx::task::LIST_MAX,
            agents_len,
        ),
    }
}

/// 直接设置输出视口顶部偏移（滚动条轨道点击/thumb 拖拽）；返回是否变化。
pub fn set_task_output_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    else {
        return false;
    };
    let next = offset.min(page.output_max_scroll());
    if page.output_scroll == next {
        return false;
    }
    page.output_scroll = next;
    true
}

/// 滚动后台任务面板的输出视口（滚轮/翻页共用）；返回视口是否移动。
pub fn scroll_task_output(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    else {
        return false;
    };
    page.scroll_output(delta)
}

/// 停止后台任务页高亮的那个进程（`^d`）。
///
/// 只发效应，不改页面：进程真正退出后由终态事件把它从列表里摘掉，页面因此
/// 不会出现"点了没反应"或"已经停了还挂着"的错觉。
pub fn stop_selected_process(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(process_session_id) = target
        .lx
        .task
        .as_ref()
        .and_then(|page| page.selected())
        .map(|process| process.session_id)
    else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::StopSessionProcess {
            pane,
            process_session_id,
        });
    true
}
