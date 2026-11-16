# 沙箱与权限模式验证记录

日期：2026-11-17
分支：`feature/sandbox-permissions`（worktree `.worktrees/sandbox-permissions`）

## 已执行

| 检查 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo clippy --lib --all-targets -- -D warnings` | 通过 |
| `cargo test` | 全部通过（5753 lib 测试 + 集成测试；0 失败） |
| macOS seatbelt 真实写入阻断 | 通过：工作区内写成功、`$HOME`/根外写失败、退出码非 0 |
| macOS seatbelt 保护 `.git` | 通过：`.git/config` 不可写，原内容保留 |
| macOS seatbelt read-only | 通过：工作区内写也被拒绝 |
| macOS seatbelt PTY | 通过：`tty: true` 同样被阻断 |
| 文件工具闸门 | 通过：read-only 拒绝、工作区内新建允许、`..`/符号链接越界拒绝、`.git` 保护 |

本机环境：macOS 26.2 arm64，`/usr/bin/sandbox-exec` 可用。

## 未验证（需要用户或后续环境）

- TUI 交互：审批 strip 的 `y/a/n`、`r/a/n`、`Esc`、窄窗格换行；`/permission` 面板视觉效果。测试覆盖了状态与键位，但未在真实终端中人工确认。
- on-failure 重试的完整链路：拒绝识别与重试代码已接入，但缺少一次真实的“命令撞沙箱 → 审批 → 重试”集成测试；现有单测覆盖识别条件与规则匹配。
- Linux bubblewrap：只做了参数构造单测（Linux 条件编译），未在云主机 `8.138.213.110` 实测。
- Windows：仅有文件工具闸门；无进程沙箱（设计决策）。
- 子 agent 的审批尚未转发到父窗格：当前子 agent 的 `ApproveSandbox` 返回稳定失败，不阻塞主 agent；与设计 §8.3 的 FIFO 队列仍有差距。
- B 批（完整 read/write/deny/glob 档案）与 C 批（Starlark 规则文件）未开始；按设计需 A 批验收后再实施。

## 回滚

所有改动在独立 worktree 与分支；未修改主 checkout 的 Git 状态。数据库迁移 v30 只增列/表，可通过 `git` 回退二进制后继续使用旧库（旧库不会自动降级，`sandbox_mode` 列与规则表保留但不被旧版本读取）。
