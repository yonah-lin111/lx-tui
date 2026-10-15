//! `agent` 步骤调用样例册的用户可见文案（`[UI]` 查看器 `agent` 分组）。
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
pub const ICON_SEARCH: &str = "⌕";
pub const ICON_WEB: &str = "⌘";
pub const ICON_THINKING: &str = "~";
pub const ICON_SUBAGENT: &str = "∞";
pub const ICON_TODO: &str = "≡";
pub const ICON_MCP: &str = "⇄";
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

/// 步骤主参数（紧跟动词后的细节串）。
pub const DETAIL_EXEC: &str = "git status";
pub const DETAIL_READ: &str = "src/ui/style.rs";
pub const DETAIL_EDIT: &str = "src/app/state.rs";
pub const DETAIL_WRITE: &str = "docs/notes.md";
pub const DETAIL_SEARCH: &str = "\"fn render\" in src/";
pub const DETAIL_WEB: &str = "\"ratatui 0.30\"";
pub const DETAIL_THINKING: &str = "for 2.4s";
pub const DETAIL_SUBAGENT: &str = "explorer";
pub const DETAIL_TODO: &str = "3 steps";
pub const DETAIL_MCP: &str = "filesystem · read_file";
pub const DETAIL_COMPACT: &str = "12.4k → 3.1k tokens";
pub const DETAIL_MODE: &str = "default → plan";
pub const DETAIL_FINAL: &str = "1 paragraph";
pub const DETAIL_ERROR: &str = "cargo test";
pub const DETAIL_QUESTION: &str = "Overwrite the existing file?";

/// 步骤子行（直角树连接下的细节）。
pub const BODY_EXEC: &[&str] = &["On branch dev", "nothing to commit, working tree clean"];
pub const BODY_READ: &[&str] = &[
    "1  //! semantic style tokens",
    "2  pub fn text() -> Style {",
];
/// edit 的 diff 计数行（`+`/`-` 分色）与紧随的正文行。
pub const EDIT_DIFF_ADD: &str = "+12";
pub const EDIT_DIFF_DEL: &str = "-4";
pub const EDIT_DIFF_PATH: &str = "src/app/state.rs";
pub const EDIT_BODY: &[&str] = &["2  pub struct AppState {"];
pub const BODY_WRITE: &[&str] = &["docs/notes.md (+18 lines)"];
pub const BODY_SEARCH: &[&str] = &[
    "src/ui/style.rs:14  pub fn text()",
    "src/ui/style.rs:19  pub fn muted()",
];
pub const BODY_WEB: &[&str] = &["ratatui 0.30 released", "docs.rs/ratatui/0.30"];
pub const BODY_THINKING: &[&str] = &["Plan the module split, then verify with tests."];
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
    "File / directory read: path plus the first lines,",
    "with the same token footer as every other step.",
];
pub const EDIT_DESC: &[&str] = &[
    "File edit: path, +/- line counts and a small diff,",
    "then the step token footer.",
];
pub const WRITE_DESC: &[&str] = &[
    "New or overwritten file: path and added line count,",
    "then the step token footer.",
];
pub const SEARCH_DESC: &[&str] = &[
    "Content / glob search: pattern, scope and matches,",
    "then the step token footer.",
];
pub const WEB_DESC: &[&str] = &[
    "Web search or fetch: query / URL and the result,",
    "then the step token footer.",
];
pub const THINKING_DESC: &[&str] = &[
    "Reasoning step: a muted, collapsed summary line",
    "with the step token footer.",
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

/// `markdown` item 的渲染器每套候选的语言标记（代码块右上角的语言名）。
pub const MD_LANG: &str = "rust";

/// `markdown` item：一份覆盖全部 markdown 格式的样例文档。
///
/// 用真实 markdown 源码写出，渲染器负责把标记“吃掉”并按主流 coding agent
/// 的做法重排；同一份文档喂给四套候选，便于横向对比渲染差异。
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
fn main() {}
```
    indented code
| Left | Center | Right |
|:-----|:------:|------:|
| a    | b      | c     |
---
[OpenAI](https://openai.com) <https://example.com>
![logo](https://example.com/logo.png)
";
