# 后台任务卡片橙色态与输出迁移到任务面板 — 设计

- 分支：`feat/lx-agent-command-plan-tools`（已合并入 `dev`；原 worktree 已删除）
- 状态：已实现、已合并。**本文档记录的是最初的 11 轮盘问结论；其中配色与面板形态在后续
  五轮用户验收反馈中被推翻并改定，最终结论以 §2 的「现行」列与 §6 为准。**
- 总设计见 [`lx-agent-command-plan-tools/design.md`](../lx-agent-command-plan-tools/design.md)，
  缺陷清单见 [`lx-agent-command-plan-tools/review.md`](../lx-agent-command-plan-tools/review.md)。

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
| 1 | 存活色范围 | ~~仅进程类卡片；深黄 `Indexed 136`；命令行原文也染此色~~ → **现行**：**所有**运行中工具（文件工具 / `exec_command` / `write_stdin` / `update_plan`）统一**橙黄 `Indexed 208`**，方块 / 动词 / 状态文字同档；**参数（命令行原文）恒为金黄、不随运行态变化**；`Completed` 才回翡翠绿 |
| 2 | 空心方块 | 保留 `■`/`□` 闪烁；**动画不推进时恒画实心 `■`**；空心从此只归 `Cancelled` |
| 3 | 兜底位置 | 字形函数 `StepMark::glyph()` 增加"动画是否推进"入参，渲染层一行派生（`lx.chat.status.streaming`），状态层不归零 |
| 4 | 面板区域（**两段式**） | **输出容器**占**原消息列表矩形**；**item 列表**仍在**原输入框矩形**（固定高度）；两块同时可见；关闭后消息列表逐字符恢复。**现行补充**：输出容器**不画任何边框/横线**（顶边栏与末行分隔线已按用户要求删除），最右一列**常驻**预留滚动条槽 |
| 5 | 卡片内容 | 不再画任何输出行，也不再有 `… +N more lines`；只留命令 |
| 6 | `[open]` 位置 | 独占末行 `  └─ [open]`，灰色 `style::agent_step_detail()` |
| 7 | 可点区域 | 只有 `[open]` 行可点；删除卡片"展开/收起"语义（`ToolDetail` 切换不再作用于进程卡片） |
| 8 | 面板内容 | 上（原消息列表位置）：高亮任务的**输出容器**；下（原输入框位置）：Processes item 列表（`❯` 高亮 + `^d stop`） |
| 9 | 高亮定位 | `[open]` 携带进程 `session_id`，打开时高亮强制落到该任务；进程已结束则回落列表首项 |
| 10 | 滚动保持 | 面板期间冻结 `history_scroll` / `follow_bottom`；关闭后按原语义恢复 |
| 11 | 键位 | 沿用 `↑/↓` / `←/→` / `^d` / `esc`；`PageUp`/`PageDown` 改为翻**输出区**；高亮切换自动贴输出尾部。**现行补充**：`^d` 只由 Ctrl+D 触发（裸 `Delete` 不再误杀进程）；滚轮可滚输出区与 item 列表；**刷新输出不改变滚动位置**，只有切任务/打开定位才贴尾 |

## 3. 实现要点

### 3.1 运行态橙黄（决策 1，已被后续轮次修订）

- `src/ui/style.rs` 新增 `CHAT_STEP_AMBER = Indexed 208`（`#ff8700`）：`chat_step_mark_alive()`
  （方块，加粗）、`chat_step_label_alive()`（动词与状态文字）、`chat_tool_verb_running()`
  （卡片 title 主体）。选色依据（CIEDE2000）见 `ui::style::tests`。
- `StepKind::Tool` 的 `InputStreaming` / `Pending` / `Running` **统一**取橙黄，
  `Completed` 才回翡翠绿——不再按工具种类分色（文件工具运行中同样是橙黄）。
- `StepMark` 的 `Alive` 与 `Running` 视觉统一，字形一致。
- **参数不跟随**：命令行原文等参数恒为 `chat_tool_param()`（金黄 220）。用户明确要求
  「参数颜色不要改变，颜色变化始终是小卡片和 title」。

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
- 面板绘制：**输出容器**（无边框，正文 + 常驻滚动条槽）→ **item 列表**
  （自身的顶边框 → tab 行 → 列表区）。**没有**连接两块之间的分隔线：列表的顶边框即是天然分界。
- 事件层：`open_task_page` 增加可选 `session_id`；进程快照回调同时写入高亮任务的输出。
- `PageUp`/`PageDown`：面板内改翻输出区（`task_history_delta` → `task_output_delta`）。
- `resolve_task_output_visible` 只落**输出可见行数**（历史行数在面板打开时无人消费，
  早先每帧算一遍全量 markdown 排版纯属浪费）。

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

## 6. 后续修订（用户验收驱动的最终形态）

最初 11 轮盘问得到的方案上线后，用户在 `cargo run` 验收中逐轮提出修改。以下为**最终结论**，
与 §2/§3 冲突时以本节为准。

### 6.1 配色：从「进程专属深黄」到「全工具橙黄 + 参数恒定」

| 轮次 | 用户反馈 | 结论 |
|---|---|---|
| 1 | 运行中卡片是绿的，且定格成镂空 `□`（像失败） | 存活态改橙；动画停止恒画实心 `■`，空心只归 `Cancelled` |
| 2 | 橙偏暗；文件工具运行中仍是绿的 | 改**橙黄 `Indexed 208`**；**所有**运行中工具统一橙黄，`Completed` 才绿；动词（title）与方块同色 |
| 3 | 参数不该跟着变色 | 命令行原文恢复**恒定参数金黄**；状态只由标题行的方块 + 动词表达 |
| 4 | 计划里「进行中」的步骤是完成态绿 | `PlanStepView::InProgress` 改用 `chat_tool_verb_running()`，标记也按状态取色 |

`Indexed 208` 的选色依据（CIEDE2000）：与金黄参数 220 差 27.9、工具绿 42 差 56.0、
品红 218 差 40.7、天蓝 117 差 50.9——既"橙黄一点"，又不会与既有任何结构色混淆。

### 6.2 面板：去掉两条横线 + 输出可滚动

- **删除输出容器的顶边栏**（`Output <命令>` + 右端 `[close esc]`）与**末行分隔线**：第一行
  直接是日志正文，与下方 item 列表之间由列表自己的顶边框分界。`[close esc]` 随之移除，
  `esc` 键仍可关闭（列表 tab 行右端也保留一个）。
- **输出容器可滚动**：最右一列**常驻**预留滚动条槽（与 prompt 编辑器同规则——出现/消失不改变
  日志宽度，避免刚溢出那一刻整块日志横向抖一格），溢出时只画 thumb `▐`；支持滚轮（每次 3 行）、
  轨道点击跳转、thumb 拖拽。
- **刷新输出不改变滚动位置**：只在原本贴尾时继续跟随最新；用户手动上翻过就保持阅读位置，
  只做越界钳制。「贴尾」只发生在真正切换目标的时刻（`[open]` 定位、`↑/↓` 换任务）。

### 6.3 交互：面板打开时的「失效面」

面板打开后，输出容器与历史区是**同一块矩形**，但历史根本没画。所有基于历史几何的入口都必须
一起失效，否则输出区会成为巨大的点击穿透面。这条规则收在 `lx::chat_geometry_is_live()`：

- 历史命中（置顶行跳转 / thinking 折叠 / 代码块复制 / `[open]`）；
- 历史滚动条与历史滚轮区；
- 聊天栏（loading 行 / `↓ jump to bottom`）——它服务的是消息列表；
- 入口标签（agent 切换区）——面板接管了那块区域。

命中与渲染必须**成对失效**：这是本特性反复出 bug 的根因，已在
`docs/standards/tui-design-requirements.md` 里写成硬性规范。

### 6.4 其他修正

- `^d` 只由 **Ctrl+D** 触发；裸 `Delete` 在面板里不再等价于「停止进程」（原先会误杀服务）。
- 切 tab 往返不丢进程高亮（窗口状态按进程列表长度钳制，而非当前 tab 的条目数）。
- `[open]` / `^o` 的定位请求要等列表回投后才落地（早先在列表为空时就 `take()` 掉了）。
- 面板打开期间不再每轮置脏（原先空闲也按 60fps 重绘并反复克隆全部进程快照）。
