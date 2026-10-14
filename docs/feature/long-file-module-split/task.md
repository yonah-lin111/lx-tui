# 超长生产文件拆分任务清单

- 状态：设计待用户审核；以下实施项尚未执行。
- 配套设计：`docs/feature/long-file-module-split/design.md`。
- 范围：四个已确认生产模块及必要的同层子模块、定向测试。
- 执行原则：每阶段只搬移或按职责拆分现有逻辑，保留调用路径和行为；不改变 Git 状态。

## 0. 审核门槛与基线

- [x] 阅读三份项目规范、Grill Me 规则及已有 feature 文档格式。
- [x] 使用 codegraph 和 codebase memory 核对目标模块的符号、调用关系及索引覆盖；使用实际源码物理行数确认阈值。
- [x] 用户确认四文件范围：三个超过 1000 行的生产文件，加 975 行的 `ui/overlay.rs`。
- [x] 用户确认允许拆分 `handle_terminal_event` 内部事件分派。
- [x] 用户确认约 500 行是目标、1000 行是硬上限。
- [x] 用户确认现有对外路径继续可用。
- [ ] 用户审核并明确批准 `design.md` 和本任务清单后，才开始修改生产代码。
- [ ] 实施开始前复核工作区状态及四文件当前行数；如源码已变化，更新映射和风险，再执行。

## 1. 实施准备

- [ ] 枚举四个父模块的全部 `pub` 项、`pub(crate)` 项及被外部模块调用的函数；形成迁移清单，确认保留 `event::run`、`app::state::*`、`app::update::*`、`ui::overlay::*` 现有入口。
- [ ] 标出 `event::handle_terminal_event` 原事件分支和鼠标命中优先级，明确每个分支是“消费并返回”还是“继续命中”。
- [ ] 标出 `update.rs` 中纯状态转换和与现有 `prompt/worktree/trust` 子模块的边界；核对 `state.rs` 的类型依赖和 `ui/overlay.rs` 几何与绘制配对。
- [ ] 记录基线的定向测试结果；如已有失败，先记录，不把基线问题归因于拆分。

## 2. `app::state`，先稳定数据边界

- [ ] 新建 `state/lx.rs`：移动 `LxEntry`、`Lx`、快照及 LX 内存编辑方法；父模块重导出原路径。
- [ ] 新建 `state/pane.rs`：移动 `Tab`、`PaneKind`、`PaneView`、`Pane` 及 pane 生命周期方法；保持 `Pane::new` 的默认视图、终端尺寸和 ID 分配。
- [ ] 新建 `state/workspace.rs`：移动 `Workspace`、`PromptDraft`、`WorkspaceGit`、`WorkspaceRow` 及相关工作区命名/分组查询；父模块仍保留 `AppState` 唯一状态定义和外部入口。
- [ ] 检查 `AppState.prompt` 与挂起的 `Workspace.prompt` 的唯一显示草稿不变量；检查 `workspace_rows` 分组顺序、`pane_anywhere` 查询和 `select_all_prompt` 行为。
- [ ] 运行 `cargo test app::state::tests`；如新增 LX 专项测试，仅验证移动时最易破坏的撤销/光标/宽字符场景。
- [ ] 记录格式化后父文件和子文件行数；若父文件仍明显超目标，按只读查询职责进一步整理，但不改变状态数据结构。

## 3. `app::update`，按状态变化对象拆分

- [ ] 以 `update.rs` 保留 `apply(Action)` 和对外入口；用 `pub use` 或薄转发维持现有路径，避免调用者全面改名。
- [ ] 新建 `update/editor.rs`：编辑命令、候选面板、草稿切换与 pin；与既有 `update/prompt.rs` 分工明确，不向其无序塞代码。
- [ ] 新建 `update/pane.rs`：LX 输入、窗格焦点、PTY 输出喂入、窗格尺寸/滚动及元数据状态变更。
- [ ] 新建 `update/selection.rs`：prompt/terminal/LX 选区、复制范围、自动滚动；保持坐标、宽字符和清空时机。
- [ ] 新建 `update/interaction.rs`：resize、hover、toast/tooltip、tick；只有体积和职责证据都支持时才抽 `timers.rs`。
- [ ] 新建 `update/workspace.rs`：工作区创建/切换/cwd/关闭；新建 `update/workspace_list.rs`：列表滚动/排序/悬停/可见区钳制，保持工作区实体操作与列表交互分离。
- [ ] 新建 `update/menu.rs` 与 `update/dialog.rs`：菜单构造/激活和浮层输入/确认分别归属；维持 `activate_menu`、`apply_overlay_key` 等原公开路径。
- [ ] 核对 `Effect` 出箱、git/trust 请求队列、工作区切换时草稿交换和菜单动作的先后顺序。
- [ ] 运行 `cargo test app::update::tests`，再运行受影响的 `app::update::prompt::tests` 与相关集成测试；只在现有测试无法覆盖明确的拆分风险时补少量有行为断言的回归测试。
- [ ] 复核 `update.rs` 与新子文件的职责、行数、`pub` 可见性及 `app -> event/ui/pty` 禁止依赖。

## 4. `event`，拆开超长分派函数

- [ ] 移动命令重载至 `event/commands.rs`、会话/几何辅助至 `event/session.rs`，保留 `run_loop` 的事件调度和帧节流主线。
- [ ] 把按键、粘贴与鼠标事件从 `handle_terminal_event` 抽到 `event/input.rs` 和 `event/mouse/`；父模块保留薄分派及测试入口。
- [ ] 鼠标按下逻辑进 `mouse/press.rs`：按原顺序逐段迁移命中链，检查浮层、toast、退出/标签/设置、折叠/边界、侧栏、prompt 面板、窗格的优先级。
- [ ] 鼠标拖动/释放进 `mouse/drag.rs`：检查 resize、工作区排序、prompt/terminal/LX 选区和复制结束时机。
- [ ] 滚轮进 `mouse/wheel.rs`：检查浮层吞掉、候选面板优先、选区拖动、PTY 鼠标上报与普通滚动互斥。
- [ ] 移动悬停进 `mouse/hover.rs`，命中/几何辅助进 `mouse/hit.rs`：检查 tooltip 定时、按钮高亮和指针形状恢复。
- [ ] 浮层点击及右键菜单进 `event/overlay.rs`；保留点击消耗和 `dirty` 标记语义。
- [ ] 只有 `mod.rs` 留下超过目标或后台职责与事件循环交织时才新增 `event/background.rs`；保留 `AppEvent` 通道、PTY 处理及后台结果投递时序。
- [ ] 运行 `cargo test event::tests`；对命中顺序中现有测试未覆盖的高风险路径补必要用例，例如 toast/浮层遮挡、滚动条点击与编辑器点击互斥、鼠标释放后复制只发生一次。
- [ ] 核对 `event/mod.rs` 及全部新文件行数；`handle_terminal_event` 应成为可快速读懂的薄入口，不允许只是把 1087 行搬到另一个单文件。

## 5. `ui::overlay`，按浮层类型拆分

- [ ] 保留 `ui/overlay.rs` 的 `render` 分派和原公开布局/命中路径；既有 `page_menu.rs`、`worktree.rs` 不重命名。
- [ ] 新建 `ui/overlay/menu.rs`：菜单标题、标签、目标名称和几何；绘制仍使用 `widgets::menu`。
- [ ] 新建 `ui/overlay/dialog.rs`：重命名、新建、关闭与路径确认模态；如实际体积超过目标，再按输入/确认职责拆开。
- [ ] 新建 `ui/overlay/save_draft.rs`：草稿选择器布局、滚动窗口、条目/按钮命中与绘制。
- [ ] 信任模态归入 `dialog.rs`；若输入类与确认类模态合计超过目标，按这两个完整职责进一步拆分。
- [ ] 运行 `cargo test ui::overlay::tests`；保留宽字符输入光标、窄屏换行、各按钮命中与 `render` 的 `Option<(u16,u16)>` 返回语义。
- [ ] 核对新子模块纯渲染、只读状态、文案取自 `ui::text`，并确认 `ui/widgets` 未引入领域数据。

## 6. 最终校验与交付

- [ ] 用相同口径重新统计所有 `src/**/*.rs` 生产文件，列出每个目标父文件和新子文件的实际行数；确认无文件超过 1000 行，说明任何超过约 500 行的职责理由。
- [ ] 运行 `cargo fmt`；检查其变更只涉及本任务文件。
- [ ] 运行 `cargo clippy --all-targets -- -D warnings`。
- [ ] 运行 `cargo test`，覆盖单元与集成测试；如失败，记录命令和具体失败项并修复本次造成的问题。
- [ ] 复核公开路径、模块依赖方向、生产代码无新增 `unwrap()/expect()`，以及无新增重依赖、IO 或 OS 分支。
- [ ] 提交实施结果说明：各文件最终行数、职责位置、测试结果、剩余风险；明确 TUI 交互与外观需用户运行 `cargo run` 人工确认。
- [ ] 不创建分支、不暂存、不提交、不重置工作区；用户没有授权修改 Git 状态。

## 验收时需重点观察的行为

1. 点击浮层、toast、滚动条、工具栏和底层编辑器时，命中互不穿透；右键菜单和键盘菜单执行同一状态动作。
2. 选择/拖动/滚轮在 prompt、终端和 LX 视图之间不串用；鼠标松开后的复制次数与 toast 反馈保持原样。
3. 创建/关闭/切换工作区及标签后，焦点、滚动可见区、prompt pin 和挂起草稿仍指向正确工作区。
4. 宽字符输入、模态框按钮、标题换行、硬件光标定位及窄屏降级与拆分前一致。
5. PTY 输出、后台 git/提及/worktree 结果、帧节流和设置事件仍在原事件循环内按序处理。
