//! 事件层顺序回归：`sync_mention_root` 先于编辑命令执行，第一次按键就要出面板。

use super::*;
use crate::app::markdown::MentionEntry;

/// 打开条目编辑视图，在空白之后键入 `@` 并写入一条扫描候选。
fn state_with_mention_cache() -> AppState {
    let mut state = item_editing_state();
    state.workspaces[0].cwd = Some(std::path::PathBuf::from("/tmp/ws"));
    apply_editor(&mut state, EditorCommand::InsertChar(' '));
    apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("工作区 cwd 提供扫描根");
    assert!(state.prompt.apply_mention_entries(
        generation,
        vec![MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        }],
    ));
    assert!(state.prompt.mention().is_some());
    state
}

#[test]
fn first_keystroke_after_mention_confirm_opens_panel() {
    let mut state = state_with_mention_cache();
    assert!(state.prompt.mention_confirm());
    assert_eq!(state.prompt.text(), "nested content @src/app.rs ");
    assert!(state.prompt.mention().is_none());

    // 事件层顺序：先同步根（把 panel_suppressed 镜像进 MentionState），再插入字符。
    apply_editor(&mut state, EditorCommand::InsertChar('@'));

    assert!(
        state.prompt.mention().is_some(),
        "确认候选后的第一次 @ 必须当次打开面板"
    );
}

#[test]
fn arrow_after_mention_confirm_does_not_reopen_panel() {
    let mut state = state_with_mention_cache();
    assert!(state.prompt.mention_confirm());
    for _ in 0..4 {
        apply_editor(&mut state, EditorCommand::Left);
    }
    assert!(
        state.prompt.mention().is_none(),
        "纯移动只收敛已打开面板，不得把光标移进路径后重新开面板"
    );
}
