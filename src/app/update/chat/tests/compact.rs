//! `/compact` 的状态测试：空闲门禁、冻结请求、成功/失败回填与撤销选择。

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::{PaneView, RequestState};
use crate::app::update::pane::apply_lx_input;
use crate::db::chat::{COMPACT_ACTIVE, COMPACT_REVERTED, CompactCheckpoint};

fn checkpoint(id: &str, sequence: i64, state: &str) -> CompactCheckpoint {
    CompactCheckpoint {
        id: id.into(),
        sequence,
        operation_id: format!("op-{id}"),
        base_checkpoint_id: None,
        start_sequence: 0,
        end_sequence: sequence - 1,
        summary_text: "summary text".into(),
        model_label: "gpt-5".into(),
        state: state.into(),
        created_at_ms: 1_700_000_640_000,
    }
}

/// 绑定会话的 lx 窗格（空闲、有模型）。
fn ready_compact_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.chat.session_id = Some("sess-compact".into());
    target.lx.chat.revision = 7;
    (state, pane)
}

/// 唯一一条压缩效应。
fn compact_effects(state: &AppState) -> Vec<&crate::app::effects::CompactRequest> {
    state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            crate::app::effects::Effect::ExecuteCompact(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// 空闲时 `/compact` 冻结请求：revision、模型与幂等键都在发起时确定。
#[test]
fn compact_freezes_request_when_idle() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .replace(vec![crate::app::state::CompactNoticeView::from_record(
            &checkpoint("c1", 4, COMPACT_ACTIVE),
        )]);
    assert!(compact_command(&mut state, pane));
    let requests = compact_effects(&state);
    assert_eq!(requests.len(), 1, "只入队一次压缩");
    let request = requests[0];
    assert_eq!(request.session_id, "sess-compact");
    assert_eq!(request.expected_revision, 7);
    assert_eq!(request.base_checkpoint_id.as_deref(), Some("c1"));
    assert!(!request.model_label.is_empty());
    assert!(!request.operation_id.is_empty());
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .is_running(),
        "执行期间必须标记为压缩中"
    );
}

/// 流式回复中拒绝，且不入队任何压缩请求。
#[test]
fn compact_is_rejected_while_streaming() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.request = RequestState::Streaming {
            key: crate::app::state::RequestKey {
                pane,
                session: "sess-compact".into(),
                generation: 1,
            },
            assistant_id: "a1".into(),
        };
    }
    assert!(compact_command(&mut state, pane));
    assert!(compact_effects(&state).is_empty(), "忙碌时不得入队");
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .is_running()
    );
}

/// 已有一个压缩在跑时再次 `/compact` 被拒绝。
#[test]
fn compact_is_rejected_while_compacting() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .running = Some("op-running".into());
    assert!(compact_command(&mut state, pane));
    assert!(compact_effects(&state).is_empty());
}

/// 没有绑定会话时拒绝（没有可压缩的上下文）。
#[test]
fn compact_requires_a_session() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    state.pane_mut_anywhere(pane).expect("pane").view = PaneView::Lx;
    assert!(compact_command(&mut state, pane));
    assert!(compact_effects(&state).is_empty());
}

/// 成功后回填检查点：展示实际模型名、清执行态、跟随底部。
#[test]
fn success_backfills_checkpoints_and_model_label() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .running = Some("op".into());
    let checkpoints = vec![
        checkpoint("c1", 4, COMPACT_ACTIVE),
        checkpoint("c2", 6, COMPACT_REVERTED),
    ];
    assert!(on_compact_finished(
        &mut state,
        pane,
        "sess-compact",
        checkpoints
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.compact.checkpoints.len(), 2);
    assert_eq!(
        target
            .lx
            .compact
            .active()
            .map(|item| item.model_label.as_str()),
        Some("gpt-5"),
        "必须展示实际摘要模型"
    );
    assert!(!target.lx.compact.is_running());
    assert!(target.lx.chat.follow_bottom);
    // 默认折叠。
    assert!(!target.lx.compact.is_expanded("c1"));
}

/// 失败：清执行态、保留已有检查点与展开状态（不写检查点）。
#[test]
fn failure_keeps_all_state() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView::from_record(
                &checkpoint("c1", 4, COMPACT_ACTIVE),
            )]);
        target.lx.compact.toggle("c1");
        target.lx.compact.running = Some("op".into());
    }
    assert!(on_compact_failed(&mut state, pane, "network"));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.compact.checkpoints.len(), 1);
    assert!(target.lx.compact.is_expanded("c1"), "失败不得改变折叠状态");
    assert!(!target.lx.compact.is_running());
}

/// 迟到回执：窗格已切到别的会话时不覆盖。
#[test]
fn late_receipt_for_another_session_is_ignored() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .running = Some("op".into());
    assert!(!on_compact_finished(
        &mut state,
        pane,
        "other-session",
        vec![checkpoint("c1", 4, COMPACT_ACTIVE)]
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.compact.checkpoints.is_empty());
    assert!(
        target.lx.compact.is_running(),
        "迟到的回执不能清掉新执行的标记"
    );
}

/// `/compact` 出现在命令面板并可直接采纳（不写入草稿）。
#[test]
fn compact_is_a_slash_command() {
    let matches = crate::app::state::SlashPanel::commands_matching("/comp");
    assert_eq!(
        matches.first(),
        Some(&crate::app::state::SlashCommand::Compact)
    );
    assert!(
        crate::app::state::SlashCommand::ALL
            .iter()
            .any(|command| command.name() == "compact")
    );
    // 帮助描述非空且指向压缩语义。
    let description = crate::app::state::SlashCommand::Compact.description();
    assert!(description.to_lowercase().contains("compact"));
}

/// 会话重载替换检查点：保留仍存在的展开项。
#[test]
fn reload_replaces_checkpoints() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .replace(vec![crate::app::state::CompactNoticeView::from_record(
            &checkpoint("c1", 4, COMPACT_ACTIVE),
        )]);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .toggle("c1");
    assert!(set_compact_checkpoints(
        &mut state,
        pane,
        &[
            checkpoint("c1", 4, COMPACT_ACTIVE),
            checkpoint("c2", 8, COMPACT_REVERTED)
        ]
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.compact.checkpoints.len(), 2);
    assert!(target.lx.compact.is_expanded("c1"));
}

/// 展开/折叠检查点条目（默认折叠，点击或 Enter 才展开）。
#[test]
fn toggle_expands_a_checkpoint() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .replace(vec![crate::app::state::CompactNoticeView::from_record(
            &checkpoint("c1", 4, COMPACT_ACTIVE),
        )]);
    assert!(toggle_checkpoint(&mut state, pane, "c1"));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .is_expanded("c1")
    );
    assert!(
        !toggle_checkpoint(&mut state, pane, "missing"),
        "未知条目不变"
    );
}

/// compact 撤销请求冻结检查点 id 与 revision，且不创建文件恢复请求。
#[test]
fn revert_request_is_context_only() {
    let (mut state, pane) = ready_compact_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .replace(vec![crate::app::state::CompactNoticeView::from_record(
            &checkpoint("c1", 4, COMPACT_ACTIVE),
        )]);
    let (effect, target) = compact_revert_request(&state, pane).expect("request");
    match effect {
        crate::app::effects::Effect::RevertCompact(request) => {
            assert_eq!(request.checkpoint_id, "c1");
            assert_eq!(request.expected_revision, 7);
        }
        other => panic!("expected compact revert, got {other:?}"),
    }
    assert_eq!(target.kind, crate::app::state::RewindTargetKind::Compact);
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::PrepareRewind { .. })),
        "compact 撤销不得请求文件快照预检"
    );
}

/// 没有任何 active 检查点时不能发 compact 撤销。
#[test]
fn revert_request_requires_active_checkpoint() {
    let (state, pane) = ready_compact_state();
    assert!(compact_revert_request(&state, pane).is_none());
}

/// 撤销成功：回填状态、提示并清掉撤销面板。
#[test]
fn revert_success_backfills_state() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView::from_record(
                &checkpoint("c1", 4, COMPACT_ACTIVE),
            )]);
        target.lx.rewind.selected = Some(crate::app::state::RewindTarget {
            session_id: "sess-compact".into(),
            user_message_id: "c1".into(),
            expected_revision: 7,
            pane,
            input_at_open: String::new(),
            user_text: String::new(),
            kind: crate::app::state::RewindTargetKind::Compact,
        });
    }
    assert!(on_compact_reverted(
        &mut state,
        pane,
        "sess-compact",
        vec![checkpoint("c1", 4, COMPACT_REVERTED)]
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.rewind.selected.is_none());
    assert!(!target.lx.compact.checkpoints[0].active);
}

/// `/compact` 通过输入路由可被直接采纳：命令文本不进入草稿。
#[test]
fn slash_compact_is_accepted_without_typing() {
    let (mut state, pane) = ready_compact_state();
    state.pane_mut_anywhere(pane).expect("pane").lx.input = "/compact".into();
    state.pane_mut_anywhere(pane).expect("pane").lx.cursor = "/compact".len();
    state.pane_mut_anywhere(pane).expect("pane").lx.slash =
        Some(crate::app::state::SlashPanel::Commands {
            items: vec![crate::app::state::SlashCommand::Compact],
            window: crate::app::state::PanelWindow::new(0),
        });
    apply_lx_input(&mut state, LxInput::Enter);
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "",
        "命令文本必须被消费"
    );
    assert_eq!(compact_effects(&state).len(), 1);
}

/// 撤销顺序：compact 之后的新普通轮次先撤销；没有更晚轮次才撤销 compact。
#[test]
fn rewind_selection_prefers_the_newer_of_turn_and_checkpoint() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![
            user_message_with_sequence("u1", "old", 0),
            assistant_message_with_sequence("a1", 1),
            user_message_with_sequence("u2", "newer", 4),
            assistant_message_with_sequence("a2", 5),
        ];
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView::from_record(
                &checkpoint("c1", 2, COMPACT_ACTIVE),
            )]);
    }
    // 更晚的普通轮次（sequence 4）胜出。
    assert!(crate::app::update::chat::rewind::select_latest(&mut state, pane).is_some());
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(selected.kind, crate::app::state::RewindTargetKind::Turn);
    assert_eq!(selected.user_message_id, "u2");
}

/// 没有更晚的普通轮次时，选中最近的 compact 检查点。
#[test]
fn rewind_selection_falls_back_to_the_checkpoint() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![
            user_message_with_sequence("u1", "old", 0),
            assistant_message_with_sequence("a1", 1),
        ];
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView::from_record(
                &checkpoint("c1", 2, COMPACT_ACTIVE),
            )]);
    }
    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(selected.kind, crate::app::state::RewindTargetKind::Compact);
    assert_eq!(selected.user_message_id, "c1");
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::PrepareRewind { .. })),
        "compact 选中不得请求文件快照预检"
    );
}

/// compact 撤销面板只有两项（恢复 + 取消），不提供文件动作。
#[test]
fn compact_panel_has_context_only_actions() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![
            user_message_with_sequence("u1", "old", 0),
            assistant_message_with_sequence("a1", 1),
        ];
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView::from_record(
                &checkpoint("c1", 2, COMPACT_ACTIVE),
            )]);
    }
    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    // Enter 一次：选中态 → 面板。
    apply_lx_input(&mut state, LxInput::Enter);
    let Some(crate::app::state::SlashPanel::Rewind(panel)) =
        state.pane_anywhere(pane).expect("pane").lx.slash.clone()
    else {
        panic!("expected rewind panel");
    };
    assert_eq!(
        panel.actions,
        vec![
            crate::app::state::RewindAction::RestoreCompact,
            crate::app::state::RewindAction::Cancel
        ]
    );
    assert_eq!(panel.len(), 2);
}

/// 连续多次 compact：撤销最新的一条后，active 指向上一个检查点。
#[test]
fn consecutive_compact_rewinds_step_back_one_checkpoint() {
    let (mut state, pane) = ready_compact_state();
    let first =
        crate::app::state::CompactNoticeView::from_record(&checkpoint("c1", 2, COMPACT_ACTIVE));
    let second =
        crate::app::state::CompactNoticeView::from_record(&checkpoint("c2", 6, COMPACT_ACTIVE));
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .replace(vec![first, second]);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .active()
            .map(|item| item.id.as_str()),
        Some("c2")
    );
    // 第一次撤销：c2 变为 reverted。
    assert!(on_compact_reverted(
        &mut state,
        pane,
        "sess-compact",
        vec![
            checkpoint("c1", 2, COMPACT_ACTIVE),
            checkpoint("c2", 6, COMPACT_REVERTED),
        ]
    ));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .active()
            .map(|item| item.id.as_str()),
        Some("c1"),
        "撤销后 active 必须回退到上一个检查点"
    );
    // 第二次撤销：c1。
    assert!(on_compact_reverted(
        &mut state,
        pane,
        "sess-compact",
        vec![
            checkpoint("c1", 2, COMPACT_REVERTED),
            checkpoint("c2", 6, COMPACT_REVERTED),
        ]
    ));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .active()
            .is_none(),
        "全部撤销后没有 active 检查点（回到完整历史）"
    );
}

/// compact 之后新消息：撤销顺序先普通轮次，再 compact。
#[test]
fn new_messages_after_compact_are_rewound_first() {
    let (mut state, pane) = ready_compact_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![
            user_message_with_sequence("u1", "before", 0),
            assistant_message_with_sequence("a1", 1),
            user_message_with_sequence("u2", "after compact", 6),
            assistant_message_with_sequence("a2", 7),
        ];
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView::from_record(
                &checkpoint("c1", 2, COMPACT_ACTIVE),
            )]);
    }
    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(
        selected.kind,
        crate::app::state::RewindTargetKind::Turn,
        "compact 之后的普通轮次必须先撤销"
    );
}

/// 构造一条用户消息视图（指定 sequence）。
fn user_message_with_sequence(
    id: &str,
    text: &str,
    sequence: i64,
) -> crate::app::state::ChatMessageView {
    crate::app::state::ChatMessageView {
        sequence,
        id: id.into(),
        kind: "chat".into(),
        shell: None,
        role: "user".into(),
        status: "complete".into(),
        turn_index: sequence / 2,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: text.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

/// 构造一条 assistant 消息视图（指定 sequence）。
fn assistant_message_with_sequence(id: &str, sequence: i64) -> crate::app::state::ChatMessageView {
    crate::app::state::ChatMessageView {
        sequence,
        id: id.into(),
        kind: "chat".into(),
        shell: None,
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: sequence / 2,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}
