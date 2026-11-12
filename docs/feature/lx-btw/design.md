# `/btw`：按主 QA 隔离的持久化旁路对话

> 状态：**已在 `feat/lx-btw` 实施最小可用版本；未合并，遗留项见 [task.md](task.md) §11.2。**
>
> 创建：2026-11-11；本次整理：2026-11-12。
>
> §2 是用户逐项确认的产品决策；§5 起是据此收敛的实现方案和建议默认值。`feat/lx-btw` 已合并到 `dev`（merge `4c925888`）；执行清单见 [task.md](task.md)，问题与修复状态见 [review.md](review.md)。

## 1. 目标与范围

在 Native Agent 的主输入框增加 `/btw [prompt]`，采用 `/rename` 同款命令回显和灰色参数提示。在现有后台任务面板追加 `BTW` tab，提供来源明确、可连续提问、可恢复的独立对话。BTW 输入框复用主输入框的编辑、提及与粘贴能力；用户可以与 monitor、运行中或已结束的 subagent 交流。

功能入口在 `src/ui/main_content/lx/`，实现跨越 `app/`、`event/`、`chat/`、`db/`；不是仅增加一个 UI tab。渲染只读状态，状态更新只产生 Effect；文件、数据库与网络 IO 由对应服务及事件层执行。

## 2. 已确认决策（D1～D17）

| 编号 | 用户确认的最终结论 |
| --- | --- |
| D1 | BTW 问答及其中 `@monitor/@subagent` 的直接回复不自动回传主 agent；只有用户明确转交才进入主会话。 |
| D2 | BTW 及其中被 `@` 唤醒的问答轮次只读，可读取文件、搜索代码、查看状态；修改文件和运行命令要转交主 agent。 |
| D3 | 归属为 **主会话 → 主 QA → 一条连续 BTW**。同一主会话的不同 QA 也相互独立。最初“整个会话共用一条”的推荐已被用户修正。 |
| D4 | 一条主用户问题及其后续思考、工具调用和回答构成主 QA。BTW 自己的追问不是新的主 QA。 |
| D5 | BTW 读取截至所属 QA 的主会话历史，不自动带入其他 BTW 或所属 QA 之后的主对话。 |
| D6 | 保存 BTW 问答、主会话/QA 归属及目标身份；退出并恢复主会话后仍可继续。 |
| D7 | 删除主 QA 时删除对应 BTW，并停止在途回答。仅恢复文件、保留 QA 时保留 BTW。重发相同文字是新 QA。 |
| D8 | BTW 中 `@subagent` 使用独立只读回答流，引用其身份与任务记录；不写入原任务会话，不打断原任务，不产生原任务父报告。 |
| D9 | 可主动提及同一主会话中其他 QA 创建的实例，仅显式引入该实例记录，不连带加载其他 QA 的主对话或 BTW。 |
| D10 | 一条问题可提及多个实例，各自回答并显示身份；同一实例按稳定 ID 去重，仅执行一次。 |
| D11 | 没有再次选择 agent 时沿用当前 BTW 上次选择的回答者集合；初始为默认 BTW 助手。输入框显示目标，可切回默认助手。 |
| D12 | 主任务执行中的 steer 补充属于原 QA，共用 BTW；结束后正式提交新问题才建立新 QA。 |
| D13 | 每次发送刷新允许范围内的已完成内容和任务状态；不把半截生成回复当成完整结论。 |
| D14 | 转交先把内容及 QA、回答者来源放入所属主会话输入框，用户编辑后发送；点击转交本身不启动主请求。 |
| D15 | 本次完整支持 Native Agent。Web、Hybrid、Automation 不在此次范围。 |
| D16 | 关闭面板或切换主会话后，BTW 继续后台运行；主动停止、删除所属 QA/会话或退出应用时停止。重启不自动重发。 |
| D17 | BTW 仅提及已有实例，不因为角色提及自动创建任务。找不到实例时提示并保留草稿，新任务由主 agent 创建。 |

## 3. 参考实现与检索依据

### 3.1 对照结论

| 参考 | 可借鉴之处 | 与本需求的区别 |
| --- | --- | --- |
| Claude Code | 基于主上下文回答旁路问题，不污染主历史 | 官方文档描述无工具访问；终端旁路记录只留内存 |
| OpenCode TUI | `/btw` 注册、当前 session/model、临时生成 | one-shot，无工具执行循环，答案显示在 dialog |
| OpenCode GUI | session 范围的 side tab、来源/模型与加载状态 | 每题一个 tab，不是 tab 内连续多轮 |
| Codex | `/btw` 为 `/side` 别名，独立 fork、多轮输入、父来源展示 | ephemeral，关闭丢弃；旁路指令禁止接触 subagent |

本需求的“持久化 + 按 QA 分组 + 多目标 subagent 对话”需要自行设计，不能宣称参考项目已有同等功能。

Claude Code 官方资料经 Context7 查询：<https://code.claude.com/docs/en/interactive-mode>。

OpenCode 根目录 `/Users/yonah/projects/agent/opencode-dev`：

- `packages/tui/src/feature-plugins/prompt/btw.tsx`：`session.aside`、`slash: { name: "btw", arguments: true }`。
- `packages/core/src/session/generate.ts`：`SessionGenerate.generate`，预览主上下文，在内存中追加问题，不修改历史。
- `packages/protocol/src/groups/session.ts`：`/api/session/:sessionID/generate` 契约。
- `packages/gui-plugins/src/btw/{index.ts,model.ts,renderer.tsx,panel.tsx}`：session 存储和 side tab。
- `packages/app/e2e/regression/btw-sidebar.spec.ts`：独立 one-shot 答案及 reload 恢复。

Codex 根目录 `/Users/yonah/projects/agent/codex-main`：

- `codex-rs/tui/src/slash_command.rs`、`chatwidget/slash_dispatch.rs`：命令与提问分派。
- `codex-rs/tui/src/app/side.rs`：`handle_start_side`、`side_fork_config`、参考历史边界和关闭清理。
- `codex-rs/tui/src/app_server_session.rs::fork_side_thread`：独立 thread 创建。

### 3.2 MCP 使用与覆盖边界

当前仓库已结合 codegraph 和 codebase-memory 的图发现、调用关系、源码片段与 coverage 检查。codebase-memory 使用较新的 `Users-yonah-projects-agent-lx-tui` 项目；首批关键路径返回 `metadata_match / no_recorded_issue`，仅作为 best-effort 信号。

两个参考仓库没有 codegraph 索引，按工具要求停止调用；使用 codebase-memory 图发现后直接读取源码补足过期元数据。后续调研中 codebase-memory 连接关闭并从工具目录移除，追加事实使用 codegraph 与直接源码读取；没有伪称追加 coverage 已通过。

## 4. 当前实现中的关键事实

### 4.1 命令与输入

`src/app/state/lx/slash.rs` 已支持 `echo_prefix()`、`argument_hint()`、`active_argument_hint()`；`src/app/update/chat/slash.rs::accept_slash_command` 复用这些元数据完成 `/rename ` 回显。

```rust
// 现有机制；BTW 按相同分支扩展。
match self {
    Self::Rename => Some("/rename "),
    _ => None,
}
```

`src/app/update/chat/mod.rs::try_send` 目前先处理 shell、rename、shell 忙时排队、steer。BTW 必须在聊天排队及 steer 之前拦截，否则生成期间的 `/btw` 会进入主任务。

### 4.2 面板与编辑器

- `src/app/state/lx/task.rs` 只有 `TaskTab::{Processes, Agents}`，`TaskPage` 派生 `Clone/PartialEq/Eq`。
- `src/ui/main_content/lx.rs::layout` 在 task 打开时把主 `input` 几何置空；现有 `@`、paste 等浮层因此隐藏。
- `src/app/update/pane/task_page.rs` 接管 task 模式键盘；现有 agent reply 是单行 `TextInput`，不能满足完整 composer。
- `src/app/state/lx/ask.rs::AskEditor` 已有“私有包装 Lx，仅暴露编辑方法”的先例。
- `SubagentRun` 已用独立 `LxChat` 和 `HistorySource::from_chat` 渲染子会话，可复用阅读/卡片/选区基础能力。
- 提及扫描目前仅按 pane、generation、root 路由；新增多个 composer 后必须加强归属键。

### 4.3 agent 与权限

- 控制工具已有 `spawn_agent/send_input/interrupt_agent/resume_agent/wait_agent/close_agent/list_agents/cancel_agent_input`。
- `runner::send_input` 在终态要求先 resume；`send_user_input_from_panel` 已能先恢复 completed 再发送，但 UI 拒绝 cancelled，工具层允许恢复，需统一。
- `@` 候选目前只列活动实例；`subagent_reference_blocks` 已能解析手写的已结束实例并指导主 agent 复用。
- Monitor 的工具面仍含 `exec_command/write_stdin`，不符合 D2，不能直接拿来当 BTW 权限。
- 原任务追问写入原 child session/输入队列，并在 `finish_subagent` 后产生父报告。BTW 不能直接走此链。
- `run_agent_turn` 自身与报告生命周期解耦，且检查冻结 `request.tools`；可复用 loop，独立编排与结算。
- `McpToolEntry.read_only_hint` 已存在；普通 `turn_tools_for_subagent` 并未按这个字段过滤，BTW 需专用过滤路径。
- `LxChat` 的流方法通过 `streaming_message_mut()` 定位单个 assistant。多目标共用一个 BTW 展示历史时，不能把不同 recipient 的 delta 直接交给该单请求入口。

### 4.4 QA 与数据库

- `latest_user_turn` 只排除 `user_shell`。steer 和自动报告也创建 user/assistant 和 `turn_index`，不能直接用它划分主 QA。
- `src/db/chat/steer.rs` 的 steer user 带 `stop_reason='steer'`；自动报告 user 带 `subagent_report_json`。
- `turn_index`、sequence 在删尾后可能重新使用；QA 身份必须用稳定 message ID。
- `/compact` 保留原始消息，可按旧 QA 范围重建上下文。
- `commit_rewind` 只在 `delete_conversation` 时删除消息；仅恢复文件不删除消息。
- 调研时 `SCHEMA_VERSION=27`；`agent_session.source` CHECK 只有 native/subagent。实施前再次核对实际版本。
- `src/db/chat/tools.rs::begin_tool_call` 按 **session_id + call_id** 检查重复，不是按 assistant 消息隔离；多目标共享 BTW child session 时需显式命名空间化存储调用 ID。

## 5. 审核方案的默认行为

以下是整份设计的建议默认值，**不是额外已经确认的用户答复**。

| 场景 | 方案 |
| --- | --- |
| 主输入 `/btw prompt` | 归属当前主会话最新正式 QA，打开其 BTW 并提交问题；不随主历史滚动位置改变归属 |
| 空 `/btw` | 采纳候选回显 `/btw ` + 灰色 `[prompt]`；再次空参提交提示 `Enter a question after /btw`，保留草稿 |
| 没有主 QA | 提示 `Start a main conversation first`，保留草稿；不隐式创建主 QA |
| 多行 | `/btw` 必须是首个非空白命令词，后续可以多行；`/btwfoo` 不匹配；代码围栏内文字不触发 |
| 查看旧 QA | BTW tab 的 QA 选择器列出当前主会话的正式 QA，可打开尚无 BTW 的旧 QA；第一条有效提问才创建持久化线程 |
| 模型 | 默认助手每批冻结当前主会话有效模型；指定实例优先其持久化模型。模型不可用则提交前提示，不静默换模型 |
| 目标集合 | 本次明确实例集合替换旧集合，非追加；普通文件/Skill/MCP 引用不改集合；`Reset to lx` 切回默认助手 |
| 终态实例 | completed/failed/interrupted/cancelled 都可被提及；deleted/不存在的实例拒绝，保留草稿 |
| 同名实例 | 候选显示昵称、角色、状态、短 ID；采纳保存稳定 ID。手写歧义时提示选择，不猜最近一个 |
| 生成中再发送 | 每个 BTW 线程按 FIFO 排队批次，不 steer、不打断前批；每线程最多 4 个待运行批次，满时保留草稿 |
| 并发 | 全局最多 2 个 BTW 回答执行流，独立于主 subagent 配额；每批最多 8 个去重目标，公平分配运行槽位 |
| 转交 | 单条回答下 `Send to main`，追加“该问题 + 该回答 + 来源”到主草稿，一次撤销；不自动汇总整个 BTW |
| `/fork` | 新主会话不复制旧 BTW，避免复制原实例关系；旧主会话的 BTW 保留 |
| 草稿 | 按 pane + 主 session + QA 独立保留，切面板/QA/主会话均不串用；重启保证已发送问答恢复，未发送草稿不承诺跨重启 |

## 6. 界面与交互契约

### 6.1 正常布局

> **历史方案**：下面的“多行 QA 列表 + QA 列表焦点”已被 §15 的单 QA 视图取代；当前 `/btwList` 是唯一 QA 切换入口。本节保留用于追溯，不得按旧契约验收。

```text
┌─ lx-[Native Agent] ────────────────────────────────────────┐
│ BTW · Fix login · QA 3: Support Safari                    │
│ > Why does this check fail?                               │
│ Turing [explorer]                                         │
│ ● The callback drops the cookie ...                       │
│                                      [Send to main]      │
│ Hopper [monitor]                                          │
│ ● The last completed run failed in ...                    │
│                                      [Send to main]      │
│ ─ Processes ─ Agents ─ BTW ───────────────── [close esc] ─ │
│ QA  ❯ 3  Support Safari                 2 replies         │
│       2  Fix login                      running           │
│ To: Turing, Hopper                         [Reset to lx]  │
│ ──────────────────────────────────────────────────────── │
│ > Follow-up question · @ files and subagents              │
│ ──────────────────────────────────────────────────────── │
└─ [tab focus] [↵ send] [^d stop] ────────────────────────────┘
```

这是同一后台管理页内的第三个 tab。对话区复用原任务输出区；底部按“tab 行 → QA 列表 → 回答者 → composer”组织。Processes/Agents 保持原交互。

QA 列表默认最多 3 行，超过可滚动；只有一个 QA 时可收成单行来源。composer 使用主输入框相同软换行/滚动规则，不因 hover、加载动画改变尺寸。对话按批次及目标选择顺序固定位置；后完成的回答不重新排序。

### 6.2 焦点和键位

> **历史方案**：QA 列表焦点行已移除；实际焦点只有 composer / conversation，`/btwList` 用二级面板。Shift+Tab 的完整支持仍列在 review.md P3。

| 焦点 | 方向键 | Enter | Tab / Shift+Tab | Esc |
| --- | --- | --- | --- | --- |
| Composer | 正常编辑、选择 | 发送/排队 | 无候选时切下一/上一焦点区域 | 有浮层先关浮层，否则关后台页；任务继续 |
| QA 列表 | 上下选 QA；左右切后台 tab | 打开选中 QA 并聚焦 composer | 切焦点 | 关闭后台页 |
| 对话 | 上下滚动，PageUp/Down 翻页 | 保留已有卡片交互规则 | 切焦点 | 关闭后台页 |

- 浮层优先消费方向键、Tab、Enter、Esc；Shift/Ctrl+Enter 换行。
- `Ctrl+D` 仅在 QA 列表/对话焦点用于停止当前 BTW 的活动及待运行批次；composer 内保留编辑键语义。另提供明确 `Stop` 点击目标。
- 关闭面板不等于停止；不把主聊天的双 Esc 停止误路由到后台 BTW。
- 点击输入框聚焦并定位；滚轮分别作用于对话、QA 列表、composer/提及面板，不穿透底层主历史。
- 对话复制、代码块复制、工具树折叠与主聊天一致；所有反馈使用该 lx 窗格 toast。

### 6.3 小尺寸降级

按显示列计算，先保留 tab、至少一行 composer、可辨认的当前 QA/目标，再压缩列表和对话。

- 宽度不足 40 列：QA 列表收成 `QA 3/12` 和可点击前后项；长昵称截断，短 ID 保留可辨认尾部。
- 高度不足 12 行：QA 列表收成单行，不再固定占 3 行。
- 高度不足以同时显示三块：先隐藏对话，保留输入和切换入口；极小区域使用现有尺寸不足提示。
- 所有 rect 使用 saturating/clamp，不把浮层挤出窗格；新几何同时用于绘制、光标、点击、拖拽和选区。

## 7. 状态与编辑器复用

### 7.1 归属键

以下均为拟新增类型示意，具体可见性按模块边界收敛：

```rust
struct BtwOwner {
    parent_session_id: String,
    qa_anchor_message_id: String,
}

struct BtwRunKey {
    request: RequestKey,
    thread_id: String,
    batch_id: String,
    recipient_id: String,
}

enum ComposerOwner {
    Main { pane: PaneId, session_id: Option<String>, editor_epoch: u64 },
    Btw { pane: PaneId, owner: BtwOwner, composer_id: u64 },
}
```

`RequestKey.session` 是 BTW child session，generation 为每个 recipient 分配，不能从主聊天当前 generation 推导。pane 仅为发起/显示来源，不决定请求能否继续。运行时以 owner + recipient 查找真实归属。

### 7.2 独立 composer

采用既有 AskEditor 包装模式，以最小改动复用编辑器：

```rust
struct BtwComposer(Box<Lx>); // 字段私有，不暴露 DerefMut

struct BtwThreadView {
    owner: BtwOwner,
    thread_id: Option<String>,
    composer: BtwComposer,
    chat: LxChat,
    recipient_states: BTreeMap<String, BtwRecipientView>,
    targets: Vec<BtwTarget>,
    follow: bool,
    scroll: usize,
}
```

- `Lx.btw` 保存当前 pane 的 BTW 视图缓存，按完整 owner 键索引，不能仅按当前 QA 列表下标索引。
- `LxBtw::default()` 只创建空映射，绝不创建 composer。打开 QA 时才构造 `Box<Lx>`，其内部 `btw` 仍为空；Box 解决尺寸递归，惰性构造另外解决无限初始化。
- 每个 QA 有自己的文本、光标、撤销、选择、粘贴附件、提及状态；不能只维护一个共用 composer。
- 保留 markdown 面板、列表续写、原子芯片、多行粘贴、图片校验、编辑快捷键。禁用 shell 自动识别及 `/new` 等会改变主会话的 slash 分派。
- 主输入的 edit helper 应抽出必要的纯编辑分派供两侧调用；不把 BTW 输入重新送入 `apply_lx_input` 造成递归开页。
- 切主会话只切 BTW 当前可见 owner，保留原 owner 草稿缓存。关闭 pane 后已发送请求继续，由数据库恢复已发送历史。
- 历史消息可按现有 Agents 惰性加载策略卸载；有未发送草稿的 composer 不作为历史缓存随意淘汰。

### 7.3 浮层与异步扫描

不能只修改 `input_rect()`：浮层还读取编辑器里的 mention/paste/refer 状态。新增只读 composer view，同步提供 `{ owner, editor, input_rect }`；渲染和命中必须使用同一 editor 与 rect。

扫描回投携带 `ComposerOwner + scan_generation + root`；接收端逐项校验。旧 QA、旧主 session、旧 composer 实例的结果不能落到当前 composer。路径根取所属主会话冻结的工作区/cwd，不取切换后的当前工作区。

主 composer 的 `editor_epoch` 在切主会话、`/new` 和编辑器重建时递增，即使 cwd 相同也必须变化；scan generation 只反映扫描本身，不能替代编辑器生命周期键。空白会话的 `session_id=None` 也由 epoch 区分。

同样的 owner 校验应用于异步路径粘贴解析、剪贴板图片准备与引用预览结果；不能只修复文件扫描，却让其他粘贴 Effect 仍凭 pane 写回当前主输入。提交附件引用的文本 revision 也要冻结，用户切 QA 或编辑后，旧结果只能回到原 composer 或被丢弃。

### 7.4 回答者与模型信息

用 `HistorySource` 的类型化 speaker 映射 `(assistant_message_id → nickname/role/agent_id)` 绘制回答者行；普通主历史映射为空。保留 `ChatMessageView.model_label` 的真实模型用途。speaker 行纳入 snapshot 行数、缓存 key 与命中几何，不在 paint 阶段额外补行。

`BtwThreadView.chat` 仅作为有序阅读投影，不能用它唯一的 `RequestState` 表达多路生成。每个 recipient 单独保存流状态、usage、耗时与 assistant ID。对 `src/app/state/lx/chat/stream.rs` 做必要的职责提取：消息级变换接收明确的 `&mut ChatMessageView`，主聊天原方法仍选中原 streaming message 后调用；BTW reducer 按 assistant ID 找消息再调用同一原语。禁止临时交换整个 thread 的 request 状态来“假装当前目标”，也不在每个 token 后克隆全部历史。

## 8. 数据库模型与迁移

### 8.1 source 方案

采用独立 `source='btw'`，复用 session/message/part/tool/attachment 表，新增三张关联表。相比 native + 隐藏关联的方式，普通 `source='native'` 查询天然排除 BTW，减少漏过滤。

迁移版本在实施前重新核对；若仍为 27，则使用 v28。下面 `agent_session_btw_next` 为迁移临时表名，不承诺版本号占用。

```sql
CREATE TABLE agent_session_btw_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL REFERENCES workspace(external_id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'native'
        CHECK (source IN ('native', 'subagent', 'btw')),
    title TEXT NOT NULL,
    first_user_message TEXT NOT NULL DEFAULT '',
    model_provider TEXT,
    model_id TEXT,
    variant TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'closed')),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    agent_nickname TEXT,
    agent_role TEXT,
    agent_path TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    last_message_at_ms INTEGER NOT NULL,
    total_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_input_tokens >= 0),
    total_output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_output_tokens >= 0),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)
);
INSERT INTO agent_session_btw_next (
    id, external_id, workspace_id, source, title, first_user_message,
    model_provider, model_id, variant, status, archived,
    agent_nickname, agent_role, agent_path, created_at_ms, updated_at_ms,
    last_message_at_ms, total_input_tokens, total_output_tokens, revision
)
SELECT id, external_id, workspace_id, source, title, first_user_message,
       model_provider, model_id, variant, status, archived,
       agent_nickname, agent_role, agent_path, created_at_ms, updated_at_ms,
       last_message_at_ms, total_input_tokens, total_output_tokens, revision
FROM agent_session;
DROP TABLE agent_session;
ALTER TABLE agent_session_btw_next RENAME TO agent_session;
CREATE INDEX idx_agent_session_workspace_recent
    ON agent_session(workspace_id, archived, updated_at_ms DESC, id DESC);
```

执行前取实际 schema 的列、索引、触发器清单，防止实施前外部变更被丢弃。按 SQLite 官方 generalized ALTER TABLE：事务外关闭外键 → 事务内建新表/逐列复制/校验/替换/恢复索引触发器 → `foreign_key_check` → 提交 → 恢复外键。失败路径同样恢复原外键配置，禁止半迁移推进版本。

资料经 Context7 核实：<https://www.sqlite.org/lang_altertable.html>、<https://www.sqlite.org/foreignkeys.html>。

### 8.2 关联表

```sql
CREATE TABLE agent_btw_thread (
    external_id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL
        REFERENCES agent_session(external_id) ON DELETE CASCADE,
    parent_user_message_id TEXT NOT NULL
        REFERENCES agent_message(external_id) ON DELETE CASCADE,
    child_session_id TEXT NOT NULL UNIQUE
        REFERENCES agent_session(external_id) ON DELETE CASCADE,
    answerer_set_json TEXT NOT NULL DEFAULT '[]',
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(parent_session_id, parent_user_message_id)
);
CREATE INDEX idx_btw_thread_parent ON agent_btw_thread(parent_session_id);

CREATE TABLE agent_btw_batch (
    external_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES agent_btw_thread(external_id) ON DELETE CASCADE,
    user_message_id TEXT NOT NULL UNIQUE
        REFERENCES agent_message(external_id) ON DELETE CASCADE,
    target_set_json TEXT NOT NULL,
    snapshot_at_ms INTEGER,
    reference_boundary_json TEXT,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'running', 'completed', 'partial', 'failed', 'cancelled', 'interrupted'
    )),
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
);
CREATE INDEX idx_btw_batch_thread ON agent_btw_batch(thread_id, created_at_ms);
CREATE UNIQUE INDEX idx_btw_batch_one_running
    ON agent_btw_batch(thread_id) WHERE status = 'running';

CREATE TABLE agent_btw_recipient (
    external_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES agent_btw_batch(external_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('default', 'subagent')),
    target_key TEXT NOT NULL,
    agent_id TEXT,
    source_child_session_id TEXT,
    display_nickname TEXT NOT NULL,
    role_name TEXT,
    model_provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    variant TEXT,
    assistant_message_id TEXT UNIQUE
        REFERENCES agent_message(external_id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'running', 'completed', 'failed', 'cancelled', 'interrupted'
    )),
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE(batch_id, target_kind, target_key),
    UNIQUE(batch_id, ordinal),
    CHECK (
        (target_kind = 'default' AND target_key = 'default' AND agent_id IS NULL)
        OR (target_kind = 'subagent' AND agent_id IS NOT NULL AND target_key = agent_id)
    )
);

CREATE TRIGGER trg_btw_thread_delete_child
AFTER DELETE ON agent_btw_thread
BEGIN
    DELETE FROM agent_session
    WHERE external_id = OLD.child_session_id AND source = 'btw';
END;
```

服务层在同一事务验证：parent 是所属 workspace 的 native session；anchor 是它的正式 QA user；child 是同 workspace 的 btw session；提及实例属于该 parent 的任务树；所有绑定 message 均属于 child。跨表语义不能仅靠文本 ID 外键保证。

source agent 的 ID/昵称/角色作为历史来源快照保存，不用它的删除去级联删除已经产生的 BTW 答案。当前目标缺失时只能重新选择，不伪造复活。

### 8.3 消息、排序与幂等

- BTW `agent_message.kind='chat'`，`subagent_report_json=NULL`；不创建 `agent_subagent` 或父报告行。
- `submit_id` 作为 batch external ID，在 DB 唯一约束下确保重复 Effect/回执只插入一次。
- 接受问题时写入一条 user、批次、去重 recipient，并更新 answerer 集合；落库 ACK 且草稿 revision 未改变才清输入。
- 每个 recipient 保存预检通过的 provider/model/variant 快照；排队时主模型切换不改已提交问题的模型。密钥只存在运行时 `FrozenModel` 中，不能序列化进目标 JSON 或数据库快照。
- 排队 recipient 尚无 assistant，`assistant_message_id=NULL`；获得运行槽时才分配并写 streaming assistant，避免重启产生虚假“已开始”输出。
- 同一 batch 的 user/assistant 可共用 turn_index；展示顺序按 batch + recipient ordinal，不按 delta 到达或 assistant sequence 排序。
- 普通 `plan_turns` 只取一个 assistant，不能用于 BTW 的完整多目标回放。
- 每个 recipient 用 `finish_with_billing` 结算，避免重复 Finished 双计费用；更新 recipient/batch 的状态与时间。
- 用量继续按现有 `chat` 类别记录在 BTW child session，`StreamLog.kind='btw'` 用于诊断；不为了额外统计分类再重建 usage_log。原主 QA 的 token 合计不混入 BTW。

拟议 DB API 与事务边界：

| API | 同一事务内保证 |
| --- | --- |
| `accept_btw_batch` | 校验 owner/目标，唯一创建 thread/child，写 user/batch/recipient，更新目标集合；重复 submit_id 返回原记录 |
| `start_btw_recipient` | 以 `status='queued' AND assistant_message_id IS NULL` 条件认领，创建 assistant 并绑定模型/turn，返回唯一执行凭据 |
| `finish_btw_recipient` | assistant 终态/parts/账单、recipient 终态和 batch 聚合状态一并提交；相同结算幂等，不同终态冲突显式报错 |
| `load_btw_thread` | workspace/owner/source 校验后读取多目标投影与默认回答者，不经普通主会话 loader |
| `recover_btw_startup` | 收敛 queued/running 关系与残留 assistant 状态，保存内容并标记中断，不生成请求 |

结算复用 `finish_with_billing` 的内部写入逻辑；若其公共方法自行开事务，则先提取接受事务连接的内部 helper，不能在已有事务中再调用开事务的包装层。相同原则适用于创建消息和 session 的已有 CRUD。

### 8.4 删除与恢复

单删关系不会反向删除 child session，因此保留上述 AFTER DELETE trigger 覆盖 QA、主 session、workspace 级联删除。显式删除入口先收集 owner/run keys，发取消、标记归属失效，再在事务中删关系/主消息；FK + trigger 清理 child 及其消息/工具/附件元数据。

触发器 SQL需验证直接删 child、删 thread、删 anchor、删 parent、删 workspace、rollback 六种路径，无递归死循环或孤儿。实际文件归档清理沿用既有 GC 机制，不承诺 DB 外键删除文件本体。

`/rewind` 保持既有操作含义：按实际被删消息判断哪些 QA anchor 消失，不擅自把一次 steer 的撤销扩大成整 QA 删除。若只删 QA 的后续补充，anchor 仍在，BTW 保留；受已删除参考消息影响的在途回答取消并标记 `context_changed`，后续问答重建上下文。仅恢复文件不清理 BTW。

启动时 BTW 的 queued/running 批次与 recipient 统一标记 interrupted，保留已有正文；不恢复网络任务。普通 startup recovery 处理 assistant 后，再按真实状态收敛 BTW 关系记录。

取消已生效但删除事务失败时，保留历史和线程、显示删除失败并重新载入；不会自动恢复已取消的网络请求。不得先从 UI 永久移除内容再吞掉 DB 删除错误。

普通会话列表、fork 已按 native 筛选；直接 ID 的 `/resume`/CLI show、compact、rewind 等仍要显式检查 source。专用 BTW loader 才能打开 child session。

## 9. QA 与上下文算法

### 9.1 正式 anchor

```sql
SELECT external_id, sequence
FROM agent_message
WHERE session_id = ?1 AND role = 'user' AND kind = 'chat'
  AND subagent_report_json IS NULL
  AND COALESCE(stop_reason, '') <> 'steer'
ORDER BY sequence;
```

`resolve_qa(owner)` 验证 anchor 存活，得到从该 anchor 到下个正式 anchor 之前的 QA 区间。**提供给 BTW 的主历史是从会话开始到该 QA 末尾**，不是只取该 QA；SQLite 背景等先前信息必须保留。

steer、自动报告、本地 shell 不生成新的 anchor。已有报告属于主历史的参考数据，不应把它误排成新问题；用户已见的已完成 shell 输出可作为范围内参考，禁止因此获取执行工具。

### 9.2 获取一致快照

批次正式开始发送模型请求前，在同一 DB 读取事务内解析 QA 边界、已完成消息/工具、此前终态 BTW 批次，以及明确选定实例的自身任务记录。记录 snapshot 时间、边界消息 ID、来源 ID；排队期间不反复生成请求，开始运行时更新允许范围内的最新状态。

主历史包含：已完成正文、已封口的完整工具调用/结果、用户问题和补充；排除未封口 tool args、缺结果 call、仍 streaming 的半截正文。状态单独注明 `running/blocked/...`，不可冒充结论。思考元数据不作为其他模型的历史 reasoning 回放。

同批多个目标共享同一主 QA/BTW 前缀快照；每个目标只额外读取自己的任务记录。当前批其他目标的在途回复不进入其请求；下一批才看到前批的多目标答案。

### 9.3 来源与协议

父历史、其他实例记录和多回答者 BTW 历史作为带来源的只读参考内容；不把别人的 tool call 重新当成当前模型待执行调用。复用既有消息→内容、工具结果限额、预算裁剪能力，保留调用参数与完整结果的对应关系。当前 recipient 自己的新工具循环继续使用正常 `AssistantToolCall/ToolResult` 协议。

模型上下文包裹使用英文并转义 XML 特殊字符：

```text
<btw-context parent-session="..." qa-anchor="..." snapshot-at="...">
  <main-reference>...</main-reference>
  <agent-reference agent-id="..." role="monitor">...</agent-reference>
  <btw-history>...</btw-history>
</btw-context>
```

按完整问答/工具结果单元裁剪，不留下未配对 call。单工具结果复用现有 64 KiB 回放限制；当前问题和最近 BTW 回答优先，较早主历史其次。显示引用裁剪提示；预算无法容纳最小请求时失败并保留问题，不伪称有完整上下文。

### 9.4 compact 与读取当前文件

可以使用结束边界严格早于所属 QA anchor 的 active checkpoint 作为先前背景；检查其累计 base 链均在允许范围内。覆盖当前 QA 内部或后续 QA 的摘要不用，回到原始消息后做预算裁剪。不要只调用“最新 active checkpoint”而忽略旧 QA 边界。

显式读取磁盘得到当前文件内容，不是历史文件快照；上下文注明文件读取时点。跨 QA 实例引用遵循 D9，仅引入其自身 child 消息/报告，不拉入其父主历史或其他 BTW。

## 10. 只读权限、提及和身份

### 10.1 具体能力矩阵（审核默认值）

| 能力 | BTW 行为 |
| --- | --- |
| `read/search/curr_time` | 允许；workspace/cwd 冻结到来源会话 |
| `list_agents` | 允许只读专用处理，返回同一主会话树的身份/状态；不附带未选实例的完整报告，不触发任何恢复 |
| monitor 状态与日志 | 每批注入选定实例状态、最后完整工具结果和受控进程输出快照；不调用 `write_stdin` 获取日志 |
| `edit/write/exec_command/write_stdin/update_plan/ask` | 不声明且执行层拒绝 |
| 其他 agent 控制工具 | 不声明且执行层拒绝；用户选择目标是宿主调度，不是模型委派 |
| `@Files/refDir` | 保留文件/目录引用，按现有规则解析 |
| 图片/粘贴 | 保留现有输入、限额与模态校验；任一目标模型不支持图片时整批提交前提示并保留草稿 |
| `@Skills` | 作为限额内只读指令文本载入，不扩大工具面；不通过 skill 执行命令 |
| `@MCP` | 可选择现有已启用 server；只暴露冻结快照中 `read_only_hint=true` 的工具及只读 resource 桥。缺标记、false、未就绪时不放行 |
| `web_search` | 仅在来源配置已允许时加入，保持现有搜索配额；不自动打开浏览器或 shell |

本地服务执行白名单必须同时作用于声明、实际分派和 host 控制请求。MCP 只读标记是配置服务的能力声明，不是对远端代码的沙箱；未标记的工具不能凭名字或模型解释获得权限。只读业务边界允许保存聊天、图片归档和用量等应用自身持久化，不允许修改用户工程或控制原任务。

> **实现现状**：首版实际只读工具面为 `read/search/curr_time` 与 MCP 只读快照；`list_agents`、条件式 `web_search` 未接入，见 task.md §11.2。上表的这两项不是已完成事实。

### 10.2 关键过滤示意

```rust
// 拟议工具面；精确规格复用现有 schema。
const BTW_BUILTINS: &[&str] = &["read", "search", "curr_time", "list_agents"];

// 本轮快照必须按 disabled servers、角色能力、只读标记取交集。
let mcp_tools = snapshot.tools().iter().filter(|tool| tool.read_only_hint);
```

不要把 `RequestPurpose::Btw` 标成 `Subagent(Explorer)`：既有委派深度守卫会拒绝 list 操作，也容易进入子任务自动报告路径。新增显式 purpose/tool surface；禁止工具返回稳定英文错误 `Tool is unavailable in BTW read-only mode.`。

### 10.3 主任务唤醒与 BTW 问答是两个入口

BTW 提及解析顺序：候选采纳的稳定 ID → 手写精确 ID → 唯一昵称 → 仅有一个已有实例时的角色名。每步均限制在所属主会话树内；多个同名/同角色实例要求选择具体候选，零实例报错，不回退 spawn。沿用既有引用语法，候选的 reference target 写稳定 ID、显示层仍使用昵称/角色；旧的昵称引用继续支持唯一匹配。

提交前验证整组目标；任一目标无效时整批不提交，草稿与旧回答者集合都保留。已通过预检且运行后才遇到单目标错误，则按 recipient 分别结算，其余目标继续。

| 入口 | 行为 |
| --- | --- |
| 主输入 `@` 已结束实例 | 候选扩展到终态，插入稳定 ID 并注入复用指引；由主 agent 执行 **`resume_agent → send_input(trigger_turn=true)`** 恢复原任务。仅采纳/插入 `@` 不立即启动任务，不能把候选可见等同于已唤醒 |
| Agents 面板直接追问 | 非活动实例先 resume 再 send；统一允许 cancelled 的显式恢复，仍使用原角色权限 |
| 主 agent 自主恢复 | 复用已有 `resume_agent` / `send_input`，原并发/深度/归属约束保持 |
| BTW 追问 | 只建立独立 recipient，引用实例记录，不修改原任务状态/队列/报告 |

原任务正在运行时 BTW 可回答；原任务 completed 时其状态仍保持 completed，界面另显示 BTW answer 的运行状态。身份来自稳定实例 ID、昵称、角色与持久化模型；原 system prompt 未持久化的部分不能声称完整还原，按现有角色配置重建并强制应用 BTW 约束。agent 本体身份保持 `chat::prompt::AGENT_NAME` 的 `lx`。

## 11. 执行、路由与状态机

### 11.1 唯一数据流

```text
main /btw or BTW composer
  -> app: freeze owner + draft revision + targets; produce SubmitBtw effect
  -> event: resolve references/models/owner; prepare attachments
  -> db transaction: get-or-create thread + user + batch + recipients
  -> ACK: clear only matching draft; show queued/running batch
  -> scheduler: same-thread serial, global recipient limit = 2
  -> snapshot + RequestPurpose::Btw + frozen tool whitelist
  -> run_agent_turn with BTW-aware host event routing
  -> DB pending/running/result handshake per assistant
  -> recipient finish_with_billing + aggregate batch state
  -> update matching visible BTW projection; otherwise persist for later load
```

### 11.2 Effect 与回投（拟议）

```rust
enum BtwEffect {
    LoadQaList { pane: PaneId, parent_session_id: String },
    LoadThread { pane: PaneId, owner: BtwOwner },
    Submit(BtwSubmission),
    Stop { owner: BtwOwner },
}

struct BtwSubmission {
    submit_id: String,
    composer: ComposerOwner,
    owner: BtwOwner,
    workspace_key: PathBuf,
    reference_root: PathBuf,
    draft_revision: u64,
    text: String,
    attachments: Vec<BtwDraftAttachment>,
    skill_blocks: Vec<BtwSkillReference>,
    targets: Vec<BtwTarget>,
}
```

可封装为 `Effect::Btw(...)` 减少顶层枚举膨胀。回投同样携带 owner、thread、batch、recipient 和 revision，复用 `ChatDelta`，不另造流协议。

提交时冻结数据库 workspace key 与实际执行/引用 root，排队及运行时随 submission/run 保存；切 pane、切工作区后不能重新从活动 workspace 推导路径。批次快照元数据记录本次 root，恢复后重发重新预检来源路径，不以当前进程 cwd 静默替代缺失目录。

附件快照包含现有图片/粘贴对象的稳定路径或共享字节及标记位置，skill 引用包含稳定路径和已加载正文（若尚未加载则在事件层准备阶段读取并冻结）。事件层只消费提交快照，不回读当前 composer；图片准备和 skill 加载完成后，冻结的 `PreparedUserMessage`/正文块随批次持有，入库附件经既有归档流程保存。草稿 revision 仅决定 ACK 后能否清空，不作为重读附件的理由。

### 11.3 运行时与 host

- `ChatStreams` 增加 `btw_runs` 索引，独立于 `streams.subagents`。每个 run 保存 owner、assistant_id、自己的 `FrozenModel`、取消句柄和时间线状态。
- BTW 不使用按 session ID 索引的 `streams.frozen`；同一 thread 的不同 recipient 可以使用不同模型。BTW finish 只移除精确 run key 的模型和执行状态，不调用会按 session 清理 frozen 的主 finish 路径。
- `handle_net_event`、host BeginTool/FinishTool、最终结算先按 BTW 路由分类。未知/已结束 BTW key 直接拒收，不能回落主路径。
- 推荐以显式 BTW net event/host route 标记来源，避免仅依靠“session 曾出现在 HashSet”而无限保留 tombstone。
- `reap_stale_streams` 必须无条件先按 `btw_runs` 识别活动 BTW，再判断主 pane 的 `active_assistant_id`；无论 executor 是否创建 ProcessRuntime，都不能仅因没有主 pane streaming 状态而取消 BTW。
- BTW host 的 `SteerInputs/UserShellUpdates` 返回空，`HasPendingSteers` 返回 false；不消费主队列。ask/mutation/控制型 Subagent 请求在 host 层再次拒绝。
- `SubagentOp::List` 在 BTW 分支做纯查询，归属从 run.owner 解析；不能伪造主 RequestKey 并调用主控制分派。
- 可见工具 pending 仍遵守先绘制再 running ACK；不可见 BTW 按已有子轮规则在落库后放行，不等待不存在的窗格帧。
- 两个 recipient 可能拿到相同 provider call_id，而现有 DB 要求 session 内唯一。BTW 存储层统一映射为 `btw:<recipient-id>:<provider-call-id>`；同一 helper 同时处理 UI 的 ToolInput 事件、host BeginTool 与持久化 ticket，避免标题和结果对不上。
- provider 原始调用 ID 保留在 loop 的 `ReadyToolCall` 和当前模型上下文中；`begin_tool` 只在 host 路由边界克隆并变换入库调用，返回包含 storage ID 的 ticket，FinishTool 按 ticket 结算。现有 loop 的 `context.push_tool_result(&call.call_id, ...)` 使用原始 call ID，因此模型的 call/result 仍配对。不要把命名空间 ID 直接回传给供应商，或为了 BTW 修改所有主会话的唯一约束。
- 不注册可执行 shell manager 作为 BTW 工具入口；若复用 executor 所需的轻量 runtime，则 sweep 将活动 BTW session 纳入 bound 集，结算只清理对应 recipient。

执行器明确复用 `AgentTools` 以支持 MCP/web_search，不仅使用 FileTools。其所需 ProcessManager 是内部依赖，不向 BTW 声明进程工具；执行入口再次限制为冻结白名单。runtime 的 turn key 包含 recipient/generation，活动 BTW session 纳入 `reap_orphan_runtimes` 的 bound 集；最后一个 recipient 结算后才回收该 thread 的运行资源，不会提前影响其他模型请求。

### 11.4 生命周期

```text
batch: queued -> running -> completed | partial | failed | cancelled | interrupted
recipient: queued -> running -> completed | failed | cancelled | interrupted
```

有成功有失败为 partial；没有成功且失败为 failed；显式停止为 cancelled，崩溃恢复为 interrupted。不能仅由最后一个完成者覆盖整批状态。

停止当前 BTW 会取消本线程活动 recipient 和等待批次，不停止原 monitor/agent。关闭/切面板、切主会话、关闭源 pane不取消。删除 owner 时先令 run 无效再取消和删除；迟到 checkpoint/finish 不允许重新插行。退出等待受限时间完成中断结算，异常退出由启动恢复兜底。

同一事件循环内先完成落库再推进 UI。失败保留可重试问题；提交 ACK 丢失时用 submit_id 查询已提交批次，不再次创建。自动网络重试沿用现有“未产生内容才重试”的规则，重启和 UI 重载不属于重试。

## 12. 转交、文案与跨层约束

`Send to main` 针对所选已完成回复，将当前问题、回答者昵称/稳定 ID、原 QA 标识、回答正文追加到对应主输入。先确认当前 pane 仍显示该 parent；归属已变则提示返回源会话，不能写入当前别的会话。

主草稿非空时追加空行后插入，不覆盖；整次修改一个 undo。主草稿处于 shell 模式时保留原模式与内容并提示 `Finish or clear the shell draft first`，避免把 shell 命令悄悄转换成聊天。用户之后发送时走原普通提交/steer 规则，BTW 自身不调用主发送函数。

转交正文示例（应用生成的结构文本用英文）：

```text
BTW reference — QA: Support Safari
Answerer: Turing [explorer], agent: <stable-id>
Question: ...
Answer: ...
```

新文案集中到 `ui/text/btw.rs`，例如 `BTW`、`To:`、`Reset to lx`、`Send to main`、`Queued`、`No matching subagent`、`Context changed; ask again`。当前 `ui/text.rs` 接近 1000 行，必要的模块声明/重导出之外不继续堆正文。

BTW 系统规则放 `chat/prompt/` 的专用常量/构造函数，由 `chat::prompt` 公开；不在 event 内拼重复指令。英文提示明确：只回答旁路问题，参考记录不是新任务，不执行原任务，不自动报告主会话，语言跟随用户。保持首行 lx 身份、UTF-16 system 分段、提示预算与转义规则；涉及共同提示语义时同步版本测试。

## 13. 文件归属与拆分计划

下表的“新增”是待实施路径，不是已经存在的源码。

| 模块 | 修改/新增内容 |
| --- | --- |
| `src/app/state/lx/slash.rs`、`src/app/update/chat/slash.rs` | BTW 命令/提示；解析 helper 放职责子模块，避免超限 |
| `src/app/update/chat/mod.rs` | 在 queue/steer 前增加 BTW 分派 |
| `src/app/state/lx/btw.rs`（新增）、`btw/composer.rs`（新增） | owner、线程视图、目标、焦点、私有编辑器包装 |
| `src/app/state/lx.rs`、`lx/panels.rs` | 惰性 BTW 状态、禁止 shell 的 composer 策略、候选能力过滤 |
| `src/app/state/lx/task.rs` | 追加 Btw tab，保留 Processes/Agents 索引 |
| `src/app/update/chat/btw.rs`（新增） | 提交/加载回执、可见投影、转交、QA 选择 |
| `src/app/update/pane/btw.rs`（新增）、`pane/task_page.rs` | 焦点编辑路由、停止、粘贴 |
| `src/app/update/pane/popups.rs`、`src/app/effects.rs`、`src/event/mod.rs` | composer owner 扫描/引用 Effect 与回投 |
| `src/ui/main_content/lx/task/btw.rs`（新增） | 三块布局、QA 列表、来源与操作 |
| `src/ui/main_content/lx.rs`、`lx/panels.rs`、`lx/task.rs` | 主布局接入和共享 composer view；接近上限时把布局整体按职责下沉 |
| `src/ui/main_content/lx/history/source.rs`、`history/snapshot.rs`、`history/cache.rs` | 类型化 speaker 和稳定多目标展示 |
| `src/app/state/lx/chat/stream.rs` | 提取消息级流变换供显式 assistant ID 更新，多路回答不共用唯一 RequestState |
| `src/ui/main_content/lx/history_hit.rs`、`lx/selection.rs` | BTW 对话的卡片、复制、滚动命中 |
| `src/event/mouse/press/lx.rs`、`mouse/wheel.rs`、`mouse/drag.rs` | BTW 三区及浮层路由 |
| `src/chat/btw.rs`（新增）与 `src/chat/context/` | 有界参考装配、预算、来源边界 |
| `src/chat/events.rs`、`src/chat/agent/mod.rs`、`src/chat/tools/schema.rs` | Btw purpose/工具面及能力交集 |
| `src/event/chat/btw/`（新增） | submit/context/runtime/host/settle/delete 按职责拆分，实际需要时建文件 |
| `src/event/chat/mod.rs`、`tool_calls.rs`、`process_page.rs` | BTW 路由、ACK、sweep；mod.rs 已接近上限，新增主体必须下沉 |
| `src/db/chat/btw/`（新增）、`src/db/service/` | QA resolver、三表CRUD、批次幂等、专用加载 |
| `src/db/migration/`、`src/db/mod.rs` | source 扩展、索引/触发器、迁移版本与恢复 |
| `src/db/chat/rewind.rs`、session 删除与 CLI 直接ID入口 | anchor 删除联动、source 守卫，不扩大原撤销语义 |
| `src/app/state/lx/mention.rs`、`src/event/chat/subagent/snapshot.rs`、`src/app/update/pane/task_page.rs` | 终态实例候选、稳定引用、cancelled 直接恢复一致性 |
| `src/ui/text/btw.rs`（新增）、相关规范文档 | 英文文案、目录与后台页行为更新 |

生产文件上限 1000 行；新增职责文件目标约 500 行。测试跟随生产模块下沉，禁止往现有超大父 tests.rs 追加大量场景。实际 `src/db/chat/btw.rs` 已拆出 `btw/recovery.rs`；`src/event/chat/btw.rs` 仍超限，见 review.md P2-10。

## 14. 验证与实施门禁

完整测试矩阵、分阶段任务与人工步骤见 task.md。重点必须通过：

1. 不同主 QA/会话/目标/代次的请求与异步扫描不串写。
2. readonly schema、loop 和 host 均拒绝写入/执行/控制操作；原任务继续运行。
3. 多目标各自流式/失败/计费，不漏答、不互相覆盖，不产生父报告。
4. 关闭面板、切会话、无源 pane 后仍可完成；取消/删除后晚到事件不复活记录。
5. source 隔离、真实 child 清理、数据库迁移回滚、旧 QA compact 边界正确。
6. input、焦点、selection、候选、speaker 的渲染行数和命中同源。

> 上述门禁是实施前方案；`feat/lx-btw` 已按后续用户批准落地并完成 v28 迁移。真实开发库升级仍需按项目规范备份、关闭 lx 进程；不要把 worktree 隔离库复制回主库。

## 15. 第五轮交互落地（2026-11-12）

- **视图模型**：BTW 页签只显示当前选中的单个 QA 线程；`/btwList` 是唯一的 QA 切换入口。取消“全部 BTW / QA 分割线 / 全量线程加载”方案。
- **loading**：线程历史加载与生成 loading 统一落在 tab 行与 composer 之间的底部空行；对话区不再画居中 loading。`/btwList` 面板自己的列表 loading 留在二级面板内部。
- **回到底部**：BTW 复用主会话 chat bar 的 `↓ jump to bottom`，由活动线程的 `follow`/滚动偏移判定显示；点击恢复贴尾。
- **交互**：BTW 对话与 Agents 详情对齐——滚轮 3 行/格、滚动条 thumb 拖拽/轨道点击、`↑↓` 1 行、`PgUp/PgDn` 10 行、正文选区与复制、代码块复制按钮、卡片折叠。
- **实现归属**：滚动状态仍在 `BtwThreadState`；几何/命中在 `task_btw_scrollbar`、`scroll_btw_detail`、`set_task_btw_scroll` 与 `render_chunk` 的 Btw 分支；不引入跨 QA 的合成历史源。

## 16. 第六轮 E2E 收口（2026-11-12）

- `/btw prompt` 首次打开时线程尚未建立：正文以 `Option<String>` 暂存，QA 列表回投后写入新线程 composer 再提交，绝不提交空草稿。
- 无新 `@` 的追问沿用当前线程上一次回答者集合；从 `BtwTargetView` 回查实例生成提交 choice，实例缺失时拒绝并保留草稿，不静默退回默认助手。
- 验证由 `tests/btw_verify.rs`（ignored）覆盖：CLI 建工作区/会话、`/resume`、BTW 首问、连续追问、`@` 已完成 subagent、只读拒写、回到底部、`/btwList`；报告见 `verification.md`。

## 17. 工具 pending 时间线统一（2026-11-12）

- 主轮、子 agent 与 BTW 的工具 pending 前统一调用 `checkpoint_before_tool`：先提交工具之前的 parts，再让 DB 分配 position；避免长工具循环中 app 时间线与 DB 位置漂移。
- BTW 的 checkpoint/pending 失败现在记录完整 `error`（包含 SQLite 子错误），不再只保留 `sqlite` code。
- 用户报告 `sess-18e73a11...` 的 QA 3 BTW 失败已按此修复；旧库无法反推具体 SQLite 子错误，重放副本未在 120s 内复现。


## 18. 评审修复（2026-11-12）

- **P1 已修**：切主会话/关 pane 的输出缓冲、跨实例启动恢复 guard、结算单事务、认领失败收敛、`/btw prompt` 最新 QA、BTW composer 输入路由、`default` 目标哨兵、迟到 ThreadLoaded 校验。实现与复测见 [review.md](review.md)。
- **P2 未修**：单 `pending_submit` 生命周期、部分目标缺失静默缩小、多 pane 回投只更新首个、picker 指针优先级、恢复 `partial` 聚合、MCP disabled 冻结、已完成 subagent 报告注入、工具回放预算/参数、事件层单文件超限。
- **文档**：§6.1/6.2 标为历史方案；§10.1 标注 `list_agents`/`web_search` 未接入；§14 的实施门禁已从“未实施”改为历史说明。
