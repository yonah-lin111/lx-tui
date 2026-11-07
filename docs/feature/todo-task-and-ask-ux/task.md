# Todo 面板折行缩进、后台任务回到底部与 Ask 交互重构 — 执行任务清单

> **状态**：全部阶段（阶段 0 ~ 阶段 4）均已完成、通过 Code Review 修复并合并入 `dev`
> **关联设计文档**：`docs/feature/todo-task-and-ask-ux/design.md`
> **执行策略**：使用最多 3 个并行 subagent 按模块边界推进，完成后统一执行编译、Clippy 与针对性单元测试校验。

---

## 阶段划分与详细任务清单

### 阶段 0：需求盘问与技术设计（已完成）
- [x] **0.1 代码检索与现状摸底**：结合 `codegraph` 与 `codebase-memory-mcp` 检索 `src/ui/right_panel/todo/`、`src/ui/main_content/lx/`、`src/chat/agent/` 与 `opencode-dev` 表单实现。
- [x] **0.2 Grill Me 边界盘问（共 7 轮全部确认）**：
  - [x] Q1：确认 Todo 条目最多显示 3 行、续行 6 列缩进（与 `P(n)` 左对齐）、第 3 行末 `…` 截断、整块高亮底色，以及行内编辑 1~3 行软折行与视口内滚动。
  - [x] Q2：确认后台任务面板「回到底部」按钮同时覆盖 `Processes` 与 `Agents` 两个 Tab，复用 `chat_bar` 位置与样式，点击回到最底部并恢复跟随。
  - [x] Q3：确认 Ask 面板取消左右分栏、选项描述缩进 6 列完整换行、`preview` 移至所有选项正下方且共用面板右侧唯一一条滚动条（无 `preview` 时完全隐藏，不显示 `(no preview)`）。
  - [x] Q4：确认所有题目（单选/多选）与 `Submit` 汇总页均移除行内提交按钮，直接使用键盘操作；单题单选点击或回车直接选中提交，多题或单题多选通过 `Submit` 汇总 Tab 按 `Enter` 提交。
  - [x] Q5：确认 Ask 自定义输入（`Other...`）移除底部独立编辑框，原地变为 6 列缩进的 1~3 行行内输入框；高亮在 `Other...` 时键入任意字符（含数字 `1..9`）自动进入行内编辑。
  - [x] Q6：确认 Ask 面板对齐 `opencode-dev`：单题单选无顶部 Tab，多题或单题多选显示 `1 题一  2 题二  Submit` Tab 栏，支持 `←`/`→`、`Tab`/`Shift+Tab` 循环切换与 `↑`/`↓`、`1..9` 选项导航。
  - [x] Q7：确认 Ask 期间隐藏 `chat_bar` loading 与计时、停止历史步骤方块呼吸，并在 Ask 结束/取消及会话重载时将等待耗时从轮次耗时中剔除。
- [x] **0.3 编写设计文档与任务文档**：输出 `docs/feature/todo-task-and-ask-ux/design.md` 与 `docs/feature/todo-task-and-ask-ux/task.md`。

---

### 阶段 1：右侧 Todo 面板条目最多 3 行折行缩进与行内多行编辑
**目标文件**：`src/ui/widgets/input.rs`、`src/ui/right_panel/todo.rs`、`src/app/state/todo.rs`、`src/app/update/todo.rs`、`src/ui/right_panel/todo/tests.rs`

- [x] **1.1 多行软折行与 3 行截断/视口计算原语（`src/ui/widgets/input.rs`）**
  - 实现 `wrap_clamped_lines(text: &str, width: usize, max_rows: usize) -> Vec<String>`：按 `UnicodeWidthChar` 计算列宽软折行，超过 `max_rows`（3 行）时在第 3 行末尾留出显示宽度追加 `…`。
  - 实现 `multiline_view(text: &str, cursor: usize, width: usize, max_rows: usize) -> MultilineInputView`：将编辑文本按 `width` 软折行为视觉行列表（含每行起止字节与字符索引），计算光标所在视觉行与列，并在超过 3 行时以 `cursor_row` 为基准计算可见的 `1..=3` 行滑动窗口。
  - 实现 `multiline_char_index_at(text: &str, cursor: usize, width: usize, max_rows: usize, visual_row_offset: usize, col: u16) -> usize`：将鼠标点击的行偏移与列偏移还原为全文字符索引。

- [x] **1.2 Todo 列表动态 1~3 行布局与渲染（`src/ui/right_panel/todo.rs`）**
  - 定义 `pub const MAX_TODO_ITEM_ROWS: usize = 3`。
  - 构建统一的行几何计算函数：为可见范围内的每条 Todo（含新建幽灵行）计算占用的视觉行数 `1..=3` 及起始 `y` 坐标。
  - 更新 `render_row` 与 `render_edit_row`：
    - 选中或悬停时，将该条目占用的 `1..=3` 行矩形整块铺设 `style::selection()`；
    - 首行绘制 `❯`、`[ ]`/`[~]`/`[x]`、`P0`..`P3` 徽标及第 1 行正文；
    - 第 2、3 行左侧 6 列留空（与 `row_priority_x(list)` 即 `P(n)` 左边缘对齐），绘制后续折行正文或编辑框可见行及选区高亮，并返回硬件光标坐标。
  - 同步更新命中测试函数 `row_at`、`priority_at`（仅首行徽标列命中）、`edit_input_char_at`（支持多行点击/拖拽选区）。

- [x] **1.3 Todo 单元测试更新与新增（`src/ui/right_panel/todo/tests.rs`）**
  - 新增 2~3 行自动折行与 6 列缩进对齐断言测试；
  - 新增超 3 行文本末尾 `…` 截断断言测试；
  - 新增多行编辑态展开、超 3 行视口滚动及光标坐标测试。

---

### 阶段 2：后台任务面板（Processes · Agents）内容区「回到底部」按钮
**目标文件**：`src/app/state/lx/task.rs`、`src/ui/main_content/lx/chat_bar.rs`、`src/ui/main_content/lx/history_hit.rs`、`src/app/update/chat/viewport.rs`、`src/app/update/pane/task_page.rs`、`src/event/mouse/`

- [x] **2.1 后台任务面板触底状态判定（`src/app/state/lx/task.rs`）**
  - 实现 `TaskPage::is_at_bottom(&self, agents: &AgentsState) -> bool`：
    - `TaskTab::Processes`：`self.output_visible == 0 || self.output_scroll >= self.output_max_scroll()`；
    - `TaskTab::Agents`：当前选中 `SubagentRun` 消息为空或 `run.follow == true`。

- [x] **2.2 `chat_bar` 在后台任务面板下的显隐与布局（`src/ui/main_content/lx/chat_bar.rs`）**
  - 修改 `chat_bar::visible(lx)` 与 `chat_bar::jump_visible(lx)`：当 `lx.task.is_some()` 时，若 `!page.is_at_bottom(&lx.agents)` 则显示 `chat_bar` 并绘制右侧 `↓ jump to bottom` 按钮（此时 `lx.rs::layout` 中的 `out.task_output` 自动让出底部 1 行）。

- [x] **2.3 鼠标悬停、点击触发与滚轮接线（`src/ui/main_content/lx/history_hit.rs`、`src/app/update/chat/viewport.rs`、`src/event/mouse/`）**
  - 更新 `jump_lx_history_bottom(state, pane)`：当 `target.lx.task` 打开时，按当前 Tab 执行 `page.pin_output_to_tail()`（Processes）或将当前子 Agent 的 `run.follow` 设为 `true`（Agents），并重置 `jump_bottom_hover = false`；
  - 在 `event/mouse/press.rs`、`event/mouse/hover.rs` 与 `event/mouse/wheel.rs` 中接入后台任务面板的 `jump_bottom_hit` 点击/悬停，以及 `Processes` 输出区的鼠标滚轮上下滚动。

- [x] **2.4 后台任务面板回到底部单元测试（`src/ui/main_content/lx/chat_bar/tests/`、`src/ui/main_content/lx/tests.rs`）**
  - 测试 `Processes` 日志上翻后出现 `↓ jump to bottom`、点击后回到尾部且按钮消失；
  - 测试 `Agents` 子 Agent 消息流上翻后出现 `↓ jump to bottom`、点击后恢复 `follow = true`。

---

### 阶段 3：Ask 工具交互全面重构与暂停计时/Loading
**目标文件**：`src/app/state/lx/ask.rs`、`src/ui/main_content/lx/ask.rs`、`src/app/update/chat/ask.rs`、`src/app/update/chat/mod.rs`、`src/event/chat/ask.rs`、`src/ui/main_content/lx/chat_bar.rs`、`src/ui/main_content/lx/history.rs`、`src/ui/text/agent.rs`

- [x] **3.1 重构 `AskState` 状态模型（`src/app/state/lx/ask.rs`）**
  - 对齐 `opencode-dev`：新增 `is_single_select_only()`；单题单选无顶部 Tab，多题或单题多选包含 `Submit` 汇总 Tab（`tab_len() = questions.len() + 1`）；
  - 移除行内提交按钮（`has_submit_row` / `submit_row` / `RowKind::Submit` / `AskHit::Submit`）；
  - 新增 `started_at_ms: i64`（初始化为 `crate::db::now_ms()`）与 `question_has_preview(&self) -> bool`；
  - 更新 `MAX_OTHER_EDIT_ROWS = 3` 行内编辑高度同步，移除独立的 `preview_scroll` 与 `preview_visible`，统一使用单一 `viewport` 滚动。

- [x] **3.2 重构 Ask 面板布局与渲染（`src/ui/main_content/lx/ask.rs`、`src/ui/text/agent.rs`）**
  - 移除左右分栏（`SPLIT_MIN_WIDTH`）、底部独立编辑框（`lay.editor`、`draw_editor`）与行内提交按钮；
  - **选项描述完整换行**：将 `RowKind::OptionDesc` 按 `options.width - ROW_PREFIX` 全宽软折行展开为多行，每行缩进 6 列绘制；
  - **Other 原地行内编辑**：将 `RowKind::Other` 在编辑态或有内容时按 `1..=3` 行展开，左侧 6 列绘制 `❯ (•) ` / `❯ [x] `，右侧原地绘制 `AskEditor` 的 1~3 行软折行内容、空态占位符 `Other...`、选区反显，并返回硬件光标坐标；
  - **Preview 置底且与问题选项共用唯一滚动条**：移除字符图独立滚动条，将 `PreviewGap` / `PreviewLine` / `PreviewPlaceholder` 直接纳入统一的 `rows_model`，使问题选项与字符图共用面板最右侧唯一一条滚动条；本题无任何 `preview` 时完全隐藏预览区。

- [x] **3.3 重构 Ask 键盘与鼠标交互（对齐 `opencode-dev`，`src/app/update/chat/ask.rs`）**
  - 支持 `←`/`→`、`Tab`/`Shift+Tab` 在各题与 `Submit` 汇总 Tab 间循环切换，`↑`/`↓` 循环切换选项，`1..9` 数字键快捷选择；
  - 单题单选按 `Enter` 或鼠标单击立即提交；多题单选按 `Enter` 或鼠标单击选中并自动切到下一 Tab；多选按 `Enter`/`Space` 切换勾选；
  - 高亮停在 `Other...` 时直接打字自动进入行内编辑并插入字符；行内编辑首行按 `↑` 自动退出编辑并回到上一项，按 `Tab`/`Shift+Tab` 自动保存并切题，按 `Enter` 确认输入，按 `Esc` 退出行内编辑。

- [x] **3.4 Ask 过程中暂停 Agent 计时与 Loading（`src/ui/main_content/lx/chat_bar.rs`、`src/ui/main_content/lx/history.rs`、`src/app/update/chat/mod.rs`、`src/event/chat/ask.rs`）**
  - `chat_bar::loading_visible`：当 `lx.ask.is_some()` 时返回 `false`；
  - `history.rs`：当 `lx.ask.is_some()` 时 `animating = false`，停止步骤方块呼吸动画；
  - `advance_lx_loading`：当 `target.lx.ask.is_some()` 时跳过 `turn_elapsed_ms` 刷新与主会话 `loading_frame` 递增；
  - 在 `submit_all` 与 `event::chat::ask::clear` 中，将 `now_ms() - ask.started_at_ms` 累加补偿进 `turn_started_at_ms`。

- [x] **3.5 Ask 单元测试与回归测试更新（`src/ui/main_content/lx/ask/tests.rs`、`src/app/update/chat/tests/ask.rs`、`src/app/state/lx/ask/tests.rs`）**
  - 更新既有测试以匹配对齐 `opencode-dev` 的 `Submit` 汇总 Tab、无行内提交按钮、原地行内 `Other...` 编辑（含数字直打与折行同源）、Preview 置底共用统一滚动条及暂停计时的最新行为。

---

### 阶段 4：规范文档同步、Code Review 与全链路修复验证
- [x] **4.1 同步更新 `docs/standards/tui-design-requirements.md`**
- [x] **4.2 输出代码审查报告 `docs/feature/todo-task-and-ask-ux/review.md`（按 P0 ~ P2 排序）**
- [x] **4.3 按 `review.md` 逐项修复 P0 ~ P2 问题并拆分超限文件**
- [x] **4.4 执行代码格式化、Clippy 静态检查与全量测试套件验证**
