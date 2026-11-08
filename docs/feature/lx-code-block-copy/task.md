# lx 消息列表编程代码块边框样式与复制功能：任务清单

> 状态：**已完成**
> 日期：2026-11-08
> 设计依据：[design.md](./design.md)

> **视觉修订（最终）**：代码块最终采用仅左侧 1 列 `▎` 的轻量单栏 + `─ <lang> ───── [❐] ─` 规则线头部，无顶边、右边、底边；下文涉及 `▂` / `TopBorder` 的勾选项为历史执行记录。

---

## 1. 当前已完成的设计准备

- [x] 阅读并核对项目规范文档（`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`、`ai-web-deepseek.md`）
- [x] 使用 `codegraph_explore` 与 `codebase-memory-mcp` 梳理 Prompt 区 `&&&` 模板块样式/按钮命中实现、`markdown_render` 增量解析器、`lx/history` 渲染快照、命中与选区复制全链路
- [x] 使用 `grill-me` 逐项向用户确认 4 个核心边界（生效范围、视觉布局与配色、复制行为与拖拽选区净化、语言兜底与窄屏/空块边界）
- [x] 编写详细设计文档 `docs/feature/lx-code-block-copy/design.md` 与执行任务清单 `docs/feature/lx-code-block-copy/task.md`

---

## 2. 执行任务拆分（待用户确认后按序执行）

### T1：重构 `markdown_render` 并新增 `code_block.rs` 子模块

- **目标文件**：
  - `src/ui/markdown_render/code_block.rs`（新建）
  - `src/ui/markdown_render/table.rs`
  - `src/ui/markdown_render.rs`
  - `src/ui/markdown_render/incremental.rs`
- **具体步骤**：
  - [x] 将 `src/ui/markdown_render.rs` 顶部的表格元数据结构体（`TableColumnSpan`、`TableCellSlice`、`TableLogicalRow`、`TableLineKind`、`TableLayoutMeta`、`TableLineRef`）移入 `src/ui/markdown_render/table.rs` 并在 `markdown_render.rs` 中 `pub use` 重导出，控制单文件行数在 850 行以内。
  - [x] 在 `src/ui/markdown_render.rs` 中定义 `CodeLineKind`（`TopBorder`、`Header { block_index, button_cols }`、`Body`），并在 `MdLine` 上新增 `pub code: Option<CodeLineKind>`。
  - [x] 新建 `src/ui/markdown_render/code_block.rs`，实现：
    - `top_border_line(width)`：整行铺满 `text::PROMPT_BLOCK_TOP_BORDER`（`▂`），样式 `style::md_read_rule()`；
    - `header_line(lang, block_index, width)`：首列 `▎`（`style::md_read_rule()`），左侧语言名（空串或缩进块回退为 `"text"`，窄屏以 `…` 截断，样式 `style::muted()`），右侧右对齐 `[❐]`（`text::TEMPLATE_BUTTON_COPY`，样式 `style::template_button()`），记录 `button_cols`；
    - `render_code_line(line, lang, code_chrome, lines)` 与 `prepend_body_bar(spans, width)`：代码行按 `width - 1` 折行后，每个视觉行（含软换行续行及空代码行）首列前置 `▎`（`style::md_read_rule()`）；
    - `extract_code_block(source, target_index)`：按与 `render_blocks_stateful` 完全同口径的块扫描规则，提取第 `target_index` 个代码块（含未闭合流式围栏代码块）的原始未折行代码正文。
  - [x] 在 `ParserState` 中新增 `next_code_block_index: usize`；新增 `pub fn render_wrapped_code_blocks(source: &str, width: u16) -> Vec<MdLine>`（启用 `code_chrome = true`），并在 `IncrementalMarkdownState::update` 中启用 `code_chrome = true`；保持 `render`、`render_wrapped`、`render_wrapped_literal_rules` 为 `code_chrome = false`（确保思考块与工具/汇报卡片不受影响）。

---

### T2：接入 `lx/history` 网格绘制与拖拽选区去装饰复制

- **目标文件**：
  - `src/ui/main_content/lx/grid.rs`
  - `src/ui/main_content/lx/history/paint.rs`
  - `src/ui/main_content/lx/history/snapshot.rs`
  - `src/ui/main_content/lx/selection.rs`
- **具体步骤**：
  - [x] 在 `src/ui/main_content/lx/grid.rs` 中新增 `GridCodeRow`（`Chrome` 与 `Body { bar_col: u16 }`），在 `Grid` 中维护 `code_rows: Vec<Option<GridCodeRow>>` 并提供 `set_code_row` / `code_row_at`。
  - [x] 在 `src/ui/main_content/lx/history/paint.rs` 的 `draw_markdown` 中，根据 `line.code` 调用 `grid.set_code_row(y, ...)` 标记装饰行（`TopBorder` / `Header` → `GridCodeRow::Chrome`）与代码正文行（`Body` → `GridCodeRow::Body { bar_col }`）。
  - [x] 在 `src/ui/main_content/lx/history/snapshot.rs` 的非用户 `PartKind::Text` 回退分支同步调用 `markdown_render::render_wrapped_code_blocks(&part.content, content_width)`。
  - [x] 在 `src/ui/main_content/lx/selection.rs` 的 `copy_text` 中：遇到 `GridCodeRow::Chrome` 整行跳过；遇到 `GridCodeRow::Body { bar_col }` 将有效起始列钳到 `bar_col + 1`，剥离 `STEP_GUTTER` 空白与 `▎` 左边框，保留代码内部缩进与空代码行。

---

### T3：接入 `[❐]` 复制按钮命中、剪贴板写入与窗格内 Toast

- **目标文件**：
  - `src/ui/main_content/lx/history_hit.rs`
  - `src/event/mouse/press.rs`
  - `src/event/mouse/hit.rs`
- **具体步骤**：
  - [x] 在 `src/ui/main_content/lx/history_hit.rs` 的 `LxChatHit` 枚举中新增 `CopyCode(String)` 变体。
  - [x] 实现 `code_copy_at_source` 与 `task_agents_code_copy_at`，并在 `chat_hit` 中接入（主对话在 `sticky_rect` 拦截之后、`view.body` 之内检测；Subagent 详情页在 `TaskTab::Agents` 输出区检测）。
  - [x] 在 `src/event/mouse/press.rs`（或抽取出 `hit::apply_lx_chat_hit` 辅助函数以控制 `press.rs` 行数）中处理 `LxChatHit::CopyCode(code)`：当 `!code.is_empty()` 时调用 `copy_with_toast(state, &code, Some(pane))`，在当前 `lx` 窗格内弹出 `Clipboard: Copied N chars` Toast，且不清除已有文本选区。

---

### T4：补充单元测试与同步规范文档

- **目标文件**：
  - `src/ui/markdown_render/tests.rs`
  - `src/ui/markdown_render/incremental/tests.rs`
  - `src/ui/main_content/lx/history/tests.rs`
  - `src/event/tests/mouse_click.rs`
  - `docs/standards/tui-design-requirements.md`
- **具体步骤**：
  - [x] 在 `src/ui/markdown_render/tests.rs` 中新增 `render_wrapped_code_blocks` 与 `extract_code_block` 的单元测试（有语言/无语言默认 `text`/4 空格缩进块、窄屏语言名 `…` 截断、`[❐]` 常显、软换行续行 `▎` 前缀、多块/未闭合流式块/空块提取）。
  - [x] 在 `src/ui/markdown_render/incremental/tests.rs` 中更新增量一致性基准为 `render_wrapped_code_blocks`，并新增多代码块流式定型测试。
  - [x] 在 `src/ui/main_content/lx/history/tests.rs` 与 `src/event/tests/mouse_click.rs` 中新增：
    - 助手正文代码块渲染与 `[❐]` 点击复制 + `lx` 窗格内 Toast 测试；
    - 空代码块点击不弹 Toast 测试；
    - 置顶用户行（sticky header）与候选面板遮挡防穿透测试；
    - 鼠标框选代码块 `copy_text` 自动剥离 `▂`、头部行与 `▎` 左边框的测试。
  - [x] 更新 `docs/standards/tui-design-requirements.md` 中关于助手正文代码块样式与复制交互的描述。
  - [x] 使用 `rtk cargo fmt --check`、`rtk cargo clippy --all-targets -- -D warnings`、`rtk cargo test` 对受影响模块执行精确校验。
