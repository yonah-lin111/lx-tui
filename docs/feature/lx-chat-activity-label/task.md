# lx 输入框动态任务文案：任务清单

> 状态：已实现并完成自动验证（人工终端验收待用户执行）。
> 日期：2026-10-23。
> 设计依据：[design.md](./design.md)。
> 门槛：用户已批准（见 §3 批准记录），T1 及后续代码修改已执行。

## 1. 本轮授权范围

当前允许：只读检索、逐题确认需求、在本任务目录创建详细设计和执行文档。当前不执行实现、测试代码改动、构建发布或 Git 状态变更。

用户通过设计后，按下列任务修改相关文件并做定向验证。批准实施不自动包含创建分支/worktree、暂存、提交、合并、发布或安装开发二进制。

## 2. 已完成的设计准备

- [x] 使用 grill-me，每轮一个问题，并给出推荐及通俗示例。
- [x] 确认当前 todo 只取 InProgress，不回退第一个 Pending。
- [x] 确认无 todo 时按实际阶段显示 Thinking / Running tools / Working。
- [x] 记录用户纠正：长 todo 可隐藏/截断，耗时/token 不能因它过长而隐藏。
- [x] 确认计划按会话跨轮保留，直到更新或清空。
- [x] 阅读根 AGENTS.md 及其指定项目规范。
- [x] 使用 codegraph 与 codebase-memory 检索实际源码及调用关系。
- [x] 确认两套索引均已存在，无须重复初始化。
- [x] 检查关键路径索引覆盖；对会话加载的部分覆盖段补充直接核对。
- [x] 确认无需新增后端事件、工具协议、数据库或 runtime 缓存。
- [x] 写入当前任务 design.md 和 task.md。
- [x] 用户审核通过，记录明确批准内容（见 §3 批准记录）。

研究时已核实的关键事实：

1. `LxChat.plan` 是会话最后成功快照；revision 不是跨轮全局排序号。
2. 阶段需严格匹配 Streaming.assistant_id；本地命令消息可能追加在 assistant 之后。
3. `reasoning_open()` 可判定真正开放的思考块。
4. 原 `draw_scanned_label` 已接受任意文案，但存在双宽字先落笔后测右界的问题。
5. 当前统计放不下就整段隐藏，必须改变为统计先预算。

## 3. 审核清单 G0

以下提案随设计一并审核，不再把已确认的产品决策重新问一遍：

- [x] 接受 UI 只读派生方案及最小文件范围。
- [x] 接受阶段优先级：todo > 当前 Pending/Running 工具 > 最新开放 reasoning > Working。
- [x] 接受先为完整统计留空间，文字使用剩余宽度。
- [x] 接受 W<S 极窄时普通运行仅绘制可容纳的统计前缀，禁止越界。
- [x] 接受 Esc 待命空间充足时保留统计，极窄时停止反馈优先。
- [x] 接受测试范围及真实终端人工验收边界。

批准记录：2026-10-23 用户批准执行（本轮指令：“根据 docs/feature/lx-chat-activity-label 执行 task，不需要再使用 grill me 了”，并要求使用两套检索 MCP、在 `.worktrees` 新建 git 工作区、完成后写测试、提交代码、最后询问是否合并）。

## 4. T1：实施前定向复核

依赖：G0 通过。

- [x] 按用户本轮指令在 `.worktrees/lx-chat-activity-label`（分支 `feature/lx-chat-activity-label`）实施；未改动 `dev` 主线。
- [x] 用 codegraph 再核对拟改符号，memory graph/snippet 核对状态类型；仅在需要时补查，避免重复全仓扫描。
- [x] 检查拟改路径是否新增子目录 AGENTS.md；有则读取适用规则。
- [x] 确认 `PlanStepView::InProgress`、ToolStatus 及 RequestState 的当前导入路径，以实际公开导出为准。
- [x] 确认 chat_bar 行数仍在限额内，新增测试不会继续堆入超长 lx/tests.rs。
- [x] 核对统计格式化当前行为，保留仅输出 token、秒级耗时及未知值 None 语义。

完成产物：准确的有限文件修改范围；若现状已变化到推翻设计，先更新文档并说明具体差异。

## 5. T2：先写行为回归测试

依赖：T1。

目标文件：`src/ui/main_content/lx/chat_bar/tests.rs`，父文件添加 `#[cfg(test)] mod tests;`。测试数据在组件内构造，不启动真实模型、网络、数据库或终端。

- [x] 构造当前计划 + Streaming assistant 的最小 fixture。
- [x] 对 todo 压过 Thinking/Running tools 写断言。
- [x] 对 Pending 不被当作 todo、空计划及全部 Completed 写断言。
- [x] 对当前 Pending/Running 工具与 InputStreaming/终态工具写分组断言。
- [x] 对 reasoning 开放、关闭和后续 Text/Tool 阶段写断言。
- [x] 对 Committing/Finishing 不读取上一轮阶段写断言。
- [x] 在当前 assistant 后追加本地 shell 记录，验证仍选正确 assistant。
- [x] 为“长中文 todo + 完整统计”构造当前旧实现无法满足的 buffer 用例。
- [x] 先验证失败原因确实是目标缺陷/缺失行为，再修改实现（旧实现输出 `Working... (3s · ↓320 tok)`，todo 完全缺失）。

无需为每一行 helper 镜像写测试；重点覆盖错阶段、错消息、统计被挤掉及中文越界这些实际风险。

## 6. T3：动态状态文案与安全净化

依赖：T2。

### T3.1 只读选择

- [x] 在 chat_bar 内增加私有 ActivityLabel 枚举和纯选择函数（`activity_label` / `current_streaming_message` / `plan_todo`）。
- [x] 只读取 `lx.chat.plan.steps` 中第一个 InProgress；无则继续阶段推导。
- [x] 严格按 Streaming.assistant_id + assistant role 定位当前消息。
- [x] Pending/Running 工具显示 Running tools；InputStreaming 不冒充执行。
- [x] 最后开放 reasoning 显示 Thinking，其他回退 Working。
- [x] 不新增 app 状态、计划副本、revision 缓存、模型输出事件或计时器。

### T3.2 集中文案及净化

目标文件：`src/ui/text/chat.rs`。

- [x] 保留 Working 常量，增加 Thinking / Running tools 常量（`THINKING_LABEL` / `RUNNING_TOOLS_LABEL`）。
- [x] 增加仅用于状态栏的 todo 单行净化函数 `single_line_label`。
- [x] 将换行、Tab、CR、其他控制字符转换为安全空白并折叠，去首尾空白。
- [x] 净化结果为空时回退 Working，不留下空白动画。
- [x] 不改变计划原文、explanation、历史卡片或 result_json（测试断言原文未变）。
- [x] 新增测试文件 `src/ui/text/chat/activity_tests.rs`，测试体不内联堆入生产函数文件。

## 7. T4：统计优先布局及宽字符扫描

依赖：T3。

目标文件：`src/ui/main_content/lx/chat_bar.rs`。

- [x] 将动态文案接入 `draw_loading`，保持输入框上方一行和原左侧起点（`bar.x + 1`）。
- [x] 先计算完整统计宽度，给文字独立宽度预算及绘制右界（`title_budget`）。
- [x] 复用 `text::truncate_to_width`，用显示列宽裁切文字并添加省略号（`clip_label`）。
- [x] 短文案后统计紧跟一列，不无故固定到最右侧（按实际文字尾部定位）。
- [x] 文字预算为 0 时不画文字或额外间隔，完整统计仍画。
- [x] W<S 时按设计的物理裁切规则处理，不继续使用“放不下就整段 return”。
- [x] 去掉普通长 todo 可以挤掉完整统计的旧路径。
- [x] Esc 待命按设计 §6.4 组合停止提示和统计，待命行为/超时不变（仅改绘制编排）。
- [x] 每个字符在 put_char 之前测列宽；不得半写汉字或侵入统计区。
- [x] 零宽字符不按 1 列推进，遵守 Grid 实际支持范围（`width() == 0` 跳过）。
- [x] 以可见文字计算扫描周期，继续使用 loading_frame 和现有 style token。
- [x] 统计静态 dim，文字有动画，不给整行或 todo 增加背景。
- [x] 不新增 todo 的点击、hover、弹窗或键盘入口。

关键审核点：旧算法先 draw label 再尝试 stats；新算法必须先分区再分别绘制。只把 LOADING_LABEL 换成 todo 不能算完成。

## 8. T5：补齐定向验收用例

依赖：T4。

已在 `src/ui/main_content/lx/chat_bar/tests.rs` + `tests/{label,layout,animation,stop_and_lifecycle}.rs` 补齐设计 A01–A21（30 个测试），按职责拆分使每个文件 < 250 行，覆盖矩阵：

| 分类 | 尺寸/输入 | 必须断言 |
| --- | --- | --- |
| 正常尺寸 | 短中英文 todo + 完整统计 | todo 与统计在同一行、统计数值完整。 |
| 长 todo | 中英混合、512 字符范围 | 文字截断，统计完整，行高不变。 |
| 边界尺寸 | W=S、W=S+1、W=S+G+1 | 不多留间隔、不少画统计、不越界。 |
| 极窄 | W<S、0/1/2 列 | 按设计降级，安全绘制，不 panic。 |
| 中文边界 | 剩一列遇汉字、汉字紧邻统计 | 不覆盖相邻单元格或输入框。 |
| 不安全文本 | 换行/Tab/CR/ESC/控制符 | 始终单行，无终端转义执行，原文未改。 |
| 动画 | 同文案多帧、文案动态切换 | 可见文字扫描，统计样式不变。 |
| 生命周期 | 更新计划、清空、跨轮、切会话 | 无陈旧派生缓存，canonical plan 语义保持。 |
| 停止 | Esc 待命、超时、结束 | 正确优先级，停止反馈可读。 |

保留并定向运行已有 Working、统计、Esc 及任务面板相关回归。若新增正确行为导致旧断言冲突，先明确它是否是本次设计允许改变的行为，不能盲目改快照让测试变绿。

## 9. T6：同步标准与设计记录

依赖：T5。

- [x] 更新 `docs/standards/tui-design-requirements.md` 的动态文案、统计优先、极窄及 Esc 说明。
- [x] 更新 `docs/standards/project-directory-structure.md` 中 chat_bar 的固定 Working 职责示例。
- [x] 在本目录记录实际文件、偏离草案的原因及定向验证结果（见 §13）。
- [x] 实现后才把任务勾选完成；测试通过后才填写通过结论。

## 10. T7：精确验证命令

依赖：T6。终端优先 rtk；rtk 子命令不支持时用 `rtk proxy` 包装原命令，不改变测试语义。

### 10.1 新增组件和格式化测试

```bash
rtk cargo test --locked --lib ui::main_content::lx::chat_bar::tests
rtk cargo test --locked --lib ui::text::chat::activity_tests
rtk cargo test --locked --lib ui::text::chat::stats_tests
```

### 10.2 既有受影响显示回归

```bash
rtk cargo test --locked --lib ui::main_content::lx::tests::streaming_shows_loading_bar_above_input -- --exact
rtk cargo test --locked --lib ui::main_content::lx::tests::loading_shows_only_the_label_without_a_mark -- --exact
rtk cargo test --locked --lib ui::main_content::lx::tests::loading_stats_follow_the_label_with_seconds_and_output_tokens -- --exact
rtk cargo test --locked --lib ui::main_content::lx::tests::esc_armed_replaces_loading_label_with_stop_hint -- --exact
rtk cargo test --locked --lib ui::main_content::lx::task::tests::history_takeover::task_panel_hides_the_streaming_loading_bar -- --exact
```

核对输出实际执行了测试，不能把 `0 tests` 记为成功验证。若符号被重命名，先定位新名再运行，不扩大为全仓测试。

### 10.3 计划生命周期回归

```bash
rtk cargo test --locked --lib app::update::chat::tests::process_cards::plan_commit_updates_current_plan_and_ignores_duplicates -- --exact
rtk cargo test --locked --lib app::update::chat::tests::process_cards::plan_restore_picks_the_latest_successful_update_only -- --exact
rtk cargo test --locked --lib app::update::chat::tests::task_page::new_session_clears_the_previous_plan -- --exact
```

### 10.4 格式与静态检查

只检查本次修改的 Rust 文件格式；命令中的实际测试文件以最终落点为准：

```bash
rtk proxy rustfmt --check --edition 2024 src/ui/main_content/lx/chat_bar.rs src/ui/text/chat.rs
rtk cargo clippy --locked --lib --tests -- -D warnings
```

已核对当前 Cargo.toml 的 edition 为 2024；实施前若配置变化，以项目实际配置为准。格式修复也只作用于本次文件，不能顺手格式化无关源码。Clippy 选择受影响 lib/test target；Rust 会编译它们的依赖，不将其描述为只扫描某几个文件。

不运行全量 cargo test，不做性能压测，不新增网络测试。若遇到无关的既有 lint/测试失败，记录具体错误和文件，不擅自扩大修复范围。

## 11. T8：真实终端验收

自动验证通过后，由用户运行开发程序确认外观。Agent 不宣称已完成人工终端观感验收。

- [ ] 执行包含中文 InProgress 的计划，观察原 Working 位置出现对应步骤。
- [ ] 完成当前步骤、切换 InProgress、清空计划，观察内容即时变化。
- [ ] 无计划时分别触发开放 reasoning、工具执行和普通文本输出。
- [ ] 缩窄窗格：先省略长 todo，耗时和输出 token 保持可读；再缩到物理极窄时符合既定降级。
- [ ] 输入框高度、左右对齐与原外侧空档正常。
- [ ] 扫光颜色/节奏延续现状，统计不闪动。
- [ ] Esc 待命文字及再次停止可用，超时回到当前任务。
- [ ] 切换会话和后台任务面板，状态栏显示与消失符合现有生命周期。

## 12. 当前执行记录

| 阶段 | 状态 | 结果 |
| --- | --- | --- |
| 检索及需求盘问 | 已完成 | 两套 MCP 已使用，四个边界问题已获得回答。 |
| design.md / task.md | 已编写并完成文档自检 | 已检查相对链接、代码围栏配对、编码及示例宽度；仅新增本任务文档。 |
| 用户批准 | 已获得 | 2026-10-23 用户批准执行（本轮指令），不再重新盘问。 |
| 实现与测试代码 | 已完成 | 修改 3 个生产文件、新增 2 个测试文件，见 §13。 |
| Rust 自动验证 | 已通过 | 定向用例 + `cargo test --locked`（lib 3067 passed、集成测试全绿）+ clippy `-D warnings` + rustfmt。 |
| 人工 TUI 验收 | 未执行 | 需用户 `cargo run` 确认视觉；Agent 不宣称已完成。 |

实施阶段若需并行，最多两个子 agent，按独立大任务划分；不得同时编辑同一文件。此阶段已有一个只读检索子任务完成，不存在并行代码改动。

## 13. 实施记录（2026-10-23）

### 13.1 实际改动文件

| 文件 | 改动 |
| --- | --- |
| `src/ui/main_content/lx/chat_bar.rs` | 新增私有 `ActivityLabel` + `activity_label` / `current_streaming_message` / `plan_todo`；`title_budget` 统计优先预算；`draw_loading` 改为先算预算再分别绘制文字与统计；`draw_scanned_label` 先裁切再按列宽落笔；新增 `stop_shortest_width`；声明 `mod tests`。 |
| `src/ui/main_content/lx/chat_bar/tests.rs`（新） | 组件测试入口：共享 fixture（`text_step`/`tool_step`/`assistant`/`streaming_lx`/`plan`/`with_stats`/`draw_bar`/`row_text`/`col_of` 等）与四个子模块声明，194 行。 |
| `src/ui/main_content/lx/chat_bar/tests/label.rs`（新） | 文案派生 A01–A11：todo 优先级、Pending/Completed/空计划回退、Pending/Running/InputStreaming/终态工具、开放/关闭 reasoning、Committing/Finishing、shell 记录、计划变更、跨轮与会话隔离。 |
| `src/ui/main_content/lx/chat_bar/tests/layout.rs`（新） | 布局 A12–A17：统计优先、512 字符混合 todo、W=S / W=S+1 / 仅统计、W<S 极窄裁切、宽字符边界、不安全文本净化、零宽字符。 |
| `src/ui/main_content/lx/chat_bar/tests/animation.rs`（新） | 动画 A18：可见文字扫描、3 字符窗口、统计恒 dim 不闪动。 |
| `src/ui/main_content/lx/chat_bar/tests/stop_and_lifecycle.rs`（新） | A19–A21：Esc 待命 + 统计、极窄停止反馈优先、超时恢复、结束/面板消失、回到底部共存。 |
| `src/ui/text/chat.rs` | 新增 `THINKING_LABEL` / `RUNNING_TOOLS_LABEL` 与 `single_line_label` 单行净化；声明 `mod activity_tests`。 |
| `src/ui/text/chat/activity_tests.rs`（新） | 5 个净化/回退测试。 |
| `src/ui/main_content/lx/tests.rs` | 仅改一处既有断言 `esc_hint_shortens_on_narrow_bar`：原“窄栏先丢 token 计数”被设计 §6.4 明确推翻（待命期间空间够时保留统计），改为断言短停止短语 + 统计仍在；其余既有断言未改。 |
| `docs/standards/tui-design-requirements.md` | §输入框上方那一行 ① loading：更新为动态任务/阶段文案、阶段判定优先级与 assistant 定位、单行净化、统计优先布局、极窄降级与 Esc 组合。 |
| `docs/standards/project-directory-structure.md` | chat_bar 职责示例由固定 `Working...` 改为动态任务/阶段文案说明。 |

未改动 `src/app`、`src/event`、`src/chat`、`src/db`、`Cargo.toml`/`Cargo.lock`；没有数据库迁移，`SCHEMA_VERSION` 不变。

### 13.2 与设计草案的偏离

1. **无 `BarLayout` 结构体**：设计提到“可用小的纯函数返回文字、统计的可用区间”。实施落成单个纯函数 `title_budget(available, stats_width, stop_armed) -> u16`，统计起点由绘制顺序（`draw_scanned_label` 返回实际文字结束列）决定——这正是“短文案统计紧贴实际尾部”要求的直接实现，返回两个区间反而需要额外传递实际尾部。`draw_loading` 参数由 7 个布尔/坐标参数收敛为 `(grid, bar, loading_frame, stop_armed, activity, stats)`。
2. **净化函数返回 `Cow<str>`**：常见路径（无控制字符、无多余空白）零分配借用；异常文本才 `to_string`。净化后无可见列即回退 `LOADING_LABEL`（`Working...`）。
3. **Esc 待命阈值**：用 `stop_shortest_width()`（最短短语列宽）判定是否进入“停止反馈优先”分支；该分支下文字取全部可用宽度、统计从剩余列裁切。空间充足时统计完整保留，与 §6.4 一致。
4. **测试文件按职责拆分**：设计原定单个 `chat_bar/tests.rs`，但 30 个用例会到 740 行（超项目「子文件 ~500 行」的取向），改为 `chat_bar/tests.rs`（共享 fixture）+ `tests/{label,layout,animation,stop_and_lifecycle}.rs` 四个子模块。
5. **`row_text` 测试辅助丢弃 `CONTINUATION` 占位**：Grid 用 `\0` 标记宽字符后半格，测试按显示列换算列号，避免把半格当成字符。

### 13.3 定向验证结果（全部实际执行，非 0 tests）

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked --lib ui::main_content::lx::chat_bar::tests` | 30 passed |
| `cargo test --locked --lib ui::text::chat::activity_tests` | 5 passed |
| `cargo test --locked --lib ui::text::chat::stats_tests` | 2 passed |
| `ui::main_content::lx::tests::streaming_shows_loading_bar_above_input` | 1 passed |
| `ui::main_content::lx::tests::loading_shows_only_the_label_without_a_mark` | 1 passed |
| `ui::main_content::lx::tests::loading_stats_follow_the_label_with_seconds_and_output_tokens` | 1 passed |
| `ui::main_content::lx::tests::esc_armed_replaces_loading_label_with_stop_hint` | 1 passed |
| `ui::main_content::lx::task::tests::history_takeover::task_panel_hides_the_streaming_loading_bar` | 1 passed |
| `app::update::chat::tests::process_cards::plan_commit_updates_current_plan_and_ignores_duplicates` | 1 passed |
| `app::update::chat::tests::process_cards::plan_restore_picks_the_latest_successful_update_only` | 1 passed |
| `app::update::chat::tests::task_page::new_session_clears_the_previous_plan` | 1 passed |
| `cargo test --locked --lib ui::main_content::lx` | 378 passed |
| `cargo test --locked`（提交前全量） | lib 3067 passed；集成测试 `cursor_steady`/`lx_paste`/`popup_border`/`prompt_*` 全绿；0 failed |
| `cargo clippy --locked --lib --tests -- -D warnings` | 无警告 |
| `rustfmt --check --edition 2024`（本次 5 个 Rust 文件） | 通过 |

验证方法说明：使用 `rtk` 包装（`rtk err` / 直接 `cargo`）执行；`CARGO_TARGET_DIR` 指向主 checkout 的 `target` 以复用缓存，不改变测试语义。任务清单 §10 写的是“不运行全量”，但项目 AGENTS.md/代码规范要求提交前全量，且本轮补充要求“必须编写测试代码进行测试”，因此额外执行了全量 `cargo test --locked`；结果如上。

### 13.4 未做（超出本次范围）

- 真实终端观感验收（T8）：需用户运行 `cargo run`。
- 未新增 todo 点击/hover/弹窗/键盘入口、未新增设置项或随机文案词库、未改后端事件/DB/runtime 缓存。
- 未执行合并、推送或主 checkout 的 Git 状态变更。
