//! agent 步骤的用户可见文案：lx 聊天历史里**真实工具步骤行**
//! （`ui/main_content/lx/history/tool.rs`）与进程 / 计划卡片共用的图标 / 动词 / 计数标记
//! （同一份常量，多处渲染不得各写一份字符串）。
//!
//! 文案仍集中在 `text` 模块；拆到此子模块只为把 `text.rs` 控制在单文件行数上限内。
//! 本模块只放字符串，行结构与着色由渲染层负责。

/// 步骤图标（全部为单一显示宽度、跨平台可打印字符）。
///
/// 避免 emoji 与东亚宽字符（如 `☰`），防止在部分终端占两列破坏对齐；偏向几何符号。
pub const ICON_READ: &str = "≣";
pub const ICON_WRITE: &str = "+";
pub const ICON_EDIT: &str = "±";
pub const ICON_SEARCH: &str = "∇";
pub const ICON_WEB: &str = "⌘";

/// 直角树体的收口连接符（最后一行）。
pub const MARKER_TREE_LEAF: &str = "└─";
/// 树体中间行的连接符（`│`）：与 [`MARKER_TREE_LEAF`] 的 `└` 同列，连成一条树干。
pub const MARKER_TREE_BAR: &str = "│";
/// 工具卡片树形体（与 prompt list 同一套直角语法）。
///
/// 三条前缀**等宽 5 列**，内容一律从第 5 列起，与文件工具详情行的
/// [`detail prefix`](crate::ui::main_content::lx::history::tool::detail_prefix) 完全对齐：
///
/// ```text
///   ├─ $ cargo test
///   │  test one ... ok
///   └─ test result: ok
/// ```
///
/// - [`TREE_BRANCH`]：一个**节点**的首行，且后面还有节点（`├─`）；
/// - [`TREE_LEAF`]：最后一个节点的首行，收口（`└─`）；
/// - [`TREE_BAR`]：节点内部的续行（同一段输出的后续行），只画竖线。
///
/// `├` 与 `└` 落在同一列，节点首行因此连成一根主干；续行用 `│` 表示
/// 「还在上一个节点里」。三条前缀等宽，正文永远左对齐。
pub const TREE_BAR: &str = "  │  ";
pub const TREE_BRANCH: &str = "  ├─ ";
pub const TREE_LEAF: &str = "  └─ ";
/// 树体之外的同宽空白前缀：正文仍与树体内容左对齐，但不画连接符。
///
/// 命令卡片用它让「输出正文」继续缩进在参数之下的内容列，而不被 `│` / `└─` 包裹。
pub const TREE_PAD: &str = "     ";

/// 步骤动词的固定形式（`Read`）。工具行叙述**这次调用本身**，用现在式。
pub const VERB_READ: &str = "Read";

/// edit / write 标题里的增删行计数（紧跟文件名后，分色、颜色不由可变参数覆盖）。
pub const EDIT_DIFF_ADD: &str = "+12";
pub const EDIT_DIFF_DEL: &str = "-4";
pub const WRITE_ADD: &str = "+18";

// ===== 真实工具步骤行的方块标记 / 动词 / 计数标记 =====
//
// `ui/main_content/lx/history/tool.rs` 渲染真实工具调用时**只读**本组常量：动词与计数
// 标记不允许在渲染层再写一份字符串。行首状态标记自 G11 起改由 `history::StepMark`
// 的方块承担，不再使用按工具区分的类型图标。

/// 聊天历史每个 assistant 步骤左侧的方块标记（思考 / 工具调用 / 正文块共用）：
/// 运行中 `■`/`□` 两帧呼吸，终态停为静态方块；呼吸相位每 [`STEP_BREATH_TICKS`]
/// 个 loading 帧翻转一次（loading 帧 100ms → 约 400ms 一相，是“呼吸”而非闪烁）。
pub const STEP_MARK_FILLED: char = '■';
pub const STEP_MARK_HOLLOW: char = '□';
pub const STEP_BREATH_TICKS: usize = 4;

/// 真实工具步骤行的固定动词（设计 §10：`Read` / `Search` / `Edit` / `Write`）。
///
/// 历史行叙述**这次调用本身**，故用现在式；`read` 与 [`VERB_READ`] 同形，直接复用，
/// 不另写一份字符串。
pub const VERB_TOOL_READ: &str = VERB_READ;
pub const VERB_TOOL_SEARCH: &str = "Search";
pub const VERB_TOOL_EDIT: &str = "Edit";
pub const VERB_TOOL_WRITE: &str = "Write";

/// 工具行 diff 计数的前缀（绿 `+N` / 红 `-N`）。
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

/// 合并卡标题里的文件数：`1 file` / `3 files`。
///
/// 只出现在**连续同种文件调用的合并卡**上：单次调用的标题仍是裸动词，不写 `1 file`
/// ——那会把最常见的单文件改动也说成汇总，白占标题宽度。
pub fn tool_files_label(count: usize) -> String {
    let suffix = if count == 1 { "file" } else { "files" };
    format!("{count} {suffix}")
}

/// markdown 渲染器识别的代码块语言标记（代码块头部 `language: xxx` 的取值）。
pub const MD_LANG: &str = "rust";

/// 一份覆盖全部 markdown 格式的样例文档（供渲染器测试与 `md` 相关用例共用）。
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

// ===== 进程 / 计划卡片文案（`history/tool/command.rs` 与 `plan.rs` 共用）=====

use crate::app::state::{InteractionAction, ProcessStateView};

/// `exec_command` 卡片动词。
pub const VERB_EXEC_COMMAND: &str = "Run command";
/// `write_stdin` 卡片动词（按动作区分）。
pub const VERB_READ_OUTPUT: &str = "Read output";
pub const VERB_SEND_INPUT: &str = "Send input";
pub const VERB_STOP_COMMAND: &str = "Stop command";
/// `update_plan` 卡片动词。
pub const VERB_UPDATE_PLAN: &str = "Plan";

/// 历史内**行内动作文字**（常态次要色、悬停默认前景，见 `style::chat_action_label`）。
///
/// 文件差异卡片的展开/收起入口：折叠态是树体末尾的常驻一行（有省略时 `… more changes`，
/// 没有省略时 `Show full diff`），展开态同一行变成 `collapse`。点击展开、点正文或
/// 这一行收起；正文保留聊天选区，**拖拽选中不折叠**。
pub const FILE_DIFF_OMITTED: &str = "… more changes";
/// 折叠态没有省略时仍可展开：入口文案换成明确的动作。
pub const FILE_DIFF_SHOW_ALL: &str = "Show full diff";
/// 折叠态被省略的行数：`… 421 more lines` / `… 1 more line`。
///
/// 取代原先的 `… more changes`：用户要求「把折叠文案换成剩余行数」——数字直接
/// 回答「点开还能看到多少」，比一句没有量的提示有用。
pub fn file_diff_more_lines(count: usize) -> String {
    let suffix = if count == 1 { "line" } else { "lines" };
    format!("… {count} more {suffix}")
}

/// 展开态树体末尾的收起入口。
pub const FILE_DIFF_COLLAPSE: &str = "collapse";

/// 运行中命令卡片末行的**整卡点击引导**：点击打开后台任务面板看完整实时输出。
///
/// 这是「点整张卡」的提示，不是一行按钮；终态卡片不画这一行，改画省略行数。
pub const TASK_OPEN_GUIDE: &str = "open task panel · live output";

/// 进程状态词（文字，不只靠颜色）。
pub const STATE_STARTING: &str = "Starting";
pub const STATE_RUNNING: &str = "Running";
pub const STATE_STOPPED: &str = "Stopped";
pub const STATE_TIMED_OUT: &str = "Timed out";
pub const STATE_INTERRUPTED: &str = "Interrupted";
pub const STATE_FAILED: &str = "Failed";

/// 树体命令行的提示符前缀（`$ cargo test`）。
pub const COMMAND_PROMPT_PREFIX: &str = "$ ";

/// cwd 次要信息前缀。
pub const CWD_PREFIX: &str = "cwd:";
/// 输入预览前缀。
pub const INPUT_PREFIX: &str = "stdin:";
/// 计划完成计数后缀。
pub const PLAN_COMPLETED_SUFFIX: &str = "completed";
/// 空计划文案。
pub const PLAN_EMPTY: &str = "Plan cleared";

// ===== 文件工具差异卡片 =====

/// 文件没有内容差异（纯移动、覆盖为相同内容、补丁无实际变更）。
pub const FILE_DIFF_NO_CHANGES: &str = "No content changes";
/// 差异正文被存储预算截断（计数仍是完整值，展开也补不回来）。
pub const FILE_DIFF_TRUNCATED: &str = "Diff truncated";
/// 差异正文来自超限输入的线性简化脚本（可能比最小编辑脚本多）。
pub const FILE_DIFF_SIMPLIFIED: &str = "Diff simplified";
/// hunk 之间的分隔（不直接铺 `@@`）。
pub const FILE_DIFF_HUNK_SEP: &str = "⋮";
/// 末行无换行的元信息行（不计入增删）。
pub const FILE_DIFF_NO_FINAL_NEWLINE: &str = "No newline at end of file";
/// 无法生成文本差异（二进制 / 非 UTF-8 / 读取失败 / 结构非法）。
pub const FILE_DIFF_UNAVAILABLE: &str = "Diff unavailable";
/// 移动的两端连接符：`旧 → 新`。
pub const FILE_DIFF_MOVE_SEP: &str = " → ";
/// 计数未知（旧内容不可解码，删除后无从比较）。
pub const FILE_DIFF_COUNTS_UNKNOWN: &str = "(?)";

/// 因整体 JSON 预算被省略摘要的文件数：`3 files omitted` / `1 file omitted`。
pub fn file_diff_omitted_files(count: usize) -> String {
    let suffix = if count == 1 {
        "file omitted"
    } else {
        "files omitted"
    };
    format!("{count} {suffix}")
}

/// 差异不可用的原因（稳定原因码 → 用户可见说明，不暴露内部细节）。
pub fn file_diff_unavailable(reason: &str) -> &'static str {
    match reason {
        "binary" => "Diff unavailable: binary file",
        "not_utf8" => "Diff unavailable: not UTF-8",
        "read_failed" => "Diff unavailable: read failed",
        _ => FILE_DIFF_UNAVAILABLE,
    }
}

/// 计划三态标记（ASCII，与模型结果同一口径）。
pub const PLAN_PENDING: &str = "[ ]";
pub const PLAN_IN_PROGRESS: &str = "[>]";
pub const PLAN_COMPLETED: &str = "[x]";

/// 进程状态 → 固定文案。
pub fn proc_state_label(state: ProcessStateView) -> &'static str {
    match state {
        ProcessStateView::Starting => STATE_STARTING,
        ProcessStateView::Running => STATE_RUNNING,
        ProcessStateView::Exited { code } => {
            if code.is_some_and(|code| code != 0) {
                STATE_FAILED
            } else {
                // 退出码 0 由调用方追加 `Exited 0`，这里只给失败/成功分档。
                STATE_RUNNING
            }
        }
        ProcessStateView::Stopped => STATE_STOPPED,
        ProcessStateView::TimedOut => STATE_TIMED_OUT,
        ProcessStateView::Interrupted => STATE_INTERRUPTED,
        ProcessStateView::Failed => STATE_FAILED,
    }
}

/// 退出状态标签：`Exited 0` / `exit 2`。
pub fn proc_exit_label(state: ProcessStateView) -> String {
    match state {
        ProcessStateView::Exited { code } => match code {
            Some(0) => "Exited 0".to_string(),
            Some(code) => format!("exit {code}"),
            None => "Exited".to_string(),
        },
        other => proc_state_label(other).to_string(),
    }
}

/// `write_stdin` 动作动词。
pub fn interaction_verb(action: InteractionAction) -> &'static str {
    match action {
        InteractionAction::ReadOutput => VERB_READ_OUTPUT,
        InteractionAction::SendInput => VERB_SEND_INPUT,
        InteractionAction::StopCommand => VERB_STOP_COMMAND,
    }
}

/// 计划完成计数：`1/3 completed`。
pub fn plan_progress(completed: usize, total: usize) -> String {
    format!("{completed}/{total} {PLAN_COMPLETED_SUFFIX}")
}

/// 计划步骤标记。
pub fn plan_marker(status: crate::app::state::PlanStepView) -> &'static str {
    match status {
        crate::app::state::PlanStepView::Pending => PLAN_PENDING,
        crate::app::state::PlanStepView::InProgress => PLAN_IN_PROGRESS,
        crate::app::state::PlanStepView::Completed => PLAN_COMPLETED,
    }
}

/// 详情行：`cwd: /path`。
pub fn cwd_label(cwd: &str) -> String {
    format!("{CWD_PREFIX} {cwd}")
}

// ===== `ask` 面板 / 卡片文案（`main_content/lx/ask.rs` 与 `history/tool/ask.rs` 共用）=====

/// `ask` 卡片动词。
pub const VERB_TOOL_ASK: &str = "Ask";
/// 面板标题：`ASK 1/2 · 布局`。
pub const ASK_TITLE: &str = "ASK";
pub const ASK_TITLE_SEP: &str = " · ";
/// 选项标记：单选圆点、多选方框、高亮前缀。
pub const ASK_SINGLE_ON: &str = "(•)";
pub const ASK_SINGLE_OFF: &str = "( )";
pub const ASK_MULTI_ON: &str = "[x]";
pub const ASK_MULTI_OFF: &str = "[ ]";
pub const ASK_CURSOR: &str = "❯";
/// 固定最后一行的「其他」与单题提交入口。
pub const ASK_OTHER: &str = "Other...";
pub const ASK_OTHER_PREFIX: &str = "Other: ";
pub const ASK_SUBMIT: &str = "[ Submit ]";
/// 多题最后一步：复核作答 + 确认提交（tab 与行各一份文案）。
pub const ASK_REVIEW: &str = "Review your answers";
pub const ASK_CONFIRM_TAB: &str = "Confirm";
pub const ASK_CONFIRM: &str = "[ Confirm ]";
/// 预览区占位与降级提示。
pub const ASK_NO_PREVIEW: &str = "(no preview)";
pub const ASK_FOOTER_KEYS: &str = "enter select/next · space toggle · tab switch";
pub const ASK_FOOTER_CANCEL: &str = "esc cancel";
pub const ASK_FOOTER_CANCEL_ARMED: &str = "esc again to cancel";
/// 卡片状态文字：等待作答（运行中）。
pub const ASK_WAITING: &str = "waiting for you";
/// 历史里的空答案（异常数据兜底，不画成空行）。
pub const ASK_NO_ANSWER: &str = "(no answer)";

/// 卡片标题的题数：`2 questions` / `1 question`。
pub fn ask_question_count(count: usize) -> String {
    let suffix = if count == 1 { "question" } else { "questions" };
    format!("{count} {suffix}")
}

/// 面板标题位置：`1/2`。
pub fn ask_position(current: usize, total: usize) -> String {
    format!("{}/{total}", current + 1)
}
