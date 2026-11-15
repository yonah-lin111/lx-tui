# 验证记录：Agents / BTW 详情输入框 `/` 命令与私聊·汇报重做

- **分支**：`detail-slash-report`（worktree：`.worktrees/detail-slash-report`）
- **基线**：`dev@909c2e48`
- **状态**：自动化验证通过；TUI 交互与外观待用户实机确认。

## 自动化

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings   # 通过
cargo test --lib                            # 5591 passed, 0 failed, 6 ignored
```

按影响的定向回归：

```bash
cargo test --lib app::state::lx::slash::composer
cargo test --lib app::update::chat::tests::detail_commands
cargo test --lib app::update::chat::btw::tests
cargo test --lib event::chat::tests::detail_undo
cargo test --lib event::chat::tests::detail_compact
cargo test --lib event::chat::tests::subagent_private
cargo test --lib db::chat::subagent::tests::undo
cargo test --lib ui::main_content::lx::task::tests::detail_slash
```

新增/更新的覆盖点：

- 详情命令模型：Agents/`/undo /compact /report`，BTW/`/undo /compact /new`；`/report` 多行备注；非带参命令拒绝参数；`/undo` 主输入仍是 `/rewind` 别名。
- composer 面板：`/` 打开、过滤、`↑↓/Tab/Enter/Esc`、粘贴刷新、离开编辑/切 tab 清理；未知命令不发送。
- 私聊/汇报：普通追问与 steer 一律 `notify_parent=false`；`/report` 注入标记指令并 `notify_parent=true`；`visible_task` 不被汇报指令覆盖；先私聊再汇报时报告正文不含闲聊。
- `/undo`：只认人类面板私聊追问（`position>0` + 无 `parent_call_id` + `notify_parent=0`）；spawn / 父 `send_input` / `/report` / steer 段拒绝；两行确认默认 `Cancel`；执行后重载详情消息；BTW 删空后线程重置；迟到 operation id 忽略。
- `/compact`：详情效应只发一次；忙碌拒绝；完成只回填对应 run/thread，迟到回执忽略；BTW 线程加载恢复检查点。
- `/new`：发出 `DeleteThread`；回答中拒绝；删除回投后视图清空、刷新 QA 列表、lx 页内 toast。
- 渲染：胶囊 `private` / `steer N` / `compacting`；BTW 后缀整段丢弃；底栏 `[/ commands]`；`/` 面板贴 composer 上沿。

## 实机清单（待用户 `cargo run`）

1. 打开已完成 subagent（`/subagents`），`Shift+Enter` 进输入框，输入 `/`：只出现 `/undo` `/compact` `/report`。
2. 连续私聊 3 轮：主 agent 无报告、无自动轮。
3. `/report <note>`：子 agent 输出报告，主 agent 只收到该轮正文并自动起轮；闲聊不在报告里。
4. 私聊改文件后 `/undo`：默认 `Cancel`；直接 Enter 无变化；`↓ ↵` 恢复文件并删除最后一轮。
5. 对 spawn 轮 / `/report` 轮 `/undo`：提示 `Nothing to undo`。
6. 私聊多轮后 `/compact`：胶囊显示 `compacting`，完成出现检查点；再次执行提示 `Nothing new to compact.`。
7. BTW：多轮后 `/undo` 删除最后一轮；`/new` 清空线程且列表 `● side thread` 消失；回答中 `/new` 被拒绝。
8. 约 60 列窄窗格：胶囊、确认面板、底栏提示不截半截；文案全英文。

## 已知限制

- 含轮内 steer 的轮次不可 `/undo`：steer 段只覆盖截断之后的消息与文件，按最新段撤销只能回退半个逻辑轮次，因此直接拒绝并提示 `Nothing to undo`。
- 旧库（v29 之前）的追问行 `notify_parent` 默认 1，会被判为不可撤回；安全默认，不回填历史。
- `/report` 会额外消耗一次模型轮次（仅显式命令触发）。
