# Codex 内置 Agent Tool 补齐验证报告

本文档提供最少数量、端到端贯通本次全部新增与扩展能力（`curr_time`、`spawn_agent.items`、`send_input`、`resume_agent`、`list_agents`、`cancel_agent_input`、图片严格归档与多轮独立报告）的验证项。

---

## 验证项 1：实时 UTC 时间与结构化图文派发（贯通 `curr_time` + `spawn_agent.items` + `list_agents` + `wait_agent`）

- **输入内容**：
  在工作区准备一张测试图片（如 `test.png`），在 lx 聊天框输入：
  ```text
  先调用 curr_time 获取当前 UTC 时间，再用 spawn_agent(role="explorer") 通过 items 传入两段文字和本地图片 test.png（顺序：文字1 -> test.png -> 文字2）让子 agent 描述图片与任务，接着调用 list_agents 查看状态，最后用 wait_agent 等待结果。
  ```
- **验证步骤**：
  1. 发送上述提示词，观察主对话时间线中的 `curr_time`、`spawn_agent`、`list_agents`、`wait_agent` 工具卡片。
  2. 在子 agent 启动后删除工作区原图 `test.png`，观察子 agent 执行与历史回放是否受影响。
  3. 打开 `Background -> Agents` 面板，查看该子 agent 的状态、短标题与详情时间线。
- **期望结果**：
  1. `curr_time` 以严格空对象 `{}` 调用成功，卡片与结果返回 `YYYY-MM-DD HH:MM:SS UTC` 实时时间。
  2. `spawn_agent` 接收 `items`（文本 → 本地图片 → 文本交错顺序）成功返回 `agent_id`，图片已归档至 `~/.lx-t-dev/images/<child_session_id>/`，原图删除后仍可正常读取与回放。
  3. `list_agents` 返回当前父会话下的 `agent_id`、`role`、`title`、`status`、`queued_count`。
  4. `wait_agent` 正常等待子轮结束并恰好一次领取首轮报告正文。

---

## 验证项 2：运行中持久排队、插队中断与排队取消（贯通 `send_input` + `interrupt` + `cancel_agent_input` + 多轮报告）

- **输入内容**：
  在 lx 聊天框输入：
  ```text
  用 spawn_agent 启动一个 explorer 分析 src/chat 目录；在它运行期间：
  1. 调用两次 send_input(interrupt=false) 排入两条后续任务（其中一条用 items 带 data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg== 内联图片）；
  2. 用 cancel_agent_input 取消第 2 条排队任务的 submission_id；
  3. 调用 send_input(interrupt=true) 发送一条高优插队任务，然后用 wait_agent 等全部完成。
  ```
- **验证步骤**：
  1. 发送提示词，观察 `send_input` 返回的 `submission_id`、`status="queued"` 与 `queued_count` 变化。
  2. 打开 `Background -> Agents` 面板，观察列表行状态中的排队计数 `running (+Nq)`。
  3. 观察 `cancel_agent_input` 的返回状态，以及 `interrupt=true` 触发后子会话的多轮消息与最终报告数。
- **期望结果**：
  1. 两条普通 `send_input(interrupt=false)` 均持久入队，`data:` URL 图片被解码校验并归档，工具文本结果与 UI 卡片均不泄露 base64 原文。
  2. `cancel_agent_input` 将第 2 条 `queued` 输入转为 `cancelled`（重复调用幂等），队列剩余数减少。
  3. `send_input(interrupt=true)` 取消当前正在运行的第 1 轮（保留已有输出但**不生成**第 1 轮完成报告），封口后优先执行插队任务，再继续执行剩余的第 1 条普通排队任务；两个正常完成的子轮各自生成独立报告。

---

## 验证项 3：终态/跨重启显式恢复与缺图阻断自愈（贯通 `resume_agent` + `blocked` 阻断 + 自动压缩检查点回放）

- **输入内容**：
  在子 agent 已处于终态（`completed` / `failed` 或重启应用后）的同一会话中输入：
  ```text
  1. 先不调用 resume_agent，直接对该 agent_id 调用 send_input(message="直接发送测试")；
  2. 被拒绝后，调用 resume_agent(id=agent_id) 恢复该子 agent，再调用 send_input 继续追问上一轮的结论。
  ```
  （补充缺图阻断验证：将某条 `queued` 图片输入在 `~/.lx-t-dev/images/<child_session_id>/` 下的归档文件移除后触发消费，再调用 `list_agents` 与 `cancel_agent_input`）。
- **验证步骤**：
  1. 观察终态未恢复时直接调用 `send_input` 的返回结果。
  2. 观察调用 `resume_agent` 后的状态（无排队项时进入 `idle`），以及随后调用 `send_input` 的多轮上下文衔接表现。
  3. 在归档文件缺失场景下，观察 `list_agents` 和 `Agents` 面板状态，并用 `cancel_agent_input` 取消该阻断项后重新发送有效输入。
- **期望结果**：
  1. 终态或重启后未调用 `resume_agent` 直接 `send_input` 明确报错 `subagent is not active; call resume_agent first`，且重启不会自动偷跑任何子 agent。
  2. `resume_agent` 成功后恢复原 `child_session_id` 的历史消息与持久化压缩检查点（按 `system -> environment -> compact summary -> 后续历史` 顺序装配），后续 `send_input` 在同一子会话下继续追加轮次。
  3. 归档图片丢失或篡改时，队首输入保持 `queued` 并标记 `error_code="image_missing"`，对外与 UI 投影为 `blocked`，不降级为纯文本、不越过队首；通过 `cancel_agent_input` 取消后即可继续正常发送。
