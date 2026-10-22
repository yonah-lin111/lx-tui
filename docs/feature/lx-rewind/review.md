# lx 最新轮次撤销：实现评审

> 审查对象：`lx-rewind` 分支（worktree `.worktrees/lx-rewind`），评审时相对 `dev` 的 6 个提交，修复后共 8 个（含开发库 worktree 隔离）。
> 该分支已于 2026-10-22 合并到 `dev`，worktree 与分支已移除。
> 审查基线：同目录 [design.md](design.md)、[task.md](task.md)。
> 结论：主链路（捕获 → 预检 → 恢复 → 最终事务 → 回执）结构正确，但有 2 个会直接破坏
> 故障语义的缺陷，以及若干交互/一致性缺口。下列问题按严重度排列，均已定位到代码并在合并前修复。
> 文中的行号是**修复前**评审定位；修复后行号已变化，按函数名/片段检索。

## 1. 关键链路

```text
写入轮次
  begin_turn ── 同事务插入 user/assistant + agent_turn_file_scope(capturing) + revision+1
  write/edit ── 跨进程路径锁 → 进程内路径锁 → Prepare ACK(before/planned-after)
             → 写盘/删除/rename → 观察实际 after → Commit ACK(actual-after)
  finish     ── 无遗留 prepared 时把 scope 封为 complete

撤销
  Enter ── precheck(忙/后台任务/其他 pane) → ExecuteRewind(冻结目标与 workspace)
  run   ── 查未完成操作 → 查 operation 幂等 → restore_plan(最新轮次/线性/快照可用)
         ── 仅文件动作：session guard → 路径锁 → 全量预检(current == 期望 after)
         ── operation(preparing) → 逐路径写 before → operation(files_restored)
         ── 最终事务：revision/latest 复核 + 删轮次 + notice + revision+1 + operation committed
         ── 任一步失败：逆序补偿；补偿失败标 recovery_required
  回执 ── 成功先重载 DB 真相，再只对发起 pane 回填原文
```

## 2. 缺陷

### C1（严重）补偿前的校验对象写错，真实失败时从不回滚文件

`src/event/chat/rewind/restore.rs:287`

```rust
fn compensate(paths: &[RestorePath], preimages: &[RawImage]) -> bool {
    for (path, preimage) in paths.iter().zip(preimages).rev() {
        let target = Path::new(&path.target_path);
        if let Ok(current) = read_raw(target, false)
            && current.same_content(preimage)
        {
            if apply_image(target, preimage).is_err() {
                return false;
            }
            continue;
        }
        return false;
    }
    true
}
```

错误原因：`preimages` 是操作前的磁盘状态（等于预检时的 `expected_after`），而恢复阶段写入的是
`path.before`。补偿时磁盘上已经是 `path.before`，拿它与 `preimage` 比较必然不相等，
于是直接返回 `false`，一个字节也不会回滚。

失败场景：
1. 第二个文件恢复失败；
2. 文件已恢复但最终 DB 事务失败（例如 revision 被另一实例推进）。

两种情况都会把 operation 标成 `recovery_required` / 报告“rollback also failed”，但磁盘仍停在
“文件已恢复”的状态；用户被告知需要手工处理，实际自动补偿本可以成功。且现有测试完全没有覆盖
补偿路径（`restore/tests.rs` 只有 6 个正常/冲突用例）。

修复：补偿需要同时拿到“本操作刚写入的镜像”和“执行前镜像”，先确认 `current == 刚写入的
镜像`，再写回执行前镜像。

### C2（严重）`recovery_required` 不阻塞后续操作

`src/db/chat/rewind.rs:694`

```sql
WHERE session_id = ?1 AND phase IN ('preparing', 'restoring', 'files_restored')
```

错误原因：补偿失败时写入的 `recovery_required` 不在查询集合里，`run()` 的入口检查看不到它，
同一会话可以继续撤销；设计要求的“阻止该会话继续撤销/发送”没有落地。同时
`restore.rs:51` 的提示写着 “restart lx to recover it”，但项目并没有启动恢复流程，属于误导。

修复：把 `recovery_required` 纳入未完成操作集合，并把提示改成如实说明“需要手工恢复文件”。

### M1（主要）session guard 的作用域太短，delete_only 完全没有 guard

`src/event/chat/rewind/restore.rs:194` 只在 `restore_files()` 内取 guard，函数返回即释放；
最终 `commit_rewind()` 和补偿阶段都不持锁。`delete_only` 走不到 `restore_files`，从头到尾没有
session guard。

失败场景：撤销恢复完文件、尚未提交对话时，另一个 lx 实例对同一会话发起新轮次/删除，两个进程
同时改变同一会话。revision 复核通常能兜住最终提交，但这正是设计要求 guard 覆盖
“校验 → 文件 → 提交 → 补偿”全程的原因；仅文件动作也会在无 guard 的窗口里被其他实例改写。

修复：在 `run()` 的幂等检查之后、`restore_plan` 之前取 guard，持有到函数结束；
`begin_operation` 提前到写文件之前调用一次。

### M2（主要）其它面板打开时，Esc 被撤销待命吞掉

`src/app/update/chat/rewind.rs:35` 的 `apply_input` 在 `pane.rs` 中先于粘贴/提及/slash/块面板
执行；它只识别 `SlashPanel::Rewind`，对 `Model`、`Resume`、`History`、md 块面板、`@` 提及面板
一律不看。`/model`、`/resume` 打开时输入框已被清空，恰好满足 `arm()` 的“空闲 + 空输入”条件：

错误原因：用户按 Esc 想关闭面板，结果第一个 Esc 被撤销逻辑消费并显示
“Press esc again to rewind”，面板还开着；第二个 Esc 甚至会在面板未关闭的情况下选中撤销目标。

修复：`apply_input` 在待命/选中逻辑之前，只要存在任何其它覆盖层（`lx.slash`、`lx.panel`、
mention、paste）就返回 `Route::Pass`，把 Esc 交还给面板路由。设计 4.2 明确要求
“高优先级面板关闭所消费的 Esc 不能算作第一次 Esc”。

### M3（主要）回执阶段重新解析 workspace，未使用冻结值

`src/event/chat/rewind.rs:189`

```rust
let Some(workspace) = state.chat_session_key_for_pane(pane) else { ... };
let loaded = persistence.load_rewind_view(&workspace, session_id);
```

错误原因：执行请求已冻结 `workspace`（`RewindRequest.workspace`），但回执又按“当前窗格”解析
一次。撤销执行期间切换工作区/仓库后，DB 归属解析会指向另一个 workspace，`load_rewind_view`
返回 `SessionMissing`，界面提示 “rewind committed but reload failed”，历史停留在删除前的旧状态。
这是一次已经成功的撤销被误报为失败。

修复：`AppEvent::RewindFinished` 携带请求里的冻结 `workspace`，回执只用它做 DB 读取。

### M4（主要）已恢复文件的轮次把“含文件动作”错误禁用

`src/app/state/lx/rewind.rs:103`

```rust
pub fn files_available(self) -> bool {
    matches!(self, Self::Managed { .. } | Self::KnownEmpty)
}
```

错误原因：`AlreadyRestored` 被当作不可用，面板把两项文件动作禁用并显示
“File snapshot unavailable”。设计 4.3 要求：该轮已执行仅文件恢复后，后续仅文件恢复是幂等
无操作，组合操作此时只删除对话，并提示“此前已恢复”。执行层已经正确处理
（`restore.rs:74`），只有状态/面板把它挡掉了。

修复：`AlreadyRestored` 视为可用，并在面板条目上显示 “Files already restored”。

### m1（次要）删除轮次后不清理快照，容量只增不减

`src/db/chat/rewind.rs:792` 删除消息后，`agent_turn_file_scope`、`agent_file_mutation` 和
对应 `agent_file_image` 仍留在库里。该轮已不可能再被撤销，这些数据永远不可达；按设计上限
每轮最多 256 MiB，长期使用会持续膨胀，只有删除整个会话才随 CASCADE 清掉。

修复：删除轮次成功时同事务删除该 user message 的 scope 与 mutation，提交后由既有的
`prune_orphan_images` 回收镜像。

### m2（次要）预检失败时把 expected revision 覆盖成 0

`src/event/chat/rewind.rs:89` 在 `restore_plan` 失败后用 `latest_user_turn` 兜底，再失败就传
`0`；`on_prepared` 无条件覆盖 `selected.expected_revision`。一次瞬时读失败会让 `delete_only`
在最终提交时恒报 “latest turn changed”。

修复：数据库兜底也失败时保留选择时冻结的 revision，不写 0。

### m3（次要）空会话也进入撤销待命

`src/app/update/chat/rewind.rs:108` 的 `arm()` 只检查空闲、空输入、session 存在，没有检查是否
真的有用户消息。首次 Esc 会显示 “Press esc again to rewind”，第二次 Esc 什么也不会发生。

修复：`arm()` 要求内存中至少有一条 `role == "user"` 的消息。

### m4（次要）未完成操作的读取错误被吞掉

`src/event/chat/rewind/restore.rs:32` 使用 `if let Ok(Some(...))`；数据库读取失败等同于
“没有未完成操作”，随后继续预检。修复：读取失败直接按失败返回。

### m5（次要）`lock_external` 有一次无意义转换

`src/chat/tools/mutation.rs:214` 的 `.map(|guard| guard.or_else(|| None))` 是空操作。
改为在 `Ok(None)` 时直接构造超时错误。

### m6（次要）单文件超过 1000 行

`src/db/chat/rewind.rs` 当前 1002 行，违反项目规范。把“恢复计划 + 镜像读取 + 孤儿清理”
（`restore_plan` / `image` / `MutationRow` / `prune_orphan_images`）拆到
`src/db/chat/rewind/plan.rs`。

### m7（次要）死字段

`RewindRequest.cwd` 与 `RewindTarget.cwd` 在事件层从未使用（恢复路径全部来自快照里的绝对
`target_path`）。保留只会让后来者误以为 cwd 参与恢复。

### m8（次要）生产代码使用 `expect`

拆出恢复计划的同一段代码里有两处 `group.first().expect(...)` / `group.last().expect(...)`，
违反“生产代码禁止 unwrap/expect”。分组逻辑可证明非空，但规则就是规则，改为 `let ... else`。

### 2.1 补充评审（第二轮，全部已修）

| 编号 | 严重度 | 问题 | 修复 |
| --- | --- | --- | --- |
| S1 | major | `edit` 的 Update+Move 在无链接间接时 `resolved == target`，Update 与 MoveSource 共用同一 `target_path`；MoveSource 的 before 在写盘前读取，而工具结束后的观察只能看到 `Missing`，`restore_plan` 判为第三方交错写，整轮永久 `Incomplete`（最常见的 `*** Move to` + 内容修改形态）。 | 同一路径只记一条 mutation（合成 MoveSource，before 取写前内容）；补充 recorder 形状测试。 |
| S2 | major | `git symbolic-full-name` 不是合法命令，`head_ref` 恒为 `None`，同 OID 切分支/detach 无法被身份校验发现。 | 改用 `git symbolic-ref -q HEAD`（unborn 也能取到分支，detached 自然为 None）；测试断言 `refs/heads/...`。 |
| S3 | major | `PrepareRewind` 在事件主循环同步读取全部快照 BLOB（单份 64 MiB、单轮 256 MiB），双击 Esc 会冻结 UI。 | 新增 `rewind_availability`：只读 scope/mutation 元数据；字节与线性校验留在后台执行期。 |
| S4 | major | 撤销成功重载后 `plan = None` 且不重建，`revert_only` 也会丢计划；下一轮运行上下文缺 `current plan`。 | 与 `on_session_loaded_full` 一致，从剩余消息重建 `restore_plan`。 |
| S5 | major | 粘贴不清理双 Esc 待命，`select_latest` 也不复核空草稿 → 待命期间粘贴后仍可选中，成功回填会覆盖刚粘贴内容。 | `handle_lx_paste` 清理撤销状态（执行中则拒收）；`select_latest` 复核空输入。 |
| S6 | major | `delete_only` 与会话删除不取 session guard，另一实例可在流式期间删掉正在写入的轮次。 | `run` 全程持 guard（已含 delete_only）；`DeleteChatSession` 效果同样非阻塞取 guard，失败提示会话忙。 |
| S7 | minor | `rewind.clear()` 不关 `lx.slash`，外部重载/删除会话留下点不动的孤儿 Rewind 面板。 | 新增 `Lx::clear_rewind()`，统一清两处。 |
| S8 | minor | 迟到回执不校验 operation id，旧失败会清掉新一次执行。 | `RewindFinished` 携带 operation id，不匹配则丢弃。 |
| S9 | minor | 多路径恢复只在预检时读一次；预检后、写入前的外部修改会被覆盖。 | 每路径写前复读并与预检镜像比对，不一致则逆序补偿并失败。 |
| S10 | minor | `revert_only` 零文件时文案仍说 “Restored files”；范围说明 `LX_REWIND_SCOPE` 从未渲染。 | 改为 “No file changes; conversation kept.”；面板顶边右侧固定渲染范围说明。 |
| S11 | minor | `lock_root` 只看 `LX_TUI_ENV`，不看 `LX_TUI_DB_PATH`；同 DB 不同环境的两个进程锁目录不同。 | 显式覆盖 DB 路径时，锁目录按该路径哈希固定。 |

## 3. 实施中发现并一并修复的问题

把 `begin_operation` 提前到文件写入之前后，预检冲突（文件被外部改过）会在 operation 行已存在时
直接返回失败，不结算该行，导致 `unfinished_operation` 永久挡住该会话。修复：`run()` 在
`restore_files` 返回失败后，只在阶段仍为 `preparing/restoring` 时结算为 `failed`，保留已经写入
的 `recovery_required`。测试 `later_external_write_is_a_conflict_and_nothing_is_rewritten`
增加断言。

## 4. 第三轮：未修项补齐（全部已实现）

| 编号 | 原限制 | 实现 |
| --- | --- | --- |
| T1 | 没有启动恢复：prepared mutation / capturing scope / 未完成 operation 永久卡死 | 新增 `event::chat::rewind::recovery`：启动时扫描需要恢复的会话，**非阻塞取得 session guard** 后逐会话结算。prepared 当前等于 before/planned 才推断实际结果并封口 scope；第三种状态保持 prepared（不猜）。`preparing` 直接结算失败；`restoring/files_restored` 按恢复计划把文件回滚到操作前状态，任一第三方变化转 `recovery_required`。 |
| T2 | 启动旧恢复无条件全库 UPDATE | `recover_interrupted_streams/tool_calls/processes` 全部改为按会话列表过滤；只处理拿到 guard 的会话，另一个实例的运行态不会被第二个实例中断。 |
| T3 | `recovery_required` 不阻止继续发送 | `guard_begin_turns` 在 `begin_turn` 之前检查未完成撤销；`BeginChatTurn` 效果本身也做同样检查。命中时状态回 Idle、草稿保留、弹出原因。 |
| T4 | 新轮次 guard 晚于 `begin_turn` 事务 | 事件循环在 `persistence.pump` 之前调用 `guard_begin_turns`，按 `(pane, generation)` 暂存 guard；`start_stream` 取走。轮次失败由 `prune_pending_guards` 释放；新会话无并发者，仍在拿到 id 后补取。 |
| T5 | 父目录/链接解析身份未复核 | mutation 新增 `anchor_path` / `anchor_canonical`（捕获时的现存祖先与规范化身份）。恢复前校验祖先未被替换、中间目录不是符号链接；旧记录（两列为 NULL）跳过该检查，单边有值视为损坏拒绝。 |
| T6 | 非 UTF-8 路径经 `to_string_lossy` 丢失 | 新增 `platform::encode_path/decode_path`：UTF-8 原样、非 UTF-8 用带标记 base64；路径、目标、链接目标、worktree 身份统一编码。`RestorePath.target_path` 改为 `PathBuf`。 |
| T7 | actual-after 镜像不计入限额 | `commit_mutations`/启动结算把 actual 字节累加到 `snapshot_bytes`；写入已发生不能拒绝，但后续 prepare 的 256 MiB 限额会如实计入。 |

## 5. 仍保留的限制

1. **第三种状态只能人工处理**：删除轮次前/恢复中途文件被外部改成既非 before 也非 expected-after 时，启动恢复标 `recovery_required`，不猜也不覆盖；该会话的发送与撤销会被阻止，需人工恢复文件。
2. **会话删空后无法幂等重放**：`agent_rewind_operation` 随 session CASCADE 删除，operation id 重放只能得到 `SessionMissing`。实际影响仅限“最终事务已成功但回执丢失且刚好删空会话”，不产生重复副作用。
3. **外部写入竞态**：写出前会再次复检，但复检与 rename 之间的窗口无法由 advisory lock 消除，与设计第 7.4 节声明一致。
4. **UI 次要项未做**：撤销失败只用 toast 说明、不保留面板；焦点/标签页切换不主动清除 3 秒内的待命；仅提示历史不可滚动。均不影响文件安全与回填正确性。

## 6. 修复清单

| 编号 | 修复 |
| --- | --- |
| C1 | `compensate` 增加“刚写入镜像”参数，先比对当前内容再写回执行前镜像；补单元测试 |
| C2 | `unfinished_operation` 纳入 `recovery_required`；失败提示改为手工恢复说明 |
| M1 | guard 提升到 `run()` 全程；`begin_operation` 提前到恢复文件之前，只调用一次 |
| M2 | 任一其它覆盖层打开时撤销路由返回 `Pass`；补测试 |
| M3 | `RewindFinished` 携带冻结 workspace，回执只读冻结值 |
| M4 | `AlreadyRestored` 可用并在面板显示说明；补状态测试 |
| m1 | 删除轮次时清理 scope/mutation，提交后回收镜像 |
| m2 | 预检失败保留冻结 revision |
| m3 | `arm()` 要求存在用户消息；补测试 |
| m4 | 未完成操作读取失败按失败处理 |
| m5 | 简化 `lock_external` |
| m6 | 拆分 `db/chat/rewind.rs` → `rewind/plan.rs`（862 + 176 行） |
| m7 | 删除 `RewindRequest.cwd` / `RewindTarget.cwd` |
| m8 | 用 `let ... else` 替代 `expect` |

上述缺陷均已修复。

## 7. 验证

```text
rtk proxy cargo fmt --check       通过
rtk proxy cargo clippy --all-targets -- -D warnings
                                  通过（0 warning）
rtk proxy cargo test --lib        2930 passed, 0 failed
rtk proxy cargo test              全部通过（含集成测试）
```

新增回归测试：

- `compensation_writes_preimage_back_when_current_is_restored_content`
- `compensation_refuses_when_external_write_happened`
- `held_session_guard_blocks_rewind_without_writing`
- `deleting_turn_cleans_scope_mutations_and_images`
- `escape_closes_command_panel_without_arming_rewind`
- `empty_session_does_not_arm_rewind`
- `already_restored_keeps_file_actions_enabled`
- `update_with_move_records_one_mutation_per_path`
- `availability_is_metadata_only`
- `lx_paste_cancels_rewind_arming`
- `blobs_roundtrip_and_identity_is_per_worktree` 增加 `head_ref` 断言
- `settles_prepared_mutation_that_never_applied` / `..._already_applied`
- `leaves_unknown_mutation_unsettled`
- `rolls_back_interrupted_restore` / `third_state_blocks_interrupted_restore`
- `recovery_only_touches_listed_sessions`
- `guard_is_taken_before_begin_turn` / `busy_session_is_refused_before_begin_turn` / `unfinished_rewind_blocks_sending`
- `commit_accounts_actual_image_bytes`
- `utf8_paths_roundtrip_unchanged` / `non_utf8_paths_roundtrip_without_loss` / `chain_*`

`later_external_write_is_a_conflict_and_nothing_is_rewritten` 增加“失败后不残留未完成操作”断言。
