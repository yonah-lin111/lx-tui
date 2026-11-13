# 任务清单：Agents 面板私聊 / 汇报 / steer

- **状态**：T1–T6 已完成，待 `cargo run` 实机验收后合并。

| 任务 | 内容 | 状态 |
|---|---|---|
| **T1** | DB v29：`notify_parent` 列、enqueue/claim 透传、`private_turn_tail` 查询 | 已完成 |
| **T2** | 事件层：私聊报告抑制、面板私聊/汇报入口、steer 竞态兜底 | 已完成 |
| **T3** | 子 agent 轮内 steer：`deliver_child`、片段切换、mutation 锚点、pending 消费 | 已完成 |
| **T4** | App/UI：`AgentReplyMode`、Shift+Tab、胶囊后缀、running→steer | 已完成 |
| **T5** | `list_agents(include_private)`：schema/解析/结果/提示词 | 已完成 |
| **T6** | 测试与校验：单元/事件/渲染/迁移回归，`fmt` / `clippy` / 全量 `test` | 已完成 |

## 测试覆盖

- `pump_defers_panel_subagent_input_to_the_chat_layer` / `pump_defers_subagent_steer_to_the_chat_layer`：效应不被持久化泵吞掉。
- `private_panel_follow_up_stays_out_of_the_parent_and_is_readable_on_request`：私聊不产生 pending report、不新增父消息；`private_turn_tail` 与 `list_agents(include_private)` 能读到。
- `running_subagent_steer_commits_a_new_segment_and_returns_input`：DB 切片段、handle 锚点前移、pending 消费、UI 出现 steer 消息。
- `shift_tab_switches_agent_reply_mode_and_report_submit_carries_the_flag`：默认私聊、Shift+Tab、汇报标志。
- `enter_while_agent_running_emits_a_steer_instead_of_a_new_turn`：运行中不发新排队输入。
- `agent_composer_renders_target_badge_and_draft`：`· private` / `· report` / `· steer 1` 胶囊。
- `list_agents` schema/parse 测试。

## 验证结果（2026-11-14，本 worktree）

- `cargo fmt`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：5380 passed / 0 failed。

## 验收要点

1. 完成态子 agent：`Shift+Enter` 打开输入框，胶囊为 `· private`；发送后主 agent 不应出现报告轮。
2. `Shift+Tab` 变 `· report`；发送后主 agent 照旧收到报告并自动回复。
3. 运行态子 agent：胶囊为 `· steer`；发送后消息在下一步注入当前轮（详情出现新的 user 消息，pending 计数消失）。
4. 主 agent 主动调用 `list_agents(include_private: true)` 时能看到最近私聊；默认调用看不到。
