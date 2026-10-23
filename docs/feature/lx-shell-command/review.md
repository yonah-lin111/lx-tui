# lx 本地命令（`!`）与 rtk 集成评审

状态：**评审完成，缺陷已修复**（2026-10-23）
范围：commit `1a23369` 合入 dev 的 `lx-shell-command` 分支（`e6eca58` 本地命令 + `ad62752` rtk）
方法：codegraph / codebase-memory 结构定位 → 逐条读实现 → 临时 SQLite 与真实进程测试复现；两个只读评审代理分别覆盖 TUI 状态机与运行时/持久化，所有代理结论均经本代理复核后才修复。

本文件只记录**能给出触发场景与根因**的问题，不收录风格意见。修复前的代码行号以合并后 dev 为准。

---

## 1. 关键链路与关键代码

### 1.1 输入模式（纯状态）

`!` / `！` 仅在**整段输入的首个非空白字符**时进入 shell 模式；模式字符被吸收，正文不含前缀。

`src/app/state/lx/shell.rs:19`

```rust
pub fn marker_offset(text: &str) -> Option<usize> {
    text.char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .and_then(|(offset, ch)| matches!(ch, '!' | '！').then_some(offset))
}
```

关键点：

- 键盘插入（`insert_char`/`insert_str`）→ `adopt_shell_prefix`（`src/app/state/lx.rs:235`）在插入后重新判定；粘贴、输入历史召回、队列恢复统一走 `split_mode`/`import_full_text`。
- `revision` 单调递增（`touch()`），undo/redo 恢复 `mode` 但修订号只增不减；spawn 回执靠它决定是否清空草稿。
- shell 模式下 `refresh_panel` 清空 Markdown/提及/斜杠/路径面板，`newline` 插入字面换行，正文起点 Backspace 退出模式。

### 1.2 提交、回执、队列与停止

`src/app/update/chat/user_shell.rs:26` 冻结命令、修订号、会话与 cwd 后压入 `pending_shells` 并发出 `Effect::BeginUserShell`；DB 预登记与 spawn 由事件层回执。`Tab` 忙时入 FIFO，模型或命令终态后 `drain_queue` 逐项分派。

关键点：

- shell **不占用** `RequestState`，模型流与命令并行；所有“忙”判定必须是 `request.is_busy() || has_running_shell()` 的并集。
- `pending_shells` 是“用户已提交、尚未 spawn”的唯一凭据：取消、切会话、会话复位清空它；事件层的 `begin` 必须先校验它。
- spawn 成功后才清草稿（修订号仍匹配）并记 `!命令` 历史。
- 双 Esc 的停止目标是模型轮次 ∪ pending/running 命令；第二次 Esc 同时清空队列。

### 1.3 执行与持久化

`src/event/chat/user_shell.rs:206` 的顺序：DB 预登记 → 插入卡片（早于 spawn）→ 注册 `ShellJob` → `exec_observed(yield_time_ms: 0, timeout: 1h, login: true)`。进程事件按 `origin_call_id`（消息 ID）在 `process_events.rs` 分流，不进入工具卡片路径。

`src/db/chat/user_shell.rs:215` 写入规则：

- `revision` 不得回退；JSON 已是终态时拒绝任何 `starting/running`；
- 消息 `status` 已脱离 `streaming` 时拒绝回退到 `streaming`；
- 终态写 `completed_at_ms` 并推进会话 `updated_at_ms`。

v13 迁移（`src/db/migration.rs`）整表重建 `agent_session` 使模型三列可空，并给 `agent_message` 增加 `kind`/`user_shell_json`；重建在关闭外键的独立事务中完成，结束后做 `foreign_key_check`。v12 的全部建表语句改为**无条件幂等补齐**，兼容合并前 shell 分支遗留的 v12 库。

### 1.4 模型上下文

两条路径共用 `format_user_shell`（`src/chat/context/mod.rs:301`）：

- 历史回放：`plan_turns` 把 `kind == "user_shell"` 的消息作为**独立组**（只有 user，不伪造 assistant 配对）；`shrink_once` 按装配时记录的组边界整组删除。
- 实时注入：`run_agent_turn` 每次采样前调用 `AgentHost::user_shell_updates` → `event::chat::user_shell::poll_updates`：按 DB `sequence` 取连续终态前缀，按消息 ID 去重，单请求上限 `MAX_USER_SHELL_INJECTION_BYTES = 16 KiB`。

装配预算在扣掉输出预留与信封预留后，再扣除与注入上限等量的 4k token；注入与装配的上限同源，避免“历史占满预算 + 运行中命令结果再叠一段”把请求顶出模型上下文。

输出在进入上下文前做 `sanitize_control`（剥离 ANSI/控制字符）+ `&<>` 转义 + 64 KiB 截断，结构标签不会被输出伪造。

### 1.5 rtk

`src/chat/process/rtk/mod.rs:104`：`auto` 且找到 rtk 时把 `cargo test` 这类命令包装成 `rtk <命令>`；否则保留原命令，由 `builtin::filter_output` 在**终态**裁剪。`exec_command` 与 `!` 两条路径都接在同一处，卡片与上下文里的 `command` 始终是原始命令。

包装只发生在“首段是受支持短时命令、且整条命令不含未引号 `|`/`<`/`>`”时：rtk 压的是 stdout，写文件或喂下游必须保留原始输出。可执行文件路径的引号由 `platform::shell_quote_path` 按平台生成。

### 1.6 命令卡片

`src/ui/main_content/lx/history/user_shell.rs` 的 `lines()` 是行数与绘制的唯一来源；`layout` 用 `rows()`、`render` 用 `draw()`，避免滚动条/命中区漂移。shell 卡片在 `history.rs` 中 `is_user = false`，不参与置顶用户行。运行中为橙/呼吸点，失败红，主动停止为警告色，正常结束为绿色。

卡片与工具步骤共用直角树体（`  │  ` / `  └─ `，`TREE_COLS = 5`）：标题是唯一不带连接符的行，终态的退出码与耗时并入标题（`■ You run command · exit 2 · 1.2s`），树体首行是 `$ 命令`，终态输出折叠保留首 3 + 尾 2，末行收口写 `… N more lines`。**参数区与输出区各自独立折叠**（`CardSection::Body` / `CardSection::Output` + 各自 `CardHit`）：参数与输出各有点击面，点其中一块不会连带另一块。**参数区恒为浅黄**（不随 hover/展开变色，`… N more lines` 同色），**输出区**折叠 dim、悬停或展开为终端默认前景（只有真的能展开才响应 hover），不铺整行底色；`└─` 只收口参数区，输出内容只用等宽空白对齐。命令运行中不铺实时输出，只给一行 `open task panel · live output`（点击打开面板并定位该进程）。`exec_command` 卡片逐项同口径。

---

## 2. 缺陷与修复

### P1-1. 本地命令运行或 shell 模式下，双 Esc 被 rewind 路由抢走

**触发场景**（与设计 §8.2 “存在活动任务且没有面板时 Esc 优先走停止待命”冲突）：

1. 输入 `!sleep 60` 按 Enter，spawn 回执清空草稿后按两次 Esc：第一次进入“撤销待命”，第二次选中最新用户消息打开 rewind 面板，命令继续运行。
2. 空 shell 模式（正文为空）且会话里有历史消息时按 Esc：本应退出 shell 模式，实际进入撤销待命。

**关键代码** `src/app/update/chat/rewind.rs:117`（`arm`）与 `:148`（`select_latest`）：

```rust
if !matches!(target.lx.chat.request, RequestState::Idle)
    || !target.lx.input.trim().is_empty()
    ...
    .any(|message| message.role == "user")
{ return None; }
```

**根因**：`arm()` 只看 `RequestState::Idle`。shell 不占用 `RequestState`，命令运行时仍是 `Idle`，且运行中的卡片 `role == "user"` 恰好满足“有用户轮次”；`arm()` 也不判断 `shell_mode()`。`apply_lx_input`（`pane.rs:92`）先走 rewind 路由，于是 `escape_stop` 与空 shell 模式退出分支都收不到按键。`select_latest()` 只按 `role == "user"` 找目标，会把 `user_shell` 卡片当成可撤销轮次。

**修复**：`arm()` 排除 shell 模式、在途命令与非聊天用户消息；`select_latest()` 排除命令卡片并拒绝命令运行中选中。Esc 优先级恢复为：面板/撤销选中 > 空 shell 模式退出 > 停止待命。

### P1-2. spawn 回执晚于终态时，`pending_shells` 永久泄漏

**触发场景**：执行瞬间结束的命令（`!true`、`!printf x`）。`ProcessUiEvent::Exited` 与 `AppEvent::UserShellSpawned` 走两条独立通道，事件循环 `select!` 的取用顺序不保证；终态先被处理时 `ShellJob` 被 `finish()` 移除，随后到达的回执命中早退分支。

**关键代码** `src/event/chat/user_shell.rs:443`：

```rust
let Some(job) = streams.user_shell.jobs.get_mut(message_id) else {
    return false;
};
```

**根因**：`ShellJob` 同时承担“进程句柄”和“待提交结算凭据”两种职责；job 的生命周期比 pending 短，回执却以 job 存在为前提。

**后果**：`has_running_shell()` 恒为 true → 聊天 Enter 永远入队、`Tab` 永远排队、loading 动画不停、Esc 一直提示停止；只有切会话能恢复。

**修复**：`handle_spawned` 在 job 缺失时仍结算 pending（清草稿、记历史、按 `generation` 移除），只跳过进程元数据登记；`runtime_epoch` 改为可选，缺失时不覆盖已有快照。

### P1-3. 合并前 shell 分支的 v12 库升级后缺 rewind 表，启动恢复整体失效

**触发场景**：库由合并前的 shell 分支二进制迁移过（`user_version = 12`，有 `kind`，没有 `revision`，也没有 rewind 五表）。合并后的二进制：`version < 12` 为假 → `SCHEMA_V12` 不执行；`migrate_v13` 只补 `revision` 就推到 13。

**根因**：两个分支曾用同一个版本号指向不同迁移；合并时只补了 `revision` 列。更糟的是 `migrate` 在 `migrate_v13` 之前会执行“v12 发布前补列”的 `ALTER TABLE agent_file_mutation`，该表此时不存在，迁移直接失败。

**后果**：`sessions_needing_recovery()` 的 UNION 查询报 “no such table”，`recover_startup` 整体返回，连 `recover_interrupted_user_shells` 都不执行；残留 `streaming` 命令重载后永久显示运行中，rewind 预检/提交报错。

**修复**：主迁移事务中无条件执行 `SCHEMA_V12`（全部 `CREATE ... IF NOT EXISTS`，幂等），保证后续列补齐与 `migrate_v13` 看到完整表；`migrate_v13` 内先补 `revision` 再做整表重建（重建语句会 `SELECT revision`）。

### P2-1. 取消/切会话后，已入队的 `BeginUserShell` 仍会落库并执行

**触发场景**：同一事件批次里先产生 `BeginUserShell` 效应，随后双 Esc 取消或切会话清空了 `pending_shells`；效应仍在队列里，下一轮 `pump_user_shells` 照样预登记并 spawn。

**根因**：`begin` 从不校验 `pending_shells`，而它是“用户仍希望执行”的唯一凭据；清空 pending 的三条路径都不移除已排队的效应。

**修复**：`begin` 入口校验同一 `generation` 的 pending 仍存在，否则丢弃效应（不落库、不 spawn）。

### P2-2. 数据库侧撤销目标仍包含命令记录

**触发场景**：先聊天，再执行 `!ls`；命令结束后对上一聊天轮执行撤销。App 侧已排除命令卡片，但 `latest_user_turn` 与 `commit_rewind` 的“最新用户消息”查询仍取到 shell 记录，提交被判定为 `RewindTargetChanged`；若绕过校验，则会按 `turn_index` 删掉命令记录并把命令原文回填成聊天草稿。

**根因**：所有 rewind 目标查询只按 `role='user'`，与 `latest_user_draft` 的 `!is_user_shell()` 口径不一致。

**修复**：两处 SQL 增加 `AND kind <> 'user_shell'`。

### P2-3. 队列分派覆盖用户正在输入的草稿

**触发场景**：`!sleep 5` 运行中 → `Tab` 把 `!echo B` 入队 → 用户继续输入 `!echo C` 或聊天草稿 → sleep 结束触发 `drain_queue`，`import_full_text` 无条件替换输入，C 丢失；C 的 spawn ACK 又因修订号变化不再清草稿。

**根因**：分派前没有“输入为空”检查。

**修复**：输入非空时分派返回 false，队首保留；用户下一次提交完成后自然再分派。

### P2-4. 内置 rtk 过滤把失败命令改写成 “ok”

**触发场景**：`cargo` 不在 PATH、`cargo nextest`（无 `test result:` 行）、jest/vitest、`tsc`/`dotnet`（错误行不以 `error` 开头）。退出码非 0，过滤结果却是 `ok (no failures detected)` / `ok`，同时进入模型上下文与落库输出。

**关键代码** `src/chat/process/rtk/builtin.rs:112`：

```rust
if kept.is_empty() {
    return match summary {
        Some(line) => format!("{line}\n"),
        None => "ok (no failures detected)\n".to_string(),
    };
}
```

**根因**：把“没有匹配到已知标记”当成“成功”；过滤器只应裁剪，不应下结论。

**修复**：未命中摘要时回退 `filter_generic`（短输出原样、长输出首尾裁剪并标注省略）；构建过滤同理。非终态快照在 `persist_reaped` 中不再过滤，避免 `interrupted / ok` 的自相矛盾记录。

### P2-5. rtk 包装忽略管道/重定向，压缩结果写进文件或喂给下游

**触发场景**：`!curl -s URL > resp.json`、`!git diff | patch -p1`（安装外部 rtk 时）变成 `rtk curl … > resp.json`，落盘/下游拿到的是 rtk 摘要而非真实输出。

**根因**：`wrap_supported` 只看首个词，注释“管道/重定向不影响语义”不成立。

**修复**：增加引号感知的 `has_shell_operators`，命令含未引号的 `|`/`<`/`>` 时不包装；引号内的字面字符不影响包装。

### P2-6. 运行中命令的增量注入不计入上下文预算

**触发场景**：历史装配刚好占满预算（只留 256 token 信封余量），本轮命令终态再注入最多 96 KiB（约 24k token），请求直接超过模型上下文；生产代码没有 `shrink_once` 兜底调用点。

**根因**：装配与注入各自独立设上限，没有共享预算。

**修复**：注入上限降到 16 KiB；装配预算扣除等量 4k token（`USER_SHELL_INJECTION_RESERVE_TOKENS`），两边同源。

### P2-7. `replace_input` 不重算输入模式

**触发场景**：shell 模式下完成撤销，`on_rewind_succeeded` 用 `replace_input` 回填聊天原文，模式仍是 Shell，下一次 Enter 会被当命令执行。

**根因**：新增 `input_mode` 后遗漏了唯一的旧文本替换入口。

**修复**：`replace_input` 与历史召回同口径调用 `shell::split_mode`。

### P3（低危，均已修复）

| 问题 | 修复 |
| --- | --- |
| 仅空白正文的 shell 模式 Esc 无反应（`pane.rs:351`） | 判空改为 `trim().is_empty()` |
| `exit_shell` 不压撤销快照，Esc 退出不可 undo | 先 `snapshot()` 再改模式 |
| 用户主动停止的命令显示绿色成功点 | `Stopped` 使用 `agent_warn()` |
| 任一 shell 结束会清掉模型轮的停止待命 | 只在模型不忙时清除 |
| `begin_user_shell` 只按 `agent_message` 取 sequence，撤销提示遗留高水位时会撞号 | 与 `begin_turn` 共用 message ∪ notice 的联合取号 |
| Windows 下 `shell_quote` 用 POSIX 单引号，`cmd.exe` 无法执行 | 引号逻辑移入 `platform::shell_quote_path`，按平台生成 |
| `start_stream` 用 `role == "user"` 找本轮锚点 | 排除 `is_user_shell()`（当前事件顺序下不可达，防御性修正） |

### 未修复项（已确认风险与取舍）

- **rtk 白名单仍按首词放行长驻/交互子命令**：`npx prisma studio`、`gh run watch`、`playwright codegen`、`psql`。rtk 对未支持命令通常直接透传，未观察到缓冲；但无法在本仓库内证明，保留为已知风险。需要时可改为精确子命令白名单。
- **终态落库失败无重试**：`SQLITE_BUSY`/磁盘满时命令行保持 `streaming`，队列暂停分派，重启恢复会标记 interrupted；不自动重跑命令。与设计“只允许重试持久化”仍有差距，本次未引入重试泵。
- **多实例并发**：本地命令不持有 session guard，另一实例启动恢复可能把运行中的命令标成 interrupted，且后到的真实终态可能因 revision 被抬高而写不进去。
- **外部 rtk 无能力探测/回退**：只按“文件存在”判定；rtk 版本过旧或不支持某子命令时，原命令不会执行，也不会回退。
- **`shrink_once` 无生产调用点**：为旧有遗留；本次通过装配预留解决注入溢出，不引入溢出重试链路。

---

## 3. 已核对、未发现问题的部分

- `update_user_shell` 的 revision 单调与终态粘滞在单连接事务下成立；v13 整表重建保留主键、external ID、revision、索引与外键数据。
- 上下文去重与顺序：`seen` 按稳定消息 ID 去重，运行中的命令作为顺序屏障；`mark_initial_seen` 以装配来源为准。
- `UserShellUpdates` 握手不会多发模型请求；`Final` 阶段不触发额外采样。
- 卡片几何：`layout` 与 `render` 共用 `user_shell::rows/draw`，前导空行归后一条消息，末尾留一行呼吸位，行数守恒。
- `streaming_message(_mut)` 按 `assistant_id` 定位，命令卡片追加后模型增量不会写错消息。
- 输出净化：`sanitize_control` + `&<>` 转义 + 截断；控制序列不能伪造结构标签。
- 迁移 v11→v13 与 shell-v12→v13 均保留历史数据并通过 `foreign_key_check`。

---

## 4. 修复内容

| 文件 | 修改 |
| --- | --- |
| `src/app/update/chat/rewind.rs` | `arm`/`select_latest` 排除 shell 模式、在途命令、命令卡片；数据库目标查询同口径排除 |
| `src/app/update/chat/user_shell.rs` | 队列分派不覆盖草稿；停止待命只在模型不忙时清除 |
| `src/app/update/pane.rs` | 空 shell 模式判空改 `trim()` |
| `src/app/state/lx.rs` | `replace_input` 重算模式；`exit_shell` 进撤销栈 |
| `src/event/chat/user_shell.rs` | spawn 回执缺 job 仍结算 pending；`begin` 校验 pending；reap 仅终态过滤 |
| `src/event/chat/mod.rs` | 本轮 user message 锚点排除命令卡片 |
| `src/db/migration.rs` | 无条件幂等补齐 v12 表；`migrate_v13` 先补 `revision` 再重建 |
| `src/db/chat/rewind.rs` | 两处最新用户消息查询排除 `user_shell` |
| `src/db/chat/user_shell.rs` | sequence 与 notice 联合取号 |
| `src/chat/process/rtk/mod.rs` | 拒绝包装含管道/重定向的命令；路径引号走 platform |
| `src/chat/process/rtk/builtin.rs` | 未识别输出回退通用裁剪，不再合成 `ok` |
| `src/chat/context/mod.rs` | 注入上限 16 KiB 并在装配预算中预留等量 token |
| `src/platform/mod.rs` | 新增 `shell_quote_path`（POSIX/Windows） |
| `src/ui/main_content/lx/history/user_shell.rs` | 主动停止使用警告色 |
| 文档 | 停止窗口统一为 3 秒（design/task/standards） |

新增回归用例：

- `escape_in_empty_shell_mode_exits_mode_instead_of_arming_rewind`
- `escape_while_shell_runs_arms_stop_not_rewind`
- `rewind_never_selects_shell_command`
- `shell_only_session_does_not_arm_rewind`
- `spawn_ack_after_terminal_still_clears_pending`
- `cancelled_pending_never_reaches_database_or_spawn`
- `latest_user_turn_skips_shell_command_records`
- `shell_sequence_is_after_persisted_notice`
- `shell_branch_v12_upgrade_creates_rewind_tables`
- `queued_dispatch_never_overwrites_an_active_draft`
- `shell_terminal_keeps_model_stop_arm`
- `replace_input_recomputes_shell_mode`
- `exit_shell_is_undoable`
- `stopped_command_does_not_use_success_mark`
- `unrecognized_output_is_never_reported_as_ok`
- `pipes_and_redirections_are_never_wrapped` / `quoted_operators_do_not_block_wrapping`
- `body_rows_use_the_shared_tree_prefixes` / `hover_and_expand_lift_folded_output_to_default_foreground` / `narrow_width_truncates_title_and_keeps_tree_within_bounds`

## 5. 验证

修复后在 dev 工作区执行：

- `cargo fmt -- --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过，无告警。
- `cargo test`：3087 项通过（lib 3027 + 16 个集成套件），0 失败。
- 定向测试：`app::update::chat::tests::rewind` 22 项、`app::update::chat::tests::user_shell` 12 项、`event::chat::tests::user_shell` 7 项、`chat::process::rtk` 17 项、`db::migration::tests` 全通过。

## 6. 遗留风险

- 交互与外观仍需用户 `cargo run` 手验（`task.md` §6）：双 Esc 停止、空 shell 模式退出、命令卡片展开、队列提示。
- 命令 cwd 取工作区执行根，不复用模型在途轮次的冻结 cwd。
- 增量注入现在有 4k token 预算预留与 16 KiB 上限；预算极小的模型仍可能被其他上下文（工具结果）挤压。
- 外部 rtk 失败无自动回退；`LX_RTK_MODE=off` 可关闭。
- 上述“未修复项”保持不变，需另行设计（重试泵、能力探测、多实例存活标记）。
