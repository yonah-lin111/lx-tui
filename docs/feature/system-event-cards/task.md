# 消息列表系统事件卡片（`/compact`、`/model`、`/rewind`）：执行任务清单

- 状态：**已完成（分支 `feat/system-event-cards`，待合并 `dev`）**
- 关联文档：
  - 设计方案：`docs/feature/system-event-cards/design.md`
  - 代码审查：`docs/feature/system-event-cards/review.md`

---

## Task 1：步骤标记体系扩展为系统事件菱形

- [x] **`src/ui/text/agent.rs`**：新增 `STEP_MARK_DIAMOND_FILLED`（`◆`）与
  `STEP_MARK_DIAMOND_HOLLOW`（`◇`）。
- [x] **`src/ui/text/tests.rs`**：把两个菱形纳入单列宽/非 PUA 字形测试。
- [x] **`src/ui/main_content/lx/history.rs`**：新增 `StepKind::System`，补齐
  `glyph` / `style_running` / `style_done` / `label` 分支（完成态 accent、运行中橙黄）。
- [x] **`src/ui/main_content/lx/history/tests/cards.rs`**：覆盖系统事件运行/终态/取消
  字形与颜色。

## Task 2：新增系统事件卡片统一排版 `history/system_event.rs`

- [x] 新增 `SystemCard`：标题行（菱形 + accent）+ 固定元信息树体 + 可折叠正文。
- [x] 正文使用工具卡同款 `tree::Section` + `Preview::head(CONTENT_PREVIEW_ROWS = 2)`，
  折叠态追加 `… N more lines`。
- [x] 暴露 `rows()`、`click_span()`（正文首行到卡片末尾，含折叠提示行）与 `draw()`。
- [x] 元信息与正文共用同一份 `tree::layout`，行数、点击面与绘制不各算一份。

## Task 3：重构三类系统事件卡片

- [x] **`history/compact.rs`**：标题 `Context compacted`；元信息为 `model: … · 时间` +
  `covered sequence a–b`；正文为压暗 Markdown 摘要；运行中保留单行
  `◇ Compacting context... · 3s`。
- [x] **`history/model_switch.rs`**：标题 `Model switched`；元信息为模型/时间、指令源、
  完整跃迁链（`A -> B -> A` 不退化）；正文为最后一次注入的系统提示词。
- [x] **`history/rewind.rs`**：标题 `Rewind`；唯一详情为撤销描述，无折叠正文。
- [x] **`src/ui/text.rs`**：标题常量去掉 `--` 包裹线；拆分
  `model_switch_notice_head` / `model_switch_notice_sources`，避免把指令源和模型行
  拼成一段。

## Task 4：运行中 `/compact` 时间线条目

- [x] **`history/timeline.rs`**：`compact.is_running()` 时追加
  `TimelineEntry::RunningCompact`。
- [x] **`history/snapshot.rs`**：构建 `RunningCompact` 条目与稳定块键。
- [x] **`history/paint.rs`**：`BlockSource::RunningCompact` 绘制呼吸条目。
- [x] **`history/source.rs`**：`animating` 纳入 `compact.is_running()`；
  `has_history` 纳入 running（空消息列表也不退回欢迎页）。
- [x] **`history/cache.rs`**：running 块指纹与列表指纹包含幂等键和秒级耗时。

## Task 5：折叠交互与点击面对齐工具卡

- [x] 元信息默认全部可见，只有正文折叠；折叠态保留前 2 行 + `… N more lines`。
- [x] 点击面只覆盖可折叠正文（含提示行），标题与元信息不响应。
- [x] 正文未超出折叠预算时不登记点击面，避免死点击。
- [x] **`history/paint.rs`**：新增 `BlockPaint::SystemEvent`，完成态卡片只排版一次，
  绘制直接复用；运行中条目仍走 `Legacy` 动态路径。

## Task 6：消息级系统提示改用菱形

- [x] **`src/ui/main_content/lx/notice.rs`**：`Failed` / `Interrupted` 行首标记改用
  `StepKind::System`，颜色仍按失败红 / 中断黄覆盖。
- [x] **`src/ui/main_content/lx/notice/tests.rs`**：更新标题文本、字形与样式断言。

## Task 7：规范、测试与缓存正确性

- [x] **`docs/standards/tui-design-requirements.md`**：步骤标记规则补充系统事件菱形；
  新增系统事件卡片语法、折叠预算与点击面口径；消息级通知改为 `◆`。
- [x] 更新 compact / model switch / rewind / cards / text 全部相关测试。
- [x] 新增缓存回归测试：注入提示词变化、覆盖范围变化后复用同一 `PaneCache` 必须
  渲染新内容。
- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test --lib` 通过。
