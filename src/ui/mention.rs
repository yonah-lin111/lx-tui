//! `@` 提及条目展示构造：目录/文件图标、名称与父目录明细，以及 Skills / MCP 条目。
//!
//! prompt 与 lx 的提及面板共用同一套条目外观；这里只做纯数据到
//! `CommandItem` 的映射，不读取状态、不计算几何。

use unicode_width::UnicodeWidthStr;

use crate::app::markdown::MentionEntry;
use crate::ui::text;
use crate::ui::widgets::command_panel::CommandItem;

/// 父目录明细相对面板宽度的留白（图标列 + 两边框 + 左侧内边距）。
pub const MENTION_DETAIL_MARGIN: usize = 5;

/// 提及条目的展示行：文件/目录/skill/MCP/subagent（图标 + 名称 + 次行明细）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionRow {
    icon: &'static str,
    icon_style: ratatui::style::Style,
    label_style: ratatui::style::Style,
    label: String,
    detail: String,
}

impl MentionRow {
    /// 组装命令面板条目：目录、文件、skill/MCP 与各类 subagent 分色。
    pub fn item(&self) -> CommandItem<'_> {
        CommandItem::Stacked {
            icon: Some(self.icon),
            icon_style: self.icon_style,
            label: &self.label,
            label_style: self.label_style,
            detail: &self.detail,
        }
    }
}

/// 构造文件/目录条目展示：第一行图标 + 文件名（目录带 `/`），第二行 `└─ 父目录`。
///
/// 父目录超出可用宽度时按显示列中间截断（保留路径首尾）。
pub fn file_row(entry: &MentionEntry, detail_width: usize) -> MentionRow {
    let (directory, name) = entry
        .path
        .rsplit_once('/')
        .unwrap_or(("", entry.path.as_str()));
    let label = if entry.is_directory {
        format!("{name}/")
    } else {
        name.to_string()
    };
    let detail = if directory.is_empty() {
        String::new()
    } else {
        let prefix = format!("{} ", text::WORKSPACE_TREE_LAST);
        let path_width = detail_width.saturating_sub(prefix.width());
        format!("{prefix}{}", text::ellipsize_middle(directory, path_width))
    };
    let icon = if entry.is_directory {
        text::MENTION_DIR_ICON
    } else {
        text::MENTION_FILE_ICON
    };
    let (icon_style, label_style) = if entry.is_directory {
        let directory = crate::ui::style::directory();
        (directory, directory)
    } else {
        (crate::ui::style::muted(), crate::ui::style::text())
    };
    MentionRow {
        icon,
        icon_style,
        label_style,
        label,
        detail,
    }
}

/// 构造 skill 条目展示：`◆ name` + `Skill · description`。
pub fn skill_row(name: &str, description: &str, detail_width: usize) -> MentionRow {
    let detail = if description.is_empty() {
        text::MENTION_SKILL_LABEL.to_string()
    } else {
        format!(
            "{}{}{}",
            text::MENTION_SKILL_LABEL,
            text::LX_CHAT_TOOL_STATUS_SEP,
            description
        )
    };
    let skill = crate::ui::style::chat_tool_param();
    MentionRow {
        icon: text::MENTION_SKILL_ICON,
        icon_style: skill,
        label_style: skill,
        label: name.to_string(),
        detail: text::truncate_to_width(&detail, detail_width),
    }
}

/// 构造 MCP server 条目展示：`◆ name` + `MCP · detail`。
pub fn mcp_row(name: &str, detail: &str, detail_width: usize) -> MentionRow {
    let detail_text = if detail.is_empty() {
        text::MENTION_MCP_LABEL.to_string()
    } else {
        format!(
            "{}{}{}",
            text::MENTION_MCP_LABEL,
            text::LX_CHAT_TOOL_STATUS_SEP,
            detail
        )
    };
    let skill = crate::ui::style::chat_tool_param();
    MentionRow {
        icon: text::MENTION_SKILL_ICON,
        icon_style: skill,
        label_style: skill,
        label: name.to_string(),
        detail: text::truncate_to_width(&detail_text, detail_width),
    }
}

/// 按 subagent 候选类别与存活状态返回首行 `● + 名称` 的样式。
pub fn subagent_item_style(item: &crate::app::state::SubagentMentionItem) -> ratatui::style::Style {
    match item {
        crate::app::state::SubagentMentionItem::Active(run) => {
            crate::ui::style::mention_subagent_active(&run.status)
        }
        crate::app::state::SubagentMentionItem::Role(role) => {
            if role.is_builtin {
                crate::ui::style::mention_subagent_builtin()
            } else {
                crate::ui::style::mention_subagent_custom()
            }
        }
    }
}

/// 构造 subagent 候选条目展示：首行 `● label`（按类型/状态着色），次行类别与状态/描述明细。
pub fn subagent_row(
    item: &crate::app::state::SubagentMentionItem,
    detail_width: usize,
) -> MentionRow {
    let (label, detail) = subagent_item_text(item);
    let style = subagent_item_style(item);
    MentionRow {
        icon: text::MENTION_SUBAGENT_ICON,
        icon_style: style,
        label_style: style,
        label,
        detail: text::truncate_to_width(&detail, detail_width),
    }
}

/// 组装 subagent 候选项的 `(label, detail)` 文案。
pub fn subagent_item_text(item: &crate::app::state::SubagentMentionItem) -> (String, String) {
    let sep = text::LX_CHAT_TOOL_STATUS_SEP;
    match item {
        crate::app::state::SubagentMentionItem::Active(run) => {
            let label = run.display_label();
            let detail = if run.title.trim().is_empty() {
                format!("{}{sep}{}", text::MENTION_SUBAGENT_ACTIVE_LABEL, run.status)
            } else {
                format!(
                    "{}{sep}{}{sep}{}",
                    text::MENTION_SUBAGENT_ACTIVE_LABEL,
                    run.status,
                    run.title.trim()
                )
            };
            (label, detail)
        }
        crate::app::state::SubagentMentionItem::Role(role) => {
            let prefix = if role.is_builtin {
                text::MENTION_SUBAGENT_BUILTIN_LABEL.to_string()
            } else {
                let scope = if role.is_project_scoped {
                    "project"
                } else {
                    "user"
                };
                format!(
                    "{} ({scope}{sep}{})",
                    text::MENTION_SUBAGENT_CUSTOM_LABEL,
                    role.base_role.as_str()
                )
            };
            let detail = if role.description.trim().is_empty() {
                prefix
            } else {
                format!("{prefix}{sep}{}", role.description.trim())
            };
            (role.name.clone(), detail)
        }
    }
}

/// 构造引用文件夹条目展示：第一行目录图标 + 目录名 `/`，第二行 `└─ 绝对路径`。
pub fn folder_row(name: &str, path: &str, detail_width: usize) -> MentionRow {
    let label = format!("{name}/");
    let prefix = format!("{} ", text::WORKSPACE_TREE_LAST);
    let path_width = detail_width.saturating_sub(prefix.width());
    let detail = format!("{prefix}{}", text::ellipsize_middle(path, path_width));
    let directory = crate::ui::style::directory();
    MentionRow {
        icon: text::MENTION_DIR_ICON,
        icon_style: directory,
        label_style: directory,
        label,
        detail,
    }
}
