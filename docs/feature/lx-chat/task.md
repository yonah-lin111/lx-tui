# lx 页流式对话与模型设置：执行任务（已完成）

- 状态：**全部实施完成并合入 `dev`**（merge commit `59157e5`，2026-10-17）；`feature/lx-chat` 分支与工作区已清理。**落地偏差见 `design.md` 第 13 节**。
- 验证结论：`cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings` 干净；`cargo test --locked` **1910 通过 / 0 失败**（本地 mock + 隔离 HOME，未打真实供应商）。`dev` 尚未推送远端。
- 实施原则（沿用）：按影响范围拆步提交可验证的代码改动；所有 IO 在事件层/后台服务，`app/` 纯状态与更新，`ui/` 纯渲染。生产代码无 `unwrap()`/`expect()`。
- 勾选口径：`[x]` = 合并后的 `dev` 代码与自动测试已覆盖；少数与设计基线不同的条目保留 `[ ]` 并注明差异。

## 0. 开工前核对

- [x] 记录用户对两份文档的审核意见，将修改后的边界写回设计，再实施。
- [x] 只读检查工作区变动；用户已有改动原样保留，不修改分支、索引、暂存区或提交历史。
- [x] 复查四份规范：`docs/standards/project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`。
- [x] 用 codegraph + codebase memory 重新确认待改符号和受影响调用链；改动后只针对索引未覆盖或已过期片段直接读源。
- [x] 明确真实 lx-agent 源配置仅作只读导入；所有开发测试以临时 HOME / 独立路径和假密钥运行。
- [x] 确定依赖的最小集合与锁文件变更：结构化 JSON 用 `serde`/`serde_json`；HTTPS 流请求用 `reqwest`（`rustls`，按实际接口启用最少 feature）；只有在代码确需时加入取消/SSE 辅助依赖。优先查询 Context7 的当前 API 文档。

## 1. SQLite v7：结构、迁移和 CRUD

交付边界：`src/db/mod.rs` 只负责版本迁移和连接，聊天 CRUD 放 `src/db/` 的职责文件；不要把 SQL、HTTP 与 TUI 状态塞进同一个模块。先实现迁移测试，再实现写入事务。正式 DDL 以 `design.md` 第 5.2 节为审核基线。

### 1.1 schema

- [x] 在 `src/db/` 按现有 `SCHEMA_VERSION` 规则新增 v7 迁移，建 `agent_session`、`agent_message`、`agent_message_part`、`agent_tool_call`、`agent_subagent`，保持 v1-v6 迁移原样。
- [x] 每表用自增整数内部 ID + 唯一稳定 external ID；新增外键引用 external ID；定义 role/status/kind 约束、唯一顺序约束和最少必要索引。
- [x] `agent_session.workspace_id` 引用既有 workspace；消息/内容块/工具/子 Agent 关系使用级联；子 Agent 关系的 `child_session_id` 唯一并拒绝自指。
- [x] 无需用户数据回填。v6→v7 在事务内完成；失败时 user_version 和旧数据不变；新库、重复打开结果一致。
- [x] 给新增 schema 和记录类型写公共文档注释，避免把 Codex 的 rollout 文件路径字段照抄到本项目。

### 1.2 持久化服务

- [x] 定义会话列表分页、加载单会话及有序消息/parts、新建首轮、增量 checkpoint、完成/中断/失败终态、重命名、删除 API；每个按 workspace + session 双条件校验，跨工作区读写返回拒绝或空结果。
- [x] 首轮 `session + user + assistant(streaming)` 同事务。后续每轮 `user + assistant(streaming)` 同事务，生成 request generation/sequence。空草稿不写库。
- [x] 流中 checkpoint 合并相邻 part；内容类型切换时新建 part。终态更新内容、usage、结束原因和 session 最近活动时间；聚合 token 只计已返回的 provider usage，未知不写 0 冒充真实值。
- [x] 异常重启时仅把残留 `streaming` 标为 `interrupted`，保留最后 checkpoint，不自动重发或生成第二条用户消息。
- [x] 删除有在途生成时先取消、等待最终 DB 写入顺序，再执行事务级删除；过期 generation 事件不能在删除后重建历史。
- [x] 聊天写盘运行于受控后台 worker，事件循环只收结果；worker 串行/有界队列，避免高频 delta 阻塞 TUI。现有 `Persistence` 的提示词和信任服务行为不改变。

### 1.3 精确测试

- [x] `src/db/tests.rs` 覆盖空库创建、v6 升级、幂等、外键检查、级联、非法 version、事务回滚。
- [x] 新增聊天 DB 测试：同工作区分页顺序、跨工作区拒绝、同 session sequence、交错 text/reasoning 顺序、checkpoint 覆盖、崩溃恢复、重命名/删除及孤儿记录为零。
- [x] 针对 `db` 模块先执行 `cargo test db::`；再按编译结果修复直接受影响测试。

### 1.4 关键提交次序

1. 在 `Database::migrate` 的既有 v1-v6 路径后追加 v7 事务；v6 旧数据快照升级前后逐字段比对，`PRAGMA user_version` 只在建表和索引成功后推进。
2. `begin_turn` 参数含 workspace、可选已有 session、冻结的 user 文本及 `ModelSnapshot`；写事务一次产生 session（首轮）、user、user text part、streaming assistant。返回两条 message ID；失败时返回错误且不产生半轮。
3. `checkpoint` 接收完整 parts 快照与 assistant ID；用 `status='streaming'` 约束更新，按 position upsert/替换。`finish` 在同一事务写最后 parts、状态、usage 和 session 时间；重复 finish 不能重复累计 token。
4. 删除先由事件层取消并等待 finish，再由存储层校验 `workspace_id + session_id` 后级联删除。空白草稿只清内存，无 DB 行。

验收样例：连续两轮的 `sequence` 为 `0,1,2,3`，`turn_index` 为 `0,0,1,1`；第一轮停止后仍有四条 message 行，但组装第二轮上下文时跳过第一轮未完成的 assistant 及其 user。此样例专门检验“保留 partial”和“只用完整轮次”同时成立。

## 2. 模型配置、导入与设置服务

交付边界：磁盘类型与 Web 脱敏 DTO 分离。`settings.json` 不承载模型配置；`ai.json` 是唯一写入目标。设置服务沿用现有 token/Origin/静态资源机制，模型发现是独立异步请求。

### 2.1 文件配置

- [x] 在 `src/config/` 增加独立模型配置类型、验证和路径函数，开发路径 `~/.lx-t-dev/config/ai.json`、生产路径 `~/.lx-t/config/ai.json`；现有布局 `settings.json` 契约不改。
- [x] 解析 lx-agent provider/model/selection 字段；保留模型级 transport、limit、modalities、variants、variant、pricing；未知字段往返保留，所有模型默认选择必须指向存在且已启用的 provider/model。
- [x] 校验 provider type 和模型级 transport；base URL 限制合法方案与主机，剔除不合法控制字符；API Key 只在内存/权限受控文件中保存。
- [x] 使用同目录临时文件、flush 与原子 rename；Unix 临时文件和目标文件 `0600`、目录 `0700`，OS 专属权限代码只放 `src/platform/`；失败不破坏旧文件。读取非法文件给显式错误，不能静默覆盖。
- [x] 首次目标文件不存在时只读导入 lx-agent `ai.json`，必要时读取 `builtin-providers.json` 补充有效预置项；目标存在即跳过。缺源走空配置，损坏源保持可重试；不导入无关设置。
- [x] 单独测试首次、重复、目标已存在、缺源、源损坏、默认选择无效、权限与原子保存；只用临时目录和假 Key。

### 2.2 HTTP 服务

- [x] 为 `src/settings/service/` 定义模型设置 GET/PUT 和远端模型发现 POST；保留现有 token、Origin、回环监听、请求体上限。
- [x] GET 仅返回 `hasApiKey`，不返回明文；PUT 支持保留原 Key 和显式清除，并对完整配置做原子验证/保存；保存后发 `SettingsEvent::ModelsChanged` 给事件循环。
- [x] 远端列表获取在后台异步执行，不能堵住本地 HTTP 连接或 TUI；使用当前编辑中的 base URL/key 进行一次性请求，解析并排序 `{data:[{id,owned_by}]}`。规定超时、最大响应尺寸、无跨主机重定向、状态码与脱敏错误。
- [x] 验证设置页修改时在途聊天请求继续使用启动时的配置快照，下一轮才用新配置；失效模型的当前选择回退到有效默认且给可见反馈。
- [x] 服务端测试覆盖未带 token、跨源写入、超大请求、明文 Key 不出 GET、坏配置 422 且文件不改、获取模型成功/失败/超时/重复 ID。

### 2.3 浏览器设置页

- [x] 仅在 `assets/settings.html`、`.css`、`.js` 扩充 Models 视图，保持三文件分离、零构建、无外部静态资源。
- [x] 供应商列表：添加、选择、编辑名称/type/base URL/API Key、启用/停用、复制、删除；不可用动作有明确禁用态和错误反馈。
- [x] 模型列表：手工添加、编辑 id/name/transport/limit/variants/variant/pricing、复制、删除；从远端获取候选后明确选择加入，不覆盖现有编辑；全局默认模型可设置。
- [x] 创建/复制供应商和模型生成不同配置 ID；删除默认或禁用默认 provider 之前解决新的有效默认；保存成功后页面状态与 TUI 同步。
- [x] 动态数据只用 `textContent` 等安全 DOM API；Key 输入框默认遮蔽，后端不回传密钥。小屏不溢出、操作按钮可辨且有文字/tooltip。
- [x] 对应 `src/settings/service/tests.rs` 分别请求 HTML/CSS/JS 验证路由、静态资源白名单和关键交互入口；浏览器手动验证留给用户审核阶段。

### 2.4 设置页交互验收脚本

| 操作 | 预期状态与文件结果 |
| --- | --- |
| 添加 provider，修改显示名 | 配置 key `id` 保持稳定，会话引用不因 `name` 改动失效。 |
| 复制 provider | 新 ID、新对象；GET 不泄露旧 Key，复制项默认需重新输入 Key。 |
| 添加/复制模型 | 模型 ID 唯一；复制后修改 limit/variant 不反向改变原模型。 |
| 获取远端模型 | 候选先展示并可选择添加；现有模型编辑内容不被请求结果覆盖。 |
| 两个浏览器页同时编辑 | 后保存的旧 revision 收到 409，文件保持先保存版本。 |
| 留空/替换/清除 Key | 留空保持旧 Key，非空替换，显式清除删除；GET 始终只返回 `hasApiKey`。 |
| 删除当前默认模型或禁用 provider | 服务端 422 或要求先选有效替代；TUI 的下一轮选择不得悬空。 |
| 保存中断或磁盘写失败 | 旧文件仍可解析，revision 不推进，`ModelsChanged` 不广播。 |

实现时用假 Key 的本地测试端点检查 URL 与 header；不能在浏览器控制台、HTTP 错误正文或 tracing 中输出 Key。对 Anthropic/Google 发现能力，以实际官方接口与 lx-agent 行为确定可用端点；不存在可用端点时明确禁用“获取模型”，手工添加仍可用，不伪造列表。

## 3. 五种协议的流式请求与 thinking

交付边界：公共网络入口只接收 `ModelSnapshot + ordered context + cancellation`，输出带 request key 的纯数据事件。每种 transport 分别封装请求 JSON、URL/header、事件解码；SSE 字节帧解析器共用，不能把 Google/Anthropic payload 当成 OpenAI delta。

### 3.1 纯数据事件与解析器

- [x] 定义统一 `ChatEvent`：开始、text delta、reasoning delta、reasoning metadata、usage、完成、失败、取消。事件携带 session external ID、request generation、part 顺序，网络模块不直接修改 `AppState`。
- [x] 编写可增量喂入的 SSE 帧解析器；覆盖 chunk 切在 UTF-8 字符中、多行 data、CRLF、空心跳、`[DONE]`、无完整终止帧、超大帧和 JSON 错误。
- [x] 单独实现 transport 适配：OpenAI Chat、OpenAI-compatible Chat、OpenAI Responses、Anthropic Messages、Google Gemini。只暴露公共事件类型给 `event/`；模型级 transport 覆盖 provider type。
- [x] variants 按各协议映射思考参数；没有等级配置不发送伪参数。provider 的 URL 组合有专门测试，避免重复 `/v1`、丢模型 ID 或把 Key 发到重定向目标。
- [x] 从协议原生 reasoning 事件提取 thinking；对正文中的 `<think>`、`<thinking>` 用跨 chunk 状态机拆分。Anthropic `signature_delta` 与对应 part 绑定并可恢复。
- [x] 解析结束时准确提取 usage；没有 provider usage 保持 `None`。错误/停止保留已收到 delta。服务返回 tool call 时标记不支持，不执行工具。
- [x] 请求支持显式取消、连接超时与流空闲超时；关闭窗格/工作区及 TUI 退出均取消归属请求。

### 3.2 上下文

- [x] 只从 DB/内存的完整轮次构建历史；停止/失败 partial 仅供展示。当前用户消息永远纳入请求。
- [x] 按模型 `limit.context`/`limit.output` 和明确标记的估算选择最近完整轮次，保持轮次顺序；没有上限时不假定具体窗口。
- [x] 上下文溢出最多去最老完整轮重试一次，复用同一轮 DB 记录，UI 显示已省略旧轮；单条当前消息超限只报错，不截断用户输入。
- [x] 跨 provider/model 切换时只回传兼容消息；专有 thinking 元数据不跨 provider 传输。保存每轮实际模型和等级，恢复后仍能正确显示。

### 3.3 协议测试

- [x] 本地 mock HTTP server 逐一覆盖五种协议的 text、thinking、usage、完成、缺字段/错误字段与截断流；不依赖公网或用户真实 Key。
- [x] 测试混排 reasoning/text/reasoning、空 thinking、标签跨 chunk、Anthropic signature 独立帧、Google thought part、Responses summary/raw 事件。
- [x] 测试取消与网络中断后的 partial、状态与下次上下文；上下文超额单次重试不得重复插入 user message。
- [x] 对网络适配模块执行对应精确 `cargo test` 过滤器，不因可选公网不可用而跳过本地协议覆盖。

### 3.4 解析与请求 fixture 最小集合

| fixture | 输入切分/事件 | 断言 |
| --- | --- | --- |
| SSE 字节边界 | 中文 UTF-8 分成多个网络 chunk；`data:` 分行、CRLF、空心跳。 | 不丢字、不重字，仅完整帧进入 JSON 解析。 |
| 交错内容 | text → 原生 reasoning → text；标签 `<thi` + `nk>` 跨 chunk。 | part position 严格递增，正文不残留标签。 |
| Anthropic 签名 | thinking delta 后单独 signature delta。 | 签名附原 part metadata，恢复仍能读到；空文本帧不生成空 part。 |
| Responses / Google | summary/raw reasoning 事件、Google `thought` part。 | thinking 与正文隔离，来源 metadata 可区分。 |
| usage 缺失 | 正常完成但未返回 token 数。 | DB 为 NULL，状态行显示 `--`，不写估算数。 |
| 异常终止 | HTTP 错、SSE 错 JSON、半帧 EOF、空闲超时、用户取消。 | 每次恰好一个终态；已收 partial 可查看，后续上下文跳过。 |
| 意外工具调用 | 任意 provider 返回 tool call/function call。 | 显示不支持并结束，DB 工具表仍为零行，无系统动作。 |

请求 fixture 还要逐协议断言：OpenAI Chat 用 `messages`，Responses 用 `input`，Anthropic 有 `max_tokens` 和协议 header，Google 用 `contents`；模型级 `openai-responses` 覆盖 provider type。不存在 variant 时不得额外发送推理等级字段。mock server 记录的认证目标必须与显式配置主机相同，重定向不携密钥。

## 4. app / event：窗格独立会话状态机

交付边界：`src/app/state/lx/` 存会话、草稿、面板和展示快照，`src/app/update/` 只产生命令/更新纯状态；`src/event/` 消费 effect、管理任务和 DB worker。`AppEvent` 是后台结果回主循环的入口，`SettingsEvent` 是配置保存后的入口。

- [x] `src/app/state/lx/` 增加会话引用、历史分页/滚动、消息/part 展示数据、请求状态、当前模型选择、上下文 usage 与面板状态；仅存纯数据，不放 DB connection/HTTP handle。
- [x] `src/app/actions.rs` / `src/app/update/` 明确发送、停止、流事件、打开历史、恢复、新建、重命名、删除确认、模型/等级选择等动作。每条动作由状态机约束合法状态。
- [x] 发送前维护编辑器草稿与输入选区；DB ack 成功后才清草稿；失败仍可修改/重试。同一窗格 streaming 时 Enter 不入队、不造消息；其他窗格可独立发送。
- [x] `/` 命令面板与原粘贴、`@`、Markdown 面板互斥；`/model` 两级选择、`/resume`、`/new` 的命令文本不被发送。普通文本含 `/` 和围栏内 `/` 保持输入文本。
- [x] `src/event/` 接入 Chat DB worker、网络任务和设置事件；终端输入、DB ack、stream delta、取消、关闭窗格都经唯一事件循环收敛；按 pane + session + generation 检查过期结果。
- [x] 在生成中切工作区/标签/终端视图不串流；删除/关闭时先取消且晚到事件不改其他会话。恢复同一运行中的 session 有明确占用提示。
- [x] 使用现有 toast/overlay 模式告知配置错误、落库失败、网络失败、上下文省略；用户可见文案集中 `src/ui/text.rs`。
  - 补充：轮次的失败/中断不再用方括号标记，改为消息底部通知组件（`! Failed` 红 / `! Interrupted` 黄 + `└─ 原因`，见 `design.md` 13.1）。
- [x] 纯状态测试覆盖首轮/续轮、重复 Enter、两个窗格并发、模型切换只影响下一轮、切换焦点、流终态、删除竞争、无 provider、非 Native 不发送、面板优先级。

### 4.1 时序验收

```text
Enter -> Committing -> DB begin_turn ack -> 清冻结草稿 -> Streaming
                                      -> 启动 HTTP -> delta -> checkpoint
                                      -> end/cancel/error -> final DB ack -> Idle
```

`Committing` 期间重复 Enter 不再入队；DB 失败走回 Idle 且保留用户草稿。若 DB ack 前关闭窗格/选择新会话，按原 session 完成写入并立刻终止请求，不让迟到 ack 绑定到新窗格。流 delta 只更新匹配 `(pane, session, generation)` 的视图；其持久化以 assistant ID 为目标。`Finishing` 期间不允许新一轮写入同一 session；final DB ack 失败要显示“保存失败”，保留内存 partial，不能伪装成可恢复历史。

并发矩阵：A/B 两窗格同时发起不同 session 可并行；A 流中切焦点到 B 不改变 A 的归属；A 关闭时只取消 A；删除 A 当前 session 时先收到 final ack 再删除；B 不能把正在 A 生成的同一 session 作为可写会话恢复。为每一格写纯状态测试或事件层集成测试，避免只测试 happy path。

## 5. ui / input：历史、模型状态和响应式 TUI

交付边界：`src/ui/main_content/lx/` 拆历史、状态行、命令面板的纯渲染文件；已有 `src/ui/main_content/lx.rs` 保持整体布局入口。`src/input/` 把鼠标与键盘转为动作，命中矩形由同一布局函数提供。

- [x] 在 `src/ui/main_content/lx/` 按职责新建聊天历史与状态行文件，不直接复用 UI 画廊的假数据；复用其视觉 token、`ui::markdown` 和公共滚动条/面板能力。
  - 补充：阅读渲染已从画廊迁出为 `src/ui/markdown_render.rs`，并新增 `main_content/lx/{grid,history,status,chat_bar,notice,slash,brand}.rs`。
- [x] 欢迎页首轮前仍显示三入口；首轮后内容区转可滚动历史，输入框与 existing 编辑规则保留。正文流增量绘制；thinking 默认折叠，点击/键盘可展开。
- [ ] 输入框周围安排实际 provider/model、variant、上下文分子/分母、停止状态、历史入口。未知 token 用 `--`；模型变更立即更新上限并清掉旧模型的用量分子。
  - **差异**：落地为 `模型名 · 供应商名 · context:用量(百分比)`；思考等级未展示（仅 `/model` 面板可见），未知用量/上限整段隐藏而非 `--` 占位；模型变更清空旧分子与上限更新已实现。
- [x] 按终端单元格宽度预留固定槽位；长名称截断，窄窗格先分行/精简状态信息，不能遮住输入文字、命令面板或上下文提示。历史用户滚动后停止自动跟底。
- [x] 历史浮层只列当前工作区，支持分页、打开、新建、重命名和有确认的删除；模型浮层支持候选筛选、禁用项解释、二级等级选择。
- [x] `src/input/` 保持 Enter、Shift/Ctrl+Enter、Esc、方向键既有优先级；新增 slash/历史/思考展开/停止的键鼠路由。终端视图按键编码路径不改变。
- [x] `ui/` 只接收 `&AppState`，命中几何与绘制共享纯函数；不在 render 中读取数据库、配置文件或执行网络请求。
- [x] buffer/geometry 测试覆盖宽/窄/极矮、中文和宽字符、长模型名、折叠 thinking、在途流、用量未知、历史空态/溢出/滚动、浮层遮挡；既有 lx 输入、`@`、Markdown、粘贴和终端切换测试保持通过。

### 5.1 可见状态验收

| 场景 | 必须看见或发生 |
| --- | --- |
| 新窗格无历史 | 欢迎页和空输入，模型选择可见；DB 不创建空 session。 |
| 正在生成 | 正文增量出现、thinking 默认折叠、停止控件可用；Enter 不重复发送。 |
| 结束且有 usage | `实际输入 token / 当前模型上限`，例如 `42k / 128k`。 |
| 没有 usage 或模型上限 | 分别显示 `-- / 128k`、`42k / --`，不显示伪 0。 |
| `/model` | 先选启用模型，再选其真实 variants；Esc 从等级退回模型。 |
| `/resume` 与历史入口 | 只见当前 workspace；重命名、确认删除、新建、恢复均能完成。 |
| 窄窗格和长模型名 | 状态区按布局换行/截断，输入光标、面板和消息无重叠。 |

人工 TUI 验证由用户执行 `cargo run`；Agent 的 buffer/geometry 测试只能证明已覆盖的渲染尺寸与状态，不能替代实际终端观感。

**与上表的差异（落地为准）**：状态区显示 `模型名 · 供应商名 · context:用量(百分比)`（如 `context:258.7k(26%)`），未显示思考等级；没有 usage 或没有上限时**整段隐藏**而不是 `-- / 128k`、`42k / --`。

## 6. 规范同步与验证门槛

- [x] 按实际模块路径更新 `docs/standards/project-directory-structure.md` 的模块职责与依赖方向；更新 `tui-design-requirements.md` 的 lx 聊天/状态行/历史/命令/降级规则。`code-writing-standards.md` 和 `html-assets.md` 仅在真实约束变化时改。
- [x] 检查新增文件的行数：单文件不超过 1000 行，职责拆分后目标约 500 行；公共 API 文档、错误语义齐全；生产代码不出现 `unwrap()`/`expect()`、`println!`/`eprintln!`。
- [x] 按受影响范围先跑 DB、config、settings、stream、app、input、ui、event 的精确测试；针对失败定位，不做无目的反复全量运行。
- [x] 完成实现后执行项目要求的 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`，记录每项结果。若修改 `src/tui/` 终端恢复路径，另须运行 `tui::tests` 顺序回归；本任务设计不要求修改恢复顺序。
- [ ] 人工验收清单交给用户：`cargo run` 中用测试供应商发流式请求、看到 thinking 展开/停止、`/model` 改等级、两个窗格隔离、重启后从当前工作区历史恢复；浏览器设置页测试供应商/模型 CRUD、远端获取列表与 Key 遮蔽。Agent 不声称已交互验证真实 TUI 外观。
  - **待用户执行**：真实供应商流式、设置页浏览器 CRUD/远端获取、真实终端观感（Agent 只覆盖了 buffer/geometry 与本地 mock）。
- [x] 最终报告明确修改内容、精确验证结果、未能验证的外部协议/真实供应商及用户需确认的交互点；不自动修改 Git 分支、暂存区或提交。
  - 合入与清理由用户明确授权后执行：merge commit `59157e5`，删除工作区与 `feature/lx-chat` 分支（未推送远端）。

## 7. 完成定义

以下条件全部成立才算完成实现（**核对结果：全部成立，自动测试 1910 通过 / 0 失败；真实供应商与设置页浏览器验收待用户执行**）：五种协议均有真实流式 mock 覆盖；thinking 与正文独立且顺序正确；会话可持久化/恢复/删除且工作区与窗格隔离；`/model` 真正控制下一轮请求；输入框附近的模型、等级和实际上下文用量正确；设置页能维护/复制/发现供应商模型；一次性导入既有配置且不回写 lx-agent；旧库迁移和现有编辑/终端路径通过精确回归；项目规定的 fmt/clippy/test 完成。任何缺项都在最终报告列为未完成，不用占位 UI 或空表声称功能完成。
