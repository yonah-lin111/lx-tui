# Agent 沙箱与权限模式执行任务

状态：A 批实现完成，等待用户交互验收；B/C 未开始。

- A 批（配置模型、macOS seatbelt、Linux bwrap 参数、进程接入、文件工具闸门、显式提权审批、on-failure 重试、`/permission`、DB v30、提示词、设置页/API）已落地并提交。
- 验证结果见同目录 `verification.md`。交互外观与真实 on-failure 链路仍需用户运行 `cargo run` 确认。

## 0. 开工门槛

- [ ] 用户审核并明确确认 `design.md` / `task.md`（含决策 #18 分 A/B/C 三批、#19 完整权限档案、#20 Starlark 规则）；审核要求改动时先更新文档再实施。
- [ ] 确认三批的独立验收：A 未通过不得进入 B；B 未通过不得进入 C；不得把三批混成一个无法逐段验证的变更。
- [ ] 重读当前 `AGENTS.md` 与 `docs/standards/` 四份规范，核对未提交改动；不得修改 Git 状态。
- [ ] 复核 `src/db/mod.rs::SCHEMA_VERSION`（当前 29）与其他分支迁移；冲突时顺延并保留旧库升级路径。
- [ ] 用 codegraph + codebase-memory 复核每个拟改关键函数的调用关系与覆盖；codex 仓库只读。
- [ ] 已完成的 macOS 预验证（本机 macOS 26.2 arm64）：`sandbox-exec` 可用；最小 profile 会打断 rustc/cargo；Codex 基础策略 + `(allow file-read*)` 后 `cargo`/`rustc`/`git`/`zsh -l` 正常；`-D` 参数可传带空格路径；`.git` 保护"先允许后拒绝"生效；拒绝网络时 DNS 失败；加固行 `(deny mach-lookup (xpc-service-name-prefix ""))` 与 `(deny system-fcntl (fcntl-command 80 110))` 不影响工具链。

## 1. 配置与策略模型

- [ ] 新建 `src/config/sandbox.rs`：`SandboxMode`（`read-only`/`workspace-write`/`danger-full-access`）、`EscalationPolicy`（`ask`/`never`）、`SandboxSettings`；serde 默认值与校验。
- [ ] `config/sandbox.json`：缺失→默认、损坏→报错但回落默认 + 告警、保存走 `0700/0600` + 临时文件 + 原子 rename。
- [ ] `writable_roots`：绝对路径、去重、最多 16 条、无控制字符；不存在允许保存但执行期跳过并警告。
- [ ] 新建 `src/chat/sandbox.rs`：`SandboxPlan`（mode、可写根、保护路径、network、platform 能力）、`SandboxOverride`、根解析（cwd + `$TMPDIR` + `/tmp` + 额外根；`.git`/`.agents`/`.aws` 保护）。
- [ ] 单测：默认值、未知键、非法值、往返、0600、根去重与 `.git` 文件形态。

## 2. macOS Seatbelt

- [ ] 复制 `assets/sandbox/seatbelt_base_policy.sbpl` 与 `seatbelt_network_policy.sbpl`（保留来源与许可证注释），`include_str!` 进二进制。
- [ ] `src/platform/sandbox/seatbelt.rs`：profile 拼接（基础策略 → `(allow file-read*)` → 写策略 → 保护 → 网络 → 加固行）；路径走 `-D` 参数；固定 `/usr/bin/sandbox-exec -p <profile> -- <shell> <args>`。
- [ ] 写策略用 `require-all` + `require-not (literal/subpath)` 排除保护路径；保护祖先追加 `deny file-write-unlink`；`deny system-fcntl` 与 `deny mach-lookup` 必须存在。
- [ ] `read-only` 不生成写规则；`danger-full-access` 不包装；`network_access=true` 追加网络策略。
- [ ] 失败：`sandbox-exec` 缺失/参数生成失败 → `sandbox_unavailable`（fail-closed）。
- [ ] 单测：profile 快照（四种组合）、带空格路径、控制字符拒绝；macOS 集成测试真实阻断写/网络（`#[cfg(target_os = "macos")]`）。

## 3. Linux Bubblewrap

- [ ] `src/platform/sandbox/bwrap.rs`：参数构造（`--ro-bind / /`、`--dev /dev`、`--unshare-user/pid/ipc`、`--unshare-net`、浅→深 `--bind`、保护路径 `--ro-bind`、`--proc /proc`、`--chdir`、`--cap-drop ALL`、`--new-session --die-with-parent`）。
- [ ] 能力探测：`bwrap` 缺失或无法建 user namespace → `Unavailable`，受限档 fail-closed。
- [ ] 单测：参数快照；云主机（8.138.213.110）安装 bwrap 后实测并写 `verification.md`。

## 4. 进程接入

- [ ] `ProcessManager::exec_observed` 增加 `&SandboxPlan` 参数；`spawn_pipe` 与 `spawn_pty` 统一改为调用 `platform::sandbox::wrap` 得到程序与 argv；`ToolContext`/执行器不直接拼沙箱命令。
- [ ] 用户 `!` 命令（`src/event/chat/user_shell.rs`）传 `SandboxPlan::none()`；MCP/终端不接入。
- [ ] `src/chat/agent/exec/mod.rs` 与 `src/event/chat/stream_start.rs` 在 turn 起点冻结 `SandboxPlan`（会话覆盖优先级：session → global）。
- [ ] 子 agent 创建时冻结父会话当前 plan 并写入子会话 `sandbox_mode`。
- [ ] 保留 process group、停止、超时、输出上限语义；macOS 退出码/信号透传（已实测）。
- [ ] 单测：包装命令、danger 直通、用户 shell 不包装、停止回归。

## 5. 文件工具闸门

- [ ] 新建 `src/chat/tools/sandbox_gate.rs`：`ensure_writable` / `ensure_writable_resolved`；`ToolContext` 增加 `SandboxPolicyView`。
- [ ] `write` / `edit` / `apply_patch` 所有磁盘变更前调用；多目标逐个校验，任一失败不产生部分写入。
- [ ] `read-only` 一律 `sandbox_denied`；`workspace-write` 限可写根并保护 `.git`/`.agents`/`.aws`；`danger-full-access` 放行。
- [ ] 判定逻辑路径 + 解析路径；不存在目标用最近存在祖先 canonicalize + 剩余组件；按组件边界匹配保护路径。
- [ ] 单测：越界绝对路径、`..`、符号链接、`.git` 内写入、工作区正常写、danger 全通。

## 6. 提权与审批

- [ ] `process_args.rs`：`ExecRequest` / `SandboxEscalation` / `AdditionalPermissions` 解析与严格校验（`justification`、`prefix_rule` 危险前缀与元字符、`additional_permissions` 仅 network + write）。
- [ ] `schema.rs`：`exec_command` 增加四个参数；`escalation=never` 时生成 `ToolDecl` 后裁剪；同步 transport/工具声明测试。
- [ ] `AgentHostRequest::ApproveSandbox` + `AgentHost::approve_sandbox` 默认方法；`AgentHostEvent` 新增变体。
- [ ] `agent_loop/mod.rs`：`parse_exec_request` → 规则命中/策略检查/审批 → `SandboxOverride` 传给执行器；拒绝文本固定英文；审批结果写入工具结果。
- [ ] 规则：`shlex` token 精确前缀；`ForSession` 落库 + 内存；`/permission` 切档清空；fork 不复制。
- [ ] 决策优先级严格按 design §18.4：规则 `Forbidden` → 规则 `Allow` → 会话精确规则 → 模型请求 → 规则 `Prompt` → 默认沙箱；先命中者生效。
- [ ] 无头 `lx exec`：escalation 一律 `sandbox_denied: escalation requires an interactive session`。
- [ ] 单测：参数校验、前缀匹配、危险前缀拒绝、Once/Session/Deny/Cancelled、规则命中后免审批。

## 7. 审批 UI

- [ ] `pane.lx.approval` FIFO 状态 + `src/event/chat/approval.rs` 请求处理；找不到 lx 窗格立即 Denied。
- [ ] `src/ui/main_content/lx/approval.rs`：输入框上方固定 strip（宽度与输入框一致、命令/理由截断、三动作、子 agent 来源标签）。
- [ ] 布局集成 `src/ui/main_content/lx.rs`（steer 之上）；`apply_lx_input` 优先级 `task > approval > ask`；`y/a/n`、`Esc=Deny`、等待期间 Enter 不提交、`Ctrl+C` 取消。
- [ ] 文案入 `ui::text`；失败/拒绝走现有工具结果与 toast，不新增 part 类型。
- [ ] 单测：队列、键位、Esc、与 ask/steer 共存、窄窗渲染不重叠。

## 8. `/permission`

- [ ] `SlashCommand::Permission`（第 13 项）、别名、描述；`commands_matching` 与面板测试同步。
- [ ] `open_permission_picker`：三项候选、当前档标记、session override/global default 文案。
- [ ] 选择落库 `agent_session.sandbox_mode`；未落库会话存 pending override 并在首条消息写入；`/new` 清空；fork 复制模式、不复制规则。
- [ ] toast：`Sandbox mode: <mode> (applies to the next turn)`，切档时清空规则并提示。
- [ ] 单测：候选、持久化、pending override、fork 语义、非法值。

## 9. 数据库

- [ ] 迁移 v30（以实际版本为准）：`agent_session.sandbox_mode TEXT`、`agent_session_sandbox_rule` 表与索引；只增表/列，旧库可升级。
- [ ] DB API：读取/写入会话模式、加载/插入/清空规则；fork 复制 `sandbox_mode`。
- [ ] 单测：旧库升级、NULL 默认、规则唯一约束、级联删除、fork。

## 10. 提示词与环境快照

- [ ] `EnvironmentSnapshot` 增加 `<sandbox mode platform network>` + `<writable_root>` + `<protected>`；8 条截断；整体 4096 字节预算。
- [ ] 删除 `shell commands are not sandboxed`；`## Sandbox` 段说明边界、提权参数、`escalation=never` 行为、用户 `!` 不受沙箱约束。
- [ ] `PROMPT_VERSION` → `lx-15`；跑现有提示预算/分段/transport 测试。

## 11. 设置页与 API

- [ ] `src/settings/service/sandbox.rs`：`GET/PUT /api/sandbox`、校验、原子保存、`SettingsEvent::SandboxChanged`。
- [ ] `handlers.rs` 路由与 `state_json` 能力信息（platform/process_sandbox/reason）。
- [ ] `web.html` Sandbox tab + 表单（模式、网络、escalation、额外可写根、能力提示）；`settings.js` 懒加载/脏检查/保存；`settings.css` 复用现有类。
- [ ] Windows 显示 `file tools only`；Linux 缺 bwrap 显示 `Unavailable`。
- [ ] 若新增 JS 文件：同步 `page::asset` 与 `tests/web_assets.rs::SCRIPTS`。
- [ ] 测试：API 校验/往返/权限、前端资源存在性与 `getElementById` 契约。

## 12. 验证与交付

- [ ] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响模块 `cargo test`。
- [ ] macOS 端到端：workspace-write 内写成功/外写失败、断网失败、read-only 禁写、danger 全通、PTY 同样受限、审批 Once/Session/Deny、Esc 停止无残留。
- [ ] A 批端到端：`touch ~/x` 被拒绝 → 弹出 `Sandbox blocked command` → `r` 重试成功；`n` 后模型看到原始 `Operation not permitted`；同命令二次调用在 `a` 后不再询问。
- [ ] B 批端到端：`additional_permissions.file_system={read:["..."], write:["..."], deny:[".../**"]}` 批准后，命令只能在授权范围内读写；glob deny 命中项不可读写；未批准的路径仍失败。
- [ ] C 批端到端：写 `rules/default.rules` 的 `prefix_rule(["echo"], decision="allow")` 后，`echo hi` 不套沙箱；`forbidden` 规则直接拒绝；非法规则文件只告警不阻塞启动。
- [ ] Linux 云主机实测 bwrap（安装允许时）；记录版本、内核、userns 状态与断言结果。
- [ ] 文档记录未验证项：Windows 进程沙箱（后续实现）、Linux 若无法实测。
- [ ] 更新 `verification.md`（新建）与 `review.md`（如项目惯例要求）。

## 13. 验收命令与判定

```bash
# 1) 受影响模块单测（按实施顺序逐个跑，避免全量拖时间）
cargo test config::sandbox
cargo test platform::sandbox
cargo test chat::sandbox
cargo test chat::sandbox::permissions
cargo test chat::exec_policy
cargo test chat::tools::process_args
cargo test chat::tools::sandbox_gate
cargo test chat::agent_loop
cargo test event::chat::approval
cargo test app::state::lx::slash
cargo test db::migration
cargo test settings

# 2) 静态检查
cargo fmt --check
cargo clippy --all-targets -- -D warnings

# 3) macOS 真实沙箱（可用临时目录脚本或 ignored 集成测试）
#    断言：workspace 内写成功；$HOME 写失败；curl 解析失败；read-only 连 workspace 内也失败；
#    danger-full-access 全部成功；pty=true 同样失败；Esc 后 ps 无残留进程。
```

判定标准：

- 所有新增单测通过，既有 `chat::process` / `event::chat::user_shell` / `settings` 回归通过。
- macOS 端到端七条断言全部符合 §14.2。
- 设置页表现：切换模式后 `/api/sandbox` 往返稳定；Windows/Linux 能力提示正确；无中文泄漏。
- 提示词：`cargo test chat::prompt` 通过，`PROMPT_VERSION` 已递增，环境快照不含 `not sandboxed` 旧文案。

## 14. 实施约束

- 除新增 `shlex = "1"`、`globset`（已在依赖树）、C 批的 `starlark = "0.14"` 外不引入新依赖；不引入 `tree-sitter`。
- OS 专属代码只放 `src/platform/sandbox/` 并按 `#[cfg(...)]` 门控；`chat/` 与 `app/` 不出现 `cfg(target_os)`。
- 生产代码禁止 `unwrap()` / `expect()`；错误用 `Result` + `?`；日志用 `tracing`。
- 用户可见文案一律英文，集中在 `src/ui/text.rs`；中文只用于注释与文档。
- 不修改 Git 状态；不创建无关目录/空抽象；单文件 ≤1000 行，超限按职责拆分。
- 审批是安全决策：默认拒绝、Esc=拒绝、拒绝路径不得执行任何命令。
- 任何"沙箱不可用"场景必须 fail-closed，禁止静默裸跑。

## 15. A 批：on-failure 重试与审批

- [ ] `src/chat/sandbox/denial.rs`：`is_likely_sandbox_denied(result, plan)`；关键词、退出码排除、Linux `128+SIGSYS`、状态过滤。
- [ ] `ProcessResult` 增加 `sandbox_retry: Option<SandboxRetry>`（`mode`、`attempt`、`approved_by`），`to_json` 与 `format_for_model` 同步；旧数据反序列化保持兼容。
- [ ] `agent_loop/mod.rs`：executor 返回后识别拒绝；`escalation=ask` 且未重试过 → `host.approve_retry`；批准后调用 `executor.retry_process`（复用原 `ExecArgs` + rtk 预处理命令）。
- [ ] 重试只发生一次；重试结果加 `sandbox denied the first attempt; retried with escalation.` 前缀；拒绝时返回第一次结果原文。
- [ ] 会话精确命令规则：`kind='exact'`、token 全等；命中后直接裸跑；`/permission` 切换清空。
- [ ] 审批种类 `Retry`：`r/a/n`，`Esc=n`；与显式 `Escalation` 共用 FIFO，但标题/选项不同。
- [ ] 无头 `lx exec`、Windows、`danger-full-access`、`escalation=never` 全部跳过该流程。
- [ ] 单测：误报排除、停止/超时、重试一次上限、规则命中、拒绝原文、子 agent 审批排队。

## 16. B 批：完整权限档案

- [ ] `src/chat/sandbox/permissions.rs`：`FileSystemAccessMode`/`FileSystemPath`/`FileSystemEntry`/`FileSystemPolicy`/`AdditionalPermissions`。
- [ ] 基准条目生成（§17.2）；`workspace-write` 的保护路径为 `Read` carveout，`additional_permissions` 可追加 `Read/Write/Deny`。
- [ ] 解析器：字面路径按组件边界前缀匹配；glob 用 `globset`（`literal_separator(true)`）；具体度 + `Deny > Write > Read`；单测覆盖边界与冲突。
- [ ] 工具 schema：`additional_permissions.file_system.{read,write,deny}` + `network.enabled`；每列表 ≤16 条、总长 ≤4096；拒绝相对路径、`~`、控制字符。
- [ ] 审批 UI 展示请求的路径与网络；批准后仅本次命令生效；`Always` 不用于 additional permissions（只允许 once）。
- [ ] Seatbelt：写根 carveout、`Deny` 字面/glob 的读与写拒绝、`globset` regex 转义、祖先 unlink 拒绝。
- [ ] Bwrap：`--ro-bind / /` + 写根 bind + `Read` carveout `--ro-bind` + `Deny` 字面/扫描后挂载；深度上限；跨符号链接拒绝。
- [ ] 文件工具闸门改用条目解析器；基准 `Read` 不阻止读，`Write` 才允许写。
- [ ] 单测：glob 语法、具体度、deny 优先、seatbelt/bwrap 投影快照、越界拒绝、批准后仅本次生效。

## 17. C 批：Starlark 规则

- [ ] `Cargo.toml` 增加 `shlex`、`globset`、`starlark`（`default-features=false`）；确认编译时间与二进制体积可接受。
- [ ] 移植 `src/chat/exec_policy/{decision,error,rule,parser,policy,mod}.rs`；去掉 CLI/amend/migration；单文件 ≤1000 行。
- [ ] 规则目录 `<user_data_root>/rules/*.rules`：加载排序、语法错误跳过并告警、空目录不报错。
- [ ] 决策：`Allow < Prompt < PromptUser < Forbidden` 取最严格；`match`/`not_match` 加载期自检；空 justification 拒绝。
- [ ] 保守分段 `split_shell_segments`：无元字符才允许 `allow` 绕过；含元字符时仅整串 `Prompt`/`Forbidden` 生效。
- [ ] 接入执行链：`Forbidden` → `sandbox_denied`；`Allow` → 裸跑；`Prompt` → 审批 strip（once/always/deny）；无匹配 → 沙箱 + A 批 on-failure。
- [ ] 决策优先级严格按 design §18.4；规则优先于模型请求；`escalation=never` 只降级审批、不关闭规则 `allow`。
- [ ] `Always` 写入 `rules/default.rules`（`prefix_rule(..., decision="allow")`），加锁、`0600`、reload；先过 banned prefix 与元字符校验。
- [ ] `escalation=never`：`Prompt`→拒绝；`Allow` 仍生效；文档与提示词说明。
- [ ] 热加载：turn 起点比较 mtime/文件集合；失败保留旧策略并告警。
- [ ] 设置页只读状态：规则文件数量、目录路径、`Open folder`；不提供网页编辑器。
- [ ] 单测：解析、匹配、最严格决策、banned prefix、分段保守性、落盘与 reload、非法文件跳过。

## 18. 批次验收线

- **A 批完成**：被沙箱拒绝的命令能弹出重试审批；批准后成功，拒绝后模型看到原始失败；不产生重复 tool call；既有停止/超时回归通过。
- **B 批完成**：`additional_permissions` 的 read/write/deny/glob 在 seatbelt 与 bwrap 上都按解析结果生效；文件工具闸门与命令沙箱结果一致；glob 扫描有深度上限。
- **C 批完成**：`rules/*.rules` 能被加载并影响决策；`allow` 可跳过沙箱（仅在无 shell 元字符时）；模型提议的规则经审批后落盘并立即生效；非法规则不阻塞启动。
