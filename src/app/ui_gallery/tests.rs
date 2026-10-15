//! 单元测试：组件树行序列、折叠、选择移动与左右导航。

use super::*;

use crate::app::ui_gallery::AgentStepId;

fn folder_row(id: UiGalleryFolderId) -> UiGalleryNode {
    UiGalleryNode::Folder(id)
}

fn component_row(id: UiGalleryComponentId) -> UiGalleryNode {
    UiGalleryNode::Component(id)
}

/// `agent` 分组的全部步骤行（按固定顺序）。
fn agent_rows() -> Vec<UiGalleryNode> {
    AgentStepId::ALL
        .iter()
        .map(|step| component_row(UiGalleryComponentId::Agent(*step)))
        .collect()
}

#[test]
fn default_rows_list_folders_then_components() {
    let gallery = UiGallery::new();
    let nodes: Vec<UiGalleryNode> = gallery.rows().into_iter().map(|row| row.node).collect();
    let mut expected = vec![
        folder_row(UiGalleryFolderId::Global),
        component_row(UiGalleryComponentId::Toast),
        component_row(UiGalleryComponentId::Modal),
        component_row(UiGalleryComponentId::Tooltip),
        component_row(UiGalleryComponentId::Button),
        component_row(UiGalleryComponentId::Menu),
        component_row(UiGalleryComponentId::Fullscreen),
        folder_row(UiGalleryFolderId::Prompt),
        component_row(UiGalleryComponentId::PromptEditor),
        component_row(UiGalleryComponentId::PromptList),
        component_row(UiGalleryComponentId::PromptToolbar),
        component_row(UiGalleryComponentId::ReferencedFolders),
        component_row(UiGalleryComponentId::FolderSearch),
        folder_row(UiGalleryFolderId::Agent),
    ];
    expected.extend(agent_rows());
    assert_eq!(nodes, expected);
    assert_eq!(gallery.selected(), 0);
    assert_eq!(
        gallery.selected_node(),
        folder_row(UiGalleryFolderId::Global)
    );
}

#[test]
fn row_depth_marks_folders_at_zero_and_components_at_one() {
    let gallery = UiGallery::new();
    for row in gallery.rows() {
        match row.node {
            UiGalleryNode::Folder(_) => assert_eq!(row.depth, 0),
            UiGalleryNode::Component(_) => assert_eq!(row.depth, 1),
        }
    }
}

#[test]
fn collapsing_folder_hides_its_components() {
    let mut gallery = UiGallery::new();
    assert!(gallery.toggle_folder(UiGalleryFolderId::Global));
    let nodes: Vec<UiGalleryNode> = gallery.rows().into_iter().map(|row| row.node).collect();
    let mut expected = vec![
        folder_row(UiGalleryFolderId::Global),
        folder_row(UiGalleryFolderId::Prompt),
        component_row(UiGalleryComponentId::PromptEditor),
        component_row(UiGalleryComponentId::PromptList),
        component_row(UiGalleryComponentId::PromptToolbar),
        component_row(UiGalleryComponentId::ReferencedFolders),
        component_row(UiGalleryComponentId::FolderSearch),
        folder_row(UiGalleryFolderId::Agent),
    ];
    expected.extend(agent_rows());
    assert_eq!(nodes, expected);
    assert!(gallery.is_collapsed(UiGalleryFolderId::Global));
    assert!(!gallery.is_collapsed(UiGalleryFolderId::Prompt));
    assert!(!gallery.is_collapsed(UiGalleryFolderId::Agent));
}

#[test]
fn collapse_moves_selection_to_the_folder_row() {
    let mut gallery = UiGallery::new();
    // 高亮移到 global 的末项 fullscreen（行 6）。
    gallery.set_selected(6);
    assert_eq!(
        gallery.selected_node(),
        component_row(UiGalleryComponentId::Fullscreen)
    );
    // 折叠 global 后其组件行消失，高亮回到 global 文件夹行。
    gallery.toggle_folder(UiGalleryFolderId::Global);
    assert_eq!(
        gallery.selected_node(),
        folder_row(UiGalleryFolderId::Global)
    );
}

#[test]
fn move_selection_clamps_without_wrapping() {
    let mut gallery = UiGallery::new();
    assert!(!gallery.move_selection(-1));
    assert_eq!(gallery.selected(), 0);
    let last = gallery.rows().len() - 1;
    gallery.set_selected(last);
    assert!(!gallery.move_selection(1));
    assert_eq!(gallery.selected(), last);
    assert!(gallery.move_selection(-3));
    assert_eq!(gallery.selected(), last - 3);
}

#[test]
fn set_selected_rejects_out_of_range() {
    let mut gallery = UiGallery::new();
    let len = gallery.rows().len();
    assert!(!gallery.set_selected(len));
    assert!(!gallery.set_selected(0));
    assert!(gallery.set_selected(1));
    assert_eq!(gallery.selected(), 1);
}

#[test]
fn activate_toggles_only_folder_rows() {
    let mut gallery = UiGallery::new();
    assert!(gallery.activate());
    assert!(gallery.is_collapsed(UiGalleryFolderId::Global));
    // 高亮仍在文件夹行：再次回车展开。
    assert!(gallery.activate());
    assert!(!gallery.is_collapsed(UiGalleryFolderId::Global));
    // 移到组件行：回车无动作。
    gallery.set_selected(1);
    assert_eq!(
        gallery.selected_node(),
        component_row(UiGalleryComponentId::Toast)
    );
    assert!(!gallery.activate());
}

#[test]
fn left_from_component_moves_to_owner_folder() {
    let mut gallery = UiGallery::new();
    gallery.set_selected(9);
    assert_eq!(
        gallery.selected_node(),
        component_row(UiGalleryComponentId::PromptList)
    );
    assert!(gallery.move_left());
    assert_eq!(
        gallery.selected_node(),
        folder_row(UiGalleryFolderId::Prompt)
    );
}

#[test]
fn left_from_expanded_folder_collapses_it() {
    let mut gallery = UiGallery::new();
    assert!(gallery.move_left());
    assert!(gallery.is_collapsed(UiGalleryFolderId::Global));
    // 已折叠时再按左键无动作。
    assert!(!gallery.move_left());
}

#[test]
fn right_expands_collapsed_folder_only() {
    let mut gallery = UiGallery::new();
    assert!(!gallery.move_right());
    gallery.toggle_folder(UiGalleryFolderId::Global);
    assert!(gallery.move_right());
    assert!(!gallery.is_collapsed(UiGalleryFolderId::Global));
    // 组件行上右键无动作。
    gallery.set_selected(1);
    assert!(!gallery.move_right());
}

#[test]
fn selected_component_exposes_description() {
    let mut gallery = UiGallery::new();
    gallery.set_selected(1);
    let component = gallery.selected_component().expect("component selected");
    assert_eq!(component.id, UiGalleryComponentId::Toast);
    assert!(!component.description.is_empty());
    gallery.set_selected(0);
    assert!(gallery.selected_component().is_none());
}

#[test]
fn folder_and_component_lookup_by_id() {
    let gallery = UiGallery::new();
    assert_eq!(gallery.folders().len(), 3);
    assert_eq!(
        gallery
            .folder(UiGalleryFolderId::Global)
            .map(|f| f.components.len()),
        Some(6)
    );
    assert_eq!(
        gallery
            .folder(UiGalleryFolderId::Prompt)
            .map(|f| f.components.len()),
        Some(5)
    );
    assert_eq!(
        gallery
            .folder(UiGalleryFolderId::Agent)
            .map(|f| f.components.len()),
        Some(17)
    );
    assert_eq!(
        gallery
            .component(UiGalleryComponentId::PromptToolbar)
            .map(|c| c.id),
        Some(UiGalleryComponentId::PromptToolbar)
    );
    assert_eq!(
        UiGalleryFolderId::Global.label(),
        text::UI_GALLERY_FOLDER_GLOBAL
    );
    assert_eq!(UiGalleryComponentId::Modal.label(), text::UI_GALLERY_MODAL);
    assert_eq!(UiGalleryFolderId::Agent.label(), text::agent::FOLDER);
    // 每个 agent 步骤都能按 id 查到，且指向同一 step。
    for step in AgentStepId::ALL {
        let id = UiGalleryComponentId::Agent(step);
        assert_eq!(gallery.component(id).map(|c| c.id), Some(id));
        assert_eq!(id.label(), step.label());
        assert!(!gallery.component(id).unwrap().description.is_empty());
    }
}
