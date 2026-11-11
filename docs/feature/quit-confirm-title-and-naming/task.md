# 退出二次确认、模板块标题生成与 UI 命名整理 — 执行任务清单

> 状态：**已实现、已 review 修复、已合并至 `dev`；退出提示已改为全局 toast；原工作区已清理；待用户 `cargo run` 人工验收**
>
> 设计文档：`docs/feature/quit-confirm-title-and-naming/design.md`
>
> Review 文档：`docs/feature/quit-confirm-title-and-naming/review.md`
>
> 执行原则：阶段之间独立收口；阶段 3 只做文件移动/路径修正，不夹带行为修改。

---

## 阶段 0：审核与基线

- [x] 用户审核 `design.md` 与 `task.md`，确认三块需求的边界与文件映射。
- [x] 确认当前工作区无未提交改动（`git status --short`），避免结构改名与用户改动混在一起。
- [x] 记录基线：`cargo test` 当前是否全绿；若有既有失败，先记录失败用例，不归因于本次改动。
- [x] 文件移动使用普通 `mv`；最终提交在用户明确授权后执行（已完成 commit `d6d1a159`）。

**验收**：用户明确回复“可以执行”。

---

## 阶段 1：退出二次确认

> **历史计划（已两次修订）**：初版 `Overlay::ConfirmQuit` + `suspended_overlay` 会丢失异步回执；review 后曾改为独立 `quit_confirm` 层；最终按要求**移除退出模态**，改为全局 toast + 2s 确认窗口。下列 1.1-1.3 仅保留当时计划，当前实现见 `design.md` 第 10.1 节。
>
> **当前实现**：`quit_requested_at: Option<Instant>` + `QUIT_CONFIRM_WINDOW`；首次请求 `show_toast(anchor=None)`，窗口内第二次 `Ctrl+Q`/`[EXIT]` 退出；无 `cancel_quit`、无 Esc/Enter 确认、无模态拦截。

### 1.1 状态与 Action

- [x] `src/app/actions.rs`：`Action::Quit` 拆为 `RequestQuit` / `ConfirmQuit` / `CancelQuit`，更新所有构造点与测试。
- [x] `src/app/overlay.rs`：`OverlayKind` 增加 `ConfirmQuit`；`Overlay` 增加 `ConfirmQuit` 变体；`kind()`、`input()`、`input_mut()` 等匹配分支补全。
- [x] `src/app/state.rs`：`AppState` 增加 `suspended_overlay: Option<Overlay>`；`demo()` / 初始化处补 `None`。
- [x] 新增 `src/app/update/quit.rs`：`request_quit` / `confirm_quit` / `cancel_quit`。
- [x] `src/app/update.rs`：声明 `mod quit;`，`apply` 分派三个新 Action。
- [x] `src/app/update/menu.rs`：`close_overlay` 遇到 `ConfirmQuit` 时转 `cancel_quit`。

### 1.2 键盘

- [x] `src/input/mod.rs`：`Ctrl+Q` 按当前 `overlay` 返回 `RequestQuit` 或 `ConfirmQuit`。
- [x] `src/input/mod.rs`：`overlay_key` 增加 `OverlayKind::ConfirmQuit` 的 `Esc` / `Enter` 映射，其余键吞掉。
- [x] `src/app/update/dialog.rs`：`apply_overlay_key` 最前面处理 `ConfirmQuit`，必须在 `escape_clear_selection` 之前。
- [x] `src/event/input.rs`：裸 `Esc` 清选区的提前返回增加 `!matches!(state.overlay, Some(Overlay::ConfirmQuit))` 前置条件，确保确认浮层下 `Esc` 归取消退出。

### 1.3 渲染与鼠标

- [x] `src/ui/text/workspace.rs`：新增 `EXIT_CONFIRM_TITLE`、`EXIT_CONFIRM_QUESTION`（英文），`ui/text.rs` 保持 ≤1000 行。
- [x] `src/ui/overlay/dialog.rs`：新增 `quit_confirm_shell`、`render_quit_confirm`；复用 `CONFIRM_BUTTONS` / `render_buttons`。
- [x] `src/ui/mod.rs`：在底层 overlay → toast/tooltip 之后绘制独立 `quit_confirm` 层；短屏跳过标题并裁剪问题行。
- [x] `src/event/overlay.rs`：`handle_overlay_click` 优先处理 `quit_confirm`；点击弹窗外不关闭；确认/取消按钮分别调用 `confirm_quit` / `cancel_quit`。
- [x] `src/event/mouse/press.rs`：以 `state.quit_confirm.is_some()` 提前返回，避免取消退出后误触发 `reveal_active_tab`；拖拽/松开/滚轮同时拦截。
- [x] 确认 `[EXIT]` 按钮点击路径仍走 `Action::RequestQuit`（`src/event/mouse/press.rs`）。
- [x] 确认 `Ctrl+Q` 在 `/connect`、重命名、Todo 等焦点/浮层下都优先进入退出确认，而不是直接退出。

### 1.4 测试

- [x] `src/input/tests.rs`：
  - `ctrl_q_requests_quit`；
  - `ctrl_q_confirms_when_confirm_quit_open`；
  - `confirm_quit_routes_esc_and_enter`。
- [x] `src/app/update/tests.rs`：
  - `request_quit_does_not_set_should_quit_or_commit`；
  - `confirm_quit_sets_should_quit_and_commits`；
  - `cancel_quit_preserves_underlying_overlay`；
  - `request_quit_leaves_underlying_overlay_untouched`；
  - `quit_confirm_survives_trust_resolution`。
- [x] `src/app/update/prompt/tests.rs`：
  - 原 `quit_commits_pre_paste_edits_without_placeholder` 改为两步（Request → Confirm）后断言。
- [x] `src/event/tests/`：
  - 点击 `[EXIT]` 打开确认；
  - 点击确认退出；
  - 点击取消恢复；
  - 点击弹窗外不关闭；
  - `Esc` 取消、`Enter` 确认；
  - 已有浮层时 `Ctrl+Q` → 取消后原浮层仍在。
- [x] `src/ui/overlay/dialog/tests.rs`：
  - `quit_confirm_shell` 在窄屏/矮屏下的几何；
  - `confirm_button_at` 命中确认/取消；
  - 渲染快照包含 `Exit lx` 与两个按钮。

### 1.5 验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test input::tests
cargo test app::update::tests
cargo test app::update::prompt::tests
cargo test event::tests
cargo test ui::overlay
```

**阶段验收**：

- [x] 第一次 `Ctrl+Q` / `[EXIT]` 只打开确认；`should_quit == false`，编辑器内容未提交。
- [x] 确认后只提交一次编辑器并退出。
- [x] 取消后恢复原浮层，编辑器内容与光标不变。
- [x] 弹窗打开时再次 `Ctrl+Q` 直接退出。
- [x] 点击弹窗外不关闭。

---

## 阶段 2：模板块标题生成

### 2.1 内容清洗

- [x] `src/app/markdown/template.rs`：
  - 重写 `extract_block_title_source`：子块整块剔除、围栏优先、标题行只跳首个、默认大标题最后过滤；
  - 新增 `template_title_source(body: &str) -> String`；
  - 新增 `supple_title_source(outer_body: Option<&str>, supple_body: &str) -> String`；
  - 保留 `copy_template_content` 原语义不变。
- [x] `src/app/prompt/panel.rs`：
  - `confirm_generate_block_title` 改用新纯函数；
  - 新增 `enclosing_template_body`，用 `block_scan` 定位外层 `&&&`；
  - 保持“空内容 → `PromptTitleAction::EmptyBlock`”行为。
- [x] 确认 `+++` 嵌套在 `&&&` 内时，外层正文剔除全部 `+++`（含当前），再追加当前 `+++` 正文。
- [x] 确认顶层 `+++` 只取自身正文。

### 2.2 提示词

- [x] `src/chat/title.rs`：
  - 抽取 `resolve_title_language`；
  - 新增 `TEMPLATE_TITLE_PROMPT_MAX_BYTES = 2048`；
  - 新增 `template_title_instructions_for`，追加中文/英文 few-shot 与“动词 + 具体对象”约束；
  - `template_block_title_prompt_for` 改用新指令与新预算；
  - `session_title_prompt_for` / `title_instructions_for` 行为保持不变；
  - `sanitize_title` 增加开头列表标记（`- ` / `* ` / `+ ` / `1. `）剥离。
- [x] 确认所有新增提示词文本为英文 ASCII（few-shot 示例除外，示例本身是中文/英文标题文本）。
- [x] 确认 `template_title_instructions_for` 只被模板块标题使用，不泄漏到会话标题。

### 2.3 测试

- [x] `src/app/markdown/tests.rs` 或 `src/app/markdown/template/tests.rs`：
  - `template_title_source_excludes_nested_supple_and_logs`；
  - `template_title_source_keeps_fenced_subblock_markers`；
  - `supple_title_source_includes_outer_template_without_siblings`；
  - `supple_title_source_top_level_uses_own_body_only`；
  - `template_title_source_empty_boilerplate_is_empty`。
- [x] `src/app/prompt/block/tests.rs`：
  - 嵌套 `+++` 的 `&&&` `/title` 的 `content` 只含外层任务；
  - `+++` `/title` 的 `content` 含外层任务 + 当前补充；
  - 兄弟 `+++` 不出现。
- [x] `src/chat/title/tests.rs`：
  - `template_prompt_contains_db_style_examples`；
  - `session_prompt_has_no_template_examples`；
  - `template_prompt_clamps_to_2048_and_keeps_utf8_boundary`；
  - 语言分支仍正确（简中/繁中/日/韩/英/Auto）。
- [x] `src/app/update/chat/title/tests.rs`：
  - `content` 从 `PromptTitleAction` 到 `template_block_title_prompt_for` 的端到端断言。

### 2.4 验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test app::markdown
cargo test app::prompt
cargo test app::update::chat::title
cargo test chat::title
```

**阶段验收**：

- [x] `&&&` 标题输入不包含任何嵌套 `+++` / `%%%` / `+++ log` 正文。
- [x] `+++` 标题输入包含外层 `&&&` 正文与当前 `+++` 正文，不包含其他 `+++`。
- [x] 代码围栏内的 `+++` / `%%%` 不被误删。
- [x] 提示词总长 ≤ 2048 字节，UTF-8 截断不 panic。
- [x] 会话标题提示词行为与预算不变。
- [x] 无模糊词黑名单、无自动重试。

---

## 阶段 3：UI 命名与结构整理

> 本阶段只做文件移动、`mod`/`use` 路径修正、`ViewLayout` 字段改名与两个超限文件拆分；不改行为。

### 3.1 根级 layout → tiling

- [x] `src/layout.rs` → `src/tiling.rs`；删除 `mod prompt; pub use prompt::*;`。
- [x] `src/layout/tests.rs` → `src/tiling/tests.rs`，保留 BSP 测试。
- [x] `src/layout/prompt.rs` → `src/ui/right_panel/prompt/geometry.rs`。
- [x] `src/layout/tests.rs` 中 15 个 `prompt_*` 测试 → `src/ui/right_panel/prompt/geometry/tests.rs`；若依赖 `layout/tests.rs` 的共用 helper，随测试一并迁移或下沉为测试局部函数。
- [x] `src/lib.rs`：`pub mod layout;` → `pub mod tiling;`。
- [x] 全局替换 `crate::layout::` → `crate::tiling::`，保留 `crate::ui::layout` 与 `super::layout`（UI 屏幕分区）。

### 3.2 左栏 / 右栏 / 主内容

- [x] `src/ui/sidebar.rs` → `src/ui/left_panel/mod.rs`。
- [x] `src/ui/sidebar/history.rs` → `src/ui/left_panel/history.rs`。
- [x] `src/ui/sidebar/tests.rs` → `src/ui/left_panel/tests.rs`。
- [x] `src/ui/right_panel.rs` → `src/ui/right_panel/mod.rs`。
- [x] `src/ui/prompt.rs` → `src/ui/right_panel/prompt/mod.rs`。
- [x] `src/ui/prompt/{header,pages,panels,recent,tree,tests}.rs` → `src/ui/right_panel/prompt/`。
- [x] `src/ui/prompt_frame.rs` → `src/ui/right_panel/prompt/frame.rs`。
- [x] `src/ui/panes.rs` → `src/ui/main_content/panes.rs`。
- [x] `src/ui/main_content.rs` → `src/ui/main_content/mod.rs`，声明 `pub(crate) mod panes;`。
- [x] `src/ui/mod.rs`：更新 `pub mod` / `pub use`；移除 `prompt_frame` 顶层导出。
- [x] 全局替换：
  - `ui::sidebar` → `ui::left_panel`；
  - `ui::prompt_frame` → `ui::right_panel::prompt::frame`；
  - `crate::layout::prompt_*` → `ui::right_panel::prompt::geometry::*`；
  - `ui::panes` → `ui::main_content::panes`。
- [x] 更新 `event/`、`ui/`、`app/` 中所有旧路径 import；确保 `app` 不依赖 `ui`。实际落点：app 使用的纯几何下沉到 `src/app/prompt/geometry.rs`，UI 从 app 取。

### 3.3 controls 拆分

- [x] `src/ui/controls.rs` → `src/ui/controls/mod.rs`。
- [x] 新增 `controls/panels.rs`：折叠按钮、Agents 按钮、Prompt pin、折叠窄条、`collapsed_content_x`。
- [x] 新增 `controls/footer.rs`：设置/用量按钮。
- [x] 新增 `controls/exit.rs`：`[EXIT]` 按钮。
- [x] 新增 `controls/swap.rs`：交换按钮。
- [x] 新增 `controls/resize.rs`：拖拽提示与高亮。
- [x] `controls/tests.rs` 保持一个测试模块；通过 `super::*` 访问拆出的函数。
- [x] `src/ui/mod.rs` 的 `pub use controls::*` 仍能导出所有公开几何/命中函数。

### 3.4 `ViewLayout` 字段改名

- [x] `src/ui/layout.rs`：
  - `sidebar` → `left_panel`；
  - `prompt` → `right_panel`；
  - `panes` → `main_content`；
  - `prompt_left` → `right_panel_left`。
- [x] 全局替换 `view.sidebar` / `view.prompt` / `view.panes` / `view.prompt_left`（约 558 处）。
- [x] 确认 `AppState.sidebar_*` / `prompt_*` / `panes_*` 字段名保持不变。
- [x] 全局搜索旧字段名，确认零残留（排除 `AppState` 字段与业务变量）。

### 3.5 超限生产文件拆分

#### `src/chat/process/manager/mod.rs`

- [x] 新增 `manager/types.rs`：`ProcessLimits`、`ProcessUiEvent`、`ProcessError`、`ProcessSnapshot`。
- [x] 新增 `manager/handle.rs`：`ProcessHandle` 及其 `impl`。
- [x] 新增 `manager/lifecycle.rs`：`StopReason`、`stop_handle`、`resolve_cwd`、`StopWaiter`。
- [x] `manager/mod.rs`：保留 `ProcessManager`、`next_process_id`、`global_running`、`spawn_pipe_reader`、`exit_state`、`estimated_tokens` 与 `pub use`。
- [x] 函数体逐字搬迁，不改逻辑；`manager/spawn.rs` 不动。
- [x] 目标：`mod.rs` ≤ 700 行。

#### `src/app/update/prompt.rs`

- [x] 新增 `update/prompt/items.rs`：条目/临时提示词生命周期（编辑、打开、保存、删除、复制）。
- [x] 新增 `update/prompt/menu.rs`：右键菜单打开、重命名、删除确认。
- [x] 新增 `update/prompt/save.rs`：编辑缓冲提交、页持久化标记、自动保存复位。
- [x] `update/prompt/mod.rs`：树模式切换、`apply_prompt_tree`、`show_prompt_tree`、`stash_editing_prompt`、`select_tree_entry_row` 与 `pub use`。
- [x] 现有 `draft.rs` / `drag.rs` / `pages.rs` / `pages_view.rs` / `recent.rs` / `tree_view.rs` 不动。
- [x] 测试按项目标准跟随模块拆分；本次新增 quit 测试落 `app/update/quit/tests.rs`。既有巨型 `update/prompt/tests.rs` 未在本期强行拆分，已记录在 review P2-6。
- [x] 目标：`mod.rs` ≤ 500 行。

### 3.6 文档同步

- [x] `docs/standards/project-directory-structure.md`：更新目录树、模块职责、依赖方向中的路径（`layout` → `tiling`，`sidebar` → `left_panel`，`prompt` 收进 `right_panel`，`panes` 收进 `main_content`，`controls/` 子模块）。
- [x] `docs/standards/tui-design-requirements.md`：更新 `sidebar` / `right_panel` / `prompt` / `main_content` 路径描述。
- [x] `docs/standards/code-writing-standards.md`：更新涉及 `ui::main_content::lx`、`app::update` 的路径引用（如有）。
- [x] `AGENTS.md`：补充 `left_panel` / `right_panel` / `main_content` 目录约定（如需要）。
- [x] 编写 `docs/feature/quit-confirm-title-and-naming/review.md`，按 P0-Pn 记录关键点、错误原因与修复结果。
- [x] 不重写历史 `docs/feature/*` 文档。

### 3.7 验证

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- [x] `rg "crate::layout"` 只余 `crate::layout` 相关误报为零（或仅剩文档字符串）。
- [x] `rg "ui::sidebar|ui::prompt_frame|ui::panes"` 零命中。
- [x] `rg "view\.(sidebar|prompt|panes|prompt_left)"` 零命中。
- [x] 全量测试通过。

**阶段验收**：

- [x] 目录结构与 `design.md` 6.1 一致。
- [x] `ui::left_panel` / `ui::right_panel::prompt` / `ui::main_content` 命名对称。
- [x] 两个超限生产文件均低于 1000 行（目标 700 / 500）。
- [x] 行为零变化：阶段 1、2 的测试仍全绿。

---

## 最终验收清单

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`（移除退出模态后 5222 个 lib 测试 + 全部集成测试）
- [ ] 用户 `cargo run` 人工验收（待执行）：
  - [ ] `Ctrl+Q` / `[EXIT]` 第一次只显示全局 `Exit lx` toast，编辑器内容不变；
  - [ ] 确认窗口内再次 `Ctrl+Q` / 点击 `[EXIT]` 退出；
  - [ ] 等待超过 2s 后再次请求，重新显示 toast 而不是退出；
  - [ ] 有底层弹窗时退出请求只显示 toast，底层弹窗仍可正常操作；
  - [ ] `&&&` `/title` 不被嵌套 `+++` / 日志带偏；
  - [ ] `+++` `/title` 包含外层任务 + 当前补充；
  - [ ] 左栏/右栏/主内容折叠、交换、拖拽、Prompt/Todo 切换正常；
  - [ ] 设置页、用量页、`/connect`、`/resume`、引用文件夹弹窗正常。

---

## 合并 dev 与清理工作区（2026-11-11）

- [x] 在 `dev` 上合并 `feat/quit-confirm-title-and-naming`。
- [x] 解决 `chat/process/manager/{mod,handle}.rs` 与 `docs/standards/project-directory-structure.md` 冲突：
  - `manager/` 采用 `mod.rs` + `types.rs` + `handle.rs` + `lifecycle.rs` + `spawn.rs`；保留 dev 的 50 KiB 输出边界与 `enforce_output_bounds`。
  - `project-directory-structure.md` 合并 `history/tool/group.rs`、`header.rs`、`right_panel/geometry.rs` 等描述。
- [x] 合并后 `ui/main_content/lx/history/tool.rs` 超过 1000 行，抽出 `history/tool/header.rs`（标题、分段、网格写入、折行原语），分组/树体行为不变。
- [x] `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全通过（移除退出模态后 5222 个 lib 测试）。
- [x] 更新 `design.md`、`review.md` 与相关 feature 文档的合并记录。
- [x] 删除 `.worktrees/quit-confirm-title-and-naming`、分支 `feat/quit-confirm-title-and-naming` 与隔离数据库。

---

## 交付物

- `docs/feature/quit-confirm-title-and-naming/design.md`
- `docs/feature/quit-confirm-title-and-naming/task.md`
- `docs/feature/quit-confirm-title-and-naming/review.md`
- 代码改动按阶段 1 → 2 → 3 顺序完成。
- 最终验证命令输出（`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`）。
