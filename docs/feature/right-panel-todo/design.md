# 右侧容器 Todo 面板 — 设计

> 状态：**已实施**（实施记录见 §9）。
>
> 任务目录：`docs/feature/right-panel-todo/`
>
> 任务范围：`src/db/{mod,migration,todo,service}.rs`、`src/app/{state,effects,actions}.rs`、
> `src/app/update/`（新增 `todo.rs`）、`src/input/mod.rs`、`src/event/{input,mouse/*}`、
> `src/ui/right_panel/`（新增 `todo.rs`）、`src/ui/prompt.rs`、`src/ui/text.rs`、
> `src/ui/style.rs`、`src/ui/tooltip.rs`、`docs/standards/tui-design-requirements.md`。
>
> 会改数据库（`SCHEMA_VERSION` 14 → 15）。不改配置项、不改终端 PTY 行为、不改 prompt 正文语义。
>
> 参考实现：`/Users/yonah/projects/agent/lx-agent`（Electron + TS）的日程待办
> （`ScheduleComposer` / `ScheduleItemRow` / `scheduleService`）。

---

## 1. 背景与目标

### 1.1 现状（代码事实，均经 codegraph/Read 核对）

**容器与占位**

- `RightTab { Prompt, Todo }` 定义在 `src/app/state.rs:74-81`，注释明确「纯视图偏好：
  不落库、不写配置，启动恒为 `Prompt`」。
- Todo 面板体只有 `render_empty_body`（`src/ui/right_panel.rs:112-117`）：只画
  `prompt::render_divider` 一条分割线，工具栏整行空白、内容区空白。
- `render` 的 Todo 分支返回 `None`（`src/ui/right_panel.rs:104-107`），因此**没有硬件光标**，
  IME 预输入不可见。
- 点击 Todo 整块被吞：`src/event/mouse/press.rs:269-273`
  （`!prompt_panel_active(state) && view.prompt.contains(...)`）。
- 滚轮在 Todo 下直接 `return`：`src/event/mouse/wheel.rs:263`。
- 拖拽光标跟随有 `prompt_panel_active` 闸门：`src/event/mouse/drag.rs:266-271`；
  `pane_content_inner` 在 Todo 下返回 `Rect::ZERO`：`src/event/mouse/hit.rs:418-433`。
- `prompt_scrollbar` 硬闸门 `right_tab != Prompt → None`：`src/ui/prompt_frame.rs:232-240`。
- 树行悬停闸门：`src/event/mouse/hover.rs:243-257`；tooltip 的 prompt 按钮组闸门：
  `src/ui/tooltip.rs:154`；树右键菜单闸门：`src/event/overlay.rs:456`。
- 既有回归用例锁定「吞掉」语义：`src/event/tests.rs:4980-5020`
  （`mouse_wheel_over_todo_does_not_scroll_prompt`）、`src/ui/tests.rs:1437-1441`
  （Todo 下不画滚动条）。

**键盘与粘贴通道**

- 唯一吃键条件是 `state.prompt_focused`（`src/input/mod.rs:55`）；`select_right_tab`
  在切走 Prompt 时把它置假（`src/app/update/right_panel.rs:18`），于是 Todo 下按键
  原样编码写进焦点终端（`src/input/mod.rs:89`）。
- 粘贴同理：`src/event/input.rs:151-155` 只在 `prompt_focused` 时进 prompt，否则写终端。
- 单行输入现成件：`TextInput`（`src/app/overlay.rs:538-650`，char 索引光标 +
  insert/backspace/delete/left/right/home/end/clear/insert_str）；
  键映射现成件：`input::page_rename_key`（`src/input/mod.rs:175-196`）。
- 页重命名输入拦截的现成写法：`src/event/input.rs:32-41`（Ctrl+Q 例外优先，其余整键吞掉）。

**持久化地基**

- `SCHEMA_VERSION = 14`（`src/db/mod.rs:25`）；迁移无数组，是 `migrate()` 里
  `if version < N` 顺序执行（`src/db/migration.rs:522-632`）。
- 「只增表」先例：v10 `prompt_history`（`migration.rs:288-299`）、v14（`:623-630`），
  写法为 `unchecked_transaction` + `execute_batch` + `ensure_no_foreign_key_violation`
  + `pragma_update(user_version)` + `commit`。
- 会话循环内的写入口是 `Effect` + `Persistence::pump`
  （`src/app/effects.rs:18+`、`src/db/service.rs:444-474`、单条执行 `execute` `:493+`）。
- 启动期「一次性读进 state」先例：`load_prompt_history`（`src/db/service.rs:429-437`，
  失败只 `tracing::warn!`、降级为空）。
- 全局不分区表的先例与理由：`prompt_history`——「不按 workspace/session 分区：历史是全局共享的」
  （`migration.rs:286-287`）。
- 时间戳：新表用 `*_at_ms INTEGER` + `now_ms()`（`src/db/mod.rs:829`），与 v10~v14 一致。
- 生成稳定 id：`new_external_id(prefix)`（`src/db/mod.rs:813-821`）。

**滚动条与列表窗口**

- `ui::widgets::scrollbar`（`src/ui/widgets/scrollbar.rs`）：`layout(area, total, visible, offset)`、
  `thumb_grab_offset`、`offset_from_track_row`、`offset_from_drag_row`、`render_buffer`；
  **只画 thumb `▐`，轨道不画符号**。
- 整页视口语义的可照抄实现：`TaskPage::scroll_window`（`src/app/state/lx/task.rs:236-257`，
  `max = len - visible`，`viewport: Option<usize>` 显式视口；滚轮不动高亮）。
- 滚轮步长：prompt 正文 `WHEEL_LINES = 3`；树视图每格 1 行。

**布局几何**

- `prompt_header_rect` / `prompt_toolbar_rect` / `prompt_divider_rect` / `prompt_text_rect` /
  `prompt_scrollbar_rect` 都在 `src/layout.rs:740-826`，可**直接复用**给 Todo 面板体
  （同一容器、同一表头两行、同一贴右缘滚动条槽）。
- `src/layout.rs` 已 **999 行**，是设计文档明令的硬顶，**本次一行不加**。
- 工具栏按钮宽度是写死的常量阶梯（`src/layout.rs:23-61`）：
  `PROMPT_SELECT_ALL_WIDTH = 3`、`PROMPT_PIN_WIDTH = 3`、`PROMPT_FOLDERS_WIDTH = 5`、
  以及 `PROMPT_SELECT_ALL_MIN_WIDTH` / `PROMPT_PIN_MIN_WIDTH_EDITOR|TREE` /
  `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 三处阈值。

**文案与样式**

- 工具栏常量在 `src/ui/text.rs:87-108`：`[*]` 全选、`[○]`/`[◉]` 钉住、`[←]`/`[→]` 撤销重做、
  `[≡]`/`[/]` 模式、`[dir]` 引用文件夹、`[tmp]` 临时提示词。
- 字形守卫测试 `src/ui/text/tests.rs:232-`（`prompt_button_icons_are_single_width_and_non_pua`）
  要求按钮标签形如 `[x]` **只含一个图标字符**——改成 `[all]`/`[pin]`/`[unpin]` 后这条断言
  必然失败，需按新口径改写（见 §6.3）。
- 选中标记统一 `❯`（`text::AGENT_SELECTED_MARKER` / `CURRENT_ITEM_MARKER`，`text.rs:22/25`）。
- 列表选中行样式：`style::selection()`（`src/ui/style.rs:81`）；禁用/次要 `style::muted()`；
  删除线 `Modifier::CROSSED_OUT`（既有先例 `style.rs:318/455`）。

### 1.2 目标

1. Todo 面板成为**可交互的用户清单**：新建 / 编辑 / 循环状态 / 删除 / 清空已完成，
   全部落 SQLite，重启后原样恢复。
2. 列表行渲染状态图标与选中标记，右侧有滚动条；条目多时可滚轮 / 拖 thumb / 方向键导航。
3. 工具栏提供 4 个功能按钮，并用 tooltip 说明动作。
4. Todo 面板拥有独立的键盘焦点与输入通道（含粘贴、IME 硬件光标）。
5. 顺手把 prompt 工具栏的钉住与全选从图标改为英文单词（用户明确要求，范围见 §2 决策 5）。

### 1.3 非目标

- **不做** agent 读写 todo（不加工具、不注入 prompt、不做模型驱动清单）。
- **不做**父子层级、优先级、截止日期、多列表、拖拽排序、`J`/`K` 手动重排。
- **不做**跨 worktree / 跨数据库同步（隔离库各自独立，见 §2 决策 16）。
- **不改** prompt 正文、分页、树视图、浮层、自动保存的既有语义。
- **不改** `right_tab` 的持久化策略（仍不落库、启动恒 `Prompt`）。
- **不复活** 项目里的 `TemplateStatus::Todo`（markdown 模板块三态，`app/markdown/template.rs:5-9`）
  与 todo 面板的任何联动。

---

## 2. 已确认决策（Grill Me 结论，17 条全部经用户确认）

| # | 决策项 | 结论 |
| --- | --- | --- |
| 1 | 存储层 | 落 SQLite；**全局一份，不按工作区隔离**（无 `workspace_id` 列） |
| 2 | 数据模型 | 扁平单列表；三态 `todo / in_progress / done`；`position` 手动排序；无父子层级 |
| 3 | 输入形态 | 列表内 inline 单行输入（`TextInput`）；Esc 取消 = 撤销（新建则不创建、编辑则恢复原文） |
| 4 | 焦点模型 | 新增 `todo_focused: bool`，与 `prompt_focused` 平行且互斥；不迁移 prompt 既有字段。切到 Todo 不保留 Prompt 焦点，切回也不自动聚焦 |
| 5 | 按钮文案 | 只改两处：钉住 `[pin]`/`[unpin]`、全选 `[all]`；`[≡] [dir] [←] [→] [tmp]` 保持原样；放不下时按既有阶梯丢弃整颗按钮 |
| 6 | 与 agent 的关系 | 纯用户清单；agent 不读不写、不注入 |
| 7 | 状态切换 | `Space` 循环三态 `[ ]→[~]→[x]→[ ]`；按键表见 §3.2 |
| 8 | 写入时机 | 每次编辑提交（Enter / 失焦 / 切 tab / 切工作区）落库一条；失败**不回滚内存**，全局 toast `todo save failed`；逐条 `UPSERT` 幂等 |
| 9 | 排序 | `position` 密集递增 + 事务内顺移；不支持拖拽与手动上下移动 |
| 10 | 滚动 | 右侧 1 列滚动条槽（复用 `widgets::scrollbar`）；滚轮每格 **3 行**、只动视口不动高亮；键盘移动高亮时自动滚入视口 |
| 11 | 工具栏按钮 | 4 个：`[new] [edit] [del] [clear done]`；`[clear done]` 不确认、只删 done、可撤销；无效时置灰；空态文案 `no todos yet` |
| 12 | 字形 | `[ ]` / `[~]` / `[x]` + 选中 `❯`；`[ ]` 内留一个空格；**不引入** `●`/`○`/`■`/`□`（后者专属于消息与工具调用） |
| 13 | 打断语义 | 切 tab / 点别行 / 切工作区 = **提交**；`Esc` = 取消。空输入提交不创建；清空为纯空白提交 = 删除该条；提交 trim；提交不改选中（除「点另一行」） |
| 14 | 撤销 | 单槽删除撤销，只覆盖 `d` 与 `[clear done]`；`Ctrl+Z` 无内容时**吞掉不穿透** |
| 15 | 工作区切换 | 列表内容、选中、滚动、焦点**原样保留**；编辑中切工作区走提交 |
| 16 | 全局的边界 | 跟随所在数据库：同一 lx 进程内所有工作区共享一份；不同 worktree 因库隔离各有一份（不做跨库同步） |
| 17 | 文档落点 | 新建 `docs/feature/right-panel-todo/{design,task}.md`，用户审核后才动代码 |

**明确推翻的旧决策**（必须记录，避免后人以为实现跑偏）：

- `docs/feature/right-panel-tabs/design.md` 决策 **#10**「Todo 功能按钮行整行空白 + 内容区空白 + 零占位文案」→ 作废，改为真实面板 + 空态文案 `no todos yet`。
- 同文件决策 **#11**「点击 Todo 任何区域一律吞掉、不聚焦」→ 作废，改为「按钮/行/滚动条命中 → 动作；空白处吞掉」。
- `docs/standards/tui-design-requirements.md:25` 中「Todo 面板为占位……零占位文案；点击任何区域与滚轮一律吞掉」→ 改写为真实面板描述（见 §7）。

---

## 3. 目标布局与交互

### 3.1 布局

```text
╭─ Prompt ─ Todo ──────────────────────────────────╮  ← 容器顶边框（不变）
│ [new] [edit] [del] [clear done]                    │  ← Todo 工具栏
├──────────────────────────────────────────────────┤  ← 分割线（复用 render_divider）
│ ❯ [~] 写迁移脚本                          ▐      │  ← 选中行：❯ + 状态框 + 文本
│   [x] 修登录超时                          ▐      │  ← 已完成：删除线 + muted
│   [ ] 开会                                        │
├──────────────────────────────────────────────────┤  ← 列表与统计块的分割线（第三轮）
│ total 5 · doing 1 · done 2 40%                    │  ← 摘要行
│      ███                       ███                │
│      ███          ███          ███                │  ← 三态竖向柱图（与列表平分高度）
│    open 2       doing 1      done 2               │  ← 轴标签行
╰─lx-tui · b:main ───────────────────── [▸]────────╯  ← 容器底边框（不变）
```

- 工具栏左端 4 个按钮；**工具栏右端不再放计数**，完成情况统一交给底部统计行。
- **宽度阶梯（精确到列，内宽 = 面板宽 − 2）**：按钮宽 `[new]`=5、`[edit]`=6、`[del]`=5、
  `[clear done]`=12，间隔 1；四按钮合计 31 列。
  计数 `done/total` 右对齐，**只在「按钮右边剩余列 ≥ 计数实际宽度 + 1」时绘制**
  （不预留固定宽度，`3/8` 与 `12/12` 各按自身宽度算）。

  | 内宽 | 显示 |
  | --- | --- |
  | ≥ 31 且剩余够 | 四按钮 + 计数 |
  | ≥ 31 | 四按钮（丢计数） |
  | ≥ 18 | `[new] [edit] [del]` |
  | ≥ 12 | `[new] [edit]` |
  | ≥ 5 | `[new]` |
  | < 5 | 无按钮 |

  > **默认宽度放不下四按钮**：`DEFAULT_PROMPT_WIDTH = 30`（`src/app/state.rs:56`），内宽 28，
  > 落在「`[new] [edit] [del]`」档。实际启动宽度是 `side_width` = 可用宽度的 2/5
  > （`src/ui/layout.rs:140-142`），120 列终端下约 38 列 → 内宽 36，四按钮齐全。
  > 窄终端只丢按钮、不截断文字。
- 行格式：`❯`/空格（1 列）+ 空格 + `[x]`（3 列）+ 空格 + 文本（截断，尾溢用单行截断不折行）。
  未选中行首列为 1 个空格，保证文本左对齐。
- 底部统计块（第三轮定型）：列表与统计块**平分**主体区（分割线 1 行，余数留给列表），
  高度 = `(body.height - 1) / 2`；主体区高度 < 7 时不切分，列表独占。
  块内三行结构：
  1. 摘要 `total N · doing N · done N N%`（标签 `style::muted()`、数值 `style::text()`，
     段间 `" · "`；`N%` = 已完成/总数向下取整；整行量宽，放不下整行不画）。
  2. 三态竖向柱图：`open`/`doing`/`done` 各占 `宽/3` 的等宽槽位，柱宽 `min(槽宽, 3)`、
     槽内水平居中，柱高 = `count × 可用行数 / 最大值`，非零计数至少 1 行。
     配色 `todo_bar_open()`（muted）/ `todo_bar_active()`（accent）/ `todo_bar_done()`（Green）。
  3. 轴标签行：各槽居中 `open N` / `doing N` / `done N`，`style::muted()`。
  空列表仍占同样版面（摘要归零、不画柱子），面板高度不随数据跳变。
- 滚动条：占列表区最右 1 列，轨道高度**只到列表底边**（不覆盖统计块），thumb `▐`；
  `total <= visible` 时不画。
- 空态：列表区首行居中 `no todos yet`（`style::muted()`）。
- 编辑态：该行文本区变成输入内容，行尾补硬件光标；行首仍是 `❯` + 空格 + 状态框。
- 窄面板降级：

```text
╭─ Prompt ─ Todo ─────────────╮
│ [new] [edit] [del]          │  ← 内宽 28（面板 30 列）：丢 [clear done] 与计数
│ [new] [edit]                │  ← 内宽 18：丢 [del]
│ [new]                       │  ← 内宽 12：只留 [new]
╰─────────────────────────────╯  ← 内宽 < 5：按钮全丢，只剩列表
```

### 3.2 按键表

**列表态（`todo_focused = true`，无编辑中）**

| 键 | 行为 |
| --- | --- |
| `j` / `↓` | 下移选中（末项钳制，不循环） |
| `k` / `↑` | 上移选中 |
| `g` `g` | 跳首（两次 `g`，间隔不限，中间任何非 `g` 按键重置） |
| `G` | 跳尾 |
| `Space` / `Tab` | 循环当前项状态 `todo → in_progress → done → todo` |
| `a` | 末尾追加新条目并进入编辑 |
| `Enter` | 编辑当前项（列表为空时新建） |
| `Shift+Enter` / `o` | 在当前项之后插入新条目并进入编辑 |
| `e` | 编辑当前项 |
| `d` | 删除当前项（立即，无确认） |
| `Ctrl+Z` | 撤销最近一次删除（单槽） |
| `Esc` / `q` | 释放焦点（`todo_focused = false`，`q` 仅在无修饰键时） |
| `Tab` | 同 `Space`：循环当前项状态（不再吞掉；本轮按 lx-agent 口径改） |

**编辑态**

| 键 | 行为 |
| --- | --- |
| `Enter` / `Shift+Enter` | 提交（trim；空则新建不创建 / 编辑则删除该条；单行输入没有换行语义） |
| `Esc` | 取消（新建消失、编辑恢复原文） |
| `←`/`→`/`Home`/`End`/`Backspace`/`Delete` | 单行编辑 |
| `Ctrl+C` | 清空输入（与页重命名同口径，`OverlayKey::Clear`） |
| `Tab` | 切换到下一状态并登记写库（新建态无记录可切，吞掉） |
| `Ctrl+Z` | 同列表态（吞掉不穿透） |
| 其余 | 吞掉，绝不写 PTY |

**何时获得焦点**：左键点列表区任意行、工具栏按钮行为（按钮动作隐含聚焦）、
点击分割线下方内容区。**不**因切换到 Todo tab 自动聚焦（对齐 `right-panel-tabs` 决策 #5）。

**焦点不变量**：`todo_focused ⇒ right_tab == Todo && !prompt_collapsed`；
`prompt_focused ⇒ right_tab == Prompt`（既有不变量保持）。
`focus_prompt`（`src/app/update/editor.rs:536-540`）需追加 `todo_focused = false`。

### 3.3 工具栏按钮

| 按钮 | 动作 | 禁用条件 | tooltip |
| --- | --- | --- | --- |
| `[new]` | 末尾新建并进入编辑 | 无 | `new todo` |
| `[edit]` | 编辑当前选中行 | 列表为空 | `edit todo` |
| `[del]` | 删除当前选中行 | 列表为空 | `delete todo` |
| `[clear done]` | 删除所有 `done` 项 | 无 `done` 项 | `clear done todos` |

禁用态用 `style::muted()` 绘制且点击无效（不产生任何状态变化、不置脏）。

### 3.4 鼠标

| 区域 | 行为 |
| --- | --- |
| 工具栏按钮 | 动作；无效按钮吞掉 |
| 列表行 | 单击选中 + 聚焦（若正在编辑另一行先提交）；**双击进入该行的行内编辑** |
| 滚动条 thumb | 开始拖拽（`todo_scroll_drag`） |
| 滚动条轨道 | 点击跳转（`offset_from_track_row`） |
| 顶边框空白 / 分割线 / 统计块 / 列表外空白 | **让右侧容器进入选中态**（`todo_focused = true`），不动数据、不打断编辑、不穿透 |
| 滚轮 | 视口移动 3 行（不动高亮、不聚焦） |
| 右键 | 吞掉（本轮不做 Todo 右键菜单） |

---

## 4. 架构与数据流

### 4.1 数据模型（`src/db/todo.rs`，新文件）

```rust
/// todo 条目状态；库内以 TEXT 存储并对齐 CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    Todo,
    InProgress,
    Done,
}

impl TodoStatus {
    /// 库内文本值。
    pub fn as_str(self) -> &'static str;
    /// 由库内文本解析；未知值视为 `Todo`（避免一条脏数据毁掉整张列表）。
    pub fn from_db(value: &str) -> Self;
    /// `Space` 循环：todo → in_progress → done → todo。
    pub fn cycle(self) -> Self;
}

/// 一条 todo（内存视图与库记录同构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRecord {
    /// 稳定 id（`todo-<nanos>-<seq>`），用于 UPSERT 与删除。
    pub id: String,
    pub text: String,
    pub status: TodoStatus,
    /// 密集递增排序位（0 基）。
    pub position: i64,
}
```

### 4.2 表结构与迁移（v15）

`src/db/migration.rs` 追加（不改任何历史迁移文本）：

```rust
/// v15 全局 todo 表：不按工作区/会话分区（同 `prompt_history` 的全局口径）。
///
/// `position` 为密集递增排序位，插入时由应用层在事务内顺移；
/// `id` 为稳定业务键，UPSERT 与删除都以它为准。
const SCHEMA_V15_TODO: &str = r#"
CREATE TABLE IF NOT EXISTS todo (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'done')),
    position INTEGER NOT NULL CHECK (position >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_todo_position ON todo(position);
"#;
```

`migrate()` 尾部追加（照抄 v14 的「只增表」写法，`src/db/migration.rs:623-630`）：

```rust
if version < 15 {
    let transaction = self.conn.unchecked_transaction()?;
    transaction.execute_batch(SCHEMA_V15_TODO)?;
    ensure_no_foreign_key_violation(&transaction)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
}
```

同时 `src/db/mod.rs:25` 改 `SCHEMA_VERSION = 15`，`src/db/mod.rs` 加 `pub mod todo;`。

> **唯一索引的必要性**：`position` 上的 `UNIQUE` 是「顺移正确性」的最后一道闸门——
> 若某次事务漏顺移，插入会直接 `UNIQUE constraint failed` 而不是悄悄出现两条同位置条目。
> 顺移必须在同一事务内**从大到小**更新，否则中间态会撞唯一约束（见 §4.3）。

### 4.3 数据库 API（`src/db/todo.rs`）

```rust
impl Database {
    /// 读取全表，按 `position` 升序。
    pub fn list_todos(&self) -> Result<Vec<TodoRecord>>;

    /// 写入一条（UPSERT 语义，对齐决策 8「逐条 UPSERT 幂等」）。
    ///
    /// - 行已存在：只更新 `text` / `status` / `updated_at_ms`，**不动 `position`**；
    /// - 行不存在：在 `record.position` 处插入，并把 `position >= at` 的行顺移 +1。
    ///
    /// 同一条目重复写入（改文本、切状态）是幂等的；「插入曾失败、之后再编辑」
    /// 也会因为走插入分支而自愈，不会留下一条库中不存在的幽灵行。
    pub fn save_todo(&self, record: &TodoRecord) -> Result<()>;

    /// 删除单条；`affected == 0` 不算错误（幂等），删除后把剩余位置重排为 `0..n`。
    pub fn delete_todo(&self, id: &str) -> Result<()>;

    /// 删除全部指定状态的条目，返回**被删记录**（供单槽撤销回填）；
    /// 删完后把剩余位置重排为 `0..n`。
    pub fn delete_todos_with_status(&self, status: TodoStatus) -> Result<Vec<TodoRecord>>;

    /// 批量恢复被删记录（撤销）：按各自原 `position` 插入并重排为 `0..n`。
    pub fn restore_todos(&self, records: &[TodoRecord]) -> Result<()>;
}
```

> **为什么不拆成 `insert` / `update` 两条**：拆开后「插入失败 → 之后编辑该条」会走
> `update` 而永远命中 0 行，导致这条 todo 再也存不进库（永久幽灵行）。UPSERT 的
> 「不存在则插」分支恰好把它救回来，且不引入任何额外状态。

写路径约定（照抄 `db/history.rs` 与 `db/mod.rs` 的既有风格）：

- 全部写操作用 `self.conn.unchecked_transaction()`，单事务完成；
- 文本在 DB 层再校验一次：`trim()` 后为空返回 `DbError::InvalidDraft`；
  超过 500 **字符**（非字节）返回 `DbError::MessageTooLarge`；
- 时间戳 `now_ms()`；
- 删除/恢复同样重排 `position` 为 `0..n`（保持密集），重排在同一事务内。

顺移算法（**从大到小**，避免中间态撞唯一索引）：

```rust
// 1) 先取需要顺移的行（降序），2) 逐条按 id 更新。
// 不能用 `UPDATE ... ORDER BY`：那需要 SQLITE_ENABLE_UPDATE_DELETE_LIMIT 编译选项。
let ids: Vec<String> = tx
    .prepare("SELECT id FROM todo WHERE position >= ?1 ORDER BY position DESC")?
    .query_map(params![at], |row| row.get(0))?
    .collect::<rusqlite::Result<_>>()?;
for id in &ids {
    // 降序保证每条落到"刚刚腾出来的位置"，中间态永不撞 `idx_todo_position`。
    tx.execute("UPDATE todo SET position = position + 1 WHERE id = ?1", params![id])?;
}
```

> 列表规模是十位数，逐条更新完全可接受；换来的好处是不依赖 SQLite 的可选编译特性，
> 也不会因为一条 `UPDATE` 的隐式执行顺序写出「先更新小位置再撞大位置」的 bug。

### 4.4 状态层（`src/app/state/todo.rs`，新文件）

**为什么不挂在 `Workspace` 上**：数据全局一份（决策 1/16），挂工作区意味着切换时要做
「保留同一份、但路径不同」的假动作。挂在 `AppState` 与 `prompt_history` 同级，
语义最直白。

```rust
/// todo 面板状态：列表、选中、视口、编辑现场与删除撤销槽。
#[derive(Debug, Default)]
pub struct TodoPanel {
    /// 全量列表，始终按 `position` 升序且 `position == 下标`（不变式）。
    items: Vec<TodoRecord>,
    /// 选中行下标；列表为空时为 None。
    selected: Option<usize>,
    /// 视口首行下标；0 表示贴顶。
    scroll: usize,
    /// 编辑现场：目标（新建时为 None）与单行输入。
    editing: Option<TodoEdit>,
    /// 删除撤销槽：一次删除（单条或一批）的记录快照。
    undo: Option<Vec<TodoRecord>>,
    /// 等待第二个 `g` 的标记（`gg` 跳首）。
    pending_g: bool,
}

/// 编辑现场：`target` 为 None 表示新建（提交时才分配 id 与 position）。
#[derive(Debug)]
pub struct TodoEdit {
    pub target: Option<String>,
    pub input: TextInput,
    /// 新建插入位置（`items.len()` 或当前项下标 + 1）。
    pub at: usize,
}
```

`AppState` 新增字段（`src/app/state.rs`，紧邻 `prompt_focused` / `right_tab`）：

```rust
/// todo 面板是否持有键盘焦点；为真时按键进入 todo 而非焦点窗格。
/// 不变量：true ⇒ right_tab == Todo && !prompt_collapsed。
pub todo_focused: bool,
/// 全局 todo 列表状态（不随工作区变化）。
pub todo: TodoPanel,
/// 拖拽 todo 滚动条 thumb 时相对顶部的抓取偏移。
pub todo_scroll_drag: Option<u16>,
/// 鼠标悬停的 todo 行（列表可见行下标）；None 表示未悬停。
pub todo_hover: Option<usize>,
```

> 视口只保留 `TodoPanel.scroll` 一个字段：滚轮与键盘都写它，但语义分开——
> 滚轮只改 `scroll`，键盘移动选中时调用 `scroll_into_view()` 保证选中可见。
> 这与 `TaskPage` 的 `viewport` / `active` 双字段等价，但少一个字段就能少一类
> 「两个字段不一致」的 bug；代价是「键盘移动后无法恢复滚轮前的位置」，而这在
> 清单场景里不是需求。

### 4.5 更新层（`src/app/update/todo.rs`，新文件）

全部函数返回 `bool`（是否产生变化，供事件层置脏），全部不碰 IO：

```rust
/// 左键点击列表行：提交进行中的编辑 → 选中该行 → 聚焦。
pub fn select_todo_row(state: &mut AppState, index: usize) -> bool;

/// 聚焦/失焦。失焦一律先 `commit_todo_edit`（与切 tab 同口径）。
pub fn focus_todo(state: &mut AppState) -> bool;
pub fn blur_todo(state: &mut AppState) -> bool;

/// 键盘/悬停移动选中；`delta` 为 ±1（跳首尾由调用方传 `isize::MIN/MAX`，内部饱和）。
pub fn move_todo_selection(state: &mut AppState, delta: isize) -> bool;
pub fn set_todo_hover(state: &mut AppState, hover: Option<usize>) -> bool;

/// 循环当前项状态并落库。
pub fn cycle_todo_status(state: &mut AppState) -> bool;

/// 进入编辑：`New`（末尾追加）| `After(index)`（当前项之后插入）| `Current`。
pub fn begin_todo_edit(state: &mut AppState, target: TodoEditTarget) -> bool;

/// todo 按键总入口：按 `panel.editing` 分派到列表动作或单行编辑。
/// 返回是否消费该键（`false` 表示吞掉且无变化）。
pub fn apply_todo_key(state: &mut AppState, key: TodoKey) -> bool;

/// 粘贴：编辑态写入输入框，列表态吞掉；返回是否有变化。
pub fn apply_todo_paste(state: &mut AppState, text: &str) -> bool;

/// 启动加载：整体替换列表并重置选中/视口/编辑/撤销槽。
pub fn apply_todo_list(state: &mut AppState, items: Vec<TodoRecord>);

/// 提交编辑：`Esc` 之外的打断源统一调用；返回是否产生变化。
pub fn commit_todo_edit(state: &mut AppState) -> bool;

/// 取消编辑：新建丢弃、编辑恢复原文。
pub fn cancel_todo_edit(state: &mut AppState) -> bool;

/// 删除当前项（写入撤销槽）。
pub fn delete_todo_selected(state: &mut AppState) -> bool;

/// 删除全部完成项（写入撤销槽）。
pub fn clear_done_todos(state: &mut AppState) -> bool;

/// 撤销最近一次删除。
pub fn undo_todo_delete(state: &mut AppState) -> bool;

/// 滚轮：只动视口。`direction` 为 ±1 的滚轮步，内部乘 `WHEEL_LINES`(=3,
/// `src/app/update/selection.rs:15`)；`rows` 为当前可见行数。
pub fn scroll_todo(state: &mut AppState, direction: isize, rows: usize) -> bool;

/// 拖拽滚动条 / 轨道点击。
pub fn set_todo_scroll(state: &mut AppState, offset: usize, rows: usize) -> bool;

/// 服务回执：写入失败；弹全局 toast（内存列表保持不变）。错误详情由服务层
/// `tracing::warn!` 记录，不经 app 层传递。
pub fn fail_todo_write(state: &mut AppState);
```

**`commit_todo_edit` 的完整分支**（这是整个状态机唯一的汇合点，必须逐条写死）：

| 编辑目标 | trim 后文本 | 结果 | 登记的效应 |
| --- | --- | --- | --- |
| 新建 | 空 | 不创建，编辑态结束，选中不变 | 无 |
| 新建 | 非空 | 在 `edit.at` 处插入，选中新行 | `SaveTodo { record }` |
| 已有 | 空 | 删除该条（进撤销槽），选中钳制 | `DeleteTodo { id }` |
| 已有 | 非空且与原文本相同 | 不写库，编辑态结束 | 无 |
| 已有 | 非空且不同 | 改文本，选中不变 | `SaveTodo { record }` |

**写入登记**：每个会改动列表的函数在改完内存后立即 `state.effects.push(...)`（见 §4.6）。
一次用户动作登记**一条**效应（`[clear done]` 也是单条 `DeleteTodosWithStatus`，
删除与重排在库内同一事务完成），由 `pump` 在同一帧内执行。

**没有「待确认」状态**：内存里的列表就是唯一真源，库只是副本。写成功无需回报
（不像 prompt 自动保存要靠修订号确认「已保存」）；写失败只提示、不改内存，
下一次成功写入自然把库拉回一致。

### 4.6 效应枚举（`src/app/effects.rs`）

```rust
/// 写入一条（UPSERT：不存在则插入并顺移位置，存在则只改文本/状态）。
SaveTodo { record: TodoRecord },
/// 删除一条 todo。
DeleteTodo { id: String },
/// 删除某状态的全部 todo（`[clear done]`）。
DeleteTodosWithStatus { status: TodoStatus },
/// 恢复一批被删 todo（撤销）。
RestoreTodos { records: Vec<TodoRecord> },
```

服务侧分支（`src/db/service.rs::execute`）逐条调用 §4.3 的 API；失败
`update::fail_todo_write(state)` 并 `tracing::warn!(%error, "todo write failed")`；
成功不改状态（内存已是真源）。**无持久化（`db: None`）时直接返回**——内存模式可用，
与 `load_prompt_history` 的降级口径一致。

启动加载：`Persistence::start` 之后照 `load_prompt_history` 加一段

```rust
/// 载入全局 todo 列表；库不可用或读取失败时保持空列表，不阻断启动。
pub fn load_todos(&self, state: &mut AppState) {
    let Some(items) = self.with_db(|db| db.list_todos()) else { return };
    update::apply_todo_list(state, items);
}
```

`src/main.rs:31` 附近追加 `persistence.load_todos(&mut state);`。

### 4.7 键盘与粘贴通道

**键盘**：`src/input/mod.rs`

新增路由变体与入口参数（**不加 `Action` 变体**——todo 命令需要携带字符，
与 `Action` 的 `Copy` 无参语义不符；照现状由 `Routed::Todo(TodoKey)` 承载）：

```rust
/// todo 面板按键：**按键原样**，不做语义解释。
///
/// 「`Space` 是循环状态还是输入空格」「`Enter` 是插入新行还是提交」
/// 全部由 `update::apply_todo_key` 按 `panel.editing.is_some()` 判定，
/// 输入层不感知应用状态（同 lx 输入「清空还是复制由 update 决定」的既有口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoKey {
    Char(char),
    /// 裸空格（不带修饰键）。
    Space,
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Ctrl+Z / Cmd+Z：撤销删除（无内容时吞掉）。
    Undo,
    /// Ctrl+C / Cmd+C：编辑态清空输入（列表态吞掉）。
    Clear,
}

pub fn route(
    key: KeyEvent,
    mode: TermMode,
    prompt_focused: bool,
    overlay: Option<OverlayKind>,
    pane_lx: bool,
    todo_focused: bool, // 新增尾参
) -> Option<Routed>
```

路由规则（顺序不可调换）：

1. `Release` → `None`（既有）；
2. `Ctrl+Q` → `Action::Quit`（**保持最高优先**，既有 `input/mod.rs:49-51`）；
3. `overlay` → 浮层键（既有）；
4. `prompt_focused` → 编辑器/复制/保存（既有，**不变**）；
5. **新增** `todo_focused` → `todo_key(key).map(Routed::Todo)`；**未映射按键返回 `None`
   （即被吞掉）**，与 `prompt_focused` 分支同口径，绝不写 PTY；`None` 不置脏（状态未变）；
6. `pane_lx` → lx 输入（既有）；
7. 其余 → 编码写窗格（既有）。

`todo_key(key)` 只做**逐键翻译**（`KeyCode::Char('a')` → `TodoKey::Char('a')`），
不判断上下文；未识别的键返回 `None`（被 `route` 吞掉）。
`apply_todo_key(state, key)` 按 `panel.editing` 分派：

- 编辑态：`Char`/`Space`/`Backspace`/`Delete`/`Left`/`Right`/`Home`/`End` → 输入框；
  `Enter` → 提交（并顺带 `focus_todo`）；`Esc` → 取消；`Clear` → 清空输入；
  其余（`Tab`/`Up`/`Down`/`Undo`）→ 吞掉（编辑中不允许上下移动选中，避免与光标键歧义）；
- 列表态：`Char('j'|'k'|'g'|'G'|'a'|'o'|'e'|'d'|'q')` → 对应动作；
  `Space` → 循环状态；`Up`/`Down` → 移动选中；`Enter` → 在当前项后插入；
  `Esc` → 失焦；`Undo` → 撤销删除；`Clear`/`Tab`/`Backspace`/`Delete`/`Left`/`Right`/`Home`/`End`
  → 吞掉。

**事件层**：`src/event/input.rs`

- 页重命名拦截之后、`input::route` 之前**不加**额外拦截；
- `input::route` 调用处补 `state.todo_focused` 实参（`src/event/input.rs:51`）；
- `match routed` 新增：

```rust
Routed::Todo(key) => {
    if update::apply_todo_key(state, key) {
        *dirty = true;
    }
}
```

**粘贴**：`src/event/input.rs:151` 的 `if state.prompt_focused` 分支**之前**新增：

```rust
if state.todo_focused {
    // 编辑态写入输入框；列表态吞掉，绝不写隐藏终端。
    if update::apply_todo_paste(state, &text) {
        *dirty = true;
    }
    return;
}
```

**硬件光标**：`ui::right_panel::render` 的 Todo 分支在有编辑现场时返回输入框光标坐标，
沿用 `src/ui/mod.rs:96` 的 `prompt_cursor` 链路即可获得 IME 支持。

### 4.8 渲染层（`src/ui/right_panel/todo.rs`，新文件）

`src/ui/right_panel.rs` 从单文件改为目录模块：

```text
src/ui/right_panel.rs        // 容器：外框 / tab / 底边框 / 分发（保留，改为 mod 声明）
src/ui/right_panel/todo.rs   // Todo 面板体：几何 + 绘制 + 命中（新）
src/ui/right_panel/tests.rs  // 既有测试 + 新测试
```

`right_panel.rs` 的 Todo 分支改为：

```rust
RightTab::Todo => todo::render(frame, area, state),
```

几何函数（**绘制与命中共用同一函数**，这是项目已确立的不变量）：

```rust
/// 工具栏按钮。
pub enum TodoButton { New, Edit, Delete, ClearDone }

/// 工具栏按钮矩形；放不下时返回 None（按 §3.1 的丢弃阶梯）。
pub fn button_rect(panel: Rect, button: TodoButton) -> Option<Rect>;

/// 按钮命中。
pub fn button_at(panel: Rect, column: u16, row: u16) -> Option<TodoButton>;

/// 列表区矩形：**直接委托** `layout::prompt_text_rect(panel)`
/// （已扣表头两行、已让出最右 1 列滚动条槽）。不新增几何算法。
pub fn list_rect(panel: Rect) -> Rect;

/// 可见行数（`list_rect(panel).height`）。
pub fn visible_rows(panel: Rect) -> usize;

/// 行命中：返回列表下标（已加 `scroll` 偏移）。
pub fn row_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize>;

/// 滚动条几何（无溢出时为 None）。
pub fn scrollbar(panel: Rect, state: &AppState) -> Option<widgets::scrollbar::ScrollbarLayout>;
```

绘制要点：

- 工具栏行沿用 `layout::prompt_toolbar_rect`；按钮逐字符 `cell.reset()` + `set_char` + `set_style`
  （照抄 `src/ui/prompt.rs:130-145` 的写法，避免样式继承——这是 `right-panel-tabs` 踩过的坑）；
- 分割线复用 `prompt::render_divider(panel, header, buf, focused)`；
- 行样式：选中行与悬停未选中行**都用** `style::selection()`（与 prompt 树视图的
  `row_at` / 悬停口径一致，`src/ui/prompt/tree.rs:199,296`）——先 `buf.set_style(rect,
  Style::reset())` 再逐段套，防继承聚焦边框的 `accent()`；
  已完成行文本 `style::muted()` + `CROSSED_OUT`；进行中行状态框 `style::accent()`；
  待办行状态框 `style::text()`；
- 状态框字符由 `ui::text` 常量提供（见 §4.10），渲染层不写字面量；
- 编辑行：文本区绘制 `TextInput.text()`（超宽时按光标位置横向滚动，保证光标始终可见），
  **不画任何伪光标字符**；`render` 返回真实光标坐标 `(x, y)`，由
  `src/ui/mod.rs:96` 的既有链路交给 `frame.set_cursor_position`，IME 候选框自动跟随；
- 滚动条：`widgets::scrollbar::render_buffer`。

### 4.9 事件层闸门（逐个落点）

| 文件:行 | 现状 | 改动 |
| --- | --- | --- |
| `event/mouse/press.rs:269-273` | `!prompt_panel_active` 整块吞掉 | 拆两支：`right_tab == Todo` 时按「按钮 → 行 → 滚动条 → 分割线/空白吞掉」处理；其余情况保留原吞掉 |
| `event/mouse/press.rs:209-212` | 三个滚动条 press | 新增 `handle_todo_scrollbar_press`（照抄 `handle_prompt_scrollbar_press`，`src/event/mouse/hit.rs:41-66`） |
| `event/mouse/wheel.rs:263` | `!prompt_panel_active → return` | Todo 时改为 `update::scroll_todo(state, dir, visible)` 并置脏 |
| `event/mouse/hover.rs:243-257` | 树行悬停 | 新增 `todo_hover` 计算与 `set_todo_hover`；TAB 悬停逻辑不变 |
| `event/mouse/drag.rs:81-97` | prompt 滚动条拖拽 | 新增 `todo_scroll_drag` 分支，`handle_mouse_up_left` 一并 `take()` |
| `event/mouse/hit.rs:418-433` | `pane_content_inner` 返回零矩形 | **保持**：Todo 内容区不是 prompt 文本区，拖选与光标定位都不该进来 |
| `ui/prompt_frame.rs:232-240` | `prompt_scrollbar` 闸门 | **保持**：Todo 有独立滚动条函数 |
| `ui/tooltip.rs:154` | prompt 按钮 tooltip 闸门 | 新增 Todo 按钮 tooltip 目标（`TooltipTarget::TodoButton(TodoButton)`） |
| `event/overlay.rs:456` | 树右键菜单闸门 | **保持**：Todo 右键吞掉 |

> **顺序要求**：Todo 的命中链必须排在上面这些「整块吞掉」分支**之前**，
> 否则永远点不到（`right-panel-tabs/design.md:471-476` 记录的坑）。

### 4.10 文案（`src/ui/text.rs`）

新增（**放文件末尾新分区**，注释标明「右侧容器 Todo 面板」；该文件已 1260 行，
若新增后越界，则新建 `src/ui/text/todo.rs` 子模块并在 `text.rs:3` 声明 `pub mod todo;`）：

```rust
/// Todo 面板：工具栏按钮。
pub const TODO_NEW_LABEL: &str = "[new]";
pub const TODO_EDIT_LABEL: &str = "[edit]";
pub const TODO_DELETE_LABEL: &str = "[del]";
pub const TODO_CLEAR_DONE_LABEL: &str = "[clear done]";
/// 状态框（三态各 3 列，`[ ]` 内含一个空格与 markdown 任务列表口径一致）。
pub const TODO_STATUS_TODO: &str = "[ ]";
pub const TODO_STATUS_IN_PROGRESS: &str = "[~]";
pub const TODO_STATUS_DONE: &str = "[x]";
/// 空态文案。
pub const TODO_EMPTY: &str = "no todos yet";
/// 写库失败提示。
pub const TOAST_TODO_SAVE_FAILED: &str = "todo save failed";
```

tooltip 文案加进 `tooltip_hint`（`src/ui/text.rs:900-`）：
`new todo` / `edit todo` / `delete todo` / `clear done todos`。

**prompt 工具栏文案改动**（决策 5）：

```rust
pub const PROMPT_SELECT_ALL_LABEL: &str = "[all]";   // 原 "[*]"
pub const PROMPT_UNPINNED_LABEL: &str = "[pin]";     // 原 "[○]"
pub const PROMPT_PINNED_LABEL: &str = "[unpin]";     // 原 "[◉]"
```

宽度常量必须同步（`src/layout.rs:34-55`，**只改数值不加行**）：

```rust
const PROMPT_SELECT_ALL_WIDTH: u16 = 5;   // "[all]"
const PROMPT_PIN_WIDTH: u16 = 7;          // "[unpin]" 是最宽态，按最宽算
```

`PROMPT_SELECT_ALL_MIN_WIDTH` / `PROMPT_PIN_MIN_WIDTH_EDITOR` / `PROMPT_PIN_MIN_WIDTH_TREE` /
`PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 由上述常量**引用推导**，无需手改数值。
推导结果（内宽阈值）：

| 常量 | 改前 | 改后 |
| --- | --- | --- |
| `PROMPT_SELECT_ALL_MIN_WIDTH` | 17 | 19 |
| `PROMPT_PIN_MIN_WIDTH_EDITOR` | 21 | 27 |
| `PROMPT_PIN_MIN_WIDTH_TREE` | 14 | 18 |
| `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` | 27 | 33 |

> **为什么按 `[unpin]` 而不是 `[pin]` 算宽度**：同一颗按钮两种状态宽度不同会让
> 相邻按钮在切换时左右跳动。统一取最宽态，`[pin]` 绘制时右补 2 个空格。
>
> ⚠️ **已知副作用**：`[dir]` 的可见阈值从「内宽 27」涨到「内宽 33」，
> 即**面板宽 29~34 时 `[dir]` 会消失**，而改前可见。终端 100 列（侧栏 24）时
> `side_width` = (100−24)×2/5 = 30 列 → 内宽 28，正好落在该区间。
> 这是「图标改英文单词」的直接代价，用户已确认改文案，因此接受；
> 若后续觉得不可接受，唯一干净的解法是把 `[dir]` 也换成更短的词（如 `[ref]`），
> 但那超出本次授权范围。

### 4.11 样式（`src/ui/style.rs`）

新增 3 个 token（**放文件末尾**；文件已 1006 行，若越界则新建 `src/ui/style/todo.rs`
并在 `style.rs` 声明 `mod todo; pub use todo::*;`——与 `text/agent.rs` 的拆分先例一致）：

```rust
/// Todo 待办行状态框：正文色（未开始，不强调）。
pub fn todo_status_open() -> Style;
/// Todo 进行中行状态框与文本：强调色（与 prompt 焦点同色系，一眼可见）。
pub fn todo_status_active() -> Style;
/// Todo 已完成行：muted + 删除线。
pub fn todo_status_done() -> Style;
```

选中行底色仍用 `style::selection()`，不新增 token。

### 4.12 不变量

1. **内存与库同序**：`TodoPanel.items` 始终按 `position` 升序，且 `position == 下标`。
   任何改动后立刻重排为 `0..n`（内存侧用 `Vec` 的天然顺序，库侧由写操作保证）。
   由于这条成立，「行下标」可以直接当「position」用，无需在渲染/命中里换算。
2. **选中合法**：`items.is_empty() ⇒ selected == None`；否则 `selected < items.len()`。
   每次列表变动后调用 `clamp_todo_selection`。
3. **编辑唯一**：`editing.is_some() ⇒ todo_focused`。
4. **焦点互斥**：焦点由 `AppState.focused: FocusedContainer`（`Sidebar`/`Panes`/`Prompt`/`Todo`）
   **单一真源**决定，同一时刻只有一个容器描边强调色、持有硬件光标并接收按键；
   焦点为 `Todo` ⇒ `right_tab == Todo`。（第三轮从两个互不同步的 bool 收敛而来。）
5. **绘制即命中**：行/按钮/滚动条矩形的绘制与命中调用**同一个**几何函数。
6. **渲染只读**：`ui/` 不改状态、不做 IO；所有改动经 `app/update`。
7. **写失败不丢数据**：库写失败只 toast，内存列表不变；下次成功写入自然补齐。

---

## 5. 边界与降级

### 5.1 打断矩阵

| 场景 | 行为 |
| --- | --- |
| 编辑中按 `Esc` | 取消：新建消失、编辑恢复原文，不写库 |
| 编辑中按 `Enter` | 提交：trim 后非空则写库；新建空输入不创建；编辑清空则删除该条 |
| 编辑中点另一行 | 先提交，再选中该行 |
| 编辑中切 tab | 提交 |
| 编辑中切工作区 | 提交 |
| 编辑中按 `Tab` | 切换到下一状态并立即写库；**不**提交、**不**退出编辑（正文草稿保留） |
| 编辑中双击同一行 | 不打断编辑（草稿保留，不重开输入框） |
| 编辑中面板折叠（`Ctrl+B` 之类） | 提交，并清 `todo_focused` |
| 编辑中程序退出 | 提交（`Persistence::shutdown` 前调用 `commit_todo_edit`） |
| 编辑中库写失败 | 保留内存改动、toast `todo save failed`、编辑态已结束 |
| `todo_focused` 时点终端窗格 | 提交编辑 + 失焦（`focus_pane`，`src/app/update/pane.rs:66-72`，补 `todo_focused = false` + `commit_todo_edit`） |

### 5.2 降级矩阵

| 场景 | 表现 |
| --- | --- |
| 库不可用（`db: None`） | 全部功能可用但**不落库**；重启即丢；不弹 toast（与 `prompt_history` 同口径） |
| 迁移失败 | `Database::open` 失败 → 持久化整体禁用（既有行为），Todo 内存可用 |
| 面板高度 ≤ 2 | 只画外框 + tab，不画工具栏与列表（`prompt_header_rect` 返回 None） |
| 面板内宽 < 5（面板宽 < 7） | 工具栏全丢，只画列表 |
| 只剩 1 行可见 | 仍能编辑与选行；滚动条 `visible == 1` 时 thumb 高 1 行、仍可拖 |
| 列表为空 | 首行居中 `no todos yet`；`[edit]`/`[del]`/`[clear done]` 置灰；统计行显示 `total 0 · doing 0 · done 0 0%` |
| 主体区高度 ≤ 2 | 分割线与统计行一并隐藏，列表退回整块（不与表头抢同一行） |
| 条目超长 | 显示时按行宽截断（不加省略号，保持列对齐）；存储上限 500 字符 |
| 条目数 0 且未聚焦 | 仍显示空态文案（不是全空白） |
| 折叠态容器 | 不画 Todo 任何内容（既有 `render_collapsed_strip`） |

### 5.3 视觉验收要点（用户 `cargo run` 人工确认）

1. 工具栏在 40 列宽（内宽 38）下四按钮完整可见；缩到 30 列时只剩三按钮；
   缩到 20 列时只剩 `[new] [edit]`；缩到 14 列时只剩 `[new]`。
2. 三态状态框宽度一致，行文本起始列在三种状态下对齐。
3. 选中行整行有底色，且底色不覆盖状态框与文本各自的颜色。
4. 已完成行删除线清晰、不明显暗到看不清。
5. 滚动条只在溢出时出现，thumb 随滚动移动，拖拽跟手。
6. 编辑时硬件光标与文本齐平，中文输入法候选框位置正确。
7. prompt 工具栏 `[pin]` / `[all]` 与原 `[○]` / `[*]` 的观感对比，窄面板降级顺序是否合理。
8. 底部统计块：与列表**是否肉眼平分**；三根柱子是否等高对齐、轴标签是否居中；
   `total N · doing N · done N N%` 在 40 列下不断行；空列表时统计归零且不画柱子。
   柱高比例是否正确（2 条 vs 1 条应看得出高低差）。
9. 双击行进入编辑：**硬件光标准确落在被编辑的那一行**（含编辑首行、后面还有行的情况），
   输入法候选框跟在该行而不是跑到 lx 输入框；中文/宽字符不出现半格错位；
   `Tab` 切状态时编辑不中断、左侧状态框跟着变。
10. 点 Todo 容器内任意位置（顶边框空白 / 工具栏空白 / 分割线 / 统计块）后边框变强调色，
    并且光标不出现、不改数据。
11. 列表溢出时滚动条 thumb 不得压到统计分割线上。

---

## 6. 测试计划

### 6.1 数据库层（`src/db/todo/tests.rs`，新）

- `save_todo_insert_shifts_positions`：中间插入后 position 为 `0..n` 连续。
- `save_todo_keeps_unique_positions`：连续中间插入不产生重复 position（唯一索引兜底）。
- `save_todo_is_idempotent`：同 id 同值二次写入不新增行、不改 position。
- `save_todo_inserts_missing_row_at_position`：行不存在时按 `position` 插入并顺移。
- `save_todo_updates_existing_without_moving_position`：已存在时只改文本/状态。
- `delete_todo_is_idempotent`：删除不存在的 id 返回 Ok。
- `delete_todos_with_status_reports_removed_records`：返回值可用于撤销。
- `restore_todos_reinserts_at_original_positions`。
- `empty_text_rejected` / `oversized_text_rejected`（500 字符边界，含多字节）。
- `status_roundtrips_all_three_values`。

### 6.2 迁移（`src/db/migration/tests.rs`）

- `v14_database_upgrades_to_v15_with_todo_table`：造 v14 库 → `open` → 断言
  `user_version == 15`、`todo` 表存在且列/索引齐、既有数据未动。
- 既有 `assert_eq!(version, SCHEMA_VERSION)` 断言随版本号自动通过（用的是常量）。

### 6.3 状态与更新层（`src/app/update/todo/tests.rs`，新）

- `space_cycles_todo_status_and_emits_save`：三态循环 + 每步一个 `SaveTodo`。
- `new_todo_on_empty_enter_creates_nothing`。
- `editing_cleared_text_deletes_item`。
- `esc_cancels_edit_restoring_original_text`。
- `switch_tab_commits_edit`、`switch_workspace_commits_edit`。
- `delete_then_undo_restores_position`。
- `clear_done_removes_only_done_and_can_be_undone`。
- `selection_clamped_after_delete`：删最后一条后 `selected` 落到新的末项。
- `write_failure_keeps_items_and_toasts`。
- `tab_in_list_mode_cycles_status`、`tab_while_editing_cycles_status_and_keeps_editing`、
  `tab_while_editing_new_todo_is_swallowed`（第二轮 `Tab` 语义）。
- `click_row_twice_within_window_enters_edit`、`click_different_row_resets_double_click`、
  `click_outside_window_is_single_click`、`double_click_editing_row_does_not_restart_edit`（第二轮双击）。

### 6.4 渲染层（`src/ui/right_panel/todo/tests.rs`，新）

- `toolbar_buttons_drop_from_right_when_narrow`：内宽 38 / 28 / 18 / 12 / 4 五档。
- `list_rows_render_status_boxes_aligned`：三态行首列位置一致。
- `selected_row_uses_selection_style_without_inheriting_border_accent`。
- `done_row_is_muted_and_crossed_out`。
- `empty_list_shows_placeholder`。
- `scrollbar_only_when_overflowing`。
- `edit_row_returns_cursor_position`。
- `stats_block_sits_below_list_without_overlap`、`stats_hidden_when_body_too_short`、
  `scrollbar_track_stops_above_stats`（第二轮统计块几何）。
- `stats_row_shows_counts_and_percent`、`empty_list_still_shows_zero_stats`。
- `edit_row_cursor_uses_display_width`、`edit_row_reuses_input_view_scrolling`、
  `placeholder_only_for_new_todo`（第二轮输入框复用）。

### 6.5 事件层（`src/event/tests.rs`）

- `mouse_click_todo_row_selects_and_focuses`。
- `mouse_click_todo_toolbar_button_runs_action`。
- `mouse_click_todo_blank_still_swallowed`（替换既有 `mouse_click_todo_body_swallowed_...`）。
- `mouse_wheel_over_todo_scrolls_list_without_moving_selection`（替换既有用例）。
- `todo_keys_never_reach_pty`：聚焦 Todo 时按 `a`/`j`/任意字符，断言 PTY 缓冲无字节。
- `paste_into_todo_edit_inserts_text`、`paste_in_todo_list_mode_is_swallowed`。
- `ctrl_q_wins_over_todo_focus`。
- `focus_pane_releases_todo_focus`。
- `ctrl_z_without_undo_is_consumed_not_forwarded`：无删除可撤销时 `Ctrl+Z` 不写终端。
- `mouse_double_click_todo_row_enters_edit`、`mouse_click_todo_stats_row_is_swallowed`（第二轮）。

### 6.6 文案守卫（`src/ui/text/tests.rs`）

- 改写 `prompt_button_icons_are_single_width_and_non_pua`：
  原断言「标签只含一个图标字符」对 `[all]`/`[pin]`/`[unpin]` 不再成立。
  新口径：**要么**恰好一个单宽非 PUA 字符（`[≡]`/`[dir]`/`[←]`/`[tmp]`），
  **要么**全 ASCII 小写单词（`all`/`pin`/`unpin`）。两组分别断言，不允许混进 PUA 或宽字符。
- 新增 `todo_labels_align_to_three_columns`：三个状态框宽度均为 3。
- 新增 `stats_labels_are_lowercase_ascii_words`（第二轮统计文案）。

---

## 7. 文档同步

1. `docs/standards/tui-design-requirements.md:25`（右栏整段）：
   - 删除「Todo 面板为占位：功能按钮行整行空白、标题行为纯 `─`、内容区空白，零占位文案」；
   - 改为描述真实 Todo 面板：工具栏 4 按钮、底部统计块（分割线 + `total/doing/done`）、
     三态状态框、`❯` 选中标记、行内单行编辑（`Tab` 切状态）、行双击进编辑、右侧滚动条、
     空态 `no todos yet`、独立键盘焦点 `todo_focused`（与 `prompt_focused` 互斥）、
     `Ctrl+Z` 单槽删除撤销；
   - 保留「`right_tab` 不持久化、启动恒为 `Prompt`」。
2. 同文件工具栏文案段（若提及 `[*]` / `[○]`）改为 `[all]` / `[pin]` / `[unpin]`。
3. 本目录 `task.md` 维护执行清单与完成记录。

---

## 8. 风险与取舍

| # | 风险 | 处置 |
| --- | --- | --- |
| 1 | 事件闸门分散在 6 个文件，漏一处就出现「点得到但键不到」或「键漏进终端」 | §4.9 给出逐文件落点表；`todo_keys_never_reach_pty` 用 PTY 缓冲断言兜底 |
| 2 | `src/layout.rs` 已 999 行，容易顺手加几何 | Todo 几何全部放 `ui/right_panel/todo.rs`；只改 `layout.rs` 的 4 个常量数值，**零新增行** |
| 3 | `ui/text.rs`(1260) / `ui/style.rs`(1006) / `db/service.rs`(1223) 均已超 1000 行 | 新增文案与样式优先落子模块（`text/todo.rs`、`style/todo.rs`）；服务分支控制在 20 行内，超则抽 `db/todo.rs` 内的函数 |
| 4 | 三态循环与「清空即删」组合出的边界（编辑既清空又被打断） | 判定顺序固定：先 trim → 空则按「新建不建 / 编辑删除」分支 → 非空才写；`Esc` 路径完全不看 trim |
| 5 | 单槽撤销在连续删除时被覆盖 | 明确语义：只保留**最近一次**删除批次；`d` 连按三次只有最后一次可撤销。这是刻意取舍，不引入栈 |
| 6 | 全局列表在「多工作区同开」时没有归属感 | 计数 `3/8` 与空态文案都写英文、不带工作区名，避免暗示归属；文档明确「跟随数据库」 |
| 7 | 500 字符上限对中文不友好（按字符而非字节） | 显式按 `chars().count()` 判定，与 lx-agent 的 `content: 1..500` 一致 |
| 8 | 与 `TemplateStatus::Todo` 语义撞名 | 代码层用 `TodoStatus` 区分；文档与注释都写明「与 markdown 模板块无关」 |

---

## 9. 实施记录（与设计的差异）

实施完成。格式照 `right-panel-tabs/design.md` §9：

| # | 设计写法 | 实际实现 | 原因 |
| --- | --- | --- | --- |
| 1 | `TodoPanel.scroll` 由调用方传 `rows` 钳制（`scroll_todo(dir, rows)` / `set_todo_scroll(offset, rows)`） | 改由 `TodoPanel` 内部持 `rows`（`set_viewport_rows`），`scroll_by(dir)` / `set_scroll(offset)` 不再收参 | `app/` 拿不到布局；每帧由事件循环 `resolve_todo_viewport` 回写可见行数，几何只有渲染层一个来源。同时把「选中滚入视口」收进 `select`/`move_selection`/`clamp_selection`，删掉 6 处重复调用 |
| 2 | `render_edit_row` 编辑既有条目；新建时无行可画 | 新增「幽灵行」：`editing.target == None` 时在 `edit.at` 处补一行渲染输入内容 | 否则新建时用户敲字屏幕一片空白——编辑态的唯一可见输入位置不能靠空白糊过去 |
| 3 | `editing_row(state, index, record)` 按 `edit.at` 猜新建行 | 只按 `target == record.id` 判定；新建走幽灵行分支 | `edit.at` 与既有条目下标会互相误导，混在一个函数里必然出错 |
| 4 | `select_todo_row(index)` 直接 `commit` 再按下标选中 | 先判「点的是不是正在编辑的那一行」（含幽灵行下标），再提交、再按下标判定 | 提交会改变列表长度（幽灵行消失），旧下标在提交后会指向别的条目 |
| 5 | `apply_todo_button` 只看传进来的按钮 | 新增 `todo_button_enabled`，渲染层与点击行为共用 | 「看着亮却点不动」是两处判定分叉的必然结果 |
| 6 | `TodoPanel::restore` 照单全收 | 跳过 id 已在列表里的记录 | 撤销槽一次性，但重复恢复不该造出重复条目；列表不变式优先 |
| 7 | `prompt.rs::render_toolbar` 逐字符写按钮 | 先按矩形宽度整格填空格再写字 | `PROMPT_PIN_WIDTH` 改成最宽态 7 后，`[pin]` 只有 5 列，不补空格会留下上一帧的 `n]` 残影 |
| 8 | `todo` 文案与样式追加到 `text.rs` / `style.rs` 末尾 | 拆成 `ui/text/todo.rs` 与 `ui/style/todo.rs` 子模块 | 两个文件实施前已分别 1260 / 1006 行，直接追加必然越 1000 行硬顶 |
| 9 | 点击 Todo 空白「吞掉」 | 吞掉且**不**失焦 | 点空白不该结束正在进行的编辑；失焦只由 `Esc`/`q`/切面板/切工作区/聚焦别处触发 |
| 10 | 挂 `todo_focused` 后仍需手工处理 `prompt_focused` 互斥 | `focus_todo` / `focus_prompt` / `focus_pane` / `select_right_tab` / 折叠 各自维护 | 互斥是双向的，单点置位不够；已在 §4.12 不变量 4 记录 |
| 11 | 工具栏右端显示完成计数 `done/total` | 计数整体移到列表下方的新统计行（`total N · doing N · done N N%`），工具栏只剩按钮 | 用户第二轮要求「中间加分割线、上方列表下方统计」；两处都显示计数是重复，且工具栏窄时计率先被丢 |
| 12 | 统计行不存在 | 新增 `stats_divider_rect` / `stats_rect`（`layout.rs::prompt_body_rect` 就地公开）+ `prompt::render_rule(panel, y, …)` 复用同一条 `├ ─ ┤` | 样式与容器其他分割线必须同源，否则焦点态配色会分叉；`prompt_body_rect` 本来就在 `layout.rs`，只需去掉私有 |
| 13 | 列表区 = `layout::prompt_text_rect`（整块主体区） | 列表区 = 主体区 − 统计块 2 行；滚动条轨道改用列表矩形推算 | 列表若仍占整块，统计行会落在列表区内部、被行命中吞成「点空白」；必须让几何先扣干净 |
| 14 | 列表态 `Tab` 吞掉 | 列表态与编辑态 `Tab` 都循环状态，编辑态切换后立即登记写库 | 用户第二轮要求「支持 tab 切换状态」，对齐 lx-agent 行内编辑的 `Tab` 语义（`Enter` 提交、`Esc` 取消、`Tab` 切状态） |
| 15 | 行只有单击选中 | 新增 `click_todo_row`：同一次点击内先判双击（`todo_row_click`，窗口复用 `PROMPT_DOUBLE_CLICK_WINDOW`）再选中，双击进入行内编辑 | 双击必须与选中在同一入口完成：选中会推进视口但**不改行下标**，拆成两次调用会让「第二次点击的 index」与「第一次的 index」对不上 |
| 15b | 点击行按显示下标直接选中 | 先按显示下标锚定**条目 id**，提交后按 id 找回新位置；幽灵行被提交的那一击同时清空 `todo_row_click` | 幽灵行占一个显示槽位，它下方的行在列表里前移一位——按行号选中会点 b 选中 c（第一轮遗留缺陷，第二轮补测试时暴露）。提交还让屏上内容整体上移，同一套「双击」第二击会落到别的条目，必须作废序列 |
| 16 | `render_edit_row` 自写按字符数滚动的视口 | 复用 `widgets::input::view`（按显示宽度滚动、宽字符占 2 列），空输入显示 muted 占位 `new todo`（仅新建态） | 用户第二轮要求「输入框光标逻辑复用其他输入框」；自写版把 CJK 光标放在字符边界上，是真实缺陷 |
| 17 | 编辑行无占位 | 新建幽灵行空输入时画 `new todo`；编辑既有条目清空时不画 | 清空既有条目意味着「提交即删除」，此时画「new todo」是指引错误 |
| 18 | 统计行恒显示 | 列表为空也显示 `total 0 · doing 0 · done 0 0%` | 统计是版面固定部分，随列表消失会让面板高度跳变 |
| 19 | 统计块固定 2 行（分割线 + 一行摘要） | 统计块与列表**平分**主体区，块内三段：摘要 / 三态竖向柱图 / 轴标签；主体区 < 7 时不切分 | 用户第三轮要求「统计与列表平分高度，并加字符图表」；固定 2 行让统计在长面板里几乎不可见 |
| 20 | 点分割线 / 统计块 / 空白「吞掉，不聚焦」 | 改为**让容器进入选中态**（`focus_todo_from_click`），顶边框空白行也覆盖 | 用户第三轮口径：点右侧容器就该高亮，不该"只有点进输入框才算选中"；焦点高亮本应由边框表达 |
| 21 | 编辑行光标恒交出 | 仅当 `todo_focused` 为真才交出硬件光标 | 真机缺陷：未聚焦时上层把它当输入位置，IME 画在没有输入框的 Todo 行上 |
| 22 | `render_rows` 逐行 `cursor = render_row(…)` | 改为 `cursor = cursor.or(render_row(…))` 累积 | 真机缺陷：编辑行不在最后一行时，后面的「无光标」行把已拿到的光标清成 `None`，上层回落到窗格光标——IME 跑到 lx 输入框（用户报告的 bug） |
| 23 | 焦点是两个独立 bool（`prompt_focused` / `todo_focused`），各容器自己判 | 收敛成单一枚举 `FocusedContainer`（`Sidebar`/`Panes`/`Prompt`/`Todo`）+ `set_focus` / `prompt_focused()` 等读侧谓词 | 真机缺陷：`panes.rs` 只看 `!prompt_focused`、侧栏边框恒 `false`，于是点右侧容器后主区**也**描边强调色（截图里两个容器同时高亮），且光标归属分裂。两个 bool 无法表达"同一时刻只有一个" |
| 24 | 侧栏没有任何焦点概念、点侧栏空白不产生状态 | 新增 `FocusSidebar` 分支 + `focus_sidebar()`；侧栏边框随焦点描边，Agents 条目 `❯` 标记改由 `panes_focused()` 决定 | 用户口径：点哪个容器就高亮哪个容器。侧栏持焦点时 `route` 直接返回 `None`，按键绝不落进别的容器输入框 |
| 25 | `route(key, mode, prompt_focused, overlay, pane_lx, todo_focused)` | 改为 `route(key, mode, focus: FocusedContainer, overlay, pane_lx)` | 五个入场状态里两个是互斥布尔，调用方稍一漏改就分裂；换成枚举后「不可能同时为真」由类型保证 |
| 26 | `select_right_tab` 切 tab 后把焦点退回主区 | 改为**直接聚焦到目标面板**（Prompt → `FocusedContainer::Prompt`，Todo → `Todo`） | 真机缺陷：点右侧容器的 tab 后高亮跑回主区。点 tab 本身就是「点了右侧容器」，焦点必须留在右栏 |
| 27 | 列表态 `Enter` = 在当前项之后新建 | `Enter` 编辑当前项（空列表退化为新建），`Shift+Enter` 才是其后新建；编辑态两者都提交 | 用户第三轮口径（对齐 lx-agent「点行即改」）。单行输入没有换行语义，编辑态不必区分 |
| 28 | Todo 工具栏单组从右往左排，`[new]` 宽 5 | 分**左右两组**：左端恒为 `[+]`（宽 3、贴左边框），右端贴右缘依次 `[clear done]` `[del]` `[edit]`，从右往左丢弃；两组间至少 1 列空白 | 用户口径：设置左右都有、添加按钮用加号。`[+]` 恒在意味着「新建」永远不会被窄面板丢掉 |
| 29 | 工具栏标签一律左对齐 | prompt 工具栏标签改为**槽内右对齐**（其余按钮槽宽 == 标签宽，效果不变） | `[pin]` 固定占 `[unpin]` 的 7 列、自身只有 5 列，左对齐把 2 列余量甩到右侧，加 1 列间距就是用户看到的「pin 与 all 之间大间隙」 |

