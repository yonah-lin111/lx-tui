# unified-text-selection 功能评审

状态：**评审完成，P0~P2 列出的缺陷已修复；待用户在真实终端验收交互手感**（2026-10-28）

范围：`69eb5bf merge: unified text selection and editing shortcuts` 合并到 `dev` 的改动，以及后续 `c0e3413`、`5264c20` 与本次 review 修复。以当前 `dev` 工作树为准，不包含浏览器设置页。

方法：codegraph/codebase memory 追调用链 → 对照 `design.md`、`task.md` 与 `docs/standards/tui-design-requirements.md` → 对每个触发条件写回归测试；只列能复现或能从代码路径证明的问题。

## 0. 结论

统一选择的主链路成立：非空选区由 `Ctrl/Cmd+C` 显式复制，松开鼠标只结束拖动；屏幕取样发生在其最终可见 Buffer 上；prompt/todo/ask/overlay 的编辑命令与异步剪贴板事务有明确的目标校验。

问题集中在合并时的**优先级与生命周期**，以及两类端点边界：

1. **全局快捷键抢在控件/编辑器语义之前**：Shift+点击绕过控件命中；Ctrl+C 在 prompt 树搜索和 todo 行内编辑中丢失清空行为。
2. **模态切换没有同步清理屏幕选区**：右键菜单或浮层关闭后，旧冻结块仍可被复制或反显。
3. **可编辑目标的双击/三击没有回到本地编辑器选区**：prompt 中输入不会替换选中的词/行；单字符词/行因为端点相同被误判为空。
4. **合并重构丢掉了专用的模板块复制路径**：`Ctrl/Cmd+Shift+C` 失效。

## 1. 关键链路

- 鼠标按下：控件（浮层、toast、tab、滚动条、分隔线）优先；只读文字起屏幕选择，可点击文字行登记延迟点击；prompt 的普通拖动走本地编辑选区。
- 鼠标拖动/松开：屏幕选区只更新冻结块与端点；松开不复制、不清选区；延迟点击只有全程未位移才补执行。
- 屏幕取样：`ScreenText::from_buffer` 在最终图层之后、反显之前取样；复制、重定位、宽字符续格都只读该快照。
- 复制/剪切：`copyable_selection` 从屏幕/编辑器取真实选区；剪切经后台原生写入，确认成功且修订未变才删除。
- 输入路由：全局编辑键先于焦点分流，但无选区时必须回落到 prompt 树、todo、lx 等目标自己的语义。

## 2. 缺陷与修复

### P0-1 Shift+左键绕过控件命中

- **触发**：按住 Shift 点击浮层关闭按钮、tab、滚动条、分隔线或 toast `[✕]`；控件不执行，反而在文字层开始屏幕选择。
- **错误原因**：`handle_mouse_down_left` 在浮层、toast、tab、工具栏和滚动条判断之前无条件检查 `SHIFT` 并 `return`；`screen_text.cell()` 任意可绘制格都满足条件，控件格也不例外。
- **修复**：删除早期 Shift 短路；只在终端窗格内容命中处、以及侧栏文字（新增按钮/分组箭头/滚动条/折叠按钮除外），且已通过浮层/控件/滚动条分支后，才用 Shift 强制屏幕选择。普通只读文字、toast、浮层文字仍走各自的选择路径。
- **验证**：新增 `shift_click_keeps_overlay_close_button_priority`；既有 `shift_drag_selects_screen_text_without_forwarding_to_pty` 继续通过。
- **关键文件**：`src/event/mouse/press.rs`、`src/event/tests.rs`。

### P0-2 Ctrl+C 在 prompt 树搜索和 todo 行内编辑中失效

- **触发**：prompt 树视图搜索框有过滤词时按 `Ctrl+C`，过滤词不清空；todo 行内编辑有文本时按 `Ctrl+C`，输入框不清空。
- **错误原因**：统一编辑快捷键把 `Ctrl+C` 提前拦截为全局复制；无选区分支只处理 lx 页清空，没有再看 prompt 树搜索与 todo 编辑态，随后直接返回 `true`，原输入层永远收不到该键。
- **修复**：无选区时先按焦点分派：prompt 树用现有的搜索清空路径，todo 编辑态调用输入框 `clear()`；只有都不是时才进入 lx/普通吞键逻辑。列表态 todo 仍不修改数据。
- **验证**：新增 `ctrl_c_clears_prompt_tree_search`、`ctrl_c_clears_todo_editing_input`。
- **关键文件**：`src/event/input.rs`、`src/event/tests.rs`。

### P0-3 模态打开/关闭后旧屏幕选区仍可复制或反显

- **触发**：在 lx 或 prompt 上建立屏幕选区后右键打开菜单；或在浮层内拖选文字后按 Esc 关闭浮层。旧选区没有被清除，`Ctrl+C` 仍复制冻结的底层/旧浮层文本，高亮也可能叠在新画面上。
- **错误原因**：右键菜单与键盘关闭浮层只修改 `state.overlay`，没有调用 `clear_selection`；`copyable_selection` 又先检查屏幕选区，后检查浮层，导致隐藏内容有更高优先级。
- **修复**：右键打开任何菜单前清除选区；键盘操作使浮层关闭或替换时清除选区；鼠标左键路径原本已在按下时清选区。这样“屏幕选区只能属于当前可见层”重新成立。
- **验证**：新增 `right_click_menu_clears_screen_selection_before_opening`、`closing_overlay_clears_screen_selection`。
- **关键文件**：`src/event/overlay.rs`、`src/event/input.rs`、`src/event/tests.rs`。

### P0-4 prompt 模板块专用复制快捷键被统一重构删除

- **触发**：prompt 光标位于 `&&&` 模板块或 `+++` 补充块内，按 `Ctrl/Cmd+Shift+C`；没有复制、没有 toast。
- **错误原因**：统一重构删除了 `Routed::CopyTemplateBlock`，同时新的 `global_edit_key` 明确拒绝 Shift 组合；`handle_global_edit` 也没有对应分支，快捷键在到达输入层前就被吞掉。
- **修复**：增加独立的 `GlobalEdit::CopyTemplateBlock`；仅在 prompt 聚焦、无 overlay、无页重命名时，调用光标所在块的正文提取与复制反馈；普通 `Ctrl/Cmd+C` 不改变选区复制语义。
- **验证**：`cmd_shift_c_copies_template_block_without_title`（真实 PTY + pbcopy 桩）与 input 路由单测。
- **关键文件**：`src/input/mod.rs`、`src/event/input.rs`、`tests/prompt_copy.rs`。

### P1-1 prompt 双击/三击只形成屏幕选区，不能编辑替换

- **触发**：在 prompt 中双击一个词或三击一行，随后输入字符、粘贴或 `Ctrl+X`；文字没有替换选中的词/行，屏幕上却仍有反显。
- **错误原因**：双击/三击分支直接调用 `begin_screen_selection_unit`，只冻结最终画面；prompt 的编辑命令读取 `state.selection` 本地选区，此时仍为空。三击还只按当前视觉行处理，软折行时会切断逻辑行。
- **修复**：prompt 的双击/三击先映射回 prompt 的内容视觉行/列，建立本地 `Selection`；三击按 `Prompt::visual_rows` 的 `line` 覆盖同一逻辑行的全部软折行。无法映射时保留原屏幕选择兜底。
- **验证**：新增 `prompt_word_selection_uses_local_editing_selection`、`prompt_triple_click_selects_full_wrapped_logical_line`。
- **关键文件**：`src/app/update/selection.rs`、`src/event/mouse/press.rs`、`src/app/update/selection/tests.rs`。

### P1-2 单字符词/行因端点相同被当成空选区

- **触发**：双击单字符词（如 `a`、标点）或三击只含一个字符的行；没有高亮、复制不到内容。
- **错误原因**：`ScreenText::selection_cells` 的 `normalize` 把 `start == end` 直接判空；`ScreenSelection::range` 也把相同端点判空。词/行单位在单字符时本来就只能给出折叠端点。
- **修复**：为 Word/Line 的单非空白格增加 `selection_cells_single`；`ScreenSelection::range` 只在“端点相同且冻结块为空”时返回 None。Character 单击仍是空选区。
- **验证**：新增 `single_character_word_selection_is_not_empty`。
- **关键文件**：`src/app/selection.rs`、`src/app/update/selection.rs`、`src/ui/screen.rs`。

### P1-3 lx 消息列表 / Agents 详情拖选没有边缘滚动与滚轮延伸

- **触发**：在 lx 主聊天历史或后台任务页 Agents 详情中按住拖动文字，把指针停在内容区上/下边缘；视口不滚，选区也不继续增长。拖选中直接滚轮时，内容滚动了但选区终点仍停在旧屏幕行。
- **错误原因**：屏幕选区与 lx 历史选区各自维护，但自动滚动只接到 prompt/terminal 的 `selection_autoscroll`；lx 的 `lx_selection` 和 `screen_selection` 既没有目标区域/总行数登记，也没有在滚动后平移端点。subagent 详情使用屏幕选区，同样没有接入。
- **修复**：新增 `LxSelectionAutoscroll`，按 lx 历史或 Agents 详情登记目标、内容区、总行数/可视行数；`tick` 在边缘滚动后按实际滚动行数平移选区端点，滚轮走同一路径。屏幕选区滚完后等下一次最终画面取样再重算冻结块，避免用旧画面取词。
- **验证**：新增 `lx_selection_edge_autoscroll_moves_history_and_extends_selection`、`lx_selection_wheel_scrolls_viewport_and_extends_selection`、`agents_detail_edge_drag_scrolls_subagent_transcript`。
- **关键文件**：`src/app/update/selection.rs`、`src/app/update/interaction.rs`、`src/event/mouse/drag.rs`、`src/event/mouse/wheel.rs`、`src/event/mouse/hit.rs`、`src/ui/main_content/lx.rs`。

### P2-1 终端双击/三击的 Down 不置脏

- **触发**：在终端窗格双击/三击后暂时按住鼠标，不松开；高亮要等到 Up 才出现。
- **错误原因**：两处 `begin_screen_selection_unit` 的提前返回漏了 `*dirty = true`；正常快速双击时 Up 会补重绘，所以问题不容易被看到。
- **修复**：两处提前返回都置脏。
- **验证**：全量回归；已有双击/三击测试通过。
- **关键文件**：`src/event/mouse/press.rs`。

### P2-2 注释与实现窗口不一致

- **触发**：代码注释仍写“400 ms 同格计数”，实际 `MULTI_CLICK_WINDOW` 已是 250 ms。
- **错误原因**：上一轮把卡片延迟从 400 ms 调到 250 ms 时只改了常量和规范，漏了函数注释。
- **修复**：注释改为 250 ms。
- **关键文件**：`src/app/update/selection.rs`。

### P2-3 折叠卡片有选区时仍要求第二次点击

- **触发**：卡片内容有选中高亮时点击卡片；第一次只清选区，必须再点一次才折叠/展开。
- **错误原因**：上一轮为保护选区加入了 `had_card_selection` 早退分支，把一次点击拆成两次；这与用户期望的“点到卡片就切换”冲突。
- **修复**：删除该早退；点击仍先清旧选区，同一次点击继续进入延迟折叠。普通拖动仍只选字，不折叠。
- **验证**：`card_click_clears_existing_selection_and_toggles`。
- **关键文件**：`src/event/mouse/press.rs`、`src/event/tests.rs`。

## 3. 已核对但未列入缺陷的部分

- **选区保留/清除**：松开不复制、不清选区；`Ctrl+C` 后高亮仍在；点击其他位置清除；滚动后按冻结完整行重定位，原文消失时隐藏高亮但保留可复制 payload。
- **宽字符**：取样与冻结均跳过续格；端点落在续格时回首格；复制只输出一次。
- **容器边界**：跨行长按拖动按起止点容器边界裁剪；首末行不越界；竖边框、纯横边框行和 `▐` 滚动条不进入整行复制。
- **异步粘贴/剪切**：粘贴目标带焦点、窗格、浮层与修订号快照；终端 Paste 提升 generation 取消按键回退；剪切只有原生写入确认且文本/修订未变才删除。
- **lx-subagent 兼容**：Agents 列表行、详情卡片与滚动条走同一套屏幕选择/延迟点击；折叠卡片 250 ms 延迟、双击/三击取消、已有选区时点击先清选区，同一次点击仍切换；详情区和主历史一样支持边缘滚动选字。

## 4. 验证

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：当前 `dev` 全量 **3758 passed / 0 failed / 2 ignored**（含全部集成套件）。
- 新增/更新的关键回归用例：
  - `shift_click_keeps_overlay_close_button_priority`
  - `shift_drag_selects_screen_text_without_forwarding_to_pty`
  - `ctrl_c_clears_prompt_tree_search`
  - `ctrl_c_clears_todo_editing_input`
  - `right_click_menu_clears_screen_selection_before_opening`
  - `closing_overlay_clears_screen_selection`
  - `cmd_shift_c_copies_template_block_without_title`
  - `prompt_word_selection_uses_local_editing_selection`
  - `prompt_triple_click_selects_full_wrapped_logical_line`
  - `single_character_word_selection_is_not_empty`
  - `lx_selection_edge_autoscroll_moves_history_and_extends_selection`
  - `lx_selection_wheel_scrolls_viewport_and_extends_selection`
  - `agents_detail_edge_drag_scrolls_subagent_transcript`
  - `pending_screen_selection_uses_the_next_frame`
  - `card_click_clears_existing_selection_and_toggles`

## 5. 未纳入本次修复的已知限制

以下确认存在，但不属于本轮 P0~P2 的合并回归修复；需要稳定 `TextSource`/输入几何映射或交互决策后再做：

- **非 prompt 可编辑目标的双击/三击**：lx 输入、ask 手写、overlay 单行输入仍可能落在屏幕选区路径；普通拖动与复制可用，但输入替换/剪切尚未完全同步到本地编辑器。
- **通用只读屏幕的逻辑行**：除 prompt 外，`ScreenText` 没有逻辑行映射，三击仍按当前视觉行；prompt 已按真实逻辑行处理。
- **慢速双击的卡片延迟**：卡片延迟和双击窗口统一为 250 ms；超过窗口的第二击会按两次单击处理。
