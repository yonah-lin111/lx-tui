# `lx-tui` 全功能无头 CLI 代码审查报告（Review）

- **审查分支**：`feature/lx-full-cli`（已合入主工作区 `dev`，原工作树 `.worktrees/lx-full-cli` 已清理）
- **审查范围**：`src/cli/**`、`src/main.rs`、`src/lib.rs`、`src/db/mod.rs`、`src/platform/**`、`src/event/chat/rewind/**`、`src/settings/service/**` 以及 `docs/feature/lx-full-cli/{design,task,verification}.md`、`docs/standards/project-directory-structure.md`
- **优先级定义**：
  - **P0（致命）**：可能导致用户数据或工作区历史被误删、破坏性极强的缺陷
  - **P1（高危）**：核心子命令功能失效、跨工作区越权修改、非交互调用静默清空用户配置/正文、更新配置丢失密钥
  - **P2（中危）**：边界条件缺陷、信任门控不一致、多步操作非原子、参数扫描吞词、测试断言不充分
  - **P3（低危/规范）**：单文件行数超 500 行软上限、测试环境隔离遗漏、Lint 属性注释缺失、设计/任务/验证文档与实现漂移

---

## 1. P0 级问题（致命）

### P0-1：`workspace delete --include-worktrees` 在仓库子目录目标上会误删主仓库及全部 linked worktree
- **定位**：`src/cli/workspace/exec.rs:309`、`src/cli/workspace/exec.rs:340-363`
- **关键点**：当用户将某个 Git 仓库的子目录（例如 `<repo>/src`）登记为工作区，并对其执行 `lx-tui workspace delete <repo>/src --include-worktrees` 时，命令不仅删除 `<repo>/src`，还会把主仓库 `<repo>` 及该仓库下所有 linked worktree 一并从历史记录和会话库中删除。
- **错误原因**：
  1. 代码使用 `!target.join(".git").is_file()` 判断 `target` 是否为“主工作树根”，但普通仓库子目录（如 `<repo>/src`）同样不存在 `.git` 文件，因此被误判为主仓库；
  2. `linked_worktrees(db, target)` 调用 `git::list_worktrees(target)` 后，仅过滤 `entry.path != target`，未校验 `list.repo_root == target`，也未排除主工作树 `list.repo_root`，导致主仓库和所有 linked worktree 全部进入删除列表；
  3. 历史记录兜底扫描使用 `record.path.starts_with(target) && record.path.join(".git").is_file()`，会把子目录内的嵌套子模块误判为 linked worktree。
- **修复方案**：
  1. 仅当 `git::list_worktrees(target)` 返回 `list.repo_root == target` 时，才允许将其作为主仓库展开关联的 linked worktree（并且只收集 `entry.path != list.repo_root` 的 linked worktree）；
  2. 对不在当前 `git worktree list` 中的历史残留目录，解析其 `.git` 文件中的 `gitdir:` 路径，仅当其指向 `target/.git/worktrees/` 下时才归入同一仓库的 linked worktree；
  3. 补充针对 `<repo>/subdir` 执行 `workspace delete --include-worktrees` 的回归测试。
- **状态**：已修复

---

## 2. P1 级问题（高危）

### P1-1：Linked Worktree 下 `workspace delete/add/trust/status` 默认目标取主仓库根，无参 `delete` 会误删主仓库
- **定位**：`src/cli/context.rs:46-48`、`src/cli/workspace/exec.rs:174, 298, 438, 494-499`
- **关键点**：在 linked worktree 目录（如 `.worktrees/feat`）中不传 `<DIR>` 执行 `lx-tui workspace delete`、`workspace add`、`workspace trust` 或 `workspace status` 时，默认操作目标是主仓库根而非当前所在的 linked worktree；其中无参 `workspace delete` 会直接删掉主仓库及其会话。
- **错误原因**：`WorkspaceContext.workspace` 为了对齐会话/提示词归属被解析为主仓库 `repo_root`，而 `workspace` 领域的 `resolve_path(ctx, None)` 直接返回了 `ctx.workspace`，将“数据库归属工作区键”与“当前检出工作树（checkout root）”混为一谈（仅 `rename` 对 `ctx.cwd` 做了特殊判断）。
- **修复方案**：在 `WorkspaceContext` 新增 `checkout_path(&self) -> PathBuf`（通过 `git::list_worktrees(&self.cwd)` 与 `git::checkout` 解析当前检出根，若非 Git 仓库则回退 `workspace`）；`workspace add/delete/trust/status/rename` 在未显式传入 `<DIR>` 时统一以 `ctx.checkout_path()` 为默认目标。
- **状态**：已修复

### P1-2：项目级写操作的信任门控仅检查主仓库根，已单独信任的 Linked Worktree 被误判为未信任
- **定位**：`src/cli/skill/exec.rs:42-45`、`src/cli/command/exec.rs:32-35`、`src/cli/mcp/exec.rs:33-36`、`src/cli/subagent/exec.rs:24-27`
- **关键点**：TUI 的信任判定逻辑为 `db.is_trusted(cwd) || db.is_trusted(repo_root)`（见 `src/event/trust.rs:45-55`），而 CLI 各模块仅检查 `db.is_trusted(&ctx.workspace)`。当用户对某个 linked worktree 单独执行信任后，在该 worktree 下执行 `--scope project` 写入仍被拒绝。
- **错误原因**：`WorkspaceContext` 拆分 `cwd` 与 `workspace` 后，各领域模块各自编写了只查 `ctx.workspace` 的 `is_trusted` 辅助函数。
- **修复方案**：在 `WorkspaceContext` 上提供统一的 `is_trusted(&self, db: &Database) -> bool`（综合检查 `self.cwd`、`self.checkout_path()` 与 `self.workspace`），并在 `mcp`、`skill`、`command`、`subagent`、`settings` 中统一复用。
- **状态**：已修复

### P1-3：`prompt move --folder <ID>` 因读取错误字段完全失效
- **定位**：`src/cli/prompt.rs:137-144`、`src/cli/prompt/exec.rs:219-225`
- **关键点**：执行 `lx-tui prompt move <PROMPT_ID> --folder <FOLDER_ID>` 时始终报错 `specify '--folder <ID>' or '--root'`，无法将提示词移入文件夹。
- **错误原因**：解析器把 `--folder <ID>` 存入 `PromptCommand.folder`，但 `PromptCommand::destination()` 内部写死读取 `self.parent`（该字段仅用于 `prompt folder move --parent <ID>`）。
- **修复方案**：修正 `PromptCommand::destination` 使其按调用场景分别读取 `self.folder`（用于 `prompt move`）与 `self.parent`（用于 `prompt folder move`），并补充双向单元测试。
- **状态**：已修复

### P1-4：`prompt page edit/rename/active/delete` 未校验当前工作区归属，可跨工作区篡改或删除分页
- **定位**：`src/cli/prompt/exec.rs:344-400`
- **关键点**：`prompt page edit`、`page rename`、`page active`、`page delete` 直接使用全局自增 `PAGE_ID` 操作数据库，未验证该分页所属的提示词是否位于当前工作区 `ctx.ws`；若在工作区 B 传入工作区 A 的唯一分页 ID 执行 `prompt page delete`，由于工作区 B 的提示词列表不含该分页，“至少保留一页”的保护也会被绕过，直接把工作区 A 的提示词删成 0 页。
- **错误原因**：底层 `Database` 的分页接口仅按 `id` 主键更新/删除，而 CLI 层缺少基于 `ctx.ws` 的归属校验。
- **修复方案**：在 `src/cli/prompt/exec.rs` 中新增工作区归属校验函数，所有 `prompt`、`folder`、`page`、`ref` 写操作及按 ID 查询操作均先确认目标归属于当前 `ctx.ws`（且 `page active <PROMPT_ID> <PAGE_ID>` 确认 `PAGE_ID` 属于 `PROMPT_ID`）。
- **状态**：已修复

### P1-5：非交互式环境下执行 `skill edit` / `command edit` 会从空 `stdin` 读取并静默清空正文
- **定位**：`src/cli/args.rs:110-145`、`src/cli/skill/exec.rs:313, 351`、`src/cli/command/exec.rs:248, 290`
- **关键点**：在脚本、CI 或后台子进程（`stdin` 重定向为 `/dev/null` 或空管道）中执行 `lx-tui skill edit <name> --description "new"` 或 `lx-tui command edit <name> --description "new"`（仅改元数据、未传 `--content`/`-f`）时，原有的正文会被空字符串覆盖；`command` 正文被清空后甚至无法再被解析加载。
- **错误原因**：`ContentInput::resolve_with(false, ...)` 只要检测到 `!stdin_is_terminal` 就无条件读取 `stdin` 并返回 `Ok(Some(""))`；调用方 `unwrap_or(old_body)` 拿到 `Some("")` 后覆盖了原有正文；此外 `command create/edit` 与 `skill create/edit` 缺少非空正文与单行元数据（禁止换行破坏 frontmatter）校验。
- **修复方案**：
  1. 当 `required == false` 且未显式指定 `--content` / `-f` 时，若自动从非 TTY `stdin` 读到的文本为空，则返回 `Ok(None)`，保留原有正文；
  2. 在 `skill create/edit` 与 `command create/edit` 中校验最终正文 `trim()` 后非空，且拒绝含 `\n`/`\r` 的 frontmatter 单行元数据字段（`description`、`title`、`argument-hint`、`when-to-use`）。
- **状态**：已修复

### P1-6：`mcp add-http` / `mcp add-stdio` 更新已有服务时会丢失未重传的认证、密钥与列表参数
- **定位**：`src/cli/mcp.rs:74, 339-341`、`src/cli/mcp/exec.rs:215-289`
- **关键点**：对已存在的 HTTP MCP 服务执行 `lx-tui mcp add-http <name> --url <new_url>` 时，未传的 `--auth` 会被重置为 `none`，原有的 `bearer_token`、`client_id`、`client_secret`、`http_headers`、`env_http_headers`、`scopes` 全部丢失；对已有 stdio 服务仅传 `--command` 更新时，原有的 `args`、`env`、`cwd` 也会被清空。
- **错误原因**：`AddHttpArgs.auth` 定义为非 `Option` 的 `McpHttpAuth`（默认 `None`），且 `save_server` 仅合并了顶层 `enabled`/超时/工具列表，未与同名同类型旧服务的 `McpTransport` 字段合并。
- **修复方案**：将 `AddHttpArgs.auth` 改为 `Option<McpHttpAuth>`；当目标作用域已存在同类型服务时，`build_stdio` / `build_http` 对未显式传入的可选参数（`auth`、token/secret、headers、scopes、args、env、cwd）自动保留现有配置，同时补充 `--env-http-header <HEADER=ENV_VAR>` 支持。
- **状态**：已修复

---

## 3. P2 级问题（中危）

### P2-1：`workspace` 历史与工作树边界缺陷（live worktree 删除、主仓库级联、会话图片清理、非法路径校验、`worktrees` 当前标记与 `head`）
- **定位**：`src/cli/workspace/exec.rs:237-475`
- **关键点**：
  1. `workspace delete` 仅在 `db.list_workspaces()` 中查找目标，拒绝删除 `workspace list` 中可见但尚未入库的 live linked worktree；
  2. 删除主仓库时若数据库仍留有同仓库 linked worktree 记录，`list_workspace_history_entries()` 会重新合成主仓库父项；
  3. `workspace delete` 删除工作区及级联会话后，未调用 `platform::remove_session_images` 清理磁盘上的会话图片目录；
  4. `workspace add` 未校验目标路径存在且为目录（甚至可把不存在路径或普通文件加入历史），且 JSON `"trusted"` 字段直接回显 `--trust` 入参而非数据库真实状态；`workspace rename` 对未登记路径会隐式插入新记录；`workspace reorder` 未覆盖未列出的工作区导致 `position` 重复；
  5. `workspace worktrees` 用 `current.starts_with(&entry.path)` 判断 `current`，导致在 linked worktree 中主仓库和子 worktree 同时为 `true`，且缺少设计规定的 `head` 字段。
- **错误原因**：CLI 层未完全对齐 TUI 的工作区历史树删除语义（`src/db/workspace.rs`）与 `git::checkout` 判定规则。
- **修复方案**：
  1. `workspace add` 要求目标路径存在且 `is_dir()`，JSON `"trusted"` 返回 `db.is_trusted` 真实结果；
  2. `workspace rename` 要求目标已存在于历史列表中；
  3. `workspace reorder` 将未列出的现有工作区按原相对顺序追加在末尾，重写全部连续 `position`；
  4. `workspace delete` 支持历史列表中的 live/historical worktree，删除主仓库时同步清理关联子项，并在删除成功后对所有被删会话调用 `platform::remove_session_images`；
  5. `workspace worktrees` 使用 `git::checkout(&list, &ctx.cwd)` 精确判断 `current`，并通过 `git rev-parse --short HEAD` 补充 `head` 字段。
- **状态**：已修复

### P2-2：会话解析与操作边界（`resolve_target_session` 200 条截断与空标题盲区、`list --limit`、`rename --last`、`rewind` 加锁顺序与 `restore-compact` 预检、`compact` 工具调用拦截、多级子会话删除）
- **定位**：`src/cli/context.rs:113-135`、`src/cli/session/mod.rs:272-281`、`src/cli/session/views.rs:40-46, 116-124`、`src/cli/session/ops.rs:98-125, 364-392`、`src/db/chat/mod.rs:730-742`
- **关键点**：
  1. `resolve_target_session` 仅拉取前 200 条会话（`list_chat_sessions` 内部 clamp 到 200），且回退 `chat_session_title` 会过滤空标题会话，导致空标题或超过 200 条后的会话无法按 ID/前缀解析；
  2. `session list --limit 0` 未报错，`--limit 500` 实际只返回 200 条；
  3. `session rename <SESSION_ID> --last` 会把最新会话标题改成 `<SESSION_ID>`；
  4. `session rewind` 在非 dry-run 时先读 `latest_user_turn` 后加锁；`--dry-run --mode restore-compact` 未按检查点模式预检；
  5. `session compact` 流式回调未拦截 `ChatDelta::ToolInput*`（TUI 端明确拒绝含工具调用的摘要响应）；
  6. `delete_chat_session` 仅删除直接子会话，多级子 Agent 的孙会话及其图片可能残留。
- **错误原因**：会话枚举未做分页循环，且 `rewind`/`compact`/`delete` 缺少与 TUI 一致的守卫条件。
- **修复方案**：
  1. 在 `context.rs` 中分页拉取工作区全部会话用于精确匹配与唯一前缀匹配；`session list` 拒绝 `--limit 0` 并支持分页拉取满足指定 `limit`；
  2. `session rename` 支持 `--title <TITLE>`，且在使用 `--last` 时若位置参数形如 `sess-...` 且未用 `--title` 则报错提示；
  3. `session rewind` 在执行前先获取会话锁，`--dry-run --mode restore-compact` 精确预检 `active_session_checkpoint`；
  4. `session compact` 检测到 `ChatDelta::ToolInputStarted | ToolInputDelta | ToolInputEnded` 立即中止报错；
  5. `delete_chat_session` 与 CLI `session delete` 递归收集全部后代子会话 ID 并清理图片。
- **状态**：已修复

### P2-3：`prompt create` 非原子初始化与 `todo` 状态流转守卫缺失
- **定位**：`src/cli/prompt/exec.rs:146-174`、`src/cli/todo.rs:262-265, 347-359, 404-435`
- **关键点**：
  1. `prompt create` 先用空切片创建默认页，再分两次调用更新页名与正文，中途失败会留下半成品提示词；
  2. `todo done` 对已完成的待办重复执行会覆盖原有 `completed_date`；`todo rollover --id` 允许将已完成的待办顺延到今天；`todo add --text "..."` 解析了 `--text` 但报错缺少 `<TEXT>`。
- **错误原因**：未直接利用 `db.create_prompt` 的初始分页参数，且 `todo` 状态变更缺少前置状态检查。
- **修复方案**：
  1. `prompt create` 直接将 `&[(page_name, content)]` 传入 `db.create_prompt` 单事务完成；
  2. `todo done` 对已处于 `Done` 状态的条目保留原 `completed_date`；`todo rollover --id` 拒绝状态为 `Done` 的条目；`todo add` 兼容位置文本与 `--text`。
- **状态**：已修复

### P2-4：配置与扩展领域（`mcp` / `command` / `skill` / `subagent` / `model` / `settings`）一致性与安全缺口
- **定位**：
  - `src/cli/mcp/exec.rs:56-199, 405-465`
  - `src/cli/command/exec.rs:142-177`
  - `src/cli/skill/exec.rs:347-388`
  - `src/cli/subagent/exec.rs:297, 376-388`
  - `src/cli/model/exec.rs:208-210, 386-390, 494-523`
  - `src/cli/settings.rs:308-322`
- **关键点**：
  1. `mcp`：未信任工作区下 `effective_servers_for` 仍加载项目级 `mcp.json`（若文件损坏会阻断 `mcp list`）；`mcp show/logout --scope project` 未校验工作区信任与 `project_root`；`mcp list --scope user` 因复用合并后视图导致被项目覆盖的用户级服务消失；`mcp show` 未掩码 `http_headers` 与 `env` 的值；`mcp login --json` 输出了两次 JSON 文档。
  2. `command`：`command list --kind` 先按名称合并再过滤 `kind`，导致同名异类型命令被覆盖漏列；`command list` 输出缺少设计要求的 `mdScope`/`title`/`blockType`。
  3. `skill`：`skill edit/delete` 使用 frontmatter 内的 `name` 而非实际目录名定位文件，当目录名与 frontmatter `name` 不一致时会新建或误删目录。
  4. `subagent`：在 linked worktree 下，`subagent role-set/role-rm --scope project` 写删 `ctx.workspace`，而读取优先发现 `ctx.cwd`（checkout 根），导致读写路径不一致。
  5. `model`：`provider-set` 与 `model-set` 未拒绝空 ID（如 `""` 或 `"prov/"`）；`model fetch` 缺少对非 OpenAI 兼容协议（`anthropic`/`google`）、空 API Key 与非法 Base URL 的前置校验。
  6. `settings open`：仅注入了用户级自定义命令，未在工作区受信任时注入项目级自定义命令快照。
- **错误原因**：CLI 各配置子模块在对接底层存储时，部分跳过了 Web Settings / TUI 事件层的前置校验与作用域规则。
- **修复方案**：逐项补齐上述 6 个模块的信任门控、原始作用域枚举、header/env 掩码、单次 JSON 输出、目录名原位更新、checkout 根对齐及项目级命令快照注入。
- **状态**：已修复

### P2-5：全局选项扫描器吞掉带值选项的参数值且未处理 `--` 分隔符
- **定位**：`src/cli/args.rs:169-211`、`src/cli/args.rs:60-108`
- **关键点**：`extract_global_opts` 在子命令解析前扫描整条参数链，未在 `--` 处停止，也未跳过带值选项（如 `--content`、`--description`、`--title` 等）的参数值，导致 `lx-tui prompt edit 1 --content --json` 中的 `--json` 被当成全局标志吞掉，或 `lx-tui todo add -- -C` 报错 `unexpected argument '--'`。
- **错误原因**：全局选项扫描与子命令扫描分成两阶段执行，第一阶段缺乏对 `--` 终止符和常见带值标志值槽的识别。
- **修复方案**：`extract_global_opts` 遇到 `--` 立即停止全局标志扫描并保留后续全部 token；遇到已知带值选项时原样保留该选项及其紧随的取值 token；`Scanner` 与位置参数解析支持 `--` 终止选项扫描。
- **状态**：已修复

---

## 4. P3 级问题（低危与规范对齐）

### P3-1：5 个生产代码文件超过 500 行软上限，且 `todo.rs` 未拆分子模块
- **定位**：`src/cli/model/exec.rs`（604 行）、`src/cli/prompt/exec.rs`（590 行）、`src/cli/todo.rs`（581 行）、`src/cli/mcp/exec.rs`（546 行）、`src/cli/workspace/exec.rs`（530 行）
- **关键点**：虽未触及 1000 行硬上限，但超出 `AGENTS.md` 与 `docs/standards/code-writing-standards.md`“拆分后子文件控制在 500 行左右”及 `task.md`“300～500 行”的要求。
- **错误原因**：领域执行逻辑集中在单个文件中未做二次拆分。
- **修复方案**：将 `src/cli/todo.rs` 拆分为 `src/cli/todo.rs` + `src/cli/todo/exec.rs`，并精简/拆分 `model/exec.rs`、`prompt/exec.rs`、`mcp/exec.rs`、`workspace/exec.rs`，使 `src/cli/` 下所有生产文件均控制在 500 行以内。
- **状态**：已修复

### P3-2：测试环境隔离遗漏 `~/.agents/skills` 且部分 `#[allow(...)]` 缺少原因注释
- **定位**：`src/config/skills.rs:85-91`、`src/cli/mod.rs:30`、`src/cli/mcp.rs:25`、`src/cli/subagent.rs:25`
- **关键点**：
  1. `user_agents_skills_root()` 直接读取 `std::env::var_os("HOME")`，未接入 `crate::platform::test_home_override()`，导致单元测试会扫描宿主机真实的 `~/.agents/skills`；
  2. 3 处 `#[allow(clippy::large_enum_variant)]` 缺少规范要求的原因说明注释。
- **错误原因**：测试覆盖 `set_test_env` 时遗漏了 `config/skills.rs` 中的 `HOME` 读取点。
- **修复方案**：在 `user_agents_skills_root()` 中优先读取 `crate::platform::test_home_override()`；为所有 `#[allow(clippy::large_enum_variant)]` 补齐注释；扩充单元测试覆盖所有 P0–P2 修复场景。
- **状态**：已修复

### P3-3：`design.md`、`task.md`、`verification.md` 与 `project-directory-structure.md` 文档漂移
- **定位**：`docs/feature/lx-full-cli/{design,task,verification}.md`、`docs/standards/project-directory-structure.md:177`
- **关键点**：
  1. `design.md` 与 `task.md` 的模块清单未反映 `*/exec.rs` 子模块分层、真实类型名（`WorkspaceContext`/`Output`/`FileGuard`）及 `platform::set_test_env` 测试隔离机制；
  2. `verification.md` 中部分字段名描述与实际 JSON 输出存在偏差；
  3. `docs/standards/project-directory-structure.md` 的 `cli` 依赖列表未包含 `app` 与 `layout`。
- **错误原因**：实现阶段拆分了 `*/exec.rs` 并适配了合并分支后，未反向同步设计文档与任务清单。
- **修复方案**：全面更新 `design.md`、`task.md`、`verification.md` 与 `docs/standards/project-directory-structure.md`，确保文档与代码完全一致。
- **状态**：已修复
