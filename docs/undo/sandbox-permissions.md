# Sandbox & Permissions — 未完成 / 延后事项清单（Undo）

- **来源分支**：`feature/sandbox-permissions`（基线 `41d5977b`，合入提交 `9e9d0ae6`，已于 2026-11-17 以 `--no-ff` 合入 `dev`；原 worktree 与分支已移除）
- **参考文档**（均已在 `dev`）：
  - `docs/feature/sandbox-permissions/design.md`
  - `docs/feature/sandbox-permissions/task.md`
  - `docs/feature/sandbox-permissions/review.md`
  - `docs/feature/sandbox-permissions/verification.md`

本文统一收拢 `design.md`、`task.md` 与 `review.md` 中**尚未验证或尚未实现**的事项。Schema 已在合入时推进至 **v33**（v30 `agent_compact_checkpoint.kind` + v31 沙箱模式/规则 + v32 权限切换 + v33 幂等兜底），原 worktree 隔离库已随 worktree 删除；以下仅保留后续工作。

---

## 1. 远程 Linux（`bwrap`）编译与实机验证（延后）

- **目标主机参考状态**（已探明 `8.138.213.110`）：
  - `CentOS Linux 7.9`（内核 `3.10.0-1160.119.1.el7.x86_64`、`glibc 2.17`、`1756 MiB` 内存 + `1024 MiB` swap）；
  - 未安装 Rust/Cargo；已通过 `yum install -y bubblewrap` 安装 setuid `/usr/bin/bwrap 0.3.0`；
  - `/proc/sys/user/max_user_namespaces = 0`（`/proc/cmdline` 无 `user_namespace.enable=1`），`bwrap 0.3.0` 不支持 `--perms`。
- **已完成的代码适配（待 Linux 实机运行验证）**：
  - `src/platform/sandbox/bwrap.rs` 已补齐 `OnceLock` 能力探测（支持 `--unshare-user` / setuid 无 userns 回退 / `Unavailable` fail-closed、`--perms` 探测）、`read-only` 与写根外 `deny` 遮盖、真实 `workdir` `--chdir`、浅→深挂载分层、避免 `/tmp/.git` 宿主机残留；`src/chat/sandbox/denial.rs` 已排除 `bwrap` 启动报错。
- **待执行项**：
  1. 在本地准备快速交叉编译镜像/工具链（推荐内网加速后的 `x86_64-unknown-linux-musl` + `zig` 或现代 Linux 容器，避开 `8.138.213.110` 的 1.7 GiB 内存 OOM 与 `glibc 2.17` 限制）或直接在现代 Linux（Ubuntu 22.04/24.04）编译执行。
  2. 运行全套 Linux 专属单元测试：`cargo test --lib platform::sandbox::bwrap`（验证 `SetuidFallback`、`Unavailable` fail-closed、`supports_perms=false` 拒绝 `deny` 目录、`read_only` 写根外 `deny`、嵌套 `Write/Read/Write`、无 `/tmp/.git` 残留）。
  3. 实机端到端断言：
     - `read-only` 禁写工作区但允许读；
     - `workspace-write` 限写工作区/`$TMPDIR`/`/tmp`、`.git` 只读、`--unshare-net` 断网；
     - `danger-full-access` 全通；
     - PTY 模式隔离与 `Esc`/Stop 无进程树残留；
     - 缺失 `bwrap` 或非 setuid 且禁 `userns` 时 `support()` 返回 `Unavailable` 且严格 fail-closed。
  4. 将实际内核版本、`bwrap` 版本、userns 状态与命令实测结果补入 `verification.md` 与 `task.md`。

---

## 2. Windows 进程级沙箱与端到端验证（v1 明确延后）

- **现状**：
  - v1 仅实现了纯 Rust 的文件工具闸门（`src/chat/tools/sandbox_gate.rs` 约束 `write`/`edit`/`apply_patch`）；
  - `SandboxSupport::FileToolsOnly` 下 `exec_command` 直通执行，`sandbox_decision.rs` 已屏蔽无意义的 `require_escalated` 弹窗，设置页与 `<sandbox>` 快照标注 `file-tools-only`。
- **待做项**（`design.md` §7、§19）：
  1. 在 `src/platform/sandbox/` 下实现 Windows 进程级沙箱（受限令牌 + Job Object，或对齐 `codex-rs/windows-sandbox-rs` 落地工作区读写与网络限制）。
  2. 在真实 Windows 环境完成文件工具闸门与进程沙箱的端到端集成测试（写入越界、符号链接、`.git` 保护、断网、设置页提示条），并补录 `verification.md`。

---

## 3. macOS TUI 交互外观与真实 LLM 端到端验收（待人工）

Agent 无法交互式操作全屏 TUI，`verification.md` §7 下列场景需在 `dev` 主工作区运行 `cargo run` 人工确认：

1. **审批与 `/permission` 命令面板（`P-1` / `P-2`）**：
   - 无边框蓝线 `CommandPanelView` 贴输入框上方渲染；
   - `↑/↓`、`Enter`/`Tab`、鼠标悬停/点击、`Esc`/`Ctrl+C` 与 `y/r/a/n` 行为一致；普通按键与粘贴不落入底层草稿；
   - 多条审批排队显示 `+N pending`、有 `ask` 并发时优先遮罩渲染审批面板、压制 `@`/`/`/`paste` 等下层浮层；
   - `/permission` 面板 `[Default]` 标记、过滤、回显 toast 及左上角 `lx-<Entry> [<Mode>]` 绿/黄/红配色（`lx-Web` 省略），分屏窄窗格不撞 `[>_]`。
2. **真实 LLM 子 agent 自主提权与审计日志（`P-6`）**：
   - 使用提示词测试“使用 subagent 在桌面编写 ts 冒泡排序”，核对 `lx-18` 下主 agent 不再禁掉子 agent 提权，由 `Turing [executor]` 自主触发带 `[Subagent]` 与 `Subagent: Turing [executor]` 的审批面板；
   - 批准后确认子 agent 立即完成（不再卡在 `running`），并核查 `~/.lx-t-dev/logs/lx-tui.log.YYYY-MM-DD` 中 `lx::agent::approval` 结构化审计日志。
3. **权限切换时间线与跟随（`D-4` / `D-5` / `D-6`）**：
   - 切档后渲染可折叠 `◆ Permission switched` 卡片（`  └─ ` 树形前缀、连续合并折叠正常），`/fork` / `/rewind` 正确跟随回滚；
   - subagent（`resume`/`send_input`）与 `/btw` 跟随主会话最新模式。
4. **设置页视觉与交互（`P-3` / `C-7`）**：
   - Sandbox tab 表单与 `#view-sandbox` 纵向滚动、坏 `sandbox.json` 告警回落恢复、保存后左上角模式与 `/permission` 面板实时刷新、`Open folder` 打开 `~/.lx-t-dev/rules`。

---

## 4. 合入后遗留重构与边界项

1. **既有超行文件拆解（`review.md` P3-3）**：
   - 合入 `dev` 后当前行数：
     - 生产文件（>1000）：`src/app/update/chat/mod.rs`（1146）、`src/app/state/lx/chat.rs`（1085）、`src/ui/text.rs`（1085）、`src/db/chat/mod.rs`（1077）、`src/app/update/chat/subagent.rs`（1049）、`assets/web.html`（1018）。
     - 测试文件（>1500）：`src/app/update/tests.rs`（6100）、`src/ui/tests.rs`（3298）、`src/settings/service/tests.rs`（1914）、`src/db/chat/subagent/tests.rs`（1748）、`src/db/tests.rs`（1671）、`src/app/update/chat/tests.rs`（1532）。
   - 拆分时保持 `app/` 与 `ui/` 边界、测试跟随被测模块层级；优先从 `src/ui/text.rs` 继续把沙箱/权限文案迁入 `src/ui/text/sandbox.rs`。
2. **独立 `PermissionSwitch` 单独撤销入口（`review.md` P1-8 边界说明）**：
   - 目前 `/rewind` 删对话已联动回滚 `agent_session.sandbox_mode` 并清理 `agent_session_sandbox_rule`；仅“切换了 `/permission` 但尚未发下一条消息”时直接按双 `Esc` 独立撤销单条 `PermissionSwitch` 卡片（等价 `RewindTargetKind::ModelSwitch`）未单独新增 UI 动作（可通过 `/permission` 直接切回或撤销所在轮次）。如有需要，后续可补 `RewindTargetKind::PermissionSwitch` 与 `RevertPermissionSwitch` 效应。
3. **旧二进制回滚限制**：
   - v33 库不能被 `SCHEMA_VERSION < 33` 的旧二进制直接打开；回退前必须使用迁移前的数据库备份，不能仅替换二进制。

---

## 5. `design.md` 明确暂不纳入范围项（保留备忘）

以下能力在 `design.md` §1 / §19 中明确标记不做（如后续产品演进需要再单独立项）：

- `PathUri` / `FileSystemSpecialPath` 特殊变量解析（当前仅支持本地绝对路径与 `deny` glob）；
- 远程 `app-server` / `exec-server` 沙箱代理；
- HTTP/SOCKS 代理路由、MITM 域名粒度放行（当前仅 `network_access` 布尔开关）；
- 独立 `request_permissions` 工具（仅通过 `exec_command` 参数与 on-failure 重试）；
- MCP / Skill 调用纳入沙箱或审批体系；
- 跨会话 / 跨工作区的会话级审批缓存（跨会话仅通过 `rules/*.rules`）；
- 浏览器设置页内直接编辑 `.rules` 文件。
