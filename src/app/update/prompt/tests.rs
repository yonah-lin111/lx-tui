//! 提示词树行为单元测试：视图切换、导航、菜单命令分发与编辑提交。

use super::*;
use crate::app::actions::EditorCommand;
use crate::app::overlay::{MenuCommand, NewNodeKind, Overlay, OverlayTarget};
use crate::app::prompt::PromptSection;
use crate::app::state::AppState;
use crate::app::update::{apply_editor, apply_overlay_key, tick};
use crate::db::{FolderRecord, PromptRecord};
use std::time::{Duration, Instant};

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
    assert_eq!(
        rows.len(),
        7,
        "tmp: + default + saved: + root, sub, nested, loose"
    );
}

#[test]
fn toggle_mode_and_keyboard_navigation() {
    let mut state = demo_state();
    // 默认编辑视图下 Esc 无绑定不切树。
    show_prompt_tree(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);

    // 上移到头 no-op；下移跳过标签行（saved: 行 2）落到 root。
    assert!(!move_prompt_selection(&mut state, -1, 8));
    assert!(move_prompt_selection(&mut state, 1, 8));
    assert_eq!(state.workspaces[0].prompt_panel.selected, 3);
    assert!(move_prompt_selection(&mut state, 1, 8));
    assert_eq!(state.workspaces[0].prompt_panel.selected, 4);

    // 折叠/展开选中文件夹（折叠经由行激活/点击；←/→ 已归搜索框光标）。
    let mut state = demo_state();
    enter_tree(&mut state);
    let mut state = {
        // 行序：tmp:(0)、default(1)、saved:(2)、root(3)、sub(4)…；选中子文件夹后折叠。
        move_prompt_selection(&mut state, 2, 8);
        state
    };
    assert!(collapse_prompt_row(&mut state));
    let rows = state.workspaces[0].prompt_panel.rows();
    assert_eq!(rows.len(), 6, "子文件夹折叠隐藏 nested");
    assert!(expand_prompt_row(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.rows().len(), 7);
}

#[test]
fn enter_on_item_opens_editor_and_escape_returns() {
    let mut state = demo_state();
    enter_tree(&mut state);
    // 行序：tmp:(0)、default(1)、saved:(2)、root(3)、sub(4)、nested(5)、loose(6)；选中 nested 并回车。
    move_prompt_selection(&mut state, 3, 8);
    assert!(activate_prompt_row(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
    assert_eq!(state.prompt.text(), "nested content");
    assert!(state.prompt_focused);

    // Esc 返回树并保持绑定清除。
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
    assert_eq!(state.workspaces[0].prompt_panel.editing, None);
}

#[test]
fn prompt_row_double_click_detects_same_row_within_window() {
    let mut state = demo_state();
    enter_tree(&mut state);
    let t0 = Instant::now();

    assert!(!prompt_row_double_click(&mut state, 5, t0));
    assert!(
        !prompt_row_double_click(&mut state, 6, t0 + Duration::from_millis(50)),
        "不同行不构成双击"
    );
    assert!(
        !prompt_row_double_click(
            &mut state,
            6,
            t0 + Duration::from_millis(50) + PROMPT_DOUBLE_CLICK_WINDOW + Duration::from_millis(1)
        ),
        "超出窗口不构成双击"
    );
    assert!(
        prompt_row_double_click(
            &mut state,
            6,
            t0 + Duration::from_millis(50) + PROMPT_DOUBLE_CLICK_WINDOW + Duration::from_millis(2)
        ),
        "窗口内同一行构成双击"
    );
    assert!(
        !prompt_row_double_click(
            &mut state,
            6,
            t0 + Duration::from_millis(50) + PROMPT_DOUBLE_CLICK_WINDOW + Duration::from_millis(3)
        ),
        "双击后点击序列复位"
    );
}

#[test]
fn open_prompt_row_edits_item_and_temp_but_not_folder() {
    // 条目行：进入编辑视图并载入内容。
    let mut state = demo_state();
    enter_tree(&mut state);
    // 行序：tmp:(0)、default(1)、saved:(2)、root(3)、sub(4)、nested(5)、loose(6)。
    assert!(open_prompt_row(&mut state, 5));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
    assert_eq!(state.prompt.text(), "nested content");

    // 临时提示词行：进入其编辑视图。
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_prompt_row(&mut state, 1));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(matches!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    ));

    // 文件夹行：不响应（折叠/展开由单击处理）。
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(!open_prompt_row(&mut state, 3));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
}

#[test]
fn typing_in_tree_mode_filters_like_modal_input_and_escape_clears() {
    let mut state = demo_state();
    enter_tree(&mut state);
    for ch in "nested".chars() {
        apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }
    assert_eq!(state.prompt.text(), "", "树视图输入不进入编辑器");
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.search.text(), "nested");
    assert_eq!(panel.selected, 0, "过滤后选中首个结果");
    assert_eq!(panel.rows().len(), 3, "root + sub + nested");
    // 单行输入语义：Home/End/←/→ 移动光标，Delete/Backspace 编辑过滤词。
    apply_editor(&mut state, EditorCommand::Home);
    apply_editor(&mut state, EditorCommand::Delete);
    assert_eq!(state.workspaces[0].prompt_panel.search.text(), "ested");
    apply_editor(&mut state, EditorCommand::End);
    apply_editor(&mut state, EditorCommand::Backspace);
    assert_eq!(state.workspaces[0].prompt_panel.search.text(), "este");
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(state.workspaces[0].prompt_panel.search.text().is_empty());
    assert_eq!(state.workspaces[0].prompt_panel.rows().len(), 7);

    // 工具栏 [tmp] 入口：新建空临时提示词并进入编辑视图。
    assert!(open_temp_prompt(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(
        matches!(
            state.workspaces[0].prompt_panel.editing,
            Some(PromptEditing::Temp(_))
        ),
        "绑定新建 temp"
    );
    assert_eq!(state.prompt.text(), "", "新 temp 内容为空");
}

#[test]
fn editing_item_persists_on_commit_and_action_quit() {
    let mut state = demo_state();
    enter_tree(&mut state);
    // 行序：tmp:(0)、default(1)、saved:(2)、root(3)、sub(4)、nested(5)…；选中 nested 打开编辑。
    move_prompt_selection(&mut state, 3, 8);
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
    assert_eq!(state.workspaces[0].prompt_panel.selected, 5);

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
    // 菜单打开命名浮层（提示词）：输入名称后回车登记创建效应。
    let Some(Overlay::NewNode(dialog)) = &state.overlay else {
        panic!("naming dialog open");
    };
    assert_eq!(dialog.kind, NewNodeKind::Prompt { folder_id: None });
    assert_eq!(dialog.name.text(), "", "输入框不预填");
    for ch in "draft".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
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
    // 菜单打开命名浮层：输入名称后回车登记创建效应。
    let Some(Overlay::NewNode(dialog)) = &state.overlay else {
        panic!("naming dialog open");
    };
    assert_eq!(dialog.kind, NewNodeKind::Folder { parent_id: Some(1) });
    assert_eq!(dialog.name.text(), "", "输入框不预填");
    for ch in "docs".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
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
    // 选中 nested（行 5）打开编辑，再级联删除其所在文件夹。
    move_prompt_selection(&mut state, 3, 8);
    activate_prompt_row(&mut state);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
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
    // 行数 7（tmp:、default、saved:、4 树行），视口 2 行 → 最大滚动 5。
    assert!(scroll_prompt_tree(&mut state, 5, 2));
    assert_eq!(state.workspaces[0].prompt_panel.scroll, 5, "越界钳制到末行");
    move_prompt_selection(&mut state, 3, 2);
    assert!(state.workspaces[0].prompt_panel.scroll <= state.workspaces[0].prompt_panel.selected);
    assert!(clamp_prompt_tree_scroll(&mut state, 4));
}

#[test]
fn mode_roundtrip_without_selection_keeps_temp_editor() {
    let mut state = AppState::demo();
    state.workspaces[0].prompt_panel.tree.replace(
        vec![],
        vec![PromptItem {
            id: 10,
            folder_id: None,
            name: "loose".into(),
            content: "body".into(),
            status: PromptItemStatus::Todo,
        }],
    );
    // 初始编辑视图绑定空 temp；列表恒有 default temp，其后才是条目。
    assert!(matches!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    ));
    show_prompt_tree(&mut state);
    let rows = state.workspaces[0].prompt_panel.rows();
    assert_eq!(rows[1].kind, PromptRowKind::Temp, "默认高亮 default temp");
    assert_eq!(rows[3].kind, PromptRowKind::Item);
    // 未显式选中任何行：返回编辑保持 temp 绑定，不自动打开任何条目。
    toggle_prompt_panel_mode(&mut state);
    assert!(matches!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    ));
    assert_eq!(state.prompt.text(), "");
}

#[test]
fn mode_roundtrip_restores_open_item_without_selection() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 12));
    show_prompt_tree(&mut state);
    // 过滤自动把选中落到命中行（非用户显式选中）；"loo" 只命中 loose。
    for ch in "loo".chars() {
        apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }
    assert_eq!(
        state.workspaces[0].prompt_panel.selected, 0,
        "过滤命中 loose"
    );
    toggle_prompt_panel_mode(&mut state);
    // 恢复进入列表前的条目（而非过滤高亮行）。
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
    assert_eq!(state.prompt.text(), "nested content");
}

#[test]
fn mode_roundtrip_with_explicit_selection_opens_selected_item() {
    let mut state = demo_state();
    enter_tree(&mut state);
    // 行序：tmp:(0)、default(1)、saved:(2)、root(3)、sub(4)、nested(5)…；选中 nested。
    move_prompt_selection(&mut state, 3, 8);
    toggle_prompt_panel_mode(&mut state);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
}

#[test]
fn mode_roundtrip_falls_back_to_temp_when_return_target_deleted() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    show_prompt_tree(&mut state);
    delete_prompt_item(&mut state, 0, 10);
    toggle_prompt_panel_mode(&mut state);
    assert!(matches!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    ));
}

#[test]
fn list_entry_highlights_open_item() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 12));
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.selected_prompt_id(), Some(12), "高亮当前编辑条目");
    assert_eq!(
        panel.selected_row().map(|row| row.kind),
        Some(PromptRowKind::Item)
    );
}

#[test]
fn list_entry_highlights_open_temp_over_default() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    apply_editor(&mut state, EditorCommand::InsertText("body".into()));
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(
        panel
            .selected_row()
            .and_then(|row| panel.temp_of_row(row))
            .map(|temp| temp.id),
        Some(temp_id),
        "高亮当前编辑 temp 而非 default"
    );
    assert_eq!(panel.selected, 1);
}

#[test]
fn tree_entry_keeps_default_temp_and_selects_it() {
    let mut state = demo_state();
    enter_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(
        panel.rows()[0].kind,
        PromptRowKind::Section(PromptSection::Temp)
    );
    assert_eq!(panel.rows()[1].kind, PromptRowKind::Temp);
    assert_eq!(
        panel
            .temp_of_row(panel.rows()[1])
            .map(|temp| temp.display_name()),
        Some(text::PROMPT_TEMP_DEFAULT_NAME),
        "default temp 置顶且展示占位名"
    );
    assert_eq!(panel.selected, 1, "进入列表默认高亮 default temp");
}

#[test]
fn deleted_default_temp_is_recreated_on_tree_entry() {
    let mut state = demo_state();
    enter_tree(&mut state);
    let blank = state.workspaces[0]
        .prompt_panel
        .blank_temp()
        .expect("default temp");
    assert!(delete_prompt_temp(&mut state, 0, blank));
    assert_eq!(state.workspaces[0].prompt_panel.blank_temp(), None);

    // 重新进入列表：default temp 自动补建并默认高亮。
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert!(panel.blank_temp().is_some(), "进入列表补建 default temp");
    assert_eq!(panel.selected, 1);
}

#[test]
fn edit_falls_back_to_new_temp_when_selection_is_not_item() {
    let mut state = demo_state();
    enter_tree(&mut state);
    // 选中根文件夹（展示行 3）后 [edit]：新建空 temp，不再无响应。
    move_prompt_selection(&mut state, 1, 8);
    toggle_prompt_panel_mode(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(matches!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    ));
    assert!(state.prompt_focused);

    // 空树同样回退新建 temp：清空全部条目后 [edit] 仍进入编辑视图。
    let mut state = AppState::demo();
    enter_tree(&mut state);
    toggle_prompt_panel_mode(&mut state);
    assert!(matches!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    ));
}

#[test]
fn temp_roundtrip_keeps_content_and_picker_lists_folders() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    apply_editor(&mut state, EditorCommand::InsertText("draft body".into()));
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(
        panel.temp(temp_id).map(|temp| temp.content.as_str()),
        Some("draft body")
    );
    // 行序：tmp:、draft temp、default、saved:、root、sub、nested、loose。
    assert_eq!(panel.rows().len(), 8, "{:?}", panel.rows());

    // 再次打开该 temp：内容保留（不入库也不丢失）。
    assert!(edit_prompt_temp(&mut state, 0, temp_id));
    assert_eq!(state.prompt.text(), "draft body");

    // 保存选择器：名称预填默认名 + 根层 + 层级顺序文件夹 + 新建文件夹动作行。
    open_save_draft(&mut state);
    let Some(Overlay::SaveDraft(dialog)) = &state.overlay else {
        panic!("picker open");
    };
    assert_eq!(dialog.name.text(), text::PROMPT_DEFAULT_NAME);
    assert_eq!(dialog.entries.len(), 4, "{:?}", dialog.entries);
    assert!(matches!(dialog.entries[0], SaveDraftEntry::Root));
    assert!(matches!(
        &dialog.entries[1],
        SaveDraftEntry::Folder { id: 1, depth: 0, name } if name == "root"
    ));
    assert!(matches!(
        &dialog.entries[2],
        SaveDraftEntry::Folder { id: 2, depth: 1, name } if name == "sub"
    ));
    assert!(matches!(dialog.entries[3], SaveDraftEntry::AddFolder));

    // 回车保存到根层：登记 SaveDraft 效应并关闭选择器。
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(state.overlay.is_none());
    let queued = state
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::SaveDraft { .. }))
        .cloned();
    let Some(Effect::SaveDraft {
        folder_id,
        name,
        content,
        ..
    }) = queued
    else {
        panic!("save draft effect queued");
    };
    assert_eq!(folder_id, None);
    assert_eq!(name, text::PROMPT_DEFAULT_NAME);
    assert_eq!(content, "draft body");

    // 服务保存成功后的收尾：移除已升格的 temp 并清空搜索词。
    finish_draft_save(&mut state, 0);
    assert!(state.workspaces[0].prompt_panel.temp(temp_id).is_none());
}

#[test]
fn temp_button_creates_new_temp_each_time_and_keeps_default() {
    let mut state = demo_state();
    enter_tree(&mut state);
    // 第一次：空白 temp 离开编辑视图保留在列表（default temp，占位名 default）。
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(first)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert!(panel.temp(first).is_some(), "空白 default temp 保留在列表");
    assert_eq!(panel.blank_temp(), Some(first), "空白项即 default temp");
    assert_eq!(panel.rows().len(), 7, "tmp: + default + saved: + 4 树行");
    assert_eq!(panel.selected, 1, "默认高亮 default temp");

    // 第二次：新建独立 temp；内容非空后与 default temp 同列，列表恒有空白项。
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(second)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    assert_ne!(first, second, "每次点击新建独立 temp");
    apply_editor(&mut state, EditorCommand::InsertText("body".into()));
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert!(panel.temp(second).is_some());
    assert!(
        panel.blank_temp().is_some(),
        "列表恒有空白 default temp（缺失时补建）"
    );
    let rows = panel.rows();
    assert_eq!(rows[0].kind, PromptRowKind::Section(PromptSection::Temp));
    assert_eq!(rows[1].kind, PromptRowKind::Temp);
    assert_eq!(rows[2].kind, PromptRowKind::Temp);
    assert_eq!(rows[3].kind, PromptRowKind::Section(PromptSection::Saved));
    assert_eq!(panel.selected, 1, "高亮当前编辑 temp 行");
}

#[test]
fn activate_temp_row_opens_editor_and_escape_returns() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    apply_editor(&mut state, EditorCommand::InsertText("temp body".into()));
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.selected, 1, "高亮当前编辑 temp 行");
    assert!(panel.temp(temp_id).is_some());

    // 显式选中内容 temp 行后回车打开编辑视图。
    assert!(select_prompt_temp(&mut state, 0, temp_id));
    assert!(activate_prompt_row(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(temp_id))
    );
    assert_eq!(state.prompt.text(), "temp body");

    // Esc 返回树：内容保留、绑定清除、内容 temp 与 default temp 同列。
    apply_editor(&mut state, EditorCommand::Escape);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.mode, PromptMode::Tree);
    assert_eq!(panel.editing, None);
    assert_eq!(
        panel
            .rows()
            .iter()
            .filter(|row| row.kind == PromptRowKind::Temp)
            .count(),
        2
    );
}

#[test]
fn temp_menu_lists_commands_and_deletes_immediately() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    apply_editor(&mut state, EditorCommand::InsertText("doomed".into()));
    show_prompt_tree(&mut state);
    let panel = &state.workspaces[0].prompt_panel;
    let temp_id = panel
        .rows()
        .into_iter()
        .find_map(|row| panel.temp_of_row(row).map(|temp| temp.id))
        .expect("temp row visible");

    open_prompt_temp_menu(&mut state, temp_id, (5, 5));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("menu open");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::EditPrompt,
            MenuCommand::CopyPromptContent,
            MenuCommand::DeleteTemp,
        ]
    );
    assert!(matches!(
        menu.target,
        OverlayTarget::TempPrompt { temp_id: id, .. } if id == temp_id
    ));
    assert_eq!(
        state.workspaces[0].prompt_panel.selected, 1,
        "打开菜单即选中该行"
    );

    // 复制命令 → 剪贴板效应。
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (5, 5),
        target: OverlayTarget::TempPrompt {
            workspace: 0,
            temp_id,
        },
        commands: vec![MenuCommand::CopyPromptContent],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CopyToClipboard(text) if text == "doomed"
    )));

    // 删除命令 → 立即移除（不打开确认浮层）。
    state.overlay = Some(Overlay::Menu(crate::app::overlay::Menu {
        anchor: (5, 5),
        target: OverlayTarget::TempPrompt {
            workspace: 0,
            temp_id,
        },
        commands: vec![MenuCommand::DeleteTemp],
        selected: 0,
    }));
    crate::app::update::activate_menu(&mut state);
    assert!(state.overlay.is_none(), "删除不经确认浮层");
    assert!(state.workspaces[0].prompt_panel.temp(temp_id).is_none());
}

#[test]
fn save_draft_new_folder_opens_dialog_and_queues_folder_effect() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    apply_editor(&mut state, EditorCommand::InsertText("body".into()));
    show_prompt_tree(&mut state);
    open_save_draft(&mut state);

    // 回车选中动作行 → 打开命名浮层（不直接建库），Esc 取消恢复选择器。
    set_save_draft_selection(&mut state, 3);
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    let Some(Overlay::NewNode(folder)) = &state.overlay else {
        panic!("new folder dialog open");
    };
    assert_eq!(folder.kind, NewNodeKind::Folder { parent_id: None });
    assert_eq!(folder.name.text(), "", "输入框不预填");
    assert!(folder.resume.is_some(), "来自选择器");
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Esc);
    assert!(
        matches!(state.overlay, Some(Overlay::SaveDraft(_))),
        "取消恢复选择器"
    );

    // 再次打开：清空后输入 docs，回车确认 → 恢复选择器并登记建文件夹效应。
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(matches!(state.overlay, Some(Overlay::NewNode(_))));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Clear);
    for ch in "docs".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    let Some(Overlay::SaveDraft(dialog)) = &state.overlay else {
        panic!("picker restored");
    };
    assert_eq!(dialog.name.text(), text::PROMPT_DEFAULT_NAME);
    let Some(Effect::CreateFolder {
        parent_id, name, ..
    }) = state
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::CreateFolder { .. }))
        .cloned()
    else {
        panic!("create folder effect queued");
    };
    assert_eq!(parent_id, None);
    assert_eq!(name, "docs");

    // 空名拒绝：对话框保持打开且不登记效应。
    state.effects.clear();
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Clear);
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(
        matches!(state.overlay, Some(Overlay::NewNode(_))),
        "空名拒绝"
    );
    assert!(state.effects.is_empty());
}

#[test]
fn refresh_save_draft_rebuilds_entries_and_selects_new_folder() {
    let mut state = demo_state();
    open_save_draft(&mut state);
    // 服务建库后由树记录重建条目：新文件夹追加在子层之后、动作行之前并选中。
    let tree = &mut state.workspaces[0].prompt_panel.tree;
    tree.replace(
        vec![crate::app::prompt::PromptFolder {
            id: 7,
            parent_id: None,
            name: "docs".into(),
            collapsed: false,
        }],
        vec![],
    );
    refresh_save_draft(&mut state, 7);
    let Some(Overlay::SaveDraft(dialog)) = &state.overlay else {
        panic!("picker open");
    };
    assert!(matches!(
        &dialog.entries[dialog.selected],
        SaveDraftEntry::Folder { id: 7, .. }
    ));
    assert!(matches!(
        dialog.entries.last(),
        Some(SaveDraftEntry::AddFolder)
    ));
}

#[test]
fn save_draft_commits_with_custom_name_and_rejects_empty_names() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    apply_editor(&mut state, EditorCommand::InsertText("body".into()));
    show_prompt_tree(&mut state);
    open_save_draft(&mut state);

    // 名称改为自定义（选中行为根层 → 字符输入归属顶部名称框）。
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Clear);
    for ch in "My prompt".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    // 名称为空时拒绝执行（保持弹窗）。
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Clear);
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(state.overlay.is_some(), "空名拒绝保存");
    assert!(state.effects.is_empty(), "未登记效应");
    for ch in "My prompt".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    let Some(Effect::SaveDraft {
        name, folder_id, ..
    }) = state
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::SaveDraft { .. }))
        .cloned()
    else {
        panic!("save draft effect queued");
    };
    assert_eq!(name, "My prompt");
    assert_eq!(folder_id, None);
}

#[test]
fn save_picker_selection_moves_and_clamps() {
    let mut state = demo_state();
    open_save_draft(&mut state);
    assert!(!move_save_draft_selection(&mut state, -1), "已在头部");
    assert!(move_save_draft_selection(&mut state, 3));
    assert!(!move_save_draft_selection(&mut state, 1), "已在尾部");
    assert!(!set_save_draft_selection(&mut state, 99), "越界忽略");
    assert!(set_save_draft_selection(&mut state, 0));
}

#[test]
fn new_prompt_dialog_creates_with_custom_name_and_no_edit() {
    let mut state = demo_state();
    enter_tree(&mut state);
    open_new_prompt(&mut state, Some(1));
    let Some(Overlay::NewNode(dialog)) = &state.overlay else {
        panic!("new prompt dialog open");
    };
    assert_eq!(dialog.kind, NewNodeKind::Prompt { folder_id: Some(1) });
    assert_eq!(dialog.name.text(), "", "输入框不预填");

    // 空名拒绝：浮层保持打开且不登记效应。
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Clear);
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(
        matches!(state.overlay, Some(Overlay::NewNode(_))),
        "空名拒绝"
    );
    assert!(state.effects.is_empty());

    // 自定义名 + 回车：登记创建效应并关闭浮层（不进入编辑视图）。
    for ch in "docs".chars() {
        apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(state.overlay.is_none());
    let Some(Effect::CreatePrompt {
        folder_id, name, ..
    }) = state
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::CreatePrompt { .. }))
        .cloned()
    else {
        panic!("create prompt effect queued");
    };
    assert_eq!(folder_id, Some(1));
    assert_eq!(name, "docs");

    // Esc 取消：不登记效应。
    state.effects.clear();
    open_new_prompt(&mut state, None);
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Esc);
    assert!(state.overlay.is_none());
    assert!(state.effects.is_empty());
}

#[test]
fn autosave_commits_item_after_idle_delay() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str(" edited");
    assert!(!state.prompt.is_saved());
    let now = Instant::now();
    assert!(!tick(&mut state, now), "首个观察点只记录计时");
    assert!(!tick(&mut state, now + Duration::from_millis(799)));
    assert!(state.effects.is_empty(), "未达静置时长不落库");
    assert!(!state.prompt.is_saved(), "未落库保持黄点");
    assert!(
        tick(&mut state, now + Duration::from_millis(800)),
        "静置达标触发保存"
    );
    assert!(!state.prompt.is_saved(), "保存回执（服务）到达前保持未保存");
    let revision = state.prompt.revision();
    let prompt = state.prompt.id();
    assert!(crate::app::update::mark_prompt_saved(
        &mut state, prompt, revision
    ));
    assert!(state.prompt.is_saved(), "回执确认后复位为已保存");
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::SavePromptContent { id: 10, .. })),
        "条目内容登记落库效应"
    );
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(10)
            .map(|item| item.content.as_str()),
        Some("loose content edited"),
        "内存树同步更新"
    );
}

#[test]
fn autosave_resets_timer_on_new_edits() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str("a");
    let now = Instant::now();
    assert!(!tick(&mut state, now));
    state.prompt.insert_str("b");
    assert!(
        !tick(&mut state, now + Duration::from_millis(700)),
        "新修订重置计时"
    );
    assert!(
        !tick(&mut state, now + Duration::from_millis(1400)),
        "距新起点仅 700ms"
    );
    assert!(state.effects.is_empty());
    assert!(
        tick(&mut state, now + Duration::from_millis(1500)),
        "距新起点达标"
    );
    let revision = state.prompt.revision();
    let prompt = state.prompt.id();
    assert!(crate::app::update::mark_prompt_saved(
        &mut state, prompt, revision
    ));
    assert!(state.prompt.is_saved());
}

#[test]
fn temp_never_autosaves() {
    let mut state = demo_state();
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    state.prompt.insert_str("draft body");
    let now = Instant::now();
    assert!(!tick(&mut state, now + Duration::from_secs(30)));
    assert!(state.effects.is_empty(), "临时提示词不落库");
    assert!(!state.prompt.is_saved(), "临时提示词保持未保存（圆点恒灰）");
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(temp_id)
            .map(|temp| temp.content.as_str()),
        Some(""),
        "未提交时 temp 内容不动"
    );
}

#[test]
fn save_prompt_now_saves_item_and_opens_picker_for_temp() {
    // 条目：立即复位保存点并登记落库效应，无需等待静置。
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str("!");
    save_prompt_now(&mut state);
    assert!(state.prompt.is_saved());
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::SavePromptContent { id: 10, .. }))
    );

    // 临时提示词：提交缓冲到 temp 并打开保存选择器。
    let mut state = demo_state();
    assert!(open_temp_prompt(&mut state));
    let Some(PromptEditing::Temp(temp_id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp bound");
    };
    state.prompt.insert_str("draft body");
    save_prompt_now(&mut state);
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(temp_id)
            .map(|temp| temp.content.as_str()),
        Some("draft body"),
        "提交缓冲到 temp"
    );
    assert!(matches!(state.overlay, Some(Overlay::SaveDraft(_))));
}

#[test]
fn next_deadline_includes_prompt_autosave() {
    let mut state = demo_state();
    assert!(crate::app::update::next_deadline(&state).is_none());
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str("x");
    assert!(
        crate::app::update::next_deadline(&state).is_none(),
        "尚无静置观察不安排唤醒"
    );
    let now = Instant::now();
    assert!(!tick(&mut state, now), "首个观察点只记录计时");
    let deadline = crate::app::update::next_deadline(&state).expect("自动保存待触发");
    assert!(tick(&mut state, deadline), "到期唤醒触发保存");
    // 回执未到时仍安排重试；模拟服务回执确认后不再有待触发。
    let revision = state.prompt.revision();
    let prompt = state.prompt.id();
    assert!(crate::app::update::mark_prompt_saved(
        &mut state, prompt, revision
    ));
    assert!(
        crate::app::update::next_deadline(&state).is_none(),
        "确认后无待触发"
    );

    // 临时提示词不产生自动保存期限（空闲不轮询）。
    let mut state = demo_state();
    assert!(open_temp_prompt(&mut state));
    state.prompt.insert_str("x");
    assert!(!tick(&mut state, Instant::now()));
    assert!(crate::app::update::next_deadline(&state).is_none());
}
