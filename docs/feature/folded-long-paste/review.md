# 文本折叠粘贴功能代码审查 (Code Review)

本次 Review 覆盖工作区 `.worktrees/folded-long-paste`（分支 `feat/folded-long-paste`，基线 `dev...HEAD`）引入的全部改动，包含折叠 token 编解码与投影核心（`src/app/paste/{folded,text}.rs`）、Prompt 编辑器集成（`src/app/prompt/*`、`src/ui/prompt*`）、Lx 输入框与聊天历史用户消息芯片集成（`src/app/state/lx/*`、`src/ui/main_content/lx/*`）、事件路由（`src/event/*`）及上下文/标题链路（`src/app/update/*`、`src/chat/*`）。

审查发现的问题按照严重级别从高到低（P0 - P2）归类如下，并明确记录关键点、错误原因与修复方案。

---

## 缺陷清单与优先级排序

### [P0-1] Prompt 选区替换粘贴确认后未清除 `AppState.selection`，导致后续输入直接误删刚插入的粘贴芯片

- **关键点**：
  - `src/app/update/editor.rs` 的 `route_paste_panel`（键盘确认文本粘贴或路径粘贴）与 `select_paste`（鼠标点击确认文本粘贴或路径粘贴）在调用 `state.prompt.text_paste_confirm(action)` / `state.prompt.paste_confirm(action)` 成功后，仅更新了 `Prompt` 内部文本与光标，未清除外层 `AppState.selection`。
- **错误原因**：
  - `Prompt` 的普通文本选区存储在 `AppState.selection` 中，`Prompt` 自身方法无法直接访问 `AppState`。
  - 当用户先通过 `Ctrl/Cmd+A` 全选或鼠标拖选一段文本，再执行粘贴并从面板确认（如选择 `Paste text`）时，旧选区虽已被替换为折叠芯片，但 `AppState.selection` 仍保留在状态机中。
  - 用户紧接着输入任意字符、按 `Backspace` 或 `Delete` 时，`apply_editor`（`src/app/update/editor.rs:67-85`）检测到 `state.selection` 非空，会再次按旧选区范围执行替换删除，直接把刚刚粘贴进去的折叠芯片整块吞掉。
- **修复方案**：
  - 在 `src/app/update/editor.rs` 的 `route_paste_panel` 与 `select_paste` 确认粘贴成功后，统一调用 `clear_selection(state)` 清除残留选区；
  - 在 `src/event/paste.rs` 的 `handle_terminal_paste` 执行 `paste_stage` 后同步清理选区，并在 `paste_stage` 返回 `false`（如受保护行）时回退普通粘贴，避免丢内容；
  - 在 `Prompt::replace_at` 中同步清空 `preview_selection` 与 `preview_drag`。

---

### [P0-2] `Projection::display_to_raw_directed` 在 `after = true` 时将芯片左边界误判为芯片内部，导致选中芯片左侧文字后删除/替换会连坐吞掉整块折叠芯片

- **关键点**：
  - `src/app/prompt/viewport.rs` 的 `selection_bounds_with` 调用 `content_offset(last, true)`（内部转发给 `Projection::display_to_raw_directed(display, true)`）计算选区结束位置的原始字节偏移；
  - `src/app/paste/folded.rs` 的 `display_to_raw_directed` 与 `display_range_to_raw` 负责显示坐标到原始缓冲区坐标的转换。
- **错误原因**：
  - 在 `src/app/paste/folded.rs:288-308` 中，`display_to_raw_directed(display, after)` 仅通过 `if display < seg.display.end` 查找所属段。当用户仅选中折叠芯片左侧紧邻的普通字符（例如 `"ab[Pasted text 3 lines]cd"` 中仅选中 `"ab"`）时，选区右端点的半开显示偏移 `display` 恰好等于前一个 `Plain` 段的 `display.end`、同时也等于紧随其后的 `Chip` 段的 `display.start`。
  - 循环跳过 `Plain` 段后命中 `Chip` 段，并因 `after == true` 直接返回 `chip_seg.raw.end`！结果是：`selection_text` 复制时只复制 `"ab"`，但 `selection_bounds` 删除或打字替换时却返回 `0..chip.raw.end`，将紧邻但并未被选中的整块折叠芯片一并删除。
  - 同时，`Projection::display_range_to_raw`（`src/app/paste/folded.rs:311-327`）在遍历相交段时未区分 `Plain` 与 `Chip`，无条件将普通 `Plain` 段也扩展到了整段边界 `seg.raw.start..seg.raw.end`。
- **修复方案**：
  - 在 `display_to_raw_directed(display, after)` 中，当 `after == true` 且 `display == seg.display.start` 时，该点位于前一段的末尾边界而非当前芯片内部，直接返回 `seg.raw.start`；
  - 在 `display_range_to_raw` 中，仅当相交段为 `SegmentKind::Chip` 时才吸附扩展到 `seg.raw.start..seg.raw.end`，`SegmentKind::Plain` 保持精确字节偏移映射。

---

### [P1-1] Lx 会话自动标题生成与初始标题守卫未对折叠 token 做显示投影，导致生成的标题无法落库或泄漏原始标签

- **关键点**：
  - 创建新会话并写入首条用户消息时，`src/db/chat/mod.rs:408-423`（`begin_turn_with_attachments`）调用 `strip_references_from_inputs`（会将 `<pasted-...>` 投影为 `[Pasted text N lines]`）生成数据库中的初始会话标题。
  - 而 `src/app/update/chat/title.rs:42-54`（`maybe_spawn_title_generation`）与 `src/db/chat/mod.rs:949-960`（`initial_session_title_for_meta`）却调用了**未投影折叠 token** 的 `strip_references_for_display`。
- **错误原因**：
  - `initial_session_title_for_meta` 计算出的 `initial_title` 保留了 `<pasted-text lines="N">...` 原始标签与正文首行，与数据库创建会话时写入的 `[Pasted text N lines]` 标题不一致。
  - 当异步标题生成完成并执行 `rename_chat_session_if_title_matches`（SQL 要求当前数据库 `title == initial_title` 才允许更新）时，字符串比对必然失败，导致 AI 生成的会话标题被静默丢弃。
  - 若异步标题生成失败走回退分支，还会把含 `<pasted-text lines="N">...` 的 `initial_title` 刷入内存会话列表；且 `strip_references_for_display` 在未遮蔽折叠 token 的情况下直接剥离 `@[refer-*]`，若粘贴正文含该字面量还会截断破坏折叠 token。
- **修复方案**：
  - 在 `src/chat/prepare.rs` 的 `strip_references_for_display` 中使用 `mask_pasted_text` 保护折叠 token 正文不被误删引用；
  - 在 `src/db/chat/mod.rs` 的 `initial_session_title_for_meta` 与 `src/app/update/chat/title.rs` 的 `maybe_spawn_title_generation` 中，统一对折叠 token 做折叠态投影（转换为 `[Pasted text/code/log N lines]`），使初始标题守卫与数据库落库口径完全一致，同时避免百行粘贴日志淹没标题生成提示词。

---

### [P1-2] Lx 聊天历史用户消息中的折叠芯片点击热区缺失列范围（整行命中），导致点击同行普通文字误触芯片、同行多芯片无法点击第二个

- **关键点**：
  - `src/ui/main_content/lx/history/rows.rs` 的 `UserPasteSpan` 与 `src/ui/main_content/lx/history/snapshot.rs` 的 `CardRegion` 只记录了视觉行偏移 `offset`/`card_start` 与行数 `rows`/`card_rows`，没有记录行内起止列坐标；
  - `src/ui/main_content/lx/history_hit.rs:184-213` 将 `CardRegion` 的命中矩形构造为整行宽度 `Rect::new(body.x, ..., width, ...)`。
- **错误原因**：
  - 当用户消息形如 `请看日志 [Pasted log 6 lines] 然后分析` 时，点击同行芯片左侧的“请看日志”、右侧的“然后分析”甚至行首 `> ` 提示符，都会命中 `CardKind::UserPaste` 并切换芯片展开态。
  - 当同一行内包含两个折叠芯片（如 `[Pasted text 1 line] [Pasted code 2 lines]`）时，两个 `CardRegion` 的行区间完全重合，`snapshot.cards.iter().find(...)` 永远只返回第一个芯片，导致同行第二个芯片无法通过鼠标点击展开。
  - 此外，`src/app/update/chat/viewport.rs` 的 `toggle_lx_card` 在切换 `CardKind::UserPaste` 时未调用 `pin_lx_viewport`，展开长粘贴块时会导致聊天视口跳动。
- **修复方案**：
  - 在 `UserPasteSpan` 与 `CardRegion` 中增加可选的列区间 `cols: Option<(u16, u16)>`（单行芯片精确记录相对于内容区起点的显示列范围 `(col_start, col_end)`，包含 `USER_INDENT` 偏移；多行展开态覆盖整行内容宽）；
  - 在 `history_hit.rs` 的 `card_at_source` 中校验鼠标列坐标是否落在 `cols` 范围内，并按精确列区间构造 `CardHit.rect`，支持同行多芯片独立点击与同行普通文字正常选区；
  - 在 `toggle_lx_card` 的 `CardKind::UserPaste` 分支中补充 `pin_lx_viewport`。

---

### [P1-3] `Prompt` 与 `Lx` 在展开预览选区激活后执行全选（`Ctrl/Cmd+A`）或键盘扩选时未清理预览选区，且 `Prompt` 窄屏软换行下复制折叠芯片会重复拼接 token

- **关键点**：
  - `src/app/state.rs:614-624`（`AppState::select_all`）、`src/app/state/lx/selection.rs:137-148`（`Lx::select_all`）及 `Lx::extend_selection_to`（`Shift+方向键`）仅设置了普通文本选区，未清除 `preview_selection`；
  - `src/app/prompt/viewport.rs:327-372`（`Prompt::selection_text_with`）在复制普通选区时按每个视觉行循环调用 `proj.copy_display_range(from..to)`。
- **错误原因**：
  - 渲染与复制（`Ctrl/Cmd+C`）均优先检查 `preview_selection`，而删除、打字替换与剪切（`Ctrl/Cmd+X`）则读取普通选区。若用户在展开预览区内拖选几个字后按 `Ctrl/Cmd+A`，画面仍只高亮那几个预览字符、`Ctrl/Cmd+C` 也只复制那几个字符，但按 `Backspace` 或打字却会把整个编辑器的全部内容删空。
  - 在 `Prompt::selection_text_with` 中，当编辑器较窄导致单个折叠芯片 `[Pasted text 3 lines]` 软换行跨越多行时，每一行都会调用一次 `copy_display_range`，而 `copy_display_range` 对与折叠芯片相交的任意子区间都会输出完整 `<pasted-text ...>` token，导致复制一次却把同一个完整 token 重复拼接了多次。
- **修复方案**：
  - 在 `AppState::select_all`、`Lx::select_all`、`Lx::begin_keyboard_selection` / `extend_selection_to`、`Prompt::clear_panel` 中统一调用 `clear_preview_selection()`；
  - 重构 `Prompt::selection_text_with`：直接取首行起点 `first_display_start` 到末行终点 `last_display_end` 的连续显示字节区间，调用一次 `proj.copy_display_range(first_display_start..last_display_end)`，既避免软换行凭空插入换行符，又彻底消除折叠芯片跨行重复拼接。

---

### [P1-4] `Prompt` 的块/模板操作与 `newline_below`、以及 `Lx` 的斜杠/历史引用/技能扫描未统一遮蔽折叠 token，导致正文被误判或截断

- **关键点**：
  - `src/app/prompt.rs:405-421` 的 `newline_below`（`Ctrl/Cmd+Enter`）与 `src/app/prompt/template.rs:154` 的 `insert_template_title_line` 直接使用原始 `line_end(&self.text, ...)`；
  - `src/app/prompt/block.rs`、`src/app/prompt/template.rs`、`src/app/prompt/panel.rs:324` 在执行块扫描、删除补充块、清理模板块时直接读取原始 `self.text`；
  - `src/app/update/pane.rs:657`（`refresh_slash_panel`）、`src/app/state/lx/chat.rs:975`（`user_history_reference_paths`）、`src/event/chat/skills.rs:226`（`reference_blocks`）直接扫描未遮蔽的原始文本。
- **错误原因**：
  - 当行内包含多行折叠 token 时，原始 `line_end` 会停在 token 正文的第一行末尾（位于 token 内部），`newline_below` 会把 `\n` 直接插进 `<pasted-...>` 内部，破坏 token。
  - 当折叠 token 正文包含 `+++ supplement --end`、`&&&`、未闭合的 ```` ``` ````、`@path` 或 `$skill` 时：
    1. `block.rs` / `template.rs` 会把 token 内部的标记当作真实块边界，删除块时从 token 正文中间截断；
    2. `Lx` 折叠块内若含未闭合的 ```` ``` ````，会导致光标在块后永远被判定为“处于代码围栏内”，`/` 斜杠命令面板失效；
    3. 折叠块内的 `@foo` 与 `$skill` 会泄漏进历史引用候选与技能引用注入。
- **修复方案**：
  - `Prompt::newline_below` 与 `insert_template_title_line` 改用 `clamp_line_end` 确保行尾偏移跨过整块折叠 token；
  - `src/app/prompt/{block,template,panel}.rs` 的结构扫描统一使用 `mask_pasted_text(&self.text)`（等长遮蔽，字节偏移与原始文本完全一致），`clean_template_block` 对包含折叠 token 的模板块保留 token 原文不拆碎；
  - `refresh_slash_panel`、`user_history_reference_paths`、`skills::reference_blocks` 在扫描触发词或引用前统一调用 `mask_pasted_text`。

---

### [P2-1] 粘贴面板生命周期、互斥关闭与预览区单击收起边界细节遗漏

- **关键点**：
  1. `src/event/paste.rs:54` 的 `handle_lx_paste` 在取消已打开的粘贴面板之前就因 `text.is_empty()` 提前返回（与 `handle_terminal_paste` 不一致）；
  2. `src/app/update/pane.rs` 的 `set_lx_entry`、`toggle_pane_view` 只取消了路径面板 `paste_cancel()`，漏掉了 `text_paste_cancel()`；`src/app/update/prompt.rs` 的 `save_prompt_now` 只守卫了 `paste_panel().is_some()`，漏掉了 `text_paste_panel().is_some()`；`src/event/mouse/press.rs:769` 点击 Lx 面板外只取消了 `text_paste_cancel()`，漏掉了 `paste_cancel()`；
  3. `Lx::begin_text_paste` 与 `paste_open` 打开粘贴面板时未关闭已打开的 `slash` 与 `refer` 面板；`Prompt::begin_text_paste` 打开时未调用 `clear_other_panels()`；
  4. `Prompt::drag_preview_selection` 与 `Lx::drag_preview_selection` 在鼠标按下后同单元格微动（`probe == anchor`）时走 `probe >= anchor`（`after = true`）选中了 1 个字符，导致松开鼠标时无法单击收起芯片；且在展开区空行上双击/三击因选区为空反而误触发收起芯片。
- **错误原因**：
  - 粘贴面板与已有浮层面板的状态清理散落在各入口，缺少对称的取消/互斥调用；预览选区拖拽对 `probe == anchor` 边界及多击 `SelectionUnit` 缺少守卫。
- **修复方案**：
  - `handle_lx_paste` 在校验 `view`/`rewind` 后立即调用 `text_paste_cancel()` 与 `paste_cancel()`，再判空；
  - `set_lx_entry`、`toggle_pane_view`、`press.rs` 点击外部统一同时取消 `text_paste` 与 `paste`；`save_prompt_now` 同时守卫两个粘贴面板；
  - `Lx::begin_text_paste` 与 `paste_open` 同步清空 `slash` 与 `refer`，`Prompt::begin_text_paste` 同步调用 `clear_other_panels()`；
  - `drag_preview_selection` 在 `SelectionUnit::Char` 且 `probe == anchor` 时保持空选区 `(anchor, anchor)`，且 `finish_preview_selection` 仅当 `unit == SelectionUnit::Char` 且未产生非空选区时才触发单击收起芯片。

---

### [P2-2] `folded::tokens` 遇到未闭合起始标签字面量时会吞并后续合法 token

- **关键点**：
  - `src/app/paste/folded.rs` 的 `try_parse_token` 在匹配到 `<pasted-text lines="N">` 后，直接调用 `body_rest.find(close_tag)` 查找第一个闭合标签。
- **错误原因**：
  - 若用户在编辑器中通过 `Paste raw` 或手写输入了一个未闭合的字面量 `<pasted-text lines="1">`，随后又通过折叠粘贴插入了一个合法的 `<pasted-text lines="3">abc</pasted-text>`，`try_parse_token` 在扫描第一个未闭合标签时会把中间的普通文字连同第二个合法 token 的起始标签一并当作第一个 token 的 `body`，直到第二个 token 的 `</pasted-text>` 才闭合。
- **修复方案**：
  - 在 `try_parse_token` 截取 `raw_body = &text[body_start..body_end]` 后，检查 `raw_body` 长度是否超过 `MAX_FOLDED_BYTES * 2` 以及内部是否包含未转义的同类型起始标签前缀（如 `<pasted-text lines="`）；若包含，则判定当前起始标签未闭合，返回 `None`，使扫描器跳过该字面量并正确识别后续真正的合法折叠 token。
