# lx 消息列表工具与非正文块显示规范

> 状态：**目标规范；合并形态尚未实现**。
> 本文件是 `tui-design-requirements.md` 的 lx 消息块分册，专门记录消息列表里每个工具、每个非正文块的单体显示与合并显示。
> 图例：✅ 当前已实现；⏳ 目标形态，实施见 `docs/feature/lx-tool-call-grouping/`。
> 未标 ⏳ 的条目以当前代码为准；标 ⏳ 的条目在实施完成前，主规范中的旧描述可能仍与本文冲突，实施时必须同步修正。

当前代码与本文的已知差异：

- `read` 当前标题带 `file` / `folder · N files` / `image`，详情为路径与参数；本文要求保留标题状态，标题下方**只显示一条路径**。
- 连续同名工具当前各自成卡；本文的组头与组内树体尚未实现。
- `update_plan` / `list_agents` 当前各自成卡；本文的快照合并尚未实现。

## 1. 适用范围

本文覆盖 lx 聊天历史区（消息列表）里的全部非正文块：

- 全部工具调用：`read` / `search` / `web_search` / `edit` / `write` / `exec_command` / `write_stdin` / `update_plan` / `ask` / `curr_time`；
- 八个子 agent 控制工具：`spawn_agent` / `send_input` / `interrupt_agent` / `resume_agent` / `wait_agent` / `close_agent` / `list_agents` / `cancel_agent_input`；
- MCP 工具与 MCP resources 桥接工具；
- 未知工具名的兜底卡片；
- 用户消息、助手正文、思考块；
- 本地命令卡片、消息底部通知、统计行、附件引用树、子 agent 汇报块；
- 系统事件卡片：`Rewind`、`Context compacted`、`Model switched`、`Recap`，以及两个运行中占位行；
- 置顶用户消息行（sticky prompt header）。

不覆盖：输入框、状态栏、右侧面板、弹窗、toast、后台任务页。

## 2. 全局规则

### 2.1 字形与角色

| 角色 | 字形 | 用途 |
| --- | --- | --- |
| 文字数据 | `●` / `○` | 用户消息、助手正文、思考块 |
| 工具调用 | `■` / `□` | 所有工具卡片、工具合并组头、本地命令卡片 |
| 子 agent 汇报 | `▲` / `△` | 仅 `Subagent report` |
| 系统事件 | `◆` / `◇` | `Rewind`、`Context compacted`、`Model switched`、`Recap`、失败/中断通知 |

- 运行中为实心/空心两帧呼吸；终态一律实心。
- 字形只在 `ui::text::agent::STEP_MARK_*` 定义，渲染层不得写死字符。

### 2.2 颜色

- 工具运行中：橙黄 `Indexed 208`，方块与动词加粗呼吸；完成：翡翠绿 `Indexed 42`，去加粗。
- 失败：红色（`style::agent_fail`）；中断：黄色（`style::agent_warn`）；取消沿用角色色实心。
- 工具参数（路径、搜索模式、命令）用浅黄 `chat_tool_param()`；只读元信息（计数、行号、状态词）用 `muted()`。
- 系统事件标题用 accent；详情用 muted。
- 合并组头方块取组内**最差状态**：失败 > 中断 > 运行中 > 取消 > 完成。

### 2.3 树体

- 标题行是唯一不带连接符的行。
- 树体使用 `├─` / `└─` / `│`，前缀固定 5 列；末节点续行用等宽空白，不补 `│`。
- 工具合并组内：第一个调用节点用 `├─`，最后一个调用用 `└─`；调用体内部仍按自己的树规则。
- 极窄宽度放不下树体前缀时整段让位，内容按整宽折行，不画半个宽字符。

### 2.4 空行

- 同一条消息内任意两个块之间**空一行**。
- **例外**：工具合并组内部不空行；组头与第一个调用、调用与调用之间都紧邻。
- 组与前后块之间仍空一行。
- 空行只由 `history::snapshot` 累加，绘制路径不得另算。

### 2.5 折叠

- 每个可折叠块独立；点击一块只切换该块。
- 工具合并组头不可点击，不提供“整组折叠”。
- `read` 标题下方只显示一条路径；路径不换行，结果正文与参数事实不占第二行。
- 折叠提示行属于折叠区，计入点击面；`… N more lines` 不单独收口。

## 3. 工具合并总则

### 3.1 合并键

- 同一条 assistant 消息内、相邻的 `Tool` 步骤。
- 精确工具名相同（`call.name`）才合并；`edit` 与 `write` 不合并，`read` 与 `search` 不合并。
- `write_stdin` 额外按**显示动词**区分：`Send input` / `Read command output` / `Stop command` 不同动词不合并。
- 正文、思考、消息结束打断分组；跨消息不合并。
- 组内至少 2 个调用才显示组头；单调用保持单体卡片。

### 3.2 组头

```text
■ <动词> · N calls[ · 聚合信息]
```

- `N calls` 用 dim；不出现 `1 call`。
- `edit` / `write`：追加组内增删合计 `· +A -R`；任一计数未知或被裁时合计后加 `(?)`。
- `search`：追加组内命中合计 `· N matches`；未知时不写。
- 组内存在非完成态时追加 `· N running` / `· N failed` / `· N interrupted`；全部完成不写。
- 组头只有 1 行，超宽尾部省略，不换行。
- 组头不登记点击面。

### 3.3 组内调用

- 每个调用的第一个可见树节点就是该调用的摘要行。
- 原单体标题状态（`Exited 0 · 241ms`、`3 matches`、`1/3 completed`、`2 questions` 等）移动到该调用首节点尾部，仍用 ` · ` 分隔。
- 每个调用保留完整树体；`read` 例外：标题下方只显示一条路径。
- 失败/中断调用保留错误/中断文案；连接符与状态文字用对应语义色，错误正文保持正常前景。
- 每个调用体保留自己的折叠状态、点击面、行底色、链接热区。

### 3.4 合并形态分类

| 形态 | 含义 | 适用 |
| --- | --- | --- |
| 列表合并 | 组头下按顺序显示每个调用 | 大多数工具 |
| 快照合并 | 组头下只显示**最后一次**调用，组头标注更新次数 | 状态快照类工具 |
| 不合并 | 每个调用/块独立显示，中间空一行 | 阻塞交互、系统事件、消息级块 |

## 4. 工具清单

| 工具 | 单体标题 | 合并形态 | 合并键 | 实现位置 |
| --- | --- | --- | --- | --- |
| `read` | `■ Read · file` / `· folder · N files` / `· image` | 列表合并；每调用只显示一条路径 | `read` | `tool/file_summary.rs` |
| `search` | `■ Search · N matches` | 列表合并；组头可汇总命中 | `search` | `tool/search.rs` |
| `web_search` | `■ Web search · provider · 840ms` | 列表合并 | `web_search` | `tool/web_search.rs` |
| `edit` | `■ Edit` | 列表合并；组头汇总增删 | `edit` | `tool/file_diff.rs` / `file_summary.rs` |
| `write` | `■ Write` | 列表合并；组头汇总增删 | `write` | 同上 |
| `exec_command` | `■ Run command · Exited 0 · 241ms` | 列表合并；每调用保留退出状态 | `exec_command` | `tool/command.rs` |
| `write_stdin` | `■ Send input` / `■ Read command output` / `■ Stop command` | 列表合并；按显示动词分组 | `(write_stdin, 动词)` | `tool/command.rs` |
| `update_plan` | `■ Plan · 1/3 completed` | **快照合并**：只显示最新计划，组头 `· N updates` | `update_plan` | `tool/plan.rs` |
| `ask` | `■ Ask · N questions` | **不合并**；每次阻塞问答独立 | — | `tool/ask.rs` |
| `curr_time` | `■ Current time` | 列表合并 | `curr_time` | `tool/file_summary.rs` |
| `spawn_agent` | `■ Spawn agent · status` | 列表合并 | `spawn_agent` | `tool/subagent.rs` |
| `send_input` | `■ Send agent input · status` | 列表合并 | `send_input` | `tool/subagent.rs` |
| `interrupt_agent` | `■ Interrupt agent` | 列表合并 | `interrupt_agent` | `tool/subagent.rs` |
| `resume_agent` | `■ Resume agent · status` | 列表合并 | `resume_agent` | `tool/subagent.rs` |
| `wait_agent` | `■ Wait agents · 条件` | 列表合并 | `wait_agent` | `tool/subagent.rs` |
| `close_agent` | `■ Close agent` | 列表合并 | `close_agent` | `tool/subagent.rs` |
| `list_agents` | `■ List agents` | **快照合并** ⏳：只显示最新列表，组头 `· N updates` | `list_agents` | `tool/subagent.rs` |
| `cancel_agent_input` | `■ Cancel agent input · status` | 列表合并 | `cancel_agent_input` | `tool/subagent.rs` |
| MCP 工具 | `■ server · tool · 840ms` | 列表合并 | 完整 MCP 工具名 | `tool/mcp.rs` |
| MCP resources | `■ server · tool` | 列表合并 | 完整工具名 | `tool/mcp.rs` |
| 未知工具 | `■ <tool name>` | 列表合并 | 完整工具名 | `tool/file_summary.rs` |

## 5. 各工具显示

### 5.1 `read`

单体：

```text
■ Read · file
  └─ src/event/mod.rs
```

合并：

```text
■ Read · 3 calls
  ├─ src/event/mod.rs
  ├─ src/app/state.rs
  └─ src/main.rs
```

规则：

- 标题保留 `file` / `folder · N files` / `image` 状态；目录条目数照常显示。
- 标题下方**只显示一条路径**：路径、目录条目、图片路径都一样，不出现第二条内容行。
- 不显示行范围、实际行数、`output limited`、`… N more lines`，也不渲染读取结果正文。
- 路径超宽时**单行尾部省略**，不软换行成第二行。
- 失败/中断：显示一条错误/中断原因（单行尾部省略），不显示路径与结果正文。
- `read` 没有可展开内容，不登记点击面。
- 合并后每个调用只保留自己的一条路径，不扁平混合成多行参数。

### 5.2 `search`

单体：

```text
■ Search · 3 matches
  ├─ "fixme" in src
  ├─ src/ui/main_content/lx/history.rs:42: // FIXME...
  └─ … 2 more lines
```

合并：

```text
■ Search · 2 calls · 5 matches
  ├─ "fixme" in src
  │  ├─ src/ui/main_content/lx/history.rs:42: // FIXME...
  │  └─ … 1 more line
  └─ "todo" in docs
     └─ docs/standards/tui-design-requirements.md:120: TODO...
```

规则：

- 摘要节点为 `"pattern" in path`；没有 pattern 时退回路径。
- 命中行按 `path:line: text`，折叠态最多 2 行 + `… N more lines`；展开铺开全部。
- 结果级截断在标题状态里写 `results truncated`；合并后写组头。
- 组头命中合计取各调用 `summary.matches` 之和；未知时不写。

### 5.3 `web_search`

单体：

```text
■ Web search · exa · 840ms
  ├─ "rust tui"
  └─ Search result markdown...
     … 2 more lines
```

合并：

```text
■ Web search · 2 calls
  ├─ "rust tui" · exa · 840ms
  │  └─ Search result...
  └─ "ratatui" · tavily · 620ms
     └─ Search result...
```

规则：

- 组头不显示 provider/耗时，各调用首节点保留自己的 ` · provider · 耗时`。
- 查询块与结果块各自独立折叠；合并后仍按调用区分。
- 结果正文是压暗 Markdown，折叠 3 行 + `… N more lines`。

### 5.4 `edit` / `write`

单体（带 diff）：

```text
■ Edit
  ├─ src/a.rs (+5 -2)
  │  ├─ 1 - old
  │  └─ 1 + new
  └─ src/b.rs (+3 -2)
     └─ …
```

合并：

```text
■ Edit · 2 calls · +8 -4
  ├─ src/a.rs (+5 -2)
  │  ├─ 1 - old
  │  └─ 1 + new
  └─ src/b.rs (+3 -2)
     └─ …
```

单体（无 diff 的旧记录）：

```text
■ Edit
  └─ src/a.rs (+5 -2)
```

规则：

- 标题只写裸动词；文件头 `path (+A -R)`；计数未知时 `(?)`。
- 折叠态每文件最多 3 个新增/删除物理行；展开铺开全部 hunks、1 行上下文、`⋮` 分隔。
- 新增绿、删除红，整行底色；文件头路径恒 dim，展开不亮。
- 合并组头汇总全部调用的增删；同一文件被两次 `edit` 会出现两行。
- `edit` 与 `write` 不合并。

### 5.5 `exec_command`

单体：

```text
■ Run command · Exited 0 · 241ms
  ├─ $ cargo test
  └─ test one ... ok
     … 18 more lines
```

运行中：

```text
■ Run command · Running
  ├─ $ cargo test
  └─ open task panel · live output
```

合并：

```text
■ Run command · 2 calls · 1 running
  ├─ $ cargo check · Exited 0 · 1.2s
  │  └─ Finished `dev` profile ...
  └─ $ cargo test · Running
     └─ open task panel · live output
```

规则：

- 参数区恒为浅黄；输出区折叠 dim、展开为终端默认前景。
- 参数区与输出区各自独立折叠；合并后仍按调用独立。
- 进程存活时方块与动词橙黄；进程停止回绿；工具失败红、中断黄。
- 组头 `· N running` 只统计进程/调用非完成态，不替代每个调用的退出码。

### 5.6 `write_stdin`

单体：

```text
■ Send input
  ├─ $ ...
  └─ ...
```

合并：

```text
■ Send input · 2 calls
  ├─ $ ...
  └─ $ ...
```

规则：

- 显示动词由动作决定：`Send input` / `Read command output` / `Stop command`。
- 合并键包含显示动词；不同动词不合并，避免组头只能写一个动词。
- 输入预览与输出区各自折叠；`Read command output` 只显示命令快照，不跟随后台输出。

### 5.7 `update_plan` ⏳

单体：

```text
■ Plan · 1/3 completed
  └─ [x] Implement command tools
     [>] Verify process cleanup
     [ ] Review terminal display
```

快照合并：

```text
■ Plan · 3 updates · 2/3 completed
  └─ [x] Implement command tools
     [x] Verify process cleanup
     [ ] Review terminal display
```

规则：

- 连续 `update_plan` 是**同一计划的状态快照**，不逐条铺开历史计划。
- 组头 `· N updates`；正文只显示最后一次快照。
- 组头完成计数取最后一次快照。
- 这样既保留“更新过 N 次”的事实，又不让旧计划占满消息列表。

### 5.8 `ask`

不合并。单体：

```text
■ Ask · 2 questions
  ├─ Q1: 你要哪种布局？
  │  A1: 双栏
  └─ Q2: 深色还是浅色？
     A2: 深色, 用青色
```

规则：

- 每次 `ask` 是阻塞交互，问题编号与答案状态属于该次调用；连续 ask 各自成块，中间空一行。
- 运行中只写 `waiting for you`，不解析、不猜题数。
- 不显示 preview 字符画。
- 标题状态 `N questions`；整块一个 Body 点击面，展开前后行数不变时不登记。

### 5.9 `curr_time`

单体：

```text
■ Current time
  └─ <摘要路径或状态>
```

合并：

```text
■ Current time · 2 calls
  ├─ <摘要>
  └─ <摘要>
```

规则：

- 无路径时按普通工具兜底：一行状态词或空 `└─`。
- 合并键 `curr_time`；列表合并。

### 5.10 子 agent 控制工具

动词：

| 工具 | 动词 |
| --- | --- |
| `spawn_agent` | Spawn agent |
| `send_input` | Send agent input |
| `interrupt_agent` | Interrupt agent |
| `resume_agent` | Resume agent |
| `wait_agent` | Wait agents |
| `close_agent` | Close agent |
| `list_agents` | List agents |
| `cancel_agent_input` | Cancel agent input |

单体 `spawn_agent`：

```text
■ Spawn agent · running
  ├─ explorer · 调查事件流
  └─ Report: ...
```

合并 `spawn_agent`：

```text
■ Spawn agent · 2 calls
  ├─ explorer · 调查事件流
  │  └─ Report: ...
  └─ executor · 修改存储层
     └─ Report: ...
```

`wait_agent`：

```text
■ Wait agents · 2 targets · all · until done
  ├─ ■ explorer · completed
  └─ ■ executor · running
```

规则：

- 目标行是链接，点击打开 Agents 面板并定位；合并后链接偏移按调用体重新计算。
- `spawn_agent` / `send_input` / `resume_agent` / `cancel_agent_input` 的状态在首节点尾部；`wait_agent` 的等待条件在首节点尾部；`close_agent` 无状态。
- `list_agents` 连续调用是状态快照：**快照合并**，只显示最新列表，组头 `· N updates`。
- 其他子 agent 工具列表合并；`spawn_agent` 重复调用是多个独立委派，不折叠。
- `wait_agent` 不用呼吸动画，目标状态只用静态绿/黄/红。

### 5.11 MCP 工具与 resources

单体：

```text
■ filesystem · read_file · 120ms · 1 image
  ├─ path: src/main.rs
  └─ Result markdown...
     … 2 more lines
```

合并：

```text
■ filesystem · read_file · 2 calls
  ├─ path: src/main.rs
  │  └─ Result...
  └─ path: src/lib.rs
     └─ Result...
```

规则：

- 标题为 `server · tool`；状态为耗时、`result truncated`、图片数。
- 参数块与结果块各自独立折叠；图片只显示一行元数据（路径可复制），不读字节。
- 合并键是完整工具名（含 server 前缀）；同名 MCP 工具才合并。
- MCP resources 桥接工具 `list_resources` / `list_resource_templates` / `read_resource` 同规则。

### 5.12 未知工具

```text
■ <tool name>
  └─ <摘要或状态>
```

- 动词位置显示原始工具名；正文走普通工具摘要。
- 合并键为完整工具名；两个不同未知工具不合并。

## 6. 非正文块

| 块 | 单体显示 | 合并形态 |
| --- | --- | --- |
| 用户消息 | `> 文本`，可带附件引用树 | 不合并 |
| 助手正文 | `● Markdown` | 不合并 |
| 思考块 | `● Thinking` / `● Thought · 6.3s` | 不合并 |
| 工具调用 | 见 §4–§5 | 见 §3 |
| 本地命令卡片 | `■ You run command · exit 2 · 1.2s` | 不跨消息合并 |
| 消息底部通知 | `◆ Failed` / `◆ Interrupted` + 原因 | 不合并 |
| 统计行 | `模型 · 2.9k in · 307 out · 115.6 tok/s · 2.7s` | 不合并 |
| 附件/生成图片引用树 | `├─ Image (preview): ...` | 不合并 |
| 子 agent 汇报块 | `▲ Subagent report · completed` | 不合并 |
| `Rewind` | `◆ Rewind` + 描述 | 已有 `xN` 合并 |
| `Context compacted` | `◆ Context compacted` + 元信息 + 摘要 | 不合并 |
| `Model switched` | `◆ Model switched` + 元信息 + 跃迁链 + 提示词 | 已有 `xN` 合并 |
| `Recap` | `◆ Recap` + 元信息 + 摘要 | 不合并 |
| 运行中 compact | `◇ Compacting context... · 3s` | 唯一 |
| 运行中 recap | `◇ Generating recap... · 3s` | 唯一 |
| 置顶用户消息行 | `> <摘要>` | 不合并 |

### 6.1 思考块

```text
● Thinking
  └─ 正文...
```

完成：

```text
● Thought · 6.3s
```

- 折叠态只显示头部；展开显示压暗 Markdown 正文。
- 圆点与文案折叠态浅粉 182，展开态亮粉 218。
- 不合并；连续思考块各自独立。

### 6.2 本地命令卡片

```text
■ You run command · exit 2 · 1.2s
  ├─ $ cargo test
  └─ error[E0308]: mismatched...
     … 18 more lines
```

- 与工具命令卡片同一套树体与折叠；参数区恒浅黄，输出区折叠 dim、展开默认前景。
- 每条消息最多一个 `message.shell`；不跨消息合并。

### 6.3 消息底部通知

```text
◆ Failed
  └─ no error details provided
```

```text
◆ Interrupted
  └─ interrupted before completion
```

- 固定枚举标题，只有红/黄两档；原因行连接符用供应商灰 + 加粗。
- 每条消息最多一条通知；不合并。

### 6.4 统计行

```text
gpt-5 · 2.9k in · 307 out · 115.6 tok/s · 2.7s
```

- 助手消息末尾，流中不显示；无用量/耗时时不显示。
- 宽度不足依次隐藏 `in+out` → 模型名 → `tok/s`，最后对时长尾部省略。
- 不合并。

### 6.5 附件与生成图片引用树

```text
├─ Image (preview): foo.png  [Download original]
└─ File: src/main.rs
```

- 用户消息下引用文件/文件夹/图片/技能；助手消息下生成图片带预览与下载按钮。
- 预览文件名与已下载按钮需按住 Ctrl/Cmd 点击。
- 每条消息一棵引用树；不合并。

### 6.6 子 agent 汇报块

```text
▲ Subagent report · completed
  ├─ explorer · compact 的触发点
  └─ Report:
     report body...
     … 8 more lines
```

- 状态：completed / failed / cancelled；失败红、取消黄。
- 折叠预览 2 行；展开每次追加 24 行；压暗 Markdown。
- 同一消息内多份汇报各自独立，不合并。

### 6.7 系统事件卡片

共用结构：`◆ <标题>` + 常显元信息 + 可折叠正文。

`Rewind`：

```text
◆ Rewind
  └─ Conversation undone; restored files.
```

连续撤销提示合并：

```text
◆ Rewind x3
  └─ <最后一条描述>
```

- 已有 `xN` 合并；描述取最后一条；不进一步合并。

`Context compacted`：

```text
◆ Context compacted
  ├─ model: gpt-5 · 10:42
  ├─ covered sequence 0–7
  └─ summary...
```

- 每个检查点独立；**不合并**（每条摘要都要独立展开）。

`Model switched`：

```text
◆ Model switched
  ├─ model: gpt-5 (high) · 10:42
  ├─ instructions: core/model
  └─ switched A -> B -> A
     injected prompt...
```

连续切换合并：

```text
◆ Model switched x2
  ├─ ...
  └─ switched A -> B -> A
```

- 已有 `xN` 合并；保留完整跃迁链，不退化；正文是最后一次注入提示词。
- 不进一步合并。

`Recap`：

```text
◆ Recap
  ├─ model: gpt-5 · 10:42
  ├─ Next: ...
  └─ summary...
```

- 每条 recap 独立；不合并。

### 6.8 运行中占位行

```text
◇ Compacting context... · 3s
```

```text
◇ Generating recap... · 3s
```

- 单行，空心菱形；耗时按秒刷新。
- 同一时间最多一个；完成后由对应系统事件卡片替换。

## 7. 合并算法

```text
for step in message.steps:
  if step is Tool:
    if previous is same group key and groupable:
      append to group
    else:
      close previous group; start new group
  else:
    close previous group; emit part block
```

- 组大小 >= 2 才渲染组头；否则按单体。
- 快照合并工具（`update_plan` / `list_agents`）在组内只保留最后一次调用的正文，组头计数为调用次数。
- 不合并工具（`ask`）遇到同种调用也各自成块。
- 非工具块不参与工具分组。
- 组合并键在 `write_stdin` 上为 `(name, display verb)`。

## 8. 几何与命中

- 组头块 1 行，不可点击。
- 每个调用体一个独立块；组内连续放置，不插入 spacer。
- 调用体的点击面、折叠 hover 区、agent 链接偏移都相对**该调用体块首**，不含组头。
- `read` 只有一条路径，不登记点击面。
- `rows` / `draw` / 命中 / 滚动条 / 置顶行全部读同一份 `history::snapshot`，禁止在绘制路径另算行数。

## 9. 新增工具时的更新清单

新增工具必须同时完成：

- [ ] 在 §4 表格增加一行：工具名、单体标题、合并形态、合并键、实现位置。
- [ ] 在 §5 增加单体示例、合并示例与折叠规则。
- [ ] 明确状态文字：运行中 / 完成 / 失败 / 中断 / 取消。
- [ ] 明确是否有可展开内容、点击面范围、折叠提示。
- [ ] 明确合并形态：列表合并 / 快照合并 / 不合并，并写理由。
- [ ] 若新增字形或颜色，先更新 `ui::text::agent` / `ui::style`，再更新本文。
- [ ] 同步检查 `tui-design-requirements.md` 与 `project-directory-structure.md`，避免两处描述冲突。

## 10. 相关文档

- `docs/feature/lx-tool-call-grouping/design.md`：合并与 `read` 单条路径的详细设计与待确认项。
- `docs/feature/lx-tool-call-grouping/task.md`：实施步骤与测试矩阵。
- `docs/standards/tui-design-requirements.md`：lx 聊天区的完整交互与视觉规范。
- `docs/feature/lx-file-tool-diff/design.md`：文件 diff 卡的历史设计（其旧合并卡描述已被本文取代）。
