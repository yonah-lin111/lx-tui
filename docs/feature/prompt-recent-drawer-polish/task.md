# Prompt Recent 抽屉高度、选择行为与条目布局调整实施任务

- 状态：**已复审并修复（待合并）**
- 关联设计：`docs/feature/prompt-recent-drawer-polish/design.md`
- 复审报告：`docs/feature/prompt-recent-drawer-polish/review.md`
- 日期：2026-11-03

---

## 1. 需求与边界确认（Grill Me 已完成）

- [x] **问题 1（抽屉高度）**：取消“最多 3 条”硬上限，按 Prompt 内容区半屏能放多少条就显示多少条；至少 2 条、最多 20 条，总高度（含分割线）不超过内容区高度的一半。
- [x] **问题 2（选择后关闭）**：鼠标单击与键盘 `Enter` 都关闭抽屉；选中当前正在编辑的页也关闭，只收起、不重新加载正文。
- [x] **问题 3（条目布局）**：第 1 行改为页标题 + 右对齐时间；第 2 行改为提示词条目名；移除右端状态圆点；页标题按状态着色（todo 默认亮白、in_progress 黄、done 绿、无 `&&&` 暗灰）；当前页保留 `❯` 并加粗页标题；条目名恒为 muted。
- [x] **问题 4（半屏基准）**：以 Prompt 内容区 `prompt_body_rect` 为基准（已扣除顶部工具栏两行），抽屉总高度（含分割线）不超过内容区高度的一半。
- [x] **问题 5（排序）**：仅打开页面、不修改正文时，不刷新 `page.updated_at`，recent 列表顺序保持不变。

> `ref-content/ui/prompt/` 在当前工作区不存在，以上按现有 `src/ui/prompt/recent.*` 实现确认。

---

## 2. 实施步骤拆解

### Task 1：布局层改为半屏动态容量

**文件：`src/layout/prompt.rs`**

- [x] **1.1** 重写 `prompt_body_split` 的高度计算：
  - `max_height = body.height / 2`（含分割线）；
  - `recent_count` 先 `min(MAX_RECENT_PAGES)`；
  - 空态：`drawer_height = 2`，`max_height < 2` 时收起；
  - 非空：`capacity = (max_height - 1) / 2`；
  - `capacity < RECENT_MIN_VISIBLE_ENTRIES(2)` 时收起；
  - `visible = min(max(recent_count, 2), capacity)`；
  - `drawer_height = 1 + 2 * visible`。
- [x] **1.2** 把现有三处重复的“收起”返回抽成局部辅助函数（如 `fn suppressed_split(body: Rect) -> PromptBodySplit`），保持原字段语义不变。
- [x] **1.3** 保留 `PromptBodySplit` 字段与 `visible_entries` 语义；`visible_entries == 0` 仍表示空态行。
- [x] **1.4** 更新函数文档注释：
  - 删除“最多 3 条（7 行）”描述；
  - 写明“半屏容量、保底 2 条、最多 20 条、放不下最低 2 条时收起”。

**关键代码：**

```rust
let max_height = body.height / 2;
let recent_count = recent_count.min(crate::app::prompt::MAX_RECENT_PAGES);
let (visible, drawer_height) = if recent_count == 0 {
    (0, 2)
} else {
    let capacity = usize::from(max_height.saturating_sub(1) / 2);
    if capacity < crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES {
        return suppressed_split(body);
    }
    let visible = recent_count
        .max(crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES)
        .min(capacity);
    (visible, 1 + 2 * visible as u16)
};
if drawer_height > max_height {
    return suppressed_split(body);
}
```

---

### Task 2：Recent 可见条目数状态缓存

**文件：`src/app/prompt/tree/panel.rs`、`src/app/update/pane.rs`、`src/app/update/prompt/recent.rs`**

- [x] **2.1** `PromptPanel` 新增字段：

  ```rust
  /// 抽屉当前可见条目数；由 resize_panes 按几何写入，抽屉收起时为 0。
  /// 只供更新层判断选中窗口，渲染层不读取。
  pub recent_visible: usize,
  ```

  `PromptPanel::new()` 初始化为 `0`。

- [x] **2.2** `resize_panes` 在计算 `split` 后写入：

  ```rust
  let split = layout::prompt_body_split(*rect, recent_open, recent_count);
  // ...
  state.prompt.resize(cols, rows);
  let workspace = state.prompt_workspace();
  if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
      workspace_state.prompt_panel.recent_visible = split.visible_entries;
  }
  ```

- [x] **2.3** `close_prompt_recent` 把 `recent_visible` 清 0。
- [x] **2.4** `sync_prompt_recent_selection` 使用真实可见数：

  ```rust
  let visible = panel.recent_visible.max(1);
  panel.recent_active = Some(next);
  panel.recent_scroll = keep_visible(panel.recent_scroll, next, visible, rows.len());
  ```

- [x] **2.5** 删除 `RECENT_VISIBLE_ENTRIES`：
  - `src/app/prompt/recent.rs` 删除常量；
  - `src/app/prompt.rs` 删除再导出；
  - 全局搜索确认无残留引用。

---

### Task 3：选择条目后关闭抽屉

**文件：`src/app/update/prompt/recent.rs`、`src/event/mouse/press.rs`**

- [x] **3.1** `open_recent_page` 的“当前页”分支改为：

  ```rust
  if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
      workspace_state.prompt_panel.mode = PromptMode::Editor;
  }
  state.prompt_recent_hover = None;
  focus_prompt(state);
  close_prompt_recent(state);
  return true;
  ```

  页列表视图下必须切回编辑视图；鼠标从窗格焦点打开抽屉时，关闭后焦点交还 prompt。

- [x] **3.2** `open_recent_page` 正常切换分支末尾：
  - 删除重新定位 `recent_active` / `recent_scroll` 的代码块；
  - 改为 `state.prompt_recent_hover = None; close_prompt_recent(state);`。
- [x] **3.3** 更新 `open_recent_page` 文档注释：从“保持抽屉展开”改为“打开后收起抽屉”。
- [x] **3.4** 更新 `src/event/mouse/press.rs` 条目点击分支注释：
  - “抽屉保持展开” → “打开后收起抽屉”。
- [x] **3.5** 确认 `apply_prompt_recent_key` 的 `Enter` 路径无需改动（`open_active_recent_page` 已复用 `open_recent_page`）。

**关键代码：**

```rust
/// 单击抽屉条目：结算当前编辑，切换对应条目 + 页，随后收起抽屉；
/// 选中当前页只收起抽屉并把焦点/视图交还编辑器。
pub fn open_recent_page(state: &mut AppState, index: usize) -> bool {
    // ... 前略：取目标、当前页快速路径、提交当前编辑 ...
    if 已经是当前页 {
        // 页列表视图下仍须回到编辑视图。
        if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
            workspace_state.prompt_panel.mode = PromptMode::Editor;
        }
        state.prompt_recent_hover = None;
        focus_prompt(state);
        close_prompt_recent(state);
        return true;
    }
    // ... 中略：切换 active_page / mode / editing / load_editor_target / focus_prompt ...
    state.prompt_recent_hover = None;
    close_prompt_recent(state);
    true
}
```

---

### Task 4：Recent 条目布局与状态色

**文件：`src/ui/prompt/recent.rs`**

- [x] **4.1** 重写 `paint_entry`：
  - 第 1 行：`❯` / 空格 + `page_title` + 右对齐 `time_label`；
  - 页标题样式按 `row.status`：
    - `Todo` → `style::text()`；
    - `InProgress` / `Done` → `style::template_status(status)`；
    - `None` → `style::muted()`；
  - `row.current` 时页标题追加 `style::strong()`；
  - 第 2 行：2 列缩进 + `prompt_name`，样式 `style::muted()`；
  - 悬停 / 键盘选中时两行都保留 `style::selection()` 底色。
- [x] **4.2** 删除右端状态圆点绘制分支（`PAGE_STATUS_OPEN` / `PAGE_STATUS_DOT` 的 recent 用法），不删除 `src/ui/text.rs` 中的常量。
- [x] **4.3** 保持窄屏规则：
  - 页标题至少保留 4 列，否则不画时间；
  - 两行超宽均由 `tree::paint_name` 截断并追加 `…`。
- [x] **4.4** 更新文件头注释：从“双行条目、状态圆点”改为“页标题 + 条目名双行条目、状态色”。

**关键代码：**

```rust
let title_style = match row.status {
    Some(TemplateStatus::Todo) => style::text(),
    Some(status) => style::template_status(status),
    None => style::muted(),
};
let title_style = if row.current {
    title_style.patch(style::strong())
} else {
    title_style
};
tree::paint_name(
    buf,
    x,
    y,
    &row.page_title,
    base.patch(title_style),
    usize::from(title_right.saturating_sub(x)),
    title_right,
);

let name_x = tree::paint_text(buf, area.x, y + 1, "  ", base, area.right());
tree::paint_name(
    buf,
    name_x,
    y + 1,
    &row.prompt_name,
    style::muted().patch(highlight),
    usize::from(area.right().saturating_sub(name_x)),
    area.right(),
);
```

---

### Task 5：测试更新与新增

#### 5.1 `src/layout/tests.rs`

- [x] 更新 `prompt_body_split_reserves_drawer_rows` 注释：3 条来自半屏容量，不是固定常量。
- [x] 新增 `prompt_body_split_grows_to_half_height`：
  - 面板 40×30，`recent_count = 10`；
  - 断言 `visible_entries == 6`、`recent_text.height == 12`、`main.height == 13`。
- [x] 新增 `prompt_body_split_caps_at_twenty_entries`：
  - 面板足够高，`recent_count = 25`；
  - 断言 `visible_entries == 20`。
- [x] 保留并确认矮屏收起测试仍通过。

#### 5.2 `src/ui/prompt/recent/tests.rs`

- [x] 改写 `renders_two_line_entries_with_time_and_status_dot`：
  - 改名为 `renders_page_title_then_prompt_name_without_dot`；
  - 第 1 行包含页标题与时间，第 2 行包含条目名；
  - 断言两行均不含 `●` / `○`。
- [x] 新增 `page_title_uses_status_colors`：
  - `Todo` → 默认前景、无 `DIM`；
  - `InProgress` → `Color::Yellow`；
  - `Done` → `Color::Green`；
  - `None` → `DIM`。
- [x] 新增 `current_page_title_is_bold`。
- [x] 保留悬停 / 键盘选中双行底色、关闭按钮、空态、滚动窗口测试。

#### 5.3 `src/app/update/prompt/recent/tests.rs`

- [x] 改写 `open_recent_page_commits_current_and_switches_target`：
  - 断言抽屉关闭、`recent_active == None`、`recent_scroll == 0`、`recent_visible == 0`；
  - 保留旧页正文已提交的断言。
- [x] 新增 `selecting_current_page_closes_without_reload`：
  - 断言正文、光标、修订号不变；焦点从窗格回到 prompt。
- [x] 新增 `selecting_current_page_from_pages_returns_to_editor`：
  - 页列表视图 + 当前页，断言回到 `PromptMode::Editor`、抽屉关闭、正文不变、焦点回到 prompt。
- [x] 新增 `opening_page_without_edit_keeps_recent_order`：
  - Alpha `updated_at = 100`、Beta `updated_at = 200`；
  - 打开 Beta 不修改；行顺序仍为 Beta、Alpha，且两者 `updated_at` 均不变。
- [x] 新增 `sync_uses_cached_visible_entries`：
  - `recent_visible = 2`、3 条记录、目标第 3 条，断言 `recent_scroll == 1`。
- [x] 保留滚动钳制、`recent_active` 越界修正测试。

#### 5.4 `src/event/tests/prompt_recent.rs`

- [x] `clicking_recent_entry_opens_page_and_keeps_drawer` 改名为 `clicking_recent_entry_opens_page_and_closes_drawer`，断言关闭。
- [x] `clicking_recent_entry_opens_from_tree_and_pages` 增加关闭断言。
- [x] `keyboard_defaults_to_current_page_and_navigates_to_open` 改为 `Enter` 后断言关闭，并删除后续对 `Esc` 收起抽屉的依赖。
- [x] 新增 `escape_closes_drawer_without_changing_text`，补回 `Esc` 关闭抽屉的覆盖。
- [x] 调整滚动相关测试几何为 140×20：
  - `recent_scrollbar_supports_track_click_and_thumb_drag`；
  - `keyboard_navigation_keeps_selection_visible`；
  - `wheel_over_drawer_scrolls_entries_without_touching_editor`。
- [x] 保留矮屏压制、悬停、右键吞掉、删除条目钳制测试。

---

## 3. 验证命令

按影响范围执行：

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings

cargo test layout::tests::prompt_body_split
cargo test ui::prompt::recent
cargo test app::update::prompt::recent
cargo test event::tests::prompt_recent

cargo test
```

- [x] 以上命令全部通过。
- [x] 不运行全量格式化以外的大范围重构；不触碰未相关文件。

---

## 4. 验收清单

- [x] 大面板下 recent 抽屉可超过 3 条，最多到半屏容量。
- [x] 抽屉总高度（含分割线）≤ Prompt 内容区一半。
- [x] 记录不足 2 条仍保留 2 条高度；矮屏放不下 2 条时自动收起且不吞键。
- [x] 鼠标单击任意 recent 条目后抽屉关闭。
- [x] 键盘 `Enter` 打开后抽屉关闭。
- [x] 点击当前正在编辑的页只关闭抽屉，不重载正文。
- [x] 仅打开、不修改正文时 recent 顺序不变。
- [x] 第 1 行为页标题 + 时间，第 2 行为条目名；不再出现状态圆点。
- [x] 页标题颜色：todo 亮白、in_progress 黄、done 绿、无状态暗灰。
- [x] 当前页保留 `❯` 且页标题加粗。
- [x] 页列表视图（Pages）的状态圆点保持不变。
- [x] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 通过。

---

## 5. 风险与回滚

- **风险 1：布局与渲染行数不一致**：`resize_panes` 使用 `recent_page_count`，渲染使用 `recent_rows.len()`；本次在 `prompt_body_split` 内统一 `min(MAX_RECENT_PAGES)` 解决。若后续新增行来源，必须同步该截断。
- **风险 2：关闭后编辑器视口未恢复**：`close_prompt_recent` 已设置 `prompt_resize_pending`；不要绕过它直接改 `recent_open`。
- **风险 3：状态色被选中高亮覆盖**：使用 `base.patch(title_style)` 的顺序，确保黄 / 绿前景优先；测试覆盖悬停场景。
- **风险 4：误改页列表圆点**：本次只改 `src/ui/prompt/recent.rs`；`src/ui/prompt/pages.rs` 与 `PAGE_STATUS_*` 常量保持不动。
- **回滚**：本改动不涉及数据库与持久化，回滚只需还原上述文件；无数据迁移风险。

---

## 6. 实现记录

- 2026-11-03：完成半屏动态容量、选择后关闭抽屉、页标题/条目名换位与状态色、移除状态圆点、`recent_visible` 可见窗口同步。
- 2026-11-03（复审修复）：
  - P1-1：页列表视图选中当前页后切回编辑视图；
  - P2-1：当前页快速路径恢复 prompt 键盘焦点；
  - P2-2：补充 Pages 当前页、焦点/光标/修订号、目标页 `updated_at`、动态可见窗口、`Esc`、悬停状态色测试；
  - P3-1/P3-2：同步 `recent_page_count` 注释、设计/任务文档与测试名。
- 验证结果：
  - `cargo fmt --check` 通过；
  - `cargo clippy --all-targets -- -D warnings` 通过；
  - `cargo test` 全量通过（含新增布局、渲染、更新层与事件层测试）。
