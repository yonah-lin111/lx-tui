# 左侧栏工作区列表（`ws list`）删除二次点击确认、输入口令弹窗与一键清理失效项 — 执行任务清单 (`task.md`)

- **状态**：已在工作区 `.worktrees/workspace-history-delete-confirm`（分支 `feature/workspace-history-delete-confirm`）完成实现与测试，待用户确认合并
- **设计依据**：`docs/feature/workspace-history-delete-confirm/design.md`

---

## 0. 开工门禁与硬性规范

- **门禁**：必须等用户审核并明确确认本目录下的 `design.md` 与 `task.md` 后，才可修改 `src/` 与 `tests/` 代码。
- **架构红线**：
  - `app/` 只管纯内存状态与状态转换，严禁在 `ui/` 渲染帧内调用 `Path::is_dir()` 或修改状态；
  - 所有 TUI 可见文案统一使用英文并集中定义在 `src/ui/text/`；
  - 生产代码单文件严禁超过 1000 行，严禁使用 `unwrap()` / `expect()`；测试文件跟随子模块拆分；
  - 未获明确授权禁止执行任何修改 Git 状态的命令。

---

## 1. 需求与边界确认清单（Grill Me 已确认）

- [x] **Q1（菜单位置、显示条件与 2 次点击确认）**：
  - `Delete missing` 同时出现在 `ws 列表`（`WorkspaceMode::History`）的**条目右键菜单底部**（`[Open, Rename, Delete, Delete missing]`）与**空白处右键菜单底部**（`[New workspace, Delete missing]`）；
  - 打开菜单时实时刷新存在性，仅当存在至少 1 个路径不存在（`!path.is_dir()`）的条目时才显示 `Delete missing`，否则隐藏；
  - 单条 `Delete`（由原 3 击改为 2 击）与 `Delete missing` 均在菜单内点击 2 次（第 1 次切换为 `Confirm delete` / `Confirm delete missing`，第 2 次弹出确认模态弹窗）。
- [x] **Q2（`Delete missing` 删除范围与级联边界）**：
  - 严格只删除磁盘上路径已不存在（`!path.is_dir()`，列表中红字）的条目本身及其数据库关联数据，绝不连带删除磁盘上仍存在的任何同仓库主工作区或子 worktree；
  - 作用范围为整个 `ws 列表`（`workspace_history.entries`）的全部失效条目，不受顶部搜索框过滤词限制。
- [x] **Q3（弹窗确认输入短语、占位提示与匹配规则）**：
  - 确认短语固定为 `confirm delete`；
  - 输入框占位提示（placeholder）为 `type "confirm delete" to confirm`；
  - 匹配规则为去除首尾空格后不区分 ASCII 大小写（`input.text().trim().eq_ignore_ascii_case("confirm delete")`）。
- [x] **Q4（弹窗按钮样式与触发行为）**：
  - 底部按钮为 `[Confirm]  [Cancel]`；未匹配 `confirm delete` 时 `[Confirm]` 为暗色禁用态（`style::muted()`），匹配后高亮为危险红色（`style::button_danger()`）；
  - 未匹配时点击 `[Confirm]` 或按 `Enter` 均无反应、保持弹窗打开；匹配后既可鼠标点击 `[Confirm]` 执行删除，也可按 `Enter` 执行删除；`Esc`、点击 `[Cancel]` 或点击弹窗外部空白处取消并关闭弹窗。
- [x] **Q5（弹窗待删除工作区列表与滚动）**：
  - 在问题文案下方、输入框上方展示待删除工作区列表，每行格式为 `  • <name> (<path>)`；单条删除主仓库时一并列出级联子 worktree，`Delete missing` 时列出全部失效工作区；
  - 列表区域最多展示 5 行（`CONFIRM_DELETE_LIST_MAX_ROWS = 5`），超过 5 行时显示右侧滚动条并支持鼠标滚轮、键盘 `Up`/`Down` 及滚动条点击/拖拽滚动。

---

## 2. 分阶段实施步骤拆解

### Phase 1：文案子模块拆分与浮层/历史面板状态扩展（`src/ui/text/` & `src/app/`）

> 目标：在 `src/ui/text/workspace.rs` 定义新增英文文案并降低 `src/ui/text.rs` 行数；在 `src/app/overlay.rs` 与 `src/app/state/workspace_history.rs` 完成 `ConfirmClose` 输入框、`MenuCommand` 与 `OverlayTarget` 建模。

- [x] **1.1 拆分 `src/ui/text/workspace.rs` 并新增英文文案常量**
  - 新建 `src/ui/text/workspace.rs`，将 `src/ui/text.rs` 中工作区、侧栏、右键菜单、粘贴面板及身份标签相关文案迁入，并在 `src/ui/text.rs` 中 `pub mod workspace; pub use workspace::*;`，确保所有 `text::*` 调用路径完全兼容且 `src/ui/text.rs` 降至 1000 行以下；
  - 移除已废弃的 `MENU_CONFIRM_DELETE_STEP1`、`MENU_CONFIRM_DELETE_STEP2`；
  - 新增常量与函数：
    - `MENU_DELETE_MISSING_WORKSPACES = "Delete missing"`
    - `MENU_CONFIRM_DELETE_MISSING_WORKSPACES = "Confirm delete missing"`
    - `CONFIRM_DELETE_MISSING_WORKSPACES_TITLE = "Delete Missing Workspaces"`
    - `CONFIRM_DELETE_INPUT_PLACEHOLDER = "type \"confirm delete\" to confirm"`
    - `confirm_delete_missing_workspaces_question(count: usize) -> String`
- [x] **1.2 更新 `src/app/overlay.rs` 与 `src/app/state/workspace_history.rs`**
  - 在 `src/app/overlay.rs` 中：
    - 移除 `MenuCommand::DeleteWorkspaceHistoryStep1` 与 `DeleteWorkspaceHistoryStep2`；
    - 新增 `MenuCommand::DeleteMissingWorkspaces` 与 `OverlayTarget::MissingWorkspaces`；
    - 将 `MenuCommand::DeleteWorkspaceHistory` 与 `MenuCommand::DeleteMissingWorkspaces` 纳入 `MenuCommand::requires_confirm()`；
    - 为 `ConfirmClose` 增加 `pub input: TextInput` 字段、`CONFIRM_DELETE_PHRASE = "confirm delete"`、`ConfirmClose::new(target)` 与 `ConfirmClose::is_confirmed(&self) -> bool`；
    - 在 `Overlay::input`、`Overlay::input_mut`、`Overlay::clear_selection` 中接入 `Self::ConfirmClose(c)`。
  - 在 `src/app/state/workspace_history.rs` 中：
    - 新增 `has_missing_entries(&self) -> bool` 与 `missing_entry_count(&self) -> usize`。

---

### Phase 2：工作区删除子模块拆分、右键菜单 2 击与弹窗提交逻辑（`src/app/update/`）

> 目标：将 `src/app/update/workspace_history.rs`（992 行）按职责拆分为 `mod.rs` + `delete.rs`，实现 `delete_missing_workspace_history_entries`、菜单 2 击弹窗及 `confirm_close` 口令校验。

- [x] **2.1 拆分 `src/app/update/workspace_history/` 并实现批量失效清理**
  - 将 `src/app/update/workspace_history.rs` 转为 `src/app/update/workspace_history/mod.rs` + `src/app/update/workspace_history/delete.rs`；
  - 在 `delete.rs` 中提取共用清理函数 `purge_workspace_history_paths(state, paths_to_delete)`，供 `delete_workspace_history_entry` 与新增的 `delete_missing_workspace_history_entries` 复用；
  - `delete_missing_workspace_history_entries(state)` 实时刷新 `refresh_existence()`，严格仅收集 `!entry.exists` 的路径执行清理，不连带删除任何磁盘上仍存在的同仓库工作区；
  - 将原 `workspace_history/tests.rs` 中与删除相关的测试迁入 `src/app/update/workspace_history/delete/tests.rs`。
- [x] **2.2 更新 `src/app/update/menu.rs` 与 `src/app/update/dialog.rs`**
  - `open_workspace_history_menu`：刷新存在性，当 `has_missing_entries()` 为真时在菜单末尾追加 `MenuCommand::DeleteMissingWorkspaces`；
  - `open_workspace_list_menu`：当处于 `workspace_history_mode(state)` 时刷新存在性，若 `has_missing_entries()` 为真则追加 `MenuCommand::DeleteMissingWorkspaces`；
  - 移除 `reset_workspace_delete_steps` 及 `activate_menu` 中的 Step1/Step2 分支；
  - `activate_menu` 第二次点击 `DeleteWorkspaceHistory` 或 `DeleteMissingWorkspaces` 时打开 `Overlay::ConfirmClose(ConfirmClose::new(...))`；
  - `src/app/update/dialog.rs` 的 `confirm_close`：校验 `confirm.is_confirmed()`，未通过则保持弹窗打开；通过后关闭弹窗并按 `OverlayTarget::WorkspaceHistory(i)` 或 `OverlayTarget::MissingWorkspaces` 执行对应删除。

---

### Phase 3：UI 渲染层与键盘/鼠标事件层闭环（`src/ui/`、`src/input/`、`src/event/`）

> 目标：完成菜单文案映射、`ConfirmClose` 口令输入框与动态按钮渲染、硬件光标同步，以及键盘编辑与鼠标点击/拖选闭环。

- [x] **3.1 更新菜单文案映射与 `ConfirmClose` 渲染（`src/ui/overlay/`）**
  - `src/ui/overlay/menu.rs`：
    - `MenuCommand::DeleteWorkspaceHistory`：`confirming` 为真时返回 `text::MENU_CONFIRM_DELETE`，否则返回 `text::MENU_DELETE_WORKSPACE_HISTORY`；
    - `MenuCommand::DeleteMissingWorkspaces`：`confirming` 为真时返回 `text::MENU_CONFIRM_DELETE_MISSING_WORKSPACES`，否则返回 `text::MENU_DELETE_MISSING_WORKSPACES`；
  - `src/ui/overlay/dialog.rs`：
    - `confirm_text` 与 `confirm_question` 增加 `OverlayTarget::MissingWorkspaces` 分支；
    - 新增 `confirm_close_shell(screen, question)`（高度 `lines + 7`）与 `confirm_close_input_rect(shell, question)`；
    - `render_confirm` 渲染危险边框、自动换行问题、下方单行输入框（占位提示 `CONFIRM_DELETE_INPUT_PLACEHOLDER`）及底部按钮（`confirm.is_confirmed()` 控制 `[Confirm]` 为 `style::button_danger()` 或 `style::muted()`），并返回硬件光标坐标 `Option<(u16, u16)>`；
  - `src/ui/overlay.rs`：`Overlay::ConfirmClose(confirm)` 分支返回 `dialog::render_confirm(frame, screen, state, confirm)` 的光标位置。
- [x] **3.2 更新键盘路由与鼠标命中（`src/input/mod.rs`、`src/event/overlay.rs`、`src/event/mouse/drag.rs`）**
  - `src/input/mod.rs`：将 `OverlayKind::ConfirmClose` 移入 `Rename | NewWorkspace | NewNode | ConfirmClose` 分支，放行 `overlay_text_edit_key(key)`；
  - `src/event/overlay.rs` & `src/event/mouse/drag.rs`：实现 `overlay_input_rect(state, screen)` 支持 `Overlay::ConfirmClose` 输入框点击定位光标、双击/三击选词与拖拽选区；`Some(Overlay::ConfirmClose(confirm))` 按钮点击分支使用 `confirm_close_shell`。

---

### Phase 4：单元测试与事件集成测试补齐（`src/**/tests.rs`）

> 目标：覆盖 2 击菜单、口令校验拦截与放行、按钮状态切换、一键清理失效项及边界场景。

- [x] **4.1 状态与更新层测试（`src/app/overlay/tests.rs`、`src/app/update/workspace_history/delete/tests.rs`、`src/app/update/tests.rs`）**
  - 测试 `ConfirmClose::is_confirmed()` 对大小写、首尾空格与错误文本的判定；
  - 测试单条 `Delete` 2 次点击打开 `ConfirmClose`，未输入口令时按 `Enter` 不删除，输入 `confirm delete` 后按 `Enter` 完成级联删除；
  - 测试 `Delete missing` 在有/无失效条目时的菜单显隐（条目菜单、历史空白菜单、活跃空白菜单）；
  - 测试 `Delete missing` 严格只清理 `!path.is_dir()` 的失效条目，保留同仓库存活的主工作区或子 worktree，且不受搜索过滤词限制。
- [x] **4.2 UI 与事件层测试（`src/ui/overlay/tests.rs`、`src/ui/tests.rs`、`src/input/tests.rs`、`src/event/tests/workspace_history.rs`）**
  - 测试 `ConfirmClose` 渲染问题、输入框占位符、硬件光标，以及输入匹配前后 `[Confirm]` 按钮从 `style::muted()` 到 `style::button_danger()` 的样式切换；
  - 测试 `input::overlay_key` 在 `OverlayKind::ConfirmClose` 下映射字符输入、退格、编辑快捷键；
  - 测试鼠标全链路：右键条目/空白处 → 2 次点击 `Delete` 或 `Delete missing` → 弹出 `ConfirmClose` → 未输入口令点击 `[Confirm]` 被拦截 → 键入 `confirm delete` 后点击 `[Confirm]` 执行删除。

---

## 3. 验证命令

完成修改后按影响范围执行精确校验与全量验证：

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test
```

---

## 4. 完成定义（DoD）

- [x] `ws 列表` 右键菜单中的 `Delete` 调整为点击 2 次（`Delete` → `Confirm delete`）后弹出 `ConfirmClose` 模态框。
- [x] `ws 列表` 条目右键菜单与空白处右键菜单在存在失效条目（`!path.is_dir()`）时显示 `Delete missing`（无失效条目时隐藏），同样点击 2 次（`Delete missing` → `Confirm delete missing`）后弹出 `ConfirmClose` 模态框。
- [x] `ConfirmClose` 模态框在问题下方展示单行输入框（占位提示 `type "confirm delete" to confirm`），未匹配 `confirm delete`（忽略首尾空格与大小写）时 `[Confirm]` 按钮置灰且点击/`Enter` 不执行；匹配后按钮变红，点击 `[Confirm]` 或按 `Enter` 执行删除。
- [x] `Delete missing` 严格只删除磁盘上路径不存在的条目及其关联数据，不误删磁盘上仍存在的任何工作区。
- [x] 所有受影响生产代码单文件 ≤ 1000 行（完成 `src/app/update/workspace_history/` 与 `src/ui/text/workspace.rs` 拆分），无 `unwrap()` / `expect()`，`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
