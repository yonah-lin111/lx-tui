# 工作区固定路径与右键菜单增强 — 设计

- 分支：`feature/workspace-context-menu`
- 状态：**已完成**；工作区路径固定化、右键菜单扩展、Prompt 独立路径绑定与菜单按显示宽度自适应均已落地

## 1. 背景与目标

当前系统中存在以下行为与待增强项：
1. **工作区路径漂移**：`poll_process_cwds` 定期轮询根窗格进程 cwd 并更新对应工作区的 `cwd` 及自动名称，导致终端在 `cd` 到其他子目录或外部目录时，左侧栏的工作区项路径和标签随之漂移。
2. **缺乏从工作区直接开终端的能力**：在左栏工作区项上无法一键以该工作区的基准路径开出终端。
3. **主内容窗格快速回到工作区路径**：在主内容窗格（main content pane）内，缺乏快捷方式将当前窗格路径切回所属工作区基准路径。
4. **右键菜单内容被省略/截断**：`MenuLayout` 计算菜单宽度时按字符计数（`chars().count()`）而非终端显示宽度（`unicode_width`），导致多字节/中文字符及长菜单项被 `text::ellipsize` 截断，无法完整显示。
5. **右键打开 Prompt 并绑定就地上下文**：在左栏工作区项或主内容窗格中，无法通过右键直接展开/聚焦 Prompt 并让 Prompt 文件提及直接对齐当前工作区或窗格目录。

## 2. 已确认决策（Grill Me 结论）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 左栏工作区“添加当前路径的终端”新建的窗口形态 | **在目标工作区追加独立 Tab**：初始 cwd 为该工作区固定根路径，直接进入 `PaneView::Terminal`，并自动切换激活目标工作区与新标签页 |
| 2 | 主内容“切换到当前 workspace 路径”的技术实现机制 | **向 PTY 会话注入 `cd "<workspace_path>"\n`**；二次确认浮层展示目标路径，确认后发送命令，保留用户 Shell 环境变量与历史 |
| 3 | 菜单与二次确认文案语言与风格 | **保持统一英文风格，工作区简写为 ws**：左栏菜单 `New terminal`、主内容菜单 `Switch to ws path`、确认浮层标题 `switch to ws path`、提问 `switch cwd to "<path>"?` |
| 4 | 菜单内容完整显示与宽度计算 | 采用真实终端列宽（`UnicodeWidthStr::width`）动态适配最长项，正常窗口杜绝截断与省略；在终端物理宽度极端不足时安全夹取至屏幕宽度 |
| 5 | 右键“打开当前 prompt”的状态联动与路径绑定 | 两处均命名为 `Open prompt`；若 Prompt 折叠则自动展开并聚焦；左栏触发同步切换激活目标工作区并将 Prompt 根路径切至工作区路径；主内容触发保持当前工作区并将 Prompt 根路径切至该窗格当前 cwd |

## 3. 架构与数据流

### 3.1 状态层（`src/app/`）
- **工作区路径固定化**：
  - 解除 `poll_process_cwds` 对 `update_workspace_cwd` 的调用，使 `workspace.cwd` 在生命周期内保持不可变。
  - 窗格自身继续通过 `update_pane_cwd` 跟踪各自独立的进程 cwd 与 `cwd_label`。
- **Prompt 独立路径绑定**：
  - `AppState` 维护 `prompt_root: Option<PathBuf>` 或在 `Prompt` 内记录当前生效的上下文根路径。
  - 打开 Prompt 时更新该根路径，驱动 `@` 提及扫描缓存与顶栏路径名展示。
- **右键菜单命令扩展**：
  - `MenuCommand::NewTerminal`：工作区右键菜单新增项，位于首位。
  - `MenuCommand::OpenPrompt`：工作区右键菜单及窗格右键菜单新增项。
  - `MenuCommand::SwitchToWorkspaceCwd`：主内容窗格右键菜单新增项（仅在当前工作区存在有效 `cwd` 时展示）。
- **二次确认浮层状态**：
  - `Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd)`：持有目标工作区、标签页、窗格 ID 及目标路径。
  - 支持 `Enter` 确认与 `Esc` 取消；鼠标点击 `[confirm enter]` 或 `[cancel esc]` 按钮分派对应事件。

### 3.2 事件与 PTY 交互层（`src/event/`）
- **路径切换执行**：
  - `update::apply_overlay_key` 在确认切换时返回需要写入 PTY 的动作，或在 `AppState` 中置入待写入指令；
  - 事件循环调用 `write_to_pane(sessions, pane, command.as_bytes())`，向目标窗格写入 `cd "<target_path>"\n`。
- **新建工作区终端**：
  - `create_terminal_in_workspace(state, target)`：追加 `Tab::single_terminal()` 并将其窗格设置为 `PaneView::Terminal`，将 `active_workspace` 切换至 `target`；
  - `reconcile_sessions` 自动依据目标工作区固定 `cwd` 启动 PTY。
- **打开 Prompt**：
  - 展开 Prompt（`prompt_collapsed = false`），聚焦 Prompt（`prompt_focused = true`）；
  - 更新 Prompt 根路径并触发提及扫描根重置（`set_mention_root`）。

### 3.3 渲染层（`src/ui/`）
- **菜单组件尺寸与文本计算（`src/ui/widgets/menu.rs`）**：
  - 改用 `unicode_width::UnicodeWidthStr::width` 计算每个标签及标题的视觉列宽。
  - 菜单宽度设为 `max_label_width + HORIZONTAL_PADDING`，若小于 `max_title_width` 则扩充，仅在超出 `screen.width` 时做兜底钳制。
  - 渲染文本时，在宽度充裕时不强加 `ellipsize`，保证文本 100% 完整显示。
- **Prompt 顶栏路径展示（`src/ui/mod.rs`）**：
  - 优先读取 Prompt 当前生效的显式绑定路径末段名，回退工作区路径名。
- **文案收敛（`src/ui/text.rs`）**：
  - 新增常量：
    - `MENU_NEW_TERMINAL = "New terminal"`
    - `MENU_OPEN_PROMPT = "Open prompt"`
    - `MENU_SWITCH_TO_WORKSPACE_CWD = "Switch to ws path"`
    - `CONFIRM_SWITCH_CWD_TITLE = "switch to ws path"`
  - 新增函数：
    - `confirm_switch_cwd_question(path: &str) -> String`（输出 `switch cwd to "{path}"?`）

## 4. 边界与不变量

- 生产代码严格禁止 `unwrap()` / `expect()`。
- 遵循 `app/`（纯状态更新）与 `ui/`（纯只读渲染）的严格边界，UI 层不得触发状态改变或执行 IO。
- 终端切换路径的二次确认框确保前台进程在用户知情并许可的前提下执行 `cd` 注入。
- 特殊字符防护：对注入的路径进行安全转义与双引号包裹，杜绝命令注入。
