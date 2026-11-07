# 左侧栏工作区列表（`ws list`）删除二次点击确认、输入口令弹窗与一键清理失效项 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/workspace-history-delete-confirm/`
- **关联任务清单**：`docs/feature/workspace-history-delete-confirm/task.md`
- **当前状态**：已在工作区 `.worktrees/workspace-history-delete-confirm`（分支 `feature/workspace-history-delete-confirm`）完成实现与测试，待用户确认合并
- **目标范围**：
  - `src/app/overlay.rs`（`MenuCommand`、`OverlayTarget`、`ConfirmClose`、`CONFIRM_DELETE_PHRASE`）
  - `src/app/state/workspace_history.rs`（`has_missing_entries`、`missing_entry_count`）
  - `src/app/update/menu.rs`（`open_workspace_history_menu`、`open_workspace_list_menu`、`activate_menu`）
  - `src/app/update/dialog.rs`（`confirm_close` 校验 `confirm.is_confirmed()` 并分派单条/批量失效项删除）
  - `src/app/update/workspace_history/`（拆分为 `mod.rs` 与 `delete.rs`，提取公共级联清理 `purge_workspace_history_paths` 并新增 `delete_missing_workspace_history_entries`）
  - `src/input/mod.rs`（`OverlayKind::ConfirmClose` 接入单行文本编辑键路由 `overlay_text_edit_key`）
  - `src/event/overlay.rs`（`ConfirmClose` 输入框鼠标点击定位/拖选与按钮点击、`overlay_input_rect`）
  - `src/ui/text.rs` & `src/ui/text/workspace.rs`（新增菜单与弹窗英文文案常量，并将工作区/菜单相关文案按现有 `connect`/`todo` 子模块模式拆入 `src/ui/text/workspace.rs`，使 `src/ui/text.rs` 降至 1000 行以内）
  - `src/ui/overlay.rs` & `src/ui/overlay/menu.rs` & `src/ui/overlay/dialog.rs`（`ConfirmClose` 渲染问题 + 单行确认输入框 + 动态高亮/禁用 `[Confirm]` 按钮，并返回硬件光标坐标）
- **架构约束**：
  - 严格保持 `app/`（纯状态与更新，禁止 IO、不持有 UI 展示文案）与 `ui/`（纯渲染只读状态，禁止修改状态或执行 IO）边界清晰；
  - 所有 TUI 用户可见文案一律使用英文并集中定义在 `src/ui/text`；
  - 生产代码单文件行数上限 1000 行（`src/app/update/workspace_history.rs` 现状 992 行，本次按职责拆分为 `mod.rs` + `delete.rs`，对应删除测试同步迁入 `delete/tests.rs`），禁止 `unwrap()` / `expect()`。

---

## 0. 现状核实与事实基线（CodeGraph + Codebase Memory 实测）

| 维度 | 现状实测（带文件与行号） | 对本特性的设计影响 |
| --- | --- | --- |
| **右键菜单删除点击次数** | `src/app/overlay.rs:408-413` 定义了 `DeleteWorkspaceHistory`、`DeleteWorkspaceHistoryStep1`、`DeleteWorkspaceHistoryStep2`；`src/app/update/menu.rs:281-292, 367-373` 通过 3 次点击（`Delete` → `Confirm delete (1/2)` → `Confirm delete (2/2)`）后才弹出 `Overlay::ConfirmClose`。 | 1. 移除 `DeleteWorkspaceHistoryStep1`、`DeleteWorkspaceHistoryStep2`、`MENU_CONFIRM_DELETE_STEP1`、`MENU_CONFIRM_DELETE_STEP2` 及 `reset_workspace_delete_steps`；<br>2. 将 `DeleteWorkspaceHistory` 与新增的 `DeleteMissingWorkspaces` 统一纳入 `MenuCommand::requires_confirm()`，复用通用的 `menu.confirming` 行内二次确认机制（第 1 次点击切换为 `Confirm delete` / `Confirm delete missing`，第 2 次点击关闭菜单并弹出 `Overlay::ConfirmClose`）。 |
| **`ws 列表` 右键菜单入口** | `src/event/overlay.rs:514-520`：在历史列表（`WorkspaceMode::History`）中，右键条目调用 `open_workspace_history_menu`，右键空白处调用 `open_workspace_list_menu`。 | 在 `open_workspace_history_menu` 与 `open_workspace_list_menu`（仅当 `workspace_history_mode(state)` 为 `true` 时）中先执行 `state.workspace_history.refresh_existence()`，若 `state.workspace_history.has_missing_entries()` 为 `true`，在菜单末尾追加 `MenuCommand::DeleteMissingWorkspaces`。 |
| **`ConfirmClose` 浮层现状** | `src/app/overlay.rs:453-455`：`ConfirmClose { pub target: OverlayTarget }` 无输入框；全代码库生产路径中仅工作区历史删除使用 `Overlay::ConfirmClose`（引用文件夹删除使用独立的 `Overlay::ReferencedFolderDelete`）。 | 为 `ConfirmClose` 增加 `pub input: TextInput` 字段及 `ConfirmClose::new(target)`、`ConfirmClose::is_confirmed()`；在 `Overlay::input` / `input_mut` / `clear_selection` 与 `input::overlay_key` 中接入 `ConfirmClose`，使其天然获得键入、退格、光标移动、选区、复制、剪切、粘贴、撤销/重做与硬件光标能力。 |
| **确认弹窗渲染与按钮状态** | `src/ui/overlay/dialog.rs:102-120, 407-419, 480-523`：`confirm_shell` 高度为 `question_lines + 6`，仅渲染问题文本与底部固定红色的 `[Confirm]`、`[Cancel]`，不返回光标。 | 新增 `confirm_close_shell`（高度为 `question_lines + 7`，在问题下方预留 1 行输入框 + 1 行空白呼吸区 + 1 行按钮）与 `confirm_close_input_rect`；当 `!confirm.is_confirmed()` 时 `[Confirm]` 使用 `style::muted()`（暗色禁用态），匹配 `confirm delete` 后切换为 `style::button_danger()`（危险红色），并返回输入框硬件光标坐标 `Option<(u16, u16)>`。 |
| **文件行数水位** | `src/app/update/workspace_history.rs` 已达 992 行；`src/ui/text.rs` 已达 1488 行。 | 1. 将 `src/app/update/workspace_history.rs` 拆分为 `src/app/update/workspace_history/mod.rs`（~750 行）与 `src/app/update/workspace_history/delete.rs`（~200 行），并将删除相关单测迁入 `src/app/update/workspace_history/delete/tests.rs`；<br>2. 将工作区/侧栏/菜单文案抽入 `src/ui/text/workspace.rs` 并由 `src/ui/text.rs` `pub use workspace::*;` 重导出，使两处文件均严格满足 ≤ 1000 行规范。 |

---

## 1. 已确认的边界决策汇总（Grill Me 结论表）

| 编号 | 决策维度 | 已确认方案 |
| --- | --- | --- |
| **Q1** | **「一键删除全部不存在的 ws item」菜单位置、显示条件与点击次数** | 1. **菜单位置**：同时挂载在 `ws 列表`（`WorkspaceMode::History`）的**条目右键菜单底部**（`[Open, Rename, Delete, Delete missing]`）和**列表空白处右键菜单底部**（`[New workspace, Delete missing]`）；默认活跃工作区视图（`WorkspaceMode::Active`）不显示。<br>2. **显示条件**：打开右键菜单时实时刷新 `refresh_existence()`，**仅当历史列表中存在至少 1 个路径不存在（`!path.is_dir()`）的条目时才显示 `Delete missing`**，无失效条目时自动隐藏。<br>3. **菜单点击次数**：与单条 `Delete` 完全一致，在右键菜单内点击 2 次（第 1 次点击切换为行内确认文案 `Confirm delete missing`，第 2 次点击关闭菜单并弹出确认模态框）。 |
| **Q2** | **一键清理失效项（`Delete missing`）的作用范围与级联边界** | 1. **严格只删失效目录**：只删除 `path.is_dir() == false`（列表中标红）的条目本身及其数据库关联记录（会话、提示词、文件夹、引用目录），**绝不连带删除磁盘上仍然存在的任何同仓库主工作区或子 worktree**。<br>2. **全量历史范围**：作用于整个 `workspace_history.entries` 中的全部失效条目，不受顶部搜索框当前过滤词限制；打开弹窗与执行删除时均以实时磁盘存在性为准。 |
| **Q3** | **模态弹窗确认输入短语、占位符与匹配规则** | 1. **确认短语**：`confirm delete`（常量 `CONFIRM_DELETE_PHRASE = "confirm delete"`）。<br>2. **输入框占位提示（Placeholder）**：`type "confirm delete" to confirm`。<br>3. **匹配规则**：去除首尾空白后**不区分 ASCII 大小写**精确匹配，即 `input.text().trim().eq_ignore_ascii_case("confirm delete")`，单条删除与批量清理失效项共用同一规则。 |
| **Q4** | **模态弹窗按钮视觉状态与确认触发方式** | 1. **按钮组成与样式**：底部保留 `[Confirm]  [Cancel]` 两个按钮；当输入未匹配 `confirm delete` 时，`[Confirm]` 渲染为暗色禁用态（`style::muted()`），输入匹配后立即变为危险红色（`style::button_danger()`）。<br>2. **触发与拦截**：未匹配时无论按键盘 `Enter` 还是鼠标点击 `[Confirm]` 均为 no-op（不执行删除、不关闭弹窗）；匹配后**既支持鼠标点击 `[Confirm]` 执行删除，也支持按键盘 `Enter` 执行删除**；按 `Esc`、点击 `[Cancel]` 或点击模态框外部空白区域均取消并关闭弹窗。 |
| **Q5** | **模态弹窗内「待删除工作区列表」展示与滚动** | 1. **每行格式与范围**：在问题文案下方、输入框上方以列表形式展示将被删除的 `ws item`，每行格式为 `  • <名称> (<路径>)`（名称用 `style::text()`，路径用 `style::muted()`）；单条删除 Git 主仓库时一并列出级联删除的全部子 worktree，批量清理失效项时列出全部失效工作区。<br>2. **最大可视行数与滚动**：列表最多展示 **5 行**（`CONFIRM_DELETE_LIST_MAX_ROWS = 5`）；超出 5 行时右侧渲染滚动条（`widgets::scrollbar`），支持**鼠标滚轮滚动**、**键盘 `Up`/`Down` 滚动**以及**滚动条点击/拖拽**。 |
| **Q6** | **弹窗宽度与跨操作系统路径粘贴统一兼容** | 1. **弹窗宽度扩宽至 `56` 列**：将 `CONFIRM_WIDTH`（`Delete Workspace` / `Delete Missing Workspaces`）与 `NEW_WORKSPACE_WIDTH`（`New workspace`）统一扩宽至 `56` 列（与 `WORKTREE_DIALOG_WIDTH` 对齐，窄终端自动贴边收缩）。<br>2. **全入口统一跨平台粘贴路径清洗与解析（`src/platform/paste_path.rs`）**：提供 `normalize_pasted_path_input(raw: &str) -> String`，统一兼容：<br>   - macOS / Linux 终端反斜杠转义（如 `/Users/yonah/Desktop/test\ one`）及拖拽尾随空格；<br>   - POSIX 单引号包裹（含 `'\''` 转义，如 `'/Users/yonah/Desktop/test one'`）与 Windows/跨平台双引号包裹（如 `"C:\Users\yonah\Desktop\test one"`）；<br>   - 文件管理器 `file://` URI 与百分号编码（如 `file:///Users/yonah/Desktop/test%20one`、`file:///C:/Users/...`）；<br>   - 在 `New workspace` 与 `ReferencedFolderInput` 粘贴时即时将输入框内容净化为干净路径，并在 `parse_workspace_path`、`validate_referenced_path`、`parse_path_text`、`resolve_pasted_path` 全链路复用。 |

---

## 2. 核心架构与数据流

```text
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ 1. 右键菜单打开与展现（WorkspaceMode::History）                                          │
│                                                                                          │
│   event::overlay::open_right_click_menu()                                                │
│     ├─ 右键命中历史条目 row ──► update::open_workspace_history_menu(state, row, anchor)  │
│     │    ├─ state.workspace_history.refresh_existence()                                  │
│     │    ├─ 基础命令: [OpenWorkspaceHistory, RenameWorkspace, DeleteWorkspaceHistory]    │
│     │    └─ 若 has_missing_entries() ──► 追加 DeleteMissingWorkspaces                    │
│     │                                                                                    │
│     └─ 右键命中历史空白区   ──► update::open_workspace_list_menu(state, anchor)          │
│          ├─ 基础命令: [NewWorkspace]                                                     │
│          └─ 若 workspace_history_mode(state):                                            │
│               ├─ state.workspace_history.refresh_existence()                             │
│               └─ 若 has_missing_entries() ──► 追加 DeleteMissingWorkspaces               │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ 2. 菜单内二次点击确认（2-Click Inline Confirm）                                          │
│                                                                                          │
│   MenuCommand::requires_confirm():                                                       │
│     ├─ DeleteWorkspaceHistory  (第 1 击: "Delete" -> "Confirm delete", confirming=true)  │
│     └─ DeleteMissingWorkspaces (第 1 击: "Delete missing" -> "Confirm delete missing")   │
│                                                                                          │
│   activate_menu() 第 2 次触发（menu.confirming == true）:                                │
│     ├─ (DeleteWorkspaceHistory, OverlayTarget::WorkspaceHistory(i))                      │
│     │    └─ state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(                │
│     │           OverlayTarget::WorkspaceHistory(i)                                       │
│     │       )))                                                                          │
│     └─ (DeleteMissingWorkspaces, OverlayTarget::WorkspaceHistory(_) | WorkspaceList)     │
│          ├─ state.workspace_history.refresh_existence()                                  │
│          └─ 若 missing_entry_count() > 0:                                                │
│               state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(              │
│                   OverlayTarget::MissingWorkspaces                                       │
│               )))                                                                        │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ 3. 删除确认模态弹窗（Overlay::ConfirmClose）渲染与交互                                   │
│                                                                                          │
│   ┌─ Delete Workspace / Delete Missing Workspaces ─────────┐                             │
│   │                                                        │                             │
│   │ delete workspace "demo" and all its data?              │  <-- 自动换行问题提示       │
│   │ > type "confirm delete" to confirm                     │  <-- 单行输入框 + 硬件光标  │
│   │                                                        │  <-- 1 行空白呼吸区         │
│   │              [Confirm]  [Cancel]                       │  <-- 未匹配 muted / 匹配红  │
│   └────────────────────────────────────────────────────────┘                             │
│                                                                                          │
│   键盘/鼠标事件处理:                                                                     │
│     ├─ 字符编辑 / 退格 / 移动 / 选区 / 复制 / 剪切 / 粘贴 / 撤销重做                     │
│     │    └─ 经 Overlay::input_mut() 写入 ConfirmClose.input                              │
│     ├─ Enter 或鼠标左键点击 [Confirm]:                                                   │
│     │    └─ update::dialog::confirm_close(state)                                         │
│     │         ├─ !confirm.is_confirmed() ──► return（保持弹窗打开，不误删）              │
│     │         └─ confirm.is_confirmed()  ──► 关闭弹窗并按 target 分派：                  │
│     │              ├─ WorkspaceHistory(i) ──► delete_workspace_history_entry(state, i)   │
│     │              └─ MissingWorkspaces   ──► delete_missing_workspace_history_entries() │
│     └─ Esc / 点击 [Cancel] / 点击弹窗外部 ──► close_overlay(state)                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. 详细技术设计与关键代码

### 3.1 浮层与菜单状态定义（`src/app/overlay.rs` & `src/app/state/workspace_history.rs`）

#### 关键点
1. **移除旧 3 击枚举项**：删除 `MenuCommand::DeleteWorkspaceHistoryStep1` 与 `MenuCommand::DeleteWorkspaceHistoryStep2`。
2. **新增 `MenuCommand::DeleteMissingWorkspaces` 与 `OverlayTarget::MissingWorkspaces`**：将 `DeleteWorkspaceHistory` 与 `DeleteMissingWorkspaces` 一并加入 `MenuCommand::requires_confirm()`。
3. **升级 `ConfirmClose` 结构体**：包含 `pub target: OverlayTarget` 与 `pub input: TextInput`，提供 `CONFIRM_DELETE_PHRASE = "confirm delete"`、`ConfirmClose::new(target)` 与 `ConfirmClose::is_confirmed(&self) -> bool`。
4. **接入 `Overlay` 单行输入统一接口**：在 `Overlay::input`、`Overlay::input_mut`、`Overlay::clear_selection` 中加入 `Self::ConfirmClose(c)` 分支。
5. **历史面板失效项统计**：在 `WorkspaceHistoryPanel` 中新增 `has_missing_entries(&self) -> bool` 与 `missing_entry_count(&self) -> usize`。

#### 关键代码（`src/app/overlay.rs`）

```rust
/// 删除确认浮层要求输入的固定英文确认口令（忽略首尾空白与 ASCII 大小写）。
pub const CONFIRM_DELETE_PHRASE: &str = "confirm delete";

impl Overlay {
    pub fn input(&self) -> Option<&TextInput> {
        match self {
            Self::Rename(r) => Some(&r.input),
            Self::ConfirmClose(c) => Some(&c.input),
            Self::NewWorkspace(ws) => Some(&ws.input),
            // ... 其余分支保持不变
            _ => None,
        }
    }

    pub fn input_mut(&mut self) -> Option<&mut TextInput> {
        match self {
            Self::Rename(r) => Some(&mut r.input),
            Self::ConfirmClose(c) => Some(&mut c.input),
            Self::NewWorkspace(ws) => Some(&mut ws.input),
            // ... 其余分支保持不变
            _ => None,
        }
    }

    pub fn clear_selection(&mut self) {
        match self {
            Self::Rename(r) => r.input.clear_selection(),
            Self::ConfirmClose(c) => c.input.clear_selection(),
            Self::NewWorkspace(ws) => ws.input.clear_selection(),
            // ... 其余分支保持不变
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayTarget {
    Workspace(usize),
    /// 工作区历史列表条目（`entries` 中的原始下标）。
    WorkspaceHistory(usize),
    /// 工作区历史列表中全部路径不存在的失效条目。
    MissingWorkspaces,
    /// 工作区列表空白区域。
    WorkspaceList,
    // ... 其余分支保持不变
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    // ...
    /// 工作区历史列表：打开该工作区。
    OpenWorkspaceHistory,
    /// 工作区历史列表：删除工作区（菜单内 2 击确认 + 模态弹窗输入确认后级联删除）。
    DeleteWorkspaceHistory,
    /// 工作区历史列表：一键删除全部路径不存在的工作区（菜单内 2 击确认 + 模态弹窗输入确认）。
    DeleteMissingWorkspaces,
}

impl MenuCommand {
    /// 该命令是否需要在右键菜单行内进行二次确认。
    pub fn requires_confirm(self) -> bool {
        matches!(
            self,
            Self::CloseWorkspace
                | Self::CloseTab
                | Self::ClosePane
                | Self::DeleteFolder
                | Self::DeletePrompt
                | Self::DeleteTodo
                | Self::DeletePage
                | Self::DeleteWorkspaceHistory
                | Self::DeleteMissingWorkspaces
                | Self::SwitchToWorkspaceCwd
                | Self::SyncWorkspaceToTerminalCwd
        )
    }
}

/// 关闭/删除确认浮层：目标与确认短语单行输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmClose {
    pub target: OverlayTarget,
    pub input: TextInput,
}

impl ConfirmClose {
    /// 构造带空输入框的确认浮层。
    pub fn new(target: OverlayTarget) -> Self {
        Self {
            target,
            input: TextInput::new(String::new()),
        }
    }

    /// 输入文本是否匹配 `confirm delete`（忽略首尾空格与 ASCII 大小写）。
    pub fn is_confirmed(&self) -> bool {
        self.input
            .text()
            .trim()
            .eq_ignore_ascii_case(CONFIRM_DELETE_PHRASE)
    }
}
```

#### 关键代码（`src/app/state/workspace_history.rs`）

```rust
impl WorkspaceHistoryPanel {
    /// 当前历史列表中是否存在路径已失效（`!exists`）的工作区条目。
    pub fn has_missing_entries(&self) -> bool {
        self.entries.iter().any(|entry| !entry.exists)
    }

    /// 当前历史列表中路径已失效（`!exists`）的工作区条目数量。
    pub fn missing_entry_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.exists).count()
    }
}
```

---

### 3.2 右键菜单构建与激活逻辑（`src/app/update/menu.rs`）

#### 关键点
1. **`open_workspace_history_menu`**：
   - 调用 `state.workspace_history.refresh_existence()` 刷新全部条目的磁盘存在性；
   - 基础命令为 `[OpenWorkspaceHistory, RenameWorkspace, DeleteWorkspaceHistory]`；
   - 若 `state.workspace_history.has_missing_entries()` 为 `true`，追加 `MenuCommand::DeleteMissingWorkspaces`。
2. **`open_workspace_list_menu`**：
   - 基础命令为 `[NewWorkspace]`；
   - 当处于历史列表模式（`super::workspace_history::workspace_history_mode(state)`）时，调用 `state.workspace_history.refresh_existence()`，若 `state.workspace_history.has_missing_entries()` 为 `true`，追加 `MenuCommand::DeleteMissingWorkspaces`。
3. **清理冗余步骤状态机**：
   - 删除 `reset_workspace_delete_steps` 函数；
   - `activate_menu` 中删除 `DeleteWorkspaceHistory` 与 `DeleteWorkspaceHistoryStep1` 的特殊分支，直接走 `_ if command.requires_confirm() && !menu.confirming` 通用分支；
   - 第二次点击（`menu.confirming == true`）时：
     - `(Some(MenuCommand::DeleteWorkspaceHistory), OverlayTarget::WorkspaceHistory(entry_index))` 打开 `Overlay::ConfirmClose(ConfirmClose::new(OverlayTarget::WorkspaceHistory(entry_index)))`；
     - `(Some(MenuCommand::DeleteMissingWorkspaces), OverlayTarget::WorkspaceHistory(_) | OverlayTarget::WorkspaceList)` 先刷新 `state.workspace_history.refresh_existence()`，若 `state.workspace_history.has_missing_entries()` 为 `true`，打开 `Overlay::ConfirmClose(ConfirmClose::new(OverlayTarget::MissingWorkspaces))`。

#### 关键代码（`src/app/update/menu.rs`）

```rust
/// 打开工作区列表空白区域右键菜单：新建工作区；在历史列表模式下若存在失效条目则追加一键清理项。
pub fn open_workspace_list_menu(state: &mut AppState, anchor: (u16, u16)) {
    state.workspace_scroll_drag = None;
    state.workspace_drag = None;
    state.workspace_dragging = false;
    state.workspace_drag_moved = false;
    state.workspace_history.scroll_drag = None;
    state.workspace_history.search_drag = false;
    let mut commands = vec![MenuCommand::NewWorkspace];
    if super::workspace_history::workspace_history_mode(state) {
        state.workspace_history.refresh_existence();
        if state.workspace_history.has_missing_entries() {
            commands.push(MenuCommand::DeleteMissingWorkspaces);
        }
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::WorkspaceList,
        commands,
        selected: 0,
        confirming: false,
    }));
}

/// 打开工作区历史列表条目右键菜单：
/// 全部工作区均支持 `[Open, Rename, Delete]`，若存在失效条目则追加 `Delete missing`。
pub fn open_workspace_history_menu(state: &mut AppState, visible_row: usize, anchor: (u16, u16)) {
    state.workspace_history.scroll_drag = None;
    state.workspace_history.search_drag = None;
    state.workspace_history.search_drag = false;
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
    super::workspace_history::select_workspace_history_row(state, visible_row);
    let Some(&entry_index) = state.workspace_history.filtered_indices().get(visible_row) else {
        return;
    };
    state.workspace_history.refresh_existence();
    if state.workspace_history.entries.get(entry_index).is_none() {
        return;
    }
    let mut commands = vec![
        MenuCommand::OpenWorkspaceHistory,
        MenuCommand::RenameWorkspace,
        MenuCommand::DeleteWorkspaceHistory,
    ];
    if state.workspace_history.has_missing_entries() {
        commands.push(MenuCommand::DeleteMissingWorkspaces);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::WorkspaceHistory(entry_index),
        commands,
        selected: 0,
        confirming: false,
    }));
}
```

以及 `activate_menu` 中的分派：

```rust
pub fn activate_menu(state: &mut AppState) -> Option<(PaneId, Vec<u8>)> {
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut()
        && let Some(command) = menu.commands.get(menu.selected).copied()
        && command.requires_confirm()
        && !menu.confirming
    {
        menu.confirming = true;
        state.menu_hover = true;
        return None;
    }

    let Some(Overlay::Menu(menu)) = state.overlay.take() else {
        return None;
    };
    match (menu.commands.get(menu.selected), menu.target) {
        // ...
        (
            Some(MenuCommand::DeleteWorkspaceHistory),
            OverlayTarget::WorkspaceHistory(entry_index),
        ) => {
            state.overlay = Some(Overlay::ConfirmClose(
                crate::app::overlay::ConfirmClose::new(OverlayTarget::WorkspaceHistory(
                    entry_index,
                )),
            ));
        }
        (
            Some(MenuCommand::DeleteMissingWorkspaces),
            OverlayTarget::WorkspaceHistory(_) | OverlayTarget::WorkspaceList,
        ) => {
            state.workspace_history.refresh_existence();
            if state.workspace_history.has_missing_entries() {
                state.overlay = Some(Overlay::ConfirmClose(
                    crate::app::overlay::ConfirmClose::new(OverlayTarget::MissingWorkspaces),
                ));
            }
        }
        // ...
    }
    None
}
```

---

### 3.3 单条删除与批量清理失效项逻辑及模块拆分（`src/app/update/workspace_history/` & `src/app/update/dialog.rs`）

#### 关键点
1. **拆分 `src/app/update/workspace_history.rs`（现状 992 行）**：
   - `src/app/update/workspace_history/mod.rs`（~750 行）：保留模式切换、顺序同步、选中滚动、双击/回车打开、搜索输入与粘贴，并 `pub use delete::{delete_missing_workspace_history_entries, delete_workspace_history_entry};`；
   - `src/app/update/workspace_history/delete.rs`（~200 行）：承载 `delete_workspace_history_entry`、`delete_missing_workspace_history_entries` 及内部共用的 `purge_workspace_history_paths`；
   - `src/app/update/workspace_history/delete/tests.rs`：将删除相关的状态机单元测试迁入子模块对应测试文件。
2. **`delete_missing_workspace_history_entries` 核心逻辑**：
   - 先执行 `state.workspace_history.refresh_existence()`；
   - 仅收集 `!entry.exists` 的路径集合 `paths_to_delete`（**不展开**存活的主仓库或存活的 linked worktree）；
   - 对失效的 linked worktree 同样登记 `hidden_worktrees` 防护；
   - 调用 `purge_workspace_history_paths(state, paths_to_delete)`：中断归属这些路径的运行中 Agent / 子 Agent / 用户 Shell，关闭已打开的对应活跃工作区（若仅剩最后一个工作区则清空其提示词树与引用目录），从 `state.workspace_history.entries` 中移除，重新夹取 `selected` 与 `scroll`，并为每个路径推入 `Effect::DeleteWorkspaceHistory { path }`。
3. **`confirm_close` 口令门禁（`src/app/update/dialog.rs`）**：
   - 仅当 `confirm.is_confirmed()` 为 `true` 时才消费 `state.overlay.take()` 并执行删除；
   - 未匹配 `confirm delete` 时直接返回，保持弹窗打开。

#### 关键代码（`src/app/update/workspace_history/delete.rs`）

```rust
use std::path::PathBuf;

use crate::app::effects::Effect;
use crate::app::state::AppState;

/// 级联删除单条工作区历史条目（若为主仓库则一并级联删除同仓库全部 linked worktree）。
pub fn delete_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    let mut paths_to_delete = vec![entry.path.clone()];
    if entry.git.as_ref().is_some_and(|git| !git.is_linked) {
        for other in &state.workspace_history.entries {
            if other.path != entry.path
                && other
                    .git
                    .as_ref()
                    .is_some_and(|git| git.repo_root == entry.path)
            {
                paths_to_delete.push(other.path.clone());
            }
        }
    } else if entry.git.as_ref().is_some_and(|git| git.is_linked)
        && !state
            .workspace_history
            .hidden_worktrees
            .contains(&entry.path)
    {
        state
            .workspace_history
            .hidden_worktrees
            .push(entry.path.clone());
    }

    purge_workspace_history_paths(state, paths_to_delete);
    true
}

/// 一键级联删除全部路径不存在（`!path.is_dir()`）的工作区历史条目；返回实际删除的条目数。
///
/// 严格只清理磁盘上已不存在的条目本身，绝不连带删除磁盘上仍存在的同仓库主工作区或子 worktree。
pub fn delete_missing_workspace_history_entries(state: &mut AppState) -> usize {
    state.workspace_history.refresh_existence();
    let mut paths_to_delete = Vec::new();
    for entry in &state.workspace_history.entries {
        if !entry.exists {
            if entry.git.as_ref().is_some_and(|git| git.is_linked)
                && !state
                    .workspace_history
                    .hidden_worktrees
                    .contains(&entry.path)
            {
                state
                    .workspace_history
                    .hidden_worktrees
                    .push(entry.path.clone());
            }
            paths_to_delete.push(entry.path.clone());
        }
    }
    let count = paths_to_delete.len();
    if count == 0 {
        return 0;
    }
    purge_workspace_history_paths(state, paths_to_delete);
    count
}

/// 对给定的待删工作区路径集合执行统一清理：
/// 中断运行中 Agent/子 Agent/Shell、关闭已打开工作区、移除内存历史条目并派发 `Effect::DeleteWorkspaceHistory`。
fn purge_workspace_history_paths(state: &mut AppState, paths_to_delete: Vec<PathBuf>) {
    // 复用原 delete_workspace_history_entry L750-841 的完整清理实现
}
```

#### 关键代码（`src/app/update/dialog.rs`）

```rust
/// 确认关闭浮层回调（要求确认短语匹配 `confirm delete` 后才执行单条或批量失效工作区级联删除）。
fn confirm_close(state: &mut AppState) {
    let confirmed = matches!(
        state.overlay.as_ref(),
        Some(Overlay::ConfirmClose(confirm)) if confirm.is_confirmed()
    );
    if !confirmed {
        return;
    }
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.take() else {
        return;
    };
    match confirm.target {
        crate::app::overlay::OverlayTarget::WorkspaceHistory(entry_index) => {
            super::workspace_history::delete_workspace_history_entry(state, entry_index);
        }
        crate::app::overlay::OverlayTarget::MissingWorkspaces => {
            super::workspace_history::delete_missing_workspace_history_entries(state);
        }
        _ => {}
    }
}
```

---

### 3.4 UI 文案、确认弹窗几何与渲染（`src/ui/text/`、`src/ui/overlay/menu.rs`、`src/ui/overlay/dialog.rs`、`src/ui/overlay.rs`）

#### 关键点
1. **英文文案常量（`src/ui/text/workspace.rs`）**：
   - `MENU_DELETE_MISSING_WORKSPACES: &str = "Delete missing"`
   - `MENU_CONFIRM_DELETE_MISSING_WORKSPACES: &str = "Confirm delete missing"`
   - `CONFIRM_DELETE_MISSING_WORKSPACES_TITLE: &str = "Delete Missing Workspaces"`
   - `CONFIRM_DELETE_INPUT_PLACEHOLDER: &str = "type \"confirm delete\" to confirm"`
   - `confirm_delete_missing_workspaces_question(count: usize) -> String`：生成 `"delete {count} missing workspace(s) and all their data?"`
2. **弹窗几何（`confirm_close_shell` & `confirm_close_input_rect`）**：
   - `confirm_shell` 保持原样供 `ReferencedFolderDelete`（无输入框）使用；
   - 新增 `confirm_close_shell(screen: Rect, question: &str) -> Option<ModalShell>`：
     - 宽度：`CONFIRM_WIDTH`（`40` 列，内容区 `inner.width = 38` 列，足以完整容纳 34 列的 `> type "confirm delete" to confirm`）；
     - 高度：`question_lines + 7`（顶栏 3 行 + 问题 `N` 行 + 输入框 1 行 + 空白间隔 1 行 + 按钮行 1 行 + 底边框 1 行）；
   - 新增 `confirm_close_input_rect(shell: &ModalShell, question: &str) -> Rect`：
     - 计算问题实际渲染行数 `shown_lines = wrap(question, shell.inner.width).len().min(max_question_rows)`，输入框紧随问题下方：`Rect::new(shell.inner.x, shell.inner.y + shown_lines as u16, shell.inner.width, 1)`。
3. **渲染与硬件光标（`render_confirm`）**：
   - `render_confirm` 返回 `Option<(u16, u16)>`（输入框硬件光标坐标），由 `ui::overlay::render` 直接透传给终端帧；
   - 当 `confirm.is_confirmed()` 为 `true` 时，左按钮 `(text::BUTTON_CONFIRM, style::button_danger())`；为 `false` 时，左按钮 `(text::BUTTON_CONFIRM, style::muted())`。

#### 关键代码（`src/ui/overlay/dialog.rs`）

```rust
/// 带口令输入框的关闭/删除确认浮层几何：问题自动换行 + 下方 1 行输入框 + 1 行空白间隔 + 底部按钮行。
pub fn confirm_close_shell(screen: Rect, question: &str) -> Option<widgets::modal::ModalShell> {
    let width = CONFIRM_WIDTH.min(screen.width);
    let inner_width = usize::from(width.saturating_sub(2)).max(1);
    let lines = text::wrap(question, inner_width).len().max(1);
    let height = u16::try_from(lines)
        .unwrap_or(u16::MAX)
        .saturating_add(7)
        .min(screen.height);
    widgets::modal::layout(screen, width, height)
}

/// 计算 `ConfirmClose` 浮层内单行确认输入框的矩形区域（紧接在问题文本下方）。
pub fn confirm_close_input_rect(shell: &widgets::modal::ModalShell, question: &str) -> Rect {
    let width = usize::from(shell.inner.width).max(1);
    let lines = text::wrap(question, width);
    let max_question_rows = usize::from(shell.inner.height.saturating_sub(3)).max(1);
    let shown_rows = lines.len().min(max_question_rows) as u16;
    let y = shell
        .inner
        .y
        .saturating_add(shown_rows.min(shell.inner.height.saturating_sub(2)));
    Rect::new(shell.inner.x, y, shell.inner.width, 1)
}

/// 关闭/删除确认浮层：问题、口令输入框与底部按钮（口令匹配前 Confirm 按钮呈暗色禁用态）。
pub(crate) fn render_confirm(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    confirm: &ConfirmClose,
) -> Option<(u16, u16)> {
    let question = confirm_question(state, &confirm.target);
    let shell = confirm_close_shell(screen, &question)?;
    let (title, _) = confirm_text(state, &confirm.target);
    widgets::modal::render_danger(frame, &shell, title);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let width = usize::from(shell.inner.width);
    let lines = text::wrap(&question, width);
    let max_rows = usize::from(shell.inner.height.saturating_sub(3)).max(1);
    for (index, line) in lines.iter().take(max_rows).enumerate() {
        let clipped = lines.len() > max_rows && index + 1 == max_rows;
        let visible = if clipped {
            text::ellipsize(&format!("{line}…"), width)
        } else {
            line.clone()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(visible, style::text())),
            Rect::new(
                shell.inner.x,
                shell.inner.y.saturating_add(index as u16),
                shell.inner.width,
                1,
            ),
        );
    }
    let confirm_style = if confirm.is_confirmed() {
        style::button_danger()
    } else {
        style::muted()
    };
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &CONFIRM_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_CONFIRM, confirm_style),
            (text::BUTTON_CANCEL, style::button_secondary()),
        ],
    );
    let input_rect = confirm_close_input_rect(&shell, &question);
    render_input(
        frame,
        input_rect,
        confirm.input.text(),
        confirm.input.cursor(),
        confirm.input.selection(),
        Some(text::CONFIRM_DELETE_INPUT_PLACEHOLDER),
    )
}
```

以及 `confirm_text` 与 `confirm_question` 对 `OverlayTarget::MissingWorkspaces` 的支持：

```rust
fn confirm_text(state: &AppState, target: &OverlayTarget) -> (&'static str, String) {
    match target {
        // ...
        OverlayTarget::MissingWorkspaces => (
            text::CONFIRM_DELETE_MISSING_WORKSPACES_TITLE,
            String::new(),
        ),
        // ...
    }
}

pub fn confirm_question(state: &AppState, target: &OverlayTarget) -> String {
    let (_, name) = confirm_text(state, target);
    match target {
        OverlayTarget::WorkspaceHistory(index) => {
            let worktrees = state.workspace_history.delete_worktree_count(*index);
            if worktrees > 0 {
                text::confirm_delete_workspace_group_question(&name, worktrees)
            } else {
                text::confirm_delete_workspace_question(&name)
            }
        }
        OverlayTarget::MissingWorkspaces => {
            let count = state.workspace_history.missing_entry_count();
            text::confirm_delete_missing_workspaces_question(count)
        }
        OverlayTarget::Workspace(_) => text::confirm_delete_workspace_question(&name),
        _ => text::confirm_delete_page_question(&name),
    }
}
```

---

### 3.5 键盘路由与鼠标命中（`src/input/mod.rs` & `src/event/overlay.rs` & `src/event/mouse/drag.rs`）

#### 关键点
1. **键盘路由（`src/input/mod.rs`）**：
   - 将 `OverlayKind::ConfirmClose` 从 `ConfirmClose | TrustWorkspace` 移至 `Rename | NewWorkspace | NewNode | ConfirmClose` 分支，使 `ConfirmClose` 支持 `overlay_text_edit_key(key)`（字符输入、Backspace、Delete、左右移动、Home/End、SelectAll、Undo/Redo 等）。
2. **鼠标点击与拖选（`src/event/overlay.rs` & `src/event/mouse/drag.rs`）**：
   - 将原 `overlay_input_hit_mut(screen, overlay)` 升级为 `overlay_input_rect(state: &AppState, screen: Rect) -> Option<Rect>`（先只读借用 `state` 计算输入框矩形，再通过 `state.overlay.as_mut().and_then(Overlay::input_mut)` 获取可变 `TextInput`），既支持 `ConfirmClose` 动态高度问题下的输入框点击/拖选，又避免同时借用 `state` 与 `state.overlay` 的借用冲突；
   - 在 `handle_overlay_click` 的 `Some(Overlay::ConfirmClose(confirm))` 分支中改用 `ui::overlay::confirm_close_shell(screen, &question)`。

#### 关键代码（`src/event/overlay.rs`）

```rust
/// 获取当前浮层中单行输入框的屏幕矩形区域（无输入框或处于锁定态时返回 None）。
pub(crate) fn overlay_input_rect(state: &AppState, screen: Rect) -> Option<Rect> {
    match state.overlay.as_ref()? {
        Overlay::Rename(_) => {
            let shell = ui::overlay::rename_shell(screen)?;
            Some(Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1))
        }
        Overlay::ConfirmClose(confirm) => {
            let question = ui::overlay::confirm_question(state, &confirm.target);
            let shell = ui::overlay::confirm_close_shell(screen, &question)?;
            Some(ui::overlay::confirm_close_input_rect(&shell, &question))
        }
        Overlay::NewWorkspace(_) => {
            let shell = ui::overlay::new_workspace_shell(screen)?;
            Some(Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1))
        }
        // ... 其余分支保持不变
        _ => None,
    }
}
```

---

## 4. 涉及文件与预估行数表

| 文件路径 | 操作 | 预估行数 | 说明（生产代码均 ≤ 1000 行） |
| --- | --- | --- | --- |
| `src/app/overlay.rs` | 修改 | ~520 | 移除 `DeleteWorkspaceHistoryStep1/2`，新增 `DeleteMissingWorkspaces`、`OverlayTarget::MissingWorkspaces`、`ConfirmClose { target, input }` 与 `is_confirmed()` |
| `src/app/state/workspace_history.rs` | 修改 | ~535 | 新增 `has_missing_entries()` 与 `missing_entry_count()` |
| `src/app/update/menu.rs` | 修改 | ~655 | 菜单挂载 `DeleteMissingWorkspaces`、统一 2 次点击确认、移除 3 击步骤状态机 |
| `src/app/update/dialog.rs` | 修改 | ~425 | `confirm_close` 校验 `confirm.is_confirmed()` 并分派单条或批量失效项删除 |
| `src/app/update/workspace_history/mod.rs` | 拆分修改 | ~780 | 迁出删除函数，保留模式/排序/导航/打开/搜索逻辑并重导出 `delete::*` |
| `src/app/update/workspace_history/delete.rs` | 新建 | ~210 | `delete_workspace_history_entry`、`delete_missing_workspace_history_entries`、`purge_workspace_history_paths` |
| `src/app/update/workspace_history/delete/tests.rs` | 新建 | ~260 | 单条删除、父项组级联、批量删除失效项及边界测试 |
| `src/input/mod.rs` | 修改 | ~530 | `OverlayKind::ConfirmClose` 接入 `overlay_text_edit_key` |
| `src/event/overlay.rs` | 修改 | ~575 | `ConfirmClose` 改用 `confirm_close_shell`；`overlay_input_rect` 支持 `ConfirmClose` 输入框点击与拖选 |
| `src/event/mouse/drag.rs` | 修改 | ~698 | 适配 `overlay_input_rect` |
| `src/ui/text/workspace.rs` | 新建 | ~240 | 承接工作区/侧栏/菜单/确认弹窗英文文案，新增 `Delete missing` 与确认输入占位符文案 |
| `src/ui/text.rs` | 修改 | ~950 | 重导出 `pub use workspace::*;`，生产代码降至 1000 行以内 |
| `src/ui/overlay/menu.rs` | 修改 | ~195 | 映射 `MenuCommand::DeleteWorkspaceHistory` 与 `DeleteMissingWorkspaces` 的常规/行内确认文案 |
| `src/ui/overlay/dialog.rs` | 修改 | ~710 | `confirm_close_shell`、`confirm_close_input_rect`、`render_confirm`（渲染输入框、按钮禁用/高亮与光标返回） |
| `src/ui/overlay.rs` | 修改 | ~105 | `Overlay::ConfirmClose` 分支透传 `render_confirm` 返回的硬件光标 |

---

## 5. 测试矩阵与风险控制

### 5.1 单元测试与集成测试矩阵
1. **右键菜单与 2 击确认测试（`src/app/update/workspace_history/delete/tests.rs`、`src/app/update/tests.rs`、`src/ui/overlay/tests.rs`）**：
   - 在 `ws 列表` 中右键条目：当无失效条目时菜单为 `[Open, Rename, Delete]`；当有失效条目时菜单为 `[Open, Rename, Delete, Delete missing]`；
   - 在 `ws 列表` 中右键空白处：当无失效条目时菜单为 `[New workspace]`；当有失效条目时菜单为 `[New workspace, Delete missing]`；在默认活跃工作区视图（`WorkspaceMode::Active`）右键空白处始终只显示 `[New workspace]`；
   - 单条 `Delete` 与 `Delete missing` 均为**第 1 次点击切换 `menu.confirming = true`（文案变为 `Confirm delete` / `Confirm delete missing`），第 2 次点击关闭菜单并弹出 `Overlay::ConfirmClose`**；切换菜单高亮行时自动复位 `confirming = false`。
2. **模态弹窗输入口令与按钮状态测试（`src/app/overlay/tests.rs`、`src/ui/overlay/tests.rs`、`src/event/tests/workspace_history.rs`）**：
   - `ConfirmClose::is_confirmed()`：空串、`"delete"`、`"confirm"` 返回 `false`；`"confirm delete"`、`"  Confirm Delete  "` 返回 `true`；
   - 未输入正确口令前：按 `Enter` 或鼠标点击 `[Confirm]` 按钮均不关闭弹窗、不执行删除，`[Confirm]` 按钮样式为 `style::muted()`；
   - 输入 `confirm delete` 后：`[Confirm]` 按钮样式变为 `style::button_danger()`，按 `Enter` 或鼠标点击 `[Confirm]` 均关闭弹窗并执行删除；
   - 弹窗内支持字符输入、退格、粘贴、全选、鼠标点击输入框定位光标及 `Esc` / `[Cancel]` 取消。
3. **批量清理失效工作区测试（`src/app/update/workspace_history/delete/tests.rs`）**：
   - 混合存在与失效工作区（含有效 Git 主仓库下挂失效 worktree、以及失效 Git 主仓库下挂仍然存活的 worktree）：执行 `delete_missing_workspace_history_entries` 后，**仅 `!path.is_dir()` 的失效条目被删除**，所有磁盘上仍存在的条目（包括存活的子 worktree）完整保留；
   - 顶部搜索框有过滤词时触发 `Delete missing`，依然清理全量历史列表中的所有失效条目；
   - 若被清理的失效工作区当前仍在 `state.workspaces` 中打开且有运行中 Agent/Shell，同样触发取消效应并关闭对应工作区。

### 5.2 风险与回滚
- **对现有 `ConfirmClose` 测试的影响**：现有 `src/ui/overlay/tests.rs`、`src/ui/tests.rs`、`src/event/tests/workspace_history.rs`、`src/input/tests.rs` 中直接构造 `ConfirmClose { target }` 的测试需同步更新为 `ConfirmClose::new(target)`，并在验证确认提交的测试中先输入 `"confirm delete"`，同时覆盖未输入口令时的拦截断言。
- **回滚策略**：本次改动不涉及数据库 Schema 变更，纯属应用状态机与 UI/事件层变更，可随时无损回滚。
