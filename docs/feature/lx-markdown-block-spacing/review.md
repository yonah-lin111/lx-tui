# lx 助手正文 Markdown 块间距与超链接/生图交互：代码审查报告（Review）

> 状态：**已审查并完成全部修复（P0 ～ P3，共 14 项）**
> 日期：2026-11-10
> 分支：`feat/lx-markdown-block-spacing`（对比基线：`dev`）
> 工作区：`/Users/yonah/projects/agent/lx-tui/.worktrees/lx-markdown-block-spacing`
> 配套文档：[design.md](./design.md) · [task.md](./task.md)

---

## 审查概览

本次对工作区分支 `feat/lx-markdown-block-spacing` 新增的 **Phase 1（助手正文顶层块间距与列表间距状态机）** 与 **Phase 2（Markdown 超链接隐藏 `(url)` 及 `Ctrl + 点击` 打开网页/生图文件）** 共 24 个改动文件进行了两轮深度对抗式代码审查，共识别出 **14 项问题（P0 × 1、P1 × 5、P2 × 6、P3 × 2）**，涵盖运行时崩溃隐患、项目单文件行数红线、流式状态机与代码块序号失步、行内解析边界、渲染样式污染与浮层穿透等，并已全部完成修复与回归测试覆盖。

| 优先级 | 编号 | 关键点 | 涉及文件 | 状态 |
| :---: | :---: | :--- | :--- | :---: |
| **P0** | **P0-1** | 表格列宽收缩 `shrink_widths` 在含隐藏长 URL 的短标签超链接单元格下触发 `usize::clamp` Panic | `src/ui/markdown_render/table.rs` | 已修复 |
| **P1** | **P1-1** | 生产文件 `src/event/mouse/press.rs` 行数达 1039 行，超出项目 1000 行上限红线 | `src/event/mouse/press.rs`、`src/event/mouse/press/lx.rs` | 已修复 |
| **P1** | **P1-2** | 标题行（`#`..`######`）内的网页超链接既隐藏了 `(url)` 又丢弃了 `MdLinkSpan`，导致链接彻底丢失 | `src/ui/markdown_render.rs`、`src/ui/markdown_render/inline.rs` | 已修复 |
| **P1** | **P1-3** | 列表上下文在缩进顶层块处未清空，且列表内松散续段后接同级列表项漏插空行 | `src/ui/markdown_render.rs` | 已修复 |
| **P1** | **P1-4** | 流式增量渲染 `safe_line_count` 硬编码 `min_indent = 4`，列表后接缩进表格在流式分块时被提前固化拆裂 | `src/ui/markdown_render/incremental.rs` | 已修复 |
| **P1** | **P1-5** | `extract_code_block` 未在顶层块处清空列表栈且 `>` 引用循环贪婪吞行，导致代码块 `[❐]` 复制序号偏移 | `src/ui/markdown_render/code_block.rs` | 已修复 |
| **P2** | **P2-1** | 表格单元格以超链接开头时，单元格左右补齐空格继承 `UNDERLINED` 导致整格空白被画下划线 | `src/ui/markdown_render/table.rs` | 已修复 |
| **P2** | **P2-2** | 裸 URL 紧接中文标点或中文正文（无空格）时将后续整句中文吞入 URL | `src/ui/markdown_render/inline.rs` | 已修复 |
| **P2** | **P2-3** | 相邻同样式超链接分段被合并为单个 `(String, Style)`，导致 `tool.rs::md_line_segments` 串链 | `src/ui/markdown_render/inline.rs`、`src/ui/main_content/lx/history/tool.rs` | 已修复 |
| **P2** | **P2-4** | `handle_mouse_down_right` 的 macOS `Ctrl + 右键` 快速路径未拦截 Toast、命令面板与 Ask 面板 | `src/event/mouse/press.rs`、`src/event/mouse/press/lx.rs` | 已修复 |
| **P2** | **P2-5** | `inline::link` 未跳过 `[label]` 内反引号代码片段，且允许 `(url)` 内含空白字符从而吞掉括号内真实链接 | `src/ui/markdown_render/inline.rs` | 已修复 |
| **P2** | **P2-6** | `draw_markdown` 与 `draw_thinking` 登记超链接热区时未按 `rect.right()` 裁剪，窄视口下热区越界入滚动条列 | `src/ui/main_content/lx/history/paint.rs` | 已修复 |
| **P3** | **P3-1** | `selection::press_content_hit` 在 `Ctrl + 点击` 时重复计算 `content_geom` 并二次渲染 1 行 `Grid` | `src/ui/main_content/lx/selection.rs` | 已修复 |
| **P3** | **P3-2** | 部分文档注释与 `design.md` / `task.md` 仍残留 `Ctrl/Cmd` 描述，与统一 `Ctrl + 点击` 实现不一致 | `src/app/state/lx/chat.rs`、`src/ui/main_content/lx/selection.rs`、`docs/feature/lx-markdown-block-spacing/*` | 已修复 |

---

## 详细问题清单（按 P0 → P3 排序）

### P0：运行时崩溃（Panic）风险

#### [P0-1] 表格列宽收缩 `shrink_widths` 在含隐藏长 URL 的短标签超链接单元格下触发 `usize::clamp` Panic

- **关键点**：
  - 位置：`src/ui/markdown_render/table.rs:118-147`、`src/ui/markdown_render/table.rs:179`
  - 现象：当 Markdown 表格中某列的所有单元格可见文字均为 1 列宽 ASCII（如 `[1](https://example.com/中文路径)` 或 `[A](https://example.com/—)`），且表格总宽超出视口宽度触发按比例列宽收缩（`shrink_widths`）时，主线程在 `share.clamp(floors[index], widths[index])` 处直接 panic 崩溃。
- **错误原因**：
  - Phase 2 将 `cell_measure` 改为基于 `render_cell_spans` 计算可见宽度，使 `[1](https://example.com/中文路径)` 的自然列宽 `widths[col]` 仅为 **`1`**（隐藏了 `(url)`）。
  - 但 `table::table` 在计算每列最小单字符下限 `floors[col]` 时，仍直接遍历**未剥离隐藏 URL 的原始源码字符串** `cell.chars()`。一旦被隐藏的 URL（或图片语法）内含任意 2 列宽字符（中文、全角字符、破折号等），`floors[col]` 即被算为 **`2`**，出现 `floors[col] (2) > widths[col] (1)`。
  - 进入 `shrink_widths` 后，只要 `floor_total <= available`，`share.clamp(floors[index], widths[index])` 就会调用 `share.clamp(2, 1)`，违反 Rust 标准库 `Ord::clamp(min, max)` 的 `min <= max` 前置断言而触发 panic。
- **修复内容**：
  1. 计算 `floors` 时改为遍历 `render_cell_spans(cell, style::text())` 的可见文本字符，确保 `floors` 与 `widths` 同源；
  2. 在 `shrink_widths` 中增加防御性约束 `let floor = floors[index].min(widths[index]);`，从根本上杜绝 `clamp` 的 `min > max` 异常；
  3. 在 `src/ui/markdown_render/tests.rs` 新增回归测试覆盖该场景。

---

### P1：规范红线违背与核心功能缺陷

#### [P1-1] 生产文件 `src/event/mouse/press.rs` 超出 1000 行上限（1039 行）

- **关键点**：
  - 位置：`src/event/mouse/press.rs`（1039 行）、`src/app/state/lx/chat.rs`（1000 行临界）
  - 现象：违反 `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 中“**生产代码单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右**”的硬性规范。
- **错误原因**：
  - `press.rs` 原本已接近 983 行，Phase 2 在 `handle_mouse_down_left` 与 `handle_mouse_down_right` 中直接内联追加了 `lx` 聊天区 `Ctrl + 点击` 立即打开与 macOS `Ctrl + 右键` 兼容逻辑，导致单文件膨胀至 1039 行。
- **修复内容**：
  - 按职责将 `PaneView::Lx` 窗格内部的鼠标左键按下分派与 `Ctrl + 右键` 超链接/生图打开分派拆分为子模块 `src/event/mouse/press/lx.rs`（386 行），使 `src/event/mouse/press.rs` 降至 681 行；同时精简 `src/app/state/lx/chat.rs` 至 997 行。

#### [P1-2] 标题行（`#`..`######`）内的网页超链接既隐藏了 `(url)` 又丢弃了 `MdLinkSpan`

- **关键点**：
  - 位置：`src/ui/markdown_render.rs:518-526`（`heading_line`）、`src/ui/markdown_render/inline.rs`
  - 现象：当模型输出带网页链接的标题（如 `### [金十数据早报](https://flash.jin10.com/...)` 或 `## 参考：[Rust Book](https://doc.rust-lang.org)`）时，界面既不显示 `(https://...)`，`Ctrl + 点击` 标题文字也无法打开网页。
- **错误原因**：
  - `heading_line` 调用 `inline_spans(body)` 将分段文本拼接成单个纯字符串并构造 `MdLine::plain(0, display, heading_style)`（`links: Vec::new()`）。
  - Phase 2 让 `inline_spans` 对网页命名链接隐藏了 `(url)`，而 `heading_line` 未提取并保留 `MdLinkSpan`，导致标题中的超链接元数据被丢弃。
- **修复内容**：
  - 在 `inline.rs` 提供 `heading_spans_and_links(body, heading_style)`：无超链接的标题保持单段 `heading_style`；含超链接的标题保留 `MdLinkSpan` 区间，并对链接文字在 `heading_style` 基础上叠加 `style::md_read_link_text()`（链接蓝 `#4493f8` + 下划线），使标题超链接既可见又可 `Ctrl + 点击` 打开。

#### [P1-3] 列表上下文在缩进顶层块处未清空，且列表内松散续段后接同级列表项漏插空行

- **关键点**：
  - 位置：`src/ui/markdown_render.rs:278-334`（`should_insert_blank_line`）、`src/ui/markdown_render.rs:426-492`（`render_blocks_stateful`）
  - 现象：
    1. 列表项内出现隔了 `\n\n` 的缩进续段（如 `1. step\n\n    explanation\n2. next`）后，若 `explanation` 与 `2. next` 之间未再写第二个 `\n\n`，`explanation` 与 `2. next` 之间缺少 1 行空行（与 `design.md` Q2 第 2 条及第 3.3 节矩阵不一致）；
    2. 列表后若紧跟带 2～3 空格缩进的 `## 标题`、`---`、`| 表格 |` 或 `> 引用`，`state.list_indents` 未清空；
    3. 列表内缩进图片行 `  ![alt](url)` 未去除前导空白且缩进写死为 `0`。
- **错误原因**：
  1. `transition_block` 在产出 `    explanation` 时已将 `state.saw_blank_line` 重置为 `false`，而 `PrevBlock::Paragraph { in_list: true, depth }` 未记录该续段是否为松散续段（`loose: bool`），导致后续 `2. next` 看到 `saw_blank_line == false` 且 `depth == prev_depth` 时误判为紧凑续行；
  2. `render_blocks_stateful` 的 `heading`、`is_rule`、`table`、`blockquote` 分支漏调列表栈清理函数；
  3. `image` 分支在计算 `cont_indent` 与 `body` 之前直接用 `trimmed` 匹配。
- **修复内容**：
  1. 在 `PrevBlock::Paragraph` 中记录 `loose: bool`（进入续段时若 `saw_blank_line` 为真或前一行已是松散续段则为 `true`），当 `Paragraph { in_list: true, depth: prev_depth, loose: true }` 后接同级或外层 `ListItem` 时固定插入 1 行空行；
  2. 抽取 `clear_list_context(state)` 并在 `heading`、`is_rule`、`table`、`blockquote` 分支统一调用；
  3. 先计算列表续行 `cont_indent` 与 `body`，再执行 `image(body)` 匹配并传入 `cont_indent`。

#### [P1-4] 流式增量渲染 `safe_line_count` 硬编码 `min_indent = 4`，导致列表后接缩进表格在流式分块时被拆裂

- **关键点**：
  - 位置：`src/ui/markdown_render/incremental.rs:127-180`（`safe_line_count`）
  - 现象：当流式输出列表项后紧跟 4 空格缩进的表格（如 `- item\n    | A | B |\n  | --- | --- |\n  | 1 | 2 |\n`）时，流式增量渲染将表头行提前固化成独立单行表，与全量渲染结果不一致。
- **错误原因**：
  - `render_blocks_stateful` 根据列表上下文动态计算 `min_indent = list_code_min_indent(state)`（在 `- item` 内为 `6`），因此 `    | A | B |` 被识别为表格起始行；
  - 而 `safe_line_count` 未维护列表缩进栈，硬编码了 `is_indented_code(line, 4)` 与 `indented_block_end(lines, index, 4)`，把 `    | A | B |` 误判为已结束的 4 空格缩进代码块，并在下一行 `  | --- | --- |` 前提前提交固化。
- **修复内容**：
  - 在 `safe_line_count` 中克隆 `local_state = state.clone()`，按与 `render_blocks_stateful` 完全一致的规则推进 `update_list_context`、`list_depth`、`list_code_min_indent` 与 `clear_list_context`。

#### [P1-5] `extract_code_block` 未在顶层块处清空列表栈且 `>` 引用循环贪婪吞行，导致代码块 `[❐]` 复制序号偏移

- **关键点**：
  - 位置：`src/ui/markdown_render/code_block.rs:339-391`（`extract_code_block`）
  - 现象：当列表后出现带缩进的标题/分割线/表格/引用再接 4 空格缩进代码块，或 `> quote` 下紧跟 `    > redirect` 缩进代码块时，点击代码块头部 `[❐]` 复制按钮会复制错块或无法复制。
- **错误原因**：
  1. `extract_code_block` 未在 `heading`、`is_rule`、`|`、`>` 分支调用 `clear_list_context(&mut list_state)`，导致 `min_indent` 仍停留在列表内的 `6` 而漏计后续 4 空格缩进代码块；
  2. `extract_code_block` 在 `>` 分支使用贪婪 `while` 循环跳过所有 `trim_start().starts_with('>')` 的行，把 4 空格缩进的 `    > redirect` 代码行误当成引用行吞掉。
- **修复内容**：
  - 在 `extract_code_block` 中同步调用 `list_depth` 与 `clear_list_context`，并将 `>` 分支改为与 `render_blocks_stateful` 一致的单行推进。

---

### P2：渲染与交互边界 Bug

#### [P2-1] 表格单元格以超链接开头时，单元格左右空白填充列全部被画上下划线

- **关键点**：
  - 位置：`src/ui/markdown_render/table.rs:648-696`（`pad_line`）
  - 现象：当表格单元格以超链接 `[文字](https://...)`（或删除线 `~~文字~~`）开头且单元格列宽大于文字宽度时，文字右侧补齐的空白区域全部带有蓝色下划线（或删除线）。
- **错误原因**：
  - `pad_line` 使用 `let pad_style = line.first().map(|span| span.style).unwrap_or_default();` 作为 `before` 与 `after` 填充空格的样式。当首段为超链接时，`pad_style` 携带 `Modifier::UNDERLINED` 与链接蓝色，污染了整格空白填充列。
- **修复内容**：
  - 将 `pad_line` 中的空白填充样式固定为 `style::text()`，且仅当相邻分段的样式与 `link` 同时一致时才合并文本。

#### [P2-2] 裸 URL 紧接中文标点或中文正文（无空格）时将后续整句中文吞入 URL

- **关键点**：
  - 位置：`src/ui/markdown_render/inline.rs:295-309`
  - 现象：在中文无空格排版如 `详见 https://example.com/path，其中提到…` 或 `（见 https://example.com/path）后文` 中，裸 URL 会把 `，其中提到…` 或 `）后文` 整句吞入超链接。
- **错误原因**：
  - 裸 URL 分支使用 `rest.find(char::is_whitespace)` 寻找粗略右边界；当 URL 后紧跟中文标点且句中无空格时，`raw_end` 直达行尾，而 `trim_bare_url_end` 只能剥离最末尾的标点，无法截断句中标点后的中文正文。
- **修复内容**：
  - 裸 URL 扫描右边界时，遇到非 ASCII 字符、空白字符、控制字符或 HTML/Markdown 分隔符（`<>\"'`\`|\\^`）即截断，再由 `trim_bare_url_end` 剥离尾随 ASCII 句读标点与未配对右括号。

#### [P2-3] 相邻同样式超链接分段被合并为单个 `(String, Style)`，导致 `tool.rs::md_line_segments` 串链

- **关键点**：
  - 位置：`src/ui/markdown_render/inline.rs:76-107, 152-172`、`src/ui/main_content/lx/history/tool.rs:187-215`
  - 现象：同一行内两个紧邻的网页链接（如 `[链接A](https://a.com)[链接B](https://b.com)`）在工具/汇报卡片（`tool.rs`）中渲染时，点击 `链接B` 会打开 `https://a.com`。
- **错误原因**：
  - `build_spans_and_links` 与 `slice_spans_and_links` 在聚合 `MdLine.spans` 时只检查 `last.1 == final_style`，把两个相同样式但不同 URL 的链接合并成了一个 `(String, Style)`；而 `tool.rs::md_line_segments` 仅按每个 `span` 的起始列 `col` 查找一次 `line.links`。
- **修复内容**：
  - 在 `build_spans_and_links`、`slice_spans_and_links` 与 `table::pad_line` 中，仅当相邻分段的 `style` 与 `link` 同时相等时才合并 `spans`，确保不同链接分段在 `MdLine.spans` 中严格独立。

#### [P2-4] `handle_mouse_down_right` 的 macOS `Ctrl + 右键` 快速路径未拦截浮层面板与 Toast

- **关键点**：
  - 位置：`src/event/mouse/press.rs`、`src/event/mouse/press/lx.rs`
  - 现象：在 macOS 下（`Ctrl + 左键` 映射为 `Right + CONTROL`），当全局 Toast、窗格内 `lx` Toast、命令面板（`/`、`@`、`@ref`、粘贴候选）或 `ask` 面板浮在聊天区某条超链接上方时，在浮层内 `Ctrl + 点击` 会穿透浮层直接触发底层超链接打开。
- **错误原因**：
  - `handle_mouse_down_right` 的 `Ctrl + 右键` 分支只检查了 `state.overlay.is_none()`，未像 `handle_mouse_down_left` 一样排除 `ui::toast::rect`、`lx_toast_at`、`command_panel_contains` 与 `ask_rect` 覆盖区域。
- **修复内容**：
  - 在 `press/lx.rs::handle_lx_ctrl_right_click` 分派前统一检查全局 Toast、`lx` 窗格 Toast、命令面板与 `ask` 面板矩形，落在上述浮层内时不穿透到底层聊天历史。

#### [P2-5] `inline::link` 未跳过 `[label]` 内反引号代码片段，且允许 `(url)` 内含空白字符从而吞掉括号内真实链接

- **关键点**：
  - 位置：`src/ui/markdown_render/inline.rs:549-592`（`link`）
  - 现象：
    1. 当链接标题包含带单侧方括号的行内代码（如 `` [`]`](https://example.com) ``）时，方括号配对深度被代码内的 `]` 提前归零，导致链接解析失败；
    2. 当正文中出现 `[注意](详见 [官方文档](https://example.com))` 时，外层 `[注意](...)` 被整体识别为非网页链接并输出为纯文本，导致括号内真正的 `[官方文档](https://example.com)` 丢失超链接样式与点击热区。
- **错误原因**：
  - `link(rest)` 扫描 `label_close` 时未像 `delimited_emphasis` 一样跳过反引号代码片段；扫描 `url_close` 后也未校验 `url` 是否含空白或控制字符。
- **修复内容**：
  - 在 `link(rest)` 扫描 `label_close` 时遇到 `` ` `` 先跳过 `code_span`；解析出 `url` 后若 `url.trim()` 含空白或控制字符则返回 `None`。

#### [P2-6] `draw_markdown` 与 `draw_thinking` 登记超链接热区时未按 `rect.right()` 裁剪

- **关键点**：
  - 位置：`src/ui/main_content/lx/history/paint.rs:740-747, 871-878`
  - 现象：当视口极窄或行尾字符被 `rect.right()` 截断时，超链接热区未按 `rect.right()` 裁剪，可能延伸到右侧滚动条列。
- **错误原因**：
  - 字符绘制循环以 `cx.saturating_add(cell) > rect.right()` 裁剪，而 `set_link_span` 直接传入了未与 `rect.right()` 取最小值的列宽。
- **修复内容**：
  - 在 `draw_markdown` 与 `draw_thinking` 中计算 `let end_x = base_x.saturating_add(link.end_col).min(rect.right());`，仅在 `start_x < end_x` 时登记裁剪后的热区宽度。

---

### P3：性能与文档/注释一致性

#### [P3-1] `selection::press_content_hit` 在 `Ctrl + 点击` 时重复计算 `content_geom` 并二次渲染 1 行 `Grid`

- **关键点**：
  - 位置：`src/ui/main_content/lx/selection.rs:259-360`
  - 现象：按住 `Ctrl` 点击非网页链接区域（如点击 AI Web 预览图文件名或普通正文）时，`press_content_hit` 先后调用 `content_geom` 3 次，并对同一目标行重复分配 `Grid`、执行 2 次 `render_chunk`。
- **错误原因**：
  - `link_at_cell` 与 `is_table_cell_at` 各自独立计算 `content_geom` 并渲染单行 `Grid`，未复用 `press_content_hit` 已算好的几何信息与单行渲染结果。
- **修复内容**：
  - 在 `selection.rs` 抽取 `probe_row_cell(geom, lx, cell, cache) -> (Option<String>, bool)`，复用已计算的 `ContentGeom`，在一次 1 行 `Grid` 渲染中同时提取超链接 URL 与表格命中状态。

#### [P3-2] 部分文档注释与 `design.md` / `task.md` 仍残留 `Ctrl/Cmd` 描述

- **关键点**：
  - 位置：`src/app/state/lx/chat.rs:458`、`src/ui/main_content/lx/selection.rs:313-315`、`docs/feature/lx-markdown-block-spacing/design.md`、`docs/feature/lx-markdown-block-spacing/task.md`
  - 现象：代码实现与测试已按用户最终决策统一为 `Ctrl + 点击`（仅识别 `KeyModifiers::CONTROL`，并兼容 macOS `Ctrl + Right` 映射），但部分 Rust 文档注释与特性文档仍残留 `Ctrl/Cmd` 字样。
- **错误原因**：
  - 统一手势为 `Ctrl + 点击` 时仅更新了 `is_open_modifier` 与 `tui-design-requirements.md`，漏改了 `chat.rs`、`selection.rs` 的文档注释以及 `design.md` / `task.md` 的部分章节。
- **修复内容**：
  - 同步更新 `chat.rs`、`selection.rs` 注释以及 `design.md`、`task.md`，确保代码、注释与文档三者完全一致。
