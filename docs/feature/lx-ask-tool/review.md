# lx `ask` 工具实施记录

> 分支：`feat/lx-ask-tool`（worktree `.worktrees/lx-ask-tool`，基线 `dev@f74e3f6`）
> 规范：`design.md`（16 节）、`task.md`（T0.1–T5）

## 1. 交付内容

| 层 | 文件 | 说明 |
|---|---|---|
| 工具 | `src/chat/tools/ask.rs` + `ask/tests.rs` | 参数模型、上限常量、`parse_arguments`（结构→数量→字符→重复→preview，整次拒绝）、`answered_result`；24 例单测 |
| 工具 | `types.rs` / `mod.rs` / `schema.rs` / `output.rs` | `TOOL_NAMES` 8 项、`ToolExecutionResult::Ask`、第 8 个 ToolSpec（数字引用 ask.rs 常量）、`format_for_model` / `to_result_json` |
| 卡片数据 | `tool_view.rs` / `steps.rs` / `event/chat/tool_calls.rs` | `AskDetailView`/`AskAnsweredView`，结算与重载两条路径同构 |
| 握手 | `chat/agent_loop/mod.rs` / `event/chat/host.rs` | `AgentHostRequest::AskUser` + `AgentHost::ask_user`（oneshot，丢弃=取消）+ `AgentHostEvent::AskUser` |
| 轮次 | `run_agent_turn` | ask 分支：非法参数→Failed 不打扰用户；取消→`finish_tool(Cancelled)` + `cancel_unstarted` + interrupted；`Failed{code}`→整轮失败 |
| 事件 | `event/chat/ask.rs` / `mod.rs` / `app/effects.rs` | `ChatStreams.asks`、归属校验、`Effect::AnswerAsk{key,call_id,answers}`、取消/结束/孤儿 sweep |
| 状态 | `app/state/lx/ask.rs` / `lx.rs` / `pane.rs` | `AskState`/`AskEditor`（独立缓冲，无 `DerefMut`）、`Lx.ask`、`Tab::has_pending_ask` |
| 输入 | `app/update/chat/ask.rs` / `pane.rs` | 键位（§7.1/§7.2）、单选/多选/手写规则、双击 Esc 取消、`apply_ask_paste`、`apply_editor_edit` 抽取复用 |
| 渲染 | `ui/main_content/lx/ask.rs` | 几何（70% 上限、最小 10 行、60 列分栏、矮窗格隐藏预览）、绘制、命中、滚轮分区（同一份 `AskLayout`） |
| 渲染 | `ui/main_content/lx/history/tool/ask.rs` | 卡片标题 `■/□ Ask`、`waiting for you` / `N questions`、每题树行、展开题干、**不展示 preview** |
| 渲染 | `ui/tab_bar.rs` / `ui/text*.rs` | 挂起 ask 的标签 +2 列并显示 `?`（宽度/绘制/命中同步） |
| 接线 | `event/mouse/{press,wheel}.rs`、`event/input.rs` | 面板内点击吞掉、滚轮分区分派、粘贴只进手写缓冲 |
| 尺寸 | `layout.rs` / `resize_panes` | `LX_ASK_*` 常量 + `lx_ask_editor_size`，折行宽度与渲染同源 |
| 提示词 | `chat/prompt.rs` | 第 8 个工具 + ask 段（对齐 codex default.md）+ `PROMPT_VERSION = "lx-3"` |

## 2. 验证记录

| 命令 | 结果 |
|---|---|
| `cargo test --lib chat::tools::ask` | 24 passed |
| `cargo test --lib chat::agent_loop` | 40 passed（含成功/取消/非法参数三例） |
| `cargo test --lib event::chat` | 79 → 加入阶段 3 后仍全绿（含 ask 9 例） |
| `cargo test --lib app::update` | 582 + 本阶段新增 |
| `cargo test --lib ui::main_content::lx::ask` | 12 passed |
| `cargo test --lib history::tool::ask` | 5 passed |
| `cargo test --lib ui::tab_bar` | 9 passed |
| `cargo test --lib event::tests::ask_` | 2 passed（点击选中、粘贴只进缓冲） |
| `cargo test`（全量） | **3291 lib + 16 个 target 全绿** |
| `cargo fmt` / `cargo clippy --all-targets -- -D warnings` | 零警告 |

未验证（Agent 无法交互）：真实 TUI 外观、鼠标手感、真实模型发起 ask 的端到端；需用户 `cargo run` 确认。

## 3. 与 design/task 的偏差

1. **单选鼠标单击 = 独占选中，不做「只高亮」**（design §7.3 已同步修改）。
   - 原设计只高亮不选中，导致纯鼠标用户永远交不出答案（键盘有 Space，鼠标没有）；现为「单击选中、Submit/Enter 提交」。
2. **任务顺序：T2.1 提前到 T1.3 之前**。T1.3 要在事件层写 `pane.lx.ask = AskState`，必须先有 `AskState`。
3. **修了一个既有崩溃**（阶段 1 测试撞出）：`Lx::delete_word_forward` 删除后把光标设到已删除的 `end`，缓冲区缩短后越界，下一次编辑在 `slash_trigger` 里 `&input[..cursor]` panic。改为光标不动（readline `M-d` 语义），1 行改动。
4. **手写缓冲初始宽度**：事件层用「输入框宽度 − 4」初始化；窗格 resize 后由 `layout::lx_ask_editor_size` 精确同步，渲染以缓冲区自身宽度为准（软换行与画框始终同源）。
5. **卡片运行态只写 `waiting for you`**：不解析流式参数、不猜题数（design §8.4 的取舍）。
6. **`AskHit`/`AskWheelZone` 只做状态迁移**，几何全部由 UI 层 `layout()` 给出，事件层不重算（design §7.3 的硬约束）。

## 4. 已知限制 / 后续

- 面板在极小窗格（`area.height < 12`）只显示题干 + 选项 + 底栏，预览隐藏并在底栏提示；无法滚动查看被隐藏的预览——这是设计取舍，不是缺陷。
- 历史卡片不保存/不展示 preview（`result_json` 自包含、不含 preview），与 design §9 一致。
- 标签栏 `?` 标记只在「该标签任一窗格挂着 ask」时出现；窗格切到终端视图同样计数（防御：面板仍挂着）。
- `/compact` 等本地命令在 ask 等待期间不会执行：输入被面板整块接管，这是 design §9.1 的预期。

## 5. 合并注意

- 无数据库迁移、无 `SCHEMA_VERSION` 变更。
- 与 `.worktrees/lx-history-perf`（未提交改动含 `history/tool.rs`）存在同文件冲突风险：对方合并前需先 rebase，`tool.rs` 的 ask 分支只有两处（`rows` / `draw_animated`）。
- 合并后建议实机走一遍：模型发起 ask → 单选/多选/手写/字符画预览 → 鼠标点选与滚轮 → 标签 `?` → Esc 双击取消 → 历史卡片与草稿状态。

## 6. v2 修订（用户复审后）

按复审意见重做了面板外观与交互，对应 commit 见分支历史：

1. **去掉四周边框**：只保留顶部一条通栏分隔线（`border_title()` 样式，与后台任务面板 `task::draw_list` 同款）；不再调用 `draw_box`。
2. **高度上限改为窗格 70%**：`height = min(need, 70%)`，不再强制 10 行最小；内容超出时选项列表内部滚动，最右一列只画 thumb 的滚动条，滚动窗口跟随高亮。
3. **题目 tab 常驻**：多题时 tab 行（含 `Confirm`）无论窗格多矮都保留。
4. **选项改为双行**：第一行左侧是勾选标记 + 标签，第二行是缩进的描述；两行都命中该选项（`AskHit::Option(i)`）。
5. **空答案回车 → 窗格内 toast**（`select an answer first`），不前进、不提交；移除内联 `hint`。
6. **交互对齐 opencode**：
   - 单题单选：`Enter` 选中高亮项后立即提交；单题多选：`Space` 勾选、`Enter` 提交。
   - 多题：单选 `Enter` 选中并进入下一题；多选 `Space` 勾选、`Enter` 下一题；末题后进入 `Confirm`。
   - `Confirm` 只有全部题目都有答案时 `Enter` 才提交（否则 toast），tab 可直接点 `Confirm`。
   - 切题时高亮自动落到已选答案（`restore_highlight`），避免回车默认覆盖旧选择。
7. **状态模型简化**：删除 `hint` / `collected` / `list_scroll`，新增 `confirm: bool`；`submit_current` → `submit_all`（唯一提交入口）。

验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings` 零警告；`cargo test` **3293** 个 lib 用例 + 16 个 target 全绿（新增/改写用例覆盖：单题回车即提交、多选空格后回车提交、多题前进到 Confirm、Confirm 未答完不提交、空答案 toast、双行选项命中、70% 上限与滚动、tab 常驻、无边框单线）。

## 7. v3 评审与修复（本轮）

评审方式：两个只读 subagent 并行（协议/握手/事件层；状态/输入/渲染层），逐条要求给出触发路径。以下按严重度记录**缺陷 → 错误原因 → 修法**。

### 7.1 已修（真缺陷）

#### ① 取消竞态：面板在取消之后被重新装上，整轮永久卡住

- 位置：`src/event/chat/ask.rs::request`、`src/event/chat/mod.rs::cancel/finish`
- **错误原因**：`Esc` 只推 `Effect::CancelChatStream`，不把 `RequestState` 从 `Streaming` 改走；事件层处理该效应时若 `streams.asks` 尚空，`ChatStreams::cancel` 仅置位取消信号。随后才送达的 `AgentHostEvent::AskUser` 通过 `active_assistant_id` 校验 → 重新装面板并登记 sender，而 loop 的取消信号已被消费，`sweep_orphans` 又因 `is_busy()` 为真不清面板 → 面板与用户意图相反且轮次卡到用户作答为止。
- **修法**：`request` 增加权威判定——`streams.cancels[key]` 已为 `true` 时直接回 `Cancelled`，不写面板、不登记；`ChatStreams::finish` 自己 `wake_ask`（不再依赖调用点记得补 `ask::cancel`）。
- **代码**：
  ```rust
  if active_assistant_id(state, &key).is_none()
      || streams.cancels.get(&key).is_some_and(|cancel| *cancel.borrow())
  {
      let _ = reply.send(Err(HostError::Cancelled));
      return false;
  }
  ```
- **测试**：`event::chat::tests::ask::cancel_before_registration_does_not_reopen_the_panel`、`streams_finish_alone_wakes_and_drops_the_pending_ask`。

#### ② `AskEditor` 复用 Lx 的 shell 前缀：答案被静默改写、编辑框错位

- 位置：`src/app/state/lx/ask.rs`（`text()`）、`src/app/state/lx.rs`（`adopt_shell_prefix`）
- **错误原因**：`Lx::insert_char` 会把首个非空白 `!`/`！` 吸掉并切成 Shell 模式；`AskEditor::text()` 用 `serialized_input()`（会补回 `!`、全角转半角），而 `visual_rows()`/`cursor_visual()` 按**未加前缀的 `input`** 算字节偏移。`draw_editor` 拿后者切前者 → `text.get(range)` 落在字符中间返回 `None`，整行不画，光标悬空。答案内容同时被改写（`！重要` → `!重要`）。
- **修法**：`Lx` 增加 `plain` 开关（`Lx::plain()`）：不吸收 `!`、换行不续写列表标记、不刷新任何面板；`AskEditor::text()` 改读原始 `input`，与视觉行同源。
- **测试**：`app::state::lx::ask::tests::custom_editor_keeps_leading_bang_as_content`。

#### ③ 手写答案的硬上限只挡粘贴，键入可无限增长

- 位置：`src/chat/tools/ask.rs`、`src/app/update/chat/ask.rs`
- **错误原因**：只有 `apply_ask_paste` 按 `MAX_ASK_CUSTOM_BYTES` 检查；逐字键入完全绕过。`MAX_ASK_ANSWER_CHARS` 全仓无引用（死常量）。极端输入会让答案超过 `MAX_TOOL_RESULT_REPLAY_BYTES`，回放时 JSON 从中间截断，模型拿到非法 JSON；超过 1 MiB 时 `to_result_json` 的 shrink 不认 `questions`，历史卡片答案全丢。
- **修法**：上限在**写入处**执行——`AskEditor::can_insert` 统一把关 `insert_char`/`insert_str`/`newline`，键入与粘贴共用一条规则；超限整次拒绝并给窗格内 toast（`answer is too long`）。删除死常量 `MAX_ASK_ANSWER_CHARS`。
- **代码**：
  ```rust
  pub fn can_insert(&self, bytes: usize) -> bool {
      self.text().len().saturating_add(bytes) <= MAX_ASK_CUSTOM_BYTES
  }
  ```
- **测试**：`app::update::chat::tests::ask::custom_text_stops_at_the_byte_limit_with_a_toast`。

#### ④ 后台任务页打开时，隐藏的 ask 面板抢走任务列表的鼠标点击

- 位置：`src/event/mouse/press.rs`、`src/event/mouse/wheel.rs`
- **错误原因**：task 与 ask 共用输入框矩形，task 优先绘制。press 先按 `ask_rect` 命中并 `return`，而该矩形此时就是 task 列表——点列表任意位置都在改一个看不见的答案，列表本身点不动；wheel 同样。
- **修法**：两处命中入口都加 `target.lx.task.is_none()` 前置（不清 `lay.ask`：`build_chat` 的 `bottom_box` 靠它维持 task 打开期间的历史几何）。
- **测试**：`event::tests::task_page_swallows_ask_hits_so_the_list_stays_clickable`。

#### ⑤ 列表一滚动，单题的 Submit 行永远不可见/不可点

- 位置：`src/ui/main_content/lx/ask.rs::place_rows`
- **错误原因**：`highlight_end` 对「其他」只取到它自己的末行，而 `Submit` 恰在下一行（下标 == visible），被 `if index >= visible { break }` 丢弃。4 个带描述选项 + 16 行窗格即触发：纯鼠标用户无法提交。
- **修法**：高亮停在「其他」且其后紧跟 `Submit` 时，滚动窗口多走一行，保证 `Submit` 留在可视区。
- **测试**：`ui::main_content::lx::ask::tests::submit_row_stays_visible_when_the_other_row_is_highlighted`。

#### ⑥ 预览没溢出也能滚出空白

- 位置：`src/app/state/lx/ask.rs::preview_max_scroll`、`src/ui/main_content/lx/ask.rs::draw_preview`
- **错误原因**：滚动上限只按 `lines - 1`，不看可视高度。3 行预览 + 10 行预览区按 `PgDn` 后只剩一行内容加一片空白，且因 `total <= visible` 连 thumb 都不画——用户看起来就是「预览坏了」。
- **修法**：上限改为 `total - visible`；可视行数由事件循环在绘制前写入状态（`resolve_ask_preview_visible`，与 `resolve_task_output_visible` 同款），几何与渲染同源。
- **测试**：`preview_scroll_is_clamped_by_preview_lines`（改写）、`shrinking_the_preview_pulls_the_offset_back_into_range`。

#### ⑦ 历史 `ask` 卡片永远不能展开（expanded 分支是死代码）

- 位置：`src/ui/main_content/lx/history/tool.rs`（`fold_region` / `has_expandable_content` / `card_regions`）
- **错误原因**：三处对 `Ask` 一律返回 NONE/false/空；展开只能经 `history_hit::card_at → card_regions` 登记点击面触发，因此完整题干（≤500 字）永远看不到，折叠态的 `header` 只有 12 字。
- **修法**：为 `Ask` 登记整卡 Body 点击面（仅当展开确实多出行时），`has_expandable_content` 用 `rows(expanded) > rows(folded)` 判定；题干不做 hover 提亮。
- **测试**：`history::tool::ask::tests::the_card_registers_a_click_face_only_when_expansion_adds_rows`。

#### ⑧ `Shift+Tab` 未接线（设计文档承诺的「上一步」不存在）

- 位置：`src/app/actions.rs`、`src/input/mod.rs`、`src/app/update/chat/ask.rs`
- **修法**：新增 `LxInput::PrevStep`；`KeyCode::BackTab` 与 `Tab+SHIFT`（kitty 协议）都映射到它；ask 面板中与 `←` 同义（`Confirm → 末题 → 上一题`）。
- **测试**：`app::update::chat::tests::ask::enter_submits_and_tab_arrows_switch_questions` 追加回退断言。

#### ⑨ 空答案载荷会把「缺题」伪造成「用户全不选」

- 位置：`src/event/chat/ask.rs::answer`
- **错误原因**：`answered_result` 对缺失的 id 兜底成 `[]`，那正是「用户全不选」的形状；事件层对 `Effect::AnswerAsk` 不做覆盖校验。当前只有 `submit_all`（有 `all_answered()` 把关）一个生产者，属防御缺口，但一旦新增入口就会把假答案交给模型。
- **修法**：事件层拒收回执中任何空答案数组，回 `Failed { code: "ask answer is empty" }` 并清面板，绝不让它成为工具结果。
- **测试**：`event::chat::tests::ask::empty_answer_is_rejected_instead_of_reporting_no_selection`。

### 7.2 同口径修正（当前不可见，防回归）

#### ⑩ 实时结算与历史重载的空摘要不一致

- 位置：`src/app/state/lx/chat/stream.rs::settle_tool_result`
- **错误原因**：实时路径无条件写 `Some(空摘要)`，重载路径 `tool_view_of` 用 `is_empty()` 过滤成 `None`。ask 卡片走类型化 `detail` 分支所以没有可见差异，但一旦回退到摘要路径就会画出孤立的空 `└─`（`steps.rs` 注释记录过同类回归）。
- **修法**：实时路径同样过滤空摘要。
- **测试**：`app::state::lx::chat::stream::tests::settle_drops_an_empty_summary_like_the_reload_path`。

#### ⑪ `PendingAsk.reply: Option` 是死设施

- **错误原因**：注释声称 `None` 表示「回执已丢失」，但全仓没有任何代码写 `None`；`if let Some(reply)` 分支永远命中 `Some`。
- **修法**：改成非 `Option`，唤醒逻辑收敛到 `ChatStreams::wake_ask`（`cancel`/`finish` 共用）。

### 7.3 第二轮补报（评审 agent 复查后追加）

#### ⑫ 矮窗格出现「可见、吃键、什么都选不了」的死面板

- 位置：`src/ui/main_content/lx/ask.rs::height` / `AskLayout::compute`、`src/layout.rs`
- **错误原因**：`height()` 只有 70% 上限（`need.min(max_h).max(1)`），6 行内容区多题时 `max_h = 4`：扣掉分隔线、tab 行、题干与底栏后 `body_h = 0`，`place_rows` 直接返回、`rows` 为空；但 `apply_lx_input` 见到 `ask.is_some()` 仍整块接管按键，鼠标也因 `ask_rect` 存在被吞。
- **修法**：① 面板高度加「最小可用行数」下界（`MIN_PANEL_ROWS = 分隔线 + 一行可选行 + 底栏`，多题再加 tab），取 `need.min(max_h).max(viable.min(area_h))`；② 题干在极矮时让位（`available < 2` 时高度 0），保证列表至少落位一行；③ 删除未落地的 `LX_ASK_MIN_BOX`，注释写清「只有上限 + 最小可用行数」的优先关系。
- **测试**：`tiny_pane_keeps_at_least_one_selectable_row`；`height_never_exceeds_seventy_percent` 的 cap 断言同步改为 `max(70%, 最小可用行数)`，不再自证。

#### ⑬ `can_insert` 按净化前字节判长，`\t` 可绕过上限一倍

- 位置：`src/app/state/lx/ask.rs::insert_str`、`src/app/update/chat/ask.rs::apply_ask_paste`
- **错误原因**：上限判定用原始输入长度，实际写入经 `Lx::insert_str` → `text::sanitize`，其中 `\t` 展开为两个空格。空编辑框粘贴 8192 个 `\t`：预检通过、写入 16384 字节。
- **修法**：`AskEditor::insert_str` 内部先 `sanitize` 再按净化后长度判定，返回是否写入；`apply_ask_paste` 只转发结果，判定与写入口径合一。`Lx::plain` 的换行恒为 1 字节，`newline` 的预算因此精确。
- **测试**：`paste_budget_uses_the_sanitized_length`、`list_mode_paste_is_swallowed_and_never_touches_the_draft`。

#### ⑭ 其它小项

- 预览超宽直接截断、未补 `…`：**不改**。字符画按列硬截断是刻意的（补省略号会污染对齐，且非文字内容没有「被截断」语义）。
- 编辑框初始折行宽度（`input_width() - 4`）与 `resize_panes` 后的 `lx_ask_editor_size`（`cols - margin*2 - 4`）差 2 列：创建后第一帧到首次 resize 之间存在短暂差异，`resize_panes` 在首帧前就会覆盖，实际不可见；保留现状但已在代码注释里点明两处来源。
- 空 `questions` 的 `AskState` 索引：工具层强制 1..=3，生产不可达；不加 `debug_assert!`（会让测试构造空面板时 panic，反而降低可用性），由 `row_count()`/`question()` 的 `Option` 兜底。

### 7.4 验证

| 命令 | 结果 |
|---|---|
| `cargo fmt` | 无 diff |
| `cargo clippy --all-targets -- -D warnings` | 零警告 |
| `cargo test`（全量） | **3307 lib + 16 个 target 全绿** |

本轮新增/改写用例 16 个，覆盖 §7.1–§7.3 的每一条修复；未验证部分仍为真实 TUI 手感（需用户 `cargo run`）。

## 8. 合并前追加（提示词重构 + 代理兼容）

ask 特性完成后，系统提示在本分支上又做了一轮独立改造，随本特性一起合并：

### 8.1 提示词按 codex 结构重构

- 章节对齐 codex `base_instructions/default.md`：personality / language / AGENTS.md /
  responsiveness / planning / task execution / validating / ambition vs precision /
  final message / tools / workspace；删掉 lx 不具备的能力（沙箱、审批、MCP、图片）。
- 新增「默认回复语言 = 用户当前输入的语言」规则（不自作主张切换、只有用户明确要求才改用）。
- 系统提示 3.6 KB → 7.8 KB；`PROMPT_VERSION` → `lx-4`；测试预算改为 12 KiB / 16 KiB。
- `AGENT_NAME = "lx"` 标注为不可变（不得由环境变量/命令行/设置项改写），并加测试锁定「提示首行由 `AGENT_NAME` 生成」。
- 规范落地：`AGENTS.md` 与 `docs/standards/code-writing-standards.md` 均写明「模型提示词一律英文」。

### 8.2 代理丢弃超长 system 消息（真实故障）

**现象**：用户问「你叫什么名字」，模型答「我是混元 / 我是一个 AI 助手」。

**根因**（逐层实测定位）：

1. lx 发出的系统提示 8983 字符，模型完全没收到；
2. 直连网关复现：system 长度 2000 正常、**2001 起 `prompt_tokens` 从 263 掉到 26**（只剩 user 消息）；
3. 源码定位：9router `open-sse/executors/codebuddy-cn.js` 的
   `if (text.length > 2000 || AGENT_PATTERN.test(text))` 把超长 system **整条替换**成
   `You are a helpful AI assistant that helps with software engineering tasks.`；
4. 上游已有 issue `decolua/9router#3339`（open），PR #3342（未合并）把阈值改成
   `CODEBUDDY_SYSTEM_PROMPT_MAX_LEN` 环境变量。

**踩过的两个坑（都已修正）**：

- **按字符数计数是错的**：代理是 JS 的 `text.length`，UTF-16 码元；emoji 计 2。
  实测 1000 个 emoji（2000 码元）保留、1001 个（2002 码元）被替换。
- **多段 system 对 Anthropic/Responses 无效**：代理会把多 `text` block 或多 system 项
  **拼回一条**再判长度；实测总长 2050 就失效。只有 OpenAI Chat 的多条 `messages` 会逐条判定。

**最终形状**（`chat::transport::SystemPromptParts`，对齐 opencode 的
`SystemPart` + `joinText` + `wrapSystemUpdate`）：

```text
system / instructions   You are lx, ... # How you work ## Personality ...      (≤1900 码元)
user #1                 <system-instructions>
                        ## Planning ... ## Task execution ... ## Tools ... 环境快照 ...
                        </system-instructions>
user (真实提问)          ...
```

- 首段进各自 transport 的原生 system 槽；其余规则**合并成一条** user 前置消息。
- 包裹内部做 XML 元字符转义，防止规则里的 `</system-instructions>` 提前闭合。
- 首段固定不含环境快照，逐轮稳定以命中上游 prompt cache。
- 端到端实测：三种 transport token 数完全一致（2071），身份与规则完整送达；
  6k 码元规则段（含 3000 个 emoji）也正常，确认 user 侧无长度 gate。

### 8.3 与三个参考实现的对照

| | 系统提示位置 | 长度限制 | 是否受 2000 gate |
|---|---|---|---|
| Codex | `developer` message（input 首项） | 一条 | 是（未适配） |
| opencode | `SystemPart[]`，按协议拆/拼 | 各 provider 自定 | 是（未适配） |
| Claude Code | `system` 多 `text` block | 多块 | 是（直连无此问题） |
| **lx** | 首段 system + 其余 `<system-instructions>` user 前置 | 首段 ≤1900 码元 | 否（已绕过） |

三家都默认直连官方 API，不面对中转代理的丢弃行为，因此都没有为此做适配；
lx 是唯一需要长期跑在 9router 后面的，必须自己解决。

### 8.4 验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | 无 diff |
| `cargo clippy --all-targets -- -D warnings` | 零警告 |
| `cargo test`（全量，合并后在 `dev` 上跑） | **3321 lib + 16 个 target 全绿** |

未验证：真实 TUI 手感（`cargo run`）；规则以 user 角色承载后的长期遵从度。

## 9. 修复：滚轮不该选中（对齐 `@` 提及面板）

**现象**：ask 面板里滚动滚轮，高亮跟着动，滚到哪就选到哪。

**根因**：`place_rows` 的滚动窗口是从高亮**推导**出来的
（`scroll = highlight_end - visible`），因此"让列表滚动"的唯一办法就是移动高亮——
滚轮被实现成 `move_highlight`，浏览和选择被合并成一个动作。

**参考实现**（`@` 提及面板 / `/` 命令面板的 `PanelWindow`）早就把三态分开了：

```rust
pub struct PanelWindow {
    pub active: usize,          // 高亮
    pub anchor: usize,          // 窗口底部锚点（跟随高亮）
    pub viewport: Option<usize>,// 滚轮建立的显式视口
}
```

- 键盘移动 → `active` 移动、`anchor` 跟随、`viewport = None`；
- 鼠标悬停 → 只改 `active`；
- **滚轮 → 只改 `viewport`**，高亮与锚点不动。

**修法**（`app/state/lx/ask.rs` 新增 `viewport: Option<usize>`）：

| 操作 | 高亮 | 视口 |
|---|---|---|
| 滚轮（选项区） | 不动 | `Some(start ± WHEEL_STEP)`，按 `total - visible` 钳制 |
| 滚轮（预览区） | 不动 | 只动 `preview_scroll` |
| ↑/↓/j/k、数字键 | 移动 | 清空 → 窗口重新跟随高亮 |
| 鼠标点选、切题、Confirm 切换 | 变更 | 清空 |
| 内容装得下（`total <= visible`） | 不动 | 不变，滚轮吞掉且不产生虚假位移 |

关键代码：

```rust
// 渲染层：显式视口优先，否则从高亮推导
let scroll = match ask.viewport() {
    Some(start) => start.min(total.saturating_sub(visible)),
    None => follow_highlight(),
};
```

```rust
// 事件层：几何由渲染层给出，事件层不重算
AskWheelZone::Options => ask.scroll_viewport(
    direction.saturating_mul(WHEEL_STEP), base, total, visible),
```

新增 `ui::main_content::lx::ask_list_window(area, lx) -> Option<(起点, 总行数, 可视行数)>`，
与绘制共用 `AskLayout::compute`，保证"滚一格"与"画到哪"永远是同一份几何。

**顺手修掉的一个边界**：高亮被滚出视口后按到边界方向键，`moved == false` 时若只在
`moved` 分支清视口，窗口不会回来——键盘看起来"完全没反应"。改为「移动或曾经有视口」
都清空并返回 `true`。

**测试**（4 个新增 / 2 个改写）：

- `wheel_preview_scrolls_and_options_scroll_the_list_without_moving_the_highlight`
- `keyboard_navigation_clears_the_explicit_viewport`
- `pressing_an_arrow_at_the_boundary_restores_the_follow_window`
- `explicit_viewport_wins_over_the_highlight_and_keeps_the_highlight_stable`（UI 层）
- `list_window_matches_the_rendered_geometry`（事件层基准与渲染同源）

**验证**：`cargo fmt` 无 diff、`clippy -D warnings` 零警告、`cargo test` **3325 lib + 16 target 全绿**。
