# 全应用输入框快捷键与交互统一 Review

状态：审查完成，P1/P2/P3 问题已修复并验证；P0 无。

审查范围：`feat/unified-input-shortcuts` 分支（`a8233892`、`15fb3a8b`、`6ae1b392` 及本次修复提交），基线 `cb7bdb05`。

## 1. 结论

| 等级 | 数量 | 状态 |
| --- | --- | --- |
| P0 | 0 | - |
| P1 | 3 | 已修复 |
| P2 | 4 | 已修复 |
| P3 | 4 | 已修复 |

整体功能方向正确：12 类输入框的粘贴、撤销/重做、按词编辑、选区剪切、`Ctrl/Cmd+Shift+A` 全选已统一；`Lx` 与 `Ask` 的键盘选区、选择删除和替换语义已补齐。主要问题集中在修饰键分流、剪切空白的语义、隐藏输入框穿透、搜索框无意义重算和文件行数规范。

## 2. P1 问题

### P1-1 `Alt+Shift+A` 被误判为全选

- 位置：`src/input/mod.rs::lx_input_key`。
- 关键点：Shift 选区分支用 `(ctrl || alt, key.code)` 合并匹配，`(true, Char('a' | 'A'))` 直接返回 `LxInput::SelectAll`。
- 根因：`Ctrl` 与 `Alt` 被粗粒度合并，导致 macOS 的 `Option+Shift+A`（字符输入）命中全选；其他输入框均严格限定 `ctrl && shift && !alt`。
- 修复：将全选单独门控为 `ctrl && !alt && shift`，`Alt+Shift+A` 不再被消费。
- 验证：新增 `Alt+Shift+A` 路由为 `None` 的断言。

### P1-2 纯空白选区剪切会删除但不写剪贴板；Prompt 剪切可能提前短路

- 位置：`src/event/input.rs`（Overlay Cut、Lx Cut、Prompt Cut）。
- 关键点：Overlay/Lx 剪切用 `!text.trim().is_empty()` 才写剪贴板；Prompt Cut 把 `!text.trim().is_empty()` 放在 `replace_range` 之前，空白选区直接跳过删除。
- 根因：把“是否值得提示”的 UI 判断误当成“是否执行剪切”的业务条件；空白文本也是合法选区。
- 修复：剪切路径统一改为 `!text.is_empty()`；Prompt Cut 先执行 `replace_range`/清除选区，再按非空文本写剪贴板。
- 验证：新增浮层空白选区剪切测试与事件级浮层剪切测试。

### P1-3 任务页 / Ask 列表态会复制或剪切隐藏的底层草稿

- 位置：`src/event/input.rs::copy_lx_selection`、`cut_lx_selection`。
- 关键点：事件层在 `apply_lx_input` 之前拦截 `Ctrl+C/X`；任务页打开时输入框被隐藏，Ask 列表态也没有可见输入框。
- 根因：复制/剪切入口没有检查 `lx.task.is_some()`；复制入口在 `ask.is_some() && !ask.editing` 时回落到 `target.lx.selected_text()`。
- 修复：任务页打开时跳过输入框选区（仅保留主内容区复制）；Ask 非编辑态直接拒绝输入框复制/剪切；剪切在任务页直接返回。
- 验证：新增任务页隐藏草稿不被 `Ctrl+C/X` 改写的测试。

## 3. P2 问题

### P2-1 ReferencedFolderSearch 光标移动会重置高亮与视口

- 位置：`src/app/update/referenced/search.rs::edit_referenced_search`。
- 关键点：每次按键后无条件 `refilter()`。
- 根因：`refilter()` 内部会 `selected = 0; viewport = None`；`Left/Right/Home/End/SelectAll` 不改变查询文本，却触发了重算。
- 修复：先快照查询文本，仅文本变化时 `refilter`。
- 验证：新增“光标移动保持高亮/视口”测试。

### P2-2 `prompt_search_drag` 在模式切换后可能污染隐藏搜索框

- 位置：`src/event/mouse/drag.rs`、`src/app/update/prompt.rs`、`src/app/update/prompt/pages_view.rs`。
- 关键点：拖拽标志只在鼠标释放时清理。
- 根因：拖拽期间切换树/页/编辑模式后，后续拖拽仍按旧的搜索框目标处理，并 `return` 拦截正文拖拽。
- 修复：拖拽入口增加模式守卫，模式不匹配时丢弃标志；进入树视图、切换模式、返回编辑视图时同步清理。
- 验证：编译与既有 Prompt 交互测试通过。

### P2-3 `Lx` 无 Shift 方向键不折叠选区

- 位置：`src/app/state/lx.rs`（`move_left/right`、`word_left/right`）。
- 关键点：有选区时普通方向键应折叠到选区边界，而不是清除选区后从当前光标再移动一格。
- 根因：移动方法只依赖 `refresh_panel()` 清空选区，没有先处理选区边界。
- 修复：`←/→` 折叠到选区开头/结尾，`Ctrl/Alt+←/→` 折叠到词选区边界。
- 验证：`apply_editor_edit` 全命令一致性与 Lx 选区测试通过。

### P2-4 `src/app/state/lx.rs` 超过 1000 行红线

- 位置：`src/app/state/lx.rs`。
- 关键点：新增选区/剪切逻辑后文件达到 1122 行，违反 `AGENTS.md` 生产代码单文件上限。
- 根因：选区、文档导航、选择删除全部堆在 `Lx` 主文件。
- 修复：拆出 `src/app/state/lx/selection.rs`，`lx.rs` 降至 992 行，子模块 159 行。
- 验证：`wc -l` 确认均低于 1000 行。

## 4. P3 问题

### P3-1 `TextInput` 撤销栈无上限

- 位置：`src/app/overlay/text_input.rs::push_undo`。
- 根因：长期存在的搜索框/浮层反复编辑时 `undo_stack` 无界增长。
- 修复：增加 `MAX_UNDO_STEPS = 200`，超限丢弃最旧快照。

### P3-2 Todo 剪切 Toast 丢失容器锚点

- 位置：`src/app/update/todo.rs` 的 `TodoKey::Cut`。
- 根因：`Effect::CopyToClipboard { anchor: None }` 使 Toast 降级到屏幕顶部，脱离 Todo 面板。
- 修复：锚点改为 `Some(state.prompt.id())`。
- 验证：测试断言剪切 Effect 的锚点。

### P3-3 事件级回归测试不足

- 位置：`src/event/tests/`。
- 根因：新增的 `Routed::Cut`、浮层粘贴与任务页隔离此前只有更新层测试。
- 修复：补充浮层粘贴、浮层剪切、任务页隐藏草稿隔离的事件级测试。

### P3-4 文档与实现细节不一致

- 位置：`docs/feature/unified-input-shortcuts/design.md`、`task.md`。
- 根因：Todo 剪切最初规划在事件层，实际落在更新层；`Ctrl+A`/`Ctrl/Cmd+Shift+A` 的最终决策也需要回写。
- 修复：同步任务清单、设计决策与本 review；补充 `lx/selection.rs` 拆分说明。

## 5. 复核后排除的误报

1. **Lx `undo/redo` 选区越界 Panic**：两者都会调用 `refresh_panel()`，该方法首行即 `text_selection = None`，不存在旧选区残留。
2. **AskEditor `doc_start/doc_end` 漏清选区**：委托给 `Lx`，同样经 `refresh_panel()` 清理。
3. **Lx 普通移动后选区残留**：`refresh_panel()` 已清理选区；本次仅修正“折叠到边界”的手感，不是安全缺陷。

## 6. 验证

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test --lib`：全部通过（详见提交记录）
- `cargo test --test lx_paste --test prompt_copy --test cursor_steady`：通过
- 既有 3 个集成测试在 `dev` 基线同样失败，与本分支无关：
  - `prompt_panel_interaction::draft_save_search_and_rename_via_real_mouse_and_keys`
  - `prompt_template_commands::narrow_prompt_keeps_buttons_and_ellipsizes_header`
  - `prompt_template_commands::slash_command_inserts_template_with_highlight_and_buttons`
