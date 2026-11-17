# 沙箱与权限模式验证记录

日期：2026-11-17
分支：`feature/sandbox-permissions`（worktree `.worktrees/sandbox-permissions`）

## 已执行

| 检查 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo clippy --lib --all-targets -- -D warnings` | 通过 |
| `cargo test` | 全部通过（5786 lib 测试 + 集成测试；0 失败） |
| macOS seatbelt 真实写入阻断 | 通过：工作区内写成功、`$HOME`/根外写失败、退出码非 0 |
| macOS seatbelt 保护 `.git` | 通过：`.git/config` 不可写，原内容保留 |
| macOS seatbelt read-only | 通过：工作区内写也被拒绝 |
| macOS seatbelt PTY | 通过：`tty: true` 同样被阻断 |
| 文件工具闸门 | 通过：read-only 拒绝、工作区内新建允许、`..`/符号链接越界拒绝、`.git` 保护 |
| B 批：Deny 字面路径 | 通过：seatbelt 内读、写均被拒绝，兄弟路径不受影响 |
| B 批：Deny glob | 通过：`**/.env` 在多层目录内同时阻断读与写 |
| B 批：Read carveout | 通过：可读但不可写 |
| B 批：解析器 | 通过：上下文长度、`Deny` 绝对优先、glob 仅用于 deny、`read`/`write` 拒绝 glob、条目上限 |
| C 批：规则解析/匹配 | 通过：`allow` / `prompt` / `forbidden`、备选 token、最严格决策 |
| C 批：`match` / `not_match` | 通过：加载期自检，违反即拒绝该文件 |
| C 批：保守分段 | 通过：含 `&&`/`|`/重定向/`$`/反引号时 `allow` 不生效，`forbidden` 仍生效 |
| C 批：决策优先级 | 通过：`forbidden → allow → 会话精确规则 → 模型提权 → 规则 prompt → 默认沙箱` |
| C 批：热加载与落盘 | 通过：文件集合/mtime 变化重载；`default.rules` 追加 0600 且立即生效；非法文件跳过 |
| C 批：设置页 | 通过：`/api/sandbox` 返回规则目录与加载计数；`Open folder` 复用 reveal |

本机环境：macOS 26.2 arm64，`/usr/bin/sandbox-exec` 可用。

## 未验证（需要用户或后续环境）

- TUI 交互：审批 strip 的 `y/a/n`、`r/a/n`、`Esc`、窄窗格换行；`/permission` 面板视觉效果。测试覆盖了状态与键位，但未在真实终端中人工确认。
- 设置页的 `Open folder` 实际弹出 Finder；只验证了路由与资源契约。
- Linux bubblewrap：只做了参数构造单测（Linux 条件编译），未在云主机 `8.138.213.110` 实测。已知限制：`--bind` 要求写根事先存在（不存在的额外写根会被跳过并告警）；deny glob 在启动时扫描现有匹配项，之后新建的匹配文件不会被回溯遮盖。
- Windows：仅有文件工具闸门；无进程沙箱（设计决策）。
- 子 agent 的审批尚未转发到父窗格：当前子 agent 的 `ApproveSandbox` 返回稳定失败，不阻塞主 agent；与设计 §8.3 的 FIFO 队列仍有差距。
- B/C 的真机端到端（模型真实调用 `additional_permissions`、真实规则文件影响下一轮命令）未跑；组件级与决策级测试已覆盖。
- 规则 `prompt` 的 `Always` 会把前缀写入 `default.rules`（全局生效）；用户需知悉这是跨会话的高权限配置。

## 合并注意

- 分支基于 `dev@41d5977b`；`dev` 之后已合入 stop/report 分支（含独立的 v30 迁移）。
- 本分支的沙箱迁移已顺延为 **v31**，`PROMPT_VERSION` 顺延为 **`lx-17`**；合并时保留 dev 的 v30 `agent_compact_checkpoint.kind`，再接 v31。
- `git merge-tree --write-tree dev HEAD` 预演冲突：`src/db/migration.rs`（v30/v31 顺序）、`src/chat/prompt.rs`（提示词版本与新增段）、`src/app/state.rs`（导出列表）。三处均为可并集解决，未自动修改 Git 状态。

## 回滚

所有改动在独立 worktree 与分支；未修改主 checkout 的 Git 状态。数据库迁移 v31 只增列/表，可通过 `git` 回退二进制后继续使用旧库（旧库不会自动降级，`sandbox_mode` 列与规则表保留但不被旧版本读取）。规则文件是用户数据，不在二进制回滚范围内。
