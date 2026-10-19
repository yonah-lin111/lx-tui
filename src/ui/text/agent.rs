//! agent 步骤的用户可见文案：`[UI]` 查看器 `agent` 分组的样例册，以及 lx 聊天历史里
//! **真实工具步骤行**（`ui/main_content/lx/history/tool.rs`）与样例**共用**的图标 / 动词 /
//! 计数标记（同一份常量，两处渲染不得各写一份字符串）。
//!
//! 文案仍集中在 `text` 模块；拆到此子模块只为把 `text.rs` 控制在单文件行数上限内。
//! 结构（图标、状态、token 数值）由 `ui/overlay/ui_gallery/agent.rs` 提供，本模块只放字符串。

/// `agent` 分组文件夹展示名。
pub const FOLDER: &str = "agent";

/// 步骤 item 展示名。
pub const EXEC: &str = "exec";
pub const READ: &str = "read";
pub const EDIT: &str = "edit";
pub const WRITE: &str = "write";
pub const SEARCH: &str = "search";
pub const WEB: &str = "web";
pub const THINKING: &str = "thinking";
pub const SUBAGENT: &str = "subagent";
pub const TODO: &str = "todo";
pub const MCP: &str = "mcp";
pub const COMPACT: &str = "compact";
pub const MODE: &str = "mode";
pub const FINAL: &str = "final";
pub const MARKDOWN: &str = "markdown";
pub const ERROR: &str = "error";
pub const QUESTION: &str = "question";
pub const TOKENS: &str = "tokens";

/// 步骤图标（全部为单一显示宽度、跨平台可打印字符）。
///
/// 避免 emoji 与东亚宽字符（如 `☰`），防止在部分终端占两列破坏对齐；偏向几何符号。
pub const ICON_EXEC: &str = "$";
pub const ICON_READ: &str = "≣";
pub const ICON_WRITE: &str = "+";
pub const ICON_EDIT: &str = "±";
pub const ICON_SEARCH: &str = "∇";
pub const ICON_WEB: &str = "⌘";
pub const ICON_THINKING: &str = "~";
pub const ICON_SUBAGENT: &str = "∞";
pub const ICON_TODO: &str = "≡";
pub const ICON_MCP: &str = "#";
pub const ICON_COMPACT: &str = "-";
pub const ICON_MODE: &str = "⇆";
pub const ICON_FINAL: &str = "●";
pub const ICON_ERROR: &str = "!";
pub const ICON_QUESTION: &str = "?";
pub const ICON_TOKENS: &str = "%";

/// 结构标记（可见符号集中处；均为单宽、跨平台）。
pub const MARKER_DOT: &str = "●";
pub const MARKER_TREE_LEAF: &str = "└─";
/// todo 三态标记（取 claude 一组：完成 / 进行中 / 待办）。
pub const TODO_DONE: &str = "✔";
pub const TODO_IN_PROGRESS: &str = "◼";
pub const TODO_PENDING: &str = "◻";

/// token 页脚的标签与分隔符（对齐 lx-agent 的 `IN · OUT · CACHE`）。
pub const TOKEN_IN: &str = "IN";
pub const TOKEN_OUT: &str = "OUT";
pub const TOKEN_CACHE: &str = "CACHE";
pub const SEPARATOR: &str = " · ";

/// 步骤动词（标题固定部分，品牌天蓝）。
pub const VERB_EXEC: &str = "Ran";
pub const VERB_READ: &str = "Read";
pub const VERB_EDIT: &str = "Edited";
pub const VERB_WRITE: &str = "Wrote";
pub const VERB_SEARCH: &str = "Searched";
pub const VERB_WEB: &str = "Searched web";
pub const VERB_THINKING: &str = "Thought";
pub const VERB_SUBAGENT: &str = "Spawned";
pub const VERB_TODO: &str = "Updated plan";
pub const VERB_MCP: &str = "Called";
pub const VERB_COMPACT: &str = "Compacted";
pub const VERB_MODE: &str = "Switched";
pub const VERB_FINAL: &str = "Replied";
pub const VERB_ERROR: &str = "Failed";
pub const VERB_QUESTION: &str = "Asked";

/// 步骤主参数的各分段文案；渲染层组合成着色分段（见 `ui/overlay/ui_gallery/agent.rs`）。
///
/// 除 read 的行范围（另用 [`crate::ui::style::agent_range`]）与 edit/write 的
/// 增删计数（另用绿/红）外，其余可变参数一律用固定金黄（[`crate::ui::style::agent_param`]）。
pub const PARAM_EXEC_CMD: &str = "git status";
/// read：文件路径 + 行范围（行范围用另色、无下划线）。
pub const PARAM_READ_FILE: &str = "src/ui/style.rs";
pub const PARAM_READ_RANGE: &str = "(lines 1–2)";
pub const PARAM_EDIT_FILE: &str = "src/app/state.rs";
pub const PARAM_WRITE_FILE: &str = "docs/notes.md";
pub const PARAM_SEARCH: &str = "\"fn render\" in src/";
pub const PARAM_WEB_QUERY: &str = "\"ratatui 0.30\"";
/// thinking：这次思考的标题（只显示标题，不展开思考内容）。
pub const PARAM_THINKING_TITLE: &str = "split module then verify";
pub const PARAM_SUBAGENT_AGENT: &str = "explorer";
pub const PARAM_TODO_STEPS: &str = "3 steps";
pub const PARAM_MCP: &str = "filesystem · read_file";
pub const PARAM_COMPACT: &str = "12.4k → 3.1k tokens";
pub const PARAM_MODE: &str = "default → plan";
pub const PARAM_FINAL_SUMMARY: &str = "1 paragraph";
pub const PARAM_ERROR_CMD: &str = "cargo test";
pub const PARAM_QUESTION: &str = "Overwrite the existing file?";

/// edit / write 标题里的增删行计数（紧跟文件名后，分色、颜色不由可变参数覆盖）。
pub const EDIT_DIFF_ADD: &str = "+12";
pub const EDIT_DIFF_DEL: &str = "-4";
pub const WRITE_ADD: &str = "+18";

// ===== 真实工具步骤行（lx 聊天历史）与样例共用的方块标记 / 动词 / 计数标记 =====
//
// `ui/main_content/lx/history/tool.rs` 渲染真实工具调用时**只读**本组常量与上面的
// `VERB_*`：动词与计数标记不允许在渲染层再写一份字符串。行首状态标记自 G11 起改由
// `history::StepMark` 的方块承担，不再使用按工具区分的类型图标。

/// 聊天历史每个 assistant 步骤左侧的方块标记（思考 / 工具调用 / 正文块共用）：
/// 运行中 `■`/`□` 两帧呼吸，终态停为静态方块；呼吸相位每 [`STEP_BREATH_TICKS`]
/// 个 loading 帧翻转一次（loading 帧 100ms → 约 400ms 一相，是“呼吸”而非闪烁）。
pub const STEP_MARK_FILLED: char = '■';
pub const STEP_MARK_HOLLOW: char = '□';
pub const STEP_BREATH_TICKS: usize = 4;

/// 真实工具步骤行的固定动词（设计 §10：`Read` / `Search` / `Edit` / `Write`）。
///
/// gallery 样例叙述已完成的调用（`Edited` / `Wrote` / `Searched`），实时历史行叙述
/// **这次调用本身**，故用同一词根的现在式。`read` 两处同形，直接复用
/// [`VERB_READ`]，不另写一份字符串。
pub const VERB_TOOL_READ: &str = VERB_READ;
pub const VERB_TOOL_SEARCH: &str = "Search";
pub const VERB_TOOL_EDIT: &str = "Edit";
pub const VERB_TOOL_WRITE: &str = "Write";

/// 工具行 diff 计数的前缀（绿 `+N` / 红 `-N`，与样例 `+12` / `-4` 同色）。
pub const TOOL_DIFF_ADD_PREFIX: &str = "+";
pub const TOOL_DIFF_REMOVE_PREFIX: &str = "-";

/// 内容被截断的显式提示（追加在计数末尾，不与计数同色）。
pub const TOOL_TRUNCATED: &str = "…[truncated]";

/// `edit` 部分成功的尾部标记：本次补丁里有 hunk 应用后内容与原文相同（无实际变更）。
pub const TOOL_PARTIAL: &str = "partial";

/// `search` 命中数后缀（单复数分开，避免 `1 matches` 这类错句）。
pub const TOOL_MATCH_ONE: &str = "match";
pub const TOOL_MATCH_MANY: &str = "matches";

/// `read` 行范围：`12-40`（起止均为 1-based 闭区间）。
pub fn range_label(start: usize, end: usize) -> String {
    format!("{start}-{end}")
}

/// `read` 实际读到的行数：`(31 lines)` / `(1 line)`；与区间并列显示在括号里。
pub fn lines_label(count: usize) -> String {
    let suffix = if count == 1 { "line" } else { "lines" };
    format!("({count} {suffix})")
}

/// `search` 的搜索模式：加引号，便于与路径区分（`"测试"`）。
pub fn pattern_label(pattern: &str) -> String {
    format!("\"{pattern}\"")
}

/// `search` 的搜索范围连接词（`"pat" in path` 里的 `in`）。
///
/// 渲染层把它作为**独立的一段**（dim）放在 pattern 与 path 之间，窄宽度时整段让位
/// （review P2-7：三段的着色粒度与 codex 一致）。不要把它拼进 pattern 或 path ——
/// 拼成一段后只能对整串做中间省略，窄宽度会把 pattern 切碎。
pub const IN_SCOPE_SEP: &str = "in ";

/// 命中 / 条目数：`3 matches`、`1 match`。
pub fn matches_label(count: usize) -> String {
    let suffix = if count == 1 {
        TOOL_MATCH_ONE
    } else {
        TOOL_MATCH_MANY
    };
    format!("{count} {suffix}")
}

/// 新增行数：`+12`。
pub fn added_label(count: usize) -> String {
    format!("{TOOL_DIFF_ADD_PREFIX}{count}")
}

/// 删除行数：`-4`。
pub fn removed_label(count: usize) -> String {
    format!("{TOOL_DIFF_REMOVE_PREFIX}{count}")
}

/// 步骤子行（直角树连接下的细节）。
pub const BODY_EXEC: &[&str] = &["On branch dev", "nothing to commit, working tree clean"];
pub const BODY_WEB: &[&str] = &["ratatui 0.30 released", "docs.rs/ratatui/0.30"];
/// subagent 子任务：只显示“进行中”的那一步（一个直角 icon 右侧一条）。
pub const SUBAGENT_IN_PROGRESS: &str = "read src/ui/style.rs";
pub const BODY_MCP: &[&str] = &["filesystem · read_file", "ok · 128 lines"];
pub const BODY_FINAL: &[&str] = &["Added the agent gallery folder and its variants."];
pub const BODY_ERROR: &[&str] = &["exit code 1 · 2 failures"];
pub const BODY_QUESTION: &[&str] = &["Overwrite the existing file?", "[yes] [no]"];

/// `tokens` 聚合 item：回合汇总条与上下文占用 pill 的样例片段。
///
/// 汇总条的 `IN/OUT/CACHE` 明细由渲染层用 `format_tokens` 生成，
/// 这里只放前缀文案。
pub const TOKENS_BAR_LEAD: &str = "3 calls · ";
pub const TOKENS_PILL_LABEL: &str = "context";
/// 上下文 pill 的三个占用阈值样例（绿 / 黄 / 红）与占用百分比。
pub const TOKENS_PILL_GREEN: u8 = 42;
pub const TOKENS_PILL_YELLOW: u8 = 78;
pub const TOKENS_PILL_RED: u8 = 92;

/// 右侧用途说明（逐行 muted 渲染）。
pub const EXEC_DESC: &[&str] = &[
    "Shell command call: verb, command and its output",
    "under a right-angle connector; token footer below.",
];
pub const READ_DESC: &[&str] = &[
    "File / directory read: the path and the line range",
    "read, with the same token footer as every other step.",
];
pub const EDIT_DESC: &[&str] = &[
    "File edit: the path followed by the +/- line counts",
    "in the header, then the step token footer.",
];
pub const WRITE_DESC: &[&str] = &[
    "New or overwritten file: the path followed by the",
    "added line count in the header, then the token footer.",
];
pub const SEARCH_DESC: &[&str] = &[
    "Content / glob search: only the query and scope,",
    "then the step token footer.",
];
pub const WEB_DESC: &[&str] = &[
    "Web search or fetch: query / URL and the result,",
    "then the step token footer.",
];
pub const THINKING_DESC: &[&str] = &[
    "Reasoning step: just the title, no reasoning body,",
    "then the step token footer.",
];
pub const SUBAGENT_DESC: &[&str] = &[
    "Sub-agent (Task) spawn: the single in-progress",
    "step under a right-angle connector, one token total.",
];
pub const TODO_DESC: &[&str] = &[
    "Plan / todo update: checkbox list (done/busy/open),",
    "then the step token footer.",
];
pub const MCP_DESC: &[&str] = &[
    "MCP tool call: server and method, then the result,",
    "with the step token footer.",
];
pub const COMPACT_DESC: &[&str] = &[
    "Context compaction divider: tokens before and after",
    "the compaction.",
];
pub const MODE_DESC: &[&str] = &["Mode / model switch divider: from and to values."];
pub const FINAL_DESC: &[&str] = &[
    "Final assistant reply: the answer text followed by",
    "the step token footer.",
];
pub const ERROR_DESC: &[&str] = &[
    "Failed or interrupted call: failure marker (red)",
    "with the error detail and step token footer.",
];
pub const QUESTION_DESC: &[&str] = &[
    "Ask-the-user step: the question and its options,",
    "then the step token footer.",
];
pub const TOKENS_DESC: &[&str] = &[
    "Token usage: per-step footers plus the turn summary",
    "bar and the context-usage pill.",
];

/// `markdown` item：agent 最终回答的 markdown 渲染说明。
pub const MARKDOWN_DESC: &[&str] = &[
    "Markdown answer rendering: headings, lists, tasks,",
    "quotes, code, tables, rules, images and inline styles.",
];

/// `markdown` item 的渲染器代码块的语言标记（代码块右上角的语言名）。
pub const MD_LANG: &str = "rust";

/// `markdown` item 引用块前缀（`>` 而非左边框）。
pub const MD_QUOTE_MARKER: &str = ">";

/// `markdown` item：一份覆盖全部 markdown 格式的样例文档。
///
/// 用真实 markdown 源码写出，渲染器负责把标记“吃掉”并按主流 coding agent
/// 的做法重排（含代码块的轻量语法高亮）。
pub const MD_SAMPLE: &str = "\
# Heading 1
## Heading 2
### Heading 3
#### Heading 4
##### Heading 5
###### Heading 6
**bold** *italic* ***both*** ~~strike~~ `code`
- bullet one
  - nested bullet
1. ordered one
- [x] done task
- [ ] open task
> quoted line one
```rust
// compute the greeting
fn main() {
    let name: String = \"world\".to_string();
    println!(\"hello, {name} {42}\");
}
```
    indented code
| Left | Center | Right |
|:-----|:------:|------:|
| a    | b      | c     |
---
[OpenAI](https://openai.com) <https://example.com>
![logo](https://example.com/logo.png)
";

/// lx 页 `Native Agent` 标签渐变的**去重色序列**（218 粉 → 182 淡紫 → 153 浅蓝 → 117 天蓝）。
///
/// 输入框上方 loading 的扫描高亮按这 4 档循环换色（`style::chat_loading_scan`），
/// 保证「loading 的色 = Native 入口标签的色」，不引入第二套品牌色。
pub fn native_label_cycle() -> &'static [u8] {
    &[218, 182, 153, 117]
}
