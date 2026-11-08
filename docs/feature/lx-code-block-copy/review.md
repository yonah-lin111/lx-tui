# lx 代码块样式与复制功能复审报告

- 复审对象：`feat/lx-code-block-copy`（`ac1f174` 之后的工作区）
- 复审范围：代码块渲染（`markdown_render/code_block.rs`、`markdown_render.rs`、`incremental.rs`）、lx 历史网格与选区、`[❐]` 命中与剪贴板反馈、相关测试与 feature 文档
- 结论：P0 1 项（已在 `66ef242` 修复）、P1 2 项、P2 4 项、P3 5 项；P1/P2 在本轮修复并补测试。
- 关联设计：[design.md](./design.md)
- 关联任务：[task.md](./task.md)

---

## P0（阻断 / panic / 数据丢失 / 安全）

### P0-1 代码块内容含控制字符时触发 ratatui `cell_width` panic

**关键点**

- 助手代码块里出现制表符（常见于 Makefile、复制来的缩进代码）或 CRLF 的 `\r` 时，`render_code_line` 原样把 `\t`/`\r` 交给高亮器与 `Grid::put_char`。
- `Grid` 最终通过 `Buffer::cell_mut(...).set_char(ch)` 写入 ratatui；`set_char` 对控制字符调用 `cell_width` 时 panic：
  `control character passed to cell_width without filtering`。

**错误原因**

代码正文只做了语法高亮与折行，没有在进入渲染管线前净化控制字符；`put_char` 又用 `ch.width().unwrap_or(1).max(1)` 把宽度未知的控制字符当作 1 列，既触发 panic，也破坏列宽计算。

**修复**

- `code_block::sanitize_code_line`：`\t` 按 4 列制表位展开，其余控制字符替换为空格。
- `lx` 网格 blit 增加最终兜底 `drawable_char`：任何控制字符落格前统一变空格，覆盖其他 Markdown/文本路径。
- 复制仍通过 `extract_code_block` 返回原始代码，不把展示用空格写回源码。

**验证**

- `code_lines_expand_tabs_and_replace_control_chars`
- `code_block_renders_left_bar_with_lang_and_copy_button`（真实渲染含 `\t`/`\r` 的代码块，断言 buffer 无控制字符）
- 提交：`66ef242`

---

## P1（功能错误，用户可稳定复现）

### P1-1 从左侧起止的多行选区会整段漏掉代码正文

**关键点**

- 在代码块上方普通行第 0 列按下，拖到代码块下方普通行第 0 列（常见的“从左上拖到左下”整段选择）。
- `copy_text` 与 `paint_selection` 在代码正文行上执行：
  `if start.1 < min_col && end.1 < min_col { continue; }`
- 两个端点列都在 `▎` 左侧，条件成立，于是**所有**代码正文行被跳过：`Ctrl/Cmd+C` 复制结果没有代码正文，拖拽时代码行也不反显。
- 选区中间行实际覆盖到代码列，`row_col_bounds` 已给出 `0..=max_col` 的完整范围，却被这个全局端点判断提前否决。

**错误原因**

用选区两个端点的列判断整行是否落在左边栏，忽略了多行选区在中间行会横跨整行；逐行的 `eff_col0 > raw_col1` 已经足够表达“本行是否只覆盖左边栏”。

**修复**

- 删除 `copy_text` 与 `paint_selection` 中基于 `start.1`/`end.1` 的全局跳过条件，仅保留逐行的 `eff_col0 > raw_col1`。
- 新增回归：选区 `(0,0) -> (5,0)` 必须复制出代码正文，且正文行 `bar_col + 1` 起的单元格被 REVERSED。

**验证**

- `copy_text_keeps_code_body_when_selection_endpoints_are_left_of_bar`（同一用例内同时断言复制结果与 `paint_selection` 反显）

### P1-2 代码块语言标签未净化，控制字符会造成排版与点击热区错位

**关键点**

- 围栏信息串可包含制表符或其他控制字符，例如 ` ```ru\tst `。
- `normalize_lang_label` 只做 `trim`，`display_width` 把 `\t` 计为 0 列，而 `Grid::put_char`/blit 把控制字符按 1 列落格（或替换为空格）。
- `fill`、`button_cols` 按 0 列计算，实际按钮右移，绘制位置与 `[❐]` 命中区间不再一致；极端情况下最后一个空白列被挤出内容区。
- 语言标签里的控制字符也可能进入其他渲染路径，虽然 lx blit 已有兜底，但列宽仍会漂移。

**错误原因**

只净化了代码正文行，没有净化 header 的语言标签；宽度计算与实际落格对控制字符的口径不一致。

**修复**

- `normalize_lang_label` 返回 `Cow<str>`：空/空白回退 `text`，其他控制字符统一替换为空格，再参与截断与列宽计算。
- 新增 `header_sanitizes_control_chars_in_language_label`，断言 header 文本无控制字符、宽度仍等于内容宽度。

**验证**

- `header_sanitizes_control_chars_in_language_label`

---

## P2（体验缺陷 / 重要测试缺口）

### P2-1 CRLF 代码块复制结果带 `\r`

**关键点**

- 模型输出 CRLF 时，`render_code_line` 把行尾 `\r` 渲染为空格（或净化），但 `extract_code_block` 仍把 `\r` 原样拼进复制文本。
- 粘贴到编辑器后可能产生多余回车或混合换行。

**错误原因**

提取路径只按 `\n` 切行，没有剥离行尾 `\r`；展示路径与复制路径对行尾处理不一致。

**修复**

- `extract_code_block` 对围栏与缩进代码块的每一行剥离行尾 `\r`，保留其余原始内容。
- 新增 `extract_code_block_strips_crlf_line_endings`。

**验证**

- `extract_code_block_strips_crlf_line_endings`

### P2-2 feature 文档仍描述旧几何、旧函数签名与不存在的测试落点

**关键点**

- `design.md` 仍写 `▎<lang> ... [❐]` 无左侧空白、整个代码块右侧留 1 列、代码块上下各留空行；§5 的示例还是 `PROMPT_BLOCK_BAR`、旧 `header_line`/`wrap_md_line` 实现。
- `task.md` 的 T4 仍把新增测试指向 `src/ui/markdown_render/tests.rs`、`src/event/tests/mouse_click.rs`、`src/ui/main_content/lx/history/tests.rs`，实际测试在 `code_block/tests.rs`、`history_hit/tests.rs`、`selection/tests.rs`、`src/event/tests/lx_view.rs`。
- 文档缺少本轮复审与修复记录。

**错误原因**

功能经过多轮视觉修订（顶边独立、头部无边框、移除右边距、只保留下方空行、控制字符净化），设计/任务文档只追加了顶部修订提示，没有同步正文与测试清单。

**修复**

- 重写 `design.md` 的最终几何、数据流、关键代码与测试清单，旧修订仅保留历史说明。
- 重写 `task.md` 的完成项与测试落点，新增“复审修复”清单。
- 新增本 `review.md` 并在设计/任务文档中互链。

**验证**

- 文档路径、函数名、常量名、测试名与当前源码逐项核对。

### P2-3 关键路径测试缺口

**关键点**

- 原测试没有覆盖“选区两个端点都在 `▎` 左侧、中间行跨代码块”的场景（P1-1 因此漏网）。
- 没有语言标签控制字符用例（P1-2）。
- 没有 CRLF 提取用例（P2-1）。
- 原 `[❐]` 测试的代码行都很短，从未软折行；即使实现错误地复制折行后的视觉行也会通过。已补窄宽度长行原文断言。
- `extract_code_block` 没有覆盖代码块位于列表/引用/表格之后的块序号对齐。
- 事件测试只断言 `pane.lx.toast.is_some()`，没有断言锚点、标题与 `Copied N chars` 正文。
- `chat_hit_on_sticky_prompt_header_blocks_underlying_code_copy_button` 的名称声称“挡住底层按钮”，但 sticky 布局本身已把 body 下移一行，断言只验证了 PromptHeader 命中；属于命名与覆盖强度问题，不是功能缺陷。

**错误原因**

测试主要覆盖单行、右端结束的选区与正常语言标签，缺少边界组合；置顶测试复用了通用布局，没有构造“按钮被覆盖”的精确场景。

**修复**

- 补充 P1-1、P1-2、P2-1 的回归测试。
- 补充窄宽度软折行长代码的原文复制断言。
- 补充事件层 Toast 锚点/标题/正文断言。
- 补充极窄宽度（1..6 列）不越界测试。
- 补充 `extract_code_block` 在列表/引用/表格之后的块序号对齐测试。
- 置顶条用例保留为行为回归，名称问题记入 P3。

**验证**

- 见 `design.md` §7 测试清单；相关模块测试全绿。

### P2-4 Agents 详情页 `[❐]` 命中走非就绪视图，鼠标回调可能同步重排快照

**关键点**

- `task_agents_code_copy_at` 使用 `history::view_source_plain`，而同文件 `task_agents_card_target_at` 与滚动条命中使用 `view_source_plain_ready`。
- 高频鼠标按下的约定是只读已就绪快照；使用非就绪视图在缓存未命中时会同步构建整份历史快照，可能卡住事件循环。

**错误原因**

新增复制命中路径复制了绘制路径的 `view_source_plain`，没有沿用同文件命中路径的 ready 版本。

**修复**

- 改为 `history::view_source_plain_ready(rect, source, cache)`，与卡片/滚动条命中保持一致。

**验证**

- `chat_hit_supports_code_copy_button_in_task_agents_detail_view` 继续通过。

---

## P3（可维护性 / 文档一致性 / 边缘行为）

### P3-1 极窄宽度下 header 兜底行宽小于内容宽度

`header_line` 在 `cols < 7` 时走兜底：`cols=6` 只输出 `▎ [❐]`（5 列），最后一列留空；按钮仍可点击，但 header 的 `MdLine.width()` 与顶边/正文不一致。实际影响仅限 6 列极窄窗格，暂不阻塞。

### P3-2 单空行代码块与空代码块无法区分

围栏内只有一个空行时 `fence_lines.join("\n")` 与零行都得到 `""`，点击 `[❐]` 都不复制。视觉上两者都为空，按“空代码块不触发复制”处理可接受；若未来需要保留单个空行，应在提取结果中携带行数而不是只看 `is_empty()`。

### P3-3 `ui/main_content/lx/tests.rs` 仍超过测试文件建议上限

该文件当前约 2260 行（dev 上已超 1500），本轮又新增了代码块渲染用例。新增测试已尽量放到 `code_block/tests.rs`、`history_hit/tests.rs`、`selection/tests.rs`；后续应把 lx 主渲染测试按场景拆分，而不是继续堆在根部。

### P3-4 `extract_code_block` 文档声称与解析器“完全一致”

实现与 `render_blocks_stateful` 的块扫描顺序一致，但没有处理 `~~~` 围栏、四反引号闭合长度等 CommonMark 细节；当前解析器本身也不支持这些形式，文档应写成“与当前解析器的块扫描口径一致”，避免误导。

### P3-5 `snapshot.rs` 的 `Text` fallback 分支不可达

外层 `AssistantStepView::Part { part, .. } if !is_user && part.kind == Text` 已捕获所有非用户正文 Text，内层 `Text` 分支仅为 match 穷尽保留。本轮保留该分支并修正文档描述，不再把它当作实际生效路径。

---

## 复审后验证

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test markdown_render`：66 passed
- `cargo test ui::main_content::lx`：620 passed，3 ignored
- `cargo test event::tests::lx_view`：15 passed
- `cargo test --lib`：4864 passed，4 ignored（首次运行 `event::chat::tests::user_shell::short_script_runs_persists_output_without_model` 曾因登录 shell 管道环境偶发失败，单测重跑与全量重跑均通过，与本功能无关）
