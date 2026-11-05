# lx 全功能无头 CLI (`lx-full-cli`) — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-full-cli/`
- **关联任务清单**：[`task.md`](task.md)
- **参考基线**：`src/chat/agent/exec/`（既有 `lx-tui exec` 无头 CLI）、`src/db/`、`src/config/`、`src/settings/`、`src/mcp/`、`src/skills/`、`src/git/`
- **目标范围**：`src/main.rs`、`src/lib.rs`、`src/cli/`（新增）、`src/chat/agent/exec/`、`src/event/chat/rewind.rs`、`src/settings/service/models/mod.rs`、`docs/standards/project-directory-structure.md`
- **当前状态**：**已在 `.worktrees/lx-full-cli` (`feature/lx-full-cli`) 完成实现与测试验证**

---

## 0. 代码库事实核验与前置基线

通过 `codegraph` 与 `codebase-memory-mcp` 检索核实，当前代码库真实状态与签名如下（设计严格建立在真实代码之上）：

| 核验维度 | 代码库当前真实状态 | 设计影响与应对策略 |
| :--- | :--- | :--- |
| **入口与 `exec` 解析器体积** | `src/main.rs`（103 行）直接调用 `chat::agent::exec::{parse_cli, run_main, help_text}`；而 `src/chat/agent/exec/cli.rs` 已达 **847 行**，逼近单文件 1000 行红线。 | 严禁往 `src/chat/agent/exec/cli.rs` 继续堆叠全项目子命令。新增顶层 `src/cli/` 模块负责顶层路由与新增领域子命令；`exec` 保持原有结构体与解析函数，由 `src/cli/` 直接委托转发。 |
| **工作区与开发库隔离** | `src/chat/agent/exec/mod.rs` 通过 `resolve_cwd(-C)` 解析目录，`git::list_worktrees(&cwd).repo_root` 解析项目工作区根，开发态（`db::is_dev_environment()`）通过 `git::linked_worktree_root(&cwd)` 定位隔离库 `Database::open_default(linked_worktree)`。 | 在 `src/cli/context.rs` 中提炼统一的 `CliWorkspaceContext`，供 `session`、`prompt`、`workspace`、`mcp`、`skill`、`command` 共用，保证 CLI 与 TUI / `exec` 读写完全相同的数据库与项目根。 |
| **工作区与历史列表 CRUD（兼容刚合入的 `feature/workspace-history-list` `07553c8c`）** | `SCHEMA_VERSION` 已升至 **24**（v23 新增 `workspace.position`，v24 新增 `workspace_hidden`）。`src/db/workspace.rs` 已从 `src/db/mod.rs` 拆出，并提供完整工作区历史列表能力：`list_workspaces()`、`list_workspace_history_entries()`（含 Git 主工作区与 linked worktree 树归并、`exists`、`trusted`、过滤 `workspace_hidden`）、`record_workspace_open`、`rename_workspace`、`delete_workspace`（`ON DELETE CASCADE` 级联清理文件夹/提示词/引用目录/聊天会话）、`save_workspace_order`、`hide_workspace`/`unhide_workspace`。 | 1. `lx-tui workspace` 子命令同步扩展完整工作区历史 CRUD：`list`（树形历史列表 / `--raw` 扁平表）、`add`（`record_workspace_open` + `unhide_workspace`）、`rename`（`rename_workspace`）、`delete`（校验无运行中会话锁 → linked worktree 写 `hide_workspace` → `delete_workspace` 级联删除，支持 `--include-worktrees` 一并清理同仓库子 worktree）、`reorder`（`save_workspace_order`）、`status`、`trust`、`worktrees`。<br>2. 写命令（`prompt create` 等）先调 `db.ensure_workspace(&ctx.workspace)`（`DO NOTHING` 不覆盖自定义工作区名）。 |
| **子 Agent 角色扩展与层级树（兼容刚合入的 `feature/lx-agent-roles-residency-and-panel` `65f90a62` 与 `339065ce`）** | 1. 内置角色已扩为 **3 个**：`explorer`、`executor`、**`monitor`**（`BaseRole::Monitor` / `AgentRole::Monitor`）；<br>2. `SubagentsSection` 新增 `delegation_mode: DelegationMode`（`auto\|proactive\|explicit`）；<br>3. `RoleOverrideConfig` / `CustomRoleConfig` 新增 `lock_model: bool` 与 `capabilities: RoleCapabilities`（`allow_web_search`/`allow_mcp`/`allow_skills`）；<br>4. 新增项目级角色发现 `src/config/subagents/discover.rs`（`discover_project_roles`、`merge_project_roles_for_paths` 扫描 `<workspace>/.lx/agents/*.json` 与 `.agents/roles/*.json`）；<br>5. `src/chat/agent/{path,status}.rs` 提供 `decode_stored_role_and_path` 与 `projected_agent_status`（7 态投影与队列感知）。 | 1. `lx-tui subagent` 完整对接 3 个内置角色（含 `monitor`）、`--delegation-mode`、`--lock-model`、`--allow-web-search`/`--allow-mcp`/`--allow-skills`，支持 `--scope <user\|project>` 查看合并后的项目级角色或读写 `<workspace>/.lx/agents/<name>.json`；<br>2. `lx-tui session agents` 复用 `decode_stored_role_and_path`、`has_queued_input` 与 `projected_agent_status` 输出准确的层级路径（`agent_path`）与 7 态状态。 |
| **跨进程会话锁** | `crate::platform::try_lock_session(session_id) -> io::Result<Option<SessionLockGuard>>` 已在 `exec` 与 `event/chat/rewind/restore.rs` 中用于防并发篡改。 | `session delete`、`session compact` 在 CLI 执行前必须调用 `platform::try_lock_session`；`session rewind` 复用 `event::chat::rewind::execute_rewind`（内部已持 `try_lock_session`）。若返回 `Ok(None)`，立即报错退出码 `1`。 |
| **Rewind 与 Compact 复用** | `src/event/chat/rewind/restore.rs:30` 的 `run(database, workspace, request)` 已完整实现预检、路径加锁、文件恢复、失败逆序补偿与 `commit_rewind`；`src/chat/compact.rs` 已提供纯数据 `playable_messages`、`incremental`、`summary_request`、`validate_summary`。 | 将 `restore::run` 通过 `src/event/chat/rewind.rs` 暴露为 `pub fn execute_rewind(database: &Database, workspace: &Path, request: &RewindRequest) -> RewindOutcome`；CLI 的 `session rewind` 与 `session compact` 直接复用现有事务与校验链路，零重复实现。 |
| **配置与扩展真实类型名** | 经核对源码：<br>- Todo：`TodoRecord`、`TodoStatus`、`TodoPriority`、`TodoDayStats`（`src/db/todo.rs`）<br>- AI：`AiSettings`、`ProviderConfig`、`ModelConfig`、`ModelSelection`（`src/config/ai/mod.rs`）<br>- MCP：`McpConfigFile`、`McpServerConfig`、`McpScope`、`effective_servers`（`src/config/mcp.rs`）<br>- Skills：`SkillsConfigFile`、`SkillEntry`、`discover::discover`、`store::{read_skill_at, write_skill, delete_skill}`（`src/config/skills.rs`、`src/skills/`）<br>- Commands：`CommandRecord`、`CommandKind`、`CommandScope`（`src/settings/commands.rs`）<br>- Subagents：`SubagentsConfigFile`、`SubagentsSection`、`CustomRoleConfig`（`src/config/subagents.rs`）<br>- Settings Web：`SettingsService::start`（`src/settings/service.rs`） | 所有子模块严格对接上述真实 Rust 类型与公开 API，所有配置文件保存均走已有的 0600 原子写函数（`save_ai`、`save_mcp`、`save_skills`、`save_subagents`、`config::save`、`commands::save`）。 |

---

## 1. Grill Me 已确认决策表

| # | 决策维度 | 最终确认结论 |
| :--- | :--- | :--- |
| **Q1** | **CLI 覆盖的功能域边界** | 覆盖全部 6 大持久化/独立业务功能域：**1. `session`（会话管理）**、**2. `prompt`（提示词库/文件夹/分页/引用目录）**、**3. `todo`（日程待办）**、**4. `workspace`（工作区信任与 Git worktree）**、**5. 配置与扩展生态（`model` / `mcp` / `skill` / `command` / `subagent`）**、**6. `settings`（通用布局设置读写与浏览器设置页服务）**，外加既有 `exec`。排除纯 TUI 内存态的分屏窗格与内置终端模拟器画面操作。 |
| **Q2** | **顶层命令树结构** | 采用**一级平铺结构**，命令深度控制在 2～3 级。顶层子命令固定为：`exec`、`session`、`prompt`、`todo`、`workspace`、`model`、`mcp`、`skill`、`command`、`subagent`、`settings`、`help`、`version`（无参数时保持进入全屏 TUI）。 |
| **Q3** | **输出格式与工作区定位** | 1. **双模式输出**：所有子命令默认在 `stdout` 输出人类易读文本/表格，统一支持 `--json` 输出结构化 JSON，支持 `--color <auto\|always\|never>`（遵循 `NO_COLOR`）；错误信息统一输出至 `stderr`，退出码 `0` 成功、`1` 失败、`130` 中断。<br>2. **工作区定位**：所有工作区相关命令默认取当前终端 `cwd`，统一支持 `-C, --cd <DIR>`，自动解析 Git 主 checkout 根路径与开发态 linked worktree 隔离库。 |
| **Q4** | **运行中会话的并发保护** | 1. **写操作加锁**：`session delete`、`session rewind`、`session compact` 执行前必须通过 `platform::try_lock_session` 获取跨进程文件锁；若会话正在 TUI 或其它 `exec` 进程中运行，立即拒绝并返回退出码 `1`。<br>2. **只读与安全派生免锁**：`session list/show/agents/processes/rename/fork` 不抢占运行锁（`fork` 仅克隆已结算轮次）。 |
| **Q5** | **非交互式长文本输入与特殊动作** | 1. **长文本写入（`prompt` / `skill` / `command`）**：统一支持 `--content <TEXT>`、`-f, --file <PATH>` 以及 `stdin` 管道（或 `-`）三种输入方式。<br>2. **`session rewind`**：纯参数驱动，支持 `--mode <revert-and-delete\|delete-only\|revert-only\|restore-compact>`（默认 `revert-and-delete`）与 `--dry-run` 预检。<br>3. **`settings open`**：启动本地 `127.0.0.1` 设置服务、自动打开浏览器（支持 `--no-open` 仅打印 URL），前台驻留直至 `Ctrl+C` 退出。 |
| **Q6** | **会话 `--last` 寻址、`--scope` 与代码落点** | 1. **会话寻址**：`session` 下所有单会话命令（`show/rename/delete/fork/rewind/compact/agents/processes`）统一支持 `<SESSION_ID> \| --last`。<br>2. **作用域参数**：`mcp`、`skill`、`command` 统一支持 `--scope <user\|project>`（写默认 `user`，`list` 默认合并展示全部并标注来源）。<br>3. **代码落点**：新增顶层 `src/cli/` 模块并按领域拆分子文件（单文件控制在 500 行以内），同步更新 `docs/standards/project-directory-structure.md`。 |

---

## 2. 总体架构与目录设计

### 2.1 新增与修改文件清单

```text
src/
  main.rs                        薄入口：调用 lx_tui::cli::parse_cli 与 lx_tui::cli::run_cli
  lib.rs                         导出 pub mod cli;
  cli/
    mod.rs                       顶层 CliAction 枚举、parse_cli 入口、run_cli 分发器 (~220 行)
    args.rs                      通用零依赖参数扫描器 RawArgs、GlobalFlags(-C/--json/--color)、长文本输入解析 (~320 行)
    context.rs                   CliContext：cwd/workspace/linked_worktree 解析、Database 打开、会话锁辅助 (~200 行)
    output.rs                    OutputWriter：Human 表格/键值排版、ANSI 颜色控制、--json 序列化输出 (~240 行)
    help.rs                      全局与各领域子命令英文帮助文本（HelpTopic 扩展与常量文本） (~380 行)
    session/
      mod.rs                     `lx-tui session` 参数解析与子命令分发（list/show/rename/delete/fork/agents/processes） (~450 行)
      ops.rs                     `session rewind`（含 --dry-run 与 4 种模式）与 `session compact`（含 --revert 与摘要流） (~380 行)
    prompt.rs                    `lx-tui prompt` 参数解析与执行（条目 CRUD、folder 子命令、page 子命令、ref 子命令） (~480 行)
    todo.rs                      `lx-tui todo` 参数解析与执行（list/add/update/done/undone/delete/reorder/rollover/stats） (~380 行)
    workspace.rs                 `lx-tui workspace` 参数解析与执行（list/add/rename/delete/reorder/status/trust/worktrees） (~340 行)
    model.rs                     `lx-tui model` 参数解析与执行（list/default/show/provider-set/provider-rm/model-set/model-rm/fetch） (~460 行)
    mcp.rs                       `lx-tui mcp` 参数解析与执行（list/show/add-stdio/add-http/remove/enable/disable/probe/login/logout） (~480 行)
    skill.rs                     `lx-tui skill` 参数解析与执行（list/show/enable/disable/create/edit/delete） (~360 行)
    command.rs                   `lx-tui command` 参数解析与执行（list/show/create/edit/rename/delete） (~360 行)
    subagent.rs                  `lx-tui subagent` 参数解析与执行（show/set/role-set/role-reset/role-rm，含 monitor 与项目级角色） (~420 行)
    settings.rs                  `lx-tui settings` 参数解析与执行（show/set/reset/open 前台驻留 Web 服务） (~320 行)
    tests/
      mod.rs                     CLI 测试入口与公共临时环境 Fixture (~120 行)
      args_and_help_tests.rs     顶层路由、全局标志、帮助主题与内容输入测试 (~280 行)
      session_tests.rs           session list/show/rename/delete/fork/rewind/compact/lock 测试 (~420 行)
      prompt_and_todo_tests.rs   prompt/folder/page/ref 与 todo 全链路 CRUD 测试 (~420 行)
      config_domains_tests.rs    workspace/model/mcp/skill/command/subagent/settings 测试 (~450 行)
```

### 2.2 模块依赖方向合规性

在 `docs/standards/project-directory-structure.md` 中：
- `main -> cli -> {chat, db, config, settings, mcp, skills, git, platform}`
- `cli/` **不依赖** `tui`、`ui`、`input`、`terminal`、`pty`（无头执行不初始化备用屏或原始模式）；
- 唯一复用的 `event` 能力是 `event::chat::rewind::execute_rewind`（纯同步 DB + 文件恢复函数，不启动 TUI 事件循环）。
- 所有 CLI 用户可见文案（帮助、表头、状态提示、错误消息）**100% 使用英文**，中文仅用于代码注释与 `tracing` 日志。

---

## 3. 全量命令树与参数规格（Command Specification）

### 3.1 全局通用标志（适用于除 `exec` 外的所有新增子命令）

| 标志 | 适用范围 | 说明 |
| :--- | :--- | :--- |
| `-C, --cd <DIR>` | 所有命令（对工作区命令生效） | 指定工作目录（默认当前进程 `cwd`），自动向上解析 Git 主仓库根路径作为 `workspace` |
| `--json` | 所有命令 | 将输出结果格式化为单行或标准结构化 JSON 打印到 `stdout` |
| `--color <auto\|always\|never>` | 所有命令 | 控制人类可读输出的 ANSI 颜色（默认 `auto`，遵守 `NO_COLOR` 环境变量） |
| `-h, --help` | 任意层级 | 打印当前命令或子命令的详细英文帮助并退出（退出码 `0`） |

---

### 3.2 各领域子命令完整清单

#### 1) `lx-tui exec`（完整保留既有行为）
- `lx-tui exec [OPTIONS] [PROMPT]`
- `lx-tui exec resume [SESSION_ID | --last] [OPTIONS] [PROMPT]`

#### 2) `lx-tui session`（聊天会话管理）
会话目标参数统一支持 `<SESSION_ID>` 或 `--last`（自动选取当前工作区最近更新的会话）：
- `lx-tui session list [--limit <N>] [--offset <N>] [-C <DIR>] [--json]`
  - 列出当前工作区的会话摘要（ID、标题、模型、状态、Token 用量、最后更新时间）。
- `lx-tui session show (<SESSION_ID> | --last) [--tools] [--thinking] [-C <DIR>] [--json]`
  - 查看会话完整消息历史（默认展示用户/助手正文与工具单行摘要；`--tools` 展开工具输入输出 JSON；`--thinking` 展示思考过程；同时展示 active compact checkpoint 与 notices）。
- `lx-tui session rename (<SESSION_ID> | --last) <NEW_TITLE> [-C <DIR>] [--json]`
  - 修改会话标题。
- `lx-tui session delete (<SESSION_ID> | --last) [-C <DIR>] [--json]`
  - **需获取会话锁**：级联删除会话及其消息、工具记录、进程记录与归档图片。
- `lx-tui session fork (<SESSION_ID> | --last) [--before <USER_MESSAGE_ID>] [-C <DIR>] [--json]`
  - 分叉会话（默认克隆全部已完成轮次；传 `--before` 则克隆该条用户消息之前的历史并返回 `restore_text`），打印新会话 ID。
- `lx-tui session rewind (<SESSION_ID> | --last) [--mode <revert-and-delete|delete-only|revert-only|restore-compact>] [--dry-run] [-C <DIR>] [--json]`
  - **需获取会话锁**：对最新一轮用户消息执行回滚（默认 `revert-and-delete`）。`--dry-run` 仅运行 `restore_plan` / `rewind_availability` 预检并列出受影响文件路径与可用性状态，不改磁盘与数据库。
- `lx-tui session compact (<SESSION_ID> | --last) [-m, --model <MODEL>] [--revert] [-C <DIR>] [--json]`
  - **需获取会话锁**：
    - 默认模式：加载会话可回放增量轮次，使用指定模型（或会话模型/默认模型）发起流式摘要请求，校验通过后调用 `db.commit_compact` 落库检查点；
    - `--revert` 模式：将当前 `active` 的压缩检查点还原（调用 `db.revert_compact`）。
- `lx-tui session agents (<SESSION_ID> | --last) [--tree] [--filter <all|active|done>] [-C <DIR>] [--json]`
  - 列出该会话派生的子 Agent（ID、昵称、角色、`agent_path` 树路径、模型、经 `projected_agent_status` + `has_queued_input` 投影后的 7 态状态、标题、修改文件数；`--tree` 递归展示整棵多级子 Agent 树；`--filter` 支持对齐 TUI 面板的 `all|active|done` 过滤）。
- `lx-tui session processes (<SESSION_ID> | --last) [-C <DIR>] [--json]`
  - 列出该会话关联的 `exec_command` 进程记录（进程号、状态、退出码、命令、耗时与尾部输出）。

#### 3) `lx-tui prompt`（提示词库、文件夹、多分页与引用目录）
内容输入统一支持 `--content <TEXT>`、`-f, --file <PATH>` 或管道 `stdin`（传 `-`）：
- **提示词条目**：
  - `lx-tui prompt list [--folder <FOLDER_ID>] [-C <DIR>] [--json]`：列出当前工作区的文件夹树与提示词条目。
  - `lx-tui prompt show <PROMPT_ID> [--page <PAGE_ID_OR_INDEX>] [-C <DIR>] [--json]`：查看提示词详情及全部（或指定）分页内容。
  - `lx-tui prompt create <NAME> [--folder <FOLDER_ID>] [--page-name <NAME>] [--content <TEXT> | -f <FILE>] [-C <DIR>] [--json]`：新建提示词（未提供内容时默认空串，首个分页名默认 `"Page 1"`）。
  - `lx-tui prompt edit <PROMPT_ID> [--page <PAGE_ID>] [--content <TEXT> | -f <FILE>] [-C <DIR>] [--json]`：更新提示词指定分页（未指定 `--page` 时默认更新 `active_page_id` 或第一页）的内容。
  - `lx-tui prompt rename <PROMPT_ID> <NEW_NAME> [-C <DIR>] [--json]`：重命名提示词。
  - `lx-tui prompt move <PROMPT_ID> [--folder <FOLDER_ID> | --root] [-C <DIR>] [--json]`：移动提示词到目标文件夹或根层。
  - `lx-tui prompt delete <PROMPT_ID> [-C <DIR>] [--json]`：删除提示词及其所有分页。
- **文件夹子命令 (`prompt folder`)**：
  - `lx-tui prompt folder list [-C <DIR>] [--json]`
  - `lx-tui prompt folder create <NAME> [--parent <FOLDER_ID>] [-C <DIR>] [--json]`（遵循最多 2 层约束）
  - `lx-tui prompt folder rename <FOLDER_ID> <NEW_NAME> [-C <DIR>] [--json]`
  - `lx-tui prompt folder move <FOLDER_ID> (--parent <FOLDER_ID> | --root) [-C <DIR>] [--json]`
  - `lx-tui prompt folder delete <FOLDER_ID> [-C <DIR>] [--json]`
- **分页子命令 (`prompt page`)**：
  - `lx-tui prompt page list <PROMPT_ID> [-C <DIR>] [--json]`
  - `lx-tui prompt page create <PROMPT_ID> <PAGE_NAME> [--content <TEXT> | -f <FILE>] [-C <DIR>] [--json]`
  - `lx-tui prompt page edit <PAGE_ID> [--content <TEXT> | -f <FILE>] [-C <DIR>] [--json]`
  - `lx-tui prompt page rename <PAGE_ID> <NEW_NAME> [-C <DIR>] [--json]`
  - `lx-tui prompt page active <PROMPT_ID> <PAGE_ID> [-C <DIR>] [--json]`
  - `lx-tui prompt page delete <PAGE_ID> [-C <DIR>] [--json]`
- **引用目录子命令 (`prompt ref`)**：
  - `lx-tui prompt ref list [-C <DIR>] [--json]`
  - `lx-tui prompt ref add <DIR_PATH> [-C <DIR>] [--json]`
  - `lx-tui prompt ref remove <REF_ID> [-C <DIR>] [--json]`

#### 4) `lx-tui todo`（全局日程待办）
- `lx-tui todo list [--date <YYYY-MM-DD> | --all | --past-incomplete] [--status <todo|in_progress|done>] [-p, --priority <P0|P1|P2|P3>] [--json]`
  - 默认列出今日（`today_date()`）待办；`--all` 列出全部日期；`--past-incomplete` 列出今日之前未完成的遗留项。
- `lx-tui todo add <TEXT> [--date <YYYY-MM-DD>] [-p, --priority <P0|P1|P2|P3>] [--status <todo|in_progress|done>] [--json]`
  - 新增待办（默认日期为今日，默认优先级 `P1`，默认状态 `todo`）。
- `lx-tui todo update <TODO_ID> [--text <TEXT>] [--status <todo|in_progress|done>] [-p, --priority <P0|P1|P2|P3>] [--date <YYYY-MM-DD>] [--json]`
  - 修改指定待办（支持传完整 ID 或唯一前缀）。
- `lx-tui todo done <TODO_ID> [--json]` / `lx-tui todo start <TODO_ID> [--json]` / `lx-tui todo reopen <TODO_ID> [--json]`
  - 快捷切换待办状态为 `done`（自动填入 `completed_date`）、`in_progress`、`todo`。
- `lx-tui todo delete <TODO_ID> [--json]`
  - 删除指定待办。
- `lx-tui todo reorder --date <YYYY-MM-DD> <ID1> <ID2> ... [--json]`
  - 按给定 ID 顺序重排某日待办。
- `lx-tui todo rollover [--to <YYYY-MM-DD>] [--id <TODO_ID>...] [--json]`
  - 将历史未完成待办（或指定 ID）顺延到目标日期（默认今日 `today_date()`）。
- `lx-tui todo stats [--days <N> | --from <YYYY-MM-DD> --to <YYYY-MM-DD>] [--json]`
  - 输出指定日期范围（默认最近 7 天）的计划数、完成数与按状态/优先级汇总统计。

#### 5) `lx-tui workspace`（工作区历史列表 CRUD、信任与 Git Worktree，兼容 `workspace-history-list`）
- `lx-tui workspace list [--raw] [--search <QUERY>] [--json]`
  - 默认调用 `db.list_workspace_history_entries()` 列出与 TUI 左侧栏 `ws list` 完全同源的工作区历史树（主工作区父项 + linked worktree 子项、显示名、路径、路径是否存在 `exists`、`trusted`、全局 `position`；支持 `--search` 按名称/分支/路径不区分大小写过滤）；传 `--raw` 则直接输出 `db.list_workspaces()` 原始表记录。
- `lx-tui workspace status [-C <DIR>] [--json]`
  - 显示当前工作区路径、保存的自定义名称、外部 ID、是否已信任、是否在 `workspace_hidden` 中、当前所在 Git 分支、是否属于 linked worktree 及对应的数据库文件路径。
- `lx-tui workspace add [<DIR>] [--name <NAME>] [--trust] [-C <DIR>] [--json]`
  - 显式将工作区登记到历史列表（调用 `db.record_workspace_open` + `db.unhide_workspace` 解除隐藏；若传 `--trust` 则同步调用 `db.trust_workspace` 授权信任）。
- `lx-tui workspace rename <NEW_NAME> [-C <DIR>] [--json]`
  - 重命名目标工作区（调用 `db.rename_workspace(&target, new_name)`，与 TUI 活跃/历史列表双向共享 `workspace.name`）。
- `lx-tui workspace delete [<DIR>] [--include-worktrees] [-C <DIR>] [--json]`
  - 从历史列表中级联删除目标工作区（对齐 `Effect::DeleteWorkspaceHistory`）：
    1. 分页遍历该工作区下全部聊天会话并检查 `platform::try_lock_session`，若有任一会话正在运行则报错拒绝删除；
    2. 若目标仍是磁盘上存在的 linked worktree（`path.join(".git").is_file()`），调用 `db.hide_workspace(&path)` 防止重启后通过 `git worktree list` 自动复活；
    3. 调用 `db.delete_workspace(&path)` 触发 SQLite `ON DELETE CASCADE` 级联删除该工作区下的文件夹、提示词、分页、引用目录及全部聊天会话；
    4. 若目标为 Git 主仓库且传入 `--include-worktrees`，一并级联删除并隐藏同仓库下的所有 linked worktree 历史记录。
- `lx-tui workspace reorder <PATH1> <PATH2> ... [--json]`
  - 按给定路径顺序重排工作区全局 `position`（调用 `db.save_workspace_order`）。
- `lx-tui workspace trust [--name <NAME>] [-C <DIR>] [--json]`
  - 将目标工作区标记为已信任（调用 `db.trust_workspace` + `db.unhide_workspace`）。
- `lx-tui workspace worktrees [-C <DIR>] [--json]`
  - 执行 `git::list_worktrees` 列出当前 Git 仓库的所有 worktree（路径、分支、HEAD、是否主 checkout、是否当前所在）。

#### 6) `lx-tui model`（AI 模型与 Provider 配置）
- `lx-tui model list [--all] [--json]`
  - 列出已启用（`--all` 含未启用）的 Provider 及其下的模型列表，标注当前 `defaultModel` 与 `titleSummary` 模型。
- `lx-tui model show [<PROVIDER_ID>[/<MODEL_ID>]] [--json]`
  - 查看全局 AI 配置概览（默认模型、标题模型、标题语言、流空闲超时、WebSearch Key 掩码状态）或单个 Provider/Model 详情（API Key 脱敏显示）。
- `lx-tui model default [<PROVIDER>/<MODEL>[#<VARIANT>] | --clear] [--title-summary] [--json]`
  - 查看或设置默认对话模型（加 `--title-summary` 则设置标题总结模型；`--clear` 清空）。
- `lx-tui model provider-set <PROVIDER_ID> [--type <openai|anthropic|google|openai-compatible>] [--name <NAME>] [--base-url <URL>] [--api-key <KEY> | --clear-api-key] [--enable | --disable] [--json]`
  - 新增或更新一个 Provider，并校验保存至 `config/ai.json`。
- `lx-tui model provider-rm <PROVIDER_ID> [--json]`
  - 删除指定 Provider（自动调用 `sanitize_optional_selections` 清理悬挂引用）。
- `lx-tui model model-set <PROVIDER_ID>/<MODEL_ID> [--name <NAME>] [--transport <openai|anthropic|google|openai-compatible|openai-responses>] [--context-limit <N>] [--output-limit <N>] [--variant <NAME>] [--image-input <true|false>] [--json]`
  - 在指定 Provider 下新增或更新模型定义。
- `lx-tui model model-rm <PROVIDER_ID>/<MODEL_ID> [--json]`
  - 删除指定模型定义。
- `lx-tui model fetch <PROVIDER_ID> [--base-url <URL>] [--api-key <KEY>] [--save] [--json]`
  - 调用远端 `/models` 接口拉取可用模型列表（默认使用该 Provider 已保存的 `baseURL`/`apiKey`）；传 `--save` 时自动将新发现的模型合并保存到该 Provider 下。
- `lx-tui model config [--title-language <auto|zh-CN|zh-TW|en|ja|ko>] [--timeout-ms <MS>] [--exa-key <KEY>] [--tavily-key <KEY>] [--json]`
  - 修改全局 AI 附属配置项（标题语言、流式超时、Exa/Tavily 搜索 Key）。

#### 7) `lx-tui mcp`（MCP 服务配置、探测与 OAuth）
- `lx-tui mcp list [--scope <all|user|project>] [-C <DIR>] [--json]`
  - 列出 MCP 服务（默认合并展示 `user` 与 `project` 作用域，标明传输类型、命令/URL、启用状态、是否覆盖用户级配置）。
- `lx-tui mcp show <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 查看单个 MCP 服务的详细配置（敏感 token 脱敏）。
- `lx-tui mcp add-stdio <NAME> --command <CMD> [--arg <ARG>...] [--cwd <DIR>] [--env <KEY=VAL>...] [--env-var <VAR>...] [--startup-timeout <SEC>] [--tool-timeout <SEC>] [--scope <user|project>] [-C <DIR>] [--json]`
  - 新增或更新 Stdio 类型 MCP 服务。
- `lx-tui mcp add-http <NAME> --url <URL> [--auth <none|bearer|oauth>] [--bearer-token <TOKEN>] [--bearer-env <VAR>] [--header <KEY=VAL>...] [--scope-item <SCOPE>...] [--client-id <ID>] [--client-secret <SECRET>] [--startup-timeout <SEC>] [--tool-timeout <SEC>] [--scope <user|project>] [-C <DIR>] [--json]`
  - 新增或更新 HTTP/SSE 类型 MCP 服务。
- `lx-tui mcp remove <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 删除指定作用域下的 MCP 服务配置。
- `lx-tui mcp enable <NAME> [--scope <user|project>] [-C <DIR>] [--json]` / `lx-tui mcp disable <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 启用或禁用指定 MCP 服务。
- `lx-tui mcp probe <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 实际启动/连接该 MCP 服务并握手探测（调用 `mcp::manager().probe`），输出发现的工具名称列表。
- `lx-tui mcp login <NAME> [--scope <user|project>] [--no-open] [-C <DIR>] [--json]`
  - 为 `auth=oauth` 的 HTTP MCP 服务启动临时本地回调监听、调用 `mcp::oauth::begin_authorization` 并打开浏览器授权，收到回调后调用 `complete_authorization` 将凭据写入 `<data>/mcp/credentials.json`。
- `lx-tui mcp logout <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 清除该 MCP 服务在 `credentials.json` 中的 OAuth 凭据。

#### 8) `lx-tui skill`（Skills 发现、启停与 CRUD）
- `lx-tui skill list [--scope <all|user|project>] [-C <DIR>] [--json]`
  - 调用 `skills::discover::discover` 列出四个来源根（项目 lx、项目 `.agents`、用户 lx、用户 `.agents`）发现的全部技能、启用状态、遮蔽关系（`shadowed_by`）与描述。
- `lx-tui skill show <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 查看指定技能的元数据、文件路径、是否可编辑及完整 `SKILL.md` 正文。
- `lx-tui skill enable <NAME_OR_PATH> [-C <DIR>] [--json]` / `lx-tui skill disable <NAME_OR_PATH> [-C <DIR>] [--json]`
  - 在 `config/skills.json` 中启用或禁用指定技能（兼容 `lx` 自有技能与 `.agents` 外部技能）。
- `lx-tui skill create <NAME> --description <DESC> [--content <TEXT> | -f <FILE>] [--scope <user|project>] [-C <DIR>] [--json]`
  - 在 `user` 或 `project` 的 lx 自有技能目录下创建 `<NAME>/SKILL.md`（调用 `skills::store::write_skill`）。
- `lx-tui skill edit <NAME> [--description <DESC>] [--content <TEXT> | -f <FILE>] [--scope <user|project>] [-C <DIR>] [--json]`
  - 修改已有 lx 自有技能的描述或正文（`.agents` 来源只读，修改时返回明确错误）。
- `lx-tui skill delete <NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 删除指定 lx 自有技能（调用 `skills::store::delete_skill`）。

#### 9) `lx-tui command`（自定义斜杠命令 `agentMD` / `agentBlock`）
- `lx-tui command list [--scope <all|user|project>] [--kind <agentMD|agentBlock>] [-C <DIR>] [--json]`
  - 列出自定义命令（名称、类型、作用域、块类型/MD 范围、描述、是否覆盖用户级同名命令）。
- `lx-tui command show <NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>] [-C <DIR>] [--json]`
  - 查看指定自定义命令的完整元数据与正文。
- `lx-tui command create <NAME> --kind <agentMD|agentBlock> --description <DESC> [--md-scope <global|template>] [--block <template|supple>] [--arg-hint <HINT>] [--title <TITLE>] [--content <TEXT> | -f <FILE>] [--scope <user|project>] [--overwrite] [-C <DIR>] [--json]`
  - 创建自定义命令（调用 `settings::commands::save`）。
- `lx-tui command edit <NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>] [--description <DESC>] [--md-scope <global|template>] [--block <template|supple>] [--arg-hint <HINT>] [--title <TITLE>] [--content <TEXT> | -f <FILE>] [-C <DIR>] [--json]`
  - 编辑已有自定义命令并原子保存。
- `lx-tui command rename <OLD_NAME> <NEW_NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>] [-C <DIR>] [--json]`
  - 重命名自定义命令（调用 `settings::commands::rename`）。
- `lx-tui command delete <NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>] [-C <DIR>] [--json]`
  - 删除自定义命令（调用 `settings::commands::delete`）。

#### 10) `lx-tui subagent`（子 Agent 全局与角色配置，兼容 `lx-agent-roles-residency-and-panel`）
- `lx-tui subagent show [<ROLE_NAME>] [--scope <all|user|project>] [-C <DIR>] [--json]`
  - 查看全局并发上限（`max_concurrent`）、最大深度（`max_depth`）、默认 Token 预算（`default_max_tokens`）、委派模式（`delegation_mode: auto|proactive|explicit`）以及所有内置角色（`explorer`、`executor`、**`monitor`**）、全局自定义角色与受信任工作区自动发现的项目级角色（`<workspace>/.lx/agents/*.json` 与 `.agents/roles/*.json`）。
- `lx-tui subagent set [--max-concurrent <1..12>] [--max-depth <1..3>] [--default-max-tokens <N> | --clear-max-tokens] [--delegation-mode <auto|proactive|explicit>] [--json]`
  - 更新子 Agent 全局参数并保存至 `config/subagents.json`。
- `lx-tui subagent role-set <NAME> [--base <explorer|executor|monitor>] [--description <DESC>] [--prompt <TEXT> | -f <FILE>] [--model <PROVIDER>/<MODEL>[#<VARIANT>] | --clear-model] [--lock-model <true|false>] [--allow-web-search <true|false>] [--allow-mcp <true|false>] [--allow-skills <true|false>] [--nicknames <N1,N2,...>] [--scope <user|project>] [-C <DIR>] [--json]`
  - 若 `<NAME>` 为 `explorer`、`executor` 或 `monitor`（仅 `--scope user`）：更新内置角色覆写（`BuiltinRoleOverrides`）；
  - 若 `<NAME>` 为自定义角色名：
    - `--scope user`（默认）：新增或更新 `config/subagents.json` 中的 `CustomRoleConfig`；
    - `--scope project`：原子写入 `<workspace>/.lx/agents/<NAME>.json` 项目级角色文件。
- `lx-tui subagent role-reset <explorer|executor|monitor> [--json]`
  - 清空指定内置角色的覆写配置，恢复内置默认。
- `lx-tui subagent role-rm <CUSTOM_ROLE_NAME> [--scope <user|project>] [-C <DIR>] [--json]`
  - 删除全局 `subagents.json` 中的自定义角色，或删除 `<workspace>/.lx/agents/<CUSTOM_ROLE_NAME>.json`（及 `.agents/roles/<CUSTOM_ROLE_NAME>.json`）项目级角色文件。

#### 11) `lx-tui settings`（通用设置与 Web 设置中心）
- `lx-tui settings show [--json]`
  - 显示 `settings.json` 中的 8 项布局/尺寸参数及数据目录路径（`user_data_root`、DB 路径、各配置文件路径）。
- `lx-tui settings set [--sidebar-width <N>] [--sidebar-min-width <N>] [--sidebar-max-width <N>] [--narrow-width <N>] [--min-width <N>] [--min-height <N>] [--min-pane-width <N>] [--min-pane-height <N>] [--json]`
  - 修改通用布局配置并通过 `Config::validate()` + `config::save_settings` 原子保存。
- `lx-tui settings reset [--json]`
  - 将 `settings.json` 重置为 `Config::default()`。
- `lx-tui settings open [--no-open] [-C <DIR>] [--json]`
  - 启动 `SettingsService::start` 本地回环 Web 服务，注入当前工作区项目信息与自定义命令快照，自动调用 `platform::open_browser(&url)` 打开浏览器（传 `--no-open` 时不拉起浏览器），在终端打印 URL 并前台等待 `tokio::signal::ctrl_c()` 后优雅关闭服务。

---

## 4. 核心模块与关键代码设计

### 4.1 顶层路由与入口分发 (`src/main.rs` & `src/cli/mod.rs`)

`src/main.rs` 保持精简，将 CLI 解析与执行交由 `lx_tui::cli`：

```rust
// src/cli/mod.rs
pub mod args;
pub mod command;
pub mod context;
pub mod help;
pub mod mcp;
pub mod model;
pub mod output;
pub mod prompt;
pub mod session;
pub mod settings;
pub mod skill;
pub mod subagent;
pub mod todo;
pub mod workspace;

use std::ffi::OsString;
use std::path::Path;

use crate::chat::agent::exec::{self, ExecCli};
pub use crate::chat::agent::exec::{EXIT_ERR, EXIT_INTERRUPTED, EXIT_OK};
pub use help::{HelpTopic, help_text};

/// 顶层 CLI 动作枚举：进程启动时一次性解析。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // 仅在启动入口构造一次，按值匹配清晰直接
pub enum CliAction {
    /// 无参数：进入全屏 TUI。
    Tui,
    /// `-h` / `--help` 或 `lx-tui help [topic...]`。
    Help(HelpTopic),
    /// `-V` / `--version` / `version`。
    Version,
    /// `lx-tui exec ...`：无头运行 Agent。
    Exec(ExecCli),
    /// `lx-tui session ...`：聊天会话管理。
    Session(session::SessionCli),
    /// `lx-tui prompt ...`：提示词库管理。
    Prompt(prompt::PromptCli),
    /// `lx-tui todo ...`：全局日程待办管理。
    Todo(todo::TodoCli),
    /// `lx-tui workspace ...`：工作区信任与 Git worktree。
    Workspace(workspace::WorkspaceCli),
    /// `lx-tui model ...`：AI 模型与供应商配置。
    Model(model::ModelCli),
    /// `lx-tui mcp ...`：MCP 服务管理与探测/授权。
    Mcp(mcp::McpCli),
    /// `lx-tui skill ...`：Skills 发现、启停与读写。
    Skill(skill::SkillCli),
    /// `lx-tui command ...`：自定义斜杠命令管理。
    Command(command::CommandCli),
    /// `lx-tui subagent ...`：子 Agent 角色与并发配置。
    Subagent(subagent::SubagentCli),
    /// `lx-tui settings ...`：通用设置与浏览器设置服务。
    Settings(settings::SettingsCli),
}

pub fn parse_cli<I, T>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let Some(first) = args.get(1) else {
        return Ok(CliAction::Tui);
    };
    let sub = args::utf8(first)?;
    let tail = &args[2..];
    match sub {
        "-h" | "--help" => Ok(CliAction::Help(HelpTopic::Root)),
        "-V" | "--version" | "version" => Ok(CliAction::Version),
        "help" => help::parse_help(tail),
        "exec" => match exec::parse_cli(args) {
            Ok(exec::CliAction::Exec(cli)) => Ok(CliAction::Exec(cli)),
            Ok(exec::CliAction::Help(topic)) => Ok(CliAction::Help(HelpTopic::from_exec(topic))),
            Ok(exec::CliAction::Version) => Ok(CliAction::Version),
            Ok(exec::CliAction::Tui) => Ok(CliAction::Tui),
            Err(err) => Err(err),
        },
        "session" => session::parse(tail),
        "prompt" => prompt::parse(tail),
        "todo" => todo::parse(tail),
        "workspace" => workspace::parse(tail),
        "model" => model::parse(tail),
        "mcp" => mcp::parse(tail),
        "skill" => skill::parse(tail),
        "command" => command::parse(tail),
        "subagent" => subagent::parse(tail),
        "settings" => settings::parse(tail),
        other => Err(format!(
            "unexpected argument '{other}'; run 'lx-tui --help' for usage"
        )),
    }
}

/// 统一执行除 `Tui` 外的全部 CLI 动作。
pub fn run_cli(
    action: CliAction,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    match action {
        CliAction::Tui => Ok(EXIT_OK),
        CliAction::Help(topic) => {
            print!("{}", help_text(topic));
            Ok(EXIT_OK)
        }
        CliAction::Version => {
            println!("lx-tui {}", env!("CARGO_PKG_VERSION"));
            Ok(EXIT_OK)
        }
        CliAction::Exec(cli) => exec::run_main(cli, linked_worktree),
        CliAction::Session(cli) => session::run(cli, linked_worktree),
        CliAction::Prompt(cli) => prompt::run(cli, linked_worktree),
        CliAction::Todo(cli) => todo::run(cli, linked_worktree),
        CliAction::Workspace(cli) => workspace::run(cli, linked_worktree),
        CliAction::Model(cli) => model::run(cli),
        CliAction::Mcp(cli) => mcp::run(cli, linked_worktree),
        CliAction::Skill(cli) => skill::run(cli, linked_worktree),
        CliAction::Command(cli) => command::run(cli, linked_worktree),
        CliAction::Subagent(cli) => subagent::run(cli, linked_worktree),
        CliAction::Settings(cli) => settings::run(cli, linked_worktree),
    }
}
```

---

### 4.2 通用参数扫描器与长文本输入解析 (`src/cli/args.rs`)

所有子命令共享 `GlobalOpts` 与 `ContentSource`，避免各模块重复手写 `--flag=val`、`-C`、`--json`、`--color` 与文件/`stdin` 读取：

```rust
// src/cli/args.rs
use std::ffi::{OsStr, OsString};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use crate::chat::agent::exec::ColorMode;

/// 所有新增子命令共用的全局选项。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlobalOpts {
    /// `-C, --cd <DIR>`：目标工作目录。
    pub cwd: Option<PathBuf>,
    /// `--json`：输出结构化 JSON。
    pub json: bool,
    /// `--color <auto|always|never>`。
    pub color: ColorMode,
}

/// 多行长文本输入来源（用于 prompt / skill / command / subagent prompt）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentInput {
    /// `--content <TEXT>`
    pub inline: Option<String>,
    /// `-f, --file <PATH>`
    pub file: Option<PathBuf>,
}

impl ContentInput {
    /// 解析最终文本：优先级 `--content`（若为 `"-"` 则读 stdin） > `--file` > 非终端管道 stdin。
    /// `required = true` 时若三者皆无则报错；`required = false` 时返回 `Ok(None)`。
    pub fn resolve(
        &self,
        required: bool,
        stdin_is_terminal: bool,
        read_stdin: impl FnOnce() -> std::io::Result<String>,
    ) -> Result<Option<String>, String> {
        if self.inline.is_some() && self.file.is_some() {
            return Err("cannot specify both '--content' and '--file'".to_string());
        }
        if let Some(raw) = &self.inline {
            if raw == "-" {
                let text = read_stdin().map_err(|e| format!("failed to read stdin: {e}"))?;
                return Ok(Some(text));
            }
            return Ok(Some(raw.clone()));
        }
        if let Some(path) = &self.file {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
            return Ok(Some(text));
        }
        if !stdin_is_terminal {
            let text = read_stdin().map_err(|e| format!("failed to read stdin: {e}"))?;
            return Ok(Some(text));
        }
        if required {
            Err("missing content; pass '--content <TEXT>', '-f <FILE>', or pipe via stdin".to_string())
        } else {
            Ok(None)
        }
    }
}

/// 扫描并剥离全局选项（`-C/--cd`、`--json`、`--color`、`-h/--help`），保留其余参数按原顺序交由子命令解析。
pub fn extract_global_opts(
    raw_args: &[OsString],
) -> Result<(GlobalOpts, Vec<String>, bool), String> {
    let mut global = GlobalOpts::default();
    let mut rest = Vec::with_capacity(raw_args.len());
    let mut help_requested = false;
    let mut i = 0;
    while i < raw_args.len() {
        let arg = utf8(&raw_args[i])?;
        if arg == "--" {
            rest.push("--".to_string());
            for item in &raw_args[i + 1..] {
                rest.push(utf8(item)?.to_string());
            }
            break;
        }
        if matches!(arg, "-h" | "--help") {
            help_requested = true;
            i += 1;
            continue;
        }
        if arg == "--json" {
            global.json = true;
            i += 1;
            continue;
        }
        if let Some(val) = take_opt_value(raw_args, &mut i, arg, "-C", "--cd")? {
            global.cwd = Some(PathBuf::from(required_non_empty(&val, "--cd")?));
            i += 1;
            continue;
        }
        if let Some(val) = take_opt_value(raw_args, &mut i, arg, "", "--color")? {
            global.color = ColorMode::parse(&val)?;
            i += 1;
            continue;
        }
        rest.push(arg.to_string());
        i += 1;
    }
    Ok((global, rest, help_requested))
}
```

---

### 4.3 工作区上下文、数据库打开与会话锁 (`src/cli/context.rs`)

严格对齐 `src/chat/agent/exec/mod.rs` 的目录解析与开发态 worktree 数据库隔离规则：

```rust
// src/cli/context.rs
use std::path::{Path, PathBuf};

use crate::db::{self, Database};
use crate::git;
use crate::platform::SessionLockGuard;

/// 工作区与数据库解析上下文。
pub struct WorkspaceContext {
    /// 实际执行目录（`-C` 或当前进程 `cwd`，已 canonicalize）。
    pub cwd: PathBuf,
    /// 项目主工作区根路径（Git 主 checkout 根；非 Git 目录则等于 `cwd`）。
    pub workspace: PathBuf,
    /// 开发环境下的 linked worktree 根路径（用于隔离开发数据库）。
    pub effective_worktree: Option<PathBuf>,
}

impl WorkspaceContext {
    pub fn resolve(
        cwd_override: Option<&Path>,
        entry_linked_worktree: Option<&Path>,
    ) -> Result<Self, String> {
        let base = std::env::current_dir()
            .map_err(|e| format!("failed to read current directory: {e}"))?;
        let target = match cwd_override {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => base.join(path),
            None => base,
        };
        let meta = std::fs::metadata(&target)
            .map_err(|e| format!("working directory '{}' is not accessible: {e}", target.display()))?;
        if !meta.is_dir() {
            return Err(format!("working directory '{}' is not a directory", target.display()));
        }
        let cwd = std::fs::canonicalize(&target).unwrap_or(target);
        let workspace = git::list_worktrees(&cwd)
            .map(|list| list.repo_root)
            .unwrap_or_else(|_| cwd.clone());
        let effective_worktree = if cwd_override.is_some() {
            if db::is_dev_environment() {
                git::linked_worktree_root(&cwd)
            } else {
                None
            }
        } else {
            entry_linked_worktree
                .map(Path::to_path_buf)
                .or_else(|| db::is_dev_environment().then(|| git::linked_worktree_root(&cwd)).flatten())
        };
        Ok(Self {
            cwd,
            workspace,
            effective_worktree,
        })
    }

    /// 打开默认 SQLite 数据库。
    pub fn open_db(&self) -> Result<Database, String> {
        Database::open_default(self.effective_worktree.as_deref())
            .map_err(|e| format!("failed to open database: {}", e.code()))
    }
}

/// 解析 `<SESSION_ID> | --last` 目标会话（支持精确匹配或唯一前缀匹配）。
pub fn resolve_target_session(
    db: &Database,
    workspace: &Path,
    session_id: Option<&str>,
    last: bool,
) -> Result<String, String> {
    if last && session_id.is_some() {
        return Err("cannot specify both a session ID and '--last'".to_string());
    }
    if !last && session_id.is_none() {
        return Err("specify a <SESSION_ID> or pass '--last'".to_string());
    }
    let sessions = db
        .list_chat_sessions(workspace, 0, 200)
        .map_err(|e| format!("failed to list sessions: {}", e.code()))?;
    if last {
        return sessions
            .into_iter()
            .next()
            .map(|s| s.external_id)
            .ok_or_else(|| format!("no sessions found in workspace '{}'", workspace.display()));
    }
    let query = session_id.unwrap_or_default().trim();
    if let Some(exact) = sessions.iter().find(|s| s.external_id == query) {
        return Ok(exact.external_id.clone());
    }
    // 若不在前 200 条但数据库中精确存在：
    if db
        .chat_session_title(workspace, query)
        .map_err(|e| format!("failed to query session: {}", e.code()))?
        .is_some()
    {
        return Ok(query.to_string());
    }
    let prefix_matches: Vec<&str> = sessions
        .iter()
        .filter(|s| s.external_id.starts_with(query))
        .map(|s| s.external_id.as_str())
        .collect();
    match prefix_matches.as_slice() {
        [single] => Ok((*single).to_string()),
        [] => Err(format!(
            "session '{query}' not found in workspace '{}'",
            workspace.display()
        )),
        many => Err(format!(
            "session prefix '{query}' is ambiguous (matches: {})",
            many.join(", ")
        )),
    }
}

/// 获取跨进程会话独占锁；被 TUI 或其它 `exec` 占用时返回可读英文错误。
pub fn acquire_session_lock(session_id: &str) -> Result<SessionLockGuard, String> {
    match crate::platform::try_lock_session(session_id) {
        Ok(Some(guard)) => Ok(guard),
        Ok(None) => Err(format!(
            "session '{session_id}' is currently active in another lx-tui process; stop the active turn first"
        )),
        Err(err) => Err(format!("failed to lock session '{session_id}': {err}")),
    }
}
```

---

### 4.4 双模式输出器 (`src/cli/output.rs`)

统一处理 `--json` 与人类可读表格/键值输出，保证脚本解析与人工阅读体验：

```rust
// src/cli/output.rs
use std::io::{IsTerminal, Write};
use serde::Serialize;
use crate::chat::agent::exec::ColorMode;

pub struct Output {
    json: bool,
    color: bool,
}

impl Output {
    pub fn new(json: bool, color_mode: ColorMode) -> Self {
        let is_tty = std::io::stdout().is_terminal();
        Self {
            json,
            color: !json && color_mode.should_color(is_tty),
        }
    }

    pub fn is_json(&self) -> bool {
        self.json
    }

    /// 输出 JSON 或执行人类可读渲染闭包。
    pub fn emit<T, F>(&self, value: &T, human: F) -> Result<(), String>
    where
        T: Serialize,
        F: FnOnce(&Self, &mut dyn Write) -> std::io::Result<()>,
    {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        if self.json {
            serde_json::to_writer_pretty(&mut handle, value)
                .map_err(|e| format!("failed to serialize JSON: {e}"))?;
            writeln!(handle).map_err(|e| format!("stdout write failed: {e}"))?;
        } else {
            human(self, &mut handle).map_err(|e| format!("stdout write failed: {e}"))?;
        }
        handle.flush().map_err(|e| format!("stdout flush failed: {e}"))
    }

    /// 打印对齐的纯文本表格（自动计算列宽，表头加粗/暗色）。
    pub fn print_table(
        &self,
        out: &mut dyn Write,
        headers: &[&str],
        rows: &[Vec<String>],
    ) -> std::io::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        for row in rows {
            for (idx, cell) in row.iter().enumerate() {
                if idx < widths.len() {
                    widths[idx] = widths[idx].max(cell.chars().count());
                }
            }
        }
        // 打印表头与数据行...
        Ok(())
    }
}
```

---

### 4.5 会话回滚与压缩关键实现 (`src/cli/session/ops.rs`)

#### 4.5.1 `session rewind`：复用 `event::chat::rewind`
在 `src/event/chat/rewind.rs` 中公开：
```rust
/// 供无头 CLI 与事件层后台线程共用的同步撤销执行器。
pub fn execute_rewind(
    database: & crate::db::Database,
    workspace: &std::path::Path,
    request: &crate::app::effects::RewindRequest,
) -> RewindOutcome {
    restore::run(database, workspace, request)
}
```
在 `src/cli/session/ops.rs` 中：
- 若 `--mode restore-compact`：先获取 `acquire_session_lock(&session_id)`，查询 `db.active_checkpoint(&ctx.workspace, &session_id)` 与 `db.session_revision(&session_id)`，调用 `db.revert_compact(...)`。
- 若 `--dry-run`：查询 `db.latest_user_turn(&ctx.workspace, &session_id)`，调用 `db.restore_plan(&ctx.workspace, &session_id, &latest.user_message_id)`，输出目标用户消息、文件可用性（`Ready`/`KnownEmpty`/`AlreadyRestored`/`Unavailable`/`Incomplete`）与受影响文件路径清单，不写库不改盘。
- 若正常执行（`revert-and-delete` / `delete-only` / `revert-only`）：构造 `RewindRequest`，直接调用 `crate::event::chat::rewind::execute_rewind(&db, &ctx.workspace, &request)`，根据 `RewindOutcome::Succeeded` / `Failed` 输出结果或返回退出码 `1`。

#### 4.5.2 `session compact`：复用 `chat::compact` 与 `db::chat::compact`
```rust
// src/cli/session/ops.rs (关键代码)
pub fn run_compact(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    model_override: Option<&str>,
    revert: bool,
    out: &Output,
) -> Result<i32, String> {
    let _guard = acquire_session_lock(session_id)?;
    let revision = db
        .session_revision(session_id)
        .map_err(|e| format!("failed to read session revision: {}", e.code()))?;
    let active = db
        .active_checkpoint(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to query checkpoint: {}", e.code()))?;

    if revert {
        let Some(cp) = active else {
            return Err(format!("session '{session_id}' has no active compact checkpoint to revert"));
        };
        let reverted = db
            .revert_compact(
                &ctx.workspace,
                &crate::db::chat::CompactRevert {
                    operation_id: format!("cli-compact-revert-{}", unique_suffix()),
                    session_id: session_id.to_string(),
                    checkpoint_id: cp.id,
                    expected_revision: revision,
                },
            )
            .map_err(|e| format!("failed to revert compact checkpoint: {}", e.code()))?;
        // 输出 JSON 或人类可读确认
        return Ok(EXIT_OK);
    }

    let messages = db
        .load_chat_session(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to load session: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' not found"))?;
    let playable = crate::chat::compact::playable_messages(&messages);
    let input = crate::chat::compact::incremental(&playable, active.as_ref().map(|c| c.end_sequence));
    if input.is_empty() {
        return Err("no new completed turns available to compact".to_string());
    }

    // 解析模型目标（优先级：-m > 会话模型 > 默认模型 > 首个可用模型）
    let ai_settings = load_ai_settings()?;
    let session_model = db.chat_session_model(&ctx.workspace, session_id).ok().flatten();
    let target = crate::chat::agent::exec::resolve_model_target(
        &ai_settings,
        model_override,
        session_model.as_ref(),
    )?;
    let snapshot = target.snapshot();
    let chat_request = crate::chat::compact::summary_request(
        &input,
        &snapshot,
        target.variant_params.clone(),
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;
    let summary = runtime.block_on(async {
        let (_cancel_tx, cancel_rx) = crate::chat::client::uncancellable();
        let mut text = String::new();
        let outcome = crate::chat::client::run_stream(
            &target.transport,
            &target.base_url,
            Some(target.api_key.as_str()),
            std::time::Duration::from_millis(target.idle_timeout_ms),
            chat_request,
            cancel_rx,
            |delta| {
                if let crate::chat::events::ChatDelta::Text(chunk) = delta {
                    text.push_str(&chunk);
                }
            },
        )
        .await;
        if outcome.stop != crate::chat::events::StopReason::Complete {
            return Err(format!("compact summary request failed: {}", outcome.stop.as_str()));
        }
        crate::chat::compact::validate_summary(&text).map_err(str::to_string)
    })?;

    let committed = db
        .commit_compact(
            &ctx.workspace,
            &crate::db::chat::CompactCommit {
                session_id: session_id.to_string(),
                expected_revision: revision,
                base_checkpoint_id: active.map(|c| c.id),
                start_sequence: input.start_sequence,
                end_sequence: input.end_sequence,
                summary_text: summary,
                model_label: target.display_name,
                operation_id: format!("cli-compact-{}", unique_suffix()),
            },
        )
        .map_err(|e| format!("failed to commit compact checkpoint: {}", e.code()))?;
    // 输出 committed.checkpoint
    Ok(EXIT_OK)
}
```

---

### 4.6 MCP 探测与 OAuth 登录 (`src/cli/mcp.rs`) & 设置服务驻留 (`src/cli/settings.rs`)

#### 4.6.1 `mcp probe` 与 `mcp login`
- `mcp probe <NAME>`：读取用户级与项目级 `McpConfigFile`，构造目标 `EffectiveServer`，在 Tokio runtime 中调用 `crate::mcp::manager().probe(&effective, &ctx.workspace).await`，成功后调用 `crate::mcp::manager().shutdown().await` 清理子进程，打印工具列表。
- `mcp login <NAME>`：
  1. 绑定临时本地回环端口 `tokio::net::TcpListener::bind("127.0.0.1:0")`，构造 `redirect_uri = format!("http://127.0.0.1:{port}/mcp/oauth/callback")`；
  2. 调用 `crate::mcp::oauth::begin_authorization(&effective, &ctx.workspace, &redirect_uri).await` 获取 `PendingAuthorization`；
  3. 打印 `Authorize URL: ...`，若未传 `--no-open` 则调用 `crate::platform::open_browser(&pending.authorize_url)`；
  4. 等待浏览器回调请求，校验 `state == pending.state`，提取 `code`，调用 `crate::mcp::oauth::complete_authorization(&pending, &code, &store).await` 落盘到 `<data>/mcp/credentials.json`，向浏览器返回成功 HTML 页面并退出。

#### 4.6.2 `settings open` 前台驻留服务
```rust
// src/cli/settings.rs (关键代码)
fn run_open(ctx: &WorkspaceContext, no_open: bool, out: &Output) -> Result<i32, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to initialize async runtime: {e}"))?;
    runtime.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let service = crate::settings::service::SettingsService::start(tx)
            .await
            .map_err(|e| format!("failed to start settings server: {e}"))?;
        // 注入当前工作区项目视图，使设置页中的 Project 级 MCP/Skills 可见
        if let Ok(db) = ctx.open_db() {
            let trusted = db.is_trusted(&ctx.workspace).unwrap_or(false);
            let name = ctx
                .workspace
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_string());
            let scope = crate::platform::path_text::scope_dir_name(&ctx.workspace);
            service.set_projects(vec![crate::settings::service::ProjectView {
                scope,
                root: ctx.workspace.clone(),
                name,
                trusted,
            }]);
        }
        let url = service.url();
        if !no_open {
            let _ = crate::platform::open_browser(&url);
        }
        if out.is_json() {
            println!(
                "{}",
                serde_json::json!({
                    "url": url,
                    "port": service.port(),
                    "workspace": ctx.workspace,
                })
            );
        } else {
            eprintln!("Settings server running at: {url}");
            eprintln!("Press Ctrl+C to stop.");
        }
        // 消费后台事件防止通道堆积，同时等待 Ctrl+C
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                 maybe_ev = rx.recv() => {
                    if maybe_ev.is_none() {
                        break;
                    }
                }
            }
        }
        service.shutdown();
        crate::mcp::manager().shutdown().await;
        Ok(EXIT_OK)
    })
}
```

---

## 5. 错误处理、安全边界与退出码契约

1. **退出码契约**：
   - `EXIT_OK (0)`：命令成功完成；
   - `EXIT_ERR (1)`：命令行参数非法、目标资源不存在、会话锁冲突、配置校验不通过、数据库或网络请求失败；
   - `EXIT_INTERRUPTED (130)`：`exec` 或阻塞命令被 `Ctrl+C` 中断。
2. **敏感信息脱敏**：
   - `model show`、`mcp show` 在人类可读与 `--json` 输出中默认对 `apiKey`、`bearer_token`、`client_secret`、`exa_api_key`、`tavily_api_key` 执行掩码脱敏（如仅显示前 3 位与后 4 位 `sk-...abcd` 或 `"***"`），防止终端录屏或 Agent 调用日志泄露密钥。
3. **文件权限与原子性**：
   - 所有配置写入严格复用既有 `save_ai`、`save_mcp`、`save_skills`、`save_subagents`、`config::save`、`commands::save`（内部均走 `platform::write_file_atomic` 与 `0600` 权限）。
4. **代码质量硬约束**：
   - 生产代码零 `unwrap()` / `expect()`；
   - 单文件严格控制在 500 行左右（绝不超过 1000 行上限）；
   - 所有用户可见输出与帮助文本 100% 英文。
