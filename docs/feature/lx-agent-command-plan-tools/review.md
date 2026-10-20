# 代码评审：后台进程管理 + 进程/计划工具 + 后台任务面板

评审范围：分支 `feat/lx-agent-command-plan-tools` 相对 `dev`（merge-base `d0ea193`）的全部改动，
121 个文件、约 1.9 万行。按四个层次分工评审：进程内核层、事件运行时层、应用状态层、UI 渲染层。

本文件记录**真实缺陷**（会 panic / 丢数据 / 用户可见错误 / 违反项目硬性规范），不含风格建议。
每条给出：**错误原因**（为什么是 bug、什么条件触发、用户看到什么）、**关键代码**、**修复建议**。

> 修复状态标记：`[已修]` 表示本轮已改并补了回归测试；`[已修-部分]` 表示主体已修、
> 有明确记录的残余风险；`[待修]` 表示记录在案、未在本轮处理（均有说明为什么留待后续）。
>
> 统计：**28 条已修**（含 1 条部分修复），**1 条待修**。

---

## 一、进程内核层（`src/chat/process/`、`src/db/`、`src/chat/tools/`）

### [高] `settle()` 递减了从未递增的全局计数，一次 spawn 失败永久掐死全进程的 `exec_command`

- 文件：`src/chat/process/manager/mod.rs:431`（配合 `:272`）
- 错误原因：`settle()` 无论该 handle 是否被计入过都会 `GLOBAL_RUNNING.fetch_sub(1)`，而
  `fetch_add` 要到 spawn **成功之后**才执行。spawn 失败（模型传了不存在的 `shell`、PTY 打开失败）
  走 `handle.settle(Failed)` → 计数从 0 减 1 → `u64` 下溢成 `u64::MAX`。此后任何会话的 `exec()`
  在 `:405` 的 `GLOBAL_RUNNING.load() >= global_max_running` 判断恒为真，**整个 lx 进程生命周期内
  所有 `exec_command` 都返回 `resource_limit`**，且无法恢复（静态计数器，无重同步路径）。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:431 —— spawn 失败路径
if let Err(error) = spawned {
    handle.settle(ProcessState::Failed {          // ← handle 从未 fetch_add 过
        code: error.code.to_string(),
    });
    return Err(error);
}

// src/chat/process/manager/mod.rs:264 —— settle 无条件递减
fn settle(&self, next: ProcessState) -> bool {
    let mut state = lock(&self.state);
    if state.is_terminal() {
        return false;
    }
    *state = next.clone();
    *lock(&self.ended_at_ms) = Some(crate::db::now_ms());
    drop(state);
    GLOBAL_RUNNING.fetch_sub(1, Ordering::Relaxed);   // ← 无前置条件
```

- 关键点：`settle()` 同时承担「状态转移 + 全局计数 + UI 通知」三件事，却被多条前置条件不同的路径
  调用；「是否已计数」这个前置状态没有被建模。
- 修复建议：把「是否已计入全局运行数」建模到 handle 上（`counted: AtomicBool`，仅在 `fetch_add`
  之后置位），`settle()` 只在 `counted.swap(false)` 为真时 `fetch_sub`。`[已修]`

### [高] 登记后无条件把状态写回 `Running`，秒退进程永远停在 `Running` 并泄漏全局计数

- 文件：`src/chat/process/manager/mod.rs:446`
- 错误原因：supervisor 任务在多线程 runtime 上 `tokio::spawn`，完全可能在主任务执行到 `:446` 之前
  就 `child.wait()` 完成并 `settle(Exited)`。随后 `*lock(&handle.state) = ProcessState::Running` 是
  **无条件覆盖**，把已终态的 handle 改回 `Running`：supervisor 已返回、没人再 settle，于是该进程
  在任务面板里永远「运行中」、`GLOBAL_RUNNING` 永久 +1，模型拿到的也是 `running`。
  `true` / `echo hi` 这类秒退命令最容易撞上。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:446
GLOBAL_RUNNING.fetch_add(1, Ordering::Relaxed);
*lock(&handle.state) = ProcessState::Running;   // ← 无条件覆盖，可能把 Exited 改回 Running
```

- 关键点：状态机缺「`Starting → Running` 才合法」这条守卫；写状态的地方没有做 CAS。
- 修复建议：改成条件转移 `mark_running() -> bool`，只在 `state == Starting` 时写入；返回 false
  时不要 `fetch_add`。`[已修]`

### [高] `runtime_epoch` 跨 lx 重启不唯一，`/resume` 后新进程记录静默丢失

- 文件：`src/db/chat/process.rs:100`（唯一约束 `:326`；epoch 生成在 `src/event/chat/runtime.rs:88`）
- 错误原因：epoch 是 `format!("{session_id}:{counter}")`，`counter` 存在**内存** `HashMap` 里，
  重启后从 1 重新开始。于是「重启 lx → `/resume` 同一会话 → 再跑 `exec_command`」得到的
  `(runtime_epoch, process_session_id)` 与上一个 lx 进程留下的历史行完全相同。`begin_process`
  命中 `UNIQUE(runtime_epoch, process_session_id)` → 返回 `ProcessConflict`，`register_process`
  只打一条 warn 日志就 `return`，`persist_snapshot` 根本不会被调用。用户可见现象：重启后
  `/resume` 的历史命令卡片永远停在 `Running`，任务面板里也没有对应服务可停。
- 关键代码：

```rust
// src/event/chat/runtime.rs:89 —— 进程内计数，重启后从 1 重新开始
let counter = self.epochs.entry(session_id.to_string()).or_insert(0);
*counter = counter.saturating_add(1);
let epoch = format!("{session_id}:{}", *counter);

// src/db/chat/process.rs:326
UNIQUE(runtime_epoch, process_session_id)
```

- 关键点：「进程内从 0 开始」的标识被当成跨进程唯一来用。同类问题还有进程 `session_id`（见下一条）。
- 修复建议：epoch 生成时加入跨进程唯一成分（进程启动毫秒 + pid），例如
  `format!("{session_id}:{started_ms}:{counter}")`。`[已修]`

### [高] 进程 `session_id` 每会话从 1 起算，事件按 id 全局扫描运行实例 → 多会话串台

- 文件：`src/event/chat/mod.rs:136`（配合 `src/chat/process/manager/mod.rs:346`）
- 错误原因：`next_id` 是**每个管理器一个、从 1 开始**，两个并存的聊天会话各自的第一个后台进程
  都叫 `session_id = 1`。而 `ProcessUiEvent` 只带 `session_id + origin_call_id`，事件层用
  `runtime_by_session_id` 在所有运行实例里 `find` 第一个「快照里有这个 id」的管理器
  （`HashMap` 迭代顺序，结果不确定）。于是会话 B 的进程事件可能被判给会话 A，
  `handle_process_event` 用 **A 的 workspace/epoch** 去 `finish_process`，把 A 正在跑的进程写成
  B 的终态；同时 B 自己的行永远停在 `running`。UI 侧 `apply_interaction_exit` 只按 `session_id`
  匹配**全状态所有窗格**，A 的 `Send input` 卡片会被 B 的进程终态翻掉。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:346
next_id: AtomicU64::new(1),

// src/event/chat/mod.rs:136 —— 全局扫描，取第一个匹配
self.runtimes
    .sessions()
    .into_iter()
    .filter_map(|session| self.runtimes.get(&session))
    .find(|runtime| runtime.manager.snapshot(session_id).is_some())
```

- 关键点：路由键（进程 id）不唯一，却被当作全局主键去反查归属。
- 修复建议：把进程 id 源改成**进程内全局** `AtomicU64`。`ProcessHandle` / DB 本来就用
  `(runtime_epoch, process_session_id)` 定位，id 全局唯一不影响任何逻辑。`[已修]`

### [中] `ProcessSnapshot.tail` 从保留区间的**头部**取 64 KiB，输出超 64 KiB 后预览永远落后

- 文件：`src/chat/process/manager/mod.rs:294`
- 错误原因：缓冲保留 256 KiB，但 `tail_text()` 用 `read_from(output.start(), …)` —— 从最老的
  保留字节开始读 64 KiB，而不是从 `end - 64KiB` 开始。任何输出量超过 64 KiB 的命令，
  `snapshot.tail` 带的都是「64 KiB 之前的旧内容」，UI 的 `preview_lines(&snapshot.tail)` 也显示旧行：
  用户看到卡片/任务面板输出在长时间里**不刷新**，而进程明明在持续写。`interact` 的 terminate
  分支也用同一个 `tail_text()` 补「最后输出」，同样返回旧内容。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:294
fn tail_text(&self) -> (String, u64) {
    let output = lock(&self.output);
    let start = output.start();                          // ← 保留区间起点，不是尾部起点
    let read = output.read_from(start, MAX_MODEL_OUTPUT_BYTES, true);
    (read.text, output.dropped_bytes())
}
```

- 关键点：函数名是 `tail`，实现是 `head`。这是本项目「快照语义不诚实」的典型。
- 修复建议：`let from = output.end().saturating_sub(MAX_MODEL_OUTPUT_BYTES as u64);`
  再 `read_from(from, …)`（`read_from` 内部已用 `max(self.start)` 收敛）。`[已修]`

### [中] 未闭合的 ANSI 序列被整段吞掉，跨读取边界被切开的转义序列后半段当正文渲染

- 文件：`src/chat/process/output.rs:217`（`skip_escape` 见 `:288`）
- 错误原因：`sanitize_text` 对 `skip_escape` 返回 `None`（未闭合/超长）的处理是
  `index = bytes.len()`，即把本次读取窗口剩余的**全部字节**都算作已消费、推进游标。
  当一条 CSI/OSC 正好被 64 KiB 读取窗口切开（交互式进程、进度条、`write_stdin` 250 ms 窗口下
  非常常见）时，前半段 `\x1b[3` 被丢弃并推进游标，后半段 `2mred` 在下次读取时被当成普通文本
  输出——模型与用户看到 `2mred` 这种垃圾文本，且游标无法回退。同一函数对「尾部不完整 UTF-8」
  却是**不消费**的（`None => break`），处理策略自相矛盾。
- 关键代码：

```rust
// src/chat/process/output.rs:217
if byte == 0x1b {
    index = match skip_escape(bytes, index) {
        // 未闭合序列：丢弃剩余的这几个字节，不把转义原样画出来。
        None => bytes.len(),          // ← 把后续可见文本一起吞掉，并推进游标
        Some(next) => next,
    };
    continue;
}
```

- 关键点：「未闭合（可能还有后续字节）」与「超长（确定要丢弃）」是两种语义，被合并成了一个 `None`。
- 修复建议：`skip_escape` 区分 `Pending`（在 ESC 处 `break`，不消费，下次带更多字节重试）与
  `Overlong`（丢弃）。`[已修]`

### [中] `read_for_model` 的 cursor 是「读-改-写」非原子，并发时重复输出或游标回退

- 文件：`src/chat/process/manager/mod.rs:286`
- 错误原因：`cursor` 用 `AtomicU64` 但整体操作不原子：`load` → 加锁读缓冲 → `store`。两个并发
  调用者（模型正在 `write_stdin` 等待、用户在面板按停止，`tokio::spawn(manager.stop(...))` 与
  本轮 `interact` 同时走 `result_of` → `read_for_model`）会读到同一 cursor，把**同一段输出**
  作为两次工具结果返回给模型（重复行）；先完成者可能后 `store`，把游标写回较小值，下次再重复一遍。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:286
fn read_for_model(&self, max_bytes: usize) -> OutputRead {
    let cursor = self.cursor.load(Ordering::Relaxed);
    let read = lock(&self.output).read_from(cursor, max_bytes, self.state().is_terminal());
    self.cursor.store(read.end, Ordering::Relaxed);   // ← 无比较交换，可回退
    read
}
```

- 修复建议：加一把 `std::sync::Mutex<()>` 把「读区间 + 推进游标」串行化（不能用 `output` 锁，
  `wait_window` 已在另一处 `lock(&handle.output)`，反向加锁会死锁）。`[已修]`

### [中] `max_output_tokens` 只在「何时返回」上生效，从不在「返回多少」上生效

- 文件：`src/chat/process/manager/mod.rs:698`
- 错误原因：schema 与工具描述都写明这是「返回模型的文本预算」，但 `result_of` 恒用
  `budget = MAX_MODEL_OUTPUT_BYTES`（64 KiB ≈ 16k token），`args.max_output_tokens` 只参与
  `wait_window` 的 `enough` 早退判断。模型显式请求 `max_output_tokens: 200` 时仍可能一次拿到
  64 KiB 正文，把上下文撑爆，同时模型会认为参数被忽略而反复缩小参数重试。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:698
let budget = MAX_MODEL_OUTPUT_BYTES;   // ← 完全没看 max_output_tokens
let read = handle.read_for_model(budget);
```

- 修复建议：把 `max_output_tokens` 传进 `result_of`，按 `tokens * 4` 与 `MAX_MODEL_OUTPUT_BYTES`
  取 min 作为 budget，让 `truncated` 自然驱动下一次 `write_stdin`。`[已修]`

### [中] 第二次停止请求把进程直接落成 `Interrupted`，覆盖真实的 `Stopped` / `TimedOut`

- 文件：`src/chat/process/manager/mod.rs:764`
- 错误原因：`stop_handle` 用 `handle.stop.take()` 认领唯一一次停止请求。第一个调用者拿走 sender
  后要等 supervisor 落终态（最长 2 s）；期间第二次 stop（面板 `^d` 连按、Esc 与本轮
  `write_stdin terminate` 重叠、`timeout_ms` 后台任务与 `wait_window` 生命周期分支同时到点）
  拿到 `None`，走「没有 supervisor」分支立刻 `settle(Interrupted)`——而进程其实正在被正常
  SIGTERM 掉。结果：库里/UI 上是 `interrupted`、模型收到 `command interrupted`，真实原因
  `stopped`/`timed_out` 丢失，`GLOBAL_RUNNING` 提前递减。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:764
let sender = { let mut guard = handle.stop.lock().await; guard.take() };
match sender {
    Some(sender) => { let _ = sender.send(reason); }
    None => {
        // 没有 supervisor（理论上不会发生）：退回直接标记，绝不伪报已停止。
        handle.settle(ProcessState::Interrupted);   // ← 实际是「停止请求已被别人认领」
        return;
    }
}
```

- 关键点：把「已认领」误当成「无 supervisor」。两个状态被压进一个 `Option<Sender>`。
- 修复建议：加 `stop_requested: AtomicBool` 区分两种 `None`；已认领时走与首个调用者相同的
  「有界轮询等终态」路径，而不是自行落 `Interrupted`。`[已修]`

### [中] `prune_finished` 会删掉「shell 已退出但后台作业仍在跑」的记录

- 文件：`src/chat/process/manager/mod.rs:727`
- 错误原因：淘汰条件只有「终态 + `ended_at_ms` 超 TTL / 条数超限」，不看 `has_living_group()`。
  `nohup bash task.sh &` 这类真实场景里 shell 秒退、作业长期运行，10 分钟后（或下一次 `exec`
  触发 `prune_finished`）该 handle 从 map 里被移除：任务面板不再列出它，`stop` 也找不到它——
  而进程组仍在跑，正是这个特性本来要解决的用户痛点。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:727
let mut finished: Vec<(u64, i64)> = processes
    .iter()
    .filter(|(_, handle)| handle.state().is_terminal())   // ← 没排除 background_alive
    .map(|(id, handle)| (*id, lock(&handle.ended_at_ms).unwrap_or(now)))
    .collect();
```

- 修复建议：过滤条件改成 `is_terminal() && !has_living_group()`。`[已修]`

### [中] 对已退出进程停止时直接用可能已被回收的 pid 杀「进程组」，有误杀无关进程的风险

- 文件：`src/chat/process/manager/mod.rs:757`（pid 来源 `src/platform/mod.rs:221`）
- 错误原因：shell 退出后 `pid` 只被保留在 handle 里，从不校验。`has_living_group()` 用
  `kill(-pgid, 0)` 探测——如果 pgid 已被内核回收分配给别的进程组，探测会成功；随后
  `terminate_process_group(pid, true)` 就对**那个组**发 `SIGKILL`。窗口是「shell 退出后 TTL 内
  （10 分钟）用户点了停止」，在 pid 回绕快的高负载机器上并非不可发生。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:752
if handle.state().is_terminal() {
    if handle.has_living_group()
        && let Some(pid) = *lock(&handle.pid)
    {
        crate::platform::terminate_process_group(pid, true); // ← pid 可能已属于别人
    }
```

- 关键点：只靠 pid 认进程，没有「这个 pid 还是我 spawn 的那个」的证据。
- 修复建议：记录 pid 时同时记进程启动时刻，杀之前比对；对不上就放弃。
- 实际处理：`[已修-部分]` 新增 `platform::process_start_time()`（Linux 读
  `/proc/<pid>/stat` 的 starttime；macOS 用 `ps -o lstart=` 的稳定哈希），`stop_handle`
  杀进程组前用 `leader_pid_not_recycled()` 校验：**组长仍存活**时指纹不一致即拒绝杀
  （pid 被复用会被识破）；**组长已退出**的 `nohup … &` 场景无法证明，仍按 best-effort
  执行——那是用户明确要停的后台作业，拒绝杀等于让这一行永远停不掉。
- 一个实测得到的教训：指纹**不能**放在 `set_pid()` 的 spawn 热路径上。最初版本在那
  里同步调 `ps`，把「spawn → 进入等待」拖慢，导致读端 drain 落后于进程结算，
  `echo hello` 这类命令的输出变空（并行跑测试时 3~4 条稳定失败）。现在改为**惰性**
  取：首次需要判断身份时才调用并记住结果。

### [低] 退出时串行停止 + 调用方 3 s 总超时，进程多时部分进程根本没收到信号

- 文件：`src/chat/process/manager/mod.rs:569`
- 错误原因：`shutdown()` 逐个 `await stop_handle`，每个最多等 `STOP_GRACE * 2 = 2s`；而事件层
  退出路径整体只等 3 s。3 个以上需要走 SIGTERM→1 s→SIGKILL 的真实服务，就足以让退出在
  「还没轮到第 4 个进程」时超时返回：剩下的进程连 SIGTERM 都没发过，lx 退出后以
  `process_group(0)` 建立的孤儿进程组继续存活，DB 也不会落它们的终态。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:569
pub async fn shutdown(&self) {
    let all: Vec<u64> = lock(&self.processes).keys().copied().collect();
    self.stop_many(all).await;   // 串行：N × 最多 2s
}
```

- 修复建议：先给所有目标**并发送出**停止信号（不逐个等待），再统一做有界总等待。`[已修]`

### [低] `stop.try_lock()` 失败时静默丢弃停止通道，该进程从此无法停止

- 文件：`src/chat/process/manager/spawn.rs:211`（PTY 分支 `:252`）
- 错误原因：supervisor 注册停止通道用 `try_lock`，失败（锁被 `stop_handle` 短暂持有、或 spawn 与
  stop 撞车）时 `stop_sender` 直接被 drop：supervisor 永远等不到停止请求，之后 `stop_handle`
  拿到 `None` → 落 `Interrupted` 但**不杀进程**，进程继续跑到自然结束（长时间服务就是永久泄漏），
  且 UI 显示已停止/中断。
- 关键代码：

```rust
// src/chat/process/manager/spawn.rs:211
let (stop_sender, mut stop_receiver) = oneshot::channel::<StopReason>();
if let Ok(mut guard) = handle.stop.try_lock() {
    *guard = Some(stop_sender);   // ← Err 分支静默丢弃 sender
}
```

- 修复建议：`try_lock` 失败时记 warn 并立刻 `terminate_process_group` 兜底，绝不静默丢弃。`[已修]`

### [低] PTY spawn 的错误分支泄漏子进程（不 kill、不 wait）

- 文件：`src/chat/process/manager/spawn.rs:97`
- 错误原因：`spawn_command` 成功后，`try_clone_reader` / `take_writer` / 读线程 `spawn` 任一步失败
  都直接 `return Err`，此时 `child: Box<dyn portable_pty::Child>` 被 drop——portable-pty 的 Child
  没有 `kill_on_drop` 语义，没有 kill 也没有 `wait`，该 shell 成为 lx 的僵尸子进程。pipe 分支
  因为 `kill_on_drop(true)` 没有这个问题，两条路径处理不一致。
- 关键代码：

```rust
// src/chat/process/manager/spawn.rs:97
let child = pair.slave.spawn_command(builder).map_err(...)?;
drop(pair.slave);
let reader = pair.master.try_clone_reader().map_err(|error| {
    ProcessError::new("spawn_failed", format!("failed to read pty: {error}"))
})?;   // ← 此处的 ? 会丢掉仍存活的 child
```

- 修复建议：收拢成内部闭包，出错时先 `child.kill()` + `child.wait()` 再返回错误。`[已修]`

### [低] `notify_waiters()` 无粘性，检查与注册之间的新输出被拖到等待窗口末尾

- 文件：`src/chat/process/manager/mod.rs:247`（等待侧 `:674`）
- 错误原因：`Notify::notify_waiters()` 只唤醒「此刻已注册」的 waiter，不保存 permit。
  `wait_window` 在 `:635` 检查 `has_output`（释放 output 锁）到 `:674` 首次 poll `notified()`
  之间虽无 await 点，但另一线程（reader task / PTY 线程）完全可以在这个窗口里 `push_output`：
  通知丢失，task 睡到 deadline 才醒，`exec_command` 的首次输出要等满 `yield_time_ms`
  （默认 10 s）才返回。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:247
self.notify.notify_waiters();

// :674 —— 只唤醒已注册者，检不到的 permit 就丢了
_ = handle.notify.notified() => { … }
```

- 修复建议：改用 `notify_one()`（带 permit，等价于「已发生」）。`[已修]`

### [低] 持久化的 `output_start`/`output_end` 不是原始字节偏移

- 文件：`src/db/chat/process.rs:69`（写入处 `src/event/chat/process_events.rs:171`）
- 错误原因：两列与 `OutputSnapshot` 都声明是「原始字节偏移」，实际写入恒为 `0` 和
  `snapshot.tail.len()` —— 而 `tail` 是**已剥离控制序列、lossy 转成的文本**，长度与原始字节无关，
  甚至可能小于同一行的 `dropped_bytes`。任何「按库里的 offset 判断模型读到哪」的恢复逻辑都会算错。
- 关键代码：

```rust
// src/event/chat/process_events.rs:171
let snapshot_ref = OutputSnapshot {
    output_tail: output,
    output_start: 0,
    output_end: snapshot.tail.len() as i64,   // 文本长度 ≠ 原始偏移
    dropped_bytes: i64::try_from(snapshot.dropped_bytes).unwrap_or(i64::MAX),
};
```

- 修复建议：`ProcessSnapshot` 带上真实的 `output_start()/output_end()`；
  或把两列改名为 `tail_bytes` 并删掉偏移语义，别留一个会说谎的字段。`[待修]`（涉及 DB 列语义，
  当前无消费者，改动留给后续 schema 变更）

### [低] `Effect::DeleteChatSession` 先删进程记录再删会话，删除失败会留下不一致状态

- 文件：`src/db/service.rs:664`
- 错误原因：`delete_processes_for_session` 无条件先执行且错误被 `with_db` 吞成日志；若随后的
  `delete_chat_session` 失败，会话仍在但它的 `agent_process` 记录已被清空。之后打开该会话，
  卡片只能按不可变的 `result_json` 回放，`running` 的卡片永远显示 `Running` 且没有进程可停。
- 关键代码：

```rust
// src/db/service.rs:664
self.with_db(|db| db.delete_processes_for_session(&session_id));
let deleted = self.with_db(|db| db.delete_chat_session(&workspace, &session_id));
```

- 修复建议：外键已 `ON DELETE CASCADE`，删掉这次显式预删除。`[已修]`

### [中] Windows 上 `exec_command` 100% 不可用：默认 shell 是 `cmd.exe`，却一律按 POSIX 传 `-l`/`-c`

- 文件：`src/chat/process/manager/spawn.rs:29`（默认 shell 见 `src/platform/mod.rs:139`）
- 错误原因：`platform::default_shell()` 在 Windows 返回 `%COMSPEC%`（`cmd.exe`），但两条 spawn
  路径都不区分平台，硬编码 `-l` + `-c <cmd>`。`cmd.exe -l -c "echo hi"` 不认这两个参数，直接
  报错/退出，等于 Windows 上进程工具整体失效。另外 `#[cfg(unix)] command.process_group(0)`
  把 OS 专属代码写在了 `chat/` 层，违反仓库「OS 专属代码只允许出现在 `src/platform/`」的硬性规范。
- 关键代码：

```rust
// src/chat/process/manager/spawn.rs:29
let mut command = tokio::process::Command::new(shell);
if args.login {                       // ← Windows 的 cmd.exe 没有 -l
    command.arg("-l");
}
command
    .arg("-c")                        // ← Windows 需要 /C
    .arg(&args.cmd)
```

- 修复建议：在 `platform` 里提供 `shell_args(shell, login, cmd)`（unix: `-l -c`；windows: `/C`），
  `process_group` 也收进 platform。`[已修]`

---

## 二、事件与运行时层（`src/event/`、`src/chat/agent_loop/`、`src/input/`）

### [中] 会话切换/关窗格时，已提交 pending 的工具 DB 行永远停在 `pending`

- 文件：`src/event/chat/mod.rs:182`（`drop_pending`）、`:266`（`reap_stale_streams`）
- 错误原因：`begin_tool` 已把 `agent_tool_call` 写成 `pending` 并压入 `pending_begins`（可见窗格
  要等一帧才回执）。若这一帧内窗格被关、会话被切、`/new`，`reap_stale_streams` 调 `drop_pending`
  把 `PendingBegin` 整个丢掉，`reply` 随之 drop → loop 拿 `Err(RecvError)` 直接
  `return TurnSummary::interrupted`，**永远不会再调 `finish_tool`**。于是 DB 里那一行永久是
  `pending`：`settle_begin` 里精心写的 `cancel_pending(...)`（工具状态下唯一合法的
  pending→cancelled 落库路径）被完全跳过。用户重开该会话时卡片永久显示 `Pending`。
- 关键代码：

```rust
// src/event/chat/mod.rs:184
fn drop_pending(&mut self, key: &RequestKey) {
    self.pending_begins.retain(|pending| &pending.key != key);
}
```

（被跳过的正确路径在 `src/event/chat/tool_calls.rs:124`：`if active_assistant_id(state, &pending.key).is_none() { cancel_pending(...); reply.send(Err(Cancelled)); }`）

- 修复建议：`drop_pending` 不要移除条目，只把 `reply` 置 `None`，让下一帧
  `flush_pending_tool_begins → settle_begin` 走 `active_assistant_id == None` 分支完成
  `cancel_pending`。`[待修]`（需要把 `Persistence` 贯通到 `pump_chat_streams` 的清理路径，
  改动跨 3 个函数签名；本轮的 review 修复已覆盖同源的一批问题，但这一处涉及落库授权，
  留作独立改动以便单独评审）

### [中] 后台任务面板打开期间每轮都置脏 → 空闲时永久 60fps 重绘并每帧克隆全部进程快照

- 文件：`src/event/mod.rs:243`、`src/event/chat/mod.rs:361`
- 错误原因：`refresh_open_task_pages` 的返回值语义是「**压入了请求**」，不是「状态发生了变化」，
  但事件循环把它当变化用：`dirty = true`。面板一打开，每轮循环都压一次 `ListSessionProcesses`
  并置脏 → `wait` 永远被压到 `FRAME_INTERVAL`，空闲状态也按 60fps 连续 `tui.draw`，永不进
  `IDLE_WAIT`。每次执行列表效应还会 `manager.snapshots()` 克隆每个进程的完整 tail 文本
  （最长 64 KiB）并重建 `Vec<ProcessEntry>`；`on_process_listed` 传 `now_ms()`，而
  `set_processes` 的比较含 `self.now_ms != now_ms`（毫秒分辨率）→ 每次回投几乎必然「有变化」。
  用户观感：只要停在后台任务页，风扇就转。
- 关键代码：

```rust
// src/event/mod.rs:243
if chat::refresh_open_task_pages(state) {
    // 列表请求已入队，下一轮 `apply_process_effects` 会把它执行掉。
    dirty = true;                                    // ← 「已入队」不等于「画面要变」
}
```

- 修复建议：`refresh_open_task_pages` 只入队效应、不置脏；`set_processes` 的脏判定只比
  `processes`（`now_ms` 的推进由 `tick_task_pages` 单独负责）。`[已修]`

### [中] Esc 取消时正在等待的 `exec_command` 不会被停

- 文件：`src/event/chat/mod.rs:31`、`src/chat/agent_loop/executor.rs:51`
- 错误原因：停止语义是「本轮新建集合 ∪ 当前正在等待/交互的进程」。但 `TurnExecution::observe`
  只在 `execute()` **返回之后**才被调用，`created` 与 `active_interaction` 在 `exec_command`
  执行期间（默认 yield 10s、上限 30s）都是空的。用户在命令等待窗口里按 Esc：`stop_targets()`
  空集，什么都不杀；随后 loop 把该次调用按真实结果落库（进程状态 `running`），整轮以
  `Interrupted` 结束，**命令继续在后台跑**。
- 关键代码：

```rust
// src/chat/agent_loop/executor.rs:51
fn observe(&mut self, name: &str, result: &ToolExecutionResult) {
    ...
    self.created.insert(result.session_id);     // ← 只在工具返回后才登记
```

- 修复建议：在 `exec_command` **开始**时就登记活动项（spawn 成功后回调 `TurnExecution`），
  并让 `stop_targets` 把它算进去；`stop_turn_processes` 的读取改在事件循环内同步完成，
  消除「读之前工具已换目标」的竞态。
- 实际处理：`[已修]` 新增 `ProcessManager::exec_observed(…, on_spawn)`，spawn 成功后
  立刻回调新进程 id；`TurnExecution::observe_spawned()` 当场把它记进
  `created` / `active_interaction`。`stop_turn_processes` 改为在事件循环里**同步**
  取定 `targets` 再 `spawn` 停止任务。回归测试
  `a_command_waiting_in_its_yield_window_is_already_a_stop_target`。

### [中] 后台任务列表顺序取自 `HashMap` 迭代，刷新后高亮会指向别的进程，`^d` 可能停错进程

- 文件：`src/event/chat/mod.rs:427`
- 错误原因：`list_session_processes` 直接 `manager().snapshots()`，而 `snapshots()` 是
  `HashMap::values()` —— 顺序不稳定（插入触发 rehash 后会重排）。页面用 `window.active`
  **下标**记住高亮，`set_processes` 只做 `clamp`，不按 `session_id` 重新对齐。于是「另起一个
  进程」或「某个进程退出后刷新」就可能把高亮悄悄挪到另一行，此时用户按 `^d` 停掉的是**另一个**
  进程。这是破坏性操作，后果比顺序抖动本身严重。
- 关键代码：

```rust
// src/chat/process/manager/mod.rs:377
pub fn snapshots(&self) -> Vec<ProcessSnapshot> {
    lock(&self.processes)
        .values()                    // ← HashMap 顺序不稳定
        .map(|handle| handle.snapshot())
        .collect()
}
```

- 修复建议：`snapshots()` 出口按 `started_at_ms`（次键 `session_id`）排序后再交给面板。`[已修]`

### [中] `update_plan` 的 revision 每轮从 1 重新计数，`restore_plan` 按「最大 revision」恢复 → 重开会话恢复旧计划

- 文件：`src/chat/agent_loop/executor.rs:97`（消费方 `src/app/update/chat/process.rs:439`）
- 错误原因：`AgentTools::new` 每轮新建，`plan_revision` 随之从 0 起算，所以 revision 是
  「**本轮内**单调递增」而不是 `plan.rs` 文档写的「由运行实例单调递增」。而 `restore_plan` 在整段
  会话里取 revision 最大的那条：若第 1 轮调了 3 次 `update_plan`（revision 到 3）、第 2 轮只调
  1 次（revision 1），恢复时会保留**第 1 轮**的快照 —— 用户 `/resume` 回来看到上一轮已作废的计划。
- 关键代码：

```rust
// src/chat/agent_loop/executor.rs:97
plan_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),  // 每轮新建 → 归零

// src/app/update/chat/process.rs:440
Some(previous) if previous.revision > next.revision => previous,
```

- 修复建议：`restore_plan` 改成「按消息顺序取最后一条成功的 `update_plan`」，不依赖 revision
  的全局单调性（与 `commit_plan` 的实时语义完全一致）。`[已修]`

### [低] 输出通知在「先读快照、后清 pending」之间到达会被丢掉

- 文件：`src/event/chat/process_events.rs:77`
- 错误原因：`push_output` 用 `ui_pending` 做合并（已有未处理通知时不再发事件）。
  `handle_process_event` 先 `snapshot()` 读尾部、再 `acknowledge_output()` 复位标志。若新输出
  恰好落在这两步之间：`push_output` 看到 `ui_pending == true` 不发事件，紧接着标志被清掉——
  这一批输出既进了缓冲也没有任何后续事件通知 UI。对于「输出一段后长时间静默」的进程，卡片
  预览会永久停在那一段之前。
- 关键代码：

```rust
manager.acknowledge_output(session_id);
```

- 修复建议：先 `acknowledge_output` 再读快照（复位后到达的输出会重新发事件，最多多一次空转）。`[已修]`

### [低] turn 任务无 `JoinHandle`、取消信号 `let _ =` 吞错：loop panic 后界面永久 streaming

- 文件：`src/event/chat/mod.rs:161`、`:740`
- 错误原因：整轮 agent 任务是裸 `tokio::spawn`，句柄被丢弃，panic 被 tokio 静默吞掉，
  `report_turn` 不会执行 → 没有 `ChatNetEvent::Finished` → 窗格停在 `RequestState::Streaming`，
  转轮永转、输入框拒绝发送，用户除了 `/new` 没有出路。`cancel()` 又把 `watch::Sender::send`
  的错误 `let _ =` 掉——没有接收者时它**不会写入值**却仍然 `return true`，让调用方误以为
  取消已生效。
- 关键代码：

```rust
fn cancel(&mut self, key: &RequestKey) -> bool {
    let Some(cancel) = self.cancels.get(key) else { return false; };
    let _ = cancel.send(true);
    true
}
```

- 修复建议：`cancel()` 改成 `cancel.send(true).is_ok()`；任务侧套 `catch_unwind` 保证「无论
  loop 内部怎么结束，都必定回投一条 `Finished`」。`[已修]`

### [低] `ProcessRuntimes::get_or_create` 生产路径使用 `expect`

- 文件：`src/event/chat/runtime.rs:96`
- 错误原因：违反仓库硬性规范「生产代码禁止 `unwrap()` / `expect()`」。虽然 `insert` 与
  `get_mut` 之间无 await、逻辑上不可达，但它是事件循环主路径：一旦将来有人在两行之间插入
  其它 `&mut self` 调用，就会把整个 TUI 打成 panic。
- 关键代码：

```rust
self.runtimes.insert(session_id.to_string(), runtime);
}
self.runtimes.get_mut(session_id).expect("inserted above")
```

- 修复建议：用 `entry` API 消除二次查找。`[已修]`

---

## 三、应用状态与更新层（`src/app/`）

### [高] `[open]` / `^o` 的「定位到某进程」请求在进程列表到达之前就被消费掉了

- 文件：`src/app/update/chat/session.rs:127`
- 错误原因：`open_task_page_at` 先 `open_task_page`（此时新建的 `TaskPage` 里 `processes` 还是空，
  真正的列表要等 `Effect::ListSessionProcesses` 在**下一轮循环**里回投），然后**立刻**
  `page.focus = Some(id); page.resolve_focus()`。`resolve_focus` 第一行 `self.focus.take()` 就把
  一次性请求取走，紧接着发现 `processes.is_empty()` 直接 `return` —— 请求被丢弃。列表随后经
  `on_process_listed → set_processes` 到达时，没有任何人再消费 `focus`。结果：从卡片点 `[open]`
  或按 `^o`，高亮总是落在**第一个**进程上，输出区显示的也是第一条命令的日志。
- 关键代码：

```rust
// src/app/update/chat/session.rs:127
pub fn open_task_page_at(state: &mut AppState, pane: PaneId, session_id: Option<u64>) -> bool {
    let opened = open_task_page(state, pane);
    if let Some(session_id) = session_id
        && let Some(page) = target.lx.task.as_mut()
    {
        page.focus = Some(session_id);
        page.resolve_focus();                      // ← focus 在这里被 take + 丢弃
    }
    opened
}

// src/app/state/lx/task.rs:184
pub fn resolve_focus(&mut self) {
    let Some(session_id) = self.focus.take() else { return; };   // ← 先 take
    ...
    if self.processes.is_empty() {
        return;                                    // ← 请求已丢失
    }
```

- 关键点：一次性请求的「落地时机」与「数据到达时机」脱耦，且 `take()` 在判空之前。
- 修复建议：`open_task_page_at` 只写 `page.focus`，把 `resolve_focus()` 挪到列表到达时
  （`on_process_listed` 里 `set_processes` 之后）；`resolve_focus` 里先判空再 `take`。
  `[已修]`

### [中] 输出可见行数一变就无条件 `pin_output_to_tail()`：改变窗口大小时把正在上翻的用户拽回底部

- 文件：`src/app/update/chat/mod.rs:885`
- 错误原因：`resolve_task_history_visible` 在 `page.output_visible != output_visible` 时直接
  `pin_output_to_tail()`，不看用户是否手工上翻过。`output_visible` 由 `task_output_rows` 推出，
  等于 `inner.height - task::list_height()`：只要用户改变窗口大小（哪怕只多/少一行），这个值
  就变 → `output_scroll` 被重置成新的 `max` → 用户正读到日志中间，一拉窗口就被甩到最新一屏。
- 关键代码：

```rust
// src/app/update/chat/mod.rs:885
if page.output_visible != output_visible {
    page.output_visible = output_visible;
    // 可见行数变了要重新贴尾：否则窗口缩小后视口可能停在越界位置。
    page.pin_output_to_tail();          // ← 只该做越界钳制
    changed_here = true;
}
```

- 关键点：`set_output` 已专门为这个 bug 加了 `was_at_tail` 判据，这里又开了一个后门。
- 修复建议：只做「保持内容位置的钳制」，只有原本贴尾时才跟到新尾部。`[已修]`

### [中] `/new` 不清理 `chat.plan`（及 `selected_tool` / `detail_expanded`）：新会话把上一会话的计划注入下一轮模型上下文

- 文件：`src/app/update/chat/session.rs:16`
- 错误原因：`new_session` 清了 `messages` / `follow_bottom` / `tokens` / `thinking_toggled` /
  `turn_started_at_ms`，但没有清 `plan`、`selected_tool`、`detail_expanded`。`plan` 不只是展示：
  `start_stream` 会 `pane.lx.chat.plan.clone()` 并拼进 `runtime_context_summary` 作为本轮 system
  上下文。于是 `/new` 之后第一轮，模型收到「current plan: 上一个会话的步骤…」，把上个会话的
  待办当成本会话的既有计划继续执行。对比 `on_session_loaded` 是清这三个字段的，两条路径口径不一致。
- 关键代码：

```rust
// src/app/update/chat/session.rs:16
target.lx.chat.session_id = None;
target.lx.chat.messages.clear();
target.lx.chat.follow_bottom = true;
target.lx.chat.input_tokens_reset();
target.lx.chat.thinking_toggled.clear();
target.lx.chat.turn_started_at_ms = None;
// ← 缺 plan / selected_tool / detail_expanded
```

- 修复建议：与 `on_session_loaded` 对齐，补 `plan = None; selected_tool = None;`
  `detail_expanded.clear();`。`[已修]`

### [中] 切换 tab 用「新 tab 的条目数」去 clamp 进程列表窗口：Agents 的 `len()==0` 把高亮/锚点/视口全部清零

- 文件：`src/app/state/lx/task.rs:143`
- 错误原因：`TaskPage` 只有**一份** `window`，却被两个 tab 共用，而 `clamp(self.len())` 的 `len()`
  是**按当前 tab** 算的（Agents 恒为 0）。于是 `→` 切到 Agents 时 `clamp(0)` 把
  `active/anchor/viewport` 全抹成 0，`←` 切回来时用户原来选的进程已经丢了，`^d` 会停掉列表首项
  而不是原来高亮那项。更糟的是 `set_processes` 无条件 `self.window.clamp(self.len())`，而
  `on_process_listed` 在面板打开时每轮都会执行 ⇒ 停在 Agents tab 期间**每帧**被清零。
- 关键代码：

```rust
// src/app/state/lx/task.rs:143
self.tab = tab;
self.window.clamp(self.len());   // Agents => clamp(0)，把 Processes 的窗口状态抹掉
```

- 修复建议：窗口状态用于进程列表，`clamp` 时用 `self.processes.len()`（不是当前 tab 的 `len()`）。`[已修]`

### [低] `set_task_output_scroll` 复制了一份 `output_max_scroll` 的算法

- 文件：`src/app/update/chat/session.rs:202`
- 错误原因：手写 `page.output.len().saturating_sub(page.output_visible)`，与
  `TaskPage::output_max_scroll()` 是同一个量的**第二套口径**。任何一处改公式，都会让
  「键盘滚到哪停」和「拖拽滚到哪停」分叉。同文件 `on_task_output` 的文档也自相矛盾：写「内容一变
  就贴回尾部」，而 `set_output` 的实现明确是「不贴回尾部」（正是用户报过的 bug），后人照文档改
  会直接把 bug 改回来。
- 关键代码：

```rust
// src/app/update/chat/session.rs:202
let max = page.output.len().saturating_sub(page.output_visible);
let next = offset.min(max);
```

- 修复建议：`output_max_scroll()` 提升为 `pub(crate)` 并直接调用；删掉文档里「内容一变就贴回尾部」
  的错误表述。`[已修]`

### [低] 关闭后台任务面板不清 `task_scroll_drag`：残留的抓取偏移会吞掉后续所有拖拽

- 文件：`src/app/update/chat/session.rs:140`
- 错误原因：`close_task_page` 只 `task.take()`，不碰全局 `state.task_scroll_drag`。若在按住输出区
  滚动条 thumb 时按 `Esc` 关面板，`task_scroll_drag` 仍为 `Some(grab)`；`drag.rs` 的该分支在面板
  已不存在时 `task_rects(...)` 全返回 `None`，循环结束后 `return` ——**所有**后续拖拽事件
  （窗格分割线、选区、prompt 滚动条）都被这一支吞掉，直到下一次左键释放。
- 关键代码：

```rust
// src/app/update/chat/session.rs:140
pub fn close_task_page(state: &mut AppState, pane: PaneId) -> bool {
    ...
    target.lx.task.take().is_some()   // task_scroll_drag 未清理
}
```

- 修复建议：`close_task_page` 里 `state.task_scroll_drag = None;`（`toggle_pane_view` 对
  `lx_scroll_drag` / `terminal_scroll_drag` 已有同样的清理）。`[已修]`

### [低] `PanelWindow` 对任务页是「死的三字段」：`viewport` 从不写入，`anchor` 从不读取

- 文件：`src/app/state/lx/task.rs:87`
- 错误原因：`ui/.../task.rs::window_start` 里 `if let Some(start) = page.window.viewport` 那条分支
  永远走不到——`move_active` 与 `resolve_focus` 都把它置 `None`，全仓库没有任何地方给任务页的
  `window.viewport` 赋 `Some`。因此进程数超过 `LIST_MAX`(6) 时，鼠标滚轮在列表上**完全无效**：
  落到下面的 `scroll_lx_input`，白白滚动一个不可见的输入缓冲并置脏。
- 关键代码：

```rust
// src/ui/main_content/lx/task.rs:440
fn window_start(page: &TaskPage, visible: usize) -> usize {
    if let Some(start) = page.window.viewport {   // ← 永远 None
        return start.min(len.saturating_sub(visible));
    }
```

- 修复建议：在 `event/mouse/wheel.rs` 的列表区补 `wheel → page.scroll_window(delta, visible)`。`[已修]`

### [低] 面板里 `Delete` 键 = `^d`，按物理删除键会直接杀掉后台进程（无确认）

- 文件：`src/app/update/pane.rs:790`
- 错误原因：`input/mod.rs` 把 `Ctrl+D` 和裸 `KeyCode::Delete` 都映射成 `LxInput::Delete`；
  `apply_task_page_input` 把 `LxInput::Delete` 一律当「停止高亮进程」处理。用户在列表里习惯性按
  Delete 想「删掉这一行」，实际是 SIGTERM 一个正在跑的服务，只回一个 Info toast，不可撤销。
- 关键代码：

```rust
// src/app/update/pane.rs:790
// `^d` 停止高亮进程（对齐 opencode `composer.shell.kill`）。
LxInput::Delete => {
    super::chat::stop_selected_process(state, pane);
}
```

- 修复建议：新增 `LxInput::StopProcess`（仅 Ctrl+D 映射），面板内裸 `Delete` 忽略。`[已修]`

### [低] 两处不必要的 `as` 窄化转换

- 文件：`src/app/update/chat/session.rs:349`、`src/app/update/chat/process.rs:328`
- 错误原因：`(input + output) as u64` 是「i64 加法（debug 下溢出会 panic）再 as 成 u64」，而
  同文件其它地方都在用 `.max(0)` / `saturating_*`；`record.process_session_id == session_id as i64`
  是 u64→i64 位模式转换。属隐患而非现网 bug，但本仓库明令避免。
- 关键代码：

```rust
// src/app/update/chat/session.rs:349
(input + output) as u64
```

- 修复建议：`u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX)`；
  `i64::try_from(session_id).ok() == Some(record.process_session_id)`。`[已修]`

---

## 四、UI 渲染与交互层（`src/ui/`）

### [高] 面板打开时 `chat_hit` / `code_copy_hits` 仍按隐藏的历史几何命中，输出区点击穿透

- 文件：`src/ui/main_content/lx.rs:838`（另见 `:914` `code_copy_hits`）
- 错误原因：后台任务面板打开后，`task_output` 与 `history_rect` **是同一块矩形**
  （`layout` 里 `bottom = input.y`），`build_chat` 只是通过 `if lay.task_output.is_some()` **不画**
  历史，`history::view` 与 `history::layout` 的几何照常成立。而 `chat_hit` 开头只判
  `messages.is_empty()`，没有判 `lx.task`；`press.rs` 里 `task_hit` 只覆盖 item 列表的 tab 行，
  其余落点会一路走到 `chat_hit`。于是点击输出容器任意一行，都可能命中**看不见的**历史行：
  置顶行 → `PromptHeader`、thinking 头部 → `Thinking`、代码块 `[❐]` → `CodeCopy`、
  `[open]` → `ToolOpen`。同一根因还让 `tooltip::target_at` 在输出区弹出 "copy code" 提示。
  这是前几轮 `entry_hit` / `chat_bar` 那类穿透 bug 的最后遗漏项。
- 关键代码：

```rust
// src/ui/main_content/lx.rs:838
pub fn chat_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<LxChatHit> {
    if lx.chat.messages.is_empty() {
        return None;                 // ← 缺 `lx.task.is_some()` 的护栏
    }
```

- 关键点：「画数据 → 命中数据」必须成对失效。面板打开时历史不画，所有基于历史几何的命中也要一起失效。
- 修复建议：`chat_hit` / `code_copy_hits` / `history_scrollbar` / `history_wheel_region` 入口加
  `if lx.task.is_some() { return None / Vec::new() }`。`[已修]`

### [高] 计划卡片「进行中」步骤用了完成态翡翠绿，与「运行中必须橙黄」的规则直接冲突

- 文件：`src/ui/main_content/lx/history/tool/plan.rs:146`
- 错误原因：`PlanStepView::InProgress` 取 `style::chat_tool_verb()`，它正是工具**已完成**的翡翠绿
  `Indexed 42`；而 `style.rs` 的文档明确写着「**所有**正在运行的工具（文件工具、`exec_command`、
  `write_stdin`、`update_plan`）的方块与动词统一用橙黄 208，只有 `Completed` 才回翡翠绿」。
  同一屏里一张运行中的 `read` 卡片是橙黄，而计划里真正在推进的那一步却是绿的，读起来像
  「已经做完了」。
- 关键代码：

```rust
// src/ui/main_content/lx/history/tool/plan.rs:146
let step_style = match status {
    PlanStepView::Completed => style::agent_step_detail(),
    PlanStepView::InProgress => style::chat_tool_verb(),   // ← 完成态的绿
    PlanStepView::Pending => style::text(),
};
```

- 修复建议：`InProgress => style::chat_tool_verb_running()`。`[已修]`

### [中] `plan::draw` 的解释行把 `row_index += 1` 写进了 `visible_row` 分支内，滚过卡片后整段内容上移一行

- 文件：`src/ui/main_content/lx/history/tool/plan.rs:181`
- 错误原因：其它所有绘制路径（`body.rs`、`tool.rs`、`command.rs`、`notice.rs`）都是「不论是否
  可见都推进行号」，只有解释行把推进放进了 `let Some(y) = visible_row(...)` 里。当这张计划卡的
  解释行滚出视口时，`row_index` 少加 1，其后的空行、统计行、下一条消息全部**比 `history::layout`
  认为的位置高一行**；于是滚动条总行数、置顶行判定、以及后面所有 thinking / `[open]` / code-copy
  的命中矩形都错开一行（点可见的 `[open]` 没反应、点它下面一行才触发）。
- 关键代码：

```rust
// src/ui/main_content/lx/history/tool/plan.rs:181
if let Some(explanation) = explanation
    && let Some(y) = visible_row(rect, scroll, *row_index)
{
    let segments = explanation_line(&explanation, width);
    put_segments(grid, rect.x, y, width, &segments);
    *row_index += 1;   // ← 只在可见时推进
}
```

- 修复建议：`put` 放进 if，`*row_index += 1;` 放到 if 之后无条件执行。`[已修]`

### [中] 空计划 + 非空解释时 `rows()` 比 `draw()` 多算 1 行，其后所有内容绘制与命中错位

- 文件：`src/ui/main_content/lx/history/tool/plan.rs:45`（绘制侧 `:120`）
- 错误原因：`rows()` 的公式是 `title_rows() + step_rows(...) + usize::from(explanation.is_some())`，
  **没有排除空计划**；而 `draw()` 在 `view.is_empty()` 分支画完 `Plan cleared` 就 `return`，
  从不画解释行。模型发一次 `{steps: [], explanation: "..."}`（清空/收尾计划时很常见）就会让布局
  多出 1 行：总行数、滚动上限、置顶行、以及该卡片之后所有步骤的命中矩形统统一行偏移。
- 关键代码：

```rust
// src/ui/main_content/lx/history/tool/plan.rs:45
title_rows() + step_rows(view, width) + usize::from(explanation_of(view).is_some())
```

- 修复建议：解释项改成 `usize::from(!view.is_empty() && explanation_of(view).is_some())`。`[已修]`

### [中] 底边框后台任务计数的宽度判断漏了左右边距与空档

- 文件：`src/ui/main_content/lx/status.rs:72`
- 错误原因：`keep_tasks = inner.width > task_w + MIN_MODEL` 把可用宽度直接按 `inner.width` 估，
  实际右侧要占 `task_w + GAP`、左侧还要扣 `EDGE_PAD * 2`，判断比真实可用宽度宽了 3 列。
  以 `1 process`（`task_w = 9`）为例：`inner.width = 13..25` 时 `keep_tasks` 成立，但
  `avail = inner.width - 12` 只有 1..13 列，小于 `MIN_MODEL + ctx_w`，`layout` 于是判
  `keep_ctx = false`（意图是「丢弃上下文、保留模型名」）；可 `render` 完全不看这个结论，
  `model_available_width` 仍然无条件为上下文预留列，把 `label_avail` 压到 0，模型名被剪成空串，
  随后 `if let Some(context) = context.filter(|_| x < limit)` 又把 `" · 100(50%)"` 截半画上去。
  用户看到的是「模型名消失、底边框尾部一段莫名其妙的 `· 10`」。
- 关键代码：

```rust
// src/ui/main_content/lx/status.rs:72
let keep_tasks = task_w > 0 && inner.width > task_w.saturating_add(MIN_MODEL);
let avail = end.saturating_sub(left).saturating_sub(if keep_tasks {
    task_w.saturating_add(GAP)
} else {
    0
});
```

- 修复建议：阈值按真实可写宽度比较（`end - left > task_w + GAP + MIN_MODEL`）；同时 `render`
  以 `layout` 的结论为准（`lay.context.width == 0` 时不画上下文），让「是否显示上下文」只有一份真相。`[已修]`

### [中] 命令卡片的进程状态词恒为 dim 灰，`chat_step_label_alive` 从未接线

- 文件：`src/ui/main_content/lx/history/tool.rs:411`
- 错误原因：`header_with_mark` 把 `■ 动词 · 状态` 里的状态段写死成 `style::agent_step_detail()`。
  设计文档要求「进程仍存活时……方块加粗、**动词与状态文字同色不加粗**」，`style.rs` 也专门提供了
  `chat_step_label_alive()` 并注明它是「工具运行中的动词**与状态文字**」。结果是
  `■ Run command · Running` 里方块与动词橙黄、状态词却是灰的，一张卡片三种观感。
- 关键代码：

```rust
// src/ui/main_content/lx/history/tool.rs:411
let tail = Segment::new(
    format!("{}{status}", text::LX_CHAT_TOOL_STATUS_SEP),
    style::agent_step_detail(),      // ← 恒 dim，运行中也是灰
);
```

- 修复建议：状态段按 `mark` 取色（`Alive | Running => chat_step_label_alive()`，终态保持 dim）。`[已修]`

### [中] 面板打开时历史滚动条的命中仍然生效：点输出容器右侧那一列会滚动看不见的历史

- 文件：`src/ui/main_content/lx.rs:798`
- 错误原因：`history_scrollbar` 只依赖 `history_view`，不知道历史有没有被画出来。`press.rs` 的
  落点链是「输出容器滚动条 → tab/close → 提及 → `chat_hit` → … → `handle_lx_history_scrollbar_press`」。
  当输出**没有溢出**（选中进程只吐了几行）而历史很长时，输出区右端那 1 列滚动条槽会掉进历史
  滚动条分支：轨道点击 → 改动 `history_scroll`/`follow_bottom`；按在 thumb 区 → 设置
  `lx_history_scroll_drag`，之后一路滚隐藏历史。用户表现是「点输出区右边缘什么都没发生」，
  关掉面板后消息列表莫名跳到中间并浮出 `↓ jump to bottom`。
- 关键代码：

```rust
// src/ui/main_content/lx.rs:798
pub fn history_scrollbar(area: Rect, lx: &Lx) -> Option<scrollbar::ScrollbarLayout> {
    let view = history_view(area, lx)?;      // ← 不知道历史没被画
    ...
}
```

- 修复建议：`history_scrollbar` / `history_wheel_region` 在 `lx.task.is_some()` 时返回 `None`。`[已修]`

### [低] 无消息时走 `build()` 分支，面板打开也会画出一个空输入框（不画面板）

- 文件：`src/ui/main_content/lx.rs:636`
- 错误原因：`build()`（欢迎页路径）完全没有看 `lx.task`，仍无条件 `input::draw(...)`，于是画出
  上下两条 `─` 和 `Ask anything…` 占位，而 `task_output` / `task_list` 只在 `build_chat` 里画。
  一旦出现「面板打开 + 消息被清空」，画面就是「空输入框 + 无面板 + 按键全被 `apply_task_page_input`
  吞掉」的死状态。
- 关键代码：

```rust
// src/ui/main_content/lx.rs:636
chat_bar::draw(&mut grid, lay.chat_bar, lx);
let mut cursor = None;
if let Some(input) = lay.input {
    cursor = input::draw(&mut grid, input, lx, focused, ...);   // ← 没看 lx.task
}
```

- 修复建议：`build()` 在 `lx.task.is_some()` 时与 `build_chat` 对齐（画面板，不画输入框）。`[已修]`

### [低] `task::put_buf` 把 `CONTINUATION`（`'\0'`）直接写进 `Buffer`

- 文件：`src/ui/main_content/lx/task.rs:486`
- 错误原因：`CONTINUATION` 是**网格层**的占位标记，`render()` 的 blit 会对它 `set_diff_option(Skip)`
  跳过。但 `put_buf` 是直接写 `ratatui::Buffer` 的路径（`draw_footer` 用），它把宽字符的后续列
  `set_char(CONTINUATION)`，没有 `Skip` 也没有设成空格——ratatui 会把这格当作普通字符输出。
  目前底边框键位提示全是 ASCII，所以暂时打不到；但只要有人给 `Hint` 加中文/emoji 就会立刻可见。
- 关键代码：

```rust
// src/ui/main_content/lx/task.rs:486
for offset in 1..width {
    if let Some(cell) = buf.cell_mut((cx.saturating_add(offset), y)) {
        cell.reset();
        cell.set_char(CONTINUATION);     // ← 网格私有标记溢出到 Buffer
        cell.set_style(style);
    }
}
```

- 修复建议：直接写 Buffer 的路径写 `' '`（`history::put_buffer_line` 就是这么做的）。`[已修]`

### [低] `task::hit_in` 收了 `output` 与 `page` 却不使用，文档还承诺输出容器上的 `[esc]`

- 文件：`src/ui/main_content/lx/task.rs:318`
- 错误原因：`hit_in(rect, _output, _page, …)` 三个参数里两个被忽略；`TaskHit::Close` 的文档仍写
  「右上角的 `[esc]`」、模块头注释的示意图也把 `esc` 画在输出容器右上角，而实际 `tab_row_rects`
  把它画在 item 列表的 tab 行右端。接口形状与文档都在描述一个已经不存在的可点区域。
- 关键代码：

```rust
/** ~行 318 **/
pub(crate) fn hit_in(
    rect: Rect,
    _output: Option<Rect>,      // ← 空转
    page: &TaskPage,
    column: u16,
    row: u16,
) -> Option<TaskHit> {
    hit(rect, page, column, row)
}
```

- 修复建议：删掉 `_output` 参数（调用点同步收窄），文档改成「item 列表 tab 行右端」，
  合并相邻两条重复的 doc 注释。`[已修]`

---

## 五、整体评价

**分层纪律总体扎实。** `app/`（纯状态）与 `ui/`（纯渲染）的边界在本轮改动中基本守住了：`TaskPage`
只存纯数据，输出/进程数据的唯一来源都在事件层，`task_rects` / `task_output_rows` /
`output_scrollbar` 确实都从同一份 `layout` 派生，「几何只有一份真相」这一条比前几轮明显改善；
`output.rs`（偏移与文本化）、`plan.rs`（全量替换语义）、`ProcessError`（稳定错误码）是这套代码
最扎实的部分。「真实退出码只来自 `wait`」「非零退出不是失败」「读端 EOF ≠ 退出」这几条不变量
也确实被贯彻了。

**系统性问题有三类：**

1. **生命周期与计数的所有权不清。** `settle()` 同时承担状态转移、全局计数、UI 通知三件事，
   却被多条前置条件不同的路径调用，于是同时产生「未计数就递减」（计数下溢，全进程 `exec_command`
   失效）与「已终态又被写回 Running」（计数泄漏）两个相反方向的 bug。同一根因还体现在
   `stop` 通道把「已认领」与「无 supervisor」压进一个 `Option`。
   **修法**：给 `settle` 引入显式 `counted` 状态位，给停止路径引入 `stop_requested` 位。

2. **「进程内从 0 开始」的标识被当成跨进程/跨会话唯一。** `GLOBAL_RUNNING`、`plan revision`、
   `runtime_epoch`、进程 `session_id` 四个标识都有这个问题。其中进程 `session_id` 已在多会话
   并存时**写坏另一个会话的库行**，属于必须修的架构性缺口。
   **修法**：id 源一律改成进程内全局；跨重启的 epoch 加入启动时刻成分；依赖「全局单调」的
   消费方（`restore_plan`）改为「按时间线取最后一条」。

3. **「失效面」不完整。** 这是本分支用户反复报 bug 的集中地：几何算对了，但「面板打开时哪些
   命中该消失」只补了 `entry_hit` 与 `chat_bar`，`chat_hit` / `code_copy_hits` / 历史滚动条
   三处仍按隐藏历史的几何作答，输出区因此成了最大的点击穿透面。
   **修法**：把「面板打开 ⇒ 历史相关渲染与命中一起失效」这条规则收进一个判定函数，
   所有基于历史几何的入口统一调用它，而不是每个入口各写一遍。

**次要但会持续咬人的：「快照语义不诚实」。** `tail_text()` 名为尾部实为头部、`process_events`
把文本长度当原始字节偏移写库、`max_output_tokens` 只用于早退——单点都不致命，但会让 UI 与模型
对「我看到的输出是最新的吗 / 这就是我要的量吗」持续误判。建议把「模型游标」「UI 预览窗口」
「落库快照」三者的取数函数统一到 `OutputBuffer` 上一次实现（`read_tail(n)` /
`read_from(cursor, n)` / `snapshot_range()`）。

**测试分布的盲区**（本轮已补）：现有用例刚好绕过了本清单里几个最容易复现的 bug —— 没有
「计划卡片的滚动几何」断言（所以解释行行号错位没被发现）、没有「多任务计数下的窄窗格状态」用例
（所以底边框模型名被挤没没被发现）、`open_from_card_focuses_that_process` 因为先
`open_with_processes` 才通过（掩盖了 `[open]` 定位失效）。补测试时应优先覆盖这三类「先构造
前置状态再触发」的场景，而不是只断言最终文案。

---

## 六、本轮修复清单

按文件归并，便于对照 diff。

| 层 | 文件 | 修复内容 |
|---|---|---|
| 内核 | `chat/process/manager/mod.rs` | `counted` 位修正全局计数下溢；`mark_running` 条件转移；`cursor_lock` 串行化模型游标；`tail_text` 改读真正的尾部；`max_output_tokens` 参与预算；`stop_requested` 区分「已认领」与「无 supervisor」；`snapshots()` 按启动时刻排序；`prune_finished` 保留 `background_alive`；`shutdown` 先广播再统一等待；`pid` 指纹校验（惰性）；`exec_observed` spawn 回调 |
| 内核 | `chat/process/manager/spawn.rs` | `register_stop_channel` 抢锁失败兜底终止而非丢弃；PTY 各失败分支 `kill + wait`；shell 参数改走 platform |
| 内核 | `chat/process/output.rs` | `Escape::{Complete,Pending,Overlong}` 三态，未闭合序列不消费（与不完整 UTF-8 同策略） |
| 内核 | `platform/mod.rs` | `shell_arguments()`（POSIX `-l -c` / Windows `/C`）、`process_start_time()` |
| 内核 | `event/chat/runtime.rs` | epoch 加进程启动毫秒前缀（跨重启唯一）；`entry` 消除 `expect` |
| 内核 | `db/service.rs` | 去掉删会话前的进程记录预删（靠外键级联） |
| 事件 | `event/mod.rs` | 面板刷新不再每轮置脏（消除 60fps 空转） |
| 事件 | `event/chat/process_events.rs` | 先复位 `ui_pending` 再读快照（不丢输出通知）；落库写真实偏移 |
| 事件 | `event/chat/mod.rs` | `cancel()` 上报发送失败；`stop_turn_processes` 同步取目标 |
| 事件 | `event/mouse/wheel.rs` | 补 item 列表滚轮分支 |
| 状态 | `app/state/lx/task.rs` | `resolve_focus` 先判空再 `take`；窗口 `clamp` 用进程数而非 tab 数；`set_processes` 只在列表变化时置脏；`resize_output_viewport` 保持阅读位置；`scroll_window`；`output_max_scroll` 提升可见性；删两个死字段 |
| 状态 | `app/state/lx/chat.rs` | `reset_session_view()` 统一会话级复位（含 `plan`） |
| 状态 | `app/update/chat/session.rs` | `[open]` 定位改到列表到达时；`/new` 与删除共用复位；`scroll_task_list`；`close_task_page` 清拖拽标记；复用 `output_max_scroll` |
| 状态 | `app/update/chat/mod.rs` | `resolve_task_history_visible` → `resolve_task_output_visible`（不再每帧跑 markdown 排版、不再无条件贴尾） |
| 状态 | `app/update/chat/process.rs` | `restore_plan` 改按时间线取最后一条；`i64::try_from` 替代 `as` |
| 状态 | `app/actions.rs` / `input/mod.rs` / `app/update/pane.rs` | 新增 `LxInput::StopProcess`（仅 Ctrl+D），裸 `Delete` 不再误杀进程 |
| UI | `ui/main_content/lx.rs` | `chat_geometry_is_live()` 统一守卫 `chat_hit` / `code_copy_hits` / `history_scrollbar` / `history_wheel_region` / `sticky_prompt_jump`；`build()` 面板路径 |
| UI | `ui/main_content/lx/history/tool/plan.rs` | `InProgress` 改橙黄 + 标记按状态取色；解释行无条件推进行号；空计划不折算解释行 |
| UI | `ui/main_content/lx/history/tool.rs` | 运行中状态词与方块同档 |
| UI | `ui/main_content/lx/status.rs` | 任务计数阈值按真实可写宽度；上下文以 `layout` 结论为唯一真相 |
| UI | `ui/main_content/lx/task.rs` | `put_buf` 写空格而非 `CONTINUATION`；`hit` 去掉空转参数；`LIST_MAX` 可见性 |

### 新增回归测试

| 测试 | 锁住的行为 |
|---|---|
| `failed_spawn_never_underflows_the_global_running_counter` | spawn 失败不再破坏全局计数 |
| `a_short_lived_command_never_stays_running` | 秒退命令的终态不被写回 `Running` |
| `process_ids_are_unique_across_managers` | 进程 id 跨管理器唯一（防事件串台） |
| `tail_snapshot_tracks_the_newest_output_past_the_budget` | 快照跟着尾部走 |
| `escape_sequence_split_across_reads_is_not_swallowed` | 跨边界转义序列不被吞、不泄漏成正文 |
| `unflushed_split_utf8_and_split_escape_behave_the_same_way` | 两种「数据不够」同策略 |
| `task_panel_kills_every_history_hit` | 面板打开时历史相关命中全部失效 |
| `resizing_the_window_keeps_a_manually_scrolled_output_position` | 改窗口不拽回底部 |
| `focus_on_empty_list_keeps_the_request` | `[open]` 定位请求留到列表到达 |
| `task_panel_hides_the_jump_to_bottom_bar` 等 | 聊天栏随面板一起消失 |
| `switching_tabs_keeps_the_process_highlight` | 切 tab 往返不丢高亮 |
| `new_session_clears_the_previous_plan` | `/new` 不把计划带进新会话上下文 |
| `ctrl_d_stops_the_highlighted_process` | 停止只由 Ctrl+D 触发 |
| `a_command_waiting_in_its_yield_window_is_already_a_stop_target` | 等待窗口内的命令可被 Esc 停掉 |
| `plan_in_progress_step_uses_the_running_amber` | 计划「进行中」是橙黄 |
| `empty_plan_with_an_explanation_keeps_rows_and_draw_in_step` | 空计划 + 解释的行数口径 |
| `plan_row_indices_advance_even_when_rows_scroll_out_of_view` | 滚出视口仍推进号（命中不错位） |
| `tail_snapshot_keeps_up_past_the_model_budget` | 落库/预览的尾部口径 |

### 一条值得记下来的教训

本轮有三处修复是**被测试逼着改回来**的，值得写在规范里：

1. **热路径上不要做同步外部调用。** 为防 pid 复用，最初在 `set_pid()`（spawn 之后
   立刻执行）里同步调 `ps` 取指纹。功能正确，但把「spawn → 进入等待」拖慢，
   导致 reader 任务 drain 落后于进程结算 —— `echo hello` 的输出变空。并行跑测试时
   稳定复现 3~4 条失败。改成惰性取指纹后立刻恢复。
2. **`Notify` 用 `notify_waiters()` 而不是 `notify_one()`。** 后者会把 permit 留在
   通道里，后续每次 `notified()` 都立刻返回，让 `wait_window` 变成忙等。
3. **「一次性请求」不要在数据未到达时消费。** `resolve_focus()` 里 `take()` 早于判空，
   请求被静默吃掉；这类 bug 单测很难写（要「先构造空数据、再补数据」），
   而现有测试因为「先塞数据再调用」恰好绕过了它 —— 测试的前置构造顺序本身
   就是一种覆盖率。
