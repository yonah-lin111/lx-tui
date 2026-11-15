# Agents / BTW 详情输入框 `/` 命令与私聊·汇报重做 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-detail-slash-and-report/`
- **关联任务**：[`task.md`](task.md)
- **当前状态**：**已在 `detail-slash-report` 分支实现，自动化与 CLI 实机验证通过；TUI 交互/外观待用户确认后合并（含 2026-11-15 T10 修复）**（见 [`verification.md`](verification.md)）。
- **2026-11-15 调整**：详情命令由 `/undo` 更名为 `/rewind`（`/undo` 保留隐藏别名）；确认面板与主聊天四项动作对齐；新增 compact 检查点撤销（`RestoreCompact`）。
- **2026-11-15 修复**：详情 `/compact` 纳入 `has_streaming_lx` 心跳（耗时与菱形呼吸、聊天栏 `Compacting context... (Ns)`）；后台详情滚动条按下/拖拽上限改用 `ScrollbarLayout` 的 `total`/`visible`，不再读滞后一帧的缓存行数。
- **目标范围**：
  - `src/app/state/lx/{slash.rs,slash/composer.rs,agent.rs,rewind.rs,task.rs,detail.rs,panels.rs}`
  - `src/app/update/chat/{slash.rs,composer.rs,detail_rewind.rs,detail_report.rs,btw/*,compact.rs}`
  - `src/app/update/pane/{task_page.rs,popups.rs}`
  - `src/event/chat/{detail_rewind.rs,detail_compact.rs,compact.rs,btw.rs,btw/maintenance.rs,rewind.rs}`
  - `src/db/chat/subagent/queue.rs`
  - `src/ui/main_content/lx/{slash.rs,panels.rs,task/agents.rs,task/btw.rs,task.rs}`
  - `src/ui/text/detail.rs`（新增）、`src/ui/text.rs`（只加 `pub mod detail;`）
  - `src/chat/agent/report.rs`（报告请求正文）
- **约束**：不新增 SQLite 迁移；不新增顶层模块；超过 1000 行的生产文件不再加逻辑（现状见 §0）。

---

## 0. 代码库现状核验（codegraph / codebase-memory 结果）

| 事实维度 | 文件与行号 | 当前行数 | 与本设计的关系 |
| :--- | :--- | ---: | :--- |
| **汇报正文 = 本轮 assistant 原文** | `src/event/chat/subagent/settle.rs:527-556`（`turn_messages` / `report_body`） | 604 | 目前 `report_body` 把本轮全部 assistant 文本直接拼成报告；闲聊原文就是这样进主 agent 的。 |
| **是否汇报只看 `notify_parent`** | `src/event/chat/subagent/settle.rs:78-109` | 604 | `notify_parent=true` 才生成 `ReportPayload`；本次保留该列/参数，只改“谁来置 true”。 |
| **粘性 report 模式** | `src/app/state/lx/agent.rs:392-421`（`AgentReplyMode`）、`:438`（`reply_modes`）、`:467-476`（`reply_mode` / `toggle_reply_mode`） | 617 | 本次删除该模式与按 agent 保存的状态。 |
| **Shift+Tab 切换模式** | `src/app/update/pane/task_page.rs:254-270`（`apply_agent_edit_input` → `toggle_agent_reply_mode`） | 856 | `Shift+Tab` 不再切模式，改为吞掉。 |
| **追问提交与 notify_parent** | `src/app/update/pane/task_page.rs:680-786`（`submit_agent_edit`） | 856 | 普通追问与 steer 回退一律 `notify_parent=false`；新增 `/report` 专门置 true。 |
| **Agents composer 能力** | `src/app/state/lx.rs:270-276`（`agent_composer`） | 858 | 当前 `allow_panels=false`，`/` 被关闭；本次只开 `/` 与 `@`，其余浮层继续关闭。 |
| **编辑器浮层总闸** | `src/app/state/lx/panels.rs:25-49`（`refresh_panels`） | 485 | `!allow_panels` 分支无条件 `slash=None`，需要按新 `slash_enabled` 放行。 |
| **当前生效编辑器** | `src/app/state/lx.rs:451-464`（`active_editor`）、`:503-516`（`with_active_editor`） | 858 | UI 层 `/` 面板已经用 `active_editor()`；本次把 app 层路由也改为它。 |
| **`/` 命令枚举与过滤** | `src/app/state/lx/slash.rs:71-215`（`SlashEntry`/`SlashCommand`）、`:694-713`（`command_entries_for`） | 981 | 主输入命令集不动；新增详情命令子模块与 `SlashEntry::Detail`。 |
| **`/` 面板状态** | `src/app/state/lx/slash.rs:660-857`（`SlashPanel`） | 981 | 复用 `Commands`；详情撤销确认复用 `Rewind` 变体 + `RewindPanel.detail`。 |
| **行首 `/` 解析** | `src/app/state/lx/slash.rs:906-920`（`slash_trigger`） | 981 | 直接复用；详情命令要求整段草稿独立成命令。 |
| **带参命令提示** | `src/app/state/lx/slash.rs:926-944`（`active_argument_hint`）、`src/ui/main_content/lx/input.rs:173` | 981 | `/report ` 需要灰色 `[note]` 提示。 |
| **`/` 采纳与过滤** | `src/app/update/chat/slash.rs:566-623`（`apply_slash_command`）、`:692-765`（`refresh_slash_panel`）、`:782-825`（`accept_slash_command`） | 903 | 新增详情分支；主路径保持。该文件已 903 行，详情逻辑必须外移。 |
| **`/` 面板 UI** | `src/ui/main_content/lx/slash.rs:21-175`（`items`）、`:236-316`（`view`）、`:324-373`（`layout`/`render`） | 373 | `items` 增加 `Detail` 条目；几何/绘制完全复用。 |
| **输入框矩形（含 composer）** | `src/ui/main_content/lx/panels.rs:88-106`（`input_rect`） | 473 | 已支持 BTW/Agents composer，面板会贴 composer 上沿。 |
| **鼠标 hover/点击/滚轮** | `src/app/update/pane/popups.rs:248-278` | 242 | 当前读 `pane.lx.slash`；改为 `with_active_editor`。 |
| **Agents 胶囊** | `src/ui/main_content/lx/task/agents.rs:275-309`（`draw_agent_badge`） | 528 | 显示 `private` / `steer N` / `compacting`，删除 `report`。 |
| **BTW 胶囊** | `src/ui/main_content/lx/task/btw.rs:161-205`（`draw_qa_badge`） | 292 | 追加 ` · compacting`（宽度不足整段丢弃）。 |
| **Agents 编辑键位提示** | `src/ui/text.rs:699-705`（`LX_TASK_HINTS_AGENTS_EDIT`） | 1015 | 删除 `[⇧⇥ mode]`，新增 `[/ commands]`。 |
| **BTW 编辑键位提示** | `src/ui/text/btw.rs:55-60`（`LX_BTW_HINTS_EDIT`） | 74 | 新增 `[/ commands]`。 |
| **压缩命令（主会话）** | `src/app/update/chat/compact.rs:19-100`（`compact_command`）、`src/event/chat/compact.rs:33-125`（`execute_compact`）、`:284-353`（`handle_compact_finished`） | 323 / 446 | 详情压缩复用同一套摘要请求与 `commit_compact`，只新增目标解析与回投路由。 |
| **压缩状态** | `src/app/state/lx/compact.rs:93-120`（`LxCompact`：`running`/`started_at_ms`/`elapsed_ms`） | 200 | `run.compact` / `thread.compact` 已是 `LxCompact`，可直接复用。 |
| **loading/耗时 tick** | `src/app/update/chat/mod.rs:1019-1099`（`advance_lx_loading`） | >1000 | 目前只刷新主 `lx.compact.elapsed_ms`；需要覆盖 run / btw thread。 |
| **撤销执行入口** | `src/event/chat/rewind.rs:36-42`（`execute_rewind`）、`src/event/chat/rewind/restore.rs:30-125`（`run`） | 429 | 这是同步执行入口，接受任意 `session_id`；详情撤销直接复用它，避免重写文件恢复。 |
| **撤销提交事务** | `src/db/chat/rewind.rs:649+`（`commit_rewind`）、`:245`（`latest_user_turn`）、`src/db/chat/rewind/plan.rs:20`（`rewind_availability`） | 1010+ | 删除对话/恢复文件/会话删空/模型回滚/usage 重算都已处理；详情只负责目标与回投。 |
| **可撤回的人类追问判据** | `src/db/chat/subagent/queue.rs:149-230`（`enqueue_input` 写 `parent_call_id`/`notify_parent`/`position`）、`src/db/migration.rs:288-297`（v29 `notify_parent`） | 460+ / — | 人类面板追问：`position > 0 && parent_call_id IS NULL && notify_parent = 0`；spawn 输入 `position = 0`。 |
| **BTW 线程删除** | `src/db/chat/btw.rs:832-846`（`delete_btw_thread`） | 979 | 已有“线程 + child session”级联删除，`/new` 直接复用。 |
| **BTW 线程加载** | `src/event/chat/btw.rs:258-312`（`load_thread`）、`src/app/update/chat/btw/events.rs`（`on_thread_loaded`） | 1335 / 417 | 需要补 `compact_checkpoints` 回填，否则压缩卡片不显示。 |
| **报告请求正文落点** | `src/chat/agent/report.rs:157-233`（信封/截断） | 270 | 新增 `report_request_message`，与信封格式化同文件。 |
| **超 1000 行的既有文件** | `src/ui/text.rs` 1015、`src/db/chat/subagent.rs` 1050、`src/event/chat/btw.rs` 1335、`src/event/mod.rs` 989 | — | 新逻辑一律放子模块/新文件，**不得**继续膨胀这些文件；`event/mod.rs` 不加新事件变体。 |

> 结论：`notify_parent` 列、`delete_btw_thread`、`commit_rewind`、`execute_rewind`、`LxCompact`、`HistorySource` 的 compact 渲染都已存在。本次**不需要 DB 迁移**，核心是补命令入口、撤销目标解析、详情压缩回投与模式删除。

---

## 1. 目标与非目标

### 1.1 目标

1. **Agents composer 支持 `/` 命令面板**：`/rewind`、`/compact`、`/report`；`/undo` 为 `/rewind` 的隐藏别名。
2. **BTW composer 支持 `/` 命令面板**：`/rewind`、`/compact`、`/new`；同样保留 `/undo` 别名。
3. **删除粘性 `private` / `report` 模式**：默认永远私聊；只有显式 `/report [note]` 才生成一轮面向主 agent 的汇报。
4. **汇报内容由子 agent 专门生成**：`/report` 注入英文汇报指令（独立一轮、`notify_parent=true`），报告正文只取这一轮的 assistant 输出，不再转发闲聊原文。
5. **详情 `/rewind` 语义与主聊天 `/rewind` 对齐**：
   - 取最新时间线事件：普通轮次或 compact 检查点；
   - 普通轮次提供与主聊天相同的四项动作（恢复文件并撤销对话 / 只撤销对话 / 只恢复文件 / 取消），默认第一项；
   - compact 检查点只回退上下文指针（恢复与取消两项），不碰消息与文件；
   - Agents 只允许撤回**人类私聊追问**；spawn 初始任务轮、`/report` 轮、父 agent `send_input` 轮与含 steer 的轮次不可撤回；
   - 只允许空闲、无排队输入时执行；文件快照不可用时含文件动作禁用，仍可只撤销对话。
6. **`/compact` 支持详情会话**：手动压缩子 agent / BTW 线程上下文，复用现有摘要请求、检查点与卡片渲染。
7. **BTW `/new`**：删除当前 QA 的旁路线程（DB 级联）并从空白开始。
8. 所有用户可见文案英文；底边框提示同步更新。

### 1.2 非目标

- 不加 `/model`、`/status`、`/help`（`/` 面板本身就是命令清单）。
- 不改主输入框的命令集与 `/rewind` 行为。
- 不改主 agent 的模型驱动报告（`spawn_agent` / `send_input` 完成后照旧自动报告 + 自动轮）。
- 不改 DB schema / 不新增迁移。
- 不改 `agent_subagent_input` 表结构；`notify_parent` 继续作为报告开关。
- 不新增顶层 `panels/` / `commands/` 模块；全部落在既有职责目录。

---

## 2. Grill Me 已确认决策

| 编号 | 决策 | 用户确认结论 |
| :--- | :--- | :--- |
| **Q1** | report 模式转发什么 | 删除粘性模式，改一次性 `/report [note]`；由子 agent 专门生成汇报正文，默认永远私聊。 |
| **Q2** | 子 agent 撤回范围 | 与主聊天 `/rewind` 一致：最新轮次或 compact 检查点；Agents 只认人类私聊追问，BTW 认最新问答；空闲才可执行；spawn / 报告 / 父 `send_input` 轮不可撤回。 |
| **Q3** | 命令集合 | Agents：`/rewind` `/compact` `/report`；BTW：`/rewind` `/compact` `/new`。`/undo` 作为 `/rewind` 隐藏别名；不加 `/model` `/status`。 |
| **Q4** | 确认策略 | `/rewind` 与主聊天相同的四项动作面板（默认第一项）；compact 只有恢复与取消；`/new`、`/compact` 直接执行。 |
| **Q5** | `/report` 是否自动起轮 | 保持自动起轮；复用现有报告批次与 `drain_reports`，不做“只投递不起轮”。 |

> 本设计取代 `docs/feature/lx-agent-private-followup/design.md` §2/§3 中的 `AgentReplyMode` 交互；该文档保留为历史记录。

---

## 3. 交互总览

### 3.1 Agents composer（空闲）

```text
╭─ lx-[Native Agent] ──────────────────────────────────────────╮
│ Turing: 登录超时根因在 src/auth.rs，建议把超时改成 30s。        │
│                                                              │
│ ╭ Commands ────────────────────────────────────────────────╮  │
│ │ ❯ /rewind            Rewind the last turn or a compact …   │  │
│ │   /compact         Compact this conversation's context    │  │
│ │   /report          Report progress to the parent agent    │  │
│ ╰ ↑↓ select · ↵ open · esc close ───────────────────────────╯  │
├─ Processes · Agents · BTW ────────────────────────────────────┤
│ Subagent  /re…                              Turing [executor] private │
╰─ [↵ send] [⇧↵ newline] [/ commands] [^d stop] [esc back] ─────╯
```

### 3.2 `/rewind` 动作面板（与主聊天一致，默认第一项）

```text
│ ╭ Rewind ───────────────────────────── last private turn ───╮ │
│ │ ❯ Restore files and undo conversation                      │ │
│ │   Undo conversation only                                   │ │
│ │   Restore files only                                       │ │
│ │   Cancel                                                   │ │
│ ╰ ↑↓ select · ↵ confirm · esc cancel ───────────────────────╯ │
```

- 默认高亮第一项，与主聊天 `/rewind` 相同；`Enter` 直接执行高亮项。
- 文件快照不可用/不完整时，两个含文件动作照常展示但不可执行（右侧显示原因）；`Undo conversation only` 仍可用。
- compact 检查点比最新轮次更晚时，面板只显示 `Restore the context before this checkpoint` 与 `Cancel`。

### 3.3 BTW composer（编辑态）

```text
├─ Processes · Agents · BTW ────────────────────────────────────┤
│ BTW  /ne…                                          QA 2 · 标题 │
╰─ [↵ send] [⇧↵ newline] [/ commands] [^d stop] [esc back] ─────╯
```

`/new` 回车直接清空该 QA 的旁路线程（有活动回答时拒绝）。

---

## 4. 详细设计（含关键代码）

### 4.1 详情命令模型（新增 `src/app/state/lx/slash/composer.rs`）

`src/app/state/lx/slash.rs` 已 981 行，新枚举/解析必须放子模块，`slash.rs` 只加声明与 `SlashEntry` 变体：

```rust
// src/app/state/lx/slash.rs（仅新增）
pub mod composer;
pub use composer::DetailCommand;

pub enum SlashEntry {
    Command(SlashCommand),
    /// 详情输入框（Agents / BTW composer）的上下文命令。
    Detail(composer::DetailCommand),
}
```

`src/app/state/lx.rs` 的 `pub use slash::{...}` 追加 `DetailCommand`，供 UI 层匹配标签/描述。

```rust
// src/app/state/lx/slash/composer.rs（新增）
use super::{SlashEntry, alias_prefix_score, command_score};

/// 详情输入框里的命令；只在 composer 上下文出现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailCommand {
    Undo,
    Compact,
    Report,
    New,
}

/// 两类详情 composer；决定候选集合与执行语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailContext {
    Agent,
    Btw,
}

impl DetailCommand {
    pub const AGENT: [Self; 3] = [Self::Rewind, Self::Compact, Self::Report];
    pub const BTW: [Self; 3] = [Self::Rewind, Self::Compact, Self::New];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Rewind => "rewind",
            Self::Compact => "compact",
            Self::Report => "report",
            Self::New => "new",
        }
    }

    pub const fn text(self) -> &'static str {
        match self {
            Self::Rewind => "/rewind",
            Self::Compact => "/compact",
            Self::Report => "/report",
            Self::New => "/new",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Rewind => &["undo"],
            Self::New => &["clear", "reset"],
            _ => &[],
        }
    }

    /// 文案保持上下文中立：同一句在两个 composer 都成立。
    pub const fn description(self) -> &'static str {
        match self {
            Self::Rewind => "Rewind the last turn or a compact checkpoint",
            Self::Compact => "Compact this conversation's context",
            Self::Report => "Report progress to the parent agent",
            Self::New => "Start a fresh side thread",
        }
    }

    pub const fn echo_prefix(self) -> Option<&'static str> {
        match self {
            Self::Report => Some("/report "),
            _ => None,
        }
    }

    pub const fn argument_hint(self) -> Option<&'static str> {
        match self {
            Self::Report => Some("[note]"),
            _ => None,
        }
    }
}

/// 按上下文生成候选（命令名用模糊匹配，别名只用精确/前缀，与主命令一致）。
pub fn entries(context: DetailContext, query: &str) -> Vec<SlashEntry> {
    let query = query.trim_start_matches('/').to_lowercase();
    let all = match context {
        DetailContext::Agent => DetailCommand::AGENT.as_slice(),
        DetailContext::Btw => DetailCommand::BTW.as_slice(),
    };
    let mut scored: Vec<(u32, usize, DetailCommand)> = all
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            let name = command_score(command.name(), &query);
            let alias = command
                .aliases()
                .iter()
                .filter_map(|alias| alias_prefix_score(alias, &query).map(|s| s.saturating_sub(1)))
                .max();
            name.into_iter()
                .chain(alias)
                .max()
                .map(|score| (score, index, *command))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .map(|(_, _, command)| SlashEntry::Detail(command))
        .collect()
}
```

`command_score` 目前是 `pub(crate)`，同模块子模块可直接用；不要把 `entries` 塞回 `slash.rs`。

### 4.2 草稿解析（整段独立命令）

主输入 `/undo` 是 `/rewind` 的别名，因此详情解析**不能**复用 `SlashCommand` 的名称匹配，必须单独解析：

```rust
/// 解析结果：`Report` 携带已 trim 的备注。
pub enum ParsedDetailCommand<'a> {
    Rewind,
    Compact,
    Report(&'a str),
    New,
}

/// 整段草稿必须是一条详情命令：首个非空白字符是 `/`，命令词后只允许参数。
/// `/report` 的备注允许换行；其余命令不允许任何参数。
pub fn parse(input: &str, context: DetailContext) -> Option<ParsedDetailCommand<'_>> {
    let trimmed = input.trim_start_matches([' ', '\t']);
    let line = trimmed.strip_prefix('/')?;
    let (word, rest) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(word, rest)| (word, rest));
    let command = match (context, word) {
        (_, "rewind") | (_, "undo") => DetailCommand::Rewind,
        (_, "compact") => DetailCommand::Compact,
        (DetailContext::Agent, "report") => DetailCommand::Report,
        (DetailContext::Btw, "new") | (DetailContext::Btw, "clear")
        | (DetailContext::Btw, "reset") => DetailCommand::New,
        _ => return None,
    };
    // 同一行必须结束在命令词之后；`/rewind x` 不是命令，交给未知命令提示。
    if command != DetailCommand::Report && !rest.trim().is_empty() {
        return None;
    }
    if command != DetailCommand::Report && rest.contains('\n') {
        return None;
    }
    Some(match command {
        DetailCommand::Rewind => ParsedDetailCommand::Rewind,
        DetailCommand::Compact => ParsedDetailCommand::Compact,
        DetailCommand::Report => ParsedDetailCommand::Report(rest.trim()),
        DetailCommand::New => ParsedDetailCommand::New,
    })
}
```

> 为什么整段：和 `/rewind` 一样，命令不能夹在长草稿中悄悄执行；`/report` 的 note 本身就是整段剩余文本。

`active_argument_hint` 增加一行委托：

```rust
// src/app/state/lx/slash.rs
if let Some(hint) = composer::argument_hint(input) {
    return Some(hint);
}
```

```rust
// slash/composer.rs
pub fn argument_hint(input: &str) -> Option<&'static str> {
    let trimmed = input.trim_start_matches([' ', '\t']);
    let line = trimmed.strip_prefix('/')?;
    if line.starts_with("report")
        && line
            .strip_prefix("report")
            .is_some_and(|rest| rest.starts_with(' ') && rest.trim().is_empty())
    {
        return DetailCommand::Report.argument_hint();
    }
    None
}
```

### 4.3 composer 的 `/` 面板生命周期

#### 4.3.1 打开能力

`Lx` 增加一个显式开关，避免破坏“Agents composer 只有 `@`”的既有边界：

```rust
// src/app/state/lx.rs
pub struct Lx {
    // ...
    allow_panels: bool,
    /// 是否允许 `/` 命令面板（Agents composer 单独打开；其余浮层仍关闭）。
    slash_enabled: bool,
}

pub fn agent_composer() -> Self {
    let mut lx = Self::build(false);
    lx.no_shell = true;
    lx.allow_panels = false;
    lx.mention_minimal = true;
    lx.slash_enabled = true;   // 新增
    lx
}

// build() 内默认 `slash_enabled: true`；只有 agent_composer 需要显式确认。
```

```rust
// src/app/state/lx/panels.rs（!allow_panels 分支）
if !self.allow_panels {
    self.panel = None;
    self.paste = None;
    self.text_paste = None;
    self.refer = None;
    if !self.slash_enabled {
        self.slash = None;
    }
    if self.slash.is_some() {
        self.mention.suppress();
    } else if self.mention_minimal {
        // 原有的最小 @ 刷新
    }
    self.scroll_cursor_into_view();
    return;
}
```

普通分支（BTW composer / 主输入）增加统一压制：

```rust
// 任意 `/` 面板打开时，@/md/refer 不得抢按键（原先只覆盖二级面板）。
if self.slash.is_some() {
    self.panel = None;
    self.refer = None;
    self.mention.suppress();
    self.scroll_cursor_into_view();
    return;
}
```

#### 4.3.2 刷新与路由

新增 `src/app/update/chat/composer.rs`，主 `slash.rs` 只做分派：

```rust
// src/app/update/chat/slash.rs
pub(crate) fn refresh_slash_panel(state: &mut AppState, pane: PaneId) {
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.agents_editing() || target.lx.btw_editing())
    {
        super::composer::refresh_slash_panel(state, pane);
        return;
    }
    // ... 以下完全保留现有主输入逻辑
}
```

```rust
// src/app/update/chat/composer.rs
pub(crate) fn context(lx: &Lx) -> Option<DetailContext> {
    if lx.agents_editing() {
        Some(DetailContext::Agent)
    } else if lx.btw_editing() {
        Some(DetailContext::Btw)
    } else {
        None
    }
}

pub(crate) fn refresh_slash_panel(state: &mut AppState, pane: PaneId) {
    let Some((context, query)) = state.pane_anywhere(pane).and_then(|target| {
        let context = context(&target.lx)?;
        let editor = target.lx.active_editor();
        let masked = crate::app::paste::mask_pasted_text(&editor.input);
        slash_trigger(&masked, editor.cursor).map(|query| (context, query))
    }) else {
        // 离开命令位置：关闭详情命令候选，但不要动撤销确认面板。
        close_command_candidates(state, pane);
        return;
    };
    let matching = slash::composer::entries(context, &query);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    target.lx.with_active_editor(|editor| {
        // 高亮保持规则与主命令面板一致：查询词不变则保留。
        let (window, prev) = match &editor.slash {
            Some(SlashPanel::Commands { window, query, .. }) => (*window, Some(query.clone())),
            _ => (PanelWindow::default(), None),
        };
        let mut window = if prev.as_deref() == Some(query.as_str()) {
            window
        } else {
            PanelWindow::default()
        };
        window.clamp(matching.len());
        editor.slash = Some(SlashPanel::Commands {
            items: matching,
            window,
            query,
        });
    });
}
```

调用点：

| 场景 | 位置 | 动作 |
| :--- | :--- | :--- |
| Agents composer 编辑 | `task_page.rs::edit_agent_composer` | `apply_editor_edit` 后若 revision 变化，调用 `refresh_slash_panel` |
| BTW composer 编辑 | `btw/input.rs::edit_composer` | 同上 |
| composer 粘贴 | `task_page.rs::apply_task_paste`（Agents 编辑分支） | 插入后调用 `refresh_slash_panel` |
| 进入/退出编辑 | `enter_task_agent_edit` / `leave_agent_edit` / `enter_btw_edit` / `leave_btw_edit` | 清空 composer 的 `slash` 与 `TaskPage.detail_rewind_pending` |
| 切 tab / 关面板 / 打开 agent 或 btw 页 | `cycle_task_tab` / `close_task_page` / `open_agent_page` / `open_btw_tab` | 清空 composer 的 `slash` 与 `detail_rewind_pending` |
| 会话切换 | `clear_agents` / BTW 复位 | composer 随状态销毁 |

#### 4.3.3 按键路由

Agents 编辑态在 `@` 之前处理 `/`：

```rust
// src/app/update/pane/task_page.rs
fn apply_agent_edit_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    if super::super::chat::composer::apply_slash_input(state, pane, command) {
        return true;
    }
    if apply_agent_mention_input(state, pane, command) {
        return true;
    }
    match command {
        LxInput::Enter => submit_agent_edit(state, pane),
        // 删除 `LxInput::PrevStep => toggle_agent_reply_mode(...)`
        LxInput::Tab | LxInput::PrevStep => true,
        // ...
    }
}
```

BTW 编辑态同样在 `panel_open` 之前（或作为 `panel_open` 的第一个分支）：

```rust
// src/app/update/chat/btw/input.rs
if super::super::composer::apply_slash_input(state, pane, command) {
    return true;
}
```

`apply_slash_input` 只消费：`Up` / `Down` / `Tab` / `Enter` / `Escape`（面板打开时）；其余键回落给编辑：

```rust
pub(crate) fn apply_slash_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(panel) = state.pane_anywhere(pane).and_then(|target| {
        context(&target.lx)?;
        target.lx.active_editor().slash.clone()
    }) else {
        return false;
    };
    match command {
        LxInput::Up | LxInput::Down => {
            let delta = if command == LxInput::Up { -1 } else { 1 };
            move_panel(state, pane, delta);
            true
        }
        LxInput::Tab | LxInput::Enter => {
            accept(state, pane);
            true
        }
        LxInput::Escape => {
            close(state, pane);
            true
        }
        _ => false,
    }
}
```

`close(state, pane)` 同时清空 active editor 的 `slash` 与 `TaskPage.detail_rewind_pending`，避免确认计划在面板关闭后残留。

`Enter` 的执行顺序（`submit_agent_edit` / `submit_current` 开头）：

1. 若 composer 的 `slash` 是 `Commands` / 撤销确认 → 交给 `composer::apply_slash_input`（已在按键入口处理，Enter 不会走到发送）。
2. 否则若 `composer::parse(draft, context)` 命中命令 → 执行并消费草稿。
3. 否则若草稿 trim 后以 `/` 开头 → `unknown_command` 提示，不发送。
4. 否则正常发送。

#### 4.3.4 鼠标

`pane/popups.rs` 的三个入口改为 active editor：

```rust
pub fn hover_lx_slash(state: &mut AppState, id: PaneId, index: usize) -> bool {
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx
            .with_active_editor(|lx| lx.slash.as_mut().is_some_and(|p| p.set_active(index)))
    })
}

pub fn select_lx_slash(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let valid = state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx.with_active_editor(|lx| {
            lx.slash.as_mut().is_some_and(|panel| {
                if index >= panel.item_count() {
                    return false;
                }
                let _ = panel.set_active(index);
                true
            })
        })
    });
    valid && super::super::chat::apply_slash_enter(state, id)
}
```

（`apply_slash_enter` 也改为读 `active_editor().slash`；它统一负责主面板、详情命令、详情撤销确认三种分支。）

#### 4.3.5 采纳入口的 active editor 化

以下函数当前都读 `target.lx.slash` / `target.lx.input` / `target.lx.cursor`，必须统一改为 active editor：

| 函数 | 改法 |
| :--- | :--- |
| `app/update/chat/slash.rs::apply_slash_enter` | `let panel = target.lx.active_editor().slash.clone();` |
| `app/update/chat/slash.rs::accept_slash_command` | `match entry { SlashEntry::Command(cmd) => 主路径, SlashEntry::Detail(cmd) => composer::execute(...) }`；输入/光标也读 active editor |
| `app/update/chat/slash.rs::refresh_slash_filter` | 编辑态没有二级面板，直接 `return` |
| `app/update/pane/popups.rs::{hover,select,scroll}_lx_slash` | `with_active_editor` |
| `app/update/chat/composer.rs::close_command_candidates` | 只关 `Commands`，保留 `Rewind` 确认面板 |

主输入的 `SlashEntry::Command` 分支行为不变；`Detail` 分支只在 composer 上下文出现。

#### 4.3.6 命令分派（`app/update/chat/composer.rs`）

| 命令 | 上下文 | 动作 |
| :--- | :--- | :--- |
| `/rewind` | Agent / Btw | `detail_rewind::prepare`（清命令 → `Effect::PrepareDetailRewind` → 确认面板） |
| `/compact` | Agent / Btw | 清命令 → `Effect::CompactDetail { pane, target }`；空闲检查在事件层 |
| `/report` | Agent | `detail_report::submit`（清命令 → `SendSubagentUserInput { notify_parent: true }`） |
| `/new` | Btw | `btw::request_reset_thread`（运行中拒绝 → `BtwEffect::DeleteThread`） |

分派入口：

```rust
pub(crate) fn execute(
    state: &mut AppState,
    pane: PaneId,
    command: crate::app::state::lx::slash::composer::ParsedDetailCommand<'_>,
) -> bool {
    match command {
        ParsedDetailCommand::Rewind => detail_rewind::prepare(state, pane),
        ParsedDetailCommand::Compact => detail_compact::request(state, pane),
        ParsedDetailCommand::Report(note) => detail_report::submit(state, pane, note),
        ParsedDetailCommand::New => super::btw::request_reset_thread(state, pane),
    }
}
```

`Enter` 的发送路径（Agents `submit_agent_edit` / BTW `submit_current`）必须在“正常发送”之前调用：

```rust
let draft = target.lx.active_editor().input.clone();
if let Some(command) = slash::composer::parse(&draft, context) {
    clear_composer(state, pane);
    return composer::execute(state, pane, command);
}
if draft.trim_start().starts_with('/') {
    // 与主输入同一口径：只拦「整段是一行、无空格、未知命令」的草稿；
    // `/path/to/file 说明` 这类带空格文本照常发送。
    if let Some(command) = slash::unknown_slash_command(&draft) {
        show_lx_hint(..., text::chat::unknown_command(&command));
        return true;
    }
}
```

### 4.4 `/report`：显式汇报

#### 4.4.1 报告请求正文（`src/chat/agent/report.rs`）

```rust
/// `/report` 请求消息的固定首行（也是私聊/汇报的可见分界）。
pub const REPORT_REQUEST_MARKER: &str = "[Report to parent agent]";

/// 人类在详情面板显式要求汇报时注入的用户消息。
///
/// 指令只约束“写什么”，不把闲聊原文转发；报告正文仍由本轮 assistant 输出生成。
pub fn report_request_message(note: &str) -> String {
    let note = note.trim();
    let mut out = String::from(
        "[Report to parent agent]\n\
Write a report for the parent agent. Include only what it needs to act: the user's request, \
what you did or found, evidence (files, commands, results), current status, blockers, and \
next steps. Do not include greetings, small talk, or casual conversation. Be concise and factual.",
    );
    if !note.is_empty() {
        out.push_str("\nUser note: ");
        out.push_str(note);
    }
    out
}
```

> 该文本只作为一条普通 user 消息入队（`notify_parent=true`），不新增消息 kind、不改 DB。

`src/chat/agent/mod.rs` 的 `pub use report::{...}` 追加 `report_request_message` / `REPORT_REQUEST_MARKER`。

#### 4.4.2 App 侧提交

```rust
// src/app/update/chat/detail_report.rs（新增）
pub fn submit(state: &mut AppState, pane: PaneId, note: &str) -> bool {
    let Some((agent_id, running, session_id)) = state.pane_anywhere(pane).and_then(|target| {
        let page = target.lx.task.as_ref()?;
        let agent_id = page.agent_edit.clone()?;
        let run = target.lx.agents.by_agent(&agent_id)?;
        Some((
            agent_id,
            run.is_running(),
            target.lx.chat.session_id.clone(),
        ))
    }) else {
        return false;
    };
    if running {
        show_lx_toast(state, pane, toast(text::LX_DETAIL_REPORT_BUSY));
        return true;
    }
    let Some(session_id) = session_id else {
        return false;
    };
    let message = crate::chat::agent::report_request_message(note);
    if message.len() > crate::db::chat::MAX_MESSAGE_BYTES {
        show_lx_toast(state, pane, toast(text::LX_REPORT_NOTE_LIMIT));
        return true;
    }
    state.effects.push(Effect::SendSubagentUserInput {
        pane,
        session_id,
        agent_id,
        text: message,
        notify_parent: true,
    });
    clear_composer(state, pane);  // 先清命令文本，避免残留
    crate::app::update::close_detail_slash(state, pane);
    true
}
```

- 事件路径完全复用 `event/chat/subagent/panel.rs::send_from_panel` → `snapshot::send_user_input_from_panel`。
- 报告结算复用 `settle.rs::report_body`（本轮 assistant 文本）+ `drain_reports`（自动轮）。
- 父 agent 侧无新逻辑。

#### 4.4.3 普通追问与 steer

```rust
// submit_agent_edit
let notify_parent = false;                  // 普通追问永远私聊
...
Effect::SteerSubagent { ..., notify_parent: false }   // steer 回退也永远私聊
```

删除 `reply_mode` 读取；保留 `Effect` 的 `notify_parent` 参数（`/report` 使用 true）。

**报告轮不得污染父可见任务摘要**：`visible_task` 当前在 `notify_parent=true` 时用本轮输入覆盖 `run.task`/`handle.task`，而 `/report` 的输入是汇报指令。给 `visible_task` 加一条稳定判据：

```rust
// src/app/state/lx/agent.rs
use crate::chat::agent::REPORT_REQUEST_MARKER;

pub(crate) fn visible_task(
    task_text: &str,
    notify_parent: bool,
    previous: Option<&str>,
    title: &str,
) -> String {
    // 显式 /report 的输入是给子 agent 的指令，不是父可见任务；
    // 保留原任务/标题，避免 <subagents> 与 @ 引用块出现汇报指令全文。
    let is_report_request = task_text
        .lines()
        .next()
        .map(str::trim)
        == Some(REPORT_REQUEST_MARKER);
    if notify_parent && !is_report_request {
        return task_text.to_string();
    }
    previous
        .map(str::trim)
        .filter(|task| !task.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title.to_string())
}
```

报告生成本身仍由本轮 `notify_parent=true` 驱动，不受影响。

### 4.5 `/rewind`（Agents / BTW）：最新轮次或 compact 检查点

#### 4.5.0 共享类型（`src/app/state/lx/detail.rs`）

```rust
/// 详情会话的稳定指向：Agent 用 agent_id，BTW 用 thread_id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailTarget {
    Agent(String),
    Btw(String),
}

/// 撤销确认面板只区分文案，不携带执行数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailRewindKind {
    Agent,
    Btw,
}

/// 预检成功后冻结的撤销计划；确认后原样转成 RewindRequest。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRewindPlan {
    pub target: DetailTarget,
    pub workspace: std::path::PathBuf,
    pub child_session: String,
    /// 普通轮次为用户消息 ID；compact 为检查点 ID。
    pub user_message_id: String,
    pub expected_revision: u64,
    /// 确认后若草稿仍等于 input_at_open，则回填这段原文。
    pub input_at_open: String,
    pub user_text: String,
    /// 用户确认的动作（打开面板时先填默认项）。
    pub action: crate::app::state::RewindAction,
    pub snapshot: crate::app::state::SnapshotState,
    /// Turn 或 Compact：决定面板动作集与执行路径。
    pub kind: crate::app::state::RewindTargetKind,
}

/// 执行中的幂等键；TaskPage 持有它用于迟到回执过滤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRewindExecution {
    pub operation_id: String,
    pub target: DetailTarget,
}
```

`TaskPage` 增加：

```rust
pub detail_rewind_pending: Option<DetailRewindPlan>,
pub detail_rewind_executing: Option<DetailRewindExecution>,
```

#### 4.5.1 可撤回判据（`src/db/chat/subagent/queue.rs` 新增）

`notify_parent` 不能单独作为判据：模型 `send_input` 也是 true。人类面板追问的三元组是：

- `agent_subagent_input.position > 0`（spawn 输入固定 position 0）；
- `parent_call_id IS NULL`（模型工具调用必有 call_id）；
- `notify_parent = 0`（`/report` 轮是 1）。

```rust
/// 最后一轮是否可被人类面板撤回；返回该轮 user 消息、revision 与原文。
pub fn latest_undoable_private_turn(
    &self,
    workspace: &Path,
    child_session: &str,
) -> Result<Option<crate::db::chat::LatestTurn>> {
    let Some(latest) = self.latest_user_turn(workspace, child_session)? else {
        return Ok(None);
    };
    // 含轮内 steer 的轮次不可撤回：steer 会新增同 turn_index 的 user 消息，
    // 而文件 scope / mutation 只锚在该轮第一条 user 消息上；按最新 steer 消息
    // 走 commit_rewind 会丢失文件恢复证据。安全限制，不硬凑。
    let users: i64 = self.conn.query_row(
        r#"SELECT COUNT(*) FROM agent_message
           WHERE session_id = ?1 AND turn_index = ?2
             AND role = 'user' AND kind = 'chat'"#,
        params![child_session, latest.turn_index],
        |row| row.get(0),
    )?;
    if users != 1 {
        return Ok(None);
    }
    let row: Option<(i64, Option<String>, i64)> = self
        .conn
        .query_row(
            r#"SELECT i.position, i.parent_call_id, i.notify_parent
               FROM agent_subagent_input i
               WHERE i.child_user_message_id = ?1
               ORDER BY i.position DESC
               LIMIT 1"#,
            params![latest.user_message_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let undoable = row.is_some_and(|(position, call_id, notify_parent)| {
        position > 0 && call_id.as_deref().is_none_or(str::is_empty) && notify_parent == 0
    });
    Ok(undoable.then_some(latest))
}
```

> `latest_user_turn` 已经做工作区归属校验、取 revision 与首段文本；不要重写。新函数放 `queue.rs`，不要放已 1050 行的 `subagent.rs`。
>
> 旧库（v29 之前的追问）`notify_parent` 默认 1，会被判为不可撤回；这是安全默认，不回填历史行。

#### 4.5.2 准备（无 IO 的 app 层 + 同步预检的事件层）

App 侧只发效应：

```rust
// src/app/update/chat/detail_rewind.rs
pub fn prepare(state: &mut AppState, pane: PaneId) -> bool {
    let Some(agent_id) = state.pane_anywhere(pane).and_then(|t| t.lx.agent_edit_id().map(str::to_string))
    else {
        return false;
    };
    // 命令文本先消费；若执行成功，撤回原文只在草稿仍为空时回填。
    clear_composer(state, pane);
    let input_at_open = String::new();
    state.effects.push(Effect::PrepareDetailRewind {
        pane,
        target: DetailTarget::Agent(agent_id),
        input_at_open,
    });
    true
}
```

事件层 `src/event/chat/detail_rewind.rs`：

```rust
pub(crate) fn prepare(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    target: DetailTarget,
    input_at_open: String,
) -> bool {
    let Some(workspace) = workspace_for_pane(state, pane) else { return false };
    let Some(db) = persistence.database() else { return false };
    // 解析稳定目标与子会话；忙碌只提示，不打开面板。
    let (target, child_session) = match &target {
        DetailTarget::Agent(agent_id) => {
            let record = resolve_owned_subagent(...);
            if agent_busy(state, pane, agent_id) { toast(BUSY); return true; }
            (DetailTarget::Agent(agent_id.clone()), record.child_session_id)
        }
        DetailTarget::Btw(thread_id) => {
            let thread = db.btw_thread(&workspace, thread_id)...;
            if btw_busy(state, pane, thread_id) { toast(BUSY); return true; }
            (DetailTarget::Btw(thread_id.clone()), thread.child_session_id)
        }
    };
    // 最新时间线事件：检查点比最新用户轮更晚时撤销 compact。
    let latest = db.latest_user_turn(&workspace, &child_session).ok().flatten();
    let checkpoint = db.compact_checkpoints(&workspace, &child_session).ok()
        .and_then(|mut list| list.pop())
        .filter(|cp| latest.as_ref().is_none_or(|turn| cp.sequence > turn.sequence));
    if let Some(checkpoint) = checkpoint {
        let revision = db.session_revision(&child_session)...;
        return on_prepared(DetailRewindPlan {
            user_message_id: checkpoint.id,
            expected_revision: revision as u64,
            user_text: String::new(),
            action: RewindAction::RestoreCompact,
            snapshot: SnapshotState::KnownEmpty,
            kind: RewindTargetKind::Compact,
            ...
        });
    }
    // 轮次：Agents 只认人类私聊追问，BTW 只认线程最新问答。
    let turn = match &target {
        DetailTarget::Agent(_) => db.latest_undoable_private_turn(...),
        DetailTarget::Btw(_) => latest,
    }.ok().flatten() else { toast(NOTHING); return true };
    let (availability, revision, user_text) = persistence.rewind_availability(
        &workspace, &child_session, &turn.user_message_id,
    )...;
    on_prepared(DetailRewindPlan {
        user_message_id: turn.user_message_id,
        expected_revision: revision as u64,
        user_text,
        action: RewindAction::RevertAndDelete,
        snapshot: SnapshotState::from_db(&availability),
        kind: RewindTargetKind::Turn,
        ...
    })
}
```

文件快照 `Unavailable` / `Incomplete` 不再拒绝预检：计划照常冻结，面板按 `SnapshotState` 禁用含文件动作。

`agent_busy` / `btw_busy` 判据：

- Agents：`run.is_running() || run.queued_count > 0 || run.chat.pending_steers` 非空，或 `run.compact.is_running()`；
- BTW：`thread.is_loading()`（含 `pending_submit` / `pending_recipients` / 流中）或 `thread.compact.is_running()`。

#### 4.5.3 确认面板

`RewindPanel` 增加一个可选的详情描述：

```rust
// src/app/state/lx/rewind.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRewindPanel {
    pub kind: DetailRewindKind, // Agent | Btw（只影响范围文案）
}

pub struct RewindPanel {
    pub window: PanelWindow,
    pub snapshot: SnapshotState,
    pub actions: Vec<RewindAction>,
    /// Some 表示详情 composer 的撤销确认；执行路由据此走详情链路。
    pub detail: Option<DetailRewindPanel>,
}

/// 轮次：与主 `/rewind` 相同的四项动作，默认第一项。
pub fn for_detail(kind: DetailRewindKind, snapshot: SnapshotState) -> Self {
    Self {
        window: PanelWindow::new(0),
        snapshot,
        actions: RewindAction::ALL.to_vec(),
        detail: Some(DetailRewindPanel { kind }),
    }
}

/// compact：只有恢复上下文与取消。
pub fn for_detail_compact(kind: DetailRewindKind) -> Self {
    Self {
        window: PanelWindow::new(0),
        snapshot: SnapshotState::KnownEmpty,
        actions: vec![RewindAction::RestoreCompact, RewindAction::Cancel],
        detail: Some(DetailRewindPanel { kind }),
    }
}
```

其余构造器（`new` / `for_compact` / `for_model_switch` / `for_recap`）保持 `detail: None`。确认时先检查 `panel.enabled(action)`，再按 `plan.kind` 校验动作：`Compact` 只接受 `RestoreCompact`，`Turn` 只接受三项轮次动作；`Cancel` 只清计划。

UI 增加 `RewindScope::Detail` 分支：

```rust
// src/ui/main_content/lx/slash.rs
SlashPanel::Rewind(panel) => (
    if panel.scope() == RewindScope::Detail {
        text::LX_DETAIL_REWIND_PANEL_TITLE   // "Undo"
    } else {
        text::LX_REWIND_PANEL_TITLE
    },
    panel.window,
    if panel.scope() == RewindScope::Detail {
        text::LX_DETAIL_REWIND_FOOTER        // "↑↓ select · ↵ confirm · esc cancel"
    } else {
        text::LX_REWIND_PANEL_FOOTER
    },
),
```

右端范围文案：`RewindScope::Detail => text::LX_DETAIL_REWIND_SCOPE`（Agents 显示 `last private turn`，BTW 显示 `last side turn`；可在 UI 层按 `DetailRewindKind` 选）。

确认执行（`src/app/update/chat/detail_rewind.rs`，由 `apply_slash_enter` 分流）：

```rust
// app/update/chat/slash.rs
SlashPanel::Rewind(panel) if panel.detail.is_some() => {
    super::detail_rewind::apply_confirm(state, pane, &panel)
}
SlashPanel::Rewind(_) => super::rewind::apply_enter(state, pane),
```

```rust
pub fn apply_confirm(
    state: &mut AppState,
    pane: PaneId,
    panel: &crate::app::state::RewindPanel,
) -> bool {
    let Some(plan) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_rewind_pending.clone())
    else {
        close_panel(state, pane);
        return true;
    };
    // 面板打开后目标可能被切走（切 agent / QA）：过期计划直接作废。
    if !target_still_active(state, pane, &plan.target) {
        clear_pending(state, pane);
        close_panel(state, pane);
        return true;
    }
    match panel.active_action() {
        crate::app::state::RewindAction::Cancel => {
            clear_pending(state, pane);
            close_panel(state, pane);
        }
        action if Some(action) == panel.detail.as_ref().map(|detail| detail.action) => {
            let operation_id = crate::db::new_external_id("detail-undo");
            let request = crate::app::effects::RewindRequest {
                pane,
                session_id: plan.child_session.clone(),
                target_user_message_id: plan.user_message_id.clone(),
                expected_revision: plan.expected_revision,
                action,
                workspace: plan.workspace.clone(),
                input_at_open: plan.input_at_open.clone(),
                user_text: plan.user_text.clone(),
                operation_id: operation_id.clone(),
            };
            state.effects.push(crate::app::effects::Effect::ExecuteDetailRewind(
                DetailRewindRequest {
                    pane,
                    target: plan.target.clone(),
                    operation_id: operation_id.clone(),
                    request,
                },
            ));
            if let Some(page) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
            {
                page.detail_rewind_executing = Some(crate::app::state::DetailRewindExecution {
                    operation_id,
                    target: plan.target,
                });
                page.detail_rewind_pending = None;
            }
            close_panel(state, pane);
        }
        _ => {}
    }
    true
}
```

#### 4.5.4 执行

`Effect::PrepareDetailRewind` / `Effect::ExecuteDetailRewind` 都挂进现有 `event/chat/rewind.rs::apply_rewind_effects`（事件循环调用点不变）：

```rust
// filter / retain 增加两个变体
Effect::PrepareDetailRewind { pane, target, input_at_open } => {
    detail_rewind::prepare(state, persistence, pane, target, input_at_open)
}
Effect::ExecuteDetailRewind(request) => detail_rewind::execute(sender, persistence, request),
```

`Effect::ExecuteDetailRewind` 的轮次分支与主撤销共用 `execute_rewind`；compact 分支调用 `revert_compact`：

```rust
// src/app/effects.rs（与 RewindRequest 同文件，Effect 变体直接携带）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRewindRequest {
    pub pane: PaneId,
    pub target: crate::app::state::DetailTarget,
    pub operation_id: String,
    pub request: RewindRequest,
}

pub(crate) fn execute(
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: DetailRewindRequest,
) {
    let Some(path) = persistence.database_path() else { return };
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = match crate::db::Database::open(&path) {
            // compact 只回退上下文指针：不碰消息、文件与撤销提示。
            Ok(database) if request.request.action == RewindAction::RestoreCompact => {
                let revert = CompactRevert {
                    operation_id: request.request.operation_id.clone(),
                    session_id: request.request.session_id.clone(),
                    checkpoint_id: request.request.target_user_message_id.clone(),
                    expected_revision: request.request.expected_revision as i64,
                };
                database.revert_compact(&request.request.workspace, &revert)
                    .map(|_| RewindOutcome::Succeeded {
                        action: RewindAction::RestoreCompact,
                        restored_files: 0,
                        already_restored: false,
                        user_text: String::new(),
                        deleted: false,
                        session_deleted: false,
                    })
                    .unwrap_or_else(|error| RewindOutcome::Failed { message: error.code().to_string() })
            }
            Ok(database) => crate::event::chat::rewind::execute_rewind(
                &database,
                &request.request.workspace,
                &request.request,
            ),
            Err(error) => RewindOutcome::Failed { message: error.code().to_string() },
        };
        let _ = sender.send(AppEvent::RewindFinished {
            pane: request.pane,
            session_id: request.request.session_id.clone(),
            workspace: request.request.workspace.clone(),
            operation_id: request.operation_id,
            outcome, // 详情执行在 handle 入口按 pending op 分流
        });
    });
}
```

**不加新的 `AppEvent` 变体**（`src/event/mod.rs` 已 989 行）。`handle_rewind_finished` 开头分流：

```rust
pub fn handle_rewind_finished(..., operation_id: &str, outcome: RewindOutcome) -> bool {
    if state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_rewind_executing.as_ref())
        .is_some_and(|exec| exec.operation_id == operation_id)
    {
        return crate::event::chat::detail_rewind::handle_finished(
            state, persistence, pane, session_id, workspace, operation_id, outcome,
        );
    }
    // ... 现有主聊天逻辑
}
```

#### 4.5.5 成功回投

```rust
pub(crate) fn handle_finished(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    child_session: &str,
    workspace: &Path,
    operation_id: &str,
    outcome: RewindOutcome,
) -> bool {
    let Some(plan) = clear_executing(state, pane, operation_id) else {
        return false; // 迟到回执
    };
    match outcome {
        RewindOutcome::Failed { message } => {
            toast(state, pane, format!("{} ({message})", text::LX_DETAIL_REWIND_FAILED));
            true
        }
        RewindOutcome::Succeeded { user_text, session_deleted, .. } => {
            let messages = persistence
                .database()
                .and_then(|db| db.load_chat_session(workspace, child_session).ok().flatten())
                .unwrap_or_default();
            let checkpoints = persistence
                .database()
                .and_then(|db| db.compact_checkpoints(workspace, child_session).ok())
                .unwrap_or_default();
            crate::app::update::on_detail_rewind_succeeded(
                state, pane, &plan, &messages, &checkpoints, session_deleted, &user_text,
            )
        }
    }
}
```

App 侧回填（`src/app/update/chat/detail_rewind.rs`）：

- Agents：按 `agent_id` 找 run →
  - `run.chat.messages = messages.iter().map(|m| view_message_of(m, &models)).collect()`
  - `run.chat.rebuild_steered_user_messages(messages)`
  - `run.chat.set_project_root(project_root)`（与 `on_subagent_loaded` 同口径）
  - `run.loaded = true; run.compact.replace(checkpoints.into_iter().map(CompactNoticeView::from_record).collect());`
  - `run.rows_dirty = true; run.follow = true; run.scroll = usize::MAX;`
- BTW：按 `thread_id` 找 thread →
  - `session_deleted` → `thread.reset_after_delete()`：`thread_id=None`、`child_session_id=None`、`chat=LxChat::new()`、`compact.reset()`、`targets.clear()`、`loaded=true`、`pending_* = 0`；并 push `LoadQaList` 刷新列表。
  - 否则同 Agents 的 messages/checkpoints/scroll 回填。
- 草稿回填：只有 `plan.action.fills_draft()`（两种删对话动作）且当前 composer 文本仍等于 `plan.input_at_open`（空串）时，写入 `user_text`；否则保留用户新输入（与主 `/rewind` 的 `input_at_open` 规则一致）。
- 关闭 composer 的 `slash`，按动作 Toast：`Conversation undone; restored files.` / `Conversation undone; files kept.` / `Restored files; conversation kept.`；compact 用 `Context restored to the previous checkpoint.`。
- `commit_rewind` 写入的 `agent_session_notice` 在子 agent/BTW 详情不渲染（现有 `HistorySource` 不带 notices）；用户反馈统一走 Toast，避免为详情再造一套提示行。

### 4.6 `/rewind`（BTW）：最新问答或 compact 检查点

- 目标：线程子会话内最新的用户轮次（BTW 只有人类问答，无需私聊判据）或最新 compact 检查点，按 sequence 取更新者。
- 轮次：与 Agents 相同的四项动作面板，快照可用性由 `rewind_availability` 计算；文件动作按快照状态启用/禁用。
- compact：只显示恢复上下文与取消，执行 `revert_compact`。
- 若轮次删空（`session_deleted`）：等同于该 QA 线程被重置；UI 按 §4.5.5 的 reset 处理。
- 不新增事件与迁移：仍复用 `RewindFinished`，compact 分支由 `RewindAction::RestoreCompact` 区分。

### 4.6.1 压缩心跳与滚动几何（2026-11-15 修复）

- 详情压缩是**静默等待**：`Lx::any_detail_compacting()` 必须进入 `AppState::has_streaming_lx`，事件循环才会每 100ms 调 `advance_lx_loading`；`tick_detail_compacting` 同时推进 `run.chat.loading_frame`（BTW 为线程帧），消息列表末尾的 `◆/◇ Compacting context... · Ns` 才会呼吸与计时。
- 聊天栏：`chat_bar::task_loading` 在列表态与编辑态都返回压缩行（当前展示的 run / 当前 BTW 线程），并带本次耗时（`(Ns)`）；其余任务 loading 仍只在编辑态显示。
- 滚动条：`set_task_agents_scroll` / `set_task_btw_scroll` 的 `total`/`visible` 由 `handle_*_scrollbar_press` / drag 传入同一份 `ScrollbarLayout`，与绘制同源；不能用 `run.total_rows` / `thread.total_rows` 这些“上一帧缓存”，否则压缩/内容变化后的首次拖拽会被旧上限钳回底部。

### 4.7 `/compact`：详情会话压缩

#### 4.7.1 效应与目标

```rust
// src/app/effects.rs
CompactDetail {
    pane: crate::tiling::PaneId,
    target: crate::app::state::DetailTarget, // Agent(agent_id) / Btw(thread_id)
},
```

挂进现有 `event/chat/compact.rs::apply_compact_effects`（事件循环调用点不变）：

```rust
Effect::CompactDetail { pane, target } => {
    detail_compact::prepare(state, sender, persistence, pane, target)
}
```

App 侧命令入口（`app/update/chat/detail_compact.rs`）：

```rust
pub fn request(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = detail_target(state, pane) else {
        return false;
    };
    clear_composer(state, pane);
    state.effects.push(Effect::CompactDetail { pane, target });
    true
}
```

`detail_target`：Agents → `DetailTarget::Agent(agent_edit_id)`；BTW → `DetailTarget::Btw(active_thread.thread_id)`；无法定位时返回 None。

`on_detail_compact_started` 置 `run.compact.start(op, now)` / `thread.compact.start(op, now)`，并关闭 composer 的 `/` 面板；找不到目标返回 false（效应被丢弃）。

#### 4.7.2 事件层准备（`src/event/chat/detail_compact.rs` 新增）

```rust
pub(crate) fn prepare(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    pane: PaneId,
    target: DetailTarget,
) -> bool {
    let Some(workspace) = workspace_for_pane(state, pane) else { return false };
    let Some(db) = persistence.database() else { return false };
    let child_session = match &target {
        DetailTarget::Agent(agent_id) => {
            let Some(main) = state.pane_anywhere(pane).and_then(|t| t.lx.chat.session_id.clone())
            else { return false };
            let Some(record) = db.resolve_owned_subagent(&workspace, &main, agent_id)
                .ok().flatten() else { return false };
            if agent_busy(state, agent_id) {
                toast(state, pane, text::LX_COMPACT_BUSY);
                return true;
            }
            record.child_session_id
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = db.btw_thread(&workspace, thread_id).ok().flatten()
            else { return false };
            if btw_busy(state, &thread.id) {
                toast(state, pane, text::LX_COMPACT_BUSY);
                return true;
            }
            thread.child_session_id
        }
    };
    // 模型：优先 child session 持久化的模型（subagent spawn 模型）；BTW 会话列通常
    // 为 NULL，回退当前窗格有效模型，再回退配置默认。摘要请求用稳定模型，
    // 避免中途换模型导致同一会话的压缩结果不可复现。
    let selection = db
        .chat_session_model(&workspace, &child_session)
        .ok()
        .flatten()
        .map(|model| {
            state.models.normalize_selection(crate::config::ai::ModelSelection {
                provider: model.provider,
                model: model.model,
                variant: model.variant,
            })
        })
        .or_else(|| {
            state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.effective_selection(&state.models))
        })
        .or_else(|| state.models.default_selection());
    let Some(selection) = selection else {
        toast(state, pane, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        toast(state, pane, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Ok(revision) = db.session_revision(&child_session) else { return false };
    let base_checkpoint_id = db.active_checkpoint(&workspace, &child_session).ok().flatten().map(|c| c.id);
    let operation_id = crate::db::new_external_id("detail-compact");
    let model_label = state.models.model_display_name(&frozen.provider, &frozen.model);
    if !crate::app::update::on_detail_compact_started(state, pane, &target, operation_id.clone()) {
        return false;
    }
    crate::event::chat::compact::execute_compact(
        sender,
        persistence,
        CompactRequest {
            pane,
            workspace,
            session_id: child_session,
            expected_revision: u64::try_from(revision).unwrap_or(0),
            base_checkpoint_id,
            model: crate::db::chat::ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
            model_label,
            transport: frozen.transport.clone(),
            base_url: frozen.base_url.clone(),
            api_key: frozen.api_key.clone(),
            variant_params: frozen.variant_params.clone(),
            idle_timeout_ms: frozen.idle_timeout_ms,
            operation_id,
        },
    );
    true
}
```

`execute_compact` 目前是私有 fn，改为 `pub(super)`（同 `event::chat` 内使用）。

#### 4.7.3 完成回投

`handle_compact_finished` 开头优先尝试详情：

```rust
pub fn handle_compact_finished(...) -> bool {
    if crate::event::chat::detail_compact::handle_finished(
        state, persistence, pane, session_id, workspace, operation_id, &outcome,
    ) {
        return true;
    }
    // ... 现有主聊天逻辑
}
```

`detail_compact::handle_finished`：

- 在 `pane.lx.agents.runs` 中按 `child_session_id` 找 run；或在 `pane.lx.btw` 中按 child session 找 thread（新增 `LxBtw::thread_by_child_session_mut`）。
- 只认 `compact.running == Some(operation_id)`，否则返回 false（迟到回执）。
- `Committed`：读 `compact_checkpoints` + revision → `run.compact.replace(...)` / `thread.compact.replace(...)`，`finish()`，`rows_dirty=true`，`follow=true`，Toast `LX_COMPACT_DONE`。
- `Empty`：`finish()`，Toast `LX_COMPACT_EMPTY`。
- `Failed`：`finish()`，Toast `format!("{} ({message})", LX_COMPACT_FAILED)`。
- 不触碰主会话的 checkpoints / revision。

#### 4.7.4 BTW 线程检查点加载

`BtwLoadedView` 增加 `checkpoints: Vec<CompactCheckpoint>`；`load_thread` 里：

```rust
let checkpoints = database
    .compact_checkpoints(&workspace, &thread.child_session_id)
    .unwrap_or_default();
```

`on_thread_loaded`：`thread.compact.replace(checkpoints.iter().map(CompactNoticeView::from_record).collect())`。

#### 4.7.5 进度与徽标

`advance_lx_loading` 只加最小接线（该文件已超 1000 行，细节放 `app/state/lx/detail.rs`）：

```rust
let detail_compacting = target.lx.any_detail_compacting();
if !streaming && !compacting && !recapping && !agents_running && !btw_running && !detail_compacting {
    continue;
}
// ...
if detail_compacting {
    target.lx.tick_detail_compacting(crate::db::now_ms());
    advanced = true;
}
```

```rust
// src/app/state/lx/detail.rs
impl Lx {
    pub fn any_detail_compacting(&self) -> bool {
        self.agents.runs.iter().any(|run| run.compact.is_running())
            || self.btw.any_compacting()
    }

    pub fn tick_detail_compacting(&mut self, now_ms: i64) {
        for run in &mut self.agents.runs {
            if run.compact.is_running() {
                run.compact.elapsed_ms =
                    run.compact.started_at_ms.map(|started| now_ms.saturating_sub(started).max(0));
            }
        }
        self.btw.tick_compacting(now_ms);
    }
}
```

徽标：

```rust
let mode = if run.is_running() {
    match run.chat.pending_steers.len() {
        0 => "steer".to_string(),
        n => format!("steer {n}"),
    }
} else if run.compact.is_running() {
    "compacting".to_string()
} else {
    "private".to_string()
};
```

BTW 胶囊在标题后追加 ` · compacting`（`thread.compact.is_running()`；宽度不足整段丢弃，不截半截）。

### 4.8 删除 `AgentReplyMode`

| 位置 | 动作 |
| :--- | :--- |
| `src/app/state/lx/agent.rs:392-421` | 删除 `AgentReplyMode` |
| `:438` | 删除 `pub reply_modes: BTreeMap<...>` |
| `:467-476` | 删除 `reply_mode` / `toggle_reply_mode` |
| `src/app/update/pane/task_page.rs:254-270` | 删除 `PrevStep => toggle_agent_reply_mode`，改为 `PrevStep => true` |
| `:680-786` | 普通追问 `notify_parent=false`；steer 回退 `notify_parent=false` |
| `src/ui/main_content/lx/task/agents.rs:275-309` | 胶囊改为 `private` / `steer` / `compacting` |
| `src/ui/text.rs:699-705` | `[⇧⇥ mode]` → `[/ commands]` |
| 测试 | `shift_tab_switches_agent_reply_mode...` 删除/替换；胶囊断言更新 |

保留：`Effect::{SendSubagentUserInput,SteerSubagent}.notify_parent`、DB `notify_parent`、`settle.rs` 报告生成。

### 4.9 BTW `/new`

- 解析命中后：

```rust
// src/app/update/chat/composer.rs
crate::app::update::chat::btw::request_reset_thread(state, pane)
```

```rust
// src/app/update/chat/btw/mod.rs
pub fn request_reset_thread(state: &mut AppState, pane: PaneId) -> bool {
    let Some((session_id, qa_id, thread)) = ... else { return false };
    if thread.is_loading() {
        show_lx_toast(..., LX_BTW_RESET_BUSY);
        return true;
    }
    let Some(thread_id) = thread.thread_id.clone() else {
        show_lx_toast(..., LX_BTW_RESET_EMPTY);
        return true;
    };
    clear_composer(state, pane);
    state.effects.push(Effect::Btw(BtwEffect::DeleteThread {
        pane, parent_session_id: session_id, qa_id, thread_id,
    }));
    true
}
```

- 事件层（新增 `src/event/chat/btw/maintenance.rs`，`btw.rs` 已 1335 行不再膨胀）：

```rust
pub(super) fn delete_thread(
    state: &mut AppState,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    parent_session_id: &str,
    qa_id: &str,
    thread_id: &str,
) -> bool {
    let Some(db) = persistence.database() else { return false };
    let Some(workspace) = workspace_for_pane(state, pane) else { return false };
    match db.delete_btw_thread(&workspace, thread_id) {
        Ok(()) => {
            let _ = sender.send(AppEvent::Btw(BtwEvent::ThreadDeleted {
                pane, parent_session_id: parent_session_id.to_string(), qa_id: qa_id.to_string(),
            }));
            true
        }
        Err(error) => {
            tracing::warn!(%error, "btw thread delete failed");
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane, qa_id: Some(qa_id.to_string()),
                message: crate::ui::text::LX_BTW_RESET_FAILED.to_string(),
            }));
            false
        }
    }
}
```

- App 侧 `on_thread_deleted`：`btw.reset_thread(qa_id)`，push `LoadQaList` 刷新 `has_thread`，Toast `LX_BTW_RESET_DONE`。
- 删除前必须无活动回答（`is_loading()` 拒绝）。

### 4.10 文案（新增 `src/ui/text/detail.rs`）

```rust
//! 详情输入框 `/` 命令、撤销确认与汇报/压缩反馈（英文）。

pub const LX_DETAIL_REWIND_PANEL_TITLE: &str = "Rewind";
pub const LX_DETAIL_REWIND_SCOPE: &str = "last private turn";
pub const LX_DETAIL_REWIND_SCOPE_BTW: &str = "last side turn";
pub const LX_DETAIL_REWIND_FOOTER: &str = "↑↓ select · ↵ confirm · esc cancel";
pub const LX_DETAIL_REWIND_FAILED: &str = "Rewind failed";
pub const LX_DETAIL_REWIND_BUSY: &str = "Wait for the current turn to finish";
pub const LX_DETAIL_REWIND_NOTHING: &str = "Nothing to rewind";
pub const LX_DETAIL_REWIND_UNAVAILABLE: &str = "Cannot rewind this turn";

pub const LX_DETAIL_REPORT_BUSY: &str = "Wait for the current turn to finish";
pub const LX_REPORT_NOTE_LIMIT: &str = "Report note is too long";

pub const LX_BTW_RESET_DONE: &str = "Side thread cleared";
pub const LX_BTW_RESET_FAILED: &str = "Could not clear the side thread";
pub const LX_BTW_RESET_BUSY: &str = "Wait for the answer to finish";
pub const LX_BTW_RESET_EMPTY: &str = "No side thread to clear";
```

`src/ui/text.rs` 只加一行 `pub mod detail;`（不要继续膨胀 1015 行的文件）。

---

## 5. 数据、并发与失败模式

### 5.1 无迁移

- 不新增表/列/索引；`notify_parent`（v29）继续服务“这一轮是否报告”。
- `/rewind` 复用 `agent_turn_file_scope` / `agent_file_mutation` / `agent_rewind_operation`；`/new` 复用 `delete_btw_thread`。
- 被撤销轮次的 `agent_subagent_input` 行保留为审计记录：状态已 `completed`，不会再被 claim；`child_turn_inputs` 以消息 join 取上下文，消息删除后自然不再回放。

### 5.2 并发与迟到回执

| 场景 | 处理 |
| :--- | :--- |
| `/rewind` 执行中重复 `/rewind` | `TaskPage.detail_rewind_executing` 已占用 → 拒绝并 Toast |
| 撤销回执迟到/换窗格 | `operation_id` 对不上 → 忽略 |
| 撤销目标不再是最新轮 | `commit_rewind` 返回 `RewindTargetChanged` → 失败 Toast，不删任何东西 |
| 含轮内 steer 的轮次 | 刻意不可撤回：同 turn_index 会新增 user 消息，文件 scope 只锚首条；`/rewind` 按 `Nothing to rewind` 拒绝 |
| 撤销期间父 agent / 其他 agent 改了同一文件 | 复用 `restore_plan` 的 `actual_after_image_id` 检查；不覆盖后续修改（与主 `/rewind` 同规则） |
| 撤销执行中应用崩溃 | 复用 `event::chat::rewind::recover_startup`；`unfinished_operation` 会阻止该会话再次撤销，直到恢复完成 |
| `/compact` 期间子轮启动 | `expected_revision` 不匹配 → 提交失败，上下文不变 |
| compact 回执迟到/换会话 | `compact.running != Some(op)` → 忽略 |
| BTW `/new` 期间有回答 | `is_loading()` 前置拒绝；删除失败 Toast 且线程保留 |
| 切换 tab / 关闭面板 | 关闭 composer 的 `/` 面板；不改 DB |

### 5.3 失败语义

- `/rewind` 预检失败（快照 Unavailable/Incomplete）→ 不弹面板、不删对话。
- `/rewind` 执行失败 → 文件恢复幂等回滚/保留，DB 事务未提交；Toast。
- `/compact` 失败 → 不写检查点、不改上下文；Toast。
- `/report` 队列失败 → 事件层 Toast（现有 `show_panel_toast`），草稿已消费（与普通追问一致）。
- `/new` 删除失败 → 线程保留，Toast。

---

## 6. 测试计划

### 6.1 状态与解析

- `src/app/state/lx/slash/composer/tests.rs`
  - `agent_context_lists_rewind_compact_report_only`
  - `btw_context_lists_rewind_compact_new_only`
  - `report_takes_multiline_note`
  - `non_report_commands_reject_arguments`
  - `btw_new_accepts_clear_and_reset_aliases`
  - `argument_hint_only_for_report_prefix`
  - `main_context_does_not_expose_detail_commands`（主输入 `/undo` 仍由 `/rewind` 别名命中）

### 6.2 composer 面板与按键

- `src/app/update/chat/tests/detail_commands.rs`
  - `typing_slash_in_agent_composer_opens_three_commands`
  - `typing_slash_in_btw_composer_opens_three_commands`
  - `enter_accepts_report_and_echoes_argument_hint`
  - `escape_closes_panel_and_keeps_edit_mode`
  - `slash_panel_is_closed_when_leaving_edit_or_switching_tab`
  - `unknown_slash_is_not_sent_to_model`
  - `paste_into_composer_refreshes_slash_panel`

### 6.3 汇报

- `src/app/update/chat/tests/detail_commands.rs`
  - `report_idle_agent_emits_send_input_with_notify_parent_true`
  - `report_running_agent_is_rejected_without_effect`
  - `private_follow_up_never_emits_report`
  - `report_message_contains_marker_and_note`
- `src/event/chat/tests/subagent_private.rs`
  - `report_turn_body_excludes_previous_private_chat`（先私聊多轮，再 `/report`，报告只含报告轮正文）
  - `report_turn_keeps_the_parent_visible_task`（`visible_task` 不把汇报指令写进 `run.task`）
  - `private_follow_up_does_not_create_pending_report`（保留现有并补充“闲聊不触发”）

### 6.4 撤销

- `src/db/chat/subagent/tests/rewind.rs`（新增子模块文件）
  - `latest_undoable_turn_requires_human_panel_follow_up`
  - `spawn_input_and_parent_send_input_and_report_turns_are_not_undoable`
  - `steered_turn_is_not_undoable`
- `src/event/chat/tests/detail_rewind.rs`（新增）
  - `detail_rewind_deletes_the_last_follow_up_and_reloads_the_run`
  - `detail_rewind_disables_file_actions_when_snapshot_is_unavailable`
  - `detail_rewind_ignores_late_operation_id`
  - `detail_rewind_rejects_busy_agent_and_btw_thread`
  - `detail_rewind_restores_the_latest_compact_checkpoint`
  - `btw_undo_deletes_the_last_turn_and_keeps_the_thread`
  - `btw_undo_of_the_only_turn_resets_the_thread`
- `src/app/update/chat/tests/detail_commands.rs`
  - `rewind_command_and_hidden_alias_emit_prepare_effect`
  - `rewind_prepare_shows_four_actions_with_restore_first`
  - `rewind_prepare_for_compact_shows_restore_and_cancel`
  - `rewind_confirm_cancel_has_no_effect`
  - `rewind_confirm_default_enter_emits_execute_effect`
  - `rewind_confirm_disabled_file_action_is_swallowed`
  - `rewind_busy_agent_does_not_open_confirm`

### 6.5 压缩

- `src/event/chat/tests/detail_compact.rs`（新增）
  - `compact_detail_commits_checkpoint_for_child_session`
  - `compact_detail_empty_reports_nothing_to_compact`
  - `compact_detail_busy_agent_is_rejected`
  - `btw_compact_persists_checkpoint_and_reloads_thread`
- `src/app/update/chat/tests/detail_commands.rs`
  - `compact_command_emits_detail_effect_once`
  - `detail_compact_finished_replaces_checkpoints_and_clears_running`
  - `late_detail_compact_finished_is_ignored`
- UI：`agents_detail_renders_manual_compact_checkpoint`；`btw_thread_renders_compact_checkpoint_after_reload`。

### 6.6 BTW 重置

- `src/app/update/chat/btw/tests.rs`
  - `btw_new_emits_delete_thread_and_rejects_while_loading`
  - `thread_deleted_resets_view_and_qalist_mark`

### 6.7 渲染/文案

- `src/ui/main_content/lx/task/tests/detail_slash.rs`（新增）
  - `agent_badge_shows_private_then_steer_then_compacting`
  - `btw_badge_appends_compacting_only_when_it_fits`
  - `edit_footer_shows_commands_hint`
  - `slash_panel_anchors_above_composer`
- `src/ui/main_content/lx/tests/slash_panels.rs`
  - `detail_commands_render_inline_labels_and_tags`

### 6.8 回归

- 主输入 `/` 面板、`/rewind`、`/compact`、`/report` 不存在于主命令集、AI Web 过滤、主输入 `/undo` 别名仍指向 `/rewind`。
- 全量：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。

---

## 7. 风险与回滚

| 风险 | 影响 | 缓解 |
| :--- | :--- | :--- |
| `/rewind` 回滚文件与并发写冲突 | 覆盖后续修改 | 复用 `restore_plan` 的 after-image 校验；含文件动作按快照状态禁用；面板默认第一项与主聊天一致 |
| 删除粘性模式改变既有习惯 | 用户找不到 `Shift+Tab` | 胶囊直接显示 `private`；底栏 `[/ commands]`；`/report` 出现在 `/` 面板 |
| `/report` 额外一轮模型调用 | Token 成本 | 只在显式命令时触发；提示词要求简洁；不自动为每轮私聊调用 |
| 详情压缩与子轮并发 | revision 冲突导致压缩失败 | 空闲/无排队检查 + `expected_revision` 提交校验；失败不改上下文 |
| composer `/` 面板与 `@`/粘贴抢键 | 按键被吞 | 统一“任意 slash 打开时压制 @/md/refer”；slash 优先处理面板键 |
| BTW `/new` 误删线程 | 历史丢失 | 命令只能整段独立输入；运行中拒绝；删除后 Toast + 列表标记消失 |
| 文件膨胀 | 违反 1000 行规范 | 全部新逻辑放子模块；`text.rs` 只加 `pub mod detail;`；`event/mod.rs` 不加新事件 |

**回滚**：本次无迁移。回滚只需恢复代码；已产生的撤回/压缩/线程删除是用户显式操作，不需要数据回滚脚本。

---

## 8. 验收清单

1. Agents 空闲输入 `/`：面板只出现 `/rewind` `/compact` `/report`；`Enter` 执行，`Esc` 关闭并留在编辑态。
2. 私聊多轮后不触发任何主 agent 报告；主 agent 不上报、不自动起轮。
3. `/report 把登录 bug 结论汇报给主 agent`：子 agent 专门输出一份报告；主 agent 只收到该报告正文并自动起轮；之前的闲聊不进报告。
4. 详情 `/rewind` 回车 → 与主聊天相同的四项动作面板（默认第一项）；删对话动作删除最后一轮私聊并恢复文件；spawn 轮/报告轮/父 send_input 轮不可撤回；`/compact` 后再次 `/rewind` 显示 compact 恢复项。
5. `/compact` 显示 `compacting` 徽标，完成后出现压缩检查点卡片；再次执行提示 `Nothing new to compact.`。
6. BTW `/rewind` 删除最后一轮问答；删空时线程重置；`/new` 直接清空线程，列表 `● side thread` 消失。
7. 任意宽度下胶囊/提示不截半截；所有新文案为英文。
8. `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿。
