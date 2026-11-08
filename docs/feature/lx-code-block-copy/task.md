# lx 消息列表编程代码块边框样式与复制功能：任务清单

> 状态：**已完成（含复审修复）**
> 日期：2026-11-08
> 设计依据：[design.md](./design.md)
> 复审报告：[review.md](./review.md)

---

## 1. 设计准备

- [x] 阅读项目规范文档（`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`、`web-deepseek.md`）
- [x] 使用 `codegraph_explore` 与 `codebase-memory-mcp` 梳理 Prompt 区模板块、`markdown_render` 增量解析器、`lx/history` 渲染快照、命中与选区复制链路
- [x] 使用 `grill-me` 确认 4 个核心边界（生效范围、视觉布局、复制/选区语义、语言/窄屏/空块）
- [x] 编写 `design.md` 与本任务清单

---

## 2. 实现任务

### T1：代码块渲染与源码提取

- **目标文件**：
  - `src/ui/markdown_render/code_block.rs`（新建）
  - `src/ui/markdown_render/code_block/tests.rs`（新建）
  - `src/ui/markdown_render.rs`
  - `src/ui/markdown_render/incremental.rs`
  - `src/ui/markdown_render/table.rs`

- [x] 将表格元数据结构下沉到 `markdown_render/table.rs` 并重导出，控制 `markdown_render.rs` 行数。
- [x] 定义 `CodeLineKind`（`TopBorder`、`Header { block_index, button_cols }`、`Body`），在 `MdLine` 上新增 `code` 字段。
- [x] 实现 `top_border_line`：独立一行铺满内容宽度 `▂`，无右边距。
- [x] 实现 `header_line`：`▎ <lang> ... [❐] `，语言左空白 1 列、按钮右空白 1 列，窄屏 `…` 截断，记录 `button_cols`。
- [x] 实现 `render_code_line`、`sanitize_code_line`、`prepend_body_bar`：正文按 `width - 1` 折行，每个视觉行前置 `▎`；`\t` 按 4 列展开，其余控制字符替换为空格。
- [x] 实现 `normalize_lang_label`：空语言回退 `text`，控制字符净化。
- [x] 实现 `extract_code_block`：围栏/4 空格缩进/未闭合流式块，按块序号提取原始代码；剥离行尾 `\r`。
- [x] `ParserState` 新增 `next_code_block_index` 与 `after_code_block`；`render_wrapped_code_blocks` 启用 `code_chrome = true`；`IncrementalMarkdownState::update` 同步启用。
- [x] 间距：代码块上方不补空行，下方有后续内容时补 1 行；消息末尾不补；相邻代码块共享 1 行。

### T2：lx 历史网格与拖拽选区

- **目标文件**：
  - `src/ui/main_content/lx/grid.rs`
  - `src/ui/main_content/lx/history/paint.rs`
  - `src/ui/main_content/lx/history/snapshot.rs`
  - `src/ui/main_content/lx/selection.rs`

- [x] `GridCodeRow::{Chrome, Body { bar_col }}` 与 `set_code_row` / `code_row_at`。
- [x] `draw_markdown` 按 `line.code` 登记代码行元数据。
- [x] 助手正文增量 Markdown 路径（`cache.markdown_block`）启用 code chrome；`snapshot.rs` 的 `Text` fallback 仅作为 match 穷尽分支保留，非用户正文实际由外层 guard 捕获。
- [x] `copy_text` / `paint_selection`：跳过 Chrome 行，正文从 `bar_col + 1` 起算，保留代码缩进与空代码行。
- [x] 修复多行选区端点都在 `▎` 左侧时中间代码正文被整段跳过的问题。

### T3：复制按钮命中与 Toast

- **目标文件**：
  - `src/ui/main_content/lx/history_hit.rs`
  - `src/ui/main_content/lx/history_hit/tests.rs`
  - `src/event/mouse/press.rs`
  - `src/event/mouse/hit.rs`
  - `src/event/tests/lx_view.rs`

- [x] 新增 `LxChatHit::CopyCode(String)`。
- [x] 实现 `code_copy_at_source` 与 `task_agents_code_copy_at`，主对话在 sticky 拦截之后、`view.body` 内命中；Agents 详情页共用同一套逻辑。
- [x] 事件层 `CopyCode` 非空时 `copy_with_toast(state, &code, Some(pane))`：写剪贴板、窗格内 Toast、保留选区；空块不触发。

### T4：测试与规范文档

- **目标文件**：
  - `src/ui/markdown_render/code_block/tests.rs`
  - `src/ui/markdown_render/incremental/tests.rs`
  - `src/ui/main_content/lx/tests.rs`
  - `src/ui/main_content/lx/history_hit/tests.rs`
  - `src/ui/main_content/lx/selection/tests.rs`
  - `src/event/tests/lx_view.rs`
  - `docs/standards/tui-design-requirements.md`

- [x] 代码块几何、语言回退/截断、窄宽、软换行、空代码行、上下间距、制表符/控制字符、语言标签净化、CRLF 提取、围栏/缩进/未闭合提取、列表/引用/表格后的块序号对齐。
- [x] 增量一致性基准改为 `render_wrapped_code_blocks`。
- [x] lx 真实渲染：顶边、头部、正文左栏、制表符展开、无控制字符。
- [x] `[❐]` 命中、原始代码返回、子 Agent 详情页、sticky 优先、空块不弹 Toast、窗格内 Toast、选区保留。
- [x] 选区 Chrome 行不可复制/反显、正文左栏剥离、左侧起止多行选区保留代码。
- [x] 更新 `docs/standards/tui-design-requirements.md` 中代码块样式与复制交互说明。

---

## 3. 复审修复（review.md）

- [x] **P0-1**：控制字符 panic —— 制表符展开 + `sanitize_code_line` + lx blit `drawable_char` 兜底；真实渲染回归。
- [x] **P1-1**：左侧起止多行选区漏代码 —— 删除 `start.1/end.1` 全局跳过，补复制与反显测试。
- [x] **P1-2**：语言标签控制字符导致排版/热区错位 —— `normalize_lang_label` 净化，补测试。
- [x] **P2-1**：CRLF 复制带 `\r` —— `extract_code_block` 剥离行尾 `\r`，补测试。
- [x] **P2-2**：设计与任务文档同步最终实现、测试落点与复审记录。
- [x] **P2-3**：补齐选区、语言、CRLF、块序号对齐测试。

---

## 4. 验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test markdown_render`
- [x] `cargo test ui::main_content::lx`
- [x] `cargo test event::tests::lx_view`
- [x] `cargo test --lib`
