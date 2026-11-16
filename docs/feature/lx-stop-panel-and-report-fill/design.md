# 主 agent 停止面板、用户主动打断语义与 subagent/BTW 报告填充 — 详细设计

- **状态**：已实施完成（分支 `feature/lx-stop-panel-and-report-fill`）；实施差异见 `task.md` 顶部。
- **范围**：主 agent 双击 Esc 停止面板；后台任务页 `^d` 的用户主动停止语义；Agents / BTW 面板 `/report`、报告二级面板与主输入框折叠芯片；两类面板的 `/rewind` 报告撤销。
- **不涉及**：数据库 schema 迁移、模型自动恢复策略之外的 `interrupt_agent` 行为、AI Web 入口、BTW 的 `^d` 停止入口。

---

## 0. 代码库现状核验

以下均基于当前 `dev` 工作区源码（codegraph / codebase-memory / 直接阅读）。

| 事实 | 当前实现 |
|---|---|
| 双击 Esc 有两套语义 | `src/app/update/pane.rs` 先走 `rewind::apply_input`；空闲空输入第一次 Esc 进入 `/rewind` 待命（`src/app/update/chat/rewind.rs:130-168`），忙时 `arm()` 返回 `None`，落到 `escape_stop`（`src/app/update/chat/mod.rs:725-764`） |
| 忙时第二次 Esc | 立即停止主轮（`stop_stream`）+ 全部用户 shell（`cancel_user_shells`），**不停 subagent** |
| 停止 subagent 的入口 | 后台任务页 `Agents` 页签 `^d`（`src/app/update/pane/task_page.rs:210`、`src/app/update/chat/task_page.rs:998`），发 `Effect::CancelSubagents { agent_id: Some(..) }` |
| 当前 `^d` 的事件语义 | `cancel_for_session`（`src/event/chat/subagent/mod.rs:760-790`）标记 `closed_without_report=true`、清 `resumed/paused/interrupted`、取消在途 handle；结算时不产出报告，也没有“用户主动停止”的独立信号 |
| 主 agent 恢复闸门 | `send_input` 只接受 running 或 resumed（`src/event/chat/subagent/runner.rs:408-415`）；面板追问 `send_user_input_from_panel` 遇到 `cancelled` 直接拒绝（`src/event/chat/subagent/snapshot.rs:429-451`） |
| `wait_agent` 结果 | `AgentStatusView` 只有 `agent_id/nickname/agent_path/title/role/status/report`（`src/chat/agent/mod.rs:472-490`），无法区分“用户主动停止”和普通 cancelled |
| 当前 `/report` | 只在 Agents 详情 composer；`detail_report::submit` 发 `SendSubagentUserInput { notify_parent: true }`（`src/app/update/chat/detail_report.rs:78-84`），报告结算后由 `drain_reports` 自动起父轮（`src/event/chat/subagent/settle.rs:400-490`） |
| 当前 `/rewind` 报告轮 | `latest_undoable_private_turn` 要求 `notify_parent == 0`（`src/db/chat/subagent/queue.rs:741-743`），因此旧 `/report` 轮不可撤回 |
| BTW `/report` | 不存在；BTW 命令集为 `/rewind` `/compact` `/new`（`src/app/state/lx/slash/composer.rs:26-28`） |
| BTW 默认目标 | 无 `@` 时 `targets` 为空，由 BTW 自己的默认助手回答 |
| BTW 多目标 | `resolve_targets` 收集全部 `@subagent` 引用（`src/app/update/chat/btw/mod.rs:590-620`）；无新 `@` 时 `previous_targets` 沿用整个目标集合（`:558-588`）。**当前不是固定一个** |
| 折叠粘贴 | `FoldedKind` 只有 `Text/Code/Log`（`src/app/paste/folded.rs:20-26`），标签 `[Pasted text N lines]`（`:79-87`），解析器按 `<pasted-{text|code|log} lines="N">` 前缀匹配（`:690-745`） |
| 折叠芯片配色 | `style::pasted_chip` / `pasted_bg`（`src/ui/style.rs:575-592`）：Text 黄/深灰、Code 青/深蓝、Log 亮绿/深绿 |
| 后台进程停止 | `ProcessManager::stop_many`（`src/chat/process/manager/mod.rs:356`）、`snapshots`（`:125`）；事件层已有 `Effect::StopSessionProcess` 单进程停止（`src/event/chat/process_page.rs:272-330`） |
| 用户 shell 停止 | `cancel_user_shells`（`src/app/update/chat/user_shell.rs:485-500`）会清 `pending_shells` 并**清空整个 `chat.queue`**，其中包含普通聊天排队项 |
| `/` 面板体系 | `SlashPanel` 枚举（`src/app/state/lx/slash.rs:666-683`）、渲染（`src/ui/main_content/lx/slash.rs`）、主输入路由（`src/app/update/pane.rs:280-370`）、详情 composer 路由（`src/app/update/chat/composer.rs`） |

---

## 1. 目标与非目标

### 1.1 目标

1. **双击 Esc 停止面板**：主 agent 有可停止工作时，第二次 Esc 不再立即停止，而是打开与 `/rewind` 同款的面板，默认 `Stop all`，可选择停止主轮 / 全部 subagent / 全部 shell。
2. **用户主动停止语义**：后台任务页 `^d` 与停止面板停止 subagent 时，明确标记 `stopped_by_user`；主 agent 不得自行恢复，只有用户明确要求才恢复。面板直接输入、主 agent 明确转发的 `send_input` 都可以唤醒。
3. **报告只进主输入框**：Agents / BTW 的 `/report` 不再自动投递主 agent 并自动起轮；报告正文折叠成 `[Pasted subagent report N lines]` / `[Pasted btw report N lines]`，固定在主输入框顶部，与草稿空一行。
4. **报告二级面板**：最后一轮已是报告时，再次 `/report` 打开 `Resend last report` / `Regenerate report` / `Cancel`，默认第一项。
5. **报告轮可 `/rewind`**：两类面板里新生成的报告轮作为普通私有轮次，可用 `/rewind` 从面板消息列表撤销；不动主输入框里的芯片。
6. **移除空闲 `/rewind` 提示**：空闲双击 Esc 进入 `/rewind` 时不再显示原来的 toast/提示文本。

### 1.2 非目标

- 不新增数据库迁移；不持久化“报告待填入主输入框”的中间状态。
- 不改变主 agent 自己调用 `interrupt_agent` 的行为；只改变“用户主动停止”的语义。
- `Stop all` 不停止 BTW 线程；BTW 仍由自己的 `^d` 停止。
- 不把 BTW 多目标拆成多个芯片；多目标合并为一份 BTW 报告。
- 不追溯把旧的 `notify_parent=1` 报告轮变成可撤销；只保证新 `/report` 轮可撤销。
- 不改 AI Web 的 `/rewind` 行为。

---

## 2. Grill Me 已确认决策

| # | 决策 | 结论 |
|---|---|---|
| 1 | 停止面板出现条件 | 只在有可停止工作时出现：主轮在途、subagent 在跑/有排队、用户 shell 在跑、受管后台进程在跑。空闲且无后台工作时双击 Esc 仍是 `/rewind` |
| 2 | 面板动作 | 五项：`Stop all`（默认）、`Stop main response`、`Stop all subagents`、`Stop all shells`、`Cancel`；`Stop all` 包含主轮 + 全部 subagent + 全部 shell/进程 |
| 3 | `/report` 是否仍起模型轮 | 是；子 agent / BTW 助手专门写一轮报告，报告文字填主输入框，不再自动投递主 agent |
| 4 | 主 agent 恢复闸门 | 用户主动停止后，主 agent 默认不得自动恢复；只有用户明确要求才恢复 |
| 5 | 报告写入位置 | 主输入框顶部；与原有草稿之间空一行；同源报告原位替换 |
| 6 | `/rewind` 撤销报告轮 | 只删面板消息列表里的报告轮，不动主输入框里的报告芯片 |
| 7 | 面板输入唤醒 | 用户在 Agents 面板输入框直接输入并发送，视为明确恢复，允许自动 `resume` |
| 8 | 主 agent 带话唤醒 | 用户在主 agent 输入框明确说“让 Curie 继续…”时，允许 `send_input` 唤醒并投递；主 agent 自己不得主动这么做 |
| 9 | 停止时排队输入 | 用户停止子 agent 时，连同其后代的 queued 输入一起取消；恢复后只执行新输入 |
| 10 | shell 范围 | 用户 `!` shell + `exec_command` 受管后台进程都算 shell |
| 11 | 报告正文 | 带来源头（subagent：名字/角色/标题；BTW：线程标题）+ 正文 + 变更文件摘要 |
| 12 | 待填充状态持久化 | 不持久化；进程退出后重新 `/report` 从面板取回报告 |
| 13 | 二级面板备注 | `/report <note>` 仍打开二级面板；备注只作用于“重新生成”，“重新发送”忽略备注 |
| 14 | 芯片配色 | subagent：`LightMagenta` 前景 + `Indexed(53)` 背景；BTW：`LightBlue` 前景 + `Indexed(17)` 背景 |
| 15 | 停止待命提示 | 有可停止工作时第一次 Esc 显示统一提示；空闲 `/rewind` 提示移除 |
| 16 | 重新生成 | 追加新报告轮，保留旧报告轮；主输入框同源芯片用新报告替换 |
| 17 | 停止作用域 | 当前窗格所属会话树；不碰其他窗格/标签页 |
| 18 | 最终唤醒规则 | 允许带提示词的 `send_input` 唤醒用户停掉的 agent；提示词禁止主 agent 自行恢复 |
| 19 | BTW 是否纳入 Stop all | 不纳入；BTW 有独立 `^d` |
| 20 | BTW 多目标 `/report` | 合并成一份 BTW 报告、一个芯片；正文按目标分节；默认无目标时由 BTW 默认助手生成 |
| 21 | BTW 部分目标失败 | 成功的目标成节，失败/超时目标写 `[name] report unavailable (failed)`；至少一个成功才填芯片，全失败只 Toast |

---

## 3. 交互总览

### 3.1 双击 Esc 停止面板

#### 触发与待命

- 可停止工作判据（统一函数，见 §4.1）：
  - 主轮：`lx.chat.request.is_busy()`（Streaming / Committing）；
  - subagent：`lx.agents.runs` 中存在 `run.is_running()` 或 `run.queued_count > 0`；
  - 用户 shell：`lx.chat.has_running_shell()`（含尚未拿到进程 ID 的 pending shell）；
  - 受管后台进程：`lx.background_tasks > 0`（事件层每轮同步）。
- 第一次 Esc：
  - 若存在 pending steer，仍按现有 LIFO 召回，不进入停止待命；
  - 否则设置 `lx.chat.stop_armed_at = Some(now)`，清掉 `lx.rewind.armed_at`；
  - 显示统一提示 `Press Esc again to stop…`（新增文案），不再只对 shell-only 显示。
- 第二次 Esc（3 秒窗口内）：
  - 清 `stop_armed_at`；
  - 计算 `StopAvailability`，打开 `SlashPanel::Stop(StopPanel)`；
  - **不立即执行任何停止**。
- 超过 3 秒：回到第一次 Esc 的语义。
- 空闲且无任何可停止工作时：
  - 双击 Esc 继续进入 `/rewind`；
  - 第一次 Esc 只设置 `rewind.armed_at`，**不再显示** `LX_REWIND_ARMED_HINT`。

#### 面板形态

与 `/rewind` 共用 `command_panel` 渲染，贴在输入框上沿、与输入框等宽：

```text
┌ Stop ─────────────────────────────────────────────┐
│ ❯ Stop all                                        │
│   Stop main response                              │
│   Stop all subagents                 3 running      │
│   Stop all shells                    2 processes    │
│   Cancel                                          │
└ ↵ run · esc close ─────────────────────────────────┘
```

- 标题：`Stop`；右上角可显示作用域 `current session`。
- 底边：`↵ run · esc close`。
- 不可用项置灰并给原因：
  - 主轮不在途 → `Stop main response` 禁用；
  - 无 subagent → `Stop all subagents` 禁用；
  - 无 shell/进程 → `Stop all shells` 禁用。
- `Stop all` 始终可用（面板只在有可停止工作时打开）。
- 高亮默认第一项 `Stop all`。
- `↑/↓` 循环移动并跳过禁用项；鼠标点击/悬停与 `/rewind` 一致。
- `Enter` 执行高亮项；`Esc` 关闭面板（不执行）。
- 面板打开期间其余编辑键被吞掉，不写入主输入框。

#### 执行动作

| 动作 | 行为 |
|---|---|
| `Stop all` | 先 `stop_stream` 停主轮；再停止当前会话树全部 subagent（用户停止语义，取消队列）；再停全部用户 shell + 受管后台进程；主轮排队项一并清空 |
| `Stop main response` | 只 `stop_stream`；subagent、用户 shell、后台进程继续跑 |
| `Stop all subagents` | 只对当前会话树全部 subagent 执行用户停止；主轮、shell、进程继续 |
| `Stop all shells` | 停全部用户 shell（含 pending shell）与受管后台进程；保留普通聊天排队项；主轮、subagent 继续 |
| `Cancel` | 清面板，无副作用 |

执行后：
- 关闭 panel；
- 按实际停止数量 Toast：
  - `Stopped 1 subagent` / `Stopped N subagents`
  - `Stopped 1 background process` / `Stopped N background processes`
  - 主轮停止沿用现有停止反馈；
- 若执行前发现目标已自然结束，重新计算可用性；全部失效则直接关面板并提示 `Nothing to stop`。

### 3.2 用户主动停止 subagent

#### 入口

1. 后台任务页 `Agents` 页签 `^d`（列表态与编辑态都可用）。
2. 停止面板 `Stop all subagents` / `Stop all`。

#### 语义

- 目标子树（目标 + 全部严格后代）全部标记为“用户主动停止”：
  - `ChatStreams.user_stopped` 插入目标 `agent_id`；
  - 运行中的 handle 标记 `stopped_by_user`，不再标记“无报告关闭”；
  - 取消在途轮次；
  - 取消该子树全部 `queued` 输入（DB 事务），返回取消条数；
  - 清除 `resumed/paused/interrupted` 标记；
  - UI 状态显示 `stopped`（不是普通 `cancelled`）。
- 结算：
  - 本轮 `StopReason::Cancelled`，不生成报告；
  - 状态持久化为 `cancelled`；
  - `user_stopped` 保留在内存；
  - `wait_agent` / `list_agents` 结果带 `stopped_by_user: true`；
  - 主 agent 提示词明确“用户主动停止，不要自动恢复”。
- 恢复：
  - `send_input` 允许把目标从 `user_stopped` 唤醒并投递（用户明确要求时）；
  - `resume_agent` 允许不带新话恢复；
  - Agents 面板输入框直接发送允许自动恢复；
  - 任意成功唤醒后从 `user_stopped` 移除，状态回到 `idle/running`。
- 不自动恢复：
  - 停止本身不启动主轮、不投递报告、不自动 resume；
  - 主 agent 下一轮看到 `stopped_by_user` 后，只有用户明确点名继续才可调用 `send_input`/`resume_agent`。

#### 与硬关闭的区别

| 场景 | 函数 | 是否用户主动 | 生成报告 | 可否被主 agent 带话唤醒 |
|---|---|---|---|---|
| Agents `^d` / 停止面板 | 新 `stop_subagents_by_user` | 是 | 否 | 是（用户明确要求） |
| `close_agent` / 会话删除 / 退出 | 现有 `cancel_for_session` / `close` | 否 | 否 | 否 |
| 主 agent `interrupt_agent` | 现有 `interrupt` | 否（模型发起） | 半程报告 | 是（现有语义） |

### 3.3 `/report` 流程

#### Agents 详情 composer

```text
空闲 Curie 详情：
  输入 /report [note] → 起一轮报告模型调用
  → Curie 消息列表出现一轮报告
  → 报告正文折叠成 [Pasted subagent report N lines] 放到主输入框顶部
  → 不自动投递主 agent、不自动起父轮

最后一轮已经是报告：
  输入 /report [note] → 打开二级面板
  ┌ Report ──────────────────────────────┐
  │ ❯ Resend last report                 │
  │   Regenerate report                  │
  │   Cancel                             │
  └ ↵ run · esc close ───────────────────┘
  - Resend：直接复用上一轮报告正文，不调用模型
  - Regenerate：按 note 起新一轮报告；完成后追加新报告轮并替换主输入框同源芯片
  - Cancel：关闭面板，无副作用
```

- `/report` 忙碌（目标 running / queued / compacting）拒绝，不产生效应，保留草稿。
- `/report` 目标无 chat session / 已被移除：Toast `Cannot report from this agent`。
- 报告轮失败/取消：
  - 不填芯片；
  - Toast `Report unavailable`；
  - 报告轮保留在面板消息列表（可见错误/半程正文）。
- 报告正文为空：不填芯片，Toast。

#### BTW composer

- 命令集新增 `/report`：`/rewind` `/compact` `/report` `/new`。
- 默认无目标：由 BTW 自己的默认助手生成报告。
- 有目标：沿用当前线程目标集合（可多个）。
- 多目标：等待全部目标结束，合并成一份报告：
  ```text
  [BTW report] <thread title>
  [Curie] ...
  [Turing] ...
  ```
- 部分失败：
  ```text
  [BTW report] <thread title>
  [Curie] ...
  [Turing] report unavailable (failed)
  ```
- 全失败：不填芯片，Toast `BTW report unavailable`。
- 最后一轮已是报告：同样打开二级面板；`Resend` 复用上一轮合并报告，`Regenerate` 按当前目标集合重新生成。

### 3.4 主输入框报告芯片

- 标签：
  - `[Pasted subagent report 12 lines]`
  - `[Pasted btw report 9 lines]`
  - 单行：`1 line`。
- 颜色：
  - subagent：`LightMagenta` 前景 + `Indexed(53)` 深紫背景；
  - BTW：`LightBlue` 前景 + `Indexed(17)` 深蓝背景；
  - 展开态只铺对应背景，不改变正文前景。
- 位置：固定主输入框顶部；与原有草稿之间恰好一个空行：
  ```text
  [Pasted subagent report 12 lines]

  先看登录流程
  ```
- 同源替换：
  - 同一个 `agent_id` 的 subagent 报告 → 替换已有 subagent 报告芯片；
  - 同一个 BTW `thread_id` 的 BTW 报告 → 替换已有 BTW 报告芯片；
  - 不同来源可共存，新报告仍插到顶部（同源原位替换除外）。
- 芯片可展开/收起、可整块删除、可复制；与现有 text/code/log 芯片一致。
- 发送时 `expand_all` 把芯片替换成正文，模型看到的是来源头 + 报告正文 + 变更文件摘要。

---

## 4. 详细设计

### 4.1 停止面板状态与路由

#### 4.1.1 新增状态类型

文件：`src/app/state/lx/stop.rs`（新）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAction {
    All,
    Main,
    Subagents,
    Shells,
    Cancel,
}

impl StopAction {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Main,
        Self::Subagents,
        Self::Shells,
        Self::Cancel,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StopAvailability {
    pub main: bool,
    pub subagents: usize,
    pub shells: usize,
    pub processes: usize,
}

impl StopAvailability {
    pub fn from_lx(lx: &Lx) -> Self;
    pub fn any(&self) -> bool;
    pub fn enabled(&self, action: StopAction) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPanel {
    pub window: PanelWindow,
    pub actions: Vec<StopAction>,
    pub availability: StopAvailability,
}

impl StopPanel {
    pub fn new(availability: StopAvailability) -> Self;
    pub fn len(&self) -> usize;
    pub fn active_action(&self) -> StopAction;
    pub fn enabled(&self, action: StopAction) -> bool;
}
```

- `StopAvailability::from_lx`：
  - `main = lx.chat.request.is_busy()`；
  - `subagents = lx.agents.stoppable_count()`（新增：running 或 queued > 0 的 run 数）；
  - `shells = usize::from(lx.chat.has_running_shell())`；
  - `processes = lx.background_tasks`。
- `StopAvailability::any = main || subagents > 0 || shells > 0 || processes > 0`。
- `enabled`：
  - `All => any()`；
  - `Main => main`；
  - `Subagents => subagents > 0`；
  - `Shells => shells > 0 || processes > 0`；
  - `Cancel => true`。

#### 4.1.2 接入 `SlashPanel`

文件：`src/app/state/lx/slash.rs`

```rust
pub enum SlashPanel {
    // ...
    Rewind(Box<crate::app::state::lx::rewind::RewindPanel>),
    Stop(Box<crate::app::state::lx::stop::StopPanel>),
    Report(Box<crate::app::state::lx::report::ReportPanel>),
    // ...
}
```

所有 `match` 必须补齐：
- `item_count` / `window` / `set_window` / `move_active` / `current_index`；
- `src/ui/main_content/lx/slash.rs` 的 `items` / `view` / `is_placeholder` / `disabled_mask`；
- `src/app/update/pane.rs` 的 Esc 分支；
- `src/app/update/chat/slash.rs` 的 `apply_slash_enter`；
- `src/app/update/chat/composer.rs` 的详情 composer 分支。

#### 4.1.3 `escape_stop` 重写

文件：`src/app/update/chat/mod.rs`

```rust
pub fn escape_stop(state: &mut AppState, pane: PaneId) -> bool {
    // 1. pending steer 仍优先 LIFO 召回
    if state.pane_anywhere(pane).is_some_and(|t| t.lx.ask.is_none())
        && steer::recall_last(state, pane)
    {
        return true;
    }

    let Some(target) = state.pane_anywhere(pane) else { return false };
    let availability = crate::app::state::lx::stop::StopAvailability::from_lx(&target.lx);
    if !availability.any() {
        return false;
    }

    let now = Instant::now();
    let armed = target
        .lx
        .chat
        .stop_armed_at
        .is_some_and(|at| now.saturating_duration_since(at) <= STOP_ARM_WINDOW);

    if !armed {
        let Some(target) = state.pane_mut_anywhere(pane) else { return false };
        target.lx.chat.stop_armed_at = Some(now);
        target.lx.rewind.armed_at = None;
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_STOP_ARMED);
        return true;
    }

    let Some(target) = state.pane_mut_anywhere(pane) else { return false };
    target.lx.chat.stop_armed_at = None;
    target.lx.slash = Some(SlashPanel::Stop(Box::new(
        crate::app::state::lx::stop::StopPanel::new(availability),
    )));
    true
}
```

关键点：
- **第二次 Esc 只开面板，不执行动作**。
- 旧的 `stop_stream` + `cancel_user_shells` 立即停止路径移除。
- `steer::recall_last` 仍然优先；有 pending steer 时不会误开停止面板。

#### 4.1.4 `/rewind` 待命避让

文件：`src/app/update/chat/rewind.rs`

`arm()` 在进入 `/rewind` 待命前增加：

```rust
if crate::app::state::lx::stop::StopAvailability::from_lx(target.lx).any() {
    return None;
}
```

这样主 agent 空闲但后台仍有 subagent/shell/进程时，第一次 Esc 会落到 `escape_stop`，不会被 `/rewind` 抢走。

同时删除 `arm()` 末尾的：

```rust
show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_REWIND_ARMED_HINT);
```

只删提示，不清 `armed_at`、不改撤销逻辑；`LX_REWIND_ARMED_HINT` 若无其它引用则一并删除。

#### 4.1.5 停止动作执行

文件：`src/app/update/chat/stop.rs`（新）

```rust
pub fn apply_enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = state.pane_anywhere(pane)
        .and_then(|t| match t.lx.slash.as_ref() {
            Some(SlashPanel::Stop(panel)) => Some(panel.clone()),
            _ => None,
        })
    else { return false };

    let action = panel.active_action();
    if !panel.enabled(action) {
        return true;
    }

    // 关闭面板
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
    }

    match action {
        StopAction::Cancel => return true,
        StopAction::All => {
            // 顺序：先停主轮，防止继续派生；再停 subagent；最后停 shell/进程。
            if main_busy(state, pane) { stop_stream(state, pane); }
            push_stop_all_subagents_by_user(state, pane);
            stop_shells(state, pane, ClearQueue::All);
        }
        StopAction::Main => { stop_stream(state, pane); }
        StopAction::Subagents => { push_stop_all_subagents_by_user(state, pane); }
        StopAction::Shells => { stop_shells(state, pane, ClearQueue::ShellsOnly); }
    }
    true
}
```

- `push_stop_all_subagents_by_user` 只是 app 层辅助函数，负责取当前 `session_id` 并入队 `Effect::StopSubagentsByUser { session_id, agent_id: None }`；真正的取消与队列清理由事件层执行。
- `stop_shells`：
  - 发 `Effect::CancelUserShells { pane, scope: ShellsOnly | All }`；
  - 若 `processes > 0`，发 `Effect::StopAllSessionProcesses { pane }`。
- 执行前重新读取 `StopAvailability`，若动作目标已消失：
  - 不执行；
  - Toast `Nothing to stop`；
  - 面板已关闭。

### 4.2 停止效应与事件层

#### 4.2.1 新效应

文件：`src/app/effects.rs`

```rust
StopSubagentsByUser {
    session_id: String,
    agent_id: Option<String>,
},
StopAllSessionProcesses {
    pane: PaneId,
},
CancelUserShells {
    pane: PaneId,
    scope: UserShellStopScope, // All | ShellsOnly
},
```

- `agent_id: Some(id)`：只停目标子树（`^d`）。
- `agent_id: None`：停当前会话树全部可停止 subagent（停止面板）。
- `CancelUserShells.scope`：
  - `All`：`Stop all`，保留旧行为（清 `pending_shells` + 整个 `chat.queue`）；
  - `ShellsOnly`：只清 `pending_shells` 与 `QueuedSubmission::Shell`，保留 `QueuedSubmission::Chat`。

#### 4.2.2 用户停止 subagent

文件：`src/event/chat/subagent/mod.rs`

新增：

```rust
pub(crate) fn stop_subagents_by_user(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    agent_id: Option<&str>,
) -> StopCounts
```

流程：
1. 解析当前根会话树（`resolve_session_tree_root`）。
2. `Some(id)`：解析目标 record + `subtree_paths`；`None`：取整树全部 record。
3. 对每个目标：
   - `streams.user_stopped.insert(agent_id)`；
   - 运行中的 handle：`handle.stopped_by_user = true`、`cancel.send(true)`；
   - 清除 `resumed/paused/interrupted`；
   - `database.cancel_queued_inputs_for_agent(workspace, parent_session, agent_id)`（新增 DB 方法）返回取消条数；
   - idle/无 handle 的 record：`database.close_idle_subagent(...)` 落 `cancelled`；
   - 更新 UI run：`run.status` 保持 `cancelled`（DB 也是 `cancelled`），只置 `run.stopped_by_user = true`、`run.queued_count = 0`。列表文案按 `stopped_by_user` 显示 `stopped`；**不要**把 `run.status` 写成 `"stopped"`，否则 `AgentStatus::parse` 会失败，Agents 的 active/done 过滤会把该 run 误判为 completed。
4. 返回 `StopCounts { subagents, queued_inputs }`，用于 Toast。

**不**调用 `cancel_for_session`；`cancel_for_session` 继续用于会话删除/退出/硬关闭。

#### 4.2.3 handle 标记

两类 handle 各加一个字段：

```rust
// src/event/chat/subagent/mod.rs: SubagentHandle
pub stopped_by_user: bool,

// src/chat/agent/exec/subagent/mod.rs: ChildHandle
stopped_by_user: bool,
```

- `launch_child_turn` / `spawn` 初始为 `false`。
- `finish_subagent_with_billing`：
  ```rust
  let was_stopped_by_user = handle.stopped_by_user;
  let was_closed = handle.closed_without_report;
  let report = (!was_closed && !was_stopped_by_user && handle.notify_parent)
      .then(|| ...);
  ```
  用户停止不生成报告。
- 结算 Cancelled 分支：
  - 如果 `was_stopped_by_user`，不要设置 `paused_by_interrupt` / `resumed`；
  - 清 `user_stopped`？不，保留到显式恢复；
  - 状态 cancelled。

#### 4.2.4 队列取消 DB 方法

文件：`src/db/chat/subagent/queue.rs`

```rust
pub fn cancel_queued_inputs_for_agent(
    &self,
    workspace: &Path,
    parent_session: &str,
    agent_id: &str,
) -> Result<usize>
```

- 校验归属；
- 事务内：

```sql
UPDATE agent_subagent_input
SET status = 'cancelled', completed_at_ms = ?now
WHERE agent_id = ?agent_id AND status = 'queued';
```

- 返回受影响行数；
- 不改 `running` 行（在途轮次由 cancel 信号收敛）；
- 子 agent 的 `queued_count` 由后续 snapshot 同步归零。

#### 4.2.5 停止全部后台进程

文件：`src/event/chat/process_page.rs`

```rust
fn stop_all_session_processes(state: &mut AppState, streams: &ChatStreams, pane: PaneId) {
    let Some(session_id) = /* pane 当前 session */;
    let Some(manager) = streams.runtimes.get(&session_id).map(|r| r.manager().clone())
    else { toast; return; };

    let ids: Vec<u64> = manager
        .snapshots()
        .into_iter()
        .filter(|snapshot| !snapshot.state.is_terminal())
        .map(|snapshot| snapshot.session_id)
        .collect();

    if ids.is_empty() { return; }
    tokio::spawn(async move { manager.stop_many(ids).await; });
    toast "Stopping N background processes";
}
```

- 只作用于当前 session 的 `ProcessManager`，不跨窗格。
- 与 `StopSessionProcess` 一样异步，最终列表由进程终态事件刷新。

#### 4.2.6 作用域

- 所有停止路径都以 `state.pane_anywhere(pane).lx.chat.session_id` 为根：
  - subagent：`resolve_session_tree_root`；
  - process：`streams.runtimes[session_id]`；
  - user shell：当前 pane 的 `lx.chat`。
- 不遍历其他 pane；侧栏/其他标签页不受影响。

### 4.3 用户停止状态与主 agent 恢复闸门

#### 4.3.1 `ChatStreams` 状态

文件：`src/event/chat/mod.rs`

```rust
pub(crate) struct ChatStreams {
    // ...
    /// 被用户主动停止、尚待用户显式恢复的 agent。
    pub(crate) user_stopped: HashSet<String>,
}
```

- 只在内存；应用重启后丢失，DB 状态为 `cancelled`。
- 新 spawn / 显式 resume / 成功 send_input / 面板发送成功后移除。

#### 4.3.2 UI 投影

- `SubagentRun` 新增：
  ```rust
  /// 是否由用户主动停止（仅内存；DB 仍为 cancelled）。
  pub stopped_by_user: bool,
  ```
- `SubagentRun::from_record` 默认 `false`；事件层在 `set_subagent_list` / snapshot 回投时按 `streams.user_stopped` 覆写。
- Agents 列表状态文案：
  - `stopped_by_user` → `stopped`；
  - 否则沿用 DB 状态（`cancelled`）。
- 状态色/步骤标记：
  - `stopped` 用 `StepMark::Cancelled`（空心），文案 `stopped`；
  - 不新增全局 `AgentStatus` 枚举，避免 DB/协议大改。

#### 4.3.3 `wait_agent` / `list_agents` 结果

文件：`src/chat/agent/mod.rs`

```rust
pub struct AgentStatusView {
    // ...
    pub status: String,
    /// 仅当状态来自用户主动停止时为 true。
    pub stopped_by_user: bool,
    pub report: Option<String>,
}
```

`SubagentOutcome::Waited` JSON 仅在 `true` 时序列化 `"stopped_by_user": true`，避免旧快照噪音。

`AgentListItemView` 同样增加 `stopped_by_user: bool`。

`settle_wait`：

```rust
let stopped_by_user = streams.user_stopped.contains(target);
```

`list_agents`：

```rust
stopped_by_user: streams.user_stopped.contains(&record.agent_id),
```

#### 4.3.4 `send_input` 唤醒

文件：`src/event/chat/subagent/runner.rs`

```rust
let is_running = ...;
let is_resumed = streams.resumed_subagents.contains(target);
let is_user_stopped = streams.user_stopped.contains(target);
if !is_running && !is_resumed && !is_user_stopped {
    return Err(NOT_ACTIVE_CODE.to_string());
}
```

- 若 `is_user_stopped`：
  - `streams.user_stopped.remove(target)`；
  - `database.recover_subagent_for_resume(&workspace, &owner_session, target)`（幂等，允许 cancelled → idle；失败则把标记放回并返回错误）；
  - `streams.resumed_subagents.insert(target)`；
  - 走与 idle 恢复相同的 enqueue / schedule 路径。
- 不新增“拒绝发送”的分支；是否主动恢复由系统提示词约束（见 §4.8）。

#### 4.3.5 `resume_agent` 唤醒

文件：`src/event/chat/subagent/runner.rs` `resume()`

- 开头移除 `streams.user_stopped.remove(agent_id);`。
- 其余逻辑不变（`recover_subagent_for_resume` 已支持 cancelled）。

#### 4.3.6 面板输入唤醒

文件：`src/event/chat/subagent/snapshot.rs` `send_user_input_from_panel`

当前 `closed_in_db` 会拒绝 `cancelled`。改为：

```rust
let user_stopped = streams.user_stopped.contains(agent_id);
let closed_in_db = !user_stopped && record.as_ref().is_some_and(... == STATUS_CANCELLED);
if closed_in_memory || closed_in_db {
    // 仅非用户停止才拒绝
}
```

- `user_stopped` 时允许继续；
- 后面的 `is_active` 判断要包含 `user_stopped`：

```rust
let is_active = running || resumed || user_stopped;
```

- 若 `!is_active`，调用 `runner::resume` 前先移除 `user_stopped`。
- 发送成功后移除 `user_stopped`，UI 状态回到 idle/running。

#### 4.3.7 UI 入口

- `src/app/update/pane/task_page.rs` 的 `enter_task_agent_edit` / `submit_agent_edit`：
  - 当前 `run.status == "cancelled"` 时拒绝；
  - 改为 `run.status == "cancelled" && !run.stopped_by_user` 才拒绝；
  - `stopped_by_user` 时允许进入编辑与发送。
- `stop_selected_subagent`：
  - 可停止判据从 `run.is_running() || has_running_descendant` 扩展为 `run.is_running() || run.queued_count > 0 || has_running_descendant`，让“空闲但仍有排队输入”的 agent 也能被 `^d` 停掉；
  - 改用 `Effect::StopSubagentsByUser { session_id, agent_id: Some(agent_id) }`；
  - Toast 改为 `Stopped <name>` 或 `Stopping <name>`；
  - 不再发 `Effect::CancelSubagents`。
- 停止面板 `Subagents` / `All`：
  - 发 `agent_id: None`。

### 4.4 `/report` 命令与报告轮

#### 4.4.1 命令模型

文件：`src/app/state/lx/slash/composer.rs`

```rust
impl DetailCommand {
    pub const AGENT: [Self; 3] = [Self::Rewind, Self::Compact, Self::Report];
    pub const BTW: [Self; 4] = [Self::Rewind, Self::Compact, Self::Report, Self::New];
}
```

- `parse`：
  - `(DetailContext::Agent | DetailContext::Btw, "report") => DetailCommand::Report`；
  - `/report` 的 note 仍允许换行。
- `description`：
  - `Report a summary to the main agent input`
  - 避免说“自动汇报给父 agent”。
- `echo_prefix` / `argument_hint`：不变。
- 测试更新：
  - BTW 命令列表包含 `/report`；
  - `parse("/report note", DetailContext::Btw)` 返回 `Report("note")`。

#### 4.4.2 报告请求标记

文件：`src/chat/agent/report.rs`

```rust
/// 面板 `/report` 请求消息的固定首行；报告正文只进入主输入框，不自动投递。
pub const PANEL_REPORT_MARKER: &str = "[Report for main agent]";

/// 旧版自动汇报标记；仅用于识别历史报告轮。
pub const REPORT_REQUEST_MARKER: &str = "[Report to parent agent]";

pub fn report_request_message(note: &str) -> String {
    // 首行改用 PANEL_REPORT_MARKER
}
```

- `visible_task` 需要同时识别两个 marker，避免报告指令覆盖父可见任务标题。
- 旧记录仍按 `REPORT_REQUEST_MARKER` 识别。

#### 4.4.3 Agents `/report` 提交流程

文件：`src/app/update/chat/detail_report.rs`（重写）

```rust
pub(crate) fn submit(state: &mut AppState, pane: PaneId, note: &str) -> bool {
    let Some(context) = resolve_context(state, pane) else { toast; return true; };

    if context.busy() {
        clear_composer(state, pane);
        toast "Cannot report while the agent is running";
        return true;
    }

    if let Some(last) = context.last_report_body() {
        return open_report_panel(state, pane, context.target, note, last);
    }

    // 没有上一轮报告：起一轮新的报告模型调用
    state.effects.push(Effect::SendSubagentReport {
        pane,
        session_id: context.session_id,
        agent_id: context.agent_id,
        note: note.to_string(),
    });
    clear_composer(state, pane);
    true
}
```

- `last_report_body()`：
  - Agents：DB 最新 user turn 的 user 文本首行是 `PANEL_REPORT_MARKER` 或旧 marker，且该轮已有 assistant 正文；返回 body。
  - BTW：当前线程最后一条 user 消息首行是 marker，且已有 assistant 正文。
- 忙碌判据：
  - Agents：`run.is_running() || run.queued_count > 0 || !run.chat.pending_steers.is_empty() || run.compact.is_running()`；
  - BTW：`thread.is_loading() || thread.compact.is_running()`。

#### 4.4.4 新效应与事件层落地

文件：`src/app/effects.rs`

```rust
SendSubagentReport {
    pane: PaneId,
    session_id: String,
    agent_id: String,
    note: String,
},
SendBtwReport {
    pane: PaneId,
    parent_session_id: String,
    qa_id: String,
    note: String,
},
```

Agents 事件层：
- 校验目标归属与空闲；
- `enqueue_input` 使用 `report_request_message(note)`；
- `notify_parent = false`；
- 在 `ChatStreams.report_to_input` 插入 `submission_id`；
- 立即调度（目标空闲）；
- 记 `report_to_input_turn`，避免与新报告混淆。

BTW 事件层：
- 复用 `Submit` 链路，`BtwSubmission` 增加 `report_to_main_input: bool`；
- `BtwStreams` 增加 `report_batches: HashMap<String, BtwReportBatch>`：
  ```rust
  struct BtwReportBatch {
      thread_id: String,
      expected: usize,
      sections: Vec<(String, String)>, // (label, body)
      failed: Vec<String>,
  }
  ```
- 每个 recipient 结算时写入 section/failed；
- 全部收口后发 `AppEvent::BtwReportReady`。

#### 4.4.5 报告轮结算与捕获

Agents：
- `SubagentHandle` 增加 `report_to_input: bool`。
- `ChatStreams` 增加 `report_to_input: HashSet<String>`（按 submission id）。
- `schedule_next_queued_input_with_done` 认领后：
  ```rust
  let report_to_input = streams.report_to_input.remove(&next_input.submission_id).is_some();
  launch_child_turn(..., report_to_input, ...)
  ```
- `launch_child_turn` 把 `report_to_input` 写入 handle。
- `finish_subagent_with_billing`：
  ```rust
  if handle.report_to_input && matches!(reason, StopReason::Complete) {
      let body = report_body(&handle);
      let changed_paths = database.list_changed_paths_for_turn(...).unwrap_or_default();
      if !body.trim().is_empty() {
          // 事件层直接回投 app 状态；Effect 是 app→event 方向，不能反向 push。
          crate::app::update::on_subagent_report_ready(state, SubagentReportReady {
              pane: handle.parent.pane,
              parent_session: handle.parent.session.clone(),
              agent_id: handle.agent_id.clone(),
              nickname: handle.nickname.clone(),
              title: handle.title.clone(),
              role: handle.role,
              body,
              changed_paths,
          });
      } else {
          crate::app::update::show_lx_toast(... "Report unavailable" ...);
      }
  }
  ```
- 不传 `report` payload 给 `finish_subagent_turn`（`None`），因此不写 `agent_subagent_report`、不触发 `drain_reports`。
- `on_subagent_report_ready` 在 app 层组装来源头 + 变更文件摘要，再调用 `report_fill::fill_main_input`。

BTW：
- `BtwSubmission.report_to_main_input` 存入 `BtwPending` / `BtwRun`；
- `on_finished` / 事件层结算时按 batch 聚合；
- 全部完成后通过 `BtwEvent::ReportReady { ... }` / `AppEvent` 回投 app 层，由 `update::on_btw_report_ready` 组装合并报告并填主输入框；事件层不直接 push `Effect`。

#### 4.4.6 报告正文格式

Agents：
```text
[Subagent report] Curie · executor · Fix login redirect
<assistant 报告正文>
Changed files: src/auth/session.rs, src/auth/login.rs
```

- 来源头：
  - `[Subagent report]`；
  - `nickname` 或 `agent_id`；
  - `role.as_str()`；
  - `title`（空时省略）。
- `Changed files`：
  - 只在 `changed_paths` 非空时追加；
  - 最多 `MAX_REPORT_CHANGED_PATHS` 条，余量 `(+N more)`；
  - 与现有 `format_report_envelope` 的路径展示一致，但不使用 XML 信封。

BTW：
```text
[BTW report] <thread title>
[Curie] <body>
[Turing] <body>
```

- 默认助手 section label 用 `[BTW]`。
- 失败 section：`[Turing] report unavailable (failed)`。
- 只使用纯文本，不把 XML 信封塞进主输入框。

#### 4.4.7 报告轮可撤销

- 新 `/report` 轮是 `notify_parent=false` 的普通私有轮次：
  - `latest_undoable_private_turn` 的 `notify_parent == 0` 条件自然满足；
  - Agents `/rewind` 可以删除它；
  - BTW `/rewind` 走 `latest_user_turn`，同样可以删除。
- 旧 `notify_parent=1` 报告轮继续不可撤销，避免影响已自动投递的历史。
- 撤销只删面板消息；主输入框芯片不变（决策 6）。

### 4.5 报告二级面板

文件：`src/app/state/lx/report.rs`（新）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportAction {
    Resend,
    Regenerate,
    Cancel,
}

impl ReportAction {
    pub const ALL: [Self; 3] = [Self::Resend, Self::Regenerate, Self::Cancel];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPanel {
    pub window: PanelWindow,
    pub actions: Vec<ReportAction>,
    pub target: crate::app::state::DetailTarget,
    pub note: String,
    /// 上一轮报告正文；`Resend` 直接使用。
    pub last_report: Option<String>,
}
```

- `ReportPanel::new(target, note, last_report)`：默认 `Resend`；`last_report` 为空时 `Resend` 禁用并高亮 `Regenerate`。
- `len` / `enabled` / `active_action`。

#### 4.5.1 打开

- `detail_report::submit` 检测到最后一轮是报告：
  - 读取报告正文（Agents：DB；BTW：内存 thread）；
  - `target.lx.with_active_editor(|editor| editor.slash = Some(SlashPanel::Report(...)))`；
  - 不清空 composer 草稿（命令已由调用方消费）。
- 面板与 `/rewind` 一样是细节 composer 的模态面板，打开期间吞掉编辑键。

#### 4.5.2 执行

`src/app/update/chat/detail_report.rs::apply_confirm`：

```rust
match action {
    ReportAction::Cancel => close,
    ReportAction::Resend => {
        fill_main_input(state, pane, target, last_report);
        close;
    }
    ReportAction::Regenerate => {
        close;
        push Effect::SendSubagentReport / SendBtwReport { note };
    }
}
```

- `Resend` 不调用模型，不新增报告轮。
- `Regenerate` 按当前目标集合重新生成；Agents 单目标，BTW 多目标。
- 若面板打开后目标被切走（切 agent / 切 QA），执行前校验 `target.is_active(&lx)`；失效则关闭面板并 Toast `Report target changed`。
- `note` 只传给 `Regenerate`；`Resend` 忽略备注（决策 13）。

#### 4.5.3 UI

文件：`src/ui/main_content/lx/slash.rs`

- `items`：
  - `Resend last report`（禁用原因：`No previous report`）
  - `Regenerate report`
  - `Cancel`
- 标题 `Report`；底边 `↵ run · esc close`；右上角 `subagent` / `btw`。
- 文案集中在 `src/ui/text.rs` / `src/ui/text/detail.rs`。

### 4.6 主输入框折叠芯片

#### 4.6.1 扩展 `FoldedKind`

文件：`src/app/paste/folded.rs`

```rust
pub enum FoldedKind {
    Text,
    Code,
    Log,
    SubagentReport,
    BtwReport,
}

impl FoldedKind {
    pub const fn tag_name(self) -> &'static str {
        match self {
            // ...
            Self::SubagentReport => "pasted-subagent-report",
            Self::BtwReport => "pasted-btw-report",
        }
    }

    pub const fn chip_word(self) -> &'static str {
        match self {
            // ...
            Self::SubagentReport => "subagent report",
            Self::BtwReport => "btw report",
        }
    }
}
```

- `label()` 复用现有 `[Pasted {word} N lines]`：
  - `[Pasted subagent report 12 lines]`
  - `[Pasted btw report 9 lines]`
  - 单行 `1 line`。
- `CLOSE_TAG_SUFFIXES` 增加：
  - `/pasted-subagent-report>`
  - `/pasted-btw-report>`

#### 4.6.2 `FoldedToken.source`

```rust
pub struct FoldedToken {
    pub range: Range<usize>,
    pub kind: FoldedKind,
    /// 报告类芯片的来源：subagent agent_id 或 btw thread_id；普通粘贴为 None。
    pub source: Option<String>,
    pub line_count: usize,
    pub body: String,
    pub id: u64,
}
```

- `id` 哈希加入 `source`，确保同正文不同来源的展开状态互不串用。
- 普通 `encode_pasted_kind` 生成 `source: None`。

#### 4.6.3 Token 语法

报告类：

```text
<pasted-subagent-report source="agent-123" lines="12">
...body...
</pasted-subagent-report>
```

```text
<pasted-btw-report source="thread-456" lines="9">
...body...
</pasted-btw-report>
```

- 解析器：
  - 现有 text/code/log 保持 `<pasted-{kind} lines="N">` 原语法；
  - 报告类先匹配 `<pasted-subagent-report source="` / `<pasted-btw-report source="`，读到下一个 `"`，再要求 ` lines="N">`。
  - `source` 中的 `"` 与换行必须编码/拒绝；来源都是稳定 id，正常不会出现。
- `escape_body` / `unescape_body` 同时处理报告闭合标签。
- 旧的 text/code/log token 完全兼容。

#### 4.6.4 配色

文件：`src/ui/style.rs`

```rust
pub fn pasted_chip(kind: FoldedKind) -> Style {
    let (fg, bg) = match kind {
        FoldedKind::Text => (Color::Yellow, Color::Indexed(236)),
        FoldedKind::Code => (Color::Cyan, Color::Indexed(23)),
        FoldedKind::Log => (Color::LightGreen, Color::Indexed(22)),
        FoldedKind::SubagentReport => (Color::LightMagenta, Color::Indexed(53)),
        FoldedKind::BtwReport => (Color::LightBlue, Color::Indexed(17)),
    };
    // ...
}

pub fn pasted_bg(kind: FoldedKind) -> Style {
    let bg = match kind {
        // ...
        FoldedKind::SubagentReport => Color::Indexed(53),
        FoldedKind::BtwReport => Color::Indexed(17),
    };
    // ...
}
```

- 展开态只铺背景；正文前景保持默认。
- 更新 `style` 的配色不重叠测试。

#### 4.6.5 写入主输入框

文件：`src/app/update/chat/report_fill.rs`（新，或并入 `detail_report.rs`）

```rust
pub(crate) fn fill_main_input(
    state: &mut AppState,
    pane: PaneId,
    parent_session_id: &str,
    kind: FoldedKind,
    source: &str,
    body: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else { return false };
    if target.lx.chat.session_id.as_deref() != Some(parent_session_id) {
        // 会话已切走：不污染新会话输入框；报告轮仍在面板里可再次 /report
        return false;
    }

    // 编辑缓冲的 snapshot/touch/光标换算只能在 app/state/lx 内部完成，
    // 因此把真正的文本改写收进 `Lx::insert_report_chip`。
    target
        .lx
        .insert_report_chip(kind, source, body)
}
```

`src/app/state/lx/folded.rs` 新增：

```rust
impl Lx {
    /// 插入/替换报告芯片：同源原位替换，否则插到顶部并与原草稿空一行。
    pub(crate) fn insert_report_chip(
        &mut self,
        kind: FoldedKind,
        source: &str,
        body: &str,
    ) -> bool {
        let token = crate::app::paste::encode_report(kind, source, body);
        let existing = crate::app::paste::folded_tokens(&self.input)
            .into_iter()
            .find(|t| t.kind == kind && t.source.as_deref() == Some(source));

        self.snapshot();
        if let Some(hit) = existing {
            let old_cursor = self.cursor;
            let delta = token.len() as isize - hit.range.len() as isize;
            self.input.replace_range(hit.range.clone(), &token);
            self.cursor = shift_cursor(old_cursor, &hit.range, delta);
        } else {
            let prefix = format!("{token}\n\n");
            self.cursor = self.cursor.saturating_add(prefix.len());
            self.input.insert_str(0, &prefix);
        }
        self.touch();
        self.scroll_cursor_into_view();
        true
    }
}
```

细节：
- `encode_report` 使用 `escape_body`，并计算 `line_count = body.lines().count().max(1)`。
- `shift_cursor` 是纯函数：光标在 token 内夹到起点，在 token 后按长度差平移。
- 报告正文由 app 层 `on_subagent_report_ready` / `on_btw_report_ready` 组装好（来源头 + 正文 + 变更文件），`fill_main_input` 只负责 token 化与落位。
- 同源替换时，如果光标在 token 内，夹到 token 起点；如果在 token 后，按长度差平移。
- 顶部插入时：
  - 若已有同源芯片，不走顶部插入，避免重复；
  - 不同来源的新报告插到最顶部，形成最新在上；
  - 原有草稿与芯片之间始终恰好一个空行：`token + "\n\n" + old_input`。
- 空输入时：`token + "\n\n"`，光标落在空行后，方便继续输入。
- 仅当 pane 当前 session 与报告所属主会话一致时写入；否则丢弃并 `tracing::debug!`，不 Toast（避免在别的会话弹无关提示）。
- 主输入框已有内容且光标在中间：顶部插入后光标按插入长度平移，保持用户原编辑位置。

#### 4.6.6 展开与发送

- `Projection::new` 已按 `SegmentKind::Chip` 处理，报告类自动生效。
- `expand_all` 用 token.body 替换；主 agent 看到来源头 + 正文。
- `mask` 用于触发词扫描，自动把报告芯片视为空白，避免报告正文里的 `@`/`/` 误触发面板。
- 选择、复制、Backspace/Delete 整块删除沿用现有 token 边界逻辑。

### 4.7 `/rewind` 与报告轮

- 新报告轮是普通私有轮次：
  - Agents：`latest_undoable_private_turn` 返回它（`position > 0`、无 `parent_call_id`、`notify_parent = 0`）；
  - BTW：`latest_user_turn` 返回它。
- `/rewind` 删除该轮：
  - 删除 user/assistant 消息与 parts；
  - 恢复文件（如果报告轮有文件改动）；
  - 主输入框芯片不变。
- 报告轮删除后：
  - 面板消息列表少一轮；
  - 再次 `/report` 时，如果上一轮变成普通问答，则直接起新报告，不弹二级面板。
- 旧 `notify_parent=1` 报告轮：
  - 继续不可撤销；
  - 二级面板仍可识别并 `Resend`（从 DB 读取正文），但不支持 `/rewind`。

### 4.8 提示词

文件：`src/chat/prompt.rs`

在 subagent 控制工具说明附近新增英文规则：

```text
- A subagent may be stopped by the user from the task panel. That stop is an explicit user decision: do not resume it or send it more work unless the user explicitly asks you to continue that subagent. Results from `wait_agent` and `list_agents` mark such agents with `stopped_by_user: true`.
```

- `PROMPT_VERSION` 从 `lx-14` 递增到 `lx-15`。
- 更新 `src/chat/prompt.rs` 中锁定版本的测试。
- 不改工具 schema；只改系统提示。

### 4.9 文案

新增/修改集中在 `src/ui/text.rs`、`src/ui/text/detail.rs`：

- `LX_STOP_PANEL_TITLE = "Stop"`
- `LX_STOP_PANEL_FOOTER = "↵ run · esc close"`
- `LX_STOP_SCOPE = "current session"`
- `LX_STOP_ACTION_ALL = "Stop all"`
- `LX_STOP_ACTION_MAIN = "Stop main response"`
- `LX_STOP_ACTION_SUBAGENTS = "Stop all subagents"`
- `LX_STOP_ACTION_SHELLS = "Stop all shells"`
- `LX_STOP_ACTION_CANCEL` 复用撤销面板的 `Cancel`
- `LX_STOP_ARMED = "Press Esc again to stop…"`
- `LX_NOTHING_TO_STOP = "Nothing to stop"`
- `LX_STOPPED_SUBAGENTS(n)` / `LX_STOPPED_PROCESSES(n)`
- `LX_REPORT_PANEL_TITLE = "Report"`
- `LX_REPORT_PANEL_FOOTER = "↵ run · esc close"`
- `LX_REPORT_ACTION_RESEND = "Resend last report"`
- `LX_REPORT_ACTION_REGENERATE = "Regenerate report"`
- `LX_REPORT_NO_PREVIOUS = "No previous report"`
- `LX_REPORT_TARGET_CHANGED = "Report target changed"`
- `LX_REPORT_UNAVAILABLE = "Report unavailable"`
- `LX_BTW_REPORT_UNAVAILABLE = "BTW report unavailable"`
- `LX_TASK_AGENT_STOPPED = "stopped"`（Agents 列表状态）
- 报告芯片文案由 `FoldedKind::chip_word` 生成，不单独写死。

移除：
- `LX_REWIND_ARMED_HINT`（若不再被引用）。

---

## 5. 数据、并发与失败模式

### 5.1 无数据库迁移

- `user_stopped`、`report_to_input`、`stopped_by_user` 都是内存状态。
- DB 继续使用现有 `cancelled` 状态；UI 通过 `SubagentRun.stopped_by_user` 显示 `stopped`。
- 应用重启后：
  - `user_stopped` 丢失，DB 显示 `cancelled`；
  - 不会自动恢复；
  - 用户可重新打开面板发送消息/`/report`，走现有 resume 路径。
- 报告待填充状态不持久化；报告轮本身已落库，用户可再次 `/report` 用 `Resend` 取回。

### 5.2 并发与迟到回执

- 所有状态修改在事件循环内完成；渲染只读。
- 停止动作与子 agent 自然完成竞争：
  - `stop_subagents_by_user` 先置 `user_stopped` 和 handle 标记，再发 cancel；
  - 若 handle 已自然完成并移除，仍把 `user_stopped` 写入，后续 `projected_agent_status` 显示 stopped；
  - 若在 settle 前移除 handle，`finish` 不会覆盖 `user_stopped`。
- 报告回执迟到：
  - 报告轮完成时 pane 已关闭：不填；报告轮仍在 DB；
  - pane 已切会话：不填，避免污染新会话输入框；
  - 同源芯片已被用户删除：重新插到顶部；
  - 主输入框已包含同源芯片：原位替换。
- 报告二级面板打开后目标被切换：
  - `ReportPanel.target.is_active(&lx)` 校验失败 → 关闭面板 + Toast；
  - 不执行 `Resend`/`Regenerate`。
- 停止面板打开后目标自然结束：
  - `Enter` 前重新计算可用性；
  - 选中项失效 → 不执行 + Toast；全部失效 → 直接关面板。

### 5.3 失败语义

| 失败 | 行为 |
|---|---|
| Agents `/report` 目标 running/queued/compacting | 拒绝，草稿保留，Toast |
| Agents 报告轮 Failed/Cancelled | 不填芯片，Toast `Report unavailable`，报告轮保留 |
| BTW 报告部分目标失败 | 成功 section + 失败占位；至少一个成功才填 |
| BTW 报告全失败 | 不填芯片，Toast `BTW report unavailable` |
| 报告正文为空 | 不填芯片，Toast |
| 报告芯片超 `MAX_FOLDED_BYTES` | 按现有折叠规则拒绝/降级；报告正文在组装阶段按 `MAX_REPORT_BODY` 截断 |
| 主输入框会话不匹配 | 不填，不弹无关 Toast，记录 debug 日志 |
| 停止面板目标已消失 | 不执行，Toast `Nothing to stop` |
| 停止进程管理器不存在 | Toast `Failed to stop the process`（沿用现有文案） |

---

## 6. 测试计划

### 6.1 停止面板状态与路由

- 空闲无后台工作：双击 Esc 进入 `/rewind`；不再有 `LX_REWIND_ARMED_HINT`。
- 主轮运行：第一次 Esc 只待命 + 统一提示；第二次 Esc 打开 `StopPanel`，默认 `Stop all`，**不立即停止**。
- 只有 subagent 在跑、主轮空闲：双击 Esc 打开停止面板，不进入 `/rewind`。
- 只有 shell/后台进程：双击 Esc 打开停止面板。
- 面板 `↑/↓` 跳过禁用项；`Esc` 关闭；`Enter` 执行。
- `Stop all` 触发主轮 + subagent + shell + process 四类效应。
- `Stop main response` 只触发 `CancelChatStream` / `CancelChatCommit`。
- `Stop all subagents` 只触发 `StopSubagentsByUser`。
- `Stop all shells` 只触发 `CancelUserShells(ShellsOnly)` + `StopAllSessionProcesses`，不清普通聊天 queue。
- 作用域：另一个 pane 的 subagent/shell/process 不受影响。
- 面板目标在打开后结束：Enter 不执行 + Toast。

### 6.2 用户停止 subagent

- `^d` 发 `StopSubagentsByUser`，不再发 `CancelSubagents`。
- 停止后 `ChatStreams.user_stopped` 包含目标；handle `stopped_by_user=true`。
- 结算不生成 report、不写 `agent_subagent_report`、不自动起父轮。
- 队列中的输入全部变 `cancelled`；`queued_count=0`。
- `wait_agent` 结果包含 `stopped_by_user: true`；`list_agents` 同样。
- `send_input` 能唤醒用户停止的 agent 并投递；成功后 `user_stopped` 移除。
- `resume_agent` 能唤醒；成功后移除。
- Agents 面板 composer 对 `stopped_by_user` 的 run 可进入编辑并发送；对普通 `cancelled` 仍拒绝。
- UI 状态显示 `stopped`，步骤标记空心。
- 主 agent 提示词包含“不要自动恢复”的规则；`PROMPT_VERSION` 更新。
- 旧 `close_agent` / 会话删除仍走 `cancel_for_session`，行为不变。

### 6.3 `/report` 命令与报告轮

- Agents `/report` 无上一轮报告：发 `SendSubagentReport`，`notify_parent=false`，报告轮落库。
- 报告轮完成后主输入框出现 `[Pasted subagent report N lines]`；不自动起父轮。
- BTW `/report` 无上一轮报告：发 `SendBtwReport`；完成后出现 `[Pasted btw report N lines]`。
- 默认无目标 BTW 报告由 BTW 默认助手生成。
- BTW 多目标合并成一份报告，按目标分节。
- BTW 部分失败 → 失败占位；全失败 → 不填芯片。
- 忙碌目标拒绝且不产生效应。
- 报告轮失败/空正文 → 不填芯片 + Toast。
- 报告正文包含来源头 + 变更文件摘要。

### 6.4 报告二级面板

- 最后一轮是报告：`/report` 打开 `ReportPanel`，默认 `Resend`。
- `Resend`：不产生模型效应；直接填芯片；备注忽略。
- `Regenerate`：产生新的报告请求；带 note；追加报告轮；同源芯片替换。
- `Cancel`：无效应。
- 目标切换后执行：面板关闭 + Toast，不产生效应。
- `last_report` 为空：`Resend` 禁用，默认落到 `Regenerate`。

### 6.5 折叠芯片

- `encode_report` / parser 往返：kind、source、line_count、body 正确。
- 标签：`[Pasted subagent report 12 lines]` / `[Pasted btw report 9 lines]` / `1 line`。
- 配色：subagent `LightMagenta + Indexed(53)`；BTW `LightBlue + Indexed(17)`；与 text/code/log 不重叠。
- 顶部插入：`token + "\n\n" + old_input`；光标按插入长度平移。
- 同源替换：只替换对应 token；不产生第二个同源芯片。
- 不同来源共存，新报告在顶部。
- 展开/发送：模型看到来源头 + 正文；芯片标签不进入正文。
- 报告正文含 `</pasted-subagent-report>` 等闭合标签时正确转义/反转义。
- 选择/复制/整块删除/展开状态沿用现有测试。

### 6.6 `/rewind` 报告轮

- Agents 新报告轮是 `latest_undoable_private_turn` 的目标；`/rewind` 删除它。
- BTW 新报告轮同样可删除。
- 删除报告轮后主输入框芯片不变。
- 旧 `notify_parent=1` 报告轮仍不可撤销。
- 报告轮删除后再次 `/report` 不弹二级面板（上一轮已不是报告）。

### 6.7 渲染与文案

- `SlashPanel::Stop` 渲染标题/动作/禁用原因/默认高亮。
- `SlashPanel::Report` 渲染标题/动作/禁用原因/默认高亮。
- 新增文案无中文泄漏；所有用户可见文案英文。
- `LX_REWIND_ARMED_HINT` 删除后无悬空引用。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / 受影响测试全绿。

---

## 7. 风险与回滚

| 风险 | 说明 | 缓解 |
|---|---|---|
| 双击 Esc 语义变化 | 用户已习惯第二次 Esc 立即停止；现在变成面板确认 | 默认 `Stop all`，回车即停；第一次 Esc 有统一提示 |
| 误开停止面板 | 主轮空闲但后台 subagent 在跑时，双击 Esc 不再进 `/rewind` | 符合“有可停止工作”的决策；`/rewind` 仍可通过命令打开 |
| `Stop all` 破坏性 | 会停主轮 + 全部 subagent + shell/进程 | 作用域限制当前会话树；面板明确五项动作；默认项高亮 |
| 用户停止后主 agent 仍唤醒 | 提示词是软约束 | wait/list 结果带 `stopped_by_user`；面板输入/显式 `send_input` 才唤醒；后续可在评审中加硬闸门 |
| 报告芯片解析兼容 | 新增两种 token 与 source 属性 | 旧 text/code/log 语法不变；新增往返/兼容测试 |
| 报告自动投递改变 | 用户可能期待主 agent 自动继续 | 这是本次明确需求；报告只进输入框，用户可编辑后再发 |
| BTW 多目标报告 | 合并可能丢失目标独立性 | 正文按目标分节；失败占位；后续可扩展为多芯片 |
| 无迁移 | 重启后 `user_stopped` 丢失 | 不自动恢复；DB `cancelled` 仍可见；重新发送即恢复 |

**回滚**：
- 纯代码回滚，无数据库迁移。
- 报告芯片 token 若已写入输入框草稿，回滚后旧版本不认识新 token，会当普通文本发送；这是可接受的草稿级影响。
- 用户停止状态仅在内存，重启即清空。

---

## 8. 验收清单

### 8.1 停止面板

1. 主 agent 空闲、无后台任务：双击 Esc 进入 `/rewind`，不再出现撤销提示。
2. 主 agent 正在生成：第一次 Esc 显示 `Press Esc again to stop…`；第二次弹出停止面板，默认 `Stop all`。
3. 主 agent 空闲但 Curie 在跑：双击 Esc 弹出停止面板；`Stop main response` 置灰。
4. 只有 shell 在跑：双击 Esc 弹出面板；`Stop all subagents` 置灰。
5. 选 `Stop all`：主轮、全部 subagent、用户 shell、后台进程都停；BTW 线程不停。
6. 选 `Stop main response`：只有主轮停；subagent/shell/进程继续。
7. 选 `Stop all shells`：用户 `!` 命令与 `exec_command` 后台进程都停；普通聊天排队项保留。
8. 另一个窗格的后台任务不受影响。
9. 面板打开后目标自然结束：Enter 不执行 + `Nothing to stop`。

### 8.2 用户主动停止

1. Agents 页签 `^d` 停 Curie：状态显示 `stopped`；Curie 消息保留；不生成报告。
2. 主 agent 等待 Curie 的 `wait_agent` 返回 `stopped_by_user: true`；主 agent 不自动恢复。
3. 用户在主 agent 输入“让 Curie 继续检查 auth.rs”：Curie 被唤醒并收到这句话。
4. 用户在 Agents 面板 Curie 输入框直接发送：Curie 被唤醒并处理。
5. 用户不说继续：主 agent 不调用 `send_input`/`resume_agent`。
6. 停止时 Curie 队列里有 3 条旧追问：全部取消；恢复后只执行新消息。

### 8.3 报告

1. Curie 空闲，输入 `/report 总结登录 bug`：Curie 生成一轮报告；主输入框顶部出现 `[Pasted subagent report N lines]`，与草稿空一行；主 agent 不自动起轮。
2. 展开芯片：正文以 `[Subagent report] Curie · ...` 开头，含变更文件摘要。
3. 再次 `/report`：弹出 `Resend last report` / `Regenerate report` / `Cancel`，默认第一项。
4. `Resend`：不调用模型，芯片内容不变。
5. `Regenerate`：Curie 追加一轮新报告；主输入框同源芯片替换；面板保留旧报告轮。
6. 同源报告反复填充：主输入框始终只有一个对应芯片。
7. subagent 与 BTW 报告可同时存在，颜色分别为亮品红/深紫、亮蓝/深蓝。
8. BTW 默认无目标：`/report` 由 BTW 默认助手生成报告。
9. BTW 同时 @Curie @Turing：`/report` 生成一份合并报告，按目标分节；部分失败有占位；全失败不填芯片。
10. 对新报告轮执行 `/rewind`：面板消息列表删除该轮；主输入框芯片保留。
11. 旧报告轮（本次改动前生成）不因 `/rewind` 被删除。

### 8.4 代码质量

- `cargo fmt` 通过。
- `cargo clippy --all-targets -- -D warnings` 通过。
- 受影响测试与全量 `cargo test` 通过。
- 生产代码无 `unwrap()/expect()`；无中文 UI 文案；无 schema 版本变更。
- 文件行数：新增/修改文件遵守 1000 行上限；测试按场景拆分。

---

## 9. 实施前待确认

本文档已按 21 项 Grill Me 决策收口。实施前只需用户确认：

1. 是否同意“无数据库迁移、用户停止状态仅内存”的取舍；
2. 是否同意“旧 `notify_parent=1` 报告轮不追溯可撤销”；
3. 是否同意“BTW 多目标合并为一份报告芯片”。

若三项均同意，按 `task.md` 的顺序执行。
