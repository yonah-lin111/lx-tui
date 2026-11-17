//! `/permission`：第 13 个 slash 命令、三项候选、选中写入会话并 toast。

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::{SlashCommand, SlashPanel};
use crate::config::sandbox::SandboxMode;

#[test]
fn permission_is_the_thirteenth_command_with_aliases() {
    assert_eq!(SlashCommand::ALL.len(), 13);
    assert_eq!(SlashCommand::Permission.name(), "permission");
    assert_eq!(SlashCommand::Permission.text(), "/permission");
    assert!(SlashCommand::Permission.aliases().contains(&"sandbox"));
    assert_eq!(
        SlashPanel::commands_matching("sandbox"),
        vec![SlashCommand::Permission]
    );
}

#[test]
fn picker_opens_with_global_default_marked_and_highlighted() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(super::super::slash::open_permission_picker(
        &mut state, pane
    ));
    let Some(panel) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    else {
        panic!("permission picker");
    };
    let SlashPanel::Permission(picker) = &panel else {
        panic!("permission picker");
    };
    assert_eq!(picker.candidate_count(), 3);
    assert_eq!(picker.effective(), SandboxMode::WorkspaceWrite);
    assert_eq!(panel.current_index(), Some(1));
    assert_eq!(panel.window().active, 1, "打开时必须高亮当前生效档位");
}

#[test]
fn selecting_danger_writes_the_session_override_and_clears_input() {
    let mut state = ready_state("filter");
    let pane = focus_pane(&state);
    super::super::slash::open_permission_picker(&mut state, pane);
    // 初始高亮在 Workspace (1)；下移一格到 Full (2)。
    crate::app::update::apply_lx_input(&mut state, LxInput::Down);
    crate::app::update::apply_lx_input(&mut state, LxInput::Enter);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.sandbox_mode, Some(SandboxMode::DangerFullAccess));
    assert!(target.lx.slash.is_none());
    assert!(target.lx.input.is_empty(), "选中后清空输入搜索词");
    let toast = target.lx.toast.as_ref().expect("toast");
    assert!(toast.message.contains("Full"));
    assert!(toast.message.contains("next turn"));
}

#[test]
fn filtering_and_mid_session_switch_records_notice() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some("sess-1".into());
        target.lx.chat.messages.push(ChatMessageView {
            id: "m1".into(),
            sequence: 0,
            turn_index: 0,
            role: "user".into(),
            status: "complete".into(),
            kind: "chat".into(),
            shell: None,
            steps: Vec::new(),
            reports: Vec::new(),
            attachments: Vec::new(),
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
        });
    }
    super::super::slash::open_permission_picker(&mut state, pane);
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('r'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('e'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('a'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('d'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Enter);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.sandbox_mode, Some(SandboxMode::ReadOnly));
    assert_eq!(target.lx.permission_switch.switches.len(), 1);
    assert_eq!(target.lx.permission_switch.switches[0].to_mode, "read-only");
    assert!(
        target.lx.permission_switch.switches[0]
            .injected_prompt
            .contains("Read-only")
    );
}

#[test]
fn new_session_clears_the_override() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.sandbox_mode = Some(SandboxMode::ReadOnly);
    }
    assert!(super::super::slash::apply_slash_command(
        &mut state,
        pane,
        SlashCommand::New
    ));
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.sandbox_mode,
        None
    );
}
