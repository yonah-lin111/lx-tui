# lx 连续同种工具调用合并：代码与文档审查报告（Review）

> 日期：2026-11-11
> 分支：`feat/lx-tool-grouping`
> 审查范围：`src/ui/main_content/lx/history/`（`snapshot`、`paint`、`cache`、`tool` 及全部工具子模块）、`src/ui/text/agent.rs`、`docs/standards/`、`docs/feature/lx-tool-call-grouping/`。

---

## 审查问题总览（按优先级 P0 → P2 排序）

| 级别 | 编号 | 模块 | 关键问题摘要 | 状态 |
| --- | --- | --- | --- | --- |
| **P0** | P0-1 | `tree.rs` / `file_diff.rs` / `group.rs` / 各工具子模块 | 合并组二级子树 8 列前缀（`TREE_NESTED_*`）与 5 列排版宽度（`TREE_COLS`）失配，导致折行处每行右侧被静默裁切 3 列且窄屏吞字 | 已修复 |
| **P1** | P1-1 | `tree.rs` / `file_diff.rs` / `file_summary.rs` / `subagent.rs` / `web_search.rs` / `mcp.rs` | 合并组内失败/中断/运行中调用的首节点连接符与状态文字未按语义色渲染，部分异常分支丢失状态或错误行 | 已修复 |
| **P1** | P1-2 | `group.rs` / `tool.rs` | 组头 `worst_status`、`aggregate_status` 与 `display_verb` 忽略后台运行进程、子 agent 状态与未结算流式态 | 已修复 |
| **P1** | P1-3 | `group.rs` / `plan.rs` / `snapshot/steps.rs` | `update_plan` 快照合并进度计数被拼到步骤行而非组头；`GroupMode::Snapshot` 未结算尾项覆盖上一份已生效快照 | 已修复 |
| **P1** | P1-4 | `mcp.rs` / `subagent.rs` / `tree.rs` / `web_search.rs` | 无参数 MCP 调用在合并组内缺少一级首节点导致耗时拼入结果首行；多目标 `wait_agent` 第 2+ 个平级目标被误缩进为二级子节点 | 已修复 |
| **P1** | P1-5 | `docs/standards/*` / `docs/feature/lx-tool-call-grouping/*` | 规范文档与 `design.md` / `task.md` 部分描述滞后于二级树与 diff 底色实现，`task.md` 复选框未勾选 | 已修复 |
| **P2** | P2-1 | `group.rs` / `tree.rs` / `text/agent.rs` | `search` 组头遗漏 `results truncated`、`edit/write` 全未知计数显示 `+0 -0 (?)`、`append_status` 窄余量截断出裸 `"…"` 且只看首行、组头状态文案硬编码 | 已修复 |
| **P2** | P2-2 | `cache/tests.rs` / `tests/tool_geometry.rs` | `cache/tests.rs` 缺少组头槽位隔离与追加失效测试；`tests/tool_geometry.rs` 超出 1000 行建议上限 | 已修复 |

---

## 详细审查分析

### [P0-1] 合并组二级子树 8 列前缀（`TREE_NESTED_*`）与 5 列排版宽度（`TREE_COLS = 5`）失配

- **定位**：
  - `src/ui/text/agent.rs:38-57`（一级前缀 5 列，二级前缀 `TREE_NESTED_*` 8 列）
  - `src/ui/main_content/lx/history/tool/tree.rs:143-159, 184, 269-304`
  - `src/ui/main_content/lx/history/tool/file_diff.rs:375-421`
  - `src/ui/main_content/lx/history/tool/group.rs:136-163`
  - `src/ui/main_content/lx/history/tool/{command,search,web_search,mcp,subagent}.rs`
- **关键点**：
  1. 合并组（`GroupMode::List`）中，调用体的二级子节点（`node != first_node` 以及 `file_diff` 的 diff 行）使用 **8 列**前缀（`TREE_NESTED_*`），但 `tree::layout`、`file_diff::file_body` 及 `muted_markdown_lines` 仍统一按一级树 **5 列**前缀（`width - 5`）计算内容列宽与折行。
  2. `tool::body_rows` 与 `tool::body_card_regions` 直接复用单卡的 5 列排版结果。
  3. 极窄宽度 `width = 6..=8` 时，`keeps_tree(width)`（`width > 5`）仍为 `true`，8 列前缀直接占满或超出整行宽度，二级子节点正文 0 列可见。
- **错误原因**：
  - 在将合并组树体升级为二级嵌套树时，仅在 `draw_body` 阶段把二级节点的前缀字符串从 5 列替换为 8 列，未把二级缩进列宽（`TREE_NESTED_COLS = 8`）同步回传给排版折行阶段与 `body_rows` / `body_card_regions`。由于 `put_segments` 按 `rect.width` 硬裁剪，折行后的二级节点每行达到 `8 + (width - 5) = width + 3` 列，末尾 3 列字符被直接裁掉且复制文本一并丢失。
- **修复方案**：
  - 在 `tree` 中引入 `TREE_NESTED_COLS = 8` 与 `layout_in_group`（或带 `nested: bool` 的排版入口）：当处于合并组二级嵌套模式时，首节点按 `width - 5` 折行，后续二级子节点按 `width - 8` 折行（极窄阈值同步为 `width > 8`）。
  - 各工具模块的 `muted_markdown_lines`、`file_diff::file_body`、`body_rows`、`body_card_regions` 与 `draw_body` 统一按组内二级宽度排版，确保快照行数、点击热区与实际绘制行数 100% 一致且零字符裁切。

---

### [P1-1] 合并组内失败/中断/运行中调用的连接符、状态文字未按语义色渲染，部分异常分支丢失状态或错误行

- **定位**：
  - `src/ui/main_content/lx/history/tool/tree.rs:307-347`
  - `src/ui/main_content/lx/history/tool/file_diff.rs:330`
  - `src/ui/main_content/lx/history/tool/file_summary.rs:209-233, 463-478`
  - `src/ui/main_content/lx/history/tool/search.rs:106-112, 181-184`
  - `src/ui/main_content/lx/history/tool/web_search.rs:105-125, 200-203, 289-292`
  - `src/ui/main_content/lx/history/tool/mcp.rs:86-100, 187-190, 263-266`
  - `src/ui/main_content/lx/history/tool/subagent.rs:259-272, 516-524`
- **关键点**：
  1. 规范（`lx-message-block-display.md` §3.3、`design.md` §3.2/§4.2）要求组内失败/中断调用的首节点连接符与状态文字使用语义色（`agent_fail` / `agent_warn`），运行中状态保持橙黄，错误正文保持正常前景色 `style::text()`。
  2. 当前 `tree::draw` 与 `file_diff` 恒用灰色 `style::chat_notice_connector()` 绘制连接符，`tree::append_status` 恒用暗灰 `style::agent_step_detail()` 绘制状态后缀。
  3. `file_summary`、`search`、`web_search`、`mcp` 在 `Failed | Interrupted` 时 `status_of` 返回 `None`；当无 `error` 文本或错误在二级节点时，组内首节点看不出失败或中断。
  4. `web_search` 与 `mcp` 将错误回退文本送入 `muted_markdown_lines`（变成暗灰），`subagent::error_node` 漏判 `Interrupted` 且将错误文本染红，同时 `subagent::draw_body` 在 `view(call) == None`（工具在生成详情前失败）时直接 `return` 提前退出，吞掉错误节点。
- **错误原因**：
  - 单卡模式下失败/中断颜色由卡片标题行承担，合并组去掉单卡标题行后，`tree::Row` 未携带连接符样式覆盖与状态后缀样式，各模块直接照搬了单卡的 `title_status` 返回值。
- **修复方案**：
  - 为 `tree::Row` 增加 `prefix_style: Style`（默认 `style::chat_notice_connector()`），并提供 `apply_first_node_connector_style` 与带样式的 `append_status_styled`；`file_diff` 的 `file_header` 在合并组下同样按调用状态设置首节点连接符颜色。
  - 失败/中断调用在组内首节点连接符与状态后缀统一使用 `style::agent_fail()` / `style::agent_warn()`，运行中使用存活橙色 `style::agent_running()`；当失败/中断且首节点并非错误文本本身时，补齐 `"failed"` / `"interrupted"` 状态后缀。
  - 修复 `subagent::draw_body` 在 `view(call) == None` 时仍正常绘制 `layout`（含 `error_node`），并将 `web_search`、`mcp`、`subagent` 的错误回退正文统一为 `style::text()`。

---

### [P1-2] 合并组头 `worst_status`、`aggregate_status` 与 `display_verb` 忽略后台运行进程、子 agent 状态与未结算流式态

- **定位**：
  - `src/ui/main_content/lx/history/tool/group.rs:21-28, 44-61, 75-92, 321-368`
  - `src/ui/main_content/lx/history/tool.rs:666-676`
- **关键点**：
  1. 后台运行的 `exec_command` / `write_stdin` 在工具返回后 `call.status` 为 `ToolStatus::Completed`，但进程本身仍在运行（`view.state.is_running() == true`）；`spawn_agent` 返回后子 agent 的运行/失败/中断状态保存在 `view.agent_status`。`worst_status` 与 `aggregate_status` 只检查 `call.status`，导致组头方块静止变绿且不显示 `· N running`。
  2. `display_verb` 在 `call.detail == None`（`InputStreaming` / `Pending` / 无详情失败）时回退到 `verb_label`，而 `verb_label` 只覆盖了 5 个基础工具名，导致流式阶段组头暴露 `exec_command`、`update_plan`、`spawn_agent` 等内部名称，且 `write_stdin` 在结算前分组键为 `"write_stdin:write_stdin"`、结算后跳变为 `"write_stdin:Send input"`。
- **错误原因**：
  - 组头状态推导未复用 `command` 与 `subagent` 单卡的状态判定逻辑；`verb_label` 未覆盖全部内置工具与 MCP 前缀，`write_stdin` 未在 `detail == None` 时从 `arguments_json` 解析动作类型。
- **修复方案**：
  - 新增 `effective_call_status(call)`，综合 `call.status`、`Command`/`Interaction` 的 `ProcessStateView` 以及 `Subagent` 的 `agent_status` 計算组头最差状态与运行/失败/中断聚合计数。
  - 补全 `display_verb` / `verb_label` 对全部内置工具、子 agent 工具、MCP 工具（`mcp__<server>__<tool>`）及 `write_stdin` 参数（`kill` / `chars`）的回退映射，消除流式态分组键跳变与内部名称泄漏。

---

### [P1-3] `update_plan` 快照合并进度位置错误及 `GroupMode::Snapshot` 未结算尾项覆盖有效快照

- **定位**：
  - `src/ui/main_content/lx/history/tool/group.rs:370-381`
  - `src/ui/main_content/lx/history/tool/plan.rs:150-154`
  - `src/ui/main_content/lx/history/snapshot/steps.rs:61-64`
- **关键点**：
  1. `lx-message-block-display.md` §5.7 规定 `update_plan` 快照合并组头为 `■ Plan · 3 updates · 2/3 completed`，正文只渲染步骤列表且步骤行不拼 `·`；但当前实现未在 `group::aggregate_metric` 中汇总计划进度，反而在 `plan::draw_body` 调用 `tree::append_status` 将 ` · 2/3 completed` 拼到了第一条计划步骤末尾。
  2. `snapshot/steps.rs` 在 `GroupMode::Snapshot` 下无条件只取最后一个调用 `end - 1`；当组尾新发起的 `update_plan` 或 `list_agents` 尚处于 `InputStreaming`/`Pending` 时，会用空的占位行覆盖掉前一次已生效的完整快照。
- **错误原因**：
  - `plan::draw_body` 机械套用了列表合并工具的 `append_status` 模式；`steps.rs` 未检查组尾调用是否已产出有效快照详情。
- **修复方案**：
  - 在 `group::aggregate_metric` 中为 `update_plan` 提取组内最新有效计划快照的 `X/Y completed` 拼入组头，移除 `plan::draw_body` 的 `tree::append_status`。
  - 在 `snapshot/steps.rs` 的 `GroupMode::Snapshot` 分支中，从后往前选取最后一个具有有效快照详情的调用（`update_plan` 含 `ToolDetailView::Plan`，`list_agents` 已完成或含非空 `agents`；若全部未就绪则回退末项）。

---

### [P1-4] 无参数 MCP 调用与多目标 `wait_agent` 在合并组内的树结构异常

- **定位**：
  - `src/ui/main_content/lx/history/tool/mcp.rs:29-31, 133-167, 248-267`
  - `src/ui/main_content/lx/history/tool/subagent.rs:300-334, 502-525`
  - `src/ui/main_content/lx/history/tool/tree.rs:269-304`
  - `src/ui/main_content/lx/history/tool/web_search.rs:141-175, 274-293`
- **关键点**：
  1. 当无参数的 MCP 工具（如 `list_projects({})`）在合并组内展示时，`arguments_section` 返回 `None`，`result_section` 成为唯一节点（`first_node`），导致耗时 ` · 4ms` 被拼进 JSON/Markdown 结果首行（如 `├─ { · 4ms`），且结果块未缩进为二级子节点；若运行中尚无结果则退化为空连接符行。
  2. 当单次 `wait_agent` 同时等待多个目标并处于连续合并组内时，`built.nodes` 中每个目标都是独立 `Node`，`tree::apply_group_connector` 将第 1 个目标作为一级分支 `├─`，而将第 2..N 个平级目标缩进成了第 1 个目标的二级子节点 `│  └─`。
- **错误原因**：
  - `mcp.rs` 未在合并组无参场景下提供一级首节点占位；`tree::apply_group_connector` 默认假定只有 `node == first_node` 是一级节点，未考虑 `wait_agent` 存在多个一级平级目标节点的形态。
- **修复方案**：
  1. 按用户确认方案：在合并组（`in_group = true`）下，若 MCP 调用无参数（`view.arguments.is_empty()`），构造 `(no arguments)` 作为一级首节点（承载耗时/截断状态），使结果块统一缩进为二级子节点 `│  └─`；`web_search` 在查询为空时同样提供兜底首节点。
  2. 按用户确认方案：为 `tree::apply_group_connector_with_roots`（或指定一级节点数量 `root_nodes: usize`）支持多一级节点——`wait_agent` 的所有目标行均作为一级平级分支（非末调用或后接报告时用 `  ├─ `，末调用且为最后一个节点时用 `  └─ `），仅展开后的 `Report:` 与错误节点缩进为二级子树。

---

### [P1-5] 规范文档与 `design.md` / `task.md` 滞后于最新实现

- **定位**：
  - `docs/standards/tui-design-requirements.md:127, 129`
  - `docs/standards/lx-message-block-display.md:47-48, 161, 249, 444-449`
  - `docs/feature/lx-tool-call-grouping/design.md:44-46, 102, 183, 193-196, 203`
  - `docs/feature/lx-tool-call-grouping/task.md:9-170`
- **关键点**：
  1. `tui-design-requirements.md` 仍写 diff 底色与展开灰底“铺满 100% 宽度（含行首缩进）”，与“从 `└─` 右侧 1 列空白起铺至行尾、不覆盖左侧树连接符”的实际实现冲突。
  2. `lx-message-block-display.md` 与 `design.md` 的文字规则未完整记录合并组二级子树（8 列 `TREE_NESTED_*`）、无参 MCP 首节点 `(no arguments)`、以及 `snapshot/steps.rs` 与 `paint/legacy.rs` 模块拆分与最新函数签名。
  3. `task.md` 状态虽标为“已实施”，但全文复选框仍为 `- [ ]`。
- **错误原因**：
  - 前两轮交互调整后仅局部修改了 ASCII 示例，未全量同步规范描述与任务清单状态。
- **修复方案**：
  - 同步更新 `tui-design-requirements.md`、`lx-message-block-display.md`、`design.md`、`task.md`（勾选全部已完成任务并记录本轮 Review 修复结果），并同步主工作区文档副本。

---

### [P2-1] 组头聚合指标、窄宽 `append_status` 与硬编码文案问题

- **定位**：
  - `src/ui/main_content/lx/history/tool/group.rs:359-416`
  - `src/ui/main_content/lx/history/tool/tree.rs:307-326`
  - `src/ui/text/agent.rs`
- **关键点**：
  1. `group::search_metric` 未汇总 `summary.truncated`（`results truncated`），且在 `summary.matches` 为 `None` 时未回退读取 `SearchDetailView.matches.len()`。
  2. `group::edit_write_metric` 在组内所有调用计数未知且尚无增删行数时拼出 `+0 -0 (?)`，应与单卡一致仅显示 `(?)`。
  3. `tree::append_status` 在首行剩余列宽仅 1~3 列时截断出误导性 `"…"` / `" ·…"`，且当首节点折成多行时只尝试追加到第 0 行而非首节点的最后一行。
  4. `group.rs` 中 `"failed"`、`"interrupted"`、`"running"`、`" (?)"` 直接硬编码，违反 `code-writing-standards.md` 文案集中管理规范。
- **错误原因**：
  - 组头聚合逻辑与单卡状态逻辑未完全对齐；`append_status` 缺少最小追加宽度判断与首节点末行定位；新增文案未抽入 `src/ui/text/agent.rs`。
- **修复方案**：
  - 补齐 `search_metric` 的截断标记与详情命中数回退；修正 `edit_write_metric` 在全未知且无计数时返回 `(?)`。
  - `append_status` 定位首节点的最后一个物理行，且当剩余列宽不足以放下 `" · "` + 至少 1 个状态字符 + `"…"`（`< 5` 列）时整段省略状态后缀。
  - 将组头聚合状态文案与无参 MCP 占位文案收口至 `src/ui/text/agent.rs`。

---

### [P2-2] 缓存测试缺口与测试文件行数超标

- **定位**：
  - `src/ui/main_content/lx/history/cache/tests.rs`
  - `src/ui/main_content/lx/history/tests/tool_geometry.rs`（1025 行）
- **关键点**：
  1. `task.md` 明确要求在 `history/cache/tests.rs` 增加组头槽位隔离（`GROUP_HEADER_SLOT_BASE`）与组内追加调用后旧末项 `└─ -> ├─` 重建测试，但 `cache/tests.rs` 尚未覆盖。
  2. `history/tests/tool_geometry.rs` 已达 1025 行，超过 `code-writing-standards.md` 单测试文件建议上限（1000 行）。
- **错误原因**：
  - 分组测试集中在 `tool_geometry.rs` 编写，未拆分场景子文件，也未在 `cache/tests.rs` 补充缓存层单测。
- **修复方案**：
  - 在 `src/ui/main_content/lx/history/cache/tests.rs` 新增组头槽位隔离与组内动态追加缓存失效测试。
  - 将 `src/ui/main_content/lx/history/tests/tool_geometry.rs` 拆分为 `tool_geometry.rs`（基础工具几何）与同级子模块（如 `tool_grouping.rs`），使每个测试文件均保持在 600 行以内。
