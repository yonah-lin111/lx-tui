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
fn picker_opens_with_global_default_marked() {
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
}

#[test]
fn selecting_danger_writes_the_session_override_and_toasts() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    super::super::slash::open_permission_picker(&mut state, pane);
    // 候选固定为 Read only / Workspace write / Danger full access。
    crate::app::update::apply_lx_input(&mut state, LxInput::Down);
    crate::app::update::apply_lx_input(&mut state, LxInput::Down);
    crate::app::update::apply_lx_input(&mut state, LxInput::Enter);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.sandbox_mode, Some(SandboxMode::DangerFullAccess));
    assert!(target.lx.slash.is_none());
    let toast = target.lx.toast.as_ref().expect("toast");
    assert!(toast.message.contains("danger-full-access"));
    assert!(toast.message.contains("next turn"));
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
