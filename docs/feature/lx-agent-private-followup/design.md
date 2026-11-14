# Agents 面板私聊 / 汇报 / 轮内 steer

- **状态**：已完成（分支 `feat/btw-agent-natural-dialogue`），待用户实机验收。
- **关联**：BTW/Agents 自然对话（`docs/feature/lx-btw-agent-natural-dialogue/`）。

## 1. 目标

人类在 `Agents` 详情输入框里对子 agent 说话时，需要三种明确语义，而不是全部走“汇报主 agent”：

1. **私聊（默认）**：主 agent 不收到报告、不被唤醒；内容只留在该子 agent 的会话里。
2. **汇报**：沿用原来的逐轮报告与父会话自动轮。
3. **轮内 steer**：子 agent 正在输出时，消息注入当前轮，在下一个安全边界生效；不新起一轮。

模型自己的 `send_input` 工具保持原语义（汇报），不受面板模式影响。

## 2. 交互

- `Shift+Tab`（`LxInput::PrevStep`）切换当前 agent 的 `private` / `report` 模式；模式按 agent 保存，新建/首次使用默认 `private`。
- 胶囊后缀显示当前语义：
  - 空闲/完成：`Turing [executor] · private` / `· report`；
  - 运行中：`· steer`，有 pending 时为 `· steer N`。
- 运行中按 `Enter` 永远走 steer；`Shift+Tab` 只影响下一轮空闲时的模式。
- 窄窗格放不下后缀时先降级为 `Turing [executor]`，再整块隐藏；不截半截。

## 3. 私聊语义

- 持久化：`agent_subagent_input.notify_parent`（v29 迁移，默认 1 兼容旧库）。
- `notify_parent=0` 的轮次：
  - 子 agent 照常运行、落库、显示在 `Agents` 详情；
  - 结算时 **不生成** `ReportPayload`，不写 `agent_subagent_report`，`drain_reports` 无内容可投递；
  - 主 agent 的 `<subagents>` 快照只看到状态/标题，看不到正文。
- 主 agent 显式查看：`list_agents(include_private: true)` 为每个 agent 附带最近一次私聊的紧凑 transcript（user/assistant 文本，含轮内 steer，上限 4 KiB）。默认 `false`。
- 隐私边界说明：该参数由模型在工具调用里填写，因此“私聊”是**不自动进入主 agent 上下文**，不是密码学隔离；系统提示要求模型仅在用户明确要求时传 `include_private`。

## 4. 轮内 steer

- 面板提交运行中 agent 时发 `Effect::SteerSubagent`；事件层若找到 live handle，就把文本挂到该 run 的 `chat.pending_steers`，否则回退为普通追问（按当前模式）。
- 子轮 loop 在安全边界调用 `SteerInputs`；`event/chat/steer.rs::deliver_child`：
  1. 关闭当前 assistant 片段并取最终 parts；
  2. `Database::begin_steer_turn` 落库新 user/assistant 片段；
  3. 更新 `SubagentHandle.child_assistant_id` / `child_user_message_id` / mutation 归属；
  4. 同步事件侧 `handle.chat` 与 UI `run.chat`，消费 pending；
  5. 回执 `SteerInput` 给 loop。
- steer 不改变本轮的私聊/汇报策略：策略在轮次开始时冻结。
- 竞态兜底：轮次在 steer 被取走前结束，`settle` 把未消费的 pending steer 落成一条新的追问（沿用本轮 `notify_parent`），避免用户文本丢失。

## 5. 关键改动

| 文件 | 内容 |
|---|---|
| `src/db/migration.rs` / `src/db/mod.rs` | v29：`agent_subagent_input.notify_parent` |
| `src/db/chat/subagent/queue.rs` | enqueue/claim 携带 `notify_parent` |
| `src/db/chat/subagent.rs` | `private_turn_tail`（最近私聊 transcript） |
| `src/event/chat/subagent/{runner,launch,settle}.rs` | 策略透传、报告抑制、steer 竞态兜底、mutation Arc |
| `src/event/chat/subagent/panel.rs` | 面板私聊/汇报/steer 效应入口 |
| `src/event/chat/steer.rs` | `deliver_child` / `apply_steer_segment` |
| `src/app/state/lx/agent.rs` | `AgentReplyMode` 与 per-agent 模式 |
| `src/app/update/pane/task_page.rs` | Shift+Tab、running → steer、效果字段 |
| `src/ui/main_content/lx/task/agents.rs` | 胶囊模式/steer 后缀 |
| `src/chat/agent/{mod,parse}.rs`、`src/chat/tools/subagent.rs` | `list_agents.include_private` 与 `private_tail` |
| `src/chat/tools/output.rs` | 工具结果文本渲染 `private_tail`（模型实际读取的通道） |
| `src/chat/prompt.rs` | 提示模型不要主动窥探私聊；`PROMPT_VERSION=lx-14` |

## 6. 边界

- `BTW` 仍是“就主对话问一个旁路助手”，不读取子 agent 会话；与私聊不重叠。
- 私聊轮如果后续被模型 `send_input` 继续，后续正常轮的报告可能包含模型自己引用的私聊上下文；这是子 agent 的模型行为，不做硬隔离。
- `exec` 无头模式没有面板入口，`notify_parent` 恒为 true。
