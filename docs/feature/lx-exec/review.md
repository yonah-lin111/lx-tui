# lx Exec 无头 CLI 执行模式 — 代码评审报告（Review）

> 工作区：`.worktrees/lx-exec`（分支 `feat/lx-exec`，基线 `dev @ b8df8c0d`）
> 范围：CLI 入口与装配（`src/main.rs`、`src/chat/agent/exec/mod.rs`、`src/chat/agent/exec/cli.rs`）、无头宿主运行器、文件变更持久化桥与子 Agent 池（`src/chat/agent/exec/runner.rs`、`src/chat/agent/exec/mutation.rs`、`src/chat/agent/exec/subagent/{mod,turn,settle}.rs`）、事件与输出处理器（`src/chat/agent/exec/events.rs`、`src/chat/agent/exec/processor.rs`）、工具面与内存库（`src/chat/agent/mod.rs`、`src/chat/events.rs`、`src/db/mod.rs`）、拆分后的单元测试（`src/chat/agent/exec/tests/{cli_tests,processor_tests,events_tests,runner_tests,subagent_tests}.rs`）及特性文档（`design.md`、`task.md`）。
> 状态：已完成全部 P0–P3 问题定位、修复与回归测试验证，已合入主工作区 `dev` 分支。

---

## 0. 结论速览

| 严重度 | 数量 | 代表性问题 | 状态 |
| --- | --- | --- | --- |
| P0 | 3 | `JsonlOutputProcessor` 在 `on_tool_started` 与 `on_tool_finished` 中重复递增 `next_item_id` 导致同一工具调用 `item.id` 断裂；主轮 `AutoCompact` 仅打印告警未调用 `db.commit_compact` 落库检查点；冷启动在 Tokio runtime 创建前调用 `mcp_snapshot_for_turn` 导致 MCP 恒为空，且 `resume` 未水合历史轮次图片附件 | 已修复并锁回归 |
| P1 | 6 | 主轮与子 Agent 硬编码 `skills: None` / `skill_blocks: &[]` 且子 Agent 缺失环境与 `AGENTS.md` 指令；`HumanOutputProcessor` 多段文本拼接导致重复输出且 `final_message` 误取工具前说明；子 Agent `wait_agent(all)` 缺少 `auto_continue` 且 `send_input(interrupt=true)` 缺少中断续跑状态机；`events.rs` 子 Agent 工具参数名失配；`-C` 下相对 `-i` 路径与 worktree 隔离库解析偏差；`runner.rs` 与 `subagent.rs` 贴近 1000 行上限且使用 `#[rustfmt::skip]` 压行 | 已修复并锁回归 |
| P2 | 3 | `format_tool_outcome_line` 中 `write_stdin` 误用进程总生存时间、`web_search` 缺命中数、`wait_agent` 超时未标红；子 Agent 多轮历史回放缺失图片输入项；`-i "a.png, b.png"` 逗号分隔未 `trim()` 空白 | 已修复并锁回归 |
| P3 | 1 | 同步更新 `design.md` 与 `task.md` 的模块结构、核心代码片段与 Review 修复清单 | 已完成 |

---

## P0（阻断性缺陷：协议状态机断裂 / 检查点丢失 / 冷启动 MCP 与历史图片失效）

### P0-1 `JsonlOutputProcessor` 在 `on_tool_started` 与 `on_tool_finished` 中重复递增 `next_item_id`，导致同一工具调用的 `item.started` 与 `item.completed` 携带不同的 `item.id`

- **位置**：`src/chat/agent/exec/processor.rs:300-307, 336-344`、`src/chat/agent/exec/tests/processor_tests.rs:349-376`
- **关键点**：
  1. `design.md` §4.3.1 与 Codex `exec --json` 协议规定：同一工具调用的 `item.started` 与 `item.completed` 属于同一个 `ThreadItem` 的生命周期演变，必须保持相同的稳定 `item.id`；
  2. 实际代码中 `on_tool_started` 调用 `self.next_item(...)` 分配如 `item_1`，而 `on_tool_finished` 再次调用 `self.next_item(...)` 分配了新的 `item_2`；
  3. 单元测试 `processor_tests.rs:365` 将该错误行为固化为 `assert_eq!(item.id, "item_2")`。
- **错误原因**：
  `JsonlOutputProcessor` 未维护 `call.call_id -> item_id` 的在途映射，每次回调无条件自增 `next_item_id`，导致下游 JSONL 消费者看到 `item_1` 永远停留在 `in_progress`、而 `item_2` 未经启动凭空以 `completed` 出现。
- **修复方案**：
  1. 在 `JsonlOutputProcessor` 中维护 `active_tool_items: HashMap<String, String>`（`call.call_id -> item.id`）；
  2. `on_tool_started` 分配新 `item.id` 并存入 `active_tool_items`，`on_tool_finished` 按 `call.call_id` 取出并复用同一 `item.id`（无前置 `started` 时回退新分配）；
  3. 修正 `processor_tests.rs` 断言并补充并发多工具调用的 `item.id` 配对测试。

---

### P0-2 主轮 `ExecLoopEvent::AutoCompact` 仅打印告警未落库 `agent_compact_checkpoint`，且 `ExecHost::run_compact_summary` 为死代码

- **位置**：`src/chat/agent/exec/runner.rs:133-189, 262-268`
- **关键点**：
  1. 主轮触发自动上下文压缩时，`agent_loop::maybe_auto_compact` 生成摘要后通过 `host.notify_auto_compact(&notice)` 请求宿主持久化检查点；
  2. `runner.rs:262-268` 收到 `ExecLoopEvent::AutoCompact` 后只执行了 `processor.on_warning(...)` 便直接回执 `Ok(())`，**未调用 `db.commit_compact(...)` 将检查点写入 `agent_compact_checkpoint` 表**；
  3. `ExecHost` 上定义的 `run_compact_summary` 方法及 `model`/`stream_factory` 字段并不在 `AgentHost` trait 中，完全无调用方，靠 `#[allow(dead_code)]` 压制警告。
- **错误原因**：
  主轮事件循环漏掉了 `db.commit_compact` 持久化步骤，导致内存中的历史已被 `apply_auto_compact` 裁剪，但 SQLite 中无检查点记录，后续 `exec resume` 或 TUI `/resume` 恢复该会话时上下文与检查点断裂；同时残留了未清理的死代码。
- **修复方案**：
  1. 在 `runner.rs` 处理 `ExecLoopEvent::AutoCompact` 时，查询 `db.session_max_sequence(&self.session_id)`，构造 `CompactCommit` 并调用 `db.commit_compact(&self.workspace, &commit)`，落库失败时向 `reply` 返回 `Err(HostError::Failed)`；
  2. 删除 `ExecHost` 上的死方法 `run_compact_summary`、无用字段 `model`/`stream_factory` 及 `#[allow(dead_code)]`；
  3. 新增主轮自动压缩落库 `agent_compact_checkpoint` 的回归测试。

---

### P0-3 冷启动在 Tokio 运行时创建前调用 `mcp_snapshot_for_turn` 导致 MCP 恒为空，且 `resume` 恢复会话时未水合历史轮次图片附件

- **位置**：`src/chat/agent/exec/mod.rs:105, 228-230, 303-321, 522-535, 565-580`
- **关键点**：
  1. `run_main` 在第 303 行调用 `assemble_exec_turn`（内部调用 `mcp_snapshot_for_turn`），而 Tokio `runtime` 直到第 319 行才构建；`McpManager::prewarm` 首行检查 `tokio::runtime::Handle::try_current().is_err()` 直接提前返回，且即便有运行时，`prewarm` 为后台异步任务，紧接着同步调用 `cached_snapshot` 在单次 CLI 进程中必然拿到空快照；
  2. `assemble_exec_turn_with_snapshot` 仅调用 `hydrate_current_turn_images` 水合当前轮草稿图片，完全忽略了 `assembly.item_image_attachments` 中的**历史轮次图片附件**（未调用 `db.load_image_payloads`），导致 `exec resume` 续跑含图会话时历史 `ModelItem::UserContent.images` 为空，且切换到不支持图片的模型时也缺少降级提示文本。
- **错误原因**：
  将 TUI 常驻进程的异步后台 `prewarm + cached_snapshot` 直接套用到短生命周期 CLI 进程且放在了 Tokio 运行时之外；图片水合只处理了末尾当前轮，遗漏了历史轮次的附件加载。
- **修复方案**：
  1. 在 `run_main` 中将 Tokio `runtime` 提前构建，在运行时内通过 `manager.snapshot_for(&workspace, &servers, cancel_rx).await` 等待生效的 MCP server 完成握手（受各自 `startup_timeout_sec` 约束）并返回真实快照；
  2. 在 `assemble_exec_turn_with_snapshot` 中实现完整的 `hydrate_turn_images`：遍历 `assembly.item_image_attachments`，对历史轮次调用 `db.load_image_payloads(workspace, &committed.session_id, ...)` 水合图片字节（若当前模型不支持图片则降级为与 TUI 一致的 `ERROR: Cannot read "..."` 提示），对当前轮填入 `prepared.model_images`；
  3. 新增 `resume` 历史含图会话的水合与降级回归测试。

---

## P1（重要缺陷：Skills 与子 Agent 提示词缺失 / 正文重复输出与最终回复误判 / 子 Agent 状态机缺陷 / 路径基准与文件规模违规）

### P1-1 主轮与子 Agent 硬编码 `skills: None` / `skill_blocks: &[]`，且子 Agent 系统提示词缺失模型身份、环境快照与 `AGENTS.md`

- **位置**：`src/chat/agent/exec/mod.rs:208-211`、`src/chat/agent/exec/subagent/turn.rs`（`compose_subagent_prompt` 装配处，原 `subagent.rs:518-521`）
- **关键点**：
  1. `assemble_exec_turn_with_snapshot` 将 `AssemblyInput.skills` 硬编码为 `None`、`skill_blocks` 硬编码为 `&[]`，导致全局与项目 `SKILL.md` 目录及 `$skill` 点名注入在 `exec` 下完全失效；
  2. `subagent.rs` 调用 `compose_subagent_prompt` 时将 `model_identity`、`environment`、`instructions`（`AGENTS.md`）、`skills` 全数传 `None`，子 Agent 既不知道自身模型身份与工作区环境，也无法遵守项目 `AGENTS.md` 规范。
- **错误原因**：
  未将 `crate::skills`（纯文件 IO 模块）接入 `exec` 装配链路，且子 Agent 启动时漏传了 `PromptParts` 的核心上下文段。
- **修复方案**：
  1. 在 `src/chat/agent/exec/` 中封装无状态的 `skills_prompt_and_blocks(workspace, raw_prompt, is_subagent)`（复用 `crate::config::skills` 与 `crate::skills::{discover, render_catalog_with_omitted, load_skill_body, render_injection}`），为主轮装配 `skills` 目录与 `$skill` 引用块；
  2. 在子 Agent 装配 `compose_subagent_prompt` 时完整注入 `model_identity`、`environment_snapshot`、`load_instructions`（`AGENTS.md`）与 `skills` 目录。

---

### P1-2 `HumanOutputProcessor` 多段正文拼接导致 `stderr` 重复输出，且 `ExecRunner::final_message` 在工具后无新文本时误取工具前说明

- **位置**：`src/chat/agent/exec/processor.rs:174-185, 210-244`、`src/chat/agent/exec/runner.rs:406-412`
- **关键点**：
  1. `HumanOutputProcessor::on_assistant_text` 在已有 `pending_text` 时用 `\n\n` 拼接为 `"Part 1\n\nPart 2"`，而 `runner.final_message()` 只返回最后一段 `"Part 2"`；结算时 `pending != text` 触发 `write_intermediate`，导致 `"Part 2"` 在 `stderr` 与最终回复中被重复打印（管道模式下还会泄漏进 `stderr`）；
  2. `ExecRunner::final_message` 仅对 `self.parts` 倒序取任意 `PartKind::Text`，若模型在 Step 1 输出说明文字后调用工具、而在 Step 2 失败/中断或未再输出文本，Step 1 已作为中间说明打印到 `stderr` 的文本会被再次当作整轮 `final_message` 写入 `stdout` 与 `-o` 文件。
- **错误原因**：
  `on_assistant_text` 收到新闭合文本段时未先刷出前一段 `pending_text`；`final_message` 未限定只取最后一个工具调用之后的文本段。
- **修复方案**：
  1. `HumanOutputProcessor::on_assistant_text` 在 `self.pending_text.is_some()` 时先调用 `self.flush_pending_text()` 将前一段作为中间说明输出到 `stderr`，再缓存最新文本段；
  2. `ExecRunner` 记录 `last_tool_position: Option<i64>`，`final_message()` 仅在 `TurnOutcome::Complete` 且 `part.position > last_tool_position` 时提取最后一个非空 `PartKind::Text`。

---

### P1-3 子 Agent `wait_agent(all)` 缺少超时自动续等（`auto_continue`），且 `send_input(interrupt=true)` 缺少中断标记导致插队输入不自动调度

- **位置**：`src/chat/agent/exec/subagent/{mod,settle}.rs`（原 `subagent.rs:309-317, 623-629, 657-727`）
- **关键点**：
  1. TUI（`src/event/chat/subagent/wait.rs`）在 `targets` 为空（`all = true`）且 `mode == WaitMode::All` 时启用 `auto_continue`：单次 `timeout_ms` 到期但仍有子 Agent 运行时自动续等，不向模型返回 `timed_out: true`；而 `exec/subagent.rs` 直接向模型返回 `timed_out: true`；
  2. `send_input(..., interrupt = true)` 打断运行中子轮时未记录 `interrupted` 标记，而 `finish` 仅凭 `self.resumed.contains(agent_id)` 决定是否调度队列——既导致未调用 `resume_agent` 的活跃子 Agent 被 `send_input(interrupt=true)` 打断后新输入永远滞留在 `queued` 不启动，又导致被 `close_agent` 取消的 `resumed` 子 Agent 错误地继续消费队列。
- **错误原因**：
  遗漏了 TUI `subagent/wait.rs` 的 `auto_continue` 续等逻辑与 `subagent/settle.rs` 的 `interrupted_subagents` 状态机分支。
- **修复方案**：
  1. 在 `PendingWait` 中记录 `auto_continue: bool`（`all && mode == WaitMode::All`）与 `timeout_ms: u64`；`handle_wait_ready` 中若 `timed_out && pending.auto_continue` 且仍有目标在途，则自动重新派生等待任务；
  2. 在 `ExecSubagentPool` 中新增 `interrupted: HashSet<String>`：`send_input(interrupt=true)` 打断运行中子轮时插入 `interrupted`（`close` 时移除）；在 `finish` 中若 `was_interrupted_by_input` 为真则置 `resumed` 并立即调度队首输入，若因其它原因 `Interrupted` 则移除 `resumed`。

---

### P1-4 `events.rs` 子 Agent 控制工具参数字段名失配（`target`/`id`/`targets`）且 `wait_agent` 多 Agent 结果标识丢失

- **位置**：`src/chat/agent/exec/events.rs:171-177, 346-357, 512-538`
- **关键点**：
  1. `send_input`、`close_agent` 的参数字段为 `"target"`，`resume_agent` 为 `"id"`，`wait_agent` 为 `"targets"`，`cancel_agent_input` 为 `"submission_id"`，但 `item_details_for_start` 与 `format_tool_header` 只读取 `"agent_id"`/`"role"`/`"title"`，导致除 `spawn_agent` 外的 6 个子 Agent 工具在 `item.started` 中 `agent_id` 恒为 `null`，人类模式标题也缺失目标标识；
  2. `collab_identity` 处理 `SubagentOutcome::Waited { agents, .. }` 时只取 `agents.first()`，丢弃其余已完成子 Agent 的标识与标题。
- **错误原因**：
  未按 7 个子 Agent 控制工具各自的 schema 字段提取目标标识，也未聚合多目标 `Waited` / `Listed` 的标识信息。
- **修复方案**：
  新增 `collab_start_identity(name, &arguments)` 按工具名兼容提取 `"agent_id"` / `"target"` / `"id"` / `"targets"` / `"submission_id"`，并在 `format_tool_header` 与 `collab_identity` 中支持多目标合并展示（如 `sub-1, sub-2` 或 `(all)`）。

---

### P1-5 `-C/--cd` 下相对路径 `-i/--image` 解析基准错误、worktree 隔离库推导偏差，以及新会话锁异常时遗留 `streaming` 记录

- **位置**：`src/chat/agent/exec/mod.rs:256-318, 538-562`、`src/main.rs:49, 85-90`
- **关键点**：
  1. `load_draft_images` 直接用原始 `path`（相对进程启动目录）而非解析后的 `cwd`（`-C <DIR>`）加载图片；
  2. `main.rs` 传入的 `database_worktree()` 固定检查 `Path::new(".")`，当用户在主仓库通过 `lx-tui exec -C .worktrees/xxx` 指向某个 linked worktree 时，会误连主库而非目标 worktree 的隔离库；
  3. 新建会话时，`assemble_exec_turn` 落库 `streaming` 消息后若 `lock_session` 失败直接 `?` 退出，数据库会残留 `streaming` 状态的半轮记录。
- **错误原因**：
  `-i` 路径与 `linked_worktree` 未以解析后的 `cwd` 为基准；新会话锁失败分支未调用 `db.finish` 兜底结算。
- **修复方案**：
  1. `load_draft_images(cwd, image_paths, model)` 将相对路径统一按 `cwd.join(path)` 解析；
  2. 在 `run_main` 中当 `linked_worktree` 为 `None` 且 `crate::db::is_dev_environment()` 为真时，按解析后的 `cwd` 调用 `crate::git::linked_worktree_root(&cwd)` 推导目标 worktree 隔离库；
  3. 若新会话 `lock_session` 失败，先调用 `db.finish` 将刚创建的 `assistant_id` 结算为 `TurnOutcome::Failed` 再返回错误。

---

### P1-6 `runner.rs`（958 行）与 `subagent.rs`（987 行，含 `#[rustfmt::skip]` 压行）逼近/规避生产代码 1000 行上限

- **位置**：`src/chat/agent/exec/runner.rs`、`src/chat/agent/exec/subagent.rs`（987 行，已拆分至 `src/chat/agent/exec/subagent/{mod,turn,settle}.rs`）、`src/chat/agent/exec/mutation.rs`
- **关键点**：
  1. `AGENTS.md` 规定“生产代码单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右”；
  2. `subagent.rs` 第 166 行使用 `#[rustfmt::skip]` 将多处代码压成单行以凑在 987 行（标准 `rustfmt` 展开后超过 1200 行），`runner.rs` 也达 958 行。
- **错误原因**：
  单文件承载过多职责（`runner.rs` 混放主循环与文件 mutation recorder 桥；`subagent.rs` 混放池控制、子轮上下文/图片归档与等待结算），并用 `#[rustfmt::skip]` 规避行数上限。
- **修复方案**：
  1. 移除全部 `#[rustfmt::skip]`；
  2. 将 `runner.rs` 中的文件变更持久化桥拆分为 `src/chat/agent/exec/mutation.rs`（`ExecMutationEvent` 与 `MutationRecorderFactory`）；
  3. 将 `subagent.rs` 按职责拆分为 `src/chat/agent/exec/subagent/mod.rs`（池状态与控制入口）、`subagent/turn.rs`（子轮上下文装配、四段提示词注入、图片归档与历史图片回放）与 `subagent/settle.rs`（`wait_agent` 超时续等 `auto_continue`、子轮终态结算与批量关闭）；
  4. 将膨胀的测试按被测模块拆分到 `src/chat/agent/exec/tests/{fixtures,events_tests,subagent_tests}.rs`（与 `cli_tests.rs`、`processor_tests.rs`、`runner_tests.rs` 并列），确保全部生产与测试文件严格 < 1000 行。

---

## P2（健壮性与细节改进）

### P2-1 `format_tool_outcome_line` 中 `write_stdin` 误用进程总生存时间、`web_search` 缺少结果数、`wait_agent` 超时未标错误

- **位置**：`src/chat/agent/exec/events.rs:381, 460-476, 540-578`
- **关键点**：
  1. `write_stdin` 返回 `ToolExecutionResult::Process(process)` 时，`process_summary_line` 使用了进程自启动以来的 `process.wall_time_ms` 而非本次 `write_stdin` 调用耗时 `duration_ms`；
  2. `web_search` 结算行仅重复打印查询词 `Searched "..."`，未包含命中条数 `result.num_results`；
  3. `wait_agent` 超时（`SubagentOutcome::Waited { timed_out: true, .. }`）时 `is_error` 返回 `false`，未按错误高亮。
- **修复方案**：
  `write_stdin` 按本次动作与 `duration_ms` 格式化摘要；`web_search` 结算行附带结果条数（如 `Searched "..." · N results (Xms)`）；`wait_agent` 超时时将 `is_error` 设为 `true`。

---

### P2-2 子 Agent 跨轮历史回放未加载历史图片输入项（`load_subagent_turn_items`）

- **位置**：`src/chat/agent/exec/subagent/turn.rs`（`child_context` 历史装配，原 `subagent.rs:482-513`）
- **关键点**：
  `child_context` 在构建多轮子 Agent 历史时只提取了 `ChatMessage` 的纯文本，未调用 `db.load_subagent_turn_items` 重放历史轮次中的图片输入项。
- **修复方案**：
  在子 Agent 历史装配中查询 `db.load_subagent_turn_items(&self.workspace, &self.parent_session_id, agent_id)`，对含图轮次构造 `ModelItem::UserContent`（模型不支持图片时降级为可读提示文本）。

---

### P2-3 CLI `-i "a.png, b.png"` 逗号分隔未 `trim()` 空白，且 `resume --last` 冲突位置参数校验可收敛

- **位置**：`src/chat/agent/exec/cli.rs:350-357, 460-468`
- **关键点**：
  1. `split_images` 检查了 `part.trim().is_empty()`，但构造 `PathBuf::from(part)` 时未调用 `part.trim()`，导致 `-i "a.png, b.png"` 第二张图路径带前导空格；
  2. 当用户执行 `lx-tui exec resume --last sess-xxx`（误把 `--last` 与 `sess-` 开头的会话 ID 同时传入）时，直接将 `sess-xxx` 当作了提示词。
- **修复方案**：
  `split_images` 使用 `PathBuf::from(part.trim())`；当 `resume.last` 为真且第一个位置参数以 `sess-` 开头时，直接报错 `cannot specify both '--last' and a session ID`。

---

## P3（文档同步与测试补齐）

### P3-1 同步更新 `design.md`、`task.md` 与目录结构标准

- **位置**：`docs/feature/lx-exec/design.md`、`docs/feature/lx-exec/task.md`、`docs/standards/project-directory-structure.md`
- **关键点**：
  同步更新 `design.md` 中的模块拆分结构（`mutation.rs`、`subagent/{mod,turn,settle}.rs`、`tests/{events_tests,subagent_tests}.rs`）、JSONL `active_tool_items` 映射、主轮 `AutoCompact` 落库、`snapshot_for` 同步等待、历史图片水合与 Skills 注入；在 `task.md` 中新增 Phase 7（Review 修复项）并更新验证结果；在 `project-directory-structure.md` 的 `agent/exec/` 条目补齐 `cli.rs`、`events.rs`、`processor.rs`、`runner.rs`、`mutation.rs` 与 `subagent/`，并确认全部生产与测试文件严格 < 1000 行。
