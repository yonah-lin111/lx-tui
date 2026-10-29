# Codex 内置 Agent Tool 补齐设计

状态：已完成（2026-10-29 实施并通过评审与端到端验证，见同目录 `task.md`、`verification.md`、`review.md`）。参考仓库固定为 `/Users/yonah/projects/agent/codex-main`，目标仓库为本项目当前 checkout。本文记录已确认的需求边界及实现契约。

## 1. 目标和边界

目标是按**能力**比较 Codex 内置工具与 lx 当前工具，补齐 lx 现有架构能承载的能力。不是复制 Codex 的工具注册器、权限体系或 MCP/插件运行时。新增工具均走 lx 现有 `ToolSpec -> transport -> agent_loop -> AgentHost -> event -> db/app` 链路，模型可见文字保持英文；本文中的说明和注释用中文。

本次新增主 agent 工具：`send_input`、`resume_agent`、`list_agents`、`cancel_agent_input`、`curr_time`。其中 `cancel_agent_input` 是为可靠队列新增的 lx 工具，Codex 无同名工具。`spawn_agent` 仍是现有名字，但新增结构化 `items` 首次输入。`wait_agent`、`close_agent` 保留现名，行为需与多轮生命周期兼容。主工具总数由 11 到 16；`explorer` / `executor` 的工具白名单保持原权限，不允许子 agent 自行控制别的子 agent。

| Codex 内置能力 | lx 当前情况 | 本次决定 |
|---|---|---|
| shell / stdin | `exec_command` / `write_stdin` | 已有，不加别名 |
| apply patch | `edit` 已处理 Codex 补丁 | 已有，不加 `apply_patch` |
| view image | `read` 已可读图片，受模型图像能力门控 | 已有，不加 `view_image` |
| plan | `update_plan` | 已有 |
| request user input | `ask` | 已有；异步提问另议 |
| spawn / wait / close | 已有，当前仅一次子轮 | 扩展多轮语义 |
| `send_input` / `resume_agent` | 无 | 新增，含跨重启恢复 |
| `list_agents` | UI/DB 可列，模型不可列 | 新增 |
| `curr_time` | 环境快照只有发送时日期 | 新增实时 UTC 时间 |
| `sleep` | 无 | 本次不加 |
| context remaining / new context window | 自动压缩、`/compact` 不等价于精确额度或新窗口 | 本次不加 |
| async user messaging | 聊天流与阻塞 `ask` | 本次不加 |
| MCP、插件、网络托管工具、审批 | 无对应宿主体系 | 不在本次范围 |

### 1.1 参考事实与检索说明

- Codex 工具总入口在 `codex-rs/core/src/tools/spec_plan.rs::add_core_tool_sources`，分 shell、MCP、utility、collaboration；V1 协作工具包括 `send_input` / `resume_agent`，V2 另有 `list_agents`。实现与 schema 分别在 `handlers/multi_agents/{send_input,resume_agent}.rs` 和 `handlers/multi_agents_spec.rs`。
- Codex `send_input` 接受 `message` 或 `items`、`interrupt`；`resume_agent` 按 ID 恢复关闭的 agent。`parse_collab_input` 明确两种输入互斥。
- Codex 的 app-server `image_url.rs` 和 core `image_preparation.rs::resize_image` 均拒绝远程 `http(s)` 图片；`local_image` 与内联 `data:` 图片是可用路径。Codex rollout 测试证明结构化图片输入被记录并参与重放。
- lx 主工具声明在 `src/chat/tools/schema.rs::TOOLS`，权限表在 `src/chat/agent.rs::MAIN_TOOLS`；现有 `ToolSurface` 按主 agent / explorer / executor 分流。子 agent 的持久化在 `src/db/chat/subagent.rs`，运行时在 `src/event/chat/subagent/mod.rs`。
- codegraph 已用于 lx 的入口/调用影响分析；codebase-memory 已用于两仓图查询和覆盖检查。Codex `codex-rs/core/src/tools/` 被 codebase-memory 索引规则排除，故该目录使用限定范围直接源码读取，不把索引缺失当成“没有工具”。

## 2. 用户已确认的行为

1. 按实际能力去重，不为 `edit` / `read` 新增 `apply_patch` / `view_image` 别名。
2. `send_input` 和 `resume_agent` 都要有；同一 `agent_id` 可跨多个子轮继续工作，退出 lx 后也可显式恢复。
3. 运行中 `send_input(interrupt=false)` 持久排队；`interrupt=true` 取消当前轮、保存已有输出，再优先执行新输入。旧队列保留，并向主 agent 报告剩余数。
4. 工具返回成功意味着排队输入已持久化。重启不自动跑子 agent；`resume_agent` 后按队列顺序继续。
5. 每个正常完成或失败的子轮各有独立报告与投递状态；被中断/取消的子轮保留历史但不生成完成报告。
6. `list_agents` 提供可恢复 `agent_id`。终态 agent 必须先 `resume_agent` 再 `send_input`。
7. `send_input.items` 和 `spawn_agent.items` 支持文本、本地图片路径、内联图片 `data:` URL；拒绝远程图片。首次任务旧 `task` 参数仍可用。
8. 图片输入接收前校验并归档；归档丢失或损坏则暂停该队列项并报告，不把缺图任务降级为纯文本。
9. 恢复后旧轮的文本、图片及持久化压缩摘要参与上下文装配；预算裁剪完整轮次，图片原件在子会话存续期间保留。
10. `cancel_agent_input(submission_id)` 只取消尚未开始的排队输入，重复取消幂等；不回滚已开始的工作。
11. 新增实时 `curr_time`；本次不做 `sleep`、上下文窗口工具、异步用户交互工具。
12. `data:` URL 图片受现有 1 MiB 工具参数总上限约束；较大图片使用 `local_image.path`，不提高全局工具参数上限。

## 3. 现状和关键缺口

`src/chat/agent.rs::SubagentOp` 当前只有 `Spawn/Wait/Close`，`parse_control` 只识别三个名字。`src/chat/tools/subagent.rs` 声明相同三个工具。`src/event/chat/subagent/mod.rs::spawn` 使用 `begin_subagent_turn` 创建子会话的一对 user/assistant 消息，并直接用 `ModelItem::UserText(task)` 起子轮；`finish_subagent` 从运行 map 移除句柄。`Database::finish_subagent_turn` 在 `agent_subagent` 行上写唯一 `report_json/report_delivered`，因此多轮报告无法逐轮保存。

现有 `Database::begin_turn_with_attachments` 已能在已有会话追加 user/assistant 消息并建立文件 mutation scope，`agent_message_attachment` 已支持 image metadata，`platform::archive_session_image` 已按会话归档处理后图片。但 `attachment::insert_attachments` 在归档失败时会退回原路径，不满足“成功接收即跨重启可重放”；子 agent 路径必须采用更严格的提交前校验。`recover_running_subagents` 当前把所有运行中 agent 标成 cancelled，并封口 streaming assistant；这可作为崩溃恢复底座，但队列状态需要额外协调。

`src/chat/agent_loop/auto_compact.rs::apply_auto_compact` 仅在内存保留摘要并清理较早 items。子 agent 多轮恢复若只读 DB 原始消息，会丢掉上一轮自动压缩上下文的精确信息；需要持久化摘要检查点或重新从完整历史按同一预算装配，不能把上一轮内存摘要当作永久事实。

## 4. 对外工具契约

以下 JSON 是目标形状，实际 `ToolSpec` 必须使用严格 schema、`additionalProperties=false`，执行层再次白名单校验；所有描述、字段错误、模型结果文本为英文。`items` 的顺序必须保留，文本和图片可以交错，不能把全部文本先拼接再把图片放末尾。

```json
{"role":"explorer","title":"Inspect render","task":"Find the bug"}
{"role":"explorer","title":"Inspect render","items":[{"type":"text","text":"Find the bug in this screenshot"},{"type":"local_image","path":"screenshots/bug.png"}]}
{"target":"sub_123","items":[{"type":"text","text":"Continue with this image"},{"type":"image","image_url":"data:image/png;base64,..."}],"interrupt":false}
{"id":"sub_123"}
{"submission_id":"input_123"}
{}
```

| 工具 | 输入 | 成功结果的稳定字段 | 失败边界 |
|---|---|---|---|
| `spawn_agent` | `role,title` + `task` 或 `items`，二者互斥；`items` 非空 | `agent_id,role,title` | 角色/模型/图片/并发/持久化失败均不启动子轮 |
| `send_input` | `target` + `message` 或 `items`，互斥；可选 `interrupt=false` | `submission_id,agent_id,status,queued_count` | 目标必须同父会话、已恢复；接收前处理图片；重复提交不会因工具内部重试而重复入队 |
| `resume_agent` | `id` | `agent_id,status,queued_count` | 只恢复当前父会话拥有的 ID；已活动时幂等；不自动重复已完成输入 |
| `list_agents` | 空对象 | 列表 `agent_id,role,title,status,queued_count` | 仅列当前父会话，不泄露完整正文或别的工作区 |
| `cancel_agent_input` | `submission_id` | `submission_id,status` | 只把 queued 变 cancelled；running/done 返回现状，不回滚 |
| `curr_time` | 空对象 | `current_time`：`YYYY-MM-DD HH:MM:SS UTC` | 取时失败返回工具错误，不伪造值 |

`items` 本次只允许 `{type:"text",text}`、`{type:"local_image",path}`、`{type:"image",image_url}`。其他 Codex item 类型（音频、skill、mention）明确报 unsupported；remote `http(s)` 报 remote image unsupported。`local_image.path` 相对父轮冻结的工作区 cwd 解析；绝对路径可用，仍走现有图片格式/大小校验。调用时模型不支持图片输入则拒绝，不能只把图片说明写进文本。

本次新增工具规格只给主 agent，包含 `curr_time`。`explorer` / `executor` 的白名单不因时间工具扩大；将来若子角色确需实时时间，再作为独立授权变更。

## 5. 子 agent 状态机

### 5.1 agent 与输入分层

`agent_subagent` 保持稳定身份和子会话链接；**轮次**由子会话 `agent_message` 的 user/assistant 配对表示；**输入队列**单独持久化。不可继续把 `agent_subagent.status` 与一条 assistant 的状态等同。运行时持有显式恢复标记：已恢复且无执行中轮次为 `idle`，正在执行为 `running`；无标记时 DB 保留历史兼容值 `completed/failed/cancelled`，直接 `send_input` 必须报错。`blocked` 由队列头图片错误投影；`closed` 由无标记且曾显式关闭投影。不能直接扩大旧 `CHECK` 值而不做迁移；优先沿用已有值并以最新输入/轮次判断可发送性，必要时用新状态列而非重建旧表。

输入状态：`queued -> running -> completed|failed|interrupted`，以及 `queued -> cancelled`。归档图片丢失时状态仍是 `queued`，另存 `error_code=image_missing` 并在工具/UI 投影成 `blocked`；主 agent 可用 `cancel_agent_input` 取消该项，再发送正确图片。不覆盖原提交记录。`submission_id` 稳定；队列单会话按单调 `position` 排序。`interrupt=true` 新项设置优先级并在当前轮结算后先取它；未处理旧项保持其相对顺序。连续多个 interrupt 依提交序执行，不能只保留最后一条。

**事务边界**：接收图片 -> 暂存归档并完成校验 -> DB 原子插入队列记录、item 元数据、图片引用 -> 返回成功。文件写与 SQLite 不能组成一个原子事务，DB 失败时可留下未引用的归档文件，交由引用感知清理；不允许 DB 成功引用一个尚未写完整的图片。若任一步失败，工具返回失败且队列无新项。需要用 `call_id + parent turn/user message id` 唯一约束阻止内部重放造成双提交；`submission_id` 回执可重取。

### 5.2 发送、插队、恢复、关闭

1. `spawn_agent` 预校验 role/输入/图像模态/并发，DB 创建子会话与第一条输入，再启动后台轮；`agent_id` 在同一事务产生。初始图片必须归档到**子会话 ID**命名空间。
2. 运行中 `send_input(interrupt=false)` 只持久入队，不修改当前模型请求。当前轮完成后，事件循环按队列取下一条并追加子会话轮次。工具返回 `queued_count`。
3. `interrupt=true` 先持久化新输入，再发送当前轮 cancel。已产生 part/tool/mutation 按现有停止路径结算为 interrupted；只在旧轮封口后启动新轮。取消失败/事件重排时新输入仍在队列中，不得丢失或与旧轮并发执行。旧队列在新输入之后处理。
4. 子轮完成但队列为空时释放运行时恢复标记，agent 进入可恢复终态；需显式 `resume_agent` 才可接收下一条 `send_input`。轮完成时已有队列则保持恢复标记并持续消费，直到队列空。子轮失败时**清除恢复标记并暂停整个 agent 的后续队列**，保留所有未处理输入，必须显式 `resume_agent` 才能继续。`close_agent` 关闭活动运行或 idle 身份，不删除子会话、队列和历史；恢复后可以继续。
5. `resume_agent` 查询 DB 校验 `parent_session_id` 和 workspace，复用原 `child_session_id`、角色、标题与历史，修复上次崩溃留下的 streaming 状态；只由显式工具触发恢复。若有 queued 项则启动首项；若无 queued 项保持 idle 并等待 `send_input`。同一个 agent 不得因两次 resume 启动并发轮。
6. 重启时 `recover_running_subagents` 只把正在执行的轮次封为 interrupted，不能把 queued 项删掉，也不能自动执行队列。若此前已接收但未完成的 running 输入，恢复时应按明确规则回到 queued 重试或保留 interrupted 等主 agent 决定；本文拟采用 **保留 interrupted，不自动重试**，避免重做已产生副作用的任务。`list_agents` 显示该状态。
7. 父会话删除时子会话/输入/报告由外键级联删除，图片归档按引用清理。父窗格关闭/切会话时运行中任务遵循现有生命周期，但成功排队的输入仍可经同一父会话 ID 恢复。

上述第 6 项是实现细节审核点：与“成功排队不可丢”兼容，因为未开始的 queued 项保留，已开始的 running 项已不是队列项；主 agent 可根据 interrupted 状态决定是否重新发送。

### 5.3 报告与等待

每个完成/失败子轮生成独立 `report_id`、`agent_id`、`child_user_message_id`、`status`、`report_json`、`delivered_at_ms`。取消轮无报告。`wait_agent` 的完成通知针对目标当前轮的状态，不应因为前一轮完成就立即返回；若目标已在 idle 且无队列，则可返回最近终态和未领取报告。报告按完成时间和稳定 ID 排序，`begin_report_batch` 在**同一事务**插入父会话报告消息并将所选 `report_id` 标记 delivered，避免崩溃双投或漏投。旧 `agent_subagent.report_json/report_delivered` 的已有数据需迁移为一条 legacy 报告，且不能重复投递。

### 5.4 历史与压缩

启动每个新子轮前，从 DB 加载子会话历史，按 user/assistant/tool 的稳定顺序组装 `ChatRequest.items`；当前 input 的文本与图片保留原始 item 顺序。使用 `chat::context` 现有完整轮次预算裁剪规则，不拆开单轮的文本/图片关系；模型图像能力不支持时在受理阶段拒绝，历史回放遇到旧图像也应明确失败或依现有模型选择能力规则处理，不能静默删除。

自动压缩摘要需要持久化检查点（建议绑定子会话与 `end_sequence`），恢复时按 `system prompt -> environment -> compact summary -> checkpoint 后历史` 装配；摘要写入成功前不得丢掉旧 items。已有 `/compact` 检查点表若能复用，优先复用，避免第二套摘要表。压缩失败保持未压缩历史，按当前预算收缩且显式记录，不伪造摘要。图片归档生命周期不以“当前上下文是否引用”为准，直到子会话删除或明确清理。

## 6. 图片持久化与重放

### 6.1 输入校验

复用 `platform::load_and_process_image_file`、现有图片格式、像素和总量限制（目前单张原始 20 MiB、单次最多 4 张、总原始 40 MiB；最终数值以本仓常量为准）。`data:` URL 的**整条工具参数 JSON**受 `src/chat/events.rs::MAX_TOOL_ARGUMENTS_BYTES = 1 MiB` 约束，不能把全局上限提高到图片大小；本地大图片必须走 `local_image.path`。`data:` URL 必须在有界长度检查后解码，再按与本地文件相同的格式/像素/尺寸规则处理。只允许 `image/*` 中现有解码器支持的真实格式，不能仅信 MIME。去重可按处理后字节摘要，但 item 顺序和重复引用语义必须保留。

### 6.2 归档契约

当前 `archive_session_image` 是按 `session_id` 写处理后图片，输出路径可作为附件引用；子 agent 路径不得沿用 `insert_attachments` 的“归档失败回退原路径”。应增加显式“已归档附件”入口或参数，要求写完、可读、元数据与摘要一致后再 DB 提交。路径和摘要进 DB；路径必须在会话图片根目录内且经过规范化验证，不能信任模型传入的 `source_path`。写入同目录临时文件后原子替换，限制目录权限。重启重放时重新验证存在性、可读性、实际 MIME/尺寸和摘要；失败把对应输入标为 blocked，向主 agent 返回结构化错误，保留原记录。不能从原始路径重新读取，因为原件可能已变化。

### 6.3 清理与引用

现有图片清理从 `agent_message_attachment` 收集引用；新增 queued 输入中的图片必须纳入同一引用集，否则 30 天清理可能误删尚未处理的消息图片。子会话删除才允许删其归档目录；会话仍存在但图片因上下文裁剪未出现在本轮请求时仍保留。旧用户图片路径继续按既有规则处理，本次只为子 agent 输入建立严格保障。`data:` URL 的 base64 正文不得写入日志、UI 卡片或模型文本结果，DB 只保存归档路径、摘要、可读标签和顺序索引。

## 7. 代码归属与拟改关键点

| 模块 | 拟改职责 |
|---|---|
| `src/chat/agent.rs` | 扩 `SubagentOp/Outcome`、解析新工具、稳定状态与英文错误；主/子工具白名单 |
| `src/chat/tools/schema.rs`、`subagent.rs`、`types.rs` | 五个新工具和扩展 `spawn_agent` 的英文描述/严格 schema；工具名清单与角色测试 |
| `src/chat/agent_loop/mod.rs` | 控制工具通过 `AgentHost::subagent` 请求事件层；图片结果进入有序 `ModelItem::UserContent` |
| `src/event/chat/subagent/` | 从大 `spawn` 抽出可复用的“启动已有子轮”函数；队列调度、interrupt、resume、取消、报告结算；事件循环单写 `AppState` |
| `src/db/chat/subagent.rs` | 拆 agent 身份/输入队列/逐轮报告 API；事务幂等、父会话归属、重启恢复 |
| `src/db/migration.rs`、`src/db/mod.rs` | 迁移版本顺延，新增输入/报告表和必要索引；保留旧库升级路径 |
| `src/db/chat/attachment.rs`、`src/platform/clipboard_image.rs` | 严格归档入口、数据 URL 接入、摘要验证、引用感知清理 |
| `src/chat/context/`、`src/chat/agent_loop/auto_compact.rs` | 子会话多轮回放、图片有序输入、压缩检查点持久化 |
| `src/app/state/lx/chat/`、`src/app/update/chat/subagent.rs`、`src/ui/main_content/lx/task/agents.rs` | 从 DB 投影 queued/blocked/idle/多报告；渲染只读，不做 IO；用户可见文案统一 `ui/text.rs` |
| `src/chat/prompt.rs` | 只在工具能力确实可用后更新英文工具说明；若系统提示结构变化，同步 `PROMPT_VERSION` 和预算测试 |

关键分层伪代码，具体类型名以实现时现有 API 为准：

```rust
// event/chat/subagent：唯一事件循环内的控制入口。
match op {
    SubagentOp::SendInput { target, input, interrupt } => {
        let submitted = db.enqueue_input(parent, target, input, call_id)?;
        if interrupt { cancel_current_turn(target); }
        schedule_if_idle_and_resumed(target);
        Ok(SubagentOutcome::InputAccepted(submitted))
    }
    SubagentOp::Resume { id } => {
        let record = db.owned_subagent(parent, id)?;
        resume_runtime_from_history(record)?;
        schedule_next_queued(id);
        Ok(SubagentOutcome::Resumed { /* ... */ })
    }
    // List/Cancel/Time 分别走只读或受控事件层入口。
    _ => { /* ... */ }
}
```

`curr_time` 只取实时 UTC 时钟，不能复用发送时冻结的环境快照日期。`src/platform/environment.rs::utc_datetime` 已用 `SystemTime` 和 `civil_from_days` 生成日期，但只格式化到分钟；实施时复用其日期算法并补秒级输出，避免新依赖。工具结果仍按现有 DB/时间线通道落库。

## 8. 迁移与失败恢复

当前 `SCHEMA_VERSION = 18`，设计以 **v19** 为占位；实施前必须再读 `src/db/mod.rs` 和未合并分支迁移，如 v19 已被占用则顺延。迁移只新增表/索引和可空列，不重建 `agent_subagent`，避免旧数据丢失。建议：

```sql
CREATE TABLE agent_subagent_input (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    agent_id TEXT NOT NULL REFERENCES agent_subagent(external_id) ON DELETE CASCADE,
    dedupe_key TEXT NOT NULL UNIQUE,
    parent_call_id TEXT,
    parent_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    position INTEGER NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK (status IN ('queued','running','completed','failed','interrupted','cancelled')),
    child_user_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    completed_at_ms INTEGER,
    error_code TEXT,
    UNIQUE(agent_id, position)
);
CREATE INDEX idx_subagent_input_next ON agent_subagent_input(agent_id, status, priority DESC, position);
CREATE TABLE agent_subagent_input_item (
    input_id TEXT NOT NULL REFERENCES agent_subagent_input(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('text','image')),
    text_content TEXT,
    archive_path TEXT,
    mime TEXT,
    content_hash TEXT,
    width INTEGER,
    height INTEGER,
    PRIMARY KEY(input_id, position)
);
CREATE TABLE agent_subagent_report (
    external_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agent_subagent(external_id) ON DELETE CASCADE,
    child_user_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('completed','failed')),
    report_json TEXT NOT NULL,
    completed_at_ms INTEGER NOT NULL,
    delivered_at_ms INTEGER
);
CREATE INDEX idx_subagent_report_pending ON agent_subagent_report(agent_id, delivered_at_ms, completed_at_ms);
```

旧 `agent_subagent.report_json` 有值时迁为一条报告，保持原 `report_delivered`。迁移检查外键与行数，版本号只在事务成功后推进。迁移失败不得标记新版本。`dedupe_key` 由父会话、父 assistant 消息和工具 `call_id` 组成并强制非空；不能靠 nullable 列的联合唯一键判断幂等，因为 SQLite 允许多个 `NULL`。队列/报告表的模型返回内容需做长度上限和控制字符净化。

失败矩阵：

| 失败点 | 必须保持的状态 |
|---|---|
| 图片校验/归档失败 | 不入队、不启动轮次，返回明确错误 |
| DB 入队失败 | 不返回成功；未引用归档可异步清理 |
| DB 成功、回复通道断开 | 以调用键幂等重取同一 `submission_id`，不双投 |
| interrupt 请求已保存但旧轮取消延迟 | 新消息留队列，旧轮结束后按优先级执行，不并发 |
| 子轮网络/工具失败 | 该 input failed，写逐轮报告；整个 agent 队列暂停，未处理输入保留，必须显式 resume 后继续 |
| 崩溃时 input running | 封为 interrupted，不自动重跑；queued 保留 |
| 归档缺失/摘要不符 | input 保持 queued 且设置 error_code，投影为 blocked；普通队列不越过该项，主 agent 可取消并补发 |
| 报告批次提交失败 | 报告仍 pending；下次空闲重试 |
| 父会话删除 | 子会话/队列/报告级联删除，图片引用清理 |

## 9. 验收标准

1. 工具面准确：主 agent 有 16 项；子角色只得到其授权工具；四种 transport 的 schema 与历史 call/result 形状一致。未知字段、空 items、`task/items` 同传、远程 URL、非图像 data URL、超出 1 MiB 的工具参数均明确拒绝。
2. `spawn_agent` 带文本+本地图片、`send_input` 带文本+data URL 的实时请求与重启回放顺序相同，原路径删除后仍可回放；图片内容不写入日志/卡片。
3. 运行中普通输入排队、interrupt 插队、旧队列保留、`cancel_agent_input` 只取消未开始项；每个操作在 DB 故障/重试时幂等。
4. 完成、失败、取消、关闭、重启恢复的状态准确；终态直接 send 失败，resume 后可以继续；重启不自动启动。
5. 多轮报告逐轮落库且恰好一次投递；`wait_agent` 不把历史完成误当当前轮完成；父会话历史回放准确。
6. 图片缺失/损坏阻止任务执行并显示明确状态；清理不删仍被子会话或队列引用的图片。
7. `curr_time` 返回实时 UTC 格式，参数严格为空对象。
8. 生产代码遵守 1000 行上限、测试按子模块下沉；精确测试、`cargo fmt`、受影响 clippy/test 通过。TUI 交互最终由用户 `cargo run` 确认。

## 10. 文档审核点

用户已确认第 2 节。以下拟定实现细节请随文档一并审核：运行中输入崩溃后标 interrupted 而不自动重试；图片错误项挡住其后的普通队列；输入队列与报告使用新增表并将现有单报告迁移；`send_input` 只支持 text/image/local_image 三种 item。审核通过后方进入 `task.md` 的代码阶段。
