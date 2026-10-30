# `/rename` 会话重命名命令与内联参数说明 — 代码评审报告 (Code Review)

> 评审范围：`feat/lx-slash-rename`（已合并入主工作区 `dev` 分支）
>
> 检索与分析工具：`codegraph`（调用链与影响面分析）、`codebase-memory-mcp`（`detect_changes` 变更爆炸半径、`trace_path` 调用链追踪、`search_graph` 符号检索）
>
> 优先级定义：
> - **P0**：功能性缺陷 / 终端渲染截断与命令语义不一致 Bug（直接影响用户可见输出或命令提交行为）
> - **P1**：编辑器状态机不变量缺失（撤销栈、修订号 `revision`、子面板清理与软换行视口）
> - **P2**：极窄窗格与别名内联提示的渲染边界防护
>
> 结论：P0 / P1 / P2 全部已修复，回归测试已补齐；`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、相关单元与渲染测试全绿。

---

## P0-1：`lx` 页内 Toast 与全局 Toast 渲染中文等宽字符时未按显示列宽推进，导致 `/rename 询问用户` 只显示一半字符

### 1. 关键点
- **涉及文件**：
  - `src/ui/main_content/lx/toast.rs`（`draw`、`draw_title`）
  - `src/ui/toast.rs`（`render`、`box_size`）
  - `src/ui/main_content/lx/toast/tests.rs`

### 2. 错误原因
1. 在 `src/ui/main_content/lx/toast.rs` 的 `draw` 函数中，原本使用 `for (index, ch) in line.chars().enumerate()` 逐字符遍历，并将单元格横坐标计算为 `x = text_x + index`。
2. 中文字符（CJK 宽字符，如 `询问用户`）每个字符在终端占 **2 列**显示宽度，但 `enumerate()` 步长固定为 **1 列**。
3. 当绘制 `Renamed to "询问用户"` 时：
   - `'询'` 写在列 `c`，在终端占据 `c` 和 `c+1` 两列；
   - `'问'` 被写在列 `c+1`（恰好是 `'询'` 的右半格占位列）；
   - Ratatui 的 `Buffer::diff` 在扫描到宽字符 `'询'` 时，会自动跳过其紧随的右半格 `c+1`（`to_skip = width - 1`），导致写在 `c+1` 的 `'问'` 与写在 `c+3` 的 `'户'` 在下发终端时被静默丢弃，或后续字符覆盖右半格，造成屏幕上只显示 `"询问"` 甚至残缺字符。
4. 同理，`src/ui/toast.rs` 的 `box_size` 使用 `line.chars().count()` 计算盒宽，遇到中文时盒宽少算一半；溢出截断使用按字符计数的 `text::ellipsize` 而非按显示列宽计数的 `text::truncate_to_width`。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/main_content/lx/toast.rs` 中，正文行、顶边标题与右上角关闭按钮统一改用 `buf.set_stringn(x, y, &line, max_width, style)`，由 Ratatui 按 Unicode 显示列宽（`ch.width()`）推进列坐标并维护宽字符后续占位格。
- [x] 将多行溢出截断从 `line.chars().take(...)` 改为 `text::truncate_to_width(line, text_width.saturating_sub(1))`。
- [x] 在 `src/ui/toast.rs` 中，将 `box_size` 改为 `UnicodeWidthStr::width`，标题与正文截断改为 `text::truncate_to_width`。
- [x] 在 `src/ui/main_content/lx/toast/tests.rs` 新增 `wide_cjk_characters_advance_by_display_width_without_clipping` 回归测试，通过 `Buffer::diff_iter` 验证终端实际下发流完整包含 `"询问用户"`。

---

## P0-2：手敲别名 `/title <name>` 提交时未被 `try_submit_rename_command` 识别，被当成普通消息发给模型

### 1. 关键点
- **涉及文件**：
  - `src/app/state/lx/slash.rs`（`SlashCommand::Rename::aliases`、`parse_rename_command`、`active_argument_hint`）
  - `src/app/update/chat/session.rs`（`try_submit_rename_command`）

### 2. 错误原因
1. `SlashCommand::Rename` 定义了别名 `&["title"]`，使得用户在命令面板输入 `/title` 时能搜索并选中 `/rename`。
2. 但当用户不经过面板采纳，而是直接在输入框手敲 `/title 新会话名` 并按回车时：
   - `unknown_slash_command` 对带空格的输入一律放行（返回 `None`）；
   - `try_submit_rename_command` 仅检查了 `trimmed_start.strip_prefix("/rename")`，未匹配别名 `"/title"`；
   - 结果导致 `/title 新会话名` 穿透到 `try_send` 的后续流程，作为普通用户聊天消息发送给了大模型。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/app/state/lx/slash.rs` 中抽取纯函数 `parse_rename_command(input: &str) -> Option<&str>`，统一匹配 `SlashCommand::Rename.name()`（`"rename"`）及其 `aliases()`（`"title"`）。
- [x] `active_argument_hint` 同步支持 `/title ` 状态下的 `[name]` 灰色提示。
- [x] 在 `src/app/update/chat/tests/slash_panels.rs` 与 `src/app/state/lx/slash/tests.rs` 新增别名 `/title` 的参数提示与直接提交测试。

---

## P1-1：采纳 `/rename` 回显 `"/rename "` 时直接裸写 `lx.input`，绕过了 `Lx::replace_input` 导致未记录 Undo 快照、未推进 `revision`、未压制残留子面板

### 1. 关键点
- **涉及文件**：
  - `src/app/update/pane.rs`（`accept_slash_command`）
  - `src/app/update/chat/session.rs`（`apply_slash_command`、`try_submit_rename_command`）
  - `src/app/state/lx.rs`（`Lx::replace_input`）

### 2. 错误原因
1. 在 `accept_slash_command`、`apply_slash_command` 和 `try_submit_rename_command` 中，回显 `"/rename "` 时直接执行了：
   ```rust
   target.lx.input.clear();
   target.lx.input.push_str(prefix);
   target.lx.cursor = prefix.len();
   ```
2. 直接修改字段跳过了 `Lx` 编辑器的核心状态机方法（如 `Lx::replace_input`）：
   - 未调用 `self.snapshot()`，导致用户在 `/ren` 按回车变成 `/rename ` 后无法通过 `Ctrl+Z`（`Undo`）撤回到 `/ren`，且未递增 `self.revision`；
   - 未清理可能并存的 `refer`（快捷引用面板）、`panel`（markdown 块面板）、`history` 浏览态，也未调用 `scroll_cursor_into_view()`。

### 3. 修复方案与状态（已修复）
- [x] 在 `Lx::replace_input` 中补齐 `self.scroll_cursor_into_view()`。
- [x] 在 `accept_slash_command`、`apply_slash_command` 与 `try_submit_rename_command` 中统一改用 `target.lx.replace_input(prefix)` 完成命令回显（若当前输入已等于 `prefix` 则仅复位光标与选区，避免空参连按回车产生冗余撤销步）。
- [x] 新增单元测试验证采纳 `/rename` 后 `revision` 递增且 `Undo` 可撤回 `/ren`。

---

## P2-1：极窄窗格下 `/rename ` 软换行或使用 `/title ` 时，内联参数提示 `[name]` 位置计算未绑定单视觉行与实际命令词宽

### 1. 关键点
- **涉及文件**：
  - `src/ui/main_content/lx/input.rs`（`draw` L134-156）

### 2. 错误原因
1. `input::draw` 原先仅判断 `scroll == 0`，未判断 `rows.len() == 1`。当窗格被拖拽到极窄（宽度小于 8 列）导致 `"/rename "` 自身发生软换行（`rows.len() > 1`）时，在 `text_top`（第一视觉行）绘制 `[name]` 会与折行内容错位。
2. 原先 `min_offset` 硬编码为 `SlashCommand::Rename.text().len() + 2`（9 列），若用户输入的是别名 `"/title "`（7 列），`[name]` 会离 `/title` 产生 3 列间距而非标准的 2 列间距。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/main_content/lx/input.rs` 中增加 `rows.len() == 1` 守卫（仅当命令未发生跨行折行时绘制同行幽灵提示）。
- [x] 将 `min_offset` 改为 `display_width(lx.input.trim()).saturating_add(2)`，使 `/rename ` 与 `/title ` 均保持统一的 2 列视觉间距。
