# lx Recap、消息尾部文件摘要与首页快捷键 — 执行任务清单

> 状态：**执行中**（用户已授权按 [design.md](design.md) 与本清单直接执行，无需再次审阅）
> 设计依据：[design.md](design.md)
> 执行位置：`.worktrees/lx-recap-and-turn-summary`（新建 git 工作区）
> 完成标准：实现 + 测试 + `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + 定向/全量测试；完成后提交代码并询问用户是否合并。

---

## P0. 开工门槛

- [x] 在 `.worktrees/` 新建 git 工作区 `lx-recap-and-turn-summary`，从当前 `dev`（或当前分支）切出特性分支 `feat/lx-recap-and-turn-summary`。
- [x] 将本会话工作目录切到工作区。
- [x] 在工作区初始化代码检索索引：
  - `codegraph init`（若 CLI 可用）；
  - codebase-memory `index_repository`（若未索引）。
- [x] 用 codegraph / codebase-memory 复核本清单涉及的核心符号：`Builder::tail`、`BlockPaint`、`LxChat.detail_expanded`、`LoadedSession`、`commit_rewind`、`copy_fork_rows`、`AiSettings`、`event::handle_terminal_event`、`LxInput`。
- [x] 复核 `SCHEMA_VERSION` 仍为 24；若已被其他分支占用，顺延到下一个可用版本并同步所有测试。

---

## P1. DB v25 与 recap 持久化（`src/db/`）

- [x] `src/db/mod.rs`：`SCHEMA_VERSION` 24 → 25；`migration_report` 表清单加入 `agent_message_recap`。
- [x] `src/db/migration/schema.rs`：追加 `SCHEMA_V25_RECAP`（DDL 见 design §4.2.1）。
- [x] `src/db/migration.rs`：追加 v25 分支与 `migrate_v25_recap`。
- [x] 新增 `src/db/chat/recap.rs`：
  - [x] `MessageRecap`、`RecapCommit`；
  - [x] `Database::commit_recap`（校验 session/workspace/message role，upsert by message_id，不递增 revision）；
  - [x] `Database::load_recaps`（按 session 批量加载）。
- [x] `src/db/chat/mod.rs`：`mod recap;` + 导出。
- [x] `src/db/chat/fork.rs`：两处复制路径新增 `copy_recaps`，按 `message_ids` 映射、按 boundary 截取。
- [x] `src/db/service/chat.rs`：`LoadedSession` 增加 `Vec<MessageRecap>`；`load_chat_session_into_pane` 读取并调用 `update::set_message_recaps`。
- [x] 测试：
  - [x] `src/db/migration/tests.rs`：v24 → v25 建表/索引/幂等/外键。
  - [x] `src/db/chat/recap/tests.rs`：commit/upsert/load/工作区隔离/非 assistant 拒绝。
  - [x] `src/db/chat/rewind/tests.rs`：rewind 级联删除 recap。
  - [x] `src/db/chat/fork/tests.rs`：fork 复制 recap、message_id 重映射、边界前不复制。

---

## P2. 配置与设置页（`src/config/`、`src/settings/`、`assets/`）

- [x] `src/config/ai/mod.rs`：
  - [x] `recap_enabled`（`recapEnabled`，default true）、`recap_summary`（`recapSummary`）；
  - [x] `default_true`、`empty()`、`sanitize_optional_selections_with_connect`、`validate_inner`。
- [x] `src/config/ai/connect.rs`：`sanitize_runtime_selections`、断开连接清理、变更比较加入 `recap_summary`。
- [x] `src/settings/service/models/mod.rs`：`revision_value`、GET、PUT 加入 recap 字段。
- [x] `src/settings/service/models/connect.rs`：两处 runtime/custom 同步加入 `recap_summary`。
- [x] `assets/settings.html`：Model Tab 新增 Recap fieldset（checkbox + select host）。
- [x] `assets/settings_models.js`：
  - [x] `isModelSettingsDirty`、`snapshotToDraft`；
  - [x] `deleteProvider` / `removeModel` 清理 recap；
  - [x] `renderDefaultModel` 同步 select/checkbox；
  - [x] `buildPayload`、`initModelsView`。
- [x] `assets/settings_connect.js`：悬空清理列表与关闭 provider 清理加入 `recapSummary`。
- [x] 测试：
  - [x] `src/config/ai/tests.rs`：默认 true、false、悬空清理、validate。
  - [x] `src/settings/service/models/tests.rs`：PUT/GET round-trip、revision、旧 payload。
  - [x] `src/settings/service/tests.rs`：HTML/JS 资源断言。
- [x] 修正所有 `AiSettings` 字面量编译错误。

---

## P3. Recap 纯逻辑（`src/chat/recap.rs`）

- [x] 新增 `src/chat/recap.rs`：
  - [x] 常量：8 KiB 估算、8 组、700/200 字符、60s 超时、16 KiB 输出上限；
  - [x] `RECAP_SYSTEM_PROMPT`（英文）；
  - [x] `RecapExchange`、`RecapResult`；
  - [x] `build_recap_prompt`（compact 前缀、最近 8 组、omitted/excerpt、compact 最后裁）；
  - [x] `build_recap_request`（无工具、purpose `CompactSummary`）；
  - [x] `parse_generated_recap`（think/围栏/JSON/纯文本/长度校验）。
- [x] `src/chat/mod.rs`：`pub mod recap;`。
- [x] 测试 `src/chat/recap/tests.rs`（design §6.1）。

---

## P4. App 状态、Effect、事件、焦点与自动调度

### P4.1 状态

- [x] `src/app/state/lx/recap.rs`：`LxRecap`、`RecapPending`、`MessageRecapView`、`EMPTY`、`reset`、`reset_for_session`、`is_empty`。
- [x] `src/app/state/lx.rs`：`pub recap: LxRecap`；`Lx::build` 初始化；`home_visible()`；会话切换/重置点清理。
- [x] `src/app/state/lx/chat.rs`：
  - [x] `ChatStatus.cache_hit_percent`；
  - [x] `recap_selection`、`resolve_recap_model`；
  - [x] `refresh_status` 计算累计缓存命中率。
- [x] `src/app/state.rs`：`recap_due_at: Option<Instant>`；`demo()` 初始化。

### P4.2 事件与请求

- [x] `src/app/effects.rs`：`GenerateRecap { ... }`。
- [x] `src/event/mod.rs`：`AppEvent::RecapGenerated`；pump `apply_recap_effects`；dispatch `handle_recap_generated`；FocusLost/FocusGained 处理。
- [x] `src/event/chat/recap.rs`：`RecapGeneratedEvent`、`apply_recap_effects`、`execute_recap_stream`、`handle_recap_generated`。
- [x] `src/event/chat/mod.rs`：`pub mod recap;`。
- [x] `src/db/service.rs`：把 `GenerateRecap` 加入非 DB effect 的 filter/retain 列表（与 `GenerateSessionTitle` 同处）。

### P4.3 调度与命令

- [x] `src/app/update/chat/recap.rs`：
  - [x] `RECAP_IDLE` / `RECAP_RETRY`；
  - [x] `on_terminal_focus_lost` / `on_terminal_focus_gained`；
  - [x] `tick_recap`（pending elapsed、due、eligibility、retry）；
  - [x] `schedule_recap`（模型、prompt、pending、effect）；
  - [x] `recap_command`（手动校验与 toast）；
  - [x] `take_pending_recap`、`apply_recap`、`cancel_pending_recap`；
  - [x] `set_message_recaps`（填充 map、completed/last turn/sequence）。
- [x] `src/app/update/chat/mod.rs`：
  - [x] `on_stream_finished` Complete 分支 `completed_turns += 1`；
  - [x] `on_turn_committed`/begin_turn 取消 pending；
  - [x] 导出 recap 模块。
- [x] `src/app/update/interaction.rs`：`tick` 调用 `tick_recap`；`next_deadline` 加入 recap due 与 pending elapsed。
- [x] `src/app/update/chat/slash.rs`：`/recap` dispatch。
- [x] `src/app/state/lx/slash.rs`：`SlashCommand::Recap` 全链路。
- [x] `src/ui/text.rs`：`LX_RECAP_COMMAND_DESC`。
- [x] `src/platform/mod.rs`：`enable_focus_capture` / `disable_focus_capture`。
- [x] `src/tui/mod.rs`：init/restore 接入 focus capture，保持恢复顺序。

### P4.4 测试

- [x] `src/app/update/chat/recap/tests.rs`：手动/自动/取消/失败/active pane/rounds gap。
- [x] `src/event/tests/`：FocusLost 设置 deadline、FocusGained 取消、`/recap` AI Web 拒绝。
- [x] `src/tui/tests.rs`：focus disable 顺序断言。
- [x] 更新因 `AppState`/`ChatStatus`/`LxInput`/`SlashCommand` 新增字段或变体而失败的测试。

---

## P5. UI：统计行、状态行、文件变更卡、Recap 卡

### P5.1 统计行

- [x] `src/ui/main_content/lx/history/stats.rs`：`stats_line(message, width)` 单行降级；移除 cached。
- [x] `src/ui/text.rs`：`LX_CHAT_CACHE_SUFFIX`；移除/别名 `LX_CHAT_STATS_CACHE_SUFFIX`。
- [x] `src/ui/main_content/lx/history/snapshot.rs`：不再 `wrap` 统计行；调用新签名。
- [x] 更新 `stats_line` 所有调用点与测试。

### P5.2 状态行

- [x] `src/ui/main_content/lx/status.rs`：
  - [x] `StatusLayout.cache`；
  - [x] `cache_text`；
  - [x] 候选组合与位置计算；
  - [x] 渲染 ` · 70% cached`。
- [x] 测试：累计命中率、布局降级、渲染文本。

### P5.3 文件变更卡

- [x] 新增 `src/ui/main_content/lx/history/file_changes.rs`：
  - [x] `FileChangeSummary` 聚合（同路径合并、kind 规则、计数求和）；
  - [x] `FileChangesCard`（rows、click_spans、draw、签名）；
  - [x] `change_header`（add/edit/del、moved、计数未知）；
  - [x] 复用 `file_diff::file_body` / `apply_body_prefixes`（改为 `pub(super)`）。
- [x] `src/ui/main_content/lx/history/paint.rs`：`BlockPaint::FileChanges`，补 rows/bytes/draw/card_spans。
- [x] `src/ui/main_content/lx/history/cache.rs`：`FILE_CHANGES_SLOT_BLOCK` / `RECAP_SLOT_BLOCK` / `RECAP_RUNNING_SLOT_BLOCK`。
- [x] `src/ui/main_content/lx/history/cache/block_signature.rs`：`file_changes_block_signature`、`recap_block_signature`。
- [x] `src/ui/main_content/lx/history/cache/signature.rs`：列表指纹加入文件行/列表/recap 展开状态与 recap 数据。
- [x] `src/ui/main_content/lx/history/snapshot.rs`：`tail()` 追加文件变更块并登记 `CardRegion`。
- [x] `src/ui/main_content/lx/history/cards.rs`：`CardKind::FileChange { message_id, slot }`、`CardKind::Recap { message_id }`。
- [x] `src/app/state/lx/chat.rs`：`CardTarget::FileChange { message_id, slot }`、`CardTarget::Recap { message_id }`。
- [x] `src/ui/main_content/lx/history_hit.rs`：`target_of` 映射。
- [x] `src/app/update/chat/viewport.rs`：`toggle_lx_card` 分支（pin viewport + toggle_section）。
- [x] `src/app/update/chat/subagent.rs`：`toggle_agent_card` 分支。

### P5.4 Recap 卡

- [x] 新增 `src/ui/main_content/lx/history/recap.rs`：`build`（SystemCard）+ `running_line`。
- [x] `snapshot.rs`：recap 卡 + 运行中占位 + 点击面登记。
- [x] `HistorySource` 增加 `recap` 字段；`from_chat` 使用 `LxRecap::EMPTY`。
- [x] 测试：渲染、Next、折叠、运行中、空行。

---

## P6. 首页快捷键

- [x] `src/app/actions.rs`：`LxInput::SwitchEntry(u8)`、`SwitchAiWebSite`。
- [x] `src/input/mod.rs`：Alt+1..4、Alt+W 映射（大小写兼容）。
- [x] `src/app/update/pane.rs`：`apply_lx_input` 开头分派；`switch_lx_entry_from_home` / `switch_ai_web_site_from_home`。
- [x] `src/app/state/lx.rs`：`home_visible()`。
- [x] Tooltip（可选但按 D13 实现）：入口 hover 显示 `Alt+1..4` / `Alt+W`。
- [x] 测试：
  - [x] `src/input/tests.rs`：路由。
  - [x] `src/app/update/tests.rs` / `src/app/update/chat/web/tests.rs`：入口/站点切换、历史存在时 no-op、Alt+W 语义。
  - [x] `src/ui/main_content/lx/tests.rs`：选中态。

---

## P7. 定向验证

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test chat::recap`
- [x] `cargo test db::chat::recap`
- [x] `cargo test db::migration`
- [x] `cargo test db::chat::rewind`
- [x] `cargo test db::chat::fork`
- [x] `cargo test config::ai`
- [x] `cargo test settings::service`
- [x] `cargo test app::update::chat::recap`
- [x] `cargo test ui::main_content::lx`
- [x] `cargo test input::`
- [x] `cargo test tui::`
- [x] `cargo test`（全量；若时间允许）
- [x] 修正所有失败；禁止用 `unwrap()`/`expect()` 进生产代码。
- [x] 检查新增生产文件行数 < 1000，测试文件 < 1500。

---

## P8. 收尾

- [x] `git status` 确认只包含本任务文件。
- [ ] `git add` + `git commit`（提交信息：`feat(lx): add recap, turn file summary and home shortcuts`）。
- [ ] 输出变更摘要、验证结果、风险与未验证部分。
- [ ] 询问用户是否合并回原分支；合并前核对 `SCHEMA_VERSION` 与迁移版本冲突。
- [ ] 若用户确认合并，按项目规范处理 worktree 与隔离数据库清理。
