# 退出二次确认、模板块标题生成与 UI 命名整理 — 设计方案

> 状态：**已实现、已 review 修复、自动化验证通过；待用户 `cargo run` 人工验收；未合并**
>
> 关联文档：`docs/feature/quit-confirm-title-and-naming/task.md`、`review.md`
>
> 代码检索依据：codegraph + codebase-memory（当前工作区 `lx-tui`，branch `dev`）
>
> Review 修订：初版 `Overlay::ConfirmQuit` + `suspended_overlay` 在 review 中确认会让被挂起浮层丢失异步回执（`/connect` OAuth、引用文件夹校验/搜索、Worktree 列表），已改为独立 `quit_confirm` 层。第 4 节保留初版方案作为历史记录，**实际实现以第 10 节为准**。

---

## 1. 背景与现状

### 1.1 退出路径

当前退出只有一条应用级路径，没有二次确认：

- `src/input/mod.rs:151`：`Ctrl+Q` 在路由最前面直接返回 `Action::Quit`。
- `src/app/update.rs:51`：`Action::Quit` 立即执行 `commit_editing_prompt(state)` 并置 `state.should_quit = true`。
- `src/event/mouse/press.rs:168`：点击标签栏右端 `[EXIT]` 按钮同样直接 `Action::Quit`。
- `src/event/mod.rs:303`：事件循环只检查 `state.should_quit`，置位后退出。

问题：用户误触 `[EXIT]` 或 `Ctrl+Q` 时，编辑中的 prompt 已被提交、应用直接退出，没有反悔机会。

### 1.2 模板块标题生成

`/title` 的现有链路：

```text
Prompt::confirm_generate_block_title (src/app/prompt/panel.rs:322)
  ├─ find_enclosing_supple_block 优先定位 +++，否则 template_block_start_at 定位 &&&
  ├─ template_block_body / supple_body_at_start 取整段正文
  ├─ markdown::extract_block_title_source(body) 清洗
  └─ PromptTitleAction::Generate { content, ... }
        │
        ▼
flush_prompt_title_action (src/app/update/chat/title.rs:106)
  └─ chat::title::template_block_title_prompt_for(content, language)
        │
        ▼
template_block_title_prompt_for (src/chat/title.rs:164)
  └─ title_instructions_for(language, sample) + "Task template content:\n" + 正文
```

现存问题：

1. `template_block_body` 返回 `&&&` 起止行之间的全部正文，**包含嵌套的 `+++` 块**；给外层 `&&&` 起标题时会被补充说明带偏。
2. `copy_template_content` 只删除 `%%%` / `+++ log` 的起止标记，**保留其正文**；日志和模型输出被当成任务描述喂给标题模型。
3. `title_instructions_for` 同时服务会话标题与模板块标题，只要求 “summarize the topic or intent”，没有“具体对象/模块名”的约束，也没有参考数据库标题风格。
4. 会话标题与模板块标题共用 `TITLE_PROMPT_MAX_BYTES = 960`；直接追加 few-shot 会挤压正文预算。

数据库现状（`~/.lx-t-dev/db/lx.db`，`workspace_prompt_page.content` 中的 `{title: ...}`）：

| 标题 | 字符数 |
| --- | --- |
| `联网搜索` | 4 |
| `修改工具交互` | 6 |
| `添加/mcp` | 6 |
| `调整项目，屏蔽权限` | 9 |
| `左侧ws添加历史列表` | 10 |
| `html添加统计use` | 11 |
| `后台任务面板添加滚动条` | 11 |
| `添加全部功能的cli工具` | 12 |
| `修改todo item的缩进` | 14 |
| `web端的deepseek添加到lx` | 18 |
| `为lx和prompt输入框添加opencode文字省略` | 27 |

平均 11.6 字，绝大多数 ≤18 字，共同特征是 **“动词 + 具体对象/模块名”**，而不是“优化功能”“调整逻辑”这类泛化概括。

### 1.3 目录与命名

已核实的不一致：

| 现状 | 问题 |
| --- | --- |
| `src/ui/sidebar.rs` | 左栏叫 `sidebar`，右栏却叫 `right_panel`，没有 `left_panel` |
| `src/ui/right_panel.rs` + `src/ui/right_panel/todo/` | 右栏容器与 Todo 子面板 |
| `src/ui/prompt.rs` + `src/ui/prompt/` | Prompt 编辑器渲染独立在右栏之外，没有收进 `right_panel` |
| `src/ui/prompt_frame.rs` | Prompt 几何与命中 helper 独立成文件 |
| `src/ui/panes.rs` | 主区终端网格叫 `panes`，而 lx 主内容在 `src/ui/main_content/` |
| `src/layout.rs` | 根级 BSP 平铺模型，与 `src/ui/layout.rs`（屏幕分区）同名不同职责 |
| `src/layout/prompt.rs` | Prompt 几何放在根 `layout` 下，与 UI 渲染/事件命中分离 |
| `src/ui/controls.rs` | 设置、用量、退出、折叠、交换、拖拽提示混在一个 447 行文件里 |

超限生产文件（项目规范上限 1000 行）：

- `src/chat/process/manager/mod.rs`：1077 行
- `src/app/update/prompt.rs`：1019 行

---

## 2. 目标与不做的事

### 2.1 目标

1. **退出二次确认**：第一次 `Ctrl+Q` 或点击 `[EXIT]` 只打开确认弹窗；确认后或再次 `Ctrl+Q` 才真正退出；取消后原界面与编辑器内容原样保留。
2. **模板块标题生成**：
   - `&&&` 标题只使用 `&&&` 自身的直接正文，剔除全部嵌套 `+++` / `%%%` / `+++ log` / 注释 / 标题行 / `{wt:}`；
   - `+++` 标题使用“外层 `&&&` 直接正文 + 当前 `+++` 自身正文”，剔除其他 `+++` 与日志子块；
   - 标题风格对齐数据库中的短标题：6–20 字、动词 + 具体对象/模块名，few-shot 写进英文提示词。
3. **UI 命名与结构整理**：只统一 UI 容器命名与落位、拆分两个超限生产文件；`app/`、`event/`、`chat/`、`db/` 的模块归属与分层不变。

### 2.2 不做的事

- 不把 `app/state/update` 按领域垂直重切。
- 不重命名 `AppState` 字段、`FocusedContainer` 与 `Action` 的业务变体（`Sidebar` / `Panes` 等），避免把文件移动扩大成行为层重构。
- 不改会话标题生成的行为与 960 字节预算（只新增模板块专用指令与预算）。
- 不拆分现有巨型测试文件（如 `src/app/update/tests.rs`），只随被测模块移动。
- 不做 git 提交、`git add`、`git mv`；文件移动使用普通 `mv`，由 Git 自动识别 rename。

---

## 3. 已确认边界（Grill Me 决策表）

| # | 决策维度 | 已确认结论 |
| --- | --- | --- |
| 1 | 结构重整边界 | 只做 UI 容器命名对齐 + 超限文件拆分；`app/event/chat/db` 模块归属不动 |
| 2 | `&&&` 内容选取 | 只取 `&&&` 直接正文；整块剔除所有嵌套 `+++`、`%%%`、`+++ log`、注释、标题行、`{wt:}` |
| 3 | `+++` 内容选取 | 外层 `&&&` 直接正文 + 当前 `+++` 自身正文；剔除全部 `+++` 子块，其他兄弟 `+++` 不参与 |
| 4 | 标题风格 | 中文 6–20 字、英文 ≤36 字符且尽量 ≤5 词；动词 + 具体对象/模块名；静态 few-shot，不运行时查库 |
| 5 | 模糊标题处理 | 不做模糊词黑名单、不自动重试；只保留现有格式清洗、拒答拦截与 36 字符硬截断 |
| 6 | 退出确认交互 | 第一次只开弹窗；`Esc` 取消并恢复底层弹窗；`Enter` / 点击确认 / 再次 `Ctrl+Q` 才退出；点击弹窗外不关闭；确认时才提交编辑器 |
| 7 | 文档与顺序 | 一个 feature 文件夹、三阶段；“退出确认 → 标题生成 → 结构改名”，改名最后 |
| 8 | 超限文件拆分 | 只拆 `chat/process/manager/mod.rs` 与 `app/update/prompt.rs`；950–999 行文件不动 |

---

## 4. 退出二次确认详细设计

> **本节为初版方案（已被 Review 修订）**：实际实现使用独立 `quit_confirm` 层，不再使用 `Overlay::ConfirmQuit` / `suspended_overlay`。保留本节用于对照 review 结论；当前架构、输入/渲染/鼠标边界见第 10 节。

### 4.1 状态模型

新增一个**应用级确认浮层**，仍然遵守“同一时刻最多一个 `Overlay`”的约束：

```rust
// src/app/overlay.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    // ...
    ConfirmQuit,
}

#[derive(Debug)]
pub enum Overlay {
    // ...
    ConfirmQuit,
}
```

`Overlay` 是单值 `Option`。为了在用户取消时恢复打开确认前的弹窗，`AppState` 增加一个暂存槽：

```rust
// src/app/state.rs
pub struct AppState {
    // ...
    /// 打开退出确认时暂存的原浮层；取消时原样恢复，确认时清空。
    pub suspended_overlay: Option<Overlay>,
}
```

`AppState::demo()` / `Default` 初始化为 `None`。

**为什么不用 `bool` 字段**：`bool` 需要额外分支处理“确认浮层 + 底层浮层同时存在”，会绕开现有的单浮层路由与渲染；用 `Overlay::ConfirmQuit` + `suspended_overlay` 可以复用现有模态点击、压暗、键盘过滤和光标处理。

### 4.2 Action 拆分

`Action::Quit` 拆成三个语义：

```rust
// src/app/actions.rs
pub enum Action {
    /// 请求退出：打开确认浮层，不提交、不置 should_quit。
    RequestQuit,
    /// 确认退出：提交编辑中的 prompt，置 should_quit。
    ConfirmQuit,
    /// 取消退出：恢复打开确认前的浮层。
    CancelQuit,
    // ... 其余不变
}
```

`src/app/update.rs` 的 `apply`：

```rust
Action::RequestQuit => super::update::quit::request_quit(state),
Action::ConfirmQuit => super::update::quit::confirm_quit(state),
Action::CancelQuit => super::update::quit::cancel_quit(state),
```

新增 `src/app/update/quit.rs`：

```rust
//! 退出二次确认：请求、确认与取消。

use crate::app::overlay::Overlay;
use crate::app::state::AppState;

/// 第一次退出请求：只打开确认浮层，暂存底层浮层。
pub fn request_quit(state: &mut AppState) {
    if matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        return;
    }
    if state.overlay.is_some() {
        state.suspended_overlay = state.overlay.take();
    }
    super::clear_selection(state);
    state.overlay = Some(Overlay::ConfirmQuit);
}

/// 确认退出：只有此时才提交编辑器并置 should_quit。
pub fn confirm_quit(state: &mut AppState) {
    if !matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        return;
    }
    state.overlay = None;
    state.suspended_overlay = None;
    super::commit_editing_prompt(state);
    state.should_quit = true;
}

/// 取消退出：恢复打开确认前的浮层（没有则回到无浮层）。
pub fn cancel_quit(state: &mut AppState) {
    if !matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        return;
    }
    state.overlay = state.suspended_overlay.take();
}
```

`close_overlay` 增加保护，避免其他路径把确认浮层当普通浮层直接丢掉：

```rust
// src/app/update/menu.rs
pub fn close_overlay(state: &mut AppState) {
    if matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        super::quit::cancel_quit(state);
        return;
    }
    state.overlay = None;
}
```

### 4.3 键盘路由

`src/input/mod.rs` 的 `route` 仍把 `Ctrl+Q` 放在最前面，但按当前浮层决定语义：

```rust
if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
    let action = if overlay == Some(OverlayKind::ConfirmQuit) {
        Action::ConfirmQuit
    } else {
        Action::RequestQuit
    };
    return Some(Routed::Action(action));
}
```

`overlay_key` 增加分支：

```rust
OverlayKind::ConfirmQuit => match key.code {
    KeyCode::Esc => Some(OverlayKey::Esc),
    KeyCode::Enter => Some(OverlayKey::Enter),
    _ => None,
},
```

`src/app/update/dialog.rs` 的 `apply_overlay_key` 在**最前面**处理退出确认，必须早于 `escape_clear_selection`，否则有文字选区时 `Esc` 会被选区清除逻辑吞掉：

```rust
if matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
    match key {
        OverlayKey::Esc => super::quit::cancel_quit(state),
        OverlayKey::Enter => super::quit::confirm_quit(state),
        _ => {}
    }
    return None;
}
```

另外，`src/event/input.rs:36` 在路由之前还有一次裸 `Esc` 清选区的提前返回：

```rust
if key.code == KeyCode::Esc && key.modifiers.is_empty() && update::escape_clear_selection(state) {
    *dirty = true;
    return;
}
```

`request_quit` 打开确认浮层时已经调用 `clear_selection`，所以正常路径下没有选区可清；为避免未来其他路径再次留下选区，仍建议给该提前返回加一条前置条件：`!matches!(state.overlay, Some(Overlay::ConfirmQuit))`，确保确认浮层下的 `Esc` 永远归取消退出。

### 4.4 渲染与鼠标

文案常量放在 `src/ui/text.rs`，与 `EXIT_LABEL` 同处：

```rust
pub const EXIT_CONFIRM_TITLE: &str = "Exit lx";
pub const EXIT_CONFIRM_QUESTION: &str = "Exit lx?";
```

按钮复用现有确认弹窗的 `BUTTON_CONFIRM`（`↵ confirm`）与 `BUTTON_CANCEL`（`esc cancel`），不新增按钮语义。

`src/ui/overlay/dialog.rs` 新增：

```rust
/// 退出确认浮层几何。
pub fn quit_confirm_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    confirm_shell(screen, text::EXIT_CONFIRM_QUESTION)
}

/// 渲染退出确认：普通强调色模态（非危险色），问题行 + 底部按钮行。
pub(crate) fn render_quit_confirm(
    frame: &mut Frame<'_>,
    screen: Rect,
) -> Option<(u16, u16)> {
    let question = text::EXIT_CONFIRM_QUESTION;
    let shell = quit_confirm_shell(screen)?;
    widgets::modal::render(frame, &shell, text::EXIT_CONFIRM_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let width = usize::from(shell.inner.width);
    for (index, line) in text::wrap(question, width).iter().enumerate() {
        frame.render_widget(
            Paragraph::new(Span::styled(line.clone(), style::text())),
            Rect::new(
                shell.inner.x,
                shell.inner.y.saturating_add(index as u16),
                shell.inner.width,
                1,
            ),
        );
    }
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &CONFIRM_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_CONFIRM, style::button_primary()),
            (text::BUTTON_CANCEL, style::button_secondary()),
        ],
    );
    None
}
```

`src/ui/overlay.rs` 的 `render` 增加：

```rust
Overlay::ConfirmQuit => dialog::render_quit_confirm(frame, screen),
```

鼠标点击在 `src/event/overlay.rs` 增加分支，**点击弹窗外不关闭**：

```rust
Some(Overlay::ConfirmQuit) => {
    let Some(shell) = ui::overlay::quit_confirm_shell(screen) else {
        update::quit::cancel_quit(state);
        return None;
    };
    if !shell.area.contains((column, row).into()) {
        return None;
    }
    match ui::overlay::confirm_button_at(&shell, column, row) {
        Some(ui::overlay::ConfirmButton::Confirm) => update::quit::confirm_quit(state),
        Some(ui::overlay::ConfirmButton::Cancel) => update::quit::cancel_quit(state),
        None => {}
    }
    None
}
```

> 注意：该分支必须放在 `modal_dismissed` 通用逻辑之前，不能复用“点击外部关闭”的默认行为。

`src/event/mouse/press.rs` 在调用 `handle_overlay_click` 之前记录 `was_confirm_quit`；点击结束后若 `was_confirm_quit` 为真，直接 `*dirty = true; return;`，避免取消退出后误触发 `reveal_active_tab` 等底层点击后续逻辑：

```rust
let was_confirm_quit = matches!(state.overlay, Some(Overlay::ConfirmQuit));
// ...
if let Some((pane, bytes)) = handle_overlay_click(state, *screen, mouse.column, mouse.row) {
    write_to_pane(sessions, pane, &bytes);
    update::reset_pane_scroll(state, pane);
}
if was_confirm_quit {
    *dirty = true;
    return;
}
```

### 4.5 退出确认边界表

| 场景 | 行为 |
| --- | --- |
| 无浮层，第一次 `Ctrl+Q` / 点击 `[EXIT]` | 打开 `ConfirmQuit`，`should_quit = false`，编辑器不变 |
| `ConfirmQuit` 打开，再按 `Ctrl+Q` | `ConfirmQuit`，提交编辑器并退出 |
| `ConfirmQuit` 打开，按 `Enter` / 点击 `↵ confirm` | 提交编辑器并退出 |
| `ConfirmQuit` 打开，按 `Esc` / 点击 `esc cancel` | 恢复 `suspended_overlay`，编辑器不变 |
| `ConfirmQuit` 打开，点击弹窗外 | 不关闭、不退出 |
| 其他浮层（重命名、`/connect` 等）打开，按 `Ctrl+Q` | 暂存该浮层并打开 `ConfirmQuit`；取消后原浮层原样恢复 |
| 其他浮层打开，点击 `[EXIT]` | 鼠标被模态拦截，不会命中底层退出按钮 |
| 确认退出时 | 只执行一次 `commit_editing_prompt`，随后事件循环按 `should_quit` 收尾 |

### 4.6 测试点

- `src/input/tests.rs`：
  - `Ctrl+Q` → `Routed::Action(Action::RequestQuit)`；
  - 当 `overlay = Some(ConfirmQuit)` 时 `Ctrl+Q` → `Action::ConfirmQuit`；
  - `ConfirmQuit` 下 `Esc` / `Enter` 映射为 `OverlayKey::Esc` / `Enter`。
- `src/app/update/tests.rs`：
  - `RequestQuit` 不开退出、不清编辑器，只开 `ConfirmQuit`；
  - `ConfirmQuit` 置 `should_quit` 并提交编辑器；
  - `CancelQuit` 恢复暂存浮层。
- `src/app/update/prompt/tests.rs`：
  - 原 `quit_commits_pre_paste_edits_without_placeholder` 改为“请求 → 确认”两步后再断言提交内容。
- `src/event/tests/`：
  - 点击 `[EXIT]` 打开确认；点击确认按钮退出；点击取消恢复；
  - 点击弹窗外不关闭；`Esc` 取消；`Enter` 确认。
- `src/ui/overlay/dialog/tests.rs`：
  - `quit_confirm_shell` 在小屏降级、按钮命中与渲染快照。

---

## 5. 模板块标题生成详细设计

### 5.1 内容选取规则

| 目标块 | 参与内容 | 必须剔除 |
| --- | --- | --- |
| `&&&` | 起始行之后、结束行之前的直接正文 | 全部嵌套 `+++` 块（起止行 + 正文）、`%%%` 块、`+++ log` / `+++ logTemplate` 块、`//` 注释、`{title:}` 行、`{wt:}` 绑定行、内置默认大标题 |
| `+++` | 外层 `&&&` 的直接正文（同样剔除全部 `+++` 子块）+ 当前 `+++` 自身正文 | 其他兄弟 `+++`、`%%%`、`+++ log`、注释、标题行、绑定行、默认大标题 |
| 顶层 `+++`（不在 `&&&` 内） | 仅当前 `+++` 自身正文 | 同上 |

代码块围栏内的 `+++` / `%%%` **不当作子块**，原样保留；`@[refer-*]`、文件路径、命令名保留，因为它们是标题最有信息量的定位信息。

### 5.2 纯函数与数据流

新增两个只做字符串处理的纯函数（放在 `src/app/markdown/template.rs`）：

```rust
/// `&&&` 标题生成正文：剔除全部嵌套子块、注释、标题行与绑定行，
/// 清理未填写空项与内置默认大标题。
pub fn template_title_source(body: &str) -> String;

/// `+++` 标题生成正文：外层 `&&&` 正文 + 当前 `+++` 正文；
/// 外层为 None 表示顶层 `+++`，只取自身。
pub fn supple_title_source(outer_body: Option<&str>, supple_body: &str) -> String;
```

数据流：

```text
Prompt::confirm_generate_block_title
  ├─ masked = mask_pasted_text(&self.text)            // 结构定位：粘贴内容不参与块解析
  ├─ kind == Supple:
  │    ├─ own   = self.supple_body_at_start(start_line)      // 原始正文，保留粘贴内容
  │    └─ outer = self.enclosing_template_body(start_line)   // 外层 &&& 正文
  │         └─ markdown::supple_title_source(outer.as_deref(), &own)
  └─ kind == Template:
       └─ body = self.template_block_body(start_line)
            └─ markdown::template_title_source(&body)
```

`Prompt::enclosing_template_body` 用 `markdown::block_scan` 定位父模板，避免手写嵌套扫描：

```rust
/// 当前 `+++` 所在外层 `&&&` 的正文；顶层 `+++` 返回 None。
fn enclosing_template_body(&self, supple_start_line: usize) -> Option<String> {
    let masked = mask_pasted_text(&self.text);
    let span = markdown::block_scan(&masked)
        .spans
        .into_iter()
        .find(|span| {
            span.kind == markdown::BlockKind::Template
                && span.start < supple_start_line
                && supple_start_line <= span.end
        })?;
    self.template_block_body(span.start)
}
```

`block_scan` 只把首个顶层 `&&&` 识别为 `Template`，嵌套 `+++` 在该模板区间内按普通 middle 行处理；因此这个查找天然覆盖“`+++` 嵌套在 `&&&` 内”的情况，且不会把 `&&&` 之后的顶层 `+++` 误判为子块。

### 5.3 清洗算法

新增 `extract_block_title_source` 的替换实现（`copy_template_content` 保留原语义，不修改）：

```rust
pub fn extract_block_title_source(body: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut subblock_depth = 0usize;
    let mut first_content = true;

    for line in lines {
        if subblock_depth > 0 {
            if subblock_end(line) {
                subblock_depth -= 1;
            }
            continue;
        }
        if let Some((marker, length)) = fence {
            kept.push(line);
            if fence_delimiter(line).is_some_and(|(m, l)| m == marker && l >= length) {
                fence = None;
            }
            continue;
        }
        if let Some(delim) = fence_delimiter(line) {
            fence = Some(delim);
            kept.push(line);
            continue;
        }
        if subblock_start(line) {
            subblock_depth += 1;
            continue;
        }
        if first_content && is_title_line(line) {
            first_content = false;
            continue;
        }
        if !line.trim().is_empty() {
            first_content = false;
        }
        if parse_worktree_line(line).is_some() || is_comment_line(line) {
            continue;
        }
        kept.push(line);
    }

    clean_template_content(&kept.join("\n"))
        .lines()
        .filter(|line| !DEFAULT_TEMPLATE_HEADINGS.contains(&line.trim()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}
```

关键点：

1. **先处理子块**：`subblock_depth > 0` 时整行丢弃，`%%%` / `+++` / `+++ log` 的正文不会进入标题输入。
2. **代码围栏优先**：围栏内原样保留，不把 `+++ not-a-block --start` 误判为子块。
3. **标题行只跳首个内容行**：只跳过紧跟在起始行后的 `{title: ...}`，避免误删正文里恰好长得像标题的行。
4. **默认大标题最后剔除**：`# Execute Task` 等内置标题行在清理后过滤，避免成为标题依据。
5. `copy_template_content` 不动：复制快捷键仍保留 `%%%` / `+++ log` 正文，这是复制语义，不是标题语义。

### 5.4 提示词与 few-shot

`src/chat/title.rs` 增加**模板块专用**指令函数，不修改会话标题使用的 `title_instructions_for`：

```rust
/// 模板块标题专用最大字节数；比会话标题的 960 更宽，容纳 few-shot 后仍保留正文预算。
pub const TEMPLATE_TITLE_PROMPT_MAX_BYTES: usize = 2048;

/// 模板块标题指令：基础语言约束 + 数据库标题风格 few-shot。
pub fn template_title_instructions_for(
    language: TitleLanguage,
    sample_text: &str,
) -> String {
    let base = title_instructions_for(language, sample_text);
    let examples = match resolve_title_language(language, sample_text) {
        TitleLanguage::SimplifiedChinese => {
            "\n\nExamples of good titles:\n\
             - 修改 todo item 的缩进\n\
             - 添加 /mcp 命令\n\
             - 左侧 ws 添加历史列表\n\
             - 后台任务面板添加滚动条\n\
             - 为 prompt 输入框添加省略\n\
             Use the same pattern: a specific verb plus the concrete module, \
             file, command, or panel name. Never output a generic verb alone."
        }
        TitleLanguage::English => {
            "\n\nExamples of good titles:\n\
             - Fix todo item indentation\n\
             - Add /mcp command\n\
             - Add workspace history list\n\
             - Add scrollbar to task panel\n\
             - Truncate long prompt input\n\
             Use the same pattern: a specific verb plus the concrete module, \
             file, command, or panel name. Never output a generic verb alone."
        }
        TitleLanguage::Auto => {
            "\n\nExamples of good titles:\n\
             - Fix todo item indentation\n\
             - Add /mcp command\n\
             - Add workspace history list\n\
             - Add scrollbar to task panel\n\
             - Truncate long prompt input\n\
             Use the same pattern: a specific verb plus the concrete module, \
             file, command, or panel name. Never output a generic verb alone."
        }
        _ => {
            "\n\nUse the same pattern: a specific verb plus the concrete module, \
             file, command, or panel name. Never output a generic verb alone."
        }
    };
    format!("{base}{examples}")
}
```

`template_block_title_prompt_for` 改用该函数与新预算：

```rust
pub fn template_block_title_prompt_for(
    block_content: &str,
    language: TitleLanguage,
) -> Option<String> {
    let trimmed = block_content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let instructions = template_title_instructions_for(language, trimmed);
    let prefix = format!("{instructions}\n\nTask template content:\n");
    let budget = TEMPLATE_TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}
```

语言解析抽成 `resolve_title_language(language, sample) -> TitleLanguage`，`title_instructions_for` 与 `template_title_instructions_for` 共用，避免两套判断漂移。

**约束不变**：结果仍走 `parse_generated_title`（去 `<think>`、代码围栏、`Title:` 前缀、引号/标点、拒答拦截、36 字符硬截断）。不新增模糊词黑名单、不自动重试。

few-shot 以 `-` 列表形式展示，模型有概率原样带回 `- `；`sanitize_title` 增加一条纯格式清理：在现有 `trim_start_matches('#')` 之后，剥离开头的 `- ` / `* ` / `+ ` / `1. ` 列表标记。这是格式清洗，不是模糊词审查，会话标题与模板块标题共用。

### 5.5 字节预算

| 用途 | 常量 | 说明 |
| --- | --- | --- |
| 会话标题 | `TITLE_PROMPT_MAX_BYTES = 960` | 不变，避免回归 |
| 模板块标题 | `TEMPLATE_TITLE_PROMPT_MAX_BYTES = 2048` | 新增，容纳 few-shot |
| 输出硬上限 | `TITLE_MAX_CHARS = 36` | 不变 |

按中文 3 字节/字估算：few-shot 约 220 字节，基础指令约 600 字节，剩余约 1.2 KB，足够 300+ 中文字符的正文；实际正文还会被 `truncate_utf8_bytes` 按 UTF-8 边界安全截断。

### 5.6 关键代码改动清单

| 文件 | 改动 |
| --- | --- |
| `src/app/markdown/template.rs` | 新增/重写 `extract_block_title_source`、`template_title_source`、`supple_title_source`；保留 `copy_template_content` |
| `src/app/markdown/tests.rs` 或 `src/app/markdown/template/tests.rs` | 新增内容选取、子块剔除、围栏、空项清理测试 |
| `src/app/prompt/panel.rs` | `confirm_generate_block_title` 改用两个新纯函数；新增 `enclosing_template_body` |
| `src/app/prompt/block/tests.rs` | `/title` 在嵌套 `+++`、日志、多个兄弟 `+++` 下的 `content` 断言 |
| `src/chat/title.rs` | 新增 `TEMPLATE_TITLE_PROMPT_MAX_BYTES`、`template_title_instructions_for`、`resolve_title_language`；改 `template_block_title_prompt_for` |
| `src/chat/title/tests.rs` | few-shot 存在性、2048 预算、会话标题不带 few-shot、语言分支 |
| `src/app/update/chat/title/tests.rs` | 端到端 `content` 传递与 prompt 前缀断言 |

### 5.7 测试点

1. **`&&&` 只取自身**：
   - 输入含嵌套 `+++`、`%%%`、`+++ log`、`//` 注释、`{wt:}`；
   - 断言 `template_title_source` 不包含子块正文与标记，保留 `- Requirements:` 及其子项。
2. **`+++` 取外层 + 自身**：
   - 输入为 `&&&` 内嵌 `+++`；断言结果同时包含外层任务描述与当前 `+++` 正文；
   - 同级另一个 `+++` 不出现；
   - 顶层 `+++` 只包含自身正文。
3. **代码围栏**：
   - 围栏内 `+++ foo --start` / `%%% foo --start` 原样保留。
4. **空块**：
   - 只有默认大标题与空列表项时返回空串，`/title` 仍走 `EmptyBlock`。
5. **提示词**：
   - `template_block_title_prompt_for` 包含 `修改 todo item 的缩进` 等 few-shot；
   - `session_title_prompt_for` 不包含 few-shot；
   - 总长度 ≤ `TEMPLATE_TITLE_PROMPT_MAX_BYTES`，UTF-8 边界不 panic。

---

## 6. UI 命名与结构整理

### 6.1 目标目录树

```text
src/
  tiling.rs                    # 原 src/layout.rs：BSP 平铺模型，不再含 prompt 几何
  tiling/
    tests.rs                   # 原 layout/tests.rs 的 BSP 测试
  ui/
    layout.rs                  # 屏幕区域划分（保留）
    left_panel/                # 原 ui/sidebar
      mod.rs
      history.rs
      tests.rs
    main_content/              # 原 ui/main_content.rs + ui/panes.rs
      mod.rs
      panes.rs
      lx/ ...
      tests.rs
    right_panel/               # 原 ui/right_panel.rs
      mod.rs
      todo/ ...
      prompt/                  # 原 ui/prompt.rs + ui/prompt/* + ui/prompt_frame.rs
        mod.rs
        geometry.rs            # 原 src/layout/prompt.rs
        frame.rs               # 原 ui/prompt_frame.rs
        header.rs
        pages.rs
        panels.rs
        recent.rs
        tree.rs
        tests.rs
    controls/                  # 原 ui/controls.rs
      mod.rs
      panels.rs
      footer.rs
      exit.rs
      swap.rs
      resize.rs
      tests.rs
    tab_bar.rs
    style.rs
    text.rs
    ...
```

### 6.2 文件映射表

| 现状 | 目标 | 说明 |
| --- | --- | --- |
| `src/layout.rs` | `src/tiling.rs` | 根级 BSP 平铺模型；`mod prompt` 移出 |
| `src/layout/prompt.rs` | `src/ui/right_panel/prompt/geometry.rs` | Prompt 几何属于 UI |
| `src/layout/tests.rs` 中 15 个 `prompt_*` 测试 | `src/ui/right_panel/prompt/geometry/tests.rs` | 随几何代码移动 |
| `src/ui/sidebar.rs` | `src/ui/left_panel/mod.rs` | 左栏统一命名 |
| `src/ui/sidebar/history.rs` | `src/ui/left_panel/history.rs` | |
| `src/ui/sidebar/tests.rs` | `src/ui/left_panel/tests.rs` | |
| `src/ui/right_panel.rs` | `src/ui/right_panel/mod.rs` | 右栏容器改为目录 |
| `src/ui/right_panel/todo/` | `src/ui/right_panel/todo/` | 位置不变 |
| `src/ui/prompt.rs` | `src/ui/right_panel/prompt/mod.rs` | Prompt 收进右栏 |
| `src/ui/prompt/*.rs` | `src/ui/right_panel/prompt/*.rs` | 保持子模块名 |
| `src/ui/prompt/tests.rs` | `src/ui/right_panel/prompt/tests.rs` | |
| `src/ui/prompt_frame.rs` | `src/ui/right_panel/prompt/frame.rs` | Prompt 几何/命中 helper |
| `src/ui/panes.rs` | `src/ui/main_content/panes.rs` | 终端网格收进主内容 |
| `src/ui/main_content.rs` | `src/ui/main_content/mod.rs` | 主内容改为目录 |
| `src/ui/controls.rs` | `src/ui/controls/mod.rs` + 子模块 | 按按钮组拆分 |

`src/ui/controls/` 子模块划分：

| 子模块 | 内容 |
| --- | --- |
| `mod.rs` | 模块声明与 `pub use` / `pub(crate) use` 重导出（常量实际定义在 `panels.rs`） |
| `panels.rs` | `PANEL_BUTTON_WIDTH` / `PANEL_BUTTON_MARGIN`、折叠按钮、Agents 按钮、Prompt pin、折叠窄条、`collapsed_content_x` |
| `footer.rs` | 底边框设置按钮、用量按钮 |
| `exit.rs` | `[EXIT]` 按钮几何、命中与绘制 |
| `swap.rs` | 主区/右栏交换按钮 |
| `resize.rs` | 分割线拖拽提示与高亮 |

### 6.3 `ViewLayout` 字段命名

`src/ui/layout.rs` 的 `ViewLayout` 是 UI 容器几何的唯一载体，字段名同步到新语义：

```rust
pub struct ViewLayout {
    pub left_panel: Rect,
    pub tab_bar: Rect,
    pub main_content: Rect,
    pub right_panel: Rect,
    /// 右栏是否位于主内容左侧（与 main_content 互换）。
    pub right_panel_left: bool,
}
```

只改 `ViewLayout` 字段与 `layout::compute` 内部变量；`AppState.sidebar_*` / `prompt_*` / `panes_*` 字段保持不变（避免把文件移动扩展成 app 状态重命名）。

### 6.4 引用路径迁移

| 旧路径 | 新路径 |
| --- | --- |
| `crate::layout::*` | `crate::tiling::*`（`PaneId`、`pane_rects`、`pane_inner_rect` 等） |
| `ui::sidebar::*` | `ui::left_panel::*` |
| `ui::prompt_frame::*` | `ui::right_panel::prompt::frame::*` |
| `crate::layout::prompt_*` | `ui::right_panel::prompt::geometry::*`（app 实际使用的纯几何下沉到 `app::prompt::geometry`，UI 再重导出） |
| `ui::panes::*` | `ui::main_content::panes::*` |
| `view.sidebar` | `view.left_panel` |
| `view.prompt` | `view.right_panel` |
| `view.panes` | `view.main_content` |
| `view.prompt_left` | `view.right_panel_left` |

迁移顺序：

1. 先移动文件并修正 `mod` 声明，让 `cargo check` 报错集中暴露旧路径；
2. 按编译器错误逐个替换 import / 路径；
3. 最后做 `ViewLayout` 字段改名（一次性 `sed` 式替换 + `cargo check` 收口）。

### 6.5 超限文件拆分

#### `src/chat/process/manager/mod.rs`（1077 行）

| 目标文件 | 内容 |
| --- | --- |
| `manager/types.rs` | `ProcessLimits`、`ProcessUiEvent`、`ProcessError`、`ProcessSnapshot` |
| `manager/handle.rs` | `ProcessHandle` 及其 `impl`（输出缓冲、游标、tail、UI pending） |
| `manager/mod.rs` | `ProcessManager` 结构与公开方法、`next_process_id`、`global_running`、`spawn_pipe_reader`、`exit_state`、`estimated_tokens`、`lock` |
| `manager/lifecycle.rs` | `StopReason`、自由函数 `stop_handle`、`resolve_cwd`、`StopWaiter` |
| `manager/spawn.rs` | 保持现状 |

`mod.rs` 只保留 `pub use` 与 `ProcessManager` 的实现，目标 ≤700 行。

#### `src/app/update/prompt.rs`（1019 行）

| 目标文件 | 内容 |
| --- | --- |
| `update/prompt/items.rs` | 条目/临时提示词生命周期：`edit_prompt_item`、`edit_prompt_temp`、`edit_prompt_temp_row`、`open_temp_prompt`、删除/复制/保存 |
| `update/prompt/menu.rs` | 条目/临时/文件夹/树右键菜单打开与重命名、删除确认 |
| `update/prompt/save.rs` | `commit_editing_prompt`、`commit_prompt_buffer`、`mark_prompt_page_persisted`、`reset_prompt_autosave` |
| `update/prompt/mod.rs` | 树模式切换、选中恢复、`apply_prompt_tree`、`show_prompt_tree`、`stash_editing_prompt`、`select_tree_entry_row` 与 `pub use` |

现有 `draft.rs`、`drag.rs`、`pages.rs`、`pages_view.rs`、`recent.rs`、`tree_view.rs` 保持不动；`tests.rs` 保持一个测试模块，通过 `super::*` 访问拆出的函数。

### 6.6 文档同步

必须更新：

- `docs/standards/project-directory-structure.md`：目录树、模块职责、依赖方向中的路径。
- `docs/standards/tui-design-requirements.md`：`sidebar` / `right_panel` / `prompt` / `main_content` 路径描述。
- `docs/standards/code-writing-standards.md`：涉及 `ui::main_content::lx`、`app::update` 等路径的引用（如有）。
- `AGENTS.md`：涉及 `ui::main_content::lx` 的路径保持不变；若新增 `left_panel` 命名，补一句目录约定。

不重写历史 `docs/feature/*` 文档（它们描述过去的状态）。

### 6.7 结构与行为分离

结构阶段只允许：

- 文件移动、`mod` 声明、`use` 路径、`ViewLayout` 字段改名；
- 超限文件内的函数搬迁（函数体逐字复制，不借机改逻辑）；
- 文档路径更新。

禁止在结构阶段夹带行为修改；任何 `cargo test` 失败必须先判断是路径迁移错误还是原有行为差异。

---

## 7. 影响文件清单

### 7.1 退出确认

- `src/app/actions.rs`
- `src/app/overlay.rs`
- `src/app/state.rs`
- `src/app/update.rs`
- `src/app/update/quit.rs`（新增）
- `src/app/update/menu.rs`
- `src/app/update/dialog.rs`
- `src/input/mod.rs`
- `src/event/overlay.rs`
- `src/event/mouse/press.rs`（仅 `[EXIT]` 命中语义不变，确认由 `Action` 分派）
- `src/ui/overlay.rs`
- `src/ui/overlay/dialog.rs`
- `src/ui/text.rs`
- 测试：`src/input/tests.rs`、`src/app/update/tests.rs`、`src/app/update/prompt/tests.rs`、`src/event/tests/*`、`src/ui/overlay/dialog/tests.rs`

### 7.2 标题生成

- `src/app/markdown/template.rs`
- `src/app/markdown/tests.rs` / `template/tests.rs`
- `src/app/prompt/panel.rs`
- `src/app/prompt/block/tests.rs`
- `src/chat/title.rs`
- `src/chat/title/tests.rs`
- `src/app/update/chat/title/tests.rs`
- `src/chat/prompt.rs`（如提示词规范文档需要同步，仅加注释，不改行为）

### 7.3 结构整理

- `src/lib.rs`
- `src/layout.rs` → `src/tiling.rs`
- `src/layout/prompt.rs` → `src/ui/right_panel/prompt/geometry.rs`
- `src/layout/tests.rs` → `src/tiling/tests.rs` + `geometry/tests.rs`
- `src/ui/mod.rs`
- `src/ui/sidebar*` → `src/ui/left_panel*`
- `src/ui/right_panel*` / `src/ui/prompt*` / `src/ui/prompt_frame.rs`
- `src/ui/main_content*` / `src/ui/panes.rs`
- `src/ui/controls*`
- `src/ui/layout.rs` 与所有 `view.*` 调用点
- `src/chat/process/manager/*`
- `src/app/update/prompt*`
- `docs/standards/*.md`、`AGENTS.md`

---

## 8. 验证计划

每个阶段独立收口，禁止最后一次性验证：

### 阶段 1：退出确认

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test input::tests
cargo test app::update::tests
cargo test app::update::prompt::tests
cargo test event::tests
cargo test ui::overlay
```

### 阶段 2：标题生成

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test app::markdown
cargo test app::prompt
cargo test app::update::chat::title
cargo test chat::title
```

### 阶段 3：结构整理

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

阶段 3 的 `cargo test` 必须全量跑：纯文件移动没有行为差异，全量失败通常意味着路径遗漏或 `ViewLayout` 字段漏改。

### 人工验收

Agent 无法交互式验证 TUI，以下由用户运行 `cargo run` 确认：

1. `Ctrl+Q` 第一次只出现 `Exit lx?`，编辑器内容不变；
2. `Esc` 取消后回到原界面；若之前在重命名弹窗，取消后重命名弹窗仍在；
3. `Ctrl+Q` 两次或点击 `↵ confirm` 才退出；
4. 点击 `[EXIT]` 同样先确认；
5. 在嵌套 `+++` 的 `&&&` 块内执行 `/title`，生成的标题只概括外层任务，不被补充块/日志带偏；
6. 在 `/supplement` 生成的 `+++` 内执行 `/title`，标题同时包含关联的外层 `&&&` 任务与当前补充说明；
7. 新目录结构下 `cargo run` 界面、鼠标命中、折叠/交换按钮、Prompt/Todo 切换均正常。

---

## 9. 风险与回滚

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| ~~`suspended_overlay` 与 `Overlay` 生命周期不一致~~ | 已在 review 中确认会丢失异步回执 | 已改为独立 `quit_confirm` 层；底层 `overlay` 不再移走 |
| ~~`Ctrl+Q` 在 `apply_overlay_key` 中被选区清除逻辑抢先~~ | 初版风险 | 独立确认层在 `event/input.rs` 最前面处理，早于清选区/浮层过滤 |
| 按键自动重复绕过二次确认 | 长按 `Ctrl+Q`/`Esc` 可能直接退出或穿透 | 连续 Press 每次刷新 300ms 静默窗口 + `Repeat` 忽略；取消后 300ms latch 随重复刷新；仍建议人工长按验收 |
| 标题清洗误删代码块 | 标题丢失关键信息 | 围栏优先于子块识别，新增围栏测试 |
| `block_scan` 的“仅首个 `&&&`”约束 | 渲染/复制仍只识别首个模板 | `/title` 关联扫描已支持多 `&&&` 与第二个模板内的 `+++`；其他功能保持现状 |
| `TEMPLATE_TITLE_PROMPT_MAX_BYTES` 增大 | 个别供应商上下文限制 | 2048 字节远低于常规上下文；不改变会话标题 960 预算 |
| 结构改名路径遗漏 | 编译失败或运行时命中错误 | 阶段 3 全量 `cargo test` + `cargo clippy`；`ViewLayout` 字段改名后全局搜索旧名确认零命中 |
| 文件移动导致 Git 误判 | review 噪音 | 使用普通 `mv`，不 `git add`/`git mv`，由 Git 自动识别 rename；不提交 |

回滚策略：

- 阶段 1、2 是行为改动，若验收失败，按文件粒度回滚（`app/update/quit.rs`、`overlay`、`chat/title.rs`、`markdown/template.rs`）。
- 阶段 3 是纯移动/改名，若出现大面积编译失败且短时间无法收口，直接按 `task.md` 的文件映射反向移动即可；不影响前两阶段已完成的逻辑。

---

## 10. Review 修复后修订（当前实现）

> 本节覆盖第 4 节初版退出方案，并记录标题/结构 review 修复后的实际落点。详细问题与证据见 `review.md`。

### 10.1 退出确认：独立层而非替换 `overlay`

- `AppState`：
  - `quit_confirm: Option<QuitConfirmState>`（`last_shortcut_press`）
  - `quit_cancel_latch: Option<Instant>`
  - `prompt_committed_for_quit: bool`
  - 删除 `suspended_overlay`；删除 `Overlay::ConfirmQuit` / `OverlayKind::ConfirmQuit`。
- `request_quit`：只打开 `quit_confirm`，不触碰 `state.overlay`、焦点、选区；清理正在进行的鼠标手势（分割线/滚动条/拖拽），避免确认期间继续改状态。
- `confirm_quit`：关闭确认层，提交一次编辑器，置 `should_quit=true` 与 `prompt_committed_for_quit=true`。
- `cancel_quit`：关闭确认层，记录 300ms latch；底层浮层与异步生命周期原样保留。
- 输入：
  - 确认层打开时，`event/input.rs` 最前面只处理 `Ctrl+Q` / `Esc` / `Enter`，其余键吞掉。
  - `Ctrl+Q` 按住时每次自动重复都刷新 300ms 静默窗口，`Repeat` 直接忽略；松开后两次 Press 间隔满 300ms 才确认。
  - `Esc` 取消后 300ms 内吞 `Esc`/`Enter`，每次自动重复刷新窗口，松开并静默后才恢复底层浮层按键。
- 渲染：底层 `overlay` → tooltip/toast → 退出确认；确认层最后绘制，短屏跳过标题并裁剪问题行，任何 toast/浮层不能盖住它。
- 鼠标：确认层打开时，按下/拖拽/松开/滚轮/右键/粘贴全部拦截；`quit_confirm_shell` 为空时只吞点击，不取消；点击弹窗外不关闭。
- 异步：Connect/Referenced/Worktree/Trust 的 handler 继续看到原来的 `state.overlay`，OAuth/模型刷新/校验/搜索回执不会丢；Trust 只能更新底层浮层，不能顶掉确认层。
- 收尾：确认路径已提交 prompt，`Persistence::shutdown` 通过 `prompt_committed_for_quit` 跳过第二次提交；普通退出仍提交。

### 10.2 标题生成修订

- `extract_block_title_source`：fence 优先；`+++`/`%%%` 子块用标记栈配对，嵌套结束/围栏内结束不再提前闭合外层。
- 标题源专用清理：只对围栏外片段执行 `clean_template_content` 与默认大标题过滤，围栏内占位项/标题/空行逐字保留；`copy_template_content` 语义不变。
- `enclosing_template_body`：支持“包含当前 `+++` 的 `&&&`”和“最近前置闭合 `&&&`”，正常 `/supplement` 的兄弟 `+++` 也能带出外层任务；支持多个 `&&&`。
- `find_enclosing_supple_block` / `supple_body_at_start`：`+++` 栈式扫描，嵌套 `+++` 不在内层 `--end` 截断。
- 提示词：模板专用 few-shot 明确中文 6–20 字、英文 ≤36 字符且 ≤5 词，示例不超过 5 词，示例后要求“Return exactly one title, not a list.”；会话标题 960 预算与指令不变。
- `sanitize_title`：循环清理 braces/quotes/`#`/列表符/标题标签/强调/尾随标点；拒答改为整句或句首匹配，合法标题“修复无法生成标题的 bug”不再误杀。

### 10.3 结构修订

- app 实际使用的纯几何下沉到 `src/app/prompt/geometry.rs`（`PROMPT_BLOCK_GUTTER_LEFT`、`PROMPT_TREE_HEADER_HEIGHT`、`PromptBodySplit`、`prompt_body_split`、body rect 等）；`app` 不再引用 `ui::right_panel::prompt::geometry`。
- 右栏通用几何移到 `src/ui/right_panel/geometry.rs`；Todo/右栏容器使用它，左栏历史使用本地 `LIST_HEADER_HEIGHT`，避免反向依赖 prompt 内部。
- `manager/lifecycle.rs` 承载 `StopReason` / `stop_handle` / `resolve_cwd` / `StopWaiter`；`manager/mod.rs` 保留 `ProcessManager` 与主循环辅助。
- `controls/panels.rs` 定义 `PANEL_BUTTON_WIDTH` / `PANEL_BUTTON_MARGIN`；`controls/mod.rs` 只做声明与重导出。
- `app/update/prompt/pages.rs` 删除重复 `workspace_cwd`，统一走 `prompt/mod.rs`。
- 退出确认文案移入 `ui/text/workspace.rs`，`ui/text.rs` 回到 1000 行内。

### 10.4 验证与未完成项

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全通过；lib 测试 5192 个。
- `tests/prompt_copy.rs` 的 pbcopy 桩改为原子写入，消除全量并发下的空文件竞态。
- 未完成：既有超限测试文件 `app/update/prompt/tests.rs`（2516）、`app/update/tests.rs`（5653）、`ui/tests.rs`（3321）的拆分；以及真实 TUI 人工验收。两者记录在 `review.md`，不与本次行为修复混做。
