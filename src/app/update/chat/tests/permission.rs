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

#[test]
fn apply_sandbox_changed_refreshes_open_permission_pickers() {
    let mut state = ready_state("");
    let pane_default = focus_pane(&state);
    let pane_override = state
        .active_tab_mut()
        .split_pane(pane_default, ratatui::layout::Direction::Horizontal)
        .expect("split");
    assert_ne!(pane_default, pane_override);

    if let Some(target) = state.pane_mut_anywhere(pane_override) {
        target.lx.sandbox_mode = Some(SandboxMode::DangerFullAccess);
    }
    assert!(super::super::slash::open_permission_picker(
        &mut state,
        pane_default
    ));
    assert!(super::super::slash::open_permission_picker(
        &mut state,
        pane_override
    ));

    let mut updated = state.sandbox.clone();
    updated.mode = SandboxMode::ReadOnly;
    assert!(crate::app::update::apply_sandbox_changed(
        &mut state,
        updated.clone()
    ));
    assert_eq!(state.sandbox, updated);

    let Some(SlashPanel::Permission(picker_default)) = state
        .pane_anywhere(pane_default)
        .and_then(|target| target.lx.slash.as_ref())
    else {
        panic!("default picker expected");
    };
    assert_eq!(picker_default.global, SandboxMode::ReadOnly);
    assert_eq!(picker_default.effective(), SandboxMode::ReadOnly);
    assert_eq!(picker_default.current, 0);

    let Some(SlashPanel::Permission(picker_override)) = state
        .pane_anywhere(pane_override)
        .and_then(|target| target.lx.slash.as_ref())
    else {
        panic!("override picker expected");
    };
    assert_eq!(picker_override.global, SandboxMode::ReadOnly);
    assert_eq!(picker_override.effective(), SandboxMode::DangerFullAccess);
    assert_eq!(picker_override.current, 2);
}

#[test]
fn sync_session_sandbox_mode_and_set_sandbox_rules_update_matching_panes() {
    use crate::db::sandbox::{SandboxRuleKind, SessionSandboxRule};

    let mut state = ready_state("");
    let pane_a = focus_pane(&state);
    let pane_b = state
        .active_tab_mut()
        .split_pane(pane_a, ratatui::layout::Direction::Horizontal)
        .expect("split");
    for pane in [pane_a, pane_b] {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-shared".into());
        target.lx.sandbox_exact_rules = vec![vec!["ls".into()]];
        target.lx.sandbox_prefix_rules = vec![vec!["git".into()]];
    }

    assert!(crate::app::update::sync_session_sandbox_mode(
        &mut state,
        "sess-shared",
        Some(SandboxMode::ReadOnly),
        true
    ));
    for pane in [pane_a, pane_b] {
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.sandbox_mode, Some(SandboxMode::ReadOnly));
        assert!(target.lx.sandbox_exact_rules.is_empty());
        assert!(target.lx.sandbox_prefix_rules.is_empty());
    }

    let rules = vec![
        SessionSandboxRule {
            kind: SandboxRuleKind::Exact,
            tokens: vec!["touch".into(), "/tmp/x".into()],
        },
        SessionSandboxRule {
            kind: SandboxRuleKind::Prefix,
            tokens: vec!["cargo".into(), "publish".into()],
        },
    ];
    assert!(crate::app::update::set_sandbox_rules(
        &mut state, pane_a, &rules
    ));
    assert!(!crate::app::update::set_sandbox_rules(
        &mut state, pane_a, &rules
    ));
    let target_a = state.pane_anywhere(pane_a).expect("pane_a");
    assert_eq!(
        target_a.lx.sandbox_exact_rules,
        vec![vec!["touch".to_string(), "/tmp/x".to_string()]]
    );
    assert_eq!(
        target_a.lx.sandbox_prefix_rules,
        vec![vec!["cargo".to_string(), "publish".to_string()]]
    );
}

#[test]
fn entry_and_ai_web_site_switch_reset_session_sandbox_state() {
    use crate::app::state::{AiWebSite, ApprovalState, LxEntry, PermissionSwitchNoticeView};
    use crate::chat::sandbox::{SandboxApprovalKind, SandboxApprovalRequest};

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-native".into());
        target.lx.sandbox_mode = Some(SandboxMode::ReadOnly);
        target.lx.sandbox_exact_rules = vec![vec!["echo".into()]];
        target.lx.sandbox_prefix_rules = vec![vec!["cargo".into()]];
        target.lx.approvals.push_back(ApprovalState::new(
            RequestKey {
                pane,
                session: "sess-native".into(),
                generation: 1,
            },
            "call-1".into(),
            SandboxApprovalRequest {
                kind: SandboxApprovalKind::Escalation,
                command: "echo hi".into(),
                cwd: "/ws".into(),
                justification: "need".into(),
                prefix_rule: None,
                additional: None,
                denial_excerpt: None,
                source: None,
            },
        ));
        target
            .lx
            .permission_switch
            .switches
            .push(PermissionSwitchNoticeView {
                id: "psw-1".into(),
                sequence: 1,
                from_mode: "workspace-write".into(),
                to_mode: "read-only".into(),
                injected_prompt: "switched".into(),
                created_at_ms: 1_000,
            });
    }

    crate::app::update::set_lx_entry(&mut state, pane, LxEntry::AiWeb);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.sandbox_mode, None);
    assert!(target.lx.sandbox_exact_rules.is_empty());
    assert!(target.lx.sandbox_prefix_rules.is_empty());
    assert!(target.lx.approvals.is_empty());
    assert!(target.lx.permission_switch.is_empty());

    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.sandbox_mode = Some(SandboxMode::DangerFullAccess);
        target.lx.sandbox_exact_rules = vec![vec!["x".into()]];
    }
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.sandbox_mode, None);
    assert!(target.lx.sandbox_exact_rules.is_empty());
    assert!(target.lx.permission_switch.is_empty());
}
