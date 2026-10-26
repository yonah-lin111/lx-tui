//! 后台任务页（进程 tab）：列表回投、停止、时长节拍与运行实例回收。
//!
//! 进程句柄只存在于事件层的运行实例里，`app/` 与 `db/` 都碰不到它们，因此这些
//! 操作只能在这里做。子 agent 页签的同类操作在 `subagent` 模块。

use super::*;

/// 执行进程类效应（后台任务页的列表与停止）。
///
/// 与 `pump_chat_streams` 同一处置方式：从 `state.effects` 取出自己关心的效应执行，
/// 其余的留给持久化泵/网络泵。进程句柄只在事件层的运行实例里，所以这两件事只能
/// 在这里做——`app/` 与 `db/` 都碰不到进程。
///
/// 返回是否产生状态变化。
pub fn apply_process_effects(state: &mut AppState, streams: &mut ChatStreams) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::ListSessionProcesses { .. } | Effect::StopSessionProcess { .. }
            )
        })
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state.effects.retain(|effect| {
        !matches!(
            effect,
            Effect::ListSessionProcesses { .. } | Effect::StopSessionProcess { .. }
        )
    });
    let mut changed = false;
    for effect in effects {
        match effect {
            Effect::ListSessionProcesses { pane } => {
                changed |= list_session_processes(state, streams, pane);
            }
            Effect::StopSessionProcess {
                pane,
                process_session_id,
            } => {
                stop_session_process(state, streams, pane, process_session_id);
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// 把每个 lx 窗格的会话后台任务数同步到底部状态行指标。
///
/// 每轮循环都算（`running_count` 只是一次加锁 + 计数），这样无论用户是否打开
/// 后台任务页，都能随时看到"还有几个服务在跑"——这正是此前"忘了清理"的根因：
/// 进程一直活着，界面上却没有任何痕迹。
///
/// 返回是否有窗格的计数发生变化（用于置脏重绘）。
pub fn sync_background_task_counts(state: &mut AppState, streams: &ChatStreams) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        let count = target
            .lx
            .chat
            .session_id
            .as_deref()
            .and_then(|session_id| streams.runtimes.get(session_id))
            .map(|runtime| runtime.manager().running_count())
            .unwrap_or(0);
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        if target.lx.background_tasks != count {
            target.lx.background_tasks = count;
            changed = true;
        }
    }
    changed
}

/// 面板打开时每轮重新拉取一次进程列表。
///
/// `ListSessionProcesses` 原来只在 `open_task_page` 时发一次——面板**打开之后**
/// 才启动的进程永远进不了列表（用户报的"启动任务后没有在面板中显示"）。
///
/// 改成每轮嗅探：只要某窗格的面板开着且绑定了会话，就发一次列表请求。列表回投
/// 是幂等的（`set_processes` 按内容比较），无变化不会置脏，因此不会引起重绘风暴。
///
/// 返回是否新压入了请求（供调用方判断要不要多跑一轮效应泵）。
pub fn refresh_open_task_pages(state: &mut AppState) -> bool {
    let mut requested = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        let Some(page) = target.lx.task.as_ref() else {
            continue;
        };
        let Some(session_id) = target.lx.chat.session_id.clone() else {
            continue;
        };
        if page.tab == crate::app::state::TaskTab::Agents {
            // 懒加载：高亮项还没有消息流时补一次（历史 run 只在选中后读库）。
            let pending_load = target
                .lx
                .agents
                .runs
                .get(page.agents_window.active)
                .filter(|run| !run.loaded)
                .map(|run| (run.agent_id.clone(), run.child_session_id.clone()));
            state
                .effects
                .push(crate::app::effects::Effect::ListSubagents {
                    pane,
                    parent_session: session_id,
                });
            if let Some((agent_id, child_session)) = pending_load {
                state
                    .effects
                    .push(crate::app::effects::Effect::LoadSubagentSession {
                        pane,
                        agent_id,
                        child_session,
                    });
            }
        } else {
            state
                .effects
                .push(crate::app::effects::Effect::ListSessionProcesses { pane });
        }
        requested = true;
    }
    requested
}

/// 后台任务面板的运行时长节拍：把各面板的 `now_ms` 推到当前时刻。
///
/// 面板每行右侧显示 `running 1m 30s`，而它是 `now_ms - started_at_ms` 现算的。
/// `now_ms` 原来只在**列表回投**时更新——进程不变就再也不会变，时长会冻住
/// （用户报的"时间不更新"）。
///
/// 每轮循环调一次（一次 `now_ms()` + 几个比较，成本可忽略）；只在面板打开时
/// 才追高 `now_ms`，返回值用于置脏重绘。按秒对齐，避免每帧都重绘。
pub fn tick_task_pages(state: &mut AppState) -> bool {
    let now = crate::db::now_ms();
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(page) = target.lx.task.as_mut() else {
            continue;
        };
        // 同一秒内不重绘：时长显示精度是秒。
        if now.saturating_sub(page.now_ms) >= 1_000 {
            page.now_ms = now;
            changed = true;
        }
    }
    changed
}

/// 列出某窗格所绑会话仍在运行的进程，回投给后台任务页的进程 tab。
///
/// 两条准入条件（任一成立即列出）：
///
/// 1. 进程本身还没到终态（`starting`/`running`）；
/// 2. **shell 已退出，但它拉起的后台作业还活着**（`background_alive`）。
///
/// 第 2 条是必须的：`nohup bash task.sh &` 之后 shell 立刻退出，用户的服务却还在
/// 跑。只看 `state` 会把它过滤掉，面板显示「no running processes」而任务明明活着
/// ——这正是用户报的情形（进程组仍存活，组长 shell 已退出）。
///
/// 已彻底结束（终态且组内无成员）的进程不列：没有可停的东西，列出来只会让用户
/// 以为还有服务在跑。窗格没有绑定会话（空白草稿）时回投空列表。
///
/// 会话切换后页面可能还开着：此时回投的是新会话的进程，页面据此整体翻新。
fn list_session_processes(state: &mut AppState, streams: &ChatStreams, pane: PaneId) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return update::on_process_listed(state, pane, Vec::new());
    };
    let snapshots: Vec<crate::chat::process::ProcessSnapshot> = streams
        .runtimes
        .get(&session_id)
        .map(|runtime| runtime.manager().snapshots())
        .unwrap_or_default()
        .into_iter()
        .filter(|snapshot| !snapshot.state.is_terminal() || snapshot.background_alive)
        .collect();
    // 高亮任务的输出尾部：与列表同一次回投，保证"上面选谁、下面看谁"永远同步。
    // `app/` 不持有进程句柄，所以输出也必须由这里切好行交过去。
    let active = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.selected().map(|process| process.session_id));
    let output = active
        .and_then(|session| {
            snapshots
                .iter()
                .find(|snapshot| snapshot.session_id == session)
        })
        .map(|snapshot| process_output_lines(&snapshot.tail))
        .unwrap_or_default();
    let entries = snapshots
        .into_iter()
        .map(|snapshot| crate::app::state::ProcessEntry {
            session_id: snapshot.session_id,
            command: snapshot.command,
            cwd: snapshot.cwd,
            tty: snapshot.tty,
            started_at_ms: snapshot.started_at_ms,
        })
        .collect();
    let listed = update::on_process_listed(state, pane, entries);
    let shown = update::on_task_output(state, pane, output);
    listed || shown
}

/// 进程输出尾部 → 展示行（保留中间空行；末尾换行不额外占行）。
///
/// 面板输出区**不做软换行**（终端日志按行读，换行会让行数与滚动错位），
/// 超宽由渲染层硬切，因此这里只需要按 `\n` 切分。
fn process_output_lines(tail: &str) -> Vec<String> {
    if tail.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = tail
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// 停止后台任务页选中的进程，并给出结果反馈。
///
/// 停止是异步的（真实退出码只能由 `wait` 得到）；这里不阻塞事件循环。
fn stop_session_process(
    state: &mut AppState,
    streams: &ChatStreams,
    pane: PaneId,
    process_session_id: u64,
) {
    let session_id = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone());
    let manager = session_id
        .as_deref()
        .and_then(|session_id| streams.runtimes.get(session_id))
        .map(|runtime| runtime.manager().clone());
    let Some(manager) = manager else {
        // 会话运行实例已经不在（刚切会话/刚退出）：页面上还挂着这一行，
        // 不能静默失败，否则用户会以为按了没生效。
        //
        // 用 toast 而不是 hint 行：这一页已经把输入框换成了控制面板。
        // 反馈落在该窗格自己的 toast 槽里（lx 页内），不占用全局覆盖层。
        update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Danger,
                crate::ui::text::LX_TASK_STOP_FAILED,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return;
    };
    tokio::spawn(async move {
        match manager.stop(process_session_id).await {
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    target: "lx::process",
                    process_session_id,
                    code = error.code,
                    "stopping a background process failed"
                );
            }
        }
    });
    // 停止是异步的，但"按了就要有反馈"。
    //
    // 这里只确认"已发出停止请求"；真正的结果由进程终态事件驱动列表刷新。
    // 用 Info 而不是 Error：这是用户主动要求的动作，不是出错。
    update::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Default,
            crate::ui::text::LX_TASK_STOPPED,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
}

/// 回收不再被任何窗格绑定的会话运行实例（`/new`、切换会话、关窗格）。
///
/// 只移除运行实例并**异步停止**其进程：事件循环不阻塞等待，回执由后台任务完成。
///
/// 停止完成后把可获取的终态快照经 [`AppEvent::ProcessesReaped`] 回投事件层落库。
/// 设计 §8 要求「关闭旧运行实例，清理其进程**并记录终态**」——只杀不记，库里会
/// 永远留着 `running` 行：重开该会话时卡片按记录判活显示 Running，而后台任务面板
/// 读的是内存运行实例（已随会话回收而空），两处对同一件事给出相反答案。
///
/// 返回是否回收了运行实例（后台任务计数随之变化，调用方需要重绘）。
pub(crate) fn reap_orphan_runtimes(
    state: &AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> bool {
    let bound: Vec<String> = state
        .all_pane_ids()
        .into_iter()
        .filter_map(|pane| state.pane_anywhere(pane))
        .filter_map(|pane| pane.lx.chat.session_id.clone())
        .collect();
    // 运行中的子会话同样「有主」：否则它的一轮工具进程会被下一次 sweep 杀掉。
    let mut bound = bound;
    bound.extend(
        streams
            .subagents
            .iter()
            .filter(|(_, handle)| handle.status == subagent::STATUS_RUNNING)
            .map(|(session, _)| session.clone()),
    );
    let orphans: Vec<String> = streams
        .runtimes
        .sessions()
        .into_iter()
        .filter(|session| !bound.contains(session))
        .collect();
    let mut reaped = false;
    for session in orphans {
        if let Some(runtime) = streams.runtimes.take(&session) {
            reaped = true;
            let manager = runtime.manager.clone();
            let workspace = manager.owner().workspace.clone();
            let epoch = runtime.epoch.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                manager.shutdown().await;
                // 停止之后才读快照：此时才是真正的终态（Stopped / Interrupted）。
                let snapshots = manager.snapshots();
                if snapshots.is_empty() {
                    return;
                }
                let _ = sender.send(AppEvent::ProcessesReaped {
                    workspace,
                    epoch,
                    snapshots,
                });
            });
        }
    }
    reaped
}
