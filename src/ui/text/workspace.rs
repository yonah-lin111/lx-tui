//! 工作区、侧栏、右键菜单、模态弹窗、右栏与身份标签的用户可见文案。

use unicode_width::UnicodeWidthStr;

use crate::app::markdown::{TemplateCounts, TemplateStatus};
use crate::app::state::WorkspaceIdentity;
use crate::detect::{AgentKind, AgentState};
use crate::layout::PaneId;

use super::ellipsize_middle;

// ===== 侧栏：标题、Agents 分区与工作区列表 =====

pub const SIDEBAR_TITLE: &str = "Workspaces";
/// 侧栏下半分区标题。
pub const SIDEBAR_AGENTS_TITLE: &str = "Agents";
pub const MIN_SIZE_HINT: &str = "terminal too small";

/// Agents 分区空状态。
pub const AGENTS_EMPTY: &str = "no agents running";
/// 列表当前项标记：Agents 条目、工作区列表、提示词列表与页码菜单共用（不独立着色）。
pub const AGENT_SELECTED_MARKER: &str = "❯";
/// 浮层列表（`/model`、`/resume`）"当前已生效项"标记：与侧栏同一字形，
/// 使"我现在用的是哪个"在任何列表里都是一个符号。
pub const CURRENT_ITEM_MARKER: &str = AGENT_SELECTED_MARKER;

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

/// Agents 条目归属标签 `[tab N]`：N 为标签位置（1 基）。
pub fn agent_location(tab_index: usize) -> String {
    format!("[tab {}]", tab_index.saturating_add(1))
}

// ===== 右栏：提示词树、编辑器与引用文件夹 =====

/// 右侧容器顶边框标签页文案。
pub const RIGHT_TAB_PROMPT_LABEL: &str = "Prompt";
pub const RIGHT_TAB_TODO_LABEL: &str = "Todo";
/// prompt 底边框分支前缀。
pub const PROMPT_BRANCH_PREFIX: &str = "b:";
/// prompt 底边框 linked worktree 标记前缀。
pub const PROMPT_WORKTREE_PREFIX: &str = "wt:";
/// prompt 工具栏按钮与保存状态点。
pub const PROMPT_UNDO_LABEL: &str = "[←]";
pub const PROMPT_REDO_LABEL: &str = "[→]";
/// 条目保存状态点：已保存实心绿、未保存实心黄。
pub const PROMPT_SAVE_DOT: &str = "●";
/// 临时草稿保存状态点：空心（与空闲 agent 状态圆点一致），恒灰。
pub const PROMPT_DRAFT_DOT: &str = "○";
/// prompt 底边钉住按钮：未钉显示 `[pin]`、已钉显示 `[unpin]`。
pub const PROMPT_UNPINNED_LABEL: &str = "[pin]";
pub const PROMPT_PINNED_LABEL: &str = "[unpin]";
/// prompt 视图模式按钮：编辑视图显示 `[≡]`（切到列表）、树视图显示 `[/]`（切到编辑）。
pub const PROMPT_MODE_LIST_LABEL: &str = "[≡]";
pub const PROMPT_MODE_EDIT_LABEL: &str = "[/]";
/// 单行输入框左侧统一前缀图标：模态输入框与 prompt 列表搜索框共用（置于输入区左侧、不占输入内容）。
pub const INPUT_PREFIX_ICON: &str = ">";
/// 提示词树：搜索框、临时提示词按钮与连接符（子层末项 `└─`、其余 `├─`，与侧栏工作区树一致）。
pub const PROMPT_TREE_SEARCH_PLACEHOLDER: &str = "search prompts…";
/// 树视图 `[tmp]`：新建临时提示词编辑视图（仅内存、不入库）。
pub const PROMPT_TEMP_PROMPT_LABEL: &str = "[tmp]";
/// 编辑视图引用文件夹按钮：打开引用文件夹菜单。
pub const PROMPT_FOLDERS_LABEL: &str = "[refDir]";
/// `[rec]`：开关底部最近修改页面抽屉（三种视图常显）。
pub const PROMPT_RECENT_LABEL: &str = "[rec]";
/// 抽屉顶部分割线标签。
pub const PROMPT_RECENT_SECTION_LABEL: &str = " recent ";
/// 抽屉右上角关闭按钮。
pub const PROMPT_RECENT_CLOSE: &str = "[x]";
/// 抽屉空态。
pub const PROMPT_RECENT_EMPTY: &str = "no recent pages";

/// 引用文件夹弹窗文案。
pub const MENU_TITLE_REFERENCED_FOLDERS: &str = "Folders";
/// 引用文件夹弹窗空状态提示（不可选）。
pub const REFERENCED_MENU_EMPTY: &str = "no folders referenced";
/// 引用文件夹弹窗末行的新建动作项。
pub const REFERENCED_MENU_ADD_ROW: &str = "+ add folder…";
/// 引用文件夹弹窗底边快捷键提示。
pub const REFERENCED_MENU_FOOTER: &str = "↵ open · ^c copy · ^d del · esc close";
/// 引用文件夹路径输入弹窗标题与占位。
pub const REFERENCED_INPUT_TITLE: &str = "Add folder";
pub const REFERENCED_INPUT_PLACEHOLDER: &str = "path to directory";
/// 引用文件夹删除确认浮层标题。
pub const REFERENCED_DELETE_TITLE: &str = "Remove folder";
pub fn referenced_delete_question(name: &str) -> String {
    format!("remove \"{name}\" from referenced folders?")
}
/// 引用文件夹内容搜索弹窗标题与占位、状态与底边快捷键提示。
pub const REFERENCED_SEARCH_TITLE: &str = "Folder contents";
pub const REFERENCED_SEARCH_PLACEHOLDER: &str = "search files…";
pub const REFERENCED_SEARCH_LOADING: &str = "scanning…";
pub const REFERENCED_SEARCH_FAILED: &str = "folder is not accessible";
pub const REFERENCED_SEARCH_EMPTY: &str = "no matching files";
pub const REFERENCED_SEARCH_TRUNCATED: &str = "results truncated";
pub const REFERENCED_SEARCH_FOOTER: &str = "↵/^c copy · ⇧↵ open · ^z back · esc close";
/// 引用文件夹搜索结果行图标：目录 / 文件。
pub const REFERENCED_DIR_ICON: &str = "▸";
pub const REFERENCED_FILE_ICON: &str = "•";

pub const REFERENCED_ERROR_EMPTY: &str = "Path is empty";
pub const REFERENCED_ERROR_NOT_ABSOLUTE: &str = "Path must be absolute";
pub const REFERENCED_ERROR_NOT_FOUND: &str = "Directory does not exist";
pub const REFERENCED_ERROR_NOT_DIR: &str = "Path is not a directory";
pub const REFERENCED_ERROR_NOT_REPRESENTABLE: &str = "Path cannot be referenced";
pub const REFERENCED_ERROR_DUPLICATE: &str = "Folder already referenced";
/// 引用文件夹保存/加载失败提示（数据库不可用）。
pub const REFERENCED_DB_UNAVAILABLE: &str = "cannot load or save referenced folders";
/// 引用文件夹写库/删除失败 toast。
pub const TOAST_REFERENCED_SAVE_FAILED: &str = "Failed to save referenced folder";
pub const TOAST_REFERENCED_DELETE_FAILED: &str = "Failed to remove referenced folder";

/// 引用文件夹路径校验错误文案；`Ok`（无错误）返回 None，其余返回对应原因。
pub fn referenced_path_error(
    error: &crate::app::referenced::PathValidation,
) -> Option<&'static str> {
    use crate::app::referenced::PathValidation;
    match error {
        PathValidation::Ok { .. } => None,
        PathValidation::Empty => Some(REFERENCED_ERROR_EMPTY),
        PathValidation::NotAbsolute => Some(REFERENCED_ERROR_NOT_ABSOLUTE),
        PathValidation::NotFound => Some(REFERENCED_ERROR_NOT_FOUND),
        PathValidation::NotDirectory => Some(REFERENCED_ERROR_NOT_DIR),
        PathValidation::NotRepresentable => Some(REFERENCED_ERROR_NOT_REPRESENTABLE),
        PathValidation::Duplicate => Some(REFERENCED_ERROR_DUPLICATE),
    }
}
/// 提示词树分区标签：临时区与已入库区（仅两侧都有内容时成对出现）。
pub const PROMPT_SECTION_TMP: &str = "tmp:";
pub const PROMPT_SECTION_SAVED: &str = "saved:";
/// 空白 default temp 在列表中的占位展示名（每个工作区恒保留一条）。
pub const PROMPT_TEMP_DEFAULT_NAME: &str = "default";
/// 临时提示词在编辑视图分割线行的标题。
pub const PROMPT_DRAFT_TITLE: &str = "Temp prompt";
pub const PROMPT_TREE_EXPANDED: &str = "▾";
pub const PROMPT_TREE_COLLAPSED: &str = "▸";
pub const PROMPT_TREE_ITEM_ICON: &str = "•";
pub const PROMPT_TREE_LAST: &str = "└─";
pub const PROMPT_TREE_MIDDLE: &str = "├─";
/// 提示词树空状态与搜索无命中占位。
pub const PROMPT_TREE_EMPTY: &str = "no prompts yet";
pub const PROMPT_TREE_NO_MATCHES: &str = "no matching prompts";
/// 拖拽落点为根层（末行以下空白）时的落点提示。
pub const PROMPT_TREE_DROP_ROOT: &str = "→ move to root";
/// 临时草稿保存选择器：标题、根层项与新建文件夹动作项。
pub const SAVE_DRAFT_TITLE: &str = "Save prompt to folder";
pub const SAVE_DRAFT_ROOT: &str = "/";
pub const SAVE_DRAFT_NEW_FOLDER: &str = "+ New folder";
/// 新建提示词与文件夹的默认名（可随后重命名）。
pub const PROMPT_DEFAULT_NAME: &str = "New prompt";
pub const PROMPT_FOLDER_DEFAULT_NAME: &str = "New folder";
/// prompt 编辑器空内容时的输入说明占位。
pub const PROMPT_PLACEHOLDER: &str = "Write your prompt…";

// ===== 布局标签、右键菜单与模态弹窗 =====

/// 折叠按钮：按面板所在侧镜像，箭头指向点击后移动的方向。
pub const SIDEBAR_COLLAPSE_LABEL: &str = "[◀]";
pub const SIDEBAR_EXPAND_LABEL: &str = "[▶]";
/// 右列面板（prompt 或主区）的收起/展开按钮。
pub const RIGHT_COLLAPSE_LABEL: &str = "[▶]";
pub const RIGHT_EXPAND_LABEL: &str = "[◀]";
/// 主区与 prompt 分隔线中点的交换按钮：左格 `<`、右格 `>`。
pub const SWAP_PROMPT_LABEL: &str = "<>";
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
/// 侧栏工作区历史列表：搜索占位与空态文案。
pub const WORKSPACE_HISTORY_SEARCH_PLACEHOLDER: &str = "search workspaces…";
pub const WORKSPACE_HISTORY_EMPTY: &str = "no workspaces yet";
pub const WORKSPACE_HISTORY_NO_MATCHES: &str = "no matching workspaces";
pub const MENU_OPEN_WORKSPACE_HISTORY: &str = "Open";
pub const MENU_DELETE_WORKSPACE_HISTORY: &str = "Delete";
pub const MENU_DELETE_MISSING_WORKSPACES: &str = "Delete missing";
pub const MENU_CONFIRM_DELETE_MISSING_WORKSPACES: &str = "Confirm delete missing";
pub const CONFIRM_DELETE_WORKSPACE_TITLE: &str = "Delete Workspace";
/// 一键清理失效工作区确认弹窗标题。
pub const CONFIRM_DELETE_MISSING_WORKSPACES_TITLE: &str = "Delete Missing Workspaces";
/// 删除确认弹窗的单行口令输入占位提示。
pub const CONFIRM_DELETE_INPUT_PLACEHOLDER: &str = "type \"confirm delete\" to confirm";

/// 标签栏：新建标签按钮、标签分隔线、溢出滚动按钮与条目左侧图标。
pub const ADD_TAB_LABEL: &str = "[+]";
pub const TAB_SEPARATOR: &str = "│";
pub const TAB_SCROLL_LEFT_LABEL: &str = "[<]";
pub const TAB_SCROLL_RIGHT_LABEL: &str = "[>]";
pub const TAB_ITEM_ICON: &str = "▪";
/// 该标签有窗格挂着 `ask` 面板时的标记（标签标题后追加，位于标签背景内部）。
pub const TAB_ASK_MARKER: &str = "?";

/// 初始工作区标记：不可移除，跟随启动工作区。
pub const INITIAL_WORKSPACE_MARKER: &str = " *";

/// 工作区分组折叠箭头：展开 / 折叠。
pub const WORKSPACE_GROUP_EXPANDED: &str = "▾";
pub const WORKSPACE_GROUP_COLLAPSED: &str = "▸";
/// 分组子项树形连接符：非末位 / 末位（按可见子项判定）。
pub const WORKSPACE_TREE_MIDDLE: &str = "├─";
pub const WORKSPACE_TREE_LAST: &str = "└─";

/// 右键菜单项；按命令映射，禁止在逻辑层硬编码。
pub const MENU_NEW_TAB: &str = "New tab";
pub const MENU_NEW_TERMINAL: &str = "New terminal";
pub const MENU_NEW_WORKSPACE: &str = "New workspace";
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
pub const MENU_COPY_PATH: &str = "Copy path";
pub const MENU_CLOSE_PANE: &str = "Close";

/// 提示词树右键菜单项。
pub const MENU_PROMPT_NEW_PROMPT: &str = "New prompt";
pub const MENU_PROMPT_NEW_FOLDER: &str = "New folder";
pub const MENU_PROMPT_NEW_SUBFOLDER: &str = "New subfolder";
pub const MENU_PROMPT_EDIT: &str = "Edit";
pub const MENU_PROMPT_RENAME: &str = "Rename";
pub const MENU_PROMPT_COPY: &str = "Copy content";
pub const MENU_PROMPT_DELETE: &str = "Delete";

/// 右键菜单项行内二次确认文案。
pub const MENU_CONFIRM_CLOSE: &str = "Confirm close";
pub const MENU_CONFIRM_DELETE: &str = "Confirm delete";
pub const MENU_CONFIRM_SWITCH_TO_WORKSPACE_CWD: &str = "Confirm switch to ws path";
pub const MENU_CONFIRM_SYNC_WS_TO_TERMINAL_CWD: &str = "Confirm sync ws to terminal path";

/// 标题行的页切换箭头（区别于折叠 `◀`/`▶` 与撤销/重做 `←`/`→`）。
pub const PROMPT_PAGE_PREV: &str = "‹";
pub const PROMPT_PAGE_NEXT: &str = "›";
/// 页码文本 `2/4`（当前页 / 总页数）。
pub fn page_counter(current: usize, total: usize) -> String {
    format!("{current}/{total}")
}
/// 顶边框标题里页名段的前缀（`条目名 · 页名`）。
pub const PROMPT_PAGE_NAME_PREFIX: &str = "·";
/// 默认页名 `Page N`；`index` 从 1 起。
pub fn default_page_name(index: usize) -> String {
    format!("Page {index}")
}

/// 页列表与页右键菜单文案。
pub const MENU_PAGE_OPEN: &str = "Open";
pub const MENU_PAGE_SELECT_ALL: &str = "Select All";
pub const MENU_PAGE_RENAME: &str = "Rename Page";
pub const MENU_PAGE_NEW: &str = "New Page";
pub const MENU_PAGE_FORK: &str = "Fork Page";
pub const MENU_TITLE_PAGES: &str = "Pages";
pub const PROMPT_PAGES_LABEL: &str = "pages";
pub const PROMPT_PAGES_SEARCH_PLACEHOLDER: &str = "search pages";
pub const PROMPT_PAGES_NO_MATCHES: &str = "no matching pages";
pub const PAGE_STATUS_OPEN: &str = "○";
pub const PAGE_STATUS_DOT: &str = "●";

/// 模板块状态计数分隔符：`|`。
const TEMPLATE_COUNT_SEPARATOR: &str = "|";

/// 模板块状态计数段：`  n|  m|  x`（todo / run / done 顺序，仅统计闭合顶层 `&&&`）。
///
/// 各数字固定 3 列右对齐（不足 3 位左侧补空格，超过 999 封顶为 999）；
/// 数字段带对应状态（用于分色），分隔符段为 None（用于弱化）；
/// 三态合计为 0 时返回空，调用方据此整组省略。页码菜单与提示词列表共用。
pub fn template_count_segments(counts: &TemplateCounts) -> Vec<(String, Option<TemplateStatus>)> {
    if counts.todo + counts.in_progress + counts.done == 0 {
        return Vec::new();
    }
    let mut segments = Vec::with_capacity(5);
    for (index, (count, status)) in [
        (counts.todo, TemplateStatus::Todo),
        (counts.in_progress, TemplateStatus::InProgress),
        (counts.done, TemplateStatus::Done),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            segments.push((TEMPLATE_COUNT_SEPARATOR.to_string(), None));
        }
        let clamped = count.min(999);
        segments.push((format!("{clamped:>3}"), Some(status)));
    }
    segments
}

/// 删除非空页的二次确认标题。
pub const CONFIRM_DELETE_PAGE_TITLE: &str = "Delete page";

/// 重命名与关闭确认浮层标题。
pub const RENAME_WORKSPACE_TITLE: &str = "Rename workspace";
pub const NEW_WORKSPACE_TITLE: &str = "New workspace";
pub const RENAME_PROMPT_TITLE: &str = "Rename prompt";
pub const RENAME_FOLDER_TITLE: &str = "Rename folder";
pub const NEW_FOLDER_TITLE: &str = "New folder";
pub const NEW_PROMPT_TITLE: &str = "New prompt";
/// rename 浮层输入为空时的输入说明占位（工作区与标签共用）。
pub const RENAME_PLACEHOLDER: &str = "new name";
/// new workspace 路径输入为空时的输入说明占位。
pub const NEW_WORKSPACE_PLACEHOLDER: &str = "path to directory";
pub const RENAME_TAB_TITLE: &str = "Rename tab";

/// 信任工作区模态：标题、问题与按钮。
pub const TRUST_WORKSPACE_TITLE: &str = "Trust workspace";
pub const TRUST_WORKSPACE_QUESTION: &str = "Do you trust the files in this folder?";
pub const BUTTON_TRUST: &str = " ↵ trust & open ";
pub const BUTTON_DONT_TRUST: &str = " esc don't trust ";
/// 启动信任页（行内，进入应用前）：标题行、安全提示、说明、可选项与页脚。
pub const TRUST_CONSOLE_HEADING: &str = "Accessing workspace:";
/// 安全提示段落；`:` 前的前缀在渲染时加粗强调。
pub const TRUST_CONSOLE_SAFETY: &str = "Quick safety check: Is this a project you created or one you trust? (Like your own code, a well-known open source project, or work from your team). If not, take a moment to review what is in this folder first.";
/// 能力说明：加入工作区 + 文件读写执行。
pub const TRUST_CONSOLE_HINT: &str =
    "lx-tui will add this folder as a workspace and be able to read, edit and execute files here.";
/// 能力说明：提示词管理（本地持久化、文件夹组织与编辑）。
pub const TRUST_CONSOLE_PROMPT_HINT: &str = "Prompt management is included: create, organize into folders, edit and delete workspace prompts here.";
/// 可选项：`No, exit` 在前为默认选中；`Yes, I trust this folder` 在后。
pub const TRUST_OPTION_NO: &str = "No, exit";
pub const TRUST_OPTION_YES: &str = "Yes, I trust this folder";
/// 信任页选项选中标记（与侧栏 Agents 选中行同符号）。
pub const TRUST_SELECTION_MARKER: &str = "▸";
pub const TRUST_CONSOLE_FOOTER: &str = "↑↓ select · ↵ confirm · esc cancel";
/// 启动拒绝信任后的终端提示（不进入应用，直接退出）。
pub const TRUST_REFUSED_MESSAGE: &str = "workspace not trusted; exiting";

/// worktree 对话框：标题、搜索占位、状态行与条目标记。
pub const WORKTREE_OPEN_TITLE: &str = "Open worktree";
pub const WORKTREE_OPEN_FILTER: &str = "filter worktrees";
pub const WORKTREE_OPEN_LOADING: &str = "loading…";
pub const WORKTREE_OPEN_FAILED: &str = "failed to list worktrees";
pub const WORKTREE_OPEN_EMPTY: &str = "no worktrees found";
pub const WORKTREE_OPEN_MARKER: &str = "›";
pub const WORKTREE_STATUS_OPEN: &str = "open";
pub const WORKTREE_STATUS_DETACHED: &str = "detached";
pub const WORKTREE_STATUS_ROOT: &str = "root";

/// 模态底部胶囊按钮：快捷键符号在前、功能名在后，左右各留 1 列内边距（对齐 herdr 实心胶囊按钮）。
pub const BUTTON_SAVE: &str = " ↵ save ";
pub const BUTTON_CREATE: &str = " ↵ create ";
pub const BUTTON_CLEAR: &str = " ^c clear ";
pub const BUTTON_CANCEL: &str = " esc cancel ";
pub const BUTTON_CONFIRM: &str = " ↵ confirm ";
/// `/resume` 会话编辑模态的删除与二次确认按钮。
pub const BUTTON_DELETE: &str = " ^d delete ";
pub const BUTTON_CONFIRM_DELETE: &str = " ↵ confirm del ";
/// 模态关闭图标常量（与模板块删除按钮 `TEMPLATE_BUTTON_DEL` 保持同字符）。
pub const MODAL_CLOSE: &str = "[✕]";
pub const BUTTON_OPEN: &str = " ↵ open ";

/// 删除提示词页确认问题文案。
pub fn confirm_delete_page_question(name: &str) -> String {
    format!("delete page \"{name}\"?")
}

/// 删除工作区最终确认弹窗问题文案。
pub fn confirm_delete_workspace_question(name: &str) -> String {
    format!("delete workspace \"{name}\" and all its data?")
}

/// 删除 Git 仓库父项时，明确提示会一并删除的同仓库 linked worktree 数量。
pub fn confirm_delete_workspace_group_question(name: &str, worktrees: usize) -> String {
    format!("delete workspace \"{name}\", {worktrees} linked worktree(s), and all their data?")
}

/// 一键清理失效工作区确认弹窗问题文案。
pub fn confirm_delete_missing_workspaces_question(count: usize) -> String {
    format!("delete {count} missing workspace(s) and all their data?")
}

// ===== 工作区反馈 toast =====

/// 工作区反馈 toast 的边框标题。
pub const TOAST_WORKSPACE_TITLE: &str = "Workspace";
pub const TOAST_WORKSPACE_WRITE_FAILED: &str = "Workspace update failed";
/// 工作区路径不存在。
pub const TOAST_DIRECTORY_NOT_FOUND: &str = "Directory does not exist";
/// 工作区路径非目录。
pub const TOAST_PATH_NOT_DIR: &str = "Path is not a directory";

// ===== 粘贴与快捷引用面板 =====

/// 粘贴二级面板与附件条文案。
pub const PASTE_PANEL_TITLE: &str = "Paste";
pub const PASTE_ACTION_REFERENCE: &str = "Insert reference";
pub const PASTE_ACTION_PATH: &str = "Insert path";
pub const PASTE_REFERENCE_UNSUPPORTED: &str = "path cannot be referenced";
/// 文本粘贴二级面板文案。
pub const TEXT_PASTE_PANEL_TITLE: &str = "Paste";
pub const TEXT_PASTE_ACTION_RAW: &str = "Paste raw";
pub const TEXT_PASTE_ACTION_TEXT: &str = "Paste text";
pub const TEXT_PASTE_ACTION_CODE: &str = "Paste code";
pub const TEXT_PASTE_ACTION_LOG: &str = "Paste log";
pub const TEXT_PASTE_TOO_LARGE: &str = "too large to collapse";

/// 字母快捷候选面板标题。
pub const REFER_PANEL_TITLE: &str = "References";
/// 附件条：类型标签、缺失状态与移除按钮。
pub const PROMPT_ATTACH_FILE_TAG: &str = "[file]";
pub const PROMPT_ATTACH_IMAGE_TAG: &str = "[img]";
pub const PROMPT_ATTACH_MISSING: &str = "(missing)";
pub const PROMPT_ATTACH_REMOVE: &str = "[x]";

/// 粘贴候选数量说明。
pub fn paste_item_count(count: usize) -> String {
    if count == 1 {
        "1 item".to_string()
    } else {
        format!("{count} items")
    }
}

// ===== 菜单/面板标题、模板块与提及面板 =====

/// 右键菜单边框标题：目标名缺失时按种类回退。
pub const MENU_TITLE_WORKSPACE: &str = "Workspace";
pub const MENU_TITLE_WORKSPACES: &str = "Workspaces";
pub const MENU_TITLE_TAB: &str = "Tab";
pub const MENU_TITLE_PANE: &str = "Pane";
pub const MENU_TITLE_PROMPT: &str = "Prompt";
pub const MENU_TITLE_FOLDER: &str = "Folder";
pub const MENU_TITLE_PROMPT_TREE: &str = "Prompts";

/// 浮层命令面板顶边左侧标题：markdown 块命令 / 文件提及 / 斜杠命令 / worktree 选择器。
pub const BLOCK_PANEL_TITLE: &str = "Commands";
pub const MENTION_PANEL_TITLE: &str = "@Command";
pub const MENTION_TAB_FILES: &str = "Files";
pub const MENTION_TAB_REF_DIR: &str = "refDir";
pub const MENTION_TAB_SKILLS: &str = "Skills";
pub const MENTION_TAB_MCP: &str = "MCP";
pub const MENTION_REF_DIR_EMPTY: &str = "no matching folders";
pub const MENTION_SKILLS_EMPTY: &str = "no matching skills";
pub const MENTION_MCP_EMPTY: &str = "no matching MCP servers";
pub const MENTION_MCP_LABEL: &str = "MCP";
pub const SLASH_PANEL_TITLE: &str = "/Command";
pub const WORKTREE_PANEL_TITLE: &str = "Worktrees";

/// 斜杠命令面板右侧来源标签（不含方括号）。
pub const SLASH_TAG_BUILTIN: &str = "Built-in";

pub const SLASH_TAG_USER: &str = "User";
pub const SLASH_TAG_PROJECT: &str = "Project";
pub const SLASH_TAG_GIT: &str = "Git";

/// `/gitWorkTree` 候选标签与预览。
pub const SLASH_WORKTREE_LABEL: &str = "/gitWorkTree";
pub const SLASH_WORKTREE_PREVIEW: &str = "bind a worktree to this block";

/// prompt 底边框左侧设置按钮与 tooltip。
/// 设置按钮文案（纯 ASCII，宽度稳定、无 emoji 呈现）。
pub const SETTINGS_LABEL: &str = "[SET]";

/// 模板块操作按钮文案（复制/清理/删除图标）。
pub const TEMPLATE_BUTTON_COPY: &str = "[❐]";
pub const TEMPLATE_BUTTON_CLEAN: &str = "[⌫]";
pub const TEMPLATE_BUTTON_DEL: &str = "[✕]";

/// 补充块操作按钮：清理与删除复用模板块图标。
pub const SUPPLE_BUTTON_CLEAN: &str = TEMPLATE_BUTTON_CLEAN;
pub const SUPPLE_BUTTON_DEL: &str = TEMPLATE_BUTTON_DEL;
/// prompt 块左侧色条字形（`&&&` 状态色 / `+++` 品红）。
pub const PROMPT_BLOCK_BAR: &str = "▎";
/// prompt 块顶部边框字形：下 1/4 块（与左侧 `▎` 等厚，紧贴起始行顶部）。
pub const PROMPT_BLOCK_TOP_BORDER: &str = "▂";
/// 块结构分割线（标题下 / 绑定行上的空行）。
pub const PROMPT_BLOCK_RULE: char = '─';

/// @ 面板与命令面板底边快捷键提示（Prompt 右栏带边框面板使用）。
pub const MENTION_FOOTER_FILES: &str = "←→ tab · ↑↓ select · ↵ confirm · ⇧↵ enter · esc cancel";
pub const MENTION_FOOTER_REF_DIR: &str = "←→ tab · ↑↓ select · ↵ insert · esc cancel";
pub const COMMAND_PANEL_FOOTER: &str = "↑↓ select · ↵ confirm · esc cancel";

/// `@` 提及面板状态行文案（Prompt 与 lx 面板共用）。
pub const LX_MENTION_LOADING: &str = "scanning…";
pub const LX_MENTION_EMPTY: &str = "no matches";

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

// ===== 身份标签、窗格标题与工作区路径 =====

/// 身份标签段间分隔符。
pub const IDENTITY_SEPARATOR: &str = " · ";
/// 边框身份标签片段：`emphasis` 为真取强调色（项目名与 `b:`/`wt:` 前缀），否则取默认灰。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentitySpan {
    pub text: String,
    pub emphasis: bool,
}

/// 边框身份标签：`项目 · b:分支 · wt:工作区名`，缺段自动省略，` · ` 连接。
///
/// 降级阶梯：整段 → 砍 `wt:` → 项目名中间省略（保留 `b:`） → 砍 `b:` → 不绘制。
/// 返回 None 表示可用宽度连项目名开头的省略形式都放不下。
pub fn workspace_identity_spans(
    identity: &WorkspaceIdentity,
    available: usize,
) -> Option<Vec<IdentitySpan>> {
    if available < 2 {
        return None;
    }
    let project = identity.project.as_str();
    let branch = identity.branch.as_deref();
    let worktree = identity.worktree.as_deref();
    let full = identity_spans(project, branch, worktree);
    if spans_display_width(&full) <= available {
        return Some(full);
    }
    if worktree.is_some() {
        let trimmed = identity_spans(project, branch, None);
        if spans_display_width(&trimmed) <= available {
            return Some(trimmed);
        }
    }
    if let Some(branch) = branch {
        // 保留 ` · b:分支`，项目名按剩余预算中间省略。
        let overhead = IDENTITY_SEPARATOR.width() + PROMPT_BRANCH_PREFIX.width() + branch.width();
        let budget = available.saturating_sub(overhead);
        if budget >= 2 {
            let clipped = ellipsize_middle(project, budget);
            return Some(identity_spans(&clipped, Some(branch), None));
        }
    }
    Some(vec![IdentitySpan {
        text: ellipsize_middle(project, available),
        emphasis: true,
    }])
}

/// 按给定段组装身份标签片段。
fn identity_spans(
    project: &str,
    branch: Option<&str>,
    worktree: Option<&str>,
) -> Vec<IdentitySpan> {
    let mut spans = vec![IdentitySpan {
        text: project.to_string(),
        emphasis: true,
    }];
    if let Some(branch) = branch {
        spans.push(IdentitySpan {
            text: IDENTITY_SEPARATOR.to_string(),
            emphasis: false,
        });
        spans.push(IdentitySpan {
            text: PROMPT_BRANCH_PREFIX.to_string(),
            emphasis: true,
        });
        spans.push(IdentitySpan {
            text: branch.to_string(),
            emphasis: false,
        });
    }
    if let Some(worktree) = worktree {
        spans.push(IdentitySpan {
            text: IDENTITY_SEPARATOR.to_string(),
            emphasis: false,
        });
        spans.push(IdentitySpan {
            text: PROMPT_WORKTREE_PREFIX.to_string(),
            emphasis: true,
        });
        spans.push(IdentitySpan {
            text: worktree.to_string(),
            emphasis: false,
        });
    }
    spans
}

/// 片段显示列宽合计。
fn spans_display_width(spans: &[IdentitySpan]) -> usize {
    spans.iter().map(|span| span.text.width()).sum()
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

/// lx 窗格顶边框右侧 `ws:` 值：路径末两段（`a/b`）；不足两段用现有全部段，
/// 无可用段回退完整路径；超出可用宽度按列宽中间省略。可用宽度连省略号也放不下时返回 None。
pub fn workspace_path_label(path: &std::path::Path, available: usize) -> Option<String> {
    if available == 0 {
        return None;
    }
    let label = crate::app::state::path_tail_two(path);
    if label.is_empty() {
        return None;
    }
    if label.width() <= available {
        return Some(label);
    }
    (available >= 2).then(|| ellipsize_middle(&label, available))
}
