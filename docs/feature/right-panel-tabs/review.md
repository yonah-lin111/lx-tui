# 右侧统一容器与 Prompt / Todo 标签页 — 评审记录

- 评审对象：`feat/right-panel-tabs`（`846685b` 功能提交 + `2dcc75e` 修正提交 + 本轮修复）
- 评审方式：三个 reviewer 并行（状态+事件 / 渲染+布局 / 测试+文档），互不重叠
- 记录口径：**只收会导致错误行为、panic、数据丢失、交互错乱、假阳性测试的问题**；
  风格偏好与"建议加注释"一律不收
- 验证方式：每条修复都做**变异验证**——把被测代码改回去，确认用例真的转红。
  没转红的一律如实标注，不当成"已修复"

---

## 0. 结论速览

| 严重度 | 数量 | 状态 |
| --- | --- | --- |
| 高（真缺陷 / 假阳性测试 / 文档说谎） | 5 | 全部修复并锁回归 |
| 中（覆盖缺口 / 阈值错位） | 6 | 全部修复或明确记录为冗余防线 |
| 低（潜在 / 理论不可达） | 3 | 全部加固 |

**最重要的一条**：`select_right_tab` 的"数据不丢失"和"清瞬时状态"原先各有一条**空转测试**
（断言的是"本来就不存在的东西"），以及一个**真实缺陷**——异步 worktree 回调会在 Todo 期间
把刚收起的提及面板重新点亮。

---

## 1. 高：`select_right_tab` 的两条测试是空转

### 1.1 `select_right_tab_preserves_prompt_content` 的"滚动保留"是 `0 == 0`

**位置**：`src/app/update/tests.rs`

**关键代码（修改前）**：

```rust
state.prompt.resize(40, 10);            // 视口 10 行
state.prompt.insert_str("\nsecond line"); // 正文 2 行
set_prompt_scroll(&mut state, 1);
// ...
assert_eq!(state.prompt.scroll(), scroll, "滚动位置原样保留");
```

**错误原因**：正文 2 行 + 底部虚拟留白 2 行 − 视口 10 行 ⇒ 可滚动上限为 0，
`set_prompt_scroll` 直接返回 `false`，`scroll` 恒为 0。这条断言实际在验证 `0 == 0`。
实测：

```text
set_prompt_scroll -> false, scroll = 0
visual_rows = 2
bottom_padding = 2
```

同理 `item.pages.push(Page 2)` 之后从未切到第 2 页，也没断言 `active_page`——
设计 §6.4 写的"第二页逐字段相等"根本没兑现。

**修复**：视口压到 6 行、正文灌到溢出、断言 `scroll > 0`；真的切到 `Page 2`，
往返后校验 `active_page` 与页名。

**变异验证**：删掉 `select_right_tab` 里对 `right_tab` 的赋值后用例转红。

---

### 1.2 `select_right_tab_commits_rename_and_clears_transient_state` 有 5 条断言是空转

**位置**：`src/app/update/tests.rs`

**关键代码（修改前）**：

```rust
// 测试只打开了 mention 面板
assert!(state.prompt.panel().is_none(), "块命令面板收起");
assert!(state.prompt.slash_panel().is_none());
assert!(state.prompt.mention().is_none());
assert!(state.prompt.refer_panel().is_none());
assert!(state.prompt.paste_panel().is_none());
assert_eq!(state.right_tab_hover, None, "tab 悬停清空");
```

**错误原因**：五种输入面板（块 `#` / 斜杠 `/` / 提及 `@` / 引用 / 粘贴）**互斥**，
同一状态只能打开一个。测试只制造了 mention，其余四个从头到尾没被打开过，
`right_tab_hover` 也一直是 `None`——删掉 `select_right_tab` 里对应的清理代码，
这 5 条断言照样通过。它们证明的是"这些东西本来就不存在"。

实测五种面板可独立构造：

```text
hash:    block=true  slash=false mention=false refer=false paste=false
slash:   block=false slash=true  mention=false refer=false paste=false
refer:   block=false slash=false mention=false refer=true  paste=false
mention: block=false slash=false mention=true  refer=false paste=false
paste:   block=false slash=false mention=false refer=false paste=true
```

**修复**：新增 `select_right_tab_clears_each_transient_panel`，逐类单独构造、
单独断言"打开 → 切 Tab → 收起"；原用例保留 mention 路径并补 `right_tab_hover`
前置。

**变异验证**：逐行移除清理代码，用例分别转红：

| 移除的代码 | 结果 |
| --- | --- |
| `state.prompt.clear_panel()` | 2 failed |
| `state.prompt_tree_hover = None` | 1 failed |
| `clear_selection(state)` | 1 failed |

---

## 2. 高：`paint_tabs` 的样式继承缺陷（真 bug）

**位置**：`src/ui/right_panel.rs::paint_tabs`

**关键代码（修改前）**：

```rust
let tab_style = if tab == active {
    style::selected_item()
} else if hover == Some(tab) {
    style::selection()
} else {
    style::muted()
};
buf.set_stringn(rect.x, rect.y, &text, usize::from(rect.width), tab_style);
```

**错误原因**：`set_stringn` 是 **patch 语义**——只覆盖样式里显式给出的字段。
`style::muted()` 只带 `DIM`、不带前景；`selection()` 只带中灰底。
tab 画在容器顶边框上，而聚焦态边框是 `style::accent()`（Cyan + BOLD），
于是非激活 tab 会继承 `fg=Cyan, BOLD`，**看起来两个 tab 都在发亮**。
实测聚焦态：

```text
Todo tab: bg=Reset fg=Cyan modifier=BOLD|DIM
```

**修复**：每格先 `buf.set_style(rect, Style::reset())` 再套 token。
这是项目里已有先例（`style::muted_on_border()` 的文档注释就在讲同一个坑）。

**回归用例**：`tabs_render_on_top_border_with_accent_active` 的聚焦段。

---

## 3. 高：点"当前 tab"既不提交页重命名，也无条件置脏

**位置**：`src/event/mouse/press.rs`

**关键代码（修改前）**：

```rust
} else if !state.prompt_collapsed
    && let Some(tab) = ui::right_panel::tab_at(view.prompt, mouse.column, mouse.row)
{
    update::select_right_tab(state, tab);
    *dirty = true;
}
```

**错误原因**：`select_right_tab` 开头对"同一 tab"早退：

```rust
if state.right_tab == tab {
    return false;
}
finish_page_rename(state);   // ← 永远走不到
```

所以页重命名输入中点击**已激活**的 tab，既不提交也不退出输入态，输入框僵在原处；
而点 Todo 再点回 Prompt 却会提交——行为不一致。另外 `*dirty = true` 无条件执行，
同 tab 点击白白重绘一次。

**修复**：

```rust
let was_renaming = update::page_rename_active(state);
let mut changed = update::finish_page_rename(state) || was_renaming;
changed |= update::select_right_tab(state, tab);
if changed {
    *dirty = true;
}
```

`was_renaming` 这一项是必要的：输入**空名**提交时 `finish_page_rename` 返回 `false`
（空名保留旧名），但它确实结束了输入态，必须置脏。

**变异验证**：回退成 `select_right_tab(...); *dirty = true;` 后
`mouse_click_current_tab_commits_page_rename` 与
`mouse_click_current_tab_without_changes_is_not_dirty` 双双转红。

---

## 4. 高：异步 worktree 回调会复活隐藏的提及面板（真 bug）

**位置**：`src/event/mod.rs::AppEvent::PromptWorktreeListed`、`pump_prompt_worktree`

**关键代码**：

```rust
// 事件分支：只判 targeted，不判面板是否可交互
if targeted {
    if failed {
        state.prompt.worktree_failed();
    } else if state.prompt.apply_worktree_entries(candidates) {
        state.prompt.rebuild_worktree_entries();
    }
}
```

```rust
// mention.rs：clear() 只 bump generation，不清 cache
pub fn clear(&mut self) {
    self.panel = None;
    self.chooser = None;
    ...
    self.generation = self.generation.wrapping_add(1);
}
```

```rust
// apply_worktree_entries 结尾无条件重算
pub fn apply_worktree_entries(&mut self, candidates, text, cursor) {
    self.worktree_pending = false;
    if self.worktree_candidates.as_deref() != Some(candidates.as_slice()) {
        self.cache = None;   // 只有候选**变化**时才清 cache
        ...
    }
    self.refresh(text, cursor);   // ← 无条件，refresh 末尾 self.panel = Some(...)
}
```

**错误原因**：`select_right_tab` 切走时用 `clear_panel()` 收回提及面板，
但 `clear()` **保留 cache**。只要 worktree 回调带回**相同**候选（cache 不失效），
`refresh` 仍能命中缓存并把 `panel` 重新置 `Some`——一个看不见、却仍能吞滚轮的面板。

实测：

```text
准备完成: panel=true
切到 Todo: panel=false
相同候选回调后: panel=true     ← 复活
right_tab = Todo
```

**触发条件**：git 工作区 + 光标处留着 `@` 触发词（cache 已加载）+ 切到 Todo
+ 任一次 worktree 结果到达（首轮加载、`{wt:}` 绑定 Missing/Ambiguous 的 2s 重试、
`list_worktrees` 失败后的每次重试）。

**后果**：
- `wheel.rs` 的提及面板分支先于 prompt 分支命中，滚轮被吞 + 置脏（滚动一个看不见的列表）；
- 切回 Prompt 时面板凭空出现，破坏设计 §5.1 的"瞬时浮层切走即收回"；
- `list_worktrees` 持续失败时，Todo 下每次鼠标移动都重新发起一次阻塞 git 查询。

**修复**：`PromptWorktreeListed` 分支与 `pump_prompt_worktree` 都改为
`if !update::prompt_panel_active(state) { return; }`。

**回归用例**：`worktree_candidates_do_not_resurrect_mention_panel_on_todo`。

**变异验证（重要）**：这条用例**第一次写成了假阳性**。删掉闸门后测试仍然通过——
因为 `targeted` 检查要求工作区有匹配的 `repo_root`，而 `AppState::demo()` 的
工作区没有 git 元数据，回调从一开始就被丢弃了。补上 `WorkspaceGit` 前置后，
删闸门即转红。

---

## 5. 高：标准文档里的成句谎言 —— 底边框 `ws:路径末段名`

**位置**：`docs/standards/tui-design-requirements.md:25`（prompt 右栏整段）

**文档原文**：

> 底边框**右侧**（折叠按钮左隔 1 列）为 `ws:路径末段名`（优先 Prompt 显式绑定根路径，
> 回退 checkout 路径与工作区 cwd，无路径时不显示…）

**代码事实**：`src/ui/right_panel.rs::render_bottom` 的身份标签**左对齐**贴左边框后 2 列，
内容是 `项目 · b:分支 · wt:工作区名`；`prompt_identity` 明确**忽略** `prompt_root`；
全文没有任何 `ws:` 段。更糟的是同一句后半段又写"底边框（属于容器…）**左侧**嵌入
激活工作区的 git 状态"——**一句话里两个互相矛盾的底边框描述**。

**为什么没被发现**：这是 `dev` 就有的旧谎言（`7b0f784` 把 `ws:` 移到底边框右侧，
`5c2ffb1` 又改成 `项目 · b:分支 · wt:` 但没删这段）。本任务按设计 §7 整行重写了这段，
仍然照抄保留，而 `task.md` 声称"已同步"。

**修复**：删除该虚假描述；连带删除因此变成死代码的 `WORKSPACE_PREFIX` /
`PROMPT_WORKSPACE_PREFIX`（`grep` 确认零引用）。

---

## 6. 中：tab 与右角之间的 `─` 缺失（宽 11 / 18）

**位置**：`src/ui/right_panel.rs::tab_rects`

**关键代码（修改前）**：

```rust
// 必须给右边角留出 1 列，否则 tab 会盖掉 `╮`。
if x.saturating_add(width) >= row.right() {
    break;
}
```

**错误原因**：判据只保证"不压住右角"，没保证"右角前留 1 列边框线"。
实测（`Prompt` tab 宽 8、起排 2 列、`Todo` 宽 6）：

```text
req=11 actual=11 tabs=1 row="╭─ Prompt ╮"     ← tab 紧贴圆角
req=18 actual=18 tabs=2 row="╭─ Prompt ─ Todo ╮" ← 同样紧贴
```

宽 11 是**可达**的（`min_pane_width = 10`，用户可把右栏拖到 11）。

**修复**：判据改为 `>= row.right().saturating_sub(1)`。

**回归用例**：`tabs_never_touch_the_right_corner` 扫遍宽度 10–24，
断言"右角前一列恒为 `─`"。变异验证：回退判据后报
`宽 11: 第 0 个 tab(Prompt) 必须给右角前留 1 列 ─: ╭─ Prompt ╮`。

---

## 7. 中：tab 宽度按字符数算，宽字符文案会画成半截

**位置**：`src/ui/right_panel.rs::tab_rects`

**关键代码（修改前）**：

```rust
let width = u16::try_from(tab_label(tab).chars().count())
    .unwrap_or(u16::MAX)
    .saturating_add(TAB_PAD.saturating_mul(2));
```

**错误原因**：`width` 是"字符数 + 2"，而 `set_stringn` 按**显示列**截断。
换成中文文案（`待办`）时：rect 宽 4、文字显示宽 6 → 写入 `" 待"` 后剩余 1 列，
`办`（2 列）被整字丢弃 → 可见文案被砍成半截，末尾那格只剩 reset 后的默认样式；
`tab_at` 的命中矩形也只有 4 列，与可见文字不一致；下一个 tab 起点偏 1~2 列。

**修复**：改用 `UnicodeWidthStr::width`（`prompt/header.rs` 里已有同样的用法）。

**状态**：当前文案是 ASCII，两者相等，所以是**潜在缺陷**；无回归用例
（写不出触发它又不改文案的输入）。

---

## 8. 中：Todo 下滚动条 / 右键 / 折叠顺序 —— 闸门零覆盖

三处新增闸门原本没有任何用例，删掉后测试全绿。逐条补测并做变异验证：

| 闸门 | 用例 | 删闸门后 |
| --- | --- | --- |
| `prompt_scrollbar` 的 `right_tab == Prompt` | `prompt_scrollbar_hidden_on_todo_tab`（带 Prompt 态正向对照） | 转红 ✅ |
| `overlay.rs` 的提示词树右键 | `mouse_right_click_prompt_tree_on_todo_opens_no_menu`（带 Prompt 态正向对照） | 转红 ✅ |
| Todo 整块吞掉分支的位置 | `mouse_click_prompt_collapse_button_works_on_todo_tab` | 把吞掉分支提到折叠按钮之前 → 转红 ✅ |
| `drag.rs` 的光标跟随 | `mouse_drag_on_todo_does_not_move_hidden_cursor` | **不转红**（见下） |

### 8.1 关于拖拽闸门：这是"冗余防线"，不是测试失效

拖拽有**两道**防线：`drag.rs` 的 `prompt_panel_active` 闸门，与
`pane_content_inner` 的零矩形兜底。删闸门后零矩形提前返回，行为不变；
删兜底后闸门兜住，行为也不变。两道都删才会漏。
用例注释里已写明这一点，避免后人误判为假阳性。

### 8.2 折叠态点击：原提案的用例本身是空转

reviewer 建议的"折叠态点顶边框不切 tab"用例，实测**删掉两处
`!state.prompt_collapsed` 守卫后仍然通过**——折叠时 `tab_rects` 恒空
（`panel.width == 4`），无论有没有守卫都落到吞点击分支。已替换为真正承重的
"折叠按钮在 Todo 下仍可点"（锁分支顺序）。

---

## 9. 中：`pane_content_inner` 零矩形兜底无测试 + `press.rs` 无守卫减法

**位置**：`src/event/mouse/hit.rs`、`src/event/mouse/press.rs`

**关键代码（修改前）**：

```rust
} else if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
    let inner = pane_content_inner(state, view, pane, inner);
    let row = mouse.row - inner.y;      // ← 零矩形时 y=0，用屏幕坐标当内容坐标
    let col = mouse.column - inner.x;
    if pane == state.prompt.id() {
```

**错误原因**：`Rect::ZERO` 的 `x/y` 都是 0，所以不会下溢 panic；且 Todo/折叠态的
整块吞掉分支先于此处，今天不可达。风险在于：一旦有人调整分支顺序或换命中源，
这里会拿**屏幕坐标**当内容坐标去 `place_prompt_cursor` + `begin_selection`。

**修复**：
1. 新增 `pane_content_inner_is_zero_on_todo_tab`（带 Prompt 态正向对照）；
   删兜底 → 转红。
2. 把 `row`/`col` 的减法挪到零矩形早退之后，所有分支共用同一道守卫
   （lx 窗格分支同样用到 `row`/`col`，原先也吃这个隐患）。

---

## 10. 低：渲染层三处边界

| # | 问题 | 处理 |
| --- | --- | --- |
| 1 | 内框为空（高 ≤ 2）时 `render` 提前返回，但 `tab_rects` 仍给出矩形 → "看不见却能点"的 tab | `tab_rects` 加 `panel.height <= 2` 返回空。正常操作不可达（`min_height = 10`） |
| 2 | 非激活 tab 的 hover 分支零渲染测试 | 新增 `hovered_inactive_tab_uses_hover_style` |
| 3 | Todo 态顶边框是否仍画 tab 零断言（`paint_tabs` 挪进 Prompt 分支则测试全绿） | 新增 `tabs_stay_on_border_and_follow_active_tab_on_todo`；变异验证：把 `paint_tabs` 挪进 `RightTab::Prompt` 分支 → 转红 |

---

## 11. 设计文档的阈值错位

**位置**：`docs/feature/right-panel-tabs/design.md` §5.2、§6.2

**错误**：§5.2 写"`width < 10` 整排 tab 不画"（即宽 10 仍应画），
但实现（`x + width >= row.right()`）与测试（宽 10 断言为空）都在宽 10 就不画。

**修复**：改为 `width <= 10`，并说明 `min_pane_width = 10` 使 10 成为可达最窄值；
§6.2 的"宽 8 一个都不画"改为"宽 10"（宽 8 会被 `clamp_prompt_width` 钳回 10，测不到）。

---

## 12. 确认没问题的点（reviewer 逐条验证过，不重复排查）

1. **绘制顺序**在所有分支成立：Block → tabs → 底边框 → 面板体（分割线 → 标题覆盖）
   → 内容 → 滚动条。几条矩形互不越界；Todo 内容区空白由 ratatui 每帧
   `Buffer::reset()` 保证，不是靠"没人写"。
2. **几何同源**：tab 绘制与命中同用 `tab_rects`；标题/页控件绘制与命中同用
   `title_row` + `page_controls`。事件层无重算列号。
3. **列号一致**（实算）：`panel x=0,width=56` 时旧 `prompt_border_rect` 与新
   `title_row` 都是 `x=0,w=56` → `prev=47`、`counter=[49,51]`、`next=53`，逐列相同。
   反之若直接拿 `prompt_divider_rect` 的原始矩形（`x=1,w=54`）会整组左移一列
   —— 所以 `title_row` 里覆盖 `x/width` 是必要的，不是冗余。
4. **样式继承**：`Buffer::set_style` 只改 fg/bg/modifier，不改 symbol；
   tab 之间那 1 列 `─` 不在任何 rect 内，实测仍是 `DIVIDER_MID`。
5. **`draft` 同源**：`render_header` 与 `paint_title_row` 收到的是同一份
   `title_source` 结果；树视图恒 `false` 正确（树视图工具栏没有 Save 点）。
6. **规范符合性**：`src/layout.rs` 未改（999 行）、生产代码无新增
   `unwrap()`/`expect()`、用户可见文案进 `ui::text.rs`、`ui/` 无 `Color::` 直写、
   渲染只读状态。
7. **不变量** `prompt_focused ⇒ right_tab == Prompt` 成立（`focus_prompt` 一处兜住）。

---

## 13. 明确保留的问题

### 13.1 异步回调会从 Todo 静默切回 Prompt

`open_created_page`（`pages.rs`）与 `load_editor_target`（`prompt.rs`）在 DB 回调返回后
调用 `focus_prompt`，会同时把 `right_tab` 拉回 Prompt 并置 `prompt_focused = true`。
用户在 Prompt 里发起 `SaveDraft` / `CreatePage` 后立刻点 Todo，回调到达时画面会自行跳回 Prompt。

**判定**：语义上"回调即打开该提示词"可以自圆其说，且 `CreatePage` 还会就地开始页重命名
输入——若不聚焦，用户会失去刚创建的页的输入焦点。**保留当前行为**，在此记录为已知取舍。
它与设计决策 5（"切回 Prompt 不自动聚焦"）的差异是**有意**的：那条约束针对的是
"用户点 tab 切回"，不针对"系统替用户打开一个目标"。

### 13.2 `src/event/tests.rs` 已 5500+ 行

远超项目单文件 1000 行上限。这是既有债，拆文件会动模块结构，超出本任务范围。

### 13.3 命名债

`ViewLayout.prompt` / `AppState.prompt_*` 语义已是"容器"，未重命名（40+ 调用点，
且与 `state.prompt` 编辑器语义冲突）。

---

## 14. 修复清单（按文件）

| 文件 | 改动 |
| --- | --- |
| `src/app/update/right_panel.rs` | 无（本轮未改） |
| `src/app/update/tests.rs` | 重写 2 条空转测试，新增 1 条分面板用例 |
| `src/event/mod.rs` | worktree 回调与 pump 加 `prompt_panel_active` 闸门 |
| `src/event/mouse/press.rs` | tab 分支提交重命名 + 按变化置脏；零矩形早退 |
| `src/event/tests.rs` | 新增 6 条（含 1 条 worktree 复活回归） |
| `src/ui/right_panel.rs` | tab 宽度改显示列；右角留 1 列；高度 ≤ 2 无可点 tab |
| `src/ui/right_panel/tests.rs` | 新增 3 条（宽度扫描、hover 样式、Todo 顶边框） |
| `src/ui/tests.rs` | 新增滚动条闸门用例 |
| `src/ui/text.rs` | 删除死常量 `WORKSPACE_PREFIX` / `PROMPT_WORKSPACE_PREFIX` |
| `docs/standards/tui-design-requirements.md` | 删 `ws:` 谎言；同步 tab 配色规范 |
| `docs/feature/right-panel-tabs/design.md` | §5.2 阈值修正；§9 差异表补充 |
