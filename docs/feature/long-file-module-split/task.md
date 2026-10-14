# 超长生产文件拆分任务清单

- 状态：已全部完成并通过全量回归测试。
- 配套设计：`docs/feature/long-file-module-split/design.md`。
- 范围：`src/event/mod.rs`、`src/app/update.rs`、`src/app/state.rs`、`src/ui/overlay.rs`，以及补充拆分的 `src/ui/mod.rs` 与 `src/app/prompt/tree.rs`。
- 执行原则：每阶段只搬移或按职责拆分现有逻辑，保留调用路径和行为；不改变外部公有接口与契约。

## 0. 审核门槛与基线

- [x] 阅读三份项目规范、Grill Me 规则及已有 feature 文档格式。
- [x] 使用 codegraph 和 codebase memory 核对目标模块的符号、调用关系及索引覆盖；使用实际源码物理行数确认阈值。
- [x] 用户确认四文件范围：三个超过 1000 行的生产文件，加 975 行的 `ui/overlay.rs`。
- [x] 用户确认允许拆分 `handle_terminal_event` 内部事件分派。
- [x] 用户确认约 500 行是目标、1000 行是硬上限。
- [x] 用户确认现有对外路径继续可用。
- [x] 用户审核并明确批准 `design.md` 和本任务清单后，开始修改生产代码。
- [x] 实施开始前复核工作区状态及四文件当前行数；确认基线行数与符号依赖。

## 1. 实施准备

- [x] 枚举四个父模块的全部 `pub` 项、`pub(crate)` 项及被外部模块调用的函数；形成迁移清单，确认保留 `event::run`、`app::state::*`、`app::update::*`、`ui::overlay::*` 现有入口。
- [x] 标出 `event::handle_terminal_event` 原事件分支和鼠标命中优先级，明确每个分支是“消费并返回”还是“继续命中”。
- [x] 标出 `update.rs` 中纯状态转换和与现有 `prompt/worktree/trust` 子模块的边界；核对 `state.rs` 的类型依赖和 `ui/overlay.rs` 几何与绘制配对。
- [x] 记录基线的定向测试结果，确保重构前处于完全可测可运行状态。

## 2. `app::state`，稳定数据边界

- [x] 新建 `state/lx.rs`：移动 `LxEntry`、`Lx`、快照及 LX 内存编辑方法；父模块重导出原路径。
- [x] 新建 `state/pane.rs`：移动 `Tab`、`PaneKind`、`PaneView`、`Pane` 及 pane 生命周期方法；保持 `Pane::new` 的默认视图、终端尺寸和 ID 分配。
- [x] 新建 `state/workspace.rs`：移动 `Workspace`、`PromptDraft`、`WorkspaceGit`、`WorkspaceRow` 及相关工作区命名/分组查询；父模块仍保留 `AppState` 唯一状态定义和外部入口。
- [x] 检查 `AppState.prompt` 与挂起的 `Workspace.prompt` 的唯一显示草稿不变量；检查 `workspace_rows` 分组顺序、`pane_anywhere` 查询和 `select_all_prompt` 行为。
- [x] 运行 `cargo test app::state::tests`；验证撤销/光标/宽字符/标签切换等场景。
- [x] 记录格式化后父文件和子文件行数：`state.rs` 从 1131 行降为 350 行，子文件 `lx.rs` 339 行、`pane.rs` 180 行、`workspace.rs` 283 行。

## 3. `app::update`，按状态变化对象拆分

- [x] 以 `update.rs` 保留 `apply(Action)` 和对外入口；用 `pub use` 维持现有路径，避免调用者改名。
- [x] 新建 `update/editor.rs`：编辑命令、候选面板、草稿切换与 pin；与既有 `update/prompt.rs` 分工明确。
- [x] 新建 `update/pane.rs`：LX 输入、窗格焦点、PTY 输出喂入、窗格尺寸/滚动及元数据状态变更。
- [x] 新建 `update/selection.rs`：prompt/terminal/LX 选区、复制范围、自动滚动；保持坐标、宽字符和清空时机。
- [x] 新建 `update/interaction.rs`：resize、hover、toast/tooltip、tick。
- [x] 新建 `update/workspace.rs`：工作区创建/切换/cwd/关闭；新建 `update/workspace_list.rs`：列表滚动/排序/悬停/可见区钳制，保持工作区实体操作与列表交互分离。
- [x] 新建 `update/menu.rs` 与 `update/dialog.rs`：菜单构造/激活和浮层输入/确认分别归属；维持 `activate_menu`、`apply_overlay_key` 等原公开路径。
- [x] 核对 `Effect` 出箱、git/trust 请求队列、工作区切换时草稿交换和菜单动作的先后顺序。
- [x] 运行 `cargo test app::update::tests`，验证受影响子模块无回归。
- [x] 记录格式化后各文件行数：`update.rs` 从 2492 行降至 58 行，子文件均控制在 200–511 行以内。

## 4. `event`，拆开超长分派函数

- [x] 移动命令重载至 `event/commands.rs`、会话/几何辅助至 `event/session.rs`，保留 `run_loop` 的事件调度和帧节流主线。
- [x] 把按键、粘贴与鼠标事件从 `handle_terminal_event` 抽到 `event/input.rs` 和 `event/mouse/`；父模块保留薄分派及测试入口。
- [x] 鼠标按下逻辑进 `mouse/press.rs`：按原顺序逐段迁移命中链，检查浮层、toast、退出/标签/设置、折叠/边界、侧栏、prompt 面板、窗格的优先级。
- [x] 鼠标拖动/释放进 `mouse/drag.rs`：检查 resize、工作区排序、prompt/terminal/LX 选区和复制结束时机。
- [x] 滚轮进 `mouse/wheel.rs`：检查浮层吞掉、候选面板优先、选区拖动、PTY 鼠标上报与普通滚动互斥。
- [x] 移动悬停进 `mouse/hover.rs`，命中/几何辅助进 `mouse/hit.rs`：检查 tooltip 定时、按钮高亮和指针形状恢复。
- [x] 浮层点击及右键菜单进 `event/overlay.rs`；保留点击消耗和 `dirty` 标记语义。
- [x] 运行 `cargo test event::tests`；全量通过。
- [x] 记录格式化后行数：`event/mod.rs` 从 2625 行降为 430 行，子文件均控制在 44–401 行之间。

## 5. `ui::overlay`，按浮层类型拆分

- [x] 保留 `ui/overlay.rs` 的 `render` 分派和原公开布局/命中路径；既有 `page_menu.rs`、`worktree.rs` 保持不动。
- [x] 新建 `ui/overlay/menu.rs`：菜单标题、标签、目标名称和几何；绘制仍使用 `widgets::menu`。
- [x] 新建 `ui/overlay/dialog.rs`：重命名、新建、关闭与路径确认模态及信任模态。
- [x] 新建 `ui/overlay/save_draft.rs`：草稿选择器布局、滚动窗口、条目/按钮命中与绘制。
- [x] 运行 `cargo test ui::overlay::tests`；保持全部用例通过。
- [x] 记录格式化后行数：`ui/overlay.rs` 从 975 行降为 80 行，子模块最大 597 行。

## 6. 补充拆分（按用户追评）：`ui/mod.rs` 与 `app/prompt/tree.rs`

- [x] `src/ui/mod.rs`（原 949 行）：按渲染职责拆分为：
  - `src/ui/mod.rs`: 139 行（模块声明与顶级渲染入口）
  - `src/ui/controls.rs`: 297 行（底部按键栏与控制栏）
  - `src/ui/panes.rs`: 319 行（主窗格布局、终端/LX 窗格与分割线渲染）
  - `src/ui/prompt_frame.rs`: 217 行（Prompt 输入框与外框骨架渲染）
- [x] `src/app/prompt/tree.rs`（原 944 行）：按数据模型与面板交互拆分为：
  - `src/app/prompt/tree.rs`: 508 行（树形数据结构、节点操作与模型状态）
  - `src/app/prompt/tree/panel.rs`: 447 行（树形候选面板、交互状态与渲染适配）

## 7. 最终校验与交付

- [x] 统计全量 `src/**/*.rs` 生产文件，确认**零文件超过 1000 行**（最大生产文件为 920 行）。
- [x] 运行 `cargo fmt --check` 保证代码风格整洁规范。
- [x] 运行 `cargo clippy --all-targets -- -D warnings` 保证零 warning。
- [x] 运行 `cargo test`，全套 1331 个测试通过。
- [x] 复核公开路径、模块依赖方向、生产代码无新增 `unwrap()/expect()`，以及无新增重依赖、IO 或 OS 分支。

## 最终文件行数统计一览表

| 原始文件 | 原行数 | 拆分后文件及行数 |
| --- | ---: | --- |
| `src/event/mod.rs` | 2625 | `event/mod.rs` (430), `event/commands.rs` (125), `event/session.rs` (178), `event/input.rs` (152), `event/overlay.rs` (370), `event/mouse/mod.rs` (44), `event/mouse/press.rs` (401), `event/mouse/drag.rs` (326), `event/mouse/wheel.rs` (252), `event/mouse/hover.rs` (211), `event/mouse/hit.rs` (196) |
| `src/app/update.rs` | 2492 | `update.rs` (58), `update/editor.rs` (511), `update/pane.rs` (256), `update/selection.rs` (279), `update/interaction.rs` (229), `update/workspace.rs` (355), `update/workspace_list.rs` (202), `update/menu.rs` (382), `update/dialog.rs` (223) |
| `src/app/state.rs` | 1131 | `state.rs` (350), `state/lx.rs` (339), `state/pane.rs` (180), `state/workspace.rs` (283) |
| `src/ui/overlay.rs` | 975 | `ui/overlay.rs` (80), `ui/overlay/dialog.rs` (597), `ui/overlay/menu.rs` (209), `ui/overlay/save_draft.rs` (190) |
| `src/ui/mod.rs` | 949 | `ui/mod.rs` (139), `ui/controls.rs` (297), `ui/panes.rs` (319), `ui/prompt_frame.rs` (217) |
| `src/app/prompt/tree.rs` | 944 | `app/prompt/tree.rs` (508), `app/prompt/tree/panel.rs` (447) |
