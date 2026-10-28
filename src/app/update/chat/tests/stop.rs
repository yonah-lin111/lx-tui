//! Esc 双击停止与流中时间戳测试。

use super::*;

#[test]
fn escape_first_press_arms_second_press_stops() {
    let (mut state, pane, key) = streaming_state("sess-esc");
    state.effects.clear();
    // 首次 Esc：只待命并提示，不取消请求。
    assert!(escape_stop(&mut state, pane), "首次 Esc 生效（待命）");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_some(),
        "记录待命时刻"
    );
    assert!(state.effects.is_empty(), "首次 Esc 不取消请求");
    // 窗口内第二次 Esc：真正停止。
    assert!(escape_stop(&mut state, pane), "二次 Esc 停止");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_none(),
        "停止后清除待命"
    );
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::CancelChatStream { key: k } if *k == key)),
        "发出取消流效应"
    );
}

#[test]
fn escape_is_swallowed_when_idle() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(!escape_stop(&mut state, pane), "空闲吞掉 Esc");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_none()
    );
}

#[test]
fn escape_after_window_rearms_without_stopping() {
    let (mut state, pane, _key) = streaming_state("sess-esc-window");
    state.effects.clear();
    // 待命已超窗口：再按只重新待命（不停止）。
    state.pane_mut_anywhere(pane).unwrap().lx.chat.stop_armed_at = Some(expired_arm());
    assert!(escape_stop(&mut state, pane));
    assert!(state.effects.is_empty(), "过期待命不停止");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_some(),
        "重新待命"
    );
}

#[test]
fn expired_stop_arm_is_cleared_by_loading_tick() {
    let (mut state, pane, _key) = streaming_state("sess-esc-expire");
    state.pane_mut_anywhere(pane).unwrap().lx.chat.stop_armed_at = Some(expired_arm());
    assert!(advance_lx_loading(&mut state), "流中转轮推进");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_none(),
        "提示随窗口超时消失"
    );
}

#[test]
fn stop_stream_clears_armed_state() {
    let (mut state, pane, _key) = streaming_state("sess-esc-clear");
    state.pane_mut_anywhere(pane).unwrap().lx.chat.stop_armed_at = Some(std::time::Instant::now());
    assert!(stop_stream(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.chat.stop_armed_at.is_none());
    assert!(target.lx.chat.main_stopped, "显式停止主轮后标记 stopped");
}

#[test]
fn stream_parts_record_start_and_end_timestamps() {
    let (mut state, pane, key) = streaming_state("sess-part-time");
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::Part(PartDelta {
            kind: PartKind::Reasoning,
            text: "思考".into(),
            metadata: None,
        })],
    );
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::Part(PartDelta {
            kind: PartKind::Reasoning,
            text: "继续".into(),
            metadata: None,
        })],
    );
    let target = state.pane_anywhere(pane).unwrap();
    let part = target.lx.chat.messages[1].parts().next().expect("part");
    assert_eq!(part.content, "思考继续", "同 kind 合并");
    let started = part.started_at_ms.expect("起点时刻");
    let ended = part.ended_at_ms.expect("终点时刻");
    assert!(started > 0, "毫秒时刻有效");
    assert!(ended >= started, "终点不早于起点");
}
