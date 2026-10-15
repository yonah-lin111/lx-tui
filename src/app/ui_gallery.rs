//! UI 组件查看弹窗的纯内存模型：静态组件树、折叠状态与高亮行。
//!
//! 组件目录为编译期常量，不进数据库、不依赖工作区；唯一可变状态是
//! 折叠的文件夹集合与当前高亮可见行，供浮层渲染与键鼠命中只读读取。

use crate::ui::text;

/// 组件种类标识：右侧预览按此分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiGalleryComponentId {
    Toast,
    Modal,
    Tooltip,
    Button,
    Menu,
    Fullscreen,
    PromptEditor,
    PromptList,
    PromptToolbar,
}

impl UiGalleryComponentId {
    /// 右侧预览标题（英文短名）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Toast => text::UI_GALLERY_TOAST,
            Self::Modal => text::UI_GALLERY_MODAL,
            Self::Tooltip => text::UI_GALLERY_TOOLTIP,
            Self::Button => text::UI_GALLERY_BUTTON,
            Self::Menu => text::UI_GALLERY_MENU,
            Self::Fullscreen => text::UI_GALLERY_FULLSCREEN,
            Self::PromptEditor => text::UI_GALLERY_PROMPT_EDITOR,
            Self::PromptList => text::UI_GALLERY_PROMPT_LIST,
            Self::PromptToolbar => text::UI_GALLERY_PROMPT_TOOLBAR,
        }
    }
}

/// 顶层文件夹标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiGalleryFolderId {
    Global,
    Prompt,
}

impl UiGalleryFolderId {
    /// 文件夹展示名（英文小写）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => text::UI_GALLERY_FOLDER_GLOBAL,
            Self::Prompt => text::UI_GALLERY_FOLDER_PROMPT,
        }
    }
}

/// 一个组件（叶子）：种类与右侧说明段落。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGalleryComponent {
    pub id: UiGalleryComponentId,
    /// 右侧用途/触发说明，逐行渲染。
    pub description: &'static [&'static str],
}

/// 顶层文件夹：种类与下属组件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGalleryFolder {
    pub id: UiGalleryFolderId,
    pub components: &'static [UiGalleryComponent],
}

/// `global` 文件夹下的组件。
pub const GLOBAL_COMPONENTS: [UiGalleryComponent; 6] = [
    UiGalleryComponent {
        id: UiGalleryComponentId::Toast,
        description: text::UI_GALLERY_TOAST_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::Modal,
        description: text::UI_GALLERY_MODAL_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::Tooltip,
        description: text::UI_GALLERY_TOOLTIP_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::Button,
        description: text::UI_GALLERY_BUTTON_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::Menu,
        description: text::UI_GALLERY_MENU_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::Fullscreen,
        description: text::UI_GALLERY_FULLSCREEN_DESC,
    },
];

/// `prompt` 文件夹下的组件。
pub const PROMPT_COMPONENTS: [UiGalleryComponent; 3] = [
    UiGalleryComponent {
        id: UiGalleryComponentId::PromptEditor,
        description: text::UI_GALLERY_PROMPT_EDITOR_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::PromptList,
        description: text::UI_GALLERY_PROMPT_LIST_DESC,
    },
    UiGalleryComponent {
        id: UiGalleryComponentId::PromptToolbar,
        description: text::UI_GALLERY_PROMPT_TOOLBAR_DESC,
    },
];

/// 组件树全部顶层文件夹（固定顺序）。
pub const GALLERY_FOLDERS: [UiGalleryFolder; 2] = [
    UiGalleryFolder {
        id: UiGalleryFolderId::Global,
        components: &GLOBAL_COMPONENTS,
    },
    UiGalleryFolder {
        id: UiGalleryFolderId::Prompt,
        components: &PROMPT_COMPONENTS,
    },
];

/// 左侧树的可见行种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiGalleryNode {
    /// 文件夹行（可折叠）。
    Folder(UiGalleryFolderId),
    /// 组件行（叶子）。
    Component(UiGalleryComponentId),
}

/// 左侧树的一行：节点种类与缩进深度（文件夹 0、组件 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGalleryRow {
    pub node: UiGalleryNode,
    pub depth: usize,
}

/// UI 组件查看弹窗状态：折叠集合与高亮可见行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiGallery {
    folders: &'static [UiGalleryFolder],
    collapsed: Vec<UiGalleryFolderId>,
    selected: usize,
}

impl Default for UiGallery {
    fn default() -> Self {
        Self::new()
    }
}

impl UiGallery {
    /// 默认状态：全部展开、高亮首行。
    pub fn new() -> Self {
        Self {
            folders: &GALLERY_FOLDERS,
            collapsed: Vec::new(),
            selected: 0,
        }
    }

    /// 全部文件夹。
    pub fn folders(&self) -> &'static [UiGalleryFolder] {
        self.folders
    }

    /// 按种类取文件夹。
    pub fn folder(&self, id: UiGalleryFolderId) -> Option<&'static UiGalleryFolder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// 按种类取组件；跨全部文件夹查找。
    pub fn component(&self, id: UiGalleryComponentId) -> Option<&'static UiGalleryComponent> {
        self.folders
            .iter()
            .flat_map(|folder| folder.components.iter())
            .find(|component| component.id == id)
    }

    /// 文件夹是否折叠。
    pub fn is_collapsed(&self, id: UiGalleryFolderId) -> bool {
        self.collapsed.contains(&id)
    }

    /// 高亮可见行下标。
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 可见行：文件夹行在前，展开时紧随其组件行。
    pub fn rows(&self) -> Vec<UiGalleryRow> {
        let mut rows = Vec::new();
        for folder in self.folders {
            rows.push(UiGalleryRow {
                node: UiGalleryNode::Folder(folder.id),
                depth: 0,
            });
            if self.is_collapsed(folder.id) {
                continue;
            }
            for component in folder.components {
                rows.push(UiGalleryRow {
                    node: UiGalleryNode::Component(component.id),
                    depth: 1,
                });
            }
        }
        rows
    }

    /// 高亮行节点；行集合恒非空，越界回退首行。
    pub fn selected_node(&self) -> UiGalleryNode {
        let rows = self.rows();
        rows.get(self.selected)
            .or_else(|| rows.first())
            .map(|row| row.node)
            .unwrap_or(UiGalleryNode::Folder(UiGalleryFolderId::Global))
    }

    /// 高亮组件；高亮为文件夹行时为 None。
    pub fn selected_component(&self) -> Option<&'static UiGalleryComponent> {
        match self.selected_node() {
            UiGalleryNode::Component(id) => self.component(id),
            UiGalleryNode::Folder(_) => None,
        }
    }

    /// 把高亮钳到可见行范围内。
    fn clamp(&mut self) {
        let len = self.rows().len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// 高亮按步长在可见行间钳制移动（不循环）；返回是否变化。
    pub fn move_selection(&mut self, delta: isize) -> bool {
        let len = self.rows().len();
        if len == 0 {
            return false;
        }
        let next = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        if next == self.selected {
            return false;
        }
        self.selected = next;
        true
    }

    /// 直接设置高亮行；越界忽略并返回是否变化。
    pub fn set_selected(&mut self, index: usize) -> bool {
        let len = self.rows().len();
        if index >= len || index == self.selected {
            return false;
        }
        self.selected = index;
        true
    }

    /// 切换文件夹折叠；不存在时 no-op。
    ///
    /// 折叠时若高亮正落在将被隐藏的后代组件行上，则把高亮移到该文件夹行，
    /// 避免高亮漂到无关行；其余情况高亮越界则钳回。
    pub fn toggle_folder(&mut self, id: UiGalleryFolderId) -> bool {
        let Some(folder) = self.folder(id) else {
            return false;
        };
        if self.is_collapsed(id) {
            self.collapsed.retain(|candidate| *candidate != id);
        } else {
            let mut selected = self.selected;
            if let UiGalleryNode::Component(component) = self.selected_node()
                && folder
                    .components
                    .iter()
                    .any(|candidate| candidate.id == component)
                && let Some(index) = self.folder_row_index(id)
            {
                selected = index;
            }
            self.collapsed.push(id);
            self.selected = selected;
        }
        self.clamp();
        true
    }

    /// 文件夹行在可见行中的下标；不存在为 None。
    fn folder_row_index(&self, id: UiGalleryFolderId) -> Option<usize> {
        self.rows()
            .iter()
            .position(|row| matches!(row.node, UiGalleryNode::Folder(folder) if folder == id))
    }

    /// 回车/点击激活：文件夹行切换折叠，组件行无动作；返回是否变化。
    pub fn activate(&mut self) -> bool {
        match self.selected_node() {
            UiGalleryNode::Folder(id) => self.toggle_folder(id),
            UiGalleryNode::Component(_) => false,
        }
    }

    /// 高亮当前所在文件夹（用于鼠标点击定位）。
    fn folder_of_selected(&self) -> Option<UiGalleryFolderId> {
        match self.selected_node() {
            UiGalleryNode::Folder(id) => Some(id),
            UiGalleryNode::Component(_) => {
                let rows = self.rows();
                let mut folder = None;
                for row in rows.iter().take(self.selected + 1) {
                    if let UiGalleryNode::Folder(id) = row.node {
                        folder = Some(id);
                    }
                }
                folder
            }
        }
    }

    /// `←`：组件行或折叠/顶层文件夹行移到所属文件夹行；返回是否变化。
    pub fn move_left(&mut self) -> bool {
        match self.selected_node() {
            UiGalleryNode::Component(_) => {
                let Some(folder) = self.folder_of_selected() else {
                    return false;
                };
                match self
                    .rows()
                    .iter()
                    .position(|row| matches!(row.node, UiGalleryNode::Folder(id) if id == folder))
                {
                    Some(index) => self.set_selected(index),
                    None => false,
                }
            }
            UiGalleryNode::Folder(id) => {
                if self.is_collapsed(id) {
                    return false;
                }
                self.toggle_folder(id);
                true
            }
        }
    }

    /// `→`：展开当前高亮文件夹；返回是否变化。
    pub fn move_right(&mut self) -> bool {
        match self.selected_node() {
            UiGalleryNode::Folder(id) => {
                if self.is_collapsed(id) {
                    self.toggle_folder(id);
                    true
                } else {
                    false
                }
            }
            UiGalleryNode::Component(_) => false,
        }
    }
}

#[cfg(test)]
mod tests;
