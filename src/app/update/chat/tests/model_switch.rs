//! 会话模型切换状态机测试：空会话不留痕、非空会话产生时间线条目与点击展开切换。

use super::*;

#[test]
fn empty_session_model_switch_does_not_add_timeline_notice() {
    let mut state = ready_state("hello");
    let pane = focus_pane(&state);

    super::super::session::finish_model_selection(
        &mut state,
        pane,
        ModelSelection {
            provider: "q".into(),
            model: "n".into(),
            variant: None,
        },
    );

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(
        lx.model_switch.is_empty(),
        "未发送消息的空会话切模型不应产生时间线卡片"
    );
}

#[test]
fn active_session_model_switch_records_notice_and_toggles_card() {
    let mut state = ready_state("first message");
    let pane = focus_pane(&state);
    assert!(try_send(&mut state, pane));
    on_turn_committed(
        &mut state,
        pane,
        1,
        &TurnCommitted {
            session_id: "sess-switch".into(),
            user_message_id: "u-1".into(),
            assistant_message_id: "a-1".into(),
            turn_index: 0,
            user_sequence: 0,
            assistant_sequence: 1,
            created_at_ms: 1000,
            revision: 0,
            attachments: Vec::new(),
        },
    );

    super::super::session::finish_model_selection(
        &mut state,
        pane,
        ModelSelection {
            provider: "q".into(),
            model: "n".into(),
            variant: Some("max".into()),
        },
    );

    let switch_id = {
        let lx = &state.pane_anywhere(pane).expect("pane").lx;
        assert_eq!(lx.model_switch.switches.len(), 1);
        let sw = &lx.model_switch.switches[0];
        assert_eq!(sw.sequence, 2);
        assert_eq!(sw.to_provider, "q");
        assert_eq!(sw.to_model, "n");
        assert!(sw.injected_prompt.contains("# Your Model"));
        sw.id.clone()
    };

    // 点击卡片展开/折叠
    assert!(toggle_lx_card(
        &mut state,
        pane,
        CardTarget::ModelSwitch {
            switch_id: switch_id.clone()
        }
    ));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .model_switch
            .is_expanded(&switch_id)
    );
    assert!(toggle_lx_card(
        &mut state,
        pane,
        CardTarget::ModelSwitch {
            switch_id: switch_id.clone()
        }
    ));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .model_switch
            .is_expanded(&switch_id)
    );
}
