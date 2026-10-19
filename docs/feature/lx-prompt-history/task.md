# lx 输入框历史上下文切换：执行任务

- 状态：**已实施**（`feature/prompt-history` 分支，待用户验收与合并）。
- 前置：`design.md` 已由用户逐项确认（10 个决策，见其 §2）。
- 工作区：`.worktrees/prompt-history`，分支 `feature/prompt-history`（基于 `dev`）。

## 0. 开发门禁与现状核查

- [x] 用 codegraph `explore`/`node` 与 codebase-memory `search_code` 复查将改动的符号；
      确认 `apply_lx_input`、`Lx::move_up/move_down`、`on_turn_committed`、`Persistence`
      出箱路径、`SCHEMA_VERSION` 与 v9 迁移的实际形态无漂移。
- [x] 复核 `src/db/mod.rs` 中 `Result`/`DbError` 变体，决定是否需要新增错误码。
- [x] 复核 `AppState` 构造点（事件层初始 `AppState` 与测试构造函数），列出全部需要
      补 `prompt_history` 字段的调用点。
- [x] 确认现有 `Lx::snapshot` 的连续编辑合并语义，历史恢复不得被合并成一步。
- [x] 确认 `MAX_MESSAGE_BYTES`（2 MiB）与本功能 32 KiB 上限的关系，避免文案矛盾。

## 1. 全局历史模型（app 状态）

- [x] 新增 `src/app/state/prompt_history.rs`：`PROMPT_HISTORY_LIMIT = 100`、
      `PROMPT_HISTORY_MAX_BYTES = 32 * 1024`、`PromptHistory`。
- [x] `PromptHistory::record`：trim、空过滤、超长过滤、`position` 去重（存在则移到末尾）、
      裁剪到上限；返回是否有变化。
- [x] `PromptHistory::replace_all`：保持传入顺序（尾部最新），逐条按 `record` 口径过滤。
- [x] `AppState` 增加 `pub prompt_history: PromptHistory`，全部构造点补默认值。
- [x] `src/app/state.rs` 模块声明与 re-export。

## 2. 每窗格浏览态（Lx）

- [x] 新增 `src/app/state/lx/history.rs`：`HistoryBrowse { index, draft }`。
- [x] `Lx` 增加私有 `history: HistoryBrowse` 与转发方法：
      `history_browsing()`、`end_history_browse()`、`apply_history_text()`。
- [x] `apply_history_text`：同文本只移动光标到末尾；否则 `snapshot()` + 整体替换 +
      `cursor = len` + 清选区 + `scroll_cursor_into_view` + `refresh_panel`。
- [x] 确认 `snapshot()` 不被连续编辑合并：历史恢复必须独立成一步 undo。

## 3. 按键接管（update 层）

- [x] `apply_lx_input` 在面板链之后、`match command` 之前插入历史接管分支；严格判定
      「无任何面板」（paste / `mention_open` / `mention_visible` / slash / md panel）。
- [x] `try_history(state, pane, direction)` 实现 `design.md` §5.1 的 7 条判定与迁移规则。
- [x] 编辑类命令前置 `end_history_browse()`；`Escape` 在浏览态消费并退出浏览。
- [x] `on_turn_committed` 发送成功后清浏览态。

## 4. 持久化

- [x] `SCHEMA_VERSION` 9 → 10；`src/db/migration.rs` 新增 `SCHEMA_V10` 与 `version < 10` 分支。
- [x] 新增 `src/db/history.rs`：`load_prompt_history(limit)`、`record_prompt_history(text, limit)`。
- [x] `record_prompt_history` 单事务：`ON CONFLICT(text) DO UPDATE` + 裁剪最旧；
      空串返回 `DbError::InvalidDraft`。
- [x] `Effect::RecordPromptHistory { text }`；`src/db/service.rs` 出箱，失败仅 `tracing::warn`。
- [x] `on_turn_committed` 内同步 `state.prompt_history.record(&frozen_draft)` 并压效应。
- [x] `Persistence` 暴露 `load_prompt_history()`；事件层首次构造 `AppState` 后 `replace_all`。
- [x] DB 不可用时全部路径安全降级（`with_db` 返回 None，内存历史照常工作）。

## 5. 精确测试

- [x] `PromptHistory`：空/空白/超长过滤、去重置顶、上限裁剪、`replace_all` 顺序。
- [x] `Lx` 历史游标：首次 `↑` 取最新、连续 `↑` 钳制、`↓` 越界恢复草稿、
      光标非边界不接管、浏览中编辑退出、`Esc` 退出不改文本、
      同文本不压快照、恢复后 `Ctrl+Z` 回上一份文本。
- [x] `apply_lx_input`：四类面板各自打开时 `↑`/`↓` 归面板；无面板时历史/光标移动分派。
- [x] 多窗格：A 窗格浏览不影响 B 窗格游标；新记录对两窗格可见。
- [x] DB：v9→v10 升级、二次打开幂等、去重置顶、裁剪 100、顺序、空串拒绝。
- [x] 服务/事件：`on_turn_committed` 压效应 + 内存记录；`on_turn_failed` 不记录。

## 6. 交付

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿并记录结果。
- [x] 在工作区提交（`feature/prompt-history`）。
- [ ] 请用户运行 `cargo run` 验收交互与观感。
- [ ] 询问用户是否合并回 `dev`。
