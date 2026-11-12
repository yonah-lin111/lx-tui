# `/btw` 验证报告

- 分支：`feat/lx-btw`
- 日期：2026-11-12
- 范围：`task.md` §11～§17 已落地的 BTW 功能；不含尚未实现的 `Send to main`、异步图片/剪贴板 owner 校验、完整 MCP 资源桥。
- 方法：自动化回归 + 真实 CLI 新建工作区/会话 + PTY 驱动 TUI 的端到端验证。

## 1. 环境与准备

- 平台：macOS，Rust 工作树 `.worktrees/lx-btw`。
- 自动化：`cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt`。
- E2E：真实模型 `provider-alq374/cbcn/deepseek-v4.1-flash#medium`；独立临时工作区与独立 SQLite 库；TUI 通过 `tests/btw_verify.rs` 在真实 PTY 中驱动。
- CLI 建会话（完整 prompt 见 `tests/btw_verify.rs` 文件头）：

```text
WS=$(mktemp -d /tmp/lx-btw-verify.XXXXXX)
DB=$(mktemp -d /tmp/lx-btw-verify-db.XXXXXX)/lx.db
printf 'BTW_VERIFY_TOKEN=42\n' > "$WS/note.txt"
LX_TUI_DB_PATH="$DB" lx-tui workspace add --trust -C "$WS"
LX_TUI_DB_PATH="$DB" lx-tui exec -C "$WS" "Call spawn_agent with role explorer ... wait_agent ... reply CLI_VERIFY_OK"
LX_TUI_DB_PATH="$DB" lx-tui session rename -C "$WS" --last "BTW verify"
```

`exec` 生成 1 条主会话和 1 个已结束子 agent `Turing [explorer]`；`note.txt` 内容为 `BTW_VERIFY_TOKEN=42`。
PTY 用 `/resume` 打开该会话，再执行以下用例。手工复跑命令：

```text
BTW_VERIFY_WS="$WS" BTW_VERIFY_DB="$DB" \
  cargo test --test btw_verify -- --ignored --nocapture
```

## 2. 验证项

| ID | 验证项 | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- | --- |
| V01 | CLI 新建工作区/会话 | `workspace add --trust` + `exec` 创建子 agent | 执行两条命令；`session list` / `session agents --last` / `session show` 检查 | 工作区已登记且可信；1 条主会话；`Turing [explorer]` 为 completed；会话含 CLI prompt 与工具记录 | 通过；`BTW verify` / `CLI_VERIFY_OK`，会话可见 `read`、`spawn_agent`、`wait_agent` |
| V02 | `/btw` 直接提问 | `/btw Reply with exactly SIDE_OK.` | TUI `/resume` 打开主会话；输入 `/btw`，Tab 采纳，补问题后回车 | 打开 BTW tab，归属 QA 1；返回 `SIDE_OK`；主会话不新增消息 | 通过 |
| V03 | BTW 连续追问 | 首问后发送 `Write 99 ...` | 不重新 `@`，直接在 BTW composer 发送 | 仍在 QA 1 同一线程；沿用上次目标；回答追加 | 通过；DB 第 3 批目标仍为 `subagent` |
| V04 | `@` 已结束 subagent | `What token did you find? ... @Turing` | BTW composer 输入 `@`，右切到 Subagents，输入 `Turing`，回车确认，再回车发送 | 目标胶囊为 `To: Turing [explorer]`；回答包含 `42`；不新建子任务 | 通过；回答 `42`，recipient 为 `Turing/explorer` |
| V05 | 只读边界 | `Write 99 into note.txt. Use the write tool.` | 在 BTW 中发送；等待完成；读取 `note.txt` | 拒绝写入；文件仍为 `BTW_VERIFY_TOKEN=42` | 通过；文件未变 |
| V06 | `/btwList` 面板 | `/btwList` | BTW composer 输入 `/btwList`，Tab 采纳；`Esc` 关闭 | 出现 `Side questions`、`QA 1`、`● side thread`；Esc 可关闭 | 通过 |
| V07 | 线程历史 loading 位置 | 切换到未加载的 QA 线程 | 运行 `cargo test --lib btw_history_loading` | `Loading side conversation…` 只在 tab 与 composer 之间的空行；对话区无居中占位 | 通过 |
| V08 | 回到底部按钮 | BTW 长线程 + 鼠标滚轮上滚 | 在 BTW 对话区滚轮上滚；点击 `↓ jump to bottom` | 按钮出现；点击后回到底部并消失 | 通过（E2E）；自动化 `chat_hit_allows_jump_bottom_in_btw_view` 通过 |
| V09 | 消息列表交互对齐 | 长线程、代码块、折叠卡片 | 运行 `cargo test --lib btw_wheel_scroll`、`btw_scrollbar`、`btw_thread_copy`、`chat_hit_supports_code_copy` | 滚轮 3 行/格；滚动条 thumb 拖拽/轨道点击；正文/代码复制；卡片折叠 | 滚轮/复制/折叠/几何通过；滚动条拖拽与轨道点击走 Agents 共享路径，未单独 PTY 覆盖 |
| V10 | 单 QA 视图与 QA 隔离 | 同一主会话多个 QA；不同 QA 草稿 | 运行 `cargo test --lib btw`；检查 `each_qa_keeps_its_own_draft`、`btw_context_stops_at_the_owning_qa` | 只渲染当前 QA 线程；草稿/目标/滚动不串；不出现 All 视图 | 通过 |
| V11 | BTW 子会话隔离 | `session list`；`session show <btw-child>` | 查询 `agent_session.source='btw'`；执行两个 CLI | 主列表不出现 BTW 子会话；`session show` 拒绝按主会话打开 | 通过；CLI 返回 not found，`source='btw'` 仍保留 |
| V12 | 持久化与重启 | 已完成的 BTW 线程 | 结束 TUI，重新启动并打开同一主会话/`/btw` | 历史恢复；不自动重发请求 | 通过；E2E 多次重启后历史仍在 |
| V13 | DB v28 迁移 | 真实 v26 开发库副本 `lx.db.v26.bak` | 拷贝到临时库；设置 `LX_TUI_DB_PATH` 后运行 `session list`；检查 `user_version`、`integrity_check`、行数、表/触发器 | 升级到 28；完整性 ok；原 63 会话/211 消息保留；`source` 列与 3 张 BTW 表/触发器存在 | 通过 |
| V14 | 级联删除 | 删除 BTW 线程 / 删除 QA anchor | 运行 `cargo test --lib deleting_btw_thread`、`deleting_the_qa_anchor` | child session 被删除；关联批次/recipient 回收；其他 QA 不受影响 | 通过 |
| V15 | 批次/recipient 幂等 | 重复提交、同目标去重、重复结算 | 运行 `cargo test --lib accept_batch_is_idempotent`、`finish_recipient_aggregates` | 一次提交只建一个 batch；同目标只建一个 recipient；重复结算不重复计费 | 通过 |
| V16 | QA 上下文截止 | steer、报告、shell、后续 QA、其他 BTW | 运行 `cargo test --lib latest_qa_anchor`、`btw_context_stops` | 主参考只到所属 QA；排除 steer/报告/shell/后续 QA；不混入其他 BTW | 通过 |
| V17 | 只读工具面 | 伪造写工具/命令 | 运行 `cargo test --lib readonly` 与 `btw_tool_surface_is_read_only` | 仅 `read` / `search` / `curr_time`；写/编辑/命令不在声明面 | 通过 |
| V18 | 启动恢复 | 遗留 running/queued 批次 | 运行 `cargo test --lib startup_recovery` | 标记 interrupted，保留已生成正文，不自动重发 | 通过 |
| V19 | 全量回归 | `cargo test` | 执行 `cargo test`、`clippy`、`fmt` | 全部通过 | 通过；lib 5264 passed，5 ignored；所有集成测试通过 |

## 3. 结果摘要

- 最新复测：`cargo test --lib` **5272 passed / 0 failed / 5 ignored**；BTW 相关过滤 **50 passed**。
- 历史基线：5264 passed / BTW 42 passed（评审修复前）。`tests/btw_verify.rs` 为 ignored 的真实模型 E2E。
- 迁移过滤：24 passed；选区过滤：180 passed；只读过滤：1 passed。
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt`：通过。
- E2E 数据库证据（本轮持久化环境）：
  - 主会话 `sess-18e74140de5035d8-1`（`BTW verify`）：2 条消息；包含 CLI prompt、`read`、`spawn_agent`、`wait_agent` 与 `CLI_VERIFY_OK`。
  - BTW child `sess-18e741745e967f08-0`：6 条消息；普通 `session list` 不显示，`session show` 拒绝。
  - `agent_btw_batch`：`["default"]`、`["sub-…"]`、`["sub-…"]` 三批均 completed。
  - `agent_btw_recipient`：`lx`、`Turing/explorer`、`Turing/explorer`。
  - `note.txt` 最终仍为 `BTW_VERIFY_TOKEN=42`。

## 4. E2E 发现并修复的问题

1. **首次 `/btw prompt` 丢失 prompt**：首次打开时活动线程尚未建立，原实现把 prompt 写入 `active_thread_mut()`，返回 `None` 后只留下 `pending_prompt=true`，列表到达后提交空草稿。
   - 修复：`pending_prompt` 改为 `Option<String>`，暂存正文；QA 列表/线程建立后写入 composer 再提交。
   - 回归：`btw_command_stashes_prompt_until_the_first_thread_is_created`。
2. **无新 `@` 的追问退回 lx**：胶囊仍显示上次目标，但提交时 `resolve_targets` 返回空集合，事件层按默认助手执行。
   - 修复：空集合时从 `BtwTargetView` 回查实例生成 `previous_targets`；实例缺失则拒绝提交并保留草稿，绝不静默换目标。
   - 回归：`btw_follow_up_without_mention_keeps_previous_agent_target`；E2E 第三批 recipient 仍为 `Turing`。

## 5. 本轮持久化复测（可检查）

- 工作区：`/Users/yonah/.lx-t-dev/verify/lx-btw-cli-20261112`（`lx-btw-cli-20261112`，worktree DB 内已 trust）。
- 数据库：`/Users/yonah/.lx-t-dev/db/worktrees/lx-btw-df3a5393/lx.db`。
- 主会话：`sess-18e74140de5035d8-1`，标题 `BTW verify`；子 agent：`sub-18e74141bc725c38-16`（`Turing`，explorer，completed）。
- BTW child：`sess-18e741745e967f08-0`；主会话 2 条消息、BTW child 6 条消息。
- 内容检查（只读）：

```text
DB=/Users/yonah/.lx-t-dev/db/worktrees/lx-btw-df3a5393/lx.db
WS=/Users/yonah/.lx-t-dev/verify/lx-btw-cli-20261112
LX_TUI_DB_PATH="$DB" lx-tui session list -C "$WS"
LX_TUI_DB_PATH="$DB" lx-tui session show -C "$WS" --last --tools
LX_TUI_DB_PATH="$DB" lx-tui session agents -C "$WS" --last
```

`session show` 会显示 CLI prompt、`read` / `spawn_agent` / `wait_agent` 工具记录、`Turing` 报告 `42` 与最终 `CLI_VERIFY_OK`。

- TUI 检查：从本 worktree 启动 `cargo run`，在工作区列表切到 `lx-btw-cli-20261112`，`/resume` 选 `BTW verify`；主会话先显示 CLI prompt 与工具卡片，按 `Esc` 或切到 BTW 可看旁路消息。
- 注意：本 worktree 的 `workspace add --trust` 必须与 `exec`/TUI 使用同一个 `LX_TUI_DB_PATH`；否则 trust 落到主库，worktree 库里的工作区仍是 untrusted，表现为“工作区下没有 session/CLI 信息”。本轮已用 worktree DB 的 `workspace trust` 修正并验证。

## 6. 未覆盖 / 已知限制

- 关闭 BTW 面板后继续后台生成、切主会话再返回的完整手工链路未单独跑 E2E；相关代码路径存在，但本轮没有专门的 PTY 用例。
- BTW 滚动条 thumb 拖拽/轨道点击未单独 PTY 覆盖；几何与滚动 setter 有测试，press/drag 走 Agents 共享路径。
- 迁移实测为真实 **v26 → v28**；未单独构造 v27 库副本。
- `Send to main` 转交、异步图片/剪贴板 owner 校验、完整 MCP 资源桥未实现，不在本次验证范围。
- 真实模型 E2E 依赖本机 provider 与 `sqlite3`，已标记 `#[ignore]`，不进入 CI。
- 视觉样式（配色、窄窗格像素级布局）未做截图比对；交互几何由单元测试覆盖。

## 7. 用户报告：BTW 工具 pending 落库失败

- 现场：`sess-18e73a11a5a0d138-0` 的 QA 3；BTW 线程 `btw-18e73d802af62678-f`，child `sess-18e73d802af60f08-e`；失败批次 `btw-1794470597214-2-1`；assistant `msg-18e73d802b1fa390-13` 状态 failed，错误 `tool call pending record failed · sqlite`。
- 已完成的 7 个工具（2 read / 4 search / 1 read）均有 `agent_tool_call` 行；失败工具没有行，说明 pending INSERT 失败。旧代码的 BTW `begin_tool` 不记录 DB 错误，旧库无法反推具体 SQLite 子错误。
- 代码缺陷：BTW 的 `begin_tool` 缺少主轮/子 agent 都有的 `checkpoint_before_tool` 时间线同步；长工具循环可能让 app 与 DB position 漂移。
- 修复：抽共享 `checkpoint_before_tool`，BTW pending 前同步 parts；BTW 的 checkpoint/pending 错误记录完整 `error`，主轮/子 agent 同步补全。
- 验证：`cargo test --lib` 5264 passed；clippy 通过；用该 session 的数据库副本重放同类 search/read 长链，120s 内未再出现 pending 失败（仍在长任务中，不能证明所有 SQLite 锁场景已消除）。若复现，新日志会显示具体 SQLite 子错误。

> 说明：本报告此前的 `sess-18e74140de5035d8-1` 位于独立工作区 `lx-btw-cli-20261112`，不在当前 `lx-tui` 工作区的 `/resume` 列表中；当前问题使用你指定的 `sess-18e73a11a5a0d138-0`。

## 8. “No matching subagent” 阻止 BTW 追问

- 现象：BTW composer 输入 `继续执行` 后 toast `No matching subagent; start a task from the main agent`，消息未发送。
- 根因：线程上一轮的目标实例（如 `Turing`）仍在 `BtwTargetView`，但重启/会话加载后 `lx.agents.runs` 尚未回填；旧的 `previous_targets` 找不到实例就直接返回错误，导致无 `@` 的追问被卡住。
- 修复：无新 `@` 且上次目标实例缺失时，清空过期目标、提示 `BTW targets reset to lx`，并继续提交给默认助手；显式 `@` 找不到实例仍保留错误，避免误投。
- 数据库处理（仅 worktree 库）：把 `btw-18e741745e9682f0-1` 的 `answerer_set_json` 重置为 `["default"]`，备份为 `lx.db.bak-targetfix-20261112175420`；主库只清理了此前误加的测试 workspace 记录，未改任何主工作区 session/message。
- 回归：`btw_follow_up_missing_previous_target_falls_back_to_default`；`cargo test --lib` 5264 passed，BTW 过滤 42 passed。

## 9. 2026-11-12 CLI 复测（`BTW verify 2`，历史记录）

> 本节是第九轮结果；该 session 已不是 fresh BTW 历史。当前 fresh 全链路复测以 §10 为准。

- 环境：workspace `lx-btw-cli-20261112`；worktree 库 `/Users/yonah/.lx-t-dev/db/worktrees/lx-btw-df3a5393/lx.db`。
- CLI seed：`sess-18e74800241594d0-0`（`BTW verify 2`），subagent `sub-18e74800a9368b60-e`（`Turing` / explorer / completed），BTW 线程 `btw-18e7480653045320-1`。
- 全链路 E2E（fresh session，非旧 session 复用）：
  - `cargo test --test btw_verify btw_chat_and_agent_mention_e2e -- --ignored --nocapture --test-threads=1`
  - 通过；新建 3 个批次：默认 `lx` → `@Turing` → 不再 `@` 的只读追问仍为 `subagent`，3 个批次均 completed。
  - BTW 内实际执行 `read`；`note.txt` 保持 `BTW_VERIFY_TOKEN=42`，未被写入 99；jump-to-bottom 与 `/btwList` 面板通过。
- 缺失目标回退 E2E：
  - 先把线程 `answerer_set_json` 置为不存在的 `sub-deadbeef`，启动 TUI 后在 BTW 发 `继续执行`。
  - 新批次 `btw-1794482298853-2-1` 的 `target_set_json=["default"]`，completed；线程目标落库为 `["default"]`；最终 composer 徽标为 `To: lx`；未出现 `No matching subagent`。
  - 修复了加载/提交竞态：`on_thread_loaded` 在 `submit_seq > 0` 时不再用提交前快照覆盖本地已清除的目标，否则数据库已回退、徽标仍显示旧目标。
- 复测命令（完整链路必须用**没有 BTW 历史**的新 seed session；当前会话只能复跑回退用例）：

```bash
WS=/Users/yonah/.lx-t-dev/verify/lx-btw-cli-20261112
DB=/Users/yonah/.lx-t-dev/db/worktrees/lx-btw-df3a5393/lx.db

# fresh session 的全链路 E2E
BTW_VERIFY_WS="$WS" BTW_VERIFY_DB="$DB" BTW_VERIFY_TITLE='BTW verify 3' \
  cargo test --test btw_verify btw_chat_and_agent_mention_e2e -- --ignored --nocapture --test-threads=1

# 当前 BTW verify 2：缺失目标回退 E2E
BTW_VERIFY_WS="$WS" BTW_VERIFY_DB="$DB" BTW_VERIFY_TITLE='BTW verify 2' \
  cargo test --test btw_verify btw_missing_previous_target_falls_back_e2e -- --ignored --nocapture --test-threads=1
```

- 复测后的会话可直接在 TUI 打开：`BTW verify 2`；线程目标为默认 `lx`，发送 `继续执行` 会走 `lx`，历史里的 `Turing` 批次与只读证据保留。
- 旧会话 `sess-18e74140de5035d8-1`（`BTW verify`）仍保留，目标同样为默认；两套 session 都只存在于 worktree 库，不影响主库会话。


## 10. 评审后的 CLI+PTY 复测（`BTW verify 3`）

- fresh seed：`sess-18e74a2bb0702e18-0`（`BTW verify 3`），subagent `sub-18e74a2c3df86b38-e`（`Turing` / explorer / completed），BTW 线程 `btw-18e74a342652ee60-1`。
- 全链路 `btw_chat_and_agent_mention_e2e`：8.56s 通过；默认 `lx`、`@Turing`、无新 `@` 的只读追问均完成；`read` 只读、`note.txt` 未改、jump-to-bottom、`/btwList` 通过。
- 缺失目标 `btw_missing_previous_target_falls_back_e2e`：7.34s 通过；最终批次 `btw-1794484584713-2-1` 目标 `["default"]` completed，线程目标 `["default"]`，徽标 `To: lx`，无 `No matching subagent`。
- 本轮修复与未完成项详见 [review.md](review.md)；历史会话 `BTW verify`/`BTW verify 2` 仍保留在 worktree 库。
