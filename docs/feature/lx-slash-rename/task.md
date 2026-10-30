# `/rename` 会话重命名命令与内联参数说明：执行任务清单

- 状态：**已完成（分支 `feat/lx-slash-rename`，待用户确认合并）**
- 关联设计：`docs/feature/lx-slash-rename/design.md`
- 工作区：`.worktrees/lx-slash-rename`

---

## Task 1：新增 `/rename` 文案常量与格式化函数

- **目标文件**：
  - `src/ui/text.rs`
  - `src/ui/text/chat.rs`
- **具体修改**：
  1. 在 `src/ui/text.rs` 新增常量：
     - `LX_RENAME_COMMAND_DESC: &str = "Rename the current session"`
     - `LX_RENAME_ARG_HINT: &str = "[name]"`
     - `LX_RENAME_EMPTY_NAME: &str = "Session name cannot be empty"`
     - `LX_RENAME_NO_SESSION: &str = "No active session to rename"`
     - `LX_RENAME_STANDALONE_FAILED: &str = "Run /rename on its own line"`
  2. 在 `src/ui/text/chat.rs` 新增函数 `session_renamed(title: &str) -> String`：
     - 去除首尾空白，按字符上限（24 字符）截断并附 `…`，返回 `Renamed to "{sample}"`，并补充单元测试。

---

## Task 2：扩展 `SlashCommand::Rename`、触发规则与保留词

- **目标文件**：
  - `src/app/state/lx/slash.rs`
  - `src/settings/commands.rs`
- **具体修改**：
  1. 在 `SlashCommand` 枚举中新增 `Rename`，更新 `SlashCommand::ALL`（长度 6），实现 `name() -> "rename"`、`aliases() -> &["title"]`、`text() -> "/rename"`、`description() -> text::LX_RENAME_COMMAND_DESC`。
  2. 为 `SlashCommand` 新增 `echo_prefix(self) -> Option<&'static str>`（`Rename` 返回 `Some("/rename ")`）与 `argument_hint(self) -> Option<&'static str>`（`Rename` 返回 `Some(text::LX_RENAME_ARG_HINT)`）。
  3. 新增纯函数 `active_argument_hint(input: &str) -> Option<&'static str>`：当 `input` 为单行、以 `"/rename "` 开头且后方无非空白字符时返回 `Some("[name]")`。
  4. 在 `slash_trigger(input, cursor)` 中：当 `line_prefix` 含空白字符（`char::is_whitespace`）时返回 `None`，确保输入 `"/rename "`（带空格）后自动收起 `/` 命令候选面板。
  5. 在 `src/settings/commands.rs` 的 `RESERVED_NAMES` 中加入 `"rename"` 与 `"title"`（数组长度相应增加）。

---

## Task 3：命令面板采纳回显与回车提交重命名逻辑

- **目标文件**：
  - `src/app/update/pane.rs`
  - `src/app/update/chat/session.rs`
  - `src/app/update/chat/mod.rs`
- **具体修改**：
  1. 在 `src/app/update/pane.rs` 的 `accept_slash_command` 中：
     - 若 `command.echo_prefix()` 返回 `Some(prefix)`（即 `/rename`），检查当前输入是否为独立单行命令前缀；
     - 若含多行或光标前后有非命令残留，调用 `show_lx_hint` 弹出 `LX_RENAME_STANDALONE_FAILED` 并保留原草稿；
     - 若校验通过，将 `target.lx.input` 设为 `"/rename "`，`target.lx.cursor = prefix.len()`，清除选区并关闭 `target.lx.slash`。
  2. 在 `src/app/update/chat/session.rs` 中：
     - `apply_slash_command` 补齐 `SlashCommand::Rename` 分支（正常路径由 `accept_slash_command` 直接回显，若直接调用则写入 `"/rename "`）。
     - 新增 `pub fn try_submit_rename_command(state: &mut AppState, pane: PaneId) -> bool`：
       - 识别单行 `/rename` 或 `/rename <args>`；
       - 参数为空时重置输入框为 `"/rename "`（保留回显与 `[name]` 提示），弹出 `LX_RENAME_EMPTY_NAME` Warning Toast；
       - 参数非空但 `session_id` 为 `None` 时清空输入框，弹出 `LX_RENAME_NO_SESSION` Warning Toast；
       - 参数非空且有 `session_id` 时，调用 `rename_session(state, pane, &session_id, title)` 入队 `Effect::RenameChatSession`，清空输入框，并弹出 `text::chat::session_renamed(title)` Toast。
  3. 在 `src/app/update/chat/mod.rs` 的 `try_send` 开头（紧随 `user_shell::try_submit_user_shell` 之后）调用 `session::try_submit_rename_command(state, pane)`。

---

## Task 4：输入框渲染灰色不可删改的参数说明 `[name]`

- **目标文件**：
  - `src/ui/main_content/lx/input.rs`
- **具体修改**：
  1. 在 `draw` 函数中，当 `!shell && scroll == 0` 且 `active_argument_hint(&lx.input)` 返回 `Some(hint)` 时：
     - 计算提示起始列偏移 `offset = display_width(&lx.input).max(SlashCommand::Rename.text().len() + 2)`（在 `"/rename "` 的光标列后再空 1 列，形成 `❯ /rename  [name]` 的视觉间距）；
     - 若 `offset < content_w`，按剩余宽度截断 `hint`，并以 `style::muted()` 灰色样式写入 `grid` 第 0 行（`text_top`）。
  2. 确保该文本只写入渲染网格 `Grid`，绝不修改 `lx.input` 与 `lx.cursor`。

---

## Task 5：单元测试与回归验证

- **目标文件**：
  - `src/app/state/lx/slash/tests.rs`
  - `src/app/update/chat/tests/slash_panels.rs`
  - `src/ui/main_content/lx/tests/slash_panels.rs`（或对应 `input` 测试模块）
- **具体测试项**：
  1. `slash/tests.rs`：
     - `/rename` 与别名 `/title` 在 `commands_matching` 中可被搜索命中；
     - `slash_trigger` 在输入 `"/rename "`（带空格）时返回 `None`；
     - `active_argument_hint` 在 `"/rename "` 时返回 `Some("[name]")`，在 `"/rename"`、`"hello"`、`"/rename my-title"`、多行文本时返回 `None`。
  2. `update/chat/tests/slash_panels.rs`：
     - 面板选中 `/rename` 按 `Enter` 或 `Tab` 后，`lx.input` 变为 `"/rename "`，`lx.cursor == 8`，面板关闭；
     - 输入框为 `"/rename "` 时直接按 `Enter`，弹出 `LX_RENAME_EMPTY_NAME` Toast，且输入框仍保持 `"/rename "`；
     - 输入任何编辑键（如 `Backspace`、`Left`、`Delete`）无法删除或修改 `[name]`，在 `"/rename "` 按一次 `Backspace` 变为 `"/rename"` 并重开命令面板；
     - 无 `session_id` 时输入 `"/rename new title"` 按 `Enter`，弹出 `LX_RENAME_NO_SESSION` Toast 且清空输入框；
     - 有 `session_id` 时输入 `"/rename new title"` 按 `Enter`，入队 `Effect::RenameChatSession`（`title == "new title"`），清空输入框并弹出成功 Toast。
  3. `ui` 渲染测试：
     - 当 `lx.input == "/rename "` 时，渲染网格在 `/rename` 右侧绘制出 `style::muted()` 的 `[name]`；
     - 当 `lx.input == "/rename abc"` 时，网格中不再包含 `[name]`。
  4. 运行校验：
     - `rtk cargo fmt --check`（或 `cargo fmt`）
     - `rtk cargo clippy --all-targets -- -D warnings`
     - `rtk cargo test`（针对受影响模块精确执行验证）
