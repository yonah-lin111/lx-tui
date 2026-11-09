# lx 输入框 `@` 命令新增 `Subagents` Tab — 实施任务拆解 (`task.md`)

- **关联设计**：[`design.md`](design.md)
- **关联复审**：[`review.md`](review.md)
- **执行前提**：用户审核确认 `design.md` 与本 `task.md` 后，按以下任务顺序逐步落地并执行精准校验。
- **当前状态**：T1–T7 全部完成，已合并到 `dev`（merge commit `b8d8533c`）。

---

## 任务总览

| 任务 ID | 任务名称 | 涉及核心文件 | 预计改动后行数（红线 ≤1000/1500） | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **T1** | 扩展 `ReferKind::Subagent` 引用语法、编辑器高亮、快捷引用与发送预处理 | `src/app/markdown/refer.rs`<br>`src/app/markdown/refer_shortcut.rs`<br>`src/ui/markdown.rs`<br>`src/chat/prepare.rs`<br>`src/chat/prepare/web.rs` | `refer.rs`: ~215 行<br>`refer_shortcut.rs`: ~495 行<br>`ui/markdown.rs`: ~390 行<br>`prepare.rs`: ~625 行<br>`prepare/web.rs`: ~288 行 | 已完成 |
| **T2** | 扩展 `MentionTab`（5-Tab vs 4-Tab 隔离）、`LxMention` 候选模型、三类排序过滤与确认插入 | `src/app/prompt/mention.rs`<br>`src/app/update/editor.rs`<br>`src/app/state/lx/mention.rs`<br>`src/app/state/lx/panels.rs`<br>`src/app/state/lx.rs` | `prompt/mention.rs`: ~685 行<br>`lx/mention.rs`: ~750 行<br>`lx/panels.rs`: ~345 行 | 已完成 |
| **T3** | 打通存活实例与角色配置同步链路，以及当轮 `<subagent_reference>` 上下文注入 | `src/app/state/pane.rs`<br>`src/app/update/chat/subagent.rs`<br>`src/db/service/chat.rs`<br>`src/event/inventory.rs`<br>`src/event/mod.rs`<br>`src/event/chat/subagent/snapshot.rs`<br>`src/event/chat/mod.rs` | `inventory.rs`: ~695 行<br>`snapshot.rs`: ~310 行<br>`event/chat/mod.rs`: ~902 行 | 已完成 |
| **T4** | TUI 渲染层接入 `Subagents` Tab 与 `subagent_row` 双行展示 | `src/ui/text/workspace.rs`<br>`src/ui/mention.rs`<br>`src/ui/main_content/lx/mention.rs` | `ui/mention.rs`: ~195 行<br>`lx/mention.rs`: ~335 行 | 已完成 |
| **T5** | 单元测试覆盖与精准验证（`cargo fmt`、`clippy`、受影响模块测试） | `src/app/markdown/refer/tests.rs`<br>`src/app/state/lx/mention/tests.rs`<br>`src/chat/prepare/tests.rs`<br>`src/event/chat/tests/subagent_roles.rs` | `lx/mention/tests.rs`: ~650 行<br>`prepare/tests.rs`: ~310 行 | 已完成 |
| **T6** | 对齐 Codex 内嵌 `lx-guide` 系统自省 Skill 并补全 `<subagent_reference>` 自身规格注入 | `src/skills/system.rs`<br>`src/skills/assets/lx-guide/*`<br>`src/skills/mod.rs`<br>`src/skills/discover.rs`<br>`src/event/chat/skills.rs`<br>`src/settings/service/skills.rs`<br>`assets/settings_skills.js`<br>`src/event/chat/subagent/snapshot.rs` | `skills/system.rs`: ~120 行<br>`skills/discover.rs`: ~325 行<br>`snapshot.rs`: ~410 行 | 已完成 |
| **T7** | Review 问题修复（P0–P3 全量闭环，见 `review.md`） | `src/app/state/lx/*`、`src/app/state/pane.rs`、`src/app/update/{workspace,chat/subagent}.rs`、`src/event/chat/{mod,subagent/*}.rs`、`src/ui/widgets/command_panel.rs`、`src/skills/*`、`src/settings/service/skills.rs`、`assets/settings_skills.js` | `snapshot.rs`: ~620 行<br>`event/chat/mod.rs`: ~998 行<br>`runner.rs`: ~990 行 | 已完成 |

---

## 详细任务步骤与关键代码检查点

### Task 1 (T1)：扩展 `ReferKind::Subagent` 引用语法、编辑器高亮与发送预处理

- **目标文件**：
  - `src/app/markdown/refer.rs`
  - `src/app/markdown/refer_shortcut.rs`
  - `src/ui/markdown.rs`
  - `src/chat/prepare.rs`
  - `src/chat/prepare/web.rs`
- **实施细节与关键代码**：
  1. **`src/app/markdown/refer.rs`**：
     - 在 `ReferKind` 枚举新增 `Subagent` 变体；
     - `ReferKind::as_str` 返回 `"subagent"`，`ReferKind::parse("subagent")` 返回 `Some(Self::Subagent)`。
  2. **`src/app/markdown/refer_shortcut.rs`**：
     - 在 `ReferTag::label` 中新增分支：
       ```rust
       Self::Refer(ReferKind::Subagent) => "refer-subagent",
       ```
  3. **`src/ui/markdown.rs`**：
     - 在 `refer_token` 中将 `markdown::ReferKind::Subagent` 映射到 `TokenKind::ReferFile`：
       ```rust
       markdown::ReferKind::Skill
       | markdown::ReferKind::Mcp
       | markdown::ReferKind::Subagent => TokenKind::ReferFile,
       ```
  4. **`src/chat/prepare.rs` & `src/chat/prepare/web.rs`**：
     - `prepare_user_message` 中将 `ReferKind::Subagent` 与 `ReferKind::Mcp` 合并：
       ```rust
       ReferKind::Mcp | ReferKind::Subagent => {
           // MCP 与 Subagent 引用为编辑器纯文本语法糖（已记入 claimed_ranges），随正文原样发送。
       }
       ```
     - `prepare_ai_web_message` 中将 `ReferKind::Subagent` 与 `ReferKind::Skill | ReferKind::Mcp` 合并返回 `Err(AiWebPrepareError::LocalReferenceUnsupported)`。
- **验证点**：
  - `rtk cargo test --lib app::markdown::refer`
  - `rtk cargo test --lib chat::prepare`

---

### Task 2 (T2)：扩展 `MentionTab`、`LxMention` 候选模型、三类排序过滤与确认插入

- **目标文件**：
  - `src/app/prompt/mention.rs`
  - `src/app/update/editor.rs`
  - `src/app/state/lx/mention.rs`
  - `src/app/state/lx/panels.rs`
  - `src/app/state/lx.rs`
- **实施细节与关键代码**：
  1. **`MentionTab` 扩展与右侧 `Prompt` 4-Tab 隔离 (`src/app/prompt/mention.rs`, `src/app/update/editor.rs`)**：
     - `MentionTab` 新增 `Subagents` 变体；
     - `MentionTab::ALL` 更新为 5 项 `[Files, RefDir, Skills, Mcp, Subagents]`，新增 `PROMPT_ALL: [Self; 4] = [Files, RefDir, Skills, Mcp]` 与 `from_prompt_index`、`next_prompt`、`prev_prompt`；
     - `MentionState::switch_tab` 改调 `next_prompt()` / `prev_prompt()`，`MentionState::set_tab` 对 `MentionTab::Subagents` 返回 `false`；
     - `src/app/update/editor.rs::switch_mention_tab` 改调 `MentionTab::from_prompt_index(tab)`。
  2. **`SubagentRunMentionEntry` 与 `SubagentMentionItem` (`src/app/state/lx/mention.rs`)**：
     - 定义 `SubagentRunMentionEntry { agent_id, nickname, role, status, title }` 与 `SubagentMentionItem { Active(SubagentRunMentionEntry), Role(ResolvedRoleSpec) }`；
     - 在 `src/app/state/mod.rs`（或 `src/app/state/lx.rs`）导出 `SubagentMentionItem` 与 `SubagentRunMentionEntry` 供 `ui::mention` 使用；
     - 手动实现 `Default for LxMention`，使 `subagent_roles` 默认填充 `crate::config::subagents::SubagentsSection::default().all_roles()`（3 个内置角色 `explorer` / `executor` / `monitor`）；
     - 实现 `LxMention::filtered_subagents(&self) -> Vec<SubagentMentionItem>`：
       ```rust
       pub fn filtered_subagents(&self) -> Vec<SubagentMentionItem> {
           let query_lower = self
               .trigger
               .as_ref()
               .map(|t| t.query.to_lowercase())
               .unwrap_or_default();
           let active_iter = self
               .subagent_runs
               .iter()
               .cloned()
               .map(SubagentMentionItem::Active);
           let builtin_iter = self
               .subagent_roles
               .iter()
               .filter(|role| role.is_builtin)
               .cloned()
               .map(SubagentMentionItem::Role);
           let project_custom_iter = self
               .subagent_roles
               .iter()
               .filter(|role| !role.is_builtin && role.is_project_scoped)
               .cloned()
               .map(SubagentMentionItem::Role);
           let user_custom_iter = self
               .subagent_roles
               .iter()
               .filter(|role| !role.is_builtin && !role.is_project_scoped)
               .cloned()
               .map(SubagentMentionItem::Role);
           active_iter
               .chain(builtin_iter)
               .chain(project_custom_iter)
               .chain(user_custom_iter)
               .filter(|item| item.matches_query(&query_lower))
               .collect()
       }
       ```
     - 扩展 `secondary_count()`、`is_open()`、`view()`、`escape()` 与 `confirm()` 支持 `MentionTab::Subagents`：
       - 选中 `SubagentMentionItem::Active(run)` 插入 `@[refer-subagent](<nickname_or_id>:<role>) `；
       - 选中 `SubagentMentionItem::Role(role)` 插入 `@[refer-subagent](<role_name>) `。
  3. **`Lx` 包装方法 (`src/app/state/lx/panels.rs`, `src/app/state/lx.rs`)**：
     - 新增 `mention_subagent_roles(&self)`、`set_mention_subagent_roles(&mut self, roles)`、`sync_mention_subagent_runs(&mut self) -> bool`；
     - 在 `Lx::refresh_panel(&mut self)` 与 `Lx::clear_agents(&mut self)` 中调用 `self.sync_mention_subagent_runs()`。
- **验证点**：
  - `rtk cargo test --lib app::state::lx::mention`
  - `rtk cargo test --lib app::prompt`

---

### Task 3 (T3)：打通存活实例与角色配置同步链路，以及当轮 `<subagent_reference>` 上下文注入

- **目标文件**：
  - `src/app/state/pane.rs`
  - `src/app/update/chat/subagent.rs`
  - `src/db/service/chat.rs`
  - `src/event/inventory.rs`
  - `src/event/mod.rs`
  - `src/event/chat/subagent/snapshot.rs`
  - `src/event/chat/mod.rs`
- **实施细节与关键代码**：
  1. **窗格分割复制与运行态同步 (`src/app/state/pane.rs`, `src/app/update/chat/subagent.rs`, `src/db/service/chat.rs`)**：
     - `src/app/state/pane.rs` 分屏复制源窗格时，同步 `new_pane.lx.set_mention_subagent_roles(source.lx.mention_subagent_roles())`；
     - `src/app/update/chat/subagent.rs`：在 `on_subagent_started`、`on_subagent_finished`、`set_subagent_list` 中调用 `target.lx.sync_mention_subagent_runs()`；
     - `src/db/service/chat.rs::load_chat_session_into_pane`：加载会话后推入 `Effect::ListSubagents { pane, parent_session: session_id.to_string() }`，确保 `/resume` 恢复会话后立即同步该会话的存活子 Agent。
  2. **配置与项目角色同步 (`src/event/inventory.rs`, `src/event/mod.rs`)**：
     - 在 `src/event/inventory.rs` 实现 `refresh_subagent_mentions(state, persistence)`；
     - 在 `SettingsEvent::SubagentsChanged` 分支调用 `refresh_subagent_mentions(state, persistence)`；
     - 在 `src/event/mod.rs` 启动初始化、`skills_reload` 与 `current_agents_scopes != agents_scopes` 分支调用 `inventory::refresh_subagent_mentions(state, persistence)`。
  3. **当轮模型上下文提示块生成 (`src/event/chat/subagent/snapshot.rs`, `src/event/chat/mod.rs`)**：
     - 在 `src/event/chat/subagent/snapshot.rs` 实现 `subagent_reference_blocks(draft_text, effective_subagents, runs) -> Vec<String>`；
     - 在 `src/event/chat/mod.rs::start_stream` 中将 `subagent_reference_blocks` 追加到 `skill_blocks`（命名为 `turn_reference_blocks`）传入 `assemble_turn_request_with_context`。
- **验证点**：
  - `rtk cargo test --lib event::chat`
  - `rtk cargo test --lib event::inventory`

---

### Task 4 (T4)：TUI 渲染层接入 `Subagents` Tab 与 `subagent_row` 双行展示

- **目标文件**：
  - `src/ui/text/workspace.rs`
  - `src/ui/mention.rs`
  - `src/ui/main_content/lx/mention.rs`
- **实施细节与关键代码**：
  1. **`src/ui/text/workspace.rs`**：
     - 新增常量 `MENTION_TAB_SUBAGENTS`、`MENTION_SUBAGENTS_EMPTY`、`MENTION_SUBAGENT_ACTIVE_LABEL`、`MENTION_SUBAGENT_BUILTIN_LABEL`、`MENTION_SUBAGENT_CUSTOM_LABEL`（并在 `src/ui/text.rs` 自动导出）。
  2. **`src/ui/mention.rs`**：
     - 新增 `subagent_row(label, detail, detail_width) -> MentionRow` 与 `subagent_item_text(item) -> (String, String)`。
  3. **`src/ui/main_content/lx/mention.rs`**：
     - 在 `panel_data` 的非 `AiWeb` Tab 数组末尾加入 `PanelTab { label: text::MENTION_TAB_SUBAGENTS, active: tab == MentionTab::Subagents }`；
     - 新增 `(MentionTab::Subagents, LxMentionView::Empty | LxMentionView::Loading)` 与 `(MentionTab::Subagents, LxMentionView::Items)` 渲染分支。
- **验证点**：
  - `rtk cargo test --lib ui::main_content::lx`

---

### Task 5 (T5)：单元测试补充与精准校验

- **目标文件**：
  - `src/app/state/lx/mention/tests.rs`
  - `src/chat/prepare/tests.rs`
  - `src/event/chat/tests/subagent_roles.rs`
- **新增测试用例清单**：
  1. `subagents_tab_lists_active_runs_then_builtin_then_custom_roles_and_inserts_references`（覆盖三类候选排序、模糊过滤、选中存活实例插入 `@[refer-subagent](Turing:explorer) `、选中角色插入 `@[refer-subagent](explorer) `、一次退格整块原子删除）；
  2. `ai_web_entry_blocks_subagents_tab_and_rejects_subagent_reference`（覆盖 `AiWeb` 入口 Tab 锁定与 `prepare_ai_web_message` 返回 `LocalReferenceUnsupported`）；
  3. `subagent_reference_blocks_render_active_instance_and_role_guidance`（覆盖当轮 `<subagent_reference>` 英文引导块生成、去重、已结束实例 `finished_instance` 保留与仅在无实例时回退角色）。
- **最终校验命令**：
  - `rtk cargo fmt --check`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - `rtk cargo test --lib app::state::lx::mention`
  - `rtk cargo test --lib chat::prepare`
  - `rtk cargo test --lib event::chat::tests::subagent_roles`

---

## Task 6 (T6)：对齐 Codex 内嵌 `lx-guide` 系统自省 Skill 并补全 `<subagent_reference>` 自身规格注入

- **涉及文件**：
  - `src/skills/assets/lx-guide/SKILL.md`
  - `src/skills/assets/lx-guide/references/tools.md`
  - `src/skills/assets/lx-guide/references/subagents.md`
  - `src/skills/assets/lx-guide/references/commands-and-mentions.md`
  - `src/skills/system.rs`
  - `src/skills/mod.rs`
  - `src/skills/discover.rs`
  - `src/skills/discover/tests.rs`
  - `src/event/chat/skills.rs`
  - `src/chat/agent/exec/mod.rs`
  - `src/cli/skill/exec.rs`
  - `src/settings/service/skills.rs`
  - `assets/settings_skills.js`
  - `src/event/chat/subagent/snapshot.rs`
  - `src/event/chat/tests/subagent_roles.rs`
- **关键要点**：
  1. **内嵌 `lx-guide` 系统自省 Skill（`src/skills/system.rs` + `src/skills/assets/lx-guide/`）**：
     - 对齐 Codex `install_system_skills`：使用 `include_str!` 内嵌 `SKILL.md`、`references/tools.md`、`references/subagents.md`、`references/commands-and-mentions.md`；
     - `system_skills_root(user_root: &Path) -> PathBuf` 返回 `user_root.join(".system")`；
     - `install_system_skills(user_root: &Path) -> std::io::Result<()>` 计算内嵌文件列表的确定性 FNV-1a 哈希指纹并与 `<user_root>/.system/.lx-system-skills.marker` 比对，一致且文件齐全则跳过；不一致时通过 `INSTALL_LOCK` 串行、逐文件 `platform::write_file_atomic` 原子替换 `.system/lx-guide/` 内容并写回 marker（不删除 `.system` 整目录）；
     - `SkillSource` 新增 `System`（`as_str() => "system"`，`editable() => false`，默认启用 `!disabled.contains(&canonical)`）；
     - `skills::discover` 在 `user_agents_root` 之后扫描 `system_skills_root(user_root)`（若存在），优先级最低，允许被用户或项目同名 skill 覆盖；
     - 在 `event::chat::skills::build_job`、`settings::service::skills::list_skills`、`cli::skill::exec::catalog`、`chat::agent::exec::discover_workspace_skills` 中调用 `install_system_skills`，失败时 `tracing::warn!`；
     - 在 `assets/settings_skills.js` 中将 `entry.source === 'system'` 显示为 `'system (built-in)'`，且新建技能重名检查只针对同作用域可编辑条目，允许用户覆盖内置 skill。
  2. **补全 `<subagent_reference>` 当轮自身规格注入（`src/event/chat/subagent/snapshot.rs`）**：
     - 角色块（`kind: role`）新增 `tools`（按 `ToolSurface::from_role` + `spec.capabilities` 计算生效工具名；`executor` 在 `max_depth > 1` 时追加 8 个控制工具说明）、`capabilities`（`allow_web_search`、`allow_mcp`、`allow_skills`）、`model`（`inherits parent model` 或显式模型及是否锁定）、`system_prompt`（`spec.system_prompt_override` 或内置 `AgentRole::from(spec.base_role).system_prompt()`），以及区分“解释询问”与“任务委派”的双意图英文 `guidance`；
     - 活动实例块（`kind: active_instance`）新增 `title`、`task`（非空时）与 `latest_report`（若有），以及区分“询问状态/任务”与“追加指令/等待”的双意图英文 `guidance`；
     - 已结束实例块（`kind: finished_instance`）保留 `agent_id` / `status` / `latest_report`，引导 `resume_agent` / `send_input`，仅在 `runs` 完全无匹配时回退 `kind: role`。

---

## Task 7 (T7)：Review 问题修复（P0–P3 全量闭环）

- **涉及文件**：
  - `src/app/state/lx/mention.rs`、`src/app/state/lx/mention/tests.rs`、`src/app/state/lx/panels.rs`
  - `src/app/state/pane.rs`、`src/app/update/workspace.rs`、`src/app/update/chat/subagent.rs`
  - `src/event/chat/subagent/{settle,runner,launch,mod,snapshot}.rs`
  - `src/event/chat/mod.rs`、`src/chat/agent/exec/mod.rs`、`src/config/subagents.rs`
  - `src/ui/widgets/command_panel.rs`、`src/ui/main_content/lx/mention.rs`、`src/ui/main_content/lx/tests/slash_panels.rs`
  - `src/app/prompt/mention.rs`
  - `src/skills/system.rs`、`src/skills/assets/lx-guide/*`、`src/skills/discover/tests.rs`
  - `src/event/chat/skills.rs`、`src/cli/skill/exec.rs`、`src/settings/service/skills.rs`、`assets/settings_skills.js`
  - `src/event/chat/tests/subagent_roles.rs`
- **关键要点**：
  1. **P0**：`<subagent_reference>` 对已结束实例输出 `kind: finished_instance`，不再静默退化为角色规格并丢失 `agent_id`/`latest_report`。
  2. **P1**：`sync_mention_subagent_runs` 覆盖 settle/队列/恢复/关闭/切换 Tab 全部状态变更；新标签页/同路径工作区继承源窗格 `@` 快照；`lx-guide` description ≤200 字符；`install_system_skills` 原子安装、进程内串行、不删除 `.system`；自定义昵称拒绝 `:`/`(`/`)`。
  3. **P2**：`<subagent_reference>` 字段单行化 + 闭合标签转义 + 分字段截断 + 单轮 8 块上限；`executor` 工具面补齐控制工具说明；合成报告轮跳过引用块；`lx-tui exec` 接入 `subagent_reference_blocks`；窄窗格 Tab 滑动窗口保证第 5 个 Tab 可达；`SubagentMentionItem<'a>` 改为借用避免每帧深拷贝；`take_scan_request` 仅 `Files` Tab 触发；`view()` 全空判定纳入 subagents。
  4. **P3**：移除 `MentionState::confirm` 的 `unreachable!()`；状态字符串归一化；`lx-guide` 文档修正 `/rename`、存储路径、CLI 示例与 `spawn_agent`/`send_input` 参数表。
- **验证**：见 `review.md` 复审验证章节。
