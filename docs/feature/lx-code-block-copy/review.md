# lx 代码块样式与复制功能复审报告

- 复审对象：`feat/lx-code-block-copy`（`ac1f174` 之后的工作区）
- 复审范围：代码块渲染（`markdown_render/code_block.rs`、`markdown_render.rs`、`incremental.rs`）、lx 历史网格与选区、`[❐]` 命中与剪贴板反馈、相关测试与 feature 文档
- 结论：P0 1 项（已在 `66ef242` 修复）、P1 3 项、P2 10 项、P3 8 项；P1/P2 在本轮修复并补测试。
- 说明：下列“关键点/错误原因”记录复审时发现的问题；修复与验证列为当前状态，文档现状以本目录当前文件为准。
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
- `header_drops_zero_width_language_chars`

### P1-3 列表续段被当成缩进代码块，列表内代码块打断兄弟项层级

**关键点**

- `1. step` 后接 4 空格解释段、`- item` 后接 4 空格续段，被 `is_indented_code` 当成缩进代码块，生成带 `[❐]` 的代码块并复制该段文字。
- 围栏/缩进代码行触发 `list_indents.clear()`，列表内代码块之后的兄弟列表项丢失嵌套层级（`• c` 从 2 列缩进掉到 0）。
- `extract_code_block` 使用同一套“只看 4 空格”的判定，块序号也会把列表续段算成代码块。

**错误原因**

`is_indented_code` 没有列表内容起始列上下文；列表项在遇到非列表行时无条件清空 `list_indents`，把续段/代码块都当成列表结束。

**修复**

- `ParserState` 增加与 `list_indents` 平行的 `list_content_indents`；列表项记录内容起始列（bullet +2、ordered +number+2、task +6）。
- 非列表行只有缩进小于当前列表内容起始列时才结束列表上下文；列表续段、列表内围栏/缩进代码块保留层级。
- 缩进代码阈值改为 `list_content_indent + 4`，不再固定 4。
- 围栏/缩进代码块不再清空列表栈；`render_blocks_stateful`、`extract_code_block`、增量 `safe_line_count` 同步判定。
- `extract` 使用同一个 `ParserState` 的列表上下文，块序号与渲染一致。
- 新增列表续段、列表内代码块后兄弟项缩进、续段不占块序号测试。

**验证**

- `list_continuation_paragraph_is_not_treated_as_indented_code`
- `code_block_inside_list_keeps_sibling_list_indent`

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

### P2-5 恢复会话加载中仍可命中/选中不可见的旧历史

**关键点**

- `build_chat` 在 `lx.chat.loading_session` 为真时不画历史，只画居中的 loading 占位；但 `chat_geometry_is_live` 只判断 `task.is_none() && has_history`，没有判断 `loading_session`。
- `resume_session` 只置 `loading_session = true`，不清旧消息；`has_history` 在加载期间仍为真。
- 因此在会话 B 加载完成前，点击屏幕上原会话 A 的 `[❐]` 位置会按旧历史几何命中，复制 A 的代码并弹 Toast；`selection::content_geom` 不经过该 guard，加载态拖拽也会选中不可见旧历史。

**错误原因**

`chat_geometry_is_live` 的“几何正在被画出来”判定漏了 loading 态；`content_geom` 主历史分支没有复用同一判定。

**修复**

- `chat_geometry_is_live` 增加 `!lx.chat.loading_session`。
- `selection::content_geom` 在主历史分支前复用 `chat_geometry_is_live`，加载态直接返回 `None`。
- 新增“旧消息 + `loading_session = true` 时 `chat_hit` 返回 `None` / `content_geom` 返回 `None`”回归测试。

**验证**

- `chat_hit_does_not_copy_stale_history_while_session_is_loading`
- `content_geom_is_none_while_session_is_loading`

### P2-6 双击/三击装饰行或 `▎` 栏留下不可见的 Word/Line 选区，吞掉 Ctrl+C

**关键点**

- `unit_cell_range` 对 Chrome 行和正文 `▎` 栏返回折叠区间 `(cell, cell)`；`press.rs` 仍按 `Word`/`Line` 粒度调用 `begin_lx_selection_range`。
- `Selection::range()` 只对 `Character` 折叠返回 `None`，Word/Line 折叠会保留一个没有反显的选区。
- 之后 `Ctrl/Cmd+C` 走 `copy_text` 得到空串，`!text.is_empty()` 为假：不复制、不弹 Toast，但按键被消费，也不再走输入框清空分支。

**错误原因**

`unit_cell_range` 用折叠区间兼作“不可选位置”的返回值，丢失了“合法单字符词”与“装饰行/左栏”的语义区别；事件层无法判断是否应建立选区。

**修复**

- 新增 `unit_cell_range_opt`：Chrome 行、`▎` 栏、空 token 等不可选位置返回 `None`；合法的单字符 Word 仍返回 `Some((cell, cell))`。
- `press.rs` 使用 opt 版本；`None` 时 `clear_selection(state)`，不建立 Word/Line 折叠选区。
- 原 `unit_cell_range` 保留折叠回退，供拖拽/滚轮等已有选区路径使用。
- 新增双击顶边、语言标签、正文 `▎` 与三击顶边的回归测试。

**验证**

- `code_block_chrome_double_or_triple_click_does_not_leave_invisible_selection`

### P2-7 未闭合围栏末尾换行多渲染一条空 `▎` 行，复制多一个 `\n`

**关键点**

- `"```rust\nlet x = 1;\n"` 渲染出顶边/头部/正文/**额外空 `▎`**，`extract_code_block` 返回带尾换行。
- `"```rust\n"` 渲染出顶边/头部/**空 `▎`**；闭合后该空行消失，流式高度跳 1 行，复制结果前后不一致。

**错误原因**

`split('\n')` 把末尾换行也当成一条空行；全量与增量路径各自用 `strip_suffix` 规避，但空 tail/未闭合路径又把它加回来。

**修复**

- 新增 `split_source_lines`：末尾 `\n` 视为行终止符，真正的空行（`\n\n`）仍保留。
- `render_blocks_stateful`、`extract_code_block` 统一使用该切分；增量提交不再自行 `strip_suffix`。
- 新增未闭合尾换行、空围栏、真正空代码行的行数与复制断言。

**验证**

- `unclosed_fence_trailing_newline_is_not_an_extra_code_line`
- 增量 `prefix + tail` 等价测试继续通过。

### P2-8 宽度 2 时宽字符正文越界，宽度 1 时按字符数爆行

**关键点**

- `render_wrapped_code_blocks("```\n日\n```", 2)` 产出 `▎日`，`width() == 3`，绘制时 `日` 被裁掉，只剩 `▎`。
- 宽度 1 时 `body_width` 被 `max(1)` 拉成 1，1000 字符代码行变成 1000 行 `▎`。

**错误原因**

`prepend_body_bar` 无条件 append 折行结果；宽度 1 仍走逐字符折行后丢弃所有 span。

**修复**

- 宽度 1 的代码正文每条逻辑行只产出一条 `▎`，不进入字符折行。
- `prepend_body_bar` 按 `width - 1` 裁剪拼接后的 span，宽字符放不下就停止，保证 `MdLine::width() <= width`。
- 极窄宽度测试加入 CJK 与长 ASCII 行。

**验证**

- `ultra_narrow_wide_code_char_stays_within_width`
- `width_one_code_line_does_not_explode_into_per_char_rows`

### P2-9 代码正文零宽/ZWJ 字符导致测量宽度与落格宽度不一致

**关键点**

- 家庭 emoji 等 ZWJ 序列在 `display_width`/`wrap_md_line_inner` 中按 0 列计算，`paint.rs` 却按 1 列落格，行尾字符被裁掉。
- 语言标签已由 `normalize_lang_label` 处理同类问题，正文路径仍存在。

**错误原因**

`sanitize_code_line` 只处理 `\t` 与控制字符，没有丢弃 `width() == 0` 的字符。

**修复**

- `sanitize_code_line` 同时丢弃零宽/组合字符，保证展示宽度与实际落格一致；复制仍保留原始源码。
- 新增真实 `MdLine` 零宽正文测试。

**验证**

- `code_body_drops_zero_width_chars`

### P2-10 四个及以上反引号围栏被三个反引号的行提前关闭

**关键点**

- ` ````markdown ` 围栏内出现 ` ```rust ` 时，旧扫描器把内层三反引号当成闭合，后续代码变成普通段落，两个空块都可复制空串。
- 模型讲解 Markdown 时常用四反引号包住三反引号示例，复制功能让失败模式更明显。

**错误原因**

opener/closer 都只判断 `starts_with("```")`，没有记录 opener 的反引号长度。

**修复**

- `ParserState` 增加 `open_fence_ticks`；`fence_ticks` 统计行首反引号数量。
- 仅当关闭行反引号数量 >= opener 数量时闭合；`render_blocks_stateful`、`incremental::safe_line_count`、`extract_code_block` 同步。
- 新增四反引号围栏保留内层三反引号与复制原文测试。

**验证**

- `four_backtick_fence_is_not_closed_by_inner_three_backticks`

---

## P3（可维护性 / 文档一致性 / 边缘行为）

### P3-1 极窄宽度下 header 兜底行宽小于内容宽度

**关键点**

`header_line` 在 `cols < 7` 时走兜底：`cols=6` 只输出 `▎ [❐]`（5 列），最后一列留空；按钮仍可点击，但 header 的 `MdLine.width()` 与顶边/正文不一致。

**错误原因**

兜底分支只拼接可点击内容，没有补足剩余列；极窄宽度下优先保证不越界，未追求三行等宽。

**处理**

已在 `code_block/tests.rs` 增加 1..6 列不越界测试，并在规范中明确极窄宽度允许省略语言/裁剪按钮；实际影响仅限极窄窗格，暂不为此增加额外填充。

### P3-2 单空行代码块与空代码块无法区分

**关键点**

围栏内只有一个空行时 `fence_lines.join("\n")` 与零行都得到 `""`，点击 `[❐]` 都不复制。

**错误原因**

提取结果只返回字符串，没有携带行数；`is_empty()` 无法区分“零行”和“一个空行”。

**处理**

视觉上两者都为空，按“空代码块不触发复制”处理；若未来需要保留单个空行，应在提取结果中携带行数。

### P3-3 `ui/main_content/lx/tests.rs` 仍超过测试文件建议上限

**关键点**

该文件当前约 2260 行（dev 上已超 1500），本轮又新增了代码块渲染用例。

**错误原因**

历史测试持续堆在 lx 根部单文件；新增的独立子模块测试已放到 `code_block/tests.rs`、`history_hit/tests.rs`、`selection/tests.rs`，但主渲染测试仍缺少拆分。

**处理**

本轮不扩大重构范围；后续应把 lx 主渲染测试按场景拆分到子模块。

### P3-4 `extract_code_block` 文档声称与解析器“完全一致”

**关键点**

实现与 `render_blocks_stateful` 的块扫描顺序一致，且已支持按 opener 长度闭合反引号围栏；但没有处理 `~~~` 围栏等 CommonMark 细节。

**错误原因**

文档沿用了旧表述；当前解析器本身也不支持 `~~~`。

**处理**

源码注释与设计文档已改为“与当前解析器口径一致”，明确只支持反引号围栏。

### P3-5 `snapshot.rs` 的 `Text` fallback 分支不可达

**关键点**

外层 `AssistantStepView::Part { part, .. } if !is_user && part.kind == Text` 已捕获所有非用户正文 Text，内层 `Text` 分支仅为 match 穷尽保留。

**错误原因**

外层 guard 覆盖了非用户正文，fallback 分支在当前控制流下没有入口；文档此前把它当成实际生效路径。

**处理**

保留分支用于 match 穷尽，设计/任务文档已改为说明其不可达，不再声称由它启用 code chrome。

### P3-6 语言标签中的零宽/组合字符仍会造成热区与绘制错位

**关键点**

- `normalize_lang_label` 最初只净化控制字符；组合字符（如 `e` + U+0301）与 ZWJ 的 `char.width()` 为 0。
- `display_width` 按 0 列计算 `fill`/`button_cols`，`paint.rs` 的 `ch.width().unwrap_or(1).max(1)` 却按 1 列落格，绘制出的 `[❐]` 右移，命中区间留在左侧。

**错误原因**

语言标签净化只覆盖控制字符，没有覆盖零宽字符；宽度计算与绘制口径仍不一致。

**处理**

`normalize_lang_label` 同时丢弃 `width() == Some(0)` 的字符，并新增组合字符标签的 `button_cols`/行宽测试。

### P3-7 交互测试仍有非阻断缺口

**关键点**

- Agents 详情 `run.follow = false` + 非零 `run.scroll` 的 `[❐]` 命中未单独覆盖；主聊天滚动已加用例。
- 代码块复制事件测试只发 `Down`，未单独断言 `Up` 之后选区/Toast 状态。
- “代码块 + 表格 + 代码块”的连续拖选未覆盖。

**错误原因**

本轮优先覆盖会导致误复制、漏复制或按键吞掉的路径；以上属于同链路但风险较低的组合。

**处理**

记录为后续测试补强项，不阻塞本轮修复。

### P3-8 列表内代码块的 chrome 未按列表内容缩进

**关键点**

- 列表内代码块现在保留列表层级，兄弟列表项缩进正确；但代码块顶边/头部/正文仍从 `STEP_GUTTER` 左缘开始，没有跟随列表内容起始列。

**错误原因**

`top_border_line` / `header_line` / `render_code_line` 目前把 `MdLine.indent` 固定为 0，未把列表内容缩进并入代码块几何；直接缩进会要求顶边宽度、`button_cols` 与 `prepend_body_bar` 同步收窄。

**处理**

属于视觉对齐增强，不影响复制/选区正确性；本轮记录为 P3，后续如需要再做整体缩进几何。

---

## 复审后验证

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test markdown_render`：76 passed
- `cargo test ui::main_content::lx`：624 passed，3 ignored
- `cargo test event::tests::lx_view`：16 passed
- `cargo test --lib`：4879 passed，4 ignored（首次运行 `event::chat::tests::user_shell::short_script_runs_persists_output_without_model` 曾因登录 shell 管道环境偶发失败，单测重跑与全量重跑均通过，与本功能无关）
