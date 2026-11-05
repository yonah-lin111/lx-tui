# lx 全功能无头 CLI (`lx-full-cli`) — 执行任务清单 (`task.md`)

- **状态**：已完成（已在 `.worktrees/lx-full-cli` 分支 `feature/lx-full-cli` 完成实现与验证，等待用户确认合并）
- **设计依据**：[`design.md`](design.md)

---

## 0. 开工门禁与硬性规范

- [x] **0.1 审核门禁**：用户审核并确认 `design.md` 与 `task.md` 后方可开始修改代码。
- [x] **0.2 规范红线重申**：
  - 生产代码单文件上限 **1000 行**，新建与拆分后的子模块严格控制在 **300～500 行** 左右；测试文件严格跟随被测子模块组织（`src/cli/tests/*.rs`），单测试文件不超过 1000 行。
  - 生产代码严禁 `unwrap()` / `expect()`，错误统一用 `Result` 与 `?` 传播。
  - CLI 帮助文本、表头、状态提示、JSON 字段名与错误消息 **100% 使用英文**；中文仅用于代码注释与内部 `tracing` 日志。
  - 保持模块职责与依赖边界清晰，同步更新 `docs/standards/project-directory-structure.md`。
  - 未经用户明确授权，严禁修改 Git 状态（不执行 `git commit` / `git checkout` 等写操作）。

---

## Phase 1：CLI 基础底座 — 顶层路由、参数扫描、工作区上下文与双模式输出 (`src/cli/`)

> **目标**：搭建 `src/cli/` 核心骨架，实现零依赖通用参数解析器、`GlobalOpts`（`-C`/`--json`/`--color`/`-h`）、多行内容输入（`--content`/`-f`/`stdin`）、工作区与开发库定位（`WorkspaceContext`）、会话锁获取（`acquire_session_lock`）与双模式输出器（`Output`），并接入 `src/main.rs` 与 `src/lib.rs`。

- [x] **1.1 新增 `src/cli/args.rs` (~300 行)**
  - 实现 `GlobalOpts { cwd, json, color }` 与 `extract_global_opts(raw_args)`。
  - 实现通用参数辅助函数：`utf8`、`take_opt_value`、`required_non_empty`、`parse_i64_arg`、`parse_usize_arg`、`parse_bool_arg`、`parse_scope_arg`。
  - 实现 `ContentInput { inline, file }` 及其 `resolve(required, stdin_is_terminal, read_stdin)`（支持 `--content`、`--content -`、`-f/--file`、非 TTY 管道 `stdin`）。
- [x] **1.2 新增 `src/cli/context.rs` (~200 行)**
  - 实现 `WorkspaceContext::resolve(cwd_override, entry_linked_worktree)`（复用 `git::list_worktrees` 解析 `repo_root` 与 `git::linked_worktree_root` 解析开发态隔离库）。
  - 实现 `WorkspaceContext::open_db(&self) -> Result<Database, String>`。
  - 实现 `resolve_target_session(db, workspace, session_id, last) -> Result<String, String>`（支持 `<SESSION_ID>` 精确匹配、唯一前缀匹配与 `--last`）。
  - 实现 `acquire_session_lock(session_id) -> Result<SessionLockGuard, String>`（包装 `platform::try_lock_session`）。
- [x] **1.3 新增 `src/cli/output.rs` (~220 行)**
  - 实现 `Output::new(json, color_mode)`、`Output::emit(value, human_fn)`、`Output::print_table(out, headers, rows)` 与 `mask_secret(raw: &str) -> String`。
- [x] **1.4 新增 `src/cli/help.rs` (~360 行)**
  - 定义顶层 `HelpTopic`（含 `Root`、`Exec`、`ExecResume`、`Session`、`Prompt`、`Todo`、`Workspace`、`Model`、`Mcp`、`Skill`、`Command`、`Subagent`、`Settings`）。
  - 实现 `parse_help(topics)` 与 `help_text(topic) -> &'static str` 全英文帮助手册。
- [x] **1.5 新增 `src/cli/mod.rs` (~200 行) 并更新 `src/lib.rs`、`src/main.rs`**
  - 定义顶层 `CliAction` 枚举、`parse_cli` 与 `run_cli`。
  - 更新 `src/main.rs` 调用 `lx_tui::cli::{CliAction, parse_cli, run_cli}`，保持无参数进入 TUI 及 `lx-tui exec` 100% 向后兼容。

---

## Phase 2：会话与工作区 CLI (`session` & `workspace`)

> **目标**：实现 `lx-tui session`（含 `list/show/rename/delete/fork/rewind/compact/agents/processes`）与 `lx-tui workspace`（含 `status/trust/worktrees`）。

- [x] **2.1 在 `src/event/chat/rewind.rs` 暴露同步回滚入口 (~15 行改动)**
  - 新增 `pub fn execute_rewind(database: &Database, workspace: &Path, request: &RewindRequest) -> RewindOutcome` 转调 `restore::run(database, workspace, request)`。
- [x] **2.2 新增 `src/cli/session/mod.rs` (~440 行)**
  - 定义 `SessionCli` 与 `SessionCommand` 枚举及 `parse(args)`。
  - 实现 `list`（分页列出 `SessionSummary`）、`show`（加载 `ChatMessage`、工具摘要/详情、思考块、`active_checkpoint` 与 `load_notices`）、`rename`（`db.rename_chat_session`）、`delete`（先 `acquire_session_lock` 再 `db.delete_chat_session`）、`fork`（`db.fork_chat_session` + `copy_session_images`）、`agents`（`db.list_subagents` / `db.list_subagent_tree` + `decode_stored_role_and_path` + `has_queued_input` + `projected_agent_status` 7 态投影 + `--filter <all|active|done>` + `count_changed_paths`）、`processes`（`db.load_processes_for_session`）。
- [x] **2.3 新增 `src/cli/session/ops.rs` (~360 行)**
  - 实现 `run_rewind`：支持 `--dry-run`（调用 `db.latest_user_turn` + `db.restore_plan` 预检输出）与 4 种模式（`revert-and-delete`、`delete-only`、`revert-only`、`restore-compact`），复用 `execute_rewind` 与会话锁。
  - 实现 `run_compact`：支持默认摘要生成提交（获取会话锁 → `chat::compact::incremental` → `resolve_model_target` → `client::run_stream` → `validate_summary` → `db.commit_compact`）与 `--revert`（`db.revert_compact`）。
- [x] **2.4 新增 `src/cli/workspace.rs` (~340 行，兼容 `workspace-history-list`)**
  - 实现 `workspace list`（默认调用 `db.list_workspace_history_entries()` 输出 Git 父子树形历史列表、`exists`、`trusted`、`position`，支持 `--search` 过滤；`--raw` 输出 `db.list_workspaces()` 扁平记录）。
  - 实现 `workspace status`（显示工作区路径、保存名称、`is_trusted`、是否在 `workspace_hidden` 中、Git 分支、worktree 状态、DB 路径）。
  - 实现 `workspace add`（调用 `db.record_workspace_open` + `db.unhide_workspace`，支持 `--trust`）。
  - 实现 `workspace rename`（调用 `db.rename_workspace`）。
  - 实现 `workspace delete`（检查目标工作区下全部会话无活跃 `try_lock_session` → linked worktree 写 `db.hide_workspace` → `db.delete_workspace` 级联删除；支持 `--include-worktrees` 一并清理同仓库 linked worktree）。
  - 实现 `workspace reorder`（调用 `db.save_workspace_order`）。
  - 实现 `workspace trust`（调用 `db.trust_workspace` + `db.unhide_workspace`）。
  - 实现 `workspace worktrees`（调用 `git::list_worktrees` 列出全部 worktree）。

---

## Phase 3：提示词库与待办日程 CLI (`prompt` & `todo`)

> **目标**：实现 `lx-tui prompt`（条目 CRUD、`folder`、`page`、`ref`）与 `lx-tui todo`（`list/add/update/done/start/reopen/delete/reorder/rollover/stats`）。

- [x] **3.1 新增 `src/cli/prompt.rs` (~460 行)**
  - 实现提示词条目子命令：`list`、`show`、`create`（先 `db.ensure_workspace` 再 `db.create_prompt`）、`edit`（`db.update_page_content`）、`rename`（`db.rename_prompt`）、`move`（`db.move_prompt`）、`delete`（`db.delete_prompt`）。
  - 实现文件夹子命令 `prompt folder`：`list`、`create`、`rename`、`move`、`delete`。
  - 实现分页子命令 `prompt page`：`list`、`create`、`edit`、`rename`、`active`、`delete`。
  - 实现引用目录子命令 `prompt ref`：`list`、`add`、`remove`。
- [x] **3.2 新增 `src/cli/todo.rs` (~360 行)**
  - 实现 `todo list`（支持 `--date`、`--all`、`--past-incomplete`、`--status`、`-p/--priority` 过滤）。
  - 实现 `todo add`、`todo update`（支持 ID 唯一前缀匹配）、`todo done`/`start`/`reopen`、`todo delete`。
  - 实现 `todo reorder`（`db.reorder_todos`）、`todo rollover`（`db.rollover_todos_to_date`）与 `todo stats`（`db.query_range_stats` + 状态/优先级分布聚合）。

---

## Phase 4：模型、MCP、Skills、自定义命令、子 Agent 与通用设置 CLI (`model` / `mcp` / `skill` / `command` / `subagent` / `settings`)

> **目标**：实现全部配置与扩展生态的无头 CLI 子命令，包括远端模型发现、MCP 探测与 OAuth 授权、Skills 发现与读写、自定义命令 CRUD、子 Agent 配置以及 `settings open` 前台驻留 Web 服务。

- [x] **4.1 在 `src/settings/service/models/mod.rs` 公开远端模型拉取函数 (~10 行改动)**
  - 将 `fetch_remote_models(base_url: &str, api_key: &str) -> Result<Vec<(String, String)>, String>` 标记为 `pub(crate) async fn`，供 `cli::model` 直接复用。
- [x] **4.2 新增 `src/cli/model.rs` (~450 行)**
  - 实现 `list`、`show`（含 API Key 掩码脱敏）、`default`（设置/清空 `defaultModel` 或 `titleSummary`）、`provider-set`、`provider-rm`、`model-set`、`model-rm`、`fetch`（调用 `fetch_remote_models`，支持 `--save` 合并落盘）、`config`（设置 `titleLanguage`、`streamIdleTimeoutMs`、WebSearch Key）。
- [x] **4.3 新增 `src/cli/mcp.rs` (~460 行)**
  - 实现 `list`（合并用户级与项目级 `effective_servers`）、`show`（敏感字段脱敏）、`add-stdio`、`add-http`、`remove`、`enable`、`disable`。
  - 实现 `probe`（调用 `mcp::manager().probe` 并输出发现的工具列表，完成后 `shutdown`）。
  - 实现 `login`（本地临时回环端口监听 OAuth callback + `mcp::oauth::begin_authorization` + `complete_authorization`）与 `logout`（清除对应 key 凭据）。
- [x] **4.4 新增 `src/cli/skill.rs` (~350 行)**
  - 实现 `list`（调用 `skills::discover::discover`）、`show`（调用 `skills::store::read_skill_at`）、`enable` / `disable`（更新 `config/skills.json` 的 `enabled`/`disabled` 集合）、`create` / `edit` / `delete`（仅限 lx 自有技能目录，调用 `skills::store::{write_skill, delete_skill}`）。
- [x] **4.5 新增 `src/cli/command.rs` (~350 行)**
  - 实现 `list`（合并 User 与 Project 作用域并标注 `overridden`）、`show`、`create`、`edit`、`rename`、`delete`（对接 `settings::commands`）。
- [x] **4.6 新增 `src/cli/subagent.rs` (~420 行，兼容 `lx-agent-roles-residency-and-panel`)**
  - 实现 `show`（结合当前受信任工作区调用 `merge_project_roles_for_paths` 展示 `explorer`/`executor`/`monitor` 三内置角色、全局角色与项目级 `.lx/agents/*.json` 角色，以及 `delegation_mode`、`lock_model`、`RoleCapabilities`）。
  - 实现 `set`（修改 `max_concurrent`、`max_depth`、`default_max_tokens`、`delegation_mode`）。
  - 实现 `role-set`（支持 3 个内置角色 `explorer`/`executor`/`monitor` 覆写、`--lock-model`、`--allow-web-search`/`--allow-mcp`/`--allow-skills`，以及 `--scope <user|project>` 写入全局 `subagents.json` 或 `<workspace>/.lx/agents/<name>.json`）。
  - 实现 `role-reset`（支持 `explorer`/`executor`/`monitor`）与 `role-rm`（支持 `--scope <user|project>`）。
- [x] **4.7 新增 `src/cli/settings.rs` (~300 行)**
  - 实现 `show`（打印 `settings.json` 与各配置文件路径）、`set`（更新布局参数并校验保存）、`reset`（恢复默认）、`open`（启动 `SettingsService`、打开浏览器、等待 `Ctrl+C` 优雅退出）。

---

## Phase 5：文档同步、单元测试与精确校验

> **目标**：更新架构规范文档，编写覆盖全子命令的集成/单元测试，执行精确验证。

- [x] **5.1 更新 `docs/standards/project-directory-structure.md`**
  - 在目录树与模块职责章节补充 `src/cli/` 模块说明与依赖边界。
- [x] **5.2 编写 CLI 单元测试 (`src/cli/tests/`)**
  - `src/cli/tests/mod.rs`：构建隔离的临时目录 + 临时 SQLite DB + 临时 `LX_TUI_DB_PATH` 测试夹具。
  - `src/cli/tests/args_and_help_tests.rs`：测试全部 11 个顶层子命令路由、`help` 主题、`--json`/`--color`/`-C` 提取、`ContentInput` 三种输入方式及冲突报错。
  - `src/cli/tests/session_tests.rs`：测试 `session list/show/rename/delete/fork/agents/processes`、`--last` 寻址、`rewind --dry-run` 与实际文件回滚、以及**会话被锁占用时 `delete`/`rewind`/`compact` 拒绝执行**。
  - `src/cli/tests/prompt_and_todo_tests.rs`：测试 `prompt`（文件夹 2 层约束、条目 CRUD、多分页切换与编辑、引用目录）与 `todo`（增删改查、状态切换、重排、顺延、统计）的 Human 与 `--json` 输出。
  - `src/cli/tests/config_domains_tests.rs`：测试 `workspace trust/status/worktrees`、`model` CRUD 与脱敏、`mcp` 配置增删改与启停、`skill` 发现/启停/读写、`command` 增删改重命名、`subagent` 角色与并发配置、`settings show/set/reset`。
- [x] **5.3 精确验证与质量门禁**
  - 运行 `cargo test cli` 与 `cargo test chat::agent::exec`；
  - 运行 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`；
  - 核查所有修改与新增生产文件行数均 ≤ 1000 行（子模块 ≤ 500 行），零 `unwrap()` / `expect()`，所有用户可见文案均为英文。
