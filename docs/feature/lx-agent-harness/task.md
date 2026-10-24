# lx Agent Harness 与上下文压缩任务清单

> 实现已完成并合入 `feat/lx-agent-harness`。本文件记录每项的落点、验证命令与未验证风险。

## A. 预备与索引

- [x] 再次确认项目规范和现有未提交工作区，不修改 Git 状态。
  - 主 checkout 只有 `docs/feature/lx-agent-harness/` 未跟踪；实现在
    `.worktrees/lx-agent-harness`（分支 `feat/lx-agent-harness`）内完成。
- [x] 使用 codegraph/codebase-memory 检查每个将修改文件的索引覆盖率；索引落后时先刷新或用源码核对。
  - `codebase-memory-mcp index_repository` 在工作区重建索引（`lx-tui-agent-harness`，
    10798 节点 / 81394 边），`codegraph query` 用于交叉核对符号落点。
- [x] 记录当前 `SCHEMA_VERSION`、迁移列表和 rewind operation/kind 约束。
  - 起点 `SCHEMA_VERSION = 13`（v4/v5/v7/v8/v9/v10/v11/v12/v13），本次顺延到 **14**
    （只增表 `agent_compact_checkpoint`）。rewind kind 约束来自
    `agent_session_notice.kind ∈ {revert_and_delete, delete_only, revert_only}`；
    compact 撤销复用 `agent_rewind_operation`（`action = 'restore_compact'`），
    不新增平行撤销表。
- [x] 以现有测试固定消息 sequence、notice sequence、revision 和 checkpoint 的预期。
  - `db::chat::tests`、`db::chat::rewind::tests` 与新增 `db::chat::compact::tests`
    共同锁定：sequence 与提示/检查点共用编号空间、revision 单调、幂等重放不重复推进。

## B. 系统提示词与环境快照

- [x] 完善 `LX_SYSTEM_PROMPT`，明确先读后改、最小修改、工具结果真实、错误不可伪造、验证命令、工作区边界、后台进程和平台陷阱。
  - `src/chat/prompt.rs`：新增 “Read before you change anything”“Tool results are the
    only source of truth”“Workspace and platform” 三段。
- [x] 增加 `lx` agent 名称和稳定的提示词版本/测试入口。
  - `AGENT_NAME = "lx"`、`PROMPT_VERSION = "lx-2"`（同时写入 `<environment_context version>`）。
- [x] 将环境快照拆为纯函数：输入平台只读信息，输出有界、转义后的结构化文本。
  - `EnvironmentFacts`（采集，`src/platform/environment.rs`）与
    `EnvironmentSnapshot::render` / `environment_snapshot`（纯渲染，`src/chat/prompt.rs`）分离。
- [x] 覆盖 cwd、workspace、OS/arch、terminal、shell、date/timezone、network 状态和受管控后台任务。
  - 快照字段：`os/arch/shell/terminal/terminal_multiplexer/date/time/timezone/network/
    workspace_root/cwd/filesystem/background_tasks`。
- [x] 明确拒绝环境变量、token、密钥、用户目录内容和完整命令行。
  - 只读 `TERM`/`TMUX`/`STY`/`SHELL`/`COMSPEC` 的存在性或白名单化值；后台任务只取
    session_id、状态、cwd 与命令首行（≤120 字符）。代码注释与测试同时锁定该边界。
- [x] 在 turn 创建时冻结快照，后续采样复用同一份快照。
  - `start_stream` 生成一次快照并写入 `ChatRequest.system_prompt`；
    `ModelContext::next_request` 原样复用该字段（`shrink_once` 同样保留）。
- [x] 为 XML/控制字符、长度上限、不可用字段和 macOS `setsid` 说明增加单元测试。
  - `chat::prompt::tests`（转义、`unknown` 字段、`truncated` 上限、`setsid`/`nohup`/`pgrep`）
    与 `platform::environment::tests`（UTC 兜底、TERM 白名单、有界网络探测）。

## C. 文件读后版本保护

- [x] 为 `read` 结果生成稳定的文件版本 token，至少覆盖内容摘要、文件身份和必要元数据。
  - `platform::version_token`（双 64 位 FNV-1a，覆盖 kind/权限/长度/字节/链接目标），
    `read` 结果新增 `file_version` 字段（模型文本、结果 JSON 与 UI 摘要同一来源）。
- [x] 将 token 关联到 session、turn、tool call 和规范化目标路径。
  - `chat::tools::read_version::ReadRegistry` 按 session 隔离、按规范化路径（与路径锁同键）
    登记；写工具在调用内核对，DB 事务再核对一次。
- [x] 在 `edit`、覆盖式 `write`、delete、move 的参数/票据中携带期望 token。
  - `write.read_version`、`edit.read_versions`（路径 → token 映射）；
    `PathMutation.expected_version` 随 Prepare 进入 DB。
- [x] 在 `MutationRecorder`/数据库 prepare 阶段核对 token；不匹配时整批零写入。
  - `db::chat::prepare_mutations` 事务内用 `before` 重算 token：缺 token（非 create）或
    token 不符都返回 `DbError::StaleRead`，事务回滚、无 mutation 行、scope 用量不变。
- [x] 新建文件提交前确认目标不存在；已存在则返回重新读取错误。
  - `read_version::require_missing` 在预检与路径锁内各查一次，返回 `stale_read`。
- [x] 保持现有路径锁、before/planned-after 快照、恢复和补偿逻辑。
  - 版本核对插在原临界区内（锁外预检 + 锁内复核），快照与补偿路径未改；
    既有 rewind 测试全部保持通过。
- [x] 覆盖并发外部修改、缺 token、路径别名和 symlink 身份变化测试。
  - `chat::tools::read_version::tests`（缺 token、外部修改、伪造 token、别名、新建冲突、
    edit 批量拒绝）与 `db::chat::rewind::tests`（持久层二次核对）。

## D. `/compact` 命令与执行

- [x] 在 `SlashCommand` 增加 `Compact`，加入解析、帮助、补全和 busy 状态提示。
  - `SlashCommand::Compact`（别名 `summarize`）+ `LX_COMPACT_*` 文案 + 命令面板条目。
- [x] 在 `apply_slash_command` 增加 idle 检查；streaming、tool running、compact running 时拒绝。
  - `app::update::chat::compact::compact_command`：`RequestState != Idle` 或
    `lx.compact.is_running()` 一律拒绝，不中断当前任务。
- [x] 增加 compact effect/request/result 状态，沿用现有 event/chat agent loop。
  - `Effect::ExecuteCompact(CompactRequest)` → `event::chat::compact` →
    `AppEvent::CompactFinished { outcome }`；不新增 agent loop，摘要请求走既有
    `chat::client::run_stream`。
- [x] 计算上一个 active checkpoint 之后的增量范围和消息边界。
  - `chat::compact::incremental`（按 `base_checkpoint.end_sequence + 1` 起算，
    只纳入可回放完整轮次与终态命令；超限从最老轮次开始丢并标注）。
- [x] 使用当前会话模型发起独立摘要请求，禁止把摘要请求作为普通消息落库。
  - `compact::summary_request`（`RequestPurpose::CompactSummary`、无工具、专用系统提示），
    执行完只写检查点，不写 user/assistant 消息。
- [x] 设计摘要专用提示，要求保留文件改动、错误、验证、决策、未完成事项和工具结论。
  - `COMPACT_SYSTEM_PROMPT` 列出 7 条保留项与 5 条禁止项。
- [x] 设置输出上限；超限、取消、网络或 provider 错误均走失败分支。
  - `MAX_SUMMARY_CHARS`（24000）→ `validate_summary` 返回错误；非 `Complete`
    非零增量与工具调用一律 `CompactOutcome::Failed`。
- [x] 成功事务写入 checkpoint、active 指针、sequence、model_label、覆盖范围和 summary。
  - `Database::commit_compact` 单事务：取号（与消息/提示/检查点共享空间）、插检查点、
    revision+1；`operation_id` 唯一保证幂等。
- [x] 失败不写 checkpoint、不更新 active 指针、不改变原始消息折叠状态。
  - 失败只在事件层清 `running` 并提示；`chat::compact` 与 `db::chat::compact` 测试锁定零写入。
- [x] 成功后 reload 当前 session 和消息列表，展示实际模型名称。
  - `handle_compact_finished` 重新读取检查点与 revision，回填所有绑定该会话的窗格；
    条目头部显示 `model: <name> · HH:MM`。

## E. 数据库与迁移

- [x] 新增或扩展 compact checkpoint/notice schema，遵守唯一 ID、workspace/session 外键和 sequence 约束。
  - `agent_compact_checkpoint`：`external_id` 唯一、`operation_id` 唯一、
    `UNIQUE(session_id, sequence)`、`session_id` 外键级联、`state ∈ {active, reverted}`、
    `CHECK(end_sequence >= start_sequence)`。
- [x] 顺延 `SCHEMA_VERSION`，保留旧库升级路径；检查并发分支迁移冲突。
  - `SCHEMA_VERSION = 14`；`version < 14` 分支只增表并在同事务推进版本。
    已在 v13 库副本上验证：`user_version 13 → 14`、`agent_message` 351 行不变、
    `PRAGMA integrity_check = ok`。
- [x] 为 checkpoint 创建、幂等完成、失败回滚、增量范围和 active/reverted 状态增加 DB 测试。
  - `src/db/chat/compact/tests.rs`（11 项）。
- [x] 为原始消息保留、checkpoint 撤销、session 删除/恢复增加级联和孤儿检查。
  - `revert_only_flips_state_and_restores_previous_active`（消息数不变）与
    `deleting_the_session_removes_checkpoints`（级联）。
- [x] 复用现有 rewind operation 表或明确新增 kind/payload，不建立平行撤销记录。
  - compact 撤销写 `agent_rewind_operation`（`action = 'restore_compact'`, `committed`），
    `revert_compact` 同时更新检查点 `reverted_by_operation_id`。

## F. `/rewind` 集成

- [x] 扩展最新可撤销操作查询，使其能识别普通 turn 和 compact checkpoint。
  - `app::update::chat::rewind::select_latest` 按 `sequence` 比较最新用户轮次与最新
    active 检查点，取更晚者。
- [x] compact rewind 以 checkpoint ID、session revision 和 operation ID 做冻结与幂等校验。
  - `CompactRevert`（checkpoint_id + expected_revision + operation_id）；
    `Database::revert_compact` 校验 session 归属、revision、只能撤销最新 active。
- [x] compact rewind 只恢复上下文指针/状态，不触碰文件 scope。
  - 走 `Effect::RevertCompact`，不发 `PrepareRewind`、不读文件、不改 `agent_turn_file_scope`。
- [x] 普通 turn rewind 继续使用现有文件恢复预检、路径锁、补偿和 revision 检查。
  - `RewindOutcome`/`restore::execute` 路径未改；只新增 `RewindTargetKind::Turn` 分支入口。
- [x] 验证顺序：新普通轮次 → 最近 compact → 更早 compact。
  - `new_messages_after_compact_are_rewound_first`、
    `consecutive_compact_rewinds_step_back_one_checkpoint`。
- [x] 失败和冲突时保持 conversation/files 原状，并显示明确 notice。
  - `handle_compact_reverted` 失败分支只清执行态并提示；DB 失败零写入。
- [x] 追加连续 compact、compact 后新消息、重复 rewind、迟到回执测试。
  - `revert_is_idempotent_for_the_same_checkpoint`、
    `revert_rejects_stale_revision_and_out_of_order_targets`、
    `late_receipt_for_another_session_is_ignored`。

## G. 消息列表显示与交互

- [x] 增加 `CompactNoticeView`，包含 ID、sequence、model、time、range、summary、expanded。
  - `app::state::lx::compact::{CompactNoticeView, LxCompact}`（展开状态存在 `LxCompact.expanded`）。
- [x] 在 timeline 中加入 compact 条目，保持与 rewind notice 相同的 sequence 排序和合并规则。
  - `ui::main_content::lx::history::timeline` 三路合并；连续 rewind 提示仍合并 `xN`，
    compact 条目各自独立。
- [x] 复用 `history/rewind.rs` 的 rows/draw 结构，确保 layout/render 行数完全一致。
  - `history/compact.rs` 与 `rewind.rs` 同结构（`rows`/`title`/`description`/`draw`），
    共用 `wrap`/`visible_row`；测试逐一比对 `layout.total_rows == render(...)`。
- [x] 默认折叠；选中后按既有 `Enter`/折叠入口切换；展开显示摘要、模型、时间和覆盖范围。
  - 鼠标点击整块、`^o`（无工具卡片时作用于最近检查点）、Alt+↑/↓ 选择后 `^o`。
- [x] 将 compact 条目排除出普通模型上下文和复制选区，除非已有历史查看语义明确要求包含。
  - 摘要只经系统段 `<compact_summary>` 注入；条目是纯渲染，不进入 `messages`、
    不参与 `assemble`、也不在复制选区里被当作消息行。
- [x] 更新窄屏换行、滚动、命中测试，避免 viewport anchor 漂移。
  - `compact_entry_wraps_on_narrow_width`（16/24/40/80 列）+ `CardHit` 注册；
    展开/折叠走 `pin_lx_viewport`。
- [x] 增加消息列表加载/重载后展开状态和 checkpoint 状态一致性测试。
  - `LxCompact::replace` 保留仍存在条目的展开状态；`reload_replaces_checkpoints`。

## H. 验证与交付

- [x] 仅针对受影响 Rust 代码运行 `cargo fmt`。
- [x] 运行 `cargo clippy --all-targets -- -D warnings`。
- [x] 运行受影响模块测试，再运行 `cargo test`。
  ```bash
  cargo fmt
  cargo clippy --all-targets -- -D warnings
  cargo test          # 3174 passed (lib) + 全部集成测试通过
  ```
- [ ] 检查 TUI 交互需用户手动 `cargo run` 验证：slash 帮助、compact、折叠/展开、rewind 顺序和窄屏布局。
  - **未验证（需用户）**：本次实现未连接真实 provider 跑过 `/compact` 端到端，也没有人工
    检查窄屏观感。
- [x] 检查迁移副本的 `user_version`、数据量和 `PRAGMA integrity_check`，保留备份。
  - 备份：`~/.lx-t-dev/db/lx.db.v12.bak`（既有）；本次在副本
    `/tmp/lx-migtest/lx.db` 上验证：`13 → 14`、`agent_message` 351 行、integrity ok。
- [x] 更新本任务文档的完成状态、验证命令和未验证风险。

## I. 修复：`/compact` 必然失败（`summary request failed: cancelled`）

- [x] 定位根因：`event/chat/compact.rs::run_compact` 写 `let cancel = watch::channel(false).1`，
      `watch::Sender` 当场被丢弃 → `run_stream` 内 `cancel.changed()` 立刻返回 `Err`，
      `select!` 把它当成用户取消，摘要请求**在发出前**就被判为 `Cancelled`。
- [x] 修复：`client::uncancellable()` 返回 `(Sender, Receiver)` 并要求调用方把 sender 一起活到请求结束；
      新增 `wait_cancel` 只在**收到 `true`** 时算取消，sender 全丢不算（改按请求自身超时/EOF 收尾）。
- [x] 回归测试（真实 mock SSE + 真实 DB）：
  - `chat::client::tests::dropped_cancel_sender_is_not_a_cancellation`
  - `chat::client::tests::explicit_cancel_signal_still_cancels`
  - `event::chat::tests::compact::{compact_commits_a_checkpoint, failed_summary_writes_nothing,
    empty_summary_is_rejected, summary_with_tool_calls_is_rejected,
    repeated_commit_with_same_operation_is_idempotent}`
  - 验证方式：把修复临时回退回原写法后，上述 compact 测试精确复现
    `Failed { message: "summary request failed: cancelled" }`。

## J. 修复：lx 页 toast 必须显示在 lx 内（不用全局 toast）

- [x] `AGENTS.md` 增加硬约束（见「Agent 执行规则」中的 lx toast 条目）并同步
      `docs/standards/tui-design-requirements.md` 第 117 行口径。
- [x] `Lx.toast: Option<Toast>`：每个 lx 窗格各一条，状态/到期/hover 语义复用 `app::toast::Toast`。
- [x] `ui/main_content/lx/toast.rs`：盒矩形**夹在窗格内容区内**（含左右 1 列留白）、长文案折行、
      正文行数上限 4、三档边框/标题色与全局 toast 同源、`[✕]` 可点关闭、整块铺 `overlay_panel` 不透明。
- [x] 写入口统一：`update::show_lx_toast` / `show_toast_at`（按锚点窗格视图分派）/
      `show_lx_toast_for_session`（会话级效应投给所有绑定该会话的 lx 窗格）。
- [x] 交互：`tick` 过期清理、`next_deadline` 纳入唤醒、hover 冻结、点 `[✕]`/点盒外关闭
      （盒外点击不吞掉本次点击）、命中与绘制共用同一份几何。
- [x] 改写的调用点：`show_lx_hint`、compact 成功/失败/空增量、rewind 提示、会话 guard 阻塞、
      工具结果落库失败、后台任务停止反馈与 user shell 快照保存失败、lx 历史/输入框复制反馈、
      `/resume` 复制会话 id（`Effect::CopyToClipboard` 的锚点在 lx 窗格时）、`LoadChatSession`/
      `DeleteChatSession` 失败。
- [x] 测试：`ui::main_content::lx::toast::tests`（9 条纯几何/绘制）、
      `event::tests::lx_toast`（8 条端到端：只写窗格槽、窄窗格内折行、整帧绘制位置、
      点击关闭、hover 冻结、多窗格互不覆盖、compact 失败/空增量落位）。

## K. 调整：压缩提示词改用 codex 原文 + 压缩中显示 loading

- [x] 摘要系统提示词改为**逐字采用 codex 的压缩提示词**
      （`codex-rs/prompts/templates/compact/prompt.md`，`CONTEXT CHECKPOINT COMPACTION` 那段），
      不再自造第二套清单；`compact_prompt_matches_codex_verbatim` 锁死原文。
- [x] lx 侧的保留要求（精确路径、命令与退出码、错误不可隐瞒、验证命令、未完成事项、
      跟随用户语言、不复述原始文件内容）移到摘要请求的 **user 段**，
      `summary_request_states_the_lx_requirements` 锁定这些要点不得丢。
- [x] 压缩期间输入框上方那一行显示 loading：`LxCompact.running` 期间固定显示
      `Compacting context...`（扫描高亮），统计段换成**本次压缩耗时**（`started_at_ms` +
      `elapsed_ms`，由 100ms 心跳刷新），不显示上一轮的耗时/token，也不显示 Esc 停止提示。
- [x] `has_streaming_lx` 与 `advance_lx_loading` 纳入压缩状态：压缩期间同样按帧推进、
      写脏并刷新计时；`start`/`finish` 成对管理幂等键与计时，会话重置一并清除。
- [x] 测试：`ui::main_content::lx::chat_bar::tests::compacting`（6 条：可见性、扫描高亮、
      本次耗时、无耗时、不出现停止提示、与回到底部共存、结束后消失）、
      `main_content::lx::tests::compacting_shows_loading_bar_above_input`、
      `advance_lx_loading_drives_the_compact_timer`、`start_and_finish_carry_the_timer`、
      `restart_resets_the_timer`，以及 `/compact` 状态测试中的计时断言。
- [x] 口径同步：`docs/standards/tui-design-requirements.md` 第 117 行 loading 条目。
- [x] 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`
      = 3188 passed (lib) + 全部集成测试通过。

## L. 评审与修复

- [x] 全量评审 `dev..feat/lx-agent-harness`，结果与逐项错误原因见 [review.md](review.md)。
- [x] 修复两个会直接让功能失效的缺陷：
  - `ModelContext::next_request` 曾把 `system_prompt` 置 `None`，冻结环境快照与 compact
    摘要从未进入模型；改为原样复用，并补“每次采样拿到同一份提示”的回归测试。
  - `edit` 的 `move_dst` 被持久层按“缺 token”拒绝，`*** Move to:` 生产路径完全不可用；
    改为与 `create` 同档，并在路径锁内复核目标不存在。
- [x] 修复回执与多窗格缺口：回执携带 `operation_id` 与冻结 `workspace`，迟到回执丢弃；
  回填读失败不再覆盖检查点与 revision；`/compact` 与发送按会话检查忙碌；
  `^o` 优先切换 Alt+↑/↓ 选中的检查点；撤销失败使用独立文案。
- [x] 修复 UI/平台细节：lx toast 宽高与折行统一并补省略号；环境快照按行截断；
  网络探测移出事件循环（缓存只读 + 后台补测）。
- [x] `history.rs` 时间线拆分到 `history/timeline.rs`（回到 1000 行内）；删除死代码，
  修正 `search`/`read` 的提示口径。
- [x] 验证：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、
  `cargo test` = 3199 passed (lib) + 全部集成测试通过。

## 未验证风险

1. **真实 provider 的摘要请求**：单测覆盖请求形状（无工具、系统提示覆写、失败分支），
   但没有对真实端点跑过 `/compact`；流式响应若带 reasoning part 会被忽略（只取 text）。
2. **长会话的摘要质量**：`MAX_TRANSCRIPT_CHARS` 超限时从最老轮次丢弃并显式标注；
   被丢轮次不会进入后续上下文（review.md m8），模型是否据此保留足够信息未做人工评估。
3. **读后版本保护的模型遵从度**：系统提示与工具描述都要求携带 `read_version`，
   但模型可能反复忘记；此时写工具返回 `stale_read` 而非自动放行（安全但可能多一轮往返）。
4. **`^o` 的语义扩展**：无工具卡片时改为切换最近 compact 检查点；如果用户习惯
   “`^o` 总是打开后台任务页”，行为发生了变化，需要人工确认。
5. **迁移并发**：v14 只增表，但若其它分支同时顺延到 14 且内容不同，仍需按
   `AGENTS.md` 的人工顺延规则处理。
