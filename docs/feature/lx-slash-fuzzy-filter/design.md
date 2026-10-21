# `/resume`、`/model` 二级面板模糊匹配：设计方案

> 状态：**用户已逐项确认边界**（见 §2），已实施待验收。
>
> 任务范围：`/resume` 历史面板与 `/model` 模型面板支持用**输入框内容**做模糊过滤。

## 1. 背景与目标

现状：`/resume`、`/model` 二级面板打开后，`apply_lx_input` 走 `_ => return` 把编辑键全部吞掉
（`src/app/update/pane.rs`）。面板只支持 `↑`/`↓` 导航、`↵`/`Tab` 采纳、`Esc` 关闭，
无法在长列表里定位条目。`/resume` 列表条目数随工作区使用增长，`/model` 候选随 provider 数增长。

目标：面板打开期间输入框照常可用，其内容作为模糊过滤词实时缩小面板候选。

成功标准：

1. 面板打开期间输入框照常编辑（打字符、退格、Ctrl+U/W 等既有编辑键全部生效）。
2. 过滤词即**输入框当前全部内容**，不做暂存/恢复/清空。
3. 过滤词不进 prompt 历史（既有实现只在发送成功且 DB ack 后记录，天然满足）。
4. **打开面板时不过滤**：显示全部候选；输入框内容变化后才开始过滤。
5. `/resume` 按**标题或 session id** 匹配；`/model` 第一级按**模型名或 provider 名**，
   第二级（variants）按 variant 名。
6. 匹配强度：复用 `command_score` 分层（精确 1000 > 前缀 800 > 子串 600 > 子序列 400），
   大小写不敏感，命中项按分数排序。
7. 过滤为空时显示 `no matches`（区别于真无数据的 `no sessions yet` / `no model`）；
   `↵`、`^C` 在空态被吞掉，面板保持打开。
8. 过滤是**即时视图**：原列表全量保留，退格删字符能恢复。

## 2. 已确认边界

| # | 决策点 | 结论 |
| --- | --- | --- |
| 1 | 过滤词载体 | **输入框本体**，不新建面板内 query 字段，不做草稿暂存/恢复。 |
| 2 | 是否进历史 | 不进。历史只在 `on_turn_committed`（DB ack）记录，过滤不触发。 |
| 3 | 打开时是否过滤 | **不过滤**，打开即全量；输入变化后才过滤。 |
| 4 | 匹配字段 | `/resume`：标题 + session id；`/model` 一级：模型名 + provider 名；二级：variant 名。时间列不参与。 |
| 5 | 匹配算法 | 复用 `command_score`：精确/前缀/子串/子序列分层打分，按分排序。 |
| 6 | 空结果 | 显示 `no matches`；`↵`/`^C` 吞掉；面板不关。 |
| 7 | 过滤实现 | 即时视图（`filter` 字段 + 全量 `entries`/`candidates`），不就地裁剪。 |

## 3. 设计

### 3.1 状态（`src/app/state/lx/slash.rs`）

- `HistoryPanel` 增加 `pub filter: String`（默认空）与 `pub filtered: Vec<usize>`
  （过滤后命中的 `entries` 索引，按分数排序；空 filter 时为恒等序列）。
- `ModelPicker` 增加同名字段 `filter` / `filtered`（作用于当前 `stage` 的候选）。
- 抽公共纯函数 `fuzzy_matches(query, targets) -> Option<u32>`：对多个候选字段取最高分
  （复用 `command_score` 的语义）。`filter` 为空返回 `None`（调用方视为“全量，不过滤”）。

### 3.2 视图与命中（`src/ui/main_content/lx/slash.rs`）

- `items()` 按 `filtered` 顺序产出条目；`filter` 非空且 `filtered` 为空时产出单行
  `no matches` 占位。
- `history_time_labels()` 同步按 `filtered` 顺序产出。
- `item_at()` 在过滤为空时返回 `None`（占位不可选）。

### 3.3 按键（`src/app/update/pane.rs`）

`/` 面板分支当前 `_ => return`。改为：

- 编辑类命令（`Char`/`Backspace`/`DeleteTo*`/`DeleteWord*`/`Undo`/`Redo`/`Clear`）
  **放行**到下方编辑分支，编辑后调用 `refresh_slash_filter(state, pane)` 重算过滤。
- `Clear`（`^C`）在 `/resume` 下仍是“复制会话 id”（既有行为，优先级更高）；
  在 `/model` 下仍吞掉。
- 其余键（导航/采纳/关闭）语义不变。

`refresh_slash_filter` 读输入框全文，写回面板 `filter` 并重算 `filtered`，
高亮钳到新范围（`PanelWindow::new(min(active, len-1))` 语义，保持最小滚动）。

### 3.4 文案（`src/ui/text.rs`）

新增 `LX_SLASH_NO_MATCHES: &str = "no matches"`。

## 4. 测试

- `slash.rs`：`fuzzy_matches` 的分层打分、多字段取最高分、大小写不敏感。
- `HistoryPanel`/`ModelPicker`：过滤重算、空结果、退格恢复、高亮钳制。
- `update` 层：面板内打字过滤、`↵` 命中的是过滤后条目、空态 `↵` 吞掉、
  `/resume` 的 `^C` 复制仍是过滤后高亮条目。
- `ui` 层：`items()` 顺序与占位、`item_at` 空态不可选。
