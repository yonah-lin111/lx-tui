# lx 连续同种工具调用合并：设计

> 状态：**已实施**（2026-11-11）。自动化验证见 `task.md` 文末；TUI 外观仍由用户运行 `cargo run` 验收。
> 日期：2026-11-10。
> 目标页面：lx / Native Agent 对话时间线。
> 相关规范：`docs/standards/project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`lx-message-block-display.md`。

## 1. 已确认的产品决策

| 决策 | 已确认行为 |
| --- | --- |
| 合并格式 | 采用**格式 1「标题上提」**：组内只保留一个 `■ 动词`，下面按调用顺序展开每个调用的树体。 |
| 合并范围 | 同一条 assistant 消息内、相邻、工具名完全相同的 `Tool` 步骤，且组内至少有 2 个调用；被正文/思考/不同工具名打断即分组。`ask` 不合并；`update_plan`/`list_agents` 快照合并（只显示最新）；`write_stdin` 按显示动词分组。 |
| 组头 | `■ 动词 · N calls`（快照合并为 `· N updates`）；`edit`/`write` 另带组内增删合计。组头不可点击，不引入“整组折叠”。 |
| 组内调用 | 每个调用保留现有树体（路径、命中、diff、命令输出等），调用之间**不空行**；每个调用仍可独立折叠。 |
| `read` 展示 | 保留正常标题状态（`file` / `folder · N files` / `image`）；**标题下方只显示一条路径**，路径不换行，结果正文与参数事实不占第二行。 |
| 跨消息 | 绝不合并。 |
| 历史一致性 | 只改派生展示，不改数据库结构；实时与 `/resume` 重进会话必须一致。 |

### 1.1 对 `read` 详情的口径

用户最终确认：标题保留种类状态，标题下方**只显示一条路径**。

```text
■ Read · file
  └─ src/event/mod.rs
```

因此：

- 标题保留 `Read · file` / `folder · N files` / `image`；目录条目数照常显示。
- 详情只有一条路径；路径超宽时单行尾部省略，不软换行。
- 不显示行范围、实际行数、`output limited`、`… N more lines`，也不渲染读取结果正文。
- 失败/中断：保留一条错误/中断原因（单行尾部省略），不显示路径。
- `read` 没有可展开内容，不登记点击面。
- 合并后每个调用只显示自己的一条路径，不扁平混合成多行参数。

## 2. 当前实现事实

以下位置是设计时定位，不是未来固定行号。

| 位置 | 当前行为 | 本次影响 |
| --- | --- | --- |
| `src/ui/main_content/lx/history/snapshot.rs` `steps()` / `build_step()` | 每个 `Tool` 步骤独立建块，块间由 `spacer()` 空一行 | 需要识别连续同名 Tool 组，并拆出组头块与调用体块 |
| `snapshot.rs` `tool_cards()` | 按单个 `call` 的 `CardSpan` 登记点击面 | 合并后每个调用体仍需独立点击面 |
| `paint.rs` `BlockSource` / `BlockPaint` | `Step` 定位单个步骤；`Legacy` 缓存行数与点击面 | 增加组头/调用体定位，保持绘制只读 |
| `tool.rs` `rows()` / `draw_animated()` / `card_regions()` | 行数含 1 行标题；绘制按工具类型分派 | 增加“只画树体、不画标题”的入口与仅树体点击面 |
| `tool/file_summary.rs` `title_status()` / `detail_layout()` | `read` 标题显示 `file`/`folder`，详情为路径 + 事实 | 保留标题状态；详情只留一条路径，隐藏范围/计数/事实 |
| `history/cache/signature.rs` `step_signature()` | 单步骤指纹，不含“组内位置” | 组头与调用体必须把组位置/邻居纳入指纹 |
| `history/tests/tool_geometry.rs` | 现有测试断言连续文件工具各自一张卡 | 改为断言合并组、组内连接符与独立点击面 |

当前 `docs/standards/tui-design-requirements.md` 仍保留旧版“连续同类文件调用合并成一张卡”的描述，但与现有代码/测试不一致；实现完成后同步修正为本文口径。

## 3. 视觉规格

### 3.1 组头

```text
■ Read · 3 calls
```

- 方块与动词沿用工具档位：运行中橙黄呼吸，完成翡翠绿，失败红，中断黄，取消沿用角色色。
- `N calls` 用 dim 元信息色；`N >= 2`，不出现 `1 call` 组。快照合并工具写 `N updates`。
- `edit`/`write` 追加组内合计：`■ Edit · 2 calls · +8 -4`；任一计数未知或被裁时合计后追加 `(?)`，绝不拿 0 冒充。
- `search` 可追加组内总命中数 `· 12 matches`；未知时不写。
- `mcp`/`web_search`/`plan`/`ask`/`subagent` 默认不追加聚合指标，各调用自己的状态跟随调用行。
- 组头只有 1 个物理行，超宽尾部省略；不换行。
- 组头不可点击、不登记点击面；展开/折叠只属于调用体。

### 3.2 组内调用

组内每个调用的**第一个可见树节点**就是这次调用的“摘要行”，沿用各工具现有树体：

```text
■ Read · 3 calls
  ├─ src/config.rs
  ├─ src/app/state.rs
  └─ src/ui/grid.rs
```

```text
■ Edit · 2 calls · +8 -4
  ├─ src/a.rs (+5 -2)
  │  ├─ 1 - old
  │  └─ 1 + new
  └─ src/b.rs (+3 -2)
     └─ …
```

```text
■ Run command · 3 calls · 1 running
  ├─ $ cargo check · Exited 0 · 1.2s
  │  └─ Finished `dev` profile …
  ├─ $ cargo test · Running
  │  └─ open task panel · live output
  └─ $ cargo clippy · Exited 0 · 0.8s
```

规则：

- 组内**不插空行**；组与前后块之间仍空一行，空行只由 `history::snapshot` 累加。
- 非最后一个调用的首节点连接符用 `├─`，最后一个调用用 `└─`；调用体内部节点继续沿用 `tree` 的 `├─`/`└─`/`│` 规则。
- 调用体自己的可折叠内容（diff 预览、命令输出、搜索命中、MCP 结果等）保持在调用体内部，展开状态按 `(message_id, call_id)` 独立保存，互不影响。
- 原来的单调用标题状态（`Exited 0 · 241ms`、`3 matches`、`1/3 completed`、`2 questions` 等）不丢失：移动为该调用首节点尾部的 ` · <status>`；`read` 的标题状态留在组头/单体标题，不追加到路径行。
- 失败/中断调用的首节点连接符与状态文字用对应语义色；错误正文仍用正常前景色。组头方块取组内最差状态：失败 > 中断 > 运行中 > 取消 > 完成。
- 组头状态文字只在组内存在非完成态时追加聚合提示：`· 1 running`、`· 1 failed`、`· 1 interrupted`；全部完成时不写。
- 极窄宽度：组头只保留方块与动词（放不下 `· N calls` 时整段省略）；调用体沿用现有极窄降级，不画树体前缀时也不越界。

### 3.3 `read` 单条路径

```text
■ Read · file
  └─ src/event/mod.rs
```

- 标题保留 `Read · file` / `Read · folder · 12 files` / `Read · image`。
- 详情只有路径；不显示 range、`N lines`、`output limited`，不渲染结果正文；目录条目数只保留在标题。
- 路径按可用列单行尾部省略；不产生第二物理行，也不登记“展开更多”的点击面。
- 目录 `read` 也只显示目录路径。
- 失败/中断显示单条错误原因（尾部省略），连接符按语义色；此时不显示路径，与原行为一致。
- 单次 `read` 保留 `■ Read · file` 标题；连续 `read` 合并后只在组头显示一次 `■ Read · N calls`。

## 4. 分组与状态规则

### 4.1 分组

从 `message.steps` 的当前下标向后扫描：

1. 连续读取 `AssistantStepView::Tool { call, .. }`。
2. 仅当相邻调用的**分组键**相同时纳入同一组：
   - 普通工具：`call.name` 完全相等；
   - `write_stdin`：`(call.name, 显示动词)` 相等（`Send input` / `Read command output` / `Stop command` 不混组）。
3. 遇到 `Part`（正文/思考）、不同分组键、消息结束即停止。
4. 组内调用数 `>= 2` 才启用合并；单调用保持现有单卡布局（但 `read` 应用单条路径规则）。
5. 跨消息不合并；`/resume` 重进会话按同一份 steps 重新分组。

合并形态：

- **列表合并**：默认，按顺序显示每个调用。
- **快照合并**：`update_plan`、`list_agents` 只显示最后一次调用的正文，组头写 `· N updates`；避免同一计划的旧快照铺满列表。
- **不合并**：`ask` 是阻塞交互，每次问答独立成块。

不按“工具族”合并：`read` 与 `search` 不合并，`edit` 与 `write` 不合并。

### 4.2 状态

- 组头方块取最差状态；运行中继续按 `loading_frame` 呼吸，动画停止时实心。
- 每个调用行保留自己的状态文字与颜色；组头聚合提示只用于快速扫读，不能替代调用行状态。
- 失败/中断调用不从组内剔除，也不因为与成功调用相邻而被“平均”成完成态。
- 组内某个调用仍在运行、组头已因另一调用失败变红时，运行中调用自己的方块/状态仍保持橙黄，避免错误归因。

## 5. 交互

- 组头：不可点击，不登记 `CardKind::Tool`。
- 调用体：沿用现有卡片点击面；点某个调用体只切换该调用的折叠状态。
- 命令卡片参数区/输出区仍各自独立；MCP/web_search 的参数/结果仍各自独立。
- `read` 只有一条路径，没有可展开内容，不登记点击面；点击路径不折叠、不弹窗。
- 选中、复制、拖拽与现有历史区一致；组头与调用体都是普通网格文本，可被选区覆盖。
- 滚动条、置顶行、`↓ jump to bottom`、历史命中几何全部继续读同一份快照，不新增第二套行数口径。

## 6. 实现架构

### 6.1 推荐结构：组头块 + 逐调用体块

不把多个调用合成一个大 `BlockPaint`，而是保持“一个调用一个缓存块”，额外增加一个 1 行的组头块：

```text
SnapBlock[group header]   rows = 1            source = ToolGroupHeader
SnapBlock[call 1 body]    rows = body_rows(1) source = ToolBody { group: Some(...) }
SnapBlock[call 2 body]    rows = body_rows(2) source = ToolBody { group: Some(...) }
```

理由：

- 每个调用体继续复用现有 `tool::rows`/`draw`/`card_regions` 与缓存失效粒度；
- 组头只有 1 行，不需要把多个工具视图塞进一个巨型绘制数据；
- 调用体块之间不加 spacer，块边界仍可被滚动/命中精确处理；
- 单调用路径完全不变，降低回归面。

### 6.2 快照

- `snapshot::steps()` 负责分组扫描；`build_step()` 继续处理单个 Part/单调用。
- 新增 `build_tool_group_header()` 与 `build_tool_body()`：
  - 组头块：`rows = 1`，`BlockPaint::Legacy { rows: 1, spans: [] }`，绘制时现算方块/状态。
  - 调用体块：`rows = max(1, tool::rows(call, width, folds) - 1)`，`BlockPaint::Legacy { rows, spans: tool::body_card_regions(...) }`。
- 组内调用体块与组头块连续放置，不调用 `spacer()`；组结束后由下一个块正常加空行。
- 空调用体（没有任何树体）必须补一行兜底，保证每个调用在组内可见。

### 6.3 绘制

- `BlockSource` 增加：
  - `ToolGroupHeader { message_index, first_step, count }`
  - `ToolBody { message_index, step_index, group: Option<GroupSlot> }`
- `GroupSlot` 至少包含 `last: bool`（连接符）与 `count`（缓存指纹）；首/尾位置变化必须使块键失效。
- `tool.rs` 新增：
  - `group_header(call, count, width, ...) -> Vec<Segment>`：`■ 动词 · N calls` + 聚合状态。
  - `draw_body_animated(...)`：按工具类型分派到各模块的“只画树体”入口。
  - `body_card_regions(...)`：仅树体的点击面几何。
- 各工具子模块新增 `draw_body`/`body_layout` 或给现有 `draw` 增加 `headerless` 参数：
  - 跳过标题行；
  - 保留原有树体、折叠提示、行底色与链接；
  - 将原标题状态追加到首节点（`read` 除外：其标题状态保留在组头/单体标题）；
  - 非末调用把首行 `└─` 改为 `├─`；`file_diff` 的首段连接符同样处理。
- `read` 的 `file_summary`：
  - `title_status(Kind::Read)` 保留 `file` / `folder · N files` / `image`；
  - `detail_layout(Kind::Read)` 只构造一个路径节点，路径按 `tree::content_width(width)` 单行尾部省略；
  - 不构造 `fact_nodes` 中的 `output limited` 等第二行内容，不登记展开点击面。
- `agent_links` 等行内命中若原来相对标题偏移，组内需要减 1（或提供 headerless 变体），避免链接热区下移一行。

### 6.4 缓存与指纹

- 组头块的 `BlockKey` 使用保留槽位（如 `GROUP_HEADER_SLOT_BASE - first_step`），避免与首个调用体的 `step_index` 撞槽。
- 组头指纹覆盖：组内每个调用的 `call_id`、`name`、状态、错误、摘要/详情中影响状态文字的字段、`count`。
- 调用体指纹覆盖：原 `step_signature` + `GroupSlot`（是否末项、组大小）+ `read` 单条路径规则版本。
- 组内追加/删除调用会改变相邻调用的 `last` 与组头 `count`，必须触发相关块重建；不能只依赖列表指纹“步骤数变化”而漏掉组内位置变化。

## 7. 兼容与边界

- 数据库、工具执行、结果 JSON、`/resume` 投影不改；UI 只读现有 `steps` 与类型化详情。
- 旧会话没有新字段也能分组；`read` 旧记录同样只显示路径。
- 极窄宽度（`width < 16` 等）沿用现有降级：组头可只剩 `■ Read`，调用体可丢弃树体前缀。
- 组内调用全部没有可展开内容时，组仍显示每个调用一行摘要，不制造空点击面。
- 组头 `N calls` 与状态文字放不下时整段省略，不截半个宽字符。
- 运行中调用追加到组尾时，原调用体的 `last` 从 true 变 false，必须重建连接符；不能出现两个 `└─`。
- 失败调用与成功调用同组时，组头红色但成功调用行保持绿色；错误文案不能被组头吞掉。

## 8. 测试与验收

### 8.1 快照/几何

- 两个连续 `read` 合并：1 行组头 + 2 行路径；`layout.total_rows` 与渲染行数一致。
- 三个连续 `edit`：组头 + 每个调用体；调用之间 0 空行；组前后各 1 空行。
- 正文/思考打断：两组独立，不跨块合并。
- 不同工具名（`edit` + `write`、`read` + `search`）不合并。
- 单调用不出现 `1 call` 组头。
- 组内每个调用点击面独立；点第一个只展开第一个。
- 运行中追加同种调用：旧末项连接符从 `└─` 变 `├─`，总行数正确。
- 宽度 0、极窄宽度、超长路径不越界、不换行、不死循环。

### 8.2 `read`

- 完成态显示 `■ Read · file` + 一条路径；不出现 `12 lines`、`output limited`、`… N more lines`。
- 目录 `read` 显示 `■ Read · folder · N files` + 目录路径。
- 超长路径单行尾部 `…`，不出现第二行。
- 失败/中断显示一条错误，连接符使用失败红/中断黄。
- `read` 不登记点击面，点击不改变布局。

### 8.3 历史与缓存

- `/resume` 重进会话得到相同分组与相同 read 单条路径结果。
- hover/loading 帧变化不使组块失效；组内位置变化使相邻块失效。
- 组头槽位不与调用体槽位冲突（缓存命中/覆写测试）。
- 滚动、置顶行、命中矩形与快照同源。

### 8.4 人工验收

- `cargo run` 观察：
  - 连续 3 个 `read` 只出现一个 `■ Read · N calls`；
  - 连续 `edit` 的组头计数与文件行；
  - 命令组内每个 `Exited` 状态；
  - 失败调用混在成功组内的颜色；
  - 极窄窗格与超长路径。
- 交互外观由用户运行确认；Agent 不代替人工验收。

## 9. 风险与待审核点

1. **`read` 单条路径的边界**：隐藏 `output limited` 会让执行阶段截断不再可见。当前按用户“标题下方只显示路径”的要求处理；若审核要求保留，可改为在同一行尾部追加 ` · output limited`，仍不产生第二行。
2. **`read` 错误行**：为保证故障可见，失败/中断保留一条错误；严格“只显示路径”会隐藏错误，建议不接受。
3. **分组范围**：默认覆盖所有同名工具；例外为 `ask` 不合并、`update_plan`/`list_agents` 快照合并、`write_stdin` 按显示动词分组。详细清单见 `docs/standards/lx-message-block-display.md`。
4. **组头聚合指标**：只对 `edit`/`write` 做增删合计、对 `search` 做命中合计；其余工具不做聚合，避免不同工具的状态语义被混在一起。
5. **旧规范冲突**：`tui-design-requirements.md` 仍有旧合并描述，实施时必须同步更新，避免下一位 Agent 按旧文档改回扁平文件组。
6. **文件行数**：预计触及 `snapshot.rs`、`paint.rs`、`tool.rs`、`cache/signature.rs`、`file_summary.rs` 及若干工具子模块；若单文件接近 1000 行，按职责拆到 `tool/group.rs` 或 `history/tool_group.rs`，不把合并逻辑堆进 `history.rs`。

## 10. 已确认

- [x] 接受 `read` 的“标题状态 + 单条路径”口径，以及失败时保留一条错误。
- [x] 接受默认合并所有同名工具；排除 `ask`，快照合并 `update_plan`/`list_agents`，`write_stdin` 按显示动词分组。
- [x] 接受组头只对 `edit`/`write` 显示增删合计、对 `search` 显示命中合计。
- [x] 接受组头不可点击、每个调用体独立展开。

## 11. 实施记录

- 新增 `history/tool/group.rs`：分组键、组头、headerless 绘制分派与组内连接符。
- `snapshot::steps` 产出组头块 + 逐调用体块；组内不调用 `spacer()`。
- 每个工具模块新增 `draw_body`；原单卡绘制路径保持不变。
- `read` 只保留标题状态与一条路径；失败/中断单行尾部省略。
- 组头与调用体分别使用独立缓存槽位/指纹；组内位置变化会使相邻块失效。
