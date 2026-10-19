//! 全部用户可见文案；组件与逻辑禁止散落硬编码字符串。

pub mod agent;
pub mod chat;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::{BlockCommandId, TemplateCounts, TemplateStatus};
use crate::app::state::{AppState, LxEntry, PaneView};
use crate::app::tooltip::{BlockButtonKind, TemplateButtonKind, TooltipTarget};
use crate::detect::{AgentKind, AgentState};
use crate::layout::PaneId;

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

/// 快捷键符号（全项目唯一来源，禁止在组件里另写字形）。
///
/// 统一规则：**修饰键与"需要按下的键"用符号，动作词保留全拼**——
/// 修饰键（Shift `⇧`、Ctrl `^`、Cmd `⌘`）用符号省地方、窄面板里不易被截断；
/// 动作词（`esc`/`cancel`/`close`）用文字一眼懂，全符号化（`⎋`/`⇥`）在部分
/// 终端字体缺字形会显示成豆腐块。
pub mod key {
    /// Shift 修饰键。
    pub const SHIFT: &str = "⇧";
    /// Enter / Return 作为"需要按下的键"出现时的符号。
    pub const ENTER: &str = "↵";
    /// Cmd / Super 修饰键（不引入第二个字形，避免与 `ICON_WEB` 混淆）。
    pub const CMD: &str = "⌘";
    /// Backspace：与模板块清理按钮共用同一字形（`[⌫]`）。
    pub const BACKSPACE: &str = "⌫";
    /// Esc 保留全拼：`⎋` 在部分终端字体缺字形，且 `esc` 已是项目既有短记法。
    pub const ESC: &str = "esc";
}
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

/// prompt 面板**树视图**顶边框标题（编辑视图改用绑定目标名，见 `ui::prompt::header`）。
pub const PROMPT_TREE_BORDER_TITLE: &str = "Prompts";
/// prompt 底边框分支前缀。
pub const PROMPT_BRANCH_PREFIX: &str = "b:";
/// 工作区路径标记前缀；prompt 底边框与 lx 窗格顶边框共用（前缀强调色、值 muted）。
pub const WORKSPACE_PREFIX: &str = "ws:";
/// prompt 底边框右侧的工作区路径名前缀。
pub const PROMPT_WORKSPACE_PREFIX: &str = WORKSPACE_PREFIX;
/// prompt 底边框 linked worktree 标记前缀。
pub const PROMPT_WORKTREE_PREFIX: &str = "wt:";
/// prompt 工具栏按钮与保存状态点。
pub const PROMPT_UNDO_LABEL: &str = "[←]";
pub const PROMPT_REDO_LABEL: &str = "[→]";
pub const PROMPT_SELECT_ALL_LABEL: &str = "[*]";
/// 条目保存状态点：已保存实心绿、未保存实心黄。
pub const PROMPT_SAVE_DOT: &str = "●";
/// 临时草稿保存状态点：空心（与空闲 agent 状态圆点一致），恒灰。
pub const PROMPT_DRAFT_DOT: &str = "○";
/// prompt 钉住按钮：未钉空心 `[○]`、已钉实心 `[◉]`（状态显示，动作见 tooltip；
/// 与保存点/agent 状态圆点同符号系）。
pub const PROMPT_UNPINNED_LABEL: &str = "[○]";
pub const PROMPT_PINNED_LABEL: &str = "[◉]";
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
pub const PROMPT_FOLDERS_LABEL: &str = "[dir]";

/// 引用文件夹弹窗文案。
pub const MENU_TITLE_REFERENCED_FOLDERS: &str = "Folders";
/// 引用文件夹弹窗空状态提示（不可选）。
pub const REFERENCED_MENU_EMPTY: &str = "no folders referenced";
/// 引用文件夹弹窗末行的新建动作项。
pub const REFERENCED_MENU_ADD_ROW: &str = "+ add folder…";
/// 引用文件夹弹窗底边快捷键提示。
pub const REFERENCED_MENU_FOOTER: &str = "[open ↵] [copy ^c] [del ^d] [close esc]";
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
pub const REFERENCED_SEARCH_FOOTER: &str = "[copy ↵/^c] [open ⇧↵] [back ^z] [close esc]";
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
/// 临时草稿保存选择器：标题、根层项与新建文件夹动作项。
pub const SAVE_DRAFT_TITLE: &str = "Save prompt to folder";
pub const SAVE_DRAFT_ROOT: &str = "/";
pub const SAVE_DRAFT_NEW_FOLDER: &str = "+ New folder";
/// 新建提示词与文件夹的默认名（可随后重命名）。
pub const PROMPT_DEFAULT_NAME: &str = "New prompt";
pub const PROMPT_FOLDER_DEFAULT_NAME: &str = "New folder";
/// prompt 编辑器空内容时的输入说明占位。
pub const PROMPT_PLACEHOLDER: &str = "Write your prompt…";
/// 主内容 lx 页标题（窗格顶边框）。
pub const LX_TITLE: &str = "lx";
/// 视图切换按钮：显示点击后的目的地视图。
pub const LX_TOGGLE_TERMINAL: &str = "[>_]";
pub const LX_TOGGLE_LX: &str = "[lx]";
/// lx 页底部输入框：`>` 前缀与占位。
pub const LX_INPUT_PROMPT: &str = ">";
pub const LX_INPUT_PLACEHOLDER: &str = "Ask anything…";
/// loading 行右端的回到底部控件：箭头 + 右侧说明文字。
pub const LX_JUMP_BOTTOM: &str = "↓";
pub const LX_JUMP_BOTTOM_HINT: &str = "jump to bottom";
/// lx 页三个入口标签。
pub const LX_ENTRY_NATIVE: &str = "Native Agent";
pub const LX_ENTRY_HYBRID: &str = "Hybrid Agent";
pub const LX_ENTRY_AUTOMATION: &str = "Automation Agent";
/// lx 页横排入口标签之间的分隔符。
pub const LX_ENTRY_SEPARATOR: &str = "·";
/// lx 页入口 Tip 提示前缀；前缀强调色、句子 muted。
pub const LX_TIP_PREFIX: &str = "Tip:";
/// lx 页入口 Tip 提示行：选中入口一句话描述。
pub const LX_ENTRY_NATIVE_DESC: &str = "talks to the model directly";
pub const LX_ENTRY_HYBRID_DESC: &str = "mixes native and external agents";
pub const LX_ENTRY_AUTOMATION_DESC: &str = "runs your tasks automatically";
/// lx 页 md 格式面板边框标题。
pub const LX_BLOCK_PANEL_TITLE: &str = "Markdown";

/// lx 页入口标签：按选中入口取。
pub fn lx_entry_label(entry: LxEntry) -> &'static str {
    match entry {
        LxEntry::Native => LX_ENTRY_NATIVE,
        LxEntry::Hybrid => LX_ENTRY_HYBRID,
        LxEntry::Automation => LX_ENTRY_AUTOMATION,
    }
}

/// lx 页入口 Tip 提示行（一句话描述）。
pub fn lx_entry_description(entry: LxEntry) -> &'static str {
    match entry {
        LxEntry::Native => LX_ENTRY_NATIVE_DESC,
        LxEntry::Hybrid => LX_ENTRY_HYBRID_DESC,
        LxEntry::Automation => LX_ENTRY_AUTOMATION_DESC,
    }
}

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

/// 页菜单动作项与标题。
pub const MENU_PAGE_NEW: &str = "[add]";
pub const MENU_PAGE_DELETE: &str = "[del]";
pub const MENU_TITLE_PAGES: &str = "Pages";

/// 模板块状态计数分隔符：`n|m|x`。
const TEMPLATE_COUNT_SEPARATOR: &str = "|";

/// 模板块状态计数段：`n|m|x`（todo / run / done 顺序，仅统计闭合顶层 `&&&`）。
///
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
        segments.push((count.to_string(), Some(status)));
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
pub const CONFIRM_CLOSE_TITLE: &str = "Close workspace";
pub const CONFIRM_CLOSE_TAB_TITLE: &str = "Close tab";
pub const CONFIRM_CLOSE_PANE_TITLE: &str = "Close pane";
pub const CONFIRM_DELETE_PROMPT_TITLE: &str = "Delete prompt";
pub const CONFIRM_DELETE_FOLDER_TITLE: &str = "Delete folder";
pub const CONFIRM_SWITCH_CWD_TITLE: &str = "Switch to ws path";
pub const CONFIRM_SYNC_WS_CWD_TITLE: &str = "Sync ws to terminal path";

/// 信任工作区模态：标题、问题与按钮。
pub const TRUST_WORKSPACE_TITLE: &str = "Trust workspace";
pub const TRUST_WORKSPACE_QUESTION: &str = "Do you trust the files in this folder?";
pub const BUTTON_TRUST: &str = "[trust & open ↵]";
pub const BUTTON_DONT_TRUST: &str = "[don't trust esc]";
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
pub const TRUST_CONSOLE_FOOTER: &str = "[↑/↓] select  [↵] confirm  [esc] cancel";
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

/// 模态底部按钮：名称在前、快捷键在后。
pub const BUTTON_SAVE: &str = "[save ↵]";
pub const BUTTON_CREATE: &str = "[create ↵]";
pub const BUTTON_CLEAR: &str = "[clear ^c]";
pub const BUTTON_CANCEL: &str = "[cancel esc]";
pub const BUTTON_CONFIRM: &str = "[confirm ↵]";
/// `/resume` 会话编辑模态的删除与二次确认按钮。
pub const BUTTON_DELETE: &str = "[delete ^d]";
pub const BUTTON_CONFIRM_DELETE: &str = "[confirm del ↵]";
/// 模态浮层与 toast 右上角的关闭按钮图标；与模板块删除按钮（`TEMPLATE_BUTTON_DEL`）
/// 共用同一字符，一致性由 `text::tests` 的回归测试钉住。
pub const MODAL_CLOSE: &str = "[✕]";
pub const BUTTON_OPEN: &str = "[open ↵]";

/// 关闭确认问题文案。
pub fn confirm_close_question(name: &str) -> String {
    format!("close \"{name}\"?")
}

/// 删除提示词确认问题文案。
pub fn confirm_delete_question(name: &str) -> String {
    format!("delete \"{name}\"?")
}

/// 删除文件夹确认问题文案：提示级联影响范围。
pub fn confirm_delete_folder_question(name: &str) -> String {
    format!("delete folder \"{name}\" and its contents?")
}

/// 删除提示词页确认问题文案。
pub fn confirm_delete_page_question(name: &str) -> String {
    format!("delete page \"{name}\"?")
}

/// 切换工作区路径确认问题文案。
pub fn confirm_switch_cwd_question(path: &str) -> String {
    format!("switch cwd to \"{path}\"?")
}

/// 同步当前工作区路径到终端路径的二次确认问题。
pub fn confirm_sync_ws_cwd_question(path: &str) -> String {
    format!("switch workspace path to \"{path}\"?")
}

/// 按显示宽度换行：优先在空白处断行，超宽单词按字符硬断（宽字符占两列）；
/// 空文本返回单行空串，保证调用方至少有一行可渲染。
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = word.width();
        let space = usize::from(!current.is_empty());
        if current_width + space + word_width <= width {
            if space == 1 {
                current.push(' ');
            }
            current.push_str(word);
            current_width += space + word_width;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if word_width <= width {
            current.push_str(word);
            current_width = word_width;
            continue;
        }
        // 超宽单词按字符硬断。
        for ch in word.chars() {
            let char_width = ch.width().unwrap_or(0);
            if current_width + char_width > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(ch);
            current_width += char_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// 标签栏右端退出按钮；贴屏幕右缘，右缘与右栏折叠态按钮对齐。
pub const EXIT_LABEL: &str = "[EXIT]";

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
/// 设置服务不可用时的提示。
pub const TOAST_SETTINGS_UNAVAILABLE: &str = "Settings service unavailable";

/// 提示词保存失败：内容保留脏状态，自动保存将重试。
pub const TOAST_PROMPT_SAVE_FAILED: &str = "Prompt save failed; will retry";

/// lx 聊天相关提示与状态文案。
///
/// **一律极简**：这些文案显示在输入框上方那一行（`chat_bar`），空间只有一行宽，
/// 过长会被 `…` 截断。因此用短语而不是整句——多余的上下文由位置本身表达
/// （它就贴在用户刚敲字的输入框上面）。
pub const LX_CHAT_ENTRY_UNAVAILABLE: &str = "Native Agent only";
pub const LX_CHAT_NO_MODEL: &str = "No model configured";
pub const LX_CHAT_NO_WORKSPACE: &str = "No workspace path";
pub const LX_CHAT_COMMIT_FAILED: &str = "Send failed, retry";
pub const LX_CHAT_SAVE_FAILED: &str = "Save failed";
pub const LX_CHAT_SESSION_BUSY: &str = "Session busy";
/// 工具结果落库失败：文件可能已变化，需要用户核查（绝不重试写入）。
pub const LX_CHAT_TOOL_RESULT_SAVE_FAILED: &str = "Tool result not saved";
/// 流中「按 Esc 停止」待命提示短语（从长到短，按可用宽度取第一条放得下的；对齐 opencode）。
pub const LX_CHAT_STOP_PHRASES: [&str; 4] = [
    "Press esc again to stop",
    "esc again to stop",
    "esc again: stop",
    "esc stop",
];
/// 数值 + 单位后缀（保留一位小数，整数值去掉 `.0`）：`1.2k` / `720k` / `1M`。
fn scaled(value: f64, suffix: &str) -> String {
    let text = format!("{value:.1}");
    let trimmed = text.strip_suffix(".0").unwrap_or(&text);
    format!("{trimmed}{suffix}")
}

/// token 短格式：`999` / `1.2k` / `12k` / `1.2M`（统计行用；≥10k 取整到 k）。
pub fn token_count(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 10_000 {
        return scaled(tokens as f64 / 1_000.0, "k");
    }
    if tokens < 1_000_000 {
        return format!("{}k", (tokens as f64 / 1_000.0).round() as i64);
    }
    scaled(tokens as f64 / 1_000_000.0, "M")
}

/// token 精确短格式（始终保留一位小数）：`999` / `1.2k` / `258.7k` / `1.2M`；
/// 状态行上下文占用用它（用量数值得看清一位小数）。
pub fn token_count_precise(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 1_000_000 {
        return scaled(tokens as f64 / 1_000.0, "k");
    }
    scaled(tokens as f64 / 1_000_000.0, "M")
}

/// 状态行：模型名与上下文占用之间的分隔（` · `）。
pub const LX_CHAT_MODEL_SEP: &str = " · ";
/// thinking 头部文案：生成中 `Thinking`、完成后 `Thought · <耗时>`
/// （`Thought` 与 UI 页 agent 步骤的动词一致）。
pub const LX_CHAT_THINKING_LABEL: &str = "Thinking";
pub const LX_CHAT_THOUGHT_LABEL: &str = "Thought";
/// 本轮统计行（assistant 消息底部）：字段分隔与各字段后缀。
pub const LX_CHAT_STATS_SEP: &str = " · ";
pub const LX_CHAT_STATS_INPUT_SUFFIX: &str = " in";
pub const LX_CHAT_STATS_OUTPUT_SUFFIX: &str = " out";
pub const LX_CHAT_STATS_CACHE_SUFFIX: &str = " cached";
pub const LX_CHAT_STATS_SPEED_SUFFIX: &str = " tok/s";
/// `/` 命令面板标题。
pub const LX_SLASH_PANEL_TITLE: &str = "/Command";
/// `/` 命令面板底边提示（与 `@` 提及面板同款）。
pub const LX_SLASH_PANEL_FOOTER: &str = "[select ↵] [close esc]";
/// `/resume` 命令描述。
pub const LX_RESUME_COMMAND_DESC: &str = "Resume a previous session in this workspace";
/// `/resume` 历史面板标题与空态。
pub const LX_HISTORY_PANEL_TITLE: &str = "Resume";
pub const LX_HISTORY_EMPTY: &str = "no sessions yet";
/// `/resume` 历史面板底边提示：`⇧↵` 打开会话编辑模态（与 `@` 面板同符号写法）。
pub const LX_HISTORY_PANEL_FOOTER: &str = "[select ↵] [edit ⇧↵] [close esc]";
/// `/model` 面板标题。
pub const LX_MODEL_PANEL_TITLE: &str = "Model";
pub const LX_MODEL_VARIANT_PANEL_TITLE: &str = "Reasoning level";

/// `/stop` 命令描述：说清它停的是**后台进程**，而不是「停止生成」。
pub const LX_STOP_COMMAND_DESC: &str = "Stop background processes in this session";
/// `/stop` 一级面板标题（进程列表）与空态。
pub const LX_STOP_PANEL_TITLE: &str = "Background processes";
pub const LX_STOP_EMPTY: &str = "no background processes";
/// `/stop` 二级面板标题（动作列表）。
pub const LX_STOP_ACTION_PANEL_TITLE: &str = "Stop process";
/// `/stop` 二级面板的唯一动作。
pub const LX_STOP_ACTION_KILL: &str = "stop this process";
/// `/stop` 两级面板底边提示：一级进二级、二级执行并关闭。
pub const LX_STOP_PANEL_FOOTER: &str = "[select ↵] [back esc]";
/// `/stop` 执行后的聊天栏提示。
pub fn lx_stop_done(count: usize) -> String {
    if count == 1 {
        "Stopped 1 background process".to_string()
    } else {
        format!("Stopped {count} background processes")
    }
}
/// `/stop` 执行失败提示（进程已退出等，不影响其它进程）。
pub const LX_STOP_FAILED: &str = "Failed to stop the process";
/// `/resume` 会话编辑模态：标题、删除确认提示与校验错误。
pub const RESUME_EDIT_MODAL_TITLE: &str = "Session";
pub const RESUME_EDIT_CONFIRM_HINT: &str = "Press ^d again to confirm";
pub const RESUME_EDIT_EMPTY_TITLE: &str = "title cannot be empty";
pub const RESUME_EDIT_PLACEHOLDER: &str = "session title";
/// 消息底部通知的标题（固定枚举，对齐 ui 页 agent error；渲染为 `! <标题>`）。
pub const LX_CHAT_NOTICE_FAILED: &str = "Failed";
pub const LX_CHAT_NOTICE_INTERRUPTED: &str = "Interrupted";
/// 通知原因（一句话；失败有原因码时用原因码）。
pub const LX_CHAT_NOTICE_INTERRUPTED_REASON: &str = "interrupted before completion";
pub const LX_CHAT_NOTICE_FAILED_REASON: &str = "no error details provided";
pub const LX_CHAT_UNSUPPORTED_TOOLS: &str = "tool calls are not supported";
pub const LX_CHAT_NO_MODEL_SHORT: &str = "no model";
/// 上下文省略提示前缀。
pub const LX_CHAT_OMITTED_PREFIX: &str = "omitted turns:";
/// markdown 代码块头部的语言前缀（`language: js`）。
pub const LX_CHAT_CODE_LANG_PREFIX: &str = "language: ";

/// 工具步骤占位摘要：工具名与状态之间的分隔（真实工具行由后续阶段重做）。
pub const LX_CHAT_TOOL_STATUS_SEP: &str = " · ";
/// 工具步骤状态文案（占位渲染用；真实工具行动词见后续阶段的 Gallery 样式）。
pub const LX_CHAT_TOOL_INPUT_STREAMING: &str = "receiving input";
pub const LX_CHAT_TOOL_PENDING: &str = "pending";
pub const LX_CHAT_TOOL_RUNNING: &str = "running";
pub const LX_CHAT_TOOL_COMPLETED: &str = "done";
pub const LX_CHAT_TOOL_FAILED: &str = "failed";
pub const LX_CHAT_TOOL_CANCELLED: &str = "cancelled";
pub const LX_CHAT_TOOL_INTERRUPTED: &str = "interrupted";

/// 工具状态 → 固定文案。
pub fn tool_status_label(status: crate::app::state::ToolStatus) -> &'static str {
    use crate::app::state::ToolStatus;
    match status {
        ToolStatus::InputStreaming => LX_CHAT_TOOL_INPUT_STREAMING,
        ToolStatus::Pending => LX_CHAT_TOOL_PENDING,
        ToolStatus::Running => LX_CHAT_TOOL_RUNNING,
        ToolStatus::Completed => LX_CHAT_TOOL_COMPLETED,
        ToolStatus::Failed => LX_CHAT_TOOL_FAILED,
        ToolStatus::Cancelled => LX_CHAT_TOOL_CANCELLED,
        ToolStatus::Interrupted => LX_CHAT_TOOL_INTERRUPTED,
    }
}

/// 信任检查拒绝时的取消提示。
pub const TOAST_CREATE_CANCELLED: &str = "Workspace creation cancelled";
pub const TOAST_SYNC_CANCELLED: &str = "Workspace sync cancelled";

/// tooltip 边框标题；当前所有按钮共用一个固定标题。
pub const TOOLTIP_TITLE: &str = "Tooltip";

/// 粘贴二级面板与附件条文案。
pub const PASTE_PANEL_TITLE: &str = "Paste";
pub const PASTE_ACTION_REFERENCE: &str = "Insert reference";
pub const PASTE_ACTION_PATH: &str = "Insert path";
pub const PASTE_REFERENCE_UNSUPPORTED: &str = "path cannot be referenced";
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

/// 按钮 tooltip 文案：显示点击后的动作，随按钮当前状态翻转（收起/展开、钉住、切换视图）。
pub fn tooltip_hint(target: TooltipTarget, state: &AppState) -> &'static str {
    match target {
        TooltipTarget::CollapseSidebar if state.sidebar_collapsed => "expand sidebar",
        TooltipTarget::CollapseSidebar => "collapse sidebar",
        TooltipTarget::CollapsePrompt if state.prompt_collapsed => "expand prompt",
        TooltipTarget::CollapsePrompt => "collapse prompt",
        TooltipTarget::CollapsePanes if state.panes_collapsed => "expand panes",
        TooltipTarget::CollapsePanes => "collapse panes",
        TooltipTarget::CollapseAgents if state.agents_collapsed => "expand agents",
        TooltipTarget::CollapseAgents => "collapse agents",
        TooltipTarget::PaneToggle(pane) => match state.pane_anywhere(pane).map(|pane| pane.view) {
            Some(PaneView::Lx) => "switch to terminal",
            Some(PaneView::Terminal) => "switch to lx",
            None => "switch view",
        },
        TooltipTarget::AddWorkspace => "new workspace",
        TooltipTarget::AddTab => "new tab",
        TooltipTarget::TabScrollLeft => "scroll tabs left",
        TooltipTarget::TabScrollRight => "scroll tabs right",
        TooltipTarget::Exit => "quit lx-tui",
        TooltipTarget::PromptMode => {
            if state.prompt_owner().prompt_panel.mode == crate::app::prompt::PromptMode::Tree {
                "open editor"
            } else {
                "open prompt list"
            }
        }
        TooltipTarget::PromptTempPrompt => "open temp prompt",
        TooltipTarget::PromptUndo => "undo (^z)",
        TooltipTarget::PromptRedo => "redo (^y)",
        TooltipTarget::PromptSelectAll => "select all",
        TooltipTarget::PromptSave => "mark saved",
        TooltipTarget::PromptPin if state.prompt_pinned.is_some() => "unpin prompt",
        TooltipTarget::PromptPin => "pin prompt",
        TooltipTarget::SwapPromptSide => "swap prompt side",
        TooltipTarget::SidebarSettings => "open settings",
        TooltipTarget::SidebarUiGallery => "open ui gallery",
        TooltipTarget::TemplateButton { button, .. } => match button {
            TemplateButtonKind::Status => "cycle block status",
            TemplateButtonKind::Copy => "copy block",
            TemplateButtonKind::Clean => "clean block",
            TemplateButtonKind::Delete => "delete block",
        },
        TooltipTarget::PageMenuAdd => "add page",
        TooltipTarget::PageMenuDelete => "delete page",
        TooltipTarget::PromptFolders => "referenced folders",
        TooltipTarget::ReferencedSearchCopyPath => "copy path",
        TooltipTarget::BlockButton { button, .. } => match button {
            BlockButtonKind::Copy => "copy block",
            BlockButtonKind::Clean => "clean block",
            BlockButtonKind::Delete => "delete block",
        },
        TooltipTarget::LxCodeCopy { .. } => "copy code",
    }
}

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

/// 侧栏底边框设置按钮右侧的 UI 组件查看按钮（纯 ASCII）。
pub const UI_GALLERY_LABEL: &str = "[UI]";
/// UI 组件查看弹窗边框标题与底部提示。
pub const UI_GALLERY_TITLE: &str = "UI";
pub const UI_GALLERY_HINT: &str = "[↑/↓] select  [↵] fold  [esc] close";

/// UI 组件查看弹窗：顶层文件夹展示名。
pub const UI_GALLERY_FOLDER_GLOBAL: &str = "global";
pub const UI_GALLERY_FOLDER_PROMPT: &str = "prompt";

/// UI 组件查看弹窗：组件展示名。
pub const UI_GALLERY_TOAST: &str = "toast";
pub const UI_GALLERY_MODAL: &str = "modal";
pub const UI_GALLERY_TOOLTIP: &str = "tooltip";
pub const UI_GALLERY_BUTTON: &str = "button";
pub const UI_GALLERY_MENU: &str = "menu";
pub const UI_GALLERY_FULLSCREEN: &str = "fullscreen";
pub const UI_GALLERY_PROMPT_EDITOR: &str = "editor";
pub const UI_GALLERY_PROMPT_LIST: &str = "list";
pub const UI_GALLERY_PROMPT_TOOLBAR: &str = "toolbar";
pub const UI_GALLERY_FOLDERS: &str = "folders";
pub const UI_GALLERY_FOLDER_SEARCH: &str = "folder search";

/// UI 组件查看弹窗右侧：预览提示与用途说明（不再使用 `Preview` 标题）。
pub const UI_GALLERY_PREVIEW_HINT: &str = "select a component on the left";

/// prompt 预览的顶边框内容：编辑视图 `条目名 · 页名` + 页控件，列表视图固定标题。
pub const UI_GALLERY_PROMPT_EDITOR_TITLE: &str = "my prompt";
pub const UI_GALLERY_PROMPT_EDITOR_PAGE: &str = "Page 1";
pub const UI_GALLERY_PROMPT_EDITOR_PAGE_TOTAL: usize = 2;

/// 右侧真实外观预览的样例文案（复用真实组件布局与样式绘制）。
pub const UI_GALLERY_TOAST_SAMPLE: &str = "Copied to clipboard";
pub const UI_GALLERY_TOOLTIP_SAMPLE: &str = "undo (^z)";
pub const UI_GALLERY_MODAL_SAMPLE_TITLE: &str = "Close workspace";
pub const UI_GALLERY_MODAL_SAMPLE_BODY: &str = "Close this workspace?";
pub const UI_GALLERY_MENU_SAMPLE_TITLE: &str = "demo";
pub const UI_GALLERY_MENU_SAMPLE: [&str; 4] = ["New terminal", "Open prompt", "Rename", "Close"];
pub const UI_GALLERY_BUTTON_SAMPLE: &str = "[SET]  [UI]  [EXIT]  [≡]  [◉]";
pub const UI_GALLERY_FULLSCREEN_SAMPLE_TITLE: &str = "UI";
pub const UI_GALLERY_FULLSCREEN_SAMPLE_HINT: &str = "[esc] close";
pub const UI_GALLERY_PROMPT_EDITOR_SAMPLE: &str = "[≡] [←] [→]              [◉] ●";
pub const UI_GALLERY_PROMPT_EDITOR_BODY: [&str; 4] =
    ["# Prompt", "", "- first item", "- second item"];
pub const UI_GALLERY_PROMPT_LIST_SAMPLE: &str = "> search prompts…";
pub const UI_GALLERY_PROMPT_LIST_BODY: [&str; 5] = ["tmp:", "> default", "", "saved:", "▾ notes"];
pub const UI_GALLERY_PROMPT_TOOLBAR_LEFT: &str = "[≡] [←] [→]";
pub const UI_GALLERY_PROMPT_TOOLBAR_RIGHT: &str = "[◉] [*] ●";
/// 引用文件夹目录菜单预览样例（选中项、普通项与「+ add folder…」动作行）。
pub const UI_GALLERY_FOLDERS_SAMPLE: [&str; 2] = ["my-project", "lx-tui"];
/// 引用文件夹内容搜索预览样例（图标、名称、父目录）。
pub const UI_GALLERY_FOLDER_SEARCH_QUERY: &str = "> search files…";
pub const UI_GALLERY_FOLDER_SEARCH_ROWS: [(&str, &str, &str); 2] = [
    ("▸", "src/", "~/projects/my-project"),
    ("•", "main.rs", "~/projects/my-project/src"),
];

pub const UI_GALLERY_TOAST_DESC: &[&str] = &[
    "Non-modal feedback anchored below the container top border.",
    "Auto-dismisses; hover pauses it. [✕] or an outside click",
    "closes it.",
];
pub const UI_GALLERY_MODAL_DESC: &[&str] = &[
    "Centered modal shell with a rounded accent border,",
    "a title and a top-right [✕] close button.",
    "Esc or an outside click closes it.",
];
pub const UI_GALLERY_TOOLTIP_DESC: &[&str] = &[
    "Small muted box shown after hovering a button.",
    "Anchored to the button, clamped inside the screen.",
];
pub const UI_GALLERY_BUTTON_DESC: &[&str] = &[
    "ASCII label drawn per-cell with the accent color,",
    "the same style as collapse and toolbar buttons.",
];
pub const UI_GALLERY_MENU_DESC: &[&str] = &[
    "Right-click context menu anchored at the pointer.",
    "Up/Down moves the highlight, Enter runs the item.",
];
pub const UI_GALLERY_FULLSCREEN_DESC: &[&str] = &[
    "Fullscreen overlay: an outer accent border with the title and",
    "hint embedded in the border line, plus a top-right [✕].",
];
pub const UI_GALLERY_PROMPT_EDITOR_DESC: &[&str] = &[
    "Markdown editor with a top toolbar row: typing,",
    "undo/redo, selection, @ mentions and template blocks.",
];
pub const UI_GALLERY_PROMPT_LIST_DESC: &[&str] = &[
    "Tree view: search box, tmp:/saved: sections,",
    "folders and prompt items.",
];
pub const UI_GALLERY_PROMPT_TOOLBAR_DESC: &[&str] = &[
    "Editor toolbar row: mode, undo/redo, pin,",
    "select all and the save dot.",
];
pub const UI_GALLERY_FOLDERS_DESC: &[&str] = &[
    "Referenced folders menu opened from the prompt toolbar;",
    "lists project directories and opens a content search.",
];
pub const UI_GALLERY_FOLDER_SEARCH_DESC: &[&str] = &[
    "Searches a referenced folder's contents and copies",
    "file or folder markdown references.",
];

/// 模板块操作按钮文案；状态按钮按当前状态取标签，动作按钮为图标。
pub const TEMPLATE_BUTTON_TODO: &str = "[todo]";
pub const TEMPLATE_BUTTON_RUN: &str = "[run]";
pub const TEMPLATE_BUTTON_DONE: &str = "[done]";
pub const TEMPLATE_BUTTON_COPY: &str = "[❐]";
pub const TEMPLATE_BUTTON_CLEAN: &str = "[⌫]";
pub const TEMPLATE_BUTTON_DEL: &str = "[✕]";

/// 状态按钮标签：todo / run / done。
pub fn template_status_label(status: TemplateStatus) -> &'static str {
    match status {
        TemplateStatus::Todo => TEMPLATE_BUTTON_TODO,
        TemplateStatus::InProgress => TEMPLATE_BUTTON_RUN,
        TemplateStatus::Done => TEMPLATE_BUTTON_DONE,
    }
}

/// 补充块操作按钮：清理与删除（复用模板块图标）。
pub const SUPPLE_BUTTON_CLEAN: &str = TEMPLATE_BUTTON_CLEAN;
pub const SUPPLE_BUTTON_DEL: &str = TEMPLATE_BUTTON_DEL;

/// @ 面板底边快捷键提示：进入目录 `Shift+Enter` / 回退上一级 `Ctrl+Z`（`^z` 沿用 `[clear ^c]` 记法）。
pub const MENTION_PANEL_FOOTER: &str = "[open ⇧↵] [back ^z]";

/// lx 输入框 `@` 提及面板：标题与状态行文案（与 prompt 面板同一套外观）。
pub const LX_MENTION_PANEL_TITLE: &str = "@Command";
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

/// 按显示宽度截断（`…` 收尾）；`max` 非正时返回空串。
pub fn truncate_to_width(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0;
    for ch in text.chars() {
        let cell = ch.width().unwrap_or(0);
        if width + cell > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        width += cell;
    }
    out.push('…');
    out
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

#[cfg(test)]
mod tests;
