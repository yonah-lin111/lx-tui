//! 提示词树行为单元测试：视图切换、导航、菜单命令分发与编辑提交。

use super::*;
use crate::app::actions::EditorCommand;
use crate::app::overlay::{MenuCommand, Overlay, OverlayTarget};
use crate::app::state::AppState;
use crate::app::update::{apply_editor, apply_overlay_key};
use crate::db::{FolderRecord, PromptRecord};

/// 一个工作区 + 已载入的示例树：根文件夹 1（子文件夹 2），条目 11/12 在文件夹内，10 在根。
fn demo_state() -> AppState {
    let mut state = AppState::demo();
    let folders = vec![
        FolderRecord {
            id: 1,
            parent_id: None,
            name: "root".into(),
        },
        FolderRecord {
            id: 2,
            parent_id: Some(1),
            name: "sub".into(),
        },
    ];
    let prompts = vec![
        PromptRecord {
            id: 10,
            folder_id: None,
            name: "loose".into(),
            content: "loose content".into(),
            status: "todo".into(),
        },
        PromptRecord {
            id: 12,
            folder_id: Some(2),
            name: "nested".into(),
            content: "nested content".into(),
            status: "completed".into(),
        },
    ];
    apply_prompt_tree(&mut state, 0, folders, prompts);
    state.prompt.resize(30, 8);
    state
}

fn enter_tree(state: &mut AppState) {
    show_prompt_tree(state);
    assert!(prompt_tree_mode(state));
}

#[test]
fn apply_tree_loads_rows_and_marks_loaded() {
    let state = demo_state();
    assert!(state.workspaces[0].prompt_panel.loaded);
    let rows = state.workspaces[0].prompt_panel.rows();
    assert_eq!(rows.len(), 4, "root, sub, nested, loose");
}

#[test]
fn toggle_mode_and_keyboard_navigation() {
    let mut state = demo_state();
    // 默认编辑视图下 Esc 无绑定不切树。
    show_prompt_tree(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);

    // 上移到头 no-op；下移跳过可见行。
    assert!(!move_prompt_selection(&mut state, -1, 8));
    assert!(move_prompt_selection(&mut state, 1, 8));
    assert_eq!(state.workspaces[0].prompt_panel.selected, 1);
    assert!(move_prompt_selection(&mut state, 1, 8));
    assert_eq!(state.workspaces[0].prompt_panel.selected, 2);

    // Left/Right 折叠/展开选中文件夹。
    let mut state = demo_state();
    enter_tree(&mut state);
    let mut state = {
        // 选中子文件夹（行 1）后折叠。
        move_prompt_selection(&mut state, 1, 8);
        state
    };
    assert!(collapse_prompt_row(&mut state));
    let rows = state.workspaces[0].prompt_panel.rows();
    assert_eq!(rows.len(), 3, "子文件夹折叠隐藏 nested");
    assert!(expand_prompt_row(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.rows().len(), 4);
}

#[test]
fn enter_on_item_opens_editor_and_escape_returns() {
    let mut state = demo_state();
    enter_tree(&mut state);
    // 行序：root(0), sub(1), nested(2), loose(3)；选中 nested 并回车。
    move_prompt_selection(&mut state, 2, 8);
    assert!(activate_prompt_row(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(state.workspaces[0].prompt_panel.editing, Some(12));
    assert_eq!(state.prompt.text(), "nested content");
    assert!(state.prompt_focused);

    // Esc 返回树并保持绑定清除。
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
    assert_eq!(state.workspaces[0].prompt_panel.editing, None);
}

#[test]
fn typing_in_tree_mode_is_ignored_and_enter_toggles_folder() {
    let mut state = demo_state();
    enter_tree(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('x'));
    assert_eq!(state.prompt.text(), "", "树视图吞掉文本输入");
    // 选中根文件夹行回车 → 折叠。
    apply_editor(&mut state, EditorCommand::Newline);
    assert!(state.workspaces[0].prompt_panel.rows().len() < 4);
}

#[test]
fn editing_item_persists_on_commit_and_action_quit() {
    let mut state = demo_state();
    enter_tree(&mut state);
    move_prompt_selection(&mut state, 2, 8);
    activate_prompt_row(&mut state);
    apply_editor(&mut state, EditorCommand::InsertText(" updated".into()));
    show_prompt_tree(&mut state);
    let save = state
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::SavePromptContent { .. }))
        .cloned();
    let Some(Effect::SavePromptContent { id, content, .. }) = save else {
        panic!("save effect queued");
    };
    assert_eq!(id, 12);
    assert_eq!(content, "nested content updated");
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(12)
            .map(|item| item.content.as_str()),
        Some("nested content updated")
    );
}

#[test]
fn item_menu_lists_commands_and_dispatches_status_copy_delete() {
    let mut state = demo_state();
    enter_tree(&mut state);
    open_prompt_item_menu(&mut state, 12, (5, 5));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("menu open");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::EditPrompt,
            MenuCommand::ChangePromptStatus(PromptItemStatus::Todo),
            MenuCommand::RenamePrompt,
            MenuCommand::CopyPromptContent,
            MenuCommand::DeletePrompt,
        ]
    );
    assert!(matches!(
        menu.target,
        OverlayTarget::PromptItem { item_id: 12, .. }
    ));
    assert_eq!(state.workspaces[0].prompt_panel.selected, 2);

    // 状态切换命令 → 内存与效应。
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (5, 5),
        target: OverlayTarget::PromptItem {
            workspace: 0,
            item_id: 12,
        },
        commands: vec![MenuCommand::ChangePromptStatus(PromptItemStatus::Todo)],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(12)
            .map(|item| item.status),
        Some(PromptItemStatus::Todo)
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::SetPromptStatus {
            id: 12,
            status: PromptItemStatus::Todo,
            ..
        }
    )));

    // 复制命令 → 剪贴板效应。
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (5, 5),
        target: OverlayTarget::PromptItem {
            workspace: 0,
            item_id: 12,
        },
        commands: vec![MenuCommand::CopyPromptContent],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CopyToClipboard(text) if text == "nested content"
    )));

    // 删除命令 → 确认浮层；确认后级联删除。
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (5, 5),
        target: OverlayTarget::PromptItem {
            workspace: 0,
            item_id: 12,
        },
        commands: vec![MenuCommand::DeletePrompt],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert!(matches!(state.overlay, Some(Overlay::ConfirmClose(_))));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(state.workspaces[0].prompt_panel.tree.item(12).is_none());
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeletePrompt { id: 12, .. }))
    );
}

#[test]
fn folder_menu_hides_subfolder_for_second_level() {
    let mut state = demo_state();
    enter_tree(&mut state);
    open_prompt_folder_menu(&mut state, 1, (5, 5));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("menu open");
    };
    assert!(menu.commands.contains(&MenuCommand::NewSubFolder));
    open_prompt_folder_menu(&mut state, 2, (5, 5));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("menu open");
    };
    assert!(!menu.commands.contains(&MenuCommand::NewSubFolder));
    assert!(menu.commands.contains(&MenuCommand::NewPrompt));
}

#[test]
fn background_and_folder_commands_queue_creation_effects() {
    let mut state = demo_state();
    enter_tree(&mut state);
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (0, 0),
        target: OverlayTarget::PromptTree { workspace: 0 },
        commands: vec![MenuCommand::NewPrompt, MenuCommand::NewFolder],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CreatePrompt {
            folder_id: None,
            ..
        }
    )));
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (0, 0),
        target: OverlayTarget::PromptFolder {
            workspace: 0,
            folder_id: 1,
        },
        commands: vec![MenuCommand::NewFolder],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CreateFolder {
            parent_id: Some(1),
            ..
        }
    )));
}

#[test]
fn rename_flow_updates_memory_and_queues_effect() {
    let mut state = demo_state();
    enter_tree(&mut state);
    open_prompt_rename(
        &mut state,
        RenameTarget::PromptItem {
            workspace: 0,
            item_id: 12,
        },
    );
    let Some(Overlay::Rename(rename)) = &state.overlay else {
        panic!("rename open");
    };
    assert_eq!(rename.input.text(), "nested");
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Clear);
    for ch in "renamed".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(12)
            .map(|item| item.name.as_str()),
        Some("renamed")
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::RenamePrompt { id: 12, name, .. } if name == "renamed"
    )));
}

#[test]
fn delete_folder_cascades_editing_item_exit() {
    let mut state = demo_state();
    enter_tree(&mut state);
    move_prompt_selection(&mut state, 2, 8);
    activate_prompt_row(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.editing, Some(12));
    delete_prompt_folder(&mut state, 0, 1);
    assert!(state.workspaces[0].prompt_panel.tree.folder(1).is_none());
    assert!(state.workspaces[0].prompt_panel.tree.item(12).is_none());
    assert_eq!(state.workspaces[0].prompt_panel.editing, None);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
}

#[test]
fn scroll_clamps_and_keeps_selection_visible() {
    let mut state = demo_state();
    enter_tree(&mut state);
    state.prompt.resize(30, 2);
    assert!(scroll_prompt_tree(&mut state, 5, 2));
    assert!(state.workspaces[0].prompt_panel.scroll <= 2);
    move_prompt_selection(&mut state, 3, 2);
    assert!(state.workspaces[0].prompt_panel.scroll <= state.workspaces[0].prompt_panel.selected);
    assert!(clamp_prompt_tree_scroll(&mut state, 4));
}
