# lx Agent 系统提示词（AGENTS.md）注入与模型切换逻辑 — 执行任务清单

> 状态：**已实现并通过验证（待合并入 `dev`）**（2026-11-01）
> 设计依据：[design.md](design.md)
> 阶段划分：P0（开工准备）→ P1（`src/chat/agent/` 拆分与 `AGENTS.md` 注入）→ P2（模型身份与切换持久化 v22）→ P3（状态层、上下文注入与跨模型清洗）→ P4（TUI 历史渲染与 `compact` 样式对齐）→ P5（定向与全量验证）。

---

## P0. 开工门槛与基线核对

- [x] 用户审核确认 [design.md](design.md) 与本 `task.md` 后开始执行。
- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 复核拟改文件的最新符号与调用链，遵守 `AGENTS.md` 与 `docs/standards/*.md` 全部规范。
- [x] 核对数据库迁移基线：当前 `src/db/mod.rs` 的 `SCHEMA_VERSION = 21`，本次新增迁移使用 `v22`；迁移前先在数据库副本上验证 `user_version`、表行数、`PRAGMA foreign_key_check` 与 `PRAGMA integrity_check`。

---

## P1. `src/chat/agent/` 模块拆分与 `AGENTS.md` 系统提示词注入

- [x] 将 `src/chat/agent.rs` 拆分为 `src/chat/agent/` 目录模块：
  - `src/chat/agent/mod.rs`：保留 `AgentRole`、`ToolSurface`、子 Agent 工具入参/状态视图与 `compose_subagent_prompt`，并通过 `pub use` 保持对外导出路径零破坏。
  - `src/chat/agent/instructions.rs`：实现 `LoadedInstructions`、`ensure_global_agents_md`、`load_instructions`。
    - 全局层：检查 `~/.lx-t`（开发 `~/.lx-t-dev`）下是否存在 `AGENTS.override.md` / `AGENTS.md` / `agent.md`（大小写兼容），若均不存在则原子创建空文件 `~/.lx-t[-dev]/AGENTS.md`；读取上限 `16 KiB`。
    - 项目层：仅在 `workspace_trusted == true` 时，从 `project_root` 到 `cwd` 逐目录按优先级 `AGENTS.override.md` > `AGENTS.md` > `agent.md` 查找第一个非空文件，共享 `32 KiB`（`MAX_PROJECT_DOC_BYTES`）预算。
    - 格式化：全局与项目文档间以 `\n\n--- project-doc ---\n\n` 分隔，外层包裹 `# AGENTS.md instructions for <cwd>\n\n<INSTRUCTIONS>\n...\n</INSTRUCTIONS>`。
  - `src/chat/agent/model_switch.rs`：实现 `ModelIdentity`（`# Your Model` 渲染）、`render_model_switch_update` 与 `wrap_system_update`。
- [x] 更新 `src/chat/prompt.rs`：
  - 将 `PROMPT_VERSION` 从 `"lx-7"` 更新为 `"lx-8"`。
  - 在 `PromptParts` 中新增 `model_identity: Option<&'a str>` 与 `instructions: Option<&'a str>`，按固定顺序 `model_identity → environment → instructions → skills → mcp` 渲染 `render_tail_with`。
  - 确保 `compose_system_prompt` 与 `compose_subagent_prompt` 均自动包含 `model_identity` 与 `instructions`。
- [x] 拆分并补充单元测试：
  - `src/chat/agent/tests/mod.rs`
  - `src/chat/agent/tests/instructions.rs`（覆盖 `~/.lx-t[-dev]/AGENTS.md` 自动创建、候选文件优先级、未信任工作区拦截、32 KiB 预算截断、`<INSTRUCTIONS>` 格式化）
  - `src/chat/agent/tests/model_switch.rs`（覆盖 `# Your Model` 段落、模型切换对比与 `<system-update>` 转义包裹）

---

## P2. 数据库 Schema v22 迁移与模型切换持久化（`src/db/`）

- [x] 在 `src/db/mod.rs` 将 `SCHEMA_VERSION` 升至 `22`。
- [x] 在 `src/db/migration.rs` 新增 v22 迁移：创建 `agent_model_switch` 表及索引 `idx_agent_model_switch_session_seq`（外键关联 `agent_session(external_id) ON DELETE CASCADE`）。
- [x] 新增 `src/db/chat/model_switch.rs`：
  - 定义 `ModelSwitchRecord` 与 `ModelSwitchCommit`。
  - 实现 `Database::record_model_switch(&self, workspace, commit) -> Result<Option<ModelSwitchRecord>>`：
    - 在同一写事务内更新 `agent_session` 的 `model_provider` / `model_id` / `variant` 三列与 `revision`；
    - 若该会话尚无任何 `agent_message`（空白会话），只更新 `agent_session` 三列，不插入 `agent_model_switch` 行；
    - 若会话已有消息且模型/思考等级实际发生变化，分配联合单调 `sequence` 插入 `agent_model_switch` 行并返回记录。
  - 实现 `Database::model_switches(&self, workspace, session_id) -> Result<Vec<ModelSwitchRecord>>` 按 `sequence` 升序加载。
- [x] 更新 `src/db/chat/mod.rs`、`src/db/chat/rewind.rs`、`src/db/chat/compact.rs` 中的 `sequence` 联合取号 SQL，将 `SELECT sequence FROM agent_model_switch WHERE session_id = ?1` 纳入 `UNION ALL`。
- [x] 在 `src/db/chat/tests.rs` 与 `src/db/migration/tests.rs` 补充 v22 迁移与 `record_model_switch` / `model_switches` 单元测试，并在开发库副本上验证迁移完整性。

---

## P3. 状态层、事件层接线与上下文/跨模型适配（`src/app/`、`src/event/`、`src/chat/context/`）

- [x] 在 `src/app/state/lx/` 新增 `model_switch.rs`：
  - 定义 `ModelSwitchNoticeView`（含 `id`、`sequence`、`from_label`、`to_provider`、`to_model`、`to_variant`、`to_label`、`updated_sources`、`injected_prompt`、`created_at_ms`）与 `LxModelSwitch`（管理 `switches: Vec<ModelSwitchNoticeView>` 与 `expanded: Vec<String>` 折叠状态）。
  - 在 `Lx` 状态结构中挂载 `pub model_switch: LxModelSwitch`，并在 `reset_session_view` / 会话切换时同步重置。
- [x] 更新 `src/app/update/chat/session.rs` 与 `src/db/service.rs`：
  - 在 `finish_model_selection` 中对比切换前后的有效 `ModelSelection`；若已绑定会话且发生变化，构造 `ModelIdentity`、变更源列表 `updated_sources`（如 `core/model`、`core/reasoning_effort`、`core/agents_md`）与 `injected_prompt`，通过 `Effect::SetChatSessionModel` 落库并同步更新 `pane.lx.model_switch`。
  - 在 `Effect::LoadChatSession` 中同步调用 `db.model_switches(&workspace, &session_id)`，通过 `update::set_model_switches(state, pane, &switches)` 回填历史切换条目。
- [x] 更新 `src/event/chat/mod.rs` 与 `src/event/chat/subagent/context.rs`：
  - 在主会话 `start_stream` 与子 Agent `assemble_child_context` 的 turn 起点，调用 `chat::agent::load_instructions` 加载全局 `~/.lx-t[-dev]` 与项目 `AGENTS.md`，并构造当前 `ModelIdentity` 传入 `PromptParts`。
- [x] 更新 `src/chat/context/mod.rs`：
  - `AssemblyInput` 接收 `model_identity: Option<&'a str>`、`instructions: Option<&'a str>` 与 `model_switches: &'a [ModelSwitchRecord]`。
  - 在 `start_after`（compact 检查点）之后按 `sequence` 将模型切换记录合并为 `<system-update>` 消息项注入上下文，使新模型明确感知中途接管与最新指令。
  - 强化跨模型兼容清洗：确保非当前 `provider/model` 的历史 `reasoning_metadata` 不跨模型回传，且当目标模型 `supports_image_input == false` 时将历史用户/工具图片统一降级为英文文本占位。

---

## P4. TUI 历史时间线渲染与交互（对齐 `compact` + 思考块压暗 Markdown）

- [x] 在 `src/ui/text.rs` 新增模型切换英文文案常量与函数：
  - `LX_MODEL_SWITCH_NOTICE_TITLE = "-- Model switched --"`
  - `model_switch_notice_title(count: usize) -> String`（`count > 1` 时返回 `"-- Model switched x{count} --"`）
  - `model_switch_notice_desc(...) -> String`
- [x] 更新 `src/ui/main_content/lx/history/compact.rs`：
  - 将展开态的 `range_line` 与 `summary` 统一接入思考块同款的 `  └─ ` 树形前缀（`style::chat_notice_connector()`，缩进 `thinking_body_indent(width)`）+ `tool::muted_markdown_lines`（`markdown_render::render_wrapped_literal_rules` + `span_style.patch(style::muted())`），保证 `rows()` 与 `draw()` 严格一致。
- [x] 新增 `src/ui/main_content/lx/history/model_switch.rs`：
  - 实现折叠态 2 行（`-- Model switched --` + `model: ...` / `variant: ...`）与展开态（`  └─ ` 树形连接符 + 压暗 Markdown 渲染切换对比及注入提示词）。
- [x] 接线历史时间线、快照缓存与点击交互：
  - `src/ui/main_content/lx/history/source.rs`：`HistorySource` 借用 `switches: &'a [ModelSwitchNoticeView]` 与 `model_switch: &'a LxModelSwitch`。
  - `src/ui/main_content/lx/history/timeline.rs`：`TimelineEntry` 新增 `ModelSwitch { last: usize, count: usize }`，按 `sequence` 升序合并，并将相邻连续的模型切换自动合并为一条 `xN`。
  - `src/ui/main_content/lx/history/cards.rs`、`snapshot.rs`、`paint.rs`、`cache.rs`：新增 `CardKind::ModelSwitch { switch_id, expanded }`、`SnapEntryKind::ModelSwitch`、`BlockSource::ModelSwitch`，并在 `app/update/chat/viewport.rs` 的 `toggle_lx_card` 中支持点击展开/折叠。
- [x] 在 `src/ui/main_content/lx/history/tests/` 下新增 `model_switch.rs` 并更新 `compact.rs` 测试，验证折叠/展开行数一致性、连续合并 `xN`、`  └─ ` 树形前缀与压暗 Markdown 样式。

---

## P5. 改后校验与交付

- [x] 使用 `rtk cargo test` 先针对受影响模块跑定向单元测试：
  - `rtk cargo test chat::agent::`
  - `rtk cargo test chat::prompt::`
  - `rtk cargo test chat::context::`
  - `rtk cargo test db::chat::`
  - `rtk cargo test ui::main_content::lx::history::`
- [x] 执行全量规范校验：
  - `rtk cargo fmt --check`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - `rtk cargo test`
- [x] 检查所有新增/修改的生产代码文件行数均 `< 1000` 行（子模块控制在 500 行左右），且无中文泄漏进模型提示词或 TUI 文案。
