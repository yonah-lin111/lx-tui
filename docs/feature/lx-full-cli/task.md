# lx 全功能无头 CLI (`lx-full-cli`) — 执行任务清单 (`task.md`)

- **状态**：已完成（已在 `.worktrees/lx-full-cli` 分支 `feature/lx-full-cli` 完成实现、代码审查修复与验证，等待用户确认合并）
- **设计依据**：[`design.md`](design.md)
- **审查修复依据**：[`review.md`](review.md)（P0–P3 全部标记已修复）

---

## 0. 开工门禁与硬性规范

- [x] **0.1 审核门禁**：用户审核并确认 `design.md` 与 `task.md` 后方可开始修改代码。
- [x] **0.2 规范红线重申**：
  - 生产代码单文件上限 **1000 行**，新建与拆分后的子模块严格控制在 **300～500 行** 左右；测试文件严格跟随被测子模块组织（`src/cli/tests/*.rs`），单测试文件不超过 1000 行。
  - 生产代码严禁 `unwrap()` / `expect()`，错误统一用 `Result` 与 `?` 传播。
  - CLI 帮助文本、表头、状态提示、JSON 字段名与错误消息 **100% 使用英文**；中文仅用于代码注释与内部 `tracing` 日志。
  - 保持模块职责与依赖边界清晰，同步更新 `docs/standards/project-directory-structure.md`。
  - 未经用户明确授权，严禁修改 Git 状态（不执行 `git commit` / `git checkout` 等写操作）。
- [x] **0.3 审查修复门禁**：按 `review.md` 的 P0–P3 逐项修复，修复后同步 `design.md`、`task.md`、`verification.md` 与目录结构规范，确保文档与实现零漂移。

---

## Phase 1：CLI 基础底座 — 顶层路由、参数扫描、工作区上下文与双模式输出 (`src/cli/`)

> **目标**：搭建 `src/cli/` 核心骨架，实现零依赖通用参数解析器、`GlobalOpts`（`-C`/`--json`/`--color`/`-h`）、多行内容输入（`--content`/`-f`/`stdin`）、工作区与开发库定位（`WorkspaceContext`）、统一信任门控、会话锁获取（`acquire_session_lock`）与双模式输出器（`Output`），并接入 `src/main.rs` 与 `src/lib.rs`。

- [x] **1.1 新增 `src/cli/args.rs`（428 行）**
  - 实现 `GlobalOpts { cwd, json, color }` 与 `extract_global_opts(raw_args)`。
  - 实现通用参数辅助函数：`utf8`、`take_opt`、`required_non_empty`、`parse_usize_flag`、`parse_u64_flag`、`parse_i64_flag`、`parse_bool_flag`、`reject_unknown_flag`，以及 `ScopeArg` / `ListScopeArg`（含“合并展示”与“单作用域写入”的区分）。
  - 实现 `ContentInput { inline, file }` 及 `resolve` / `resolve_with`（支持 `--content`、`--content -`、`-f/--file`、`-f -`、非 TTY 管道 `stdin`；可选正文从非 TTY stdin 读到空文本时返回 `Ok(None)`，避免脚本/CI 静默清空正文）。
  - `extract_global_opts` 在 `--` 处停止扫描并原样保留后续 token；对 `VALUE_OPTIONS` 中的已知带值选项保留其取值 token，避免 `--content --json` 误吞全局标志。
- [x] **1.2 新增 `src/cli/context.rs`（184 行）**
  - 实现 `WorkspaceContext::resolve(cwd_override, entry_linked_worktree)`（复用 `git::list_worktrees` 解析 `repo_root` 与 `git::linked_worktree_root` 解析开发态隔离库）。
  - 实现 `checkout_path()`（当前 linked worktree 返回自身 checkout 根，否则回退工作区根）与 `is_trusted(&db)`（cwd / checkout / workspace 根任一已信任即可）。
  - 实现 `open_db()`、`db_path()`、`open_global_db(...)` 与分页 `resolve_target_session(...)`（覆盖超过 200 条会话、空标题会话、精确匹配/唯一前缀匹配/`--last`）。
  - 实现 `acquire_session_lock(session_id) -> Result<FileGuard, String>`（包装 `platform::try_lock_session`）。
- [x] **1.3 新增 `src/cli/output.rs`（178 行）**
  - 实现 `Output::new(json, color_mode)`、`emit` / `emit_to`、`print_table`、颜色辅助函数与 `mask_secret(raw: &str) -> String`。
- [x] **1.4 新增 `src/cli/help.rs`（398 行）**
  - 定义顶层 `HelpTopic`（含 `Root`、`Exec`、`ExecResume`、`Session`、`Prompt`、`Todo`、`Workspace`、`Model`、`Mcp`、`Skill`、`Command`、`Subagent`、`Settings`）。
  - 实现 `parse_help(topics)` 与 `help_text(topic) -> &'static str` 全英文帮助手册。
- [x] **1.5 新增 `src/cli/mod.rs`（131 行）并更新 `src/lib.rs`（23 行）、`src/main.rs`（96 行）**
  - 定义顶层 `CliAction` 枚举、`parse_cli` 与 `run_cli`。
  - 更新 `src/main.rs` 调用 `lx_tui::cli::{CliAction, parse_cli, run_cli}`，保持无参数进入 TUI 及 `lx-tui exec` 100% 向后兼容。

---

## Phase 2：会话与工作区 CLI (`session` & `workspace`)

> **目标**：实现 `lx-tui session`（含 `list/show/rename/delete/fork/rewind/compact/agents/processes`）与 `lx-tui workspace`（含完整历史列表 `list/add/rename/delete/reorder/status/trust/worktrees`）。

- [x] **2.1 在 `src/event/chat/rewind.rs` / `rewind/**` 暴露同步回滚入口**
  - 新增 `pub fn execute_rewind(database: &Database, workspace: &Path, request: &RewindRequest) -> RewindOutcome` 转调 `restore::run(database, workspace, request)`。
- [x] **2.2 新增 `src/cli/session/mod.rs`（413 行）**
  - 定义 `SessionCli` 与 `SessionCommand` 枚举及 `parse(args)`。
  - 统一解析 `<SESSION_ID>`、唯一前缀与 `--last`；`rename` 额外支持 `--title <TITLE>`，避免 `--last` 与位置参数混淆。
- [x] **2.3 新增 `src/cli/session/views.rs`（500 行）**
  - 实现 `list`（分页）、`show`（`ChatMessage`、工具摘要/详情、思考块、`active_checkpoint` 与 notices）、`rename`、`delete`（会话锁 + 递归清理多级子会话与图片）、`fork`（`fork_chat_session` + `copy_session_images`）。
  - 实现 `agents`（`list_subagents` / `list_subagent_tree` + `decode_stored_role_and_path` + `has_queued_input` + `projected_agent_status` 7 态投影 + `--filter <all|active|done>` + `count_changed_paths`）与 `processes`（`load_processes_for_session`）。
- [x] **2.4 新增 `src/cli/session/ops.rs`（494 行）**
  - 实现 `run_rewind`：普通模式先取会话锁再读 `latest_user_turn`；`--dry-run` 调用 `db.restore_plan` / `db.rewind_availability` 只读预检；`--mode restore-compact`（含 dry-run）复用 `active_session_checkpoint` / `db.revert_compact`；其余 3 种模式复用 `execute_rewind`。
  - 实现 `run_compact`：获取会话锁 → `chat::compact::incremental` → `resolve_model_target` → `client::run_stream`（拦截 `ChatDelta::ToolInput*`）→ `validate_summary` → `db.commit_compact`；`--revert` 调用 `db.revert_compact`。
- [x] **2.5 新增 `src/cli/workspace.rs`（254 行）+ `workspace/exec.rs`（471 行）+ `workspace/exec/list.rs`（148 行），兼容 `workspace-history-list`**
  - `list`：树形历史列表（主 checkout 父项 + linked worktree 子项、`exists`、`trusted`、`position`、`--search`），`--raw` 输出 `list_workspaces()` 扁平记录。
  - `status`：默认目标为 `ctx.checkout_path()`，输出 `path`/`cwd`/`name`/`position`/`trusted`/`hidden`/`exists`/`git`/`database`。
  - `add` 校验目录存在且为目录，`record_workspace_open` + `unhide_workspace`，JSON `trusted` 返回数据库真实状态；`rename` 拒绝未登记路径。
  - `delete`：检查目标工作区全部会话锁 → linked worktree 写 `hide_workspace` → `delete_workspace` 级联删除 → 清理被删会话图片；`--include-worktrees` 仅当目标本身是主 checkout 时展开同仓库 linked worktree，仓库子目录/嵌套子模块不再误删主仓库。
  - `reorder` 将未列出的工作区按原相对顺序补到末尾并重写连续 `position`；`trust` 默认目标为当前 checkout；`worktrees` 用 `git::checkout` 精确标记 `current` 并补充 `head`。

---

## Phase 3：提示词库与待办日程 CLI (`prompt` & `todo`)

> **目标**：实现 `lx-tui prompt`（条目 CRUD、`folder`、`page`、`ref`）与 `lx-tui todo`（`list/add/update/done/start/reopen/delete/reorder/rollover/stats`）。

- [x] **3.1 新增 `src/cli/prompt.rs`（242 行）+ `prompt/exec.rs`（487 行）**
  - 实现提示词条目子命令：`list`、`show`、`create`（单事务传入初始分页，避免半成品）、`edit`、`rename`、`move`、`delete`。
  - `PromptCommand::folder_destination` / `parent_destination` 分别读取 `--folder` 与 `--parent`，修复 `prompt move --folder` 永久报错。
  - 所有 prompt/folder/page/ref 写操作与按 ID 查询先校验目标属于当前 `ctx.ws`；`page active` 额外校验分页属于指定提示词，跨工作区无法篡改或绕过“至少保留一页”。
  - 实现文件夹子命令 `prompt folder`、分页子命令 `prompt page` 与引用目录子命令 `prompt ref`。
- [x] **3.2 新增 `src/cli/todo.rs`（249 行）+ `todo/exec.rs`（347 行）**
  - 实现 `todo list`（`--date`、`--all`、`--past-incomplete`、`--status`、`-p/--priority`）与 `todo add`（位置文本或 `--text`）。
  - 实现 `todo update`（ID 唯一前缀）、`done`/`start`/`reopen`（`done` 对已完成项保留原 `completed_date`）、`delete`。
  - 实现 `todo reorder`、`todo rollover`（拒绝顺延已完成项）与 `todo stats`。

---

## Phase 4：模型、MCP、Skills、自定义命令、子 Agent 与通用设置 CLI (`model` / `mcp` / `skill` / `command` / `subagent` / `settings`)

> **目标**：实现全部配置与扩展生态的无头 CLI 子命令，包括远端模型发现、MCP 探测与 OAuth 授权、Skills 发现与读写、自定义命令 CRUD、子 Agent 配置以及 `settings open` 前台驻留 Web 服务。

- [x] **4.1 在 `src/settings/service/models/mod.rs` 公开远端模型拉取函数**
  - 将 `fetch_remote_models(base_url: &str, api_key: &str) -> Result<Vec<(String, String)>, String>` 标记为 `pub(crate) async fn`，供 `cli::model` 直接复用。
- [x] **4.2 新增 `src/cli/model.rs`（351 行）+ `model/exec.rs`（493 行）+ `model/exec/remote.rs`（136 行）**
  - 实现 `list`、`show`（API Key 掩码）、`default`（主模型与 `--title-summary`）、`provider-set`/`provider-rm`、`model-set`/`model-rm`、`fetch`（`--save` 合并落盘）、`config`（标题语言、流式超时、Exa/Tavily Key）。
  - 拒绝空 provider/model ID，`fetch` 前置校验非 OpenAI 兼容协议、空 API Key 与非法 Base URL。
- [x] **4.3 新增 `src/cli/mcp.rs`（494 行）+ `mcp/exec.rs`（395 行）+ `mcp/exec/add.rs`（208 行）**
  - 实现 `list`（`--scope user` 直接读用户文件，避免被项目同名服务覆盖后消失）、`show`（`bearer_token`/`client_secret`、`http_headers`/`env` 值掩码）、`remove`、`enable`/`disable`。
  - `add-stdio` / `add-http` 对已存在同名同类型服务做增量合并：未显式重传的 `args`/`env`/`cwd`、`auth`/token/client/headers/scopes 保留旧值，同时补充 `--env-http-header`。
  - 实现 `probe`（`mcp::manager::manager().probe` 后 shutdown）与 `login` / `logout`（本地回环 OAuth callback、`complete_authorization`、`<DATA_DIR>/mcp/credentials.json`）；`login --json` 只输出一次 JSON。
- [x] **4.4 新增 `src/cli/skill.rs`（289 行）+ `skill/exec.rs`（424 行）**
  - 实现 `list`（`skills::discover::discover`）、`show`、`enable`/`disable`、`create`/`edit`/`delete`（仅 lx 自有技能目录）。
  - 定位技能文件使用实际目录名（而非 frontmatter `name`），目录名与 frontmatter 不一致时原位更新/删除；项目级根为 `<DATA_DIR>/skills/projects/<scope>/<name>/SKILL.md`。
- [x] **4.5 新增 `src/cli/command.rs`（422 行）+ `command/exec.rs`（356 行）**
  - 实现 `list`（先按 `kind` 分组再合并，避免同名异类型被覆盖；输出 `mdScope`/`title`/`blockType`）、`show`、`create`、`edit`、`rename`、`delete`。
  - 非交互式 `edit` 未传正文时保留旧正文；空正文与含换行的 frontmatter 单行字段被拒绝。
- [x] **4.6 新增 `src/cli/subagent.rs`（399 行）+ `subagent/exec.rs`（476 行），兼容 `lx-agent-roles-residency-and-panel`**
  - 实现 `show`（3 个内置角色 `explorer`/`executor`/`monitor`、全局自定义角色、受信任工作区项目级角色、`delegation_mode`、`lock_model`、`RoleCapabilities`）。
  - 实现 `set`、`role-set`、`role-reset`、`role-rm`；项目级读写统一以 `ctx.checkout_path()` 为根，避免 linked worktree 下读写路径不一致。
- [x] **4.7 新增 `src/cli/settings.rs`（360 行）**
  - 实现 `show`、`set`（布局参数校验 + 原子保存）、`reset`、`open`（`SettingsService`、受信任时注入项目级命令快照、打开浏览器、等待 `Ctrl+C` 优雅退出）。

---

## Phase 5：文档同步、单元测试与精确校验

> **目标**：更新架构规范文档，编写覆盖全子命令的集成/单元测试，执行精确验证。

- [x] **5.1 更新 `docs/standards/project-directory-structure.md`**
  - 在目录树与模块职责章节补充 `src/cli/` 的 `*/exec.rs` 子模块拆分、`WorkspaceContext`/`checkout_path`/`is_trusted` 说明；依赖方向更新为 `cli -> app, chat, config, db, event, git, layout, mcp, platform, settings, skills`。
- [x] **5.2 编写 CLI 单元测试（`src/cli/tests/`，共 1801 行）**
  - `src/cli/tests/mod.rs`（164 行）：隔离临时目录 + 临时 SQLite DB + 临时 `HOME` 测试夹具，以及通过测试二进制子进程捕获真实 stdout 的 `exec_captured`。
  - `src/cli/tests/args_and_help_tests.rs`（242 行）：11 个顶层子命令路由、`help` 主题、`--json`/`--color`/`-C` 提取、`--` 终止符与带值选项保护、`ContentInput` 输入方式与冲突报错、空 stdin 可选正文、作用域解析与密钥掩码。
  - `src/cli/tests/session_tests.rs`（335 行）：`session list/show/rename/delete/fork/agents/processes`、`--last` 寻址、`rewind --dry-run` 与 `delete-only`/`compact --revert`、`--limit 0`/`rename --last --title` 校验、`restore-compact` 预检、会话锁拒绝 `delete`/`rewind`/`compact`。
  - `src/cli/tests/prompt_and_todo_tests.rs`（374 行）：`prompt` 文件夹 2 层约束、条目 CRUD、`move --folder`、跨工作区分页隔离、多分页切换与编辑、最后一页删除保护、引用目录；`todo` CRUD、`--text`、完成日期保留、重排、顺延与统计。
  - `src/cli/tests/config_domains_tests.rs`（340 行）：`workspace` 历史 CRUD/锁保护、`model` CRUD 与脱敏、`mcp` 配置增删改与启停、`skill`/`command`/`subagent`/`settings` CRUD，以及未信任工作区与非法参数拦截。
  - `src/cli/tests/config_regressions_tests.rs`（346 行）：P0–P2 回归专项，覆盖仓库子目录 `workspace delete --include-worktrees`、未登记路径拒绝、`reorder` 连续 `position`、`mcp add-http`/`add-stdio` 局部更新、非交互式 `skill/command edit` 正文保留、`command list --kind` 合并与 `model` 空 ID 拒绝。
- [x] **5.3 精确验证与质量门禁**
  - 运行 `cargo test cli` 与 `cargo test chat::agent::exec`；
  - 运行 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`；
  - 核查所有修改与新增生产文件行数均 ≤ 1000 行（子模块约 500 行以内），零 `unwrap()` / `expect()`，所有用户可见文案均为英文。

---

## Phase 6：代码审查修复与回归测试（`review.md` P0–P3）

> **目标**：逐项落实 `review.md` 的 P0–P3 修复，补齐隔离测试与端到端回归，确保文档、实现与验证报告完全同步。

- [x] **6.1 P0/P1 修复**
  - **P0-1**：`workspace delete --include-worktrees` 仅在目标本身是主 checkout（`repo_root == target`）时展开 linked worktree；历史残留目录通过 `.git` 的 `gitdir:` 是否指向 `<repo>/.git/worktrees/` 判定，仓库子目录与嵌套子模块不再误删主仓库。
  - **P1-1**：`WorkspaceContext::checkout_path()` 成为 `workspace add/delete/trust/status/rename` 的默认目标；无参 `workspace delete` 不再误删主仓库。
  - **P1-2**：`WorkspaceContext::is_trusted(&db)` 统一检查 `cwd`、`checkout_path()` 与 `workspace`，`mcp`/`skill`/`command`/`subagent`/`settings` 全部复用，已单独信任的 linked worktree 不再被误判。
  - **P1-3**：`PromptCommand::folder_destination` 读取 `--folder`（`prompt move`），`parent_destination` 读取 `--parent`（`prompt folder move`）。
  - **P1-4**：`prompt` / `folder` / `page` / `ref` 的写操作与按 ID 查询统一做当前工作区归属校验；`page active` 校验分页属于指定提示词。
  - **P1-5**：可选正文从非 TTY 空 `stdin` 返回 `Ok(None)`，非交互式 `skill edit` / `command edit` 不再清空正文；空正文与多行 frontmatter 字段被拒绝。
  - **P1-6**：`AddHttpArgs.auth` 改为 `Option<McpHttpAuth>`；`build_http` / `build_stdio` 对未显式传入的可选字段保留旧值，并补 `--env-http-header`。
- [x] **6.2 P2/P3 修复**
  - **P2-1**：补齐 `workspace add` 目录校验、`rename` 已登记校验、`reorder` 连续 `position`、live/historical worktree 删除、主仓库子项清理、会话图片清理与 `worktrees` 的 `current`/`head`。
  - **P2-2**：`resolve_target_session` 分页遍历全部会话并纳入空标题；`session list` 拒绝 `--limit 0` 且支持分页；`session rename` 支持 `--title`；`rewind` 先加锁再读、`restore-compact` dry-run 精确预检；`compact` 拦截工具调用；`delete_chat_session` 递归删除全部后代子会话并清理图片。
  - **P2-3**：`prompt create` 单事务写入初始分页；`todo done` 保留原 `completed_date`、`rollover --id` 拒绝已完成项、`todo add --text` 兼容。
  - **P2-4**：`mcp` 未信任工作区不加载项目配置、`list --scope user` 保留被覆盖项、`show` 掩码 headers/env、`login --json` 单次输出；`command list` 按 kind 分组并补字段；`skill edit/delete` 使用实际目录名；`subagent` 项目级读写统一 `checkout_path()`；`model` 拒绝空 ID 并前置校验 fetch；`settings open` 注入项目级命令快照。
  - **P2-5**：`extract_global_opts` 在 `--` 处停止并原样保留后续 token，已知带值选项保留取值 token；`todo` / `prompt` / `workspace` / `session` 等扫描器支持 `--` 终止符。
  - **P3-1**：`todo.rs` 拆出 `todo/exec.rs`，`model`/`prompt`/`mcp`/`workspace` 执行层二次拆分，`src/cli/` 生产文件均控制在 500 行以内。
  - **P3-2**：`platform::set_test_env` 补齐 `HOME` 隔离覆盖点（含 `config::skills::user_agents_skills_root()` 的 `~/.agents/skills`）；所有 `#[allow(clippy::large_enum_variant)]` 补齐原因注释。
  - **P3-3**：同步 `design.md`、`task.md`、`verification.md` 与 `docs/standards/project-directory-structure.md`。
- [x] **6.3 回归测试与测试隔离**
  - `platform::set_test_env(Some(home), Some(db_path))` 通过 `#[cfg(test)] std::thread_local!` 的 `TEST_HOME_OVERRIDE` / `TEST_DB_OVERRIDE` 注入隔离环境；`config::skills::user_agents_skills_root()`、`platform` 锁/图片归档路径与 CLI `database_file` 均优先读取 thread-local 覆盖，测试不会扫描宿主机真实的 `~/.agents/skills` 或共享开发库。
  - `CliTestEnv::new` 为每个用例创建独立临时 `HOME`、`~/.lx-t-dev/db/lx.db` 与工作区；`Drop` 时 `set_test_env(None, None)` 并删除临时根目录，保证测试并行安全。
  - P0–P2 回归由 `src/cli/tests/config_regressions_tests.rs`（7 个专项用例：`workspace_delete_include_worktrees_only_removes_subdirectory_target`、`workspace_add_and_rename_reject_unknown_paths`、`workspace_reorder_appends_unlisted_workspaces_without_duplicate_positions`、`mcp_add_updates_preserve_omitted_http_auth_headers_and_stdio_fields`、`skill_and_command_edit_preserve_body_on_disk`、`command_list_kind_keeps_user_command_shadowed_by_other_project_kind`、`model_rejects_empty_provider_and_model_ids`）与 `args_and_help_tests.rs`（`extract_global_opts_stops_at_double_dash_and_skips_value_slots`、`parse_cli_keeps_literal_tokens_after_double_dash`、`optional_content_ignores_empty_non_tty_stdin`）、`session_tests.rs`（零 limit/`rename --last --title`/`restore-compact` 预检）、`prompt_and_todo_tests.rs`（`prompt_move_and_folder_move_update_database_parents`、`prompt_writes_are_isolated_to_the_current_workspace`、`todo_text_flag_preserves_completion_and_rejects_rollover_for_done`）共同覆盖，并与 `verification.md` 的 V51–V56 端到端用例相互印证。
- [x] **6.4 最终校验**
  - `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过；`verification.md` 更新为 **56 / 56 PASS**。
