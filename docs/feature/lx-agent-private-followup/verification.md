# 验证报告：Agents 私聊 / 汇报 / steer（含 BTW / Agents 自然对话回归）

> 状态：**CLI 实机与自动化通过；TUI 交互项待用户 `cargo run` 复核**。
>
> 验证分支：`feat/btw-agent-natural-dialogue`
> 验证提交：`45dd6b17 test(lx): cover steer fallback when the turn settles first`
> 验证日期：2026-11-14
> 工作区：`/Users/yonah/projects/agent/lx-tui/.worktrees/btw-agent-natural-dialogue`
> 隔离库：`~/.lx-t-dev/db/worktrees/btw-agent-natural-dialogue-82e1f845/lx.db`
> 默认模型：`provider-alq374/cbcn/deepseek-v4.1-flash#medium`

## 1. CLI 实机验证

CLI 会话使用 `./target/debug/lx-tui exec` 创建，模型为默认模型；除 V04 为构造私聊读取链路外，均为真实模型轮次。

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| V01 | `Reply with exactly VERIFY-OK and nothing else.` | 执行 `./target/debug/lx-tui exec --json --color never "<输入>"`，再执行 `session show sess-18e7c5cf1526f9d0-0 --json` | 退出码 0；assistant 文本为 `VERIFY-OK`；会话模型为 `provider-alq374/cbcn/deepseek-v4.1-flash#medium` | PASS |
| V02 | 旧 v28 隔离库 + 一次任意 `exec` | 启动后执行 `sqlite3 <db> 'PRAGMA user_version;'` 与 `.schema agent_subagent_input` | 自动迁移到 v29；新增 `notify_parent INTEGER NOT NULL DEFAULT 1 CHECK (notify_parent IN (0,1))`；旧输入行默认 1 | PASS |
| V03 | `Use spawn_agent ... task 'Reply with exactly CHILD-OK' ... wait_agent ... send_input ... wait_agent` | 执行 `exec --json`，再执行 `session agents sess-18e7c5db0b061e20-0 --tree --json` 与 `session show ... --json` | 子 agent `sub-18e7c5db95aa1770-e`（`Turing / explorer / completed`）；子会话含两轮 user/assistant；退出码 0 | PASS |
| V04 | 将子 agent 的一条已完成输入行标记为 `notify_parent=0`，再让主 agent 调 `list_agents`（默认）与 `list_agents(include_private=true)` | `sqlite3` 更新 `input-18e7c5dd55152388-34` 后执行 `exec resume sess-18e7c5db0b061e20-0 --json "<输入>"` | 默认结果无 `private_tail`；`include_private=true` 结果含 `private_tail: user: PRIVATE-MARKER... assistant: ...` | PASS（修复后） |
| V05 | `Call list_agents once with no arguments ... include_private true ...`（对同一 session 的第二次 resume） | 执行 `exec resume sess-18e7c5db0b061e20-0 --json "<输入>"`，再执行 `session show sess-18e7c5db0b061e20-0 --json` | 复用同一 session id；3 轮共 6 条 user/assistant 消息；历史消息保留 | PASS |

> V04 首次执行失败：工具结果文本由 `format_subagent` 生成，原先只渲染 agent 元数据、丢弃 `private_tail`。修复提交 `db19e05c` 追加 `private_tail:` 段并补渲染测试，重跑通过。
>
> V04 的 `notify_parent=0` 行由隔离库注入，仅用于打通 `private_turn_tail → list_agents(include_private)` 读取链路；面板真实写入、报告抑制与 steer 由第 2 节单测覆盖。

## 2. 自动化验证

| # | 输入内容 / 场景 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| A01 | BTW 线程标题 `你好呀`，进入编辑态 | `cargo test --lib btw_edit_badge_shows_only_the_question` | 顶边框含 `QA 1 · 你好呀`，不含 `To:` | PASS |
| A02 | Agents run `Turing [executor]`，草稿 `hello`；切换私聊/汇报；运行中带 1 条 pending；窄窗格 | `cargo test --lib agent_composer_renders_target_badge_and_draft`、`agent_composer_marker_and_badge_stay_inside_narrow_rect` | 默认 `Turing [executor] · private`；切换后 `· report`；运行中 `· steer 1`；无 `To:`；窄窗格不越界、不截半截 | PASS |
| A03 | 模型配置有显示名；未知模型；状态行 | `cargo test --lib model_display_name_prefers_configured_name_and_falls_back_to_id`、`provider_name_is_not_rendered` | 优先显示配置名，缺失回退模型 id；状态行只显示模型名与上下文占用，不显示供应商 | PASS |
| A04 | Agents composer 输入 `@`，四 Tab 循环，采纳 Files/refDir/Skills/MCP；输入 `/`、粘贴路径、Markdown 块；切换 agent 后收到迟到扫描 | `cargo test --lib agent_composer_mention`、`agent_composer_inserts_plain_reference_text_for_every_tab`、`agent_composer_mention_panel_lists_four_tabs_without_subagents`、`late_mention_result_is_dropped_after_switching_agent` | 仅 Files/refDir/Skills/MCP，无 Subagents；插入朴素 `@path `（无 refer marker）；`/`、粘贴、Markdown 面板保持关闭；迟到结果被丢弃 | PASS |
| A05 | 面板 `SendSubagentUserInput` 与 `SteerSubagent` 效应经过持久化泵 | `cargo test --lib pump_defers` | 两个效应均保留给聊天层，不被泵吞掉 | PASS |
| A06 | 完成态 agent 默认模式、`Shift+Tab`、运行中按 Enter | `cargo test --lib shift_tab_switches_agent_reply_mode`、`enter_while_agent_running_emits_a_steer` | 默认 `private`；`Shift+Tab` 切 `report`；运行中发 `SteerSubagent`，不发新排队输入 | PASS |
| A07 | 面板私聊追问（`notify_parent=false`）→ 子轮完成 | `cargo test --lib private_panel_follow_up_stays_out` | 不生成 pending report；父对话不新增报告消息；`private_turn_tail` 与 `list_agents(include_private=true)` 可读到 user + assistant 文本 | PASS |
| A08 | 运行中子 agent 的 `pending_steers`，loop 请求 `SteerInputs` | `cargo test --lib running_subagent_steer_commits` | DB 新增 steer user/assistant 片段；handle 的 assistant/user/mutation 锚点前移；pending 消费；UI 与 DB 均出现 steer user 消息 | PASS |
| A09 | 轮次在 steer 被取走前结束（settle 竞态） | `cargo test --lib settle_moves_undelivered_steers_into_a_new_follow_up` | 未消费文本落成 `SendSubagentUserInput`；沿用本轮 `notify_parent=false`；pending 清空 | PASS |
| A10 | `list_agents(include_private=true)` 的模型可见文本 | `cargo test --lib subagent_list_renders_private_tail_only_when_present` | 仅带转录的条目输出 `private_tail:`，默认条目没有 | PASS |
| A11 | `list_agents` 空参数、`path_prefix`、`include_private` 与 schema | `cargo test --lib parse_list_supports_empty_and_path_prefix`、`subagent_schemas_declare_hierarchy_and_budget_fields` | 默认 `include_private=false`；显式 true 被解析；schema 暴露 boolean | PASS |
| A12 | 系统提示中的私聊约束与版本 | 检查 `LX_SYSTEM_PROMPT` 含 `private follow-ups ... not reported` 与“仅用户明确要求时查看”；`cargo test --lib prompt_documents_the_ask_tool` | 模型被明确告知私聊不自动汇报、不得主动窥探；`PROMPT_VERSION=lx-14` | PASS |
| A13 | 全量静态检查与测试 | `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` | 全部通过；lib `5382 passed / 0 failed / 6 ignored` | PASS |

## 3. TUI 手工验证（待用户）

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| M01 | `/btw` 后选一条 Q，`Shift+Enter` | 观察编辑态顶边框 | 左侧 `BTW`，右侧仅 `QA n · 标题`，任何宽度都无 `To:` | [ ] |
| M02 | 发送一条 BTW 问题，等待回答 | 观察统计行；重启后 `/resume` 再看同一线程 | 首段为模型显示名（非回答者身份）；live 与 reload 一致 | [ ] |
| M03 | 选中已完成 agent，`Shift+Enter` | 观察编辑态顶边框 | 左侧 `Subagent`，右侧 `Turing [executor] · private`，无 `To:` | [ ] |
| M04 | 在 Agents composer 输入 `@` | 左右切换 Tab；分别采纳四类条目；再输入 `/`、粘贴路径 | 仅 4 Tab 且无 `Subagents`；插入 `@path ` 朴素文本；`/` 与粘贴路径不弹面板；Esc 先关面板再退编辑 | [ ] |
| M05 | 私聊模式发送 `请只回复 PRIVATE-OK` | 等子 agent 完成，回主对话查看 | 子 agent 详情有回复；主对话无新报告卡片/自动轮；`list_agents` 默认看不到该内容 | [ ] |
| M06 | `Shift+Tab` 切到 `· report` 后发送同一问题 | 等子 agent 完成，回主对话查看 | 主对话收到报告卡片并触发自动轮 | [ ] |
| M07 | 在运行中 agent 的 composer 发送 `补充：先检查 src/db` | 观察胶囊与详情 | 胶囊为 `· steer`；消息在当前轮下一步注入；详情出现新 user 消息；pending 计数归零 | [ ] |
| M08 | 给两个 agent 分别设置 private/report，来回切换；重启应用 | 观察胶囊后缀 | 同一会话内每个 agent 保持自己的模式；重启后全部回到 private | [ ] |
| M09 | 把 agent 编辑态窗格从宽拖到窄 | 观察胶囊降级 | 先显示 `Turing [executor] · private`，放不下时降为 `Turing [executor]`，再放不下整块隐藏；不截半截 | [ ] |
| M10 | 主对话输入“看一下和 Turing 的私聊” | 观察主 agent 是否调用 `list_agents(include_private=true)` 并展示内容 | 工具卡片出现 `private_tail`；未明确要求时默认不读取 | [ ] |

## 4. 边界与备注

- `include_private` 是模型填写的工具参数，属于“不主动汇报”，不是硬隔离；系统提示要求仅在用户明确要求时查看。
- 私聊轮不生成报告，`wait_agent` 只返回状态；查看内容走 `list_agents(include_private=true)` 或 TUI 详情。
- 模式按 agent 存内存，重启复位为 private。
- CLI 会话写入当前 worktree 的隔离开发库；主 checkout 的 `~/.lx-t-dev/db/lx.db` 与生产库未受影响。
- 当前 `dev` 已前进到 `8c995980`（并行只读审查），合并本分支前需先处理 `src/chat/agent_loop` 等差异与可能的冲突。
