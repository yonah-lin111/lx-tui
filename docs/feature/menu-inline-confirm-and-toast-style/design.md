# 右键菜单行内二次确认与 Toast 样式重设计 — 详细设计方案

> 状态：已按 Grill Me 确认边界并在 `.worktrees/menu-inline-confirm-and-toast-style` 完成实施与校验。
>
> 参考实现：`@[refer-folder](/Users/yonah/projects/agent/lx-agent)`（`LxMenuItem.tsx`、`ProjectNavigationMenu.tsx`、`SkillFileMenu.tsx`、`ChatHistoryPanel.tsx`）
>
> 涉及模块：`src/app/overlay.rs`、`src/app/update/`、`src/event/overlay.rs`、`src/ui/text.rs`、`src/ui/style.rs`、`src/ui/overlay/`、`src/ui/widgets/menu.rs`、`src/ui/toast.rs`、`src/ui/main_content/lx/toast.rs`、`docs/standards/tui-design-requirements.md`

---

## 1. 背景与现状分析

### 1.1 右键菜单二次确认现状与痛点

当前 `lx-tui` 中，所有右键上下文菜单（`Overlay::Menu`）中的破坏性或路径变更操作在点击或按 `Enter` 后，都会关闭当前右键菜单并弹出一个**屏幕居中的独立确认模态框**：

| 菜单归属 | 菜单命令 (`MenuCommand`) | 初始文案 | 现状行为 |
| --- | --- | --- | --- |
| 工作区右键菜单 (`Workspace`) | `CloseWorkspace` | `Close` | 关闭菜单 → 弹出居中 `Overlay::ConfirmClose` |
| 标签页右键菜单 (`Tab`) | `CloseTab` | `Close` | 关闭菜单 → 弹出居中 `Overlay::ConfirmClose` |
| 窗格右键菜单 (`Pane`) | `ClosePane` | `Close` | 关闭菜单 → 弹出居中 `Overlay::ConfirmClose` |
| 窗格右键菜单 (`Pane`) | `SwitchToWorkspaceCwd` | `Switch to ws path` | 关闭菜单 → 弹出居中 `Overlay::ConfirmSwitchCwd` |
| 窗格右键菜单 (`Pane`) | `SyncWorkspaceToTerminalCwd` | `Sync ws to terminal path` | 关闭菜单 → `CheckTrust` → 弹出居中 `Overlay::ConfirmSyncWorkspaceCwd` |
| 提示词文件夹右键菜单 (`PromptFolder`) | `DeleteFolder` | `Delete` | 关闭菜单 → 弹出居中 `Overlay::ConfirmClose` |
| 提示词条目右键菜单 (`PromptItem`) | `DeletePrompt` | `Delete` | 关闭菜单 → 弹出居中 `Overlay::ConfirmClose` |

**痛点**：用户在侧栏、顶部标签栏、窗格或右侧提示词树边缘右键操作时，视线与鼠标焦点均在锚点菜单处；弹出屏幕居中的模态框会强制打断操作流、造成视线跳跃。

**参考 `lx-agent` 的设计**（见 `ProjectNavigationMenu.tsx` L90-98、`SkillFileMenu.tsx` L59-98、`LxMenuItem.tsx` L54-60）：
- 菜单内部维护 `isConfirming` 状态；
- **第一次点击**：不关闭菜单，将该菜单项切换为确认态，文案由 `Delete` 变为 `Confirm delete`，样式切换为 `dangerActive`（醒目红底白字）；
- **第二次点击**：真正执行删除/关闭动作并收起菜单。

### 1.2 Toast 提示背景与边框颜色现状与痛点

当前全局 Toast（`src/ui/toast.rs`）和 lx 窗格内 Toast（`src/ui/main_content/lx/toast.rs`）存在以下视觉问题：
1. **背景色厚重**：两处 Toast 均使用 `style::overlay_panel()`（`Indexed(236)` 深灰底色 + `Indexed(252)` 浅灰字）铺满整块区域，在透明或原生配色终端中形成一块突兀的深灰膏药。
2. **边框色与项目主色调严重重叠**：
   - `ToastKind::Default` 边框使用 `style::accent()`（`Color::Cyan` 加粗），标题使用 `style::border_title()`（`Color::Cyan + DIM`），右上角关闭按钮 `[✕]` 也写死为 `style::accent()`（`Color::Cyan`）。
   - 而 `Color::Cyan` 正是 `lx-tui` 的全局主色调（聚焦窗格边框、侧栏激活项、顶部激活 Tab、面板折叠按钮全部是 `Cyan`）。
   - `ToastKind::Warning`（`Color::Yellow`）与 `ToastKind::Danger`（`Color::Red`）也与终端基础 ANSI 状态色重叠。
   - 一旦移除 Toast 背景色，青色边框的 Toast 浮在青色聚焦窗格边框下方时，两者色相完全重叠，视觉层次感尽失。

---

## 2. 已确认边界（Grill Me 决策汇总）

| 编号 | 决策维度 | 最终确认口径 |
| --- | --- | --- |
| **Q1** | **右键菜单行内二次确认范围** | 覆盖右键菜单（`Overlay::Menu`）内的**全部 7 个需确认命令**：`CloseWorkspace`、`CloseTab`、`ClosePane`、`DeleteFolder`、`DeletePrompt`、`SwitchToWorkspaceCwd`、`SyncWorkspaceToTerminalCwd`。彻底移除 `Overlay::ConfirmSwitchCwd` 与 `Overlay::ConfirmSyncWorkspaceCwd`；非右键菜单触发的居中确认弹窗（如引用文件夹 `[del ^d]`、页码菜单顶边 `[del]` 按钮触发的 `PromptPage` 确认）保持不变。 |
| **Q2** | **确认态文案、样式与菜单自适应宽度** | 1. **文案**：待确认项原位切换为 `Confirm close` / `Confirm delete` / `Confirm switch to ws path` / `Confirm sync ws to terminal path`。<br>2. **样式**：初始态保持 `style::text()`（悬停 `style::overlay_selection()`）；进入待确认态后整行使用 `style::menu_confirm()`（深红底 `Color::Indexed(160)` + 亮白字 `Color::Indexed(255)` + `Modifier::BOLD`）。<br>3. **宽度自适应**：菜单布局根据当前实际渲染的文案列表动态计算宽度（`UnicodeWidthStr::width`），当文案变为 `Confirm ...` 时菜单框自动拓宽（并夹取在屏幕内），确保内容完整显示、绝不截断为 `…`。 |
| **Q3** | **确认态重置与交互流转** | 1. **焦点转移自动复位**：键盘 `↑/↓` 移动选中项、或鼠标悬停（hover）移动到**其他菜单项**时，自动清除 `confirming` 状态，文案恢复初始态。<br>2. **移出菜单不丢失**：鼠标仅移出菜单区域（未点击关闭、也未悬停到其他项）时，保持当前 `confirming` 状态。<br>3. **`Esc` 与外部点击**：按 `Esc` 或左键点击菜单外区域直接关闭整个菜单。<br>4. **`Sync ws to terminal path` 流程**：第一次触发在菜单内变为 `Confirm sync ws to terminal path`；第二次触发后关闭菜单并调用 `Effect::CheckTrust`，信任通过后直接调用 `sync_workspace_cwd` 完成同步，不再弹二次确认弹窗。 |
| **Q4** | **Toast 透明背景与全新三档边框色系** | 1. **移除背景色**：全局 Toast 与 lx 窗格内 Toast 均移除 `style::overlay_panel()` 背景填充；仅保留 `clear::render` / `clear::render_buffer` 清空底层字符，使用终端默认透明底色（`Color::Reset` / ` None`），正文用 `style::text()`。<br>2. **全新三档专属边框色板**（避开主色调 `Cyan`、工具绿 `Indexed(42)`、运行橙 `Indexed(208)`、思考粉 `Indexed(218)`）：<br>   - `ToastKind::Default`：**薰衣草紫 `Color::Indexed(141)`**<br>   - `ToastKind::Warning`：**琥珀金橙 `Color::Indexed(214)`**<br>   - `ToastKind::Danger`：**珊瑚玫红 `Color::Indexed(204)`**<br>3. **标题与 `[✕]` 关闭按钮同色**：顶边框标题与右上角 `[✕]` 按钮统一使用当前 `ToastKind` 对应的专属边框色，不再写死 `style::accent()`。 |

---

## 3. 核心架构与数据流设计

### 3.1 右键菜单行内确认状态机

```text
[打开右键菜单 open_*_menu]
       │
       ▼
Menu { selected: i, confirming: false }  ───(点击/Enter 非确认项)───► 立即执行命令并关闭菜单
       │
       │ (点击/Enter 需确认项：Close / Delete / SwitchToWorkspaceCwd / SyncWorkspaceToTerminalCwd)
       ▼
Menu { selected: i, confirming: true }
  - 该行文案变为 "Confirm ..."
  - 菜单宽度按新文案自适应撑开
  - 该行渲染为 style::menu_confirm() (红底白字加粗)
       │
       ├───(再次点击该项 / 按 Enter)───► 关闭菜单 + 立即执行目标动作
       │                                  ├─ CloseWorkspace / CloseTab / ClosePane / DeleteFolder / DeletePrompt
       │                                  ├─ SwitchToWorkspaceCwd -> 返回 Some((pane, cd_bytes)) 写入 PTY
       │                                  └─ SyncWorkspaceToTerminalCwd -> 发送 Effect::CheckTrust -> 信任后直接同步
       │
       ├───(鼠标悬停到其他项 j != i / 按 ↑↓ 切换项)───► Menu { selected: j, confirming: false } (自动复位)
       │
       └───(按 Esc / 点击菜单外区域)───► 关闭菜单 (state.overlay = None)
```

### 3.2 Toast 渲染层级流

```text
[触发 Toast: state.toast 或 pane.lx.toast]
       │
       ▼
1. 计算几何 rect() (位置、折行与宽高逻辑完全保持不变)
2. 清底 clear::render / clear::render_buffer
   - 将区域内单元格 reset() 为终端默认透明背景，并擦除边缘被切半的宽字符
   - 【变更】不再填充 style::overlay_panel() (Indexed 236 背景色)
3. 绘制圆角边框、顶边标题与右上角 [✕] 按钮
   - 【变更】边框、标题、[✕] 统一取 style::toast_border(toast.kind)：
     * Default -> Color::Indexed(141) (薰衣草紫)
     * Warning -> Color::Indexed(214) (琥珀金橙)
     * Danger  -> Color::Indexed(204) (珊瑚玫红)
4. 绘制正文
   - 使用 style::text() (终端默认前景、无背景色)
```

---

## 4. 模块级详细设计与关键代码

### 4.1 浮层状态模型（`src/app/overlay.rs`）

1. **精简 `OverlayKind` 与 `Overlay`**：
   - 移除不再使用的 `ConfirmSwitchCwd` 与 `ConfirmSyncWorkspaceCwd` 枚举变体及对应结构体。
   - 保留 `ConfirmClose` 仅供页码菜单顶栏 `[del]` 非空页删除（`OverlayTarget::PromptPage`）使用。
2. **`Menu` 结构体新增 `confirming: bool` 字段**：
   - 表示当前选中项（`menu.selected`）是否正处于第一次点击/回车后的“行内待确认态”。
3. **`MenuCommand::requires_confirm(self) -> bool` 方法**：
   - 集中定义哪 7 个右键菜单命令需要行内二次确认。

**关键代码（`src/app/overlay.rs`）**：

```rust
/// 浮层种类：按键路由与渲染按种类分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Menu,
    Rename,
    ConfirmClose,
    NewWorkspace,
    WorktreeOpen,
    TrustWorkspace,
    NewNode,
    SaveDraft,
    PageMenu,
    ReferencedFolderMenu,
    ReferencedFolderInput,
    ReferencedFolderSearch,
    ReferencedFolderDelete,
    ResumeSessionEdit,
}

/// 激活中的模态浮层。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Menu(Menu),
    Rename(Rename),
    ResumeSessionEdit(ResumeSessionEdit),
    NewWorkspace(NewWorkspace),
    ConfirmClose(ConfirmClose),
    WorktreeOpen(WorktreeOpen),
    TrustWorkspace(TrustWorkspace),
    NewNode(NewNode),
    SaveDraft(SaveDraft),
    PageMenu(PageMenu),
    ReferencedFolderMenu(ReferencedFolderMenu),
    ReferencedFolderInput(ReferencedFolderInput),
    ReferencedFolderSearch(ReferencedFolderSearch),
    ReferencedFolderDelete(ReferencedFolderDelete),
}

/// 右键菜单浮层：锚点、目标、命令项、当前高亮索引与当前项行内二次确认态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub anchor: (u16, u16),
    pub target: OverlayTarget,
    pub commands: Vec<MenuCommand>,
    pub selected: usize,
    /// 当前选中项（`selected`）是否已进入第一次点击/回车后的行内待确认状态。
    pub confirming: bool,
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
                | Self::SwitchToWorkspaceCwd
                | Self::SyncWorkspaceToTerminalCwd
        )
    }
}
```

---

### 4.2 菜单交互更新与状态转换（`src/app/update/menu.rs`、`src/app/update/prompt.rs`）

1. **打开菜单初始化**：
   - `open_workspace_menu`、`open_workspace_list_menu`、`open_tab_menu`、`open_pane_menu`（在 `src/app/update/menu.rs`）以及 `open_prompt_item_menu`、`open_prompt_temp_menu`、`open_prompt_folder_menu`、`open_prompt_tree_menu`（在 `src/app/update/prompt.rs`）构造 `Menu` 时统一初始化 `confirming: false`。
2. **移动或悬停切换菜单项时重置 `confirming`**：
   - `move_menu_selection`：当菜单项数量 `> 1` 且索引发生移动（或按上下键）时，将 `menu.confirming = false`。
   - `set_menu_selection`：当鼠标悬停到不同索引（`menu.selected != index`）时，将 `menu.confirming = false`；若仍在同一项上移动，保留 `menu.confirming` 不变！
   - `clear_menu_hover`：鼠标移出菜单容器时仅将 `state.menu_hover = false`，**不清除** `menu.confirming`（若已有 `confirming: true`，该确认项继续保持红底高亮与确认文案，方便指针移回点击第二次）。
3. **`activate_menu` 二段式执行**：
   - 返回值签名改为 `pub fn activate_menu(state: &mut AppState) -> Option<(PaneId, Vec<u8>)>`，以便在确认 `SwitchToWorkspaceCwd` 时直接将 `(PaneId, Vec<u8>)` 返回给事件层写入 PTY。
   - 先通过 `state.overlay.as_mut()` 检查当前选中的 `MenuCommand`：
     - 若 `command.requires_confirm()` 且 `!menu.confirming`：设置 `menu.confirming = true`、`state.menu_hover = true`，直接返回 `None`（保持菜单打开，不消费 `state.overlay`）。
   - 若不需要确认，或已经处于 `menu.confirming == true`：通过 `state.overlay.take()` 取出并关闭菜单，直接执行对应动作！

**关键代码（`src/app/update/menu.rs`）**：

```rust
/// 菜单高亮按步长循环移动；页码菜单按列表语义钳制移动。
pub fn move_menu_selection(state: &mut AppState, step: isize) {
    if matches!(state.overlay, Some(Overlay::PageMenu(_))) {
        move_page_menu_selection(state, step);
        return;
    }
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        return;
    };
    if menu.commands.is_empty() {
        return;
    }
    let len = menu.commands.len() as isize;
    let next = (menu.selected as isize + step).rem_euclid(len) as usize;
    if next != menu.selected {
        menu.confirming = false;
    }
    menu.selected = next;
    // 键盘移动同样显示高亮反馈。
    state.menu_hover = true;
}

/// 菜单悬停高亮；索引越界忽略；返回是否变化。
pub fn set_menu_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= menu.commands.len() {
        return false;
    }
    let selection_changed = menu.selected != index;
    if selection_changed {
        menu.confirming = false;
    }
    let changed = selection_changed || !state.menu_hover;
    menu.selected = index;
    state.menu_hover = true;
    changed
}

/// 执行菜单当前项：
/// - 需二次确认的命令在首次触发时仅切换当前项为 `confirming = true` 并保持菜单打开；
/// - 再次触发（或无需确认的命令首次触发）时关闭菜单并立即执行动作。
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
        (Some(MenuCommand::NewTab), OverlayTarget::Tab { .. }) => create_tab(state),
        (Some(MenuCommand::NewWorkspace), OverlayTarget::WorkspaceList) => {
            open_new_workspace_dialog(state);
        }
        (Some(MenuCommand::NewTerminal), OverlayTarget::Workspace(target)) => {
            create_terminal_in_workspace(state, target);
        }
        (Some(MenuCommand::OpenPrompt), OverlayTarget::Workspace(target)) => {
            open_prompt_for_workspace(state, target);
        }
        (Some(MenuCommand::OpenPrompt), OverlayTarget::Pane { pane, .. }) => {
            open_prompt_for_pane(state, pane);
        }
        (
            Some(MenuCommand::SwitchToWorkspaceCwd),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            return switch_to_workspace_cwd(state, workspace, tab, pane);
        }
        (
            Some(MenuCommand::SyncWorkspaceToTerminalCwd),
            OverlayTarget::Pane {
                workspace, pane, ..
            },
        ) => {
            // 已在菜单内完成二次确认：触发信任检查，信任通过后直接执行同步。
            if let Some(path) = state
                .workspaces
                .get(workspace)
                .and_then(|ws| ws.tabs.iter().find_map(|t| t.pane(pane)))
                .and_then(|target| target.cwd.clone())
            {
                state.effects.push(Effect::CheckTrust {
                    path,
                    root: None,
                    trigger: TrustTrigger::SyncWorkspaceCwd { workspace, pane },
                });
            }
        }
        (Some(MenuCommand::RenameWorkspace), OverlayTarget::Workspace(target)) => {
            let Some(workspace) = state.workspaces.get(target) else {
                return None;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target: RenameTarget::Workspace(target),
                input: TextInput::new(workspace.name.clone()),
            }));
        }
        (Some(MenuCommand::OpenWorktree), OverlayTarget::Workspace(target)) => {
            open_worktree_dialog(state, target);
        }
        (Some(MenuCommand::CloseWorkspace), OverlayTarget::Workspace(target)) => {
            close_workspace(state, target);
        }
        (Some(MenuCommand::RenameTab), OverlayTarget::Tab { workspace, tab }) => {
            let Some(label) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.tabs.get(tab))
                .map(|tab_state| tab_label(tab, tab_state.name.as_deref()))
            else {
                return None;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target: RenameTarget::Tab { workspace, tab },
                input: TextInput::new(label),
            }));
        }
        (Some(MenuCommand::CloseTab), OverlayTarget::Tab { workspace, tab }) => {
            close_tab(state, workspace, tab);
        }
        (
            Some(MenuCommand::SplitRight),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            split_pane(state, workspace, tab, pane, Direction::Horizontal);
        }
        (
            Some(MenuCommand::SplitDown),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            split_pane(state, workspace, tab, pane, Direction::Vertical);
        }
        (
            Some(MenuCommand::SwitchToTerminal | MenuCommand::SwitchToLx),
            OverlayTarget::Pane { pane, .. },
        ) => {
            toggle_pane_view(state, pane);
        }
        (
            Some(MenuCommand::ClosePane),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            close_pane(state, workspace, tab, pane);
        }
        (Some(MenuCommand::NewPrompt), OverlayTarget::PromptFolder { folder_id, .. }) => {
            open_new_prompt(state, Some(folder_id));
        }
        (Some(MenuCommand::NewPrompt), OverlayTarget::PromptTree { .. }) => {
            open_new_prompt(state, None);
        }
        (
            Some(MenuCommand::NewFolder | MenuCommand::NewSubFolder),
            OverlayTarget::PromptFolder { folder_id, .. },
        ) => {
            open_new_folder(state, Some(folder_id));
        }
        (Some(MenuCommand::NewFolder), OverlayTarget::PromptTree { .. }) => {
            open_new_folder(state, None);
        }
        (
            Some(MenuCommand::RenameFolder),
            OverlayTarget::PromptFolder {
                workspace,
                folder_id,
            },
        ) => {
            open_prompt_rename(
                state,
                RenameTarget::PromptFolder {
                    workspace,
                    folder_id,
                },
            );
        }
        (
            Some(MenuCommand::DeleteFolder),
            OverlayTarget::PromptFolder {
                workspace,
                folder_id,
            },
        ) => {
            delete_prompt_folder(state, workspace, folder_id);
        }
        (Some(MenuCommand::DeletePrompt), OverlayTarget::PromptItem { workspace, item_id }) => {
            delete_prompt_item(state, workspace, item_id);
        }
        // ... 其余分支保持原样
        _ => {}
    }
    None
}
```

---

### 4.3 工作区路径切换与同步逻辑（`src/app/update/workspace.rs`、`src/app/update/trust.rs`、`src/app/update/dialog.rs`、`src/event/overlay.rs`）

1. **`src/app/update/workspace.rs`**：
   - 将原 `open_confirm_switch_cwd` + `confirm_switch_cwd` 合并为直接执行函数 `switch_to_workspace_cwd(state, workspace, tab, pane) -> Option<(PaneId, Vec<u8>)>`。
   - 将原 `open_confirm_sync_workspace_cwd` + `confirm_sync_workspace_cwd` 合并为直接执行函数 `sync_workspace_cwd(state, workspace, pane)`。
2. **`src/app/update/trust.rs`**：
   - `resume_trust` 中 `TrustTrigger::SyncWorkspaceCwd { workspace, pane }` 分支直接调用 `super::sync_workspace_cwd(state, workspace, pane)`，不再弹出 `ConfirmSyncWorkspaceCwd`。
3. **`src/app/update/dialog.rs` 与 `src/event/overlay.rs`**：
   - `apply_overlay_key` 的 `OverlayKind::Menu` 分支改为直接返回 `activate_menu(state)`；移除 `OverlayKind::ConfirmSwitchCwd` 与 `OverlayKind::ConfirmSyncWorkspaceCwd` 分支。
   - `handle_overlay_click` 的 `Some(Overlay::Menu(menu))` 分支在命中菜单项后改为 `return update::activate_menu(state);`；移除 `Some(Overlay::ConfirmSwitchCwd(_))` 与 `Some(Overlay::ConfirmSyncWorkspaceCwd(_))` 分支。

**关键代码（`src/app/update/workspace.rs`）**：

```rust
/// 切换终端窗格到工作区固定路径：返回写入目标窗格的 `cd` 命令；
/// 工作区无固定路径、目标窗格不存在或非终端时返回 None。
pub(crate) fn switch_to_workspace_cwd(
    state: &AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
) -> Option<(PaneId, Vec<u8>)> {
    let ws = state.workspaces.get(workspace)?;
    let path = ws.cwd.as_deref()?;
    let pane_state = ws.tabs.get(tab)?.pane(pane)?;
    (pane_state.kind == PaneKind::Terminal).then(|| (pane, switch_cwd_command(path)))
}

/// 同步当前工作区路径到终端窗格路径：更新工作区 cwd、自动命名与 git 查询，
/// 并同步活跃 prompt 根目录。
pub(crate) fn sync_workspace_cwd(state: &mut AppState, workspace: usize, pane: PaneId) {
    let Some(new_cwd) = state
        .workspaces
        .get(workspace)
        .and_then(|ws| ws.tabs.iter().find_map(|t| t.pane(pane)))
        .and_then(|target| target.cwd.clone())
    else {
        return;
    };
    let target_ws = workspace;
    // ... 原 confirm_sync_workspace_cwd 的后续更新逻辑完全保留
}
```

---

### 4.4 菜单文案、确认态样式与自适应宽度（`src/ui/text.rs`、`src/ui/style.rs`、`src/ui/overlay/menu.rs`、`src/ui/widgets/menu.rs`）

1. **`src/ui/text.rs` 新增行内确认文案常量**：
   ```rust
   /// 右键菜单项行内二次确认文案。
   pub const MENU_CONFIRM_CLOSE: &str = "Confirm close";
   pub const MENU_CONFIRM_DELETE: &str = "Confirm delete";
   pub const MENU_CONFIRM_SWITCH_TO_WORKSPACE_CWD: &str = "Confirm switch to ws path";
   pub const MENU_CONFIRM_SYNC_WS_TO_TERMINAL_CWD: &str = "Confirm sync ws to terminal path";
   ```

2. **`src/ui/style.rs` 新增菜单行内确认高亮样式 `menu_confirm()`**：
   ```rust
   /// 右键菜单项行内待确认态：深红底（Indexed 160）配亮白字（Indexed 255）加粗，对齐 lx-agent 的 dangerActive。
   pub fn menu_confirm() -> Style {
       Style::default()
           .fg(Color::Indexed(255))
           .bg(Color::Indexed(160))
           .add_modifier(Modifier::BOLD)
   }
   ```

3. **`src/ui/overlay/menu.rs` 动态映射确认态文案**：
   - `menu_labels(menu: &Menu)` 遍历 `menu.commands.iter().enumerate()`，当 `menu.confirming && index == menu.selected` 时返回对应的 `MENU_CONFIRM_*` 常量；
   - `widgets::menu::layout` 接收 `menu_labels(menu)` 计算宽度，因此当某一项变为 `Confirm ...` 时，`max_label` 自动按新文案的 `UnicodeWidthStr::width` 增大，菜单外框宽度自动拓宽并在屏幕右缘安全夹取，保证内容 100% 完整显示。

**关键代码（`src/ui/overlay/menu.rs`）**：

```rust
/// 菜单命令到文案的映射；当选中项处于 `confirming` 状态时切换为行内确认文案。
pub(crate) fn menu_labels(menu: &Menu) -> Vec<&'static str> {
    menu.commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            let confirming = menu.confirming && menu.selected == index;
            match command {
                MenuCommand::NewTab => text::MENU_NEW_TAB,
                MenuCommand::NewTerminal => text::MENU_NEW_TERMINAL,
                MenuCommand::NewWorkspace => text::MENU_NEW_WORKSPACE,
                MenuCommand::RenameWorkspace => text::MENU_RENAME_WORKSPACE,
                MenuCommand::OpenWorktree => text::MENU_OPEN_WORKTREE,
                MenuCommand::OpenPrompt => text::MENU_OPEN_PROMPT,
                MenuCommand::CloseWorkspace | MenuCommand::CloseTab | MenuCommand::ClosePane => {
                    if confirming {
                        text::MENU_CONFIRM_CLOSE
                    } else {
                        text::MENU_CLOSE_WORKSPACE
                    }
                }
                MenuCommand::RenameTab => text::MENU_RENAME_TAB,
                MenuCommand::SplitRight => text::MENU_SPLIT_RIGHT,
                MenuCommand::SplitDown => text::MENU_SPLIT_DOWN,
                MenuCommand::SwitchToTerminal => text::MENU_SWITCH_TO_TERMINAL,
                MenuCommand::SwitchToLx => text::MENU_SWITCH_TO_LX,
                MenuCommand::SwitchToWorkspaceCwd => {
                    if confirming {
                        text::MENU_CONFIRM_SWITCH_TO_WORKSPACE_CWD
                    } else {
                        text::MENU_SWITCH_TO_WORKSPACE_CWD
                    }
                }
                MenuCommand::SyncWorkspaceToTerminalCwd => {
                    if confirming {
                        text::MENU_CONFIRM_SYNC_WS_TO_TERMINAL_CWD
                    } else {
                        text::MENU_SYNC_WS_TO_TERMINAL_CWD
                    }
                }
                MenuCommand::NewPrompt => text::MENU_PROMPT_NEW_PROMPT,
                MenuCommand::NewFolder => text::MENU_PROMPT_NEW_FOLDER,
                MenuCommand::NewSubFolder => text::MENU_PROMPT_NEW_SUBFOLDER,
                MenuCommand::RenameFolder | MenuCommand::RenamePrompt => text::MENU_PROMPT_RENAME,
                MenuCommand::DeleteFolder | MenuCommand::DeletePrompt => {
                    if confirming {
                        text::MENU_CONFIRM_DELETE
                    } else {
                        text::MENU_PROMPT_DELETE
                    }
                }
                MenuCommand::DeleteTemp => text::MENU_PROMPT_DELETE,
                MenuCommand::EditPrompt => text::MENU_PROMPT_EDIT,
                MenuCommand::CopyPromptContent => text::MENU_PROMPT_COPY,
                MenuCommand::CopyPath => text::MENU_COPY_PATH,
            }
        })
        .collect()
}
```

4. **`src/ui/widgets/menu.rs` 与 `src/ui/overlay.rs` 渲染确认态样式**：
   - `widgets::menu::render_with_confirm`（或在 `widgets::menu::render` 中增加 `confirming: Option<usize>` 参数）：
     - 若 `confirming == Some(index)`：使用 `style::menu_confirm()`（即使鼠标暂时移出菜单导致 `selected == None`，待确认项依然保持醒目红底白字）；
     - 否则若 `selected == Some(index)`：使用 `style::overlay_selection()`；
     - 否则：使用 `style::text()`。

---

### 4.5 Toast 移除背景色与全新三档边框色系（`src/ui/style.rs`、`src/ui/toast.rs`、`src/ui/main_content/lx/toast.rs`）

1. **在 `src/ui/style.rs` 中集中定义 Toast 专用三档边框样式 Token**：
   - 遵循 `AGENTS.md` 与 `tui-design-requirements.md` 的规定：“样式只从 `ui/style.rs` 的语义函数取用，禁止在组件中直接写死颜色与修饰符”。
   - 选色避开主色调 `Color::Cyan`、聊天步骤翡翠绿 `Indexed(42)`、后台存活橙黄 `Indexed(208)`、思考品红 `Indexed(218)`：
     - `ToastKind::Default`：`Color::Indexed(141)`（薰衣草紫，温和醒目，与青色主框线形成清晰冷暖/色相区分）
     - `ToastKind::Warning`：`Color::Indexed(214)`（琥珀金橙，独立于普通 ANSI `Yellow`）
     - `ToastKind::Danger`：`Color::Indexed(204)`（珊瑚玫红，独立于普通 ANSI `Red`，对齐 `lx-agent` 的 `rose-400`）

**关键代码（`src/ui/style.rs`）**：

```rust
/// Toast 提示框三档专属色（边框、顶边标题与右上角 `[✕]` 关闭按钮共用）：
/// 刻意避开项目主色调 `Cyan`、聊天步骤翡翠绿 `Indexed(42)` 与思考品红 `Indexed(218)`，
/// 确保无背景色的 Toast 在任何面板/窗格边框旁都层次分明、不撞色。
pub fn toast_border(kind: crate::app::toast::ToastKind) -> Style {
    use crate::app::toast::ToastKind;
    let color = match kind {
        ToastKind::Default => Color::Indexed(141),
        ToastKind::Warning => Color::Indexed(214),
        ToastKind::Danger => Color::Indexed(204),
    };
    Style::default().fg(color)
}
```

2. **全局 Toast（`src/ui/toast.rs`）移除背景色并统一边框/标题/`[✕]` 颜色**：

```rust
pub fn render(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    view: &ViewLayout,
    pane_rects: &[(PaneId, Rect)],
    config: &Config,
) -> Option<Rect> {
    let area = rect(state, view, pane_rects, screen, config)?;
    let toast = state.toast.as_ref()?;
    // 清空底层字符并重置为终端默认透明底色（不再铺 overlay_panel 背景）。
    clear::render(frame, area);
    let border = border_style(toast.kind);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border);
    if let Some(title) = toast.title.as_deref() {
        block = block.title(Span::styled(
            format!(
                " {} ",
                text::ellipsize(title, usize::from(area.width.saturating_sub(2)))
            ),
            title_style(toast.kind),
        ));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(rect) = crate::ui::widgets::modal::close_button_rect(area) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, border)),
            rect,
        );
    }
    if inner.width == 0 || inner.height == 0 {
        return Some(area);
    }
    let width = usize::from(inner.width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::ellipsize(&toast.message, width),
            style::text(),
        )))
        .alignment(Alignment::Center),
        inner,
    );
    Some(area)
}

/// 边框语义色：三档独立 256 色（薰衣草紫 / 琥珀金橙 / 珊瑚玫红），不与主色调 Cyan 重叠。
pub(crate) fn border_style(kind: ToastKind) -> Style {
    style::toast_border(kind)
}

/// 标题语义色：与该档边框保持同色。
pub(crate) fn title_style(kind: ToastKind) -> Style {
    style::toast_border(kind)
}
```

3. **lx 窗格内 Toast（`src/ui/main_content/lx/toast.rs`）移除背景色并同步 `[✕]` 颜色**：

```rust
pub(crate) fn draw(buf: &mut Buffer, area: Rect, toast: &Toast) -> Option<Rect> {
    let box_area = rect(area, toast)?;
    // 清空底层字符并重置为终端默认透明底色（不再铺 overlay_panel 背景色）。
    clear::render_buffer(buf, box_area);
    draw_border(buf, box_area, toast.kind);
    if let Some(title) = toast.title.as_deref() {
        draw_title(buf, box_area, title, toast.kind);
    }
    // 右上角关闭按钮：与该档边框同色，点击命中走 `close_button_at`。
    if let Some(button) = crate::ui::widgets::modal::close_button_rect(box_area) {
        let style = border_style(toast.kind);
        for (index, ch) in text::MODAL_CLOSE.chars().enumerate() {
            let x = button
                .x
                .saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
            put(buf, x, button.y, ch, style);
        }
    }
    // ... 正文折行与 style::text() 绘制保持不变
```

---

## 5. 测试与验证计划

1. **右键菜单行内二次确认测试**：
   - `app::update::tests` / `app::update::prompt::tests`：
     - 首次激活 `CloseWorkspace` / `CloseTab` / `ClosePane` / `DeleteFolder` / `DeletePrompt` / `SwitchToWorkspaceCwd` / `SyncWorkspaceToTerminalCwd` 时，断言 `state.overlay` 仍为 `Some(Overlay::Menu)` 且 `menu.confirming == true`，目标尚未被删除/切换。
     - 第二次激活时，断言菜单关闭且目标动作执行成功。
     - 处于 `confirming == true` 时调用 `move_menu_selection` 或 `set_menu_selection` 切换到别的项，断言 `menu.confirming` 自动复位为 `false`。
   - `ui::overlay::tests`：
     - 断言 `menu.confirming == true` 时，对应行渲染为 `Confirm close` / `Confirm delete` / `Confirm switch to ws path` / `Confirm sync ws to terminal path`，菜单框宽度随确认文案自适应拓宽且不出现 `…` 截断，且该行单元格具备 `style::menu_confirm()` 的红底白字加粗样式。
   - `event::tests::overlay`：
     - 鼠标第一次左键点击右键菜单中的 `Close` / `Switch to ws path` 进入确认态，第二次左键点击执行关闭或返回 `cd` 命令。
2. **Toast 透明背景与三档新边框色测试**：
   - `ui::toast::tests` 与 `ui::main_content::lx::toast::tests`：
     - 断言全局 Toast 与 lx 窗格内 Toast 渲染后，区域内所有单元格 `bg == Color::Reset`（无 `Indexed(236)` 背景色），且底层残留字符被清空。
     - 断言 `Default` / `Warning` / `Danger` 三档边框、标题与 `[✕]` 关闭按钮颜色分别为 `Indexed(141)`、`Indexed(214)`、`Indexed(204)`，三者两两不同且均不等于 `Color::Cyan`。
3. **静态检查与回归测试**：
   - `cargo fmt`
   - `cargo clippy --all-targets -- -D warnings`
   - 定向运行受影响模块测试（`cargo test overlay`、`cargo test toast`、`cargo test menu`）。
