# 功能代码评审报告（Review）：消息列表系统事件卡片

## 1. 评审范围与概览

- **状态**：**P1–P3 全部修复并验证，已合并至 `dev`（merge commit `a3ae1e13`）**
- **评审工作区**：`.worktrees/lx-system-event-cards`（已清理）
- **关联文档**：
  - `docs/feature/system-event-cards/design.md`
  - `docs/feature/system-event-cards/task.md`
  - 项目规范：`docs/standards/{project-directory-structure,code-writing-standards,tui-design-requirements}.md`
- **评审覆盖模块**：
  - 字形与步骤体系：`src/ui/text/agent.rs`、`src/ui/main_content/lx/history.rs`
  - 卡片排版：`history/system_event.rs`、`compact.rs`、`model_switch.rs`、`rewind.rs`
  - 时间线与几何：`history/timeline.rs`、`snapshot.rs`、`paint.rs`、`cache.rs`、`source.rs`
  - 消息级通知：`src/ui/main_content/lx/notice.rs`
  - 文案：`src/ui/text.rs`

### 问题优先级汇总表（P0 -> P3）

| 编号 | 优先级 | 涉及文件 | 关键点 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | — | — | 未发现数据丢失、崩溃或不可恢复的状态错误 | — |
| **P1-1** | **P1** | `history/cache.rs` | 折叠态开始渲染正文预览后，块指纹仍按旧的“仅展开态才受影响”口径计算：`model_switch` 只在 `expanded` 时哈希注入提示词，`compact` 未哈希 `auto` / 起止 sequence。复用 `PaneCache` 时会渲染过期正文与错误行数 | 已修复 |
| **P1-2** | **P1** | `history/paint.rs`<br>`history/{compact,model_switch,rewind}.rs` | 完成态卡片走 `BlockPaint::Legacy`，`draw` 每帧重新跑 `muted_markdown_lines` 与树体排版；折叠态也渲染完整摘要/提示词，长摘要时形成明显的光栅化开销 | 已修复 |
| **P2-1** | **P2** | `history/system_event.rs` | `width == 0` 时 `tree::layout` 返回空行集，但仍可能登记高度为 0 的点击面（参数为 `body.rows.len() - first`） | 已修复 |
| **P2-2** | **P2** | `history/tests/{compact,model_switch}.rs` | 缺少“同一 `PaneCache` 下正文/范围变化必须刷新”的回归测试，P1-1 类问题无法被现有测试捕获 | 已修复 |
| **P3-1** | **P3** | `history.rs`<br>`app/update/chat/viewport.rs`<br>`history/tests/cards.rs` | 文档与注释漂移：`StepKind` 颜色列表缺少 `System`；`toggle_lx_card` 仍写“整张卡片都是点击面”；字形测试名仍只提圆点/方块/三角 | 已修复 |

---

## 2. 逐项详细分析

### [P1-1] 块指纹未覆盖折叠态正文的全部输入，缓存会渲染过期内容

- **关键点**：
  - `history/cache.rs::model_switch_block_signature`
  - `history/cache.rs::compact_block_signature`
  - `history/paint.rs::BlockPaint::SystemEvent`（修复后卡片内容随 paint 一起缓存）
- **错误原因**：
  1. 旧实现里 `model_switch` 的正文只在展开态渲染，因此指纹写成
     `if expanded { hash.injected_prompt }`。本次改动把提示词预览移到折叠态，
     但指纹没有同步调整——折叠态下同一 `id` 的提示词变化会命中旧块缓存，
     继续显示旧预览与旧行数。
  2. `compact` 的 `covered sequence a–b` 从“仅展开态”变成“始终可见”，但
     `compact_block_signature` 仍只哈希 `id` / 模型 / 时间 / 摘要 / `active` /
     `expanded`，没有 `auto`、`start_sequence`、`end_sequence`。范围变化时列表
     指纹会触发快照重建，但块缓存键不变，`SystemEvent` 直接复用旧卡片。
  3. 该问题在 `Legacy` 路径下只表现为行数漂移；改为缓存整张 `SystemCard` 后会
     直接显示过期正文，因此优先级提升。
- **修复方案**：
  - `model_switch_block_signature` 无条件哈希 `injected_prompt`；
  - `compact_block_signature` 补入 `start_sequence` / `end_sequence` / `auto`；
  - 新增两条缓存回归测试：复用同一 `PaneCache`，修改提示词与覆盖范围后必须
    渲染新内容且不再出现旧文本。
- **验证**：`compact_range_change_invalidates_cached_card`、
  `prompt_change_invalidates_cached_collapsed_preview` 通过。

### [P1-2] 折叠态每帧重建完整卡片（性能回归）

- **关键点**：
  - `history/paint.rs::draw_legacy` → `compact::draw` / `model_switch::draw`
  - `SystemCard::new` 内部的 `muted_markdown_lines` 与 `tree::layout`
- **错误原因**：
  1. 旧折叠态只排版标题+描述，摘要/提示词只在用户展开时渲染。
  2. 新交互要求折叠态展示正文预览，`Legacy` 路径在**每一帧绘制**都会重建
     `SystemCard`，即使用户只看到前 2 行，也会把整段 Markdown 完整渲染一遍；
     快照重建时还会再算一遍行数。
  3. 压缩摘要由模型生成，长度不可控；在历史频繁重绘（流式输出、滚动）时是
     实打实的浪费。
- **修复方案**：
  - 新增 `BlockPaint::SystemEvent { card: Arc<SystemCard> }`：快照阶段构建一次，
    行数、点击面与绘制共用同一份排版结果，绘制时不再重建；
  - 运行中 `/compact` 条目仍走 `Legacy`（呼吸字形与耗时是动态内容，必须现算）；
  - 删除不再使用的 `compact::draw` / `model_switch::draw` / `rewind::draw`，
    `draw_legacy` 对应分支改为显式空分支（正常路径不会到达）。
- **验证**：`cargo test --lib` 4261 项通过；渲染路径不再重复构建卡片。

### [P2-1] 极窄宽度下可能登记零高度点击面

- **关键点**：`history/system_event.rs::SystemCard::new` 的 `click` 计算。
- **错误原因**：
  - `tree::layout(width = 0)` 直接返回空 `rows` / `node_rows`，而 `foldable` 仍可能
    为真（正文已按 1 列宽排版）。此时 `first` 回退为 0，`rows.len() - first` 为 0，
    会向 `snapshot.cards` 注册一个高度为 0 的 `CardRegion`。
  - 渲染宽度为 0 时不会真正绘制，但无意义点击面会污染命中集合，也不符合
    “没有可展开内容就不登记”的口径。
- **修复方案**：`(header.len() + first, body.rows.len().saturating_sub(first))`
  之后 `.filter(|(_, rows)| *rows > 0)`。
- **验证**：编译与既有零宽布局测试通过。

### [P2-2] 缺少缓存失效回归测试

- **关键点**：`history/tests/compact.rs`、`history/tests/model_switch.rs`。
- **错误原因**：现有测试每次调用 `cache()` 都新建 `PaneCache`，只能验证单次渲染，
  无法覆盖“同一缓存键命中旧内容”的路径；P1-1 正是在这种路径上出现的。
- **修复方案**：新增两条测试，复用同一个 `PaneCache`：
  - 修改 compact 的 `start_sequence` / `end_sequence`，断言范围行刷新；
  - 修改 model switch 的 `injected_prompt`，断言折叠态预览刷新。
- **验证**：两条测试在修复前会因缓存命中旧卡片而失败，修复后通过。

### [P3-1] 注释、文档与测试命名漂移

- **关键点**：
  - `history.rs` 的 `StepMark` 颜色列表只写了 Body / Thinking / Tool / Report；
  - `app/update/chat/viewport.rs::toggle_lx_card` 仍写“整张卡片都是点击面”；
  - `history/tests/cards.rs` 测试名仍为 `step_marks_use_dots_squares_and_report_triangles`。
- **错误原因**：交互从“整卡点击”调整为“仅正文点击”后，只改了实现与规范文档，
  局部注释与测试名没有同步，后续维护者容易按旧语义改动。
- **修复方案**：补齐 `StepKind::System` 说明；改写 `toggle_lx_card` 注释；测试名与
  注释加入菱形/系统事件。
- **验证**：`cargo fmt` / `cargo clippy` / `cargo test --lib` 通过。

---

## 3. 验证结果

- `cargo fmt`：通过；
- `cargo clippy --all-targets -- -D warnings`：通过；
- `cargo test --lib`：4263 通过、2 ignored；
- `cargo test history`：430 通过、2 ignored。

已知与本功能无关的既有失败：`tests/prompt_panel_interaction.rs::draft_save_search_and_rename_via_real_mouse_and_keys`
在 `dev` 主 checkout 同样失败（`[pin]` 之后缺少 `○`），不属于本分支改动。
