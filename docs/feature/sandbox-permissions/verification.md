# 沙箱与权限模式验证报告

日期：2026-11-17
分支：`feature/sandbox-permissions`（worktree `.worktrees/sandbox-permissions`）
环境：macOS 26.2 arm64；`/usr/bin/sandbox-exec` 可用；默认 `workspace-write`、`network_access=false`、`escalation=ask`。

结论：A/B/C 实现与自动化检查通过；已在当前 worktree 用 CLI 新建 session，实测沙箱内命令、工作区外写入阻断、断网、`allow` 规则放行和设置 API。TUI 外观/键位、Linux bwrap 实机、Windows 进程沙箱、子 agent 审批转发仍待验收。

## 1. 自动化检查

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| AUTO-1 | 工作区全部 Rust 源码 | `cargo fmt --check` | 无格式差异 | 通过 |
| AUTO-2 | 全部 target | `cargo clippy --lib --all-targets -- -D warnings` | 0 警告 | 通过 |
| AUTO-3 | 单元与集成测试 | `cargo test` | 0 失败 | 通过：5787 lib 测试 + 集成测试全绿 |
| AUTO-4 | `assets/web.html`、`assets/settings*.js` | `cargo test --test web_assets` | 脚本可解析，引用的 DOM id 存在 | 通过 |
| AUTO-5 | 模拟 dev v30 库（已含 `agent_compact_checkpoint.kind`） | `cargo test --lib db::sandbox::tests::v30_database_upgrades_to_v31_with_sandbox_schema` | 升到 v31；新增 `sandbox_mode` 与规则表；v30 的 `kind` 保留 | 通过 |

## 2. CLI 实测（当前 worktree）

### CLI-1 新建 session 并执行沙箱内命令
- 输入：`./target/debug/lx-tui exec --json -C . "Use the exec_command tool exactly once to run 'pwd && uname -s'. Then reply with the command output on a single line. Do not run any other command and do not modify any file."`
- 步骤：执行命令；随后 `session list --json`。
- 期望：exit 0；工具输出 worktree 路径和 `Darwin`；列表出现新 session。
- 结果：通过。session `sess-18e8bfa5323eb0c8-1`；输出 `/Users/yonah/projects/agent/lx-tui/.worktrees/sandbox-permissions` / `Darwin`。

### CLI-2 工作区外写入被 seatbelt 阻断
- 输入：`./target/debug/lx-tui exec --json -C . "Use the exec_command tool exactly once to run: printf probe > ~/lx-sandbox-cli-probe.txt. Then report whether it succeeded or the exact error text. Do not retry and do not modify any other file."`
- 步骤：删除探针文件后执行；检查 `~/lx-sandbox-cli-probe.txt`。
- 期望：工具退出码 1，输出 `operation not permitted`；探针不存在；无头模式不挂起等待审批。
- 结果：通过。session `sess-18e8bfa7e6daa8a0-0`；输出 `zsh:1: operation not permitted: /Users/yonah/lx-sandbox-cli-probe.txt`；探针不存在。

### CLI-3 `allow` 规则跳过沙箱
- 输入：`~/.lx-t-dev/rules/default.rules` 写入 `prefix_rule(pattern = ["touch"], decision = "allow", justification = "cli verification")`；提示词要求执行 `touch ~/lx-rule-allow-probe.txt`。
- 步骤：规则文件设为 0600；执行 `exec`；检查探针；删除规则与探针。
- 期望：exit 0，探针文件存在（否则默认沙箱会拒绝 `~` 写入）。
- 结果：通过。探针存在；`default.rules` 与探针已清理。

### CLI-4 设置页 `/api/sandbox`
- 输入：`./target/debug/lx-tui settings open --no-open --json`，再请求 `GET /api/sandbox?token=...`。
- 步骤：启动无头设置服务，解析 URL；请求 API；关闭服务。
- 期望：返回 `mode=workspace-write`、`network_access=false`、`escalation=ask`、平台能力 `seatbelt`、规则目录与加载计数。
- 结果：通过。返回 `platform=macos`、`process_sandbox=seatbelt`、规则目录 `~/.lx-t-dev/rules`、`total_files=0`。

### CLI-5 session 持久化与隔离库
- 输入：`session list --json`、`session show --last --json`。
- 步骤：在 worktree 执行；核对列表、详情和数据库路径。
- 期望：新 session 可列出/查看；开发库落在该 worktree 的隔离目录。
- 结果：通过。列表含本次会话；数据库为 `~/.lx-t-dev/db/worktrees/sandbox-permissions-0d844551/lx.db`。列表中的 workspace 仍为仓库根 `lx-tui`（既有语义），隔离由 worktree 数据库完成。

## 3. A 批覆盖

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| A-1 | 缺失/非法/合法的 `config/sandbox.json` | `cargo test --lib config::sandbox`；CLI-4 | 默认 `workspace-write`；非法值报错回落；保存 0700/0600 原子写 | 通过（单测 + CLI-4） |
| A-2 | cwd、额外写根、`.git` 目录/文件 | `cargo test --lib chat::sandbox` | 写根含 cwd/TMPDIR/`/tmp`/额外根；`.git` 只读；read-only 无写根；danger 不包装 | 通过 |
| A-3 | workspace-write / read-only / `.git` / PTY | `cargo test --lib platform::sandbox`（真实 `sandbox-exec`） | 工作区内可写；区外、read-only、`.git`、PTY 均被拒绝 | 通过；CLI-2 另证区外写入 |
| A-4 | `network_access=false`，`curl https://example.com` | CLI 无头会话执行一次 curl | DNS/连接失败，不返回 HTTP 状态 | 通过：`curl: (6) Could not resolve host` |
| A-5 | Linux bwrap：写根、carveout、deny、glob | `cargo test --lib platform::sandbox::bwrap`（Linux） | 参数顺序与遮盖规则正确 | 未执行：macOS 不编译该模块，需 Linux 实机 |
| A-6 | `exec_command`、用户 `!`、MCP、终端 | `cargo test --lib chat::process`；代码核对 | agent 命令被包装；`!`/MCP/终端不套沙箱 | 通过（单测 + 代码核对） |
| A-7 | `write`/`edit`/`apply_patch` 的越界、`..`、符号链接、`.git` | `cargo test --lib chat::tools::sandbox_gate` | 越界/保护路径拒绝；工作区内允许；任一失败不产生部分写入 | 通过 |
| A-8 | `require_escalated`、`with_additional_permissions` 等 JSON | `cargo test --lib chat::tools::process_args` | 合法请求接受；空理由、危险前缀、相对路径、未知字段拒绝 | 通过 |
| A-9 | 审批 `y/a/n`、`r/a/n`、`Esc`、窄窗格 | `cargo test --lib app::update::chat::tests::approval`；TUI 手工 | 按键映射正确；Esc 拒绝；队列不重叠；附加权限多一行摘要 | 单测通过；视觉/手感待人工 |
| A-10 | 沙箱拒绝后 Once/ForSession/Denied/Cancelled | `cargo test --lib chat::agent_loop`；CLI-2 | `r` 重试；`a` 写入会话规则；`n`/Esc 保留原失败；无头不挂起 | 单测与 CLI-2 通过；TUI `r/n` 待人工 |
| A-11 | `/permission` 三项模式 | slash/面板单测；TUI 手工 | 第 13 个命令；切档写会话、下一 turn 生效；切档清空会话规则 | 单测通过；视觉待人工 |
| A-12 | dev v30 库 | 见 AUTO-5 | v31；`sandbox_mode`、规则表、索引齐全 | 通过 |
| A-13 | 系统提示与环境快照 | `cargo test --lib chat::prompt` | `PROMPT_VERSION=lx-17`；声明沙箱边界、提权参数；无“未沙箱”旧文案 | 通过 |
| A-14 | `GET/PUT /api/sandbox`、Sandbox tab 资源 | `cargo test --lib settings::service::sandbox`；CLI-4；AUTO-4 | 往返、校验、能力与规则状态正确；页面可加载 | API 通过；页面视觉待人工 |

## 4. B 批覆盖

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| B-1 | 17 条、>4096 字节、相对路径、`~`、`read/write` glob | `cargo test --lib chat::sandbox::permissions` | 超限与非法条目拒绝；`deny` 接受 glob；`read/write` 只接受字面绝对路径 | 通过 |
| B-2 | 基准条目 + read/write/deny 组合 | 同上 | `Deny` 绝对优先；字面路径按组件边界；glob 不跨 `/`；无匹配继承只读 | 通过 |
| B-3 | deny 字面、`**/.env`、read carveout | `cargo test --lib platform::sandbox`（真实 seatbelt） | deny 读写均拒绝；carveout 可读不可写；兄弟路径正常 | 通过 |
| B-4 | bwrap 的 bind/ro-bind/tmpfs/dev-null/符号链接 | Linux 实机 `cargo test` | 遮盖顺序正确；跨可写符号链接 fail-closed | 未执行（同 A-5） |
| B-5 | 工作区内 deny glob、read carveout | `cargo test --lib chat::tools::sandbox_gate` | 命中项写入拒绝，兄弟路径允许 | 通过 |
| B-6 | 带 `additional_permissions` 的审批请求 | 单测 + TUI 手工 | strip 展示 `Requested: ...`，高度增加一行；`a` 仅在可写前缀时可用 | 单测通过；视觉待人工 |
| B-7 | 仅含 `deny` 的附加权限 | `cargo test --lib chat::agent_loop::tests::sandbox_rules::deny_only_additional_permissions_skip_approval` | 不弹审批，直接应用收缩权限 | 通过 |

## 5. C 批覆盖

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| C-1 | `prefix_rule` 的 allow/prompt/prompt-user/forbidden、备选 token | `cargo test --lib chat::exec_policy` | 解析正确；同前缀取最严格裁决 | 通过 |
| C-2 | 非法 `match`/`not_match`、坏 Starlark 文件 | 同上；规则目录加载 | 违反示例拒绝该文件；坏文件跳过并告警，不阻塞启动 | 通过 |
| C-3 | Forbidden/Allow/会话精确/模型提权/规则 Prompt | `cargo test --lib chat::agent_loop::tests::sandbox_rules` | 严格按 `Forbidden → Allow → 会话精确 → 模型请求 → 规则 Prompt → 默认沙箱` | 通过 |
| C-4 | `&&`、`|`、重定向、`$`、反引号 | `cargo test --lib chat::exec_policy` | `allow` 不生效；`forbidden`/`prompt` 仍生效 | 通过 |
| C-5 | 规则文件新增/修改；`default.rules` 追加 | 同上 | 按文件集合/mtime 热加载；追加文件 0600、立即生效 | 通过 |
| C-6 | 用户选择 `Always`；模型提议 `git`/`sh` 前缀 | 同上 | 用户规则可写 `git` 等前缀；模型提议过 banned 校验 | 通过 |
| C-7 | `/api/sandbox` 规则状态、`Open folder` | `settings::service::sandbox` 测试；CLI-4 | 返回目录、总数、加载数、错误；打开目录路由存在 | API 通过；实际弹出 Finder 待人工 |
| C-8 | `default.rules` 的 `touch` allow 规则 | CLI-3 | 同前缀命令不套沙箱，能写 `$HOME` | 通过 |

## 6. 平台与人工验收

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| P-1 | TUI 审批 strip：`y/a/n`、`r/a/n`、Esc、窄窗格 | `cargo run` 后触发一次拒绝与一次提权 | 文案为英文；按键与提示一致；不遮挡输入框 | 待人工 |
| P-2 | `/permission` 面板 | TUI 输入 `/permission` 并切换三档 | 当前档标记正确；下一 turn 生效；toast 落在 lx 窗格 | 待人工 |
| P-3 | 设置页 Sandbox tab 与 `Open folder` | `cargo run` → 设置页 | 表单可保存；规则状态刷新；弹出规则目录 | 待人工 |
| P-4 | Linux 云主机 `8.138.213.110` | 安装 bwrap 后跑 `platform::sandbox::bwrap` 与真实写入/断网 | 与 macOS 同等阻断；无 bwrap 时 fail-closed | 未执行 |
| P-5 | Windows | 文件工具写入工作区外 | 文件工具被闸门拒绝；进程沙箱不启用（v1 设计） | 未执行 |
| P-6 | 子 agent 的提权请求 | 主 agent 派生 subagent 后触发审批 | 请求转发到父窗格并可审批 | 未实现（当前返回 `sandbox_escalation_unavailable`） |

## 7. 合并注意

- 分支基于 `dev@41d5977b`；`dev` 之后已合入 stop/report 分支（含独立 v30 迁移）。
- 沙箱迁移为 **v31**，提示词版本为 **`lx-17`**；合并时保留 dev 的 v30 `agent_compact_checkpoint.kind`，再接 v31。
- `git merge-tree --write-tree dev HEAD` 预演冲突：`src/db/migration.rs`（v30/v31 顺序）、`src/chat/prompt.rs`（版本与新增段）、`src/app/state.rs`（导出列表）；三处均为可并集解决。

## 8. 回滚

所有改动在独立 worktree 与分支；未修改主 checkout 的 Git 状态。数据库迁移 v31 只增列/表，回退二进制后旧库仍可使用（新列/表保留但不被旧版本读取）。规则文件是用户数据，不在二进制回滚范围内。
