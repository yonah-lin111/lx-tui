# lx Recap、系统事件撤销与 AI Web 生图 — 执行任务清单

> 状态：**已完成（待合并）**（2026-11-09；实现、Review 修复、测试与提交均完成，等待用户确认合并）
> 设计依据：[design.md](design.md)
> Review 与修复清单：[review.md](review.md)
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
- [x] `src/app/state/lx.rs`：`pub recap: LxRecap`；`Lx::build` 初始化；会话切换/重置点清理。
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

## P5. UI：统计行、状态行、Recap 卡（文件变更卡已移除）

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

### P5.3 文件变更卡（已按用户要求移除）

> 后续变更：`FileChangesCard`、`BlockPaint::FileChanges`、`CardKind::FileChange`、`CardTarget::FileChange`、`FILE_CHANGES_SLOT_BLOCK` 及相关指纹/测试均已删除（review P2-12）。以下为历史记录。

- [x] 新增 `src/ui/main_content/lx/history/file_changes.rs`：
  - [x] `FileChangeSummary` 聚合（同路径合并、kind 规则、计数求和）；
  - [x] `FileChangesCard`（rows、click_spans、draw、签名）；
  - [x] `change_header`（add/edit/del、moved、计数未知）；
  - [x] 复用 `file_diff::file_body` / `apply_body_prefixes`（改为 `pub(super)`）。
- [x] `src/ui/main_content/lx/history/paint.rs`：`BlockPaint::FileChanges`，补 rows/bytes/draw/card_spans。
- [x] `src/ui/main_content/lx/history/cache.rs`：`RECAP_SLOT_BLOCK` / `RECAP_RUNNING_SLOT_BLOCK`（文件块槽位已移除）。
- [x] `src/ui/main_content/lx/history/cache/block_signature.rs`：`recap_block_signature`（`file_changes_block_signature` 已移除）。
- [x] `src/ui/main_content/lx/history/cache/signature.rs`：列表指纹加入 recap 展开状态与 recap 数据。
- [x] `src/ui/main_content/lx/history/snapshot.rs`：`tail()` 追加 recap 块并登记 `CardRegion`。
- [x] `src/ui/main_content/lx/history/cards.rs`：`CardKind::Recap { message_id }`（`FileChange` 已移除）。
- [x] `src/app/state/lx/chat.rs`：`CardTarget::Recap { message_id }`（`FileChange` 已移除）。
- [x] `src/ui/main_content/lx/history_hit.rs`：`target_of` 映射。
- [x] `src/app/update/chat/viewport.rs`：`toggle_lx_card` 分支（pin viewport + toggle_section）。
- [x] `src/app/update/chat/subagent.rs`：`toggle_agent_card` 分支。

### P5.4 Recap 卡

- [x] 新增 `src/ui/main_content/lx/history/recap.rs`：`build`（SystemCard）+ `running_line`。
- [x] `snapshot.rs`：recap 卡 + 运行中占位 + 点击面登记。
- [x] `HistorySource` 增加 `recap` 字段；`from_chat` 使用 `LxRecap::EMPTY`。
- [x] 测试：渲染、Next、折叠、运行中、空行。

---

## P6. 首页快捷键（已按用户要求移除）

> 后续变更：本阶段实现已全部撤销；`LxInput` 快捷键、input 映射、pane 切换、`home_visible`、入口 tooltip 与相关测试均已删除。以下为历史记录。

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
- [x] `cargo test app::update::chat::web`
- [x] `cargo test chat::web`
- [x] `cargo test ui::main_content::lx`
- [x] `cargo test input::`
- [x] `cargo test tui::`
- [x] `cargo test`（全量；若时间允许）
- [x] 修正所有失败；禁止用 `unwrap()`/`expect()` 进生产代码。
- [x] 检查新增生产文件行数 < 1000，测试文件 < 1500。

---

## P9. 后续修正（2026-11-09）

- [x] 移除首页快捷键与入口 tooltip（`Alt+1..4` / `Alt+W`、`LxInput` 变体、`home_visible`、相关测试）。
- [x] 移除 `FileChangesCard` 文件变更摘要卡及其 `BlockPaint` / `CardKind` / `CardTarget` / 指纹 / 测试。
- [x] `/rewind` 目标扩展到模型切换、compact 检查点与 recap，按 `sequence` 取最新事件：
  - [x] 模型切换：回滚会话模型并删除 `agent_model_switch` 条目；
  - [x] compact：`Database::revert_compact` **物理删除**检查点行（不再保留 `state='reverted'`），并把该会话剩余最新检查点恢复为 `active`；更晚的 active 检查点存在时拒绝；
  - [x] recap：删除 `agent_message_recap` 行并移除内存视图。
- [x] `compact_at_bottom` 统一只看 `active` 检查点；自动与手动 `/recap` 在时间线底部存在晚于最后一条完成态 assistant 的 `◆ Context compacted` 检查点时均被拒绝。
- [x] `migrate_v25_recap` 清理旧库遗留的 `state='reverted'` compact 检查点。
- [x] 普通轮次 rewind 后同步重载 recap 列表，避免已删消息的回顾残留。
- [x] compact / recap 运行中耗时统一为秒级（`3s` / `1m 12s`），不显示毫秒或十分之一秒。
- [x] 补充 DB 与交互测试：模型切换回滚、compact 物理删除、recap 删除、系统事件 rewind 面板。

### P9.1 AI Web 生成图片预览与原图下载

- [x] `src/app/update/chat/web.rs`：`on_ai_web_image_generated` 把生图附件挂到最后一条 assistant 消息并登记 `image_originals`。
- [x] 消息尾部预览树：`├─/└─ Image (preview): <下划线文件名>  [Download original]`；只显示 `MessageAttachmentKind::Image` 且 `source_path` 非空的附件。
- [x] 点击文件名 → `CardKind::Attachment` → `Effect::RevealInFileManager`，在系统文件管理器中定位本地预览图。
- [x] 点击 `[Download original]` → 黄色 `[downloading...]` → `[✓ Downloaded]`；再次点击 `[✓ Downloaded]` 定位原图；失败恢复 `[Download original]` 并 toast。
- [x] `Config.image_download_dir`（`settings.json`；浏览器 Settings `General → Downloads`；CLI `--image-download-dir`；空串默认 `~/Desktop`），跨平台展开 `~` / `~\`。
- [x] Gemini：`blob:` 预览用 `<canvas>` 提取；原图 URL 先剥离查询串再追加 `=s0-d-I?alr=yes`；`download_image` 重载会话页从 `performance` 恢复永久 `gg/` URL。
- [x] Grok：正文提取临时隐藏 `.not-prose`（生图卡片/加载占位/按钮），`try/finally` 保证恢复。
- [x] 实时生图 `saveGeneratedImages(..., overwrite = true)` 覆盖同名旧缓存；`load_session` 保持缓存复用；`turnNum` / `DOWNLOAD_STEM` 防御性清洗。
- [x] Gemini 多图等待全部加载（最多 250 ticks，约 15s）；生成停止且文本稳定 14 ticks 后放行 `caughtUp`。

---

## P10. Review 与修复（2026-11-09）

> 依据：[review.md](review.md)。结论：未发现 P0；3 个 P1、12 个 P2、10 个 P3 全部按优先级修复并通过回归测试。

- [x] **P1-1** 原图下载在 `SessionBound` 前发起时，回执按 `attachment_id` 定位并兼容临时/真实会话 ID，按钮不再卡在 `[downloading...]`；新增 pending → bound → 回执回归测试。
- [x] **P1-2** 自动 recap 在轮次运行中/compact 条件暂不满足时统一 `Retry`（30 秒后复检），并移除 `FocusArea::Panes` 限制。
- [x] **P1-3** `compact_at_bottom` / `select_latest` / `select_compact` 只看 `active` 检查点；`revert_compact` 增加 `state = 'active'` 条件并恢复前一条；v25 迁移清理 `reverted` 残留。
- [x] **P2-1** `recap_exchanges` 排除子 agent 报告轮次；多段文本改用换行拼接。
- [x] **P2-2** recap 跨窗格互斥、取消与回填广播；compact 撤销清理同会话窗格选中态。
- [x] **P2-3** `parse_generated_recap` 容忍额外字段、提取首个 JSON 对象、超长文本按字符边界截断。
- [x] **P2-4** 累计缓存命中率按单条消息配对分子/分母。
- [x] **P2-5** 状态行 `omitted turns` 预留宽度，避免覆盖 `context` / `cache`。
- [x] **P2-6** 助手图片仅在 `preview_path.is_some()` 时加下划线，并过滤非图片/空路径附件。
- [x] **P2-7** `saveGeneratedImages` 新增 `overwrite` 参数；`/rewind` 清理被截断消息的 `image_originals`。
- [x] **P2-8** Gemini 多图等待全部加载 + 250 ticks 上限；`caughtUp` 在文本稳定 14 ticks 后放行。
- [x] **P2-9** `toGeminiOriginalImageUrl` 先剥离 query 再剥离尺寸后缀。
- [x] **P2-10** `resolved_image_download_dir` / `sessions_root_dir` 复用跨平台 `home_dir` / `expand_tilde_path`。
- [x] **P2-11** 生产文件拆分至 < 1000 行；Gemini 图片逻辑拆到 `src/chat/web/gemini_images.js`。
- [x] **P2-12** `design.md` / `task.md` 同步移除 `FileChangesCard` 与首页快捷键，并补充系统事件 rewind、底部 compact 禁 recap、AI Web 生图与原图下载。
- [x] **P3-1** `LxRecap::reset` 保留 `next_generation` 单调递增；修正 `set_message_recaps` 返回值；清理死字段。
- [x] **P3-2** `try_send` 提交前取消 pending recap；`on_turn_committed` 先校验 generation；关闭开关时清理自动计时器与自动 pending。
- [x] **P3-3** 删除 `UserRefLine.source_path` 死字段；收紧 `file_diff` 可见性；补齐 `RefLines` 字节估算。
- [x] **P3-4** 速度过滤门槛改为 `speed >= 0.05`；`list_signature` 补 `message.turn_index`。
- [x] **P3-5** Gemini `extractCleanText` / Grok `extractText` 使用 `try/finally` 恢复临时隐藏节点。
- [x] **P3-6** `turnNum` 规范化为正整数；`DOWNLOAD_STEM` 经 `path.basename` 与非法字符过滤。
- [x] **P3-7** `--image-download-dir` 加入 `VALUE_OPTIONS`、help 与 `settings show`；`Config::from_json` 接受 `null`。
- [x] **P3-8** 设置页 Recap 开关文案改为 `Automatically generate a recap after 30 minutes away`。
- [x] **P3-9** Linux `reveal_in_file_manager` 对无父目录裸文件名回退 `.`。
- [x] **P3-10** 同步更新 `history.rs` / `status.rs` / `compact.rs` / `compact` 状态模块头注释。

---

## P8. 收尾

- [x] `git status` 确认只包含本任务文件。
- [x] `git add` + `git commit`（提交信息：`feat(lx): add recap, turn file summary and home shortcuts`）。
- [x] 输出变更摘要、验证结果、风险与未验证部分。
- [ ] 询问用户是否合并回原分支；合并前核对 `SCHEMA_VERSION` 与迁移版本冲突。
- [ ] 若用户确认合并，按项目规范处理 worktree 与隔离数据库清理。
