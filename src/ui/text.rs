//! 全部用户可见文案；组件与逻辑禁止散落硬编码字符串。

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::{BlockCommandId, TemplateCounts, TemplateStatus};
use crate::app::state::{AppState, PaneView};
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

/// prompt 面板边框标题；路径末段名以 `ws:` 前缀渲染在顶边框右侧。
pub const PROMPT_TITLE: &str = "Prompt Manager";
/// prompt 底边框分支前缀。
pub const PROMPT_BRANCH_PREFIX: &str = "b:";
/// prompt 顶边框右侧的工作区路径名前缀。
pub const PROMPT_WORKSPACE_PREFIX: &str = "ws:";
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
/// 提示词树分区标签：临时区与已入库区（仅两侧都有内容时成对出现）。
pub const PROMPT_SECTION_TMP: &str = "tmp:";
pub const PROMPT_SECTION_SAVED: &str = "saved:";
/// 空白 default temp 在列表中的占位展示名（每个工作区恒保留一条）。
pub const PROMPT_TEMP_DEFAULT_NAME: &str = "default";
/// 临时提示词在编辑视图分割线行的标题。
pub const PROMPT_DRAFT_TITLE: &str = "temp prompt";
pub const PROMPT_TREE_EXPANDED: &str = "▾";
pub const PROMPT_TREE_COLLAPSED: &str = "▸";
pub const PROMPT_TREE_ITEM_ICON: &str = "•";
pub const PROMPT_TREE_LAST: &str = "└─";
pub const PROMPT_TREE_MIDDLE: &str = "├─";
/// 提示词树空状态与搜索无命中占位。
pub const PROMPT_TREE_EMPTY: &str = "no prompts yet";
pub const PROMPT_TREE_NO_MATCHES: &str = "no matching prompts";
/// 临时草稿保存选择器：标题、根层项与新建文件夹动作项。
pub const SAVE_DRAFT_TITLE: &str = "save prompt to folder";
pub const SAVE_DRAFT_ROOT: &str = "/";
pub const SAVE_DRAFT_NEW_FOLDER: &str = "+ New folder";
/// 新建提示词与文件夹的默认名（可随后重命名）。
pub const PROMPT_DEFAULT_NAME: &str = "New prompt";
pub const PROMPT_FOLDER_DEFAULT_NAME: &str = "New folder";
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
/// 默认页名 `Page N`；`index` 从 1 起。
pub fn default_page_name(index: usize) -> String {
    format!("Page {index}")
}

/// 页菜单动作项与标题。
pub const MENU_PAGE_NEW: &str = "[add]";
pub const MENU_PAGE_DELETE: &str = "[del]";
pub const MENU_TITLE_PAGES: &str = "pages";

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
pub const CONFIRM_DELETE_PAGE_TITLE: &str = "delete page";

/// 重命名与关闭确认浮层标题。
pub const RENAME_WORKSPACE_TITLE: &str = "rename workspace";
pub const NEW_WORKSPACE_TITLE: &str = "new workspace";
pub const RENAME_PROMPT_TITLE: &str = "rename prompt";
pub const RENAME_FOLDER_TITLE: &str = "rename folder";
pub const NEW_FOLDER_TITLE: &str = "new folder";
pub const NEW_PROMPT_TITLE: &str = "new prompt";
/// rename 浮层输入为空时的输入说明占位（工作区与标签共用）。
pub const RENAME_PLACEHOLDER: &str = "new name";
/// new workspace 路径输入为空时的输入说明占位。
pub const NEW_WORKSPACE_PLACEHOLDER: &str = "path to directory";
pub const RENAME_TAB_TITLE: &str = "rename tab";
pub const CONFIRM_CLOSE_TITLE: &str = "close workspace";
pub const CONFIRM_CLOSE_TAB_TITLE: &str = "close tab";
pub const CONFIRM_CLOSE_PANE_TITLE: &str = "close pane";
pub const CONFIRM_DELETE_PROMPT_TITLE: &str = "delete prompt";
pub const CONFIRM_DELETE_FOLDER_TITLE: &str = "delete folder";
pub const CONFIRM_SWITCH_CWD_TITLE: &str = "switch to ws path";
pub const CONFIRM_SYNC_WS_CWD_TITLE: &str = "sync ws to terminal path";

/// 信任工作区模态：标题、问题与按钮。
pub const TRUST_WORKSPACE_TITLE: &str = "trust workspace";
pub const TRUST_WORKSPACE_QUESTION: &str = "Do you trust the files in this folder?";
pub const BUTTON_TRUST: &str = "[trust & open enter]";
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
pub const TRUST_CONSOLE_FOOTER: &str = "[↑/↓] select  [enter] confirm  [esc] cancel";
/// 启动拒绝信任后的终端提示（不进入应用，直接退出）。
pub const TRUST_REFUSED_MESSAGE: &str = "workspace not trusted; exiting";

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
/// 模态浮层与 toast 右上角的关闭按钮图标；与模板块删除按钮（`TEMPLATE_BUTTON_DEL`）
/// 共用同一字符，一致性由 `text::tests` 的回归测试钉住。
pub const MODAL_CLOSE: &str = "[✕]";
pub const BUTTON_OPEN: &str = "[open enter]";

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
        TooltipTarget::PromptUndo => "undo (ctrl+z)",
        TooltipTarget::PromptRedo => "redo (ctrl+y)",
        TooltipTarget::PromptSelectAll => "select all",
        TooltipTarget::PromptSave => "mark saved",
        TooltipTarget::PromptPin if state.prompt_pinned.is_some() => "unpin prompt",
        TooltipTarget::PromptPin => "pin prompt",
        TooltipTarget::SwapPromptSide => "swap prompt side",
        TooltipTarget::SidebarSettings => "open settings",
        TooltipTarget::TemplateButton { button, .. } => match button {
            TemplateButtonKind::Status => "cycle block status",
            TemplateButtonKind::Copy => "copy block",
            TemplateButtonKind::Clean => "clean block",
            TemplateButtonKind::Delete => "delete block",
        },
        TooltipTarget::PageMenuAdd => "add page",
        TooltipTarget::PageMenuDelete => "delete page",
        TooltipTarget::BlockButton { button, .. } => match button {
            BlockButtonKind::Copy => "copy block",
            BlockButtonKind::Clean => "clean block",
            BlockButtonKind::Delete => "delete block",
        },
    }
}

/// 右键菜单边框标题：目标名缺失时按种类回退。
pub const MENU_TITLE_WORKSPACE: &str = "workspace";
pub const MENU_TITLE_WORKSPACES: &str = "workspaces";
pub const MENU_TITLE_TAB: &str = "tab";
pub const MENU_TITLE_PANE: &str = "pane";
pub const MENU_TITLE_PROMPT: &str = "prompt";
pub const MENU_TITLE_FOLDER: &str = "folder";
pub const MENU_TITLE_PROMPT_TREE: &str = "prompts";

/// 浮层命令面板顶边左侧标题：markdown 块命令 / 文件提及 / 斜杠命令 / worktree 选择器。
pub const BLOCK_PANEL_TITLE: &str = "Commands";
pub const MENTION_PANEL_TITLE: &str = "Files";
pub const SLASH_PANEL_TITLE: &str = "Templates";
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

#[cfg(test)]
mod tests;
