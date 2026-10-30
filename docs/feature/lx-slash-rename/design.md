# `/rename` 会话重命名命令与内联参数说明：设计方案

> 状态：**用户已通过 Grill Me 逐项确认全部边界**（见 §2），待审核后执行。
>
> 任务范围：在 `src/ui/main_content/lx/` 聊天输入框与 `src/app/` 状态/更新层新增 `/rename` 命令，支持命令面板选择回显 `"/rename "`、输入框内联灰色不可编辑参数说明 `[name]`、空参拦截与会话重命名持久化。

---

## 1. 背景与目标

现状：
- `lx` 聊天输入框内置了 5 个斜杠命令（`/model`、`/resume`、`/new`、`/rewind`、`/compact`），全部为**无参立即执行/开二级面板**型命令，采纳后立即消费整行命令文本。
- 会话重命名目前只能先输入 `/resume` 打开历史面板，选中目标会话按 `⇧↵` 打开 `ResumeSessionEdit` 模态框修改，操作链路较长；用户在当前对话过程中无法直接通过一条斜杠命令快速给当前会话命名。
- 数据库与效应层已具备完整的重命名能力（`Effect::RenameChatSession`、`Database::rename_chat_session`、`update::on_session_renamed`），而 `src/app/update/chat/session.rs` 中的 `rename_session` 尚无斜杠命令入口接入。

目标：
1. 在 `/` 命令面板新增内置命令 `/rename`（别名 `title`）。
2. 在命令面板选择 `/rename`（按 `Enter`、`Tab` 或鼠标点击）后，不直接提交，而是将输入框内容回显为 `"/rename "`（末尾带 1 个空格），光标定位在空格之后，并关闭命令面板。
3. 当输入框处于 `/rename ` 待输入参数状态（无非空白参数）时，在输入框内联渲染灰色（`style::muted()`）参数说明 `[name]`（对标 Claude Code：`❯ /rename  [name]`）；该提示仅存在于 UI 渲染层（Ghost Text），不写入 `lx.input` 文本缓冲，因此**天然不可被删除、不可被修改、不可被光标选中**。
4. 用户按回车提交 `/rename` 时：
   - 若未输入参数（如仅 `"/rename"` 或 `"/rename   "`），拦截发送，保留输入框中的 `"/rename "` 与 `[name]` 提示，并在当前 `lx` 窗格内弹出黄色 Warning Toast。
   - 若当前窗格尚未关联任何已落库会话（`session_id` 为 `None`），清空输入框并弹出黄色 Warning Toast。
   - 若参数非空且存在活跃会话，清空输入框，入队 `Effect::RenameChatSession` 更新会话标题，并在当前 `lx` 窗格内弹出重命名成功 Toast。

---

## 2. 已确认边界（Grill Me 决策表）

| # | 决策点 | 已确认结论 |
| --- | --- | --- |
| 1 | `[name]` 参数提示的生命周期 | 仅当 `!lx.shell_mode()` 且 `lx.input` 以 `"/rename "` 开头、后方无任何非空白字符且无换行时显示灰色 `[name]`；一旦输入任意非空白字符立即消失；退格删空参数回到 `"/rename "` 时重新浮现；继续退格删掉空格变为 `"/rename"` 时，面板重新打开并隐藏 `[name]`。 |
| 2 | 面板采纳与手敲一致性、多行约束 | 1. **面板采纳**：对齐 `/rewind` 的独立单行校验——若输入框含其他行或光标后方有残留文本，拒绝回显并弹黄色 Toast；若为单行命令前缀（如 `/ren`），替换为 `"/rename "`（带尾随空格），光标置于末尾（字节偏移 `8`），关闭 `/` 面板。<br>2. **手敲识别**：用户不经面板直接手敲 `"/rename "` 同样触发 `[name]` 幽灵提示与回车提交逻辑，且输入空格后 `/` 命令候选面板自动收起。 |
| 3 | 无活跃会话（`session_id == None`）处理 | 输入阶段正常回显 `"/rename "` 与 `[name]`；按回车提交时拦截，清空输入框并弹出窗格内黄色 Warning Toast（`"No active session to rename"`）。 |
| 4 | 空参数回车与重命名成功反馈 | 1. **空参数回车**：当输入为 `"/rename"` 或 `"/rename   "`（参数 `trim` 后为空）时，按回车**保留**输入框为 `"/rename "`（不清空，方便继续打字），并在窗格内弹出黄色 Warning Toast（`"Session name cannot be empty"`）。<br>2. **重命名成功**：提取 `trim` 后的新标题，清空输入框（`lx.clear()`），入队 `Effect::RenameChatSession`，并在窗格内弹出 Toast（`Renamed to "<title>"`）。 |
| 5 | 会话忙碌（`busy`）时是否允许重命名 & 命令元数据 | 1. **忙碌时允许**：`/rename` 解析与执行位于 `try_send` 的 `busy` 检查之前，AI 流式回复或工具执行期间均可随时改名（DB 层仅更新 `agent_session.title`，不影响 `updated_at_ms` 及流式写入）。<br>2. **元数据**：命令名 `"rename"`，别名 `&["title"]`，描述 `"Rename the current session"`，保留字表 `RESERVED_NAMES` 同步加入 `"rename"` 与 `"title"`。 |

---

## 3. 核心架构与数据流设计

### 3.1 数据流全景

```text
[用户输入 `/ren` + Enter/Tab/点击面板]
       │
       ▼
accept_slash_command (src/app/update/pane.rs)
  ├─ 校验单行独立性（非单行则弹 LX_RENAME_STANDALONE_HINT 警告并保留原草稿）
  └─ 回显：lx.input = "/rename "，lx.cursor = 8，lx.slash = None
       │
       ▼
ui::main_content::lx::input::draw (src/ui/main_content/lx/input.rs)
  ├─ 正常绘制 lx.input ("/rename ") 与光标（位于 col = content_x + 8）
  └─ 检测到 argument_hint_for_input(&lx.input) == Some("[name]")：
     在 `/rename ` 后方再留 1 列视觉间距（即 content_x + 9，光标右侧 1 列）
     用 style::muted() 绘制只读幽灵文本 "[name]"（不进 lx.input，不可删改）
       │
       ▼
[用户继续输入参数或直接按 Enter]
       │
       ▼
try_send (src/app/update/chat/mod.rs)
  └─ 在 busy / model 检查前调用 session::try_submit_rename_command(state, pane)
       ├─ 若草稿匹配 `/rename` 命令：
       │    ├─ 参数为空 -> 规范化回显为 "/rename "，弹 Warning Toast ("Session name cannot be empty")
       │    ├─ session_id 为 None -> lx.clear()，弹 Warning Toast ("No active session to rename")
       │    └─ 参数非空且有 session_id -> lx.clear()，调用 rename_session 入队 Effect::RenameChatSession，
       │                                 弹 Info/Warning Toast (Renamed to "<title>")
       └─ 若不匹配 -> 返回 false，继续原有 try_send 聊天发送流程
```

---

## 4. 关键模块与关键代码设计

### 4.1 命令定义与触发边界（`src/app/state/lx/slash.rs` & `src/settings/commands.rs`）

1. **`SlashCommand` 枚举扩展**：
   在 `SlashCommand` 中新增 `Rename` 变体，`SlashCommand::ALL` 长度由 `5` 升为 `6`。
2. **带空格时关闭命令候选面板**：
   当前 `slash_trigger(input, cursor)` 对任何以 `/` 开头的行前缀都返回 `Some`，导致用户输入 `"/rename "`（带空格）后，`command_score("rename", "rename ")` 雖返回 `None`（面板因无匹配关闭），但若后续输入的内容恰好模糊命中其他子序列或需要在触发层直接语义阻断，应在 `slash_trigger` 中明确：**一旦 `/` 后的命令词出现空白字符（`char::is_whitespace`），说明用户已结束命令名输入、进入参数区，不再触发一级命令候选面板**。
3. **参数说明纯函数 `inline_argument_hint`**：
   在 `src/app/state/lx/slash.rs` 提供纯函数，供 UI 层调用：

```rust
// src/app/state/lx/slash.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/model`：先选模型，再选其 variants。
    Model,
    /// `/resume`：当前工作区历史列表。
    Resume,
    /// `/new`：清当前窗格会话关联并回空白草稿。
    New,
    /// `/rewind`（别名 `/undo`）：撤销当前会话最新一轮。
    Rewind,
    /// `/compact`：把增量历史压缩成可撤销的检查点。
    Compact,
    /// `/rename`（别名 `/title`）：重命名当前会话。
    Rename,
}

impl SlashCommand {
    /// 全部命令（固定顺序）。
    pub const ALL: [SlashCommand; 6] = [
        SlashCommand::Model,
        SlashCommand::Resume,
        SlashCommand::New,
        SlashCommand::Rewind,
        SlashCommand::Compact,
        SlashCommand::Rename,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Resume => "resume",
            Self::New => "new",
            Self::Rewind => "rewind",
            Self::Compact => "compact",
            Self::Rename => "rename",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Model => &[],
            Self::Resume => &["session"],
            Self::New => &["clear"],
            Self::Rewind => &["undo"],
            Self::Compact => &["summarize"],
            Self::Rename => &["title"],
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Self::Model => "/model",
            Self::Resume => "/resume",
            Self::New => "/new",
            Self::Rewind => "/rewind",
            Self::Compact => "/compact",
            Self::Rename => "/rename",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Model => "Choose the model and reasoning level for the next turn",
            Self::Resume => crate::ui::text::LX_RESUME_COMMAND_DESC,
            Self::New => "Start a fresh blank session",
            Self::Rewind => "Undo the latest turn (files and conversation)",
            Self::Compact => crate::ui::text::LX_COMPACT_COMMAND_DESC,
            Self::Rename => crate::ui::text::LX_RENAME_COMMAND_DESC,
        }
    }

    /// 命令采纳后回显到输入框的带空格前缀（仅带参命令返回 `Some`）。
    pub fn echo_prefix(self) -> Option<&'static str> {
        match self {
            Self::Rename => Some("/rename "),
            _ => None,
        }
    }

    /// 输入框处于等待参数态时的灰色参数说明（如 `[name]`）。
    pub fn argument_hint(self) -> Option<&'static str> {
        match self {
            Self::Rename => Some(crate::ui::text::LX_RENAME_ARG_HINT),
            _ => None,
        }
    }
}

/// 当输入框全文处于某带参命令的「已回显命令 + 尾随空格、尚无有效参数」状态时，
/// 返回应当紧跟渲染的灰色参数说明文案（如 `"[name]"`）。
pub fn active_argument_hint(input: &str) -> Option<&'static str> {
    if input.contains('\n') || input.contains('\r') {
        return None;
    }
    let rest = input.strip_prefix(SlashCommand::Rename.text())?;
    // 必须至少带一个空格（即 `/rename `），且空格后尚未输入任何非空白字符。
    if !rest.starts_with(' ') || !rest.trim().is_empty() {
        return None;
    }
    SlashCommand::Rename.argument_hint()
}
```

同时在 `slash_trigger` 中增加空白字符判断：
```rust
pub fn slash_trigger(input: &str, cursor: usize) -> Option<String> {
    let before = &input[..cursor];
    if crate::app::prompt::list::in_fence(input, cursor) {
        return None;
    }
    let line_start = before.rfind('\n').map_or(0, |offset| offset + 1);
    let line_prefix = &input[line_start..cursor];
    // 行首必须是 `/`（无前导文本），且命令词内部不含空白（输入空格即进入命令参数区，关闭候选面板）。
    if !line_prefix.starts_with('/') || line_prefix.contains(char::is_whitespace) {
        return None;
    }
    Some(line_prefix.to_string())
}
```

同步在 `src/settings/commands.rs` 的 `RESERVED_NAMES` 中加入 `"rename"` 与 `"title"`，防止用户自定义命令与内置 `/rename` 冲突。

---

### 4.2 命令面板采纳回显（`src/app/update/pane.rs`）

在 `accept_slash_command` 中处理 `SlashCommand::Rename`（利用 `command.echo_prefix()`）：
- 与 `/rewind` 相同，校验当前输入是否为独立单行命令（不含其他行、光标后无非空残留）。若夹带其他正文，弹出警告提示并保留原草稿。
- 校验通过后，将 `target.lx.input` 设为 `"/rename "`（走 `Lx` 的编辑或直接写入并重算光标 `target.lx.cursor = prefix.len()`），同时 `target.lx.slash = None` 关闭命令面板。

```rust
// src/app/update/pane.rs -> accept_slash_command 关键分支

if let Some(prefix) = command.echo_prefix() {
    let trimmed = input.trim();
    let line_start = input[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
    let line = input[line_start..].trim();
    let standalone = trimmed == line
        && input[cursor..].trim().is_empty()
        && !line.contains('\n')
        && !line.contains('\r')
        && !line.contains(char::is_whitespace)
        && line.starts_with('/');
    if !standalone {
        super::chat::show_lx_hint(
            state,
            pane,
            super::super::state::InputHintLevel::Warning,
            crate::ui::text::LX_RENAME_STANDALONE_FAILED,
        );
        return true;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.input.clear();
        target.lx.input.push_str(prefix);
        target.lx.cursor = prefix.len();
        target.lx.text_selection = None;
        target.lx.slash = None;
    }
    return true;
}
```

---

### 4.3 回车提交与重命名执行（`src/app/update/chat/session.rs` & `src/app/update/chat/mod.rs`）

1. **解析与提交函数 `try_submit_rename_command`**：
   在 `src/app/update/chat/session.rs` 中新增 `try_submit_rename_command(state: &mut AppState, pane: PaneId) -> bool`：
   - 仅当窗格处于 `PaneView::Lx` 且 `!target.lx.shell_mode()` 时生效。
   - 检查 `target.lx.input`（去除首尾换行校验单行）：
     - 若整行精确为 `"/rename"` 或以 `"/rename "`（含任意空白）开头：
       - 提取命令后的参数 `arg = line.strip_prefix("/rename").unwrap_or("").trim()`。
       - **情况 A（`arg.is_empty()`，用户未输入名称直接回车）**：
         - 确保输入框规范化为 `"/rename "`（光标在末尾 `8`），不清空命令前缀，方便用户直接继续输入名称；
         - 调用 `show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_RENAME_EMPTY_NAME)` 弹出 Toast 提示；
         - 返回 `true`（消费本次 Enter）。
       - **情况 B（`arg` 非空，但 `target.lx.chat.session_id` 为 `None`）**：
         - 调用 `clear_lx_input(state, pane)` 清空输入框；
         - 调用 `show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_RENAME_NO_SESSION)` 弹出 Toast 提示；
         - 返回 `true`。
       - **情况 C（`arg` 非空，且存在 `session_id`）**：
         - 调用 `clear_lx_input(state, pane)` 清空输入框；
         - 调用 `rename_session(state, pane, &session_id, arg)` 入队 `Effect::RenameChatSession`；
         - 调用 `show_lx_hint(state, pane, InputHintLevel::Warning, text::chat::session_renamed(arg))`（或使用 `ToastKind::Info` / 既有 `show_lx_hint` 展示 `Renamed to "..."`）在窗格内给出明确反馈；
         - 返回 `true`。

```rust
// src/app/update/chat/session.rs

/// 尝试作为 `/rename` 命令提交当前输入框草稿。
///
/// 返回 `true` 表示草稿是 `/rename` 命令（无论成功改名还是因空参/无会话被拦截），
/// 调用方 `try_send` 应立即返回，绝不将其作为普通消息发给模型。
pub fn try_submit_rename_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some((input, session_id)) = state.pane_anywhere(pane).and_then(|target| {
        if target.view != crate::app::state::PaneView::Lx || target.lx.shell_mode() {
            return None;
        }
        Some((target.lx.input.clone(), target.lx.chat.session_id.clone()))
    }) else {
        return false;
    };
    let trimmed_start = input.trim_start_matches([' ', '\t']);
    if trimmed_start.contains('\n') || trimmed_start.contains('\r') {
        return false;
    }
    let Some(rest) = trimmed_start.strip_prefix(SlashCommand::Rename.text()) else {
        return false;
    };
    // `/renamefoo` 不是 `/rename` 命令，交由后面的 unknown_slash_command 处理。
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let title = rest.trim();
    if title.is_empty() {
        // 用户未输入参数直接回车：保持 `/rename ` 回显与 `[name]` 提示，弹 Toast 提醒。
        if let Some(target) = state.pane_mut_anywhere(pane)
            && let Some(prefix) = SlashCommand::Rename.echo_prefix()
        {
            target.lx.input.clear();
            target.lx.input.push_str(prefix);
            target.lx.cursor = prefix.len();
            target.lx.text_selection = None;
            target.lx.slash = None;
        }
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_RENAME_EMPTY_NAME,
        );
        return true;
    }
    let Some(session_id) = session_id else {
        clear_lx_input(state, pane);
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_RENAME_NO_SESSION,
        );
        return true;
    };
    if !rename_session(state, pane, &session_id, title) {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    }
    clear_lx_input(state, pane);
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Warning,
        text::chat::session_renamed(title),
    );
    true
}
```

2. **在 `try_send` 中接入（`src/app/update/chat/mod.rs`）**：
   紧接在 `user_shell::try_submit_user_shell` 之后调用 `session::try_submit_rename_command(state, pane)`：
   ```rust
   pub fn try_send(state: &mut AppState, pane: PaneId) -> bool {
       if user_shell::try_submit_user_shell(state, pane) {
           return true;
       }
       if session::try_submit_rename_command(state, pane) {
           return true;
       }
       // ...
   ```
   这样即使当前会话处于流式生成（`busy`）或本地命令运行中，`/rename` 也能即时生效。

---

### 4.4 输入框内联灰色参数说明渲染（`src/ui/main_content/lx/input.rs`）

对标参考图（`❯ /rename  [name]`）：
- 在 `src/ui/main_content/lx/input.rs` 的 `draw` 函数中，当 `!shell` 且 `crate::app::state::lx::slash::active_argument_hint(&lx.input)` 返回 `Some(hint)` 时：
  - 此时 `lx.input` 为 `"/rename "`（或 `/rename` 后跟空格），位于第 0 逻辑行（`row_index == 0`）。
  - 计算已输入文本在首行的显示宽度 `typed_w = display_width(&lx.input)`。
  - 若 `lx.input == "/rename "`（末尾恰好 1 个空格），为了让光标停在 `/rename` 后的第一个空格格上、同时与 `[name]` 保持参考图中的双空格视觉间距（或紧跟在已输入内容之后，至少与 `/rename` 间隔 2 列），取 `hint_offset = typed_w.max(SlashCommand::Rename.text().len() + 2)`（即 `"/rename"` 7 列 + 2 列间距 = 第 9 列，`content_x + 9`）。
  - 计算剩余可用列宽 `avail = usize::from(content_w).saturating_sub(hint_offset)`，通过 `text::truncate_to_width(hint, avail)` 截断后，使用 `grid.put(content_x + hint_offset as u16, text_top, &truncated, style::muted())` 绘制。
- **为什么天然不可删除和修改**：
  - `hint`（`"[name]"`）完全不存于 `lx.input` 中；
  - `lx.cursor` 最大只能到 `lx.input.len()`（即 `"/rename "` 的末尾，第 8 列）；
  - 用户按 `Right` / `End` 不会进入 `[name]` 区域，鼠标点击或拖拽选区按 `lx.input` 字节长度钳制，按 `Backspace` 删除的是 `"/rename "` 末尾的空格（删完变成 `"/rename"`，同时触发命令面板重开并隐藏 `[name]`），输入任何字符则直接追加到 `"/rename "` 后方并使 `[name]` 立即消失。
- **命令高亮样式**：
  - 参考图中 `/rename` 命令本身带有柔和的主色高亮（区别于普通正文白色）。检查 `src/ui/main_content/lx/input.rs`：当 `!shell` 且首行是内置斜杠命令前缀（如 `/rename `）时，或者保持与现有输入框一致的样式。我们在 `paint_line`（或针对激活 `active_argument_hint` / 内置命令前缀的行）保持清晰可读，`[name]` 严格使用 `style::muted()`（灰色字体）。

```rust
// src/ui/main_content/lx/input.rs 关键渲染片段

if !shell
    && scroll == 0
    && let Some(hint) = crate::app::state::lx::slash::active_argument_hint(&lx.input)
{
    // `/rename ` 本身占 8 列（含 1 个尾随空格，光标停在第 8 列）；
    // 在光标右侧再留 1 列空白（共与 `/rename` 间隔 2 列），对齐参考图 `❯ /rename  [name]`。
    let min_offset = crate::app::state::SlashCommand::Rename.text().len().saturating_add(2);
    let offset = display_width(&lx.input).max(min_offset);
    let offset_u16 = u16::try_from(offset).unwrap_or(u16::MAX);
    if offset_u16 < content_w {
        let avail = usize::from(content_w - offset_u16);
        let placeholder = text::truncate_to_width(hint, avail);
        grid.put(
            content_x.saturating_add(offset_u16),
            text_top,
            &placeholder,
            style::muted(),
        );
    }
}
```

---

### 4.5 文案常量定义（`src/ui/text.rs` & `src/ui/text/chat.rs`）

遵守项目规范（所有用户可见文案集中在 `src/ui/text`）：
- `src/ui/text.rs`：
  - `pub const LX_RENAME_COMMAND_DESC: &str = "Rename the current session";`
  - `pub const LX_RENAME_ARG_HINT: &str = "[name]";`
  - `pub const LX_RENAME_EMPTY_NAME: &str = "Session name cannot be empty";`
  - `pub const LX_RENAME_NO_SESSION: &str = "No active session to rename";`
  - `pub const LX_RENAME_STANDALONE_FAILED: &str = "Run /rename on its own line";`
- `src/ui/text/chat.rs`：
  - 新增 `pub fn session_renamed(title: &str) -> String`，对标题做单行字符截断（例如最多 24 字符，超长加 `…`），格式化为 `Renamed to "{sample}"`。

---

## 5. 影响面与兼容性分析

1. **渲染纯度**：`ui::main_content::lx::input::draw` 只读 `lx.input` 并通过纯函数 `active_argument_hint` 判断是否绘制 `[name]`，零状态修改、零 IO。
2. **文件行数预算**：
   - `src/app/state/lx/slash.rs`：当前约 641 行，新增约 35 行，远低于 1000 行上限。
   - `src/app/update/pane.rs`：当前约 871 行，新增约 25 行，保持在 1000 行上限内。
   - `src/app/update/chat/session.rs`：当前约 720 行，新增约 60 行，保持在 1000 行上限内。
   - `src/ui/main_content/lx/input.rs`：当前 196 行，新增约 20 行。
   - `src/ui/text.rs`：当前约 1150 行（历史已略超，新增纯常量 5 行，动态拼串放在 `src/ui/text/chat.rs`）。
3. **既有测试影响**：
   - `SlashCommand::ALL` 从 5 项变为 6 项，检查 `src/app/state/lx/slash/tests.rs` 及 `src/app/update/chat/tests/slash_panels.rs` 中是否有断言 `SlashCommand::ALL.len() == 5` 的测试并同步更新。
   - `slash_trigger` 在包含空格时返回 `None`，验证不影响任何既有无参命令（既有命令名均不含空格）。
$: 0
