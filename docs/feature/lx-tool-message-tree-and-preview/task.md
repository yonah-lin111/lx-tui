# lx 消息列表工具树与输出预览：实施任务

> 状态：已完成并合入 `dev`。验证结果与实施偏差见文末。
> 原审核约束已由用户明确解除。

## 0. 审核与基线

- [x] 收集设计审核意见；若边界、字形或预览语义变化，先修订 `design.md` 与本清单。
- [x] 只读核对工作树最新状态；保留已有未跟踪目录与用户改动。
- [x] 复查四份项目规范；用 codegraph 和 codebase memory 更新所改符号的调用链、索引覆盖和单文件行数。
- [x] 在现有 `tool/tests`、`history/tests` 中选代表性 fixture，先确认当前 `read(directory)` 出现 `N matches`、标题无法区分种类、命令标题出现 `·` 的现状。

## 1. 状态与摘要投影

- [x] 为 `read` 结果增加仅供视图使用的 file/folder 判别字段；结果未到达时保持未知。
- [x] 修改 `chat::tools::output::to_summary`，用 `ReadKind` 填该字段；不改 `format_for_model` / `to_result_json`。
- [x] 修改 `ToolSummary::absorb_result_json` 和 `absorb_summary`：结果种类、路径、范围在实时结算与 DB 重载中一致；空文件/目录清除参数推断的文件行范围。
- [x] 停止把目录条目数当 `matches` 供 UI 使用；`search` 命中数保持原口径。旧结果 JSON 缺 `kind` 时标题回退 `Read`。
- [x] 定向测试：file/folder/空文件/分页/老记录/坏 JSON，实时摘要与重载摘要一致。

## 2. 共用树形排版

- [x] 核对并复用 `ui::text::agent::{TREE_BRANCH,TREE_LEAF,TREE_BAR,TREE_PAD}`，定义逻辑节点、折行续行和最后节点的统一规则。
- [x] 把独立字段从 `·` 拼接改为节点；同一字段内的文本空格不算分隔符。
- [x] 让布局函数一次确定折叠/展开所需物理行、连接符、内容与提示行；`rows`、`draw`、点击面共用同一口径。
- [x] 极窄宽度、宽字符、长路径和零宽时安全降级；工具标题、状态方块和重要参数优先可见。
- [x] 定向几何测试：树前缀等宽、竖线跨折行与展开行、最后一节点收口、`rows == draw`、滚动和命中区域不漂移。

## 3. 文件与搜索工具卡片

- [x] `read` 终态标题为 `Read · file` / `Read · dir · N files`；运行中或旧历史未知种类仍为 `Read`。
- [x] `read` 树体仅显示路径；文件范围不再占行，目录条目数只出现在标题；任何目录条目数都不显示 `matches`。
- [x] `search` 命中数与结果级截断进标题（`Search · 3 matches · results truncated`）；pattern/scope 是首节点，命中行合成一个多行节点（首行 `└─`、其余左对齐），截断不占树体；合成的 `…[truncated]` 不进入可见行。
- [x] `edit` / `write` 路径与增删计数同节点；差异正文是一个多行节点（首行 `└─`、其余左对齐）；`partial` 等事实单独成行。
- [x] 定向 buffer 测试覆盖 0/1/N 命中、长命中行、空/分页目录、文件 offset、多个 diff 文件、折叠/展开与旧历史。

## 4. 进程、计划、交互与本地命令

- [x] `exec_command`、`write_stdin`：退出状态与耗时进标题；命令是首节点，输出是最后一个多行节点；运行中仍用 task panel 引导；参数/输出独立折叠。
- [x] `update_plan` 完成计数进标题、步骤合成一个多行节点；`ask` 每题是 `Qn:` / `An:` 多行节点；`spawn_agent` / `wait_agent` / `close_agent` 标题保留调用条件，目标与报告行用 ` · ` 分隔角色/标题。
- [x] `!` 本地命令：退出状态/耗时进标题；命令是首节点、输出是多行节点；参数与输出原有独立折叠、FIFO/停止逻辑不变。
- [x] 定向测试覆盖正在运行、成功、失败、中断、无输出、报告长行、计划空态、问答展开、用户命令标题和输出。

## 5. 有界预览和截断事实

- [x] 对可展开的**已保存**内容，按窗格宽度折行后取首尾预览，精确计算 UI 省略的物理行数；展开显示已保存全部内容。
- [x] 执行阶段已有 `truncated` / `dropped_bytes` / `omitted_files` 时另列“输出受限”事实，不把它混入 UI 省略行数，也不伪装成可展开全文。
- [x] 对超长单行和极窄窗格限制布局工作量；禁止仅靠字符串替换吞掉用户真实输出。
- [x] 只有展开后确有更多已保存内容时注册折叠点击面；拖拽复制不触发展开。
- [x] 定向测试区分“UI 隐藏 95 行”和“执行结果本身被截断”，覆盖宽度变化、折行、重载、复制文本。

## 6. 验证与交付

- [x] 先跑受影响模块的定向测试，修正所有预期行数和 buffer 快照。
- [x] 执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（项目规范要求）；记录结果与未验证项。
- [x] 核查生产文件行数不超过 1000；超过时按职责拆分，测试文件不按体积拆分。
- [x] 用户运行 `cargo run` 验收真实 TUI：`Read · dir · N files`、角色/标题的 ` · `、目录路径、命令首尾预览、多行节点的首尾连接符、点击与复制。
- [x] 回填实施偏差与验证结果到本文；代码实施前不把勾选项标为完成。



## 7. 实施结果与偏差（回填）

实施位置：已合入 `dev`（原开发工作区 `.worktrees/tool-message-tree` 及分支 `feat/tool-message-tree` 已清理）。

### 7.1 结构

- `tree.rs` 按**逻辑节点**分配连接符：多行节点（输出 / diff / 计划步骤 / 问答对 / 报告）只在首行画 `├─`/`└─`，节点内部续行画 `│` 或空白；折叠提示是续行，不单独收口。`rows` / `draw` / 点击面 / agent 链接共用同一份 `Layout`。
- 标题右侧统一承载状态：进程 `Exited 0 · 241ms` / `Running`，`read` 的 `file` / `image` / `folder · N files`，`search` 的 `N matches · results truncated`，`plan` 的 `N/M completed`，`wait` 的调用条件，`spawn` 的子 agent 状态。
- `read` 树体只显示路径；`edit/write` 路径与 `(+N -N)` 同节点，diff 正文是多行节点；`ask` 直接使用 `Q1: <题干>`（灰色）与 `A1: <答案>`（默认前景色），不显示弹窗短标签 `header`；`Thought` 与 `Subagent report` 也使用首行 `└─`、正文左对齐，`Report:` 标签颜色与报告正文一致。
- 当前项目路径相对化（对齐 Codex `display_path_for`）：工具卡片中的路径若位于当前项目根目录下，自动剥离项目根前缀显示为相对路径（如 `.worktrees/tool-message-tree`、`src/main.rs`），非当前项目的外部路径保持完整路径。
- 连续文件工具**不再合并**：每个调用独立卡片（独立标题、独立点击面），块间保留一行空行。
- OpenAI 兼容协议同一步的前导文本（preamble）与连续 `tool_calls` 合并进同一条 `assistant` 消息，并强化系统提示词持续性约束，防止模型在输出前言后不调工具直接 `stop`。

### 7.2 与原设计的偏差

1. **`write_stdin` 显示本次保存的输出与输入预览**：命令是首节点；`Send input` 的非空输入作为下一节点显示（遵循折叠态灰、展开态默认前景），输出中与输入匹配的单次 TTY 回显会被剔除，避免同一行出现两次。
2. **`exec_command` 的耗时**：`CommandView.duration_ms` / `ProcessInteractionView.duration_ms` 取自 `wall_time_ms`，重载解析 `wall_time_seconds`；只在非运行态进标题。
3. **`read` 不再显示行范围**：按用户最新格式，标题只给 `file` / `image` / `folder · N files`，路径在树体；目录条目数来自结果体 `total_lines`。
4. **文件摘要展开不设 24 行上限**：摘要字段本身已按 512 字符/字段取样，展开显示全部已保存摘要；`DETAIL_MAX_ROWS` 仅继续服务命令参数取样预算。
5. **`wait_agent` 的调用条件留在标题右侧**：`2 targets` / `all` / `until done` 用 ` · ` 拼在 `■ Wait agents` 之后，树体只列目标与报告。
6. **输出单行有 8K 字符取样上限**（超出补 `…`）：这是渲染热路径的有界保护，不是执行截断；`read`/命令的 `output limited`、`search` 的 `results truncated`（标题右侧）、`dropped bytes` 仍独立表达。
7. **移除连续工具调用合并**：同一 `edit`/`write` 连续出现时各自成卡，不再共享合并标题；消息内每个 step 之间保持一行空行。

### 7.3 验证

- `cargo fmt`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test --lib`：3753 passed（0 failed）。
- 定向测试覆盖：`Read · file` / `Read · dir · N files` / 未知 kind、搜索命中数进标题、命中块长行、计划多行块、问答 Q/A 标签、命令/交互标题状态与耗时、`Send input` 回显去重、文件 diff 路径 `├─` + 正文 `└─`、连续工具不合并、每张卡片 `rows == draw`、零宽/极窄/宽字符、折叠提示行数与点击面。
- 生产文件行数：`tool.rs` 693、`file_diff.rs` 611、`subagent.rs` 594、`user_shell.rs` 560、`tree.rs` 372，均 < 1000。
- 无法自动验证：真实终端字形、鼠标点击/拖拽复制观感，仍需用户 `cargo run` 验收（重点：`Read · dir · N files`、目录路径、角色/标题的 ` · `、多行节点首尾连接符、命令首尾预览）。
