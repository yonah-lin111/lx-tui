# lx 连续同种工具调用合并：执行任务

> 状态：**已实施**（2026-11-11）；自动化验证通过，TUI 外观待用户运行 `cargo run` 验收。
> 对应设计：同目录 `design.md`。
> 目标：同一 assistant 消息内连续同名工具调用只显示一个 `■ 动词` 组头，下面逐调用展开；`read` 标题保留种类状态，标题下方只显示一条路径。

## 0. 审核与开工复核

- [ ] 用户审核通过 `design.md` 与本文，确认 §10 的四个待确认点。
- [ ] 只读检查工作区差异，识别用户已有变更；不覆盖、不恢复、不暂存。
- [ ] 阅读并遵守四份规范：`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`。
- [ ] 用 codegraph / codebase memory 复核 `snapshot::steps`、`paint::BlockSource`、`tool::rows/draw/card_regions`、`file_summary` 的当前符号与调用链。
- [ ] 核对本设计中的分组范围、`read` 单条路径口径、组头聚合指标；审核未确认的项不得擅自扩大。
- [ ] 核对 `docs/standards/lx-message-block-display.md` 的工具/非正文块清单与合并形态；新增工具必须同步更新该文档。
- [ ] 确认不修改 DB schema、工具执行结果 JSON、`/resume` 投影与模型上下文。

## 1. 快照分组与块拆分

依赖：步骤 0 完成。目标：分组只发生在快照层，行数、绘制与命中共用同一份结果。

- [ ] 在 `history/snapshot.rs` 的 `steps()` 中扫描连续 `AssistantStepView::Tool`：
  - 普通工具按 `call.name` 分组；`write_stdin` 按 `(name, 显示动词)`；
  - `ask` 不合并；`update_plan` / `list_agents` 快照合并（只显示最新正文）；
  - `Part`、不同分组键、消息结束即停止；
  - 单调用保持原路径。
- [ ] 新增组头块：`rows = 1`，只承载 `■ 动词 · N calls` 的定位；组头不登记点击面。
- [ ] 新增调用体块：每个调用一个块，`rows = max(1, tool::rows(...) - 1)`；组内块之间不调用 `spacer()`。
- [ ] 组前/组后仍各留一个空行；空行只由 `snapshot` 累加，绘制路径不另算。
- [ ] 空调用体补一行兜底，保证每个调用在组内可见。
- [ ] 组头块使用独立缓存槽位（建议 `GROUP_HEADER_SLOT_BASE - first_step`），避免与首个调用体 `step_index` 撞槽。
- [ ] 确认 `MessageRows`、sticky 行、滚动条、`↓ jump to bottom` 的几何仍由同一份快照派生。

测试：`history/tests/tool_geometry.rs` 新增/改写分组几何测试（见步骤 6）。

## 2. 工具体绘制入口

依赖：步骤 1 的块结构确定。目标：复用现有工具树体，不重写各工具内容。

- [ ] 在 `history/tool.rs` 增加：
  - `group_header(call, count, kind, width, loading_frame, animating) -> Vec<Segment>`（列表合并 `N calls`，快照合并 `N updates`）；
  - `group_status(calls) -> Option<String>`（运行/失败/中断计数，`edit`/`write` 增删合计，`search` 命中合计）；
  - `body_rows(...)`（= 原 rows 减标题行，至少 1）；
  - `draw_body_animated(...)`（按 `ToolDetailView` 分派到各模块的 headerless 入口）；
  - `body_card_regions(...)`（只算树体点击面，不含标题）。
- [ ] 给以下模块增加“只画树体”的入口，或在现有 `draw` 上增加 `headerless: bool`：
  - `file_summary.rs`（read/search/edit/write/Other 摘要）
  - `file_diff.rs`（diff 卡）
  - `command.rs`（exec_command / write_stdin）
  - `search.rs`
  - `mcp.rs`
  - `web_search.rs`
  - `plan.rs`
  - `ask.rs`
  - `subagent.rs`
- [ ] headerless 绘制规则：
  - 不画 `■ 动词` 标题行；
  - 保留原有树体、折叠提示、diff 行底色、命令输出样式、链接热区；
  - 原单调用标题状态追加到该调用首节点尾部（` · Exited 0 · 241ms`、` · 3 matches`、` · 1/3 completed` 等）；`read` 的标题状态保留在标题，不追加到路径行；
  - 非组内末项的调用首行 `└─` 改为 `├─`；`file_diff` 的首段连接符同样处理；
  - 失败/中断连接符与状态文字用 `agent_fail` / `agent_warn`，错误正文保持正常前景。
- [ ] 组头方块颜色取组内最差状态；运行中按 `loading_frame` 呼吸，动画停止时实心。
- [ ] 组头聚合状态只在存在非完成态时写 `· N running` / `· N failed` / `· N interrupted`；全部完成不写。
- [ ] 处理 `agent_links` 等行内热区：group body 相对块的偏移需从标题偏移中减 1，或提供 headerless 变体。
- [ ] 极窄宽度：组头放不下 `· N calls` 时整段省略；调用体沿用现有树体降级，不越界。

测试：各工具子模块的既有测试补 headerless 路径；新增组内首/末连接符、状态后缀、失败颜色断言。

## 3. `read` 单条路径展示

依赖：步骤 2 的 headerless 入口。目标：保留标题状态，标题下方只显示一条路径。

- [ ] `file_summary::title_status(Kind::Read)` 保留 `file` / `folder · N files` / `image`；不要清空标题状态。
- [ ] `file_summary::detail_layout` 的 `Kind::Read` 分支只构造一个路径节点：
  - 路径按 `tree::content_width(width)` 单行尾部省略，不软换行；
  - 不追加 `fact_nodes` 的 `output limited`、取消状态词、`… N more lines`；
  - 无路径且无错误时保留一条 dim 状态词；空状态也不能产生 0 行。
- [ ] 失败/中断仍保留**一条**错误/中断原因（单行省略），连接符用失败红/中断黄；不显示路径与其它事实。
- [ ] `read` 不登记点击面；点击路径不折叠、不触发任何动作。
- [ ] 单次 read：`■ Read · file` + `└─ <path>`；合并 read：`■ Read · N calls` + 每个 read 一条路径。
- [ ] 目录 read：`■ Read · folder · N files` + 目录路径；条目数只保留在标题。
- [ ] 更新 `src/ui/main_content/lx/history/tool/tests/extras.rs` 中 read 标题/目录计数/详情断言。
- [ ] 更新 `history/tests/transcript.rs` 中 read 步骤的行数与文本断言。

## 4. 点击面与命中

依赖：步骤 1–3。目标：组头不可点，每个调用体独立可点。

- [ ] `snapshot::tool_cards()` 为每个调用体登记 `CardKind::Tool`，`card_start` 指向该调用体块首，不含组头行。
- [ ] `tool::body_card_regions()` 的偏移从 0 开始：
  - 命令参数区/输出区仍各自独立；
  - MCP / web_search 的参数块/结果块仍各自独立；
  - 文件 diff / search / subagent 的整块点击面覆盖树体，不覆盖组头。
- [ ] 组头不产生任何 `CardRegion`；点击组头空白不展开任何调用。
- [ ] `read` 无点击面；`has_expandable_content` 对 read 返回 false。
- [ ] 确保点击第一个调用只切换第一个调用的 `detail_expanded`，不影响同组其它调用。
- [ ] 复核 `history_hit.rs` 的卡片命中、折叠内容 hover 区与滚动条几何，全部从新快照读取。

测试：`history/tests/tool_geometry.rs` 的卡片命中数、逐调用展开、组头不可点；`history/tests/cards.rs` 的块间空行例外。

## 5. 缓存与指纹

依赖：步骤 1–4。目标：组内位置变化、状态变化、read 单条路径规则都能正确失效。

- [ ] 在 `cache.rs` 增加组头保留槽位常量，确保不与 `SHELL_SLOT_BLOCK`、`REPORT_SLOT_BLOCK` 等冲突。
- [ ] 组头块指纹覆盖：组内每个调用的 `call_id`、`name`、状态、错误、标题状态来源字段、`count`。
- [ ] 调用体块指纹覆盖：原 `step_signature` + `GroupSlot`（`last`、组大小）+ headerless/read 单条路径规则版本。
- [ ] 确保追加一个同种调用时：
  - 旧末项调用体因 `last: true -> false` 失效并重画 `└─ -> ├─`；
  - 组头 `count` 变化失效；
  - 不出现两个 `└─` 或旧行数残留。
- [ ] 确认 hover / loading 帧 / 滚动位置不进入块指纹，不导致每帧重建。
- [ ] 确认列表指纹仍覆盖 steps 变化；如组内位置变化未反映到列表指纹，由块指纹兜底。

测试：`history/cache/tests.rs` 增加组头槽位隔离、组内追加后的块重建与行数一致性。

## 6. 测试与验收

依赖：步骤 1–5。先写/改测试，再实现；测试文件按被测模块职责拆分。

### 6.1 快照与几何

- [ ] 两个连续 `read`：1 行组头 + 2 行路径；`render_rows == layout.total_rows`。
- [ ] 三个连续 `edit`：组头 + 三个调用体；调用间 0 空行，组前后各 1 空行。
- [ ] 正文/思考打断：两组独立；跨消息不合并。
- [ ] `edit` + `write`、`read` + `search` 不合并。
- [ ] 单调用不出现 `1 call` 组头。
- [ ] 运行中追加同种调用：末项连接符正确，总行数正确。
- [ ] 宽度 0、极窄宽度、超长路径：不越界、不换行、不死循环。

### 6.2 工具体

- [ ] 每个工具模块的 headerless 绘制行数等于 `body_rows`。
- [ ] 组内首/末调用连接符：`├─` / `└─`；调用体内 `│` 保留。
- [ ] 单调用标题状态移动到首节点尾部后不丢失；`read` 的标题状态保留在组头/单体标题，不移动到路径行。
- [ ] 失败/中断调用在组内的连接符与状态颜色正确，错误正文不被覆盖。
- [ ] 命令参数/输出、MCP 参数/结果、web_search 查询/结果的独立点击面仍然成立。

### 6.3 `read`

- [ ] 完成态：`■ Read · file` + 一条路径；标题保留 `file`/`folder`/`image`，不显示 `N lines`/`output limited`。
- [ ] 目录 read：`■ Read · folder · N files` + 目录路径。
- [ ] 超长路径单行尾部 `…`，无第二行、无 `… N more lines`。
- [ ] 失败/中断：一条错误，颜色正确。
- [ ] read 不登记点击面。

### 6.4 历史与缓存

- [ ] `/resume` 重进会话分组与 read 单条路径一致。
- [ ] 组头槽位与调用体槽位不冲突。
- [ ] 组内追加/删除调用时缓存正确失效，快照与渲染行数一致。
- [ ] 滚动、置顶行、命中矩形与快照同源。

### 6.5 验证命令

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`（至少覆盖本次受影响的 `history`、`tool`、`cache`、`transcript` 套件）
- [ ] 用户运行 `cargo run` 人工验收：连续 read、连续 edit、命令组、失败混组、极窄宽度、超长路径。

## 7. 文档同步

- [ ] 更新 `docs/standards/tui-design-requirements.md` 的 lx 聊天工具块描述：
  - 连续同名工具调用合并为“一个组头 + 逐调用树体”；
  - 组内不空行、组间空一行；
  - `read` 标题保留种类/目录数，标题下方只显示一条路径；
  - 组头不可点、调用体独立展开；
  - 替换旧版“合并文件工具组”的过时描述。
- [ ] 如目录职责有新增文件，同步更新 `docs/standards/project-directory-structure.md` 的 lx history 条目。
- [ ] 不重写 `docs/feature/lx-file-tool-diff/` 的历史记录；在该文档或新文档中注明其合并卡描述已被本设计取代即可。
- [ ] 完成后将本目录 `design.md` / `task.md` 状态改为“已实施”，记录验证结果。

## 8. 风险与回滚

- 风险：组内位置变化未进块指纹，导致旧 `└─` 残留或行数漂移。回滚点：步骤 5 未通过前不合并提交。
- 风险：`read` 单条路径隐藏 `output limited` 造成截断不可见。审核可改为同一行尾部追加提示。
- 风险：headerless 改造触及全部工具子模块，回归面大。按工具逐个改造并保持单调用路径不变，先 read/search/edit/write，再 command，最后 plan/ask/subagent/mcp/web_search。
- 风险：文件行数超限。合并逻辑优先放 `history/tool_group.rs` 或 `history/tool/group.rs`，不堆进 `history.rs` / `tool.rs` 根部。
- 回滚：本功能不涉及 DB 迁移；代码回滚只需还原 UI 快照/绘制改动，历史数据仍可正常加载。

## 9. 实施结果（2026-11-11）

- 新增 `src/ui/main_content/lx/history/tool/group.rs`：分组键/形态、组头聚合、调用体行数与点击面、headerless 分派、组内连接符。
- `history/snapshot.rs` 的 `steps()` 产出组头块与逐调用体块；组内不累加空行，组与前后块仍空一行。
- `paint.rs` 增加 `ToolGroupHeader` / `ToolBody` 来源；`cache/signature.rs` 增加组头与调用体指纹；`cache.rs` 增加组头保留槽位。
- 全部工具模块新增 `draw_body`；单调用仍走原 `Step` 路径，组内调用跳过标题。
- `read` 只显示标题状态 + 一条路径；失败/中断为单行尾部省略。
- 更新 `history/tests/tool_geometry.rs`、`cards.rs`、`tool/tests/{extras,structure,width}.rs` 的旧断言。
- 新增分组测试：连续 read/edit、命令退出状态、搜索命中合计、计划快照、ask 不合并、write_stdin 按动词分组、组头无点击面。
- 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿（5154+ 单元测试与集成测试）。
