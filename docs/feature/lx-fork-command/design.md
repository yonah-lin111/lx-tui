# lx `/fork` 会话分支命令设计

- 状态：**已合并到 dev（c7184d86）**
- 关联任务：`docs/feature/lx-fork-command/task.md`
- 复审报告：`docs/feature/lx-fork-command/review.md`
- 日期：2026-11-04
- 范围：`src/app/state/lx/slash.rs`、`src/app/update/chat/{fork,session,mod}.rs`、`src/app/update/pane.rs`、`src/app/update/interaction.rs`、`src/app/effects.rs`、`src/db/chat/{fork,mod}.rs`、`src/db/mod.rs`、`src/db/service/chat.rs`、`src/db/service/fork_tests.rs`、`src/platform/clipboard_image.rs`、`src/ui/main_content/lx/{slash,panels}.rs`、`src/ui/text.rs` 及对应测试

---

## 0. 背景与事实核验

### 0.1 参考实现（`/Users/yonah/projects/agent/opencode-dev`）

参考实现的 `/fork` 由三部分组成：

1. `packages/tui/src/routes/session/index.tsx:1080` 注册命令：
   - `title: "Fork session"`
   - `id: "session.fork"`
   - `slash: { name: "fork" }`
   - `run` 打开 `DialogFork`。
2. `packages/tui/src/routes/session/dialog-fork.tsx`：
   - 选项第一项是 `Full session`；
   - 其余选项为 `data.session.message.list(sessionID).filter(type === "user").toReversed()`；
   - 选中后调用 `client.api.session.fork({ sessionID, before: messageID })`；
   - 选中用户消息时先 `restorePrompt(message)`，再 `route.navigate` 到新会话并把 prompt 放回输入区；
   - 成功后 toast `Forked session`。
3. `packages/core/src/session/projector.ts:112` 的 `projectFork`：
   - 新会话标题用 `forkTitle(parent.title)`，规则是 `源标题 (fork #N)`，父标题已是 `(fork #N)` 时递增；
   - 只复制 `seq < boundary.seq` 的消息（`Full session` 时复制全部）；
   - 只复制已结算消息（running assistant / shell / compaction 不复制）；
   - 压缩摘要里的 `keepID` 会重映射到 fork 后的新消息 ID；
   - 使用量统计归零；
   - 内联媒体通过 `SessionBlob.ForkMedia` 复制，父会话行不被改写。

### 0.2 lx 当前实现

- `/` 命令定义在 `src/app/state/lx/slash.rs:77` 的 `SlashCommand`，当前 6 个：
  `Model`、`Resume`、`New`、`Rewind`、`Compact`、`Rename`。
- `SlashPanel`（`src/app/state/lx/slash.rs:471`）当前有 4 个视图：
  `Commands`、`Model`、`History`、`Rewind`。
- 命令采纳链路：
  - `src/app/update/pane.rs:725` `accept_slash_command` 消费命令行文本；
  - `src/app/update/chat/session.rs:833` `apply_slash_command` 分派命令；
  - 二级面板 Enter 由 `src/app/update/chat/session.rs:536` `apply_slash_enter` 分派。
- `SlashPanel` 的通用导航/窗口方法：
  - `item_count`、`window`、`set_window`、`move_active`、`set_active`、`scroll_viewport`；
  - 新增视图必须补齐 `item_count` / `window` / `set_window` / `current_index` 的 match 分支。
- 输入框历史视图 `ChatMessageView`（`src/app/state/lx/chat/steps.rs:815`）包含：
  - `id`（DB `agent_message.external_id`）
  - `kind`（`chat` / `user_shell`）
  - `role`（`user` / `assistant`）
  - `text()`（拼接全部正文 part，引用标记 `@[refer-*]` 原样保留）
  - `attachments`（附件元数据）
- 会话持久层：
  - `SCHEMA_VERSION = 22`（`src/db/mod.rs:27`），当前没有 fork/lineage 字段；
  - 会话表 `agent_session`、消息表 `agent_message`、内容块 `agent_message_part`、工具调用 `agent_tool_call`、附件 `agent_message_attachment`、压缩检查点 `agent_compact_checkpoint`、模型切换 `agent_model_switch`；
  - 文件撤销相关表 `agent_turn_file_scope`、`agent_file_mutation`、`agent_file_image`、`agent_rewind_operation`、`agent_session_notice`；
  - 图片本体按会话存放在 `<session_root>/<session_id>/image/`，删除会话时 `remove_session_images` 会整目录删除（`src/platform/clipboard_image.rs:206`）。
- 当前 lx 没有任何会话 fork 能力；`src/app/update/prompt/pages.rs` 里的 `fork_prompt_page` 是“提示词页复制”，与本次无关。

### 0.3 结论

本次需要新增“二级选择面板 + DB 事务复制 + 新会话装载”三块，但**不需要数据库迁移**：新会话仍是一行普通 `agent_session`，只是消息/part/工具/附件等从源会话复制。

---

## 1. Grill Me 已确认决策

| 编号 | 决策 | 结论 |
| --- | --- | --- |
| Q1 | 节点范围 | 只列用户聊天消息（`role=user && kind=chat`），最新在上；顶部固定 `Full session`；选中消息时复制它之前的完整对话，并回填该消息原文 |
| Q2 | 文件撤销继承 | 不复制 rewind 相关表；fork 里复制来的旧轮次只支持“删对话”，不支持“恢复文件”；fork 后的新轮次正常可撤销 |
| Q3 | 新会话落点 | 复用当前窗格并切到新会话；源会话原样保留；不新开窗格/标签页 |
| Q4 | 回填内容 | 只回填该用户消息的原始文本（含 `@[refer-*]` 标记）；不复制图片二进制、不重建独立附件芯片；再次发送时由 `prepare_user_message` 重新解析 |
| Q5 | 流式中行为 | 当前轮次流式生成时直接拦截，提示 `Session busy — wait for the current turn to finish.`，不打开面板 |
| Q6 | 标题 | 源标题追加 ` (fork #N)`；源标题已是 `(fork #N)` 时递增；不新增 lineage 字段、不显示来源 |
| Q7 | 空会话 | 未绑定会话，或没有用户聊天消息时，提示 `Nothing to fork yet.`，不打开面板 |
| Q8 | `Full session` | 复制全部已结算消息；输入框清空，不回填任何内容 |
| Q9 | 草稿处理 | 与 `/resume` 一致：面板期间输入框全文是过滤词；选中后整框清空（再回填节点原文或保持空）；Esc 关闭并整框清空 |
| Q10 | 跨窗格占用 | 任一窗格持有该 session 且正在生成/运行本地命令时，按会话忙拦截 |
| Q11 | 失败收尾 | fork 在单个 SQLite 事务内完成；失败回滚、关面板、窗格内提示 `Fork failed.`；成功提示 `Forked session` |
| Q12 | 第一条消息 | 允许空 fork：新会话消息为空，回填该消息原文 |

---

## 2. 目标与非目标

### 2.1 目标

1. 在 lx 输入框输入 `/fork` 后，出现 `/` 命令候选；回车打开 fork 节点面板。
2. 节点面板固定首项 `Full session`，其后按时间倒序列出当前会话的用户聊天消息。
3. 选中用户消息后：
   - 新建会话；
   - 复制该消息之前的所有已结算消息、parts、工具调用、附件、压缩检查点、模型切换记录；
   - 当前窗格切到新会话；
   - 输入框回填该消息原文。
4. 选中 `Full session` 后复制全部已结算消息，输入框为空。
5. 源会话完全不受影响，仍可通过 `/resume` 打开。
6. 失败不产生半个会话，反馈落在 lx 窗格内。

### 2.2 非目标

- 不新增数据库迁移、不新增 `fork_session_id` / `fork_boundary` 字段。
- 不复制文件撤销快照、撤销提示、文件 mutation、图片镜像之外的文件状态。
- 不复制子 agent 元数据/子会话、后台进程记录。
- 不新增窗格/标签页，不改变 `/resume`、`/rewind`、`/compact` 的既有行为。
- 不实现分支树可视化，不显示“forked from …”。
- 不把 `/fork` 做成带参数命令（如 `/fork <message-id>`）。

---

## 3. 关键设计

### 3.1 命令注册与交互流程

#### 3.1.1 命令定义

在 `src/app/state/lx/slash.rs` 增加：

```rust
pub enum SlashCommand {
    Model,
    Resume,
    Fork,
    New,
    Rewind,
    Compact,
    Rename,
}

impl SlashCommand {
    pub const ALL: [SlashCommand; 7] = [
        SlashCommand::Model,
        SlashCommand::Resume,
        SlashCommand::Fork,
        SlashCommand::New,
        SlashCommand::Rewind,
        SlashCommand::Compact,
        SlashCommand::Rename,
    ];

    pub fn name(self) -> &'static str {
        match self {
            // ...
            Self::Fork => "fork",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            // ...
            Self::Fork => &[],
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            // ...
            Self::Fork => "/fork",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            // ...
            Self::Fork => crate::ui::text::LX_FORK_COMMAND_DESC,
        }
    }
}
```

`echo_prefix` / `argument_hint` 不新增分支（fork 不带参数，走默认 `None`）。

#### 3.1.2 从输入到面板

1. 用户输入 `/fork`，`refresh_slash_panel` 生成命令候选。
2. 回车进入 `accept_slash_command`，整行 `/fork` 被消费，调用 `apply_slash_command`。
3. `apply_slash_command` 新增：

```rust
SlashCommand::Fork => super::fork::open_fork_panel(state, pane),
```

4. `open_fork_panel` 依次检查：
   - 窗格存在且是 `PaneView::Lx`；
   - 不是 shell 模式（`!target.lx.shell_mode()`）；
   - 全局占用：任一窗格绑定该 session 且 `request.is_busy() || has_running_shell()` → 提示 `LX_FORK_BUSY`；
   - 已绑定 `session_id` → 否则提示 `LX_FORK_NOTHING`；
   - 至少一条 `role == "user" && kind == "chat"` 且 `reports.is_empty()` 的消息 → 否则提示 `LX_FORK_NOTHING`。
5. 通过后构造面板骨架：

```rust
pub struct ForkEntry {
    /// None = Full session
    pub message_id: Option<String>,
    /// 单行截断后的展示文本
    pub label: String,
    /// 消息创建时间；Full session / 未知时间为 None
    pub created_at_ms: Option<i64>,
}

pub struct ForkPanel {
    pub entries: Vec<ForkEntry>,
    pub window: PanelWindow,
    /// 输入框全文快照
    pub filter: String,
    /// 过滤后的 entries 索引；index 0（Full session）恒定保留
    pub filtered: Vec<usize>,
    /// 相对时间基准，由事件循环 tick 刷新
    pub now_ms: i64,
}
```

`ChatMessageView` 本身不增加时间字段；消息时间由 `LxChat.message_times`
（message_id → UTC ms）承载：会话加载时整体重建，新轮提交时为用户/assistant 各写一条。
`open_fork_panel` 按当前消息 id 取时间；未知时间只在测试/异常数据下出现，时间列留空。

6. `target.lx.slash = Some(SlashPanel::Fork(Box::new(panel)))`。

#### 3.1.3 面板内操作

- `↑` / `↓`：复用 `SlashPanel::move_active`，跳过实现无需特殊处理（所有项可选）。
- 输入字符：`refresh_slash_panel` → `refresh_slash_filter` → `ForkPanel::refilter`。
- `Enter` / 鼠标点击：`apply_slash_enter` 的 `SlashPanel::Fork` 分支。
- `Esc`：`src/app/update/pane.rs` 现有 `_` 分支已经会“关闭二级面板 + 整框清空”，Fork 自动复用；不新增分支。
- 面板不显示 `❯` 当前项标记（`current_index` 返回 `None`），与 `Commands` / `Rewind` 一致。

`Full session` 的过滤规则：**始终保留在结果首位**，不参与过滤；其余消息按 `command_score(label, query)` 模糊过滤。这样输入过滤词时仍能一键整段复制。

空 fork 允许：选第一条用户消息时复制 0 条消息，但新会话仍创建，并回填该消息原文。

### 3.2 状态模型与导航分支

`SlashPanel` 增加：

```rust
pub enum SlashPanel {
    Commands { items: Vec<SlashEntry>, window: PanelWindow, query: String },
    Model(Box<ModelPicker>),
    History(Box<HistoryPanel>),
    Fork(Box<ForkPanel>),
    Rewind(Box<crate::app::state::lx::rewind::RewindPanel>),
}
```

必须补齐的方法分支（遗漏会编译失败，正好形成检查网）：

| 方法 | Fork 分支 |
| --- | --- |
| `item_count` | `panel.filtered.len()` |
| `window` | `panel.window` |
| `current_index` | `None` |
| `set_window` | `panel.window = window` |
| `move_active` | 通用 `window.move_active(delta, len)` 已覆盖，无需专用分支 |
| `set_active` / `scroll_viewport` | 走通用 `window` / `set_window` |

`ForkPanel` 自身方法：

```rust
impl ForkPanel {
    pub fn refilter(&mut self, query: &str) {
        if self.filter != query {
            self.filter = query.to_string();
            self.window = PanelWindow::default();
        }
        // Full session 恒定保留；其余按分数降序。
        let mut filtered = vec![0];
        if query.trim().is_empty() {
            filtered.extend(1..self.entries.len());
        } else {
            let query = query.trim().to_lowercase();
            let mut scored: Vec<(u32, usize)> = self
                .entries
                .iter()
                .enumerate()
                .skip(1)
                .filter_map(|(index, entry)| {
                    command_score(&entry.label.to_lowercase(), &query)
                        .map(|score| (score, index))
                })
                .collect();
            scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
            filtered.extend(scored.into_iter().map(|(_, index)| index));
        }
        self.filtered = filtered;
        self.clamp_window();
    }

    pub fn active_entry(&self) -> Option<&ForkEntry> {
        self.filtered
            .get(self.window.active)
            .and_then(|index| self.entries.get(*index))
    }

    pub fn visible(&self) -> impl Iterator<Item = &ForkEntry> {
        self.filtered
            .iter()
            .filter_map(|index| self.entries.get(*index))
    }

    pub fn clamp_window(&mut self) {
        self.window.clamp(self.filtered.len());
    }
}
```

`ForkEntry::label` 由 `ChatMessageView::text()` 生成：

```rust
fn fork_label(text: &str) -> String {
    const MAX_CHARS: usize = 120;
    let mut out = String::new();
    let mut truncated = false;
    for ch in text.chars() {
        if out.chars().count() >= MAX_CHARS {
            truncated = true;
            break;
        }
        match ch {
            '\n' | '\r' => out.push(' '),
            ch if (ch as u32) < 0x20 || ch == '\u{7f}' => {}
            ch => out.push(ch),
        }
    }
    let out = out.trim().to_string();
    if truncated { format!("{out}…") } else { out }
}
```

> 注意：必须截断。`CommandPanelView` 会按 `label.width()` 计算列宽并生成对齐 padding；数千字的用户原文会制造巨大临时字符串。

### 3.3 面板确认与效应

`apply_slash_enter` 新增分支：

```rust
SlashPanel::Fork(panel) => {
    let Some(entry) = panel.active_entry() else {
        return true;
    };
    let before = entry.message_id.clone();
    let Some((workspace, session_id)) = state
        .pane_anywhere(pane)
        .and_then(|target| {
            let session_id = target.lx.chat.session_id.clone()?;
            let workspace = state.chat_session_key_for_pane(pane)?;
            Some((workspace, session_id))
        })
    else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_WORKSPACE);
        return true;
    };
    // 二级面板是临时搜索框：选中即整框清空；源会话先保持绑定，等 DB 回执再切。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
        target.lx.clear();
    }
    state.effects.push(crate::app::effects::Effect::ForkChatSession {
        pane,
        workspace,
        session_id,
        before,
    });
    true
}
```

`Effect` 新增（放在 `LoadChatSession` 旁边）：

```rust
/// 从当前会话的某个用户消息之前分叉出新会话。
ForkChatSession {
    pane: crate::layout::PaneId,
    workspace: PathBuf,
    session_id: String,
    /// None = Full session；Some = 该用户消息之前。
    before: Option<String>,
},
```

`db/service/chat.rs` 的 `handles` 增加 `Effect::ForkChatSession { .. }`，并在 `execute_chat_effect` 中处理。

### 3.4 服务层：fork → 装载 → 回填

当前 `Effect::LoadChatSession` 的分支（`src/db/service/chat.rs:483-572`）承担了“加载消息、模型、进程、提示、压缩点、切换记录、revision 并回投视图”的全套逻辑。fork 必须复用同一套装载流程，不能另写一份简化版。

实现方式：

1. 把 `LoadChatSession` 的主体抽成私有方法：

```rust
impl Persistence {
    fn load_chat_session_into_pane(
        &self,
        state: &mut AppState,
        pane: crate::layout::PaneId,
        workspace: &Path,
        session_id: &str,
    ) -> bool {
        // 原 483-572 行逻辑原样搬入：
        // ensure_workspace → load_chat_session / chat_session_model /
        // load_processes_for_session / load_notices / compact_checkpoints /
        // model_switches / session_revision → on_session_loaded_full →
        // set_session_title → set_compact_checkpoints → set_model_switches →
        // request_report_drain_if_idle → reconcile_process_records
    }
}
```

2. `LoadChatSession` 分支改为：

```rust
Effect::LoadChatSession { pane, session_id } => {
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        return false;
    };
    self.load_chat_session_into_pane(state, pane, &workspace, &session_id)
}
```

3. `ForkChatSession` 分支：

```rust
Effect::ForkChatSession { pane, workspace, session_id, before } => {
    let Some(forked) = self.with_db(|db| {
        db.fork_chat_session(&workspace, &session_id, before.as_deref())
    }) else {
        update::on_fork_failed(state, pane);
        return true;
    };
    let loaded = self.load_chat_session_into_pane(state, pane, &workspace, &forked.session_id);
    if !loaded {
        // fork 已落库，但装载失败：新会话可在 /resume 找到；当前窗格保持源会话。
        update::on_fork_failed(state, pane);
        return true;
    }
    update::on_fork_completed(state, pane, forked.restore_text.as_deref());
    true
}
```

> 这里是“先 fork 落库、再装载”的两步。装载失败不会回滚 fork（新会话已存在），但界面必须提示失败且不切走；这是 Q11 允许的边界：**fork 会话本身完整**，只是本次视图切换失败。

4. `app/update/chat/fork.rs` 新增：

```rust
pub fn open_fork_panel(state: &mut AppState, pane: PaneId) -> bool { /* 见 3.1.2 */ }

pub fn on_fork_completed(
    state: &mut AppState,
    pane: PaneId,
    restore_text: Option<&str>,
) -> bool {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        match restore_text {
            Some(text) if !text.is_empty() => target.lx.replace_input(text),
            _ => target.lx.clear(),
        }
    }
    crate::app::update::show_lx_toast(
        state,
        pane,
        Toast::new(ToastKind::Default, text::LX_FORKED, Some(pane), Instant::now()),
    );
    true
}

pub fn on_fork_failed(state: &mut AppState, pane: PaneId) -> bool {
    crate::app::update::show_lx_hint(state, pane, InputHintLevel::Error, text::LX_FORK_FAILED);
    true
}
```

> `replace_input` 会压一次撤销快照、光标置末尾、关闭块/提及/粘贴面板；不会重新打开 `/` 命令面板（`refresh_panel` 不处理 slash）。

### 3.5 数据库：`fork_chat_session`

新增 `src/db/chat/fork.rs`，`src/db/chat/mod.rs` 增加 `mod fork;` 与 `pub use fork::ForkedChatSession;`。

```rust
/// fork 结果：新会话 id 与被选中消息的原始正文（Full session 为 None）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkedChatSession {
    pub session_id: String,
    pub restore_text: Option<String>,
}

impl Database {
    pub fn fork_chat_session(
        &self,
        workspace: &Path,
        session_id: &str,
        before: Option<&str>,
    ) -> Result<ForkedChatSession>;
}
```

#### 3.5.1 前置校验

1. 工作区存在且会话归属当前工作区、`source = 'native'`：

```sql
SELECT id, title, first_user_message, model_provider, model_id, variant
FROM agent_session
WHERE external_id = ?1 AND workspace_id = ?2 AND source = 'native'
```

2. 源会话没有未结算消息（Q5 / Q10 的 DB 兜底）：

```sql
SELECT 1 FROM agent_message
WHERE session_id = ?1 AND status = 'streaming'
LIMIT 1
```

命中则返回 `DbError::SessionBusy`。

3. 解析边界：

```sql
SELECT sequence, role, kind
FROM agent_message
WHERE session_id = ?1 AND external_id = ?2
```

要求 `role = 'user' && kind = 'chat'`，否则返回新增错误 `DbError::ForkBoundaryMissing`。
`before = None` 时边界为 `i64::MAX`（复制全部）。

4. 读取回填正文（仅 `before = Some`）：

```sql
SELECT content FROM agent_message_part
WHERE message_id = ?1 AND kind = 'text'
ORDER BY position LIMIT 1
```

#### 3.5.2 新会话标题

```rust
fn fork_title(parent: &str) -> String {
    const MARK: &str = " (fork #";
    if let Some(start) = parent.rfind(MARK)
        && parent.ends_with(')')
        && let Some(number) = parent[start + MARK.len()..parent.len() - 1].parse::<u32>().ok()
    {
        return format!("{}{}{})", &parent[..start], MARK, number + 1);
    }
    format!("{parent} (fork #1)")
}
```

示例：
- `重构登录模块` → `重构登录模块 (fork #1)`
- `重构登录模块 (fork #1)` → `重构登录模块 (fork #2)`
- `重构登录模块 (fork #2) (fork #1)`（用户手动改名）→ `重构登录模块 (fork #2) (fork #2)`，保持与 opencode 的“尾部编号递增”一致。

#### 3.5.3 单事务落库

使用 `self.conn.unchecked_transaction()`（与 `begin_turn_with_attachments` 同款）保证全有或全无。

新会话行：

```sql
INSERT INTO agent_session
    (external_id, workspace_id, source, title, first_user_message,
     model_provider, model_id, variant, status, archived,
     created_at_ms, updated_at_ms, last_message_at_ms,
     total_input_tokens, total_output_tokens, revision)
VALUES (?1, ?2, 'native', ?3, ?4, ?5, ?6, ?7, 'active', 0, ?8, ?8, ?9, ?10, ?11, 0)
```

关键取值：

- `title = fork_title(parent.title)`，父标题空时先回退 `"Session"`；
- `first_user_message`：从**实际复制**的消息里取第一条 `role='user' && kind='chat'`，用 `super::single_line_preview(text, 120)`；没有则空串；
- `model_provider/model_id/variant`：复制父会话当前三列（新会话从同一模型继续）；
- `created_at_ms = updated_at_ms = now_ms()`；`last_message_at_ms` 取最后一条复制消息的 `created_at_ms`，空 fork 用 `now_ms()`；
- `total_input_tokens` / `total_output_tokens`：对复制消息求和，不复制父会话总量（父总量包含边界之后的消息）；
- `revision = 0`：新分支没有 compact/rewind 操作，后续 `begin_turn` 从 0 开始递增。

消息复制：

```sql
SELECT external_id, sequence, turn_index, role, status, model_provider, model_id,
       variant, stop_reason, error_code, input_tokens, output_tokens,
       cache_read_tokens, cache_write_tokens, created_at_ms, completed_at_ms,
       kind, user_shell_json, subagent_report_json
FROM agent_message
WHERE session_id = ?1 AND sequence < ?2
ORDER BY sequence
```

逐行插入新 `external_id`（`new_external_id("msg")`），保留 `sequence` / `turn_index` / 状态 / 用量 / 时间戳。维护 `HashMap<旧 message_id, 新 message_id>` 供 parts/tools/attachments 使用。

parts：

```sql
INSERT INTO agent_message_part
    (external_id, message_id, position, kind, content, provider_metadata_json,
     created_at_ms, started_at_ms, ended_at_ms, completed_at_ms)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
```

工具调用：

```sql
INSERT INTO agent_tool_call
    (external_id, session_id, message_id, call_id, namespace, name, arguments_json,
     position, status, result_json, error_text, started_at_ms, completed_at_ms, duration_ms)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
```

- `external_id` 重新生成（`new_external_id("tool")`）；
- `call_id` 保持原值（新会话内 `UNIQUE(session_id, call_id)` 不会冲突，且工具结果重放按 `call_id` 配对）；
- `result_json` 若图片目录复制成功，则做路径前缀替换（见 3.5.4）。

附件：

```sql
INSERT INTO agent_message_attachment
    (external_id, message_id, position, kind, label, marker_start, marker_end,
     source_path, mime, width, height)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
```

- `external_id` 重新生成（`new_external_id("att")`）；
- `marker_start/end` 原样保留（消息正文逐字节复制，偏移仍然有效）；
- `source_path`：图片走路径前缀替换；`file` / `folder` 是工作区路径，原样保留。

压缩检查点：

```sql
SELECT external_id, sequence, operation_id, base_checkpoint_id, start_sequence,
       end_sequence, summary_text, model_label, state, reverted_by_operation_id, created_at_ms
FROM agent_compact_checkpoint
WHERE session_id = ?1 AND state = 'active' AND end_sequence < ?2
ORDER BY sequence
```

- 只复制 `active`；`reverted` 是历史操作记录，不进入新分支；
- 只复制 `end_sequence < boundary` 的检查点：检查点摘要覆盖的消息必须全部在 fork 内，否则会把边界之后的摘要泄漏进来；
- 新 `external_id` / 新 `operation_id`；`base_checkpoint_id` 按旧→新映射重写；
- `state = 'active'`，`reverted_by_operation_id = NULL`；
- `sequence` 保持不变（同一 session 内唯一）。

模型切换：

```sql
SELECT external_id, sequence, from_provider, from_model, from_variant,
       to_provider, to_model, to_variant, from_label, to_label,
       updated_sources_json, injected_prompt, created_at_ms
FROM agent_model_switch
WHERE session_id = ?1 AND sequence < ?2
ORDER BY sequence
```

逐行插入新 `external_id`，其余原样复制。这样 fork 的上下文装配仍会在正确位置注入 `<system-update>`。

#### 3.5.4 图片目录复制

图片本体不在 DB 里，必须随 fork 复制，否则删除源会话后 fork 的图片会全部失效。

`src/platform/clipboard_image.rs` 新增：

```rust
/// 把源会话的图片目录复制到目标会话；源目录不存在时返回 Ok(false)。
pub fn fork_session_images(source_session: &str, target_session: &str) -> Result<bool, ImagePrepareError>;

/// 显式根版本，供测试使用。
pub fn fork_session_images_in(
    sessions_root: &Path,
    source_session: &str,
    target_session: &str,
) -> Result<bool, ImagePrepareError>;
```

实现要点：

1. `session_image_dir(root, source)` 不存在则 `Ok(false)`；
2. 创建目标 `<root>/<target>/image/`，逐文件复制（图片目录是扁平的 hash 文件名）；
3. 返回 `Ok(true)`；
4. DB fork 在开始事务前调用，得到 `Ok(true)` 时记录：

```rust
let rewrite = Some((
    session_image_dir(&sessions_root, source_session),
    session_image_dir(&sessions_root, target_session),
));
```

5. 旧→新路径替换：

```rust
fn rewrite_managed_path(value: &str, from: &Path, to: &Path) -> String {
    let from = from.to_string_lossy();
    let to = to.to_string_lossy();
    value.replace(from.as_ref(), to.as_ref())
}
```

用于 `agent_message_attachment.source_path` 与 `agent_tool_call.result_json`。

失败策略：

- 图片目录复制失败：`tracing::warn!`，**不阻止 fork**；路径保持源会话路径（图片仍能显示，但源会话删除后失效）。这是“图片是附属数据，不应阻塞会话分支”的取舍。
- DB 事务失败：删除刚复制出的目标图片目录（`remove_session_images(new_session_id)`），避免孤儿文件。

#### 3.5.5 不复制清单

| 表 | 是否复制 | 原因 |
| --- | --- | --- |
| `agent_session` | 新建一行 | 新分支身份 |
| `agent_message` | 复制 `sequence < boundary` | 对话历史 |
| `agent_message_part` | 复制对应消息 | 正文/思考 |
| `agent_tool_call` | 复制对应消息 | 工具卡片与上下文重放 |
| `agent_message_attachment` | 复制对应消息 | 引用/图片元数据 |
| `agent_compact_checkpoint` | 仅 `active && end_sequence < boundary` | 分支内可复用的压缩摘要 |
| `agent_model_switch` | `sequence < boundary` | 模型切换注入点 |
| `agent_session_notice` | 不复制 | 撤销提示属于源分支的文件操作 |
| `agent_turn_file_scope` | 不复制 | Q2：撤销不继承 |
| `agent_file_mutation` | 不复制 | Q2 |
| `agent_file_image` | 不复制 | Q2 |
| `agent_rewind_operation` | 不复制 | Q2 |
| `agent_process` | 不复制 | 进程仍归源会话所有，fork 不得停止它 |
| `agent_subagent` / 子会话 | 不复制 | 子 agent 是独立运行树；fork 只保留消息里的汇报文本 |
| `todo` / prompt 表 | 不复制 | 与聊天会话无关 |

---

## 4. 渲染

`src/ui/main_content/lx/slash.rs` 需要补齐：

1. `items`：

```rust
SlashPanel::Fork(panel) => panel
    .visible()
    .map(|entry| CommandItem::Inline {
        label: entry
            .message_id
            .as_ref()
            .map_or(text::LX_FORK_FULL_SESSION, |_| entry.label.as_str()),
        preview: "",
        tag: time_labels.get(index).map_or("", String::as_str),
    })
    .collect(),
```

2. `history_time_labels` 同时服务 History 与 Fork：Fork 按 `entry.time_label(panel.now_ms)` 生成同序时间标签。
3. `src/app/update/interaction.rs` 的 `tick_slash_panel_clock` 同时刷新 History 与 Fork 的 `now_ms`，面板停留期间相对时间自然推进。
4. `is_placeholder`：`SlashPanel::Fork(_) => false`（Full session 恒在，不会空）。
4. `view`：标题 `LX_FORK_PANEL_TITLE`，底边 `LX_FORK_PANEL_FOOTER`；`right_title` 保持 `None`。
5. `CommandPanelView.disabled` 不使用（fork 所有项都可选）。

---

## 5. 新增用户可见文案

全部英文，放在 `src/ui/text.rs` 的 lx `/` 命令区：

```rust
pub const LX_FORK_COMMAND_DESC: &str = "Fork the session from an earlier message";
pub const LX_FORK_PANEL_TITLE: &str = "Fork";
pub const LX_FORK_PANEL_FOOTER: &str = "↑↓ select · ↵ fork · esc close";
pub const LX_FORK_FULL_SESSION: &str = "Full session";
pub const LX_FORK_BUSY: &str = "Session busy — wait for the current turn to finish.";
pub const LX_FORK_NOTHING: &str = "Nothing to fork yet.";
pub const LX_FORK_FAILED: &str = "Fork failed.";
pub const LX_FORKED: &str = "Forked session";
pub const LX_FORK_EMPTY_MESSAGE: &str = "(empty message)";
```

`LX_FORK_BUSY` 是独立文案，不复用 `LX_CHAT_SESSION_BUSY`（后者只有 `Session busy`，语义不足以说明“等本轮结束”）。

---

## 6. 错误、并发与回滚

### 6.1 应用层拦截

- 当前窗格 `request.is_busy()` 或 `has_running_shell()`；
- 任一窗格绑定同一 session 且 `request.is_busy()` 或 `has_running_shell()`；
- 未绑定会话 / 无用户聊天消息。

以上均不产生 DB 写入。

### 6.2 DB 层兜底

- `status = 'streaming'` 的消息存在 → `DbError::SessionBusy`（覆盖另一个 lx 实例正在写同一会话的情况）；
- 边界消息不存在、不属于源会话、不是 `user/chat` → `DbError::ForkBoundaryMissing`；
- 源会话不存在或属于别的 workspace → `DbError::SessionMissing`。

### 6.3 失败收尾

- 事务内任何错误：`?` 直接返回，未提交事务自动回滚；
- 已复制图片目录：调用 `remove_session_images(new_session_id)` 清理；
- 服务层提示 `Fork failed.`，源会话与源窗格不动；
- 不把 SQL/IO 原始错误进 UI；只写 `tracing`。

### 6.4 成功收尾

- 新会话 `updated_at_ms = now`，会排在 `/resume` 列表最前；
- 当前窗格切到新会话；
- 有 `restore_text` 时回填输入框；`Full session` 时输入框为空；
- 窗格内 toast `Forked session`。

---

## 7. 测试计划

### 7.1 DB 单元测试（`src/db/chat/fork/tests.rs`）

1. `fork_before_second_user_message_copies_first_turn`：
   - 建两轮完整会话；
   - fork 到第二轮用户消息；
   - 断言新会话标题 `(fork #1)`、消息数为 2、消息 ID 全新、`sequence` / `turn_index` 保留、parts 与工具调用复制、源会话行数不变。
2. `fork_full_session_copies_all_messages`：`before = None` 复制全部。
3. `fork_first_message_creates_empty_session`：允许空 fork，`restore_text` 等于第一条消息正文。
4. `fork_rejects_missing_or_foreign_boundary`：返回 `ForkBoundaryMissing`。
5. `fork_rejects_streaming_source`：`begin_turn` 后不 finish，返回 `SessionBusy`。
6. `fork_copies_only_checkpoints_before_boundary`：
   - 造两个 compact 检查点，一个 `end_sequence < boundary`、一个跨越 boundary；
   - 断言只复制前者，`base_checkpoint_id` 已重映射为新的检查点 id，`operation_id` 全新。
7. `fork_copies_model_switches_before_boundary`：边界之后的切换不复制。
8. `fork_sums_usage_of_copied_messages`：新会话 `total_input_tokens` / `total_output_tokens` 等于复制消息之和。
9. `fork_title_increments_existing_suffix`：`x (fork #2)` → `x (fork #3)`。

### 7.2 平台测试（`src/platform/clipboard_image/tests.rs`）

1. `fork_session_images_copies_flat_files`：源目录两个文件 → 目标目录同名同内容。
2. `fork_session_images_missing_source_is_noop`：返回 `Ok(false)`。
3. 路径重写 helper 的单测（若有独立函数）。

### 7.3 App 状态机测试（`src/app/update/chat/tests/fork.rs`）

1. `fork_panel_lists_full_session_then_user_messages_newest_first`：构造 `ChatMessageView`，断言 `entries[0].message_id == None`，后续按倒序。
2. `fork_panel_filter_keeps_full_session`：输入过滤词，`filtered[0] == 0`，其余按分数过滤。
3. `fork_busy_blocks_panel_and_shows_hint`：`request = Streaming`，`open_fork_panel` 返回 true、`slash` 仍为 None、窗格 toast 为忙碌文案。
4. `fork_without_session_or_messages_blocks`：两种空态都提示 `Nothing to fork yet.`。
5. `fork_enter_pushes_effect_with_boundary_and_clears_input`：选中第二条消息，断言 `Effect::ForkChatSession { before: Some(id), .. }`，`lx.input` 空、`slash == None`。
6. `fork_full_session_pushes_none_boundary`。
7. `fork_escape_closes_and_clears`：复用现有 `_` 分支，断言输入清空。

### 7.4 服务端到端测试（`src/db/service/fork_tests.rs`）

1. `fork_selected_message_creates_and_loads_new_session`：
   - 播种两轮会话；
   - 走 `LoadChatSession` → `open_fork_panel` → 选中第一轮；
   - `service.pump`；
   - 断言窗格 `session_id` 变为新会话、`chat.messages` 只剩复制消息、输入框回填原文、toast `Forked session`。
2. `fork_full_session_keeps_input_empty`。
3. `fork_failure_keeps_source_session_bound`：删除源会话后执行 fork，断言提示 `Fork failed.`，窗格仍绑定原 session。
4. `fork_pane_closed_before_load_does_not_panic`。

### 7.5 UI 测试（`src/ui/main_content/lx/tests/fork_panels.rs`）

1. `fork_panel_items_render_full_session_and_message_labels`：条目文本、`Full session` 标签与相对时间列。
2. `fork_panel_renders_time_tag_even_with_long_labels`：超长 label 截断让位，时间标签稳定渲染。
3. `fork_panel_empty_filter_keeps_all_entries`。

### 7.6 回归

- `cargo test db::service::resume_tests`：确认抽取 `load_chat_session_into_pane` 没有改变 `/resume` 行为。
- `cargo test ui::main_content::lx::tests::slash_panels`：确认新增 `SlashPanel` 分支不影响既有面板。

---

## 8. 验收标准

1. 输入 `/fork` 能看到命令候选，描述为 `Fork the session from an earlier message`。
2. 回车后出现面板：首项 `Full session`，其后是用户聊天消息（最新在上），右侧显示相对时间。
3. 选中某条消息：
   - 当前窗格切到新标题 `源标题 (fork #N)`；
   - 新会话只含该消息之前的对话；
   - 输入框回填该消息原文（含 `@[refer-*]`）；
   - 源会话消息、标题、文件撤销能力不变。
4. `Full session` 复制全部消息，输入框为空。
5. 第一条用户消息可 fork，得到空会话 + 回填原文。
6. 流式中、无会话、无消息时不会打开面板，提示正确。
7. 另一个窗格正在生成同一会话时，fork 被拦截。
8. fork 失败不产生半截会话。
9. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、相关 `cargo test` 全绿。

---

## 9. 风险与已知限制

1. **子 agent 不复制**：fork 会复制消息里的 `subagent_report_json`，但 `agent_subagent` 记录与子会话不复制；Agents 页在 fork 中为空，历史卡片仍按汇报文本渲染。这是刻意取舍，避免复制整棵运行树。
2. **后台进程不复制**：fork 的历史工具卡片可能显示源会话 `result_json` 里的进程状态；fork 的后台任务页不会列出这些进程，也不能停止它们。源会话仍是唯一所有者。
3. **压缩摘要边界**：跨越 fork 边界的 compact 检查点会被丢弃，fork 上下文会回放更多原始消息；预算裁剪仍按完整轮次工作，不会产生 orphan tool result。
4. **图片复制失败**：不阻塞 fork，但 fork 的图片指向源会话目录；删除源会话后图片不可用。
5. **无 lineage 字段**：无法从 DB 查询“谁 fork 了谁”；标题编号是唯一线索。若未来需要分支树，必须新增迁移与 UI。
6. **`/resume` 列表即时性**：fork 后新会话会立即出现在下一次 `/resume` 列表；当前实现不主动刷新已打开的 `/resume` 面板（与 `/rename` 的就地更新不同，fork 不依赖该面板）。
7. **`load_chat_session_into_pane` 抽取**：这是本次唯一触碰既有 `/resume` 主路径的重构，必须用 `resume_tests` 回归锁住行为。

---

## 10. 文件改动清单

| 文件 | 改动 |
| --- | --- |
| `src/app/state/lx/slash.rs` | `SlashCommand::Fork`；`SlashPanel::Fork`；`ForkEntry` / `ForkPanel`；`SlashPanel` 方法分支；相对时间 helper |
| `src/app/state/lx/chat.rs` | `LxChat.message_times` 与读写方法；加载/重置/新轮同步 |
| `src/app/state/lx.rs` / `src/app/state.rs` | 重导出 `ForkEntry` / `ForkPanel` |
| `src/app/update/chat/fork.rs` | 新增：`open_fork_panel` / `on_fork_completed` / `on_fork_failed` |
| `src/app/update/chat/session.rs` | 会话生命周期（加载、重命名、删除、解绑） |
| `src/app/update/chat/slash.rs` | 拆出：`apply_slash_command` 增加 Fork；`apply_slash_enter` 增加 Fork 分支 |
| `src/app/update/chat/task_page.rs` | 拆出：后台任务页状态与操作 |
| `src/app/update/chat/mod.rs` | 导出与路由连接 |
| `src/app/update/pane.rs` | `refresh_slash_filter` 增加 Fork |
| `src/app/update/interaction.rs` | `tick_slash_panel_clock` 刷新 Fork 的相对时间基准 |
| `src/app/effects.rs` | `Effect::ForkChatSession` |
| `src/db/chat/fork.rs` | 新增：`fork_chat_session`、`fork_title`、复制逻辑 |
| `src/db/chat/fork/tests.rs` | 新增 DB 单元测试 |
| `src/db/chat/mod.rs` | `mod fork;` + 导出 |
| `src/db/mod.rs` | `DbError::ForkBoundaryMissing`（Display + code） |
| `src/db/service/chat.rs` | `handles` 增加 Fork；抽取 `load_chat_session_into_pane`；Fork 分支 |
| `src/db/service.rs` | `#[cfg(test)] mod fork_tests;` |
| `src/db/service/fork_tests.rs` | 新增端到端测试 |
| `src/platform/clipboard_image.rs` | `fork_session_images` / `_in` 与路径重写 helper |
| `src/platform/mod.rs` | 导出新 helper |
| `src/ui/main_content/lx/slash.rs` | Fork 条目、标题/底边、空态判定 |
| `src/ui/text.rs` | 新文案常量 |
| `src/ui/main_content/lx/tests.rs` | `mod fork_panels;` |
| `src/ui/main_content/lx/tests/fork_panels.rs` | 新增 UI 测试 |
| `src/app/update/chat/tests.rs` | `mod fork;` |
| `src/app/update/chat/tests/fork.rs` | 新增状态机测试 |
