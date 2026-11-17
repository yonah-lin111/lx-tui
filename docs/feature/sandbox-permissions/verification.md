# 沙箱与权限模式验证报告

日期：2026-11-17
分支：`feature/sandbox-permissions`（worktree `.worktrees/sandbox-permissions`）
环境：macOS 26.2 arm64；`/usr/bin/sandbox-exec` 可用；默认 `workspace-write`、`network_access=false`、`escalation=ask`。

结论：A/B/C 及补充交互与多 agent 权限跟随改动已全部实现并通过自动化检查；已在当前 worktree 用 CLI 新建 session 完成 v32 库与真实沙箱验证。TUI 实际渲染/手感、Linux bwrap 实机与 Windows 仍待验收。

## 1. 自动化检查

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| AUTO-1 | 工作区全部 Rust 源码 | `cargo fmt --check` | 无格式差异 | 通过 |
| AUTO-2 | 全部 target | `cargo clippy --all-targets -- -D warnings` | 0 警告 | 通过 |
| AUTO-3 | 单元与集成测试 | `cargo test` | 0 失败 | 通过：5799 lib 测试 + 集成测试全绿 |
| AUTO-4 | `assets/web.html`、`assets/settings*.js` | `cargo test --test web_assets` | 脚本可解析，引用的 DOM id 存在 | 通过 |
| AUTO-5 | 模拟 dev v30 库（已含 `agent_compact_checkpoint.kind`） | `cargo test --lib db::sandbox::tests` | 依次升到 v31/v32；新增 `sandbox_mode`、规则表与 `agent_permission_switch` 表 | 通过 |

## 2. CLI 实测（当前 worktree）

### CLI-1 新建 session 并执行沙箱内命令
- 输入：`./target/debug/lx-tui exec --json -C . "Use the exec_command tool exactly once to run 'pwd && uname -s'. Then reply with the command output on a single line. Do not run any other command and do not modify any file."`
- 步骤：执行命令；随后 `session list --json`。
- 期望：exit 0；工具输出 worktree 路径和 `Darwin`；列表出现新 session。
- 结果：通过。session `sess-18e8bfa5323eb0c8-1`；输出 `/Users/yonah/projects/agent/lx-tui/.worktrees/sandbox-permissions` / `Darwin`。

### CLI-2 工作区外写入被 seatbelt 阻断
- 输入：`./target/debug/lx-tui exec --json -C . "Use the exec_command tool exactly once to run: printf probe > ~/lx-sandbox-cli-probe.txt. Then report whether it succeeded or the exact error text. Do not retry and do not modify any file."`
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

### CLI-5 session 持久化、v32 升级与隔离库
- 输入：`./target/debug/lx-tui exec --json -C . "Use the exec_command tool exactly once to run 'echo v32-ok'. Reply with the exact output on one line."` 及 `session list --json`。
- 步骤：以升级至 Schema v32 的二进制在 worktree 执行；核对输出与列表。
- 期望：exit 0，隔离库平滑升级到 v32，新建 session 可列出且工具输出 `v32-ok`。
- 结果：通过。session `sess-18e8c991a16d9180-0` 输出 `v32-ok`；数据库为 `~/.lx-t-dev/db/worktrees/sandbox-permissions-0d844551/lx.db`。

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
| A-9 | 审批命令面板：`↑/↓`、`Enter`、`Esc`、鼠标点击与 `y/r/a/n` | `cargo test --lib app::update::chat::tests::approval`；`slash_panels` | 方向键/回车/点击/快捷键均可裁决；普通字符不进草稿；压制 slash | 单测通过；视觉/手感待人工 |
| A-10 | 沙箱拒绝后 Once/ForSession/Denied/Cancelled | `cargo test --lib chat::agent_loop`；CLI-2 | `r` 重试；`a` 写入会话规则；`n`/Esc 保留原失败；无头不挂起 | 单测与 CLI-2 通过；TUI 手工待人工 |
| A-11 | `/permission` 三项模式（`Read-only` / `Workspace` / `Full`） | `app::update::chat::tests::permission`；`slash_panels` | 初始高亮当前项；`[Default]` 标记；输入可过滤；切档清空旧规则 | 单测通过；视觉待人工 |
| A-12 | dev v30/v31 库 | 见 AUTO-5 | 升至 v32；`sandbox_mode`、规则表、`agent_permission_switch` 齐全 | 通过 |
| A-13 | 系统提示与环境快照 | `cargo test --lib chat::prompt`、`chat::agent::tests`、`chat::tools::schema::tests` | `PROMPT_VERSION=lx-18`；自闭合标签无重复闭合；按会话模式输出 `<sandbox>`；主/子 agent 均明确子 agent 可发起提权审批 | 通过 |
| A-14 | `GET/PUT /api/sandbox`、Sandbox tab 资源 | `cargo test --lib settings::service::sandbox`；CLI-4；AUTO-4 | 往返、校验、能力与规则状态正确；`SandboxChanged` 热更新 `AppState` | API 通过；页面视觉待人工 |

## 4. B 批覆盖

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| B-1 | 17 条、>4096 字节、相对路径、`~`、`read/write` glob | `cargo test --lib chat::sandbox::permissions` | 超限与非法条目拒绝；`deny` 接受 glob；`read/write` 只接受字面绝对路径 | 通过 |
| B-2 | 基准条目 + read/write/deny 组合 | 同上 | `Deny` 绝对优先；字面路径按组件边界；glob 不跨 `/`；无匹配继承只读 | 通过 |
| B-3 | deny 字面、`**/.env`、read carveout | `cargo test --lib platform::sandbox`（真实 seatbelt） | deny 读写均拒绝；carveout 可读不可写；兄弟路径正常 | 通过 |
| B-4 | bwrap 的 bind/ro-bind/tmpfs/dev-null/符号链接 | Linux 实机 `cargo test` | 遮盖顺序正确；跨可写符号链接 fail-closed | 未执行（同 A-5） |
| B-5 | 工作区内 deny glob、read carveout | `cargo test --lib chat::tools::sandbox_gate` | 命中项写入拒绝，兄弟路径允许 | 通过 |
| B-6 | 带 `additional_permissions` 的审批命令面板 | `slash_panels::approval_command_panel_renders_context_options_and_hit_tests` | 上下文区展示 `Requested:` 摘要；`Approve for session` 仅可写前缀时出现 | 单测通过；视觉待人工 |
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

## 6. 补充需求覆盖（命令面板 / 左上角 / 切换感知与跟随）

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| D-1 | 审批弹窗 `CommandPanelView` 渲染与键鼠、授权唤醒链路 | `slash_panels::approval_command_panel_*`、`approval::arrows_navigate_and_enter_*`、`db::service::sandbox_tests`、`event::chat::tests::approval` | 贴输入框上方；子 agent 审批显示 `[Subagent]` 与高亮 `Subagent:` 行；`persistence.pump` 不吞 `AnswerApproval`、`exec_command` 授权后正常落终态 | 通过 |
| D-2 | 左上角 `lx-<Entry> [<Mode>]` 显示与三色配色 | `ui::tests::lx_pane_top_border_shows_short_permission_and_omits_ai_web` | 展示 `lx-Native [Workspace]`（黄）/`[Read-only]`（绿）/`[Full]`（红），方括号暗灰；`lx-Web` 省略；不撞右侧身份标签 | 通过 |
| D-3 | `/permission` 与 `/model` 面板对齐 | `permission::picker_opens_with_global_default_marked_and_highlighted`、`slash_panels` | 初始高亮当前生效项；`[Default]` 标签；输入过滤、回车清输入 | 通过 |
| D-4 | 切换权限后主 agent 感知与时间线卡片 | `context::tests::model_switch::permission_switches_interleave_*`、`permission_switch` 卡片单测 | 下一轮 `<environment_context>` 更新；按 `sequence` 注入 `<system-update>` 并渲染折叠卡片；支持 fork/rewind | 通过 |
| D-5 | subagent 跟随主会话权限、切档后替换与审批路由 | `event::chat::subagent::tests_sandbox` | spawn 继承当前模式；主会话切档后下一轮（resume/send_input）自动替换并注入 `<system-update>`；审批带来源进父窗格 | 通过 |
| D-6 | `/btw` 跟随主会话权限与切档替换 | `chat::btw::tests::btw_context_inherits_parent_sandbox_settings` | 保持只读工具面；每轮继承主会话最新 `SandboxSettings` 到 `<sandbox>` 与 `AgentTools::with_sandbox` | 通过 |

## 7. 平台与人工验收

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
|---|---|---|---|---|
| P-1 | TUI 审批命令面板与快捷键 | `cargo run` 后触发一次拒绝与一次提权 | 无边框蓝线面板；`↑/↓/Enter` 与 `y/r/a/n` 一致；不遮挡输入框 | 待人工 |
| P-2 | `/permission` 面板与左上角 `lx-Native [<Mode>]` | TUI 输入 `/permission` 切换三档并观察左上角及对话卡片 | 初始高亮当前项；左上角同步变更为带绿/黄/红三色的 `[Read-only]`/`[Workspace]`/`[Full]`；时间线出现 `Permission switched` 卡片 | 待人工 |
| P-3 | 设置页 Sandbox tab 与 `Open folder` | `cargo run` → 设置页修改默认模式 | 保存后 `lx` 左上角默认标签实时刷新；`Open folder` 弹出规则目录 | 待人工 |
| P-4 | Linux 云主机 `8.138.213.110` | 安装 bwrap 后跑 `platform::sandbox::bwrap` 与真实写入/断网 | 与 macOS 同等阻断；无 bwrap 时 fail-closed | 未执行 |
| P-5 | Windows | 文件工具写入工作区外 | 文件工具被闸门拒绝；进程沙箱不启用（v1 设计） | 未执行 |
| P-6 | 子 agent 提权端到端交互 | 主 agent 派生 `executor` 触发工作区外写命令 | 审批面板显示子 agent 来源；批准后子 agent 继续完成 | 单测通过；端到端待人工 |

## 8. 合并注意

- 分支基于 `dev@41d5977b`；`dev` 之后已合入 stop/report 分支（含独立 v30 迁移）。
- 沙箱迁移现为 **v31**（`sandbox_mode` + 规则表）与 **v32**（`agent_permission_switch` 表），提示词版本为 **`lx-18`**；合并时保留 dev 的 v30 `agent_compact_checkpoint.kind`，再依次接 v31 / v32。
- `git merge-tree --write-tree dev HEAD` 需重点核对：`src/db/migration.rs`（v30/v31/v32 顺序）、`src/chat/prompt.rs`、`src/app/state.rs`。

## 9. 回滚

所有改动在独立 worktree 与分支；未修改主 checkout 的 Git 状态。数据库迁移 v31/v32 只增列/表，回退二进制后旧库仍可使用（新列/表保留但不被旧版本读取）。规则文件是用户数据，不在二进制回滚范围内。
