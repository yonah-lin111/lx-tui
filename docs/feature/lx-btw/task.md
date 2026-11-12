# `/btw` 开发任务与验收计划

> 状态：**详细设计审核阶段；生产代码任务全部未开始，等待用户批准。**
>
> 设计依据：[design.md](design.md)。用户对 Q1～Q17 的确认仅确认对应产品边界，不代表批准实施。

## 1. 当前进度

- [x] 阅读项目要求的七份规范文档。
- [x] 通过 codegraph 与 codebase-memory 定位命令、任务面板、subagent 恢复、报告与 QA 删除链路。
- [x] 调研 OpenCode / Codex 参考源码，记录与需求的差异。
- [x] 通过 Context7 查询 Claude Code 官方 `/btw` 文档。
- [x] 确认 BTW 默认不回传主 agent。
- [x] 确认 BTW 与被 `@` 唤醒的问答轮次只读。
- [x] 确认主会话下按主 QA 隔离；同一 QA 的 BTW 连续追问。
- [x] 确认主上下文截止范围，不混入其他 BTW 或后续 QA。
- [x] 确认持久化与重启后继续对话。
- [x] 确认删除 QA 级联删除 BTW，文件恢复不删除 BTW。
- [x] 确认 BTW 与原 subagent 任务使用独立问答流，只引用任务记录。
- [x] 确认可主动提及同一主会话中其他 QA 的 subagent，限定引入其任务记录。
- [x] 确认同一问题可提及多个实例，分别回答并按稳定实例 ID 去重。
- [x] 确认无新 agent 提及时沿用当前 BTW 回答者集合，并提供可见目标与重置入口。
- [x] 确认执行中的 steer 补充归属原主 QA，完成后新问题建立新 QA。
- [x] 确认每次发送刷新允许范围内的已完成内容与任务状态，排除半截回复和后续主 QA。
- [x] 确认转交先回填所属主会话输入框，用户编辑后发送。
- [x] 确认本次完整支持 Native Agent 入口。
- [x] 确认关闭面板/切会话仍后台运行；主动停止、删除归属或退出应用时停止，重启不自动重发。
- [x] 确认 BTW 仅提及已有实例，不因角色提及创建新任务，失败保留草稿。
- [x] 创建本任务专属设计与任务文档。
- [x] 确认 Q1～Q17 的关键产品边界。
- [x] 补齐拟议接口、三张关联表及删除触发器、状态机与 UI 示意图。
- [ ] 用户审核 `design.md` 与 `task.md` 并明确批准实施。

## 2. 阶段 A：完成设计，不修改生产代码

### A1. 原 subagent 与 BTW 问答隔离

- [x] 确认 BTW 追问使用独立问答执行流，引用目标原任务记录。
- [x] 给出独立 recipient 启动/结算时序，覆盖运行中及已结束实例（design §10～§11）。
- [x] 明确哪些事件只进入 BTW，哪些原任务事件仍可正常报告父会话。
- [x] 区分 BTW 只读问答与主 agent 发起的原任务恢复，避免统一降级原任务的权限。
- [x] 明确 agent 目标 ID、原任务 session ID、BTW thread ID、父 QA ID 的关系。

完成条件：设计能说明一次 `@executor` 追问为什么既能利用其任务记录，又不会自动恢复写权限或把回答投递主会话。

### A2. QA、上下文与历史恢复

- [x] 确认 steer 的产品归属：同一主 QA，不另建 BTW。
- [x] 核实普通发送、steer、主 agent 自动消费报告、本地 shell 对 `turn_index` 和主用户消息 ID 的影响。
- [x] 给出稳定 QA anchor 条件及上下文范围，避免使用 `messages.last()`。
- [x] 确定同一 QA 进行中的参考内容刷新策略：每次发送时更新已完成内容和任务状态。
- [x] 在审核稿提出历史 QA 入口及 `/btw` 默认选择规则（非逐项批准）。
- [x] 在审核稿明确 `/compact`、`/fork`、`/rewind` 与恢复的处理建议。

完成条件：从任一旧 QA 打开 BTW 时，有可验证的历史截止点；不同 QA 不串话。

### A3. 输入、提及与转交

- [x] 确认跨 QA 实例提及范围及其与主历史截止点的区别。
- [x] 确认多 agent 提及各自回答及单条消息内实例去重。
- [x] 明确未指定 agent 时沿用当前 BTW 目标集合，否则使用默认助手。
- [x] 提出角色名、实例昵称、同名实例的稳定 ID 路由规则。
- [x] 提出 completed/failed/interrupted/cancelled 的可提及、可回答、可恢复矩阵。
- [x] 提出只读白名单、只读 MCP 过滤、Skill 文本和图片模态校验（design §10）。
- [x] 明确转交主 agent 的发送时机：先回填草稿，由用户后续发送。
- [x] 提出单回答转交、非覆盖追加、一次 undo、shell 草稿保护方案。
- [x] 提出无主 QA、空 `/btw`、多行问题、生成中排队的默认行为。

完成条件：每种输入都有唯一处理路径，不因模型自由判断而把直接提问发给错误目标。

### A4. 最终设计评审门

- [x] 将剩余工程选择写成完整审核方案，不保留互相冲突的候选方案。
- [x] 标明代码检索来源与 coverage 边界；codebase-memory 后续不可用如实记录，未伪造覆盖检查。
- [x] 为已有事实、用户批准结论、实现方案分别标明来源。
- [x] 补齐 source 迁移 SQL、关联表/trigger、Effect/事件、状态机和受影响文件表。
- [x] 完成交叉审查并补齐调用 ID 命名空间、stale sweep、冻结附件、稳定实例引用、每 recipient 模型和主 composer epoch 六项问题。
- [x] 完成审核稿文档与内存 SQLite 原型验证（见 §10；不等同于生产迁移测试）。
- [ ] 把实施顺序与验收案例提交用户审核。

**门禁：A4 未通过，不执行阶段 B 及之后的生产代码任务。**

## 3. 阶段 B：数据与运行契约（待批准）

### B1. 持久化

- [ ] 重新核对 `SCHEMA_VERSION` 与迁移占用；使用届时正确的递增版本。
- [ ] 按 design §8 扩展 `source='btw'`；逐列保留现有 session 数据及索引，不使用 `writable_schema` 修改数据库定义。
- [ ] 验证升级失败 rollback、外键开关恢复、行数/完整性，以及 native/subagent 原有数据保留。
- [ ] 实现主 session + 主 QA 到 BTW thread 的唯一归属。
- [ ] 保存多轮消息、目标身份和恢复所需元数据。
- [ ] 实现事务化创建/追加及错误传播，DB 提交失败时保留用户草稿。
- [ ] 认领 recipient 原子创建唯一 assistant；结算消息/账单/recipient/batch 同事务，必要时提取已有 CRUD 的事务内 helper，禁止嵌套开事务。
- [ ] 实现按主会话加载列表、按 QA 打开对话。
- [ ] 实现删除 QA / 删除 session 的级联清理。
- [ ] 通过删除 trigger 实际回收 BTW child session，不只删关联行；覆盖 workspace 级联、直接删 child 与事务 rollback。
- [ ] 普通列表/fork 的 source 过滤及 `/resume`/CLI 直接 ID 入口均拒绝把 BTW 当主会话。
- [ ] 为退出/异常遗留请求写入可恢复的准确终态。

关键验收：重复创建不产生重复线程；同名/同文本 QA 不被误合并；数据库写入失败不显示虚假的发送成功。

### B2. 上下文装配与只读工具

- [ ] 复用现有模型适配与 agent loop，不复制 transport 实现。
- [ ] 装配明确截断的主会话参考历史和当前 BTW 历史。
- [ ] 主参考从会话开头到所属 QA 结束，保留早先背景；steer/report/shell 不创建新 QA。
- [ ] 正确处理完整工具结果、未封口 tool args、半截正文及后续 compact 摘要，引用预算按完整单元裁剪。
- [ ] 同批各目标使用相同主/BTW 前缀，只额外读自身任务记录；当前批兄弟回复下一批才进入上下文。
- [ ] 实现只读工具规格与执行层双重校验。
- [ ] 引用预处理复用已有文件、目录和图片能力的已批准子集。
- [ ] 阻止原 subagent 工具权限自动扩散到 BTW。
- [ ] MCP 工具与 resources 按来源配置、禁用列表、只读能力取交集；伪造调用在执行前被拒绝。
- [ ] `list_agents` 走 BTW 只读 host 查询，原控制请求均拒绝；不伪造主 RequestKey。
- [ ] 阻止 BTW 结果进入自动父报告队列。

关键验收：伪造写工具/命令调用在实际执行前被拒绝；原主任务与其原工具面保持正常。

### B3. 生命周期与归属

- [ ] 请求冻结 owner、thread、目标 agent 与 generation。
- [ ] 冻结附件/图片对象与 skill 准备结果，排队或 pane 关闭后不回读 composer。
- [ ] 每 recipient 保存 FrozenModel，绕过 session 级 frozen/finish 清理；不同目标模型不互相覆盖。
- [ ] 多目标批次的每个回答分别绑定目标和 assistant 消息，分别结算失败、取消与完成。
- [ ] 共享 stream 消息变换改为可按明确 assistant ID 调用；BTW 阅读投影不复用单一 RequestState 接收多路 delta。
- [ ] 同线程 batch FIFO，全局 2 个 BTW recipient 槽，主 subagent 池满时仍可运行；明确队列满的保留草稿反馈。
- [ ] 排队 recipient 没有 assistant ID，开始时才创建 streaming 消息；展示按 ordinal，不按乱序完成排序。
- [ ] 同一批 tool call ID 相同时以 recipient 前缀隔离入库/展示，loop 维持供应商原始 ID 配对；Finished 重复不双计费用。
- [ ] 网络、文件 IO 与模型采样由事件层调度，不进入状态更新或渲染。
- [ ] 按 D16 实现切页/切会话/面板关闭后继续运行，以及退出应用时停止。
- [ ] 修正运行时清理条件：不可因源主会话没有可见 pane 就回收 BTW；不可把回投写入当前显示的另一个主会话。
- [ ] stale stream sweep 无条件识别 btw_runs，orphan runtime sweep 纳入活动 BTW；不把两个 sweep 混为同一检查。
- [ ] 删除所属 QA 时取消请求，并拒收晚到的 delta、tool result 和 finished。
- [ ] 明确重复事件、并发取消、提交与删除竞争时的唯一结算规则。
- [ ] host 的主 steer/shell update 请求返回空且不消费主队列，未知 BTW 事件不回落主 agent 分支。
- [ ] 原 QA anchor 存活但被删补充使快照过期时取消受影响 run，保留 BTW 历史；不扩大原 rewind 操作范围。

关键验收：旧请求不能写入新 QA、别的 pane 或重建的同名会话。

## 4. 阶段 C：入口、输入与面板（待批准）

### C1. `/btw [prompt]`

- [ ] 扩展 `SlashCommand` 的命令名、描述、候选、带空格回显与参数提示。
- [ ] 复用 `echo_prefix` / `argument_hint`；`[prompt]` 不进入编辑文本。
- [ ] 按批准规则解析完整问题，处理空参数、无 QA、多行与忙碌状态。
- [ ] `/btw` 分派早于 shell 聊天排队与 steer，防止旁路问题被注入主任务。
- [ ] 提交后打开正确主 QA 的 BTW tab，按持久化回执处理草稿。
- [ ] 同步保留命令名与相关英文文案。

### C2. `BTW` tab 与对话布局

- [ ] 在现有 `Processes` / `Agents` tab 序列中增加 `BTW`。
- [ ] 渲染 QA 来源与被提及目标，长标题按显示列降级。
- [ ] 实现当前 QA 与历史 QA 的浏览、选择和对话恢复。
- [ ] 在底部绘制独立输入框及正确硬件光标。
- [ ] 复用聊天历史 markdown、代码复制、工具折叠、滚动和选择能力。
- [ ] 统一输出区/列表/输入框/浮层的绘制和命中几何。

### C3. 输入能力复用

- [ ] 主输入与 BTW 输入使用同一编辑行为，拥有独立草稿、光标、撤销、选择和输入历史归属。
- [ ] `BtwComposer(Box<Lx>)` 只能惰性构造；`LxBtw::default` 不创建任何内层 composer，增加构造回归。
- [ ] composer view 同时提供 editor 与 rect；禁止只改几何仍读取主输入的提及状态。
- [ ] `@`、粘贴、引用预览等面板归属当前 BTW composer。
- [ ] 异步文件扫描回投携带足够的 owner/composer 标识，不串到主输入框。
- [ ] 主 composer 也使用 session ID + editor_epoch，确保同 cwd 切会话仍拒收旧扫描。
- [ ] 异步粘贴解析、图片准备、引用预览同样携带 owner 和文本 revision，不能只修扫描路径。
- [ ] 编辑器聚焦与列表聚焦使用明确键位分派，不让左右键同时移动光标和切 tab。
- [ ] 所有 BTW 反馈使用当前 lx 窗格 toast 槽。
- [ ] speaker 元信息不占用 `model_label`；行数、cache key、selection 与卡片命中同步。

## 5. 阶段 D：subagent / monitor 与显式转交（待批准）

- [ ] `@` 候选展示用户批准范围内的已结束实例，并保留状态与角色信息。
- [ ] 稳定实例 ID 优先，避免把已有实例追问误解析为新建同名角色。
- [ ] 主 agent 对已结束实例的恢复复用既有 `resume_agent` / `send_input`。
- [ ] 用户在主会话的 `@`、Agents 面板追问和 BTW 追问按各自语义分派。
- [ ] monitor 的只读问答能获取批准的状态/记录来源。
- [ ] 按 Q8 结论隔离 BTW 问答与原任务的历史、取消、排队与报告。
- [ ] 显式转交按确认的 UI/提交规则完成，并携带 BTW 来源。

## 6. 验收矩阵

| 类别 | 必须验证的场景 | 核心断言 |
| --- | --- | --- |
| 命令 | 面板采纳、手敲、删除参数、空参、Unicode、多行 | 回显与提示准确；不误发主对话 |
| QA 隔离 | 同 session 的 QA1/QA2；不同 session 同文本问题 | 草稿、消息、目标与流互不串用 |
| 连续提问 | 同一 QA 两次 `/btw` 与面板直接追问 | 续接同一个持久化线程 |
| 上下文 | 旧 QA、其他 BTW、后续主 QA、本地 shell | 仅包含批准范围的上下文 |
| agent 身份 | 同昵称实例、列表重排、已结束实例、角色名 | 发送目标稳定且符合选择 |
| 权限 | 只读问答中请求 edit/write/exec/write_stdin | 实际执行层拒绝，文件与进程不被改变 |
| 报告 | BTW 回复、原子任务正常报告、显式转交 | 只有批准的路径进入主会话 |
| 生命周期 | 取消、切 pane、切 session、重启 | 状态准确，迟到事件不串写 |
| 删除 | 删除 QA、删除末轮致 session 消失、仅恢复文件 | 按 D7 清理或保留；新 QA 不复用旧线程 |
| 并发 | 双提交、双窗格创建、提交/删除竞争 | 唯一关系、一次结算、无残留执行 |
| 布局 | 宽/窄/极矮、中文、长标题、浮层打开 | 绘制/光标/点击/滚动使用同一几何 |
| 复用回归 | Processes、Agents、主输入、`/rename` | 相关既有交互与状态规则通过回归验证 |

## 7. 精确测试与实施顺序

### 7.1 顺序和检查点

按 B1 → B2 → B3 → C1/C2/C3 → D 集成推进；生产代码批准后，可以将不互相改同一文件的工作并行给最多 3 个 agent。每阶段产物可检查，不靠最终一次全量测试发现基础归属错误。

| 阶段 | 主要文件 | 必须先锁定的验收 |
| --- | --- | --- |
| B1 | 新增 `src/db/chat/btw/`、migration | stable QA anchor、单线程关系、真实级联删除、source 隔离 |
| B2 | 新增 `src/chat/btw.rs`、context、schema | QA 截止范围、readonly 实际执行闸门、多目标上下文 |
| B3 | 新增 `src/event/chat/btw/` | 每个 recipient 的 ACK/流/结算、取消与后台存活 |
| C | app 状态/update、lx task/panels/history | 独立 composer、非穿透 UI、命令忙时正确分派 |
| D | mention/subagent/snapshot、转交 | 终态实例可复用、独立 BTW 不产生原任务报告、非覆盖回填 |

### 7.2 新增测试组织

新测试跟随真实模块，可采用以下职责分组（按实际新增文件调整，不预建空模块）：

- `src/db/chat/btw/tests/{qa,storage,delete,recovery}.rs`：anchor/filter/去重/迁移/trigger；fixture 放同层 `test_support`。
- `src/chat/btw/tests/{context,capabilities}.rs`：前缀历史/多目标/compact/预算/恶意工具调用。
- `src/event/chat/btw/tests/{submit,routing,settle,lifecycle}.rs`：mock stream + host ACK + DB 结果联动。
- `src/app/state/lx/btw/tests.rs`：惰性构造、独立草稿、默认回答者。
- `src/app/update/chat/btw/tests.rs`、`src/app/update/pane/btw/tests.rs`：命令、输入、目标与转交。
- `src/ui/main_content/lx/task/btw/tests.rs`：宽窄矮布局、speaker、浮层、滚动与光标。
- event/mouse 新用例跟随 press/wheel/drag 测试模块，不追加到巨型父测试文件。

关键场景建议名：

```text
btw_owner_uses_stable_anchor_after_rewind_and_resend
btw_qa_keeps_steer_without_promoting_report_or_shell
btw_context_keeps_earlier_main_background_but_excludes_future_qa
btw_old_qa_ignores_later_compact_summary
btw_duplicate_targets_create_one_recipient_each
btw_sibling_call_ids_do_not_collide
btw_same_batch_uses_shared_prefix_without_sibling_partial_answers
btw_never_resumes_source_agent_or_creates_parent_report
btw_readonly_surface_rejects_command_and_mutating_mcp
btw_survives_panel_close_session_switch_and_pane_removal
btw_delete_anchor_cleans_child_session_and_rejects_late_finish
btw_rewind_steer_keeps_anchor_and_existing_thread
btw_restart_interrupts_pending_work_without_resending
btw_scan_and_paste_results_cannot_land_in_another_qa_composer
btw_composer_constructs_lazily_without_recursive_initialization
btw_transfer_preserves_main_draft_and_is_one_undo
```

### 7.3 既有测试回归

调研到的相关测试入口如下，实施时以实际模块布局定位运行：

- `src/app/state/lx/task/tests.rs`：tab 轮转与 label 覆盖；新增第三个 tab 后保留原索引。
- `src/app/state/lx/slash/tests.rs`、UI `slash_panels`：rename ghost hint 与命令采纳。
- `src/event/tests/lx_ask_task.rs`：task 隐藏主输入浮层、选区复制不穿透；仅 BTW 显示自己的浮层。
- `src/event/chat/tests/subagent.rs`：`child_delta_never_reaches_the_parent_conversation`、`child_tool_handshake_works_after_the_pane_is_gone`、`reap_orphan_runtimes_keeps_running_children`。
- `src/db/chat/rewind/tests.rs`：`rewind_deletes_latest_turn_and_persists_one_notice`、`rewinding_only_turn_removes_session_and_related_data`。
- `src/db/chat/compact/tests.rs`：检查点提交/撤销和 stale revision。
- agent loop 的 `tools_outside_the_role_surface_are_never_executed`；grep 定位具体 tests 子模块后精确运行。

## 8. 验证方式与完成标准

- 文档阶段：检查路径、Markdown 结构、决策一致性和未确认项，避免把草案写成已批准实现。
- 实现阶段：优先执行受影响模块的 Rust 测试；测试文件按生产职责拆分。
- 执行格式化与适当范围的静态检查；最终按项目要求执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`，不在无新增变更时反复跑全量。
- 数据迁移验证旧库升级、归属唯一约束、级联删除和 `PRAGMA integrity_check`；需要整表重建时遵守库副本验证与备份规则。
- TUI 外观与真实交互由用户 `cargo run` 验证；交付时列出具体操作步骤。
- 完成后更新本文件勾选项、实际执行的验证命令与结果，禁止提前标记通过。

## 9. 用户人工验收步骤（实施后）

1. Native 主会话提出 QA1，生成期间输入 `/btw `：看到 `[prompt]`，输入问题后打开 BTW，主任务继续。
2. 在 BTW 追问两次，引用文件/图片/skill；确认输入、粘贴、撤销、选区与主输入一致。
3. `@monitor @explorer` 同问，看到两个清晰身份的回复；再不写 `@` 追问，仍发给这两个；Reset 后由 lx 回答。
4. 添加主任务 steer，BTW 仍归 QA1；主任务完成后提交 QA2，BTW 独立；从 QA 列表回 QA1 能续聊。
5. 在 QA1 的 BTW 主动选择 QA2 的实例，能读取其记录，但不会出现 QA2 的整个主对话或 BTW。
6. 对 completed/cancelled 实例分别在 BTW 询问、在 Agents 面板恢复任务，确认前者不改原状态，后者真实恢复。
7. BTW 正在回答时关闭面板、切主会话，再返回看到结果；退出后重启只恢复内容，不重复发送。
8. 在主输入先写未发送草稿，再转交一条 BTW 回复，确认草稿保留、来源清晰、一次 undo 恢复、无自动发送。
9. 仅恢复文件时 BTW 保留；删除对应主 QA 后 BTW 消失，其他 QA 不受影响；重发相同文字建立新归属。
10. 在窄/矮 pane 下测试三个区域、@浮层、滚轮与代码复制，确认不会点击到被覆盖的主历史。

## 10. 本轮实际变更记录

- 仅新增 `docs/feature/lx-btw/design.md` 与本文件，记录调研、已确认边界和待审核任务。
- 审核稿文档检查通过：D1～D17 顺序完整，章节连续，已有源码路径和相对链接有效，代码围栏闭合，无行尾空白/替换字符。标为新增的 BTW 模块路径单独识别为计划路径。
- 内存 SQLite 原型验证通过：从文档提取迁移/关联表/trigger SQL，验证原 session/message 保留、迁移失败 rollback、删除 thread/anchor/child/parent/workspace 五条路径、删除事务 rollback、QA anchor 过滤、recipient 去重、`foreign_key_check` 与 `integrity_check`。
- recipient 增加排队模型快照后重跑 SQL 原型，验证 queued 可无 assistant ID、必须有模型来源，以及既有删除/归属约束仍通过。
- 原型使用简化消息表与内存数据库；验证的是设计 SQL 及级联方向，不替代实施阶段的真实 schema 升级测试。没有打开或迁移开发数据库。
- 本轮未运行 Cargo 测试：变更仅为 Markdown 文档。
- 交叉审查发现的六项阻塞均已在设计中给出明确修订；其中 call ID/稳定引用已在审查并行期间补齐，其余附件、模型、sweep 与 editor_epoch 随后补齐。生产实现仍待用户批准。
- 生产代码任务未开始。
- 工作区原有未跟踪目录 `docs/feature/lx-scroll-parity/` 保留。

## 11. 本轮落地记录（feat/lx-btw）

> 分支：`feat/lx-btw`（worktree `.worktrees/lx-btw`）。生产代码已按本文件阶段 B1/B2/B3/C/D 的收敛方案落地最小可用版本，未合并到 `dev`。

### 11.1 已实现

- **DB v28**：`agent_session.source` 增加 `btw`；新增 `agent_btw_thread` / `agent_btw_batch` / `agent_btw_recipient` 与 `trg_btw_thread_delete_child`。迁移先移除同名触发器再重建会话表，兼容“从更高版本回退”的合成旧库。
- **QA resolver**：`resolve_qa_anchor` / `latest_qa_anchor` / `qa_anchors` / `load_qa_context`；排除 steer、自动报告与 shell，主参考从会话开头取到所属 QA 结束。
- **持久化 API**：`accept_btw_batch`（`submit_id` 幂等、目标去重、提交时冻结模型）、`start_btw_recipient`（原子认领并创建 streaming assistant）、`finish_btw_recipient`（消息/账单 + 回答者/批次聚合，幂等）、`delete_btw_thread`、`recover_btw_startup`。
- **只读工具面**：`ToolSurface::Btw = read/search/curr_time`；`turn_tools_for_btw` 额外注入 `read_only_hint=true` 的 MCP 工具与 resources。执行层仍以冻结的工具白名单为唯一权威。
- **独立上下文**：`chat::btw::assemble_btw_context` 生成 `<btw-context>`（main-reference / agent-reference / btw-history），当前问题作为最后一条 user 消息；后续主 QA 不进入上下文。
- **事件编排**：`BtwStreams`（runs/pending/active_batches/sessions），同一线程批次串行、批内目标并行、全局 2 个回答槽；BTW 网络事件、工具握手、stale sweep、orphan runtime sweep 全部分流。回答完成走 `finish_with_optional_parts`，面板关闭时不会用空 parts 覆盖 checkpoint 正文；不写 `agent_subagent`、不产生父报告。
- **命令与状态**：`/btw` 加入 `/` 面板（别名解析拒绝 `/btwfoo`，支持多行 prompt），`/btw` 在队列/steer 之前拦截；`LxBtw` 按 QA 惰性创建独立 composer（`Lx::composer()`，禁用 shell 前缀），切 QA/切会话/`/new`/删除会话清理或保留的边界按设计实现。
- **UI**：后台任务页新增第三个 tab `BTW`；对话区复用 `HistorySource`，底部为 QA 列表 + composer；`input_rect`/`input_geom`/`input_scrollbar` 在 BTW 页回退到 composer，`@`/paste/refer/md 面板读取 composer 状态并锚定 composer 几何；文本框粘贴已路由到 composer。
- **隔离**：`load_chat_session_into_pane` 与 CLI `session show` 拒绝 `source='btw'`；普通列表/fork 原有 `source='native'` 过滤保持。
- **测试**：新增 10 个 DB/上下文测试、1 个只读工具面测试、4 个状态测试；`cargo test --lib` 全量 5238 通过；`cargo clippy --all-targets -- -D warnings` 与 `cargo fmt` 通过。

### 11.2 本轮未实现（明确遗留）

- `Send to main` 转交按钮/键位（设计 §12）未实现。
- BTW 页签的鼠标拖拽选区、滚轮分区、QA 行点击尚未接入（键位与文本编辑可用）。
- 对已结束 subagent 的 `@` 依赖 `ListSubagents` 已加载实例；未加载时提示无匹配。`list_agents` 控制工具未暴露（状态以只读参考注入）。
- 异步图片/剪贴板图片与引用预览的 owner 校验未覆盖（文本路径已路由）。
- `@MCP` 只读工具已声明；资源与工具调用依赖现有 MCP 快照，未新增独立只读资源桥。
- 未在真实开发库上执行迁移；仅用内存/临时库测试。

### 11.3 验证命令

```text
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

## 12. 修复记录（2026-11-12）

### 12.1 `/btw` 永久排队、回答无输出

- **根因**：旁路任务结束后只回投了 `BtwEvent::RecipientFinished`，没有回投 `ChatNetEvent::Finished`；`btw::finish_run` 永远不执行，assistant 停在 `streaming`，回答者/批次停在 `running`，同线程后续提交全部卡在 `queued`。
- **修复**：
  - 任务结束按主子 agent 同一协议回投 `ChatNetEvent::Finished`（`budget_exceeded`/`usage`/`billing` 全部透传），由 `handle_net_event → finish_run` 统一结算。
  - 内层 `tokio::spawn` 捕获 panic 并合成 `Failed` 终态，避免线程悄悄死掉。
  - `start_btw_recipient` 拿不到运行实例时立即结算失败；认领失败时收敛批次状态。
  - 结算改为 `finish_with_optional_parts`：面板关闭后 `parts=None`，绝不用空数组覆盖 checkpoint 正文。
- **数据库证据**（修复前）：`agent_btw_batch` 一条 `running` + 一条 `queued`，`agent_btw_recipient` 停在 `running`，assistant `status=streaming` 且无 parts；日志显示请求 `reason="complete" deltas=177` 已完成。恢复后两条均为 `interrupted`。

### 12.2 旁路消息不能折叠/展开

- `content_geom` 增加 BTW 分支（与 Agents 详情同源量行），新增 `task_btw_card_target_at` 与 `press_btw_content_hit`。
- 鼠标按下记录 `lx_card_press`，松开时 `TaskTab::Btw` 走 `update::toggle_btw_card`：思考块、工具卡片、用户消息折叠、粘贴芯片全部复用 `LxChat` 的展开状态；展开前先锚定视口，避免跳回顶部。

### 12.3 输入框不能 `@` 文件、默认不是一行

- composer 的 `@` 扫描根/项目根与 subagent 实例集合改由主编辑器注入（`sync_btw_composer_context` / `sync_btw_composer_agents`）；`mention_runs_external` 阻止 composer 用空 `agents` 覆盖注入项。
- 旁路 `@` 候选不过滤 `Active`，已结束 subagent 也能作为目标。
- 鼠标 hover/点选/滚轮/切 Tab 全部经 `with_active_editor` 路由到 composer。
- 输入框默认 1 行（3 行含边框），随软换行增长到 3 行；面板高度随之在 6..=8 行之间变化，不再固定留白。

### 12.4 顶部 QA 列表

- 删除多行 QA 列表与切换焦点：它只显示主问答，却看起来像可切换的会话列表，且会与当前线程归属不一致。
- 底部改为一行归属提示：`QA <序号> · <主问题标题>`，右侧 `To: <回答者集合>`；`/btw` 始终挂到最新主 QA，旧线程仍持久化。

### 12.5 验证

```text
LX_TUI_DB_PATH=<worktree-db> lx-tui session show <main-session>   # 主会话 6 条消息可读
LX_TUI_DB_PATH=<worktree-db> lx-tui session show <btw-child>      # 不在列表/被拒绝，确证隔离
sqlite3 <worktree-db> "select ... from agent_btw_batch/recipient" # 修复前卡死状态
cargo test --lib                                                  # 5240 passed
cargo clippy --all-targets -- -D warnings
```

### 12.6 输入框整体不可见（2026-11-12 补充）

- **根因**：`task::draw_list` 已把 `list_rect`（顶边框 + tab 行之下）传给 `btw::draw_bottom`，后者又减了一次 `TOP_BORDER + TAB_ROW`，内容区只剩 2 行，`avail < 3` 恒成立 → composer 永远返回 `None`。表现就是只剩归属行，键入内容落在不可见输入框里。
- **修复**：`draw_bottom` 直接使用传入的 list 矩形；`composer_rect`（外层入口）仍负责扣一次。
- **回归测试**：`btw_bottom_keeps_the_composer_visible_for_a_one_line_draft` 断言一行草稿时面板 6 行、composer 3 行、`draw_bottom` 返回 `Some`。

## 13. 交互修复与 `/btwList`（2026-11-12 第二轮）

### 13.1 消息折叠仍不可用

- **根因**：卡片命中走 `task::task_output_rect`，但它只允许 `TaskTab::Agents`，BTW 永远拿不到输出区矩形 → `task_btw_card_target_at` 恒为 `None`。同时主历史的 `history_view` 在任务页打开时仍返回主聊天几何，右端滚动条会抢点击。
- **修复**：`task_output_rect` 允许 `Agents | Btw`；`history_view` / `history_view_ready` 在 `lx.task.is_some()` 时返回 `None`。
- **回归**：`btw_conversation_exposes_card_hits_for_folding` 扫描 BTW 输出区并断言能命中思考卡片。

### 13.2 `/btwList`：切换主 QA 节点

- `/btwList` 只属于 BTW 页签，**不进入主会话**的 `/` 命令面板：在 BTW 输入框输入 `/` 会打开 BTW 自己的命令面板（当前只有 `/btwList`）。
- 键盘 `↑↓` 选择、`Enter`/`Tab` 采纳、`Esc` 关闭；鼠标点条目直接执行。
- 选择器画在上方对话区：标题行 + `Q<n> 标题`，右侧 `● side thread` 表示该主问题已有旁路对话；`↑↓` 环绕移动，`Enter`/`Tab` 切换，`Esc` 关闭，鼠标点行直接切换。
- 切换后按新 QA 加载线程历史，各自保留独立草稿；未加载过的 QA 首次进入仍是空线程。

### 13.3 `@` 面板打开后无法退格

- **根因**：BTW composer 的浮层处理只接管导航键，其余按键直接返回 `false`，`@` 面板打开后退格/字符输入被吞掉。
- **修复**：提及面板可见时把剩余按键交给 `apply_editor_edit`，退格可以删掉 `@`，继续输入会重新过滤候选；与主输入行为一致。
- **回归**：`backspace_removes_the_at_trigger_while_the_mention_panel_is_open`。

### 13.4 QA 归属移到输入框右上角

- 删除输入框上方的独立归属行；`QA n · 标题 · To: <回答者>` 改为输入框**顶边框右上角**的胶囊，位置对齐主会话标题。
- 胶囊使用固定暗海蓝底 + 柔天蓝字（`style::btw_qa_badge()`），不随会话配色或透明终端壁纸变化。
- 底部面板高度从固定 6..8 行降为 5..7 行（顶边框 + tab + 1..3 行正文 + 上下边框），输入框默认仍是一行正文。

## 14. BTW 命令面板、空输入框切 tab 与生成中 loading（2026-11-12 第三轮）

### 14.1 BTW 专属 `/` 命令面板

- `/btwList` 从主会话命令表移除（`SlashCommand::ALL` 不再包含它），主输入框输入 `/btwList` 不会打开 BTW 页签。
- BTW composer 复用 `Lx.slash`：输入行首 `/` 时由 `refresh_composer_slash` 生成 `SlashPanel::Commands`，候选只包含 `BtwList`。
- 键盘：`↑↓` 移动、`Enter`/`Tab` 执行、`Esc` 关闭；其余按键继续编辑并通过 `edit_composer` 重算过滤（删除 `/` 自动关闭）。
- 鼠标：`press/lx.rs` 在 BTW 页签走 `select_btw_slash`，与主输入采纳路径隔离。
- 面板几何复用 `input_rect` 的 BTW 分支（composer 上沿），因此不需要单独维护一套浮层坐标。

### 14.2 空输入框 ←/→ 切换 tab

- 焦点在 composer 且草稿为空时，`←`/`→` 调用共享的 `pane::cycle_task_tab` 轮转 Processes/Agents/BTW，并清理上一页的拖拽/高亮状态。
- 草稿非空时按键仍落到编辑器移动光标，不会误切 tab。

### 14.3 生成中 loading

- 输入框与 tab 行之间固定留 1 行空行（面板高度回到 6..=8 行）；loading 画在**底部容器左上角**（该空行左侧），不是输入框上沿。
- 文案沿用主会话阶段口径：运行中的工具 → `Running tools...`，开放思考 → `Thinking...`，其余 → `Working...`；带主会话同款扫描高亮。
- `BtwThreadState::is_loading` 覆盖 `pending_submit`、未开始回答者计数与流式 assistant 消息；`advance_lx_loading` 只在 BTW 页签可见时推进 `thread.chat.loading_frame`，面板关闭不空转。

### 14.4 验证

```text
cargo test --lib                                                  # 5251 passed
cargo clippy --all-targets -- -D warnings
```
