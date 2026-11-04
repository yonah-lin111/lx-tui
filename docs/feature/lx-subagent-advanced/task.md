# lx Subagent 进阶能力与设置页配置 — 执行任务清单 (`task.md`)

- **状态**：已完成（Phase 1 ～ Phase 8 全部代码与单元测试均已完成并通过验证）
- **设计依据**：[`design.md`](design.md)

---

## 0. 开工门禁与硬性规范

- [x] **0.1 审核门禁**：用户审核并确认 `design.md` 与 `task.md` 后方可开始修改代码。
- [x] **0.2 规范红线重申**：
  - 生产代码单文件上限 **1000 行**，超限必须按职责拆分子模块，子文件控制在 500 行左右；测试文件严格跟随被测子模块目录（`<module>/<submodule>/tests.rs`），严禁堆积大单体测试。
  - 生产代码严禁 `unwrap()` / `expect()`，错误统一用 `Result` 与 `?` 传播。
  - 模型可见文本（提示词、工具描述、Schema、错误消息、`<subagent_report>` 信封）与用户可见 UI 文案（TUI 卡片、面板、设置页 HTML/JS）**一律使用英文**；中文仅用于代码注释与内部日志。
  - 保持 `app/`（纯状态与更新）与 `ui/`（纯只读渲染）边界清晰。
  - 未经用户明确授权，严禁修改 Git 状态（不执行 `git commit` / `git checkout` 等写操作）。

---

## Phase 1：领域层 — 动态角色体系、昵称池与控制协议扩展 (`src/chat/agent/`)

> **目标**：将固定 2 变体的 `AgentRole` 升级为 `BaseRole`（权限档位）+ `AgentRole`（支持内置与自定义角色名），实现唯一昵称分配器，并在 `SubagentOp` / `SubagentOutcome` 中加入 `fork_context`、`model`、`reasoning_effort`、`nickname` 与 `Interrupt`。

- [x] **1.1 拆分角色与权限基线模块 `src/chat/agent/role.rs` (~180 行)**
  - 定义 `BaseRole { Explorer, Executor }`，提供 `as_str`、`parse`、`default_prompt`、`default_description`、`tool_surface`。
  - 定义 `AgentRole { name: String, base: BaseRole }`，提供 `AgentRole::explorer()`、`AgentRole::executor()`、`from_name_and_base`、`as_str(&self) -> &str`、`base(&self) -> BaseRole`、`is_builtin(&self) -> bool`、`tools(&self) -> &'static [&'static str]`。
  - 实现 `is_valid_role_name(name: &str) -> bool`（`[a-z][a-z0-9_]{0,31}`）。
- [x] **1.2 新增昵称分配与身份格式化模块 `src/chat/agent/nickname.rs` (~120 行)**
  - 定义 `DEFAULT_NICKNAME_CANDIDATES`（24 个经典科学家/数学家名字）。
  - 实现 `allocate_nickname(used: &[String], custom_candidates: &[String]) -> String`（大小写不敏感判重，池耗尽后按 `#2`, `#3` 递增）。
  - 实现 `format_agent_identity(nickname: Option<&str>, role: &str) -> String`（有昵称返回 `"Turing [explorer]"`，无昵称返回 `"explorer"`）。
- [x] **1.3 扩展 `src/chat/agent/mod.rs`、`parse.rs` 与 `report.rs` (~450 行)**
  - `MAIN_TOOLS` 增加 `"interrupt_agent"`（17 → 18 项），`EXEC_MAIN_TOOLS` 增加 `"interrupt_agent"`（16 → 17 项）。
  - `SubagentOp::Spawn` 新增 `fork_context: bool`、`model: Option<String>`、`reasoning_effort: Option<String>`。
  - `SubagentOp` 新增 `Interrupt { target: String }`。
  - `SubagentOutcome` 新增 `Interrupted { agent_id, nickname, previous_status, queued_count }`；在 `Spawned`、`InputAccepted`、`Resumed`、`Closed`、`AgentStatusView`、`AgentListItemView`、`SubagentReportView` 中补齐 `nickname: Option<String>` 与 `model_label: Option<String>`（`Spawned` 另含 `fork_context: bool`），保持 `to_json` / `from_json` 向后兼容。
  - `parse.rs`：
    - `parse_control` 新增 `"interrupt_agent" => Some(checked_size(arguments_json).and_then(parse_interrupt))`。
    - `parse_spawn` 白名单扩为 `&["role", "title", "task", "items", "fork_context", "model", "reasoning_effort"]`；`role` 校验非空且满足 `is_valid_role_name`。
  - `report.rs`：`format_report_envelope` 在有 `nickname` 时输出 `nickname="..."` 属性。
- [x] **1.4 阶段一单元测试 (`src/chat/agent/tests.rs`)**
  - 测试 `BaseRole` 与 `AgentRole` 内置/自定义构造及非法角色名拦截。
  - 测试 `allocate_nickname` 默认池、自定义池、大小写不敏感避重与 `#2` 溢出轮转。
  - 测试 `parse_control` 对 `spawn_agent`（含 `fork_context`、`model`、`reasoning_effort`）和 `interrupt_agent` 的解析与未知字段拒绝。
  - 测试 `SubagentOutcome` 新旧 JSON 格式往返序列化兼容。

---

## Phase 2：配置层 — Subagents 配置文件读写 (`src/config/subagents.rs`)

> **目标**：新增 `<data>/config/subagents.json` 的类型定义、校验、角色解析与 0600 原子持久化。

- [x] **2.1 新增 `src/config/subagents.rs` (~320 行) 并在 `src/config/mod.rs` 导出**
  - 定义 `SubagentsConfigFile`、`SubagentsSection`、`BuiltinRoleOverrides`、`RoleOverrideConfig`、`CustomRoleConfig`、`ResolvedRoleSpec`。
  - 实现 `SubagentsSection::validate(&self) -> Result<(), SubagentsConfigError>`：
    - `max_concurrent` 必须在 `1..=8`；
    - 自定义角色 `name` 必须满足 `is_valid_role_name`、不得为 `"explorer"` 或 `"executor"`、且列表内不得重名；
    - 自定义角色 `description`（1..=500 字符）与 `system_prompt`（1..=16000 字符）非空；
    - `nickname_candidates` 单项 1..=24 字符，最多 32 项。
  - 实现 `SubagentsSection::resolve_role(&self, role_name: &str) -> Option<ResolvedRoleSpec>` 与 `all_role_summaries(&self) -> Vec<RoleSummary>`（供工具 Schema 生成角色说明列表）。
  - 实现 `subagents_config_path()`、`load_subagents(path)`、`save_subagents(path, file)`（临时文件 → 0600 权限 → 原子 `rename`）。
- [x] **2.2 阶段二单元测试 (`src/config/subagents/tests.rs`)**
  - 测试缺失文件返回默认配置（`max_concurrent = 3`、内置 `explorer`/`executor`）。
  - 测试内置角色覆写与 Reset、自定义角色增删与名称冲突校验、原子保存无临时文件残留。

---

## Phase 3：工具规格与 Schema 层 (`src/chat/tools/`)

> **目标**：声明 `interrupt_agent` 工具 Schema，扩展 `spawn_agent` 参数 Schema，支持将已配置的角色列表动态注入 `spawn_agent` 描述。

- [x] **3.1 更新 `src/chat/tools/subagent.rs` 与 `src/chat/tools/schema.rs` (~320 行)**
  - 在 `subagent.rs` 新增 `interrupt_agent_spec()` 与 `interrupt_parameters()`。
  - 在 `spawn_parameters()` 中增加 `fork_context`、`model`、`reasoning_effort` 三个可选属性，`role` 属性改为 `"type": "string"` 并附清晰英文说明。
  - 新增 `build_spawn_role_guidance(subagents: &SubagentsSection) -> String`：把内置 `explorer` / `executor` 及所有自定义角色的描述、基础权限与锁定的默认模型格式化为英文 `Available roles:` 段落（对齐 `codex-main` 的 `spawn_tool_spec::build`）。
  - 更新 `send_input`、`wait_agent`、`close_agent`、`resume_agent` 的参数描述，明确支持传入 `agent_id` 或 `nickname`。
  - 在 `src/chat/tools/types.rs` 的 `TOOL_NAMES` 与 `SUBAGENT_TOOL_NAMES` 中加入 `"interrupt_agent"`。
- [x] **3.2 阶段三单元测试 (`src/chat/tools/tests.rs`)**
  - 验证主工具数（18 项）、`ExecMain` 工具数（17 项）、子角色工具白名单（`Explorer` 3 项、`Executor` 7 项）。
  - 验证自定义角色描述能正确渲染进 `spawn_agent` 工具描述且均为纯英文。

---

## Phase 4：数据库层 — 昵称持久化、双向寻址与 `fork_context` 历史克隆 (`src/db/chat/`)

> **目标**：复用 `agent_subagent` 与 `agent_session` 既有预留列持久化 `nickname`、自定义角色名与基础权限档位 `agent_path`，实现 `agent_id`/`nickname` 双向寻址，以及 `fork_context=true` 时的已完成历史轮次克隆。

- [x] **4.1 抽取可复用的历史克隆辅助函数 (`src/db/chat/fork.rs`) (~60 行改动)**
  - 新增 `pub(super) fn completed_boundary_sequence(conn: &Connection, session_id: &str) -> Result<i64>`：查找父会话当前正在 `streaming` 的轮次起始 `sequence`（无 `streaming` 则返回 `i64::MAX`）。
  - 公开内部事务复制函数 `pub(super) fn copy_completed_history_to_child(tx: &Transaction<'_>, source_session: &str, child_session: &str, boundary: i64, image_rewrite: &Option<(String, String)>) -> Result<(i64, i64)>`：复制 `sequence < boundary` 且非 `streaming` 的消息、parts、tool_calls、attachments、compact_checkpoints、model_switches，并返回子会话下一条消息应使用的 `(next_sequence, next_turn_index)`。
  - 公开 `pub(crate) fn copy_session_images(source_session: &str, target_session: &str) -> Option<(String, String)>`。
- [x] **4.2 扩展 `src/db/chat/subagent.rs` (~80 行改动)**
  - `SubagentRecord` 新增字段：
    ```rust
    pub nickname: Option<String>,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub variant: Option<String>,
    ```
  - `SpawnSubagentParams`（或 `begin_subagent_turn_with_options`）接收 `nickname: Option<&str>`、`fork_context: bool`、`role: &AgentRole`：
    - 将 `role.as_str()` 写入 `agent_subagent.role` 与 `agent_session.agent_role`；
    - 将 `role.base().as_str()` 写入 `agent_subagent.agent_path` 与 `agent_session.agent_path`；
    - 将 `nickname` 写入 `agent_subagent.nickname` 与 `agent_session.agent_nickname`；
    - 当 `fork_context == true` 时，先在事务内调用 `completed_boundary_sequence` 与 `copy_completed_history_to_child`，再以返回的 `(next_sequence, next_turn_index)` 插入本次子任务的 `user` 与 `streaming` `assistant` 消息。
  - 更新 `read_subagent_row`：读取 `a.nickname`、`a.agent_path`、`a.model_provider`、`a.model_id`、`a.variant`，使用 `AgentRole::from_name_and_base(role_str, base_str)` 还原角色（不再丢弃自定义角色行！）。
  - 新增 `resolve_owned_subagent(workspace, parent_session, target_or_nickname)`：先按 `external_id` 精确查，未命中再按同父会话 `nickname` ASCII 大小写不敏感匹配。
- [x] **4.3 阶段四单元测试 (`src/db/chat/subagent/tests.rs`)**
  - 测试 `nickname`、自定义角色名（如 `"reviewer"`）与 `BaseRole` 在 DB 中的完整存取，以及自定义角色在设置页删除后 `list_subagents` / `owned_subagent` 仍能正常读取。
  - 测试 `resolve_owned_subagent` 通过 `agent_id` 与大小写不敏感 `nickname`（如 `"turing"` 匹配 `"Turing"`）双向寻址。
  - 测试 `fork_context = true`：父会话有 2 轮已完成对话 + 第 3 轮正在 `streaming` 时派生子会话，验证子会话精确包含前 2 轮历史（含 tool calls 与 attachments）+ 子任务第 3 轮，且不包含父会话未完成的 streaming 轮次。

---

## Phase 5：事件层与无头执行器 — 三级模型解析、`interrupt_agent` 与运行编排 (`src/event/chat/subagent/` & `src/chat/agent/exec/`)

> **目标**：在 TUI 事件循环与 CLI `exec` 无头模式中打通自定义角色解析、动态并发上限、三级模型覆写、昵称分配、`fork_context` 上下文加载与 `interrupt_agent` 状态机。

- [x] **5.1 三级模型覆写与角色解析 (`src/event/chat/subagent/runner.rs`)**
  - 从 `state.subagents_config` 读取当前生效的 `SubagentsSection`：
    - 并发上限取 `state.subagents_config.max_concurrent`（替代硬编码常量 `MAX_CONCURRENT_SUBAGENTS`）。
    - 调用 `state.subagents_config.resolve_role(role.as_str())` 校验角色是否存在，获取角色配置的 `ResolvedRoleSpec`（含 `base_role`、`system_prompt`、`default_model`、`nickname_candidates`）。
  - 实现 `resolve_subagent_model(&state.models, &parent_frozen, role_spec.default_model.as_ref(), model_override, effort_override)`。
  - 查询当前父会话已有子 Agent 的昵称列表，调用 `allocate_nickname` 分配唯一昵称。
  - 若 `fork_context == true`，在写库前调用 `copy_session_images(&parent_session, &child_session_id)`，并在 `launch_child_turn` 中调用 `load_child_history_items` 将克隆的历史轮次装入初始 `ChatRequest.items`。
- [x] **5.2 实现 `interrupt_agent` 与双向寻址 (`src/event/chat/subagent/mod.rs` & `settle.rs`)**
  - 将 `send_input`、`resume`、`wait`、`close` 中的目标查找统一替换为 `database.resolve_owned_subagent`，解析出真实 `record.agent_id` 后再操作内存句柄与队列。
  - `ChatStreams` 新增 `paused_by_interrupt: HashSet<String>`。
  - 实现 `interrupt(state, streams, persistence, key, target)`：
    - 运行中目标：登记 `paused_by_interrupt` 与 `resumed_subagents`，发送 `cancel.send(true)`，返回 `SubagentOutcome::Interrupted`；
    - `finish_subagent_with_billing` 遇到 `was_paused_by_interrupt == true` 时：保留 `resumed_subagents`（维持 `idle`），跳过自动消费队列；
    - 已处于 `idle` 的目标：幂等返回 `SubagentOutcome::Interrupted { previous_status: "idle", .. }`；
    - 终态未恢复的目标：返回 `NOT_ACTIVE_CODE`。
- [x] **5.3 同步更新无头执行池 (`src/chat/agent/exec/subagent/mod.rs`)**
  - 同步支持 `SubagentOp::Interrupt`、双向昵称寻址、`fork_context` 与 `SubagentsSection` 配置读取。
- [x] **5.4 阶段五单元测试 (`src/event/chat/tests/subagent.rs` & `src/chat/agent/exec/tests/`)**
  - 测试三级模型覆写优先级（工具传参 > 角色默认 > 父会话模型）及非法模型/variant 报错。
  - 测试 `interrupt_agent` 打断运行中子 Agent 后状态变为 `idle`、保留半截输出、暂停排队队列，随后 `send_input` 直接唤醒继续。
  - 测试用 `nickname` 调用 `send_input`、`wait_agent`、`interrupt_agent`、`close_agent`、`resume_agent`。

---

## Phase 6：设置页后端与前端 (`src/settings/` & `assets/`)

> **目标**：在浏览器设置页新增 `Subagents` 页签，支持配置全局并发上限、覆写/重置内置角色（`explorer`/`executor`）以及增删改自定义角色。

- [x] **6.1 新增设置页后端模块 `src/settings/service/subagents.rs` (~360 行)**
  - 在 `src/settings/service.rs` 中新增 `SettingsEvent::SubagentsUpdated(SubagentsSection)`，并在 `src/event/settings.rs` 中处理该事件以实时更新 `state.subagents_config`。
  - 实现路由 `pub fn route(request: &Request, state: &ServiceState) -> Response`：
    - `GET /api/subagents`：返回 `{ maxConcurrent, builtins, roles, defaults: { explorer, executor }, models: [...] }`；
    - `PUT /api/subagents/general`：更新 `maxConcurrent`；
    - `PUT /api/subagents/builtins/<role>` 与 `DELETE /api/subagents/builtins/<role>`：保存或重置内置角色覆写；
    - `POST /api/subagents/roles`、`PUT /api/subagents/roles/<name>`、`DELETE /api/subagents/roles/<name>`：自定义角色 CRUD。
- [x] **6.2 新增设置页前端静态模块 `assets/settings_subagents.js` (~340 行) 并更新 `assets/settings.html`、`src/settings/service/page.rs`**
  - 在 `src/settings/service/page.rs` 注册 `"/settings_subagents.js"` 静态路由。
  - 在 `assets/settings.html` 导航栏加入 `Subagents` 按钮、`<section id="view-subagents" class="view hidden">` 视图结构与 `<script src="/settings_subagents.js"></script>`。
  - 在 `assets/settings.js` 的 `showTab` 中接入 `'subagents'` 分支调用 `loadSubagents()`。
- [x] **6.3 阶段六单元测试 (`src/settings/service/tests.rs`)**
  - 测试 `/settings_subagents.js` 静态资源返回 200。
  - 测试 `/api/subagents` GET/PUT/POST/DELETE 全链路、参数 422 校验、磁盘持久化与 `SettingsEvent::SubagentsUpdated` 事件投递。

---

## Phase 7：App 状态与 TUI 渲染层升级 (`src/app/` & `src/ui/main_content/lx/`)

> **目标**：在委派卡片、`▲` 汇报块与右侧 `Agents` 面板中呈现 `Nickname [role] · title`、覆写模型标签与 `forked` 标记。

- [x] **7.1 更新状态视图 (`src/app/state/lx/agent.rs`、`src/app/state/lx/chat/tool_view.rs`、`src/app/update/chat/subagent.rs`)**
  - `SubagentRun` 新增 `pub nickname: Option<String>` 与 `pub model_label: Option<String>`，新增 `pub fn display_identity(&self) -> String`（返回 `format_agent_identity(self.nickname.as_deref(), self.role.as_str())`）。
  - `SubagentDetailView` 新增 `pub nickname: Option<String>`、`pub model_label: Option<String>`、`pub fork_context: bool`。
- [x] **7.2 更新 UI 文案与绘制 (`src/ui/text.rs`、`src/ui/main_content/lx/history/tool/subagent.rs`、`report.rs`、`task/agents.rs`)**
  - `src/ui/text.rs`：新增 `VERB_TOOL_INTERRUPT_AGENT = "Interrupted"`、`FORKED_CONTEXT_TAG = "forked"`。
  - `history/tool/subagent.rs`：支持 `"interrupt_agent"` 卡片渲染；`agent_line` 与 `target_node` 使用 `format_agent_identity` 显示 `Turing [explorer] · title`。
  - `history/report.rs`：`agent_line` 显示 `Turing [explorer] · title`。
  - `task/agents.rs`：`draw_list` 使用 `run.display_identity()` 绘制 `❯ ■ Turing [explorer]  running  14s  title`。
- [x] **7.3 阶段七单元测试 (`src/ui/main_content/lx/history/tests.rs` & `task/tests.rs`)**
  - 验证有昵称（`Turing [explorer] · title`）与无昵称历史旧记录（`explorer · title`）在卡片、汇报块和 `Agents` 列表中的渲染。
  - 验证 `interrupt_agent` 工具卡片在运行中与完成态下的标题与详情行。

---

## Phase 8：系统提示词更新与精确回归验证

- [x] **8.1 更新主 Agent 系统提示词 (`src/chat/prompt.rs`)**
  - 在 Delegation 章节补充 `fork_context`、`model`/`reasoning_effort` 按需覆写、`nickname` 寻址与 `interrupt_agent` 的英文使用准则；若提示词文本变更则递增 `PROMPT_VERSION` 并同步预算测试。
- [x] **8.2 精确验证与质量门禁**
  - 运行受影响模块测试：
    - `cargo test chat::agent`
    - `cargo test config::subagents`
    - `cargo test db::chat::subagent`
    - `cargo test event::chat::tests::subagent`
    - `cargo test settings::service`
    - `cargo test ui::main_content::lx`
  - 运行 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`。
  - 核查所有修改与新增生产文件行数均 ≤ 1000 行，无 `unwrap()` / `expect()`。
