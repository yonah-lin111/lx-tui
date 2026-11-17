# Agent 沙箱与权限模式执行任务

状态：A/B/C 三批、D 批补充功能与 `review.md` P0–P3 审查修复已实现，并已合入 `dev`（合并提交 `9e9d0ae6`）；远程 Linux/Windows 实测与人工 TUI 验收已收口至 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`。

- A 批：配置模型、macOS seatbelt、Linux bwrap 参数、进程接入、文件工具闸门、显式提权审批、on-failure 重试、`/permission`、DB v31、提示词、设置页/API。
- B 批：完整 `additional_permissions` 档案（read/write/deny/glob）、seatbelt/bwrap 投影、文件工具条目解析器。
- C 批：Starlark `rules/*.rules`、决策优先级、保守分段、热加载、`Always` 落盘、设置页只读状态。
- D 批：审批与 `/permission` 统一迁移至无边框 `CommandPanelView`、左上角 `lx-<Entry> [<Mode>]` 配色显示、DB v32 `agent_permission_switch` 与时间线 `Permission switched` 卡片、subagent/`/btw` 权限动态继承、`lx-18` 子 agent 提权引导及 `lx::agent::approval` 审计日志。
- 验证结果见同目录 `verification.md`，审查发现见 `review.md`。

## 0. 开工门槛

- [x] 用户审核并明确确认 `design.md` / `task.md`（含决策 #18 分 A/B/C 三批、#19 完整权限档案、#20 Starlark 规则）；审核要求改动时先更新文档再实施。
- [x] 确认三批的独立验收：A → B → C 逐批验证推进。
- [x] 重读当前 `AGENTS.md` 与 `docs/standards/` 四份规范，核对改动；未授权不擅改 Git 状态。
- [x] 复核 `src/db/mod.rs::SCHEMA_VERSION` 与 `dev` 分支迁移（`dev` 已占 v30，本分支顺延为 v31/v32；合入时已新增 v33 幂等兜底并推进至 v33）。
- [x] 用 codegraph + codebase-memory 复核关键函数调用关系与覆盖；codex 仓库只读。
- [x] 已完成的 macOS 预验证（macOS arm64）：`sandbox-exec` 可用；Codex 基础策略 + `(allow file-read*)` 后 `cargo`/`rustc`/`git`/`zsh -l` 正常；`-D` 参数可传带空格路径；`.git` 保护生效；拒绝网络时 DNS 失败。

## 1. 配置与策略模型

- [x] 新建 `src/config/sandbox.rs`：`SandboxMode`、`EscalationPolicy`、`SandboxSettings`；serde 默认值与校验。
- [x] `config/sandbox.json`：缺失→默认、损坏→报错回落默认 + 告警、保存走 `0700/0600` + 临时文件 + 原子 rename。
- [x] `writable_roots`：绝对路径、去重、最多 16 条、无控制字符；不存在允许保存但执行期跳过并警告。
- [x] 新建 `src/chat/sandbox.rs`：`SandboxPlan`、`SandboxOverride`、根解析（cwd + `$TMPDIR` + `/tmp` + 额外根；`.git`/`.agents`/`.aws` 保护）。
- [x] 单测：默认值、未知键、非法值、往返、0600、根去重与 `.git` 文件形态。

## 2. macOS Seatbelt

- [x] 复制 `assets/sandbox/seatbelt_base_policy.sbpl` 与 `seatbelt_network_policy.sbpl`（保留来源注释），`include_str!` 进二进制。
- [x] `src/platform/sandbox/seatbelt.rs`：profile 拼接、`-D` 参数传递与 `/usr/bin/sandbox-exec` 包装。
- [x] 写策略用 `require-all` + `require-not (literal/subpath)` 排除保护路径；保护祖先追加 `deny file-write-unlink`；加固行存在。
- [x] `read-only` 不生成写规则；`danger-full-access` 不包装；`network_access=true` 追加网络策略。
- [x] 失败：`sandbox-exec` 缺失/参数生成失败 → `sandbox_unavailable`（fail-closed）。
- [x] 单测与 macOS 集成测试：`#[cfg(target_os = "macos")]` 真实阻断写/网络。

## 3. Linux Bubblewrap

- [x] `src/platform/sandbox/bwrap.rs`：参数构造（`--ro-bind / /`、`--dev /dev`、`--unshare-user/pid/ipc`、`--unshare-net`、浅→深 `--bind`、保护路径 `--ro-bind`、`--proc /proc`、`--chdir`、`--cap-drop ALL`、`--new-session --die-with-parent`）。
- [x] 能力探测：`bwrap` 缺失、不支持 `--perms` 的目录 `deny`、userns 不可用（含 CentOS 7 setuid 回退）探测逻辑与单测已补齐（`review.md` P0-1）。
- [ ] 远程 Linux 主机实机构建并跑通 `bwrap` 与沙箱单测（已按决策延后，记录于 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`）。

## 4. 进程接入

- [x] `ProcessManager::exec_observed` 增加 `&SandboxPlan` 参数；`spawn_pipe` 与 `spawn_pty` 统一调用 `platform::sandbox::wrap`。
- [x] 用户 `!` 命令（`src/event/chat/user_shell.rs`）传 `SandboxPlan::none()`；MCP/终端不接入。
- [x] `src/chat/agent/exec/mod.rs` 与 `src/event/chat/stream_start.rs` 在 turn 起点冻结 `SandboxPlan`。
- [x] 子 agent 启动与每轮续跑动态继承主会话最新 `SandboxMode` 与规则。
- [x] 保留 process group、停止、超时、输出上限与 macOS 退出码/信号透传语义。
- [x] 单测：包装命令、danger 直通、用户 shell 不包装、停止回归。

## 5. 文件工具闸门

- [x] 新建 `src/chat/tools/sandbox_gate.rs`：`ensure_writable` / `ensure_writable_resolved`；`ToolContext` 增加 `SandboxPolicyView`。
- [x] `write` / `edit` / `apply_patch` 磁盘变更前统一校验，任一失败不产生部分写入。
- [x] `read-only` 一律 `sandbox_denied`；`workspace-write` 限可写根并保护 `.git`/`.agents`/`.aws`；`danger-full-access` 放行。
- [x] 判定路径边界、不存在祖先 canonicalize 与保护路径组件边界。
- [x] 单测：越界绝对路径、`..`、符号链接、`.git` 内写入、工作区正常写、danger 全通。

## 6. 提权与审批

- [x] `process_args.rs`：`ExecRequest` / `SandboxEscalation` / `AdditionalPermissions` 解析与校验。
- [x] `schema.rs`：`exec_command` 增加四个参数；`escalation=never` 时裁剪；同步工具声明测试。
- [x] `AgentHostRequest::ApproveSandbox` + `AgentHost::approve_sandbox`；`AgentHostEvent` 新增变体。
- [x] `agent_loop/sandbox_decision.rs`：规则命中/策略检查/审批 → `SandboxOverride` 传给执行器；拒绝文本英文。
- [x] 规则：`shlex` token 前缀/精确规则、`ForSession` 落库 + 内存、`/permission` 切档清空、fork 不复制。
- [x] 决策优先级：`Forbidden` → `Allow` → 会话规则 → 模型请求 → `Prompt` → 默认沙箱。
- [x] 单测：参数校验、前缀匹配、Once/Session/Deny/Cancelled、规则命中后免审批。

## 7. 审批 UI

- [x] `pane.lx.approvals` FIFO 队列（`ApprovalState`）+ `src/event/chat/approval.rs`；找不到窗格立即 `Denied`。
- [x] `src/ui/main_content/lx/approval.rs`：复用无边框 `CommandPanelView`（顶栏蓝线、子 agent `[Subagent]` 与高亮 `Subagent:` 行、选项高亮）。
- [x] `apply_lx_input` 优先级 `task > approval > ask`、`↑`/`↓`/`Enter`/`Tab`/`Esc`/`y/r/a/n`。
- [x] 文案迁入 `src/ui/text/sandbox.rs`；`Effect::AnswerApproval` 经 `is_network_effect` 放行至事件循环唤醒。
- [x] 单测：队列、键位、Esc、CommandPanelView 布局与授权唤醒端到端回归。

## 8. `/permission`

- [x] `SlashCommand::Permission`、别名 `["permissions", "sandbox"]`、`commands_matching` 与面板测试同步。
- [x] `open_permission_picker`：复用 `CommandPanelView`，`Read-only` / `Workspace` / `Full`、`[Default]` 标记与初始高亮当前项。
- [x] 落库 `agent_session.sandbox_mode` + `pending_sandbox_mode`；`/new` 清空；fork 复制模式、不复制规则。
- [x] 窗格内 toast：`Permission: <short_label> (applies to the next turn)`，切档清空会话规则。
- [x] 单测：候选、持久化、pending override、fork 语义、非法值。

## 9. 数据库

- [x] 迁移 v31（`agent_session.sandbox_mode` + `agent_session_sandbox_rule`）、v32（`agent_permission_switch`）与 v33（合并兜底：幂等补全 `agent_compact_checkpoint.kind` 与沙箱表/列）。
- [x] DB API：读取/写入会话模式、加载/插入/清空规则、读写权限切换记录；fork 复制 `sandbox_mode` 与分叉边界前切换记录。
- [x] 单测：旧库升级、NULL 默认、规则唯一约束、级联删除、fork。

## 10. 提示词与环境快照

- [x] `EnvironmentSnapshot` 增加 `<sandbox mode platform network>` + `<writable_root>` + `<protected>`；8 条截断；整体 4096 字节预算。
- [x] 删除 `shell commands are not sandboxed`；`# Sandbox` 段说明边界、提权参数、`escalation=never`、子 agent 提权与用户 `!` 不受约束。
- [x] `PROMPT_VERSION` 递增至 `lx-18`（含 `EXECUTOR_PROMPT`/`MONITOR_PROMPT`/`spawn_agent` 子 agent 提权说明）。

## 11. 设置页与 API

- [x] `src/settings/service/sandbox.rs`：`GET/PUT /api/sandbox`、`POST /api/sandbox/open-rules`、`SettingsEvent::SandboxChanged`。
- [x] `handlers.rs` 路由与能力信息（`platform`/`process_sandbox`/`reason`）。
- [x] `web.html` Sandbox tab + `assets/settings_sandbox.js` 独立脚本（`page::asset` 与 `tests/web_assets.rs::SCRIPTS` 同步）。
- [x] 测试：API 校验/往返、前端资源与 `getElementById` 契约。

## 12. 静态检查与本地验证

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响模块 `cargo test` 全部通过（提交 `08a8bc6c` 前）。
- [x] macOS 真实沙箱与 CLI `exec` 实测（`verification.md` A/B/C/D 及 Review 修复项）。
- [x] 更新 `review.md`、`verification.md`、`docs/standards/html-assets.md` 与 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`；登记延后项与合入迁移注意事项。

## 13. 验收命令与判定

```bash
cargo test config::sandbox
cargo test platform::sandbox
cargo test chat::sandbox
cargo test chat::exec_policy
cargo test chat::tools::process_args
cargo test chat::tools::sandbox_gate
cargo test chat::agent_loop
cargo test event::chat::approval
cargo test app::state::lx::slash
cargo test db::migration
cargo test settings
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## 14. 实施约束

- 仅新增 `shlex = "1"`、`globset`、`starlark = { version = "0.14", default-features = false }`；OS 专属代码只放 `src/platform/`（`review.md` P2-1 修复 `denial.rs` 残留）。
- 生产代码禁止 `unwrap()`/`expect()`；用户可见文案与模型提示词一律英文。
- 审批遵循默认拒绝、Esc/Ctrl+C 拒绝；任何沙箱不可用场景 fail-closed。

## 15. A 批：on-failure 重试与审批

- [x] `src/chat/sandbox/denial.rs`：`is_likely_sandbox_denied(result, plan)` 与退出码排除。
- [x] `ProcessResult.sandbox_retry: Option<SandboxRetry>` 与 JSON/模型格式化同步。
- [x] `agent_loop/{mod,sandbox_decision}.rs`：单次重试、拒批返回原始失败、裸跑跳过重试。
- [x] 会话精确规则 `kind='exact'` 与 `SandboxApprovalKind::Retry` 交互。

## 16. B 批：完整权限档案

- [x] `src/chat/sandbox/permissions.rs`：条目模型与解析器（字面路径、`globset`、`Deny > Write > Read`）。
- [x] schema 支持 `additional_permissions.file_system.{read,write,deny}` + `network.enabled`；仅 `deny` 接受 glob，且仅 `deny` 变更免审批。
- [x] 审批面板展示摘要；`Additional` 仅限 `Once`；Seatbelt/bwrap 投影与 `sandbox_gate` 对齐。

## 17. C 批：Starlark 规则

- [x] 移植 `src/chat/exec_policy/{decision,error,rule,parser,policy,mod}.rs`。
- [x] `<user_data_root>/rules/*.rules` 加载、自检、保守分段、热加载、`Always` 落盘 `default.rules` 与设置页只读状态。

## 18. 批次验收线

- [x] A/B/C 三批本地实现与基础单测验收线通过（`bwrap` 与审查修复见 §20）。

## 19. D 批补充交付（命令面板 / 状态标识 / 切档感知与跟随 / 审计日志）

- [x] 审批与 `/permission` 面板统一样式与键鼠交互（`CommandPanelView`、`↑/↓`、`Enter`/`Tab`/鼠标、禁用普通键）。
- [x] subagent 申请审批时顶栏右侧标注 `[Subagent]`、上下文首行强调展示 `Subagent: <nickname> [role]`。
- [x] `lx` 非 `Web` 窗格左上角展示 `lx-<Entry> [<Mode>]`（`Read-only` 绿、`Workspace` 黄、`Full` 红，方括号暗灰，`lx-Web` 省略）。
- [x] DB v32 `agent_permission_switch` 表、`<system-update>` 按 `sequence` 注入、可折叠 `◆ Permission switched` 卡片及 `/fork` `/rewind` 链路接入。
- [x] subagent 每轮动态继承主会话最新模式（变更注入 `<system-update>`）与 `/btw` 只读面继承最新 `SandboxSettings`。
- [x] `lx::agent::approval` 审计日志覆盖规则绕过/拒绝、审批入队、裁决（`wait_ms`/`decision`）、取消与孤儿清理。

## 20. 代码审查修复与延后验证（`review.md` P0–P3）

- [x] **P0 修复**：`bwrap` 能力探测（userns / setuid 回退 / `--perms` 检测）与 `denial.rs` 排除启动错误（P0-1）、`read-only`/写根外 `deny` 挂载（P0-2）、`command_is_approved` 禁元字符绕过（P0-3）、`exec_policy::decide` 拆元字符子段防绕过（P0-4）、Seatbelt/档案符号链接规范化（P0-5）、`Rule` `Always` 写会话前缀压制 `Prompt` 循环（P0-6）、`ask` vs `approval` 键鼠与渲染优先级统一（P0-7）、审批拦截下层浮层与鼠标全矩形（P0-8）、`answer` 按 `(key, call_id)` 清理（P0-9）、v33 幂等迁移兜底（P0-10）。
- [x] **P1 修复**：`bwrap` 传真实 `workdir`（P1-1）、写根/carveout/符号链接/嵌套 `deny`/防 `/tmp/.git` 残留（P1-2）、`is_banned_prefix` basename+大小写防绕过（P1-3）、`sandbox_gate` 逻辑+解析双重判定（P1-4）、`maybe_retry_on_denial` 限 `exec_command`（P1-5）、`lx exec` 强制 `never`/裁剪/加载规则及 Windows 免弹窗（P1-6）、审批 `Ctrl+C` 与粘贴拦截（P1-7）、`/rewind` 回滚 `sandbox_mode` 与规则（P1-8；独立 `LatestKind::PermissionSwitch` 转 undo）、`fork` 按 `before` 推导 `sandbox_mode`（P1-9）、`PersistSandboxMode` 多窗格同步与错误处理+入口切站重置（P1-10）、`db/sandbox.rs` JSON 数组编码（P1-11）、上下文 `kept.is_empty()` 与动态切换/skill 预算（P1-12）、设置 API 损坏回落/500/`user_root`（P1-13）、窄窗格顶栏防撞 `[>_]`（P1-14）。
- [x] **P2 修复**：`denial.rs` 去除 `cfg(target_os)`（P2-1）、`append_allow_rule` 补前导换行（P2-2）、`save_sandbox` 先归一化后校验（P2-3）、Seatbelt `deny_globs` 祖先/写根/目录 glob 修复（P2-4）、`sweep_orphans` 二轮全检（P2-5）、审批队列 `+N pending` 与窄宽 `[Subagent]`/`[role]` 保留（P2-6）、`Permission switched xN` 折叠收敛（P2-7）、`SandboxChanged` 刷新 `/permission` + `#view-sandbox` 滚动与 `reason` 展示（P2-8）、`fork_completed_history_window_to_child` 去无效切档复制（P2-9）、`Lx::agent_state()` 待审批 `Blocked` 状态（P2-10）。
- [x] **P3 修复**：`LX_SYSTEM_PROMPT` 压回 12 KiB / 修文档与 `<truncated>` / 空串与摘要回显（P3-1）、收口文案到 `ui/text/sandbox.rs` / 清死常量 / 修单测名与 HTML 反引号（P3-2）、登记超行文件并保持增量受控（P3-3）、同步 `design.md`/`task.md`/`verification.md`/`html-assets.md` 与 `docs/undo/sandbox-permissions.md`（P3-4）。
- [ ] **远程 Linux / Windows 实机验证（延后）**：已按决策暂缓执行，具体步骤与环境前提见 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`。
