# Agent 沙箱与权限模式设计

状态：草案（待用户审核）。参考仓库：`/Users/yonah/projects/agent/codex-main`（只读）。目标仓库：`/Users/yonah/projects/agent/lx-tui` 当前 checkout。

本文中的实现细节、文件路径与关键代码在用户确认后作为 `task.md` 的实施依据。

## 1. 目标与边界

为 lx 的 agent 执行链路加入与 Codex 同源语义的沙箱：

- **OS 级沙箱**：所有模型发起的 `exec_command` 子进程在宿主平台沙箱内执行（macOS seatbelt、Linux bubblewrap）。
- **应用级闸门**：`sandbox_mode` 同时约束内置写工具（`write` / `edit` / `apply_patch`），避免模型用文件工具绕过进程沙箱。
- **可配置**：全局默认写在 `config/sandbox.json`，可在浏览器设置页编辑；会话级覆盖通过 `/permission` 切换并落库。
- **可申请提权**：模型可直接在 `exec_command` 上带 `sandbox_permissions`；沙箱拒绝后也会自动触发 on-failure 审批重试。
- **完整权限档案**（B 批）：`with_additional_permissions` 支持 `read` / `write` / `deny` 与 glob 条目，并按 Codex 的"具体路径优先、deny 优先"规则合并。
- **Starlark 规则文件**（C 批）：`<user_data_root>/rules/*.rules`，兼容 Codex 的 `prefix_rule(...)` 语法，支持 `allow` / `prompt` / `forbidden`。

交付分三批（同一份设计，独立验收）：

| 批次 | 内容 | 依赖 |
|---|---|---|
| A | on-failure 自动重试 + 申请、审批 strip、会话精确命令规则 | 现有沙箱 |
| B | 完整 `additional_permissions` 档案（read/write/deny/glob）与两种后端投影 | A 的审批通道 |
| C | Starlark execpolicy（规则目录、语法、热加载、模型 `prefix_rule` 落盘） | B 的路径匹配 |

明确不做：

- 不沙箱用户的 `!` 本地命令、MCP server 子进程、终端窗格。
- Windows v1 不做进程级沙箱（见 §7），文档标记为后续实现。
- 不复制 Codex 的 `PathUri` / `FileSystemSpecialPath` / app-server 远程执行器抽象；lx 只处理本地绝对路径与 glob。
- 不做跨工作区的全局审批缓存；`ApprovedForSession` 与规则文件是两条独立通道。

## 2. 用户已确认的决策

| # | 决策 | 结论 |
|---|---|---|
| 1 | 平台范围 | macOS + Linux + Windows（Windows 仅文件工具闸门，进程沙箱后续实现） |
| 2 | 沙箱边界 | 双轨：OS 沙箱约束 shell；模式同时闸门内置写工具 |
| 3 | 默认模式 | `workspace-write` |
| 4 | 网络默认 | 受限档默认拒绝网络，设置页可开 |
| 5 | 覆盖路径 | 只套 agent 的 `exec_command`（含子 agent 与 `lx exec`），不套 `!` 命令与 MCP |
| 6 | 提权方式 | 参考 Codex：`require_escalated` + 审批；v1 支持 `prefix_rule` 与会话级缓存 |
| 7 | 审批选项 | `Approve once` / `Approve for session (prefix)` / `Deny` |
| 8 | 能力阶段 | 分 A/B/C 三批：A on-failure 重试与审批；B 完整 `additional_permissions`（read/write/deny/glob）；C Starlark 规则文件 |
| 9 | 审批缓存 | 会话级落库，随 resume 恢复；不跨会话/工作区；fork 不继承 |
| 10 | 配置载体 | 新建 `config/sandbox.json` + `/api/sandbox` + 设置页 Sandbox 分区 |
| 11 | Linux 后端 | 只走系统 `bwrap`；缺失时 fail-closed |
| 12 | Linux 验证 | 在云主机实测 bwrap 行为，输出记录进验证章节 |
| 13 | Windows 后端 | v1 仅文件工具闸门，明确标注 `file tools only`，进程沙箱后续实现 |
| 14 | `/permission` 交互 | 一级选择面板，选中立即生效 + toast；不常驻状态栏 |
| 15 | 审批 UI | 输入框上方固定 strip，`y/a/n` 键位，Esc=Deny |
| 16 | 提权总开关 | `escalation: "ask" | "never"`，默认 `ask` |
| 17 | 凭据读取 | 不做读取限制（完全对齐 Codex）；风险显式记录 |
| 18 | 交付方式 | 同一设计分 A/B/C 三批：A on-failure、B 完整权限档案、C Starlark 规则 |
| 19 | 完整权限档案 | `additional_permissions` 支持 read/write/deny + glob；具体路径优先、deny 优先 |
| 20 | Starlark 规则 | `rules/*.rules` 兼容 Codex `prefix_rule`；支持 allow/prompt/forbidden、热加载、模型提议落盘 |

## 3. 策略模型

### 3.1 类型（`src/config/sandbox.rs`）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EscalationPolicy {
    Ask,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSettings {
    #[serde(default = "default_mode")]
    pub mode: SandboxMode,                 // 默认 workspace-write
    #[serde(default)]
    pub network_access: bool,              // 默认 false
    #[serde(default = "default_escalation")]
    pub escalation: EscalationPolicy,      // 默认 ask
    #[serde(default)]
    pub writable_roots: Vec<PathBuf>,      // 额外可写根（绝对路径）
}
```

- 文件路径：`user_data_root()/config/sandbox.json`，与 `mcp.json` / `subagents.json` 同族；缺失文件 = 全默认，损坏文件报错但不阻塞启动（回落默认 + 可见告警）。
- 保存沿用 `config::subagents` 的临时文件 → `0600` → 原子 `rename` 模式。
- 校验：`writable_roots` 必须是绝对路径、无控制字符、去重、最多 16 条；不存在的路径允许保存（用户可能先配置后创建），但执行期跳过并记录警告。
- 额外可写根**只在 `workspace-write` 下生效**；`read-only` 忽略；`danger-full-access` 无意义。

### 3.2 可写根计算（共享纯逻辑，`src/platform/sandbox/roots.rs`）

`workspace-write` 的有效可写根，按顺序去重：

1. `cwd`（每轮冻结的工作区 cwd，`ProcessManager::exec_observed` 已解析出的绝对路径）；
2. `$TMPDIR`（非空时）；
3. `/tmp`（Unix；macOS 上规范化为 `/private/tmp`）；
4. 配置里的 `writable_roots`（`~` 展开，规范化为绝对路径）。

每个根额外生成"保护子路径"（只读）：`<root>/.git`（目录或 worktree 指针文件），以及存在时的 `<root>/.agents`、`<root>/.aws`。`.git` 为文件（worktree 的 `gitdir:` 指针）时，同时保护文件本身与指针指向的真实目录。保护规则必须**排在允许规则之后**（seatbelt 后者覆盖前者，已实测）。

规范化：`std::fs::canonicalize` 失败时退回 `absolute` 路径并记录警告；SBPL 通过 `-D` 参数传路径（`(subpath (param "WRITE0"))`），**不做字符串插值**，避免路径中的引号/反斜杠破坏策略（已实测带空格路径可用）。

> B 批把上述"根 + 保护子路径"升级为 §17 的条目模型；本节描述的是 A 批的基准行为，B 批保持同一默认结果。

### 3.3 网络语义

- `read-only` / `workspace-write`：`network_access = false` 时不产生任何 network 允许规则（seatbelt 默认拒绝；bwrap 加 `--unshare-net`）。
- `network_access = true`：macOS 追加 Codex 的 `seatbelt_network_policy.sbpl` + `(allow network-outbound)(allow network-inbound)`；Linux 不加 `--unshare-net`。
- `danger-full-access`：不套沙箱，网络自然全通。

### 3.4 模式与平台能力

`SandboxSupport` 由平台模块给出，用于 UI、提示词与执行期判定：

| 平台 | `read-only` / `workspace-write` | `danger-full-access` |
|---|---|---|
| macOS | seatbelt 进程沙箱 | 不套沙箱 |
| Linux | bwrap 进程沙箱（缺 bwrap → fail-closed） | 不套沙箱 |
| Windows | 仅文件工具闸门，shell 直通（状态标注 `file tools only`） | 不套沙箱 |

## 4. macOS Seatbelt 实现

### 4.1 入口与命令行

`src/platform/sandbox/seatbelt.rs`：

```rust
pub fn wrap(spec: &SandboxSpec, program: &OsStr, args: &[OsString]) -> Result<Wrapped, SandboxError> {
    let profile = build_profile(spec)?;
    let mut argv = vec![OsString::from("-p"), OsString::from(profile)];
    for (key, value) in spec.param_dirs() {
        argv.push(OsString::from(format!("-D{key}={}", value.display())));
    }
    argv.push(OsString::from("--"));
    argv.push(program.to_owned());
    argv.extend(args.iter().cloned());
    Ok(Wrapped { program: PathBuf::from("/usr/bin/sandbox-exec"), args: argv })
}
```

- 直接用 `/usr/bin/sandbox-exec`（本机 macOS 26.2 实测可用；不存在时 fail-closed）。
- 进程退出码与信号原样透传（实测 `exit 7` → 7、`SIGTERM` → 143），不改变 `ProcessManager` 的状态语义。
- `login` shell 在沙箱内正常（实测 `zsh -l -c` 可用）。
- PTY 与 pipe 两种 spawn 都必须走同一 `wrap`：PTY 用 `CommandBuilder::new("/usr/bin/sandbox-exec")` + args。

### 4.2 Profile 组装

按 Codex 的顺序拼接（每段以换行分隔）：

1. `assets/sandbox/seatbelt_base_policy.sbpl`（从 Codex 复制，保留来源注释与许可证说明）；
2. 读策略：**v1 允许全盘读** `(allow file-read*)`（与 Codex `read-only` 语义一致；见风险 §15）；
3. 写策略：仅 `workspace-write` 生成，每个可写根一条
   `(allow file-write* (require-all (subpath (param "WRITE{n}")) (require-not (literal (param "PROTECT{m}"))) (require-not (subpath (param "PROTECT{m}"))) ...))`
   —— 保护路径必须同时排除 `literal` 与 `subpath`，否则首次创建同名路径会打洞（Codex `seatbelt.rs:550-555` 的同一理由）；
4. 保护祖先：对每个保护子路径在可写根内的祖先目录追加
   `(deny file-write-unlink (require-all (vnode-type DIRECTORY) (literal (param "PROTECT_ANCESTOR{n}"))))`，放在最后，防止通过重命名父目录绕过（Codex `seatbelt.rs:1013-1022`）；
5. `network_access = true` 时追加 `seatbelt_network_policy.sbpl` + `(allow network-outbound)(allow network-inbound)`；
6. 固定加固（已实测不影响 `cargo`/`rustc`）：
   - `(deny mach-lookup (xpc-service-name-prefix ""))`
   - `(deny system-fcntl (fcntl-command 80 110))`（F_MAKECOMPRESSED / F_TRANSFEREXTENTS 可绕过 `file-write*`，Codex `seatbelt.rs:1024-1029`）
7. `/dev/null` 写入豁免（基础策略已含，若裁剪需保留）。

保护路径列表（`workspace-write` 下每个可写根）：

- `.git`：目录形态直接保护；`.git` 为 worktree 指针文件时，同时保护文件本身与 `gitdir:` 指向的真实目录（Codex `permissions.rs:2385-2396` 的做法）；
- `.agents`、`.aws`：仅当存在时保护（对齐 Codex 默认元数据保护，去掉 lx 不使用的 `.codex`）；
- 不做"保护缺失的 `.codex`"这类 Codex 专属行为。

**关键实测结论（2026-11-16，macOS 26.2 arm64）**：

- 只用"最小 profile"（`deny default` + `process-exec/fork` + `file-read*`）时，`cargo --version` / `rustc --version` 直接 `failed to allocate a guard page: Invalid argument`（abort 134）。必须使用 Codex 基础策略里那批 `sysctl-read` / `mach-lookup` / `pseudo-tty` 允许规则。
- 基础策略**不含**全局 `file-read*`（Codex 是按根生成读策略）。lx v1 显式追加 `(allow file-read*)` 后，`cargo`、`rustc`、`git`、`zsh -l` 全部正常。
- 网络默认拒绝时 `curl` 报 `Could not resolve host`（DNS 也被挡）；`.git` 保护按"先允许后拒绝"生效。
- 路径参数用 `-D` 传递，带空格路径正常；不要拼接进 profile 字符串。

### 4.3 失败语义

| 失败 | 行为 |
|---|---|
| `sandbox-exec` 不存在 | 返回 `SandboxError::Unavailable`，`exec_command` 失败（`sandbox_unavailable`），提示切换到 `danger-full-access` 或修复系统 |
| profile 生成失败（路径非法等） | 同上，不降级裸跑 |
| 子进程被沙箱拒绝 | 命令以真实退出码/`Operation not permitted` 结束，输出原样回给模型；提示词要求模型识别并申请提权 |

## 5. Linux Bubblewrap 实现

### 5.1 依赖与能力探测

- 只使用系统 `PATH` 上的 `bwrap`；lx 不打包 bwrap。
- 启动/首次使用时探测 `bwrap --version` 与 `bwrap --unshare-user --dev-bind / / true` 是否可用（user namespace 被 AppArmor 限制时后者失败）。
- 不可用时 `SandboxSupport::Unavailable`，受限档的 `exec_command` **fail-closed**（`sandbox_unavailable: install bubblewrap and enable unprivileged user namespaces`），不静默裸跑。

### 5.2 参数构造（对齐 Codex `bwrap.rs`）

`workspace-write` 基线（Codex `linux-sandbox/src/bwrap.rs:334-378` 的裁剪版）：

```
bwrap
  --new-session --die-with-parent
  --ro-bind / /
  --dev /dev
  --unshare-user --unshare-pid --unshare-ipc
  --unshare-net                # network_access=false 时
  --bind <root> <root>         # 每个可写根（浅→深排序）
  --ro-bind <root>/.git <root>/.git   # 保护路径，排在 writable bind 之后
  --proc /proc
  --chdir <cwd>
  --cap-drop ALL
  -- <shell> <args>
```

`read-only`：去掉所有 `--bind`（`/` 保持只读）。

`danger-full-access`：不包装。

- 可写根的浅→深排序是必须的：先 bind 父根再 bind 子根，避免后者的父目录被前者覆盖时丢失（Codex `:608-643`）。
- 保护路径通过 `--ro-bind <p> <p>` 重新只读挂载（Codex `:682-708`）；路径是符号链接时先解析到真实目标，拒绝跨可写符号链接（TOCTOU）。
- 不引入 seccomp 过滤器（Codex 另有 `seccomp.rs`）：v1 依赖 bwrap 的 namespace + `--cap-drop ALL` + `no_new_privs`；`--unshare-net` 断网。此差异在风险章节标注。
- 根不存在时跳过并警告；`/tmp`、`$TMPDIR` 作为普通可写根参与排序，不额外 `--tmpfs /tmp`（避免遮蔽宿主已有内容）。
- `--new-session` 同时用于 pipe 与 PTY，保证进程组/信号语义一致；停止仍按进程组终止。
- 失败（bwrap 非 0 退出且 stderr 指向 userns 创建失败）→ 原样返回输出，工具错误码保持进程语义，不伪造"沙箱拒绝"。

## 6. 文件工具闸门

### 6.1 单一入口

新增 `src/chat/tools/sandbox_gate.rs`，所有写工具（`write` / `edit` / `apply_patch`）在**执行任何磁盘变更之前**调用同一判定：

```rust
pub(crate) fn ensure_writable(
    ctx: &ToolContext,
    targets: &[&Path],
) -> Result<(), ToolFailure>;

pub(crate) fn ensure_writable_resolved(
    ctx: &ToolContext,
    logical: &Path,
    resolved: &Path,
) -> Result<(), ToolFailure>;
```

判定输入来自 `ToolContext` 新增的只读策略快照：

```rust
pub struct SandboxPolicyView {
    pub mode: SandboxMode,
    pub writable_roots: Vec<PathBuf>,   // 已规范化、已含 cwd/TMPDIR/额外根
    pub protected: Vec<PathBuf>,        // 每个根的 .git/.agents/.aws
}
```

### 6.2 判定规则

| 模式 | 结果 |
|---|---|
| `read-only` | 一律 `sandbox_denied`（错误文案提示 `/permission`） |
| `workspace-write` | 目标必须位于任一可写根内，且不位于保护子路径内；否则 `sandbox_denied` |
| `danger-full-access` | 直接放行 |

实现细节：

- 目标路径按"逻辑路径 + 解析后路径"双重判定（沿用 `write.rs` 现有的 `path::write_target` 语义）：符号链接指向根外时，解析后路径会失败；根内符号链接指向根内时通过。
- 目标不存在时，取最近存在的祖先目录做 `canonicalize`，再拼回剩余组件；禁止 `..` 逃逸。
- `apply_patch` 的删除/重命名同样过闸（`write` 之外的文件变更也算写）。
- 保护子路径判定用规范化后的前缀比较，且要求按路径组件边界（避免 `/ws/.gitlab` 被误判为 `.git`）。
- 失败必须在**任何** mutation 发生前返回，避免部分写入；`ToolFailure` 带 `sandbox_denied` 与英文原因。
- 闸门只读 `ctx`，不访问全局状态；策略快照在每轮开始时冻结，`/permission` 只影响下一轮。

## 7. Windows

v1 明确不做进程级沙箱：

- `SandboxSupport::FileToolsOnly`：`exec_command` 直通（与当前行为一致），状态/设置页/提示词显示 `sandbox: file tools only (Windows)`。
- 文件工具闸门照常生效（`read-only` 禁写、`workspace-write` 限可写根），这部分是纯 Rust 逻辑，跨平台一致。
- `sandbox_permissions=require_escalated` 在 Windows 上无意义：直接按默认执行，不弹审批。
- 后续实现（不在本次）：受限令牌 + Job Object 的最小进程沙箱，或对齐 `codex-rs/windows-sandbox-rs`。已在 `task.md` 标记为后续项。

## 8. 提权与审批

### 8.1 模型参数与类型

`exec_command` 新增（仅在 `escalation = ask` 时出现在 schema）：

```json
{
  "sandbox_permissions": "use_default | with_additional_permissions | require_escalated",
  "justification": "user-facing question",
  "prefix_rule": ["git", "push"],
  "additional_permissions": {
    "network": {"enabled": true},
    "file_system": {"write": ["/abs/path"]}
  }
}
```

解析层（`src/chat/tools/process_args.rs`）产出：

```rust
pub(crate) struct ExecRequest {
    pub args: ExecArgs,
    pub escalation: Option<SandboxEscalation>,
}

pub(crate) struct SandboxEscalation {
    pub kind: EscalationKind,             // Additional | Unsandboxed
    pub justification: String,            // 非空，<= 500 字符
    pub prefix_rule: Option<Vec<String>>, // 仅 Unsandboxed；1..=6 token，拒绝危险前缀
    pub additional: Option<AdditionalPermissions>, // 仅 Additional
}
```

校验规则：

- `justification` 缺失或空白 → `invalid_arguments`。
- `sandbox_permissions` 缺省 = `use_default`；带其它字段但值不是对应种类 → 拒绝。
- `prefix_rule`：拒绝空数组、token 含控制字符、含 shell 元字符（``| ; & $ ` ( ) { } < >``）、以及 Codex 的 `BANNED_PREFIX_SUGGESTIONS` 列表（`codex-rs/core/src/exec_policy.rs:64-153`：shell、`rm`、`sudo`、解释器、`git`、`npm run` 等）。
- `additional_permissions.file_system.write`：绝对路径、无控制字符、最多 8 条；`read`/`deny`/glob 字段在 A 批直接拒绝（`invalid_arguments`），B 批按 §17 完整支持。
- `additional_permissions` 只允许 `with_additional_permissions`；`prefix_rule` 只允许 `require_escalated`。

### 8.2 决策流程（`agent_loop/mod.rs`）

在 `begin_tool` 之后、交给 executor 之前：

```text
parse_exec_request(arguments_json)
├─ escalation = None
│    └─ executor.execute_tracked(..., SandboxOverride::None)
├─ kind = Additional
│    ├─ escalation policy = Never → ToolFailure(sandbox_denied, "escalation disabled by settings")
│    └─ host.approve_sandbox(call_id, request).await
│         ├─ Once     → execute(..., SandboxOverride::Additional { permissions })
│         ├─ Denied   → ToolFailure(sandbox_denied, "the user denied sandbox escalation.")
│         └─ Cancelled → finish_tool(Cancelled)，按现有中断路径结束本轮
└─ kind = Unsandboxed
     ├─ 命中会话前缀规则 → execute(..., SandboxOverride::Unsandboxed)（不再询问）
     ├─ policy = Never      → ToolFailure(sandbox_denied, ...)
     └─ host.approve_sandbox(...).await
          ├─ Once             → execute(Unsandboxed)
          ├─ ForSession(rule) → 写规则表 + execute(Unsandboxed)
          └─ Denied/Cancelled → 同上
```

- 模型 `prefix_rule` 的会话规则用 `shlex` 分词后的 token 前缀比较；on-failure 的会话规则用**完整 token 序列**全等比较（不做大小写折叠，避免 `GIT` 绕过）。
- `a`（Approve for session）仅在 `require_escalated` 且模型给出合法 `prefix_rule` 时可用；`with_additional_permissions` 只能批准一次。
- 审批等待不设超时：turn 保持阻塞，用户处理或按 `Esc` 拒绝；应用退出时回执通道丢弃，按 `Cancelled` 结算。
- 审批前发送的 `BeginTool` 已落库，审批结果进入同一工具结果文本：`approved once` / `approved for session: git push` / `denied by user`。
- `danger-full-access` 会话：参数可被接受但无实际作用，**不弹审批**（命令本来就裸跑）。
- 无头 `lx exec`：没有交互 UI，任何 escalation 直接返回 `sandbox_denied: escalation requires an interactive session`；CLI 的 `escalation` 按 `never` 处理。
- Windows（file tools only）：`require_escalated` 不弹审批，按默认执行；`with_additional_permissions` 的网络/写根仍可通过文件工具闸门生效（shell 无沙箱）。
- 受保护路径同样覆盖 `git` 元数据：`git commit`、`git checkout` 等需要写 `.git` 的命令会被沙箱拒绝，模型应申请提权；这是对齐 Codex 的预期行为，不是缺陷。

### 8.3 Host 通道与 UI

- `AgentHostRequest` 新增 `ApproveSandbox { call_id, request, reply: oneshot::Sender<SandboxApprovalDecision> }`，`AgentHost` 增加默认异步方法（与 `ask_user` 同构，回执丢失按 `Cancelled`）。
- `AgentHostEvent` 新增对应变体；`src/event/chat/mod.rs` 分派到新模块 `src/event/chat/approval.rs`。
- `approval::request` 定位会话所属 lx 窗格，把请求推入 `pane.lx.approval`（FIFO），返回需要重绘；找不到窗格（headless/已关闭）→ 立即 `Denied`。
- 键盘：`apply_lx_input` 的优先级改为 `task > approval > ask`；`y`/`a`/`n` 与 `Esc=Deny`；等待期间 `Enter` 不提交输入。`Ctrl+C` 走现有取消路径（拒绝队首并中断对应轮次）。
- 审批状态挂在窗格而非消息：`ApprovalState { kind, call_id, request, highlight, scroll }`；`kind` 区分 `Escalation`（显式申请）与 `Retry`（on-failure）；`ask` 与审批同时存在时 approval 先显示，结束后继续 ask。
- 显式申请键位 `y/a/n`（`a` 仅在有 `prefix_rule` 时可用）；on-failure 重试键位 `r/a/n`（`a` 为"本会话同命令自动重试"）；`Esc` 一律等同拒绝。
- 子 agent 的审批进入父窗格队列；标签显示来源（如 `explorer · Sandbox escalation`），避免误以为是主 agent。

### 8.4 规则生命周期

- 表：`agent_session_sandbox_rule`（§11）。
- 会话启动时按 `kind` 加载到内存（`exact` 用于 on-failure 全等匹配，`prefix` 用于模型提议的前缀匹配）；`ForSession` / `Always retry` 立即插入内存与 DB。
- `/permission` 切档、`/new`、fork 均清空/不复制会话规则（§9.2）；规则文件不受影响。
- 会话删除由外键级联清理。

### 8.5 沙箱拒绝后的 on-failure 重试（A 批）

对齐 Codex `core/src/tools/orchestrator.rs:397-520` 的"先沙箱跑，被拒绝后申请重试"路径。

**识别（`src/chat/sandbox/denial.rs`）**：

```rust
pub(crate) fn is_likely_sandbox_denied(result: &ProcessResult, plan: &SandboxPlan) -> bool
```

- 仅当本次命令**确实套了沙箱**（`plan.process_sandbox == true`）且状态为 `Exited`；
- `exit_code` 为 `Some(code)` 且 `code != 0`；
- 快速失败码不判定：`2`（用法错误）、`126`、`127`（找不到命令）；Linux 上 `128 + SIGSYS(31) = 159` 直接判定为拒绝；
- 输出（stdout + stderr，已合并的 `result.output`）小写后包含任一关键词：`operation not permitted`、`permission denied`、`read-only file system`、`sandbox`、`seccomp`、`landlock`、`failed to write file`；
- `TimedOut` / `Interrupted` / `Stopped` / `Failed` 一律不判定（避免把停止误判成拒绝）。

**重试流程**（`agent_loop/mod.rs`，在 executor 返回 `ToolExecutionResult::Process` 之后）：

```text
if is_likely_sandbox_denied(result, plan) && !already_retried && escalation == Ask:
    host.approve_retry(call_id, RetryRequest { command, cwd, denial_excerpt }).await
    ├─ RetryOnce        → executor.retry_process(..., SandboxOverride::Unsandboxed)
    ├─ RetryForSession  → 写入"精确命令"会话规则 + 重试
    ├─ Denied           → 原样返回第一次的失败结果（模型看到真实错误）
    └─ Cancelled        → finish_tool(Cancelled)，中断本轮
```

- 每次工具调用**最多重试一次**，重试结果不再做拒绝识别，避免循环。
- 重试复用同一 `ExecArgs` 与 rtk 预处理后的命令；不新建 `BeginTool`，审批与两次尝试都归属同一个 tool call。
- 重试结果在模型可见文本前加一行：`sandbox denied the first attempt; retried with escalation.\n`，其余输出原样；`ProcessResult` 增加 `sandbox_retry: Option<SandboxRetry>` 供 UI 卡片标注。
- 会话规则为**精确 token 序列**（codex `ApprovalStore` 的 `ExecCommand` 键语义），不是前缀；下次同命令直接裸跑，不再先撞沙箱。表结构增加 `kind` 列区分 `exact` / `prefix`（§11）。
- 若 `escalation = never`、模式为 `danger-full-access`、或平台无进程沙箱（Windows）：不识别、不询问。
- 审批 strip 标题为 `Sandbox blocked command`，选项 `r Retry once` / `a Always retry this command (session)` / `n Keep blocked`；`Esc = n`。

### 8.6 仍与 Codex 不同的部分

- Codex 的拒绝识别是启发式（关键词子串），存在误报；lx 同样接受该代价，并在设计上限制"每次调用最多重试一次"。
- 不做 Codex 的 `ExecPolicyAmendment` 结构，规则写入用 lx 自己的 Starlark 文件（C 批，§18）。
- 不做 Codex 的 `NetworkPolicyAmendment` / MCP elicitation / skill 审批等旁路；lx 的审批只覆盖 `exec_command`。

## 9. `/permission`

### 9.1 命令与面板

- `SlashCommand` 增加 `Permission`（`ALL` 变 13 项），`name = "permission"`，别名 `["permissions", "sandbox"]`，描述 `Switch the sandbox mode for this session`。
- `/permission` 不加入 AI Web 的 5 条白名单（`command_entries_for`），AI Web 窗格不可用。
- 复用 `/model` 的一级选择面板机制（候选 + 选中回调），候选固定三项：
  - `Read only` — model 只能读，shell 不能写；
  - `Workspace write` — 仅工作区可写（默认）；
  - `Danger full access` — 无沙箱。
- 面板内显示当前档位：`session override` / `global default`；当前生效项打勾/高亮。
- 选中后立即写入会话覆盖并 toast：`Sandbox mode: workspace-write (applies to the next turn)`；运行中的 turn 保持起点冻结的策略（确定性优先），不常驻状态栏（决策 #14）。

### 9.2 会话覆盖

- 会话已落库：写 `agent_session.sandbox_mode`（`NULL` = 跟随全局默认）。
- 尚未落库（新会话还没有第一条消息）：在 `Lx` 窗格保存 pending override，首条消息创建会话时写入；`/new` 清空。
- `/permission` 切档时**清空该会话的审批规则**（`exact` + `prefix`，避免旧规则在新档位下继续放行未沙箱命令）；toast 之后追加 `session approvals cleared`。规则文件不清理。
- fork：复制当前 `sandbox_mode`（保持安全姿态），不复制审批规则；子 agent 创建时冻结父会话当前模式并写入子会话列，resume 后沿用。
- 全局默认变更（设置页）：只影响 `sandbox_mode IS NULL` 的会话；已在运行的 turn 使用 turn 起点冻结的策略，**下一个 turn 取最新**。

## 10. 配置与设置页

### 10.1 存储与 API

- `src/config/sandbox.rs` 提供 `load_sandbox(path)` / `save_sandbox(path, &SandboxSettings)`。
- 新路由组（`src/settings/service/handlers.rs`）：`(_, path) if path.starts_with("/api/sandbox") => super::sandbox::route(request, state)`。
  - `GET /api/sandbox` → `{ config, capabilities: { platform, process_sandbox, reason }, effective_default }`
  - `PUT /api/sandbox` → 校验 + 原子保存 + `SettingsEvent::SandboxChanged(Box<SandboxSettings>)`，返回归一化配置。
- 写请求沿用 `origin_ok` 与全局 `config_write_lock`。

### 10.2 设置页

- `assets/web.html`：左侧新增 `Sandbox` tab（在 `General` 之后），`#view-sandbox` 包含：
  - `Default mode`（select：Read only / Workspace write / Danger full access）；
  - `Allow network`（checkbox，附说明"Applies to read-only and workspace-write"）；
  - `Escalation requests`（select：Ask / Never）；
  - `Additional writable roots`（textarea，一行一个绝对路径，支持 `~`）；
  - 平台能力提示条：macOS `Process sandbox: seatbelt`；Linux `Process sandbox: bubblewrap` 或 `Unavailable: install bubblewrap`；Windows `Process sandbox: file tools only`.
  - C 批追加只读状态：`Rules: N files loaded` + `Open folder`（目录 `<user_data_root>/rules`）；不在网页里编辑规则。
- `assets/settings.js`：`TAB_KEYS` 加 `sandbox`，`showTab` 懒加载，`sandboxBaseline/sandboxDirty/resetSandbox/saveSandbox`，注册 `createActionBar('actions-sandbox', ...)`。
- 新增文件若拆分 JS，必须同步 `src/settings/service/page.rs::asset` 与 `tests/web_assets.rs::SCRIPTS`。

## 11. 数据库迁移

实施时以 `src/db/mod.rs::SCHEMA_VERSION` 实际值为准；若已被其他分支占用则顺延（当前 29 → 预计 30）。

```sql
ALTER TABLE agent_session ADD COLUMN sandbox_mode TEXT;

CREATE TABLE IF NOT EXISTS agent_session_sandbox_rule (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('exact', 'prefix')),
    prefix_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, kind, prefix_json)
);
CREATE INDEX IF NOT EXISTS idx_agent_session_sandbox_rule_session
    ON agent_session_sandbox_rule(session_id);
```

- `sandbox_mode` 允许 `NULL`；写入前校验枚举字符串。
- `kind = exact` 由 on-failure 重试的 `Always retry this command` 写入；`kind = prefix` 由模型 `prefix_rule` 的 `Approve for session` 写入（C 批后模型提议优先写规则文件，DB 前缀规则仅作会话缓存）。
- fork 的会话复制 SQL 增加该列；规则表不复制。
- 删除会话时规则随外键级联删除。
- 旧库升级：`PRAGMA user_version` 顺延；迁移单测覆盖旧版本、空表与重复规则约束。

## 12. 提示词与 UI

### 12.1 环境快照

`src/chat/prompt.rs::EnvironmentSnapshot::render` 把现有 `<filesystem>file tools use UTF-8 text; shell commands are not sandboxed</filesystem>` 替换为：

```xml
<filesystem>file tools use UTF-8 text</filesystem>
<sandbox mode="workspace-write" platform="seatbelt" network="blocked">
  <writable_root>/path/to/workspace</writable_root>
  <writable_root>/private/tmp</writable_root>
  <protected>/path/to/workspace/.git</protected>
</sandbox>
```

- `platform`：`seatbelt` / `bubblewrap` / `file-tools-only` / `unavailable`；
- 根列表最多 8 条，超出 `<truncated omitted="N"/>`；整体仍受 `MAX_ENVIRONMENT_BYTES` 限制；
- `danger-full-access` 显示 `mode="danger-full-access" network="unrestricted"`（无根列表）；
- 文本必须英文，字段转义复用 `escape_field`。

### 12.2 系统提示

- `LX_SYSTEM_PROMPT` 新增 `## Sandbox` 段（英文），说明：
  - 当前模式与文件工具/命令的实际边界；
  - 被沙箱拒绝时先换做法（写可写根、避免联网），必要时用 `sandbox_permissions: "require_escalated"` + `justification` 申请，`prefix_rule` 只在需要免重复审批时给出；
  - 被沙箱拒绝后 lx 可能自动弹出重试审批；用户拒绝时模型会看到原始失败输出，应如实报告而不是反复重试；
  - `with_additional_permissions` 申请的 read/write/deny/network 只对本次命令生效；
  - `rules/*.rules` 中用户配置的 `allow` 规则会让匹配命令直接跳过沙箱，模型不应假设所有命令都被沙箱约束；
  - `escalation = never` 时不要申请，直接报告边界；
  - 用户 `!` 命令不受沙箱约束（模型不应假设用户命令也在沙箱内）。
- `exec_command` 描述：把 "runs directly without an approval prompt" 改为 "runs inside the session sandbox; escalation requires user approval"（按 `escalation` 动态）。
- `PROMPT_VERSION` 从 `lx-14` 递增到 `lx-15`，跑提示预算与分段测试。

### 12.3 工具 schema

- `exec_command_parameters` 增加：
  - `sandbox_permissions`: `"use_default" | "with_additional_permissions" | "require_escalated"`（默认 `use_default`）；
  - `justification`: string，后两者必填；
  - `prefix_rule`: string array，可选，仅 `require_escalated` 时有效；
  - `additional_permissions`: object，仅 `with_additional_permissions` 时有效：
    ```json
    {"network": {"enabled": true}, "file_system": {"read": ["/abs/dir"], "write": ["/abs/out"], "deny": ["/abs/**/.env"]}}
    ```
    A 批只支持 `network.enabled` 与 `write`；B 批按 §17 支持 `read` / `write` / `deny` 与 glob。
- `escalation = never` 时从 `exec_command` schema 中移除上述属性（通过 `builtin_tools` 生成 `ToolDecl` 后按策略裁剪，避免改 18 个静态 `ToolSpec`）。
- 参数解析在 `src/chat/tools/process_args.rs` 扩展，严格 `reject_unknown`，校验失败返回 `invalid_arguments`。

### 12.4 审批 strip

- 新状态 `Lx.approval: Option<ApprovalState>`（FIFO 队列 + 当前项），布局在 steer strip 之上、输入框之下（固定 2~3 行）。
- 两种形态：
  - 显式申请：`Sandbox escalation  <justification>` + 截断命令 + `y Approve once / a Approve for session (prefix) / n Deny`；
  - on-failure 重试：`Sandbox blocked command` + 被拒绝的输出摘录 + `r Retry once / a Always retry this command (session) / n Keep blocked`。
- `Esc = Deny`；等待期间 `chat_bar` 不提交新消息；编辑仍允许；`Ctrl+C` 取消本轮（拒绝当前审批）。
- 审批结果写入工具结果文本（`approved once` / `approved for session: git push` / `retried without sandbox` / `denied by user`），随工具卡持久化，不新增 part 类型。


## 13. 失败语义

### 13.1 错误码（`ToolErrorCode`）

新增两个稳定码（`as_str` 即模型可见名）：

| 码 | 触发 |
|---|---|
| `sandbox_denied` | 文件工具被模式拒绝；用户拒绝/取消提权 |
| `sandbox_unavailable` | 平台沙箱不可用（`sandbox-exec` 缺失、bwrap 缺失、profile 生成失败） |

shell 内部被沙箱拒绝**不新增错误码**：命令以真实退出码结束，stderr（`Operation not permitted`）原样回给模型；提示词教模型"这是沙箱边界，可申请提权或换做法"。

### 13.2 模型可见文本（英文）

- 文件工具：`Tool failed [sandbox_denied]: sandbox mode "read-only" blocks file writes; switch mode with /permission or ask the user to change it.`
- 越界写：`Tool failed [sandbox_denied]: path "/x" is outside the writable roots of the current sandbox mode.`
- 平台不可用：`Tool failed [sandbox_unavailable]: bubblewrap is required for sandboxed commands on Linux; install it or switch to danger-full-access.`
- 审批拒绝：`Tool failed [sandbox_denied]: the user denied sandbox escalation.`（不带 `justification` 回显，避免二次注入）。

### 13.3 用户可见文案

统一放 `src/ui/text.rs`；toast 走 lx 窗格内 toast；审批结果只写入工具结果文本（`approved once` / `approved for session: <prefix>` / `denied by user`），随工具卡持久化，不新增 notice/part 类型。

## 14. 测试与验证

### 14.1 单元测试

- `config::sandbox`：默认值、缺失文件、损坏文件、未知键、往返序列化、0600、非法路径。
- `platform::sandbox::roots`：cwd/TMPDIR//tmp/额外根去重、`.git` 保护生成、`.git` 文件形态。
- `seatbelt` profile：文本快照（含/不含网络、含/不含写规则）、`-D` 参数、路径含空格、拒绝控制字符。
- `bwrap` 参数：只读/可写/断网/保护路径快照。
- `process_args`：新参数解析、`escalation=never` 时 schema 不含字段、`require_escalated` 缺 `justification` 报错、`prefix_rule` 校验。
- 前缀匹配：`shlex` token 前缀、大小写、空规则、危险前缀拒绝（`sh -c` 等）。
- 文件工具闸门：工作区内写、绝对路径越界、`..`、符号链接逃逸、`.git` 写入拒绝、danger 全通。
- 审批状态机：once/session/deny、Esc=Deny、并发审批 FIFO、子 agent 审批排队。
- `/permission`：候选、会话覆盖落库、`/new` 重置、fork 行为。
- A 批拒绝识别：关键词命中、快速失败码排除、Linux `128+SIGSYS`、停止/超时不误判、每次调用只重试一次。
- A 批重试：Once/Session/Deny/Cancelled、精确命令规则命中、重试输出附加说明、原失败结果在拒绝时原样返回。
- B 批权限档案：条目具体度、`Deny > Write > Read`、glob 匹配（`*`/`**`/`[...]`/`{a,b}`）、`additional_permissions` 合并、seatbelt regex 转义、bwrap 扫描与 carveout、越界仍被拒。
- C 批规则：`prefix_rule` 解析、`match`/`not_match` 自检、最严格决策、规则文件热加载、非法文件跳过、`allow` 保守分段、模型提议落盘与 banned prefix 拒绝。

### 14.2 macOS 端到端（真实 `sandbox-exec`）

临时 workspace 内用 `ProcessManager` 跑：

1. `workspace-write`：workspace 内 `touch` 成功；`$HOME` 下写失败；`net` 请求失败（`curl` 解析失败）。
2. `read-only`：workspace 内写失败；读成功。
3. `network_access=true`：`curl` 成功（允许外网；测试环境允许时）。
4. `danger-full-access`：全部成功。
5. 审批：mock 模型产生 `require_escalated`，批准后越界写成功，拒绝后失败。
6. PTY（`tty:true`）同样受限。
7. 停止语义：沙箱内长命令 Esc 后进程组无残留。

### 14.3 Linux 云主机（8.138.213.110）

安装 `bubblewrap` 后构建并执行同一组用例，记录 `bwrap` 版本、内核版本、`user.max_user_namespaces`、每条断言结果到 `verification.md`。若无法安装/无法建 userns，记录为未验证并保留 fail-closed 行为。

### 14.4 回归

`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响模块 `cargo test`；重点回归 `chat::process`（停止/超时/输出上限）、`event::chat::user_shell`（不套沙箱）、`settings`（新路由/资产）、`db::migration`。

## 15. 风险与未验证项

| 风险 | 说明 | 缓解 |
|---|---|---|
| 全盘可读 | `read-only`/`workspace-write` 允许 `file-read*`（对齐 Codex）；模型可 `cat ~/.ssh/id_rsa`，也可通过 shell 或 `read` 工具读取 lx 自己的 `~/.lx-t[-dev]/config/{ai.json,connect.json}`（含 provider API Key / OAuth 凭据） | 已确认按 Codex 对齐、不做读取限制（决策 #17）；默认断网阻断外泄；设置页与文档标注；后续如需要再引入 deny-read |
| `sandbox-exec` 废弃 | macOS 已标记 deprecated，未来可能移除 | 能力探测 + fail-closed；预留 bwrap/其他后端接口 |
| Linux userns 限制 | Ubuntu 24.04 AppArmor 默认限制非特权 userns，bwrap 可能失败 | 启动探测 + 明确错误；文档给出 `sysctl` 指引 |
| 前缀规则过宽 | 模型可提议 `["git"]` 等宽前缀，批准后覆盖大量命令 | UI 完整展示前缀；拒绝以 shell 解释器/`-c` 结尾的规则；会话级、不跨会话 |
| 命令分词与实际语义差异 | `shlex` 分词 ≠ shell 真实解析（重定向、管道、子 shell） | 前缀只用于"放宽审批"，不用于收紧；拒绝包含 `|;&$\`(){}<>` 的 token |
| 子 agent 并发审批 | 多个子 agent 同时申请 | FIFO 队列串行审批；Esc 只拒绝队首 |
| Windows 无进程沙箱 | shell 直通，可能误以为已隔离 | UI/提示词显式 `file tools only`；文档标记后续 |
| Linux 未实机验证 | 若云主机不允许装包 | 文档记录未验证项，不宣称已验证 |
| 读工具绕过写闸门 | 无（读不构成写）；但 `edit` 的写路径必须全部经过同一判定 | 单一 `sandbox_gate::ensure_writable` 入口，禁止工具自写判定 |
| 拒绝识别误报 | on-failure 用关键词启发式（Codex 同款），`permission denied` 可能来自业务命令而非沙箱 | 仅非零退出 + 排除快速失败码 + 每次调用最多重试一次；拒绝时原结果原样返回 |
| 规则 `allow` 跳过沙箱 | 用户或模型提议的 `allow` 前缀等于永久免沙箱；规则文件是高权限配置 | 模型提议必须经用户审批 + banned prefix；设置页明确警示；会话级规则仍受 `/permission` 清空 |
| 保守分段导致 allow 失效 | 含 `&&`/`|`/重定向的命令不享受 `allow` 绕过 | 明确文档与提示词；安全侧降级而非放宽 |
| glob 扫描成本 | `deny` glob 在 bwrap 下需要遍历文件系统（深度上限 16） | 深度上限 + 只扫描可写根；命中失败告警 |
| Starlark 依赖 | `starlark 0.14` 增加编译时间与二进制体积 | 仅在 C 批引入；`default-features=false`；如不可接受可回退为自研最小解析器（需重新评审） |
| 模型请求 `deny` 条目 | 模型可主动缩小自身权限（无害但可能让命令失败） | 不接受为提权理由；失败按普通命令失败返回 |

## 16. 实现落点

| 层 | 文件 | 改动 |
|---|---|---|
| 配置 | 新建 `src/config/sandbox.rs`；`src/config/mod.rs` 加 `pub mod sandbox;` | 类型、默认、校验、`config/sandbox.json` 读写 |
| 策略运行时 | 新建 `src/chat/sandbox.rs` | `SandboxPlan`（turn 冻结：mode/roots/network/platform）、`SandboxOverride`、根解析、规则匹配 |
| 平台沙箱 | 新建 `src/platform/sandbox/{mod.rs,roots.rs,seatbelt.rs,bwrap.rs,windows.rs}` | 能力探测、命令包装、profile/参数生成 |
| 策略资源 | 新建 `assets/sandbox/seatbelt_base_policy.sbpl`、`assets/sandbox/seatbelt_network_policy.sbpl` | 从 Codex 复制（保留来源注释） |
| 进程 | `src/chat/process/manager/mod.rs`、`spawn.rs`、`src/event/chat/user_shell.rs` | `exec_observed` 增加 `&SandboxPlan`；pipe/PTY 统一包装；用户 shell 传 `SandboxPlan::none()` |
| 工具参数 | `src/chat/tools/process_args.rs`、`src/chat/tools/schema.rs` | `ExecRequest`/`SandboxEscalation` 解析；schema 动态裁剪 |
| 文件闸门 | 新建 `src/chat/tools/sandbox_gate.rs`；`src/chat/tools/{write,edit,patch}.rs` | 写前统一判定；`ToolContext` 增加 `SandboxPolicyView` |
| 执行器 | `src/chat/agent_loop/executor.rs` | `AgentTools` 增加策略字段；`execute_process` 传 plan/override；`execute_file` 注入 view |
| 提权 | `src/chat/agent_loop/mod.rs`、`src/chat/agent_loop/host.rs` 或 `mod.rs` 中的 `AgentHost` | `parse_exec_request`、`approve_sandbox`、决策分支 |
| 事件 | `src/event/chat/host.rs`、`src/event/chat/mod.rs`、新建 `src/event/chat/approval.rs` | Host 请求/事件、FIFO 队列、决策回执 |
| 状态/UI | `src/app/state/lx.rs`（或 `lx/approval.rs`）、`src/app/update/pane.rs`、新建 `src/ui/main_content/lx/approval.rs`、`src/ui/main_content/lx.rs` | 审批状态、优先级路由、strip 布局与渲染 |
| Slash | `src/app/state/lx/slash.rs`、`src/app/update/chat/slash.rs`、`src/ui/main_content/lx/slash.rs` | `/permission` 命令与选择面板 |
| 提示词 | `src/chat/prompt.rs` | 环境快照 `<sandbox>`、`## Sandbox` 段、`PROMPT_VERSION` → `lx-15` |
| 设置服务 | 新建 `src/settings/service/sandbox.rs`；`src/settings/service/handlers.rs`、`src/settings/service.rs`、`src/settings/service/page.rs` | `/api/sandbox`、`SettingsEvent::SandboxChanged`（新变体） |
| 设置前端 | `assets/web.html`、`assets/settings.js`、`assets/settings.css` | Sandbox tab 与表单 |
| 数据库 | `src/db/migration/schema.rs`、`src/db/migration.rs`、`src/db/mod.rs`、`src/db/chat/mod.rs`、`src/db/chat/fork.rs` | v30 迁移与会话模式/规则 API |
| 文案 | `src/ui/text.rs` | 审批与 `/permission` 英文文案 |
| A 批：拒绝识别 | 新建 `src/chat/sandbox/denial.rs`；`src/chat/process/mod.rs` | `is_likely_sandbox_denied`、`ProcessResult.sandbox_retry` |
| A 批：重试 | `src/chat/agent_loop/mod.rs`、`executor.rs`、`src/event/chat/approval.rs` | `retry_process`、`Retry` 审批、精确命令规则 |
| B 批：权限档案 | 新建 `src/chat/sandbox/permissions.rs`；`src/chat/sandbox.rs`、`src/platform/sandbox/{seatbelt,bwrap}.rs`、`src/chat/tools/sandbox_gate.rs` | 条目模型、解析与投影、glob |
| B 批：schema | `src/chat/tools/schema.rs`、`process_args.rs` | `additional_permissions.{read,write,deny,network}` |
| C 批：规则引擎 | 新建 `src/chat/exec_policy/*`；`Cargo.toml` | Starlark 解析、规则匹配、热加载 |
| C 批：接入 | `src/chat/agent_loop/mod.rs`、`src/settings/service/sandbox.rs`、`src/config/sandbox.rs` | 决策顺序、`rules/*.rules` 目录与状态 |
| 测试 | 各模块 `tests.rs` / `tests/` | 见 §14 |

## 17. 完整权限档案（B 批）

### 17.1 数据模型（`src/chat/sandbox/permissions.rs`）

```rust
pub enum FileSystemAccessMode { Read, Write, Deny }

pub enum FileSystemPath {
    Path(PathBuf),   // 绝对路径，可指向目录或文件
    Glob(String),    // git 风格 glob（`*` `?` `**` `[...]` `{a,b}`）
}

pub struct FileSystemEntry { pub path: FileSystemPath, pub access: FileSystemAccessMode }

pub struct FileSystemPolicy {
    pub entries: Vec<FileSystemEntry>,
    pub glob_scan_max_depth: usize,   // 默认 16，对齐 Codex
}

pub struct AdditionalPermissions {
    pub network: Option<bool>,
    pub file_system: Option<FileSystemRequest>,
}

pub struct FileSystemRequest {
    pub read: Vec<String>,    // 绝对路径或 glob
    pub write: Vec<String>,
    pub deny: Vec<String>,
}
```

### 17.2 基准条目

| 模式 | 条目 |
|---|---|
| `read-only` | `(/, Read)` |
| `workspace-write` | `(/, Read)` + `(cwd, Write)` + `($TMPDIR, Write)` + `(/tmp, Write)` + 额外可写根 `Write` + 每个根的 `.git`/`.agents`/`.aws` 为 `Read`（保护写但允许读） |
| `danger-full-access` | 不生成沙箱；文件工具不受限 |
| `additional_permissions` | 追加 `read`→`Read`、`write`→`Write`、`deny`→`Deny`；仅对本次命令生效 |

### 17.3 解析规则（对齐 Codex `permissions.rs` 的优先级）

对任意绝对路径 `p`，计算有效权限：

1. 收集所有匹配条目：`Path` 条目在 `p` 以该路径为前缀（按组件边界）时匹配；`Glob` 条目用 `globset`（`literal_separator(true)`）匹配。
2. **具体度**：字面路径前缀越长越具体；`Glob` 按固定前缀长度计；同具体度按 `Deny > Write > Read`。
3. 结果：`Deny` → 不可读写；`Write` → 可读可写；`Read` → 只读；无匹配 → 继承基准（基准已含 `/ Read`，因此默认只读）。
4. 写根（用于沙箱投影）= 有效权限为 `Write` 且不是 glob 的路径；`Read` 条目作为写根内的只读 carveout；`Deny` 条目作为读写双重拒绝。

### 17.4 工具 schema 扩展

```json
"additional_permissions": {
  "network": {"enabled": true},
  "file_system": {
    "read":  ["/abs/dir", "/abs/**/*.md"],
    "write": ["/abs/out"],
    "deny":  ["/abs/secret", "/abs/**/.env"]
  }
}
```

- 每个列表最多 16 条，条目总长度 ≤ 4096 字节；必须是绝对路径或绝对路径开头的 glob。
- `~` 不允许出现在工具参数里（避免与真实用户路径混淆）；配置页的额外可写根仍支持 `~`。
- `deny` 只能减少权限，不触发额外审批；`read`/`write`/`network` 扩大权限，必须审批。
- 无 `file_system` / `network` 字段时等同于 `use_default`。

### 17.5 后端投影

**Seatbelt**：

- 读：`/` 为 `Read` 且无 `Deny` 读条目 → `(allow file-read*)`；`Deny` 字面路径 → `(deny file-read* (subpath/literal ...))`；`Deny` glob → 移植 Codex 的 `glob_to_regex`（`seatbelt.rs:683-791`，支持 `*`/`?`/`**/`/字符类/花括号）生成正则，追加 `(deny file-read* (regex #"..."))`；
- 写：每个写根一条允许规则，排除该根下的 `Read`/`Deny` 子路径与 base 保护路径；`Deny` glob 同样生成写拒绝；
- 祖先保护：`Deny` 目录与其 `Read` carveout 的祖先追加 `deny file-write-unlink`，防止重命名绕过；
- 正则转义：生成的 regex 需转义 `"` 与 SBPL 原始字符串分隔符，拼接前统一走 `sbpl_regex_literal()`；不使用 `globset::Glob::regex()`（其输出可能包含 SBPL 不支持的 flags），bwrap 侧才用 `globset` 做本地匹配。

**Bubblewrap**：

- `--ro-bind / /` 保持全盘只读；
- 写根 `--bind <root> <root>`（浅→深）；
- `Read` carveout `--ro-bind <p> <p>`；
- `Deny` 字面目录：`--perms 000 --tmpfs <p>`（需要保留子路径可达时用 `000`/`111` 的 Codex 规则）；文件：`--ro-bind-data /dev/null <p>`；
- `Deny` glob：按 `glob_scan_max_depth` 扫描匹配项，逐个应用上述规则；无匹配则跳过并记日志；
- 跨可写符号链接的 deny 目标 → 拒绝并报错（TOCTOU，对齐 Codex `bwrap.rs:683-697`）。

### 17.6 文件工具闸门

- `sandbox_gate` 从"根列表"升级为"条目解析器"：`ensure_writable` 用 §17.3 计算目标有效权限，仅 `Write` 通过。
- 基准条目由 turn 冻结；`additional_permissions` 只属于单次命令，不影响文件工具（文件工具没有"本次命令"概念）。
- `read`/`search` 不因基准 `Read` 受限；若未来配置引入 `Deny` 读条目，再在同一入口加读判定（本次不做）。

## 18. Starlark execpolicy（C 批）

### 18.1 依赖与模块

- 新增依赖：`starlark = { version = "0.14", default-features = false }`；`shlex = "1"`；`globset` 已在依赖树（`ignore` 间接引入），显式声明。
- 模块：`src/chat/exec_policy/{mod.rs,decision.rs,error.rs,rule.rs,parser.rs,policy.rs}`，单文件 ≤1000 行；移植 Codex `codex-rs/execpolicy` 的 `rule/policy/parser/decision`，去掉 CLI、`amend`、`sandbox_migration`；保留 Apache-2.0 来源注释与许可证说明。
- 规则目录：`<user_data_root>/rules/*.rules`（`0700` 目录，`0600` 文件）；按文件名排序加载；语法错误只记录 `tracing::warn` 并跳过该文件，不阻塞启动。

### 18.2 语法（Codex 兼容）

```python
prefix_rule(
    pattern = ["git", "push"],
    decision = "prompt",              # allow | prompt | forbidden，默认 allow
    match = [["git", "push", "origin", "main"]],
    not_match = [["git", "push", "--force"]],
    justification = "pushing to a remote requires approval",
)
```

- `pattern` 支持 `PatternToken::Alts`（`["git", ("push", "pull")]` 形式按 Codex 解析器）；
- `match` / `not_match` 用于加载期自检，违反即报错；
- 空 `justification` 拒绝；
- 规则按首 token 建索引，匹配时取**最严格**决策：`Allow < Prompt < PromptUser < Forbidden`（对齐 Codex `Decision` 顺序）。

### 18.3 接入执行链

在 `exec_command` 解析参数之后、决定沙箱策略之前：

```text
tokens = shlex::split(cmd)
segments = split_shell_segments(cmd)     // 保守解析；含 && || ; | & $( ` > < 时返回 None
decision = policy.evaluate(tokens, segments)
├─ Forbidden → ToolFailure(sandbox_denied, rule.justification)
├─ Allow（且 segments 全部命中 allow）→ 直接裸跑，不弹审批
├─ Prompt/PromptUser → 审批 strip：Run once / Always allow this prefix / Deny
│    └─ Always → append prefix_rule(decision="allow", justification=...) 到 default.rules + reload
└─ 无匹配 → 按 sandbox_mode 跑；失败后走 §8.5 on-failure
```

- **保守解析**：`split_shell_segments` 只在命令不含 shell 元字符时返回单段；含元字符时 `Allow` 不生效（仍可命中 `Forbidden`/`Prompt` 的整串匹配）。这是对 Codex bash 解析器的安全降级，写入文档与提示词。
- `escalation = never`：`Prompt` 视为 `Forbidden`；`Allow` 仍可绕过沙箱（规则是用户显式写的）。
- `allow` 的语义是"跳过沙箱"，与 Codex 一致；规则文件因此属于高权限配置，设置页需要明确提示。
- 规则热加载：turn 起点比较 `rules/*.rules` 的 mtime + 文件集合；变化则重新解析并替换 `Arc<Policy>`。
- 模型 `prefix_rule` 落盘：仅 `require_escalated` 且用户选择 `Always` 时；先过 `BANNED_PREFIX_SUGGESTIONS` 与元字符校验，再追加到 `default.rules`，同一进程内用 `Mutex` 串行化。
- 会话精确命令规则（§8.5）仍存 DB，不写入规则文件；两者匹配顺序：规则文件 → 会话精确规则 → on-failure 审批。

### 18.4 决策优先级（必须唯一）

同一次 `exec_command` 依次判定，先命中者生效：

1. **规则文件 `Forbidden`** → 直接拒绝（`sandbox_denied`，带规则 justification）。
2. **规则文件 `Allow`**（且命令可安全分段、所有段命中 allow）→ 不套沙箱、不弹审批。
3. **会话精确命令规则**（on-failure 的 `Always retry this command`）→ 不套沙箱、不弹审批。
4. **模型显式 `require_escalated` / `with_additional_permissions`** → 审批（§8.2）。
5. **规则文件 `Prompt` / `PromptUser`** → 审批 strip；`Always` 写入规则文件。
6. **默认沙箱执行** → 失败后走 on-failure 重试审批（§8.5）。

说明：

- 规则文件优先于模型请求：用户写下的 `Forbidden` 不能被模型用 `require_escalated` 覆盖；用户写下的 `Allow` 也不需要模型再申请。
- 会话精确规则只对同一命令生效，且不能覆盖 `Forbidden` / 命中 `Allow` 的情况（前两步先命中）。
- `escalation = never` 时第 4、5 步的审批降级为拒绝，第 2、3 步仍生效（均为用户显式授权）。
- 第 2 步的"安全分段"限制（含 `&&`/`|`/重定向等时不生效）优先于便利性，避免 allow 前缀被复合命令利用。

### 18.5 与设置页/提示词

- 设置页新增只读提示：`Rules: ~/.lx-t/rules/*.rules (N files loaded)` + `Open folder` 按钮（复用 `logs/open` 模式）。
- 提示词说明规则存在、`allow` 会跳过沙箱、语法错误不会阻塞启动。
- 规则文件不提供 UI 编辑器（v1），避免在浏览器里编辑安全策略。

## 19. 剩余差异与不做的部分

| 项 | 说明 |
|---|---|
| `PathUri` / `FileSystemSpecialPath` | lx 只处理本地绝对路径与 glob；`:minimal`/`:workspace_roots` 等特殊 token 不支持 |
| app-server / exec-server | lx 没有远程执行器；沙箱只作用于本机 `ProcessManager` |
| 网络代理 / managed network | 不做 `http`/`socks` 代理路由与 MITM；`network_access` 是布尔开关 |
| `request_permissions` 工具 | 不做独立提权工具；只通过 `exec_command` 参数与 on-failure |
| MCP / skill 审批 | 不纳入沙箱审批体系 |
| Windows 进程沙箱 | 后续实现（§7） |
| 跨会话/跨工作区审批 | 不做；规则文件是唯一跨会话通道，且由用户显式维护 |
| Codex `AskForApproval::Granular` | lx 只有 `ask`/`never`；规则文件承担细粒度 |
| `canonical_matching` 的 Unicode 折叠 | v1 按原样字节比较，避免大小写绕过与跨平台差异 |
