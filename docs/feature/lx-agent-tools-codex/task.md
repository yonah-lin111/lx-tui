# Codex 内置 Agent Tool 补齐执行任务

状态：已完成（2026-10-29 全部任务、测试、代码评审修复与验证均已完成）。设计依据：同目录 `design.md`，验证与评审记录见 `verification.md` 与 `review.md`。

## 0. 开工门槛

- [x] 用户审核并明确确认 `design.md` / `task.md`；若审核要求改动，先更新文档，再实施。
- [x] 重读当前 `AGENTS.md` 与四份 `docs/standards/` 规范，核对未提交改动；不得修改 Git 状态。
- [x] 再查 `src/db/mod.rs::SCHEMA_VERSION` 和 `src/db/migration.rs` 的所有新增迁移。若 v19 已被其他分支占用，顺延版本号并保留旧库升级路径。
- [x] 用 codegraph + codebase-memory 复核每个拟改关键函数的调用关系、覆盖和最新代码。Codex `core/src/tools/` 是索引排除目录，按已核对的精确路径直接读取，不对全仓进行无界搜索。

## 1. 工具契约与解析

- [x] 在 `src/chat/agent.rs` 增加 `SendInput/Resume/List/CancelInput` 对应 `SubagentOp`、结果类型与白名单解析。新增错误码统一英文；严格拒绝未知字段、空文本、空 items、`message/items` 或 `task/items` 同传。
- [x] 在 `src/chat/tools/subagent.rs` 增加四个控制工具 schema，扩 `spawn_agent` 的 `items`。在 `src/chat/tools/schema.rs` 加 `curr_time`，使主 agent 工具顺序稳定；同步 `MAIN_TOOLS`、`TOOL_NAMES`、`SUBAGENT_TOOL_NAMES`。本次新工具均只给主 agent，子角色白名单保持原权限。
- [x] 用现有四种 transport 的工具声明测试验证参数 schema、描述、历史回放和工具调用名；新增跨工具的“schema 声明与执行层校验一致”测试。
- [x] 保留原 11 个工具行为和既有 `spawn_agent.task` 调用兼容性；不加 `apply_patch` / `view_image` 别名。

## 2. 输入与图片归档

- [x] 定义有序的子 agent 输入 item 纯数据类型（text/image）；`local_image` 相对路径以父轮冻结 cwd 解析，`image` 只接收 data URL。data URL 连同工具参数 JSON 不得超过现有 1 MiB 总上限；本地文件仍用单张 20 MiB、单次最多 4 张、总量 40 MiB 限制。检查解码前字节上限、真实格式、像素和模型图像能力。
- [x] 复用 `platform::clipboard_image` 处理和归档图片；为子 agent 加严格归档路径，不采用 `attachment::insert_attachments` 的归档失败回退原路径。归档完成后记录路径、MIME、尺寸、摘要；读取时校验这些字段。
- [x] 扩充引用感知清理，包含未处理队列的图片和子会话历史；禁止按“当前请求上下文未引用”删除图片。删除父/子会话时按现有删除路径清理归档。
- [x] 用临时图片和 data URL 测试：源文件删除、格式伪装、超限、重复图片、损坏归档、重启后重放、清理不误删。测试不得依赖外网。

## 3. 数据库迁移与事务

- [x] 迁移新增 `agent_subagent_input`、`agent_subagent_input_item`、`agent_subagent_report` 表及索引；把旧 `report_json/report_delivered` 转成最多一条 legacy 报告。尽量只增表/列，不重建旧表。
- [x] 数据库 API：`enqueue_input`、`next_queued_input`、`claim_input`、`finish_input`、`cancel_input`、`owned_subagent`、`pending_reports`、`mark_reports_delivered_in_batch`。所有目标查询必须同时限制父会话与 workspace；工具调用幂等键非空且有唯一约束。
- [x] 复用已有 `begin_turn_with_attachments` 的消息/附件/文件 mutation scope 逻辑，追加子轮时仍在原 `child_session_id` 下取新 sequence/turn_index；避免复制第二套轮次 SQL。图片引用应指向已归档路径。
- [x] 测试迁移旧库、版本顺延、外键与行数；并发/重复提交只生成一条输入；DB 提交前失败不留下队列；报告批次提交失败可重试且不双投。
- [x] 在开发库副本上验证迁移 `user_version`、数据量和 `PRAGMA integrity_check`；若实施时需要整表重建，先按项目规范停进程、备份并完成副本验证，不能直接操作真实开发库。

## 4. 子 agent 运行时

- [x] 将现有 `event/chat/subagent/mod.rs::spawn` 拆出“新身份登记”与“运行已登记子轮”两层，避免在 `resume`/队列消费里复制 230 行 spawn 流程。文件拆到 `src/event/chat/subagent/` 按职责组织，单文件遵守 1000 行上限；测试同步下沉。
- [x] `spawn_agent`：输入/图像预校验，先原子写完归档文件，再以 DB 事务登记身份、输入和引用，最后启动首轮；DB 失败留下的未引用归档由清理器回收，失败不得留下运行句柄。保持 `MAX_CONCURRENT_SUBAGENTS` 的活动上限。
- [x] `send_input`：运行中普通输入持久排队；interrupt 输入先落库再取消当前轮，旧轮完全封口后调度新项；终态未 resume 则明确拒绝；任一子轮失败后暂停整个 agent 队列，必须显式 resume 才能继续。工具结果含 `submission_id`、状态和剩余队列数。
- [x] `resume_agent`：从 DB 取同父会话身份，恢复子会话模型/历史/角色和进程运行时；两次 resume 幂等，不允许同子会话两轮并发；重启不自动运行。
- [x] `list_agents`：限定父会话，列 ID/角色/标题/状态/队列数，不返回完整正文。`cancel_agent_input`：仅 queued 可转 cancelled，含图片错误的 queued 项也可取消，重复调用幂等。
- [x] `close_agent`、父会话删除、Esc 停止与 `wait_agent` 对齐多轮语义；取消当前轮必须保留已落库内容和文件操作事实，不能重做已开始的 input。
- [x] 用 mock 模型/真实临时 DB 验证普通排队、连续 interrupt、关闭与恢复、重启恢复、取消输入、错误输入、模型失败和各类竞态。

## 5. 历史、压缩与报告

- [x] 子轮启动时从 DB 装配有序 user/assistant/tool 历史和图片；按完整轮次预算裁剪，图片不与所属文本分离。新输入 item 原顺序进入各 transport。
- [x] 自动压缩摘要持久化为可重放检查点；先提交摘要再丢旧 items。恢复时 `system/environment/summary/后续消息` 顺序固定；压缩失败保留原历史，显式记录失败。
- [x] 完成/失败每轮落一条报告；取消轮只写历史不发报告。把现有 `finish_subagent` 和 `begin_report_batch` 调整成按 `report_id` 结算/投递；报告批次和 delivered 标记一个事务完成。
- [x] 验证两轮以上报告分别可见、父轮忙时报告延迟投递、崩溃重启后未投递报告只投一次、`wait_agent` 只盯当前活动轮。

## 6. UI 与提示词

- [x] `app/` 只存状态和更新，`ui/` 只读投影：Agents 列表/详情显示 queued、running、blocked、idle、completed 等状态及待处理数；消息与报告按现有时间线顺序展示。新用户可见文案集中 `src/ui/text.rs`，lx 页反馈走窗格内 toast。
- [x] 更新英文主/子 agent 提示：新工具用途、跨重启显式 resume、缺图暂停、排队取消。核对 `PROMPT_VERSION` 是否因系统提示结构/身份变化需递增，并执行现有提示预算与分段测试。
- [x] `curr_time` 走可测试的时钟入口，输出精确 UTC 格式；空对象以外参数拒绝；工具调用仍有 DB/历史记录。
- [x] 用 buffer/状态测试验证窄窗、长标题、长 ID、报告卡片与队列数不重叠；实际外观交互留给用户运行 `cargo run` 确认。

## 7. 精确验证与交付

- [x] 每完成一个阶段先跑对应模块测试（`chat::agent`、`chat::tools`、`db::chat::subagent`、`event::chat::tests::subagent`、相关 transport/UI 测试），只在具体风险要求时扩大范围。
- [x] 代码阶段结束执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`；若这些命令因环境/耗时不能完成，记录具体未验证项。
- [x] 检查生产文件行数、测试目录镜像、无生产 `unwrap/expect`、无绕过 ratatui 的 stdout、OS 代码仅在 `platform/`；核对 Git diff 只含本任务文件和用户原有改动。
- [x] 交付时逐项对照 `design.md` 第 9 节验收标准，列出验证证据、剩余风险和需要用户手动确认的 TUI 行为。未经明确授权不修改 Git 状态。
