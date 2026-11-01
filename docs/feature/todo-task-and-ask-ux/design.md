# Todo 面板折行缩进、后台任务回到底部与 Ask 交互重构 — 技术设计文档

- **任务目录**：`docs/feature/todo-task-and-ask-ux/`
- **关联文档**：`docs/feature/todo-task-and-ask-ux/task.md`
- **涉及模块**：
  - 右侧 Todo 面板：`src/ui/right_panel/todo.rs`、`src/ui/widgets/input.rs`、`src/app/state/todo.rs`、`src/app/update/todo.rs`、`src/event/mouse/`
  - 后台任务面板与聊天栏：`src/ui/main_content/lx.rs`、`src/ui/main_content/lx/chat_bar.rs`、`src/ui/main_content/lx/task.rs`、`src/ui/main_content/lx/history_hit.rs`、`src/app/state/lx/task.rs`、`src/app/update/pane/task_page.rs`、`src/app/update/chat/viewport.rs`
  - Ask 工具交互与计时控制：`src/ui/main_content/lx/ask.rs`、`src/app/state/lx/ask.rs`、`src/app/update/chat/ask.rs`、`src/app/update/chat/mod.rs`、`src/event/chat/ask.rs`、`src/ui/main_content/lx/history.rs`、`src/ui/text.rs`、`src/ui/text/agent.rs`
- **状态**：已完成并通过全部验证（待用户确认后合并回 `dev`）

---

## 1. 背景与目标

本次任务聚焦于提升 `lx-tui` 三个核心高频交互区域的可用性与视觉一致性：

1. **右侧面板 Todo 条目多行缩进显示与行内多行编辑（`src/ui/right_panel/todo/`）**：
   - 现状：Todo 条目固定为 1 行高，超出面板宽度直接硬截断；双击或新建进入行内编辑时仅支持单行水平滚动，长待办无法完整阅读与编辑。
   - 目标：Todo 条目支持自动软折行，**最多显示 3 行内容**；第 2、3 行缩进 9 列与首行正文左对齐，超出 3 行在第 3 行末尾以 `…` 截断；行内编辑同样原地支持软折行并最多展开至 3 行高（超出 3 行在 3 行视口内随光标上下滚动）。
2. **后台任务面板内容区新增「回到底部」按钮（`src/ui/main_content/lx/`）**：
   - 现状：主 Agent 历史消息列表向上滚动后，在输入框上沿有 `↓ jump to bottom` 按钮；但打开后台任务面板（`Processes · Agents`）并在上方日志或子 Agent 详情区向上翻阅后，没有「回到底部」按钮，且 `chat_bar` 在任务面板打开时被整体隐藏。
   - 目标：参考主 Agent 的按钮样式与位置，在后台任务面板下方列表顶部分隔线正上方一行（`chat_bar` 位置），当 `Processes` 日志输出区或 `Agents` 消息详情区离开最底部时显示 `↓ jump to bottom` 按钮，点击后立即滚回最底部并恢复自动跟随。
3. **Ask 工具交互全面重构（`src/ui/main_content/lx/` & `src/chat/agent/`）**：
   - **Preview 移至所有问题选项下方 + 选项描述完整换行**：移除原有的左右分栏（`SPLIT_MIN_WIDTH`），选项列表占满全宽；选项描述（`description`）在缩进 6 列后支持自动多行换行完整显示；当题目中有选项包含 `preview` 时，将 `preview` 区域放置在所有选项（含 `Other` 与提交按钮）的正下方，若当前题没有任何选项带 `preview` 则完全隐藏预览区（不再渲染无意义的 `(no preview)`）。
   - **提交按钮规则、多题流转与键盘导航收敛**：移除多题的 `Confirm` 汇总 Tab；单题单选不显示提交按钮（选中即提交），单题多选在 `Other` 下方显示提交按钮；多题时前 `N-1` 题不显示提交按钮，**最后一题底部显示提交按钮**；按钮采用弹窗同款实心胶囊样式（` ↵ submit `，`style::button_primary()` / `style::button_secondary()`）；面板内**只保留 `↑`/`↓` 方向键切换条目**（含聚焦到底部提交按钮），移除 `Tab`/`←`/`→`/`j`/`k`/`1..9` 快捷键，多选或最后一题必须将焦点移至提交按钮按 `Enter`（或鼠标点击提交按钮）才执行提交。
   - **自定义输入（`Other...`）原地变为多行输入框**：参考 `opencode-dev` 与 Todo item，选中 `Other...` 时不再在面板底部弹出独立分割线编辑框（移除 `lay.editor`），而是直接将 `Other` 这一项原地变成缩进对齐的 1~3 行软折行输入框。
   - **Ask 等待期间暂停 Agent 计时与 Loading**：当 `ask` 面板打开等待用户作答时，隐藏 `chat_bar` 的 `Running tools... (1m 17s · ↓86 tok)` loading 行，停止历史步骤方块呼吸动画，并暂停/补偿轮次计时，确保用户思考作答耗时不计入 Agent 执行耗时与 `tok/s`。

---

## 2. 已确认的关键决策（Grill Me 盘问结论汇总）

| 编号 | 决策维度 | 最终确认结论 |
| :--- | :--- | :--- |
| **Q1** | Todo item 最多 3 行、缩进与行内编辑 | 首行保留 9 列前缀（`❯ [ ] P0 `），第 2、3 行缩进 9 列与首行正文左对齐；非编辑态最多显示 3 行、超长在第 3 行末用 `…` 截断，选中/悬停底色铺满该条目全部视觉行（1~3 行）；编辑态原地支持软折行并最多展开至 3 行高，超过 3 行在 3 行视口内随光标上下滚动。 |
| **Q2** | 后台任务面板「回到底部」按钮 | 同时覆盖 `Processes`（进程日志）和 `Agents`（子 Agent 消息流）两个 Tab；当当前 Tab 的上方内容区未处于底部跟随态时，在下方任务列表顶边横线上方一行（复用 `chat_bar` 右对齐位置与样式）显示 `↓ jump to bottom`，上方内容区自动让出 1 行；点击后跳回最底部并恢复自动跟随。 |
| **Q3** | Ask 面板 Preview 下移与选项描述换行 | 彻底取消左右分栏，选项区占满全宽；每个选项的 `description` 缩进 6 列后按全宽软折行完整展示；若当前题目没有任何选项带 `preview`，完全隐藏预览区（不显示 `(no preview)`）；若题目中有选项带 `preview`，预览区置于列表所有项正下方。 |
| **Q4** | Ask 提交按钮样式与单选点击行为 | 单选题（单题模式或非末题）移除提交按钮，按 `Enter` 或鼠标单击选项立即选中并完成本题（单题直接提交，多题进入下一题）；提交按钮采用弹窗实心胶囊按钮样式 ` ↵ submit `（可提交时 `style::button_primary()` 青底黑字加粗，未就绪时 `style::button_secondary()` 暗灰底白字加粗）。 |
| **Q5** | Ask 自定义输入（`Other...`）原地输入框 | 移除底部独立编辑框 `lay.editor`；激活 `Other...` 时直接将该条目原地变为行内输入框（保留左侧 6 列 `❯ (•) ` / `❯ [x] ` 前缀，右侧文本软折行、续行缩进 6 列，最多展开 1~3 行高并在内部随光标上下滚动）；单选编辑态按 `Enter` 直接提交文本并完成本题，多选编辑态按 `Enter` 确认文本并勾选 `[x]` 退出编辑态。 |
| **Q6** | Ask 键盘导航收敛、移除汇总 Tab 与多题提交 | 面板只保留 `↑`/`↓` 方向键切换 item（移除 `Tab`/`Shift+Tab`/`←`/`→`/`j`/`k`/`1..9` 快捷键，顶部题目 Tab 靠鼠标点击切换或单选答完自动推进）；移除多题的 `Confirm` 汇总 Tab；多题时前 `N-1` 题无提交按钮，最后一题底部固定显示 ` ↵ submit ` 按钮；提交按钮作为最后一个 item 参与 `↑`/`↓` 聚焦，在多选或最后一题中只有焦点在提交按钮上按 `Enter`（或鼠标点击按钮）才提交。 |
| **Q7** | Ask 期间暂停 Agent 计时与 Loading | `lx.ask.is_some()` 期间，`chat_bar` 隐藏 loading 文案与统计（仅在历史未触底时保留右侧 `↓ jump to bottom`）；历史区步骤标记停止呼吸闪烁（保持实心 `■`）；记录 `ask` 开始时刻并在 `ask` 结束/取消时将等待时长补偿进 `turn_started_at_ms`，使流中计时与最终 `duration_ms` / `tok/s` 完全剔除用户作答等待时间。 |

---

## 3. 架构原则与不变量

1. **`app/` 与 `ui/` 边界严格隔离**：
   - `app/` 只维护纯内存状态与状态转换函数，绝不依赖 `ratatui` 渲染组件或执行 IO；
   - `ui/` 只接收 `&AppState` 只读绘制，命中测试函数与渲染函数共享同一套纯几何布局计算函数，绝不在渲染中修改状态。
2. **文案与样式统一收口**：
   - 所有新增或修改的用户可见英文文案集中在 `src/ui/text.rs`（及子模块 `text/agent.rs`），禁止在渲染或状态逻辑中硬编码字符串；
   - 按钮样式统一通过 `src/ui/style.rs` 的 `button_primary()`、`button_secondary()`、`chat_jump_bottom()` 等语义函数获取。
3. **单文件行数上限控制**：
   - 生产代码单文件严格控制在 1000 行以内（当前 `src/ui/right_panel/todo.rs` 约 643 行、`src/ui/main_content/lx/ask.rs` 约 936 行，重构 `ask.rs` 删除底部独立编辑器与分栏逻辑并按职责抽离辅助函数或子模块，确保远低于 1000 行上限）；
   - 测试文件严格跟随对应被测子模块组织。

---

## 4. 详细技术设计与关键代码

### 4.1 特性一：右侧 Todo 面板条目最多 3 行折行缩进与行内多行编辑

#### 4.1.1 视觉布局与缩进规格
```text
┌─ Prompt ─ Todo ──────────────────────────────┐
│ [+]                                   [sort] │
├─ 2026/11/01 ───────────────────────── 26-01 ─┤
│ ‹  Mon   Tue   Wed   Thu   Fri   Sat  [Sun •]›│
│ ❯ [ ] P0 今天下午三点前必须把登录接口的超时   │ ← 第 1 行：9 列前缀 + 正文首行
│          重试和日志埋点全部改完并提测上线，   │ ← 第 2 行：左侧 9 列空白缩进 + 正文第 2 行
│          同时同步更新接口文档与测试用例…      │ ← 第 3 行：左侧 9 列空白缩进 + 末行超出截断 `…`
│   [~] P1 补充单元测试用例                     │ ← 单行短条目：仍占 1 行，紧凑排列
└──────────────────────────────────────────────┘
```
- **前缀宽度常量**：
  - `MARKER_WIDTH (1) + 1 + STATUS_WIDTH (3) + 1 + PRIORITY_WIDTH (2) + 1 = 9` 列（即 `row_text_x(list) - list.x == 9`）。
  - 正文可用列宽：`text_width = list.right().saturating_sub(row_text_x(list))`。
- **最大显示行数常量**：
  - `pub const MAX_TODO_ITEM_ROWS: usize = 3;`

#### 4.1.2 非编辑态（展示态）软折行与 3 行省略算法
在 `src/ui/widgets/input.rs`（或 `src/ui/right_panel/todo.rs`）提供按显示列宽（`UnicodeWidthChar`）切分的纯函数 `wrap_todo_lines(text: &str, width: usize, max_rows: usize) -> Vec<String>`：
- 支持文本中包含换行符 `\n` 或单行超宽自动软折行；
- 当折行结果不超过 `max_rows`（3 行）时，原样返回 `1..=3` 行；
- 当折行结果超过 `max_rows`（3 行）时，保留前 3 行，并将第 3 行末尾裁剪留出 1 列追加省略号 `…`（`U+2026`），确保总显示宽度不超过 `width` 且不切出半个宽字符。

```rust
/// Todo 单个条目最多占用的视觉行数。
pub const MAX_TODO_ITEM_ROWS: usize = 3;

/// 按显示列宽折行并限制最多 `max_rows` 行；超出时在末行尾部用 `…` 截断。
pub fn wrap_clamped_lines(text: &str, width: usize, max_rows: usize) -> Vec<String> {
    let width = width.max(1);
    let max_rows = max_rows.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut truncated = false;

    'outer: for raw_line in text.split('\n') {
        let mut current = String::new();
        let mut col = 0usize;
        for ch in raw_line.chars() {
            let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if col + ch_w > width && col > 0 {
                lines.push(std::mem::take(&mut current));
                col = 0;
                if lines.len() == max_rows {
                    truncated = true;
                    break 'outer;
                }
            }
            current.push(ch);
            col += ch_w;
        }
        lines.push(current);
        if lines.len() == max_rows {
            // 若后续还有字符或还有下一逻辑行，标记截断
            break;
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    if truncated || text_exceeds_lines(text, width, max_rows) {
        if let Some(last) = lines.last_mut() {
            *last = truncate_with_ellipsis(last, width);
        }
    }
    lines
}
```

#### 4.1.3 行内编辑态（1~3 行自适应展开 + 视口内跟随光标滚动）
- **输入模型**：
  - 将 `TextInput`（或行内多行视口助手 `widgets::input::multiline_view`）扩展支持按 `width` 软折行的视觉行映射：
    ```rust
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct WrappedLine {
        pub start_byte: usize,
        pub end_byte: usize,
        pub start_char: usize,
        pub end_char: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MultilineInputView {
        /// 当前视口可见的 1..=3 行切片
        pub visible_lines: Vec<WrappedLine>,
        /// 光标相对于可见首行的行偏移（0..visible_lines.len()）
        pub cursor_row_offset: u16,
        /// 光标在当前行内的显示列偏移
        pub cursor_col: u16,
    }
    ```
  - 当编辑文本折行后总行数 `total_lines <= 3` 时，该编辑条目占用 `total_lines.max(1)` 行（1~3 行）；
  - 当 `total_lines > 3` 时，该编辑条目固定占用 `3` 行，视口起点 `start_row` 跟随光标所在视觉行 `cursor_row`：
    ```rust
    let visible_count = total_lines.clamp(1, MAX_TODO_ITEM_ROWS);
    let start_row = if cursor_row >= visible_count {
        cursor_row + 1 - visible_count
    } else {
        0
    };
    ```
  - 编辑态下按 `Shift+Enter` / `Ctrl+Enter`（若用户需要手动换行）或打字自动软折行均可换行；按 `Enter` 提交保存（提交前将换行符归一或保留文本）、按 `Esc` 取消编辑。

#### 4.1.4 列表布局、命中测试与滚动对齐
因为每个 Todo 条目占用的高度是 `1..=3` 行（动态视觉行高，取决于文本长度与当前列表宽度）：
- 在 `src/ui/right_panel/todo.rs` 中构建统一的纯布局函数 `layout_visible_items(panel: Rect, state: &AppState) -> TodoListLayout`：
  - 每个逻辑条目 `display_idx`（含新建幽灵行）计算出其折行后的行数 `height: usize`（`1..=3`）；
  - 选中态（`row_selected`）或悬停态（`hovered`）时，将该条目在 `list` 视口内可见的所有视觉行 `Rect::new(list.x, row_y, list.width, visible_h)` **整块涂上 `style::selection()`**；
  - 第 1 个视觉行绘制 `❯`、状态框 `[ ]`、优先级徽标 `P0` 与第 1 行文本；第 2、3 个视觉行左侧 9 列留空（保留选中背景色），从 `row_text_x(list)` 起绘制第 2、3 行文本；
  - `row_at(panel, state, column, row)`：通过同一份 `layout_visible_items` 查表，点击某条目的第 1、2、3 行均命中同一个 `display_idx`；
  - `priority_at(panel, state, column, row)`：仅当点击落在该条目的**首行**（`offset_in_item == 0`）且 X 坐标位于 `[row_priority_x, row_priority_x + PRIORITY_WIDTH)` 时才命中优先级徽标；
  - `edit_input_char_at(panel, state, column, row)`：命中正在编辑的条目的任意可见行 `visual_row_offset`，结合 `column - text_x` 换算回全文字符索引，完整支持多行点击定位光标与鼠标拖拽选区。

---

### 4.2 特性二：后台任务面板（Processes · Agents）内容区「回到底部」按钮

#### 4.2.1 布局与状态设计
后台任务面板打开时（`lx.task.is_some()`），界面分为上下两段：
- 上半区 `out.task_output`：展示 `Processes` 控制台输出或 `Agents` 子 Agent 消息流；
- 下半区 `out.task_list`：展示 `Processes · Agents` 切换栏与最多 6 行任务列表；
- **新增「回到底部」栏位（复用 `out.chat_bar`）**：
  - 当上半区内容未处于底部跟随状态时，在 `out.task_list` 顶边横线正上方 1 行（`input.y - 1`）激活 `out.chat_bar`，绘制右对齐的 `↓ jump to bottom`（窄屏退化为 `↓`）；
  - 此时 `out.task_output` 的底边自动上移至 `out.chat_bar.y`（即 `let bottom = out.chat_bar.map_or(input.y, |bar| bar.y);`——注意 `lx.rs:571` 原本就已经写了这行让位逻辑！只需让 `chat_bar::visible(lx)` 与 `chat_bar::jump_visible(lx)` 在后台任务面板未触底时返回 `true` 即可无缝衔接）。

```text
╭─ lx-[Native Agent] ─────────────────────── ws:…/lx-tui ─╮
│ [Processes 日志输出 或 Agents 子 Agent 详情消息流]       │ ← out.task_output（高度自动让出 1 行）
│ ...                                                     │
│                                        ↓ jump to bottom │ ← out.chat_bar（未处于底部时浮现）
│ ─────────────────────────────────────────────────────── │ ← out.task_list 顶部分隔线
│ Processes · Agents                          [close esc] │
│ ❯ ● explore  running  14s  Explore todo panel           │
╰─ ↑↓ select  ^d stop  tabs ←→  esc ──────────────────────╯
```

#### 4.2.2 触底状态判定与关键代码
1. **`TaskPage` 状态扩展（`src/app/state/lx/task.rs`）**：
   - `Processes` 页签：当前已有 `output_scroll: usize` 与 `output_visible: usize`，最大滚动偏移为 `output_max_scroll() = output.len().saturating_sub(output_visible)`。当 `output.len() > output_visible && output_scroll < output_max_scroll()` 时，表示用户向上翻阅了进程日志、未处于底部。
   - `Agents` 页签：当前高亮的子 Agent `SubagentRun` 已持有 `pub follow: bool` 与 `pub scroll: usize`（当用户向上滚轮/PageUp 时 `run.follow` 置为 `false`，滚到底部时置为 `true`）。当当前高亮 `run` 存在、消息非空且 `!run.follow` 时，表示子 Agent 详情未处于底部。
   - 在 `TaskPage` 中编写只读判定方法 `pub fn is_at_bottom(&self, agents: &AgentsState) -> bool`：
     ```rust
     impl TaskPage {
         /// 当前激活页签的上方内容区是否处于底部。
         pub fn is_at_bottom(&self, agents: &AgentsState) -> bool {
             match self.tab {
                 TaskTab::Processes => {
                     self.output_visible == 0 || self.output_scroll >= self.output_max_scroll()
                 }
                 TaskTab::Agents => agents
                     .runs
                     .get(self.agents_window.active)
                     .is_none_or(|run| run.chat.messages.is_empty() || run.follow),
             }
         }
     }
     ```
2. **`chat_bar.rs` 显示与绘制逻辑更新（`src/ui/main_content/lx/chat_bar.rs`）**：
   - 更新 `jump_visible(lx)` 与 `visible(lx)`：
     ```rust
     pub(super) fn visible(lx: &Lx) -> bool {
         if lx.task.is_some() {
             return jump_visible(lx);
         }
         if !super::history::has_history(lx) {
             return false;
         }
         loading_visible(lx) || !lx.chat.follow_bottom
     }

     fn jump_visible(lx: &Lx) -> bool {
         if let Some(page) = lx.task.as_ref() {
             return !page.is_at_bottom(&lx.agents);
         }
         super::history::has_history(lx) && !lx.chat.follow_bottom
     }
     ```
3. **点击与悬停事件接线（`src/ui/main_content/lx/history_hit.rs`、`src/app/update/chat/viewport.rs`、`src/event/mouse/`）**：
   - 悬停：`hover.rs` 中即使 `lx.task.is_some()`，仍检测 `lx::jump_bottom_hit(pane_rect, &pane.lx, column, row)` 并调用 `update::hover_lx_jump_bottom(state, pane, hovered)` 更新 `lx.chat.jump_bottom_hover`；
   - 点击：在 `press.rs`（或 `task::hit` / `jump_lx_history_bottom`）中，当点击命中 `jump_bottom_hit` 时，调用 `update::jump_lx_history_bottom(state, pane)`：
     ```rust
     pub fn jump_lx_history_bottom(state: &mut AppState, pane: PaneId) -> bool {
         let Some(target) = state.pane_mut_anywhere(pane) else {
             return false;
         };
         if let Some(page) = target.lx.task.as_mut() {
             target.lx.chat.jump_bottom_hover = false;
             return match page.tab {
                 TaskTab::Processes => {
                     let before = page.output_scroll;
                     page.pin_output_to_tail();
                     page.output_scroll != before
                 }
                 TaskTab::Agents => target
                     .lx
                     .agents
                     .runs
                     .get_mut(page.agents_window.active)
                     .is_some_and(|run| {
                         let changed = !run.follow;
                         run.follow = true;
                         changed
                     }),
             };
         }
         if target.lx.chat.follow_bottom {
             return false;
         }
         target.lx.chat.follow_bottom = true;
         target.lx.chat.jump_bottom_hover = false;
         true
     }
     ```
   - 同时在 `Processes` 页签支持鼠标滚轮滚动上方日志输出区（向上滚脱离底部展示 `↓ jump to bottom`，向下滚到底部隐藏按钮），与 `Agents` 页签滚轮体验完全一致。

---

### 4.3 特性三：Ask 工具交互重构与暂停计时/Loading

#### 4.3.1 整体垂直布局（取消左右分栏、完整折行描述、Preview 置底、行内 Other 输入、胶囊提交按钮）

```text
────────────────────────────────────────────────────────────────────  ← 面板顶部分隔线 (1 行)
 1 布局   2 数据库                                                    ← 多题时的题目 Tabs (仅多题显示，移除 Confirm 汇总 Tab)
请选择你期望的界面布局方案：                                          ← 题干 (1~3 行软折行)
❯ (•) 选项 A                                                          ← 选项标题行 (6 列前缀 `❯ (•) `)
      推荐项，测试默认首选项的渲染。如果描述文字非常长，会自动在      ← 选项描述行 (缩进 6 列，按全宽自动换行完整显示)
      这里折行并保持左侧 6 列缩进对齐，绝不截断。
  ( ) 选项 B
      测试非首选项的回传。
  ( ) 选项 C
      测试第三项选择与长文本完整换行展示。
  ( ) Other...                                                        ← 未编辑时显示 Other...；激活后原地变成 1~3 行软折行输入框
      ┌─ 激活 Other 时的原地行内编辑效果：─────────────────────────┐
      │ ❯ (•) 我希望采用上下分栏，并且支持快捷键折叠第一栏，       │
      │       同时记住上次关闭时的高度比例|                        │
      └────────────────────────────────────────────────────────────┘
       ↵ submit                                                       ← 实心胶囊提交按钮（仅在「单题多选」或「多题的最后一题」显示）

  ┌────────────────────────────────────────────────────────────────┐  ← Preview 预览区（仅当本题任一选项含 preview 时显示在所有选项下方）
  │ +-------------------+                                          │
  │ |     Header        |                                          │
  │ +-------------------+                                          │
  └────────────────────────────────────────────────────────────────┘
↑↓ select · enter submit                              esc cancel      ← 底栏按键提示 (1 行)
```

#### 4.3.2 关键变更点 1：选项描述完整换行与 Preview 移至下方
1. **取消左右分栏**：
   - 在 `src/ui/main_content/lx/ask.rs` 中删除 `SPLIT_MIN_WIDTH` 与 `OPTIONS_MIN_COLS` 的左右分栏分支；`lay.options` 的宽度恒为 `inner.width`。
2. **选项描述（`OptionDesc`）完整多行折行**：
   - 描述文本可用宽度为 `desc_width = inner.width.saturating_sub(ROW_PREFIX).max(1)`（其中 `ROW_PREFIX = 6`，即 `❯ ( ) ` 的宽度）；
   - 在 `rows_model(ask, width)` 与 `draw_rows` 中，对每个非空的 `option.description` 调用 `wrap(&option.description, usize::from(desc_width))`；
   - 将 `RowKind::OptionDesc { index: usize, line: usize }` 展开为对应数量的视觉行（每行高度 1，缩进 `ROW_PREFIX` 列绘制），使描述内容无论多长都能完整换行显示，且每一行点击都命中 `AskHit::Option(index)`。
3. **Preview 置于所有选项下方且按需显隐**：
   - 在 `AskState` 中增加辅助方法 `pub fn question_has_preview(&self) -> bool`：
     ```rust
     /// 当前题目是否有任何选项配置了非空 preview。
     pub fn question_has_preview(&self) -> bool {
         self.question().is_some_and(|q| {
             q.options
                 .iter()
                 .any(|opt| opt.preview.as_deref().is_some_and(|p| !p.trim().is_empty()))
         })
     }
     ```
   - 当 `!ask.question_has_preview()` 时：`lay.preview = None`，不预留任何预览高度，也不渲染 `(no preview)`；
   - 当 `ask.question_has_preview()` 为 `true` 时：在 `lay.options`（包含所有选项、`Other` 及可能存在的提交按钮）的**正下方**留出 1 行间距（`PREVIEW_GAP = 1`），分配 `lay.preview`（高度最多 8 行，或按剩余空间分配）；若当前高亮项有 `preview` 则渲染其内容，若当前高亮项无 `preview`（例如高亮移到了 `Other` 或提交按钮）则在 `lay.preview` 中渲染 `(no preview)`，保证切换高亮时面板高度恒定不抖动。

#### 4.3.3 关键变更点 2：移除汇总 Tab、收敛提交按钮与键盘导航
1. **移除多题 `Confirm` 汇总 Tab**：
   - 删除 `AskState.confirm` 字段及 `RowKind::Review` / `RowKind::Confirm`；
   - `AskState::tab_len(&self) -> usize` 直接返回 `self.questions.len()`（仅当 `questions.len() > 1` 时在顶部渲染 `1 Header`、`2 Header` ... 题目 Tab）。
2. **提交按钮的显隐条件（`has_submit_row`）**：
   - 单题（`questions.len() == 1`）：仅在多选（`question.multi_select == true`）时显示提交按钮；单选不显示提交按钮。
   - 多题（`questions.len() > 1`）：前 `N-1` 题（`current + 1 < questions.len()`）不显示提交按钮；**最后一题（`current + 1 == questions.len()`）固定显示提交按钮**。
   ```rust
   impl AskState {
       /// 当前题目底部是否显示「提交」按钮行。
       pub fn has_submit_row(&self) -> bool {
           let Some(question) = self.question() else {
               return false;
           };
           if self.questions.len() > 1 {
               self.current + 1 == self.questions.len()
           } else {
               question.multi_select
           }
       }

       /// 当前题可被 ↑/↓ 高亮的条目总数：options.len() + 1 (Other) + usize::from(has_submit_row())。
       pub fn row_count(&self) -> usize {
           self.question().map_or(0, |question| {
               question.options.len() + 1 + usize::from(self.has_submit_row())
           })
       }

       /// 「其他」行的逻辑下标。
       pub fn other_row(&self) -> usize {
           self.question().map_or(0, |question| question.options.len())
       }

       /// 提交按钮行的逻辑下标（仅当 `has_submit_row()` 为真时存在）。
       pub fn submit_row(&self) -> Option<usize> {
           self.has_submit_row()
               .then(|| self.other_row().saturating_add(1))
       }
   }
   ```
3. **提交按钮的弹窗胶囊样式渲染**：
   - 文案常量：`pub const ASK_SUBMIT_BUTTON: &str = " ↵ submit ";`（复用 `ui::text::key::ENTER` 符号 `↵`，与模态弹窗按钮 ` ↵ confirm `、` ↵ save ` 风格完全一致）。
   - 样式选择：
     - 当满足提交条件（`ask.all_answered()` 为 `true`）时，按钮胶囊采用 `style::button_primary()`（实心青底 `Color::Cyan`、黑字 `Color::Black`、加粗 `Modifier::BOLD`）；
     - 当尚未满足提交条件时，按钮胶囊采用 `style::button_secondary()`（实心暗灰底 `Color::Indexed(238)`、白字 `Color::Indexed(255)`、加粗 `Modifier::BOLD`）；
     - 当 `↑`/`↓` 焦点落在该提交按钮行（`highlight == submit_row`）时，行首第 0 列同样绘制 `❯`（`style::accent()`），第 `ROW_PREFIX`（6）列起绘制实心胶囊 ` ↵ submit `。
4. **键盘导航与回车/点击提交规则**：
   - **列表态（`!ask.editing`）按键收敛**：
     - 仅保留 `LxInput::Up`（`move_highlight(-1)`）与 `LxInput::Down`（`move_highlight(1)`）切换聚焦 item；
     - 移除 `j`/`k`、`Tab`/`Shift+Tab`、`Left`/`Right`、`1..=9` 数字键切项或切题；
     - 保留 `Space`（在普通选项上勾选/选中，在 `Other` 上进入行内编辑态）、`Enter`、`PageUp`/`PageDown`（滚预览）、`Escape`（双击取消）。
   - **列表态 `Enter` 规则**：
     - 若高亮在 `Submit` 按钮行（`Some(highlight) == ask.submit_row()`）：
       - 检查 `ask.all_answered()`：若全部题目均已作答，立即调用 `submit_all(state, pane)` 提交全部答案；
       - 若存在未作答的题目，弹出窗格内 Warning Toast `"select an answer first"`，并将 `ask.current` 自动切换到第一个未作答的题目 Tab，方便用户补答。
     - 若高亮在 `Other` 行（`highlight == ask.other_row()`）：
       - 调用 `ask.begin_edit()` 原地进入 `Other` 行内编辑态。
     - 若高亮在普通选项行（`highlight < question.options.len()`）：
       - **多选题（`question.multi_select == true`）**：按 `Enter` 只切换该选项的勾选状态（`ask.choose(highlight)`），**绝不直接提交**（必须移到提交按钮按 `Enter` 或点提交按钮才提交）；
       - **单选题（`question.multi_select == false`）**：
         - 选中当前高亮项 `ask.choose(highlight)`；
         - 若当前是单题模式（`questions.len() == 1`）：立即调用 `submit_all(state, pane)` 提交完成；
         - 若当前是多题模式的非最后一题（`current + 1 < questions.len()`）：自动推进到下一题 `ask.next_step()`；
         - 若当前是多题模式的最后一题（底部有 ` ↵ submit ` 按钮）：选中该项并将焦点自动移到底部 ` ↵ submit ` 按钮（或由用户按 `↓` 移到按钮再按 `Enter` 提交，严格遵守「聚焦在提交按钮时按 Enter 才能提交」）。
   - **鼠标点击规则**：
     - 点击顶部题目 Tab：若当前在编辑态先 `commit_edit()`，然后切换到目标题目 `goto_tab(index)`；
     - 点击普通选项：若在单选且无提交按钮的题（单题单选，或多题非末题单选），选中该项并立即提交/进入下一题；若在多选题或多题末题，选中/切换勾选该项（需点击 ` ↵ submit ` 按钮提交）；
     - 点击 ` ↵ submit ` 按钮：若 `all_answered()` 则 `submit_all`，否则弹 Toast 并跳到首个未作答题。

#### 4.3.4 关键变更点 3：自定义输入（`Other...`）原地变为 1~3 行输入框
1. **移除底部独立编辑框 `lay.editor`**：
   - 删除 `AskLayout.editor`、`AskLayout.editor_visible` 及 `draw_editor` 函数；
   - `Other` 行（`RowKind::Other { line: usize }`）直接在选项列表中原地渲染与编辑。
2. **`Other` 行的原地高度与折行（参考 Todo item 与 `opencode-dev`）**：
   - `Other` 行左侧保留 6 列前缀（`ROW_PREFIX = 6`）：第 0 列为高亮箭头 `❯`，第 2..5 列为单选 `(•)`/`( )` 或多选 `[x]`/`[ ]`；
   - 正文从 `rect.x + ROW_PREFIX` 开始，可用宽度 `editor_w = lay.options.width.saturating_sub(ROW_PREFIX).max(1)`；
   - **处于编辑态（`ask.editing == true`）或已有手写文本时**：
     - 复用 `AskEditor` 的软折行能力（将 `AskEditor` 宽度同步为 `editor_w`，最大可视高度设为 `MAX_OTHER_EDIT_ROWS = 3`）；
     - `Other` 条目在列表中动态占用 `editor.visual_rows().len().clamp(1, 3)` 行（1~3 行）；
     - 当输入文本为空时，在第 1 行 `rect.x + ROW_PREFIX` 处绘制暗色占位符 `Other...`（`style::muted()`），硬件光标停在 `(rect.x + ROW_PREFIX, row_y)`；
     - 当输入文本非空时，绘制视口内的 1~3 行文本（续行同样从 `rect.x + ROW_PREFIX` 起绘制，天然实现 6 列缩进对齐），支持选区反显（`Modifier::REVERSED`），并返回终端硬件光标坐标 `Some((cursor_x, cursor_y))`；
   - **非编辑态且无手写文本时**：
     - 占用 1 行，显示 `  ( ) Other...` 或 `  [ ] Other...`。
   - **行内编辑态按键交互**：
     - 普通字符、`Backspace`、`Delete`、`Left`/`Right`、`Home`/`End`、`Ctrl+A/E/U/K/W/Z/Y`：在 `AskEditor` 内编辑；
     - `Shift+Enter` / `Ctrl+Enter`（`LxInput::Newline`）：在行内输入框中插入换行；
     - `Enter`（`LxInput::Enter`）：调用 `ask.commit_edit()` 提交手写文本：
       - 若文本非空且当前为单题单选，直接 `submit_all` 提交；
       - 若文本非空且当前为多题非末题单选，自动进入下一题；
       - 若为多选或多题末题，退出编辑态并勾选 `[x]` / `(•)`（多题末题可将焦点移至 ` ↵ submit ` 按钮）；
     - `Esc`（`LxInput::Escape`）：退出行内编辑态（`ask.cancel_edit()`），不直接取消整轮 `ask`。
   - **鼠标定位光标与拖拽选区**：
     - `ask_editor_cell_at` 直接根据 `lay.rows` 中 `RowKind::Other { line }` 的矩形坐标换算 `(visual_row, content_col)`，使鼠标点击与拖拽选区在行内 `Other` 输入框上无缝工作。

#### 4.3.5 关键变更点 4：Ask 过程中暂停 Agent 计时与 Loading
1. **隐藏 `chat_bar` 的 loading 动画与计时统计（`src/ui/main_content/lx/chat_bar.rs`）**：
   - 在 `chat_bar::loading_visible(lx: &Lx) -> bool` 开头加入 `lx.ask.is_some()` 门控：
     ```rust
     fn loading_visible(lx: &Lx) -> bool {
         // ask 提问等待用户作答期间，不显示 Running tools... 与计时统计
         if lx.ask.is_some() {
             return false;
         }
         if lx.compact.is_running() {
             return true;
         }
         if lx.chat.main_stopped {
             return false;
         }
         lx.chat.status.streaming || lx.chat.request.is_busy() || has_running_agents(lx)
     }
     ```
2. **停止历史区步骤标记呼吸闪烁（`src/ui/main_content/lx/history.rs`）**：
   - 在 `HistorySource::from_lx`（或 `history.rs` 计算 `animating` 处）：当 `lx.ask.is_some()` 时，`animating = false`，使历史列表中的 `■ Ask · waiting for you` 方块保持静态实心 `■`，不再一闪一闪。
3. **暂停推进计时并在 Ask 结束时补偿 `turn_started_at_ms`**：
   - 在 `AskState`（`src/app/state/lx/ask.rs`）中记录提问挂起开始毫秒时间戳：
     ```rust
     pub struct AskState {
         // ...
         /// 本次提问挂起开始的毫秒时间戳（用于结束时从本轮耗时中扣除用户思考时间）。
         pub started_at_ms: i64,
     }
     ```
   - 在 `advance_lx_loading`（`src/app/update/chat/mod.rs`）中：
     - 当 `target.lx.ask.is_some()` 时，**不更新 `target.lx.chat.turn_elapsed_ms`，也不递增主会话的 `target.lx.chat.loading_frame`**（仅当后台有独立 `agents_running` 时推进子 agent 的 `loading_frame`）；
   - 当用户提交答案（`submit_all` in `src/app/update/chat/ask.rs`）或取消提问（`event/chat/ask.rs::clear`）移除 `pane.lx.ask` 时，计算本次 `ask` 挂起持续的毫秒数 `paused_ms = now_ms().saturating_sub(ask.started_at_ms).max(0)`，并补偿进 `turn_started_at_ms`：
     ```rust
     if let Some(started) = pane.lx.chat.turn_started_at_ms.as_mut() {
         let paused_ms = crate::db::now_ms().saturating_sub(ask.started_at_ms).max(0);
         *started = started.saturating_add(paused_ms);
     }
     ```
   - **效果**：
     - `ask` 期间计时器完全冻结且不显示；
     - 用户作答完毕后，Agent 继续推理时 `now_ms() - turn_started_at_ms` 精确衔接提问前的已耗秒数；
     - 轮次结束时的 `message.duration_ms` 与 `tok/s` 统计完全不包含用户思考作答的时间。

---

## 5. 影响范围与文件清单

| 模块 | 文件路径 | 修改内容概要 |
| :--- | :--- | :--- |
| **Todo 渲染与输入** | `src/ui/widgets/input.rs` | 新增多行软折行与 3 行截断/视口跟随辅助函数（`wrap_clamped_lines`、`multiline_view`、`multiline_char_index_at`） |
| **Todo 面板 UI** | `src/ui/right_panel/todo.rs` | 条目渲染改为 1~3 行动态高度、续行 9 列缩进、3 行末 `…` 截断；行内编辑支持 1~3 行软折行与光标同步；更新 `row_at`/`priority_at`/`edit_input_char_at`/`scrollbar` |
| **Todo 状态与更新** | `src/app/state/todo.rs`<br>`src/app/update/todo.rs` | 适配多行编辑视口与滚动可见性计算 |
| **后台任务面板状态** | `src/app/state/lx/task.rs` | 新增 `TaskPage::is_at_bottom(&self, agents)` 判定 |
| **聊天栏与回到底部** | `src/ui/main_content/lx/chat_bar.rs`<br>`src/ui/main_content/lx/history_hit.rs`<br>`src/app/update/chat/viewport.rs`<br>`src/event/mouse/` | 支持后台任务面板打开且未触底时显示 `↓ jump to bottom`，点击跳回 `Processes` 日志尾部或 `Agents` 消息流底部；支持 `Processes` 输出区滚轮滚动 |
| **Ask 状态机** | `src/app/state/lx/ask.rs` | 移除 `confirm` 步；新增 `has_submit_row()`、`submit_row()`、`question_has_preview()`、`started_at_ms`；更新 `row_count()` |
| **Ask 面板 UI** | `src/ui/main_content/lx/ask.rs`<br>`src/ui/text/agent.rs` | 移除左右分栏与底部独立 `lay.editor`；选项描述 6 列缩进完整换行；`Other` 原地 1~3 行行内编辑；` ↵ submit ` 实心胶囊按钮；`preview` 移至所有选项正下方且无预览时完全隐藏 |
| **Ask 交互与计时暂停** | `src/app/update/chat/ask.rs`<br>`src/app/update/chat/mod.rs`<br>`src/event/chat/ask.rs`<br>`src/ui/main_content/lx/history.rs` | 仅保留 `↑`/`↓` 导航；聚焦提交按钮按 `Enter` 提交；单选直接选中提交/下一题；`ask` 期间隐藏 loading、停止方块呼吸、暂停并补偿 `turn_started_at_ms` |
| **开发规范同步** | `docs/standards/tui-design-requirements.md` | 同步更新 Todo 折行、后台面板回到底部与 Ask 交互规范描述 |

---

## 6. 验证与测试计划

1. **Todo 面板单元测试（`src/ui/right_panel/todo/tests.rs`、`src/app/update/todo/tests.rs`）**：
   - 验证短文本占 1 行、中长文本折为 2~3 行且第 2、3 行精确缩进 9 列（与首行正文对齐）；
   - 验证超 3 行文本在第 3 行末尾以 `…` 截断，总高不超过 3 行；
   - 验证选中与悬停高亮底色完整覆盖该条目的全部 1~3 行，且优先级徽标不被污染；
   - 验证行内编辑在 1~3 行间自适应展开、超过 3 行时在 3 行视口内随光标滚动，以及多行下的鼠标点击定位字符与硬件光标位置。
2. **后台任务面板回到底部测试（`src/ui/main_content/lx/chat_bar/tests/`、`src/ui/main_content/lx/tests.rs`）**：
   - 验证 `Processes` 输出在底部时不显示 `↓ jump to bottom`，向上滚动后显示按钮且输出区高度让出 1 行，点击按钮后 `output_scroll` 回到 `output_max_scroll()`；
   - 验证 `Agents` 详情 `follow == true` 时不显示按钮，`follow == false` 时显示按钮，点击后恢复 `follow = true`。
3. **Ask 面板与暂停计时测试（`src/ui/main_content/lx/ask/tests.rs`、`src/app/update/chat/tests/ask.rs`、`src/ui/main_content/lx/chat_bar/tests/`）**：
   - 验证选项描述超长时按全宽缩进 6 列完整换行显示；
   - 验证无 `preview` 时不分配预览矩形、不渲染 `(no preview)`；有 `preview` 时预览矩形位于所有选项矩形下方 (`preview.y >= options.bottom()`)；
   - 验证单题单选无提交按钮、回车或单击选项直接提交；单题多选显示 ` ↵ submit ` 胶囊按钮且需聚焦到按钮按回车或点击按钮才提交；
   - 验证多题无 `Confirm` Tab、最后一题底部显示 ` ↵ submit ` 按钮，未答完前题时点击提交弹 Toast 并跳回未答题；
   - 验证 `Other` 原地变成 1~3 行输入框，不再出现底部独立 `lay.editor`；
   - 验证 `lx.ask.is_some()` 时 `chat_bar` 不显示 `Running tools...` 与耗时，且作答结束/取消后 `turn_started_at_ms` 正确补偿等待时间。
4. **静态检查与格式化**：
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - 针对受影响模块执行 `cargo test` 全绿通过。
