# lx 输入框上方动态任务文案：设计草案

> 状态：**已实现并合并到 dev**（2026-10-24，合并提交 `3fa9e97`）；自动验证通过，人工终端观感验收仍待用户执行。
> 日期：2026-10-23。
> 项目：`/Users/yonah/projects/agent/lx-tui`。
> 配套执行清单：[task.md](./task.md)。
> 实施门槛：用户已于 2026-10-23 批准执行；实际改动与验证结果见 [task.md](./task.md) §13，合并回写见 §15。

## 1. 目标与范围

将 lx 页输入框上方左侧目前固定的 `Working...` 改为动态任务/阶段文字。执行计划时展示当前 `in_progress` 步骤；没有执行中步骤时，按真实阶段展示 `Thinking...`、`Running tools...` 或 `Working...`。沿用现有扫描高亮 loading 动画。

参考图片：`/Users/yonah/Desktop/PixPin_2026-10-23_09-44-32.png`。借鉴其输入区域附近的短任务提示形式；具体落位以用户指定的 lx 输入框上方左侧 Working 位置为准。

本次只修改该行的信息选择和空间分配。不新增计划执行器、模型工具、事件协议、数据库字段、设置项或随机文案词库。既有计划卡片仍负责展示完整计划，状态栏只提供当前任务的简短提示。

完成标准：用户在同一位置能读到当前 todo 或可信的执行阶段；长 todo 不挤掉耗时/token 统计，中文字不越界，loading 动画持续运行。

## 2. 已确认决策及确认记录

| 编号 | 决策 | 用户确认依据 |
| --- | --- | --- |
| D1 | 当前 todo 只取 `in_progress` 项。`pending` 不作为当前任务。 | 问题 1，用户回答“是的”。 |
| D2 | 无执行中 todo 时按实际阶段切换文案，不使用随机词库。 | 问题 2，用户回答“使用推荐”。 |
| D3 | 无法可靠判断阶段时显示 `Working...`。 | 问题 2 推荐方案的一部分。 |
| D4 | 动态文字位于输入框上方原 Working 左侧位置，继续使用 loading 动画。 | 原始需求及问题 3 的展示提案。 |
| D5 | 长文字允许截断或隐藏；耗时/token 统计不能因文字过长而隐藏。 | 用户纠正：“支持隐藏长文字，的耗时/token 统计不要隐藏”。本条覆盖此前“先隐藏统计”的推荐。 |
| D6 | 保留会话级当前计划，普通新轮次继续显示旧计划尚未完成的 `in_progress`。 | 问题 4，用户回答“使用推荐”。 |
| D7 | 生成结束时 loading 状态栏照常消失；未跟随底部时仍可显示既有回到底部入口。 | 问题 4 推荐方案及现有栏位职责。 |
| D8 | 先写详细 design/task，审核通过再实施。 | 原始开发流程要求。 |

以下工程细节是本设计提出的可审核方案，不冒充已经单独确认的用户回答：单行净化、极窄屏降级、Esc 待命和统计的共同布局、阶段判定顺序、测试组织。审核本文件时一并确认。

## 3. 代码检索、项目规范与证据

已读取根 `AGENTS.md` 及其指定的目录、代码、TUI、HTML 资源规范。本任务不涉及 HTML 资源。

使用了两套检索 MCP：

- codegraph：通过 `codegraph_explore` 获取当前源码，核对 `PlanSnapshot`、`chat_bar::draw`、`draw_loading`、`draw_scanned_label`、`reasoning_open` 和统计格式化逻辑。
- codebase-memory：先调用 `list_projects` 确认主 checkout 已索引；使用 `search_graph`、`search_code`、`get_code_snippet`、`trace_path` 追踪入口和数据关系。项目标识为 `Users-yonah-projects-agent-lx-tui`。
- 本仓库两套索引均已存在，未重复初始化；未创建 worktree，未修改 Git 分支、索引或提交。
- 已对下表源码路径检查 coverage。除 `session.rs` 存在部分解析覆盖、相关恢复段另行直接核对外，相关实现文件返回无已记录覆盖问题及元数据匹配。覆盖信号只是辅助证据，不保证完整。

下列行号是设计研究时的位置，实施前以当前符号和源码为准。

| 路径 / 关键符号 | 已核实事实 | 设计含义 |
| --- | --- | --- |
| `src/chat/plan.rs:57`，`PlanStep` / `PlanSnapshot` | 步骤保存原文和 Pending/InProgress/Completed；合法计划最多一个 InProgress。 | 复用现有状态，不按列表首项猜当前任务。 |
| `src/event/chat/tool_calls.rs:173`，`finish_tool` | 工具结果持久化成功后才提交计划视图。 | 读取已提交视图，不读取流中的 update_plan 参数。 |
| `src/app/update/chat/process.rs:442`，`commit_plan` | 当前快照转换为 `PlanSnapshotView` 写入 `lx.chat.plan`。 | 该字段是当前计划的唯一来源。 |
| `src/app/update/chat/process.rs:459`，`restore_plan` | 按消息时间线恢复最后一次成功 update_plan。 | 不按最大 revision 选计划，revision 会在新轮重新计数。 |
| `src/app/state/lx/chat/tool_view.rs:205`，`PlanSnapshotView` | 包含 `steps: Vec<(String, PlanStepView)>`。 | UI 直接读取类型化步骤，无须重新解析 result_json。 |
| `src/app/state/lx/chat.rs:403`，`LxChat.plan` | 当前计划属于会话，普通新轮不会清空。 | 保留跨轮继续的语义，符合 D6。 |
| `src/app/state/lx/chat.rs:204`，`RequestState` | 区分 Idle/Committing/Streaming/Finishing。Streaming 持有 assistant_id。 | 阶段推导严格定位当前 assistant。 |
| `src/app/state/lx/chat.rs:735`，`streaming_message` | 非 Streaming 时有 messages.last() 回退。 | 阶段判定不能无条件复用该方法。 |
| `src/app/state/lx/chat/steps.rs:34`，`reasoning_open` | Reasoning 且 completed_at_ms 为 None 才是开放思考块。 | 不把历史思考文字误当成正在思考。 |
| `src/app/state/lx/chat/steps.rs`，`ToolStatus` | 有 InputStreaming/Pending/Running 和多个终态。 | 参数输入不等于执行；只对 Pending/Running 显示 Running tools。 |
| `src/app/state/lx/chat/stream.rs:115`，`start_tool_input` | 新工具输入及后续正文会关闭开放 reasoning。 | 最新有序步骤可反映阶段切换。 |
| `src/app/update/chat/session.rs:357`，`on_session_loaded_full` | 加载会话先清空计划，再恢复该会话计划。 | 沿用隔离逻辑，不建跨会话缓存。 |
| `src/ui/main_content/lx/chat_bar.rs:40`，`visible` | 后台任务面板打开时不显示；有历史且 streaming 或未跟随底部才显示。 | 不扩展栏位的显示生命周期。 |
| 同文件 `rect:51` / `draw:86` | 状态栏贴输入框上方一行，loading 优先于回到底部。 | 保持位置与高度。 |
| 同文件 `draw_loading:145` | 当前固定 Working；普通统计放不下整段隐藏；Esc 待命时也隐藏统计。 | 需要改文案输入和宽度预算，不能只替换常量。 |
| 同文件 `draw_scanned_label:187` | 已接收任意 `&str`，逐字扫描；先画字再检查右界。 | 可复用动画，但要先检查显示列宽再落笔。 |
| `src/ui/text/chat.rs:15`，`stream_stats_label` | 统计为秒级耗时与输出方向 `↓... tok`，数据皆无时返回 None。 | 保持口径，不添加输入 token 或虚假 0 值。 |
| `src/ui/text.rs:1016`，`truncate_to_width` | 按显示列宽截断并添加 `…`。 | 复用现有宽度裁切能力，不另造 ASCII 截断器。 |
| `src/app/update/chat/mod.rs:823`，`advance_lx_loading` | 既有 tick 推进 loading_frame、耗时与 Esc 超时。 | 不增加线程、计时器或 animation 状态。 |
| `src/ui/main_content/lx/tests.rs` | 现有总测试文件已超过 1000 行。 | 新测试进入组件自己的 tests.rs，不继续堆大文件。 |

## 4. 当前数据结构及数据流

现有关键数据形状，以下摘录只保留与本任务有关的字段：

```rust
pub struct PlanSnapshotView {
    pub revision: u64,
    pub explanation: Option<String>,
    pub steps: Vec<(String, PlanStepView)>,
}

pub enum RequestState {
    Idle,
    Committing { generation: u64, draft: String },
    Streaming { key: RequestKey, assistant_id: String },
    Finishing { key: RequestKey, reason: StopReason },
}

pub fn reasoning_open(&self) -> bool {
    self.kind == PartKind::Reasoning && self.completed_at_ms.is_none()
}
```

计划流向：

```text
模型 update_plan
  → 参数校验（最多一个 in_progress）
  → 工具结果成功落库
  → finish_tool / commit_plan
  → 当前窗格 LxChat.plan
  → 只读选择 in_progress 步骤
  → 单行净化 / 宽度裁切
  → 原 chat_bar 扫描动画
```

阶段流向：

```text
当前窗格 RequestState::Streaming.assistant_id
  → 按 id 且 role == assistant 定位消息
  → 当前消息的有序 steps
  → Pending/Running tool 或最后开放 reasoning
  → 阶段枚举
  → ui::text::chat 集中文案
```

保留一份 canonical plan。不增加 current_todo 字符串缓存，不给 UI 另存 revision，不改计划持久化或恢复顺序。由渲染时的只读派生保证计划更新后下一帧自然变化。

## 5. 文案选择规则

### 5.1 显示优先级

先沿用 `visible` 和 `status.streaming` 判断是否需要 loading；停止待命时仍使用停止提示。普通运行时按以下顺序选择：

| 优先级 | 条件 | 显示 |
| --- | --- | --- |
| 1 | 当前会话计划中存在 InProgress | 该步骤的文字，不额外添加 Working 前缀。 |
| 2 | 无当前 todo，当前 Streaming assistant 存在 Pending 或 Running 工具 | `Running tools...` |
| 3 | 无上述条件，当前 assistant 的最后一步是开放 reasoning part | `Thinking...` |
| 4 | 其他状态、阶段不明确、尚无消息、Committing 或 Finishing | `Working...` |

工具 Pending 表示已进入工具执行调度，沿用 Running tools 这一用户文案；InputStreaming 只是在接收参数，不能单凭它显示 Running tools。Completed/Failed/Cancelled/Interrupted 等终态不再表示正在运行工具。

只读当前 assistant 的 steps，不因其他历史消息有 Running 工具而显示运行阶段。若出现异常遗留的旧开放 reasoning，必须同时满足“当前 assistant 的最后一步”才显示 Thinking。

停止待命文案属于输入控制反馈，优先于 todo 和阶段；3 秒待命超时后恢复当前 todo/阶段，不改变原双 Esc 停止语义。

### 5.2 会话、轮次与失败边界

- 会话中存在未完成计划时，即使新一轮正在思考或执行工具，仍优先显示当前 todo；这是用户确认的摘要优先策略。
- 没有计划、计划已清空、计划全部 Completed，或只有 Pending 时，回退实际阶段，不显示 explanation 或第一个 Pending。
- 新轮次不清除计划。切换/新建会话、重载历史继续沿用现有恢复流程；不额外增加清理事件。
- update_plan 失败、未通过持久化或参数仍在输入时，保留最后成功快照。
- 计划步骤异常地有多个 InProgress 时，UI 按原计划顺序选第一个，不崩溃、不修改状态；生产参数校验仍负责拒绝非法计划。
- 生成结束/失败/取消后不留下 loading todo；若回到底部控件需要显示，其行为照旧。
- 本地 shell 可在模型生成期间追加消息，因此禁止用 messages.last() 代替 assistant_id。单独本地命令运行不新增模型 loading 行。

### 5.3 文案边界

计划原文由模型提供，不能当成终端控制序列或 Markdown 执行。显示层将换行、Tab、CR 及其他控制字符当作空白处理，连续空白合并为一个普通空格并 trim。仅净化当前展示副本，不修改计划、工具结果或数据库。

动态 todo 可以包含中文、英文、数字、路径和普通符号。不解析 Markdown，不增加引号、不追加第二份省略号或固定 `...`。实际裁切时才添加 `…`。

若异常历史数据净化后没有可见内容，使用 `Working...`，避免整行只有动画空格。零宽字符不得单独占一个显示列，也不得把光标推进到错误位置；按现有 Grid 能力处理或忽略不支持的零宽字符，不能擅自按宽度 1 绘制。

## 6. 布局、统计与动画

### 6.1 正常布局

仍占输入框上方一行，左侧起点及外侧空档保持现状。短状态文字后紧跟一列空格及静态统计：

```text
 修复输入框布局 (3s · ↓320 tok)
────────────────────────────────────────
> 用户输入
────────────────────────────────────────
```

无当前 todo 时同样的位置可显示：

```text
 Thinking... (3s · ↓320 tok)
 Running tools... (3s · ↓320 tok)
 Working... (3s · ↓320 tok)
```

示例中的数字只说明排版，不表示新增统计项。原有输出 token 估算、秒级耗时及 None 语义不变。不增加彩色标签背景，不把文案移到参考图右上角。

### 6.2 统计优先的宽度预算

不能继续采用“完整文字先画，统计剩余放不下就丢弃”的当前算法。

设去掉左右既有空档后的可用列数为 W，统计完整显示宽度为 S，状态与统计间隔为 G=1：

1. 先生成现有 `stream_stats_label`，统计有值时预留 S。
2. 若 `W >= S`，统计完整保留；文字预算为 `W - S - G` 的饱和减法结果。文字预算为 0 时不画文字、不画间隔。
3. 状态按预算省略，中文按显示列宽计算，不按字节或 char 数计算。预算为 1 且需要截断时只画 `…`。
4. 短文案不强制填满预算，统计仍紧贴实际文字尾部，不无故右对齐。
5. 当完整文字太长时，优先截断文字；空间只够统计时，完全隐藏文字并保留统计。
6. 没有统计数据时全部可用宽度用于文字；没有数据不等于被布局隐藏，不能捏造 `(0s · ↓0 tok)`。

例如 W=25、统计 `(3s · ↓320 tok)` 占 15 列，则状态最多有 9 列，长中文可显示为 `修复输入… (3s · ↓320 tok)`。若文字预算只剩 8 列，就只能显示 `修复输…`；双宽字符不能拆半，剩余一列不用于插入半个汉字。

### 6.3 物理极窄窗口的明确降级提案

“统计不能因长文字隐藏”能在 W>=S 时严格保证；W<S 时连统计自身都放不下，必须明确物理边界：

- 普通运行：状态文字先完全隐藏；统计占用所有可用列，按列安全裁切，不整段主动丢弃，不换行、不覆盖输入框。
- 此分支不引入新单位、第二套 token 缩写或假数字。统计自身被物理裁切时仍尝试显示其可见前缀，不能承诺所有数字完整。
- W=0 时不写任何单元格。极矮导致没有状态栏空间时继续沿用原布局隐藏规则。
- 若产品希望再窄也完整显示两个数值，必须允许增高栏位或修改统计格式；这超出当前单行提案，不能用越界绘制实现。

### 6.4 Esc 待命提案

用户要求保留统计，本方案在可容纳时也保留 Esc 待命期间的统计，去掉现有“stop_armed 则不生成 stats”的特别分支。停止提示仍替换 todo/阶段，继续从已有长短短语中按文字预算选择。

极窄时停止操作反馈优先：若最短停止提示与完整统计不能共存，保留现有可容纳的停止提示，统计在剩余空间安全裁切。该例外仅限停止待命反馈，不能用于普通长 todo 的布局。审核时需注意：本节改变的是 Esc 待命行的信息分配，双 Esc 行为和超时本身不变。

### 6.5 扫描高亮与中文字边界

继续使用原 loading_frame、3 字符扫描窗口和现有颜色 token，不新增 spinner、定时器或帧状态。统计保持 dim 静态，不参与扫描。

当前代码在 `put_char` 后检查右界，剩余一列时写汉字可能占用保留区域。新算法必须在写字前测量字符宽度，且让扫描器使用裁切后的可见文字计算周期，避免长 todo 被截断后扫描长时间消失。

关键约束伪代码：

```rust
// 示意：以实际 Grid 对零宽字符的能力为准。
let cell_width = ch.width().unwrap_or(0);
if cell_width == 0 {
    continue;
}
if usize::from(cell_x) + cell_width > usize::from(label_right) {
    break;
}
grid.put_char(cell_x, y, ch, cell_style);
cell_x = cell_x.saturating_add(cell_width as u16);
```

`label_right` 必须是文字独立预算的右界，不得把统计区也传给文字绘制器。绘制顺序不能让长文字覆盖统计单元格。

原 loading 优先于回到底部的规则保持；该控件的绘制与命中矩形继续共用现有函数，不新增按钮或 todo 点击区域。

## 7. 最小实现结构及关键接口

### 7.1 选择理由

本需求只改变展示，现有 app 状态已足够。优先在 `chat_bar.rs` 中实现私有的只读文案选择函数及轻量枚举，不向 app 新增持久状态，不为几十行派生逻辑新建生产 activity 模块，也不扩展已经过长的根状态文件。

UI 读取公开的现有状态并选择展示文案，符合 ui 只读状态的边界。计划执行、状态转换与 IO 继续由原 app/event 模块负责。

拟议形状，以下是设计伪代码而非已提交实现：

```rust
enum ActivityLabel<'a> {
    Todo(&'a str),
    Thinking,
    RunningTools,
    Working,
}

fn activity_label(lx: &Lx) -> ActivityLabel<'_>;

// 文案常量及 todo 展示净化位于 ui::text::chat。
// draw_loading 接收已选择文案，并分配独立的 label/stats 区间。
// draw_scanned_label 只画自己的区间，继续复用 loading_frame。
```

选择函数的核心流程：

```text
查 lx.chat.plan.steps 中第一个 InProgress → Todo
否则严格匹配 RequestState::Streaming → 按 assistant_id 找当前 assistant
有 Pending/Running tool → RunningTools
最后一步 part.reasoning_open() → Thinking
其余 → Working
```

Esc 待命放在绘制编排层覆盖 ActivityLabel，避免向业务状态新增“停止提示阶段”。固定英文文案集中定义在 `ui::text::chat`，chat_bar 不硬编码英文字符串。

宽度布局可在 `chat_bar.rs` 内用一个小的纯函数返回文字、统计的可用区间；是否需要局部结构体由实际参数数量决定。不得给现有多参数绘制函数不断追加布尔开关；优先将坐标收为已有 Rect 或明确的局部展示参数。

### 7.2 文件范围

| 文件 | 计划修改 |
| --- | --- |
| `src/ui/main_content/lx/chat_bar.rs` | 文案派生、传入动态文本、统计优先预算、Esc 组合规则、先测宽后落笔；新增 tests 模块声明。 |
| `src/ui/main_content/lx/chat_bar/tests.rs`（新） | 组件级纯状态、布局、buffer 及动画测试。新建有实际测试职责的子目录。 |
| `src/ui/text/chat.rs` | Thinking / Running tools 常量、todo 单行净化函数；保留原统计格式化。 |
| `src/ui/text/chat/activity_tests.rs`（新） | 净化和显示格式的定向测试，通过单独测试模块引入。 |
| `docs/standards/tui-design-requirements.md` | 实施时同步固定 Working、统计隐藏及 Esc 状态栏规则。 |
| `docs/standards/project-directory-structure.md` | 实施时仅更新 chat_bar 的职责说明中固定 Working 示例。 |
| 本目录 design.md / task.md | 记录审核、完成状态、实际验证结果。 |

预期不修改 `src/app`、`src/event`、`src/chat`、`src/db`、Cargo 依赖和锁文件。若实施发现公开类型无法从现有路径使用，先核实现有导出链，再做最小必要调整并记录原因，不借机重构大文件。

## 8. 验收矩阵

| 编号 | 情景 | 预期 |
| --- | --- | --- |
| A01 | 计划有唯一 InProgress，同时模型正在思考 | 显示 todo，继续扫描；不显示 Thinking。 |
| A02 | 只有 Pending，当前开放 reasoning | 显示 Thinking，不显示待办文字。 |
| A03 | 无计划，当前 tool 为 Pending/Running | 显示 Running tools。 |
| A04 | 只有 InputStreaming tool | 显示 Working，不声称已执行。 |
| A05 | 工具已终态，最后步骤是普通正文 | 显示 Working。 |
| A06 | 历史中有 reasoning，当前 reasoning 已关闭 | 不显示 Thinking。 |
| A07 | Committing/Finishing，上一条历史曾在思考 | 无当前 todo 时显示 Working。 |
| A08 | shell 记录追加到当前 assistant 之后 | 仍按 assistant_id 派生模型阶段。 |
| A09 | 计划更新、清空、全部完成 | 下一帧按新计划或实际阶段显示，无旧文字缓存。 |
| A10 | 新轮次沿用未完成计划 | 继续显示会话 InProgress。 |
| A11 | 切会话、新会话、恢复旧会话 | 沿用各自 canonical plan，不串会话。 |
| A12 | 当前 todo 很长，完整统计可容纳 | 截断/隐藏 todo，完整统计仍在。 |
| A13 | 宽度仅够统计 | 无 todo，完整统计保留，不留无意义间隔。 |
| A14 | 宽度连统计自身也不够 | 普通运行仅统计按可用列裁切，不越界、不整段主动隐藏。 |
| A15 | 文字剩一列但下一字为汉字 | 不半写汉字，不污染统计或外侧保留列。 |
| A16 | todo 含换行、Tab、CR、ESC/其他控制字符 | 单行安全文本，原计划未改。 |
| A17 | todo 净化后为空、混合中英文、零宽字符 | 合理回退或安全绘制，无 panic、无错列。 |
| A18 | 固定文字/短 todo/长 todo，不同 loading_frame | 可见文字继续扫描，统计样式不随帧闪动。 |
| A19 | 首次 Esc、待命超时、第二次 Esc | 停止提示优先；空间充足时统计保留；交互行为不变。 |
| A20 | 请求结束/失败/取消，后台任务面板打开 | loading 依现有可见性规则消失。 |
| A21 | 上滚未跟随底部，宽/窄状态栏 | 回到底部入口及命中规则不退化。 |

状态/布局自动测试可证明文字选择和 buffer 单元格边界；真实终端颜色观感、字体对组合字符的实际显示及连续扫描效果需用户交互验收。

## 9. 验证、性能与回退

按受影响组件定向验证，命令和执行顺序见 task.md。设计阶段不运行 Rust 测试，也不将源码检索描述为测试通过。

每帧仅遍历当前计划及当前 assistant 的步骤；不扫描全部历史来找 todo，不重新解析 JSON，不读取数据库，不执行 IO。todo 长度已有生产上限，净化只处理选中步骤。复用原动画 tick，不因切换文案增加刷新频率。

回退只需恢复 chat_bar 的固定 Working 选择及原布局，移除本次新增文案/测试，并同步标准文档；没有数据库迁移或数据回填。未经明确授权，不执行 Git checkout/reset/commit 等操作。

## 10. 本次审核要点

1. §5 的选择优先级及会话级计划复用是否符合预期。
2. §6.2 必须以统计优先，不能恢复“长文字挤掉统计”的旧行为。
3. §6.3 明确 W<S 的物理裁切边界，完整统计只在其本身能放下时保证。
4. §6.4 Esc 待命期间空间充足也保留统计；极窄时停止提示优先。
5. §7 仅在 UI 做只读派生，避免扩大为后台协议或持久化改造。

审核通过前，task.md 的实施与验证项全部保持未完成。
