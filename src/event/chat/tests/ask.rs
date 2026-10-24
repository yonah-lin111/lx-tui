//! `ask` 的事件层接线：归属校验、重复提问、取消唤醒、作答回执与迟到丢弃。
//!
//! 这里刻意绕开网络：`AgentHostEvent::AskUser` 的输入、`Effect::AnswerAsk` 的输出都在
//! 事件层就地可观测，`ChatStreams` 的回执通道就是唯一的真相来源。

use super::*;
use crate::chat::tools::ask::{AskOption, AskQuestion};

fn question(id: &str) -> AskQuestion {
    AskQuestion {
        id: id.into(),
        header: id.into(),
        question: format!("{id}?"),
        multi_select: false,
        options: vec![AskOption {
            label: "A".into(),
            description: "first".into(),
            preview: None,
        }],
    }
}

/// 发一次 AskUser 并返回回执（未完成时 `try_recv` 是空）。
fn request(
    state: &mut AppState,
    streams: &mut ChatStreams,
    key: RequestKey,
    call_id: &str,
) -> oneshot::Receiver<Result<crate::chat::tools::AskAnswers, HostError>> {
    let (reply, receiver) = oneshot::channel();
    handle_agent_host_event(
        state,
        streams,
        &Persistence::without_database(),
        AgentHostEvent::AskUser {
            key,
            call_id: call_id.into(),
            questions: vec![question("layout")],
            reply,
        },
    );
    receiver
}

/// 单个流中窗格（无 DB 参与：ask 不写库）。
fn streaming_state() -> (AppState, RequestKey) {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.models = models_with();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.entry = crate::app::state::LxEntry::Native;
    let key = RequestKey {
        pane,
        session: "sess-ask".into(),
        generation: 1,
    };
    target.lx.chat.session_id = Some(key.session.clone());
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: "a1".into(),
    };
    (state, key)
}

fn answers() -> crate::chat::tools::AskAnswers {
    let mut answers = crate::chat::tools::AskAnswers::new();
    answers.insert("layout".into(), vec!["A".into()]);
    answers
}

#[test]
fn ask_request_lands_in_pane_state_and_streams() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let _reply = request(&mut state, &mut streams, key.clone(), "call-1");
    let ask = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("面板已挂起");
    assert_eq!(ask.call_id, "call-1");
    assert_eq!(ask.key, key);
    assert_eq!(ask.questions.len(), 1);
    assert_eq!(streams.asks.len(), 1, "回执通道由 ChatStreams 持有");
}

#[test]
fn stale_generation_is_rejected_without_writing_ui() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    // 旧 generation：当前在途请求已经前进到 2。
    let mut stale = key.clone();
    stale.generation = 0;
    let mut reply = request(&mut state, &mut streams, stale, "call-old");
    assert!(matches!(reply.try_recv(), Ok(Err(HostError::Cancelled))));
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .ask
            .is_none(),
        "归属失效不得写面板"
    );
    assert!(streams.asks.is_empty(), "归属失效不得登记挂起项");
}

#[test]
fn missing_pane_is_rejected_without_panicking() {
    let (mut state, _key) = streaming_state();
    let mut streams = ChatStreams::default();
    let ghost = RequestKey {
        pane: crate::layout::PaneId::alloc(),
        session: "sess-ask".into(),
        generation: 1,
    };
    let mut reply = request(&mut state, &mut streams, ghost, "call-ghost");
    assert!(matches!(reply.try_recv(), Ok(Err(HostError::Cancelled))));
    assert!(streams.asks.is_empty());
}

#[test]
fn duplicate_ask_fails_and_keeps_the_first_panel() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let _first = request(&mut state, &mut streams, key.clone(), "call-1");
    let mut second = request(&mut state, &mut streams, key.clone(), "call-2");
    match second.try_recv() {
        Ok(Err(HostError::Failed { code })) => {
            assert_eq!(code, crate::event::chat::ask::ASK_BUSY_CODE);
        }
        other => panic!("重复提问必须失败，实际 {other:?}"),
    }
    let ask = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("第一个面板仍挂起");
    assert_eq!(ask.call_id, "call-1", "已有面板不得被覆盖");
}

#[test]
fn cancel_wakes_the_sender_and_clears_the_panel() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let (cancel_sender, _cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    let mut reply = request(&mut state, &mut streams, key.clone(), "call-1");
    assert!(streams.cancel(&key), "确有在途轮次");
    assert!(
        matches!(reply.try_recv(), Ok(Err(HostError::Cancelled))),
        "取消必须唤醒 ask_user"
    );
    assert!(crate::event::chat::ask::cancel(
        &mut state,
        &mut streams,
        &key
    ));
    assert!(streams.asks.is_empty());
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .ask
            .is_none(),
        "取消必须清面板"
    );
    // 再取消一次是空操作（不 panic、不重复发送）。
    assert!(!crate::event::chat::ask::cancel(
        &mut state,
        &mut streams,
        &key
    ));
}

#[test]
fn stale_answer_is_dropped_and_keeps_the_pending_ask() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let mut reply = request(&mut state, &mut streams, key.clone(), "call-1");
    let changed =
        crate::event::chat::ask::answer(&mut state, &mut streams, &key, "call-0", answers());
    assert!(!changed, "迟到的旧面板提交不得产生变化");
    assert_eq!(streams.asks.len(), 1, "当前挂起项必须保留");
    assert!(reply.try_recv().is_err(), "旧答案不得唤醒当前等待的 loop");
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .ask
            .is_some(),
        "旧答案不得清掉当前面板"
    );
}

#[test]
fn matching_answer_wakes_the_loop_and_clears_state() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let mut reply = request(&mut state, &mut streams, key.clone(), "call-1");
    let expected = answers();
    assert!(crate::event::chat::ask::answer(
        &mut state,
        &mut streams,
        &key,
        "call-1",
        expected.clone()
    ));
    match reply.try_recv() {
        Ok(Ok(received)) => assert_eq!(received, expected),
        other => panic!("作答必须回执答案，实际 {other:?}"),
    }
    assert!(streams.asks.is_empty(), "作答成功后不留挂起项");
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .ask
            .is_none()
    );
}

#[test]
fn stream_finish_sweeps_the_pending_ask() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let mut reply = request(&mut state, &mut streams, key.clone(), "call-1");
    let changed = handle_net_event(
        &mut state,
        &mut streams,
        ChatNetEvent::Finished {
            key: key.clone(),
            reason: StopReason::Cancelled,
            usage: Usage::default(),
        },
    );
    assert!(changed);
    assert!(streams.asks.is_empty(), "流结束不得留下悬挂 sender");
    assert!(matches!(reply.try_recv(), Ok(Err(HostError::Cancelled))));
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .ask
            .is_none()
    );
}

#[test]
fn pump_sweeps_the_panel_when_the_turn_is_no_longer_in_flight() {
    let (mut state, key) = streaming_state();
    let mut streams = ChatStreams::default();
    let (cancel_sender, _cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    let mut reply = request(&mut state, &mut streams, key.clone(), "call-1");
    // 会话切换：请求退回 Idle，但面板与挂起项都还在（模拟取消通道丢失）。
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .request = RequestState::Idle;
    let (sender, _receiver) = mpsc::unbounded_channel::<AppEvent>();
    let persistence = Persistence::without_database();
    assert!(crate::event::chat::pump_chat_streams(
        &mut state,
        &mut streams,
        &sender,
        &persistence
    ));
    assert!(streams.asks.is_empty(), "不得留下悬挂 sender");
    assert!(matches!(reply.try_recv(), Ok(Err(HostError::Cancelled))));
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .ask
            .is_none(),
        "不在途的轮次不得继续显示面板"
    );
}
