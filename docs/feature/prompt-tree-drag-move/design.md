# Prompt 列表拖拽移动（进/出文件夹）— 设计

> 状态：**已实现**（2026-10-25，分支 `feat/prompt-tree-drag-move`）；实施期偏差见 §11。
>
> 任务范围：`src/app/state.rs`、`src/app/effects.rs`、`src/app/update/prompt/`（新增 `drag.rs`）、
> `src/app/update/prompt.rs`、`src/app/prompt/tree.rs`、`src/db/mod.rs`、`src/db/service.rs`、
> `src/event/mouse/{press,drag,hover}.rs`、`src/ui/prompt/tree.rs`、`src/ui/prompt_frame.rs`、
> `src/ui/text.rs`、`docs/standards/tui-design-requirements.md`。
>
> 不改数据库结构、不改 `SCHEMA_VERSION`（当前为 14，`src/db/mod.rs:25`）、不改配置项、不改 PTY 行为。

## 1. 背景与目标

### 1.1 现状（代码事实，均为本次读取核实）

- prompt 列表（树视图）**完全没有拖拽代码**：`grep drag src/app/prompt src/app/update/prompt*` 无命中。
  唯一的拖拽分支是滚动条（`src/event/mouse/drag.rs:81-91`）与终端/lx 选区。
- 行命中是"渲染的逆函数"，每次事件按 `scroll` + `panel.rows()` 重算，不缓存命中区：
  `src/ui/prompt/tree.rs:171 row_at`。树行自 `area.y + 2` 起（搜索框 1 行 + 分割线 1 行，
  `src/layout.rs:17 PROMPT_TREE_HEADER_HEIGHT = 2`）。
- 左键落在 prompt 内容区时走 `src/event/mouse/press.rs:388` 分支：`focus_prompt` → 粘贴面板 →
  树视图 `row_at` → 双击判定（`prompt_row_double_click`）或 `click_prompt_row`。
  **`click_prompt_row` 在按下瞬间就折叠/展开文件夹**（`src/app/update/prompt/tree_view.rs:108-127`，
  内部 `toggle_prompt_folder` 在 `:331`）。
- 树模型是扁平存储，两层硬约束写死在三处：
  - `PromptTree::folder_depth` 只返回 0/1（`src/app/prompt/tree.rs:341`）；
  - `can_create_subfolder` 仅根层为真（`:347`）；
  - `push_children` 只递归一层（`:411`），第三层不会渲染出来；
  - DB 侧 `create_folder` 对非根层父级返回 `DbError::DepthExceeded`（`src/db/mod.rs:460-486`）。
- 顺序 = **创建顺序**：`list_folders ORDER BY f.id`（`src/db/mod.rs:324`）、
  `list_prompts ORDER BY p.id`（`:344`），内存 `PromptTree::replace` 再 `sort_by_key(id)`
  （`src/app/prompt/tree.rs:291-308`）。**没有任何 `sort_order` 列。**
- 持久化通路：`app` 层只登记 `Effect`（`src/app/effects.rs`），`Persistence::pump`
  （`src/db/service.rs:444`）在事件循环内同步执行；失败由 `with_db` 吞成 `None` + `tracing::warn`
  （`:1024`），成功后 `reload_tree`（`:1037`）整体刷新同 prompt 键的全部工作区面板。
  **没有乐观更新机制**，界面永远以 DB 为准。
- 左侧工作区的"拖拽"是**列表内重排且不落库**（`state.workspace_drag` / `workspace_dragging`，
  `src/app/state.rs:179/181`；`begin_workspace_drag` / `drag_workspace_to` 在
  `src/app/update/workspace_list.rs:88/99`；落点 `ui::workspace_drop_index` 在
  `src/ui/sidebar.rs:372`）。它**没有**"进/出文件夹"语义，也没有 hover 自动展开——
  所以本次不是"照抄"，而是新功能；只能复用它"行级阈值 + 整行选中底色 + 拖拽期抑制 hover"的
  交互口径。

### 1.2 目标

1. 在右侧 Prompt 列表页，可用鼠标把**条目**拖进文件夹、拖出到根层。
2. 可把**根层文件夹**拖进另一个根层文件夹、拖回根层。
3. 拖动期间源行压暗、目标容器行铺底色（落点实时反显）；非法落点在松手时给出可见反馈。
4. 移动结果落库，重载/重启后保持（沿用 `reload_tree`）。

### 1.3 非目标

- **不做列表内精细排序**（不引入 `sort_order`、不写迁移）。落点只决定"进哪个文件夹"，
  文件夹内先后顺序仍由 `id` 决定。
- 不做边缘自动滚动、不做 hover 自动展开、不做半透明幽灵行/插入位预览；落点反显见 §11.7。
- 不加"移动到…"右键菜单等价入口（移动只有拖拽一条入口）。
- 不支持拖 `Temp` 草稿行、`Section` 分组标题行。
- 不支持第 3 层文件夹。

## 2. 已确认决策（Grill Me 结论，逐条采纳推荐）

| # | 决策项 | 结论 |
| --- | --- | --- |
| 1 | 拖拽语义 | **只 reparent**（改 `folder_id` / `parent_folder_id`），不引入排序列，不进文件夹的相对顺序沿用 `id`。 |
| 2 | 文件夹可拖范围 | 文件夹可拖；**拖进文件夹时必须满足**：目标为根层文件夹、被拖文件夹本身是根层（`parent_id.is_none()`）、被拖文件夹不含子文件夹、不能拖进自己。其余情况拒绝。条目可拖进任意文件夹。 |
| 3 | 落点判定 | 文件夹行 → 该文件夹；条目行 → 该条目**所在的文件夹**（根层条目 → 根层）；`Temp` 行 → 根层；空白区（末行以下）→ 根层；`Section` 行 → 不响应；树外 → 取消。折叠的文件夹不自动展开。 |
| 4 | 与"按下即折叠"的冲突 | **点击语义推迟到 Up**：Down 只选中 + 登记拖拽起点；`Drag` 跨行才置 `dragging`；Up 时未拖动走原点击/双击语义，拖动则提交移动（不折叠、不开编辑器）。 |
| 5 | 持久化 | 不做乐观更新：登记 `Effect::MovePrompt` / `Effect::MoveFolder` → `pump` 写库 → `reload_tree` 刷新。写库失败补 `ToastKind::Danger`（锚点 = prompt 面板）。`Temp` 行禁止拖动。只改 `folder_id` / `parent_folder_id` 与 `updated_at`。 |
| 6 | 视觉与非法落点反馈 | **源行压暗（`style::muted()`）+ 目标容器行铺 `style::selection()` + 落点为空根层时末行以下铺 `→ move to root`**；被拒绝的落点不亮；不新增颜色/动效；非法落点松手 toast；树外静默取消。 |
| 7 | 带子文件夹的根层文件夹 | 拖进另一个文件夹 → 拒绝并 toast（否则其子文件夹会掉进 `push_children` 渲染不到的第 3 层）。拖到根层 → 允许（层数不变）。 |

### 2.1 决策 2 的补充说明（必须写清，避免实现跑偏）

决策 2 的口径是"**只有根层文件夹能接收文件夹**"。结构上，把"已是子层的文件夹"移进另一个根层
文件夹其实仍是 2 层，不违反深度约束；但按已确认口径**一律拒绝**，代价只是"先拖回根层再拖进去"。
实现与 DB 都按此口径：`target.is_some() ⇒ dragged.parent_id.is_none() && target.parent_id.is_none() && no_children`。

另外两条按推荐默认处理，不单独提问：

- **搜索词非空时禁用拖拽**：过滤视图会隐藏目标、还会忽略折叠态，拖动结果与所见不符。
  此时 Down→Up 仍是普通点击。
- **拖动期间抑制 hover 高亮与 tooltip**（与 `workspace_drag` 同口径，
  `src/event/mouse/hover.rs:113`、`:261`）。

## 3. 数据模型与规则

### 3.1 新增状态（`src/app/state.rs`）

```rust
/// 拖拽起点：被拖对象的持久身份。`Temp` 草稿没有 DB id，不进入此枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDragSource {
    Folder(i64),
    Item(i64),
}

/// 一次 prompt 列表拖拽的现场；只在 Down→Up 之间存活。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptDrag {
    /// 面板所属工作区（`prompt_workspace()` 在按下时取定）。
    pub workspace: usize,
    pub source: PromptDragSource,
    /// 按下时的可见行索引（也是未拖动时点击的目标行）。
    pub from_row: usize,
    /// 指针是否已跨行（行级阈值；只横向抖动不算拖动）。
    pub dragging: bool,
    /// 本行是否允许拖动（`Temp` 行、搜索过滤态为 false）。
    pub enabled: bool,
}
```

`AppState` 新增：

```rust
/// 正在拖动的 prompt 列表行；None 表示未拖拽。仅一次 Down→Up 内有效。
pub prompt_drag: Option<PromptDrag>,
```

`AppState::demo()` 增加 `prompt_drag: None`。

**校验口径**：`enabled = false` 时，`Drag` 事件完全忽略；`Up` 仍按点击处理。

### 3.2 落点类型（分层职责）

几何在 UI 层，语义在 app 层：

```rust
// src/ui/prompt/tree.rs —— 只算几何，不懂业务
/// 拖拽落点几何：树内某行 / 树内搜索框以下空白（根层）/ 树外。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDropHit {
    Row(usize),
    Root,
    Outside,
}

pub fn drop_hit_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> PromptDropHit
```

实现与 `row_at` 同源（`src/ui/prompt/tree.rs:171`）：区域外或搜索框/分割线行 → `Outside`；
`scroll + (row - (area.y + 2)) < rows.len()` → `Row(index)`；否则 `Root`。

### 3.3 落点语义与合法性（app 层纯函数）

```rust
// src/app/update/prompt/drag.rs
/// 移动被拒绝的原因（决定 toast 文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMoveReject {
    /// 目标会形成第 3 层，或文件夹收到了不该收的文件夹。
    Depth,
    /// 被拖文件夹含子文件夹。
    Nested,
    /// 无可移动（拖到自己、同一容器）。
    Noop,
}

/// 解析落点为容器（None = 根层）。
pub fn prompt_drop_container(state: &AppState, hit: PromptDropHit) -> Result<Option<i64>, PromptMoveReject>;
```

规则表（与决策 2/3/7 一一对应）：

| 被拖对象 | 落点 | 目标容器 | 结果 |
| --- | --- | --- | --- |
| `Item` | 文件夹行 `f` | `Some(f)` | 允许（`f` 可为任意层） |
| `Item` | 条目行（`folder_id = x`） | `x` | 允许；`x == 当前 folder_id` 时 `Noop` |
| `Item` / `Folder` | 空白 | `None` | 允许；已在根层则 `Noop` |
| `Folder` | 根层文件夹 `f` | `Some(f)` | 允许，需 `dragged.parent_id.is_none()` 且 `f.parent_id.is_none()` 且无子文件夹且 `f != dragged` |
| `Folder` | 子文件夹 `f` | — | `Depth` |
| `Folder` | 任意 | — | 被拖文件夹有子文件夹 → `Nested` |
| 任意 | `Section` 行 | — | `Outside`（不响应） |
| 任意 | 树外 | — | 取消（不 toast） |
| 任意 | 被拖对象自身 / 当前容器 | — | `Noop`（不 toast、不发 effect） |

### 3.4 不可变的层数约束

`push_children`（`src/app/prompt/tree.rs:411`）只遍历 `parent_id == Some(root)` 一层，
**任何让根层文件夹带着子文件夹进入另一文件夹的操作都必须被拒**，否则那些子文件夹与其中条目
在新一轮 `reload_tree` 后从界面消失（数据还在库里，但不可见、不可操作）。因此该检查必须
**同时在 app 层（给 toast）与 DB 层（真值防线）** 存在。

## 4. 事件与交互（状态机）

### 4.1 Down（`src/event/mouse/press.rs:388` 分支改写）

```rust
if pane == state.prompt.id() {
    update::focus_prompt(state);
    if state.prompt.paste_panel().is_some() {
        state.prompt.paste_cancel();
    } else if update::prompt_tree_mode(state) {
        if let Some(index) = ui::prompt::tree::row_at(
            inner, &state.prompt_owner().prompt_panel, mouse.column, mouse.row,
        ) {
            // 只选中并登记起点；折叠/展开与双击判定全部推迟到 Up。
            update::begin_prompt_drag(state, index);
        }
    } else { /* 原编辑器光标逻辑不变 */ }
    *dirty = true;
}
```

`begin_prompt_drag`（`src/app/update/prompt/drag.rs`）：

```rust
pub fn begin_prompt_drag(state: &mut AppState, row: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some(panel) = state.workspaces.get(workspace).map(|w| &w.prompt_panel) else { return false };
    let Some(row_data) = panel.rows().get(row).copied() else { return false };
    if !row_data.kind.selectable() { return false; }   // Section 不参与
    let source = match row_data.kind {
        PromptRowKind::Folder => match panel.tree.folder_of_row(row_data) {
            Some(folder) => PromptDragSource::Folder(folder.id),
            None => return false,
        },
        PromptRowKind::Item => match panel.tree.item_of_row(row_data) {
            Some(item) => PromptDragSource::Item(item.id),
            None => return false,
        },
        PromptRowKind::Temp | PromptRowKind::Section(_) => {
            // Temp 没有持久身份：仍登记现场（Up 走点击），但禁止拖动。
            PromptDragSource::Item(-1)  // ← 用下方 enabled 表达，不伪造 id
        }
    };
    ...
}
```

> 实现细节：`Temp` 不要伪造 id。改为 `PromptDrag { source: Option<PromptDragSource>, ... }`，
> `None` 即"可点击、不可拖"。最终签名以实现为准，但语义固定：**Temp/Section 永不产生 effect**。

选中语义保持现状：`select_prompt_row(state, row)`（`tree_view.rs:70`）在 Down 调用。

### 4.2 Drag（`src/event/mouse/drag.rs`，新增分支）

放在 `resizing_prompt` / `workspace_drag` 分支之后、`forward_pane_mouse_event` 之前
（必须在窗格转发之前 return，否则拖动会被转发给终端）：

```rust
if state.prompt_drag.is_some() {
    let hit = ui::prompt::tree::drop_hit_at(
        layout::prompt_text_rect(view.prompt),
        &state.prompt_owner().prompt_panel,
        mouse.column,
        mouse.row,
    );
    if update::drag_prompt_to(state, hit) {
        *dirty = true;
    }
    return;
}
```

`drag_prompt_to` 只做一件事（**不提交**）：落点是 `Row(i)` 且 `i != from_row` 时置
`dragging = true`；否则保持。`enabled == false` 时直接 false。返回"是否置脏"
（`dragging` 从 false→true 才返回 true）。

### 4.3 Up（`src/event/mouse/drag.rs`，新增分支）

放在 `overlay` 检查之后、与 `workspace_drag` 分支相邻处：

```rust
if state.prompt_drag.is_some() {
    let hit = ui::prompt::tree::drop_hit_at(
        layout::prompt_text_rect(view.prompt),
        &state.prompt_owner().prompt_panel,
        mouse.column,
        mouse.row,
    );
    update::finish_prompt_drag(state, hit);
    *dirty = true;
    return;
}
```

`finish_prompt_drag`：

```rust
pub fn finish_prompt_drag(state: &mut AppState, hit: PromptDropHit) -> bool {
    let Some(drag) = state.prompt_drag.take() else { return false };
    state.prompt_tree_hover = None;
    if !drag.dragging {
        // 未跨行 = 点击。双击判定从 Down 迁到 Up，窗口仍是 400ms。
        let now = Instant::now();
        if prompt_row_double_click(state, drag.from_row, now) {
            return open_prompt_row(state, drag.from_row);
        }
        return click_prompt_row(state, drag.from_row);
    }
    let Some(source) = drag.source else { return false };   // Temp 被拖（不应发生）
    match prompt_drop_container(state, hit) {
        Ok(Some(target)) | Ok(None) => queue_move(state, drag.workspace, source, target),
        Err(PromptMoveReject::Noop) => false,
        Err(reason) => { show_reject_toast(state, reason); true }
    }
}
```

`queue_move`：比较当前容器，相同则 `false`；否则 `state.effects.push(...)` 返回 `true`。
`PromptDropHit::Outside` / `Section` 行 → 直接 `false`（静默取消）。

### 4.4 不变量

1. `state.prompt_drag.is_some()` ⇒ 本轮 Down 发生在树视图的可见行上。
2. `prompt_drag.dragging == true` ⇒ 指针曾到达与 `from_row` 不同的行。
3. Down 时**不得**修改 `collapsed`、**不得**打开编辑器、**不得**登记双击。
4. 一次 `Down` 必然被一次 `Up` 收尾（`finish_prompt_drag` 用 `take()`），跨轮不残留。
   额外清场：`select_right_tab` 切离 Prompt、`switch_workspace`、`toggle_prompt_panel_mode`
   各置 `prompt_drag = None`（防御性，见 §6.3）。
5. 拖拽路径不写 DB、不产生 IO，只登记 `Effect`（架构规范：`app/` 无 IO）。

## 5. 持久化

### 5.1 新增 Effect（`src/app/effects.rs`，紧跟 `RenameFolder`/`DeleteFolder` 之后）

```rust
/// 移动提示词到另一个容器（`folder_id` 为 None 时回根层）。
MovePrompt { workspace: PathBuf, id: i64, folder_id: Option<i64> },
/// 移动文件夹到另一个容器（`parent_id` 为 None 时回根层）。
MoveFolder { workspace: PathBuf, id: i64, parent_id: Option<i64> },
```

### 5.2 DB 层（`src/db/mod.rs`，紧邻 `rename_folder` `:499`）

```rust
/// 移动提示词到指定文件夹（None 回根层）；文件夹须属同一工作区。
pub fn move_prompt(&self, id: i64, folder_id: Option<i64>) -> Result<()> {
    let folder_external: Option<String> = match folder_id {
        Some(folder) => {
            // 与 create_prompt 同款校验：external_id 解析 + 同工作区。
            ...
        }
        None => None,
    };
    self.conn.execute(
        "UPDATE workspace_prompt SET folder_id = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![folder_external, id],
    )?;
    Ok(())
}

/// 移动文件夹（None 回根层）。层数约束：目标必须是根层文件夹，
/// 被拖文件夹必须是根层且无子文件夹（对齐 create_folder 的 2 层上限）。
pub fn move_folder(&self, id: i64, parent_id: Option<i64>) -> Result<()> {
    // 1) 读被拖文件夹：(external_id, workspace_id, parent_folder_id)
    // 2) 有子文件夹 → Err(DbError::FolderHasChildren)
    // 3) Some(parent)：parent == id → Err(DepthExceeded)
    //                  parent 不存在/跨工作区 → Err(FolderMissing)
    //                  parent.parent_folder_id.is_some() → Err(DepthExceeded)
    //                  被拖文件夹 parent_folder_id.is_some() → Err(DepthExceeded)
    // 4) UPDATE workspace_folder SET parent_folder_id = ?1, updated_at = ...
}
```

新增错误变体（仅内存语义，无 schema 变化）：

```rust
/// 目标文件夹仍含子文件夹，不能整体移入另一文件夹。
FolderHasChildren,
```

`Display`（`src/db/mod.rs:108` 附近）与 `code()`（`:155` 附近）各补一行。

### 5.3 Service（`src/db/service.rs`，紧跟 `RenameFolder` 分支 `:589`）

```rust
Effect::MovePrompt { workspace, id, folder_id } => {
    let key = prompt_key_for(state, &workspace);
    let moved = self.with_db(|db| db.move_prompt(id, folder_id)).is_some();
    let changed = self.reload_tree(state, &key);
    if !moved {
        update::show_toast(state, Toast::new(
            ToastKind::Danger, text::TOAST_PROMPT_MOVE_FAILED,
            Some(state.prompt.id()), Instant::now()));
    }
    changed || !moved
}
// MoveFolder 同构
```

- 锚点用 `state.prompt.id()`：`ui::toast::rect`（`src/ui/toast.rs:24-57`）对 prompt pane 会贴
  `view.prompt` 顶边框下方居中，符合"反馈留在触发它的面板"的既有口径。
- `is_network_effect`（`src/db/service.rs:1148`）**不**包含这两个新变体：它们必须走持久化泵。
- 失败后 `reload_tree` 仍要跑：以 DB 真值把界面拉回原位，避免内存里留下"看起来移动了"的假象。

### 5.4 文案（`src/ui/text.rs`，紧邻 `TOAST_PROMPT_SAVE_FAILED` `:533`）

```rust
/// 提示词移动失败（写库失败）。
pub const TOAST_PROMPT_MOVE_FAILED: &str = "Failed to move; unchanged";
/// 目标文件夹层级不合法（会形成第三层或接收了不该接收的文件夹）。
pub const TOAST_PROMPT_MOVE_DEPTH: &str = "Cannot move: folders support two levels";
/// 被拖文件夹内含子文件夹，不能整体移入另一文件夹。
pub const TOAST_PROMPT_MOVE_NESTED: &str = "Cannot move: folder contains subfolders";
```

## 6. 渲染

### 6.1 拖拽反显（`src/ui/prompt/tree.rs`）

`render` 增加第 6 个参数 `drag: Option<PromptDrag>`（仅 `dragging == true` 时给整个现场，
渲染要同时读 `from_row` 与 `drop_mark`）：

```rust
pub fn render(
    area: Rect, buf: &mut Buffer, panel: &PromptPanel,
    focused: bool, hover: Option<usize>, drag: Option<PromptDrag>,
) -> Option<(u16, u16)>
```

`paint_row`（`src/ui/prompt/tree.rs:250`）按 `DragRow::{None, Source, Target}` 分别上样式：

```rust
let highlighted = hovered || drag_row == DragRow::Target;
let highlight = if highlighted {
    style::selection()                 // 与 workspace 拖拽一致（src/ui/sidebar.rs:92）
} else if drag_row == DragRow::Source {
    style::muted()                     // 源行压暗，表示"正在搬走"
} else {
    Style::default()
};
if highlighted {
    fill_row(buf, area, y);
}
```

不新增 `ui/style.rs` token（`docs/standards/tui-design-requirements.md:45` 禁止自造颜色）。
`drop_mark == Root` 时在末行以下空白铺一行 `text::PROMPT_TREE_DROP_ROOT`（`→ move to root`）；
被拒绝的落点 `drop_mark == None`，一律不亮。详见 §11.7。

### 6.2 调用点（`src/ui/prompt_frame.rs:179-186`）

```rust
prompt::tree::render(
    text_area,
    frame.buffer_mut(),
    &state.prompt_owner().prompt_panel,
    focused,
    state.prompt_tree_hover,
    state.prompt_drag.filter(|drag| drag.dragging),
)
```

`src/ui/prompt/tree/tests.rs` 的 20 处 `render(...)` 调用统一补 `None`（外加新增用例）。
`row_at` 不动；新增 `drop_hit_at` 复用其行换算。

### 6.3 hover / tooltip 抑制（`src/event/mouse/hover.rs`）

```rust
// :243 附近：拖动期间不计算树 hover，保持被拖行底色唯一
let tree_hover = if hovering_toast
    || state.prompt_drag.is_some()
    || !update::prompt_panel_active(state)
    || !update::prompt_tree_mode(state)
{
    None
} else { /* 原逻辑 */ };
```

```rust
// :261 附近：tooltip 触发条件追加
if hovering_toast || state.workspace_drag.is_some() || state.prompt_drag.is_some() { None } else { ... }
```

`select_right_tab`（`src/app/update/right_panel.rs:10`）、`switch_workspace`
（`src/app/update/workspace.rs:299`）、`toggle_prompt_panel_mode`
（`src/app/update/prompt.rs:237`）各加 `state.prompt_drag = None;`。

## 7. 边界与降级矩阵

| 场景 | 行为 |
| --- | --- |
| 搜索框有过滤词 | 不进入拖动；Down→Up 仍是点击 |
| 拖 `Temp` 草稿行 | 不进入拖动；Down→Up 仍是点击（双击进编辑器） |
| 拖 `Section` 标题行 | Down 不登记（不可选中）；无任何反应 |
| 拖到树外（终端/侧栏/tab） | 松手静默取消，无 toast |
| 拖到搜索框/分割线行 | 视为树外，取消 |
| 拖到末行以下空白 | 回根层 |
| 拖到自己 / 已是同一容器 | 静默 no-op |
| 文件夹进子文件夹 | toast `TOAST_PROMPT_MOVE_DEPTH` |
| 带子文件夹的文件夹进文件夹 | toast `TOAST_PROMPT_MOVE_NESTED` |
| 写库失败（库被锁/文件被删） | toast `TOAST_PROMPT_MOVE_FAILED` + reload 回原位 |
| 面板折叠 / 切到 Todo / 切工作区 | `prompt_drag` 清理，无残留 |
| 折叠态文件夹作为落点 | 允许，不自动展开 |
| 面板窄到行被截断 | 无影响（按行号判定，不依赖列宽） |

## 8. 测试计划

### 8.1 状态层（`src/app/update/prompt/tests.rs` 或新增 `drag` 子模块测试）

- `drop_on_item_uses_container_folder`：条目行 → 其所在文件夹。
- `drop_on_blank_moves_to_root`。
- `move_folder_into_root_folder_is_allowed`。
- `move_folder_into_subfolder_is_rejected_depth`。
- `move_folder_with_children_is_rejected_nested`。
- `move_onto_self_and_same_container_is_noop`。
- `drag_arms_only_after_row_change`：`begin` → 同行 Drag → `dragging == false`。
- `down_does_not_toggle_folder_until_up`：Down 后 `collapsed` 不变，Up 后翻转。
- `finish_after_drag_does_not_toggle_nor_open_editor`。
- `drag_disabled_when_search_active` / `temp_row_never_produces_effect`。
- `reject_toast_uses_prompt_anchor`。

### 8.2 渲染（`src/ui/prompt/tree/tests.rs`）

- `drop_hit_at_distinguishes_row_blank_and_outside`（含 `scroll` 偏移与搜索框行）。
- `dragged_row_gets_selection_background`（对照 `hovered_row_gets_selection_background` `:204`）。

### 8.3 DB（`src/db/tests.rs`）

- `move_prompt_updates_folder_and_persists`（含回根层 `None`）。
- `move_prompt_rejects_foreign_or_missing_folder`。
- `move_folder_rejects_depth_children_and_self`。
- `move_folder_to_root_clears_parent`。

### 8.4 事件（`src/event/tests.rs`）

- `mouse_drag_prompt_item_into_folder_queues_move_effect`（Down → Drag → Up → 断言 `effects`）。
- `mouse_up_without_drag_toggles_folder_once`（**替换**现有
  `mouse_double_click_prompt_folder_toggles_once` `:3633`：原用例只发两次 Down，
  语义迁移后必须发 Down/Up 对）。
- `mouse_double_click_prompt_item_row_opens_editor`（`:3524`）与 `:3615`、`:3551` 需改为
  Down+Up 序列（helper `prompt_tree_double_click` `:3502` 一并改）。
- `mouse_drag_prompt_row_does_not_open_editor`。

### 8.5 集成（`tests/prompt_management.rs`）

- `prompt_drag_move_persists_across_reload`：建两个文件夹 + 一条条目，拖入、`service.pump`、
  关库重开、`pump` 后断言 `item.folder_id == Some(target)`。

### 8.6 手工验收（用户执行 `cargo run`）

1. 拖动条目进文件夹、拖回根层、重启后仍在。
2. 拖动根层文件夹进另一根层文件夹、拖回根层。
3. 子文件夹作为落点、带子文件夹的文件夹拖进文件夹 → 底部 toast。
4. 单击文件夹只在松手时折叠/展开；双击条目仍进编辑器。

## 9. 文档同步

- `docs/standards/tui-design-requirements.md`「交互」一节（`:21` 已有左侧排序拖拽口径）补一条：
  prompt 列表拖拽为 **reparent**（进/出文件夹），行级阈值、拖拽期抑制 hover、
  源行压暗 + 目标容器行铺底色 + 根层提示行（§11.7）、非法落点松手 toast；不做自动展开。
- 本目录 `task.md` 记录实施偏差与验证结果。

## 10. 风险与取舍

1. **点击语义迁移（Down → Up）是本次唯一的行为变更风险**：所有 prompt 树点击/双击测试都要改，
   且必须补"Down 不折叠"的回归用例。用户感知上单击仍在松手瞬间生效，无可感差异。
2. **实现 `Temp` 不可拖时最容易走偏**：任何"伪造临时 id"的写法都会让 effect 落到错误行；
   设计上以 `Option<PromptDragSource>` 表达。
3. **DB 校验与会话内树校验是两份代码**，必须保持同口径；DB 是唯一真值防线（另一个 lx 实例
   可能同时在改同一棵树）。
4. **不引入排序列**意味着"拖进文件夹后排最后"不可控（实际按 `id` 排在已有条目之后，因为新
   移动条目的 id 本来就更小/更大并不确定 —— 它保持自己的 id 位置）。这是决策 1 已接受的代价。
5. `PromptPanel` 是 per-workspace，而 `prompt_drag` 是全局瞬时状态；拖动期间若 `prompt_pinned`
   变化（只能由其它事件触发），本轮 Up 会作用到新面板。为避免这类错位，`begin_prompt_drag`
   记录 `workspace`，`finish` 时校验 `drag.workspace == state.prompt_workspace()`，不等则丢弃。

## 11. 实施记录（与设计的差异）

实施完成后回写，逐条说明偏离及理由。

### 11.1 `PromptDropHit` 落在 app 层，不在 ui 层

§3.2 原写"`ui/prompt/tree.rs` 定义 `PromptDropHit`"。实际定义在
`src/app/prompt/tree.rs:277`，由 `app::prompt` 导出：

- `app/` 只允许依赖 `ui::text`（现存的 17 处引用全是它），若 `app/update/prompt/drag.rs`
  引用 `ui::prompt::tree::PromptDropHit`，就形成 `app → ui` 的实质依赖，破坏依赖方向。
- 几何换算仍留在 ui 层（`ui::prompt::tree::drop_hit_at` 返回它），语义解析在
  `app::update::prompt::drag::prompt_drop_container`。分层职责与设计意图一致，
  只是类型的归属层换了。

### 11.2 `prompt_drop_container` 接收 `&PromptDrag`

§3.3 原签名从 `state.prompt_drag` 读取现场；实际 `finish_prompt_drag` 已 `take()` 掉现场
（保证"一次 Down 必被一次 Up 收尾"，不跨轮残留），因此改为显式传入。语义不变，
且让纯函数不再依赖"现场仍在 state 里"这个时序假设。

### 11.3 `Root` 落点也算跨行 —— 修正设计的真缺陷

§4.2 原口径：`dragging` 只在 `hit == Row(i) && i != from_row` 时置位。**这条会让需求原文的
"拖出文件夹"在最常见场景下不可达**：

- 树行顺序是"先根层文件夹及其内容，再根层条目"（`PromptTree::rows`）。条目在文件夹内时，
  它通常是最后一行；用户把它拖到末行以下空白想拖回根层，指针全程没有别的行可跨，
  `dragging` 永不为真，松手退化成一次单击。
- 实测证据：`drop_on_blank_moves_to_root` 在修正前是红的。

修正后：`Row(i≠from_row)` 或 `Root` 都置 `dragging`。`Root` 的几何含义就是"末行以下"，
指针到达它必然已离开起点所在的物理行，仍满足行级阈值本意，不会让"按住不动"变成拖动
（crossterm 只在按住并移动时才发 Drag 事件）。

### 11.4 额外清场：新按下、切钉住、关工作区都丢弃残留现场

`handle_mouse_down_left` 入口加 `state.prompt_drag = None`。理由：指针在拖动中移出窗口时
Up 会丢失，残留现场会被下一次无关的 Up 消费——按旧行号误点（点工具栏却在松手时折叠了
树里的文件夹）。§4.4 不变量 4 的"额外清场"原本只列了三处状态切换，这里补上最直接的一处。
回归用例：`mouse_down_discards_stale_prompt_drag`。

评审补漏（§11.8 第 2 条）：`toggle_prompt_pin`（含 `release_prompt_pin`）与
`close_workspace` 同样会换掉 `prompt_workspace()` / 平移索引，一并补清理，
回归用例 `toggle_prompt_pin_clears_drag_scene`、`close_workspace_clears_drag_scene`。

### 11.5 `move_prompt` 要求条目存在

解析目标文件夹所属工作区必须先读条目行。初版只在 `Some(folder)` 路径校验、且条目不存在
时误报 `DbError::FolderMissing`，`None` 路径则完全跳过校验（更新 0 行仍返回 `Ok`）。
评审后改为**两个分支共用**一次存在性校验，并新增 `DbError::PromptMissing`；
`execute` 的影响行数为 0 也返回 `PromptMissing`。见 §11.8 第 3 条。

### 11.6 三个既有双击用例未改断言

§8.4 原预计语义迁移会让它们失败。实际没有：helper 改为 Down+Up 序列后，两次点击
（Down/Up/Down/Up）在 400ms 窗口内仍构成双击，`click_prompt_row` 的折叠与
`open_prompt_row` 的编辑器打开都发生在 Up，行为与预期一致。

### 11.7 交互手感返工：落点实时反显（用户验收反馈）

初版严格照 §6.1 实现"只有源行铺 `selection`"。用户试用后反馈"拖拽效果十分不好，
参考 ws 的，长按拖拽后需要出现移动效果和 hover 的背景颜色"。核对左侧 ws 的实现
（`src/ui/sidebar.rs:90-96`）可知**它的反显是落点反馈**：

```rust
let dragging = state.workspace_dragging && drag_block.contains(&row.index);
let highlight = if dragging || state.workspace_hover == Some(row.index) { selection() } else ...
```

即"被拖块 + 指针所在行"都反显。prompt 列表此前把 `dragging` 只给了源行，指针行因为
"拖动期抑制 hover"（§4.5）而彻底没有反馈——用户看不到会落到哪，这是手感差的根因，
不是缺动效。

改动（`PromptDrag` 增加 `drop_mark: PromptDropMark`）：

1. `PromptDropMark { None, Row(usize), Root }` 定义在 app 层，随 `PromptDrag` 一起存。
   只描述"会落到哪"，不含业务规则。
2. `drag_prompt_to` 在**置臂之后每次移动**都重算落点，`drop_mark` 变化也返回 `true` 置脏，
   否则高亮不会跟着指针刷新。
3. `prompt_drop_mark` **复用 `prompt_drop_container`** 的判定求容器：高亮与松手行为
   永远一致，不可能出现"亮着却什么也没发生"。被拒绝的落点（第 3 层、含子文件夹、
   拖到自己、同一容器）一律不亮，由松手 toast 说明原因。
4. 高亮画在**目标容器所在的那一行**，不是指针命中的那一行：条目行的容器是它所在的
   文件夹，亮条目行会被读成"插到这条旁边"。
5. 源行改为**压暗**（`style::muted()` 叠加）而不是铺底色：底色语义已被"落点"占用，
   两个都铺会让用户分不清哪一行是落点。
5.1 容器为根层时，指针在某一行上就亮**那一行**（那一行的容器本来就是根层，松手确实落在这里），
   只有指针在末行以下空白才退回 `Root` 提示行：否则用户把条目拖到另一条根层条目上时，
   高亮会莫名跳到列表底部。
6. 根层没有容器行可铺，在末行以下空白铺一行 `→ move to root`（文案在 `ui::text`），
   让"拖到空白回根层"这个最容易迷路的路径有明确落点。
7. 置臂后指针回到起点行或移出树时**保持 `dragging == true`**、只清 `drop_mark`：
   此时是"取消"而不是"没在拖"，松手不该退化成一次单击。

新增用例：状态层 `drag_marks_target_container_row`、`drag_marks_root_when_dropping_below_rows`、
`drag_mark_clears_on_rejected_drop`、`drag_mark_clears_on_same_container`、
`drag_mark_clears_when_leaving_tree`、`drag_mark_absent_before_arming`；
渲染层 `drag_target_row_gets_selection_background`、`drag_source_row_is_dimmed_not_highlighted`、
`drag_root_mark_paints_hint_below_rows`、`not_dragging_renders_no_drag_feedback`
（原 `dragged_row_gets_selection_background` 的语义已被源行压暗取代，改名重写）。

### 11.8 评审与修复（P0–P2）

交付前对 `dev...HEAD` 全量改动做了一次分模块评审，结论与复现路径落在
`review.md`（同目录），本节只记设计与实现的实质修正：

1. **P0：拖动中滚轮可改视口。** `drag_prompt_to` 的"跨行"判定、`drop_hit_at` 的行换算、
   `drop_mark` 三者都按**绝对行号**（`panel.scroll + 视口行`）。滚轮改 `scroll` 后指针
   像素未动，绝对行号却整体平移 → 被误判"已跨行"而置臂，松手走移动分支，条目被静默
   移进指针当前所在行的容器。修法：`handle_mouse_wheel` 最前加 `prompt_drag` 早退，
   与 §1.3「不做边缘自动滚动」同一约束。回归用例
   `mouse_wheel_during_prompt_drag_keeps_viewport_and_state`（反向验证：去掉守卫即
   失败于 `panel.scroll` 由 0 变 1）。
2. **P1：拖拽现场的生命周期漏了两处。** §4.4 不变量只列了 `switch_workspace` /
   `select_right_tab` / `toggle_prompt_panel_mode` / 新一次 Down，实际
   `toggle_prompt_pin`（含 `release_prompt_pin`）与 `close_workspace` 也会让索引与面板
   换人，把现场清理由四处补到六处。否则高亮按新面板的行号画、松手因 `workspace` 校验
   被丢弃，表现为"亮着但什么也没发生"。
3. **P1：`move_prompt` 会对不存在的条目返回 `Ok`。** `folder_id == None` 分支完全跳过
   校验、`execute` 的 `rows_affected` 被丢弃，另一实例删掉条目后本实例拖动会静默成功、
   不弹 `TOAST_PROMPT_MOVE_FAILED`；且条目不存在被误报成 `FolderMissing`。现新增
   `DbError::PromptMissing`，两个分支都先做存在性校验，并检查影响行数。
4. **P2：DB 拒绝原因在 UI 层只剩一句话。** 状态层已区分 `Depth` / `Nested`，但
   `with_db` 把错误压成 `Option`，落库侧再拒绝时用户只能看到笼统的
   `Failed to move; unchanged`。新增 `Persistence::db_result` 保留错误类型，按
   `DbError` 变体分流到已有两条精确文案（`with_db` 未动，其它 effect 不受影响）。
5. **P2：双击窗口从"Down→Down"漂移成"Up→Up"。** 点击语义整体推迟到 Up 后，
   `prompt_row_double_click` 收到的是松开时刻，两次按下间隔可被拉长到
   `400ms + 两次按压时长`。`PromptDrag` 记 `pressed_at`，判定改用按下时刻。
6. **P2：渲染与命中的 `scroll` 钳制口径不一。** `render` 把 `panel.scroll` 钳到
   `len-1`，`row_at` / `drop_hit_at` 直接用原值，行数骤减的瞬时窗口里会"画的是末行、
   命中的是越界行"。抽 `effective_scroll` 三处共用。**这带来一处行为变更**：
   命中语义由"越界即未命中"变为"钳到末行"，并据此改写了既有用例
   `row_at_maps_viewport_and_respects_scroll_and_search_row` 末尾一条断言
   （`scroll = 3`：`None` → `Some(2)`）——是为对齐渲染语义，不是迁就实现。
7. **P2：空树时根层提示行会盖掉空状态占位。** `rows.is_empty()` 时
   `blank == 0`，`→ move to root` 正好落在 `PROMPT_TREE_EMPTY` 那一行；加
   `!rows.is_empty()` 与占位互斥（当前不可达，属防未来改门槛时静默生效）。

关于测试文件体量：`src/app/update/prompt/tests.rs` 与 `src/event/tests.rs` 都远超 1000 行，
评审时曾列为待拆分项，后确认该上限只约束**生产代码**——测试按被测对象归属组织，
按体积切分只会让同一模块的用例散落。规范已在 `AGENTS.md` 与
`docs/standards/code-writing-standards.md` 补明，两个文件保持原样。
