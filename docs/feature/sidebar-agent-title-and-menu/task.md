# 侧栏 Agents 条目第二行标题与右键菜单 — 执行任务

> 状态：**待用户审核**（本清单审核通过后才允许修改生产代码）
> 设计依据：[design.md](design.md)
> 规则：生产代码禁止 `unwrap()` / `expect()`；用户可见文案全部收敛到 `ui/text/`；`app/` 管状态与更新、`ui/` 只读渲染、`event/` 只分派；全部标题修改只走内存，不写 DB、不改会话/终端真实标题。

---

## P0. 审核门槛与基线核对

- [ ] 用户审核确认 [design.md](design.md) 与本 `task.md`。
- [ ] 核对当前分支与工作区干净：`git status --short`，确认没有需要保留的未提交用户改动。
- [ ] 复读项目规范：
  - `docs/standards/project-directory-structure.md`
  - `docs/standards/code-writing-standards.md`
  - `docs/standards/tui-design-requirements.md`
- [ ] 用 Codegraph / Codebase Memory 再核对以下符号当前实现，避免设计文档与最新代码漂移：
  - `AppState::agent_entries`
  - `AgentListItem` / `agent_list_items` / `agent_item_index_at` / `agent_list_rows`
  - `LxChat::reset_session_view` / `on_session_loaded_full`
  - `update_pane_agent`
  - `MenuCommand` / `OverlayTarget` / `RenameTarget` / `commit_rename`
  - `open_right_click_menu`

---

## P1. 状态层：标题覆盖与条目序号（`src/app/`）

### 1.1 `SidebarAgentEntry` 与 `agent_entries`

- [ ] `src/app/state/pane.rs`：`SidebarAgentEntry` 增加 `pub pane_index: usize`，文档注明“该 Tab 内 `layout.pane_ids()` 树序位置，1 基”。
- [ ] `src/app/state.rs`：`agent_entries()` 改为 `for (pane_offset, pane_id) in tab.layout.pane_ids().into_iter().enumerate()`，`pane_index = pane_offset + 1`，lx 与终端两条记录都写入同一个 `pane_index`。
- [ ] 保持 `agent_entry_count()` 不变，避免引入标题解析或额外分配。
- [ ] 确认 `agent_panes()` 映射仍然兼容（只取 `(tab_index, pane_id)`）。

### 1.2 `LxChat` 内存标题覆盖

- [ ] `src/app/state/lx/chat.rs`：
  - `LxChat` 增加 `pub agent_title_override: Option<String>`，文档明确“仅侧栏显示、不落库、不改 `session_title`”。
  - `LxChat::new()` 初始化为 `None`。
  - `LxChat::reset_session_view()` 末尾清空 `agent_title_override`。
- [ ] `src/app/update/chat/session.rs`：`on_session_loaded_full` 的 `session_changed` 分支清空 `target.lx.chat.agent_title_override`，覆盖“从 A 会话 `/resume` 到 B 会话”不经过 `reset_session_view` 的路径。
- [ ] 检查所有会改变会话身份的路径最终都清空覆盖：
  - `/new`：`update::new_session` → `reset_session_view`
  - 删除当前会话：`on_session_deleted` → `reset_session_view`
  - `set_lx_entry` / `set_ai_web_site`：`pane.rs` 中的 reset 路径
  - AI Web 会话加载/切换：`web.rs` → `reset_session_view`
  - rewind 删除会话：`event/chat/rewind.rs` → `reset_session_view`

### 1.3 终端 Agent 内存标题覆盖

- [ ] `src/app/state/pane.rs`：
  - `Pane` 增加 `pub terminal_agent_title: Option<(AgentKind, String)>`。
  - `Pane::new()` 初始化为 `None`。
  - 新增 `Pane::agent_display_title(kind) -> Option<&str>`：
    - lx：覆盖优先，其次 `lx.chat.session_title`；
    - 终端：仅当覆盖的 `AgentKind` 与当前快照一致时使用，否则回落 `PaneAgentSnapshot.title`；
    - 空字符串按 `None` 处理。
  - 新增 `Pane::set_agent_title_override(kind, Option<String>)`：
    - lx 仅在 `has_active_session()` 时写入；
    - 终端仅在当前 `pane.agent.kind` 匹配时写入；
    - 空白值过滤为 `None`。
- [ ] `src/app/update/pane.rs`：`update_pane_agent` 在写入新快照前比较前后 `AgentKind`；`Some ↔ None` 或种类变化时清空 `terminal_agent_title`；同种类 state/标题更新保留覆盖。

### 1.4 状态层测试

新增 `src/app/state/pane/tests.rs`，在 `src/app/state/pane.rs` 增加 `#[cfg(test)] mod tests;`：

- [ ] lx 默认标题取 `session_title`。
- [ ] lx 覆盖优先于 `session_title`。
- [ ] lx 清除覆盖后回落 `session_title`。
- [ ] 终端默认标题取 OSC `title`，空字符串按 `None`。
- [ ] 终端覆盖只在同 `AgentKind` 下生效。
- [ ] `update_pane_agent` 在 `Claude → Codex`、`Claude → None` 时清空终端覆盖。
- [ ] `reset_session_view` 清空 lx 覆盖。
- [ ] `set_agent_title_override` 不修改 `session_title` 本身。

---

## P2. 菜单与更新逻辑（`src/app/`）

### 2.1 菜单模型

- [ ] `src/app/overlay.rs`：
  - `OverlayTarget` 增加 `Agent { workspace, tab, pane, kind }`。
  - `MenuCommand` 增加 `EditAgentTitle`、`CloseAgent`。
  - `MenuCommand::requires_confirm` 加入 `CloseAgent`。
  - `RenameTarget` 增加 `AgentTitle { workspace, tab, pane, kind }`。
  - 确认 `RenameTarget` 仍为 `Copy`。
- [ ] 所有对 `OverlayTarget` / `MenuCommand` / `RenameTarget` 的穷举 `match` 补齐新分支，避免编译遗漏。

### 2.2 打开菜单与执行

- [ ] `src/app/update/menu.rs` 新增 `open_agent_menu`：
  - 用 `state.agent_entries().get(index)` 防御越界；
  - 清空 `agent_scroll_drag`；
  - lx 恒提供 `EditAgentTitle + CloseAgent`；
  - 终端仅当 `tab.layout.pane_ids().len() > 1` 时提供 `CloseAgent`；
  - 菜单目标记录 `workspace/tab/pane/kind`。
- [ ] `activate_menu` 新增：
  - `EditAgentTitle` → 读取 `pane.agent_display_title(kind)` 预填 `Overlay::Rename`；
  - `CloseAgent` → 调用 `close_agent`。
- [ ] 新增私有 `close_agent`：
  - 执行前再次校验 `agent_entries()` 中仍存在完全匹配的 `(tab, pane, kind)`；
  - lx → `super::chat::new_session(state, pane)`；
  - 终端 → `super::workspace::close_pane(state, workspace, tab, pane)`；
  - 不新增 `Effect`，不写 DB。
- [ ] 确认 `new_session` 在生成中会先 `stop_stream`，关闭后 `session_id = None`、消息清空、DB 历史保留。

### 2.3 重命名提交

- [ ] `src/app/update/dialog.rs` 的 `commit_rename`：
  - 仅 `RenameTarget::AgentTitle` 允许空值；
  - 空值 = `set_agent_title_override(kind, None)` 并关闭浮层；
  - 非空 = `set_agent_title_override(kind, Some(name))`；
  - 其他目标保持“空值不保存、浮层保持打开”。
- [ ] 确认 `Overlay::Rename` 的输入框、光标、IME、Esc 取消逻辑无需改动。

### 2.4 菜单文案

- [ ] `src/ui/text/workspace.rs`：
  - `AGENT_NO_TITLE = "(no title)"`
  - `MENU_AGENT_EDIT_TITLE = "Edit title"`
  - `MENU_AGENT_CLOSE = "Close"`
- [ ] `src/ui/overlay/menu.rs`：
  - `menu_title` 增加 `OverlayTarget::Agent`，使用 `text::sidebar_agent_label(kind)`；
  - `menu_labels` 增加 `EditAgentTitle` 与 `CloseAgent`；
  - `CloseAgent` 在 `confirming` 时使用既有 `MENU_CONFIRM_CLOSE`。
- [ ] 确认所有用户可见文案均为英文，无中文泄漏。

### 2.5 菜单层测试

新增 `src/app/update/menu/tests.rs`，在 `src/app/update/menu.rs` 增加 `#[cfg(test)] mod tests;`：

- [ ] lx 条目菜单命令为 `[EditAgentTitle, CloseAgent]`。
- [ ] 终端条目多窗格时命令为 `[EditAgentTitle, CloseAgent]`。
- [ ] 终端条目唯一窗格时命令只有 `[EditAgentTitle]`。
- [ ] `EditAgentTitle` 打开 Rename 浮层并预填当前显示标题。
- [ ] `CloseAgent` 第一次 activate 只进入 `confirming`，第二次才执行。
- [ ] 切换菜单选中项后 `confirming` 复位。
- [ ] lx `CloseAgent` 后：`session_id=None`、消息清空、没有 `Effect::DeleteChatSession`。
- [ ] 终端 `CloseAgent` 后：目标窗格被移除。
- [ ] 菜单打开后目标条目消失，执行 `CloseAgent` 不误关其他 Agent。
- [ ] `commit_rename` 对 AgentTitle 空输入清除覆盖并关闭浮层。
- [ ] `commit_rename` 对工作区空输入仍保持浮层打开（回归）。

---

## P3. UI 渲染与命中（`src/ui/`）

### 3.1 文案与位置标签

- [ ] `src/ui/text/workspace.rs`：`agent_location(tab_index, pane_index)` 改为 `[T{n} P{m}]`，两者均 1 基；更新文档注释。
- [ ] 更新 `src/ui/text/tests.rs` 中所有 `agent_location` 调用与断言。

### 3.2 条目模型与两行渲染

- [ ] `src/ui/left_panel/mod.rs`：
  - `AgentListItem` 增加 `pane_index` 与 `title: Option<&'a str>`，带生命周期但保持 `Copy`。
  - `agent_items` 从 `entry.pane_index` 取序号，从 `state.pane_anywhere(pane).and_then(|p| p.agent_display_title(kind))` 取标题。
  - `agent_list_items` 第一行位置改用 `text::agent_location(item.tab_index, item.pane_index)`。
  - 每个 `ListItem` 生成两行：
    - 第一行保持 marker/dot/name/padding/location；
    - 第二行 `text::agent::TREE_LEAF` + `text::ellipsize_width(title, width - prefix.width())` + 补齐空格；
    - 标题与前缀 `style::muted().patch(highlight)`；无标题显示 `text::AGENT_NO_TITLE`。
  - 悬停样式继续作用于整个 `ListItem`，两行同时反显。
- [ ] 确认名称截断仍优先保留 `[T1 P1]`；第二行标题不会越界。

### 3.3 两行条目的滚动与命中

- [ ] `src/ui/left_panel/mod.rs` 新增 `agent_visible_items(list) -> usize = (height / 2).max(1)`。
- [ ] 新增 `agent_content_rect(list) -> Rect`：高度向下取偶，底部不足一个条目的行留空。
- [ ] `agent_list_rows` 返回可见**条目数**，更新注释与调用方语义。
- [ ] `agent_scrollbar` / `agent_scrollbar_for` 使用 `agent_content_rect` 与可见条目数。
- [ ] `render_agents` 在 `agent_content_rect(area)` 内渲染 `List`，滚动条也按该区域计算。
- [ ] `agent_item_index_at`：
  - 命中区域改为 `agent_content_rect(list)`；
  - `slot = agent_scroll + (row - content.y) / 2`；
  - 底部留空行不命中；
  - 滚动条列不命中。
- [ ] `agent_target_at` / `agent_item_at` 不需要改逻辑，自动获得两行命中。
- [ ] `agents_section_at` 保持整块区域判定（含底部留空行），滚轮仍可滚动。

### 3.4 UI 层测试

在 `src/ui/left_panel/tests.rs` 更新/新增：

- [ ] 更新所有 `text::agent_location(...)` 调用。
- [ ] `agent_item_shows_dot_name_and_right_aligned_location` 断言 `[T1 P1]`。
- [ ] 新增：同一窗格 lx + 终端两行显示相同 `[T1 P1]`。
- [ ] 新增：第二行以 `"  └─ "` 开头；无标题显示 `(no title)`。
- [ ] 新增：lx 默认标题取 `session_title`；覆盖后显示覆盖；清空后回落。
- [ ] 新增：终端 OSC 标题显示；覆盖后显示覆盖；`AgentKind` 变化后覆盖消失。
- [ ] 新增：宽字符标题按显示宽度截断，不越界。
- [ ] 新增：悬停时两行都有 selection 背景。
- [ ] 新增：点击第一行与第二行都命中同一条目。
- [ ] 新增：底部留空行不命中。
- [ ] 新增：高度为奇数时可见条目数按 `height / 2` 计算。
- [ ] 新增：窄侧栏名称截断但 `[T1 P1]` 保留。
- [ ] 更新原有“第二行/第二项”相关断言：现在第 2 个可见行是第一条目的标题行，不再假设一行一个条目。

---

## P4. 事件接入（`src/event/`）

- [ ] `src/event/overlay.rs` 的 `open_right_click_menu`：
  - 在 `workspace_list_blank_at` 之后、右栏/主区之前加入 `ui::agent_item_index_at` 判定；
  - 命中后调用 `update::open_agent_menu(state, index, (column, row))` 并 `return true`。
- [ ] 确认右键第二行同样命中（由 `agent_item_index_at` 保证）。
- [ ] 左键路径 `handle_sidebar_press` 不新增分支，继续使用 `ui::agent_target_at`。
- [ ] 悬停/滚轮路径不新增分支，继续使用 `ui::agent_item_index_at`。

### 事件层测试

新增 `src/event/tests/agent_menu.rs`，在 `src/event/tests.rs` 增加 `mod agent_menu;`：

- [ ] 右键 Agents 条目打开 `Overlay::Menu`，目标为 `OverlayTarget::Agent`。
- [ ] 右键第二行打开同一菜单。
- [ ] 右键 Agents 条目不会错误打开工作区/Tab/Pane 菜单。
- [ ] 终端 Agent 是唯一窗格时，右键菜单没有 `CloseAgent`。
- [ ] 左键第一行与第二行都能聚焦对应 Pane 与目标视图。
- [ ] 滚轮滚动后，悬停按新偏移命中正确条目。

---

## P5. 精确验证与收尾

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --lib app::state::pane::`
- [ ] `cargo test --lib app::update::menu::`
- [ ] `cargo test --lib ui::left_panel::`
- [ ] `cargo test --lib ui::text::`
- [ ] `cargo test --lib event::tests::agent_menu::`
- [ ] `cargo test --lib event::tests::mouse_click::`
- [ ] `cargo test --lib event::tests::mouse_drag::`
- [ ] 手动核对生产代码无 `unwrap()` / `expect()`；日志无 `println!` / `eprintln!`；UI 文案全英文。
- [ ] 记录实现与设计的偏差（如有）到本目录 `review.md`，等待用户复核。

---

## 验收清单

- [ ] 每个 Agents 条目固定两行；第二行为标题或 `(no title)`。
- [ ] 第一行显示 `[T1 P1]`，T/P 为 1 基位置；同窗格两行编号一致。
- [ ] 右键条目打开菜单：`Edit title` 恒有；可关闭条目有 `Close`。
- [ ] `Close` 行内二次确认；lx 只关会话视图，终端只关窗格；不删 DB。
- [ ] `Edit title` 复用居中 Rename 浮层；空输入清除覆盖。
- [ ] 所有标题修改只存在内存；重启丢失；会话/终端真实标题不变。
- [ ] 终端 Agent 是唯一窗格时不显示 `Close`。
- [ ] 两行条目的左键、右键、悬停、滚轮、滚动条、窄侧栏全部正确。
- [ ] 受影响的 fmt / clippy / 单元测试全部通过。

---

## P6. V2 补充：Recent 标题与 Open prompt

> 依据 [design.md](design.md) 第 12 节。标题不再跟随会话/终端标题。

### 6.1 状态模型

- [ ] `src/app/state/pane.rs`：新增 `AgentPromptRef { item_id, page_id }` 与 `AgentTitleOverride { text, prompt }`。
- [ ] `LxChat.agent_title_override`、`Pane.terminal_agent_title` 改为保存 `AgentTitleOverride`。
- [ ] `agent_display_title` 只读覆盖文本；`session_title` / OSC `title` 不再参与。
- [ ] `set_agent_title_override` 保持空白过滤与 AgentKind / active session 校验。
- [ ] 会话重置/切换、终端 Agent 退出/种类变化、窗格关闭继续清空覆盖。

### 6.2 标题弹窗

- [ ] 新增 `AgentTitleDialog`、`OverlayKind::AgentTitle`、`Overlay::AgentTitle`。
- [ ] `Edit title` 不再打开 `Overlay::Rename`；打开新弹窗并快照目标工作区 Recent 页。
- [ ] 输入框预填当前标题；已绑定时高亮对应 Recent 行。
- [ ] ↑/↓ 浏览 Recent：回填页标题并绑定 `(item_id, page_id)`。
- [ ] 直接编辑输入框：转为自定义标题并解除绑定与高亮。
- [ ] Enter 提交；空输入清除覆盖；Esc 取消。
- [ ] 底部 Save/Clear/Cancel 按钮、点击行、点击外部行为与键盘一致。
- [ ] 弹窗显示 Recent 两行（页标题 + 提示词名）、空态与 `recent` 分割线。
- [ ] 删除 `RenameTarget::AgentTitle` 与 `commit_rename` 的 AgentTitle 分支。

### 6.3 Open prompt

- [ ] `open_agent_menu` 在绑定有效时于 `Edit title` 后插入 `OpenPrompt`。
- [ ] `OpenPrompt` 对 Agent 目标调用 `open_agent_prompt`：释放 Prompt 钉住、结算当前编辑、打开精确 item/page、聚焦 Prompt 面板。
- [ ] 自定义标题、页已删除、绑定失效时不显示 `OpenPrompt`。
- [ ] 菜单打开后页被删除，执行 `OpenPrompt` 不得误切换。

### 6.4 测试

- [ ] `src/app/state/pane/tests.rs`：标题来源、绑定、清除、生命周期。
- [ ] `src/app/update/menu/tests.rs`：菜单命令集合、弹窗预填/选择/编辑/提交、Open prompt 打开精确页、页删除保护。
- [ ] `src/ui/left_panel/tests.rs`：会话/OSC 标题不再显示；自定义/绑定显示；CJK 截断。
- [ ] `src/ui/overlay/agent_title/tests.rs`：几何、命中、滚动、渲染、空态、按钮、窄屏。
- [ ] `src/input/tests.rs`：AgentTitle 键位映射。
- [ ] `src/event/tests/agent_menu.rs`：右键 → Edit title → 弹窗、点击行/Save/Clear/Cancel/外部、Open prompt。
- [ ] `src/ui/text/tests.rs`：新文案与 `agent_location` 保持稳定。

### 6.5 验证

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --lib`
- [ ] 更新 `review.md` 记录 V2 实现与偏差。
