# Prompt Recent 抽屉复审报告

- 复审对象：`03c35d7f feat(prompt): grow recent drawer to half height and close on select`
- 复审范围：Recent 抽屉布局、选择/关闭、条目渲染、可见窗口缓存、相关测试与设计文档
- 结论：无 P0；发现 P1 1 项、P2 2 项、P3 2 项，均已修复并补测试。
- 合并状态：已随 `aa264643` 合并到 `dev`。
- 关联设计：`docs/feature/prompt-recent-drawer-polish/design.md`
- 关联任务：`docs/feature/prompt-recent-drawer-polish/task.md`

---

## P0（阻断 / 数据丢失 / 安全问题）

无。布局计算、抽屉关闭与页面切换均未发现 panic、越界或正文丢失路径。

---

## P1（功能错误，用户可稳定复现）

### P1-1 页列表视图选中“当前页”后仍停留在页列表

**关键点**

- 从 `PromptMode::Pages` 打开 recent 抽屉。
- 点击当前正在编辑的页（`editing + item.active_page` 与目标一致）。
- `open_recent_page` 走“当前页”快速路径，只关闭抽屉并返回。

**错误原因**

快速路径只判断“目标是否等于当前编辑目标”，没有判断当前视图模式。该路径绕过了正常切换分支里的
`panel.mode = PromptMode::Editor`，因此抽屉关闭后画面仍停在页列表，与“选择条目打开页面”的行为不一致。

**修复**

快速路径补齐模式归一化：

```rust
if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
    workspace_state.prompt_panel.mode = PromptMode::Editor;
}
```

**验证**

- 新增 `selecting_current_page_from_pages_returns_to_editor`：页列表视图 + 当前页，断言回到
  `PromptMode::Editor`、抽屉关闭、正文不变。
- 修复前该测试失败（`left: Pages, right: Editor`），修复后通过。

---

## P2（体验缺陷或重要测试缺口）

### P2-1 当前页快速路径不恢复键盘焦点

**关键点**

- 通过鼠标点击 `[rec]` 按钮打开抽屉时，键盘焦点可能仍在窗格（`FocusedContainer::Panes`）。
- 点击当前页条目走快速路径，只关闭抽屉，不经过 `load_editor_target`。
- 正常条目切换路径会经 `load_editor_target` 调用 `focus_prompt`，快速路径不会。

**错误原因**

快速路径为了“不重载正文”而完全绕过了正常切换流程，但把“恢复焦点”这个必要副作用也一起绕过了。
结果是界面回到编辑器，键盘输入却仍发给终端窗格。

**修复**

快速路径关闭抽屉前调用 `focus_prompt(state)`，只恢复焦点，不触碰正文、光标或修订号。

**验证**

- `selecting_current_page_closes_without_reload` 增加焦点断言与光标/修订号断言。
- 修复前焦点断言失败，修复后通过。

### P2-2 动态可见窗口与旁路行为缺少测试锁定

**关键点**

- `sync_prompt_recent_selection` 从固定 3 条改为 `recent_visible`，但原测试只覆盖滚动钳制，没有验证
  “实际可见 2 条时选中第 3 条应滚到偏移 1”。
- `Enter` 改为打开后关闭，原 keyboard 测试删除了后续 `Esc` 段落；名为
  `close_button_and_escape_restore_previous_view` 的用例实际只点击关闭按钮，没有按 `Esc`，
  导致 `Esc` 关闭抽屉失去覆盖。
- “仅打开不修改不重排”只断言了来源页 `updated_at`，没有断言目标页也没有被刷新。
- 悬停状态色只测了 `Done`，设计文档却声称覆盖 `InProgress / Done`。

**错误原因**

实现改动集中在主路径，测试同步时只改了直接受影响的断言，没有补齐被删除/新增的旁路覆盖；
动态缓存值没有行为级断言，回退到固定常量不会被发现。

**修复**

- 新增 `sync_uses_cached_visible_entries`：`recent_visible = 2`、3 条记录、选中第 3 条，断言
  `recent_scroll == 1`。
- 新增 `escape_closes_drawer_without_changing_text`。
- `opening_page_without_edit_keeps_recent_order` 增加目标页 `updated_at` 不变断言。
- `status_color_wins_over_hover_foreground` 扩展为 `InProgress` + `Done` 两种状态。
- `toggle_reduces_editor_viewport_after_resize` 增加 `recent_visible` 写入/清零与视口恢复断言。

**验证**

相关测试全部通过；新增用例见 `cargo test --lib app::update::prompt::recent::tests` 与
`cargo test --lib event::tests::prompt_recent`。

---

## P3（可维护性 / 文档一致性）

### P3-1 `recent_page_count` 注释仍写死“前 3 条”

**关键点**

`src/app/prompt/recent.rs` 的 `recent_page_count` 文档仍写着“抽屉高度只关心前 3 条”，
但高度上限已改为 `MAX_RECENT_PAGES = 20`。

**错误原因**

删除 `RECENT_VISIBLE_ENTRIES` 时只改了常量与布局，没有同步派生模型的注释。

**修复**

注释改为“已入库页总数；抽屉几何会按 `MAX_RECENT_PAGES` 截断后再计算高度”。

### P3-2 设计与任务文档漂移

**关键点**

- `design.md` 的关键代码仍保留旧结构（`let main` 在提前返回之前、没有 `suppressed_split`）。
- 设计中的测试名 `renders_two_line_entries_with_page_title_and_time` 与实际
  `renders_page_title_then_prompt_name_without_dot` 不一致。
- “打开 Alpha”与“键盘用例仍提到 `Esc`”等描述与最终测试不符。
- `prompt_body_split` 文档写“至少 2 条”，对空态（0 条、固定 2 行）不准确。

**错误原因**

实现与测试在评审过程中有调整，文档只更新了结论，没有回写关键代码、测试名和旁路行为。

**修复**

- 更新 `design.md` / `task.md` 的代码片段、测试名、行为说明与验收状态。
- 明确空态：非空记录至少 2 条，空态固定 2 行。
- 在 `task.md` 增加本轮复审修复记录。

**验证**

文档与当前实现、测试名逐项核对一致。

---

## 修复后的验证（全部通过）

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --lib app::update::prompt::recent::tests`
- `cargo test --lib event::tests::prompt_recent`
- `cargo test --lib ui::prompt::recent`
- `cargo test --lib layout::tests::prompt_body_split`
- `cargo test`

---

## 合并记录

- 2026-11-03：以 `--no-ff` 合并到 `dev`，合并提交 `aa264643`；无冲突。
- 合并后验证：`cargo test --lib app::update::prompt::recent::tests` 通过，`cargo test` 全量通过。
- 清理：`feat/prompt-recent-drawer-polish` 分支已删除，`.worktrees/prompt-recent-drawer-polish` 工作区已移除，
  `~/.lx-t-dev/db/worktrees/prompt-recent-drawer-polish-5e704e45` 隔离库已删除。
