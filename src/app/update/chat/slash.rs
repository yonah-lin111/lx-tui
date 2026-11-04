//! `/` 斜杠命令与二级面板（`/model`、`/resume`、`/fork`、`/rename`）的分派与状态更新。

use crate::app::state::lx::slash::{SlashCommand, SlashPanel};
use crate::app::state::{AppState, InputHintLevel, ModelPicker, ModelPickerStage};
use crate::config::ai::ModelSelection;
use crate::db::chat::ModelSnapshot;
use crate::layout::PaneId;
use crate::ui::text;

use super::refresh_all_lx_status;
use super::session::{new_session, rename_session, resume_session, show_lx_hint};

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
        SlashPanel::Fork(panel) => {
            let Some(entry) = panel.active_entry() else {
                return true;
            };
            let before = entry.message_id.clone();
            let Some(session_id) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.session_id.clone())
            else {
                show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_FORK_NOTHING);
                return true;
            };
            let Some(workspace) = state.chat_session_key_for_pane(pane) else {
                show_lx_hint(
                    state,
                    pane,
                    InputHintLevel::Error,
                    text::LX_CHAT_NO_WORKSPACE,
                );
                return true;
            };
            // 二级面板是临时搜索框：选中即整框清空；源会话先保持绑定，等 DB 回执再切。
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.slash = None;
                target.lx.clear();
            }
            state
                .effects
                .push(crate::app::effects::Effect::ForkChatSession {
                    pane,
                    workspace,
                    session_id,
                    before,
                });
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
pub(crate) fn clear_lx_input(state: &mut AppState, pane: PaneId) {
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

/// 选择完成后写入窗格选择；已保存 session 同步 DB 三列与切换时间线；并写回配置默认（下次启动沿用）。
pub(crate) fn finish_model_selection(
    state: &mut AppState,
    pane: PaneId,
    selection: ModelSelection,
) {
    let models = state.models.clone();
    let workspace = state.chat_session_key_for_pane(pane);
    // 窗格真实 cwd 必须在可变借用窗格之前取：持久化层用它探测该目录（worktree/子目录）
    // 独有的 `AGENTS.md`，不能用主仓库根替代。
    let pane_cwd = state.workspace_cwd_for_pane(pane);
    let normalized = models.normalize_selection(selection.clone());
    if let Some(target) = state.pane_mut_anywhere(pane) {
        let previous = target.lx.chat.effective_selection(&models).or_else(|| {
            target
                .lx
                .chat
                .selection
                .as_ref()
                .map(|sel| models.normalize_selection(sel.clone()))
        });
        target.lx.slash = None;
        target.lx.chat.selection = Some(selection.clone());
        // 切模型时旧分子清空。
        target.lx.chat.input_tokens_reset();

        if let Some(session_id) = target.lx.chat.session_id.clone() {
            let model_snapshot = ModelSnapshot {
                provider: normalized.provider.clone(),
                model: normalized.model.clone(),
                variant: normalized.variant.clone(),
            };
            let has_messages = !target.lx.chat.messages.is_empty();
            let changed = previous.as_ref() != Some(&normalized);
            let commit = if has_messages && changed {
                let from_snapshot = previous.as_ref().map(|prev| ModelSnapshot {
                    provider: prev.provider.clone(),
                    model: prev.model.clone(),
                    variant: prev.variant.clone(),
                });
                let from_label = previous.as_ref().map(|prev| {
                    let name = models.model_display_name(&prev.provider, &prev.model);
                    match prev.variant.as_deref().filter(|v| !v.is_empty()) {
                        Some(variant) => format!("{name} ({variant})"),
                        None => name,
                    }
                });
                let to_label = models.model_display_name(&normalized.provider, &normalized.model);
                let supports_image_input =
                    crate::app::state::FrozenModel::resolve(&models, &normalized)
                        .is_some_and(|frozen| frozen.supports_image_input);
                let to_identity = crate::chat::agent::ModelIdentity {
                    display_name: to_label.clone(),
                    provider_id: normalized.provider.clone(),
                    model_id: normalized.model.clone(),
                    variant: normalized.variant.clone(),
                    supports_image_input,
                };
                let updated_sources = crate::chat::agent::compute_updated_sources(
                    from_snapshot.as_ref(),
                    &model_snapshot,
                    false,
                );
                let injected_prompt = crate::chat::agent::render_model_switch_update(
                    from_label.as_deref(),
                    &to_identity,
                    &updated_sources,
                    &[],
                );
                let next_seq = next_pane_sequence(&target.lx);
                target
                    .lx
                    .model_switch
                    .switches
                    .push(crate::app::state::ModelSwitchNoticeView {
                        id: crate::db::new_external_id("msw"),
                        sequence: next_seq,
                        from_provider: from_snapshot.as_ref().map(|s| s.provider.clone()),
                        from_model: from_snapshot.as_ref().map(|s| s.model.clone()),
                        from_variant: from_snapshot.as_ref().and_then(|s| s.variant.clone()),
                        to_provider: model_snapshot.provider.clone(),
                        to_model: model_snapshot.model.clone(),
                        to_variant: model_snapshot.variant.clone(),
                        from_label: from_label.clone(),
                        to_label: to_label.clone(),
                        updated_sources: updated_sources.clone(),
                        injected_prompt: injected_prompt.clone(),
                        created_at_ms: crate::db::now_ms(),
                    });
                Some(crate::db::chat::ModelSwitchCommit {
                    session_id: session_id.clone(),
                    model: model_snapshot.clone(),
                    from_label,
                    to_label,
                    updated_sources,
                    injected_prompt,
                    cwd: pane_cwd,
                    to_identity,
                })
            } else {
                None
            };
            if let Some(workspace) = workspace {
                state
                    .effects
                    .push(crate::app::effects::Effect::SetChatSessionModel {
                        workspace,
                        session_id,
                        model: model_snapshot,
                        commit,
                    });
            }
        }
    }
    // 持久化：写回 `ai.json` defaultModel（关闭重开仍是这个模型）。
    persist_default_model(state, &selection);
    super::refresh_lx_status(state, pane);
}

pub(crate) fn next_pane_sequence(lx: &crate::app::state::Lx) -> i64 {
    let max_msg = lx
        .chat
        .messages
        .iter()
        .enumerate()
        .map(|(idx, m)| m.sequence.max(idx as i64))
        .max();
    let max_notice = lx.chat.notices.iter().map(|n| n.sequence).max();
    let max_compact = lx.compact.checkpoints.iter().map(|c| c.sequence).max();
    let max_switch = lx.model_switch.switches.iter().map(|s| s.sequence).max();
    [max_msg, max_notice, max_compact, max_switch]
        .into_iter()
        .flatten()
        .max()
        .map_or(0, |max| max.saturating_add(1))
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
        SlashCommand::Fork => super::fork::open_fork_panel(state, pane),
        SlashCommand::New => new_session(state, pane),
        SlashCommand::Rewind => super::rewind::open_from_command(state, pane),
        SlashCommand::Compact => super::compact::compact_command(state, pane),
        SlashCommand::Subagents => super::session::open_subagents_page(state, pane),
        SlashCommand::Rename => {
            if let Some(target) = state.pane_mut_anywhere(pane)
                && let Some(prefix) = command.echo_prefix()
            {
                target.lx.replace_input(prefix);
                target.lx.slash = None;
                return true;
            }
            false
        }
    }
}

/// 尝试将当前输入框草稿作为 `/rename`（或别名 `/title`）命令提交。
///
/// 返回 `true` 表示草稿是 `/rename` 命令（无论成功改名还是因空参/无会话被拦截），
/// 调用方 `try_send` 应立即返回，绝不将其作为普通消息发给模型。
pub fn try_submit_rename_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some((input, session_id)) = state.pane_anywhere(pane).and_then(|target| {
        if target.view != crate::app::state::PaneView::Lx || target.lx.shell_mode() {
            return None;
        }
        Some((target.lx.input.clone(), target.lx.chat.session_id.clone()))
    }) else {
        return false;
    };
    let Some(title) = crate::app::state::lx::slash::parse_rename_command(&input) else {
        return false;
    };
    if title.is_empty() {
        // 用户未输入参数直接回车：保持 `/rename ` 回显与 `[name]` 提示，弹 Toast 提醒。
        if let Some(target) = state.pane_mut_anywhere(pane)
            && let Some(prefix) = SlashCommand::Rename.echo_prefix()
        {
            if target.lx.input != prefix {
                target.lx.replace_input(prefix);
            } else {
                target.lx.cursor = prefix.len();
                target.lx.clear_text_selection();
            }
            target.lx.slash = None;
        }
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_RENAME_EMPTY_NAME,
        );
        return true;
    }
    let Some(session_id) = session_id else {
        clear_lx_input(state, pane);
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_RENAME_NO_SESSION,
        );
        return true;
    };
    if !rename_session(state, pane, &session_id, title) {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    }
    clear_lx_input(state, pane);
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Warning,
        text::chat::session_renamed(title),
    );
    true
}

/// 草稿是「行首 `/` 开头的一行命令」但**不是**任何内置命令时，返回那段命令文本。
///
/// 判定口径与命令面板一致（复用 `slash_trigger` 与 `SlashPanel::commands_matching`），
/// 保证「面板里能选中的」永远不会被这里拦下：
/// - 只有**单行**命令算数：多行草稿可能是用户在写 markdown 引用或路径；
/// - **带空格一律不拦**：`/q a`、`/path/to/file.rs 说明` 这种是普通输入，不是命令；
/// - 围栏代码块内不拦（`slash_trigger` 已经处理）。
///
/// 返回值保留用户输入的原文（含 `/`），由 `text::lx_unknown_command` 负责截断与拼句。
pub(super) fn unknown_slash_command(draft: &str) -> Option<String> {
    use crate::app::state::lx::slash::slash_trigger;
    // 只处理「整段就是一行」的草稿。
    let line = draft.trim();
    if line.contains('\n') {
        return None;
    }
    // 与面板同口径：光标在末尾时的触发前缀。
    let query = slash_trigger(line, line.len())?;
    let trimmed = query.trim();
    // 单独一个 `/` 说明用户还没敲完；不拦（面板此时是完整列表）。
    if trimmed == "/" {
        return None;
    }
    // **带空格一律不拦**（`/q a`、`/path/to/file.rs 说明`、`/etc/hosts 是什么`）：
    // 用户在敲命令时不会带空格（面板采纳的是一条无参命令），带空格说明这是
    // 普通输入（路径、引用、`/` 开头的自然语言），按内容原样发送。
    let name = trimmed.trim_start_matches('/');
    if name.is_empty() || name.contains(char::is_whitespace) || name.contains('/') {
        return None;
    }
    // 面板能匹配到就交给面板（用户可能想选中后 Enter）。
    if !SlashPanel::commands_matching(&format!("/{name}")).is_empty() {
        return None;
    }
    // 回显整段原文（含参数），让用户确认自己敲了什么。
    Some(trimmed.to_string())
}
