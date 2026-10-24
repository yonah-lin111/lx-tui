# lx `ask` 工具任务拆解

> 前置：`design.md` 已获用户确认后才开始编码。
> 纪律：每个任务完成即跑该任务列出的精确测试；不做全量重复验证；不修改 Git 状态（除用户明确授权）。

---

## 阶段 0：纯工具层（无 UI、无事件）

### T0.1 新建 `src/chat/tools/ask.rs`

**目标**：ask 的参数模型、校验与结果构造全部是纯函数，可单测。

**关键点**

- 常量表见 `design.md §4.2`，全部 `pub const`，schema 与校验共用同一来源，不写第二份数字。
- 类型：

```rust
pub const ASK_TOOL: &str = "ask";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<AskOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskOption {
    pub label: String,
    pub description: String,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub answers: Vec<String>,
}
```

- `pub fn parse_arguments(arguments_json: &str) -> ToolResult<Vec<AskQuestion>>`
  - 手工逐字段解析（风格对齐 `src/chat/tools/plan_args.rs` / `src/chat/plan.rs::parse_plan`），拒绝未知字段；
  - 错误消息稳定英文，带题号/字段名，例如：
    - `ask requires 1 to 3 questions`
    - `question 2: "header" is longer than 12 characters`
    - `question 1: duplicate option label "A"`
    - `question 3: preview exceeds 4096 bytes`
    - `question 1: option 2 contains a control character`
  - 校验顺序固定：结构 → 数量 → 字符上限 → 重复 → preview 限额（便于测试与模型修正）。
- `pub fn answered_result(questions: &[AskQuestion], answers: &AskAnswers) -> ToolExecutionResult`
  - 按题目顺序组装 `AnsweredQuestion`；缺失的 id 视为空数组但**不得**发生（事件层保证全答）；
  - 去重保留首次出现；手写文本（客户端约定放在数组最后）原样保留。
- 子模块 `ask/tests.rs`（或 `ask_tests.rs`，按项目现有习惯）覆盖 `design.md §14.1` 全部条目。

**测试**：`cargo test chat::tools::ask`

**完成标准**：所有校验分支有测试；`parse_arguments` 对 1 MiB 级别输入不长循环（字符上限先于遍历）。

**依赖**：无。

---

### T0.2 接入工具表与结果类型

**文件**

- `src/chat/tools/types.rs`
- `src/chat/tools/mod.rs`
- `src/chat/tools/schema.rs`
- `src/chat/tools/output.rs`

**关键点**

1. `TOOL_NAMES` 从 `[&str; 7]` 改为 `[&str; 8]`，末尾加 `"ask"`。
2. `ToolExecutionResult` 增加 `Ask { questions: Vec<AnsweredQuestion> }`。
3. `tools::execute_with` 的守卫分支加入 `"ask"`（与 `exec_command` / `write_stdin` / `update_plan` 同一句错误文案：必须由 agent loop 执行），**绝不**落到文件执行器。
4. `schema.rs`：
   - 模块文档「七工具」→「八工具」，`available_tools` 文档同步；
   - 新增 `ask_parameters()`（JSON 见 `design.md §4.1`），数字全部引用 `ask.rs` 常量；
   - `TOOLS` 数组 7 → 8，位置放在 `update_plan` 之后。
5. `output.rs`：
   - `format_for_model`：输出 `{"answers":{"<id>":{"answers":[...]}}}`；
   - `to_result_json`：输出 `{"tool":"ask","questions":[{id,header,question,answers}]}`；
   - `to_summary` 不新增字段（返回默认摘要）。

**测试**：`cargo test chat::tools`、`cargo test chat::transport::tools`、`cargo test chat::client`

**完成标准**：三处「工具列表与 `TOOL_NAMES` 一致」的现有测试全部通过；未知工具错误消息包含 `ask`。

**依赖**：T0.1。

---

### T0.3 历史卡片数据模型

**文件**

- `src/app/state/lx/chat/tool_view.rs`
- `src/event/chat/tool_calls.rs`（`detail_of`）

**关键点**

```rust
pub enum ToolDetailView {
    Command(CommandView),
    Interaction(ProcessInteractionView),
    Plan(PlanSnapshotView),
    File(super::file_diff::FileDiffDetailView),
    Ask(AskDetailView),          // 新增
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskDetailView {
    pub questions: Vec<AskAnsweredView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAnsweredView {
    pub header: String,
    pub question: String,
    pub answers: Vec<String>,
}
```

- `AskDetailView::from_result_json(&str) -> Option<Self>`：只解析 `"tool":"ask"`；字段缺失时返回 None（沿用旧记录兜底策略）。
- `ToolDetailView::from_result_json` 增加 `"ask"` 分支（重载路径自动生效）。
- `detail_of` 增加 `ToolExecutionResult::Ask { questions } => Some(ToolDetailView::Ask(...))`。
- 编译错误会提示所有需要补 `Ask` 分支的穷尽 match（`process_session_id`、`plan_view`、`file_view` 等），一律返回 `None`。

**测试**：`cargo test app::state::lx::chat`、`cargo test event::chat`

**完成标准**：实时结算与历史重载两条路径得到逐字段相等的 detail（沿用现有 roundtrip 测试风格）。

**依赖**：T0.2。

---

## 阶段 1：阻塞握手（无渲染）

### T1.1 `AgentHost` / `AgentHostEvent` 扩展

**文件**：`src/chat/agent_loop/mod.rs`、`src/event/chat/host.rs`

**关键点**

- `AgentHostRequest::AskUser { call_id, questions, reply }`；
- `AgentHost::ask_user(...)` 默认方法（oneshot，丢弃即 `Err(Cancelled)`），实现见 `design.md §5.2`；
- `AgentHostEvent::AskUser { key, call_id, questions, reply }`，`EventHost::send` 映射；
- `AskAnswers = BTreeMap<String, Vec<String>>` 定义在 `chat/tools/ask.rs` 并导出。

**测试**：`cargo test chat::agent_loop::tests`

**完成标准**：`RecordingHost` 能记录 `AskUser` 并回答案；现有测试不改语义。

**依赖**：T0.2。

---

### T1.2 `run_agent_turn` 分支

**文件**：`src/chat/agent_loop/mod.rs`

**关键点**

- 精确替换 `executor.execute_tracked(...)` 调用点为 `design.md §5.3` 的分支；
- 取消时：`finish_tool(ticket, Cancelled)` + `cancel_unstarted(remaining)` + `interrupted`；
- `HostError::Failed` 时：把失败结果落库后整轮失败（不重试）；
- 参数校验失败：`Failed{invalid_arguments}` 结果照常 `finish_tool`，**不**调用 `ask_user`，轮次继续采样。

**测试**

- 成功：BeginTool → AskUser → FinishTool 顺序；模型下一轮上下文包含答案 JSON。
- 取消：`cancel` 后 `ask_user` 立即返回；DB 结算为 `cancelled`；summary 为 `Interrupted`。
- 非法参数：未出现 `AskUser`，结果为失败，轮次继续。

**完成标准**：测试用可控执行器/host 完成，不触网。

**依赖**：T1.1。

---

### T1.3 事件层挂起/取消/回执

**文件**

- `src/event/chat/ask.rs`（新）
- `src/event/chat/mod.rs`
- `src/app/effects.rs`

**关键点**

- `ChatStreams.asks: HashMap<RequestKey, PendingAsk>`（`design.md §5.4`）。
- `on_requested`：
  1. `active_assistant_id(state, key)` 为 None → `reply.send(Err(Cancelled))`，不写 UI；
  2. 若该窗格已有 `lx.ask` → `reply.send(Err(Failed{ code }))`；
  3. 写 `pane.lx.ask = Some(AskState::new(...))`；
  4. 存 `PendingAsk`；
  5. 返回 `true`（置脏重绘）。
- `cancel(&key)` / `finish(&key)` 取走并回 `Err(Cancelled)`。
- 效应泵处理 `Effect::AnswerAsk`：比对 `call_id` 后发送；不匹配静默丢弃。
- 流结束（`ChatNetEvent::Finished`）/取消/`reap_stale_streams`：清除对应窗格 `lx.ask`（防御 sweep）。

**测试**：`cargo test event::chat::tests`（扩展 `ack.rs`）

- AskUser 归属校验：旧 generation、窗格不存在、重复 ask；
- Esc/关窗格：pending 被唤醒、UI 清空；
- 迟到作答：call_id 不匹配 → 不发送；
- 答案发送成功后，`asks` 表为空。

**完成标准**：以上路径均无悬挂 sender；无 DB 新逻辑。

**依赖**：T1.2。

---

## 阶段 2：应用状态与输入

### T2.1 `AskState` / `AskEditor`

**文件**

- `src/app/state/lx/ask.rs`（新）
- `src/app/state/lx.rs`

**关键点**

- 结构见 `design.md §4.5`；`Lx` 增加 `pub ask: Option<AskState>`，`Lx::new()` 置 None。
- `AskEditor` 包装 `Lx`（`design.md §4.5`）：
  - `fn new() -> Self`（空的 `Lx::new()`）；
  - `fn insert_char / newline / backspace / delete / move_* / undo / redo / clear / text / set_cursor_end` 等转发；
  - **不实现** `DerefMut`，防止误用 chat/rewind 等字段。
- 从 `apply_lx_input` 抽出纯编辑分派：

```rust
/// 纯文本编辑分派（不含面板/发送/停止语义）；返回是否改写了文本。
pub(super) fn apply_editor_edit(lx: &mut Lx, command: LxInput) -> bool
```

  `apply_lx_input` 的 `Char/Newline/Backspace/Delete/Left/Right/Up/Down/Home/End/WordLeft/WordRight/DeleteToLineStart/DeleteToLineEnd/DeleteWordBackward/DeleteWordForward/Undo/Redo/Clear` 分支改为调用它；行为必须逐字节等价（现有测试守护）。
- 状态迁移：
  - `current_answers()`：单选/多选规则（`design.md §6.3`）；
  - `choose(index)` / `move_highlight(delta)` / `begin_edit()` / `commit_edit()` / `cancel_edit()` / `next_question()` / `prev_question()` / `scroll_preview(delta)`。

**测试**：`cargo test app::state::lx`（新增 `ask/tests.rs`）+ 现有 `app::update` 编辑测试全绿。

**完成标准**：草稿不变量有专门测试；单选/多选/手写规则的每条各一例。

**依赖**：T0.2（类型）、无 UI 依赖。

---

### T2.2 输入与提交（`app/update/chat/ask.rs`）

**文件**

- `src/app/update/chat/ask.rs`（新）
- `src/app/update/chat/mod.rs`（导出）
- `src/app/update/pane.rs`

**关键点**

- `apply_lx_input` 插入 ask 分支（在后台任务页之后、rewind 之前）。
- `apply_ask_input(state, pane, LxInput)`：
  - 编辑态：`Char/编辑键` → `AskEditor`；`Enter` 提交文本；`Esc` 退出编辑；`Tab` 吞掉；
  - 列表态：`↑↓/j/k` 移动；`Space/1-9` 选择；`Enter` 提交；`Tab/←→` 切题；`PgUp/PgDn` 滚动预览；`Esc` 走 `chat::escape_stop`（复用双击确认，`design.md §7.1`）。
- `submit_current(state, pane)`：
  - 空答案 → `hint`；否则写 `collected`；末题 → 组装 `BTreeMap`，`lx.ask = None`，`state.effects.push(Effect::AnswerAsk { key, call_id, answers })`。
- `apply_ask_click` / `apply_ask_wheel`：由鼠标层调用，内部只改状态。
- 鼠标点击在面板外不处理（交回正常流程）。

**测试**：`cargo test app::update::chat::ask` + `cargo test event::tests`

- 键位表逐行；
- 鼠标语义逐行（单选不提交、多选勾选、Other 进编辑、Submit、滚轮分区、吞掉）；
- `Esc` 双击取消：第一次只 arm、第二次推 `CancelChatStream`；
- 提交后 `lx.ask == None` 且草稿逐字段不变。

**完成标准**：`apply_lx_input` 的现有行为零回归（全部现有测试通过）。

**依赖**：T2.1、T1.3。

---

## 阶段 3：渲染

### T3.1 面板几何 + 绘制 + 命中

**文件**

- `src/ui/main_content/lx/ask.rs`（新）
- `src/ui/main_content/lx.rs`
- `src/ui/text/agent.rs`

**关键点**

- `ask::layout(area, lx) -> Option<AskLayout>`：外框、标题/标签、题干、选项区、预览区、编辑框、Submit、底栏、滚动条（`design.md §8.1`）。
- 布局让位：`lx.rs::layout()` 在 ask 激活时 `input = None`，并计算 70%/最小 10 行高度；`render` 在输入框原位置调用 `ask::draw`。
- 光标：编辑态返回编辑框内光标（相对 `area`），复用现有 render 返回通道。
- 面板边框复用 `draw_box`；色板复用 `style::*`，不新增主题色。
- 文案常量全部进 `ui/text/agent.rs`（ASCII）。
- `pub(crate) fn ask_rect(area, lx) -> Option<Rect>` 供鼠标层吞掉点击。

**测试**：`cargo test ui::main_content::lx`（新增 `ask/tests.rs`）

- 尺寸矩阵（80x40、50x20、30x10、20x8）；
- 70% 上限/最小 10 行/60 列阈值/极小窗格隐藏预览；
- 单选/多选标记、Other 行、预览原样保留前导空格、宽字符不裂、截断省略；
- 几何与命中一致（逐格）。

**完成标准**：`history::layout` 不受影响；无 ask 时所有现有渲染快照/几何测试不变。

**依赖**：T2.1。

---

### T3.2 历史卡片渲染

**文件**

- `src/ui/main_content/lx/history/tool/ask.rs`（新）
- `src/ui/main_content/lx/history/tool.rs`
- `src/ui/main_content/lx/history.rs`

**关键点**

- `rows()` / `draw()` 接入 `ToolDetailView::Ask`（`design.md §8.4`）。
- 标题行使用 `header_with_verb(call, ..., VERB_TOOL_ASK, status)` 与 `mark_of`；**不使用 `?` 图标**。
- 运行中：`waiting for you`；完成：`N questions`（N 来自 detail）。
- 折叠：每题一行 `header · answers`；展开：追加完整题干；永不画 preview。
- `history::layout` 与 `render` 共用 `ask::rows`。

**测试**：`cargo test ui::main_content::lx::history`

- 运行中/完成/取消/失败；
- 0 答案（异常数据）不 panic；
- 宽字符、超长题干、展开/折叠行数一致。

**依赖**：T0.3、T3.1（Grid 辅助）。

---

### T3.3 标签页 `?` 标记

**文件**：`src/ui/tab_bar.rs`、`src/ui/text.rs`、`src/app/state.rs`（或 `lx` 辅助方法）

**关键点**

- `Lx::has_pending_ask()` → `ask.is_some()`；`AppState` 提供「该标签页是否任一窗格有 ask」的只读判断（不加全局状态）。
- 标签宽度计算、绘制、命中同步 +2 列；无 ask 时布局与现状逐字段一致。
- 标记文案常量 `TAB_ASK_MARKER = "?"`，强调色。

**测试**：`cargo test ui::tab_bar`

- 有/无 ask 的标签宽度、后续标签 x 坐标、点击命中；
- 多窗格标签：任一窗格有 ask 即标记。

**依赖**：T2.1（状态）.

---

## 阶段 4：事件接线与收尾

### T4.1 鼠标与粘贴接线

**文件**：`src/event/mouse/press.rs`、`src/event/mouse/wheel.rs`、`src/event/input.rs`

**关键点**

- 点击：toast 之后、历史命中之前，检查所有可见 lx 窗格的 `ask_rect`；命中 → 聚焦/切标签 → `update::apply_ask_click` → 吞掉。
- 滚轮：面板内按分区处理并吞掉（不滚背后历史）。
- 粘贴：ask 编辑态写入缓冲；非编辑态吞掉（绝不写草稿、不触发路径面板）。

**测试**：`cargo test event::mouse`、`cargo test event::tests`

- 点击选项/Other/Submit/题目标签；
- 点击面板外仍能命中历史卡片；
- 编辑态粘贴、非编辑态粘贴被吞；
- 滚轮预览/选项分区。

**依赖**：T2.2、T3.1。

### T4.2 尺寸同步

**文件**：`src/app/update/pane.rs`、`src/layout.rs`

**关键点**

- 新增 `layout::lx_ask_editor_size(rect)`（宽度 = 面板内容宽 - 内边距；行数 = min(5, 面板可用)）。
- `resize_panes` 在 ask 激活时同步编辑缓冲尺寸。

**测试**：`cargo test app::update::pane`（软换行宽度与渲染一致）。

**依赖**：T2.1、T3.1。

### T4.3 提示词与版本

**文件**：`src/chat/prompt.rs`

**关键点**：见 `design.md §11`。

**测试**：`cargo test chat::prompt`

**依赖**：T0.2。

---

## 阶段 5：验证与评审

1. `cargo fmt`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. 用户 `cargo run` 实机确认：
   - 模型发起 ask → 面板替换输入框；
   - 单选/多选/自由输入/字符画预览；
   - 鼠标点选、滚轮、Submit；
   - 标签切换时 `?` 标记；
   - `Esc` 双击取消；编辑态 `Esc` 只退出编辑；
   - 作答后历史卡片与草稿状态。
5. 写 `docs/feature/lx-ask-tool/review.md`：实际改动、偏差、遗留项、验证记录。

**完成标准**：`design.md §16` 全部满足；review.md 已回写。

---

## 审核清单（请用户逐条确认）

- [ ] 工具名 `ask`，动词 `Ask`，工具总数 8。
- [ ] 参数上限与「整次拒绝」策略（`design.md §4.2–4.3`）。
- [ ] 阻塞整轮 + `Esc` 双击取消 + 编辑态 `Esc` 只退出编辑。
- [ ] 逐题 1–3 题、单选/多选、0 选项纯自由输入。
- [ ] 每个选项一份 `preview`，原样等宽渲染，历史卡片不展示。
- [ ] 面板替换输入框、70% 高度上限、60 列分栏。
- [ ] 独立编辑缓冲 + 复用编辑核心，草稿永不搬迁。
- [ ] 鼠标语义（单选只高亮、多选即勾选、Submit 唯一提交）。
- [ ] 不可见窗格用标签页 `?` 标记。
- [ ] 无数据库结构变更；重启按 `interrupted` 恢复。
