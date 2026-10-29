# lx 消息列表工具树与输出预览：设计

> 状态：已完成并合入 `dev`。
> 范围：lx 消息列表中的全部模型工具卡片，以及用户 `!` 本地命令卡片。

## 1. 目标与已确认边界

1. 每张卡片保持单独的工具标题；其下所有参数、结果、状态和可展开内容使用 `├─` / `└─` / `│` 构成连续的树。`│` 是现有项目中与用户示例 `|` 对应的等宽竖线，随折行和展开行延长。工具卡片中不再用 ` · ` 拼接独立字段。
2. `read` 完成后，标题先说明目标：`Read file` 或 `Read folder`。文件详情只显示路径与**本次实际读到的行号范围**，如 `1-40`；目录详情只显示路径。截图中的 `18 matches` 是目录的 18 个条目被误标为搜索命中，必须从 `read` UI 移除。
3. 文件 `read` 卡片不展示文件正文。目录 `read` 卡片不展示目录条目正文。工具执行结果仍按现有规则送给模型；目录读取能力保留。
4. 长内容只在**消息列表**中做有界预览与点击展开。模型收到的文本、执行上限、DB 中已保存的内容和现有截断元数据保持原样。展开只显示已经保存的内容，不重新执行工具，也不访问文件系统。
5. 可见工具内容不出现合成标记 `…[truncated]`。预览省略使用单独的 `… N more lines` 行；执行或存储本身已限流而无法确定省略行数时，显示单独的 `output limited` / `results truncated` 状态行，不编造 `N`。
6. `!` 本地命令卡片也执行同一树形规则；保留现有参数区、输出区独立折叠及正在运行时的引导语义。

## 2. 当前实现与问题定位

### 2.1 数据流

```text
工具参数 -> chat::tools 执行 -> ToolExecutionResult
  ├─ format_for_model -> agent loop / 后续模型采样
  ├─ to_result_json -> DB agent_tool_call.result_json
  └─ to_summary -> ToolSummary -> app 状态 -> ui 历史卡片

DB 重载 -> ToolSummary::from_arguments / absorb_result_json
        -> ToolDetailView::from_result_json -> 同一 UI 历史卡片
```

`src/chat/tools/output.rs::format_for_model`、`to_summary` 和 `to_result_json` 是不同用途的投影。不得为了隐藏 UI 中的标记而改写模型结果或持久化 JSON。实时结算与历史重载必须得到同一张卡片。

### 2.2 `read` 的真实语义

- `src/chat/tools/schema.rs::read_parameters` 明确接受 `File or directory path`。
- `src/chat/tools/read.rs::read_file` 返回 `ReadKind::File`、`start_line`、`end_line`、`total_lines`、`truncated`；空文件范围为 `0/0`。
- `read_directory` 列举目录条目，`start_line/end_line` 是**条目序号**，不是文件行号；`total_lines` 是本次枚举到的条目数，且目录枚举存在输入上限。
- `src/chat/tools/output.rs::to_summary` 把目录的 `total_lines` 放入 `ToolSummary.matches`；`ToolSummary::absorb_result_json` 重载时也这样做；`file_summary::extras` 再把它渲染为 `matches`。截图的 `18 matches` 正是这一连串口径混用造成的误导。
- `ToolSummary.range` 当前对文件结果已有权威值；`offset` / `limit` 只作流式参数兜底。文件卡片应显示结果返回的 `start_line-end_line`，不能拿总行数替代。空文件显示 `0 lines` 或 `empty file`，不能显示无效的 `0-0` 范围。

### 2.3 现有卡片差异

| 卡片 | 当前可见内容 | 需要调整的点 |
| --- | --- | --- |
| `read` | `Read` + 路径、范围/行数或目录 `N matches` | 标题区分 file/folder；文件仅路径和范围；目录仅路径 |
| `search` | pattern/path/命中数同摘要；命中预览与结果级截断 | 字段逐行成树；命中数仅此工具使用；合成行截断标记不露出 |
| `edit` / `write` | 路径、增删计数、partial；差异卡片另有文件树 | 参数和计数分节点；差异文件树保留语义，统一主干与折行 |
| `exec_command` / `write_stdin` | 命令参数、状态与输出分别有行；输出可折叠 | 不同字段不用 `·`；参数/输出与主干对齐，展开时竖线连续 |
| `update_plan` / `ask` | 计划步骤、问答摘要/题干 | 元信息与步骤成为树节点，问答摘要不以 `·` 拼接 |
| `spawn_agent` / `wait_agent` / `close_agent` | agent 状态、报告与目标；部分使用 ` · ` | agent 字段拆节点；报告的折行与展开主干连续 |
| `!` 本地命令 | 标题有 `exit N · T.s`；参数/输出已有局部树 | 退出码和耗时移到树节点；独立折叠不变 |

上述卡片的渲染分派在 `src/ui/main_content/lx/history/tool.rs::rows/draw_animated`。行数、绘制、点击命中分别走 `history::rows`、`history::paint` 和 `history_hit`，需要共享同一布局结果，避免折叠后滚动条或命中区域漂移。

## 3. 参考依据与取舍

| 参考 | 已核实机制 | 本任务采用部分 |
| --- | --- | --- |
| Codex `codex-rs/tui/src/exec_cell/render.rs::output_lines` | 输出先取头部和尾部，中间按**真实省略行数**显示提示 | 有界首尾预览和准确省略行数 |
| Codex `codex-rs/tui/src/tool_output.rs::ToolOutputPreview::push_rendered_line` | 按可见物理行预算预览；单条极长行也先限制处理成本 | 先按窗格宽度折行再计算预览预算；单行输入需有界 |
| Codex `codex-rs/utils/output-truncation/src/lib.rs::formatted_truncate_text` | 模型结果截断是另一层，附原始计数 | 明确区分模型结果限制与 UI 预览；本次不改模型层 |
| Claude Code 官方 [tools-reference](https://code.claude.com/docs/en/tools-reference) 和 [fullscreen](https://code.claude.com/docs/en/fullscreen) | Bash 结果有独立输出上限；折叠的工具结果可点击展开，只有确有额外内容才可点击；`!` 输出也可展开 | 独立展开状态、只对可展开内容注册点击面、涵盖 `!` |
| lx 现有 `src/ui/text/agent.rs` | `TREE_BRANCH` / `TREE_LEAF` / `TREE_BAR` 都是等宽 5 列 | 复用现有字形和语义，不新增第二套符号 |

Claude Code 的完整实现不在参考目录中；上表对应其官方行为文档，不把无法核实的内部算法当成设计事实。Codex 参考只取有界预览和省略说明，不照搬它的输出预算或存储格式。

## 4. 目标布局

### 4.1 标题和树体

```text
■ Read file
  ├─ /project/src/main.rs
  └─ lines 1-40

■ Read folder
  └─ /project/src

■ Search
  ├─ "package.json|Cargo.toml|README*" in /project
  │  /with/a/very/long/path/that/wraps
  ├─ 28 matches
  ├─ src/main.rs:12: match text
  └─ … 26 more lines

■ You run command
  ├─ $ cargo test
  ├─ exit 0
  ├─ 1.2s
  └─ … 95 more lines
```

示例仅说明节点顺序和语法；实际预览行数沿各卡片现有预算，再按统一规则校准。`18 matches` 不用于目录 `read`。标题仍通过工具步骤的方块标记渲染，不在子节点重复方块。

### 4.2 连接线不变量

- 一条**逻辑字段**的首物理行：后续还有兄弟节点时用 `TREE_BRANCH`，最后一个节点用 `TREE_LEAF`。
- 字段换行后的续行：只要主干下面仍有节点或当前展开块尚未结束，用 `TREE_BAR`；最后节点的最终续行用等宽 `TREE_PAD` 收口。不能把每个折行都误标为新节点。
- 展开时竖线覆盖新增的每个可见节点；折叠时以省略提示节点收口。预览提示只在确有**已保存但未展示**的内容时出现。
- 空详情或极窄窗格不画孤立的 `└─`；先保留标题和必要状态，剩余宽度不足时安全裁切内容。所有行按显示列宽而不是 UTF-8 字节数计算。
- 文件差异卡片本身有每文件层级；保留文件与 hunk 的语义层级，调整外层缩进和主干连接，避免两个独立树形前缀叠加成假层级。
- `history::snapshot` 仍是消息块之间空行的唯一来源；卡片内部新增行不在绘制路径自行插空行。

### 4.3 字段顺序与异常态

1. 参数在前，结果统计/状态在后，展开正文最后。`search` 先 pattern/scope，再命中数和命中行；`read` 先路径，再范围；命令先 `$ command`，再退出状态/耗时和输出。
2. pending/running 时 `read` 的 `kind` 尚未知：标题暂用 `Read`，工具结果结算且 `kind` 可用后切换为 `Read file` / `Read folder`。不能仅靠路径末尾 `/`、扩展名或参数猜类型。
3. 失败/取消/中断时保留已知参数与错误/状态节点。没有结果 `kind` 时标题保持 `Read`；错误信息不能伪装成行范围或成功输出。
4. 搜索 `0 matches` 是有效结果；目录 `read` 即使有 0 或 N 个条目，UI 均只显示目录路径。`edit/write` 计数未知时不显示猜测的 `+0 -0`。
5. 用户可见新增文案集中在 `src/ui/text/agent.rs`，不在各卡片写硬编码英文；模型可见文本保持英文且不在本次改动范围。

## 5. 截断与展开算法

### 5.1 三个不同事实

| 事实 | 来源 | UI 表达 |
| --- | --- | --- |
| 可见预览比**已保存结果**短 | 布局器实际生成的物理行数 | `… N more lines`，`N = 已保存可展开行数 - 当前已展示行数` |
| 工具执行阶段已限制内容 | 结果 JSON 的 `truncated`、`dropped_bytes`、`omitted_files` 等 | `output limited` / 对应工具已有专用提示；不声称还能展开原始全文 |
| 文件读取还有下一页 | `Read.next_offset` / `truncated` | 仅模型结果保留下一页信息；UI 的 `Read file` 只表示本次范围，不冒充读完整文件 |

所有预览先对**已保存文本**做安全净化与按宽度折行，再从物理行序列中选头部与尾部。折行导致物理行数变化时，省略数也必须随宽度重算。`N` 只计算 UI 隐藏的行，不把执行阶段未保存的行或字节混进去。若单条原始行超长，折行过程必须有单行工作预算，避免每帧在渲染热路径展开巨量文本。

### 5.2 折叠行为

- `read` 没有正文预览，因而不为“文件还有更多行”注册展开点击面；读取更多内容仍由模型下一次带 `offset/limit` 调用。
- `search` 展示少量命中行，展开显示已保存的全部命中行；结果级 `truncated` 单列提示，不嵌入命中正文。
- `exec_command` 与 `!` 保留各自已有的首尾预览和参数/输出独立折叠；统一省略数、树线和文字。进程存活时仍只显示 task panel 引导，不在消息列表同步铺实时日志。
- 文件差异、问答、子 agent 报告沿用已有展开态与内容上限；对仅有摘要、无可展开正文的计划或工具，不制造虚假的点击面。
- 不能通过简单 `.replace("…[truncated]", "")` 改掉用户真实输出。只在已知工具合成的标记及相应 `truncated` 元数据下转换展示；原始结果体和模型文本保持不变。旧历史缺元数据时保守呈现，不伪称完整。

## 6. 关键代码改动点（审核后实施）

| 模块 | 预定变更 | 依赖方向/约束 |
| --- | --- | --- |
| `src/app/state/lx/chat/steps.rs` | 给 `ToolSummary` 增加可选的已知 read 目标种类；`absorb_result_json` / `absorb_summary` 合并 `kind`，实时与重载一致。去掉目录条目数映射到 `matches` 的 UI 语义；搜索命中数仍保留。 | 纯状态转换；旧记录无 `kind` 时安全回退 `Read`，不迁移 DB |
| `src/chat/tools/output.rs` | `to_summary(Read)` 传递 file/directory 种类；模型文本与结果 JSON 不变。 | 仅摘要投影变化，不影响执行或上下文 |
| `src/ui/text/agent.rs` | 增加 `Read file` / `Read folder`、范围前缀和需要的“输出受限”文案；复用现有树前缀。 | 所有可见文字唯一来源 |
| `src/ui/main_content/lx/history/tool.rs` | 标题根据已知 kind 选动词；整理普通工具行的布局入口与展开判定。 | `rows` 与 `draw` 共用布局口径 |
| `.../tool/file_summary.rs` | 删除 `read` 的 `matches` 和冗余 `(N lines)`，将参数/范围/计数/partial 拆成逻辑节点；移除 `·` 拼接。 | 目前 691 行，不为拆文件而拆文件；若实施后逼近 1000 行按职责拆分 |
| `.../tool/search.rs` | pattern/scope、命中数和命中行统一树线；保留结果级截断事实，隐藏合成的行尾 `…[truncated]`。 | 预览/展开/命中几何一套布局 |
| `.../tool/command.rs`、`.../tool/{plan,ask,subagent,file_diff}.rs` | 逐类把独立字段改成节点，统一折行和省略行位置；不改变原有业务状态。 | 不把进程、计划、子 agent 等不同语义硬塞进文件摘要 |
| `src/ui/main_content/lx/history/user_shell.rs` | 标题中的退出码/耗时移到树节点；沿用参数/输出双折叠。 | 本地命令不是模型工具，但共享消息列表语法 |
| `src/ui/main_content/lx/history/{rows,paint,snapshot}.rs`、`history_hit.rs` | 仅在行数/命中几何需要时更新对接，不改变时间线顺序和块间空行。 | 渲染只读状态、无 IO |

如果各卡片都需要相同的“逻辑节点 -> 物理行 -> 树前缀”算法，可在现有 `history/tool.rs` 的近邻增加一个小型纯布局 helper；先核对现有 `TREE_*` 与各卡片函数能否复用，避免为了统一外观引入泛型卡片框架。

## 7. 验收矩阵

1. `read(file)`：默认读取、`offset/limit`、空文件、超长路径、失败、重载；标题种类正确，详情只含路径和实际范围，不含正文、总行数或 `…[truncated]`。
2. `read(directory)`：空/18 条目/分页/被条目上限截断；标题为 `Read folder`，仅路径，绝无 `matches`。目录结果仍可送模型。
3. `search`：0/1/28 命中、长命中行、行级截断、结果级截断、折叠/展开；命中数正确，合成标记不可见，省略数与可展开行数一致。
4. 命令和 `!`：运行中、成功、失败、中断、输出为空、单条超长行、首尾预览、双折叠、重载；退出码与耗时分节点，状态不被美化成成功。
5. `edit/write/plan/ask/subagent`：多文件差异、partial、问答题干、报告、空计划；所有可见独立字段不用 `·` 串接，树干在展开内容中连续，原有点击语义保留。
6. 布局：宽/窄/极窄/零宽、中文宽字符、长路径、跨行换行、折叠切换后 `rows == draw`、命中区域和滚动条同步、块间空行不变。
7. 数据：同一结果在实时结算和 `/resume` 后 UI 一致；模型工具结果字符串、DB JSON 和既有执行截断上限无变化。受影响模块的定向测试通过，再按项目规范执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。

## 8. 风险与人工验收

- 大量历史卡片在展开后可能增加渲染与滚动计算成本；布局必须有界，避免每帧重复解析大 JSON 或构造超预算行。
- 旧历史中没有 read `kind` 或结果 JSON 不完整时，只能显示 `Read`，不可根据路径猜测类型。
- `│`、`├─`、`└─` 的终端字形和鼠标点击观感无法由 Agent 交互式确认。实施后由用户运行 `cargo run`，重点检查截图所示窄窗格、折行后竖线延伸、展开/收起与复制。
- 本设计不增加 DB 迁移；若实施时发现必须增加持久化字段，先回到设计评审，不把迁移塞进渲染任务。

