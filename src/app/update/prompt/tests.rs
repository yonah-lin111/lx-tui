//! 提示词树行为单元测试：视图切换、导航、菜单命令分发与编辑提交。

use super::*;
use crate::app::actions::{Action, EditorCommand};
use crate::app::overlay::{
    ConfirmClose, MenuCommand, NewNodeKind, Overlay, OverlayTarget, PageMenuRow, SaveDraftEntry,
};
use crate::app::prompt::{PromptPage, PromptPageId, PromptRowKind, PromptSection};
use crate::app::state::AppState;
use crate::app::update::{
    apply, apply_editor, apply_overlay_key, create_workspace, create_workspace_with_cwd,
    switch_workspace, tick, toggle_prompt_pin,
};
use crate::db::{FolderRecord, PromptPageRecord, PromptRecord};
use crate::ui::text;
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
            active_page_id: Some(10),
            pages: vec![PromptPageRecord {
                id: 10,
                position: 0,
                name: "Page 1".into(),
                content: "loose content".into(),
            }],
        },
        PromptRecord {
            id: 12,
            folder_id: Some(2),
            name: "nested".into(),
            content: "nested content".into(),
            active_page_id: Some(12),
            pages: vec![PromptPageRecord {
                id: 12,
                position: 0,
                name: "Page 1".into(),
                content: "nested content".into(),
            }],
        },
    ];
    apply_prompt_tree(&mut state, 0, std::path::Path::new("/ws"), folders, prompts);
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
    assert!(state.prompt_focused());

    // Esc 返回树并保持绑定清除。
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
    assert_eq!(state.workspaces[0].prompt_panel.editing, None);

    // 树视图 Esc 切换回编辑：恢复进入列表前的绑定。
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
    assert_eq!(state.prompt.text(), "nested content");
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
fn typing_in_tree_mode_filters_like_modal_input_and_ctrl_c_clears() {
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
    // Ctrl+C 清空过滤词并留在列表；Esc 返回编辑视图。
    assert!(clear_prompt_search(&mut state));
    assert!(state.workspaces[0].prompt_panel.search.text().is_empty());
    assert_eq!(state.workspaces[0].prompt_panel.rows().len(), 7);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(
        matches!(
            state.workspaces[0].prompt_panel.editing,
            Some(PromptEditing::Temp(_))
        ),
        "Esc 恢复进入列表前的编辑绑定"
    );

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
            .map(|item| item.active_content()),
        Some("nested content updated")
    );
}

#[test]
fn item_menu_lists_commands_and_dispatches_copy_delete() {
    let mut state = demo_state();
    enter_tree(&mut state);
    let selected_before = state.workspaces[0].prompt_panel.selected;
    open_prompt_item_menu(&mut state, 12, (5, 5));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("menu open");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::EditPrompt,
            MenuCommand::RenamePrompt,
            MenuCommand::CopyPromptContent,
            MenuCommand::DeletePrompt,
        ]
    );
    assert!(matches!(
        menu.target,
        OverlayTarget::PromptItem { item_id: 12, .. }
    ));
    // 右键不改动列表选中行（对齐工作区列表：只挂菜单，不切换当前项）。
    assert_eq!(state.workspaces[0].prompt_panel.selected, selected_before);
    let row_at_12 = state.workspaces[0]
        .prompt_panel
        .rows()
        .into_iter()
        .position(|row| {
            state.workspaces[0]
                .prompt_panel
                .tree
                .item_of_row(row)
                .is_some_and(|item| item.id == 12)
        })
        .expect("item 12 row");
    assert_ne!(selected_before, row_at_12, "菜单目标行不是当前选中行");

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
        Effect::CopyToClipboard { text, .. } if text == "nested content"
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
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![PromptItem::single(10, None, "loose", "body")]);
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
fn mode_roundtrip_keeps_undo_redo_and_cursor_for_same_item() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    apply_editor(&mut state, EditorCommand::InsertText("XYZ".into()));
    // 在中间留一个可验证的光标位置：左移两次后往返。
    apply_editor(&mut state, EditorCommand::Left);
    apply_editor(&mut state, EditorCommand::Left);
    let cursor = state.prompt.cursor();
    assert!(state.prompt.can_undo());

    show_prompt_tree(&mut state);
    toggle_prompt_panel_mode(&mut state);

    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(10)),
        "返回同一目标"
    );
    assert_eq!(state.prompt.text(), "loose contentXYZ");
    assert!(
        state.prompt.can_undo(),
        "返回同一目标保留撤销记录，不因视图往返清空"
    );
    assert_eq!(state.prompt.cursor(), cursor, "光标停在离开前位置");
    // 撤销仍可回到编辑前正文。
    apply_editor(&mut state, EditorCommand::Undo);
    assert_eq!(state.prompt.text(), "loose content");
    assert!(state.prompt.can_redo(), "撤销后可重做");
}

#[test]
fn mode_roundtrip_keeps_history_for_same_temp() {
    let mut state = demo_state();
    enter_tree(&mut state);
    assert!(open_temp_prompt(&mut state));
    let temp_id = match state.workspaces[0].prompt_panel.editing {
        Some(PromptEditing::Temp(id)) => id,
        other => panic!("temp bound: {other:?}"),
    };
    apply_editor(&mut state, EditorCommand::InsertText("draft".into()));
    assert!(state.prompt.can_undo());

    show_prompt_tree(&mut state);
    // 未改选中：返回原临时提示词。
    toggle_prompt_panel_mode(&mut state);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Temp(temp_id))
    );
    assert_eq!(state.prompt.text(), "draft");
    assert!(state.prompt.can_undo(), "同一 temp 保留撤销记录");
}

#[test]
fn reload_between_list_roundtrip_discards_stash() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    apply_editor(&mut state, EditorCommand::InsertText(" edited".into()));
    show_prompt_tree(&mut state);
    assert!(
        state.workspaces[0].prompt_panel.editor_stash().is_some(),
        "切列表后暂存编辑状态"
    );
    // 同键面板在背后把条目 10 的正文改成别的（模拟外部改写/重载）：
    // 先清除待确认标记（模拟本地保存已落库），再收到不同正文的重载。
    state.workspaces[0].prompt_panel.clear_page_pending(10, 10);
    apply_prompt_tree(
        &mut state,
        0,
        std::path::Path::new("/ws"),
        vec![],
        reload_records(Some(10), "changed elsewhere", false),
    );
    // 返回条目 10：暂存正文与当前页正文不一致，按新正文重建、历史清空。
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert_eq!(state.prompt.text(), "changed elsewhere");
    assert!(!state.prompt.can_undo(), "正文被别处改写后不恢复旧历史");
}

#[test]
fn opening_other_item_discards_stash_then_returns_clean() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    apply_editor(&mut state, EditorCommand::InsertText(" edited".into()));
    show_prompt_tree(&mut state);
    // 打开另一个条目：丢弃 A 的暂存。
    assert!(edit_prompt_item(&mut state, 0, 12));
    show_prompt_tree(&mut state);
    // 回到 A：历史已在打开 B 时丢弃。
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert_eq!(state.prompt.text(), "loose content edited");
    assert!(
        !state.prompt.can_undo(),
        "中途打开别的条目后，旧目标的撤销历史不再恢复"
    );
}

#[test]
fn workspace_switch_discards_list_roundtrip_stash() {
    let mut state = demo_state();
    create_workspace_with_cwd(&mut state, Some(std::path::PathBuf::from("/ws2")));
    switch_workspace(&mut state, 0);
    assert!(edit_prompt_item(&mut state, 0, 10));
    apply_editor(&mut state, EditorCommand::InsertText(" edited".into()));
    show_prompt_tree(&mut state);
    assert!(
        state.workspaces[0].prompt_panel.editor_stash().is_some(),
        "切列表后暂存编辑状态"
    );
    // 切到别的工作区再切回：跨越工作区的切换丢弃暂存。
    switch_workspace(&mut state, 1);
    switch_workspace(&mut state, 0);
    assert!(
        state.workspaces[0].prompt_panel.editor_stash().is_none(),
        "跨工作区切换丢弃暂存"
    );
    // 回到条目 10：历史已丢弃，按新目标重建（正文取库，撤销栈为空）。
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert_eq!(state.prompt.text(), "loose content edited");
    assert!(!state.prompt.can_undo(), "跨工作区往返后不恢复旧历史");
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
    assert!(state.prompt_focused());

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
        panel.temp(temp_id).map(|temp| temp.active_content()),
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
        pages,
        active,
        ..
    }) = queued
    else {
        panic!("save draft effect queued");
    };
    assert_eq!(folder_id, None);
    assert_eq!(name, text::PROMPT_DEFAULT_NAME);
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].content, "draft body");
    assert_eq!(active, 0);

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
        Effect::CopyToClipboard { text, .. } if text == "doomed"
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
    assert!(open_temp_prompt(&mut state));
    apply_editor(&mut state, EditorCommand::InsertText("body".into()));
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
    assert!(crate::app::update::mark_prompt_page_persisted(
        &mut state,
        10,
        10,
        "loose content edited",
        prompt,
        revision
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
            .map(|item| item.active_content()),
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
    assert!(crate::app::update::mark_prompt_page_persisted(
        &mut state,
        10,
        10,
        "loose contentab",
        prompt,
        revision
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
            .map(|temp| temp.active_content()),
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
            .map(|temp| temp.active_content()),
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
    assert!(crate::app::update::mark_prompt_page_persisted(
        &mut state,
        10,
        10,
        "loose contentx",
        prompt,
        revision
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

// ===== 粘贴提案与保存门禁 =====

/// 打开 `nested` 条目编辑视图：条目编辑态才参与自动保存。
fn item_editing_state() -> AppState {
    let mut state = demo_state();
    enter_tree(&mut state);
    move_prompt_selection(&mut state, 3, 8);
    assert!(activate_prompt_row(&mut state));
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(12))
    );
    state
}

/// 在光标处写入路径占位并打开粘贴面板。
fn stage_paste(state: &mut AppState) {
    let anchor = crate::app::prompt::PasteAnchor::at_cursor(state.prompt.cursor());
    let request = state.prompt.begin_paste_read(anchor);
    let candidate = crate::app::prompt::PasteCandidate::external(
        std::path::PathBuf::from("/tmp/a.txt"),
        crate::app::prompt::PasteCandidateKind::File,
    );
    assert!(state.prompt.paste_stage(request, vec![candidate]));
    assert!(state.prompt.paste_panel().is_some());
}

#[test]
fn autosave_defers_while_paste_proposal_open() {
    let mut state = item_editing_state();
    stage_paste(&mut state);
    state.effects.clear();
    let now = Instant::now();
    assert!(!tick(&mut state, now));
    assert!(
        !tick(&mut state, now + Duration::from_millis(900)),
        "提案未收束不触发自动保存"
    );
    assert!(state.effects.is_empty(), "占位文本不落库");
    assert!(!state.prompt.is_saved(), "仍为脏状态");
    assert!(
        crate::app::update::next_deadline(&state).is_none(),
        "提案未收束不安排唤醒（空闲不轮询）"
    );

    // 面板收束（路径确认）后恢复自动保存。
    assert!(
        state
            .prompt
            .paste_confirm(crate::app::prompt::PasteAction::Path)
    );
    assert!(!tick(&mut state, now + Duration::from_millis(1800)));
    assert!(tick(&mut state, now + Duration::from_millis(3000)));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::SavePromptContent { .. })),
        "面板关闭后落库"
    );
}

#[test]
fn manual_save_is_ignored_while_paste_proposal_open() {
    let mut state = item_editing_state();
    stage_paste(&mut state);
    state.effects.clear();
    save_prompt_shortcut(&mut state);
    assert!(state.effects.is_empty(), "占位文本不落库");
    assert!(!state.prompt.is_saved(), "不改已保存态");
    assert!(state.prompt.paste_panel().is_some(), "面板保持打开");

    assert!(
        state
            .prompt
            .paste_confirm(crate::app::prompt::PasteAction::Path)
    );
    save_prompt_shortcut(&mut state);
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::SavePromptContent { .. })),
        "面板关闭后可手动保存"
    );
}

/// 落库效应里的正文；无保存效应时 panic。
fn saved_content(state: &AppState) -> String {
    state
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SavePromptContent { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("条目保存效应")
}

/// 打开条目 `nested`、写入未保存的 `X`、再打开粘贴提案。
fn item_editing_with_paste() -> AppState {
    let mut state = item_editing_state();
    state.prompt.insert_str("X");
    stage_paste(&mut state);
    state.effects.clear();
    state
}

#[test]
fn switching_to_tree_commits_pre_paste_edits_without_placeholder() {
    let mut state = item_editing_with_paste();
    show_prompt_tree(&mut state);
    let content = saved_content(&state);
    assert_eq!(content, "nested contentX", "提交粘贴前的正文");
    assert!(!content.contains("/tmp/a.txt"), "占位文本不落库");
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(12)
            .expect("item 12")
            .active_content(),
        "nested contentX",
        "条目内容为粘贴前正文"
    );
}

#[test]
fn switching_to_temp_prompt_commits_pre_paste_edits_without_placeholder() {
    let mut state = item_editing_with_paste();
    assert!(open_temp_prompt(&mut state));
    let content = saved_content(&state);
    assert_eq!(content, "nested contentX", "提交粘贴前的正文");
    assert!(!content.contains("/tmp/a.txt"), "占位文本不落库");
}

#[test]
fn quit_commits_pre_paste_edits_without_placeholder() {
    let mut state = item_editing_with_paste();
    apply(Action::Quit, &mut state);
    assert!(state.should_quit);
    let content = saved_content(&state);
    assert_eq!(content, "nested contentX", "退出前提交粘贴前的正文");
    assert!(!content.contains("/tmp/a.txt"), "占位文本不落库");
}

#[test]
fn closing_displayed_workspace_commits_pre_paste_edits_without_placeholder() {
    let mut state = item_editing_with_paste();
    // 钉住 ws0：新建工作区不切换显示，粘贴提案仍属于被关闭的显示工作区。
    toggle_prompt_pin(&mut state);
    create_workspace(&mut state);
    assert_eq!(state.prompt_workspace(), 0);
    state.effects.clear();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Workspace(0),
    }));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 1);
    let content = saved_content(&state);
    assert_eq!(content, "nested contentX", "关闭前提交粘贴前的正文");
    assert!(!content.contains("/tmp/a.txt"), "占位文本不落库");
}

#[test]
fn apply_tree_unbinds_editing_when_item_removed_by_sharing_panel() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.editing = Some(PromptEditing::Item(10));
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;
    // 同键面板删除条目 10 后重载：编辑绑定解除并退回树视图。
    apply_prompt_tree(
        &mut state,
        0,
        Path::new("/ws"),
        vec![],
        vec![PromptRecord {
            id: 11,
            folder_id: None,
            name: "remaining".into(),
            content: "c".into(),
            active_page_id: Some(11),
            pages: vec![PromptPageRecord {
                id: 11,
                position: 0,
                name: "Page 1".into(),
                content: "c".into(),
            }],
        }],
    );
    assert!(state.workspaces[0].prompt_panel.editing.is_none());
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
}

// ===== 分页行为 =====

/// 给条目 10 追加第二页（id 110）并返回两页 id。
fn add_second_page(state: &mut AppState) -> (PromptPageId, PromptPageId) {
    let item = state.workspaces[0]
        .prompt_panel
        .tree
        .item_mut(10)
        .expect("item 10");
    item.pages.push(PromptPage::saved(
        110,
        text::default_page_name(2),
        "second page",
    ));
    (PromptPageId::Saved(10), PromptPageId::Saved(110))
}

/// demo 树的完整记录快照（重载用）；`active` / 第一页正文可定制。
fn reload_records(active: Option<i64>, first_content: &str, second: bool) -> Vec<PromptRecord> {
    let mut pages = vec![PromptPageRecord {
        id: 10,
        position: 0,
        name: "Page 1".into(),
        content: first_content.into(),
    }];
    if second {
        pages.push(PromptPageRecord {
            id: 110,
            position: 1,
            name: "Page 2".into(),
            content: "second page".into(),
        });
    }
    vec![PromptRecord {
        id: 10,
        folder_id: None,
        name: "loose".into(),
        content: first_content.into(),
        active_page_id: active,
        pages,
    }]
}

#[test]
fn switch_page_settles_edits_and_isolates_undo_history() {
    let mut state = demo_state();
    let (first, second) = add_second_page(&mut state);
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str(" edited");
    assert!(state.prompt.can_undo());
    assert!(select_prompt_page(&mut state, second));
    assert_eq!(state.prompt.text(), "second page");
    assert!(!state.prompt.can_undo(), "切页清空撤销历史（与旧页隔离）");
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            Effect::SavePromptContent { id: 10, page_id: 10, content, .. }
                if content == "loose content edited"
        )),
        "旧页正文先结算保存"
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::SetActivePage {
            id: 10,
            page_id: 110,
            ..
        }
    )));
    // 切回旧页可见刚才的编辑，且页身份回写。
    assert!(select_prompt_page(&mut state, first));
    assert_eq!(state.prompt.text(), "loose content edited");
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::SetActivePage {
            id: 10,
            page_id: 10,
            ..
        }
    )));
}

#[test]
fn relative_page_switch_stops_at_edges() {
    let mut state = demo_state();
    let (_first, second) = add_second_page(&mut state);
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert!(!switch_prompt_page(&mut state, -1), "第一页左箭头置灰");
    assert!(switch_prompt_page(&mut state, 1));
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(10)
            .unwrap()
            .active_page,
        second
    );
    assert!(!switch_prompt_page(&mut state, 1), "末页右箭头不隐式新建");
    assert_eq!(
        state
            .effects
            .iter()
            .filter(|effect| matches!(effect, Effect::CreatePage { .. }))
            .count(),
        0
    );
}

#[test]
fn page_rename_commits_non_empty_and_keeps_name_on_escape() {
    let mut state = demo_state();
    let (_first, second) = add_second_page(&mut state);
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert!(select_prompt_page(&mut state, second));
    state.effects.clear();
    assert!(begin_page_rename(&mut state));
    assert!(page_rename_active(&state));
    // 空名回车拒绝并保持输入态。
    apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Clear);
    apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(page_rename_active(&state), "空名拒绝并保持输入态");
    for ch in "Intro".chars() {
        apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Char(ch));
    }
    apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(!page_rename_active(&state));
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert_eq!(item.page(second).unwrap().name, "Intro");
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::RenamePage { page_id: 110, name, .. } if name == "Intro"
    )));
    // Esc 保留改名后的名称。
    assert!(begin_page_rename(&mut state));
    apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Clear);
    apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Esc);
    assert!(!page_rename_active(&state));
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert_eq!(item.page(second).unwrap().name, "Intro");
}

#[test]
fn delete_current_page_confirms_non_empty_and_guards_last_page() {
    let mut state = demo_state();
    let (first, second) = add_second_page(&mut state);
    assert!(edit_prompt_item(&mut state, 0, 10));
    // 非空页：先二次确认；取消后页与正文保持原状。
    assert!(delete_current_page(&mut state));
    assert!(matches!(
        state.overlay,
        Some(Overlay::ConfirmClose(ConfirmClose {
            target: OverlayTarget::PromptPage { page, .. },
        })) if page == first
    ));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Esc);
    assert!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(10)
            .unwrap()
            .page(first)
            .is_some()
    );
    // 确认删除：显示原下一页。
    assert!(delete_current_page(&mut state));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert!(item.page(first).is_none());
    assert_eq!(item.active_page, second);
    assert_eq!(state.prompt.text(), "second page");
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::DeletePage {
            id: 10,
            page_id: 10,
            ..
        }
    )));
    // 空白页直接删除（无确认浮层）。
    {
        let item = state.workspaces[0].prompt_panel.tree.item_mut(10).unwrap();
        item.pages
            .push(PromptPage::saved(120, text::default_page_name(3), ""));
        item.active_page = PromptPageId::Saved(120);
    }
    assert!(delete_current_page(&mut state));
    assert!(state.overlay.is_none(), "空白页无确认直接删除");
    assert!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(10)
            .unwrap()
            .page(PromptPageId::Saved(120))
            .is_none()
    );
    // 最后一页不可删除。
    assert!(!delete_current_page(&mut state));
}

#[test]
fn page_menu_snapshots_counts_and_switches_on_activate() {
    let mut state = demo_state();
    {
        let item = state.workspaces[0].prompt_panel.tree.item_mut(10).unwrap();
        item.pages[0].content = "&&& addTemplate --start\nbody\n&&& addTemplate --end".into();
        item.pages.push(PromptPage::saved(
            110,
            "Page 2",
            "&&& bugTemplate --start\n&&& bugTemplate --end",
        ));
    }
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert!(open_prompt_page_menu(&mut state, (3, 3)));
    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    assert_eq!(menu.rows.len(), 4, "New page + 两页 + Delete page");
    assert!(matches!(menu.rows[0], PageMenuRow::NewPage));
    assert!(matches!(
        &menu.rows[1],
        PageMenuRow::Page { current: true, counts, .. } if counts.todo == 1
    ));
    assert!(matches!(
        &menu.rows[2],
        PageMenuRow::Page { current: false, counts, .. } if counts.todo == 1
    ));
    assert!(matches!(
        menu.rows[3],
        PageMenuRow::DeletePage { enabled: true }
    ));
    assert_eq!(menu.selected, 1, "当前页行默认高亮");
    assert!(set_page_menu_selection(&mut state, 2));
    assert!(activate_page_menu_row(&mut state));
    assert!(state.overlay.is_none(), "选择页行后关闭菜单");
    assert_eq!(
        state.prompt.text(),
        "&&& bugTemplate --start\n&&& bugTemplate --end"
    );
}

#[test]
fn page_menu_delete_disabled_for_single_page() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert!(open_prompt_page_menu(&mut state, (3, 3)));
    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let last = menu.rows.len() - 1;
    assert!(matches!(
        menu.rows.last(),
        Some(PageMenuRow::DeletePage { enabled: false })
    ));
    assert!(set_page_menu_selection(&mut state, last));
    assert!(!activate_page_menu_row(&mut state), "禁用删除不动作");
    assert!(state.overlay.is_some(), "菜单保持打开");
}

#[test]
fn copy_content_uses_active_page_only() {
    let mut state = demo_state();
    let (_first, second) = add_second_page(&mut state);
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert!(select_prompt_page(&mut state, second));
    state.effects.clear();
    copy_prompt_content(&mut state, 0, 10);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CopyToClipboard { text, .. } if text == "second page"
    )));
}

#[test]
fn tree_reload_preserves_pending_page_content() {
    let mut state = demo_state();
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str(" edited");
    commit_prompt_buffer(&mut state, 0);
    assert!(state.workspaces[0].prompt_panel.page_pending(10, 10));
    // 别处改名触发的重载：数据库正文仍旧，但未确认页保留内存正文与标记。
    apply_prompt_tree(
        &mut state,
        0,
        Path::new("/ws"),
        vec![],
        reload_records(Some(10), "loose content", false),
    );
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert_eq!(
        item.active_content(),
        "loose content edited",
        "未确认页不被重载覆盖"
    );
    assert!(state.workspaces[0].prompt_panel.page_pending(10, 10));
}

#[test]
fn tree_reload_rebinds_editor_when_shared_panel_switches_page() {
    let mut state = demo_state();
    {
        let item = state.workspaces[0].prompt_panel.tree.item_mut(10).unwrap();
        item.pages.push(PromptPage::saved(
            110,
            text::default_page_name(2),
            "second page",
        ));
    }
    assert!(edit_prompt_item(&mut state, 0, 10));
    state.prompt.insert_str(" edited");
    state.effects.clear();
    // 同键面板把当前页切到第二页后重载：旧页先结算，再重绑新页。
    apply_prompt_tree(
        &mut state,
        0,
        Path::new("/ws"),
        vec![],
        reload_records(Some(110), "loose content", true),
    );
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert_eq!(item.active_page, PromptPageId::Saved(110));
    assert_eq!(state.prompt.text(), "second page", "编辑缓冲重绑新当前页");
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            Effect::SavePromptContent { page_id: 10, content, .. }
                if content == "loose content edited"
        )),
        "旧页正文先结算保存"
    );
}

#[test]
fn overlay_enter_activates_page_menu_and_escape_closes_without_switch() {
    let mut state = demo_state();
    let (_first, second) = add_second_page(&mut state);
    assert!(edit_prompt_item(&mut state, 0, 10));
    assert!(select_prompt_page(&mut state, second));
    // 菜单当前页行高亮；上移到第一页行并回车：切页并关闭。
    assert!(open_prompt_page_menu(&mut state, (3, 3)));
    assert!(set_page_menu_selection(&mut state, 1));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Enter);
    assert!(state.overlay.is_none());
    assert_eq!(state.prompt.text(), "loose content");
    // Esc 关闭菜单且不改正文。
    assert!(open_prompt_page_menu(&mut state, (3, 3)));
    apply_overlay_key(&mut state, crate::app::actions::OverlayKey::Esc);
    assert!(state.overlay.is_none());
    assert_eq!(state.prompt.text(), "loose content");
}

#[test]
fn tree_load_repairs_invalid_active_page_to_first() {
    let mut state = demo_state();
    state.effects.clear();
    // 记录的当前页 id 失效：读取回退第一页并登记修复。
    apply_prompt_tree(
        &mut state,
        0,
        Path::new("/ws"),
        vec![],
        reload_records(Some(999), "loose content", false),
    );
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert_eq!(
        item.active_page,
        PromptPageId::Saved(10),
        "失效 id 回退第一页"
    );
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            Effect::SetActivePage {
                id: 10,
                page_id: 10,
                ..
            }
        )),
        "登记当前页修复: {:?}",
        state.effects
    );
}
