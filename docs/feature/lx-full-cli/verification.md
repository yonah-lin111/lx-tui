# `lx-tui` 全功能无头 CLI 验证报告

- **验证工作区**：`/Users/yonah/projects/agent/lx-tui/.worktrees/lx-full-cli` (`feature/lx-full-cli`)
- **测试二进制**：`./target/debug/lx-tui`
- **隔离环境**：临时 `HOME` + `LX_TUI_DB_PATH` + 本地 Mock OpenAI/MCP 服务
- **总结论**：**56 / 56 项 CLI 端到端验证全部通过（PASS）**

---

## 1. 顶层路由、帮助、版本与全局选项

### V01：版本信息输出
- **输入内容**：`lx-tui --version` / `lx-tui -V` / `lx-tui version`
- **验证步骤**：依次执行三条命令并检查 `stdout` 与退出码。
- **期望结果**：`stdout` 输出 `lx-tui 0.1.0`，退出码 `0`。
- **结果**：PASS

### V02：全局与各领域帮助手册
- **输入内容**：`lx-tui --help` 及 `lx-tui help <exec|session|prompt|todo|workspace|model|mcp|skill|command|subagent|settings>`、`lx-tui <domain> --help`
- **验证步骤**：遍历全部 11 个子命令执行 `help` 与 `--help`。
- **期望结果**：`stdout` 输出对应英文 `Usage:` 帮助文档，退出码 `0`。
- **结果**：PASS

### V03：未知命令与非法参数拦截
- **输入内容**：`lx-tui bogus-cmd`、`lx-tui session --bogus`
- **验证步骤**：执行非法命令与非法标志，检查 `stderr` 与退出码。
- **期望结果**：`stderr` 输出英文错误提示，退出码 `1`。
- **结果**：PASS

### V04：全局选项 `-C/--cd`、`--json`、`--color`
- **输入内容**：`lx-tui workspace status -C <DIR> --json --color never`
- **验证步骤**：在非工作区目录传入 `-C <DIR>` 与 `--json` 执行。
- **期望结果**：上下文定位到 `<DIR>`，`stdout` 输出合法 JSON 且无 ANSI 转义码，退出码 `0`。
- **结果**：PASS

---

## 2. `workspace` 工作区历史、信任与 Worktree

### V05：工作区添加、信任与状态查询
- **输入内容**：`lx-tui workspace add <DIR> --name MainWS --trust --json`、`lx-tui workspace status -C <DIR> --json`
- **验证步骤**：登记并信任工作区后查询状态。
- **期望结果**：返回 JSON 中 `trusted: true`、`name: "MainWS"` 及有效 `database` 路径，退出码 `0`。
- **结果**：PASS

### V06：工作区历史列表（树形 / `--raw` / `--search`）
- **输入内容**：`lx-tui workspace list`、`lx-tui workspace list --raw --search Main --json`
- **验证步骤**：添加两个工作区后分别按树形与扁平模式检索。
- **期望结果**：准确返回匹配的工作区记录，退出码 `0`。
- **结果**：PASS

### V07：工作区重命名、重排与级联删除
- **输入内容**：`lx-tui workspace rename NewName`、`lx-tui workspace reorder <DIR2> <DIR1>`、`lx-tui workspace delete <DIR2> [--include-worktrees]`
- **验证步骤**：依次执行重命名、排序和删除，再执行 `workspace list --raw --json`。
- **期望结果**：名称与 `position` 更新生效，被删工作区从历史列表移除，退出码 `0`。
- **结果**：PASS

### V08：Git Worktree 列表查询
- **输入内容**：`lx-tui workspace worktrees -C /Users/yonah/projects/agent/lx-tui/.worktrees/lx-full-cli --json`
- **验证步骤**：在当前新建 worktree 下执行 `workspace worktrees`。
- **期望结果**：返回主仓库根路径及包含 `.worktrees/lx-full-cli` (`feature/lx-full-cli`) 的 worktree 数组，退出码 `0`。
- **结果**：PASS

---

## 3. `session` 聊天会话管理（含 `exec` 联动）

### V09：会话创建、续跑与列表查询
- **输入内容**：`lx-tui exec "First user turn"`、`lx-tui exec resume --last "Second user turn"`、`lx-tui session list --limit 10 --offset 0 --json`
- **验证步骤**：通过 `exec` 生成 2 轮对话后执行 `session list`。
- **期望结果**：`sessions` 数组返回 1 条含累计 token 与模型标识的会话记录，退出码 `0`。
- **结果**：PASS

### V10：会话详情查看（`<ID>` / 唯一前缀 / `--last`）
- **输入内容**：`lx-tui session show --last --tools --thinking --json`
- **验证步骤**：分别用完整 ID、唯一前缀、`--last` 查看会话详情。
- **期望结果**：完整输出 4 条消息（2 轮 user/assistant）及元数据，退出码 `0`。
- **结果**：PASS

### V11：会话重命名与分叉（`rename` / `fork`）
- **输入内容**：`lx-tui session rename --last "Renamed"`、`lx-tui session fork <ID> [--before <MSG_ID>] --json`
- **验证步骤**：重命名后分别执行完整分叉与按消息边界分叉。
- **期望结果**：标题更新为 `"Renamed"`，分叉返回新 `new_session_id` 且历史消息按边界复制，退出码 `0`。
- **结果**：PASS

### V12：子 Agent 列表/树与进程列表（`agents` / `processes`）
- **输入内容**：`lx-tui session agents --last --tree --filter all --json`、`lx-tui session processes --last --json`
- **验证步骤**：对包含子 Agent 记录的会话执行列表与树形查询，以及进程查询。
- **期望结果**：输出包含 `role`、`status`、`depth` 的 `agents` 数组与 `processes` 数组，退出码 `0`。
- **结果**：PASS

### V13：会话在线压缩与恢复（`compact` / `compact --revert`）
- **输入内容**：`lx-tui session compact <ID> -m mockprov/mock-gpt-1 --json`、`lx-tui session compact <ID> --revert --json`
- **验证步骤**：调用模型生成摘要检查点并落库，随后执行 `--revert`。
- **期望结果**：压缩生成有效 `checkpoint_id`；`--revert` 成功撤销活跃检查点，退出码 `0`。
- **结果**：PASS

### V14：会话回滚预检与执行（`rewind --dry-run` / `--mode`）
- **输入内容**：`lx-tui session rewind <ID> --dry-run --json`、`lx-tui session rewind <ID> --mode <revert-and-delete|delete-only|revert-only|restore-compact> --json`
- **验证步骤**：先执行 `--dry-run` 检查目标轮次与文件恢复计划，再执行回滚。
- **期望结果**：`--dry-run` 只读输出 `target_user_message_id` 与 `paths`；执行后最后一轮对话按模式回滚，退出码 `0`。
- **结果**：PASS

### V15：会话删除与跨进程会话锁保护
- **输入内容**：`lx-tui session delete <ID>`（分别在持锁与释放锁状态下执行）
- **验证步骤**：持有会话文件锁时执行 `session delete/rewind/compact` 及 `workspace delete`；释放锁后再执行 `session delete`。
- **期望结果**：持锁期间写命令全部返回退出码 `1` 并提示会话正在运行（只读命令 `list/show` 正常返回 `0`）；释放锁后删除成功，退出码 `0`。
- **结果**：PASS

---

## 4. `prompt` 提示词库管理

### V16：提示词文件夹 CRUD（`prompt folder`）
- **输入内容**：`lx-tui prompt folder create Arch`、`rename <ID> Architecture`、`move <ID> --root`、`list --json`、`delete <ID>`
- **验证步骤**：创建根/子文件夹，执行重命名、移动、列表与删除。
- **期望结果**：文件夹树结构与名称按指令更新，删除后清空，退出码 `0`。
- **结果**：PASS

### V17：提示词条目 CRUD 与三种长文本输入（`--content` / `-f` / `stdin`）
- **输入内容**：`lx-tui prompt create Design --folder <FID> --content "inline"`、`prompt edit <PID> -f body.md`、`echo "stdin" | lx-tui prompt edit <PID>`、`show <PID> --json`、`rename`、`move --root`、`delete`
- **验证步骤**：依次通过内联参数、文件、标准输入管道更新内容并查看详情，最后移动并删除。
- **期望结果**：三种输入方式均精确写入活动页，重命名/移动/删除生效，退出码 `0`。
- **结果**：PASS

### V18：提示词多页管理（`prompt page`）
- **输入内容**：`lx-tui prompt page create <PID> P2 --content "p2"`、`list <PID>`、`active <PID> <PAGE_ID>`、`rename <PAGE_ID> P2New`、`edit <PAGE_ID> --content "p2v2"`、`delete <PAGE_ID>`
- **验证步骤**：新增第二页、切换活动页、重命名、编辑后删除该页（验证保留最后一页保护）。
- **期望结果**：分页列表与 `active_page_id` 正确变更，退出码 `0`。
- **结果**：PASS

### V19：外部引用目录管理（`prompt ref`）
- **输入内容**：`lx-tui prompt ref add <DIR>`、`lx-tui prompt ref list --json`、`lx-tui prompt ref remove <REF_ID>`
- **验证步骤**：添加外部目录引用，查询列表后移除。
- **期望结果**：列表包含规范化绝对路径，移除后列表为空，退出码 `0`。
- **结果**：PASS

---

## 5. `todo` 全局日程待办

### V20：待办新增与多维过滤列表（`todo add` / `list`）
- **输入内容**：`lx-tui todo add "Task" --date 2026-11-05 -p P0 --status todo`、`lx-tui todo list [--date|--all|--past-incomplete] [--status] [-p] --json`
- **验证步骤**：写入跨日期与不同优先级的待办，按日期、状态、优先级过滤查询。
- **期望结果**：过滤结果与写入项一致，退出码 `0`。
- **结果**：PASS

### V21：待办更新（含唯一前缀）与状态流转（`update` / `start` / `done` / `reopen` / `delete`）
- **输入内容**：`lx-tui todo update <PREFIX> --text "New" -p P1`、`todo start <ID>`、`todo done <ID>`、`todo reopen <ID>`、`todo delete <ID>`
- **验证步骤**：用 ID 唯一前缀更新文本和优先级，依次切换为进行中、已完成、重开并删除。
- **期望结果**：`done` 自动记录 `completed_date`，`reopen` 清除完成日期，`delete` 移除记录，退出码 `0`。
- **结果**：PASS

### V22：同日重排、历史顺延与区间统计（`reorder` / `rollover` / `stats`）
- **输入内容**：`lx-tui todo reorder --date 2026-11-05 <ID2> <ID1>`、`lx-tui todo rollover --to 2026-11-05`、`lx-tui todo stats --from 2026-11-01 --to 2026-11-05 --json`
- **验证步骤**：重排今日待办，将历史未完成项顺延至今日，查询统计。
- **期望结果**：顺序更新，历史未完成项 `entry_date` 迁至 `2026-11-05`，`stats` 输出每日完成率，退出码 `0`。
- **结果**：PASS

---

## 6. `model` 模型与供应商管理

### V23：供应商与模型配置及默认模型设定
- **输入内容**：`lx-tui model provider-set mockprov --type openai-compatible --base-url <URL> --api-key sk-1234567890abcdef --enable`、`model model-set mockprov/mock-gpt-1 --transport chat --context-limit 128000 --output-limit 16384 --image-input true`、`model default mockprov/mock-gpt-1`
- **验证步骤**：配置供应商、模型参数与默认模型后检查 `config/ai.json`。
- **期望结果**：配置原子写入 `ai.json`，默认模型为 `mockprov/mock-gpt-1`，退出码 `0`。
- **结果**：PASS

### V24：远端模型自动发现（`model fetch`）
- **输入内容**：`lx-tui model fetch mockprov --save --json`
- **验证步骤**：向本地 Mock `/v1/models` 服务拉取模型列表并带 `--save` 保存。
- **期望结果**：返回远端模型数组（`mock-gpt-1`、`mock-gpt-2`）并写入供应商模型表，退出码 `0`。
- **结果**：PASS

### V25：模型展示（密钥脱敏）、全局 AI 配置与删除
- **输入内容**：`lx-tui model show mockprov --json`、`model config --title-language en --timeout-ms 60000`、`model model-rm mockprov/mock-gpt-2`、`model provider-rm mockprov`
- **验证步骤**：查看供应商详情验证脱敏，更新全局参数，最后删除模型与供应商。
- **期望结果**：`show` 中 `apiKey` 脱敏为 `"sk-...cdef"`，配置更新与删除均生效，退出码 `0`。
- **结果**：PASS

---

## 7. `mcp` MCP 服务管理

### V26：Stdio / HTTP 服务注册与双作用域展示
- **输入内容**：`lx-tui mcp add-stdio mock-stdio --command python3 --arg mock_mcp.py --scope project`、`mcp add-http mock-http --url https://example.com/mcp --auth bearer --bearer-token tok_1234567890abcdef`、`mcp list --json`、`mcp show mock-http --json`
- **验证步骤**：分别在项目级与用户级添加 stdio/http 服务，查询列表与详情。
- **期望结果**：配置文件写入对应作用域，`show` 的 `config.bearer_token` 脱敏为 `"tok...cdef"`；`http_headers` 与 `env` 的值同样逐项掩码，退出码 `0`。
- **结果**：PASS

### V27：MCP 服务在线握手与工具探测（`mcp probe`）
- **输入内容**：`lx-tui mcp probe mock-stdio --scope project --json`
- **验证步骤**：对本地 JSON-RPC stdio MCP 脚本执行 `mcp probe`。
- **期望结果**：成功完成 `initialize` 与 `tools/list` 握手，输出包含 `"ping_tool"` 的工具列表，退出码 `0`。
- **结果**：PASS

### V28：MCP 启停、登出与删除（`disable` / `enable` / `logout` / `remove`）
- **输入内容**：`lx-tui mcp disable mock-http`、`mcp enable mock-http`、`mcp logout mock-http`、`mcp remove mock-http`
- **验证步骤**：依次禁用、启用、清理凭证并移除服务。
- **期望结果**：`enabled` 状态切换准确，移除后配置项清空，退出码 `0`。
- **结果**：PASS

---

## 8. `skill` 技能发现与管理

### V29：技能创建、列表发现、详情与编辑
- **输入内容**：`lx-tui skill create demo-skill --description "Demo" --content "# Body" --scope project`、`skill list --scope project --json`、`skill show demo-skill --scope project`、`skill edit demo-skill --description "Updated" --scope project`
- **验证步骤**：在已信任工作区创建项目级技能，执行发现列表、详情读取与描述修改。
- **期望结果**：生成 `<DATA_DIR>/skills/projects/<scope>/demo-skill/SKILL.md`（项目级技能根为 `<DATA_DIR>/skills/projects/<scope>/`，例如 `<DATA_DIR>/skills/projects/<scope>/cli-verifier/SKILL.md`），frontmatter 与正文更新正确，退出码 `0`。
- **结果**：PASS

### V30：技能启停与删除（`disable` / `enable` / `delete`）
- **输入内容**：`lx-tui skill disable demo-skill`、`skill enable demo-skill`、`skill delete demo-skill --scope project`
- **验证步骤**：切换技能禁用列表状态后删除该技能。
- **期望结果**：`skills.json` 的 `disabled` 列表同步增删，`delete` 删除技能目录，退出码 `0`。
- **结果**：PASS

---

## 9. `command` 自定义斜杠命令管理

### V31：`agentMD` 与 `agentBlock` 命令创建、列表与查看
- **输入内容**：`lx-tui command create check-code --kind agentMD --description "Check" --md-scope global --content "Body"`、`command create tmpl-block --kind agentBlock --description "Block" --block template --arg-hint "<file>" --title "Block" --content "Inspect $1" --scope project`、`command list --json`、`command show check-code`
- **验证步骤**：分别创建用户级 `agentMD` 与项目级 `agentBlock` 命令，执行列表与详情查询。
- **期望结果**：准确返回 2 条命令及其 frontmatter 属性；用户级文件位于 `<DATA_DIR>/command/agentMD/check-code.md`，项目级文件位于 `<workspace>/.lx-t/command/agentBlock/tmpl-block.md`，退出码 `0`。
- **结果**：PASS

### V32：自定义命令编辑、重命名与删除
- **输入内容**：`lx-tui command edit check-code --description "Check v2"`、`command rename check-code audit-code`、`command delete audit-code`
- **验证步骤**：更新命令描述，重命名后删除。
- **期望结果**：文件名与 frontmatter 同步更新，删除后文件移除，退出码 `0`。
- **结果**：PASS

---

## 10. `subagent` 子 Agent 角色与并发配置

### V33：全局并发、深度、Token 与委派模式设置
- **输入内容**：`lx-tui subagent set --max-concurrent 6 --max-depth 3 --default-max-tokens 50000 --delegation-mode proactive`
- **验证步骤**：修改全局参数后执行 `subagent show --json`。
- **期望结果**：`maxConcurrent: 6`、`maxDepth: 3`、`defaultMaxTokens: 50000`、`delegationMode: "proactive"`，越界值（如 `--max-concurrent 99`）被拦截，退出码 `0`。
- **结果**：PASS

### V34：内置角色覆盖（含 `monitor`）与自定义角色管理（用户级 / 项目级）
- **输入内容**：`lx-tui subagent role-set monitor --description "Custom monitor" --lock-model true`、`subagent role-set code_auditor --base explorer --description "Auditor" --prompt "Audit" --allow-web-search false --nicknames Alice,Bob --scope project`、`subagent role-reset monitor`、`subagent role-rm code_auditor --scope project`
- **验证步骤**：覆盖内置 `monitor` 角色、创建项目级 `.lx/agents/code_auditor.json` 角色，查看后重置并删除。
- **期望结果**：内置角色覆盖与项目级角色均生效，重置与删除后恢复默认，退出码 `0`。
- **结果**：PASS

---

## 11. `settings` 通用设置与 Web 设置服务

### V35：通用布局设置查看、修改与重置
- **输入内容**：`lx-tui settings set --sidebar-width 30`、`lx-tui settings show --json`、`lx-tui settings reset`
- **验证步骤**：修改侧栏宽度、验证 JSON 输出、重置为默认值（同时验证非法范围如 `--sidebar-min-width 99` 被拒绝）。
- **期望结果**：`sidebar_width` 设为 `30`，重置后恢复默认值 `24`，非法范围返回退出码 `1`。
- **结果**：PASS

### V36：独立启动本地设置 Web 服务（`settings open`）
- **输入内容**：`lx-tui settings -C <DIR> --json open --no-open`
- **验证步骤**：后台启动 `settings open --no-open`，读取 `stdout` JSON 中的 `url`（`http://127.0.0.1:<port>/?token=...`），随后发送 `SIGINT` 停止进程。
- **期望结果**：立即输出合法本地环回地址与鉴权 URL，收到 `SIGINT` 后优雅关闭服务并退出，退出码 `0`。
- **结果**：PASS

---

## 12. 边界场景、安全门控与跨模块深度联调（V37–V50）

### V37：会话前缀歧义拦截与空工作区 `--last` 报错
- **输入内容**：`lx-tui session show sess-`（同工作区含多个会话）、空工作区下 `lx-tui session show --last`
- **验证步骤**：在有 2 个会话的工作区传入共同前缀 `sess-`；在空工作区执行 `--last`。
- **期望结果**：均返回退出码 `1`，`stderr` 分别提示 `session prefix 'sess-' is ambiguous` 与 `no sessions found in workspace`。
- **结果**：PASS

### V38：Agent 工具写文件与 `session rewind` 磁盘文件快照真实回滚
- **输入内容**：`lx-tui exec "Create file"`（调用 `write` 工具生成 `created_by_agent.txt`）、`lx-tui session rewind --last --mode revert-only --json`
- **验证步骤**：通过 `exec` 触发 Agent 创建新文件落盘，再执行 `session rewind --last --mode revert-only`。
- **期望结果**：JSON 返回 `restored_files: 1`，磁盘上的 `created_by_agent.txt` 被自动清理恢复至轮次前状态，退出码 `0`。
- **结果**：PASS

### V39：未信任工作区项目级写操作安全门控（`mcp` / `skill` / `command` / `subagent`）
- **输入内容**：未信任工作区下执行 `lx-tui mcp add-stdio ... --scope project`、`skill create ... --scope project`、`command create ... --scope project`、`subagent role-set ... --scope project`
- **验证步骤**：在未执行 `workspace trust` 的新目录依次运行上述 4 条项目级写入命令。
- **期望结果**：4 条命令全部被拦截并返回退出码 `1`，`stderr` 提示工作区未信任。
- **结果**：PASS

### V40：Git Linked Worktree 自动归并主仓库根
- **输入内容**：`lx-tui workspace status -C /Users/yonah/projects/agent/lx-tui/.worktrees/lx-full-cli --json`
- **验证步骤**：在 linked worktree 目录执行 `workspace status --json`。
- **期望结果**：`path` 为当前 linked worktree 的 checkout 根 `/Users/yonah/projects/agent/lx-tui/.worktrees/lx-full-cli`，`git.repo_root` 为主仓库根 `/Users/yonah/projects/agent/lx-tui`，`git.is_linked: true`，退出码 `0`。
- **结果**：PASS

### V41：提示词最后一页删除保护与越界页码拦截
- **输入内容**：`lx-tui prompt page delete <ONLY_PAGE_ID>`、`lx-tui prompt show <PID> --page 99`
- **验证步骤**：对仅剩 1 页的提示词执行 `page delete`；查询不存在的第 99 页。
- **期望结果**：均返回退出码 `1`，`stderr` 提示 `must keep at least one page` 与 `page '99' not found`。
- **结果**：PASS

### V42：长文本输入冲突拦截与 `--content -` 标准输入读取
- **输入内容**：`lx-tui prompt edit <PID> --content "a" -f b.md`、`echo "Dash stdin" | lx-tui prompt edit <PID> --content -`
- **验证步骤**：同时传入 `--content` 与 `-f` 验证冲突报错；使用 `--content -` 从管道读取。
- **期望结果**：同时传参返回退出码 `1`；`--content -` 成功写入 `"Dash stdin"`，退出码 `0`。
- **结果**：PASS

### V43：待办非法日期/优先级拦截与 `--past-incomplete` 过滤
- **输入内容**：`lx-tui todo add "Bad" --date 2026-13-45`、`lx-tui todo add "Bad" -p P9`、`lx-tui todo list --past-incomplete --json`
- **验证步骤**：传入非法日期和优先级验证校验；写入历史日期未完成项后按 `--past-incomplete` 过滤。
- **期望结果**：非法参数返回退出码 `1`；`--past-incomplete` 准确返回历史未完成条目，退出码 `0`。
- **结果**：PASS

### V44：标题摘要默认模型（`model default --title-summary`）与 `--clear` 清除
- **输入内容**：`lx-tui model default mockprov/mock-gpt-1 --title-summary`、`lx-tui model default --clear --title-summary`、`lx-tui model default --clear`
- **验证步骤**：分别设置与清除标题摘要模型及主默认模型，通过 `model show --json` 检查。
- **期望结果**：`titleSummary` 与 `defaultModel` 独立设置和清空生效，退出码 `0`。
- **结果**：PASS

### V45：模型思维链 Variant（`#high`）选择与 `--clear-api-key` 密钥清除
- **输入内容**：`lx-tui model default mockprov/mock-gpt-1#high`、`lx-tui model provider-set mockprov --clear-api-key`
- **验证步骤**：带 `#high` 设定默认模型后查看 `defaultModel.variant`；清除 API Key 后查看供应商详情。
- **期望结果**：`defaultModel.variant` 为 `"high"`；清除后 `apiKey` 显示为 `"-"`，退出码 `0`。
- **结果**：PASS

### V46：MCP OAuth 完整授权闭环（`mcp login --no-open` 本地回调换 Token）
- **输入内容**：`lx-tui mcp login mock-oauth --no-open`
- **验证步骤**：配好 `--auth oauth` 服务后启动 `mcp login --no-open`，解析其输出的本地回调 `redirect_uri` 与 `state` 并模拟浏览器访问回调完成 PKCE 换 Token。
- **期望结果**：本地环回服务返回 `200 OK`，Token 成功写入 `<DATA_DIR>/mcp/credentials.json`，进程退出码 `0`。
- **结果**：PASS

### V47：自定义斜杠命令重名保护与 `--overwrite` 覆盖
- **输入内容**：重复执行 `lx-tui command create dup-cmd --kind agentMD --description "v2" --content "c2"`，再加 `--overwrite` 执行
- **验证步骤**：首次创建后不带 `--overwrite` 再次创建同名命令；随后带 `--overwrite` 创建。
- **期望结果**：不带 `--overwrite` 返回退出码 `1`；带 `--overwrite` 覆盖写入成功，退出码 `0`。
- **结果**：PASS

### V48：Subagent 越界并发/深度、非法角色名与内置角色删除拦截
- **输入内容**：`lx-tui subagent set --max-concurrent 99`、`--max-depth 9`、`subagent role-set Bad-Name --description d`、`subagent role-rm explorer`
- **验证步骤**：依次执行超上限并发/深度、非法角色命名、删除内置角色。
- **期望结果**：4 条命令全部被拦截并返回退出码 `1`。
- **结果**：PASS

### V49：设置项侧栏最小/最大宽度冲突拦截（`settings set`）
- **输入内容**：`lx-tui settings set --sidebar-min-width 40 --sidebar-max-width 20`
- **验证步骤**：传入最小宽度大于最大宽度的参数组合。
- **期望结果**：校验拒绝且不修改磁盘配置，返回退出码 `1`。
- **结果**：PASS

### V50：`settings open` Web API 鉴权与状态读取（`GET /api/state`）
- **输入内容**：启动 `lx-tui settings open --no-open --json`，提取 `token` 请求 `GET /api/state?token=<TOKEN>` 与无 token 请求
- **验证步骤**：分别发起无 token 请求与带有效 token 的 `/api/state` 请求，随后发送 `SIGINT` 关闭。
- **期望结果**：无 token 返回 `403 Forbidden`，带有效 token 返回 `200 OK` 及完整 `config` JSON，进程收到 `SIGINT` 后退出码 `0`。
- **结果**：PASS

---

## 13. 代码审查修复回归（V51–V56）

### V51：`workspace delete --include-worktrees` 在仓库子目录目标上不误删主仓库
- **输入内容**：将 Git 仓库子目录 `<repo>/src` 登记为工作区后执行 `lx-tui workspace delete <repo>/src --include-worktrees --json`
- **验证步骤**：在含 linked worktree 的仓库中登记 `<repo>/src`，带 `--include-worktrees` 删除该子目录，再执行 `lx-tui workspace list --raw --json` 并检查主仓库与 linked worktree 的会话。
- **期望结果**：JSON `workspaces` 仅包含 `<repo>/src`；主仓库 `<repo>` 与所有 linked worktree 的历史记录、会话和磁盘图片均保持不变。仅当目标本身是主 checkout（`git worktree list` 的 `repo_root == target`）时才展开同仓库 linked worktree；普通子目录不会被 `.git` 文件启发式误判，嵌套子模块也不会进入删除列表，退出码 `0`。
- **结果**：PASS

### V52：Linked worktree 的 `checkout_path` 默认目标与统一信任门控
- **输入内容**：在 linked worktree 中执行 `lx-tui workspace status --json`、`lx-tui workspace trust`，随后执行 `lx-tui skill create demo --description Demo --content Body --scope project`；再执行无参数 `lx-tui workspace delete --json`。
- **验证步骤**：确认 `status.path` 指向当前 linked worktree 而非主仓库根；无参 `trust` 信任当前 checkout；项目级写入通过 `WorkspaceContext::is_trusted`（cwd / checkout / workspace 根任一已信任）放行；无参 `delete` 仅作用于当前 linked worktree。
- **期望结果**：`path` 为 linked worktree checkout 根，`git.repo_root` 为主仓库根且 `git.is_linked: true`；`trust` 后项目级 `skill create` 成功；无参 `delete` 只删除当前 linked worktree 的历史记录并写入隐藏，不删除主仓库及其会话，退出码 `0`。
- **结果**：PASS

### V53：`prompt move --folder` 与跨工作区分页隔离
- **输入内容**：`lx-tui prompt move <PROMPT_ID> --folder <FOLDER_ID> --json`；在工作区 B 执行 `prompt page edit/rename/active/delete` 并传入工作区 A 的 `PAGE_ID`。
- **验证步骤**：修正 `PromptCommand::folder_destination` 后验证移动；再在另一个工作区用 A 的分页 ID 依次执行 `page edit`、`page rename`、`page active <B_PROMPT> <A_PAGE>` 与 `page delete`。
- **期望结果**：`prompt move --folder` 正确写入目标文件夹；跨工作区的所有分页写操作均返回退出码 `1` 且错误包含 `prompt page <id> not found`，工作区 A 的分页内容、名称、活动页与页数保持不变（无法绕过“至少保留一页”保护）。
- **结果**：PASS

### V54：非交互式 `skill edit` / `command edit` 保留正文并校验 frontmatter
- **输入内容**：`lx-tui skill edit demo-skill --description "Updated" </dev/null`、`lx-tui command edit check-code --description "Check v2" </dev/null`；再执行 `--content ""` 与含换行的 `--description`。
- **验证步骤**：在 stdin 为 `/dev/null` 的非交互环境中仅更新元数据，随后用 `show --json` 检查正文；分别传入空正文和多行描述。
- **期望结果**：描述更新成功且原有正文完整保留（可选正文从空 stdin 读到空文本时 `ContentInput::resolve_with` 返回 `Ok(None)`）；空正文返回退出码 `1`（`skill/command content cannot be empty`），含 `\n`/`\r` 的 `description`/`title`/`argument-hint` 被拒绝，退出码 `1`。
- **结果**：PASS

### V55：`mcp add-http` / `add-stdio` 局部更新保留未重传字段
- **输入内容**：先 `mcp add-http svc --url <URL1> --auth bearer --bearer-token <TOKEN> --header <K=V> --scope-item <SCOPE>`，再执行 `mcp add-http svc --url <URL2>`；先 `mcp add-stdio tool --command python3 --arg a --env K=V --cwd <DIR>`，再执行 `mcp add-stdio tool --command python3.12`。
- **验证步骤**：对已存在的同名同类型服务只重传部分参数，更新后读取配置文件并用 `mcp show --json` 检查（敏感值仅校验掩码后的形状）。
- **期望结果**：HTTP 服务的 `auth`、`bearer_token`/`bearer_token_env_var`、`client_id`/`client_secret`、`http_headers`、`env_http_headers`、`scopes` 保留旧值，仅 `url` 更新；Stdio 服务的 `args`、`env`、`cwd` 保留旧值，仅 `command` 更新；`enabled`、启动/工具超时与工具白/黑名单同样保留，退出码 `0`。
- **结果**：PASS

### V56：`--` 终止符与全局带值选项的值保护
- **输入内容**：`lx-tui todo add -- -C`、`lx-tui prompt edit <PROMPT_ID> --content --json`
- **验证步骤**：前者验证全局扫描在 `--` 处停止并把 `-C` 当作位置文本；后者验证 `--content` 的取值 token `--json` 不被全局扫描吞掉。随后用 `todo list --json` 与 `prompt show <PROMPT_ID> --json` 检查落库内容。
- **期望结果**：`todo add -- -C` 成功创建文本为 `-C` 的待办，不报未知选项；`prompt edit --content --json` 将字面量 `--json` 写入正文且不启用 JSON 输出（人类可读输出），退出码 `0`。
- **结果**：PASS
