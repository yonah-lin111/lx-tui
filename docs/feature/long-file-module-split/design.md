# 超长生产文件按职责拆分设计

- 状态：待用户审核；本文与 `task.md` 获明确确认后才可修改生产代码。
- 范围：`src/event/mod.rs`、`src/app/update.rs`、`src/app/state.rs`、`src/ui/overlay.rs`。
- 性质：结构性重构；保持功能、交互、公开调用路径和模块依赖方向。
- 依据：`docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/tui-design-requirements.md`。

## 1. 目标、口径与证据

`code-writing-standards.md` 规定生产文件超过 1000 行必须按职责拆分，子文件控制在 500 行左右，不允许仅压缩排版。这里按工作区当前源码的物理行数统计 `src/**/*.rs`，排除 `tests.rs` 和 `tests/`；测试代码不纳入超限判定。`wc -l` 是盘点口径，实施验收时还应检查末行无换行的文件，避免低估一行。工作区外的 `.worktrees/`、生成目录及文档不属于此次生产文件盘点。

| 文件 | 当前行数 | 判定 | 本次处理原因 |
| --- | ---: | --- | --- |
| `src/event/mod.rs` | 2625 | 超过硬上限 | 事件循环、输入分派、鼠标操作、会话同步和浮层点击混在一起 |
| `src/app/update.rs` | 2492 | 超过硬上限 | 编辑、选择、布局、工作区、菜单及浮层状态转换混在一起 |
| `src/app/state.rs` | 1131 | 超过硬上限 | 顶层状态、工作区模型、窗格模型、LX 编辑器及查询方法混在一起 |
| `src/ui/overlay.rs` | 975 | 用户明确纳入 | 菜单、多个模态窗、命中几何和绘制同处一个文件，接近硬上限 |

其余生产 `.rs` 文件当前均不超过 1000 行。仅统计超限不等于每个接近阈值的文件都应拆；例如 `src/ui/mod.rs` 为 949 行，但不在本轮范围。代码关系经 codegraph 与 codebase memory 的符号、调用和覆盖信息交叉核对；四个目标文件没有记录到索引跳过或解析缺口。图索引对跨文件动态关系仍是辅助证据，实施以编译和现有测试为最终校验。

## 2. 已确认决策

| 编号 | 用户确认的边界 | 落地解释 |
| --- | --- | --- |
| D1 | 三个超限文件加 `src/ui/overlay.rs` | 本轮只改上述四个生产模块及其必要的同层子模块、测试；其他文件只有编译确需的路径调整才触碰 |
| D2 | 允许拆开 `handle_terminal_event` | 以薄入口按键盘、粘贴、鼠标事件分派；鼠标按按下、拖动、释放、滚轮、移动分段，保持原命中顺序 |
| D3 | 约 500 行是目标，1000 行是硬上限 | 完整职责优先；略超 500 行须在实施记录说明，任何生产文件不得超过 1000 行 |
| D4 | 现有对外路径保持可用 | 例如 `app::state::Lx`、`app::update::activate_menu`、`ui::overlay::render` 与 `event::run` 继续可用；由原模块重导出或转发 |

## 3. 现状数据流与约束

```text
crossterm / PTY / 后台结果
          |
          v
event::run_loop -> 事件分派 -> app::update -> AppState
          |                            |
          |                            +-> Effect / 请求队列（由 event 执行）
          +-> ui::render(&AppState) -> Buffer
```

- `event` 是唯一事件循环，负责外部事件、后台任务、PTY、设置服务和绘制调度；状态修改通过 `app::update` 或事件循环内明确的状态操作发生。
- `app` 保持纯数据和纯状态转换，不引入 IO、PTY 句柄、UI 渲染或线程操作；`state` 保存数据模型和只读查询，`update` 保存动作到状态的转换。
- `ui/overlay` 只读 `AppState`，几何计算、鼠标命中与绘制共享同一布局规则；不得通过渲染修改状态或执行 IO。
- 子模块留在原所有者目录；遵守 `event -> app/ui/pty`、`ui -> app`、`app -> layout/terminal/config` 的现有依赖方向。只在边界处重导出公开能力，不让其他顶层模块深层导入实现文件。
- 这次不引入新 crate，不重做输入协议、状态形状、持久化 schema、终端生命周期或鼠标恢复顺序；若移动到 `src/tui/` 的恢复代码才需要专门的 `tui::tests` 顺序校验，本设计不移动该路径。

## 4. 拆分方案

下表给的是职责和实施时的目标文件，不是机械按行号剪切。行数为按现有函数块估算的目标区间；移动后以实际 `cargo fmt` 结果复核。若单一职责超过目标，优先再找自然边界；禁止把同一操作拆成仅为减行数的碎文件。

### 4.1 `src/event/mod.rs`

图关系显示 `handle_terminal_event` 位于约 575–1661 行，单函数约 1087 行，向大量 `app::update`、`ui` 命中函数及事件辅助函数调用。只移动周边函数不能满足硬上限，因此必须先拆内部事件分派。

| 目标 | 职责 | 现有来源与预计规模 |
| --- | --- | --- |
| `event/mod.rs` | `AppEvent`、`run`/`run_loop`、通道和帧调度、模块声明 | 入口约 270 行，移走命令重载和会话几何后约 280–400 行 |
| `event/commands.rs` | 自定义命令重载、作用域根、命令视图转换 | 现约 277–400 行，约 125 行 |
| `event/session.rs` | PTY 会话对齐、cwd/agent 轮询、几何与 resize | 现约 402–573 行，约 180 行；如与 `mod.rs` 共享 `Geometry`，只开放父模块所需可见性 |
| `event/input.rs` | `TerminalEvent` 薄分派、按键路由和粘贴进焦点窗格 | 从约 575–715 行抽出；现有 `event/paste.rs` 保留 prompt 粘贴语义 |
| `event/mouse/mod.rs` | 鼠标种类分派；按原优先级调用各处理器 | 约 50–120 行 |
| `event/mouse/press.rs` | 左/右键按下，控件优先级、菜单打开、焦点切换 | 原按下分支约 720–1120 行；控制在约 400–500 行 |
| `event/mouse/drag.rs` | 拖动和释放：调整大小、选择、滚动条与工作区排序 | 原拖动/释放分支，约 300–450 行 |
| `event/mouse/wheel.rs` | 提及/命令面板、选区、窗格及侧栏滚轮路由 | 原滚轮分支及辅助函数，约 200–300 行 |
| `event/mouse/hover.rs` | 移动悬停、tooltip、分割线与指针形状 | 原移动分支，约 200–300 行 |
| `event/mouse/hit.rs` | 与鼠标交互紧邻的命中和几何辅助函数 | 原约 2428–2622 行中的相关函数，约 150–250 行 |
| `event/overlay.rs` | 浮层点击及关闭、右键菜单入口 | 原约 1862–2223 行，约 350–450 行 |

`event/git.rs`、`event/trust.rs`、`event/paste.rs` 已存在，不创建同义模块。`pump_mention_scan`、`pump_prompt_worktree`、`handle_app_event` 按“后台结果与请求”归属决定留在 `mod.rs` 或小的 `background.rs`；只有实际留下超过 500 行或同时承担无关职责时才创建 `background.rs`。`handle_terminal_event` 在父模块保留可供现有 `event/tests.rs` 调用的薄转发，或把测试入口同步改为父模块 `use`；不更改事件的同步返回/dirty 语义。

关键顺序：浮层和 toast 命中优先于底层控件；退出、标签、设置、折叠/拖拽边界、侧栏、提示词面板及窗格命中保持原先先后。鼠标拖拽、选区复制、滚轮对打开面板的截获、指针形状恢复和 `dirty` 标记不能因抽函数改变。鼠标处理器不直接绕过 `app::update` 创建新的状态修改路径。

### 4.2 `src/app/update.rs`

目前 `update.rs` 已有 `prompt.rs`、`worktree.rs`、`trust.rs` 子模块。新增文件按状态变化对象划分，避免重复造一套“manager/service”。

| 目标 | 职责 | 现有来源与预计规模 |
| --- | --- | --- |
| `update.rs` | `apply(Action)` 和原公开 API 的 `pub use`/薄转发；各子模块声明 | 约 80–200 行 |
| `update/editor.rs` | `apply_editor`、面板路由、草稿切换与钉住、候选选择和模板块编辑动作 | 原约 79–545 行；目标约 400–500 行 |
| `update/pane.rs` | LX 输入、窗格焦点/视图、PTY 输出喂入、尺寸同步与 pane 元数据 | 原约 550–802 行，合并紧邻的 pane 操作，约 300–450 行 |
| `update/selection.rs` | prompt、终端和 LX 选区，复制范围、自动滚动 | 原约 805–1080 行，约 300–400 行 |
| `update/interaction.rs` | panel/sidebar/pane resize、hover、toast、tooltip、tick/deadline | 原约 1082–1305 行；若超过约 500 行，再按定时器职责抽 `timers.rs` |
| `update/workspace.rs` | 工作区创建/切换/cwd 确认和关闭 | 原约 1308–1623、2225–2250 行，预计约 350–450 行 |
| `update/workspace_list.rs` | 工作区与 Agents 列表滚动、悬停、排序拖拽及可见区钳制 | 原约 2297–2492 行，预计约 200–300 行 |
| `update/menu.rs` | 打开菜单、选中/激活命令，标签和窗格菜单动作 | 原约 1626–2001 行，约 350–450 行 |
| `update/dialog.rs` | overlay 按键、重命名、新建工作区、关闭确认等提交 | 原约 2006–2222 行，约 220–300 行 |

这里的 `workspace.rs` 是状态更新，和已存在的 `update/worktree.rs`（git worktree 流程）不是同一职责。`activate_menu` 同时被 `event` 和集成测试调用，保留 `app::update::activate_menu`；`apply_overlay_key` 也是公开入口。内部迁移尽量沿用现有签名，避免把几十个 `AppState` 字段包进新的上下文对象。若 `editor.rs` 因整块迁移后超过 500 行，按候选面板和草稿切换的自然分界调整到已有 `update/prompt.rs`，但须避免让该现有 858 行文件继续膨胀。

### 4.3 `src/app/state.rs`

`state.rs` 中 `Lx` 及其编辑方法约占 297–628 行；`AppState` 结构与核心查询约占 58–145、784–1127 行。把 LX 的独立内存编辑模型移出后，仍需将工作区/窗格实体及工作区列表查询按职责迁移，使主文件靠近 500 行。

| 目标 | 职责 | 迁移策略 |
| --- | --- | --- |
| `state.rs` | `AppState`、初始构造、活跃对象查询、全局状态不变量和原路径重导出 | 预计约 450–550 行 |
| `state/lx.rs` | `LxEntry`、`Lx`、撤销快照、文本导航/编辑/面板方法 | 现约 297–628 行，预计约 330–380 行；类型经 `state` 重导出 |
| `state/workspace.rs` | `Workspace`、`PromptDraft`、`WorkspaceGit`、`WorkspaceRow`、工作区标签与名称辅助函数、分组/列表查询 | 预计约 300–450 行；若部分 `impl AppState` 查询放入此文件，仅保持只读或纯内存计算 |
| `state/pane.rs` | `Tab`、`PaneKind`、`PaneView`、`Pane` 及单标签内 pane 管理 | 预计约 140–220 行 |

保持 `AppState.prompt` 与 `Workspace.prompt` 的“显示中草稿只存在一处”不变量、活动工作区/标签索引语义和 pane ID 分配时机。移动 `Lx` 时不修改撤销栈、宽字符移动、面板候选或 `Pane::new` 默认视图。`state/tests.rs` 使用 `use super::*`，从父模块重导出必要类型和辅助函数；新的私有单元测试如有必要放对应 `state/<module>/tests.rs`，测试文件行数不计生产阈值。

### 4.4 `src/ui/overlay.rs`

保留现有 `page_menu.rs` 与 `worktree.rs`。浮层共享 `widgets::modal`/`widgets::menu`，业务特定布局和绘制留在 `ui/overlay/`，不塞进无业务语义的 `ui/widgets/`。

| 目标 | 职责 | 现有来源与预计规模 |
| --- | --- | --- |
| `ui/overlay.rs` | `render` 的 `Overlay` 分派、公开几何/命中 API 重导出、公共按钮类型 | 约 100–180 行 |
| `ui/overlay/menu.rs` | 菜单标题/标签、布局、目标名称解析 | 原约 110–117 和 543–745 行，约 200–260 行 |
| `ui/overlay/dialog.rs` | 重命名、新建节点/工作区、关闭、路径确认和信任模态的几何、命中与绘制 | 原约 120–271、506–540、748–972 行，约 420–550 行；若超目标，按输入模态与确认模态分开 |
| `ui/overlay/save_draft.rs` | 草稿保存选择器的布局、滚动、条目、按钮命中与渲染 | 原约 317–503 行，约 190–250 行 |

`render(frame, screen, state)` 返回的硬件光标位置、模态背景清理、标题和输入截断、宽字符光标定位、最小尺寸降级都保持。`ui/overlay/tests.rs` 通过父模块 `use super::*` 访问布局函数；原公开命中函数仍从 `ui::overlay` 暴露。子模块只读取状态和使用 `ui::text` 文案。

## 5. 不变量、风险与处理

1. **公开路径兼容**：实施前列出四个模块所有 `pub` 项及外部调用者；迁移后父模块保留相同名称、参数和返回类型。`pub(crate)` 或私有符号只按实际父子调用调整可见性，不把内部实现公开为新 API。
2. **鼠标优先级**：原 `if/else if` 命中链是行为契约。拆开 `press` 时按原序分段并保留消费/继续传播的返回语义；不能让点击工具栏、浮层、toast 或滚动条穿透到底层编辑器。
3. **状态更新原子性**：工作区创建、切换、关闭，草稿交换与 pin、菜单确认，继续在一次事件处理内完成；`Effect` 和请求队列仍由 `event` 在原时机处理。
4. **纯渲染**：`ui/overlay` 子模块不得修改 `AppState` 或做 IO；`app/state`/`app/update` 不导入 `ui`/`pty`/`event`。
5. **测试编译可见性**：父模块测试依赖 `use super::*`；必要时用局部 `use` 补足测试符号，不为测试暴露新的公共 API。
6. **行数与代码味道**：每阶段复核全量生产 `.rs` 行数；目标子文件约 500 行，任何单文件不得超过 1000 行。不得靠空行清理、压缩格式、同义包装或层层转发凑数。
7. **回退方式**：每个模块阶段独立完成编译与定向测试；失败时在该阶段定位并修正，不使用 `git reset`/`checkout` 回滚工作区，也不改分支、暂存区或提交状态。

## 6. 验收标准与实施门槛

- 用户先明确审核确认本设计和同目录 `task.md`；确认前只允许编辑这两份文档。
- 四个目标模块按职责拆分；`src/**/*.rs` 生产文件均不超过 1000 行，新增文件尽量约 500 行，超过目标需在任务文档说明职责理由。
- 原公开路径可编译，现有交互、文本编辑、菜单、滚动、终端 PTY、渲染光标行为不变。
- 逐阶段运行对应模块的定向测试；最终按规范执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。格式化命令本身可能改动文件，实施时核对差异只限本任务。
- Agent 无法交互式验证 TUI；交互和外观由用户在实施完成后运行 `cargo run` 复核。
- 不修改 Git 状态；只使用只读 Git 查询核对工作区，不创建分支、不暂存、不提交。
