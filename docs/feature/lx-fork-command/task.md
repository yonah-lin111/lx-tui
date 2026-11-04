# lx `/fork` 会话分支命令任务清单

- 关联设计：`docs/feature/lx-fork-command/design.md`
- 状态：**已实现（待合并）**
- 日期：2026-11-04

---

## 0. 评审门禁

- [x] 用户已阅读并确认 `design.md` 中 Q1-Q12 的 12 条决策。
- [x] 用户已确认“不复制 rewind / 子 agent / 后台进程”“无数据库迁移”“当前窗格切换”三点。
- [x] 确认后再按下面顺序执行；任何范围变化先回写 design.md。

---

## 1. 状态层：命令与面板模型

### 1.1 `src/app/state/lx/slash.rs`

- [x] `SlashCommand` 增加 `Fork`：
  - `ALL` 长度改为 7，顺序 `Model, Resume, Fork, New, Rewind, Compact, Rename`；
  - `name() -> "fork"`；
  - `aliases() -> &[]`；
  - `text() -> "/fork"`；
  - `description() -> crate::ui::text::LX_FORK_COMMAND_DESC`。
- [x] 新增 `ForkEntry`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkEntry {
    /// None = Full session
    pub message_id: Option<String>,
    pub label: String,
    pub created_at_ms: Option<i64>,
}
```

- [x] 新增 `ForkPanel`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkPanel {
    pub entries: Vec<ForkEntry>,
    pub window: PanelWindow,
    pub filter: String,
    pub filtered: Vec<usize>,
    pub now_ms: i64,
}
```

- [x] 实现 `ForkPanel::refilter`：
  - 过滤词变化时重置 `window`；有搜索词且命中消息时默认高亮第一条匹配项；
  - `filtered` 首项恒为 `0`（Full session）；
  - 其余按 `command_score(label.to_lowercase(), query)` 降序；
  - 空查询恢复 `0..entries.len()` 且高亮首项；
  - 末尾 `clamp_window()`。
- [x] 实现 `ForkPanel::active_entry`、`visible`、`clamp_window`。
- [x] `SlashPanel` 增加 `Fork(Box<ForkPanel>)`。
- [x] 补齐 `SlashPanel` 的 match：
  - `item_count` → `panel.filtered.len()`
  - `window` → `panel.window`
  - `current_index` → `None`
  - `set_window` → `panel.window = window`
- [x] 检查所有 `match panel` / `match self` 的穷尽性；`move_active` / `set_active` / `scroll_viewport` 走通用实现。

**验收**：`cargo check` 时所有 `SlashPanel` 分支都被编译器要求处理；`ForkPanel` 单测覆盖过滤与 Full session 固定。

### 1.2 重导出

- [x] `src/app/state/lx.rs` 的 `pub use slash::{...}` 增加 `ForkEntry, ForkPanel`。
- [x] `src/app/state.rs` 的 `pub use lx::{...}` 增加 `ForkEntry, ForkPanel`（若上层 UI 需要直接引用）。

---

## 2. App 更新层

### 2.1 新增 `src/app/update/chat/fork.rs`

- [x] `open_fork_panel(state, pane) -> bool`：
  1. 取窗格；要求 `view == PaneView::Lx` 且 `!shell_mode()`；
  2. 取 `session_id`；无会话 → `show_lx_hint(Warning, LX_FORK_NOTHING)`，返回 `true`；
  3. 全局占用检查：遍历 `state.all_pane_ids()`，任一窗格绑定同一 session 且 `request.is_busy() || has_running_shell()` → `show_lx_hint(Warning, LX_FORK_BUSY)`，返回 `true`；先收集 `(session_id, busy)` 快照，再取 `pane_mut_anywhere`，避免同时可变/不可变借用 `state`；
  4. 从 `target.lx.chat.messages` 过滤 `role == "user" && kind == "chat"`；空 → `LX_FORK_NOTHING`；
  5. 构造 `ForkEntry`：
     - `message_id: Some(message.id.clone())`
     - `label: fork_label(&message.text())`（120 字符截断 + 换行转空格 + 控制字符剔除）
     - `created_at_ms: target.lx.chat.message_time(&message.id)`
  6. 插入 `ForkEntry { message_id: None, label: String::new(), created_at_ms: None }` 到 `entries[0]`；
  7. 其余消息 `rev()` 倒序追加；
  8. `panel.filtered = 0..entries.len()`，`panel.now_ms = crate::db::now_ms()`；
  9. `target.lx.slash = Some(SlashPanel::Fork(Box::new(panel)))`。

> **时间来源**：`LxChat.message_times`（message_id → created_at_ms）在会话加载时整体重建，新轮提交时写入 user/assistant；`on_session_loaded_full`、`on_turn_committed`、`reset_session_view` 三处必须同步维护。

- [x] `on_fork_completed(state, pane, restore_text)`：
  - `restore_text` 非空 → `target.lx.replace_input(text)`；
  - 否则 → `target.lx.clear()`；
  - 窗格内 toast `ToastKind::Default` + `LX_FORKED`；
  - 返回 `true`。
- [x] `on_fork_failed(state, pane)`：
  - `show_lx_hint(Error, LX_FORK_FAILED)`；
  - 返回 `true`。

### 2.2 `src/app/update/chat/mod.rs`

- [x] 增加 `mod fork;`。
- [x] 增加 `pub use fork::{on_fork_completed, on_fork_failed, open_fork_panel};`。
- [x] 确认 `mod tests` 仍在文件末尾。

### 2.3 `src/app/update/chat/session.rs`

- [x] `apply_slash_command` 增加：

```rust
SlashCommand::Fork => super::fork::open_fork_panel(state, pane),
```

- [x] `apply_slash_enter` 增加 `SlashPanel::Fork(panel)` 分支：
  - 读 `panel.active_entry()`；
  - 取 `session_id` 与 `workspace`；
  - 关闭面板 + `lx.clear()`；
  - push `Effect::ForkChatSession { pane, workspace, session_id, before }`；
  - 返回 `true`。
- [x] 不修改 `SlashPanel::History` / `Rewind` 分支。

### 2.4 `src/app/update/pane.rs`

- [x] `refresh_slash_filter` 增加：

```rust
Some(super::super::state::SlashPanel::Fork(panel)) => panel.refilter(&query),
```

- [x] Esc 分支不新增 Fork 专用处理：现有 `_ => { lx.slash = None; lx.clear(); }` 已符合 Q9。
  - 验证：输入框为空时 `Lx::clear()` 会提前 return，因此必须先 `lx.slash = None`（顺序不能反）。

### 2.5 消息时间索引

- [x] `src/app/state/lx/chat.rs` 增加 `message_times: HashMap<String, i64>`，键为消息 id、值为 UTC 毫秒。
- [x] 提供 `set_message_time` / `message_time`；不修改 `ChatMessageView` 的 85 处构造点。
- [x] `on_session_loaded_full` 用加载消息整体重建映射。
- [x] `on_turn_committed` 为用户消息与 assistant 占位各写一条。
- [x] `reset_session_view` 清空映射。

### 2.6 `src/app/update/interaction.rs`

- [x] `tick_slash_panel_clock` 同时刷新 History 与 Fork 的 `now_ms`。

### 2.7 `src/app/effects.rs`

- [x] `Effect` 增加：

```rust
ForkChatSession {
    pane: crate::layout::PaneId,
    workspace: PathBuf,
    session_id: String,
    before: Option<String>,
},
```

- [x] 放在 `LoadChatSession` 旁边并补 doc comment。

**验收**：`cargo check` 通过；`open_fork_panel` / `apply_slash_enter` 的单测不触 DB。

---

## 3. 服务层

### 3.1 `src/db/service/chat.rs`

- [x] `handles` 增加 `Effect::ForkChatSession { .. }`。
- [x] 抽取 `load_chat_session_into_pane`：
  - 复制现 `Effect::LoadChatSession` 分支 483-572 行逻辑；
  - 参数：`state, pane, workspace: &Path, session_id: &str`；
  - 返回 `bool`；
  - 原分支只保留 `workspace` 解析 + 调用 helper。
- [x] 新增 `ForkChatSession` 分支：
  1. `self.with_db(|db| db.fork_chat_session(...))`；
  2. `None` → `update::on_fork_failed`，返回 `true`；
  3. `load_chat_session_into_pane`；
  4. 失败 → `on_fork_failed`，返回 `true`；
  5. 成功 → `on_fork_completed(state, pane, restore_text)`。
- [x] 检查 `other =>` 兜底 arm 不会捕获新效应（`handles` 和 match 都要加）。

### 3.2 `src/db/service.rs`

- [x] 增加：

```rust
#[cfg(test)]
mod fork_tests;
```

**验收**：`cargo test db::service::resume_tests` 全绿，证明抽取无回归。

---

## 4. 持久层

### 4.1 `src/db/chat/fork.rs`（新增）

- [x] 文件头注释：说明复制范围、rewind 不继承、图片随会话复制。
- [x] `ForkedChatSession { session_id, restore_text }`。
- [x] `Database::fork_chat_session(&self, workspace, session_id, before) -> Result<ForkedChatSession>`：
  1. `workspace_external_id`，无 → `WorkspaceMissing`；
  2. 查源会话（`source='native'`、归属校验）；
  3. `SELECT 1 FROM agent_message WHERE session_id=?1 AND status='streaming' LIMIT 1` → `SessionBusy`；
  4. `before` 解析边界与 `restore_text`；非法 → `ForkBoundaryMissing`；
  5. 图片目录复制（`crate::platform::fork_session_images`），失败仅 warn；
  6. `unchecked_transaction`；
  7. 插入新 `agent_session`；
  8. 加载并逐行复制消息（新 ID 映射）；
  9. 复制 parts / tools / attachments（图片路径重写）；
  10. 复制 active compact 检查点（`end_sequence < boundary`，base 重映射）；
  11. 复制 model switches（`sequence < boundary`）；
  12. commit；失败时 `remove_session_images(new_id)` 并返回错误。
- [x] `fork_title(parent: &str) -> String`：
  - ` (fork #N)` 尾部递增；
  - 其他情况追加 ` (fork #1)`；
  - 父标题空回退 `"Session"`。
- [x] `rewrite_managed_path` helper（若平台层没有提供）。
- [x] 使用 `super::single_line_preview` 计算 `first_user_message`。
- [x] 用量按复制消息求和。
- [x] `revision = 0`。

> 文件行数上限：`fork.rs` 预计 250-350 行；测试拆到 `src/db/chat/fork/tests.rs`，不要塞回 `src/db/chat/tests.rs`。

### 4.2 `src/db/chat/mod.rs`

- [x] 增加 `mod fork;`。
- [x] 增加 `pub use fork::ForkedChatSession;`。
- [x] 确认 `fork` 子模块能访问 `super::single_line_preview`（父模块私有项对子模块可见）。

### 4.3 `src/db/mod.rs`

- [x] `DbError` 增加 `ForkBoundaryMissing`。
- [x] `Display` 增加 `"fork boundary message not found"`。
- [x] `code()` 增加稳定错误码 `"fork boundary missing"`。
- [x] 不新增 schema 迁移，`SCHEMA_VERSION` 保持 22。

### 4.4 `src/platform/clipboard_image.rs`

- [x] 新增：

```rust
pub fn fork_session_images(source_session: &str, target_session: &str)
    -> Result<bool, ImagePrepareError>;

pub fn fork_session_images_in(
    sessions_root: &Path,
    source_session: &str,
    target_session: &str,
) -> Result<bool, ImagePrepareError>;
```

- [x] 源目录不存在 → `Ok(false)`。
- [x] 创建目标 `session_image_dir`，逐文件复制；任何 IO 失败 → `Err(ImagePrepareError::IoError)`。
- [x] 目录/文件权限沿用 `restrict_dir_permissions` / `restrict_file_permissions`（与归档路径一致）。
- [x] `src/platform/mod.rs` 导出两个函数。

---

## 5. UI 层

### 5.1 `src/ui/text.rs`

- [x] 新增：

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

- [x] 放在 `/resume` / `/model` 文案附近，保持 ASCII + 英文。

### 5.2 `src/ui/main_content/lx/slash.rs`

- [x] `items` 增加 Fork 分支：
  - `Full session` 用 `LX_FORK_FULL_SESSION`；
  - 其余用 `entry.label`；
  - `tag` 用 `time_labels` 同序（`Full session` 为空）。
- [x] `history_time_labels` 同时服务 History 与 Fork。
- [x] `is_placeholder` 增加 `Fork(_) => false`。
- [x] `view` 增加标题与 footer。
- [x] 所有调用点改名（`layout` / `item_at` / `render` 内 3 处）。

### 5.3 `src/ui/main_content/lx/tests.rs`

- [x] `mod fork_panels;`。

---

## 6. 测试

### 6.1 DB（`src/db/chat/fork/tests.rs`）

- [x] `fork_before_second_user_message_copies_first_turn`
- [x] `fork_full_session_copies_all_messages`
- [x] `fork_first_message_creates_empty_session`
- [x] `fork_rejects_missing_or_foreign_boundary`
- [x] `fork_rejects_streaming_source`
- [x] `fork_copies_only_checkpoints_before_boundary`
- [x] `fork_copies_model_switches_before_boundary`
- [x] `fork_sums_usage_of_copied_messages`
- [x] `fork_title_increments_existing_suffix`

要求：
- 每个测试独立临时库；
- 断言源会话消息数/内容完全不变；
- 断言新消息 ID 与源不同；
- 断言 `agent_turn_file_scope` / `agent_file_mutation` 在新会话中为 0 行。

### 6.2 平台（`src/platform/clipboard_image/tests.rs`）

- [x] `fork_session_images_copies_flat_files`
- [x] `fork_session_images_missing_source_is_noop`

### 6.3 App（`src/app/update/chat/tests/fork.rs`）

- [x] `fork_panel_lists_full_session_then_user_messages_newest_first`
- [x] `fork_panel_excludes_subagent_report_messages`
- [x] `fork_panel_filter_keeps_full_session`
- [x] `fork_busy_blocks_panel_and_shows_hint`
- [x] `fork_without_session_or_messages_blocks`
- [x] `fork_enter_pushes_effect_with_boundary_and_clears_input`
- [x] `fork_full_session_pushes_none_boundary`
- [x] `fork_escape_closes_and_clears`

### 6.4 服务（`src/db/service/fork_tests.rs`）

- [x] `fork_selected_message_creates_and_loads_new_session`
- [x] `fork_full_session_keeps_input_empty`
- [x] `fork_failure_keeps_source_session_bound`
- [x] `fork_pane_closed_before_load_does_not_panic`

### 6.5 UI（`src/ui/main_content/lx/tests/fork_panels.rs`）

- [x] `fork_panel_items_render_full_session_and_message_labels`（含相对时间列）
- [x] `fork_panel_renders_time_tag_even_with_long_labels`
- [x] `fork_panel_empty_filter_keeps_all_entries`

---

## 7. 验证命令

按顺序执行，只跑受影响范围：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test db::chat::fork
cargo test db::service::fork_tests
cargo test db::service::resume_tests
cargo test app::update::chat::tests::fork
cargo test ui::main_content::lx::tests::fork_panels
cargo test ui::main_content::lx::tests::slash_panels
```

最后按项目规范跑一次：

```bash
cargo test
```

> Agent 无法交互式验证 TUI；实现完成后由用户执行 `cargo run` 验证：
> - 输入 `/fork` 的候选与面板；
> - 上下键/过滤/Esc；
> - 选中节点后窗格切换、输入框回填；
> - Full session；
> - 流式中拦截；
> - 源会话 `/resume` 仍可打开。

---

## 8. 完成定义（DoD）

- [x] `/fork` 出现在 `/` 命令面板，描述正确。
- [x] 面板首项 `Full session`，用户消息最新在上，右侧显示相对时间。
- [x] 选中节点后新会话标题 `(fork #N)`、消息范围正确、输入框回填原文。
- [x] `Full session` 全量复制、输入框空。
- [x] 第一条消息可 fork 成空会话。
- [x] 流式/无会话/无消息/跨窗格占用均被拦截。
- [x] 失败无半截会话，提示 `Fork failed.`。
- [x] 源会话的文件撤销、消息、标题完全不受影响。
- [x] 无数据库迁移，`SCHEMA_VERSION` 不变。
- [x] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / 受影响测试全绿。
- [x] design.md 与最终实现一致；若实现中变更决策，先回写文档。
