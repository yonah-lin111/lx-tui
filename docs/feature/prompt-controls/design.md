# Prompt 编辑器工具栏与状态信息 — 设计

- 分支：`feature/prompt-controls`（已合并进 `dev`，合并提交 `59b0c72`）
- 状态：**已完成**；实现期间的用户迭代结论（顶边框 `ws:`、底边框 `b:`/`wt:`、淡蓝色边框标题、分割线跟随焦点）已并入本文档

## 1. 背景与目标

当前 `lx-tui` 的 prompt 右栏为纯文本 markdown 编辑器面板：
- 顶部仅有外层 Block 边框与标题 ` prompt `；
- 内部直接为可滚动的 prompt 文本编辑视口（带最右侧 1 列滚动条槽）与悬浮面板；
- 底部边框右侧为折叠按钮 `[▶]`，其余边框空白。

本任务目标：参考 `lx-agent` 中的 `LxMarkdownEditor.tsx` 与底部状态栏设计，在 prompt 区域添加基础控件和数据显示：
1. **顶部功能按钮行与分割线**：
   - 左侧：`[undo]`、`[redo]`（无可撤销/重做时置灰禁用）
   - 右侧：`[select all]`、纯色点保存指示（`●` 绿色已保存 / 黄色未保存）
   - 工具栏下一行添加固定分割线（`─` 水平贯穿，不随滚动条滚动），样式跟随边框焦点态
   - UI 按钮与状态文案全英文，保存状态仅为单字符彩色圆点（无冗余文字）
   - 边框标题：左侧 `Prompt`（首字母大写、淡蓝色），顶边框右侧 `ws:路径末段名`
2. **边框状态信息**：
   - 顶边框右侧：`ws:路径末段名`（前缀强调色、值 muted，右对齐，零额外行高）
   - 底边框左侧：`b:分支`（linked worktree 显示仓库主 checkout 分支；前缀强调色、值 muted）
   - linked worktree 在底边框追加 ` wt:工作区名`
   - 右端严格避让已有的折叠按钮 `[▶]`，小宽度下省略 `wt:` 片段并截断名称

---

## 2. 已确认决策（Grill Me 盘问结论）

| # | 决策项 | 结论 | 详细说明 |
|---|--------|------|----------|
| 1 | md 保存状态的数据源与行为语义 | **纯内存脏标记状态机** | `Prompt` 维护 `is_saved: bool`。初始/手动保存为 `true`，发生任何文本修改置 `false`；右侧仅渲染单字符彩色圆点 `●`（已保存绿点 / 未保存黄点），无多余文字 |
| 2 | 工具栏与分割线物理位置与视口影响 | **侵入 inner 顶部 2 行，视口与滚动条下移** | `inner.y` 为工具栏行（高度 1），`inner.y + 1` 为固定分割线行（高度 1），`inner.y + 2` 开始为文本编辑区；滚动条槽起点同为 `inner.y + 2`，不覆盖工具栏。文案全英文 |
| 3 | 工具栏按钮排列顺序与「全选」行为 | **左 `[undo] [redo]`，右 `[select all]` + 点，标准选区语义** | 左侧顺序遵循标准习惯；历史为空时置灰且不响应点击；点击 `[select all]` 在 `AppState` 中构造覆盖 Prompt 全文的 `Selection`（全文反显高亮），不越权直接写剪贴板 |
| 4 | 边框状态信息几何与降级 | **顶边框右侧 `ws:` + 底边框左侧 `b:`/`wt:`，自右向左降级** | 顶边框右侧右对齐 `ws:路径末段名`（优先 checkout 路径末段，回退工作区 cwd，无路径不显示）；底边框左侧 `y = area.bottom() - 1` 嵌入 `b:分支`，linked worktree 追加 ` wt:工作区名`，避让右端 `[▶]`；空间不足时省略 `wt:` 片段并截断名称 |

---

## 3. 架构与数据流模型

### 3.1 状态层（`src/app/prompt.rs` 与 `src/app/selection.rs`）

#### `Prompt` 状态扩展
```rust
pub struct Prompt {
    // ... 既有字段 ...
    /// 当前是否处于已保存状态；任何文本写操作置 false。
    is_saved: bool,
}
```
- `Prompt::new(id)`：`is_saved: true`；
- `Prompt::is_saved(&self) -> bool`：只读暴露；
- `Prompt::mark_saved(&mut self)`：置为 `true`；
- `Prompt::can_undo(&self) -> bool`：`!self.undo.is_empty()`；
- `Prompt::can_redo(&self) -> bool`：`!self.redo.is_empty()`；
- 在所有修改文本的操作（`insert_at`, `record`, `backspace`, `delete`, `newline_below`, `undo`, `redo`）执行后，统一置 `self.is_saved = false`。

#### `Selection` 全选扩展（`src/app/selection.rs`）
```rust
impl Selection {
    /// 构造覆盖指定面板全文的选区（起始 (0, 0)，结束 (max_row, max_col)）。
    pub fn full(pane: PaneId, max_row: i32, max_col: u16) -> Self {
        Self {
            pane,
            anchor: (0, 0),
            cursor: (max_row, max_col),
            dragging: false,
        }
    }
}
```
- `AppState::select_all_prompt(&mut self)`：根据当前 prompt 的 `visual_rows().len()` 和宽度生成 `Selection::full`，激活全文反显高亮。

---

### 3.2 布局层（`src/layout.rs`）

#### 常量与几何
```rust
/// prompt 工具栏高度：1。
pub const PROMPT_TOOLBAR_HEIGHT: u16 = 1;
/// prompt 工具栏下方分割线高度：1。
pub const PROMPT_DIVIDER_HEIGHT: u16 = 1;
/// prompt 顶部保留行数（工具栏 + 分割线）。
pub const PROMPT_HEADER_HEIGHT: u16 = PROMPT_TOOLBAR_HEIGHT + PROMPT_DIVIDER_HEIGHT;
```

#### 矩形调整
- `prompt_text_rect(panel: Rect) -> Rect`：
  - 当 `inner.height > PROMPT_HEADER_HEIGHT` 时：
    `y = inner.y + PROMPT_HEADER_HEIGHT`，`height = inner.height - PROMPT_HEADER_HEIGHT`；
  - 否则（极矮视口）：退化为使用全部 `inner`。
- `prompt_scrollbar_rect(panel: Rect) -> Option<Rect>`：
  - 滚动条槽起始行与高度与文本区完全一致（`y = inner.y + PROMPT_HEADER_HEIGHT`，`height = inner.height - PROMPT_HEADER_HEIGHT`），最右 1 列。
- 工具栏各按钮命中矩形：
  - 工具栏行 `bar_y = inner.y`；
  - `undo`：`Rect::new(inner.x, bar_y, 6, 1)`（`[undo]` 占 6 列）；
  - `redo`：`Rect::new(inner.x + 7, bar_y, 6, 1)`（`[redo]` 占 6 列，空 1 格）；
  - `save_dot`：`Rect::new(inner.right().saturating_sub(1), bar_y, 1, 1)`；
  - `select_all`：`Rect::new(inner.right().saturating_sub(14), bar_y, 12, 1)`（`[select all]` 占 12 列，后隔 1 格为点）；
  - 当 `inner.width < 28` 时触发右侧按钮降级（优先隐藏 `select all`，保留点）。

---

### 3.3 渲染层（`src/ui/`）

#### 1. 工具栏与分割线（`src/ui/prompt.rs`）
- 在 `render_prompt` 中绘制 Block：左侧标题 `Prompt`（`style::border_title()` 淡蓝色），顶边框右侧右对齐 `ws:路径末段名`（前缀 `style::accent()`、值 `style::muted()`）；
- 若 `inner.height > PROMPT_HEADER_HEIGHT`：
  - **工具栏行（`inner.y`）**：
    - 左侧：绘制 `[undo]`（`can_undo` 为真时 `style::accent()`，为假时 `style::muted()`）；空格；绘制 `[redo]`（`can_redo` 为真时 `style::accent()`，为假时 `style::muted()`）；
    - 右侧：宽度充足时绘制 `[select all]`（`style::accent()`）；空格；绘制 `●`（`is_saved` 为真时绿色 `style::status_dot(true)`，为假时黄色 `style::status_dot(false)`）。
  - **分割线行（`inner.y + 1`）**：
    - 整行填充 `text::DIVIDER_MID`（`─`），样式跟随边框焦点态（`style::border(focused)`：聚焦强调色、失焦 muted）；
    - 两端可将外层边框衔接为 `├` 和 `┤`（同样跟随焦点态）。

#### 2. 边框状态信息（`src/ui/mod.rs`）
- 顶边框右侧：`ws:路径末段名`——优先 `git.checkout_path` 末段，回退 `workspace.cwd`，无路径不显示；前缀 `style::accent()`、值 `style::muted()`；
- 底边框左侧（`row = area.bottom().saturating_sub(1)`）：
  - 右侧预留 `PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN = 4` 列给折叠按钮 `[▶]`，左起 `area.x + 2`；
  - `b:{status_branch}`：linked worktree 显示仓库主 checkout 分支（`WorkspaceGit::status_branch()`，缺失回退自身分支），主 checkout 显示自身分支；前缀 `style::accent()`、值 `style::muted()`；
  - linked worktree 追加 ` wt:{workspace.name}`；
  - 宽度不足时省略 `wt:` 片段并截断名称；非 git 不显示。

---

### 3.4 事件与交互（`src/event/mod.rs`）

#### 鼠标点击
- 命中测试优先检查工具栏各按钮矩形：
  - 命中 `[undo]`：若 `can_undo`，执行 `prompt.undo()` 并请求重绘；
  - 命中 `[redo]`：若 `can_redo`，执行 `prompt.redo()` 并请求重绘；
  - 命中 `[select all]`：调用 `state.select_all_prompt()` 并请求重绘；
  - 命中 `●`：调用 `state.prompt.mark_saved()`（允许点击手动重置为已保存）并请求重绘；
  - 命中工具栏或分割线其他空白区域：吞掉点击，**不聚焦文本视口、不移动文本光标**。
- 命中底边框左侧：纯展示，不拦截。

---

## 4. 边界处理与降级矩阵

| 场景 | 表现 |
|------|------|
| Prompt 折叠态（4 列窄条） | 不渲染工具栏与底边框状态，维持既有窄条与居中展开按钮 |
| Prompt 高度极小（`inner.height <= 2`） | 隐藏工具栏与分割线，全部高度留给文本编辑区，防止溢出或 panic |
| Prompt 宽度极小（`inner.width < 28`） | 工具栏右侧优先隐藏 `[select all]`，仅保留右缘 `●` 保存点；底边框状态自右向左省略 |
| 非 Git 工作区 | 顶边框仍有 `ws:路径末段名`，底边框不显示任何 git 状态 |
| 主 Checkout（非 linked worktree） | 底边框显示 `b:自身分支`，不追加 ` wt:` 片段 |
| Prompt 内容为空时点击 `[select all]` | 选区为空，无崩溃、无脏状态 |
