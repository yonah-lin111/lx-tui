# lx Exec — 验证报告

## 1. 管道输入合成、`stdout`/`stderr` 分流与 `-o` 落盘

- **输入内容**：
  ```bash
  printf "world\n" | cargo run -- exec -C . -o /tmp/lx_last.txt "Reply with only: hello <stdin content>" > /tmp/lx_out.txt 2> /tmp/lx_err.txt
  ```
- **验证步骤**：
  1. 执行上述命令并检查退出码 `echo $?`。
  2. 查看 `/tmp/lx_out.txt`、`/tmp/lx_last.txt` 与 `/tmp/lx_err.txt`。
- **期望结果**：
  - 退出码为 `0`。
  - `/tmp/lx_out.txt` 与 `/tmp/lx_last.txt` 仅含最终回复文本（如 `hello world`），无横幅或进度日志。
  - `/tmp/lx_err.txt` 包含 `lx-tui exec` 启动横幅（`workdir`/`model`/`session id`）、思考步骤与 `tokens used` 统计。

## 2. `--json` 事件流、工具调用链与子 Agent 协同（无 `ask` 挂起）

- **输入内容**：
  ```bash
  cargo run -- exec --json "Use spawn_agent(explorer) and wait_agent to read Cargo.toml package name, then run 'echo ok' via exec_command and summarize."
  ```
- **验证步骤**：
  1. 在终端运行命令，观察 `stdout` 逐行输出的 JSONL。
  2. 检查事件类型序列与工具执行状态。
- **期望结果**：
  - `stdout` 每行均为合法 JSON，按序输出 `thread.started` → `turn.started` → `item.started`/`item.completed`（含 `collab_tool_call`、`command_execution`、`agent_message`）→ `turn.completed`。
  - 全程不触发交互式 `ask` 阻塞，子 agent 报告正常返回主轮并以退出码 `0` 结束。

## 3. 会话持久化、`exec resume --last` 续跑与 `--ephemeral` 无痕模式

- **输入内容**：
  ```bash
  cargo run -- exec "Remember codeword: PINEAPPLE-42"
  cargo run -- exec resume --last "What is the codeword?"
  cargo run -- exec --ephemeral "Temporary run: BANANA-99"
  ```
- **验证步骤**：
  1. 依次执行第 1、2 条命令，验证跨轮上下文记忆。
  2. 执行第 3 条 `--ephemeral` 命令后，再执行 `cargo run -- exec resume --last "What is the codeword?"` 或打开 `lx-tui` 查看 `/resume` 列表。
- **期望结果**：
  - 第 2 条命令准确回复 `PINEAPPLE-42`。
  - 第 3 条 `--ephemeral` 运行不写入磁盘数据库，再次 `resume --last` 仍指向 `PINEAPPLE-42` 会话，TUI `/resume` 中无 `BANANA-99` 记录。

## 4. `Ctrl+C` 优雅中断与错误退出码

- **输入内容**：
  ```bash
  cargo run -- exec --ephemeral --resume
  cargo run -- exec "Run 'sleep 30' via exec_command"
  ```
- **验证步骤**：
  1. 运行第 1 条冲突参数命令，检查 `echo $?`。
  2. 运行第 2 条命令，待 `stderr` 显示 `■ exec_command` 启动后按 `Ctrl+C`，检查 `echo $?` 与残留进程。
- **期望结果**：
  - 第 1 条命令在 `stderr` 报错并以退出码 `1` 退出。
  - 第 2 条命令按 `Ctrl+C` 后立即终止后台子进程，输出 `turn interrupted`，DB 状态记为 `interrupted`，退出码为 `130`。
