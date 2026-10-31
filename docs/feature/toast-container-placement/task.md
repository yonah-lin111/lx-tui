# Toast 容器归属与复制反馈位置重调整 — 执行任务清单

- **任务目录**：`docs/feature/toast-container-placement/`
- **设计文档**：`docs/feature/toast-container-placement/design.md`
- **工程约束**：严格遵守 `AGENTS.md` 及 `docs/standards/*` 规范；保持 `app/`（纯状态与更新）与 `ui/`（纯渲染）边界清晰；生产代码禁止 `unwrap()` / `expect()`；所有文案集中于 `src/ui/text.rs`；修改完成后仅针对受影响部分执行精准校验。

---

## 阶段一：需求质询与方案设计（已完成）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索全应用的 Toast 状态槽、渲染定位逻辑与所有剪贴板复制入口
- [x] **Grill Me 问题 1**：确认 `src/ui/main_content/`（lx 窗格与终端窗格）、`src/ui/right_panel/`（Prompt 与 Todo）与左侧栏（`src/ui/sidebar/`）的复制 Toast 容器归属规则
- [x] **Grill Me 问题 2**：确认居中模态弹窗（Overlay）内触发复制时保持显示在全屏顶部正中央（全局 Toast）
- [x] **Grill Me 问题 3**：确认将右侧面板剩余 2 处非复制保存失败 Toast（`SavePromptContent` 失败与 `fail_todo_write`）统一锚定到右侧面板 `Some(state.prompt.id())`
- [x] 编写详细设计文档 `docs/feature/toast-container-placement/design.md` 与执行任务清单 `docs/feature/toast-container-placement/task.md`

---

## 阶段二：事件层与状态层 Toast 容器锚点改造（已完成）

### 2.1 事件层剪贴板复制入口（`src/event/`）
- [x] **`src/event/mouse/hit.rs`**：
  - 将 `copy_with_toast(state: &mut AppState, text: &str)` 签名为 `copy_with_toast(state: &mut AppState, text: &str, anchor: Option<PaneId>)`
  - 内部改调 `update::show_toast_at(state, anchor, Toast::new(kind, message, anchor, now).with_title(ui::text::TOAST_CLIPBOARD_TITLE))`
  - 在 `apply_template_button`（`TemplateBlockButton::Copy`）中传入 `Some(state.prompt.id())`
  - 在 `apply_block_button`（`(BlockKind::Supple, BlockButton::Copy)`）中传入 `Some(state.prompt.id())`
- [x] **`src/event/input.rs`**：
  - `Routed::Overlay(OverlayKey::Clear)` 选区复制传入 `None`
  - `Routed::Copy`（Prompt 编辑器选区/全文复制）传入 `Some(state.prompt.id())`
  - `Routed::CopyTemplateBlock`（Prompt 模板块快捷键复制）传入 `Some(state.prompt.id())`
  - `Routed::Todo(TodoKey::Clear)`（Todo 行内编辑框选区复制）传入 `Some(state.prompt.id())`
  - `copy_terminal_selection`（终端窗格选区复制）传入 `Some(pane)`
  - `copy_lx_selection`（lx 窗格输入框/Ask 编辑框/历史与任务选区复制）两处分支均传入 `Some(pane)`

### 2.2 状态更新层与持久层效应分派（`src/app/update/`、`src/db/service.rs`）
- [x] **`src/app/update/prompt.rs`**：
  - `copy_prompt_content` 中 `Effect::CopyToClipboard` 的 `anchor` 改为 `Some(state.prompt.id())`
  - `copy_prompt_temp_content` 中 `Effect::CopyToClipboard` 的 `anchor` 改为 `Some(state.prompt.id())`
- [x] **`src/app/update/menu.rs`**：
  - `activate_menu` 中 `(Some(MenuCommand::CopyPath), OverlayTarget::Pane { pane, .. })` 分支的 `Effect::CopyToClipboard` `anchor` 改为 `Some(pane)`
  - `(Some(MenuCommand::CopyPath), OverlayTarget::Workspace(_))` 分支保持 `anchor: None`
- [x] **`src/app/update/todo.rs`**：
  - `fail_todo_write` 中 `Toast::new` 的 `anchor` 从 `None` 改为 `Some(state.prompt.id())`
- [x] **`src/db/service.rs`**：
  - `Effect::CopyToClipboard { text, anchor }` 分支透传 `anchor`，并改调 `update::show_toast_at(state, anchor, Toast::new(kind, message, anchor, now).with_title(text::TOAST_CLIPBOARD_TITLE))`
  - `Effect::SavePromptContent` 失败分支的 `Toast::new` `anchor` 从 `None` 改为 `Some(state.prompt.id())`

---

## 阶段三：测试更新、新增回归测试与规范文档同步（已完成）

- [x] **更新现有测试断言**：
  - `src/app/update/tests.rs`（更新终端/lx 窗格右键 `CopyPath` 的 `anchor` 断言为 `Some(pane)`，保留工作区右键 `CopyPath` 的 `anchor: None` 断言）
  - `src/app/update/prompt/tests.rs`（验证条目与临时提示词右键复制产生的 `Effect::CopyToClipboard` 携带 `anchor: Some(state.prompt.id())`）
  - `src/event/tests/lx_view.rs`（更新 lx 历史选区与任务输出选区 `Ctrl+C` 复制断言：Toast 落在 `pane.lx.toast` 且 `state.toast.is_none()`）
  - `src/event/tests/terminal.rs`（更新终端选区 `Ctrl+C` 断言 `toast.anchor == Some(pane)`；更新双击 lx 输入框复制断言落在 `pane.lx.toast`；补充右侧 Prompt 编辑器复制断言 `toast.anchor == Some(state.prompt.id())`）
  - `src/event/tests/prompt_editor.rs`（在 `mouse_supple_copy_button_writes_clipboard_toast` 中补充断言 `toast.anchor == Some(state.prompt.id())` 且渲染矩形位于 `geo.view.prompt` 容器内）
- [x] **新增回归测试**：
  - 验证 Todo 编辑框复制及 `fail_todo_write` 均锚定至 `Some(state.prompt.id())`
  - 验证 `Effect::CopyToClipboard` 按 `anchor` 分别路由到 `pane.lx.toast`、终端窗格 `state.toast`、右侧面板 `state.toast` 与全局 `state.toast`
- [x] **同步规范文档**：
  - 更新 `docs/standards/tui-design-requirements.md` 中“选择与复制 → 复制反馈”章节的描述
- [x] **执行精准校验**：
  - `rtk cargo fmt --check`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - 运行受影响模块的单元测试与集成测试
