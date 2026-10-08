# Prompt 编辑器工具栏与状态信息 — 执行任务

- 分支：`feature/prompt-controls`（已合并进 `dev`，合并提交 `59b0c72`）
- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案收敛至 `ui/text.rs`；颜色从 `ui/style.rs` 取。

## 执行清单

### 0. 需求盘问与规格确认（已完成）
- [x] 盘问 1：md 保存状态数据源与保存语义（纯内存脏标记，单字符彩色圆点 `●`，无文字）
- [x] 盘问 2：工具栏与分割线物理位置与视口影响（占 inner 顶部 2 行，分割线贯穿，全英文文案）
- [x] 盘问 3：工具栏按钮排列顺序与「全选」行为（左侧 `[undo] [redo]`，右侧 `[select all]` 生成全选 Selection）
- [x] 盘问 4：边框状态信息几何与数据源（顶边框右侧 `ws:` + 底边框左侧 `b:`/`wt:`，自右向左降级）
- [x] **用户正式审核确认 design.md 与 task.md**

### 1. 状态模型（app 层）
- [x] `src/app/prompt.rs`：
  - [x] 增加 `is_saved: bool` 字段并在修改动作中置 `false`
  - [x] 暴露 `is_saved(&self)`、`mark_saved(&mut self)`
  - [x] 暴露 `can_undo(&self)`、`can_redo(&self)`
- [x] `src/app/selection.rs`：
  - [x] 增加 `Selection::full(pane, max_row, max_col)` 构造器
- [x] `src/app/state.rs`：
  - [x] 增加 `AppState::select_all_prompt(&mut self)` 行为
  - [x] 增加 `WorkspaceGit.main_branch` 与 `status_branch()`（linked worktree 显示主 checkout 分支）
- [x] 单元测试：`src/app/prompt/tests.rs`、`src/app/selection/tests.rs`、`src/app/state/tests.rs`

### 2. 布局模型（layout 层）
- [x] `src/layout.rs`：
  - [x] 声明常量 `PROMPT_TOOLBAR_HEIGHT` (1)、`PROMPT_DIVIDER_HEIGHT` (1)、`PROMPT_HEADER_HEIGHT` (2)
  - [x] 调整 `prompt_text_rect`：在高度充足时下移 2 行，高度减 2
  - [x] 调整 `prompt_scrollbar_rect`：与文本区起始行及高度保持一致
  - [x] 新增 `prompt_header_rect` / `prompt_toolbar_rect` / `prompt_divider_rect` / `prompt_toolbar_button_rect` / `prompt_toolbar_button_at`
- [x] 单元测试：`src/layout/tests.rs`

### 3. 渲染实现（ui 层）
- [x] `src/ui/text.rs`：
  - [x] 增加 `PROMPT_TITLE = "Prompt"`、`PROMPT_UNDO_LABEL`、`PROMPT_REDO_LABEL`、`PROMPT_SELECT_ALL_LABEL`、`PROMPT_SAVE_DOT`
  - [x] 增加 `PROMPT_BRANCH_PREFIX = "b:"`、`PROMPT_WORKSPACE_PREFIX = "ws:"`、`PROMPT_WORKTREE_PREFIX = "wt:"`
- [x] `src/ui/style.rs`：
  - [x] 增加 `status_dot(saved: bool) -> Style`（保存为 Green，未保存为 Yellow）
  - [x] 增加 `border_title()`（淡蓝色：Cyan + dim、不加粗）并统一全部边框标题
- [x] `src/ui/prompt.rs`：
  - [x] 渲染工具栏行（左侧 `[undo] [redo]`、右侧 `[select all] ●`）
  - [x] 渲染固定分割线（`─` 贯穿，左右衔接 `├` 和 `┤`，样式跟随边框焦点态）
- [x] `src/ui/mod.rs`：
  - [x] 顶边框右侧右对齐 `ws:路径末段名`（前缀 accent、值 muted）
  - [x] 底边框左侧 `b:分支` 与 linked worktree 的 ` wt:工作区名`，避让右端折叠按钮
  - [x] `Agents` 表头标题对齐边框标题色
- [x] 单元测试：`src/ui/prompt/tests.rs`、`src/ui/tests.rs`

### 4. 事件与交互（event 层）
- [x] `src/event/mod.rs`：
  - [x] 鼠标左键点击命中 `PromptToolbarButton`：
    - `Undo`：触发 `prompt.undo()`
    - `Redo`：触发 `prompt.redo()`
    - `SelectAll`：触发 `state.select_all_prompt()`
    - `Save`：触发 `state.prompt.mark_saved()`
  - [x] 点击工具栏空白或分割线行时阻止穿透到文本视口定位光标
- [x] 单元测试：`src/event/tests.rs`

### 5. 文档规范同步与回归测试
- [x] 更新 `docs/standards/tui-design-requirements.md`（同步 prompt 工具栏、淡蓝色边框标题与 `ws:`/`b:`/`wt:` 状态规范）
- [x] 精确编译与校验：
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`（646 单元测试 + 集成测试全绿）
