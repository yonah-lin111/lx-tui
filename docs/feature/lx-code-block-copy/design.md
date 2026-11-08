# lx 消息列表编程代码块边框样式与复制功能：设计方案

> 状态：**已完成**
> 日期：2026-11-08
> 项目：`/Users/yonah/projects/agent/lx-tui`
> 作用域：`src/ui/markdown_render*`、`src/ui/main_content/lx/*`、`src/event/mouse/press.rs`、`docs/standards/tui-design-requirements.md`
> 配套文档：[task.md](./task.md)

> **视觉修订（最终）**：初版曾采用 `▂` 顶边 + `▎` 粗色条与 `▎<lang> ... [❐]` 头部；最终结构为顶部独立一行 `▂`（`CODE_BLOCK_TOP_BORDER`）、下一行无边框头部 `▎ <lang> ... [❐] `（左侧边框与顶边闭合）、正文左侧 1 列 `▎` 单栏，无右边、底边；代码行制表符按 4 列展开、控制字符替换为空格。本文正文中涉及旧头部几何的描述均为历史设计记录，最终视觉以 `src/ui/markdown_render/code_block.rs` 与 `docs/standards/tui-design-requirements.md` 为准。

---

## 1. 背景与目标

### 1.1 现状问题分析

1. **缺乏代码块边界与一键复制能力**：
   当前 `src/ui/markdown_render.rs` 中的 `render_code` 仅输出语法高亮后的代码正文行，不绘制任何顶/左边框、语言标签或复制按钮；同时 `src/ui/main_content/lx/history_hit.rs` 中的 `LxChatHit` 虽留有历史注释（`/// 代码块复制按钮携带正文（自有 String），因此不是 Copy`），但实际只剩 `PromptHeader` 与 `JumpBottom` 两个变体，用户无法一键复制 AI 生成的完整代码块。
2. **窄屏软换行后选区复制体验差**：
   当长代码行（如单行 Shell 命令或长函数签名）在窄窗口发生软换行时，用户用鼠标框选复制会混入折行断行或前导缩进空白，无法直接拿到未断行的原始代码文本。

### 1.2 核心目标

在 `src/ui/main_content/lx/` 消息列表的 **AI 助手正文块（Assistant 正文步骤，含主对话与 Subagent 详情页对话）** 中，为编程代码块（围栏代码块 ```` ``` ```` 与 4 空格缩进代码块）添加参考 Prompt 区域 `&&&` 模板块风格的视觉容器与一键复制功能：

- **顶部边框与左侧边框闭合**：代码块顶部独立一行铺满下 1/4 块 `▂`（`CODE_BLOCK_TOP_BORDER`，样式 `md_read_rule`），下一行头部与正文均以 `▎` 起始，左侧边框连续、与顶边在左上角闭合；无右边、底边；
- **上下间隔**：代码块与前后相邻块之间固定空一行（消息开头的代码块不前置空行、消息末尾的代码块不后置空行，避免与消息块间距叠加），跨增量分块由解析状态保留；
- **顶部边框下一行（Header 行）**：无边框，左侧 `▎ ` 后显示语言名（围栏未指定语言或 4 空格缩进代码块显示为 `text`，超窄以 `…` 截断），右侧常显 `[❐]`（`TEMPLATE_BUTTON_COPY`）复制按钮；语言左侧固定 1 列空白，按钮右侧固定 1 列空白；
- **代码行净化**：代码行制表符按 4 列制表位展开，其余控制字符（`\r`、`\x1b` 等）替换为空格，避免 ratatui 控制字符 panic；
- **点击复制与 Toast 反馈**：点击 `[❐]` 立即复制该代码块的**原始未折行纯代码文本**（流式未闭合时复制当前已生成代码，空代码块不触发复制），并在当前 `lx` 窗格内弹出标准剪贴板 Toast（标题 `Clipboard`，正文 `Copied N chars`）；
- **无 Tooltip**：悬停在 `[❐]` 上不注册、不弹出任何 Tooltip；
- **鼠标拖拽选区净化**：鼠标框选代码块行并按 `Ctrl/Cmd+C` 时，自动跳过第一行语言头部行，并剥离代码正文行左侧的 `STEP_GUTTER` 空白与 `▎` 左栏，保证局部框选也能复制出干净且保留原始缩进的代码。

---

## 2. 已确认边界与决策记录（Grill Me）

| 编号 | 决策项 | 已确认方案 | 用户确认依据 |
| :--- | :--- | :--- | :--- |
| **Q1** | **生效范围** | 仅在 **AI 助手正文消息（Assistant `PartKind::Text` 步骤，含主对话与 Subagent 详情页 `TaskTab::Agents` 中的助手正文）** 启用新样式与复制按钮；**不在**用户发送气泡（`BlockPaint::UserText`）、思考块（`BlockPaint::Thinking`）以及工具/命令/子 Agent 汇报/Compact/ModelSwitch 的压暗 Markdown（`muted_markdown_lines`）中启用。 | 问题 1：用户确认「使用推荐」 |
| **Q2** | **视觉布局与配色** | 1. **顶边**：块首独立占 1 行，从 `STEP_GUTTER`（2 列）后到内容区右缘画满 `▂`（`PROMPT_BLOCK_TOP_BORDER`）；<br>2. **左边**：顶边下每个视觉行（含 Header 行、代码行及软换行续行）首列画 `▎`（`PROMPT_BLOCK_BAR`）；<br>3. **内部第一行**：左侧紧贴 `▎` 显示语言名（无语言写 `text`），右侧右对齐显示 `[❐]`（`TEMPLATE_BUTTON_COPY`），下方直接接代码正文，不加分割横线；<br>4. **配色**：顶边 `▂` 与左边 `▎` 使用 `style::md_read_rule()`（GitHub Dark 次要灰 `#7d8590`，与表格框线同色、不与思考块 `Indexed 218` 粉色撞色）；语言名与 `[❐]` 按钮使用 `style::muted()`（与 Prompt 区 `template_button()` 一致）。 | 问题 2：用户确认「使用推荐」 |
| **Q3** | **复制语义与拖拽选区** | 1. **点击 `[❐]` 按钮**：复制该代码块的**原始未折行纯代码**（不含 ```` ``` ```` 围栏、不含顶边 `▂`、不含语言头部行、不含左边 `▎`）；流式未闭合时复制已生成代码；成功后在当前 `lx` 窗格内弹 `Clipboard: Copied N chars` Toast；被置顶用户消息行（sticky header）或底部 `@`/`/`/粘贴候选面板遮挡时不穿透触发；点击后保留已有选区高亮。<br>2. **鼠标拖拽选区 + `Ctrl/Cmd+C`**：框选提取文本时自动跳过顶边 `▂` 行与语言头部行，并剔除每行代码前的 `STEP_GUTTER` 与 `▎` 左边框，保留代码自身缩进。 | 问题 3：用户确认「使用推荐」 |
| **Q4** | **语言兜底、窄屏截断与空块** | 1. **语言名**：```` ``` ```` 后 `trim` 为空或 4 空格缩进代码块统一显示 `text`；非空则原样显示 `trim` 后的语言字符串；<br>2. **窄屏优先级**：对齐 Prompt 区 `&&&` 模板块，右侧 `[❐]` 常显优先（钳在 `▎` 之后），宽度不足时语言名在 `[❐]` 左侧预留 1 列空白处以 `…`（`style::muted()`）截断；<br>3. **空代码块**：正常渲染顶边 `▂` 与 `▎<lang> ... [❐]` 头部行；若代码正文为空（`""`），点击 `[❐]` 不写剪贴板、不弹 `Copied 0 chars` 提示。 | 问题 4：用户确认「继续」 |

---

## 3. 代码检索与索引覆盖核验

设计前已通过 `codegraph_explore` 与 `codebase-memory-mcp` 检索全部相关符号、调用链与测试影响面，并对引用路径执行 `check_index_coverage` + 源文件精读核验：

| 文件路径 | 索引状态 | 核心职责与本次改动定位 |
| :--- | :--- | :--- |
| `src/ui/markdown_render.rs` | `no_recorded_issue`（已读源码） | 面向阅读的 Markdown 渲染入口、`MdLine` 定义、`ParserState` 块级状态机与 `wrap_md_line` 折行逻辑。新增 `code_block` 子模块挂载、`CodeLineKind` 元数据与 `render_wrapped_code_blocks` / `extract_code_block` 导出。 |
| `src/ui/markdown_render/code_block.rs` | **新建文件** | 封装代码块顶边行 `▂`、内部首行 `▎<lang> ... [❐]`、正文行 `▎` 前缀拼装与窄屏截断，以及与 `ParserState` 同口径的 `extract_code_block` 原始代码提取函数。 |
| `src/ui/markdown_render/incremental.rs` | `metadata_match`（已读源码） | 流式 Markdown 增量渲染状态机（`IncrementalMarkdownState`）。开启 `code_chrome = true`，维护跨块递增的 `next_code_block_index`，保证 `prefix + tail` 与全量 `render_wrapped_code_blocks` 逐行严格相等。 |
| `src/ui/main_content/lx/grid.rs` | `no_recorded_issue`（已读源码） | `lx` 历史与任务页离屏字符网格 `Grid`。与 `table_rows` 并列新增 `code_rows: Vec<Option<GridCodeRow>>` 及 `set_code_row` / `code_row_at`，供选区复制识别装饰行与左边框列。 |
| `src/ui/main_content/lx/history/paint.rs` | `no_recorded_issue`（已读源码） | `draw_markdown` 将 `MdLine` 绘制到 `Grid`。当 `line.code` 存在时调用 `grid.set_code_row` 记录 `GridCodeRow::Chrome` 或 `GridCodeRow::Body { bar_col }`。 |
| `src/ui/main_content/lx/history/snapshot.rs` | `no_recorded_issue`（已读源码） | 构建历史快照。助手正文回退分支同步改用 `markdown_render::render_wrapped_code_blocks`。 |
| `src/ui/main_content/lx/history_hit.rs` | `metadata_match`（已读源码） | 历史区鼠标命中计算。在 `LxChatHit` 中恢复 `CopyCode(String)` 变体，新增 `code_copy_at_source`（主对话与 `TaskTab::Agents` 子 Agent 详情页共用），直接基于就绪快照的 `BlockPaint::Markdown` 行元数据命中 `[❐]` 按钮并按 `(message_index, step_index, block_index)` 提取源码。 |
| `src/ui/main_content/lx/selection.rs` | `no_recorded_issue`（已读源码） | 历史区文本选区反显与 `copy_text` 提取。在 `copy_text` 中跳过 `GridCodeRow::Chrome` 行，并对 `GridCodeRow::Body { bar_col }` 将起始列钳到 `bar_col + 1` 之后，剔除 `STEP_GUTTER` 空白与 `▎`。 |
| `src/event/mouse/press.rs` | `no_recorded_issue`（已读源码） | 鼠标左键按下事件分派。在 `chat_hit` 匹配分支中处理 `LxChatHit::CopyCode(code)`：非空时调用 `copy_with_toast(state, &code, Some(pane))`，保留已有选区高亮。 |
| `docs/standards/tui-design-requirements.md` | 文档（已读源码） | 同步更新 `§lx 聊天` 中关于助手正文代码块样式（顶边 `▂`、左边 `▎`、首行 `<lang>`/`text` + `[❐]` 复制按钮）与复制反馈的规范说明。 |

---

## 4. 核心机制与架构数据流设计

### 4.1 视觉几何模型（对齐 Prompt 区 `&&&` 模板块）

在 `lx` 消息列表中，助手正文步骤左侧固定有 `STEP_GUTTER = 2` 列（第 0 列绘制步骤圆点 `●`/`○`，第 1 列为空格）。代码块从 `x = rect.x + STEP_GUTTER` 开始，可用列宽为 `content_width = width.saturating_sub(STEP_GUTTER)`：

```text
列坐标（相对 body.x）:
0 1 2 3 4 5 6 7 8 ...                                  width-3 width-2 width-1
├─┼─┼─┴─┴─┴─┴─┴─┴─────────────────────────────────────────┴───────┴───────┤
●   Here is the Rust implementation:
                                                                          ← 段落行
    ▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂ ← TopBorder 行：整行 `▂` (style::md_read_rule)
    ▎rust                                                              [❐] ← Header 行：col 2 为 `▎`，col 3.. 为语言名 (muted)，右对齐 `[❐]` (muted)
    ▎fn main() {                                                           ← Body 行：col 2 为 `▎` (md_read_rule)，col 3.. 为语法高亮代码
    ▎    println!("hello world");                                          ← Body 行：保留 4 空格代码缩进
    ▎}                                                                     ← Body 行
```

当窗口缩窄导致：
1. **Header 行语言名超宽**（如 `width = 12, content_width = 10, lang = "typescript"`）：
   - 第 0 列（相对代码块起点 `STEP_GUTTER`）：`▎`（1 列）
   - 右端 `[❐]` 按钮固定占 3 列，起点 `btn_start = (10 - 3).max(1) = 7`，列区间 `[7, 10)`
   - 语言名最大可用宽度 `max_lang_w = btn_start.saturating_sub(1).saturating_sub(1) = 5` 列
   - `"typescript"` 截断为 `"type…"`（5 列），后补 1 列空格，再接 `"[❐]"`：
     ```text
     ▎type… [❐]
     ```
2. **代码正文行超宽软换行**：
   - 代码正文可用折行宽度为 `body_width = content_width.saturating_sub(1).max(1)`（扣除左边框 `▎` 的 1 列）；
   - 软换行拆出的**每一个视觉续行**首列同样绘制 `▎`（`style::md_read_rule()`），紧随其后绘制该段代码文本，与 Prompt 区 `&&&` 块软换行续行每行首列画 `▎` 完全一致。

### 4.2 端到端数据流图

```text
[1. 渲染与增量缓存链路]
ChatMessageView (PartKind::Text, !is_user)
  │
  ▼
PaneCache::markdown_block(key, &part.content, content_width)
  │
  ▼
IncrementalMarkdownState::update(source, content_width)
  ├─ 稳定前缀 (stable_lines: Arc<Vec<MdLine>>) ← 已闭合行 + 围栏内已换行代码行直接定型
  └─ 活跃尾部 (tail: Vec<MdLine>)             ← 克隆 ParserState (含 next_code_block_index) 解析尾部
       │
       ├─ 遇到围栏起始行 ```lang 或 4 空格缩进块起点:
       │    1) 产出 MdLine { code: Some(CodeLineKind::TopBorder), ... }      ("▂▂▂...")
       │    2) 产出 MdLine { code: Some(CodeLineKind::Header {               ("▎rust ... [❐]")
       │         block_index, button_cols: (btn_start, btn_end)
       │       }), ... }
       └─ 遇到代码块内每一行:
            产出代码高亮 MdLine { code: Some(CodeLineKind::Body), ... }
            经 wrap_md_line 按 (content_width - 1) 折行，每个视觉行前置 ("▎", md_read_rule())
  │
  ▼
history/paint.rs :: draw_markdown()
  ├─ 将 MdLine spans 写入 Grid (x = rect.x + STEP_GUTTER + line.indent)
  └─ 若 line.code.is_some():
       ├─ TopBorder | Header => grid.set_code_row(y, GridCodeRow::Chrome)
       └─ Body               => grid.set_code_row(y, GridCodeRow::Body { bar_col })

[2. 点击 [❐] 一键复制链路]
鼠标左键按下 (event/mouse/press.rs)
  │ (在 paste/mention/refer/slash 候选面板拦截之后、选区按下之前)
  ▼
ui::main_content::lx::chat_hit(inner, &target.lx, col, row, cache.pane(pane))
  ├─ 1) chat_bar::jump_bottom_rect 命中 -> LxChatHit::JumpBottom
  ├─ 2) 若处于 TaskTab::Agents 子 Agent 详情页 -> task_agents_code_copy_at -> LxChatHit::CopyCode(code)
  └─ 3) 主对话历史 (chat_geometry_is_live):
         ├─ sticky_rect 命中 -> LxChatHit::PromptHeader (置顶条优先拦截，防穿透)
         └─ view.body 内 -> code_copy_at_source(view.body, view.scroll, source, col, row, cache)
              ├─ 从就绪快照定位 content_row 落在哪个 SnapBlock (BlockPaint::Markdown)
              ├─ 读取对应行 MdLine.code == Some(CodeLineKind::Header { block_index, button_cols })
              ├─ 校验 rel_col 落在 [STEP_GUTTER + button_cols.0, STEP_GUTTER + button_cols.1)
              └─ 调用 markdown_render::extract_code_block(&part.content, block_index) 提取原始未折行代码
  │
  ▼
LxChatHit::CopyCode(code)
  └─ if !code.is_empty() { copy_with_toast(state, &code, Some(pane)); }
     (写系统剪贴板 + 在当前 lx 窗格内弹出 "Clipboard" / "Copied N chars" Toast，不清空已有选区)

[3. 鼠标拖拽选区 Ctrl/Cmd+C 复制链路]
ui::main_content::lx::copy_text(area, lx, range, cache)
  ├─ 分段渲染到无滚动条 Grid (render_chunk)
  └─ 逐行扫描 y:
       ├─ grid.code_row_at(y) == Some(GridCodeRow::Chrome) -> continue (跳过 `▂` 顶边行与 `▎lang [❐]` 头行)
       ├─ grid.code_row_at(y) == Some(GridCodeRow::Body { bar_col }) ->
       │    将提取起始列 col0 钳到 col0.max(bar_col + 1)，剥离 STEP_GUTTER 与 `▎` 左边框
       └─ 其余普通行 / 表格行保持原有提取逻辑
```

### 4.3 为什么不在 `HistorySnapshot.cards` 中预存每个代码块的 `String`？

1. **流式零拷贝与内存友好**：在流式输出期间，`HistorySnapshot` 每帧（或每 80ms）重建。如果在 `CardRegion` 中克隆每段代码块的完整 `String`，一条包含多个大代码块的长回复会在每帧产生大量无意义的堆分配与拷贝。
2. **渲染与命中 100% 同源**：`SnapBlock` 的 `BlockPaint::Markdown { prefix, tail, .. }` 已经缓存在 `PaneCache` 中，其中的 Header 行 `MdLine` 直接携带 `CodeLineKind::Header { block_index, button_cols }`（仅 `Copy` 的整数字段，零堆分配）。仅当用户真正点击 `[❐]` 按钮的那一刻，才按 `(message_index, step_index, block_index)` 从消息原文扫描一次提取 `String`。

---

## 5. 各模块详细改动与关键代码

### 5.1 新建 `src/ui/markdown_render/code_block.rs`

负责代码块装饰行构造、正文折行前缀拼装与原始代码块按序号提取。文件预计约 190 行。

```rust
//! 面向阅读的 Markdown 代码块边框装饰、头部复制按钮布局与源码提取。
//!
//! 视觉对齐 Prompt 区 `&&&` 模板块：
//! - 顶部独立一行下 1/4 粗边 `▂`（[`text::PROMPT_BLOCK_TOP_BORDER`]），样式 [`style::md_read_rule`]；
//! - 内部第一行左侧紧贴 `▎` 显示语言名（无语言或缩进代码块显示 `text`），右侧贴右缘显示 `[❐]`；
//! - 代码正文每个视觉行（含软换行续行）首列绘制 `▎`（[`text::PROMPT_BLOCK_BAR`]）。

use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

use crate::ui::{style, text};

use super::{CodeLineKind, MdLine, display_width, highlight, is_indented_code, list_item};

/// 无语言标记或 4 空格缩进代码块的默认语言标签。
pub const DEFAULT_CODE_LANG: &str = "text";

/// 构造代码块顶部边框行（整行铺满 `▂`）。
pub(super) fn top_border_line(width: Option<usize>) -> MdLine {
    let cols = width.unwrap_or(40).max(1);
    MdLine {
        indent: 0,
        hang: 0,
        spans: vec![(
            text::PROMPT_BLOCK_TOP_BORDER.repeat(cols),
            style::md_read_rule(),
        )],
        table: None,
        code: Some(CodeLineKind::TopBorder),
    }
}

/// 构造代码块内部第一行头部：`▎<lang> ... [❐]`。
///
/// 对齐 Prompt 区 `&&&` 起始行按钮几何：右侧 `[❐]` 常显优先（极窄时钳到 `▎` 之后），
/// 左侧语言名超出 `[❐]` 左缘前 1 列空白时以 `…` 截断。
pub(super) fn header_line(lang: &str, block_index: usize, width: Option<usize>) -> MdLine {
    let cols = width.unwrap_or(40).max(1);
    let label = normalize_lang_label(lang);
    let btn = text::TEMPLATE_BUTTON_COPY;
    let btn_w = display_width(btn);
    let bar_w = usize::from(cols >= 1);
    let btn_start = cols.saturating_sub(btn_w).max(bar_w);
    let btn_end = (btn_start + btn_w).min(cols);
    let text_end = btn_start.saturating_sub(1).max(bar_w);
    let max_lang_w = text_end.saturating_sub(bar_w);

    let lang_display = truncate_with_ellipsis(label, max_lang_w);
    let lang_w = display_width(&lang_display);
    let pad = btn_start.saturating_sub(bar_w + lang_w);
    let btn_avail = btn_end.saturating_sub(btn_start);
    let btn_display = clip_width(btn, btn_avail);

    let mut spans = Vec::with_capacity(4);
    if bar_w > 0 {
        spans.push((
            text::PROMPT_BLOCK_BAR.to_string(),
            style::md_read_rule(),
        ));
    }
    if !lang_display.is_empty() {
        spans.push((lang_display, style::muted()));
    }
    if pad > 0 {
        spans.push((" ".repeat(pad), style::text()));
    }
    if !btn_display.is_empty() {
        spans.push((btn_display, style::template_button()));
    }

    let button_cols = (
        u16::try_from(btn_start).unwrap_or(u16::MAX),
        u16::try_from(btn_end).unwrap_or(u16::MAX),
    );
    MdLine {
        indent: 0,
        hang: 0,
        spans,
        table: None,
        code: Some(CodeLineKind::Header {
            block_index,
            button_cols,
        }),
    }
}

/// 渲染代码块中的单行代码正文（`code_chrome = true` 时标记为 [`CodeLineKind::Body`]，
/// 由 `wrap_md_line` 折行时在每个视觉行首列统一前置 `▎`）。
pub(super) fn render_code_line(
    line: &str,
    lang: Option<&str>,
    code_chrome: bool,
    lines: &mut Vec<MdLine>,
) {
    let spans = highlight::highlight_line(lang, line);
    lines.push(MdLine {
        indent: 0,
        hang: 0,
        spans,
        table: None,
        code: code_chrome.then_some(CodeLineKind::Body),
    });
}

/// 为折行后的单个代码块正文视觉行前置左侧色条 `▎`。
pub(super) fn prepend_body_bar(mut spans: Vec<(String, Style)>, width: usize) -> Vec<(String, Style)> {
    if width == 0 {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + 1);
    out.push((
        text::PROMPT_BLOCK_BAR.to_string(),
        style::md_read_rule(),
    ));
    if width > 1 {
        out.append(&mut spans);
    }
    out
}

/// 规范化语言名：空白或未指定时回退为 `"text"`。
pub(super) fn normalize_lang_label(lang: &str) -> &str {
    let trimmed = lang.trim();
    if trimmed.is_empty() {
        DEFAULT_CODE_LANG
    } else {
        trimmed
    }
}

fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let budget = max_width - 1;
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0).min(2);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn clip_width(text: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0).min(2);
        if used + w > max_width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// 按与 [`super::render_blocks_stateful`] 完全一致的块级扫描口径，提取第 `target_index` 个
/// 代码块（围栏或 4 空格缩进代码块）的原始未折行代码正文。
///
/// 未闭合的流式围栏代码块返回当前已生成的全部代码行。
pub fn extract_code_block(source: &str, target_index: usize) -> Option<String> {
    let source_lines: Vec<&str> = source.split('\n').collect();
    let mut index = 0;
    let mut open_fence = false;
    let mut current_block = 0usize;
    let mut fence_lines: Vec<&str> = Vec::new();

    while index < source_lines.len() {
        let line = source_lines[index];
        if open_fence {
            if line.trim_start().starts_with("```") {
                if current_block == target_index {
                    return Some(fence_lines.join("\n"));
                }
                fence_lines.clear();
                open_fence = false;
                current_block += 1;
                index += 1;
                continue;
            }
            if current_block == target_index {
                fence_lines.push(line);
            }
            index += 1;
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with("```") {
            open_fence = true;
            fence_lines.clear();
            index += 1;
            continue;
        }
        if list_item(line).is_some() {
            index += 1;
            continue;
        }
        if is_indented_code(line) {
            let mut body = Vec::new();
            while index < source_lines.len()
                && (is_indented_code(source_lines[index]) || source_lines[index].trim().is_empty())
            {
                if source_lines[index].trim().is_empty() {
                    if index + 1 < source_lines.len() && is_indented_code(source_lines[index + 1]) {
                        body.push("");
                        index += 1;
                        continue;
                    }
                    break;
                }
                body.push(
                    source_lines[index]
                        .strip_prefix("    ")
                        .unwrap_or(source_lines[index]),
                );
                index += 1;
            }
            if current_block == target_index {
                return Some(body.join("\n"));
            }
            current_block += 1;
            continue;
        }
        if trimmed.trim_start().starts_with('|') {
            while index < source_lines.len() && source_lines[index].trim_start().starts_with('|') {
                index += 1;
            }
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            while index < source_lines.len() && source_lines[index].trim_start().starts_with('>') {
                index += 1;
            }
            continue;
        }
        index += 1;
    }

    if open_fence && current_block == target_index {
        return Some(fence_lines.join("\n"));
    }
    None
}
```

---

### 5.2 修改 `src/ui/markdown_render.rs` 与 `src/ui/markdown_render/incremental.rs`

1. 在 `MdLine` 上新增 `pub code: Option<CodeLineKind>` 字段：
   ```rust
   /// 代码块单条渲染行的角色（仅在助手正文启用代码块边框装饰时附着于 [`MdLine`]）。
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum CodeLineKind {
       /// 顶部边框行（整行铺满 `▂`）。
       TopBorder,
       /// 内部第一行头部：左侧语言名、右侧 `[❐]` 复制按钮。
       Header {
           /// 当前 Markdown 文本块内的代码块序号（0-based，与 [`extract_code_block`] 一致）。
           block_index: usize,
           /// 复制按钮在代码块行内的列区间 `[start_col, end_col)`（相对 `MdLine` 起点）。
           button_cols: (u16, u16),
       },
       /// 代码块正文行（含软换行续行），首列固定为 `▎` 左边框。
       Body,
   }

   #[derive(Debug, Clone, PartialEq)]
   pub struct MdLine {
       pub indent: u16,
       pub hang: u16,
       pub spans: Vec<(String, Style)>,
       pub table: Option<TableLineRef>,
       pub code: Option<CodeLineKind>,
   }
   ```
2. 新增 `pub fn render_wrapped_code_blocks(source: &str, width: u16) -> Vec<MdLine>`：
   - `render(source)`、`render_wrapped(source, width)`、`render_wrapped_literal_rules(source, width)` 保持 `code_chrome = false`（思考块、工具展开卡片不受影响）；
   - `render_wrapped_code_blocks(source, width)` 与 `IncrementalMarkdownState::update(source, width)` 传入 `code_chrome = true`（助手正文专用）。
3. 在 `ParserState` 中新增 `next_code_block_index: usize`：
   - 围栏开启分支（`trimmed.trim_start().starts_with("```")`）：
     ```rust
     if trimmed.trim_start().starts_with("```") {
         let lang = line.trim().trim_start_matches('`').trim();
         let block_index = state.next_code_block_index;
         state.next_code_block_index += 1;
         if code_chrome {
             lines.push(code_block::top_border_line(width));
             lines.push(code_block::header_line(lang, block_index, width));
         }
         state.open_fence_lang = Some(lang.to_string());
         index += 1;
         continue;
     }
     ```
   - 围栏内部行分支：调用 `code_block::render_code_line(line, (!lang.is_empty()).then_some(lang.as_str()), code_chrome, &mut lines)`；
   - 4 空格缩进代码块分支（`indented_code`）：分配 `block_index = state.next_code_block_index`，当 `code_chrome` 为真时先推入 `top_border_line(width)` 与 `header_line("", block_index, width)`，再逐行调用 `code_block::render_code_line(line, None, code_chrome, &mut lines)`。
4. 在 `wrap_md_line(line: &MdLine, width: usize, out: &mut Vec<MdLine>)` 中处理 `line.code`：
   ```rust
   if matches!(
       line.code,
       Some(CodeLineKind::TopBorder | CodeLineKind::Header { .. })
   ) {
       out.push(line.clone());
       return;
   }
   if line.code == Some(CodeLineKind::Body) {
       let body_width = width.saturating_sub(1).max(1);
       let start_len = out.len();
       wrap_md_line_inner(line, body_width, out);
       for wrapped in &mut out[start_len..] {
           let spans = std::mem::take(&mut wrapped.spans);
           wrapped.spans = code_block::prepend_body_bar(spans, width);
           wrapped.code = Some(CodeLineKind::Body);
       }
       return;
   }
   ```
   > **文件行数控制**：`src/ui/markdown_render.rs` 当前 867 行，将表格几何元数据结构体（`TableColumnSpan` 等约 70 行）下沉至 `src/ui/markdown_render/table.rs` 并 `pub use` 重导出，配合新建 `src/ui/markdown_render/code_block.rs`，使 `src/ui/markdown_render.rs` 保持在 ~810 行，远低于 1000 行红线。

---

### 5.3 修改 `src/ui/main_content/lx/grid.rs` 与 `src/ui/main_content/lx/history/paint.rs`

1. 在 `src/ui/main_content/lx/grid.rs` 中新增 `GridCodeRow` 与行级记录接口：
   ```rust
   /// 网格中单行的代码块装饰元数据（列偏移相对 `Grid.area.x`）。
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub(crate) enum GridCodeRow {
       /// 顶部边框行（`▂`）或内部第一行头部（`<lang> ... [❐]
       /// `）：拖拽选区复制时整行跳过。
       Chrome,
       /// 代码块正文行：`bar_col` 为左侧 `▎` 色条所在的相对列偏移（选区复制时剥离 `0..=bar_col`）。
       Body { bar_col: u16 },
   }
   ```
   在 `Grid` 结构体中新增 `code_rows: Vec<Option<GridCodeRow>>`，并提供：
   ```rust
   pub(super) fn set_code_row(&mut self, y: u16, row: GridCodeRow) {
       if y < self.area.y {
           return;
       }
       let dy = y - self.area.y;
       if dy < self.area.height {
           self.code_rows[usize::from(dy)] = Some(row);
       }
   }

   pub(super) fn code_row_at(&self, dy: u16) -> Option<GridCodeRow> {
       self.code_rows.get(usize::from(dy)).copied().flatten()
   }
   ```
2. 在 `src/ui/main_content/lx/history/paint.rs` 的 `draw_markdown` 中登记 `GridCodeRow`：
   ```rust
   if let Some(code_kind) = line.code {
       let code_row = match code_kind {
           crate::ui::markdown_render::CodeLineKind::TopBorder
           | crate::ui::markdown_render::CodeLineKind::Header { .. } => {
               super::super::grid::GridCodeRow::Chrome
           }
           crate::ui::markdown_render::CodeLineKind::Body => {
               super::super::grid::GridCodeRow::Body {
                   bar_col: cx.saturating_sub(grid.area().x),
               }
           }
       };
       grid.set_code_row(y, code_row);
   }
   ```

---

### 5.4 修改 `src/ui/main_content/lx/selection.rs`（拖拽选区去装饰复制）

在 `copy_text` 的逐行提取循环（`src/ui/main_content/lx/selection.rs:503-514`）中：

```rust
let code_row = grid.code_row_at(y);
if matches!(code_row, Some(super::grid::GridCodeRow::Chrome)) {
    row = row.saturating_add(1);
    continue;
}
if let Some((raw_col0, raw_col1)) = row_col_bounds(row, start, end, geom.rect.width) {
    let min_col = match code_row {
        Some(super::grid::GridCodeRow::Body { bar_col }) => bar_col.saturating_add(1),
        _ => 0,
    };
    let eff_col0 = raw_col0.max(min_col);
    if eff_col0 <= raw_col1 {
        let (col0, col1) = table::align_wide_char_cols(&grid, y, eff_col0, raw_col1);
        let mut line = String::new();
        for col in col0.max(min_col)..=col1 {
            let ch = grid.cell(col, y).ch;
            if ch != CONTINUATION {
                line.push(ch);
            }
        }
        lines.push(line.trim_end().to_string());
    } else if matches!(code_row, Some(super::grid::GridCodeRow::Body { .. })) {
        lines.push(String::new());
    }
}
```

- 当用户框选跨越代码块顶部 `▂` 行或 `▎rust ... [❐]` 头部行时，`GridCodeRow::Chrome` 被直接跳过；
- 当用户框选代码正文行时，`min_col = bar_col + 1` 自动跳过左侧 `STEP_GUTTER`（2 列空白）与 `▎` 左边框（1 列），同时完整保留 `bar_col + 1` 之后的代码缩进空格与空代码行。

---

### 5.5 修改 `src/ui/main_content/lx/history_hit.rs` 与 `src/event/mouse/press.rs`

1. 在 `src/ui/main_content/lx/history_hit.rs` 中扩展 `LxChatHit` 并实现 `code_copy_at_source`：
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub enum LxChatHit {
       /// 点击置顶用户消息行（跳到该 prompt）。
       PromptHeader,
       /// 点击「回到底部」按钮。
       JumpBottom,
       /// 点击助手正文代码块右上角 `[❐]` 复制按钮（携带该块未折行原始代码文本）。
       CopyCode(String),
   }
   ```
   新增 `code_copy_at_source` 与 `task_agents_code_copy_at`：
   ```rust
   /// 在指定历史数据源与滚动视口下命中代码块 `[❐]` 复制按钮，返回该代码块的原始代码文本。
   pub(crate) fn code_copy_at_source(
       body: Rect,
       scroll: usize,
       source: history::HistorySource<'_>,
       column: u16,
       row: u16,
       cache: &mut PaneCache,
   ) -> Option<String> {
       if !body.contains((column, row).into()) {
           return None;
       }
       let width = history::content_rect(body).width;
       let snapshot = cache.snapshot_ready_source(width, source);
       let content_row = scroll.saturating_add(usize::from(row.saturating_sub(body.y)));
       let rel_col = column.saturating_sub(body.x);
       if rel_col >= width {
           return None;
       }
       for entry in &snapshot.entries {
           if content_row < entry.start || content_row >= entry.end {
               continue;
           }
           for block in &entry.blocks {
               if content_row < block.start
                   || content_row >= block.start.saturating_add(block.rows)
               {
                   continue;
               }
               let history::BlockSource::Step {
                   message_index,
                   step_index,
               } = block.source
               else {
                   continue;
               };
               let history::BlockPaint::Markdown { prefix, tail, .. } = block.paint.as_ref()
               else {
                   continue;
               };
               let offset = content_row - block.start;
               let line = if offset < prefix.len() {
                   &prefix[offset]
               } else {
                   tail.get(offset - prefix.len())?
               };
               let Some(crate::ui::markdown_render::CodeLineKind::Header {
                   block_index,
                   button_cols: (c0, c1),
               }) = line.code
               else {
                   return None;
               };
               let base = history::STEP_GUTTER.saturating_add(line.indent);
               let start_col = base.saturating_add(c0);
               let end_col = base.saturating_add(c1).min(width);
               if rel_col < start_col || rel_col >= end_col {
                   return None;
               }
               let message = source.messages.get(message_index)?;
               let Some(crate::app::state::AssistantStepView::Part { part, .. }) =
                   message.steps.get(step_index)
               else {
                   return None;
               };
               return crate::ui::markdown_render::extract_code_block(&part.content, block_index);
           }
       }
       None
   }
   ```
   在 `chat_hit` 中接入：
   - 若 `lx.task` 打开且位于 `TaskTab::Agents`：调用 `task_agents_code_copy_at(area, lx, column, row, cache).map(LxChatHit::CopyCode)`；
   - 若位于主对话历史（`chat_geometry_is_live(lx)`）：在 `sticky_rect` 拦截之后、`view.body.contains` 校验通过后，调用 `code_copy_at_source(view.body, view.scroll, history::HistorySource::from(lx), column, row, cache).map(LxChatHit::CopyCode)`。
2. 在 `src/event/mouse/press.rs` 中处理 `LxChatHit::CopyCode`：
   ```rust
   match hit {
       ui::main_content::lx::LxChatHit::PromptHeader => {
           let target = state.pane_anywhere(pane).and_then(|target| {
               ui::main_content::lx::sticky_prompt_jump(inner, &target.lx, cache.pane(pane))
           });
           if let Some(row) = target {
               update::jump_lx_history_to_prompt(state, pane, row);
           }
           update::clear_lx_selection(state);
       }
       ui::main_content::lx::LxChatHit::JumpBottom => {
           update::jump_lx_history_bottom(state, pane);
           update::clear_lx_selection(state);
       }
       ui::main_content::lx::LxChatHit::CopyCode(code) => {
           if !code.is_empty() {
               copy_with_toast(state, &code, Some(pane));
           }
       }
   }
   ```
   > **注意**：`copy_with_toast(state, &code, Some(pane))` 内部通过 `update::show_toast_at(state, Some(pane), toast)` 将剪贴板 Toast 路由至当前 `lx` 窗格的 `pane.lx.toast` 槽位，严格遵守 `AGENTS.md` 中「`lx` 页的 toast 必须显示在 `lx` 页内」以及「复制后选区高亮保留」的规范。

---

## 6. 影响面、测试计划与规范文档同步清单

### 6.1 现有测试影响面评估

- `src/ui/markdown_render/tests.rs` 与 `src/ui/markdown_render/incremental/tests.rs`：
  - `render` / `render_wrapped` / `render_wrapped_literal_rules` 保持 `code_chrome = false`，因此针对基础 `render_wrapped` 的既有单元测试不受破坏；
  - `IncrementalMarkdownState` 开启 `code_chrome = true`，因此 `incremental/tests.rs` 中对比 `IncrementalMarkdownState::update` 与全量渲染一致性的测试同步改为对比 `render_wrapped_code_blocks`，并补充代码块 `TopBorder` / `Header` / `Body` 的增量一致性断言；
- `src/ui/main_content/lx/history/tests*.rs` 与 `src/ui/main_content/lx/history/cache/tests.rs`：
  - 助手正文中含代码块的消息，其渲染行数会比原先增加 2 行（1 行 `▂` 顶边 + 1 行 `▎<lang> ... [❐]` 头部），代码正文每行前增加 `▎`。需检查并同步涉及助手正文代码块具体行号/列文本的测试断言。

### 6.2 新增单元测试计划

1. **`src/ui/markdown_render/tests.rs`**：
   - 围栏代码块带语言（如 `rust`）与未带语言（默认 `text`）、4 空格缩进代码块（默认 `text`）在 `render_wrapped_code_blocks` 下产出 `TopBorder`（`▂`）、`Header`（左侧语言、右侧 `[❐]`）与带 `▎` 前缀的 `Body` 行；
   - 窄宽度下长语言名截断为 `…` 且右侧 `[❐]` 始终完整保留；长代码行软换行后每个续行首列均带 `▎`；
   - `extract_code_block` 对多代码块、混合缩进/围栏代码块、流式未闭合代码块、空代码块的精确提取。
2. **`src/ui/markdown_render/incremental/tests.rs`**：
   - 逐字节/逐行流式输入包含多个围栏与缩进代码块的源码时，`prefix + tail` 与 `render_wrapped_code_blocks` 在任意帧完全相等，且 `block_index` 严格递增不漂移。
3. **`src/ui/main_content/lx/history/tests.rs` & `src/event/tests/mouse_click.rs`**：
   - 助手正文代码块渲染出 `▂` 顶边、`▎rust ... [❐]` 头与 `▎` 左边框，而 Thinking 块与工具输出卡片内的代码块不渲染顶边和 `[❐]`；
   - 点击 `[❐]` 命中 `LxChatHit::CopyCode` 并返回未折行的原始代码（空代码块不弹 Toast），触发窗格内 `pane.lx.toast`；
   - 被 `sticky` 置顶用户行或 `@`/`/` 候选面板遮挡时，点击遮挡位置不会穿透触发底下的 `[❐]`；
   - 鼠标拖拽框选代码块行执行 `copy_text` 时，自动跳过 `▂` 顶边行与 `▎<lang> ... [❐]` 头部行，并剥离正文行的 `STEP_GUTTER` 与 `▎` 左边框。

### 6.3 规范文档同步

- 更新 `docs/standards/tui-design-requirements.md` `§lx 聊天`：将原「代码块只画正文（围栏、左侧竖条、`language: xxx` 头、复制按钮、上下横线全部移除）」更新为「助手正文代码块采用顶边 `▂` + 左边 `▎`（`md_read_rule`）+ 内部首行左侧语言名（缺省 `text`）/右侧 `[❐]` 复制按钮（`muted`），点击 `[❐]` 复制原始代码并在 `lx` 窗格内弹 `Clipboard` Toast，无 Tooltip；思考块与工具/汇报卡片内的压暗 Markdown 代码块保持无边框纯代码正文」。

---

## 7. 风险与边界 / Non-goals

- **Non-goals**：
  - 不为 `[❐]` 复制按钮添加悬停 Tooltip 或悬停变色（需求明确规定不需要 Tooltip，对齐 Prompt 区按钮常驻 `muted` 无 hover 变色）；
  - 不在用户发送气泡、Thinking 思考块、工具/命令/Subagent 汇报/Compact 卡片中渲染代码块边框与复制按钮；
  - 不添加代码块右侧边框、底部边框或 Header 下方分割横线。
- **潜在风险与防护**：
  - **窄窗口溢出风险**：当 `content_width < 4` 时，`header_line` 与 `prepend_body_bar` 均按可用列数裁剪，绝不产生越界列或死循环折行；
  - **大会话性能风险**：`CodeLineKind` 为 `Copy` 值类型（零堆分配），仅在鼠标左键点击命中 `[❐]` 的瞬间对目标消息执行一次 `extract_code_block`，流式渲染帧零额外分配开销。
