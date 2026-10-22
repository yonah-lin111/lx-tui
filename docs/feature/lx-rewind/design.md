# lx 最新轮次撤销：设计方案

> 状态：**已实现并合并到 dev**（2026-10-22）；实施历史见 `lx-rewind` 分支与合并提交。
>
> 日期：2026-10-22。范围：lx 页 Native Agent 输入框、最新轮次、受控文件修改、独立持久化操作提示。
>
> 本文中的 Rust/SQL 为拟议契约，不表示已经实现。实现时按职责拆分，单文件不超过 1000 行，新增子文件约 500 行以内。

## 1. 目标与明确排除的范围

提供 `/rewind`，别名 `/undo`。空闲时，在空输入框连续按两次 Esc，定位并高亮当前会话最新一条用户消息；Enter 打开四项操作面板，再按 Enter 执行所选动作。

撤销以稳定用户消息 ID 标识的一轮为单位。只能操作当前会话最后一轮；不能向前选择任意历史轮次。撤销最后一轮后，再次进入才能操作上一轮。撤销提示不算对话轮次。

文件回退只覆盖 `edit`、`write` 实际产生的修改，包含新增、覆盖、删除、移动以及工具部分失败之前已提交的修改。命令行、外部 Agent、数据库迁移、网络服务等副作用不在恢复范围。

Git 项目优先使用实际目标所属仓库的 Git 对象库；非 Git 目录使用独立持久化快照。恢复基线是 Agent 修改前的文件状态，包含用户已有的未提交修改，绝不是 HEAD。不得执行 reset、checkout/restore 工作树、clean、stash，也不得改变 index、分支、标签、提交或 refs。

本期不提供 redo、不自动合并文件冲突、不恢复命令副作用、不回收用户仓库内的 Git 对象。空目录不作为可恢复文件删除；不递归删除目录。

## 2. 逐项确认结果

| 编号 | 已确认决策 | 对应边界 |
| --- | --- | --- |
| D1 | “没有输出”指 Agent 已空闲 | Committing/Streaming/Finishing 仍走现有双 Esc 停止，即使尚无正文 |
| D2 | 双 Esc 后第一次 Enter 只打开动作面板 | 再按 Enter 才执行；默认高亮第一项 |
| D3 | 只能选择最新一轮 | 上下键不能选择更早用户消息；撤销后再进入才可操作上一轮 |
| D4 | 成功后显示独立持久化 tip | 按操作类型区分，不是 user/assistant，也不进入模型上下文 |
| D5 | 文件仅覆盖 edit/write | 有 Git 优先项目 Git，无 Git 仍可恢复 |
| D6 | Git 恢复到 Agent 操作前 | 不丢用户原有未提交内容，不改变暂存区、分支和提交 |
| D7 | 多 Agent 与 worktree 隔离 | 实际路径绑定；发现其他修改则整次文件回退拒绝，不自动合并、不删对话 |
| D8 | 空草稿保护 | 双 Esc 仅空输入生效；命令输入必须是独立单行 slash 查询，允许从候选采纳 |
| D9 | 仅恢复文件不回填消息 | 保留对话，也不跳到上一轮 |
| D10 | 本会话有存活后台任务时禁止撤销 | 提示手动停止；不自动杀进程，不停止其他会话的任务 |

用户特别强调的两项必须单独验收：

1. A/B 两个 Agent 工作时，撤销 A 不能抹掉 B 的修改。
2. 在 dev 主 checkout 发起会话，但实际修改 linked worktree 时，只恢复实际 worktree 的路径，不能恢复 dev 主 checkout 的同名文件。

## 3. 现状与参考代码依据

### 3.1 本项目已核实的结构

已使用 codegraph、codebase-memory 的 `search_graph`、`trace_path`、`get_code_snippet` 和覆盖检查。当前仓库和两个参考仓库均已有索引，未重复初始化。CBM 的旧 Codex 索引存在源码偏移，参考行为以 codegraph 当前源码及定点读取为准。索引覆盖信号仅是辅助，不能代替编译和测试。

| 文件 / 符号 | 当前事实 | 本次接入 |
| --- | --- | --- |
| `src/app/update/pane.rs::apply_lx_input` | 任务页、历史浏览、paste、mention、slash、markdown 面板优先消费输入；普通 Escape 最后走停止 | 保留优先级，普通空闲 Escape 增加 rewind 分支 |
| `src/app/update/chat/mod.rs::escape_stop` | 空闲返回 false；运行中有独立的 5 秒待命窗口 | 停止 arm 与 rewind arm 分离，不复用同一个标志 |
| `src/app/state/lx/slash.rs::SlashCommand` | Model/Resume/New，已有 aliases 与统一匹配 | 新增 Rewind，alias 为 undo |
| `src/app/state/lx/slash.rs::SlashPanel` | Commands/Model/History，共用 PanelWindow | 新增 Rewind 动作面板，不另造通用面板系统 |
| `src/app/update/chat/session.rs::apply_slash_command` | 分派内置命令 | 分派至最新轮次选择入口 |
| `src/app/state/lx.rs::apply_history_text` | 全文替换、一次输入撤销快照、光标末尾、清选区 | 抽出输入替换能力供撤销回填复用 |
| `src/chat/agent_loop/mod.rs::FileTools::execute` | spawn_blocking 调用同步文件工具；执行前已有 BeginTool ticket | 为每次文件调用注入绑定 ticket 的 MutationRecorder |
| `src/event/chat/tool_calls.rs::begin_tool/finish_tool` | DB pending/running ACK 后执行，结果 DB ACK 后可见 | 增加逐次文件 mutation 的独立持久化 ACK |
| `src/chat/tools/write.rs::run/write_content` | 旧内容读取在锁外，仅最终原子写入加锁 | 读取、规划、快照、修改、观察结果都进同一临界区 |
| `src/chat/tools/edit.rs::commit_one` | 多 hunk 可部分成功；move 先写源，再 rename | 按真实 mutation 记录，不依赖工具整体成功 |
| `src/chat/tools/lock.rs::with_path_lock` | 规范化路径的进程内短锁 | 扩展排序去重的多路径锁，与恢复共用 |
| `src/chat/tools/path.rs::resolve/write_target` | 绝对路径可超出 cwd；write/update 跟随链接，delete/move 操作目录项 | 保存实际操作对象，不能一律 canonicalize 后当普通文件恢复 |
| `src/git/mod.rs` | 唯一执行 Git 的模块；repo_root 是主 checkout | 增加基于实际文件归属的对象读写接口，不能用 repo_root 拼恢复路径 |
| `src/db/mod.rs::SCHEMA_VERSION` | 当前为 11 | 本功能拟新增 v12，实施前再确认未被其他任务占用 |
| `src/db/service.rs::Persistence::start` | 启动无条件恢复遗留 streaming/tool/process 记录 | 改为启动装配层取得 session guard 后，按可恢复 session 集合处理 |
| `src/db/chat/mod.rs::begin_turn` | sequence、turn_index 都由现存 messages 的 MAX+1 分配 | sequence 纳入独立 notice 表；目标身份不依赖可复用的 turn_index |
| `src/chat/context/mod.rs::plan_turns` | 只从 ChatMessage 构建完整 user/assistant 轮次 | notice 类型不进入该接口 |
| `src/ui/main_content/lx/history.rs::layout/render` | 当前直接遍历 messages；几何与绘制必须一致 | 只读合并 messages/notices，维持稳定消息 ID/索引映射 |

当前 schema 的 message role 只允许 user/assistant。`parts`、`tool_call` 随 message 级联删除；`agent_process` 仅关联 session，不会随轮次自动删除。不能把撤销提示伪装成 assistant，也不能认为删对话会停止后台进程。

当前 lx 草稿是 String，并无 Prompt 编辑器那套持久附件对象；路径粘贴最终表现为引用文本。因此恢复原始 user 正文即可，不引入新的附件体系，也不能把发送时附加的系统上下文回填成用户原文。

### 3.2 Codex 与 lx-agent 的取舍

| 来源 | 关键代码 | 采用 / 调整 |
| --- | --- | --- |
| Codex | `codex-rs/tui/src/app/input.rs::should_handle_backtrack_esc` | 空输入、无高优先级面板时才接管 Esc |
| Codex | `codex-rs/tui/src/app_backtrack.rs::apply_backtrack_selection_internal` | 高亮整条用户消息并滚动到可见，不是系统剪贴板文本选区 |
| Codex | `app_backtrack.rs::backtrack_revert_before_turn_id` | 执行前检查稳定目标与最新轮次未改变 |
| Codex | `codex-rs/tui/src/app/event_dispatch.rs` 的 `RevertSessionForPromptEdit` | 成功 ACK 后回填，避免 UI 乐观删除 |
| Codex | `codex-rs/app-server-protocol/src/protocol/v2/thread.rs::ThreadRevertParams` | 原版删目标及后续，且不回滚文件；本项目按已确认要求只允许最新一轮 |
| lx-agent | `src/renderer/src/features/agent/components/AgentInput/AgentInputCommandPanels/components/AgentUndoOptionsPanel.tsx` | 采用四项动作顺序，默认 index 0 |
| lx-agent | `src/renderer/src/features/agent/hooks/useAgentChatTurns.ts::applyUndoOption/removeTurn` | 采用“删对话才回填、仅文件不回填”的区分 |
| lx-agent | `src/main/agent/turnStore.ts::PendingFilePreImage` | 修改前保存原始字节；覆盖 edit/write，不承诺 shell 恢复 |
| lx-agent | `src/main/services/gitSnapshotService.ts` | 借鉴 blob 存储；本项目优先实际项目对象库并提供无 Git 后端 |

不能照搬 lx-agent 的缺陷：仅存 preimage、恢复不检查当前内容、恢复失败仍删除对话、失败工具直接丢快照。这些都会违背多 Agent 隔离要求。

## 4. 交互状态机

### 4.1 入口与键位

```text
Idle + empty input + Esc
  -> Armed（5 秒；仅保存待命状态，不修改消息）
Armed + Esc（5 秒内、相同 pane/session/cwd/revision）
  -> SelectedLatest（高亮最新用户消息，滚动到可见）
SelectedLatest + Enter
  -> ActionPanel（四项，active = 0）
ActionPanel + Enter
  -> Executing（只提交一次）
Executing + durable success ACK
  -> 刷新历史和 tip；若删对话则回填；返回 Idle
Executing + failure
  -> 保留对话与草稿；面板显示原因，允许关闭/重试
```

空闲 5 秒窗口与停止窗口取相同长度，便于使用；两个 arm 字段必须独立。第一次 Esc 后可用已有 toast 显示 `Press esc again to rewind`，不新增一条持久化历史记录。

`/rewind`、`/undo` 在命令候选中指向同一 Rewind 枚举，不出现两个重复候选。采纳命令时锁定同一个最新 user 目标并直接打开 ActionPanel，等价于 SelectedLatest 中的 Enter。也就是输入 `/rewind` 后第一次 Enter 开面板，第二次 Enter 执行；双 Esc 入口则先选中消息，再经同一面板执行。命令采纳自身不执行撤销，也不发送模型请求。

命令仅允许整份草稿是一个独立的单行 slash 查询。完整输入 `/rewind`、`/undo` 可执行；输入 `/rew`、`/und` 或 `/` 后从候选中采纳 Rewind 也可进入选择，沿用现有匹配能力。带参数、多行正文或夹带其他内容时，拒绝该撤销命令并保留草稿；不能清空正文后偷偷执行。普通 slash 候选的既有编辑/过滤行为不变。

### 4.2 各状态规则

| 状态 | Enter | Esc | 上下键 / 鼠标 |
| --- | --- | --- | --- |
| Armed | 清 arm，按普通输入规则处理 | 有效窗口内选最新轮次 | 其他输入、编辑、焦点变化清 arm |
| SelectedLatest | 打开动作面板 | 取消选择，恢复进入前视口 | 上下键不切历史目标；滚轮可浏览但不能改目标 |
| ActionPanel | 执行高亮动作 | 退回 SelectedLatest；再次 Esc 取消 | 上下循环切四项；hover 改高亮，点击按 Enter 语义 |
| Executing | 吞掉，不排第二次执行、不发送消息 | 吞掉，不能在文件恢复中途假装取消 | 禁止修改该 pane 草稿和该会话；其他会话可操作 |

高优先级面板关闭所消费的 Esc 不能算作 rewind 的第一次 Esc。KeyEvent Repeat 不推进 arm/选择/确认，防长按 Enter 连续穿透状态。确认使用离散 Press；异步成功后不得把缓存的重复 Enter 当作发送原文。

忙转闲、流终止、切换工作区、焦点/视图变化、`/new`、`/resume`、会话最新 revision 变化都使旧 arm/selection 作废。执行结果仍按原 session 落库，但迟到结果不得覆盖其他 pane 的新草稿。

### 4.3 操作面板

面板复用 `SlashPanel`、`PanelWindow`、`ui/widgets/command_panel`。贴输入框上沿、同宽，最多现有 MAX_POPUP_ROWS，高度不足时内部滚动，不改变输入框尺寸。标题 `Rewind`，底边 `[select ↵] [back esc]`，按键符号取 `ui::text::key`。

| 顺序 | 稳定动作值 | 面板文案 | 文件 | 对话 | 回填 |
| --- | --- | --- | --- | --- | --- |
| 0 | `revert_and_delete` | `Restore files and undo conversation` | 恢复 | 删最新轮次 | 原用户文本 |
| 1 | `delete_only` | `Undo conversation only` | 不动 | 删最新轮次 | 原用户文本 |
| 2 | `revert_only` | `Restore files only` | 恢复 | 保留 | 不回填 |
| 3 | `cancel` | `Cancel` | 不动 | 保留 | 不回填 |

范围说明固定为 `Files changed by edit/write only`。旧会话没有快照时，含文件动作保持可见但禁用，说明 `File snapshot unavailable`，不得降级成只删对话后声称恢复成功。

如果该轮已执行仅文件恢复，后续仅文件恢复是幂等无操作，不再插新 tip；仍可删除此轮对话。组合操作此时只执行对话删除，提示明确“文件此前已恢复，本次未改文件”，不能再次覆盖别人后来的修改。无文件变更的新轮次属于“已知 0 文件”，不同于“无快照”。

### 4.4 高亮、回填与本地编辑撤销

高亮仅作用于目标 user 消息，使用已有选区/强调样式，不写系统剪贴板、不把 assistant 高亮成用户消息。目标过长时至少显示起始部分，恢复视口使用同一份 history 布局，不另估算行数。Sticky header 若代表同一目标，也要有一致的选择反馈。

对话删除成功后，一次性替换输入正文，光标在末尾，关闭 slash/mention/paste/markdown/prompt-history/rewind 状态。复用输入全文替换方法，只记一次本地编辑 undo，不用 clear + insert 制造两步。原文以持久化 user text 为准，保留换行、中文和文本引用。

成功只回填，不调用发送。失败/取消不覆盖草稿；只有发起 pane 且 pane/session/cwd/输入代号仍匹配时才回填。其他绑定同 session 的 pane 只刷新历史，保留各自草稿。

## 5. 数据结构优先：消息、提示与快照分离

### 5.1 应用层拟议类型

```rust
pub enum RewindAction {
    RevertAndDelete,
    DeleteOnly,
    RevertOnly,
}

pub struct RewindTarget {
    pub session_id: String,
    pub user_message_id: String,
    pub expected_revision: u64,
    // pane、实际执行 cwd 和输入代号由所属状态一起冻结。
}

pub enum SessionHistoryEntry<'a> {
    Message(&'a ChatMessageView),
    RewindNotice(&'a RewindNoticeView),
}
```

`LxChat.messages` 继续只存真实模型消息；新增独立 notices 集合。`ChatMessageView` 保留 DB sequence 供显示排序。显示时间线用只读 merge iterator 合并两类记录，不复制正文、不把视觉索引当消息索引。工具/卡片命中继续携带稳定消息 ID 或显式 message index。

状态只保存选择、面板、执行进度与已加载数据；所有 IO 都经 Effect/事件层。`ui/` 不检查磁盘、不取时钟、不改选择。

### 5.2 独立撤销提示表

拟议 v12 核心表形状：

```sql
CREATE TABLE agent_session_notice (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    operation_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN
        ('revert_and_delete', 'delete_only', 'revert_only')),
    target_user_message_id TEXT NOT NULL,
    restored_file_count INTEGER NOT NULL,
    files_already_restored INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, sequence)
);
```

外键对应当前真实的 `agent_session(external_id)`。target_user_message_id 是审计值，不设到 message 的级联外键，否则撤销自身会把 tip 一起删掉。tip 只保存枚举和计数，不保存整段原文或快照正文。

新消息与 tip 共用 sequence 命名空间。在同一写事务中，对 messages 与 notices 的联合 MAX+1 取号。插 tip 时必须在删目标轮次前先取新序号。保留下来的 tip 保住高水位，下一轮不会因删除最新 message 复用显示序号。迁移不必给旧消息批量增加另一套 timeline_order。

现有 `begin_turn` 只查询 agent_message 的 MAX(sequence)，需要改为共享分配函数；核心查询形状为：

```sql
SELECT COALESCE(MAX(sequence) + 1, 0)
FROM (
    SELECT sequence FROM agent_message WHERE session_id = ?1
    UNION ALL
    SELECT sequence FROM agent_session_notice WHERE session_id = ?1
);
```

该查询和实际插入必须处于同一写事务内。`UNIQUE(session_id, sequence)` 分别存在于两表不能独自保证跨表不冲突，所以不能在事务外先取号再排队插入。

turn_index 仍是既有分组字段，不是恢复身份。快照与操作记录使用 session ID + 稳定 user message ID + operation ID，避免旧 turn_index 被复用后错绑。

### 5.3 快照和操作日志

采用职责明确的四类记录，均由 `db/` 提供 CRUD，只有 `db/` 使用 rusqlite：

| 记录 | 必要内容 | 生命周期 |
| --- | --- | --- |
| `agent_turn_file_scope` | session、稳定 user ID、capture_version、capturing/complete/incomplete、已用快照字节数、files_restored 标记 | begin_turn 同事务创建，所有 mutation 结算后 seal；缺行的旧轮次是 unavailable |
| `agent_file_image` | 原始字节 BLOB / 不存在 / 链接目标、文件类型、可恢复权限；可选 Git OID 及对象库身份 | 被 mutation 或未完成恢复引用时保留 |
| `agent_file_mutation` | session、user ID、tool ticket、mutation ordinal、原始操作路径、实际目标、worktree 身份、before/planned-after/actual-after image ID、prepared/committed/interrupted | 独立于 tool 终态，不能随失败丢弃 |
| `agent_rewind_operation` | operation ID、动作、目标 ID、expected revision、目标路径和 image 引用、阶段、错误分类、提交结果、待回填原文 | 执行幂等、崩溃恢复；成功后可释放不再引用的快照 |

快照不是 tool result_json。diff 被截断、只保留 3 行预览、工具摘要缺失都不影响恢复。二进制、BOM、CRLF、EOF 无换行都按原始字节保存。

scope 用来证明捕获范围完整：新建轮次时就写 capturing，终态持久化必须等待本轮全部文件 worker 与 Prepare/Commit ACK 结算，之后 seal complete。正常失败或手动中断也要走 drain/seal，不能只在模型回答 complete 时 seal。complete 且没有实际变更才是“已知 0 文件”；旧轮次没有 scope、异常退出遗留 capturing、存在未解决 mutation 都是 unavailable/incomplete，不能由“查不到 mutation”推断没有文件修改。恢复时先逐条解决 journal，再决定是否能够 seal；证据不足则保守保留 incomplete，仅开放 delete_only。

每次修改前，before 与 planned-after 必须已持久化；完成后记录实际观察到的 after。记录 planned-after 是为了处理“已经写文件，但还没收到 Commit ACK 就退出”的窗口。不能只在 FinishTool 时首次落库。

拟议资源上限：单份 image 64 MiB、单轮新增快照数据 256 MiB。上限适用于快照能力，不沿用 diff/模型输出截断。超限或快照写盘失败时，在该次 mutation 之前拒绝修改并报告结构化错误；若同工具先前已改了其他文件，照实返回部分成功，已存 journal 保留。这是本设计新增的保护取舍，需纳入审核。生产入口不允许悄悄使用 no-op recorder。

会话删除时清理所属 notice、journal 和孤立 image；成功撤销后，只清理没有未完成操作引用的 image。禁止为了节省空间提前删除仍可被下一轮逐次撤销使用的快照；禁止调用项目 Git GC。

## 6. 无 Git / 有 Git 的统一恢复契约

### 6.1 Git 存储不是工作树回滚

所有 Git 命令仍只在 `src/git/`。按**实际目标路径**查询归属，冻结 worktree root、每 worktree gitdir、git common dir、symbolic HEAD（分支全名或 detached）和 HEAD OID（unborn 为显式状态）。不能从会话共享 workspace key 推导恢复目标。相同目录切换分支不改变 gitdir，所以仅比较目录元数据不足；新 mutation 和恢复前必须同时核对 HEAD 身份，禁止自动跟到另一个 checkout 状态。

Git 后端用原始字节 stdin 写 blob（`hash-object -w --stdin`），避免路径过滤器或换行转换；读 blob 用 `cat-file blob`。这是对象保存/读取，不是提交或工作树重置。Git 子进程在后台执行、有超时和退出码检查，参数独立传递、不经 shell 拼接，并隔离继承的 GIT_DIR/GIT_WORK_TREE 等重定向变量。

linked worktree 共享对象库是正常现象，但对象 ID 不代表恢复位置。恢复位置始终由冻结的目标路径和 worktree 身份决定。例如：

```text
会话 cwd:       /repo/dev
Agent 实际写入: /repo/worktrees/task-a/src/main.rs
快照恢复目标:   /repo/worktrees/task-a/src/main.rs
禁止推导成:     /repo/dev/src/main.rs
```

未跟踪文件、忽略文件、dirty 工作树都能保存 blob，不能只对 Git tracked 文件生成快照。路径在 Git 仓库外时，该路径用无 Git 后端，即使发起会话在 Git 仓库内。

### 6.2 不改 refs 时的对象保活

仅 hash-object 写入的 blob 是不可达对象，可能被用户 Git GC 删除。既然已确认不改 refs，不能承诺 Git 对象永久存在。

因此 SQLite `agent_file_image` 保留字节备份作为持久性兜底：有 Git 时优先写/读项目对象库，同时在 mutation ACK 前提交字节备份；Git 对象丢失时用备份恢复。无 Git 时直接使用同一份字节存储。此处不再建立第二个隐藏 Git 仓库，也不要求无 Git 项目安装 git。

Git 写入失败或 Git 不可用时，只有独立字节快照已成功持久化才能继续；该条记录明确标记实际后端。字节备份也失败则拒绝 mutation。不能拿坏 OID 充当成功快照。

### 6.3 对象类型与路径身份

保存的对象至少区分 Missing、RegularFile、Symlink。普通文件保存原始字节与可恢复的权限元数据。目录、设备、socket 等不是本期可快照对象；在修改前明确拒绝，不做递归备份。

write/update 原本跟随 symlink 改物理文件，快照也记录物理目标与链接解析链；delete/move 操作链接目录项时，保存链接本身及原始 link target，恢复链接不能把链接目标覆盖成普通文件。OS 文件种类、权限、链接重建和文件锁均放 `src/platform/`，测试按平台门控。

新建路径用最近存在的祖先目录身份 + 规范化剩余路径定位，不因“文件原来不存在”退回进程 cwd。执行与恢复前重验父目录、链接解析和 worktree/gitdir 身份；worktree 被移走/删除、路径被重定向、checkout 身份变化都拒绝自动恢复。允许使用字节备份不等于允许换一个路径恢复。

硬链接会使不同路径指向同一对象。不能把路径锁当作硬链接隔离保证；发现需要原地写的多硬链接对象时拒绝恢复，或经平台能力证明采用不影响其他链接的目录项替换。实施测试必须覆盖，不能无说明地跟普通路径处理。

## 7. 多 Agent 并发：先证明不会抹掉别人

### 7.1 捕获和修改共用临界区

生产文件写入顺序：

```text
BeginTool 持久化 / running ACK
  -> 排序、去重并锁定本次 mutation 的全部路径（move 含源与目标）
  -> 在锁内读取当前对象、生成计划
  -> PrepareFileMutation：保存 before + planned-after，等持久化 ACK
  -> 执行实际写入/删除/移动
  -> 观察真实 after（即使实际操作报错）
  -> CommitFileMutation：保存实际结果，等 ACK
  -> 释放路径锁
  -> FinishTool（完整或部分失败结果）
```

不把整个 Agent 采样串行化，不锁全仓库。不同路径的 Agent 可以继续工作。一个 tool 中同路径多次修改分别记录，最后再派生轮次恢复计划。

### 7.2 同文件交错不能只比最后一次内容

轮次按路径保留 first-before 与 last-after，但必须额外检查连续性。该轮再次写同一路径时，新 before 必须等于该轮此前 last-after（含存在性、类型和权限）。不一致即说明中间有其他修改，该路径标记 non-linear，不能生成整轮快照恢复计划。

```text
A1: 0 -> a
B:  a -> ab
A2: ab -> abc

当前 abc 等于 A2 的 after，仍不能恢复 A 的 first-before=0；
否则 B 的 b 也会消失。A2 的 before != A1 的 after，应拒绝撤销 A 的文件。
```

不尝试自动反向 patch 或三方合并。即使两个 Agent 修改同文件的不同区域，本期也保守报冲突；这是为了保证不误删其他修改。其他 Agent 改的是不同文件时不冲突。

### 7.3 恢复前全量预检

取得 session 排他执行权，再按统一顺序取得目标全部路径锁。读取并验证每个文件：

1. target 仍是最新 user，session revision 与打开时一致。
2. 当前会话无在途请求、无存活后台任务、无其他未完成恢复。
3. 快照完整、所有路径线性、无未解决的 prepared/interrupted mutation。
4. 目标 worktree、目录项、物理路径、链接与权限身份仍一致。
5. 当前对象等于该轮最后记录的 postimage；不存在也必须精确匹配。
6. 所有补偿所需的当前状态已可持久保存，恢复目标可访问。

任一失败：零文件写入，不删对话，不回填，不插成功 tip，面板列冲突路径和原因。最多展示固定数量路径，超出显示数量；完整有界路径列表可记录在本地操作错误，不输出文件正文。

### 7.4 进程内与跨进程锁

现有 OnceLock 仅保护同进程。扩展临界区并提供多路径锁；多路径按相同 canonical key 排序，避免 move/restore 死锁。不能递归调用现有同路径非重入锁。

两个独立 lx-tui 实例还需配合跨进程 advisory lock。Rust 工具链当前 1.95，可优先使用标准库文件锁，在 `platform/` 提供稳定 sidecar 锁文件，不能锁会被原子替换的目标 inode。路径锁命名必须跨进程稳定，不能用带随机种子的 HashMap 哈希。可用固定算法的锁桶；碰撞只能导致额外串行，绝不能把两个路径当同一个恢复对象。

同 session 还需一个跨进程 session guard，并在 DB 中保留 revision 与操作幂等键。新轮次、恢复、会话删除使用同一 guard；顺序统一为 session guard -> 路径锁 -> 短 DB 事务。等路径锁期间不得持有 SQLite 写事务。后台等待不能阻塞事件循环。

新轮次必须在 BeginChatTurn 的 DB 事务之前取得 guard，由事件层 turn runtime 持有，贯穿 Committing、模型采样、全部工具、取消 drain、终态/文件 scope 持久化 ACK；只有这些都结束才释放。不能 begin_turn 后就释放，也不能网络任务取消后留下文件 worker 仍写却先释放。Prepare/Commit/FinishTool 的事件处理借用已有所有权 token，不能重新取得同一个 session guard，否则 worker 持路径锁等 ACK 时会死锁。恢复与删除同样持 guard 到最终提交/补偿结束。

如果轮次结束后还有后台进程存活，guard 转由事件层该 session 的运行态继续持有，直到进程全部终态且最后进度已落库；不能让另一个实例把仍存活进程误恢复为 interrupted。同实例后续发送复用该 owner token，但仍按现有单请求状态和 revision 禁止同 session 双轮并发。只有模型请求、文件 worker、恢复操作、后台进程都静止并完成持久化后才真正释放。不得为此在 chat/process 中导入 event 实现。

外部编辑器、Codex 或其他不遵守这些锁的程序，已有修改可以被 postimage/连续性检查发现；它恰好在检查与替换之间写文件的竞态不能被 advisory lock 彻底消除。恢复每次写前再检查，写后再观察；不能宣传为任意外部写入下的文件系统事务。

## 8. 文件恢复与数据库提交不是一个事务

### 8.1 正常执行顺序

每次确认生成唯一 operation ID。相同 ID 重试读取已有结果，不再执行第二次文件恢复或插第二条 tip。

```text
验证目标 / 获取 session guard
  -> DB 记录 operation(preparing)
  -> 获取全部路径锁、重新验证、保存执行前镜像
  -> DB operation(restoring) ACK
  -> 逐路径恢复、观察并记录恢复进度
  -> DB operation(files_restored) ACK
  -> 同一 DB 事务：
       重新验证 latest user / expected revision
       为 notice 分配 sequence
       如需删除，删除该轮真实 messages（parts/tools 级联）
       插入对应 notice
       更新 session revision / 摘要与用量状态
       operation -> committed
  -> 释放锁、成功回执
  -> 所有绑定该 session 的 pane 重载，只有发起 pane 回填
```

delete_only 不读取/恢复文件，但仍检查 latest、session guard、后台任务和幂等条件，在一个 DB 事务里删除轮次并插 tip。cancel 不创建 operation。

仅删对话时文件留在磁盘上，后续撤销更早轮次仍须正常做 postimage 检查；不能因为对应对话已删除就允许覆盖这些保留下来的修改。

### 8.2 中途失败和补偿

文件系统不支持跨多个文件与 SQLite 的统一原子事务。必须把承诺拆开：预检冲突保证零写入；写入阶段发生 IO 故障则尝试补偿，不能声称任何故障都能零影响。

在已持有全部路径锁的条件下，用执行前镜像逆序补偿已经恢复的路径；补偿前检查当前内容仍是本操作刚写的结果，防止覆盖不遵守锁的外部程序。补偿成功：文件回到执行前，对话保留，operation 标 failed。补偿失败：operation 标 recovery_required，保留全部镜像和进度，阻止该会话继续发送/撤销，并显示需要处理的路径。绝不插成功 tip 或假称恢复完成。

文件已恢复但最终 DB 提交失败，同样尝试补偿；如果 DB 提交结果不确定，先按 operation ID 查询是否 committed，再决定，不能重复删除/恢复。

### 8.3 异常退出与恢复

启动/恢复会话时先扫描未完成 operation 和 mutation：

扫描和结算前必须非阻塞取得对应 session guard；取得失败表示其他实例仍持有运行态，跳过该 session，不能标记 incomplete、补偿文件或恢复其 streaming/tool/process。既有 `Persistence::start` 的 `recover_interrupted_streams`、`recover_interrupted_tool_calls`、`recover_interrupted_processes` 也必须一起改成按 session 受保护的恢复，不能保留无条件全库 UPDATE。由启动/事件装配层持 guard 并把明确可恢复的 session 集合交给 db，保持 db 仅依赖 SQLite/标准库。拿到 guard 后仍重新读取状态，不依据启动时的过期扫描结果执行。完整性恢复完成后才释放 guard。

| 状态 | 处理 |
| --- | --- |
| prepared，当前等于 before | 说明未生效；结算无变更，保留必要审计 |
| prepared，当前等于 planned-after | 说明已写但缺 ACK；重建实际结果，不能重跑工具 |
| prepared，当前为第三种状态 | 标记不确定/冲突，禁止文件回退，不能自动覆盖 |
| restoring / files_restored 未 committed | 根据进度和执行前镜像恢复执行前状态；任何第三方变化则 recovery_required |
| committed 但 UI 未收到 ACK | 重载数据库消息和 tip；不重复副作用；待回填原文保留到对应 pane 消费或用户恢复 |

恢复不能自动重执行 edit/write，也不自动完成一次尚未提交的对话删除。成功 tip 与 message 删除在同一事务，因此不会出现“对话已删但永远没有持久提示”的半提交。

## 9. 持久化 tip 的独立表现

tip 是独立历史条目，随列表滚动、重启和 `/resume` 后仍存在；无自动消失时间。不是 toast，不挂在已删除 assistant 下，也不使用 `> ` 或 assistant 的 `■ ` 步骤格式。

建议使用独立两行通知带：第一行 `-- Rewind --`（accent），第二行描述（muted，无用户消息背景，无模型/用量页脚）。窄屏按列换行；路径不直接拼进未经净化的终端控制文本。文案全部集中 `ui/text.rs`，样式集中 `ui/style.rs`。

| 类型 | 示例文案 |
| --- | --- |
| revert_and_delete | `Conversation undone; restored 2 files.` |
| delete_only | `Conversation undone; files kept.` |
| revert_only | `Restored 2 files; conversation kept.` |
| 组合操作但此前已恢复文件 | `Conversation undone; files were restored earlier.` |
| 已知没有文件变更 | 在对应文案中明确 `no file changes`，不声称恢复了文件 |

每次实际成功操作恰一条记录。取消、冲突、失败和重复执行不插成功 tip。此前的 tip 不随下一次轮次撤销而消失；删整个 session 才级联清除。

`LxChat.notices` 没有到 `ChatMessage` 或 `ModelItem` 的转换路径。上下文装配、token 预算、历史整轮裁剪、供应商 wire 请求都只取 messages。测试需覆盖真实 assemble/transport 请求，不能只测“role != user”。删除到最后一轮后即使 messages 为空，只要 notices 非空仍显示历史，不退回盖住 tip 的欢迎页。

## 10. 分层与关键接入契约

不新增通用插件层或第二套聊天系统。建议按现有模块落点：

| 模块 | 职责 |
| --- | --- |
| `app/state/lx/rewind.rs`（新增） | arm、目标、选择/面板/执行状态、动作与 notice view |
| `app/update/chat/rewind.rs`（新增） | 纯状态转换、Effect、成功/失败回执、发起 pane 回填与其他 pane 刷新 |
| `input/` 与 `app/update/pane.rs` | 集中键位路由，接到既有输入动作 |
| `event/chat/rewind.rs`（新增） | session guard、操作协调、后台文件恢复、回执归属 |
| `event/chat/file_snapshots.rs`（新增） | 注入 recorder，桥接 Git/DB ACK，不自己重复写文件工具 |
| 启动装配 / `db/service.rs` | 按 session guard 筛选可恢复记录，避免第二个实例中断第一个实例的运行态 |
| `chat/tools/mutation.rs`（新增） | 文件镜像 DTO、MutationRecorder 契约、受锁保护的修改与恢复原语 |
| `db/chat/rewind.rs`（新增） | image/mutation/operation/notice CRUD、latest/revision 条件事务 |
| `git/snapshot.rs`（新增） | 仅 Git 身份查询、blob 读写；由 mod.rs 导出 |
| `platform/` | 跨进程文件锁、文件类型/链接/权限相关平台能力 |
| `ui/main_content/lx/history/rewind.rs`（新增） | tip 物理行布局与渲染；目标高亮接入既有 history 布局 |

新增文件以真正承担职责为条件，不预建空目录。`app/update/pane.rs`、`db/service.rs`、`history.rs` 已较大，新增分支调用专属模块，不向原文件堆几百行。

工具只依赖 recorder trait，不依赖 event/db 实现或 Git 执行。拟议同步契约：

```rust
pub trait MutationRecorder: Send + Sync {
    fn prepare(&self, change: &PreparedMutation)
        -> Result<MutationTicket, MutationError>;
    fn commit(&self, ticket: MutationTicket, actual: &ObservedMutation)
        -> Result<(), MutationError>;
}
```

每次执行把 BeginTool ticket 传给 ToolExecutor，或构造等价的每调用绑定 recorder；优先显式 ticket 参数，不能按“最后一个工具”推断所有权。user ID、session ID、request generation 来自已确认的 ticket/冻结执行上下文，不信任模型参数。

文件 worker 已运行在 spawn_blocking，可通过 channel 发 Prepare/Commit 请求，用 blocking_recv 等 ACK。事件主循环不能 blocking_recv；Git 子进程/大文件存储放后台任务，完成后由事件层提交 DB 再 ACK。事件层处理 ACK 时绝不再次获取 worker 持有的文件锁，也不能等待该 worker 的 FinishTool，避免死锁。关闭应用或 channel 断开须唤醒等待者并失败退出，不无限等待。

恢复服务复用 mutation 模块的对象读取、锁、原子替换能力，不调用完整 edit/write 工具重新制造一轮模型工具日志。外部模块只经 mod.rs 的 pub use 访问，不深层导入实现。

## 11. 兼容性和失败规则

- 旧会话缺快照：只允许 delete_only，不能拿 Git HEAD、diff 或当前文件伪造原始基线。
- 新轮次确实未写文件：记录可判定的“已知无变更”；含文件动作可成功但显示 0 文件。
- 已停止/失败轮次：只要不 busy、无后台进程且 mutation 已结算，也可操作最新一轮。
- 会话被其他 pane 使用：空闲可共享，但撤销成功刷新全部绑定 pane；有请求则拒绝。
- 命令 cwd 与 session repo key 不同：始终以实际路径记录为准；元数据共享不代表恢复路径共享。
- 恢复时 checkout 切换或 worktree 重建：拒绝自动回退，不依靠分支短名寻找“类似目录”。
- 仅删对话不等于清理后台进程，入口已要求停止所有当前会话活动任务；进程历史记录按现有生命周期保留/显示终态，不制造悬挂的活动工具更新。
- 运行中双 Esc 停止、Ctrl+Z 输入撤销、上下键 prompt-history、Ctrl+O 任务页等既有行为必须回归。
- 不改 `src/tui/` 恢复路径。若实施期间确需改动，必须保留终端恢复顺序并运行 tui::tests。

## 12. 验收重点和限制

必须包含 A/B 不同文件、同文件后写、A1/B/A2 交错、两个独立进程、dev 与 linked worktree 同名文件、会话 cwd 与实际目标不一致等场景。测试用临时目录和临时 Git 仓库，不修改开发仓库的 Git 状态。

文件故障通过注入在第 N 次恢复、Commit ACK、最终 DB commit、补偿和重启时制造，不靠时序 sleep。UI 用 buffer/geometry 测试验证普通/窄/极矮、中文长消息、tip-only 历史、滚动条、sticky 与高亮目标。精确任务和命令见 task.md。

实现前审核需同时覆盖以下新增工程取舍：64 MiB/image 与 256 MiB/turn 限额、无 refs 时保留 SQLite 字节备份、保守拒绝同文件交错、跨进程 session/path guard，以及无法把外部不遵守锁的任意写入变成文件系统事务的限制。它们不能在实现中被默默省略。具体工作拆分与校验见 [task.md](task.md)。

Agent 只做受影响模块的自动验证；实际终端按键和外观由用户运行 `cargo run` 确认。当前阶段不执行 Rust 构建、迁移或功能测试，因为产品尚未修改。
