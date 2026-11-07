//! 全部用户可见文案；组件与逻辑禁止散落硬编码字符串。

pub mod agent;
pub mod chat;
pub mod connect;
pub mod mcp;
pub mod skills;
pub mod todo;
pub mod workspace;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::BlockCommandId;
#[cfg(test)]
use crate::app::state::WorkspaceIdentity;
use crate::app::state::{AiWebSite, AppState, LxEntry, PaneView};
use crate::app::tooltip::{BlockButtonKind, TemplateButtonKind, TooltipTarget};
#[cfg(test)]
use crate::layout::PaneId;

pub use connect::*;
pub use todo::*;
pub use workspace::*;

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

/// 主内容 lx 页标题（窗格顶边框）。
pub const LX_TITLE: &str = "lx";
/// 视图切换按钮：显示点击后的目的地视图。
pub const LX_TOGGLE_TERMINAL: &str = "[>_]";
pub const LX_TOGGLE_LX: &str = "[lx]";
/// lx 页底部输入框：`>` 前缀与占位。
pub const LX_INPUT_PROMPT: &str = ">";
pub const LX_INPUT_PLACEHOLDER: &str = "Ask anything…";
/// shell 模式：与 `>` 等宽的 `! ` 前缀与规则行右端提示。
pub const LX_INPUT_SHELL_PROMPT: &str = "!";
pub const LX_INPUT_SHELL_PLACEHOLDER: &str = "Run a shell command…";
pub const LX_INPUT_SHELL_MODE: &str = "Shell mode";
/// 本地命令：
pub const LX_SHELL_EMPTY_COMMAND: &str = "Type a command after !";
pub const LX_SHELL_NO_CWD: &str = "No working directory for this command";
pub const LX_SHELL_COMMIT_FAILED: &str = "Could not save the command; draft kept";
pub const LX_SHELL_SPAWN_FAILED: &str = "Could not start the command";
pub const LX_SHELL_RUNNING: &str = "Running";
pub const LX_SHELL_COMMAND: &str = "You run command";
pub const LX_SHELL_NO_OUTPUT: &str = "(no output)";
pub const LX_SHELL_STOP_ARMED: &str = "Press Esc again to stop the running command";
/// loading 行右端的回到底部控件：箭头 + 右侧说明文字。
pub const LX_JUMP_BOTTOM: &str = "↓";
pub const LX_JUMP_BOTTOM_HINT: &str = "jump to bottom";
/// lx 页四个入口标签。
pub const LX_ENTRY_NATIVE: &str = "Native";
pub const LX_ENTRY_HYBRID: &str = "Hybrid";
pub const LX_ENTRY_AUTOMATION: &str = "Automation";
pub const LX_ENTRY_AI_WEB: &str = "AI Web";
/// AI Web 目标站点标签。
pub const AI_WEB_SITE_DEEPSEEK: &str = "DeepSeek";
/// lx 页横排入口标签之间的分隔符。
pub const LX_ENTRY_SEPARATOR: &str = "·";
/// lx 页入口 Tip 提示前缀；前缀强调色、句子 muted。
pub const LX_TIP_PREFIX: &str = "Tip:";
/// 首页资源计数：AGENTS.md / MCP / skills / error。
pub const LX_HOME_AGENTS_MD: &str = "AGENTS.md";
pub const LX_HOME_MCP: &str = "mcp";
pub const LX_HOME_SKILLS: &str = "skills";
pub const LX_HOME_ERROR: &str = "error";
pub const LX_HOME_ERRORS: &str = "errors";
/// lx 页入口 Tip 提示行：选中入口一句话描述。
pub const LX_ENTRY_NATIVE_DESC: &str = "talks to the model directly";
pub const LX_ENTRY_HYBRID_DESC: &str = "mixes native and external agents";
pub const LX_ENTRY_AUTOMATION_DESC: &str = "runs your tasks automatically";
pub const LX_ENTRY_AI_WEB_DESC: &str = "chat directly with web AI models";
/// lx 页 md 格式面板边框标题。
pub const LX_BLOCK_PANEL_TITLE: &str = "Markdown";

/// lx 页入口标签：按选中入口取。
pub fn lx_entry_label(entry: LxEntry) -> &'static str {
    match entry {
        LxEntry::Native => LX_ENTRY_NATIVE,
        LxEntry::Hybrid => LX_ENTRY_HYBRID,
        LxEntry::Automation => LX_ENTRY_AUTOMATION,
        LxEntry::AiWeb => LX_ENTRY_AI_WEB,
    }
}

/// lx 页入口 Tip 提示行（一句话描述）。
pub fn lx_entry_description(entry: LxEntry) -> &'static str {
    match entry {
        LxEntry::Native => LX_ENTRY_NATIVE_DESC,
        LxEntry::Hybrid => LX_ENTRY_HYBRID_DESC,
        LxEntry::Automation => LX_ENTRY_AUTOMATION_DESC,
        LxEntry::AiWeb => LX_ENTRY_AI_WEB_DESC,
    }
}

/// AI Web 站点显示名称。
pub fn ai_web_site_label(site: AiWebSite) -> &'static str {
    match site {
        AiWebSite::Deepseek => AI_WEB_SITE_DEEPSEEK,
    }
}

/// @ 提及面板条目标记：目录 / 文件（标准单宽通用符号，兼容无 Nerd Font 的终端）。
pub const MENTION_DIR_ICON: &str = "▸";
pub const MENTION_FILE_ICON: &str = "•";
pub const MENTION_SKILL_ICON: &str = "◆";
/// `@` 面板 skill 条目的详情前缀。
pub const MENTION_SKILL_LABEL: &str = "Skill";

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

/// 复制成功反馈文案：`Copied 1 char` / `Copied N chars`（按 Unicode 字符数计）。
pub fn toast_copied(text: &str) -> String {
    toast_copied_chars(text.chars().count())
}

/// 按字符数生成复制成功反馈文案。
pub fn toast_copied_chars(count: usize) -> String {
    if count == 1 {
        "Copied 1 char".to_string()
    } else {
        format!("Copied {count} chars")
    }
}

/// 设置服务不可用时的提示。
pub const TOAST_SETTINGS_UNAVAILABLE: &str = "Settings service unavailable";

/// 提示词保存失败：内容保留脏状态，自动保存将重试。
pub const TOAST_PROMPT_SAVE_FAILED: &str = "Prompt save failed; will retry";
/// 提示词/文件夹移动失败（写库失败）。
pub const TOAST_PROMPT_MOVE_FAILED: &str = "Failed to move; unchanged";
/// 目标会形成第 3 层文件夹，或该层不收文件夹。
pub const TOAST_PROMPT_MOVE_DEPTH: &str = "Cannot move: folders support two levels";
/// 被拖文件夹内含子文件夹，不能整体移入另一文件夹。
pub const TOAST_PROMPT_MOVE_NESTED: &str = "Cannot move: folder contains subfolders";
/// 已有未编辑的空页提示。
pub const TOAST_PAGE_ALREADY_NEW: &str = "Already have a new page";
/// 新页创建成功提示。
pub const TOAST_PAGE_CREATED: &str = "New page created";
/// 页面 Fork 成功提示。
pub const TOAST_PAGE_FORKED: &str = "Page forked";
/// 当前页面未找到可重命名的 `{title:}` 提示。
pub const TOAST_PAGE_NO_TITLE: &str = "No {title:} found in current page";
/// Prompt 模板块 `/title` 生成中占位与提示文案（英文）。
pub const PROMPT_TITLE_GENERATING: &str = "Generating title...";
pub const PROMPT_TITLE_EMPTY_BLOCK: &str = "Template block is empty";
pub const PROMPT_TITLE_NO_MODEL: &str = "No title model or default model configured";
pub const PROMPT_TITLE_FAILED: &str = "Failed to generate title";

/// lx 聊天相关提示与状态文案。
///
/// **一律极简**：这些文案显示在输入框上方那一行（`chat_bar`），空间只有一行宽，
/// 过长会被 `…` 截断。因此用短语而不是整句——多余的上下文由位置本身表达
/// （它就贴在用户刚敲字的输入框上面）。
pub const LX_CHAT_ENTRY_UNAVAILABLE: &str = "Native Agent only";
pub const LX_AI_WEB_LOGIN_REQUIRED: &str = "Please sign in to DeepSeek in ego-browser";
pub const LX_AI_WEB_EGO_UNAVAILABLE: &str = "ego-browser is not installed or unavailable";
pub const LX_AI_WEB_ONLY_NEW_CMD: &str =
    "Only /new, /resume, /rename, and /rewind are available in AI Web";
pub const LX_AI_WEB_FOLDER_UNSUPPORTED: &str =
    "Folders cannot be uploaded in AI Web; select files instead";
pub const LX_AI_WEB_FILE_NOT_FOUND: &str = "Attached file does not exist";
pub const LX_AI_WEB_LOCAL_REF_UNSUPPORTED: &str =
    "Skills and MCP references are not available in AI Web";
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

/// 耗时短格式（对齐 lx-agent）：`820ms`、`4.2s`、`38s`、`1m 12s`。
///
/// 放在 `text` 里而不是某个渲染子模块：本轮统计行、思考耗时与 loading 行的
/// 流中耗时都要用它，谁都不是它的唯一归属。
pub fn format_duration(ms: i64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1_000.0;
    if seconds < 60.0 {
        return if seconds < 10.0 {
            format!("{seconds:.1}s")
        } else {
            format!("{:.0}s", seconds)
        };
    }
    let minutes = (seconds / 60.0).floor() as i64;
    let rest = (seconds % 60.0).round() as i64;
    format!("{minutes}m {rest}s")
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
/// `/` 命令面板底边快捷键提示（无边框面板里的单行 muted 提示）。
pub const LX_SLASH_PANEL_FOOTER: &str = "↑↓ select · ↵ open · esc close";
/// `/mcp` 命令描述。
pub const LX_MCP_COMMAND_DESC: &str = "Manage MCP servers for this pane";
/// `/mcp` 面板标题。
pub const LX_MCP_PANEL_TITLE: &str = "MCP servers";
/// `/mcp` 面板底栏按键提示。
pub const LX_MCP_PANEL_FOOTER: &str = "↑↓ select · space/↵ toggle · esc close";
/// `/mcp` 面板空态文案。
pub const LX_MCP_EMPTY: &str = "No MCP servers configured";
/// `/mcp` 面板状态标签。
pub const LX_MCP_STATUS_CONNECTED: &str = "Connected";
pub const LX_MCP_STATUS_CONNECTING: &str = "Connecting";
pub const LX_MCP_STATUS_FAILED: &str = "Failed";
pub const LX_MCP_STATUS_NEEDS_LOGIN: &str = "Needs login";
pub const LX_MCP_STATUS_DISABLED: &str = "Disabled";
pub const LX_MCP_STATUS_DISABLED_SETTINGS: &str = "Disabled in settings";
/// 尝试切换已在全局/项目设置中禁用的 MCP 时的警告提示。
pub const LX_MCP_DISABLED_IN_SETTINGS: &str =
    "This MCP server is disabled in settings. Enable it in Settings first.";
/// 路径粘贴面板底边快捷键提示。
pub const LX_PASTE_PANEL_FOOTER: &str = "↑↓ select · ↵ confirm · esc cancel";
/// 快捷引用面板底边快捷键提示。
pub const LX_REFER_PANEL_FOOTER: &str = "↑↓ select · ↵ insert · esc close";
/// `@` 面板按 Tab 区分的底边快捷键提示。
pub const LX_MENTION_FOOTER_FILES: &str = "←→ tabs · ↑↓ select · ↵ insert · ⇧↵ enter · esc close";
pub const LX_MENTION_FOOTER_REF_DIR: &str = "←→ tabs · ↑↓ select · ↵ insert · esc close";

// ===== 最新轮次撤销 =====

/// 撤销动作面板标题。
pub const LX_REWIND_PANEL_TITLE: &str = "Rewind";
/// 撤销动作面板底边快捷键提示。
pub const LX_REWIND_PANEL_FOOTER: &str = "↑↓ select · ↵ open · esc back";
/// 第一次 Esc 后的待命提示（toast，不写历史）。
pub const LX_REWIND_ARMED_HINT: &str = "Press esc again to rewind";
/// 面板范围说明。
pub const LX_REWIND_SCOPE: &str = "Files changed by edit/write only";
/// 四项动作文案（顺序与 `RewindAction::ALL` 一致）。
pub const LX_REWIND_ACTION_REVERT_AND_DELETE: &str = "Restore files and undo conversation";
pub const LX_REWIND_ACTION_DELETE_ONLY: &str = "Undo conversation only";
pub const LX_REWIND_ACTION_REVERT_ONLY: &str = "Restore files only";
pub const LX_REWIND_ACTION_CANCEL: &str = "Cancel";
/// 含文件动作不可用时的说明。
pub const LX_REWIND_SNAPSHOT_UNAVAILABLE: &str = "File snapshot unavailable";
pub const LX_REWIND_SNAPSHOT_INCOMPLETE: &str = "File snapshot incomplete";
pub const LX_REWIND_SNAPSHOT_LOADING: &str = "Checking file snapshot...";
/// 该轮文件此前已恢复：动作仍可用（幂等/只删对话），但要让用户知道不会再次写文件。
pub const LX_REWIND_FILES_ALREADY_RESTORED: &str = "Files already restored";
/// 撤销提示标题（独立于 user/assistant）。
pub const LX_REWIND_NOTICE_TITLE: &str = "Rewind";

/// 撤销提示标题：连续多条合并时带数量（`Rewind x2`）。
pub fn rewind_notice_title(count: usize) -> String {
    if count > 1 {
        format!("Rewind x{count}")
    } else {
        LX_REWIND_NOTICE_TITLE.to_string()
    }
}

/// 撤销提示描述：按动作、恢复数量与“此前已恢复”生成准确文案。
pub fn rewind_notice(kind: &str, restored: usize, already_restored: bool) -> String {
    if already_restored && kind != "revert_only" {
        return LX_REWIND_NOTICE_ALREADY_RESTORED.to_string();
    }
    match kind {
        "delete_only" => LX_REWIND_NOTICE_DELETE_ONLY.to_string(),
        "revert_only" => {
            if restored == 0 {
                // 没有文件变更时不能声称“恢复了文件”（设计 §9）。
                "No file changes; conversation kept.".to_string()
            } else {
                format!("Restored {restored} files; conversation kept.")
            }
        }
        _ => {
            if restored == 0 {
                format!("Conversation undone; {LX_REWIND_NOTICE_NO_FILES}.")
            } else {
                format!("Conversation undone; restored {restored} files.")
            }
        }
    }
}
/// 三种成功提示。
pub const LX_REWIND_NOTICE_REVERT_AND_DELETE: &str = "Conversation undone; restored files.";
pub const LX_REWIND_NOTICE_DELETE_ONLY: &str = "Conversation undone; files kept.";
pub const LX_REWIND_NOTICE_REVERT_ONLY: &str = "Restored files; conversation kept.";
pub const LX_REWIND_NOTICE_ALREADY_RESTORED: &str =
    "Conversation undone; files were restored earlier.";
pub const LX_REWIND_NOTICE_NO_FILES: &str = "no file changes";
/// 撤销失败/冲突提示。
pub const LX_REWIND_FAILED: &str = "Rewind failed; conversation and files were kept.";
pub const LX_REWIND_CONFLICT: &str = "Rewind refused: files changed after this turn.";
/// 撤销未完成/需手工恢复时阻止新轮次。
pub const LX_REWIND_RECOVERY_BLOCKED: &str =
    "This session has an unfinished rewind; recover it before sending";
pub const LX_REWIND_BUSY: &str = "Stop background tasks before rewinding.";
pub const LX_REWIND_SESSION_BUSY: &str = "This session is busy; finish or stop it first.";
/// 三级面板（`/model`、`/resume`、`/compact`）过滤后无命中：与「真的没有数据」区分开。
pub const LX_SLASH_NO_MATCHES: &str = "no matches";

// ===== 上下文压缩（`/compact`）=====

/// 压缩命令的一句话描述（`/` 面板）。
pub const LX_COMPACT_COMMAND_DESC: &str = "Summarize earlier context into a compact checkpoint";
/// 有内容被压缩时的成功提示（toast）。
pub const LX_COMPACT_DONE: &str = "Context compacted.";
/// 没有可压缩的增量（已是最新检查点之后为空）。
pub const LX_COMPACT_EMPTY: &str = "Nothing new to compact.";
/// 忙碌（流式回复 / 工具执行 / 压缩进行中）时拒绝。
pub const LX_COMPACT_BUSY: &str = "Finish the current response before compacting.";
/// 未绑定会话：没有可压缩的上下文。
pub const LX_COMPACT_NO_SESSION: &str = "Send a message first; there is no context to compact.";
/// 失败（网络/超限/取消）：**不写检查点**，原始消息保持不变。
pub const LX_COMPACT_FAILED: &str = "Compact failed; conversation was left unchanged.";
/// 撤销 compact 失败：检查点与消息保持不变。
pub const LX_COMPACT_REWIND_FAILED: &str =
    "Restore failed; conversation and checkpoints were left unchanged.";
/// 撤销 compact 的提示（`/rewind` 成功后）。
pub const LX_COMPACT_REWOUND: &str = "Context restored to the previous checkpoint.";
/// compact 撤销面板的唯一动作文案。
pub const LX_COMPACT_ACTION_RESTORE: &str = "Restore the context before this checkpoint";
/// 检查点条目标题（独立时间线条目）。
pub const LX_COMPACT_NOTICE_TITLE: &str = "Context compacted";
/// 展开态的范围标签。
pub const LX_COMPACT_RANGE_LABEL: &str = "covered";
/// 子 agent 运行时自动压缩的范围文案（没有真实消息 sequence）。
pub const LX_COMPACT_AUTO_RANGE: &str = "auto · this run";

/// 检查点条目标题：连续多条合并时带数量（`Context compacted x2`）。
pub fn compact_notice_title(count: usize) -> String {
    if count > 1 {
        format!("Context compacted x{count}")
    } else {
        LX_COMPACT_NOTICE_TITLE.to_string()
    }
}

/// 检查点条目描述：`model: gpt-5 · 10:42`（时间只到分钟，由状态层传入已格式化值）。
pub fn compact_notice(model: &str, time: &str) -> String {
    format!("model: {model} · {time}")
}

/// 模型切换提示条目标题（独立时间线条目）。
pub const LX_MODEL_SWITCH_NOTICE_TITLE: &str = "Model switched";

/// 模型切换提示条目标题：连续多条合并时带数量（`Model switched x2`）。
pub fn model_switch_notice_title(count: usize) -> String {
    if count > 1 {
        format!("Model switched x{count}")
    } else {
        LX_MODEL_SWITCH_NOTICE_TITLE.to_string()
    }
}

/// 模型切换提示条目的目标行：`model: DeepSeek V4.1 Flash (max) · 16:59`。
///
/// 仅思考等级变化时改成 `variant: max (DeepSeek V4.1 Flash) · 16:59`，把变化的
/// 那一维放在行首。
pub fn model_switch_notice_head(
    variant_only: bool,
    model_label: &str,
    variant: Option<&str>,
    time: &str,
) -> String {
    if variant_only {
        let level = variant.filter(|v| !v.is_empty()).unwrap_or("default");
        format!("variant: {level} ({model_label}) · {time}")
    } else {
        let model_display = match variant.filter(|v| !v.is_empty()) {
            Some(level) => format!("{model_label} ({level})"),
            None => model_label.to_string(),
        };
        format!("model: {model_display} · {time}")
    }
}

/// 模型切换提示条目的指令源行；没有变更来源时整行省略。
pub fn model_switch_notice_sources(updated_sources: &[String]) -> Option<String> {
    (!updated_sources.is_empty()).then(|| format!("instructions: {}", updated_sources.join(", ")))
}
/// `/resume` 命令描述。
pub const LX_RESUME_COMMAND_DESC: &str = "Resume a previous session in this workspace";
/// `/fork` 命令、节点面板与反馈文案。
pub const LX_FORK_COMMAND_DESC: &str = "Fork the session from an earlier message";
pub const LX_FORK_PANEL_TITLE: &str = "Fork";
pub const LX_FORK_PANEL_FOOTER: &str = "↑↓ select · ↵ fork · esc close";
pub const LX_FORK_FULL_SESSION: &str = "Full session";
pub const LX_FORK_EMPTY_MESSAGE: &str = "(empty message)";
pub const LX_FORK_BUSY: &str = "Session busy — wait for the current turn to finish.";
pub const LX_FORK_NOTHING: &str = "Nothing to fork yet.";
pub const LX_FORK_FAILED: &str = "Fork failed.";
pub const LX_FORKED: &str = "Forked session";
/// `/rename` 命令描述、参数占位提示与反馈文案。
pub const LX_RENAME_COMMAND_DESC: &str = "Rename the current session";
pub const LX_RENAME_ARG_HINT: &str = "[name]";
pub const LX_RENAME_EMPTY_NAME: &str = "Session name cannot be empty";
pub const LX_RENAME_NO_SESSION: &str = "No active session to rename";
pub const LX_RENAME_STANDALONE_FAILED: &str = "Run /rename on its own line";
/// `/resume` 历史面板标题、加载态与空态。
pub const LX_HISTORY_PANEL_TITLE: &str = "Resume";
pub const LX_HISTORY_LOADING: &str = "loading sessions…";
pub const LX_HISTORY_EMPTY: &str = "no sessions yet";
/// 会话历史加载中占位文案（消息列表区域居中展示）。
pub const LX_SESSION_LOADING: &str = "loading history…";
/// `/resume` 历史面板底边快捷键提示（`⇧↵` 打开会话编辑模态、`^c` 复制会话 id）。
pub const LX_HISTORY_PANEL_FOOTER: &str = "↑↓ select · ↵ open · ^c copy · ⇧↵ edit · esc close";
/// `/model` 面板标题。
pub const LX_MODEL_PANEL_TITLE: &str = "Model";
pub const LX_MODEL_VARIANT_PANEL_TITLE: &str = "Reasoning level";

/// 后台任务页（窗格内整页）：tab 标签、标题、底边提示与空态。
///
/// 对齐 opencode 的 composer 多 tab：进程与子代理各占一个 tab，指标与列表同一名词。
pub const LX_TASK_PAGE_TITLE: &str = "Background";
pub const LX_TASK_TAB_PROCESSES: &str = "Processes";
pub const LX_TASK_TAB_AGENTS: &str = "Agents";
/// 进程 tab 空态。
pub const LX_TASK_PROCESSES_EMPTY: &str = "no running processes";
/// 子代理 tab 空态（当前会话没有派发过 subagent）。
pub const LX_TASK_AGENTS_EMPTY: &str = "no subagents yet";
/// Agents 详情区：历史 run 的消息流还没从库里读回来。
pub const LX_TASK_AGENTS_LOADING: &str = "loading agent transcript…";
/// 输出区空态：高亮任务还没有产出（或刚起步）。
///
/// 必须显式占位而不是留白：面板下半区突然空着，会让人以为"输出丢了"。
pub const LX_TASK_OUTPUT_EMPTY: &str = "waiting for output";
/// 输出容器顶边栏的标题（右侧接高亮任务的命令原文）。
pub const LX_TASK_OUTPUT_TITLE: &str = "Output";
/// 一条键位提示：`[键 功能]`。
///
/// **快捷键符号在前、功能名在后**（统一格式），方括号包住整条——它既是一个视觉单位，
/// 也是可点击的目标。
pub const fn hint(action: &'static str, key: &'static str) -> Hint {
    Hint { action, key }
}

/// 键位提示（`[键 功能]`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    /// 功能名称，如 `select`。
    pub action: &'static str,
    /// 快捷键，如 `↑↓`。
    pub key: &'static str,
}

impl Hint {
    /// 完整文案：`[↑↓ select]`。
    pub fn label(self) -> String {
        format!("[{} {}]", self.key, self.action)
    }
}

/// 面板 tab 行右端的关闭按钮（`[close esc]`）。
pub const LX_TASK_CLOSE: Hint = hint("close", "esc");
/// 进程 tab 的底边框键位提示（按顺序从左到右，窄窗格整项丢弃）。
pub const LX_TASK_HINTS_PROCESSES: [Hint; 4] = [
    hint("select", "↑↓"),
    hint("stop", "^d"),
    hint("tabs", "←→"),
    hint("back", "esc"),
];
/// 子代理 tab 的键位提示（`tab` 切换筛选、`↵` 内联追问、`^d`/`^a` 停止）。
pub const LX_TASK_HINTS_AGENTS: [Hint; 7] = [
    hint("select", "↑↓"),
    hint("filter", "tab"),
    hint("reply", "↵"),
    hint("stop", "^d"),
    hint("stop all", "^a"),
    hint("tabs", "←→"),
    hint("back", "esc"),
];
/// Agents 内联追问输入态的底边框键位提示。
pub const LX_TASK_HINTS_AGENT_REPLY: &str = "[send ↵]  [cancel esc]";
/// 顶边右端的 tab 指示分隔符（`·`）。
pub const LX_TASK_TAB_SEP: &str = " · ";
/// 行尾的运行时长前缀（`running 1m 30s`）。
pub const LX_TASK_AGE_PREFIX: &str = "running ";

/// 输入框底边框左端的后台任务计数指标：`1 process` / `3 processes`。
///
/// 与 opencode 的 `N shell` 同一口径：数量为 0 时整段不渲染；单复数随数量变化。
pub fn lx_task_count_label(count: usize) -> String {
    if count == 1 {
        format!("1 {}", LX_TASK_COUNT_NOUN_ONE)
    } else {
        format!("{count} {}", LX_TASK_COUNT_NOUN_MANY)
    }
}
pub const LX_TASK_COUNT_NOUN_ONE: &str = "process";
pub const LX_TASK_COUNT_NOUN_MANY: &str = "processes";

/// 输入框底边框左端的阻塞子 agent 指标：`1 blocked` / `3 blocked`。
pub fn lx_subagent_blocked_label(count: usize) -> String {
    format!("{count} blocked")
}

/// 输入框底边框左端的运行中子 agent 计数指标：`1 subagent` / `3 subagents`。
pub fn lx_subagent_count_label(count: usize) -> String {
    if count == 1 {
        format!("1 {}", LX_SUBAGENT_COUNT_NOUN_ONE)
    } else {
        format!("{count} {}", LX_SUBAGENT_COUNT_NOUN_MANY)
    }
}
pub const LX_SUBAGENT_COUNT_NOUN_ONE: &str = "subagent";
pub const LX_SUBAGENT_COUNT_NOUN_MANY: &str = "subagents";

/// Agents 面板列表行的状态文案（含排队输入数时追加 ` (+Nq)`）。
pub fn agent_status_with_queue(status: &str, queued_count: usize) -> String {
    if queued_count > 0 {
        format!("{status} (+{queued_count}q)")
    } else {
        status.to_string()
    }
}

/// 停止一个后台进程后的提示。
pub const LX_TASK_STOPPED: &str = "Stopped 1 background process";
/// Agents 面板高亮项已结束（^d 不会误停别的子 agent）。
pub const LX_TASK_SELECTED_AGENT_NOT_RUNNING: &str = "Selected subagent is not running";
/// Agents 面板内联追问目标已被关闭（cancelled 不可再接收输入）。
pub const LX_TASK_AGENT_REPLY_CLOSED: &str = "Cannot send input to a closed subagent";
/// Agents 面板停止选中子 agent 的提示。
pub fn stopping_subagents(count: usize) -> String {
    let noun = if count == 1 { "subagent" } else { "subagents" };
    format!("Stopping {count} {noun}")
}
/// 停止失败（进程已退出、运行实例已销毁等）。
pub const LX_TASK_STOP_FAILED: &str = "Failed to stop the process";

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
/// 代码块未写语言时的默认展示语言（`text`）：只作标签，不参与语法高亮。
pub const LX_CHAT_CODE_LANG_DEFAULT: &str = "text";

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
pub const TOAST_OPEN_CANCELLED: &str = "Workspace open cancelled";
pub const TOAST_SYNC_CANCELLED: &str = "Workspace sync cancelled";

/// tooltip 边框标题；当前所有按钮共用一个固定标题。
pub const TOOLTIP_TITLE: &str = "Tooltip";

/// 长文本粘贴行数说明。
pub fn text_paste_lines(count: usize) -> String {
    if count == 1 {
        "1 line".to_string()
    } else {
        format!("{count} lines")
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
        TooltipTarget::WorkspaceMode => match state.workspace_history.mode {
            crate::app::state::WorkspaceMode::Active => "open workspace history",
            crate::app::state::WorkspaceMode::History => "open active workspaces",
        },
        TooltipTarget::AddWorkspace => "new workspace",
        TooltipTarget::AddTab => "new tab",
        TooltipTarget::TabScrollLeft => "scroll tabs left",
        TooltipTarget::TabScrollRight => "scroll tabs right",
        TooltipTarget::Exit => "quit lx-tui",
        TooltipTarget::PromptMode => {
            if state.prompt_owner().prompt_panel.mode == crate::app::prompt::PromptMode::Editor {
                "open prompt list"
            } else {
                "open editor"
            }
        }
        TooltipTarget::PromptTempPrompt => "open temp prompt",
        TooltipTarget::PromptRecent => "recent pages (^r)",
        TooltipTarget::PromptRecentClose => "close recent (esc)",
        TooltipTarget::PromptPagePrev => "previous page (^←)",
        TooltipTarget::PromptPageCounter => {
            if state.prompt_owner().prompt_panel.mode == crate::app::prompt::PromptMode::Pages {
                "open editor"
            } else {
                "open page list (^p)"
            }
        }
        TooltipTarget::PromptPageNext => "next page (^→)",
        TooltipTarget::PromptUndo => "undo (^z)",
        TooltipTarget::PromptRedo => "redo (^y)",
        TooltipTarget::PromptSave => "mark saved",
        TooltipTarget::PromptPin if state.prompt_pinned.is_some() => "unpin prompt",
        TooltipTarget::PromptPin => "pin prompt",
        TooltipTarget::SwapPromptSide => "swap prompt side",
        TooltipTarget::SidebarSettings => "open settings",
        TooltipTarget::PromptTemplateStatus => "cycle block status",
        TooltipTarget::TemplateButton { button, .. } => match button {
            TemplateButtonKind::Copy => "copy block",
            TemplateButtonKind::Clean => "clean block",
            TemplateButtonKind::Delete => "delete block",
        },
        TooltipTarget::PromptFolders => "referenced folders",
        TooltipTarget::TodoButton(button) => match button {
            crate::app::tooltip::TodoButton::New => TODO_TOOLTIP_NEW,
            crate::app::tooltip::TodoButton::Sort => TODO_TOOLTIP_SORT,
        },
        TooltipTarget::ReferencedSearchCopyPath => "copy path",
        TooltipTarget::BlockButton { button, .. } => match button {
            BlockButtonKind::Copy => "copy block",
            BlockButtonKind::Clean => "clean block",
            BlockButtonKind::Delete => "delete block",
        },
    }
}

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

/// 按显示列截断并追加省略号；宽字符不会越界。
pub fn ellipsize_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    format!("{}…", take_prefix_width(text, max_width - 1))
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

#[cfg(test)]
mod tests;
