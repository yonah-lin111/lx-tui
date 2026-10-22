# lx 最新轮次撤销：执行任务

> 状态：**已实现并合并到 dev**（2026-10-22）。
>
> 依赖：同目录 [design.md](design.md)。
>
> 下方清单保留设计期勾选状态；实施结果以合并后的代码、[review.md](review.md) 与测试为准。

## 0. 已确认约束

- [x] 空闲且输入为空时，双 Esc 选最新用户消息；忙时保留双 Esc 停止。
- [x] 只操作最新一轮，不允许导航到更早轮次；成功删除后才能操作上一轮。
- [x] 第一次 Enter 开四项面板，第二次 Enter 执行；默认第一项。
- [x] 四项：恢复文件并删对话 / 仅删对话 / 仅恢复文件 / 取消。
- [x] 删除对话成功后回填原文，仅恢复文件不回填、不推进目标。
- [x] 成功 tip 独立持久化，按类型区分；不属于 user/assistant，不进入上下文。
- [x] 仅 edit/write 文件变化；有 Git 优先实际项目对象库，无 Git 可恢复。
- [x] 保留 Agent 操作前用户已有修改，不动 Git index/分支/提交/refs。
- [x] 多 Agent 冲突拒绝整次恢复，不覆盖、不删对话；绑定实际 worktree 路径。
- [x] 非空草稿不被回填覆盖；命令必须独立占满输入。
- [x] 本会话存在活动后台任务则先提示手动停止；其他会话继续工作。
- [x] codegraph、codebase-memory 已用于现状和两个参考仓库检索。
- [ ] 用户审核 design.md 与本清单，包括资源限额、字节备份、锁与异常恢复取舍。

## 1. 实施前基线与模块确认

- [ ] 重新确认当前 schema 版本；拟用 v12，若被并行任务占用则顺延，不能覆盖别人的迁移。
- [ ] 使用代码图核对下表各接入点及调用者；对索引覆盖不足或过期的具体文件定点读取。
- [ ] 记录当前相关文件行数；超限按职责拆分，新增子文件控制在约 500 行。
- [ ] 确认 `AgentHostRequest`、ToolCallTicket、RequestKey 与文件 executor 的真实字段，禁止通过 turn_index 猜目标。
- [ ] 确认 session 表实际名称和 FK，不把设计 SQL 示例直接复制成迁移。
- [ ] 只读检查现有实现；本任务不执行 git add/commit/checkout/reset，不创建分支或 worktree。

| 接入点 | 预计修改 |
| --- | --- |
| `app/state/lx/slash.rs` | Rewind 枚举、undo alias、Rewind 面板 |
| `app/state/lx/rewind.rs` | 独立状态与动作/notice 类型 |
| `app/update/chat/rewind.rs` | 状态转换、Effect 与回填 |
| `app/update/pane.rs` / `input/` | 入口优先级与防重复确认 |
| `chat/agent_loop/mod.rs` / `executor.rs` | ticket/recorder 显式传递 |
| `chat/tools/{write,edit,lock,mutation}.rs` | 快照捕获、临界区、恢复原语 |
| `event/chat/{host,tool_calls,file_snapshots,rewind}.rs` | 新 ACK、后台 IO、执行协调 |
| `db/chat/rewind.rs` / `migration.rs` | 日志、notice、条件提交、迁移 |
| 启动装配 / `db/service.rs::Persistence::start` | 遗留 stream/tool/process 与新 journal 按 session guard 安全恢复 |
| `git/snapshot.rs` / `platform/` | Git 对象读写、身份、链接/权限/跨进程锁 |
| `ui/main_content/lx/history*` / `slash.rs` / `ui/text.rs` / `ui/style.rs` | 显示时间线、tip、高亮、面板文案和几何 |

完成标准：依赖方向无反转，工具不导入 event/db 实现，app/ui 不新增 IO。不要提前创建没有实际职责的空模块。

## 2. 持久化模型与序号

- [ ] 新增 notice、turn file scope、file image、mutation、rewind operation 表与必要索引/状态约束。
- [ ] session revision / 跨进程执行权纳入发送、恢复、会话删除入口，保证同 session 不并发改变历史。
- [ ] user message ID 为稳定业务键；journal 不随目标 message 删除而过早级联消失。
- [ ] image 用原始 BLOB 保存；区分不存在、普通文件、链接，不用 UTF-8 diff 当备份。
- [ ] begin_turn 同事务写 scope(capturing, capture_version)；终态全部 mutation ACK drain 后 seal complete。异常退出证据不足时保持 incomplete，不把无记录视作零变更。
- [ ] notice operation_id 唯一；重复提交不插第二条。
- [ ] 新建 message 和 notice 在同一写事务按联合 MAX(sequence)+1 分配。
- [ ] 删除轮次前先取 notice 新序号，防删最新后重新分配旧位置。
- [ ] 视图投影保留 message sequence，notice 独立加载，不加入 messages。
- [ ] delete + notice + revision + operation committed 同事务。
- [ ] 清理流程只删无未完成引用的 image；整个 session 删除时清理所属记录。

针对验证：

- [ ] v11 升级保留历史会话、工具、进程，不误建空快照；重复启动不重复迁移。
- [ ] 连续撤销两轮后旧 tip 都在，新增轮次在 tip 后面。
- [ ] turn_index 被复用也不会绑定到旧 journal。
- [ ] transaction 注入失败时，消息和 tip 同时提交或同时保留原状。
- [ ] operation ID 重放返回原结果；取消与失败没有成功 tip。

## 3. Git 与无 Git 存储

- [ ] 在 `git/` 增加基于实际目标路径的归属查询，保存 worktree root、gitdir、common dir、symbolic HEAD/HEAD OID/unborn 身份；新 mutation 和恢复前复核。
- [ ] 原始字节经 stdin hash-object；cat-file blob 读取；超时、退出码、Git 环境变量重定向都处理。
- [ ] 有 Git 优先对象库，同时保存 SQLite 字节兜底；无 Git 直接使用独立字节存储，不创建隐藏仓库。
- [ ] Git 缺失/失败只能在字节快照持久化成功后降级；两者都失败则不修改目标。
- [ ] GC 删除不可达 blob 后仍可从字节备份恢复；不为保活创建 refs。
- [ ] 快照限额拟为 64 MiB/image、256 MiB/turn；所有超限在该次 mutation 写入之前返回错误。
- [ ] 缺快照、已知零变更、已恢复、快照损坏分别表达，不把它们合为 0 文件。

针对验证使用临时 Git 仓库：

- [ ] 保留用户未提交内容和暂存区原内容；未跟踪/忽略文件可恢复。
- [ ] 恢复前后 HEAD、refs、index 字节不变；仅允许新增 blob 对象。
- [ ] 无 Git、git 可执行文件缺失、对象写入失败、blob 丢失、备份失败分别覆盖。
- [ ] 二进制、BOM、CRLF、无末尾换行按字节一致恢复。
- [ ] 不在真实开发仓库中运行破坏性 Git 测试命令。

## 4. 受控 mutation 捕获

- [ ] 新增 PreparedMutation / MutationTicket / ObservedMutation 与同步 recorder trait。
- [ ] BeginTool ticket 通过 executor 显式到达每个文件调用；所有权不取模型参数。
- [ ] spawn_blocking 内等待 Prepare/Commit ACK，事件主循环不阻塞。
- [ ] Git/大文件 IO 在后台，DB 提交成功后 ACK；事件 ACK 处理不重复拿 worker 路径锁。
- [ ] write 将旧内容读取和编码规划移入锁内，不保留锁外旧 preimage。
- [ ] edit Add/Update/Delete/Move 都在实际发生 IO 的入口捕获，不只看最终 completed。
- [ ] Move 同时锁住源/目标，记录先改源后 rename 失败的真实状态。
- [ ] 每次写前持久化 before + planned-after，写后持久化 actual-after。
- [ ] Prepare 失败零写入；Commit ACK 丢失保留 prepared 并按不确定结果处理，不重执行。
- [ ] 同轮每路径后续 before 与前次 after 不一致时永久标记 non-linear，禁止整轮恢复。
- [ ] 正常生产入口必须有 recorder；no-op 仅能明确用于测试，不得静默绕开快照门槛。

针对验证：

- [ ] 一轮同文件写多次可恢复到第一次 before。
- [ ] 多文件工具部分失败仍可恢复已经提交的文件。
- [ ] 原内容捕获失败时目标字节不变。
- [ ] channel 关闭/应用退出会唤醒 worker，无无限等待或锁泄漏。
- [ ] A1 -> B -> A2 被标记 non-linear，即使当前等于 A2 after 也拒绝。
- [ ] 工具输出/diff 被裁切不影响独立恢复数据。

## 5. 路径、锁与 worktree 隔离

- [ ] 按路径排序去重的多目标锁，mutation 与恢复使用同一顺序。
- [ ] 现有短锁改造避免重复加同一非重入锁；读取/快照/写入/观察处于同一临界区。
- [ ] `platform/` 提供跨进程稳定 sidecar advisory lock；目标原子替换不使锁失效。
- [ ] session guard 覆盖新轮次、恢复、删除；锁等待不持 DB 写事务。
- [ ] turn runtime 从 BeginChatTurn 事务之前持 session guard，直到终态与全部 mutation ACK drain/seal；ACK handler 只验证所有权 token，不重复取锁。
- [ ] 仍有后台进程时 guard 转移到 session 运行态持有；同实例新轮次复用 owner token，全部运行态终结并落库后才释放。
- [ ] 相对路径用冻结工具 cwd；绝对路径记录真实目标，不用 repo 共享 key 重新拼接。
- [ ] 区分物理文件与链接目录项：write/update 与 delete/move 的恢复对象不同。
- [ ] 记录并重验 worktree/gitdir、父目录、链接解析身份；目标重定向不能落到主 checkout。
- [ ] 不存在目标通过现存祖先身份定位；不因 canonicalize 失败退回进程 cwd。
- [ ] 硬链接、只读权限、目标变成目录等情况明确失败或按证明安全的目录项语义处理。

必须有这些测试：

- [ ] 同进程两个 Agent 改不同文件，撤销 A 不碰 B。
- [ ] A 改完后 B 改同文件，撤销 A 全体拒绝，其他目标文件也零写入。
- [ ] 两个独立 lx-tui 进程对同路径互斥，对不同路径可前进。
- [ ] 第二实例启动不改变第一实例仍运行的 stream/tool/process 或文件 scope；仅有后台进程存活时同样受保护。
- [ ] 主 checkout dev 与 linked worktree 同名文件，仅实际修改的 worktree 改变。
- [ ] 会话 cwd 在 dev，但绝对路径指向 linked worktree，同样只恢复 worktree。
- [ ] shared Git common dir 不导致恢复路径共享。
- [ ] worktree 移动、分支切换、gitdir 变更、链接重定向、父目录被替换都拒绝。
- [ ] 链接写入/删除/移动及二进制删除恢复不误改链接目标。

## 6. 恢复协调、幂等与故障

- [ ] 执行前核对 latest stable user ID / revision / session guard / 后台任务 / pending operation。
- [ ] 对所有文件取锁后全量预检；任一冲突零写入，不删除对话。
- [ ] 持久化 operation 与执行前镜像，逐项恢复并记录进度。
- [ ] 恢复全部完成后才提交对话变化和 tip，不做 UI 乐观删除。
- [ ] IO 失败尝试逆序补偿；补偿前检查是否被外部程序再次改动。
- [ ] 补偿失败进入 recovery_required，保留快照、列出路径并阻止该会话继续发送。
- [ ] 最终 DB ACK 不确定时按 operation ID 查询，不能重复副作用。
- [ ] 启动恢复 prepared/partially-restored 状态：按 before/planned-after/第三种状态分类，禁止重跑工具。
- [ ] 启动/恢复先非阻塞取得 session guard；拿不到跳过。既有 recover_interrupted_streams/tool_calls/processes 改成同样的按 session 恢复，禁止无条件全库标中断。
- [ ] committed 但 UI 未收到结果时，只重载，不重复恢复；待回填原文可恢复消费。
- [ ] 仅恢复文件标记已恢复；重复操作不改文件、不重复 tip。
- [ ] 已恢复后执行组合动作，只删除对话，使用明确的“此前已恢复”tip。

故障注入至少覆盖：第二个文件恢复失败、删除失败、rename 失败、Commit ACK 丢失、最终 DB commit 失败/结果不确定、补偿失败、每个持久阶段异常退出后重启。

完成标准：预检冲突没有副作用；执行中失败不谎报原子成功，消息保留且有可诊断/可恢复状态。

## 7. 输入、面板与回填

- [ ] 新增 `/rewind` 与 undo alias，复用统一打分；命令候选不出现重复项。
- [ ] 全输入仅为独立单行 slash 查询才允许采纳；完整命令、前缀、从 `/` 候选选择均可；夹带正文/多行/参数拒绝且保留草稿。
- [ ] idle rewind 与 busy stop arm 分离；5 秒超时、编辑/焦点/会话变化清除 rewind arm。
- [ ] 高优先级面板消费的 Esc 不计入双 Esc；历史浏览/任务页保持原优先级。
- [ ] 最新用户消息高亮，不提供选择更早轮次的上下键入口。
- [ ] Selected Enter 开面板、再次 Enter 执行；面板 Esc 退回选择，再 Esc 取消。
- [ ] `/rewind` 或 `/undo` 的命令采纳 Enter 直接开同一动作面板；下一次 Enter 才执行，不额外增加一次隐藏确认。
- [ ] 默认第一项；四项键鼠导航与窄屏滚动复用现有组件。
- [ ] KeyEvent Repeat 不确认，Executing 吞重复输入，成功不自动发送回填文本。
- [ ] 仅对发起 pane 的匹配归属回填，一次输入 undo；其他同 session pane 更新历史但保留草稿。
- [ ] 本会话存在后台任务/其他 pane 在途请求时拒绝；不停止其他会话。
- [ ] 旧会话含文件选项禁用并解释；已知 0 文件与无快照区分。

## 8. 持久化 tip 与上下文隔离

- [ ] notices 独立加载，history 只读 merge，layout/render/hit/sticky/scrollbar 共享行数口径。
- [ ] tip 使用独立 `-- Rewind --` 样式，不用 user 的 `>` 或 assistant 的步骤方块。
- [ ] 按三种动作和“此前已恢复/没有文件变更”显示准确文案。
- [ ] 文案集中 ui/text.rs，样式集中 ui/style.rs；面板文案保持单宽。
- [ ] messages 为空而 notices 非空时，保持历史视图，tip 不被欢迎页覆盖。
- [ ] `/resume` 与重启保持 tip 原位置，多次撤销旧 tip 不丢。
- [ ] notice 不进入 ChatMessage / ModelItem、token 预算、模型历史裁剪和供应商 wire 请求。
- [ ] 撤销成功后重算最近用量、计划视图、工具/卡片选择、sticky 和滚动位置，不残留被删轮次索引。

关键验证：构造含多种 notice 的持久化历史，经真实 context assemble/transport 构造请求，断言只包含保留下来的真实对话，且 notice 文案、计数、目标 ID 均不泄漏到模型。

## 9. 验证执行策略

各阶段先执行对应模块的精确验证；通过后不反复跑无关全量。命令遵循 rtk 优先，rtk 包装不可用才退回原命令。以下模块路径是预期测试过滤器，实施后按真实模块名核对。

```text
rtk proxy cargo fmt --check
rtk proxy cargo test db::chat::rewind
rtk proxy cargo test chat::tools::mutation
rtk proxy cargo test chat::tools::edit
rtk proxy cargo test chat::tools::write
rtk proxy cargo test event::chat::rewind
rtk proxy cargo test app::update::chat::rewind
rtk proxy cargo test app::state::lx::slash
rtk proxy cargo test ui::main_content::lx::history
rtk proxy cargo test chat::context
rtk proxy cargo clippy --all-targets -- -D warnings
```

- [ ] 受影响的既有停止、prompt-history、任务页、slash/mention/paste 及会话加载测试通过。
- [ ] git/platform 新增模块、跨进程临时二进制测试、迁移与故障注入按实际测试位置补充精确过滤器。
- [ ] 触及共享 executor/context/输入分派时检查相邻回归模块，不以只测新增模块替代影响分析。
- [ ] 若影响已覆盖几乎全部 crate 或进入提交阶段，再按项目要求执行完整 cargo test；记录实际执行范围，不伪称全量通过。
- [ ] 若触及 src/tui，必须追加 tui::tests 顺序回归；本设计本来不需要改终端生命周期。
- [ ] 更新本清单，记录实际命令、结果与未验证项目；文档阶段不运行产品测试。

## 10. 用户终端验收与收口

Agent 无法交互式验证 TUI，用户在实现完成后运行 cargo run：

1. 空输入双 Esc，只高亮最新 user；一次 Enter 只开四项面板，再 Enter 才执行。
2. 忙时仍双 Esc 停止；回答刚结束时不会继承之前停止 arm 进入撤销。
3. 撤销成功原文仅回填，不自动发送；有草稿时不会覆盖。
4. 三种操作产生不同 tip；tip 格式区别双方消息，重启后仍在。
5. 无 Git 文件可恢复；Git dirty 内容、index 和 dev 主 checkout 不被误改。
6. 两个 Agent 改不同文件互不影响；同文件后写与交错写均拒绝恢复。
7. 实际目标位于 linked worktree 时，主 checkout 的同名文件逐字节不变。
8. 有后台任务先提示手动停止；取消/失败保留对话，且不出现成功 tip。
9. 窄窗格、长中文 prompt、tip-only 历史、滚动与 sticky 没有错位。

- [ ] 同步更新唯一规范来源中受影响的模块清单、slash/Esc/tip 规则；不在其他文档创造冲突规范。
- [ ] 最终说明实际修改、精确验证、跨外部进程竞态限制与用户尚需验收的外观。
- [ ] 不执行未获授权的 Git 状态修改或发布操作。
