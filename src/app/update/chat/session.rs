//! 会话生命周期与 `/` 命令：`/new`、`/resume`、`/model`、重命名与删除。
//!
//! 只改纯状态、只入队效应；DB/HTTP 由事件层执行。展示历史一律由
//! [`super::view::view_message_of`] 把 DB 的 parts 与 tools 按 position 合并成唯一步骤时间线。

use crate::app::state::lx::slash::{SlashCommand, SlashPanel};
use crate::app::state::{AppState, InputHintLevel, ModelPicker, ModelPickerStage};
use crate::config::ai::ModelSelection;
use crate::db::chat::{ChatMessage, ModelSnapshot};
use crate::layout::PaneId;
use crate::ui::text;

use super::{refresh_all_lx_status, refresh_lx_status, show_lx_hint, stop_stream, view_message_of};

/// `/new`：解绑当前窗格会话；在途请求继续按原 session 完成写入。
pub fn new_session(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 旧请求仍在途时只解绑并取消其请求（写入继续按原 ID 完成）。
    if target.lx.chat.request.is_busy() {
        stop_stream(state, pane);
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.reset_session_view();
    target.lx.clear_agents();
    target.lx.compact.reset();
    target.lx.clear_rewind();
    refresh_lx_status(state, pane);
    true
}

/// `/model` 打开模型面板（第一级）。
pub fn open_model_picker(state: &mut AppState, pane: PaneId) -> bool {
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == crate::app::state::PaneView::Lx)
    {
        return false;
    }
    let candidates = ModelPicker::candidates(&state.models.settings);
    if candidates.is_empty() {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    }
    // 当前生效的模型（行首 `❯` 标记 + 底色）；也是打开时的定位目标。
    //
    // 必须走 `effective_selection`：窗格级 `chat.selection` 只在本进程选过模型时才有值，
    // 重启后是 None，此时真正生效的是配置里的 `defaultModel`。只读 `chat.selection`
    // 会让"重启后打开面板看不到当前模型"——重启前选的模型明明还在生效。
    let current = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.effective_selection(&state.models))
        .and_then(|selection| {
            candidates.iter().position(|candidate| {
                candidate.selection.provider == selection.provider
                    && candidate.selection.model == selection.model
            })
        });
    let active = current.unwrap_or(0);
    let candidates_len = candidates.len();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates,
        current,
        current_variant: None,
        // `PanelWindow::new(active)` 即"最小滚动"：窗口以高亮项为底向前回填，
        // 选中模型在候选靠后时贴底可见、靠前时贴顶可见，已可见时窗口不动。
        window: crate::app::state::lx::slash::PanelWindow::new(active),
        // 打开面板时**不过滤**：显示全量候选，之后随输入框内容变化才过滤。
        filter: String::new(),
        filtered: (0..candidates_len).collect(),
    })));
    true
}

/// `/resume` 打开历史面板（事件层提供当前工作区分页列表）。
pub fn open_history(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 列表由事件层按需加载（Effect::ListChatSessions）；先打开面板骨架。
    target.lx.slash = Some(SlashPanel::History(Box::default()));
    state
        .effects
        .push(crate::app::effects::Effect::ListChatSessions { pane });
    true
}

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
    if target.lx.task.is_none() {
        target.lx.task = Some(crate::app::state::TaskPage::default());
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
    page.select_tab(tab)
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

/// 历史列表回投：填充 `/resume` 面板。
///
/// 当前窗格已绑定的会话打上 `❯` 标记，并作为高亮与窗口定位目标
/// （`PanelWindow::new(active)` 的最小滚动语义：它在视口外时会被拉进视口）。
pub fn on_history_listed(
    state: &mut AppState,
    pane: PaneId,
    entries: Vec<crate::app::state::lx::slash::HistoryEntry>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let bound = target.lx.chat.session_id.clone();
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    panel.entries = entries;
    panel.current = bound.as_deref().and_then(|session_id| {
        panel
            .entries
            .iter()
            .position(|entry| entry.session_id == session_id)
    });
    panel.now_ms = crate::db::now_ms();
    // 打开面板时不过滤：恢复全量视图，再按当前过滤词重算（打开时为空的恒等视图）。
    panel.filtered = (0..panel.entries.len()).collect();
    panel.clamp_window();
    let active = panel
        .current
        .and_then(|index| panel.filtered.iter().position(|item| *item == index))
        .unwrap_or(0);
    panel.window = crate::app::state::lx::slash::PanelWindow::new(active);
    true
}

/// 恢复会话：绑定当前窗格并重建展示历史（DB 行转视图）。
pub fn resume_session(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    // 同一 session 已被另一窗格占用（正在生成）时拒绝。
    let occupied = state
        .all_pane_ids()
        .into_iter()
        .filter(|other| *other != pane)
        .any(|other| {
            state.pane_anywhere(other).is_some_and(|p| {
                p.lx.chat.session_id.as_deref() == Some(session_id) && p.lx.chat.request.is_busy()
            })
        });
    if occupied {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_SESSION_BUSY,
        );
        return true;
    }
    state
        .effects
        .push(crate::app::effects::Effect::LoadChatSession {
            pane,
            session_id: session_id.to_string(),
        });
    // 面板先行关闭；列表数据到达后填充。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
    }
    true
}

/// 会话加载回投：重建消息视图与模型选择。
///
/// 同时恢复**上下文占用**：按 codex 口径取最近一次带用量的 assistant 轮次
/// （其输入 + 输出 token 近似下一轮请求的输入），因此重开历史会话不再显示空上下文。
pub fn on_session_loaded(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    messages: &[ChatMessage],
    selection: Option<ModelSelection>,
) -> bool {
    on_session_loaded_full(state, pane, session_id, messages, &[], 0, selection)
}

/// 带独立撤销提示与会话 revision 的会话加载（生产入口）。
pub fn on_session_loaded_full(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    messages: &[ChatMessage],
    notices: &[crate::db::chat::NoticeRecord],
    revision: i64,
    selection: Option<ModelSelection>,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let session_changed = target.lx.chat.session_id.as_deref() != Some(session_id);
    if session_changed {
        // Agents 面板属于会话：不清理会把上一会话的 run、计数和 ^d 目标带过来。
        target.lx.clear_agents();
        target.lx.chat.main_stopped = false;
        target.lx.chat.stop_armed_at = None;
        if let Some(page) = target.lx.task.as_mut() {
            page.agents_window = Default::default();
            page.agents_focus = None;
        }
    }
    target.lx.chat.session_id = Some(session_id.to_string());
    // 切会话：先清空 compact 状态（检查点列表随后由加载回执回填）。
    target.lx.compact.reset();
    target.lx.clear_rewind();
    // 切换会话：未执行的命令与排队项不得漂移到新会话。
    target.lx.chat.pending_shells.clear();
    target.lx.chat.queue.clear();
    if let Some(selection) = selection {
        target.lx.chat.selection = Some(selection);
    }
    target.lx.chat.messages = messages
        .iter()
        .map(|message| view_message_of(message, &models))
        .collect();
    target.lx.chat.notices = notices
        .iter()
        .map(crate::app::state::RewindNoticeView::from_record)
        .collect();
    target.lx.chat.revision = u64::try_from(revision).unwrap_or(0);
    // 上下文占用：最近一条带用量的 assistant 轮次（同模型键由 refresh 校验）。
    let recent = messages.iter().rev().find(|message| {
        message.role == "assistant"
            && (message.input_tokens.is_some() || message.output_tokens.is_some())
    });
    target.lx.chat.last_tokens_model = recent.and_then(|message| {
        message
            .model_provider
            .as_deref()
            .zip(message.model_id.as_deref())
            .map(|(provider, model)| format!("{provider}/{model}"))
    });
    target.lx.chat.context_tokens = recent.map(|message| {
        let input = message.input_tokens.unwrap_or(0).max(0);
        let output = message.output_tokens.unwrap_or(0).max(0);
        u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX)
    });
    target.lx.chat.follow_bottom = true;
    target.lx.chat.thinking_toggled.clear();
    target.lx.chat.turn_started_at_ms = None;
    target.lx.chat.turn_elapsed_ms = None;
    // 展开状态与键盘选择按稳定 ID 定位；切会话后默认折叠、不选中任何卡片。
    target.lx.chat.detail_expanded.clear();
    target.lx.chat.report_expand_steps.clear();
    target.lx.chat.selected_tool = None;
    target.lx.chat.selected_compact = None;
    target.lx.chat.plan = None;
    // 计划：从本会话最后一条**成功**的 update_plan 结果恢复（失败/中断不覆盖）。
    // 先克隆视图，避免在可变借用期间再借 `state`。
    let restored_messages = target.lx.chat.messages.clone();
    refresh_lx_status(state, pane);
    super::restore_plan(state, pane, &restored_messages);
    // 历史会话：spawn 卡片的状态可从已加载的汇报行回填。
    super::subagent::refresh_subagent_cards(state, pane);
    true
}

/// 重命名会话。
pub fn rename_session(state: &mut AppState, pane: PaneId, session_id: &str, title: &str) -> bool {
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::RenameChatSession {
            workspace,
            session_id: session_id.to_string(),
            title: title.to_string(),
        });
    true
}

/// 删除会话（有确认；先取消在途请求，等最终写入后事务级删除）。
pub fn delete_session(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    // 在途请求先取消（事件层等 finish 后再删除）。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && target.lx.chat.session_id.as_deref() == Some(session_id)
    {
        stop_stream(state, pane);
        // 本地命令不占用 RequestState，必须单独停止；删除后迟到回执不得写回。
        super::user_shell::cancel_user_shells(state, pane);
    }
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::DeleteChatSession {
            workspace,
            session_id: session_id.to_string(),
        });
    true
}

/// 删除完成：解绑当前窗格（若正是该会话）。
pub fn on_session_deleted(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() == Some(session_id) {
        target.lx.chat.reset_session_view();
        target.lx.clear_agents();
        return true;
    }
    false
}

/// `/` 面板确认（Enter / Tab / 鼠标点击）：按当前视图与高亮项推进。
///
/// - 命令候选 → 消费命令文本并执行（`/model`、`/resume`、`/new`）。
/// - `/model` 第一级 → 有 variants 进入第二级，否则直接完成。
/// - `/model` 第二级 → 以高亮 variant 完成选择（真正写入下一轮选择）。
/// - `/resume` → 打开高亮会话。
pub fn apply_slash_enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    else {
        return false;
    };
    match panel {
        SlashPanel::Commands { .. } => crate::app::update::pane::accept_slash_command(state, pane),
        SlashPanel::Model(picker) => match &picker.stage {
            ModelPickerStage::Models => {
                // 高亮位置是**过滤视图**内的位置，采纳时换算回原始候选索引。
                let Some(candidate) = picker.active_candidate().cloned() else {
                    // 空态占位：吞掉 Enter，不误采纳。
                    return true;
                };
                let selection = candidate.selection;
                let variants = variant_keys(state, &selection);
                // 选中模型即清空过滤词：无论直接完成选择，还是换到第二级继续挑等级，
                // 输入框里那段临时搜索词都不再保留。
                clear_lx_input(state, pane);
                if variants.is_empty() {
                    finish_model_selection(state, pane, selection);
                } else {
                    let active = default_variant_index(state, &selection, &variants);
                    // 当前生效的 variant（第一级高亮的模型若正是当前模型，其 variant
                    // 就是"现在生效"的那一档）；否则第二级没有"已生效项"。
                    let current_variant = (picker.current == Some(picker.window.active))
                        .then(|| {
                            state
                                .models
                                .settings
                                .providers
                                .get(&selection.provider)
                                .and_then(|provider| provider.models.get(&selection.model))
                                .and_then(|model| model.variant.clone())
                                .and_then(|variant| {
                                    variants.iter().position(|item| *item == variant)
                                })
                        })
                        .flatten();
                    let active = current_variant.unwrap_or(active);
                    if let Some(target) = state.pane_mut_anywhere(pane)
                        && let Some(SlashPanel::Model(current)) = target.lx.slash.as_mut()
                    {
                        current.stage = ModelPickerStage::Variants {
                            model: selection,
                            variants,
                        };
                        current.current_variant = current_variant;
                        // 换级后候选集变了：过滤视图必须按新阶段重建，
                        // 否则沿用上一级的索引会把高亮指到不存在的项。
                        current.filtered = (0..current.stage_len()).collect();
                        current.filter.clear();
                        current.window = crate::app::state::lx::slash::PanelWindow::new(active);
                    }
                }
                true
            }
            ModelPickerStage::Variants { model, variants } => {
                let mut selection = model.clone();
                // 采纳高亮等级（此前只完成默认 variant，未应用用户选择）。
                // 同样按过滤视图换算回原始索引。
                selection.variant = picker
                    .active_stage_index()
                    .and_then(|index| variants.get(index).cloned());
                // 选中思考等级同样结束搜索：整框清空。
                clear_lx_input(state, pane);
                finish_model_selection(state, pane, selection);
                true
            }
        },
        SlashPanel::History(history) => {
            // 高亮位置是过滤视图内的位置，采纳时换算回原始条目。
            if let Some(entry) = history.active_entry() {
                let session_id = entry.session_id.clone();
                // 选中会话即结束这次搜索：整框清空。
                clear_lx_input(state, pane);
                resume_session(state, pane, &session_id);
            }
            true
        }
        // 撤销动作面板：第二次 Enter 才执行（第一次 Enter 只是打开面板）。
        SlashPanel::Rewind(_) => super::rewind::apply_enter(state, pane),
    }
}

/// 清空窗格输入框（整框清空，压一次 undo 快照）。
///
/// 二级面板（`/model`、`/resume`）打开期间输入框内容就是过滤词，属于**临时搜索输入**：
/// `Esc` 关闭、`↵` 选中模型/会话后都必须清掉，否则过滤词会残留在草稿里被一起发送。
///
/// `⇧↵` 打开会话编辑模态、`^C` 复制会话 id **不清空**：前者仍在同一面板流程内，
/// 后者要支持连续复制多条（清了输入框过滤结果会跳回全量）。
fn clear_lx_input(state: &mut AppState, pane: PaneId) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.clear();
    }
}

/// 模型默认 variant 在候选中的索引（缺失或不在列表则 0）。
fn default_variant_index(
    state: &AppState,
    selection: &ModelSelection,
    variants: &[String],
) -> usize {
    state
        .models
        .settings
        .providers
        .get(&selection.provider)
        .and_then(|provider| provider.models.get(&selection.model))
        .and_then(|model| model.variant.clone())
        .and_then(|variant| variants.iter().position(|item| *item == variant))
        .unwrap_or(0)
}

/// 选择完成后写入窗格选择；已保存 session 同步 DB 三列；并写回配置默认（下次启动沿用）。
pub(crate) fn finish_model_selection(
    state: &mut AppState,
    pane: PaneId,
    selection: ModelSelection,
) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
        target.lx.chat.selection = Some(selection.clone());
        // 切模型时旧分子清空。
        target.lx.chat.input_tokens_reset();
        // 已保存 session：选择动作排队写 DB。
        if let Some(session_id) = target.lx.chat.session_id.clone()
            && let Some(workspace) = state.chat_session_key_for_pane(pane)
        {
            state
                .effects
                .push(crate::app::effects::Effect::SetChatSessionModel {
                    workspace,
                    session_id,
                    model: ModelSnapshot {
                        provider: selection.provider.clone(),
                        model: selection.model.clone(),
                        variant: selection.variant.clone(),
                    },
                });
        }
    }
    // 持久化：写回 `ai.json` defaultModel（关闭重开仍是这个模型）。
    persist_default_model(state, &selection);
    refresh_lx_status(state, pane);
}

/// 模型的 variants 键列表。
fn variant_keys(state: &AppState, selection: &ModelSelection) -> Vec<String> {
    state
        .models
        .settings
        .providers
        .get(&selection.provider)
        .and_then(|provider| provider.models.get(&selection.model))
        .map(|model| model.variants.keys().cloned().collect())
        .unwrap_or_default()
}

/// 设置页模型配置保存后更新快照（在途请求继续持有旧快照到结束）。
pub fn apply_models_changed(
    state: &mut AppState,
    settings: &crate::config::ai::AiSettings,
) -> bool {
    let changed = state.models.settings != *settings;
    state.models.settings = settings.clone();
    // 立即更新所有窗格的上限/标签（旧模型分子清理由 refresh 内部处理）。
    refresh_all_lx_status(state);
    changed
}

/// 把窗格当前选择的模型写回 `ai.json` 的 `defaultModel`（下次启动沿用）。
///
/// 只在选择变化或首次采用默认时入队，避免重复落盘。
pub fn persist_default_model(state: &mut AppState, selection: &ModelSelection) -> bool {
    if state.models.settings.default_model.as_ref() == Some(selection) {
        return false;
    }
    state
        .effects
        .push(crate::app::effects::Effect::SaveDefaultModel {
            selection: selection.clone(),
        });
    // 快照同步更新：内存立即生效，避免同一会话内再次入队。
    state.models.settings.default_model = Some(selection.clone());
    true
}

/// 命令采纳：`/model`、`/resume`、`/new` 的命令文本不被发送。
pub fn apply_slash_command(state: &mut AppState, pane: PaneId, command: SlashCommand) -> bool {
    match command {
        SlashCommand::Model => open_model_picker(state, pane),
        SlashCommand::Resume => open_history(state, pane),
        SlashCommand::New => new_session(state, pane),
        SlashCommand::Rewind => super::rewind::open_from_command(state, pane),
        SlashCommand::Compact => super::compact::compact_command(state, pane),
    }
}
