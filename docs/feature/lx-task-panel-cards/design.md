# 后台任务卡片橙色态与输出迁移到任务面板 — 设计

- 分支：`feat/lx-agent-command-plan-tools`（worktree `.worktrees/lx-agent-command-plan-tools`，就地修改）
- 状态：需求边界已确认（Grill Me 11 轮收口完成），实现与测试完成

## 1. 背景与目标

两个用户可见问题：

1. **运行中的后台任务卡片是绿的、且会定格成镂空**：截图里 `□ Run command · Running` 是空心方块，
   看起来像失败/取消，但进程其实还在后台跑。
2. **后台任务输出挤在消息列表里**：`Run command` 卡片直接展开命令、cwd、3 行输出和
   `… +29 more lines`，把历史列表撑得很长，而真正的任务管理面板反而没有输出。

目标：

- 进程类卡片在**进程存活**期间整体变橙，停止后恢复绿色；
- 运行中保留 `■`/`□` 闪烁，但**动画停止时恒为实心**，杜绝定格镂空；
- 消息列表里卡片收敛成 3 行（标题 / 命令 / `[open]`），不再输出任务内容；
- 点 `[open]` 在**原消息列表矩形**里打开任务面板，展示任务列表 + 高亮任务的输出；
- 关闭面板后消息列表的位置与内容原样恢复。

## 2. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 存活色范围 | 仅进程类卡片（`exec_command` / `write_stdin`）；判据是**进程是否存活**（`ProcessStateView::is_running()`），不是工具结算状态；颜色为**深黄 `Indexed 136`**（用户后续要求"深一点的黄色"），方块 / 动词 / 状态文字 / **命令行原文**全部染此色；进程停止后恢复绿与参数金黄 |
| 2 | 空心方块 | 保留 `■`/`□` 闪烁；**动画不推进时恒画实心 `■`**；空心从此只归 `Cancelled` |
| 3 | 兜底位置 | 字形函数 `StepMark::glyph()` 增加"动画是否推进"入参，渲染层一行派生（`lx.chat.status.streaming`），状态层不归零 |
| 4 | 面板区域（**两段式**） | **输出容器**占**原消息列表矩形**；**item 列表**仍在**原输入框矩形**（固定高度）；两块同时可见；关闭后消息列表逐字符恢复 |
| 5 | 卡片内容 | 不再画任何输出行，也不再有 `… +N more lines`；只留命令 |
| 6 | `[open]` 位置 | 独占末行 `  └─ [open]`，灰色 `style::agent_step_detail()` |
| 7 | 可点区域 | 只有 `[open]` 行可点；删除卡片"展开/收起"语义（`ToolDetail` 切换不再作用于进程卡片） |
| 8 | 面板内容 | 上（原消息列表位置）：高亮任务的**输出容器**；下（原输入框位置）：Processes item 列表（`❯` 高亮 + `^d stop`） |
| 9 | 高亮定位 | `[open]` 携带进程 `session_id`，打开时高亮强制落到该任务；进程已结束则回落列表首项 |
| 10 | 滚动保持 | 面板期间冻结 `history_scroll` / `follow_bottom`；关闭后按原语义恢复 |
| 11 | 键位 | 沿用 `↑/↓` / `←/→` / `^d` / `esc`；`PageUp`/`PageDown` 改为翻**输出区**；高亮切换自动贴输出尾部 |

## 3. 实现要点

### 3.1 橙色（决策 1）

- `src/ui/style.rs` 新增橙色语义 token（256 色槽，与翡翠绿 / 品红 / 金黄保持可辨色差）：
  `chat_step_mark_alive()`（方块，加粗）、`chat_step_label_alive()`（动词与状态文字）。
- `history/tool/command.rs`：`draw_command` / `draw_interaction` 里进程存活时
  （`view.state.is_running()`）改用 `StepMark::Alive`，标题整体染橙；
  进程到终态后回到 `StepMark::Done`（绿）。文件工具卡片不受影响。
- `StepMark` 增加 `Alive` 变体：字形与 `Running` 同（实心/空心闪烁），颜色取橙色。

### 3.2 镂空兜底（决策 2、3）

- `StepMark::glyph(loading_frame, animating)`：`Running | Alive` 仅在 `animating` 为真时
  按 `loading_frame` 相位交替 `■`/`□`，否则恒 `■`。
- 调用点统一传 `lx.chat.status.streaming`（已存在的公开状态），渲染层一行派生。
- `Cancelled` 仍恒空心（语义就是"没产出结果"）。

### 3.3 卡片收敛为 3 行（决策 5、6、7）

- `command_body()` 收敛：`[命令行, open 行]`，删除 `output_rows` / `cwd` / `dropped_bytes` /
  `toggle_row` 在折叠态的产出；`rows()` 恒为 `1 + 2`（宽度 0 时为 0）。
- `[open]` 行使用 `text::agent::TASK_OPEN_LABEL`（新增文案 `[open]`），样式
  `style::agent_step_detail()`。
- 命中：`history::layout` 只给 `[open]` 行登记 `ToolHit`，并携带进程 `session_id`；
  标题行不再登记。`tool::toggle_offset` 概念被 `open_offset` 取代。

### 3.4 面板接管历史区（决策 4、8、9、10、11）

- `lx.rs::layout`：`lx.task` 有值时把**历史区**交给面板，输入框仍按原逻辑计算并占位；
  面板矩形 = 历史区矩形（`history_rect` 同一份几何）。
- `TaskPage` 扩展：
  - `focus: Option<u64>`（`[open]` 带来的目标 session_id；落地后清空）；
  - `output: Vec<String>` + `output_scroll: usize`（高亮任务的输出尾部快照与视口）。
- 面板绘制：顶边框 → tab 行（沿用）→ 列表区（沿用 `draw_processes`）→ 分隔线 → 输出区。
- 事件层：`open_task_page` 增加可选 `session_id`；进程快照回调同时写入高亮任务的输出。
- `PageUp`/`PageDown`：面板内改翻输出区（`task_history_delta` → `task_output_delta`）。
- `resolve_task_history_visible` 保留（历史区矩形变了但总/可见行数仍需落状态）。

### 3.5 滚动保持（决策 10）

- 面板打开期间不写 `history_scroll` / `follow_bottom`；关闭时不重置、由既有钳制逻辑收口。

## 4. 影响与不变量

- 文件工具卡片（read / search / edit / write）与计划卡片版式不变。
- 进程卡片行数恒为 3（宽度 0 时为 0），`history::layout` / 渲染 / 命中继续共用同一份
  `rows()` 口径。
- `LX_TASK_*` 文案、底边框键位提示、`^d stop` 语义不变。
- 面板不覆盖窗格边框；`app/` 仍不持有进程句柄。

## 5. 验证

- `src/ui/main_content/lx/history/tool/tests/command_cards.rs`：3 行结构、`[open]` 灰、无输出行、
  橙色仅存活期、动画停止后实心。
- `src/ui/main_content/lx/task/tests.rs`：接管历史区、列表 + 输出区、`[open]` 定位高亮。
- `src/ui/main_content/lx/tests.rs`：面板打开时输入框仍在原位、历史矩形被面板占用。
- `src/app/update/**`：`open_task_page` 带 session_id、进程已结束时回落首项、面板期间历史滚动冻结。
- `src/input/process_keys.rs`：`PageUp`/`PageDown` 路由。
- 全量 `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt`；
  TUI 实际观感由用户 `cargo run` 验收。
