# 消息列表系统事件卡片（`/compact`、`/model`、`/rewind`）：设计方案

> 状态：**已完成（分支 `feat/system-event-cards`，待合并 `dev`）**
>
> 关联文档：
> - 任务清单：`docs/feature/system-event-cards/task.md`
> - 代码审查：`docs/feature/system-event-cards/review.md`
> - 项目规范：`docs/standards/tui-design-requirements.md`
> - 相关历史实现：`docs/feature/lx-agent-prompt-and-model-switch/`

---

## 1. 背景与目标

### 1.1 现状分析

1. `/compact` 检查点、`/model` 切换提示、`/rewind` 撤销提示原本是**独立的两行通知带**：
   `-- Model switched --` / `-- Context compacted --` / `-- Rewind --` + 一行描述。
2. 这三类条目没有复用 assistant 步骤的左侧状态标记，和正文圆点、工具方块、汇报三角
   不在同一套视觉语法里。
3. `/compact` 是异步长耗时操作，历史列表里没有 in-flight 条目；用户只能在输入框上方
   的 loading 行看到耗时，滚动历史时缺少锚点。
4. 描述一长就整行硬折行，续行从第 0 列开始，和工具卡片的树体读法不一致。

### 1.2 核心目标

1. 三类系统事件统一为**步骤卡片**：菱形标记 + 事件标题 + 树体详情。
2. 复用工具卡的树体语法（`├─` / `└─` / `│`），**元信息常显，只有正文内容可折叠**。
3. `/compact` 运行中在时间线末尾显示呼吸 loading 条目（`◇ Compacting context... · 3s`），
   完成后由完成态卡片替换。
4. 消息级系统提示（`Failed` / `Interrupted`）同步改用菱形标记，保持“系统自身生命周期
   事件”与文字数据/工具调用的区分。
5. 点击面只覆盖可折叠正文，标题与元信息不响应（对齐命令卡片的参数/输出点击区）。

### 1.3 非目标

- 不改数据库 schema，不新增持久化字段。
- 不改 `/compact`、`/model`、`/rewind` 的业务状态机与命令入口。
- 不为系统事件增加动作按钮、右键菜单或悬停高亮。
- 不把系统事件并入 assistant 消息的复制选区与模型上下文。

---

## 2. 交互与视觉规范

### 2.1 字形与配色

| 类别 | 字形 | 完成态 | 运行中 |
| :--- | :--- | :--- | :--- |
| 正文 / 思考 | 圆点 `●` / `○` | 角色色 | 两帧呼吸 |
| 工具调用 | 方块 `■` / `□` | 翡翠绿 | 橙黄呼吸 |
| 子 agent 汇报 | 上三角 `▲` / `△` | 工具档色 | 橙黄呼吸 |
| **系统事件** | **菱形 `◆` / `◇`** | **accent** | **橙黄呼吸** |

- 字形只由 `ui::text::agent::STEP_MARK_*` 提供，`StepMark::glyph(kind, frame, animating)`
  按 `StepKind::System` 选取；渲染层不写死字符。
- 消息级通知（`Failed` / `Interrupted`）是系统事件，行首同样用实心菱形，颜色仍由
  失败红 / 中断黄覆盖。

### 2.2 卡片结构

```text
◆ Model switched
  ├─ model: DeepSeek V4.1 Flash (max) · 16:59
  ├─ instructions: core/model, core/agents_md
  ├─ switched Claude Sonnet 4 (high) -> DeepSeek V4.1 Flash (max)
  └─ Your Model
     • Name: DeepSeek V4.1 Flash
     … 4 more lines
```

- 标题行是唯一不带连接符的行：`<菱形> <事件名>`（accent）；标题不再带 `--` 包裹线。
- 详情节点走工具卡同一套 `tree` 排版：节点首行 `├─` / `└─`，非末节点续行 `│`，
  末节点续行等宽空白；连接符 `style::chat_notice_connector()`，正文 `muted`。
- 元信息（模型、时间、覆盖范围、指令源、跃迁链）**默认全部可见**。
- 正文（压缩摘要 / 注入提示词）是唯一折叠项：折叠态保留前 `CONTENT_PREVIEW_ROWS = 2`
  个物理行 + `… N more lines`；展开态铺开全部，继续使用压暗 Markdown。
- 正文未超出折叠预算时不登记点击面。
- 点击面只覆盖正文区（含 `… N more lines`），标题与元信息不响应点击。

### 2.3 运行中的 `/compact`

```text
◇ Compacting context... · 3.2s
```

- 追加在时间线末尾；琥珀色呼吸菱形（`chat_step_mark_alive`），耗时 `muted`。
- 完成后 running 状态清除、检查点入库，下一次快照由完成态卡片替换该条目。
- 与输入框上方的 loading 行共用同一份 `LxCompact { running, elapsed_ms }` 状态。

---

## 3. 实现结构

| 模块 | 职责 |
| :--- | :--- |
| `history/system_event.rs` | `SystemCard`：标题 + 固定元信息 + 可折叠正文，统一行数/点击面/绘制 |
| `history/compact.rs` | `/compact` 完成态与运行中条目 |
| `history/model_switch.rs` | 模型切换卡片（元信息 + 提示词正文） |
| `history/rewind.rs` | 撤销提示（只有描述，无折叠） |
| `history/timeline.rs` | 时间线追加 `TimelineEntry::RunningCompact` |
| `history/snapshot.rs` | 构建 `SystemCard`、登记正文点击面、运行中条目快照 |
| `history/paint.rs` | `BlockPaint::SystemEvent` 缓存已排版卡片，绘制不再重建 |
| `history/cache.rs` | running 条目签名、compact/model-switch 签名补全 |
| `history/source.rs` | `animating` 纳入 `compact.is_running()`；`has_history` 纳入 running |
| `notice.rs` | 消息级通知改用 `StepKind::System` 菱形 |

### 3.1 `SystemCard`

```rust
SystemCard::new(width, header, fixed_nodes, content_lines, expanded)
```

- `fixed_nodes` 走 `Section::fixed`，永远可见；
- `content_lines` 走 `Section::new(..., Preview::head(2))`，`expanded` 时为 `None`；
- 内部调用 `tree::layout` 得到物理行、节点首行与隐藏行数；
- `click_span()` 返回正文首行到卡片末尾的 `(offset, rows)`，快照据此登记 `CardRegion`；
- `rows()` 与绘制共用同一份排版结果。

### 3.2 缓存边界

- `BlockPaint::SystemEvent { card: Arc<SystemCard> }` 缓存完成态卡片，避免每帧重跑
  `muted_markdown_lines` 与树体排版；
- 运行中条目仍是 `Legacy`（呼吸字形与耗时为动态内容，每帧现算）；
- 块指纹必须覆盖所有影响排版的输入：
  - compact：`id`、模型、时间、起止 sequence、摘要、`active`、`auto`、`expanded`；
  - model switch：区间内全部条目字段 + 注入提示词（折叠态也渲染预览）+ `expanded`；
  - running：幂等键 + 秒级耗时。

---

## 4. 测试与验证

- 字形：菱形单列宽、非 PUA，系统事件运行/终态字形切换。
- 卡片：折叠态元信息全显、正文预览与剩余行数、展开铺开全部、`├─`/`└─`/`│` 前缀、
  压暗 Markdown、宽度折行与行数一致。
- 交互：点击面从正文首行开始、只覆盖正文（含提示行）、正文未超预算时不登记。
- 运行中：时间线末尾条目、呼吸相位、耗时格式化、`has_history` 为真。
- 缓存：注入提示词 / 覆盖范围变化后复用同一 `PaneCache` 必须渲染新内容。
- 通知：`Failed` / `Interrupted` 行首菱形与级别配色。

验证命令：

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

---

## 5. 已确认决策（Grill Me）

| 编号 | 决策 |
| :--- | :--- |
| D1 | 采用系统步骤标记卡片范式，而非独立通知带 |
| D2 | 系统事件使用菱形 `◆` / `◇` |
| D3 | 树体与元信息默认全显，只有正文按 2 行预算折叠 |
| D4 | 折叠/展开复用工具卡交互：正文区可点击，标题与元信息不响应 |
| D5 | 只有异步 `/compact` 增加运行中呼吸条目，`/model`、`/rewind` 直接完成态 |
| D6 | 消息级 `Failed` / `Interrupted` 通知同步改为菱形 |
