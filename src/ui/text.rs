//! 全部用户可见文案；组件与逻辑禁止散落硬编码字符串。

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::{BlockCommandId, SlashCommandId, TemplateStatus};
use crate::detect::{AgentKind, AgentState};
use crate::layout::PaneId;

pub const SIDEBAR_TITLE: &str = "Workspaces";
/// 侧栏下半分区标题。
pub const SIDEBAR_AGENTS_TITLE: &str = "Agents";
pub const MIN_SIZE_HINT: &str = "terminal too small";

/// Agents 分区空状态。
pub const AGENTS_EMPTY: &str = "no agents running";
/// Agent 状态圆点：运行/阻塞实心、空闲空心。
pub const AGENT_STATUS_DOT: &str = "●";
pub const AGENT_STATUS_RING: &str = "○";

/// Agent 名称标签：稳定英文小写值（与进程匹配别名一致）。
pub fn agent_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::Gemini => "gemini",
        AgentKind::Antigravity => "agy",
        AgentKind::OpenCode => "opencode",
        AgentKind::Cursor => "cursor",
        AgentKind::Pi => "pi",
        AgentKind::Kimi => "kimi",
    }
}

/// Agent 状态圆点符号：Idle/Unknown 空心，Working/Blocked 实心。
pub fn agent_status_dot(state: AgentState) -> &'static str {
    match state {
        AgentState::Idle | AgentState::Unknown => AGENT_STATUS_RING,
        AgentState::Working | AgentState::Blocked => AGENT_STATUS_DOT,
    }
}

/// 窗格顶边框 Agent 标记前缀；前缀强调色、值 muted，与 prompt 的 `ws:` 样式对齐。
pub const PANE_AGENT_PREFIX: &str = "agent:";

/// Agents 条目归属标签 `[tab N:pM]`：N 为标签位置（1 基）、M 为窗格标识。
pub fn agent_location(tab_index: usize, pane: PaneId) -> String {
    format!("[tab {}:p{}]", tab_index.saturating_add(1), pane.raw())
}

/// prompt 面板边框标题；路径末段名以 `ws:` 前缀渲染在顶边框右侧。
pub const PROMPT_TITLE: &str = "Prompt";
/// prompt 底边框分支前缀。
pub const PROMPT_BRANCH_PREFIX: &str = "b:";
/// prompt 顶边框右侧的工作区路径名前缀。
pub const PROMPT_WORKSPACE_PREFIX: &str = "ws:";
/// prompt 底边框 linked worktree 标记前缀。
pub const PROMPT_WORKTREE_PREFIX: &str = "wt:";
/// prompt 工具栏按钮与保存状态点。
pub const PROMPT_UNDO_LABEL: &str = "[undo]";
pub const PROMPT_REDO_LABEL: &str = "[redo]";
pub const PROMPT_SELECT_ALL_LABEL: &str = "[select all]";
pub const PROMPT_SAVE_DOT: &str = "●";
/// prompt 未钉住时的按钮（点击钉住）。
pub const PROMPT_PIN_LABEL: &str = "[pin]";
/// prompt 已钉住时的按钮（点击释放）。
pub const PROMPT_UNPIN_LABEL: &str = "[unpin]";
/// prompt 编辑器空内容时的输入说明占位。
pub const PROMPT_PLACEHOLDER: &str = "Write your prompt…";
/// 主内容 lx 页标题。
pub const LX_TITLE: &str = "lx";
/// 视图切换按钮：显示点击后的目的地视图。
pub const LX_TOGGLE_TERMINAL: &str = "[>_]";
pub const LX_TOGGLE_LX: &str = "[lx]";
/// lx 页底部输入框（当前为纯视觉占位）。
pub const LX_INPUT_PROMPT: &str = ">";
pub const LX_INPUT_PLACEHOLDER: &str = "Ask anything…";
/// lx 页未来内容区的白色占位文案。
pub const LX_CONTENT_PLACEHOLDER: &str = "placeholder";

/// lx 页切换提示。
pub fn lx_hint() -> String {
    format!("click {} to open terminal", LX_TOGGLE_TERMINAL)
}
/// 折叠按钮：按面板所在侧镜像，箭头指向点击后移动的方向。
pub const SIDEBAR_COLLAPSE_LABEL: &str = "[◀]";
pub const SIDEBAR_EXPAND_LABEL: &str = "[▶]";
pub const PROMPT_COLLAPSE_LABEL: &str = "[▶]";
pub const PROMPT_EXPAND_LABEL: &str = "[◀]";
pub const STRIP_LINE: &str = "│";
/// 侧栏 agents 分区表头：横向分割线与竖直折叠按钮。
pub const DIVIDER_MID: &str = "─";
/// 横向分割线与外层边框的衔接符号：左端 / 右端。
pub const DIVIDER_LEFT_JOIN: &str = "├";
pub const DIVIDER_RIGHT_JOIN: &str = "┤";
pub const AGENTS_COLLAPSE_LABEL: &str = "[▼]";
pub const AGENTS_EXPAND_LABEL: &str = "[▲]";

/// 侧栏工作区列表底部的新建按钮。
pub const ADD_WORKSPACE_LABEL: &str = "[+]";

/// 标签栏：新建标签按钮、标签分隔线、溢出滚动按钮与条目左侧图标。
pub const ADD_TAB_LABEL: &str = "[+]";
pub const TAB_SEPARATOR: &str = "│";
pub const TAB_SCROLL_LEFT_LABEL: &str = "[<]";
pub const TAB_SCROLL_RIGHT_LABEL: &str = "[>]";
pub const TAB_ITEM_ICON: &str = "▪";

/// 初始工作区标记：不可移除，跟随启动工作区。
pub const INITIAL_WORKSPACE_MARKER: &str = " *";

/// 工作区条目标记：Git 仓库与普通目录（标准单宽通用符号）。
pub const WORKSPACE_GIT_ICON: &str = "⑂";
pub const WORKSPACE_NON_GIT_ICON: &str = "•";

/// 工作区分组折叠箭头：展开 / 折叠。
pub const WORKSPACE_GROUP_EXPANDED: &str = "▾";
pub const WORKSPACE_GROUP_COLLAPSED: &str = "▸";
/// 分组子项树形连接符：非末位 / 末位（按可见子项判定）。
pub const WORKSPACE_TREE_MIDDLE: &str = "├─";
pub const WORKSPACE_TREE_LAST: &str = "└─";

/// @ 提及面板条目标记：目录 / 文件（标准单宽通用符号，兼容无 Nerd Font 的终端）。
pub const MENTION_DIR_ICON: &str = "▸";
pub const MENTION_FILE_ICON: &str = "•";

/// 右键菜单项；按命令映射，禁止在逻辑层硬编码。
pub const MENU_NEW_TAB: &str = "New tab";
pub const MENU_NEW_TERMINAL: &str = "New terminal";
pub const MENU_RENAME_WORKSPACE: &str = "Rename";
pub const MENU_OPEN_WORKTREE: &str = "Open worktree";
pub const MENU_OPEN_PROMPT: &str = "Open prompt";
pub const MENU_CLOSE_WORKSPACE: &str = "Close";
pub const MENU_RENAME_TAB: &str = "Rename";
pub const MENU_CLOSE_TAB: &str = "Close";
pub const MENU_SPLIT_RIGHT: &str = "Split right";
pub const MENU_SPLIT_DOWN: &str = "Split down";
pub const MENU_SWITCH_TO_TERMINAL: &str = "Switch to terminal";
pub const MENU_SWITCH_TO_LX: &str = "Switch to lx";
pub const MENU_SWITCH_TO_WORKSPACE_CWD: &str = "Switch to ws path";
pub const MENU_SYNC_WS_TO_TERMINAL_CWD: &str = "Sync ws to terminal path";
pub const MENU_CLOSE_PANE: &str = "Close";

/// 重命名与关闭确认浮层标题。
pub const RENAME_WORKSPACE_TITLE: &str = "rename workspace";
pub const NEW_WORKSPACE_TITLE: &str = "new workspace";
/// rename 浮层输入为空时的输入说明占位（工作区与标签共用）。
pub const RENAME_PLACEHOLDER: &str = "new name";
/// new workspace 路径输入为空时的输入说明占位。
pub const NEW_WORKSPACE_PLACEHOLDER: &str = "path to directory";
pub const RENAME_TAB_TITLE: &str = "rename tab";
pub const CONFIRM_CLOSE_TITLE: &str = "close workspace";
pub const CONFIRM_CLOSE_TAB_TITLE: &str = "close tab";
pub const CONFIRM_CLOSE_PANE_TITLE: &str = "close pane";
pub const CONFIRM_SWITCH_CWD_TITLE: &str = "switch to ws path";
pub const CONFIRM_SYNC_WS_CWD_TITLE: &str = "sync ws to terminal path";

/// worktree 对话框：标题、搜索占位、状态行与条目标记。
pub const WORKTREE_OPEN_TITLE: &str = "open worktree";
pub const WORKTREE_OPEN_FILTER: &str = "filter worktrees";
pub const WORKTREE_OPEN_LOADING: &str = "loading…";
pub const WORKTREE_OPEN_FAILED: &str = "failed to list worktrees";
pub const WORKTREE_OPEN_EMPTY: &str = "no worktrees found";
pub const WORKTREE_OPEN_MARKER: &str = "›";
pub const WORKTREE_STATUS_OPEN: &str = "open";
pub const WORKTREE_STATUS_DETACHED: &str = "detached";
pub const WORKTREE_STATUS_ROOT: &str = "root";

/// 模态底部按钮：名称在前、快捷键在后。
pub const BUTTON_SAVE: &str = "[save enter]";
pub const BUTTON_CREATE: &str = "[create enter]";
pub const BUTTON_CLEAR: &str = "[clear ^c]";
pub const BUTTON_CANCEL: &str = "[cancel esc]";
pub const BUTTON_CONFIRM: &str = "[confirm enter]";
pub const BUTTON_OPEN: &str = "[open enter]";

/// 关闭确认问题文案。
pub fn confirm_close_question(name: &str) -> String {
    format!("close \"{name}\"?")
}

/// 切换工作区路径确认问题文案。
pub fn confirm_switch_cwd_question(path: &str) -> String {
    format!("switch cwd to \"{path}\"?")
}

/// 同步当前工作区路径到终端路径的二次确认问题；超宽时使用中间截断保留首尾目录信息。
pub fn confirm_sync_ws_cwd_question(path: &str, max_width: usize) -> String {
    let prefix = "switch workspace path to \"";
    let suffix = "\"?";
    let overhead = prefix.width() + suffix.width();
    if max_width <= overhead {
        return format!("{prefix}{path}{suffix}");
    }
    let path_width = max_width.saturating_sub(overhead);
    let ellipsized = ellipsize_middle(path, path_width);
    format!("{prefix}{ellipsized}{suffix}")
}

/// 标签栏右端退出按钮；贴屏幕右缘，右缘与右栏折叠态按钮对齐。
pub const EXIT_LABEL: &str = "[exit]";

/// 复制反馈 toast。
pub const TOAST_COPIED: &str = "Copied to clipboard";
pub const TOAST_COPY_FAILED: &str = "Copy failed";
/// 复制反馈 toast 的边框标题。
pub const TOAST_CLIPBOARD_TITLE: &str = "Clipboard";
/// 工作区反馈 toast 的边框标题。
pub const TOAST_WORKSPACE_TITLE: &str = "Workspace";
/// 工作区路径不存在。
pub const TOAST_DIRECTORY_NOT_FOUND: &str = "Directory does not exist";
/// 工作区路径非目录。
pub const TOAST_PATH_NOT_DIR: &str = "Path is not a directory";

/// 右键菜单边框标题：目标名缺失时按种类回退。
pub const MENU_TITLE_WORKSPACE: &str = "workspace";
pub const MENU_TITLE_TAB: &str = "tab";
pub const MENU_TITLE_PANE: &str = "pane";

/// 浮层命令面板顶边左侧标题：markdown 块命令 / 文件提及 / 斜杠模板命令。
pub const BLOCK_PANEL_TITLE: &str = "Commands";
pub const MENTION_PANEL_TITLE: &str = "Files";
pub const SLASH_PANEL_TITLE: &str = "Templates";

/// 斜杠命令面板条目：短名标签 + 长别名预览。
pub fn slash_command_text(id: SlashCommandId) -> (String, String) {
    (format!("/{}", id.short_name()), id.long_name().to_string())
}

/// 模板块操作按钮文案；状态按钮按当前状态取标签。
pub const TEMPLATE_BUTTON_TODO: &str = "[todo]";
pub const TEMPLATE_BUTTON_RUN: &str = "[run]";
pub const TEMPLATE_BUTTON_DONE: &str = "[done]";
pub const TEMPLATE_BUTTON_COPY: &str = "[copy]";
pub const TEMPLATE_BUTTON_CLEAN: &str = "[clean]";
pub const TEMPLATE_BUTTON_DEL: &str = "[del]";

/// 状态按钮标签：todo / run / done。
pub fn template_status_label(status: TemplateStatus) -> &'static str {
    match status {
        TemplateStatus::Todo => TEMPLATE_BUTTON_TODO,
        TemplateStatus::InProgress => TEMPLATE_BUTTON_RUN,
        TemplateStatus::Done => TEMPLATE_BUTTON_DONE,
    }
}

/// @ 面板底边快捷键提示：进入目录 `Shift+Enter` / 回退上一级 `Ctrl+Z`（`^z` 沿用 `[clear ^c]` 记法）。
pub const MENTION_PANEL_FOOTER: &str = "[open ⇧↵] [back ^z]";

/// markdown 块命令面板条目标签。
pub const BLOCK_HEADING_LABEL: &str = "Heading";
pub const BLOCK_UNORDERED_LABEL: &str = "Bullet List";
pub const BLOCK_TASK_LABEL: &str = "Task List";
pub const BLOCK_ORDERED_LABEL: &str = "Numbered List";
pub const BLOCK_QUOTE_LABEL: &str = "Quote";
pub const BLOCK_CODE_LABEL: &str = "Code Block";
pub const BLOCK_TABLE_LABEL: &str = "Table";

/// markdown 块命令面板条目预览（纯语法片段，不翻译）。
pub const BLOCK_UNORDERED_PREVIEW: &str = "-";
pub const BLOCK_TASK_PREVIEW: &str = "- [ ]";
pub const BLOCK_ORDERED_PREVIEW: &str = "1.";
pub const BLOCK_QUOTE_PREVIEW: &str = ">";
pub const BLOCK_CODE_PREVIEW: &str = "```";
pub const BLOCK_TABLE_PREVIEW: &str = "|  |  |";

/// 块命令面板条目（标签、预览）；标题带级别。
pub fn block_command_text(id: BlockCommandId) -> (String, String) {
    match id {
        BlockCommandId::Heading(level) => (
            format!("{BLOCK_HEADING_LABEL} {level}"),
            "#".repeat(usize::from(level)),
        ),
        BlockCommandId::UnorderedList => {
            (BLOCK_UNORDERED_LABEL.into(), BLOCK_UNORDERED_PREVIEW.into())
        }
        BlockCommandId::TaskList => (BLOCK_TASK_LABEL.into(), BLOCK_TASK_PREVIEW.into()),
        BlockCommandId::OrderedList => (BLOCK_ORDERED_LABEL.into(), BLOCK_ORDERED_PREVIEW.into()),
        BlockCommandId::Quote => (BLOCK_QUOTE_LABEL.into(), BLOCK_QUOTE_PREVIEW.into()),
        BlockCommandId::CodeBlock => (BLOCK_CODE_LABEL.into(), BLOCK_CODE_PREVIEW.into()),
        BlockCommandId::Table => (BLOCK_TABLE_LABEL.into(), BLOCK_TABLE_PREVIEW.into()),
    }
}

/// 单行截断：按字符数近似（现有文案为 ASCII），超出以省略号收尾。
pub fn ellipsize(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

/// 路径中间截断：保留首尾并在中间放省略号（对齐 opencode 的 `truncateMiddle`）。
///
/// 按显示列计算，宽字符不会越界；宽度足够时原样返回。
pub fn ellipsize_middle(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let keep = max_width - 1;
    let keep_start = keep.div_ceil(2);
    let keep_end = keep / 2;
    format!(
        "{}…{}",
        take_prefix_width(text, keep_start),
        take_suffix_width(text, keep_end)
    )
}

/// 取显示列不超过 `max_width` 的前缀；宽字符放不下时在字符边界处截断。
fn take_prefix_width(text: &str, max_width: usize) -> &str {
    let mut width = 0;
    for (offset, ch) in text.char_indices() {
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > max_width {
            return &text[..offset];
        }
        width += char_width;
    }
    text
}

/// 取显示列不超过 `max_width` 的后缀；宽字符放不下时在字符边界处截断。
fn take_suffix_width(text: &str, max_width: usize) -> &str {
    let mut width = 0;
    for (offset, ch) in text.char_indices().rev() {
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > max_width {
            return &text[offset + ch.len_utf8()..];
        }
        width += char_width;
    }
    text
}

/// 窗格标题：优先 OSC 标题，其次窗格 cwd 末段标签，最后按标识生成。
pub fn pane_title(id: PaneId, osc_title: Option<&str>, cwd_label: Option<&str>) -> String {
    match osc_title {
        Some(title) if !title.trim().is_empty() => title.to_string(),
        _ => match cwd_label {
            Some(label) if !label.trim().is_empty() => label.to_string(),
            _ => format!("pane {}", id.raw()),
        },
    }
}

#[cfg(test)]
mod tests;
