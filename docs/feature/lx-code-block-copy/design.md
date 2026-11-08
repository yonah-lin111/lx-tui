# lx 消息列表代码块样式与复制功能：设计方案

> 状态：**已完成（含复审修复）**
> 日期：2026-11-08
> 项目：`/Users/yonah/projects/agent/lx-tui`
> 作用域：`src/ui/markdown_render*`、`src/ui/main_content/lx/*`、`src/event/mouse/*`、`src/ui/text/workspace.rs`、`docs/standards/tui-design-requirements.md`
> 配套文档：[task.md](./task.md)、[review.md](./review.md)

---

## 1. 背景与目标

### 1.1 背景

1. **代码块缺少边界与一键复制**：`markdown_render` 原先只输出高亮后的代码正文，不绘制顶边、语言标签或复制按钮；`LxChatHit` 也没有代码块复制变体。
2. **窄屏软换行后选区复制不干净**：长代码行折行后，鼠标框选会把折行断行、左侧步骤缩进一起带进剪贴板，无法直接得到未折行的原始代码。

### 1.2 目标

在 `lx` 消息列表的 **AI 助手正文块**（主对话与 Subagent 详情页的 `PartKind::Text`）中：

- 为围栏代码块与 4 空格缩进代码块增加顶部 `▂` 边框、无边框语言/复制头部、正文左侧 `▎` 单栏；
- 代码块下方保留 1 行空白，上方不额外补空行；
- 点击 `[❐]` 复制该代码块的原始未折行文本（不含围栏、顶边、头部、左栏）；
- 鼠标拖拽复制时跳过装饰行与左栏，保留代码自身缩进；
- 思考块、工具/命令/Subagent 汇报/Compact 的压暗 Markdown 继续使用无 chrome 的纯代码正文。

---

## 2. 已确认边界与最终行为

| 编号 | 决策项 | 最终行为 |
| :--- | :--- | :--- |
| **Q1** | 生效范围 | 仅助手正文 `PartKind::Text`（主对话与 `TaskTab::Agents` 子 Agent 详情）；用户气泡、思考块、工具卡片、Compact、ModelSwitch 不启用。 |
| **Q2** | 视觉布局 | 顶部独立一行 `▂`（宽度 = 内容宽度，不预留右边距）；下一行无边框头部 `▎ <lang> ... [❐] `（语言左侧 1 列空白、按钮右侧 1 列空白）；正文每个视觉行（含软换行续行）首列 `▎`；无右边、底边、Header 下方分割线；`▂`/`▎` 用 `md_read_rule`，语言用 `muted`、按钮用 `template_button`（当前等价 `muted`）。 |
| **Q3** | 复制与选区 | 点击 `[❐]` 复制原始未折行代码；空代码块不写剪贴板、不弹 Toast；Toast 标题 `Clipboard`、正文 `Copied N char(s)`，落当前 `lx` 窗格；点击后保留已有选区；无 Tooltip。拖拽复制跳过 `▂` 顶边与头部行，剥离 `STEP_GUTTER` 与 `▎`；从左侧起止的多行选区也必须保留中间代码正文。 |
| **Q4** | 语言/窄屏/空块 | 围栏未写语言或缩进代码块显示 `text`；语言超宽以 `…` 截断，`[❐]` 常显优先；极窄宽度下裁剪按钮片段；语言中的控制字符替换为空格、零宽/组合字符丢弃；代码行制表符按 4 列展开、控制字符替换为空格；复制时剥离 CRLF 行尾 `\r`。 |

---

## 3. 实现落点

| 文件 | 职责 |
| :--- | :--- |
| `src/ui/markdown_render/code_block.rs` | `CodeLineKind`（`TopBorder` / `Header { block_index, button_cols }` / `Body`）；`top_border_line`、`header_line`、`render_code_line`、`sanitize_code_line`、`prepend_body_bar`、`normalize_lang_label`、`extract_code_block` 等。 |
| `src/ui/markdown_render.rs` | `MdLine.code` 元数据；`render_wrapped_code_blocks`；`ParserState` 的 `list_indents` / `list_content_indents` / `next_code_block_index` / `after_code_block` / `open_fence_ticks`；`wrap_md_line` 的代码分支；`split_source_lines` / `fence_ticks`。 |
| `src/ui/markdown_render/incremental.rs` | 流式增量渲染继续使用 `code_chrome = true`，稳定前缀/活跃尾部与全量渲染逐行一致。 |
| `src/ui/main_content/lx/grid.rs` | `GridCodeRow::{Chrome, Body { bar_col }}` 与逐行登记接口。 |
| `src/ui/main_content/lx/history/paint.rs` | `draw_markdown` 按 `line.code` 登记代码行元数据。 |
| `src/ui/main_content/lx/history/snapshot.rs` | 助手正文增量 Markdown 缓存（`markdown_block`）启用 code chrome；match 尾部保留 `Text` 穷尽分支（非用户正文实际已被外层 guard 捕获）。 |
| `src/ui/main_content/lx/history_hit.rs` | `LxChatHit::CopyCode`、`code_copy_at_source`、`task_agents_code_copy_at`；按 Header 的 `button_cols` 命中。 |
| `src/ui/main_content/lx/selection.rs` | 选区反显/复制：跳过 Chrome 行，正文行从 `bar_col + 1` 起算。 |
| `src/event/mouse/press.rs` / `hit.rs` | 左键命中分派；`CopyCode` 非空时写剪贴板并弹窗格内 Toast。 |
| `src/ui/text/workspace.rs` | `CODE_BLOCK_BAR = "▎"`、`CODE_BLOCK_TOP_BORDER = "▂"`、`TEMPLATE_BUTTON_COPY = "[❐]"`。 |

---

## 4. 几何模型

`STEP_GUTTER = 2`，代码块从 `body.x + STEP_GUTTER` 开始，内容宽度：

```text
content_width = body.width - STEP_GUTTER
```

```text
列（相对 body.x）:
0 1 2 3 4 ...                                      content_width-1
●   ▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂
    ▎ <lang>                              [❐]
    ▎fn main() {
    ▎    println!("hi");
    ▎}
```

### 4.1 顶部边框

`top_border_line(width)` 产出恰好 `content_width` 列的 `▂`，单独占一行，无右边距。

### 4.2 Header 行

`header_line(lang, block_index, width)`：

- 固定开销：左栏 1 + 语言左空白 1 + 语言/按钮间至少 1 + 按钮 3 + 按钮右空白 1 = 7 列；
- `lang_budget = content_width - 7`，语言超出时 `truncate_with_ellipsis` 以 `…` 截断；
- `fill = content_width - (1 + 1 + lang_w + 3 + 1)`，按钮起点 `1 + 1 + lang_w + fill`；
- 按钮列区间 `[btn_start, btn_start + 3)`（相对 `MdLine` 起点，供命中使用）；最后一列固定为空；
- `content_width < 7` 时走极窄兜底：`▎ ` + 裁剪后的按钮片段，避免越界。

`normalize_lang_label` 负责空值回退 `text`，把语言中的控制字符（含 `\t`）替换为空格，并丢弃零宽/组合字符，保证 `display_width` 与网格落格口径一致。

### 4.3 正文行

`wrap_md_line` 对 `CodeLineKind::Body`：

- 折行宽度 `content_width - 1`（扣除 `▎`）；
- 每个视觉行（含空代码行、软换行续行）前置 `▎`；
- 宽字符按显示宽度折行；单个超宽字符单独成行；`prepend_body_bar` 再按 `content_width - 1` 裁剪，保证 `MdLine::width() <= content_width`；
- 宽度 1 时每条逻辑代码行只产出一条 `▎`，不按字符爆行；
- 零宽/组合字符直接丢弃，保证测量宽度与绘制落格一致。

`sanitize_code_line` 在进入高亮/折行前：

- `\t` 按 4 列制表位展开；
- 其余控制字符（`\r`、`\x1b` 等）替换为空格。

### 4.4 间距

- 代码块上方不额外补空行（源码里的空行会被解析器跳过）；
- 代码块之后若还有内容，固定补 1 行空行；消息末尾不补；
- 相邻代码块共享这 1 行，不叠加；
- 跨增量分块由 `ParserState.after_code_block` 保留，稳定前缀与尾部不重复/不丢失；
- `split_source_lines` 把末尾 `\n` 视为行终止符，不额外产生空代码行；真正的空行（`\n\n`）仍保留。

---

## 5. 渲染与交互数据流

### 5.1 渲染

```text
PartKind::Text (assistant)
  → PaneCache::markdown_block
  → IncrementalMarkdownState（code_chrome = true）
  → render_blocks_stateful
       ├─ 围栏/缩进代码块 → TopBorder + Header + Body
       └─ 普通块 → 原有 Markdown 行
  → wrap_md_line
  → draw_markdown
       ├─ TopBorder/Header → GridCodeRow::Chrome
       └─ Body → GridCodeRow::Body { bar_col }
```

### 5.2 `[❐]` 命中与复制

```text
鼠标左键
  → chat_hit
       ├─ JumpBottom
       ├─ TaskTab::Agents → task_agents_code_copy_at
       ├─ sticky header（优先拦截）
       └─ code_copy_at_source
            ├─ 内容行 → block → Header { block_index, button_cols }
            ├─ 校验 rel_col ∈ [STEP_GUTTER + button_cols)
            └─ extract_code_block(part.content, block_index)
  → LxChatHit::CopyCode(code)
       └─ code 非空 → copy_with_toast(state, code, Some(pane))
```

- `code_copy_at_source` 在命中前校验 `view.body` 包含点击位置，避免穿透到不可见行；
- `copy_with_toast` 走 `show_toast_at` → `pane.lx.toast`；
- 空代码块返回 `Some("")`，事件层不写剪贴板、不弹 Toast，且不清除选区。

### 5.3 拖拽选区

`copy_text` / `paint_selection` 对每个内容行：

- `GridCodeRow::Chrome`：整行跳过（顶边与头部不复制、不反显）；
- `GridCodeRow::Body { bar_col }`：有效起始列钳到 `bar_col + 1`，剥离 `STEP_GUTTER` 与 `▎`，保留代码自身缩进与空代码行；
- 是否跳过只按**当前行**的实际列范围判断，不能用选区两个端点的列做全局跳过；否则“从左上拖到左下”的多行选区会漏掉中间代码正文；
- `content_geom` 在 `loading_session` 为真时返回 `None`，`chat_geometry_is_live` 同步返回 false，加载态旧历史不参与命中或选区；
- 双击/三击通过 `unit_cell_range_opt` 判断可选择性：Chrome 行、`▎` 栏等不可选位置返回 `None`，事件层直接清空选区，不留下折叠 Word/Line 选区。

---

## 6. 原始代码提取

`extract_code_block(source, target_index)` 按与 `render_blocks_stateful` 相同的块扫描顺序遍历：

1. 未闭合围栏内部：收集代码行，关闭行反引号数量 >= opener 数量才闭合；
2. 围栏起始行：分配块序号并记录 opener 反引号数量；
3. 列表项优先于缩进代码块；列表续段按“当前列表内容起始列 + 4”判断，不占用代码块序号；
4. 缩进代码块：阈值 = 列表内容起始列 + 4（无列表时为 4），收集连续缩进行与段内空行；
5. 表格/引用整体跳过，不占用代码块序号；
6. 未闭合围栏返回当前已生成代码；
7. `split_source_lines` 把末尾 `\n` 当行终止符，真正的空行仍保留；
8. 围栏与缩进代码行均剥离行尾 `\r`（CRLF 源），其余内容保持原始未折行。

空代码块返回空串，事件层据此 no-op。

---

## 7. 测试

实际测试落点：

- `src/ui/markdown_render/code_block/tests.rs`
  - 顶边/头部/正文几何、语言回退与截断、窄宽兜底、软换行与空代码行、上下间距、制表符/控制字符、语言标签净化、CRLF 提取、围栏/缩进/未闭合提取、列表/引用/表格后的块序号对齐、列表续段、列表内代码块兄弟项缩进、未闭合尾换行、宽字符/宽度 1/零宽正文、四反引号围栏；
  - 思考块/工具使用 `render_wrapped` 时无 chrome。
- `src/ui/markdown_render/incremental/tests.rs`
  - 逐字节增量与全量 `render_wrapped_code_blocks` 等价，含代码块与多块序列。
- `src/ui/main_content/lx/tests.rs`
  - 真实渲染助手代码块：顶边、头部、正文左栏、制表符展开、无控制字符。
- `src/ui/main_content/lx/history_hit/tests.rs`
  - 主对话/子 Agent 详情页 `[❐]` 命中与原始代码返回；语言/右侧空白/按钮边缘不命中；多代码块按各自 `block_index` 提取；恢复会话加载中不命中旧历史。
- `src/ui/main_content/lx/selection/tests.rs`
  - Chrome 行不可复制/反显；正文左栏剥离；`(0,0) → (N,0)` 左侧起止多行选区保留代码；恢复会话加载中 `content_geom` 返回 `None`。
- `src/event/tests/lx_view.rs`
  - 点击 `[❐]` 弹窗格内 Toast、空块不弹、选区保留、全局 `state.toast` 不被占用；双击/三击装饰行或 `▎` 栏不留下不可见 Word/Line 选区。

验证命令：

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test markdown_render
cargo test ui::main_content::lx
cargo test event::tests::lx_view
cargo test --lib
```

---

## 8. 复审修复

本轮复审详见 [review.md](./review.md)：

- **P0-1**：控制字符导致 ratatui `cell_width` panic —— 已由 `66ef242` 修复（制表符展开 + blit 兜底）。
- **P1-1**：左侧起止的多行选区漏掉代码正文 —— 删除基于选区端点的全局跳过，补复制/反显测试。
- **P1-2**：语言标签控制字符导致排版/热区错位 —— 语言标签净化。
- **P1-3**：列表续段误判为缩进代码、列表内代码块打断兄弟项层级 —— 列表内容缩进上下文与代码阈值同步。
- **P2-1**：CRLF 复制带 `\r` —— 提取时剥离行尾 `\r`。
- **P2-2**：设计/任务文档与实现不一致 —— 本文与 `task.md` 同步重写。
- **P2-3**：补充选区、语言、CRLF、块序号测试。
- **P2-4**：Agents 详情页命中改用 `view_source_plain_ready`。
- **P2-5**：恢复会话加载中禁止命中/选中旧历史。
- **P2-6**：双击/三击装饰行或 `▎` 栏不再留下不可见 Word/Line 选区。
- **P2-7**：未闭合围栏末尾换行不再多空行/多复制 `\n`。
- **P2-8**：宽度 1/2 的宽字符正文不再越界或爆行。
- **P2-9**：代码正文零宽/ZWJ 字符净化。
- **P2-10**：四反引号围栏按 opener 长度闭合。
- **P3-6**：语言零宽/组合字符净化。
- **P3-8**：列表内代码块 chrome 视觉缩进未做，记录为后续增强。

---

## 9. 风险与 Non-goals

- **Non-goals**：
  - 不为 `[❐]` 添加 Tooltip 或 hover 变色；
  - 不在用户气泡、思考块、工具/命令/Subagent 汇报/Compact 中启用 chrome；
  - 不添加代码块右侧边框、底部边框或 Header 下方分割线；
  - 不引入 `~~~` 围栏语义；反引号围栏按 opener 长度支持四反引号及以上闭合。
- **边界**：
  - 极窄 `content_width < 7` 时语言名可能完全让位给按钮，按钮片段可能被裁剪；
  - 单空行代码块与空代码块在提取结果上都为空串，按“空块不复制”处理；
  - 宽度 1/2 时宽字符正文可能被裁剪为仅剩 `▎`，但保证不越界、不爆行。
