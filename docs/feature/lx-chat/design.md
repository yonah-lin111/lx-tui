# lx 页流式对话与模型设置：设计（已实施）

- 状态：**已实施并合入 `dev`**（merge commit `59157e5`，2026-10-17）；`feature/lx-chat` 分支与工作区已清理。本文保留为当时的实施基线，**落地偏差与后补项见第 13 节**。
- 范围：当前仓库的 lx 页面、设置页、SQLite 与开发环境配置；不修改参考仓库。
- 日期：2026-10-16（2026-10-17 补第 13 节「实施结果与偏差」）。

## 1. 目标与可验收结果

Native Agent 入口支持纯文本多轮流式对话；正文和 thinking 按原始顺序渲染、持久化、恢复。每个 lx 窗格独立持有当前会话。重启后窗格默认是空白新会话，用户可在当前工作区的历史列表恢复旧会话。`/model` 先选模型、再选该模型已配置的思考等级。输入框附近显示实际模型、等级和上下文使用量。设置页可管理 lx-agent 同语义的供应商和模型，并从供应商获取模型列表。数据库一次建好会话、消息、内容块、工具调用和子 Agent 关系表；本期工具调用和子 Agent 表没有写入流程。

验收以真实网络流、断网/取消、重启恢复、跨窗格隔离、配置保存和旧库升级为准。只显示一段伪造的逐字动画不算流式对话。

## 2. 已确认边界

| 主题 | 确认结果 |
| --- | --- |
| 归属 | 每个 lx 窗格独立当前会话；历史按工作区过滤。窗格 `PaneId` 只作本次运行路由键。 |
| 重启 | 默认空白草稿，不自动打开最近会话；历史按钮或 `/resume` 恢复指定旧会话。 |
| 历史管理 | 可新建、重命名、删除；删除有确认，只作用于当前工作区所选会话。 |
| 表 | 本期建 session、message、message_part、tool_call、subagent 关系表；不建 Codex 已删除的 `agent_jobs` / `agent_job_items`。 |
| 配置 | 第一次从 lx-agent 导入供应商、模型、默认选择及 API Key；之后 lx-tui 独立保存，不回写、不持续同步 lx-agent。 |
| 协议 | `openai`、`openai-compatible`、`anthropic`、`google`、模型级 `openai-responses` 都能真正流式请求。 |
| thinking | 协议推理事件及 `<think>` / `<thinking>` 标记作为独立内容块保存；默认折叠，可展开恢复。 |
| 选择 | `/model` 两级面板；思考等级只来自所选模型 `variants`；无 variants 时直接使用默认值。模型/等级变动从下一轮生效，同一会话可混用模型。 |
| 并发 | 同一窗格同一时刻仅一轮请求；生成中 Enter 不新增消息；其他窗格独立。 |
| 失败 | 停止或网络失败保留已收到的部分回复和状态；后续上下文只使用完整轮次。 |
| 上下文 | DB 永久保留全历史；请求时取能装入当前模型窗口的最近完整轮次，UI 告知省略旧轮次；本期不自动摘要。 |
| 入口 | 只有 Native Agent 发送。Hybrid / Automation 不向普通对话服务发送请求。 |
| 文件 | `@` 继续作为现有输入文本/面板功能；本期不读文件、不附加内容，也不提供工具调用。 |
| 显示 | 输入框周围展示模型名称、思考等级、上下文使用量。上下文分子使用最近一次请求返回的真实输入 token；未知显示 `--`，不伪装估算为实际用量。 |

## 3. 已核实的实现基线与参考边界

| 位置 | 事实与设计影响 |
| --- | --- |
| `src/app/state/lx.rs`、`src/app/state/pane.rs` | `Pane` 内已有独立 `Lx` 草稿与编辑器状态；当前没有会话或回复列表。会话关联应加在该窗格的 lx 状态，不改全局 prompt 所有权。 |
| `src/layout.rs` | `PaneId::alloc()` 使用进程内递增值，重启后不可用于历史主键。 |
| `src/app/update/pane.rs` | `Enter` 在无粘贴/@/markdown 面板时预留给发送；新 `/` 面板必须列入现有面板优先级，不能抢走 `@`、Markdown 或粘贴输入。 |
| `src/ui/main_content/lx.rs` | 现有欢迎页、三个入口、底部可增长输入框与几何。发出第一轮后内容区转换为消息历史视图；输入框继续保留在底部。 |
| `src/ui/overlay/ui_gallery/agent.rs` | Agent 消息样式、thinking、token 页脚可借鉴视觉 token；样例数据与画廊实现不得直接成为生产聊天状态。 |
| `src/db/mod.rs` | 当前 schema v6；现有表采用 `INTEGER PRIMARY KEY AUTOINCREMENT` 内部 id、唯一 `external_id`、以稳定字符串外键关联；`PRAGMA user_version` 迁移，外键和 WAL 已启用。 |
| `src/config/mod.rs` | `settings.json` 只持久化布局整数，手写 JSON 编解码；模型配置不混入该结构。 |
| `src/settings/service/handlers.rs`、`assets/settings.*` | 本地回环 Web 设置服务已有 token、写请求同源校验及静态资源白名单；模型管理扩充独立 API 与设置视图。 |
| lx-agent `src/shared/settings.ts` | `ModelProvider` 有 `id/type/name/options/models`；模型有 `transport/limit/modalities/variants/variant/pricing`；选择为 `provider/model/variant`。配置源为 `~/.lx/config/ai.json`，可与 `builtin-providers.json` 合成有效列表。 |
| lx-agent `src/main/agent/stream/aiSdkStreamFn.ts` | 协议推理以 reasoning-start/delta/end 独立于 text；Anthropic signature 可能随空 delta 到达；模型级 variant 决定 provider 请求参数。`modelFactory.ts` 另提取 `<think>` / `<thinking>` 标记。 |
| Codex `codex-rs/state/migrations`、`protocol/v2/item.rs` | Codex SQLite 主要存 thread 元数据及 spawn edge；历史消息主要在 rollout 事件文件，不存在可照抄的 session/message/tool 四表。参考其 thread/model/reasoning/usage、typed item、tool status、父子关系字段语义，在 lx-tui SQLite 中独立建模。 |

参考代码只读。不得修改 `codex-main`、`lx-agent` 或当前用户的 `~/.lx/config/ai.json`。

## 4. 用户流程与状态语义

1. 新窗格进入 lx 欢迎页，模型取 lx-tui 配置默认选择。输入文本可保留现有 Markdown 编辑、`@` 和粘贴面板。选择 Hybrid / Automation 时发送入口不可用，并显示简短不可用状态；不静默改成 Native。
2. Native 窗格首次发送前校验非空输入、可用供应商、已启用模型与 API Key。尚无 session 的空白窗格是内存草稿，不写空 session。提交用户消息事务成功后才清空输入并启动模型请求；落库失败保留草稿且不发网络请求。
3. 首个成功提交建立 session；一个发送动作对应一条 user message 和一条 assistant message，后者按事件追加 `reasoning`/`text` part。界面增量渲染，用户可在自己的窗格滚动历史、展开 thinking 和停止生成。
4. `/model` 改变当前窗格下一轮选择；选择后不改已保存的 message 元数据。没有 `variants` 的模型跳过等级选择。`/new` 清当前窗格会话关联并回空白草稿；`/resume` 打开当前工作区的历史；历史列表提供打开、重命名、删除。
5. 切换工作区/标签/窗格或切到终端时，在途请求继续归属原 session；返回事件必须携带 session `external_id` 和 generation，不因焦点改变写进其他窗格。关闭窗格/工作区时主动取消相应运行请求并保存 partial；其已保存历史仍按原工作区可恢复。
6. 重启后历史列表从 SQLite 按当前工作区分页读取。恢复旧会话只绑定当前窗格；同一 session 不允许同时绑定两个正在运行的窗格，避免并发追加乱序。历史中的已完成正文/思考可查看，失败/中断状态可识别。

## 5. 数据库设计与迁移

### 5.1 主键与外键

新记录沿用本项目做法：`id INTEGER PRIMARY KEY AUTOINCREMENT` 是本地排序/CRUD 内部键；`external_id TEXT NOT NULL UNIQUE` 用 `new_external_id(prefix)` 生成，是跨事件、跨进程的稳定标识。所有跨表外键引用 `external_id`，不引用易变 `PaneId`。时间统一为 UTC 毫秒整数，便于同毫秒按 `id` 排序；已有表的时间格式不改。状态、role、part kind 使用 `CHECK` 或存储层校验，未知扩展字段以 nullable JSON 字符串保存，不让列数膨胀。

### 5.2 v7 新表

| 表 | 核心字段 | 约束与索引 |
| --- | --- | --- |
| `agent_session` | `id`, `external_id`, `workspace_id`, `source` (`native`), `title`, `first_user_message`, `model_provider`, `model_id`, `variant`, `status`, `archived`, `created_at_ms`, `updated_at_ms`, `last_message_at_ms`, `total_input_tokens`, `total_output_tokens`；预留 nullable `agent_nickname`, `agent_role`, `agent_path`。 | `workspace_id -> workspace(external_id) ON DELETE CASCADE`；列表索引 `(workspace_id, archived, updated_at_ms DESC, id DESC)`；`external_id` 唯一。title 初值取首条用户文本经单行截断，不启动额外模型请求。 |
| `agent_message` | `id`, `external_id`, `session_id`, `sequence`, `turn_index`, `role` (`user/assistant`), `status` (`complete/streaming/interrupted/failed`), `model_provider`, `model_id`, `variant`, `stop_reason`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `error_code`, `created_at_ms`, `completed_at_ms`。 | `session_id -> agent_session(external_id) ON DELETE CASCADE`；`UNIQUE(session_id, sequence)`；索引 `(session_id, sequence)`。用户消息 `complete`；assistant 保留实际该轮模型和等级。错误文本不存密钥或原始 HTTP 响应。 |
| `agent_message_part` | `id`, `external_id`, `message_id`, `position`, `kind` (`text/reasoning`), `content`, `provider_metadata_json`, `created_at_ms`。 | `message_id -> agent_message(external_id) ON DELETE CASCADE`；`UNIQUE(message_id, position)`。相邻同 kind delta 合并更新，类型转换新建 part；可选元数据保存 Anthropic thinking signature 等续聊所需字段。 |
| `agent_tool_call` | `id`, `external_id`, `session_id`, `message_id`, `call_id`, `namespace`, `name`, `arguments_json`, `status`, `result_json`, `error_text`, `started_at_ms`, `completed_at_ms`, `duration_ms`。 | 两个外键级联；`UNIQUE(session_id, call_id)`；按 `(session_id, started_at_ms, id)` 索引。**本期不写行、不把 tools 传给模型**；预建结构覆盖 Codex item 的名称、参数、状态和结果。 |
| `agent_subagent` | `id`, `external_id`, `parent_session_id`, `child_session_id`, `spawn_message_id`, `status`, `nickname`, `role`, `agent_path`, `model_provider`, `model_id`, `variant`, `created_at_ms`, `updated_at_ms`, `completed_at_ms`。 | parent/child 均引用 `agent_session(external_id)`，`child_session_id` 唯一，父状态索引 `(parent_session_id, status)`；不允许自指和重复 child。相当于 Codex `thread_spawn_edges` 加 agent 元数据；本期不写行。 |

审核用的 v7 目标 DDL（正式实现可只调整 SQL 排版和命名，不改变约束语义）：

```sql
CREATE TABLE agent_session (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL REFERENCES workspace(external_id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'native' CHECK (source IN ('native', 'subagent')),
    title TEXT NOT NULL,
    first_user_message TEXT NOT NULL DEFAULT '',
    model_provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
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
    total_output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_output_tokens >= 0)
);
CREATE INDEX idx_agent_session_workspace_recent
    ON agent_session(workspace_id, archived, updated_at_ms DESC, id DESC);

CREATE TABLE agent_message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    turn_index INTEGER NOT NULL CHECK (turn_index >= 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    status TEXT NOT NULL CHECK (status IN ('complete', 'streaming', 'interrupted', 'failed')),
    model_provider TEXT,
    model_id TEXT,
    variant TEXT,
    stop_reason TEXT,
    input_tokens INTEGER CHECK (input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens >= 0),
    cache_read_tokens INTEGER CHECK (cache_read_tokens >= 0),
    cache_write_tokens INTEGER CHECK (cache_write_tokens >= 0),
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE(session_id, sequence)
);

CREATE TABLE agent_message_part (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'reasoning')),
    content TEXT NOT NULL DEFAULT '',
    provider_metadata_json TEXT,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(message_id, position)
);

CREATE TABLE agent_tool_call (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    call_id TEXT NOT NULL,
    namespace TEXT,
    name TEXT NOT NULL,
    arguments_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    result_json TEXT,
    error_text TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    duration_ms INTEGER CHECK (duration_ms >= 0),
    UNIQUE(session_id, call_id)
);
CREATE INDEX idx_agent_tool_call_session_started
    ON agent_tool_call(session_id, started_at_ms, id);

CREATE TABLE agent_subagent (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    parent_session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    child_session_id TEXT NOT NULL UNIQUE REFERENCES agent_session(external_id) ON DELETE CASCADE,
    spawn_message_id TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'blocked', 'completed', 'failed', 'cancelled')),
    nickname TEXT,
    role TEXT,
    agent_path TEXT,
    model_provider TEXT,
    model_id TEXT,
    variant TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    CHECK (parent_session_id <> child_session_id)
);
CREATE INDEX idx_agent_subagent_parent_status
    ON agent_subagent(parent_session_id, status);
```

`agent_message` 的 user 行 `model_provider/model_id/variant` 为空，assistant 行在发送事务时写入实际请求快照。session 的三列模型选择表示**当前会话下次发送的选择**：`/model` 在已保存会话中更新这三列，在尚未发送的空白草稿中只改内存。message 始终保存该轮实际使用值，因此切换模型不篡改历史。`first_user_message` 是列表预览，`title` 可由用户重命名。`archived` 与 `status` 是兼容 Codex 会话生命周期的字段，本期 UI 不提供归档操作，写入值保持 `0/active`。工具与子 Agent 表只建 schema，不生成占位记录。

迁移保持旧 v1-v6 链原样，单独加 `SCHEMA_V7` 并仅在 `< 7` 时同一事务建表、建索引、更新 `user_version=7`。旧 DB 只增加表，不重建既有表、不改已有记录。运行前后 `PRAGMA foreign_key_check`；失败保持旧版本，重启可重试。新库从 v0 路径也须得到同一 v7 结构。完成时按本项目既有版本升级测试覆盖 v6→v7、v0→v7、重复打开、外键级联与非法新版本拒绝。

### 5.3 写入顺序与恢复

发送事务：建立/更新 session、插入 user message 及 user text part、插入 `streaming` assistant message，再返回 request key。网络任务只在事务确认后启动。增量先进入内存，最多每 500ms 或累计 4 KiB 做一次批量 checkpoint；停止、完成、失败时无条件提交最终内容和状态。数据库 worker 串行消费，单个 session 的 sequence 单调；最终提交必须等待此前 checkpoint，过期 generation 不能覆盖较新的内容。异常退出时下次启动将遗留 `streaming` 改为 `interrupted`，保留上次 checkpoint；不自动重发请求。未收到任何内容的失败 assistant 行可留状态记录，不凭空补正文。

重命名以 workspace + session 双条件更新。删除在事务内按相同双条件执行，级联 message/part/tool/subagent；当前选中的在途会话先取消并等待最终写入，再删除，防止晚到 checkpoint 复活。删除空草稿只清内存。数据库不存 API Key。

### 5.4 关键 SQL 与写入契约（设计草图）

下面是事务边界示意，不是现有代码；正式实现以 `rusqlite` 参数绑定执行，不能拼接正文。`sequence` 在同一事务内读取并递增；`turn_index` 同一轮的 user/assistant 相同，`sequence` 分别占两个连续位置。首轮还需在同事务创建 session 和 title。

```sql
BEGIN IMMEDIATE;
SELECT COALESCE(MAX(sequence) + 1, 0) FROM agent_message WHERE session_id = ?1;
INSERT INTO agent_message
  (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
VALUES (?2, ?1, ?3, ?4, 'user', 'complete', ?5);
INSERT INTO agent_message_part
  (external_id, message_id, position, kind, content, created_at_ms)
VALUES (?6, ?2, 0, 'text', ?7, ?5);
INSERT INTO agent_message
  (external_id, session_id, sequence, turn_index, role, status,
   model_provider, model_id, variant, created_at_ms)
VALUES (?8, ?1, ?3 + 1, ?4, 'assistant', 'streaming', ?9, ?10, ?11, ?5);
COMMIT;
```

`?1` 所指 session 必须先用 `workspace_id` 校验；不能只凭客户端传来的 `session_id` 写入。上面省略了首轮 session 插入、session 时间戳更新和错误回滚代码。`MAX(sequence)` 只在持有写事务时读取；若独立连接并行写，同一 session 还要由 worker 串行化。assistant 的 `external_id` 是该轮 checkpoint 和终态更新的唯一目标。

存储 API 的最小输入/输出契约：`begin_turn(workspace_id, existing_session_id?, user_text, model_snapshot) -> TurnCommitted { session_id, user_message_id, assistant_message_id, turn_index }`；`checkpoint(assistant_message_id, ordered_parts)` 使用完整快照幂等覆盖已有 part 内容，不按网络 delta 盲目追加；`finish(assistant_message_id, ordered_parts, outcome, usage?)` 同事务保存最后内容、终态和 session 时间戳。每条写入命令还携带运行时 `generation`，由事件层过滤旧结果；数据库通过 `status='streaming'` 条件更新防止终态被迟到 checkpoint 覆盖。`finish` 对同一请求重复投递只允许同结果幂等，不重复累计 usage。

异常退出恢复只处理 `status='streaming'` 的 assistant；user 行不修改。恢复后的 `interrupted` 是可查看记录，下一轮上下文仍跳过该 `turn_index`。session 的总 token 是统计值，`NULL` 的单轮 usage 不贡献 0，也不推导出“已知零 token”。

## 6. 模型配置与首次导入

### 6.1 文件职责

`~/.lx-t-dev/settings.json` 继续只存布局；生产对应 `~/.lx-t/settings.json`。模型配置单列 `~/.lx-t-dev/config/ai.json`（生产 `~/.lx-t/config/ai.json`），不做一个覆盖布局和模型的巨型 `config.json`。模型文件采用可扩展的 `ai` 节点：`providers`、`enabled_providers`、`defaultModel`、`streamIdleTimeoutMs`。每个 provider 沿用 lx-agent `id/type/name/options{apiKey,baseURL}/models`；每个模型沿用 `id/name/transport/limit{context,output}/modalities/variants/variant/pricing`。本期运行只用聊天相关字段，其余已知字段无损往返；未知 JSON 字段也尽量保留，避免设置保存把未来配置洗掉。配置解析用 `serde`/`serde_json`，不能继续扩展现有手写整数 JSON 解析器。

目标文件最小结构示例（假 Key）：

```json
{
  "ai": {
    "enabled_providers": ["example"],
    "defaultModel": {"provider": "example", "model": "example-chat", "variant": "high"},
    "providers": {
      "example": {
        "id": "example",
        "type": "openai-compatible",
        "name": "Example",
        "options": {"apiKey": "test-key-only", "baseURL": "https://api.example.invalid/v1"},
        "models": {
          "example-chat": {
            "id": "example-chat",
            "name": "Example Chat",
            "limit": {"context": 128000, "output": 8192},
            "variants": {"high": {"reasoningEffort": "high"}},
            "variant": "high"
          }
        }
      }
    },
    "streamIdleTimeoutMs": 60000
  }
}
```

布局和模型写入各自文件。配置读写必须基于解析后的 JSON 对象更新目标字段，而不能通过字符串替换；`providers` 以 `id` 为稳定键，修改显示名 `name` 不影响会话引用，复制时新建 `id`。`defaultModel.variant` 必须存在于该模型 `variants`，否则用模型默认 variant 或空值，不生成虚构的 low/medium/high 列表。

文件示例中的 `apiKey` 必须只用占位值，文档与测试永不包含真实密钥。保存采用同目录临时文件、flush、原子替换；Unix 目录 `0700`、文件/临时文件 `0600`，现有权限更严格时不放宽，权限设置的 OS 专属实现只放 `src/platform/`。读取配置错误在设置页和 TUI 显示可理解错误，不以空配置覆盖损坏文件。

### 6.2 一次导入算法

仅当 lx-tui 的 `config/ai.json` **不存在**时，读取 lx-agent 的 `~/.lx/config/ai.json`，必要时读取 `builtin-providers.json` 补充有效预置供应商。只导入模型相关配置和 API Key，不导入 lx-agent 的 Agent、工具、补全、搜索等独立设置。过滤/保留规则：验证 provider type、模型级 transport、模型 ID、variants 与默认选择；保留原始模型名、上下文/输出上限、计价和模态；缺省默认选择落到首个已启用且有模型的 provider。若来源不存在，创建空模型配置并在 UI 指向添加供应商；若来源存在但损坏，报错且不创建目标文件，用户修复来源或手动初始化后可重试。目标文件一旦存在（包括用户手动创建的空配置）就不再导入；以文件存在本身作为幂等标记，不写额外标记或反向同步。

读取源配置时不在日志、toast、HTTP 错误或测试快照打印 API Key。导入完成后只使用本项目文件；lx-agent 后续变化不会影响本项目。`LX_TUI_ENV` / `LX_ENV` 的开发/生产根规则与 DB 保持一致；测试以隔离 HOME/显式路径执行，绝不读取本机真实凭据。

## 7. 设置页交互与 HTTP API

设置页新增 Models 视图，保留原有布局/命令视图。左侧供应商列表可添加、自定义名称、启用/停用、复制、删除；右侧编辑 provider type、base URL、API Key、模型清单。模型可手工添加、编辑 ID/名称/transport/上下文上限/输出上限/variants/默认 variant/计价，复制、删除；选择全局默认模型。复制供应商/模型时生成新的稳定配置 ID，保留原对象其他字段，不能共用可变引用。删除当前默认模型或关闭其 provider 时要求先选择有效替代，设置保存服务端再验证一遍。

“获取模型”仅对有 endpoint 和 API Key 的兼容供应商提供；服务端代理请求，浏览器绝不直接持有跨域调用，也不把 API Key 写进静态资源。按 lx-agent 语义尝试 OpenAI 兼容 `/v1/models` 的合理 URL 候选，解析 `{data:[{id,owned_by}]}`，去重、按 id 排序；获取只显示候选，用户选择“添加”后才改配置，不自动覆盖已编辑模型。Anthropic/Google 若配置了可用模型发现端点则走对应协议适配；不支持发现的供应商仍可手动加模型，不伪装成成功。请求限制 HTTPS/显式本地 HTTP、超时、响应大小和重定向，错误提示只含状态与裁剪后的非敏感说明。

建议独立 API 契约：`GET /api/models/settings` 取脱敏 provider/model 列表和默认值，`PUT /api/models/settings` 完整验证后保存，`POST /api/models/fetch` 获取未保存的候选；API Key 更新采用“留空不改、显式清除”的字段语义，GET 只返回 `hasApiKey`，不返回原文。沿用本地 token 与写请求同源校验，新增请求体/响应体大小上限。保存成功经 `SettingsEvent::ModelsChanged` 送回唯一事件循环，后者更新模型快照；正在运行的请求持有旧配置快照直到结束。静态 HTML/CSS/JS 只改 `assets/settings.*`，不能内联入 Rust。

API 字段与错误语义（所有 JSON 请求仍带现有 query token）：

| 请求 | 关键字段 | 成功 / 失败 |
| --- | --- | --- |
| `GET /api/models/settings` | 返回 `providers`、`enabledProviders`、`defaultModel`、`streamIdleTimeoutMs`；每个 provider 仅有 `hasApiKey`。 | `200`；无配置时是空列表和空选择，不凭空出现默认模型。 |
| `PUT /api/models/settings` | 完整更新模型配置；每个 provider 的 `apiKey` 省略/空字符串表示保持，`clearApiKey: true` 表示删除，非空字符串表示替换；提交时带 `revision` 防止两个设置页互相覆盖。 | `200` 返回新 revision；校验失败 `422`，旧 revision `409`，磁盘失败 `500`；失败不得改变运行时配置。 |
| `POST /api/models/fetch` | `providerId` 或尚未保存的 `{type, baseURL, apiKey}`，当前页面明示触发。 | `200 {models:[{id,ownedBy}]}`；无 Key `422`、上游错误 `502`、超时 `504`；响应不包含 Key。 |

保存 API 接收 `revision` 的具体实现可用配置文件内容哈希或单调版本，但比较与写盘必须序列化；不需要在 DB 另建配置表。设置服务请求处理仍由现有本地服务装配，远端模型发现的异步结果不能延长本地连接到无限等待。

### 7.1 配置类型与密钥更新（设计草图）

配置对象与 Web DTO 分开：磁盘 `ProviderOptions` 含真实 `api_key`；GET DTO 只有 `has_api_key`；PUT DTO 的 `api_key` 是本次动作而非当前值。可用 `serde(flatten)` 保存未知字段，但已知字段必须做类型校验，不能因为未知字段容忍错写的 `limit.context`。

```rust
// 示意类型，字段名与 serde rename 以 lx-agent 的真实 JSON 为准。
struct AiConfig { providers: BTreeMap<String, Provider>, default_model: Option<ModelRef> }
struct ModelRef { provider: String, model: String, variant: Option<String> }
enum KeyChange { Keep, Replace(String), Clear }

fn apply_provider_edit(old: Option<&Provider>, edit: ProviderEdit) -> Result<Provider> {
    let api_key = match edit.key_change {
        KeyChange::Keep => old.and_then(|p| p.options.api_key.clone()),
        KeyChange::Replace(value) => Some(value),
        KeyChange::Clear => None,
    };
    // 合并其他字段后验证 ID、transport、URL、默认模型与 variants。
    build_validated_provider(edit, api_key)
}
```

`Keep` 对新 provider 且无旧 Key 时得到 `None`，保存前会作为未配置项显示；发送时拒绝缺 Key。复制 provider 是客户端生成新 ID 的深拷贝，出于 GET 脱敏，默认不复制旧 Key；用户显式输入新 Key 后才可启用请求。服务端以 revision 检查原配置、合并密钥、验证、原子写盘、广播新配置快照为一个串行操作；写盘失败时不广播。`POST /api/models/fetch` 使用用户此次输入的 Key 或服务端已保存 Key，不能把已保存 Key 回填到响应。

## 8. 流式协议和 thinking

网络请求在独立 Tokio 任务内运行；共享 `reqwest` 客户端负责 TLS、超时、连接复用，按 chunk 增量解析 SSE/JSON，不能在事件循环里同步网络或磁盘。使用结构化 JSON 解析事件；帧支持跨 chunk、CRLF、空行、多行 `data:`、UTF-8 分段；控制每帧和累计正文大小。`[DONE]`、完成事件、EOF、HTTP 错误与用户取消是不同结束原因。provider 原始错误体只能经过截断/脱敏再进入 UI/DB。取消 token 关联 request key；超时包括连接与流空闲，活动流不受固定总时长误杀。

| transport | 请求与文本流 | thinking 提取 | 用量 |
| --- | --- | --- | --- |
| `openai` | Chat Completions SSE，`choices[].delta.content`；provider `type=openai`。 | 解析兼容的 `delta.reasoning_content`/`reasoning`；若正文包 `<think>` 或 `<thinking>`，增量拆块。 | 末帧 `usage.prompt_tokens` 等；缺失保持未知。 |
| `openai-compatible` | 配置 baseURL 上的 OpenAI 兼容 Chat Completions SSE；URL 拼接避免重复 `/v1`。 | 同上，支持供应商额外推理字段；未知字段不当作正文。 | 同上。 |
| `openai-responses` | 模型级 transport 优先于 provider type；Responses API SSE 的 `response.output_text.delta` 等。 | `response.reasoning_summary_text.delta` 和可用 reasoning text 事件分流，保留 summary/raw kind 元数据。 | `response.completed.usage`。 |
| `anthropic` | Messages SSE 的 `content_block_start/delta/stop` 与 `text_delta`。 | `thinking_delta` 单独 part；`signature_delta` 附到对应 thinking part 元数据，供可续聊请求使用。 | `message_start`/`message_delta` 的 input/output/cache 字段。 |
| `google` | Gemini `streamGenerateContent?alt=sse` 的候选 parts。 | `thought=true` 的 part 独立为 reasoning；普通 part 为正文。 | `usageMetadata.promptTokenCount` 等。 |

`variant` 是配置中的等级键，不等同跨供应商统一的 `reasoning_effort` 字符串。按有效 transport 将已知 variant 字段映射为请求参数：OpenAI/Responses `reasoningEffort`、`reasoningSummary`、`include`；Anthropic `thinkingBudget`/`thinking`/`effort`；Google `thinkingBudget`；兼容端点保留其明确配置的附加参数。不得把任意 variant JSON 直接放进 HTTP header 或 URL。无 variants 时不发送伪造等级。模型返回内容与 thinking 可交错，part `position` 必须按收到顺序排列；识别完整的开/闭标签后切换模式，跨 chunk 的半截标签先缓冲，不泄漏到正文；流中断时保留已识别开标签后的 reasoning partial。不同来源的推理内容不可合并成一个“普通回答”。

后续请求从已完成轮次构造消息；如 provider 要求签名/opaque reasoning 元数据，使用保存的 part metadata 原样回传。若某模型不能接受旧模型的 reasoning 元数据，降为只回传该轮已完成的正文，不向其他 provider 泄露专有元数据。工具列表始终为空；若服务端仍返回 tool call，本轮以不支持错误结束，不执行任何调用。

### 8.1 统一事件、帧解析与 thinking（设计草图）

协议适配器只产出数据事件。`request_key` 在发请求时固定，包含 `pane_id + session_id + generation`；`PaneId` 仅运行期路由，持久化只用 `session_id`/message ID。`Completed` 与 `Failed`/`Cancelled` 互斥，适配器必须恰好发出一个终态；EOF 若缺协议完成信号视为中断或协议错误，不擅自标记完成。

```rust
enum ChatDelta {
    Part { kind: PartKind, text: String, metadata: Option<serde_json::Value> },
    Usage { input: Option<u64>, output: Option<u64>, cache_read: Option<u64> },
    Finished { reason: StopReason },
}
enum PartKind { Text, Reasoning }
struct RequestKey { pane: PaneId, session_id: String, generation: u64 }
```

SSE 解析分两层：字节缓冲只在得到完整 UTF-8 行后按空行组帧；`data:` 多行按规范以换行连接，然后用 `serde_json` 解码协议事件。`event:` 可决定 Anthropic/Responses 类型；心跳注释丢弃。对 Google SSE 复用帧层，但 payload 按 Gemini JSON 结构解析。设置单帧字节上限及单轮可保留正文上限；超限给明确失败并保存此前 partial。若服务以 HTTP 200 返回非 SSE 错误 JSON，也应识别并报协议错误。

正文标签解析器持有 `mode = Text | Reasoning`、尚未判定的短前缀缓冲以及当前标签类型。遇到完整 `<think>` 或 `<thinking>` 才切入 Reasoning；关闭标签切回 Text；跨 chunk 的 `<thi` 不能立即绘制成正文。普通比较符号如 `a < b` 必须原样吐出。流结束时未闭合标签若已进入 reasoning，保留已收到的 reasoning 并将终态标为不完整；不能把残余标签写进正文。协议原生 reasoning 直接进入 Reasoning part，不再二次扫描标签；相邻同 kind 且元数据兼容的 delta 合并，否则创建新的 `position`，从而保留 text/reasoning/text 交错顺序。

Anthropic 的 `signature_delta` 以 `content_block_index` 找到对应 thinking part，即使该帧没有文本也必须更新其 `provider_metadata_json`。Google 同一帧可能同时有多个 parts，要逐项保持顺序。Responses 的 summary 与原始 reasoning 用 metadata 区分，不把 summary 重复写进正文。兼容供应商的 `reasoning_content` 只按显式识别的字段解析，不把未知 JSON 字段猜成 thinking。以上映射应按实际 SDK/协议样本写 fixture，不能以一套假事件覆盖五种 transport。

### 8.2 请求字段映射与端点规则

| transport | 请求入口与核心字段 | variant 处理 | 完成判定 |
| --- | --- | --- | --- |
| OpenAI Chat | `POST /chat/completions`，`model/messages/stream=true`，请求 usage 末帧；官方 URL 由配置默认值确定。 | `reasoningEffort` 映射到协议支持的请求字段；无值不发送。 | `finish_reason` 与 `[DONE]`，保存实际 stop reason。 |
| OpenAI-compatible | base URL 与 `/chat/completions` 安全合并，保留用户配置的路径前缀；认证按 provider 选定方式。 | 仅发送该供应商配置明确支持的参数；服务拒绝可诊断。 | 兼容 SSE 的结束帧；缺失时中断。 |
| OpenAI Responses | `POST /responses`，`model/input/stream=true`，不沿用 Chat 的 `messages` 形状。 | `reasoning.effort`、summary/include 按配置构造结构化字段。 | `response.completed` 或 `response.failed`。 |
| Anthropic | `POST /v1/messages`，`model/messages/max_tokens/stream=true`；按协议放版本与 Key header。 | 有 thinking 预算时设置相应结构；`max_tokens` 必须与预算约束一致。 | `message_stop`；错误事件为失败。 |
| Google | `models/{model}:streamGenerateContent?alt=sse`，`contents/generationConfig`；Key 只按该协议放置。 | `thinkingConfig` 从已知配置映射。 | 完整候选结束/协议结束标记；只收到半个 JSON 不算完成。 |

URL 组合、认证位置、variant 字段名须以实施时 Context7 与本地适配器 fixture 校验；表中路径是边界示意，不允许通过字符串替换猜测最终 URL。所有 transport 的请求都不带 tools/function declarations，任何意外工具调用事件都终止本轮并显示不支持。

## 9. 上下文装配、用量与模型切换

每轮请求选定 `(provider, model, variant)` 快照，保存到该 assistant message。按时间从近到远选完整 user+assistant 轮次，保持原顺序回传；失败/中断 assistant 及其对应未完成轮不进入请求。当前用户消息必须保留。`limit.context` 和 `limit.output` 来自模型配置，预算扣除输出预留及请求封装开销。预选用明示的保守估算，只用于装配，**不作为 UI 的“已用 token”**；若服务端仍报告上下文溢出，最多一次去掉最老完整轮次重试，不能重复提交用户消息；单条最新消息本身超限时给可见错误，不截断用户正文。没有 `limit.context` 时不声称能保证窗口，按完整历史发送并在溢出时按同规则收缩。

输入区状态行展示 `Provider / Model`、当前 `variant` 的友好标签、`实际输入 token / 配置上限`。真实输入 token 以最近**同一模型**成功收到的 provider usage 为准；切模型时旧分子清空为 `--`，上限立即更新；无上限显示 `已用 / --`。流进行中可另标 `Generating`，不以临时字符估算替换真实值。因上下文裁剪省略旧轮时，紧邻状态行显示一次可见的省略提示。DB 保存每轮 usage 和 session 聚合；同一模型历史用量显示最近请求口径，不把所有轮次累计值误标成“当前上下文”。

上下文选择草图：按 `turn_index` 从 DB 读取，先取当前 user，再从近到远加入同时含 `complete` user 与 `complete` assistant 的整轮；估算超过 `context - output_reserve - envelope_reserve` 时停止纳入更老轮，而非拆开一轮。最后按原 `turn_index` 正序编码。历史 text 必须原样保留；跨 provider 的 reasoning metadata 不携带，必要时只将已完成 assistant 正文编码为历史 assistant。服务报告上下文超限时，从**本次已选历史**中再移除最老一轮，最多重试一次，沿用原 `assistant_message_id` 和 request generation，不重新插入 DB 行。第一次尝试可能已消耗 token，但 UI 的分子只取最终收到的、明确属于本轮的 provider usage。

## 10. lx 交互和布局

空白欢迎页保留 Native/Hybrid/Automation 入口及既有编辑器。发出首条消息后，品牌区让位给可滚动历史；输入框仍位于底部，高度继续受 40% 上限。历史区从旧到新排 user、assistant 消息；assistant 的 reasoning 默认折叠，展开后按保存顺序查看；流中状态、停止按钮、错误/中断标记不改变相邻组件尺寸。新消息默认跟随底部；用户主动上滚后暂停跟随，提供回到底部动作。窗口缩窄时优先保持输入与历史可读，状态行按 `模型 → 等级 → 用量` 次序逐级缩写/分行，不与输入、面板或消息重叠；模型名过长按终端列宽截断并可在选择面板看全名。按钮命中几何与渲染由同一纯布局函数派生。

历史按钮位于 lx 内容区域输入框周围，点击打开只作用当前窗格的列表浮层；`/resume` 同入口。列表显示标题、最近活动时间、状态，分页/滚动、不跨工作区；打开选中会话、重命名、删除确认、创建新会话。`/model` 候选只包含已启用且有模型的 provider；模型选中后仅列其 `variants`，默认等级有明确标记。`/new` 与 `/resume` 为保留命令。`/` 候选只在行首命令位置触发，围栏代码块和普通句子中的 `/` 是文本；采纳命令时不发送命令字符串。面板优先级：现有粘贴参照、`@` 提及、Markdown 块面板优先于 slash；同时可见状态最多一个。Enter 在面板打开时先采纳，在无面板且 Native 空闲时发送，在流中不重复发送；Shift/Ctrl+Enter 仍换行。Esc 关闭最上层面板；停止流由显式控件/动作触发，不借用编辑器原有 `Ctrl+C` 清空行为。

历史渲染可借鉴 `ui-gallery/agent` 的样式和 `ui::markdown`，但业务组件建在 `src/ui/main_content/lx/` 的新文件中；`ui/` 只读 `AppState`，不访问 DB/网络。状态和更新放 `app/`，按键映射放 `input/`，IO 编排放 `event/`，网络协议适配独立模块，SQLite 只在 `db/`。文件分拆遵守单文件 1000 行上限，新的职责文件控制约 500 行；不为未来工具执行建空 Rust 模块。

### 10.1 窗格状态与事件时序（设计草图）

`Lx` 保留编辑器与面板状态，增加 `session_id: Option<String>`、`request: RequestState`、历史视图与当前 `ModelRef`；持久化连接和 HTTP client 不进入 `AppState`。动作分成纯状态更新与待执行 effect；现有 `apply_lx_input` 在面板优先级处理完后把无面板 Enter 转为 `SendRequested`，不在 `app/` 同步读库。具体 Rust 名称可按现有 action 风格收敛。

```rust
enum RequestState {
    Idle,
    Committing { generation: u64, draft: String },
    Streaming { key: RequestKey, assistant_message_id: String },
    Finishing { key: RequestKey, reason: StopReason },
}
enum ChatEffect {
    CommitTurn { pane: PaneId, generation: u64, draft: String, model: ModelSnapshot },
    StartStream { key: RequestKey, assistant_message_id: String, model: ModelSnapshot },
    Checkpoint { key: RequestKey, parts: Vec<PartSnapshot> },
    Finish { key: RequestKey, parts: Vec<PartSnapshot>, outcome: TurnOutcome },
}
```

| 当前状态 / 事件 | 下一状态 | 必须完成的动作 |
| --- | --- | --- |
| Idle + Enter | Committing | 冻结草稿和模型快照，入队 `CommitTurn`；重复 Enter 无效。 |
| Committing + DB 成功 | Streaming | 绑定 session/message ID，清除**仍与冻结草稿相同**的输入，启动网络；若期间用户继续编辑，新草稿不能被清掉。 |
| Committing + DB 失败 | Idle | 保留输入和选择，展示可重试错误，不发请求。 |
| Streaming + delta | Streaming | 更新有序 parts，按时间/大小合并 checkpoint，触发重绘。 |
| Streaming + 完成/失败/停止 | Finishing | 先取消或关闭流，排在全部 checkpoint 后提交最终快照。 |
| Finishing + DB 成功 | Idle | 显示完整/失败/中断状态，释放 session 占用；必要时继续已确认的删除。 |
| Finishing + DB 失败 | Finishing（保存失败） | 保留内存 partial 并提示不可保证恢复；不能把它标记为已持久化。 |

所有后台返回通过 `AppEvent` 进入唯一事件循环，再按 `(pane, session_id, generation)` 匹配；`SettingsEvent::ModelsChanged` 在设置文件落盘成功后通知主循环。取消发生在 Committing 时，等 DB ack 后直接落 `interrupted` 而不发 HTTP。删除发生在 Streaming/Finishing 时，先标记待删除、取消流、等待终态写入，再在同一 worker 队列删除；删除成功后晚到事件只被忽略。`/new` 在旧请求仍在途时只解绑当前窗格并取消其请求，旧 session 的最终写入继续按原 ID 完成。关闭窗格同理。切焦点、切终端视图不取消请求。

后台 worker 使用有界命令队列；TUI 主循环不等待 SQLite 或 HTTP。网络 delta 可在后台合并后投递，避免逐 token 无界堆积；最终 `Finish` 不可因队列满而丢失。checkpoint 失败须在 UI 明示，并停止或重试持久化，不能继续宣称历史完整。`AppEvent` 的流事件只携数据，不携 `reqwest::Response` 或 DB connection。

### 10.2 输入区布局和命令命中（设计草图）

宽窗格底部从上到下为历史内容、单行或双行状态区、现有编辑器、操作区；状态区可显示 `模型名  ·  思考等级  ·  42k / 128k`，生成时操作区显示停止图标，历史入口常驻。中等宽度将用量折到第二行；极窄时只保留截断模型名和 `42k/128k`，等级由 `/model` 面板完整可见。`--/128k` 表示缺真实 usage，`42k/--` 表示缺上限。状态区高度由同一 `layout_lx(rect, state) -> LxRects` 计算后传给渲染和鼠标命中，不能让字符串长度改变编辑器或按钮矩形。历史内容不足一行时也保留编辑器最小高度。

slash 识别用编辑器的结构化行/光标状态：仅当前编辑段落首个非空字符是 `/` 且光标不在围栏代码块内时产生候选；输入 `/model` 并确认后消费整条命令，不把它写入草稿或 DB。采纳层次为 `model list -> variant list -> 关闭`；Esc 在等级列表退回模型列表，第二次关闭。只有已启用 provider 和有效模型进入列表，变更选择立即刷新状态行但不改在途 `ModelSnapshot`。命令面板与 `@`、粘贴、Markdown 面板的优先级在状态更新层统一决定，渲染按该结果画一个面板。

已保存 session 的模型选择也要写入 session 三列：选择动作先改窗格显示并排队写 DB；在写入确认前，该 session 的下一次发送等待同一 worker 队列中的选择写入，失败则回滚显示选择并提示。新空白草稿只改内存。这样重启后 `/resume` 恢复的是最后成功保存的选择，而在途 HTTP 始终使用发送时冻结的旧快照。

## 11. 错误、安全与竞争条件

- 三类失败分别展示：配置无效/缺密钥（请求前）、HTTP/协议/超时（请求中）、DB 保存失败（不可伪装成成功）。生成任务和持久化 worker 都用 session external ID + generation 过滤晚到事件。
- 同一 session 的发送、恢复和删除串行化；另一个窗格试图打开正在生成的同一 session 时只读查看或提示占用，不启动第二轮。跨窗格各自发送可并行。
- 不对外打印密钥、完整请求 header、用户正文或 thinking 到 tracing；模型请求仅发送到用户显式配置的端点，默认无跨主机重定向，密钥只加到对应 provider 协议要求的位置。
- Web 设置页不回传密钥原文，不把配置对象塞进静态 HTML/JS。服务仍仅监听回环并校验 token 与写请求 Origin；动态文本用 `textContent`，不以 `innerHTML` 拼接供应商返回的模型名。
- 完成/停止/失败都关闭请求、释放资源、持久化终态；TUI 退出先取消在途网络并提交可保存的 partial，再按现有终端恢复顺序退出。若 DB 不可用，显式提示保存失败，不把未保存内容声称为“已恢复”。

## 12. 验收与回归范围

1. 旧 v6 数据升级后提示词、文件夹、工作区信任记录不变；新表/外键/索引存在，重复迁移无副作用。
2. 第一次导入在隔离 HOME 下只写本项目 `config/ai.json`，源文件字节不变；第二次启动或目标文件已存在时不覆盖；密钥不出现在 API GET、日志或前端资源。
3. 五种传输用本地 mock 流测试正文、thinking、UTF-8/JSON/SSE 分块、usage、结束/停止/超时、签名保留及协议错误；没有任何 tool 执行。
4. 同一会话跨轮模型切换，历史顺序、part 顺序和每轮模型/等级正确；重启默认空白、`/resume` 恢复，工作区与窗格隔离，删除不留孤儿记录。
5. 上下文超额时只省略最老完整轮，界面提示；失败/中断 partial 可查看但不进下一次上下文；实际 usage 缺失时 UI 显示未知。
6. 输入框附近的状态行、长模型名、窄窗格、滚动历史和浮层不重叠；现有 `@`、Markdown、粘贴和终端切换不回归。TUI 人工交互和外观由用户运行 `cargo run` 确认。

详细执行顺序与验证命令见 [`task.md`](task.md)。

## 13. 实施结果与偏差（2026-10-17）

已按本文实施并合入 `dev`：merge commit `59157e5`（父提交 `21e7a0d` dev 侧 menu 复制路径 +
`9d15774` 分支末梢）。验证：`cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`
干净，`cargo test --locked` **1910 通过 / 0 失败**（全部走本地 mock 与隔离 HOME，未打真实供应商）。
`dev` 尚**未推送**到 `lx-tui/dev`（领先 39 个提交）。

### 13.1 与本文不同之处（以落地为准）

| 主题 | 本文 | 落地 |
| --- | --- | --- |
| schema 版本 | v7 建五表 | **v8**：v7 建五表，v8 给 `agent_message_part` 加 `started_at_ms`/`ended_at_ms`（思考耗时 `Thought · 6.3s` 需要，历史恢复同样显示）；现有库 `user_version = 8`，五表齐全。 |
| 状态区内容 | `模型名 · 思考等级 · 42k / 128k`（§9、§10.2） | 实际为 `模型名 · 供应商名 · context:258.7k(26%)`：**未显示思考等级**（等级只在 `/model` 面板可见），用量为「一位小数 + 四舍五入百分比」。 |
| 缺 usage / 上限 | 显示 `-- / 128k`、`42k / --` 占位（§10.2、任务书 5.1） | **整段隐藏**：未知上限、未知用量或零消耗都不显示上下文块。 |
| 停止交互 | 「显式控件/动作」（§10）、任务书 5.1「停止控件可用」 | 无停止按钮：流中 `Esc` 单击待命（转轮行提示 `Press esc again to stop`，5 秒内再按才停止，超时撤销待命）。 |
| 失败/中断展示 | 「错误/中断标记不改变相邻组件尺寸」（§10） | 顶部方括号标记已删除，改为**消息底部通知组件**：`! Failed`（红）/ `! Interrupted`（黄）+ `└─ 一句话原因`，位于统计行上方、与上方正文空一行；标题是固定枚举，原因必填。 |
| 历史滚动 | 「用户主动上滚后暂停跟随，提供回到底部动作」（§10） | 另加**置顶用户消息行**（对齐 codex `prompt_header`）：上滚时历史区第 0 行常驻「当前顶行所属那一轮的用户消息」，点击跳回该 prompt；跨帧抑制记忆防抖。 |
| 消息内排版 | — | assistant 消息内各块之间空一行；本轮统计行整行次要信息色（含模型名）；状态行的模型名用入口标签同款粉→蓝渐变。 |
| markdown | 复用 `ui::markdown` / 画廊样式（§10） | 阅读渲染独立成 `src/ui/markdown_render.rs`（从画廊迁出并扩展：表格按宽度收缩、列表续行悬挂缩进、三级及更深嵌套不再被误判为缩进代码块）。 |

### 13.2 仍未做 / 未验证

- 状态区未显示思考等级；`limit` 未知时没有 `42k / --` 部分占位（与 §9 的「输入框附近展示等级」有差）。
- `agent_tool_call`、`agent_subagent` 只有表结构、无写入流程（与本文一致，此处重申）；模型返回工具调用时本轮以「不支持」错误结束。
- 真实供应商与真实网络的端到端验收**未由 Agent 执行**：五种 transport 只有本地 mock 覆盖；上下文溢出收缩重试（`src/chat/context`）在真实供应商下未复现。
- 设置页的浏览器人工验收（供应商/模型 CRUD、远端获取、Key 遮蔽、409 revision）仍待用户执行。
- 旧库升级、跨窗格隔离、重启恢复等以自动测试覆盖为准，真实 TUI 观感由用户 `cargo run` 确认。

### 13.3 合并与清理

- `dev = 59157e5`；工作区 `.worktrees/lx-chat` 已移除，分支 `feature/lx-chat` 已删除（末梢 `9d15774`）。
- 要整体回退用 `git revert -m 1 59157e5`（或重置到 `21e7a0d`）；回退前先确认没有基于该分支的新提交。
