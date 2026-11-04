# Subagent 高级能力验证报告

以下 3 个端到端验证项贯通全部新增功能：自定义角色与 `subagents.json` 配置、模型与思考等级覆写、专属/默认昵称分配与大小写不敏感寻址、`fork_context` 上下文分叉克隆、`interrupt_agent` 独立中断与重派发。

---

## Item 1：自定义角色 + 模型/思考等级覆写 + 昵称池与大小写不敏感寻址

- **输入内容**：
  1. 配置文件 `~/.lx-t-dev/config/subagents.json`：
     ```json
     {
       "subagents": {
         "maxConcurrent": 3,
         "roles": [
           {
             "name": "reviewer",
             "baseRole": "explorer",
             "description": "Read-only code reviewer that starts its reply with [REVIEWER_OK].",
             "systemPrompt": "You are a read-only reviewer subagent. Always start your final response with [REVIEWER_OK].",
             "nicknameCandidates": ["Dijkstra"]
           }
         ]
       }
     }
     ```
  2. CLI Prompt：
     ```text
     Call spawn_agent once with role='reviewer', title='check role', model='cbcn/deepseek-v4.1-flash', reasoning_effort='low', task='Reply with [REVIEWER_OK] and nothing else.' Then call wait_agent with targets=['dijkstra'] (lowercase nickname) and return the result.
     ```
- **验证步骤**：
  1. 写入上述 `subagents.json`（或通过设置页 `Subagents` 页签保存）。
  2. 执行 `cargo run -- exec --ephemeral --json -m gemini/gemini-4-argon "<Prompt>"`。
- **期望结果**：
  - `spawn_agent` 成功派发 `role="reviewer"`，JSONL `collab_tool_call` 事件输出 `"nickname":"Dijkstra"`、`"model":"provider-alq374/cbcn/deepseek-v4.1-flash (low)"`。
  - `wait_agent(targets=["dijkstra"])` 通过小写昵称成功匹配 `Dijkstra` 并返回报告 `[REVIEWER_OK]`，且跨进程 `exec resume --last` 恢复该子 Agent 时仍保持绑定的 `cbcn/deepseek-v4.1-flash` 模型（验证通过）。

---

## Item 2：`fork_context=true` 上下文分叉继承

- **输入内容**：
  - Turn 1 Prompt：
    ```text
    Remember the codeword ALPHA-7392. Do not call any tools; reply only with ACK.
    ```
  - Turn 2 Prompt（`exec resume --last`，不在 `task` 中透传暗号）：
    ```text
    Call spawn_agent with role='explorer', title='read forked memory', fork_context=true, and task='What codeword was mentioned in our earlier conversation? Reply with the codeword only.' (Do NOT put the codeword in task). Then call wait_agent and report the subagent's answer.
    ```
- **验证步骤**：
  1. 执行 `cargo run -- exec --json -m gemini/gemini-4-argon "<Turn 1 Prompt>"` 完成首轮落库。
  2. 执行 `cargo run -- exec resume --last --json -m gemini/gemini-4-argon "<Turn 2 Prompt>"`。
- **期望结果**：
  - `spawn_agent(fork_context=true)` 克隆父会话已完成的第 1 轮历史，分配默认昵称 `Turing`。
  - 子 Agent 在 `task` 未包含暗号的情况下，从继承上下文中读取并返回 `ALPHA-7392`（验证通过）。

---

## Item 3：`interrupt_agent` 独立中断 + 昵称寻址 `send_input` 重定向

- **输入内容**：
  ```text
  1. Call spawn_agent with role='explorer', title='initial task', task='Read Cargo.toml and list dependencies.'
  2. Immediately call interrupt_agent with target='Turing'.
  3. Call send_input with target='Turing' and message='Do not read any files; reply only with [INTERRUPTED_AND_REDIRECTED].'
  4. Call wait_agent with targets=['Turing'] and report the final output.
  ```
- **验证步骤**：
  1. 执行 `cargo run -- exec --ephemeral --json -m gemini/gemini-4-argon "<Prompt>"`。
  2. 观察 JSONL 事件流中 `spawn_agent` → `interrupt_agent` → `send_input` → `wait_agent` 的状态流转。
- **期望结果**：
  - `interrupt_agent(target="Turing")` 立即中断首轮任务并保持子 Agent 处于可接收输入的活动态。
  - `send_input(target="Turing")` 直接复用同一 `agent_id` 执行新指令，`wait_agent(targets=["Turing"])` 正常结算并返回 `[INTERRUPTED_AND_REDIRECTED]`（验证通过）。
