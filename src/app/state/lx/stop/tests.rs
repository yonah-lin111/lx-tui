use super::*;

#[test]
fn defaults_to_stop_all_and_keeps_five_actions() {
    let panel = StopPanel::new(StopAvailability {
        main: true,
        subagents: 3,
        shells: 1,
        processes: 2,
    });
    assert_eq!(panel.len(), 5);
    assert_eq!(panel.active_action(), StopAction::All);
    assert!(panel.enabled(StopAction::All));
}

#[test]
fn availability_disables_missing_targets() {
    let subagents_only = StopAvailability {
        main: false,
        subagents: 2,
        shells: 0,
        processes: 0,
    };
    let panel = StopPanel::new(subagents_only);
    assert!(!panel.enabled(StopAction::Main));
    assert!(panel.enabled(StopAction::Subagents));
    assert!(!panel.enabled(StopAction::Shells));
    assert!(panel.enabled(StopAction::All));

    let shells_only = StopAvailability {
        shells: 1,
        ..StopAvailability::default()
    };
    let panel = StopPanel::new(shells_only);
    assert!(!panel.enabled(StopAction::Main));
    assert!(!panel.enabled(StopAction::Subagents));
    assert!(panel.enabled(StopAction::Shells));

    let processes_only = StopAvailability {
        processes: 1,
        ..StopAvailability::default()
    };
    assert!(StopPanel::new(processes_only).enabled(StopAction::Shells));
}

#[test]
fn move_skips_disabled_actions_and_wraps() {
    let mut panel = StopPanel::new(StopAvailability {
        main: false,
        subagents: 2,
        shells: 0,
        processes: 0,
    });
    // All -> Subagents（Main 禁用跳过）。
    assert!(panel.move_active(1));
    assert_eq!(panel.active_action(), StopAction::Subagents);
    // Subagents -> Cancel（Shells 禁用跳过）。
    assert!(panel.move_active(1));
    assert_eq!(panel.active_action(), StopAction::Cancel);
    // Cancel -> All 循环。
    assert!(panel.move_active(1));
    assert_eq!(panel.active_action(), StopAction::All);
    // 反向同样跳过禁用项。
    assert!(panel.move_active(-1));
    assert_eq!(panel.active_action(), StopAction::Cancel);
}

#[test]
fn hover_rejects_disabled_action() {
    let mut panel = StopPanel::new(StopAvailability {
        main: true,
        subagents: 0,
        shells: 0,
        processes: 0,
    });
    assert!(!panel.set_active(2));
    assert!(panel.set_active(1));
    assert_eq!(panel.active_action(), StopAction::Main);
    assert!(!panel.set_active(9));
}

#[test]
fn availability_any_requires_work() {
    assert!(!StopAvailability::default().any());
    assert!(
        StopAvailability {
            processes: 1,
            ..StopAvailability::default()
        }
        .any()
    );
}
