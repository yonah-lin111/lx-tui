# lx Agent 系统提示词（AGENTS.md）注入与模型切换 — 评审记录

- 评审对象：`feat/lx-agent-prompt-and-model-switch`（基线 `dev` / `f3f54bb7`）
- 评审范围：`git diff f3f54bb7...HEAD` 涉及的全部文件（`src/chat/agent/`、`src/chat/prompt.rs`、`src/chat/context/`、`src/db/`、`src/app/`、`src/event/chat/`、`src/ui/main_content/lx/history/` 及 `docs/feature/lx-agent-prompt-and-model-switch/`）
- 记录口径：**只收会导致测试崩溃、提示词漏注入、数据不一致、上下文超窗、子 Agent 行为违背不变量、TUI 渲染/交互异常、文件行数超限与文档漂移的真实问题**；风格偏好一律不收
- 修复后状态：**P0（2 项）、P1（6 项）、P2（3 项）共 11 项问题已全部修复并补充回归测试锁死**

---

## 0. 结论速览

| 严重度 | 数量 | 代表性问题 | 状态 |
| --- | --- | --- | --- |
| P0 | 2 | `subagent/tests.rs:559` 遗漏 `SCHEMA_VERSION` 断言更新导致全量 `cargo test` 必现失败；`finish_model_selection` 与 `Effect::SetChatSessionModel` 割裂导致落库的 `injected_prompt` 缺失 `core/agents_md` 及指令文件来源，且 `service.rs` 探测 `AGENTS.md` 时丢失窗格真实 `cwd` | 已修复并锁回归 |
| P1 | 6 | `compact.rs` 与 `model_switch.rs` 展开态缺失思考块同款 `  └─ ` 树形前缀与 5 列缩进；连续切换合并时卡片展开态被重置关闭且丢失中间 `updated_sources`、`A -> B -> A` 显示异常；父会话切模型后唤醒的子 Agent 仍沿用旧模型；上下文预算漏扣系统提示词且裁剪丢弃首轮前 `<system-update>`；多窗格/`/resume`/`/rewind` 模型状态未同步；`db/service.rs` 与 `app/update/chat/mod.rs` 超出 1000 行上限 | 已修复并锁回归 |
| P2 | 3 | `collect_search_dirs` 未做软链规范化比对且 `AGENTS.md` 预算截断无显式标记；`model_switch_block_signature` 缓存签名未覆盖合并区间与 `is_variant_only`；缺少 v21→v22 迁移测试及 `design.md`/`task.md` 文档签名漂移 | 已修复并锁回归 |

---

## P0（全量测试阻断 / 核心提示词与持久化数据分裂）

### P0-1 `src/db/chat/subagent/tests.rs:559` 硬编码版本断言导致全量 `cargo test` 必现失败

- **位置**：`src/db/chat/subagent/tests.rs:559`
- **关键点**：
  1. 本分支新增 `agent_model_switch` 表并将 `src/db/mod.rs` 的 `SCHEMA_VERSION` 推进至 `22`；
  2. 同文件第 825 行已使用 `assert_eq!(version, crate::db::SCHEMA_VERSION_FOR_TESTS)`，但第 559 行 `v18_upgrade_migrates_legacy_reports_and_creates_queue_tables` 仍硬编码 `assert_eq!(version, 20)`。
- **错误原因**：
  数据库打开并执行全量迁移后 `PRAGMA user_version` 为 `22`，与硬编码的 `20` 不一致，导致运行全量 `cargo test` 时该测试必现 Panic。
- **修复**：
  将 `src/db/chat/subagent/tests.rs:559` 的版本断言统一改为 `assert_eq!(version, crate::db::SCHEMA_VERSION_FOR_TESTS);`。
- **回归测试**：`db::chat::subagent::tests::v18_upgrade_migrates_legacy_reports_and_creates_queue_tables`。

---

### P0-2 `finish_model_selection` 与 `Effect::SetChatSessionModel` 割裂，导致落库的 `injected_prompt` 缺失 `core/agents_md` 与指令来源，且 `service.rs` 探测 `AGENTS.md` 丢失窗格 `cwd`

- **位置**：`src/app/update/chat/session.rs:696-746`、`src/db/service.rs:1084-1108`、`src/db/chat/model_switch.rs:42-56`
- **关键点**：
  1. `finish_model_selection` 在纯状态层调用 `compute_updated_sources(..., false)`（硬编码 `has_instructions = false`）与 `render_model_switch_update(..., &[])`（硬编码 `instruction_sources = &[]`），生成了不含 `core/agents_md` 和文件来源的 `injected_prompt`；
  2. `Effect::SetChatSessionModel` 在 `service.rs` 中调用 `load_instructions` 后，仅向 `commit.updated_sources` 追加了 `"core/agents_md"`，**并未重新渲染 `commit.injected_prompt`**；
  3. 同时，`service.rs:1089` 传给 `load_instructions` 的 `project_root` 与 `cwd` 均使用了主仓库根 `workspace`（而非窗格实际工作目录 `workspace_cwd_for_pane`），导致 worktree 或子目录独有的 `AGENTS.md` 在切换时完全不可见。
- **错误原因**：
  提示词渲染发生在无磁盘访问能力的 `app::update` 层，而 `AGENTS.md` 探测发生在 `db::service` 层，事后只改写了 `updated_sources` 列表而未重算 `injected_prompt`。这直接导致数据库行中 `updated_sources_json` 含 `"core/agents_md"`，但展开卡片与实际注入下一轮 `<system-update>` 的 `injected_prompt` 却缺失 `core/agents_md` 与 `Active instruction files:`。
- **修复**：
  1. 在 `ModelSwitchCommit` 中携带 `cwd: Option<PathBuf>` 与 `to_identity: crate::chat::agent::ModelIdentity`；
  2. 在 `Effect::SetChatSessionModel` 处理时，使用窗格实际 `cwd`（回退 `workspace`）与 `project_root` 加载 `LoadedInstructions`；
  3. 若 `!loaded_inst.is_empty()`，补齐 `core/agents_md` 后调用 `render_model_switch_update(commit.from_label.as_deref(), &commit.to_identity, &commit.updated_sources, &loaded_inst.source_labels())` 重新生成权威 `injected_prompt`，再落库并通过 `update::set_switches` 刷新窗格状态。
- **回归测试**：`db::service::resume_tests::set_chat_session_model_injects_agents_md_sources_into_prompt_and_syncs_panes`。

---

## P1（TUI 渲染与交互状态缺陷 / 上下文与子 Agent 不变量违背 / 文件超限）

### P1-1 `compact.rs` 与 `model_switch.rs` 展开态缺失思考块同款 `  └─ ` 树形前缀与 5 列缩进

- **位置**：`src/ui/main_content/lx/history/compact.rs:19-27, 82-93`、`src/ui/main_content/lx/history/model_switch.rs:30-44, 87-96`
- **关键点**：
  1. `docs/standards/tui-design-requirements.md` 与 `design.md` §3.6.2 明确要求：`compact` 与 `model_switch` 展开态左起空 2 列画 `└─ `（`text::agent::TREE_LEAF`，共 5 列，`style::chat_notice_connector()`），正文及软折行续行统一对齐第 5 列（`thinking_body_indent(width)`）；
  2. 实际代码中 `compact.rs` 未做修改，`model_switch.rs` 展开后也直接从 `content.x` 顶格按全宽 `content.width` 渲染压暗 Markdown，既无 `  └─ ` 前缀也无 5 列缩进。
- **错误原因**：
  实现阶段仅将 `model_switch.rs` 接入了 `tool::muted_markdown_lines`，遗漏了树形分支几何缩进（`tree_body_geometry`）与首行连接符绘制，也漏改了 `compact.rs`。
- **修复**：
  在 `compact.rs` 与 `model_switch.rs` 中统一按 `thinking_body_indent(width)` 计算 `indent` 与 `body_width = width.saturating_sub(indent)`：
  1. `rows()` 按 `body_width` 计算展开态行高；
  2. `draw()` 在展开态首行绘制 `style::chat_notice_connector()` 样式的 `text::agent::TREE_LEAF`（`"  └─ "`），并将展开态各行右移 `indent` 列以 `body_width` 绘制。
- **回归测试**：`ui::main_content::lx::history::tests::model_switch::expanded_renders_tree_leaf_prefix_and_indented_muted_markdown`。

---

### P1-2 连续切换合并（`-- Model switched xN --`）在新增切换时丢失展开态、漏合并 `updated_sources` 且回环切换显示 `switched A -> A`

- **位置**：`src/ui/main_content/lx/history/snapshot.rs:297-325`、`src/ui/main_content/lx/history/paint.rs:413-431`、`src/ui/main_content/lx/history/model_switch.rs:20-61`
- **关键点**：
  1. 连续模型切换合并为 `TimelineEntry::ModelSwitch { first, last, count }` 后，`snapshot.rs` 仅使用 `switches[last].id` 查询 `is_expanded` 并注册 `CardKind::ModelSwitch`；
  2. 折叠态第 2 行描述仅读取 `switches[last].updated_sources`；
  3. 展开态首行对比仅取 `switches[first].from_label` 与 `switches[last].to_label`。
- **错误原因**：
  1. 当用户展开第 1 条切换卡片（记录 `switches[0].id`）后紧接着再次切换模型，`last` 变为 `1`，`is_expanded(&switches[1].id)` 返回 `false`，导致原本展开的卡片在追加切换时被意外自动折叠关闭；
  2. 若第 1 次切换改变了模型（`core/model`），第 2 次切换仅改了思考等级（`core/reasoning_effort`），合并后的第 2 行只看 `switches[last]` 会丢失 `core/model`；
  3. 若用户从模型 `A` 切到 `B` 再切回 `A`，`first.from_label == last.to_label == "A"`，展开态首行会渲染出荒谬的 `switched A -> A`。
- **修复**：
  1. 合并卡片统一以 `switches[first..=last]` 区间判定展开态（区间内任一 `id` 在 `expanded` 中即为展开），并将锚定 `switch_id` 固定为区间首条 `switches[first].id`（追加新切换时 `first` 不变，展开态天然保持，点击切换时同步清理同区间其余 `id`）；
  2. 合并卡片传入 `&switches[first..=last]` 切片：折叠态描述汇总区间内去重保序的全部 `updated_sources`；展开态当 `count > 1` 时渲染完整跃迁链（如 `switched A -> B -> A`），不再退化为 `switched A -> A`。
- **回归测试**：`ui::main_content::lx::history::tests::model_switch::consecutive_switches_merge_into_xn_and_keep_expanded_and_chain`。

---

### P1-3 父会话切换模型后，唤醒（`send_input` / `resume_agent`）的子 Agent 仍沿用旧模型

- **位置**：`src/event/chat/subagent/runner.rs:493-503`、`src/event/chat/subagent/settle.rs:106-113`
- **关键点**：
  1. `design.md` D5 规定子 Agent 继承父会话最新模型快照；
  2. `settle.rs` 在子 Agent 单轮结束时将旧模型快照写回 `streams.frozen[child_session_id]`，而 `runner.rs::advance_queue` 解析模型时的优先级是 `streams.frozen.get(&record.child_session_id)` > `streams.frozen.get(&parent_session)` > `resolve_frozen_for_pane(state, parent_key.pane)`。
- **错误原因**：
  由于 `child_session_id` 的旧缓存排在最前，父会话通过 `/model` 切换模型后，后续通过 `send_input` 或 `resume_agent` 唤醒的子 Agent 永远命中旧模型快照，既不会切换到新模型，其 `# Your Model` 身份段也停留在旧模型。
- **修复**：
  在 `runner.rs::advance_queue` 中将 `resolve_frozen_for_pane(state, parent_key.pane)`（及 `streams.frozen.get(&parent_session)`）置于 `streams.frozen.get(&record.child_session_id)` 之前，并用最新解析出的 `frozen` 刷新 `streams.frozen.insert(record.child_session_id.clone(), frozen.clone())`。
- **回归测试**：`event::chat::subagent::tests::resumed_subagent_inherits_updated_parent_pane_model`。

---

### P1-4 上下文预算计算未计入新增系统提示词开销，且预算裁剪丢弃首轮时会静默丢失 `<system-update>`

- **位置**：`src/chat/context/mod.rs:231-290, 368-376`
- **关键点**：
  1. `assemble` 在计算历史轮次裁剪预算 `budget` 时只扣除了 `output_reserve`、`ENVELOPE_RESERVE_TOKENS (256)` 与 `USER_SHELL_INJECTION_RESERVE`，直到函数末尾才拼装 `system_prompt`（含 `# Your Model`、最高 48 KiB 的 `AGENTS.md`、`skills`、`mcp`）；
  2. 当 `omitted_turns > 0`（旧轮次因预算不足被从头部裁掉）时，`min_switch_sequence` 设为首个保留轮次的 `user_sequence`，早于该序号的所有 `model_switches` 被直接过滤丢弃。
- **错误原因**：
  1. 当项目与全局 `AGENTS.md` 较大时，系统提示词可达上万 tokens，远超 256 tokens 预留，导致历史轮次未被及时裁剪而超出模型上下文窗口；
  2. 若模型切换发生在被裁剪的旧轮次区间内，且保留的近期轮次也是由旧模型生成的，过滤掉 `<system-update>` 后新模型将收不到中途接管说明。
- **修复**：
  1. 将 `compose_system_prompt` 提前到预算计算前，计算系统提示词超出 `ENVELOPE_RESERVE_TOKENS` 的增量 tokens 并从 `budget` 中扣除；
  2. 当 `omitted_turns > 0` 且存在 `sequence < min_switch_sequence` 的模型切换记录时，保留紧邻 `min_switch_sequence` 之前的最后一条切换记录并在首个保留轮次前注入 `<system-update>`。
- **回归测试**：`chat::context::tests::model_switch::trimmed_history_retains_latest_prior_model_switch_and_accounts_for_system_prompt_budget`。

---

### P1-5 多窗格共享会话、`/resume` 空模型列会话与 `/rewind` 回退时的模型状态未同步

- **位置**：`src/db/service.rs:1109-1118`、`src/app/update/chat/session.rs:413-415`、`src/db/chat/rewind.rs:720-724, 782-794`、`src/event/chat/rewind.rs:212-236`
- **关键点**：
  1. `Effect::SetChatSessionModel` 将 `switches` 广播给所有绑定同一 `session_id` 的窗格时，未同步更新其余窗格的 `lx.chat.selection`；
  2. `/resume`（`on_session_loaded_full`）在 `model` 为 `None`（如 `user_shell` 创建的会话）时未清空 `pane.lx.chat.selection`，导致窗格残留上一会话的选择；
  3. `/rewind` 在删除 `sequence >= target_sequence` 的消息与 `agent_model_switch` 记录后，未回滚 `agent_session` 表的 `model_provider` / `model_id` / `variant` 三列，也未在回退完成后刷新窗格模型选择。
- **错误原因**：
  模型选择状态分散在发起窗格、共享窗格与 `agent_session` 表三处，在跨窗格广播、空模型恢复及历史回退时只更新了其中一部分，导致 UI 状态栏显示的模型与会话历史实际对应的模型脱节。
- **修复**：
  1. `Effect::SetChatSessionModel` 广播 `switches` 时同步更新所有同会话窗格的 `selection`；并在 `record_model_switch` 失败时仍从 DB 重载 `switches` 覆盖乐观状态；
  2. `on_session_loaded_full` 在 `model` 为 `None` 时显式置 `pane.lx.chat.selection = None`；
  3. `rewind.rs` 删除对话分支按“最后一条存活 `agent_model_switch` 的目标模型，否则最后一条存活 assistant 消息的模型”同步回写 `agent_session` 模型列，并在 `handle_rewind_finished` 中同步回填窗格 `selection`。
- **回归测试**：`db::chat::model_switch::tests::rewind_restores_session_model_to_surviving_state`。

---

### P1-6 生产代码文件 `src/db/service.rs` 与 `src/app/update/chat/mod.rs` 超出 1000 行规范上限

- **位置**：`src/db/service.rs`（1451 行）、`src/app/update/chat/mod.rs`（1007 行）
- **关键点**：`AGENTS.md` 规定“生产代码单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右”。
- **错误原因**：两个文件在基线分支已逼近或超过 1000 行，本次合入 `SetChatSessionModel` 与模型切换状态处理后分别达到 1451 行与 1007 行。
- **修复**：
  1. 将 `src/db/service.rs` 中的聊天会话/历史/模型切换/压缩等 Effect 处理拆分为 `src/db/service/chat.rs` 子模块，使两个文件均降至 750 行以内；
  2. 将 `src/app/update/chat/mod.rs` 中的生命周期/通知辅助逻辑收敛至子模块，使其降至 1000 行以内。
- **回归测试**：全量 `cargo clippy --all-targets -- -D warnings` 与 `cargo test`。

---

## P2（路径与截断边界 / 缓存签名 / 迁移单测与文档对齐）

### P2-1 `collect_search_dirs` 未兼容软链接路径比对，且 `AGENTS.md` 超出字节预算截断时无显式标记

- **位置**：`src/chat/agent/instructions.rs:153-175, 230-252`
- **关键点**：
  1. `collect_search_dirs` 直接判断 `start.starts_with(root)`，当 `project_root` 与 `cwd` 任一侧经过软链接（如 macOS `/tmp` 与 `/private/tmp`）时判定失败，退化为只扫描 `cwd` 单目录；
  2. `read_file_within_budget` 在文件超过 32 KiB / 16 KiB 预算时直接静默截断，未追加 `…[truncated]` 标记。
- **错误原因**：路径前缀比较未做 `canonicalize` 兜底；文件读取只截取了前 `budget` 字节而未检测是否仍有剩余字节。
- **修复**：
  1. 当 `!start.starts_with(root)` 时，尝试对 `start` 与 `root` 执行 `std::fs::canonicalize` 后再比对祖先链；
  2. `read_file_within_budget` 额外探测 1 字节，若发生截断则在 UTF-8 字符边界追加 `\n…[truncated]`。
- **回归测试**：`chat::agent::tests::instructions::project_doc_budget_truncates_at_32_kib_boundary`。

---

### P2-2 `model_switch_block_signature` 缓存哈希未覆盖连续合并区间与 `is_variant_only`

- **位置**：`src/ui/main_content/lx/history/cache.rs:814-833`
- **关键点**：`model_switch_block_signature` 仅对 `switches[last]` 的部分字段计算哈希，未覆盖 `first..=last` 合并区间内的全部条目及 `is_variant_only()`。
- **错误原因**：当乐观插入的切换条目被 DB 回投更新或连续多条切换合并时，若只哈希 `last` 的 `to_label` 与 `updated_sources`，可能命中过期缓存行高。
- **修复**：在 `model_switch_block_signature` 中遍历 `source.switches[first..=last]`，将每条记录的 `id`、`from_label`、`to_label`、`to_variant`、`is_variant_only()`、`updated_sources` 及展开态下的 `injected_prompt` 全部纳入 Fnv 哈希。
- **回归测试**：`ui::main_content::lx::history::tests::model_switch::consecutive_switches_merge_into_xn_and_keep_expanded_and_chain`。

---

### P2-3 补齐 v21→v22 数据库迁移单测并修正 `design.md` / `task.md` 文档漂移

- **位置**：`src/db/chat/model_switch/tests.rs`、`docs/feature/lx-agent-prompt-and-model-switch/design.md`、`docs/feature/lx-agent-prompt-and-model-switch/task.md`
- **关键点**：
  1. 测试集中缺少从 v21 升级至 v22 并验证 `agent_model_switch` 表结构与级联删除的专项回归测试；
  2. `design.md` 与 `task.md` 中的 `TimelineEntry::ModelSwitch { first, last, count }`、函数签名、`src/chat/agent/report.rs` 与 `src/db/service/chat.rs` 拆分路径需与最终实现完全同步。
- **修复**：
  1. 在 `src/db/chat/model_switch/tests.rs` 新增 `v21_upgrade_creates_agent_model_switch_table_and_cascades` 测试；
  2. 同步更新 `design.md` 与 `task.md`，确保文档与代码实现零漂移。
- **回归测试**：`db::chat::model_switch::tests::v21_upgrade_creates_agent_model_switch_table_and_cascades`。
