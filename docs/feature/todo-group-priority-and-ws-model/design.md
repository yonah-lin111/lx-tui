# Todo 聚合组优先级展示 + 工作区模型记忆：设计方案

> 状态：**已合并到 `dev`（merge `e8b09999`，2026-11-13）**；P0/P1/P2 review 修复完成，P3 持久化并发与大组性能项留后续。交互外观与多工作区实测仍待用户 `cargo run` 验收。
> 日期：2026-11-12（设计）；2026-11-12（实现与 review 修复）；2026-11-13（合并）。
> 项目：`/Users/yonah/projects/agent/lx-tui`（分支 `dev`）。
> 配套执行清单：[task.md](./task.md)；审查记录：[review.md](./review.md)。
> 代码检索：`codegraph`（lx-tui，indexed）+ `codebase-memory-mcp`（project `lx-tui`，indexed）。
> 参考实现：`/Users/yonah/projects/agent/opencode-dev/packages/tui/src/context/local.tsx`（`createModel` / `fallbackModel` / `newSessionModel`）与 `packages/tui/src/model-preference.ts`。

---

## 1. 需求

1. **Todo 聚合组展示格式**：聚合组只画一次优先级徽标，且使用组内最高优先级；顺序改为 `状态框 → P(n) → 树形连接符 → 正文`：

   ```
          ┌─ aaaa
   [~] P1 ├─ bbbb
          └─ cccc
   ```

2. **修复切工作区后模型变成 `no model`**：切到另一个工作区后应沿用“上次使用的模型”，对齐 opencode 的多工作区模型规则。

---

## 2. 现状与根因

### 2.1 Todo 聚合组（`src/ui/right_panel/todo/`）

| 位置 | 现状 |
| --- | --- |
| `item.rs:19-44` `DisplayGroupRole` | 仅区分 `Single` / `Member { offset, count }`，状态锚点 = `offset == (count-1)/2` |
| `item.rs:92-108` `group_connector_glyph` | 4 列字形：`" ┌─ "` / `"─┌─ "` / `" ├─ "` / `"─├─ "` / `" └─ "` / `" │  "`，锚点行在左侧多画 `─` |
| `item.rs:294-374` `render_item` | 状态框 → 连接符（`row_branch_x`）→ **每行各自的** `P(n)`（`row_priority_x_for`）→ 正文 |
| `item.rs:399-421` `render_priority_badge` | 按 `row.priority`（该条目自身优先级）绘制 |
| `todo.rs:215-264` 几何 | `row_status_x=+2`、`row_branch_x=+5`、`row_priority_x_for(组)=+9/单条=+6`、`row_text_x_for(组)=+12/单条=+9`、`row_cont_x_for=priority_x` |
| `todo.rs:453-463` `priority_at` | 每行首行都能命中优先级列（组 `+9..11`、单条 `+6..8`） |
| `update/todo.rs:147-167` `click_todo_priority` | 命中哪个 display 就选中并循环**该条**的优先级 |

现行渲染实测（`tests/grouping.rs:40`、`:57`）：

```
❯ [ ]─┌─ P1 login api
      └─ P2 login test
```

**问题**：徽标重复（每组 N 个）、顺序不是需求要求的 `[~] P1 ├─`、组级优先级不可见。

### 2.2 工作区模型（`src/app/state/lx/` + `src/app/update/`）

证据链：

1. `LxChat.selection`（`state/lx/chat.rs:480`）是**窗格内存态**，`Lx::new()` 初始化为 `None`；每个工作区各自持有自己的 lx 窗格，所以“工作区记忆”在进程内本来就存在。
2. `LxChat::effective_selection`（`state/lx/chat.rs:908-924`）：
   ```rust
   if let Some(selection) = &self.selection
       && models.settings.providers.get(&selection.provider)
           .is_some_and(|p| p.models.contains_key(&selection.model)) { return Some(normalize); }
   models.default_selection().map(normalize)   // 唯一兜底
   ```
   只有 `defaultModel` 一级；`defaultModel` 为 `None`（例如只 `/resume` 过会话、从未显式 `/model`）时直接得到 `None`，状态行渲染 `no model`（`ui/main_content/lx/status.rs:233/323` 的 `LX_CHAT_NO_MODEL_SHORT`）。
3. `switch_workspace`（`app/update/workspace.rs`）与 `create_workspace_with_cwd`（`app/update/workspace.rs`）**都没有刷新 lx 状态行**。`refresh_lx_status` 只在聊天事件、`/model`、设置页变更、窗格视图切换（`app/update/pane.rs`）时触发。新工作区/历史打开的工作区因此可能一直停在初始化时的 `model_name: None`，即便 `effective_selection` 已经能解析出模型。后续 `3567f598` 补了 `create_tab`、右键 `split_pane`、`toggle_pane_view`、worktree 直开；review 又补了工作区历史新建路径（见 §5.7）。
4. `/model` 只把选择写回 `ai.json` 的 `defaultModel`（`app/update/chat/slash.rs:432-433`、`522-532`），没有“最近使用”列表；`/resume` 绑定会话模型（`app/update/chat/session.rs:224`）不写 `defaultModel`。

opencode 的规则（`local.tsx:184-219`、`model-preference.ts:49-60,103-183`）：

- 新会话模型 = 该 location+agent 的进程内记忆 → agent 配置模型 → **fallbackModel**；
- `fallbackModel` = CLI 参数 → 该 location 的配置模型 → **全局 `preferences.recent`（持久化，最多 10 条）** → 第一个可用模型；
- 已有会话优先用会话落库的模型；
- 只有 `recent/favorite/variant` 落盘（`model.json`），location 记忆是内存态。

---

## 3. 已确认决策（Grill-Me，用户逐题确认“使用推荐”）

| 编号 | 决策 |
| --- | --- |
| D1 | 两级记忆：**每个工作区各自记住自己上次用的模型**；从未用过模型的工作区回退到**全局“最近使用”**，不再直接 `no model`。 |
| D2 | 工作区记忆只在本进程内：`Workspace.lx_model` 记录本工作区最近实际使用/选择的模型，新建 lx 窗格/标签继承；窗格级 `chat.selection` 仍可覆盖（会话/手动选择优先）。跨重启只持久化**全局最近使用列表**（≤10 条，最近在前）。 |
| D3 | 最近使用列表存进现有 `~/.lx-t[-dev]/config/ai.json` 的 `ai.recentModels`，复用原子写 `save_ai`；不新增文件、不迁移 DB。 |
| D4 | 回退链固定：**工作区窗格选择 → `defaultModel` → `recentModels`（逐条校验）→ 第一个可用模型**。同时修掉“切/建工作区不刷新 lx 状态行”的独立 bug。 |
| D5 | 聚合组只在锚点行画一次 `P(n)`，取**组内最高优先级**（`P0` 最高，枚举 `Ord` 最小者；并列取组内最靠前一条）；点击徽标循环切换**当前决定该徽标的条目**并选中它；忽略组内新建幽灵行。 |
| D6 | 聚合组折行续行：`│` 画在 `list.x+9`（与 `├` 同列），续行正文回到 `list.x+12` 与首行正文严格对齐；末项续行连接符槽留 3 空格。 |

---

## 4. Todo 聚合组改造

### 4.1 目标列位（`list.x` 为列表左边界）

| 列 | 单条 | 聚合组锚点行 | 聚合组非锚点行 |
| --- | --- | --- | --- |
| `+0` | `❯` 或空格 | 同 | 同 |
| `+1` | 空格 | 空格 | 空格 |
| `+2..+4` | `[ ]` / `[~]` / `[x]` | 同（锚点自己的状态） | 空 |
| `+5` | 空格 | 空格 | 空 |
| `+6..+7` | `P(n)`（自身） | **组内最高 P** | 空 |
| `+8` | 空格 | 空格 | 空 |
| `+9..+11` | 正文首行起点（无连接符） | `┌─ `/`├─ `/`└─ ` | 同 |
| `+12..` | —（单条正文从 +9 起） | 正文首行 | 正文首行 |

渲染示例（含行首标记列）：

```
❯ [~] P1 ├─ bbbb
         ┌─ aaaa
         └─ cccc
```

按 `DisplayGroupRole` 的锚点规则，三行组的锚点落在 `offset == 1`；两行组锚点在 `offset == 0`，首行即 `[ ] P1 ┌─ xxx`。

### 4.2 几何重构（`src/ui/right_panel/todo.rs:215-264`）

```rust
/// 聚合组树形连接符槽宽（列）：`"┌─ "` / `"├─ "` / `"└─ "` / `"│  "` / `"   "` 均为 3。
pub(super) const GROUP_BRANCH_WIDTH: u16 = 3;

/// 状态框起始 X（`list.x + 2`，宽 3）。
pub(super) fn row_status_x(list: Rect) -> u16 {
    list.x.saturating_add(MARKER_WIDTH).saturating_add(1)
}

/// 优先级徽标起始 X（`list.x + 6`，单条与聚合组统一）。
pub(super) fn row_priority_x(list: Rect) -> u16 {
    row_status_x(list).saturating_add(STATUS_WIDTH).saturating_add(1)
}

/// 聚合组连接符起始 X（`list.x + 9`，宽 3）。
pub(super) fn row_branch_x(list: Rect) -> u16 {
    row_priority_x(list).saturating_add(PRIORITY_WIDTH).saturating_add(1)
}

/// 首行正文起点：单条 `list.x + 9`（无连接符），聚合组 `list.x + 12`。
pub(super) fn row_text_x_for(list: Rect, grouped: bool) -> u16 {
    let base = row_branch_x(list);
    if grouped { base.saturating_add(GROUP_BRANCH_WIDTH) } else { base }
}

/// 折行续行起点：单条保持现状（与 `P(n)` 左对齐 `list.x + 6`），聚合组与首行正文对齐 `list.x + 12`。
pub(super) fn row_cont_x_for(list: Rect, grouped: bool) -> u16 {
    if grouped { row_text_x_for(list, true) } else { row_priority_x(list) }
}
```

- 删除 `row_priority_x_for(list, grouped)`（两态已统一）以及 `#[cfg(test)] fn row_priority_x(list)` 包装，改用真实 `row_priority_x`；
- `row_line_x_for` 逻辑不变（`offset == 0` 取 `row_text_x_for`，否则 `row_cont_x_for`）；
- `text_width_for` 不变（单条仍 `right-9`、组仍 `right-12`）；
- **聚合组** `cont_width_for` 从 `right-9` 变为 `right-12`，与首行一致（续行不再多 3 列）。

### 4.3 连接符字形（`src/ui/right_panel/todo/item.rs:92-108`）

```rust
/// 聚合组子项首行与折行续行的 3 列连接符（位于 `list.x + 9..list.x + 12`）。
///
/// 锚点行的状态框与 `P(n)` 已移到连接符左侧，`─` 前导不再需要，锚点与非锚点同形。
pub(super) fn group_connector_glyph(offset: usize, count: usize, line_offset: usize) -> &'static str {
    if line_offset > 0 {
        return if offset + 1 < count { "│  " } else { "   " };
    }
    if offset == 0 {
        "┌─ "
    } else if offset + 1 < count {
        "├─ "
    } else {
        "└─ "
    }
}
```

`is_anchor` 参数删除（字形不再依赖锚点）；`DisplayGroupRole::is_status_anchor` 继续用于状态框与徽标。

### 4.4 组内最高优先级（新增 `item.rs` helper）

```rust
/// 聚合组徽标的归属：组内最高优先级的显示槽位与优先级（并列取最靠前；忽略幽灵行）。
pub(super) struct GroupPriority {
    pub display: usize,
    pub priority: TodoPriority,
}

/// 计算显示槽位 `display` 所在聚合组的徽标归属；单条返回自身。
pub(super) fn group_priority(
    state: &AppState,
    display: usize,
    ghost: Option<usize>,
    total: usize,
) -> Option<GroupPriority> {
    let role = display_group_role(state, display, ghost, total);
    let DisplayGroupRole::Member { offset, count } = role else {
        let record = display_record(state, display)?;
        return Some(GroupPriority { display, priority: record.priority });
    };
    let start = display.saturating_sub(offset);
    let end = start.saturating_add(count).min(total);
    let mut best: Option<GroupPriority> = None;
    for candidate in start..end {
        if ghost == Some(candidate) {
            continue; // 幽灵行没有落库优先级。
        }
        let Some(record) = display_record(state, candidate) else {
            continue;
        };
        if best.as_ref().is_none_or(|current| record.priority < current.priority) {
            best = Some(GroupPriority { display: candidate, priority: record.priority });
        }
    }
    best
}
```

`TodoPriority` 的 `Ord` 顺序是 `P0 < P1 < P2 < P3`（`src/db/todo.rs:67-74`），因此“最高”= `min()`；严格小于保证并列时保留最靠前的一条。

### 4.5 渲染（`item.rs:245-374`）

- `row_view`：锚点行的 `priority` 改为组级徽标值：
  ```rust
  let badge = group_priority(state, item.display, ghost, total);
  ...
  priority: if item.role.is_status_anchor() {
      badge.map_or(record.priority, |badge| badge.priority)
  } else {
      record.priority // 非锚点行不再绘制，值仅占位
  },
  ```
  幽灵行锚点同理：`priority` 取 `group_priority` 的结果，取不到时保持 `P1`。
- `render_item`：徽标只在锚点行绘制；独立新建幽灵行没有可归属条目，不绘制（避免死控件）；组内幽灵行仍可作为锚点显示组级徽标：
  ```rust
  let standalone_ghost = !row.role.is_grouped() && row.edit.is_some_and(|edit| edit.target.is_none());
  if row.role.is_status_anchor() && !standalone_ghost {
      render_priority_badge(buf, list, row_priority_x(list), item.y, row.priority, row.status);
  }
  ```
- 连接符循环改为新签名：`group_connector_glyph(offset, count, line_offset)`。
- 正文/编辑态 X 坐标继续走 `row_text_x_for` / `row_line_x_for`，无需分支。

徽标配色仍用 `render_priority_badge`：底色由**锚点行的状态**决定（`style::todo_priority_badge(priority, status)`），与状态框同源，避免组内优先级来自别的条目时配色错位。

### 4.6 命中与点击（`todo.rs:453-463`、`update/todo.rs:147-167`）

```rust
/// 命中某行的优先级徽标区域（返回**徽标归属条目的显示下标**；只有锚点行首行命中）。
pub fn priority_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let layout = layout_visible_items(panel, state);
    if !layout.list.contains((column, row).into()) {
        return None;
    }
    let item = layout.item_at(row)?;
    if row != item.y || !item.role.is_status_anchor() {
        return None;
    }
    let px = row_priority_x(layout.list);
    if column < px || column >= px.saturating_add(PRIORITY_WIDTH) {
        return None;
    }
    let ghost = state.todo.editing().filter(|edit| edit.target.is_none()).map(|edit| edit.at);
    let total = state.todo.len() + usize::from(ghost.is_some());
    group_priority(state, item.display, ghost, total).map(|badge| badge.display)
}
```

- 返回值语义从“被点行”变成“徽标来源条目”，`click_todo_priority`（`app/update/todo.rs`）无需改动：它本来就把 display 解析成条目、选中并 `cycle_priority`，因此显示什么就改什么。`select` 会把归属条目滚入视口，让用户看到被修改的对象；这是有意行为。
- 非锚点行、续行一律不命中；幽灵行不可能是徽标来源（见 4.4）。独立新建幽灵行不绘制徽标。

### 4.7 测试与文档同步

- `src/ui/right_panel/todo/tests/grouping.rs`
  - 2 条组：`❯ [ ] P1 ┌─ login api` / `         └─ login test`；
  - 3 条组：`         ┌─ schema` / `  [~] P0 ├─ migrate`（组内最高 P0）/ `         └─ docs`；
  - 连接符颜色断言：`row_branch_x(list)` 处为 `┌`（去掉 `+1`）；
  - 折行续行：非末项 `         │  tail1`（9 空格 + `│` + 2 空格），末项 `            tail2`（12 空格）；
  - `mixed_single_and_grouped_items_align_...`：`row_priority_x(list) == list.x + 6`；单条与组锚点行命中同一列；组内非锚点行不命中；锚点行返回**最高优先级成员的 display**（示例数据 `grp-0=P1, grp-1=P2, grp-2=P3` → 返回 `1`）。
- `src/ui/right_panel/todo/tests.rs`：所有 `row_priority_x_for(list, false/true)` 改 `row_priority_x(list)`；续行宽度断言按组内新口径更新。
- `docs/standards/tui-design-requirements.md:125`：补充聚合组规则（单条条目文案不变，新增组内 3 列连接符 + `+6` 共用徽标列 + 组内续行与正文对齐）。

---

## 5. 工作区模型记忆

### 5.1 配置层：`AiSettings.recentModels`

`src/config/ai/mod.rs:333-383` 增加字段（保持 `extra` 往返兜底）：

```rust
/// 最近使用的模型（最多 `MODEL_RECENT_LIMIT` 条，最近在前；仅用于无窗格选择的兜底）。
#[serde(default, rename = "recentModels", skip_serializing_if = "Vec::is_empty")]
pub recent_models: Vec<ModelSelection>,
```

- `AiSettings::empty()` 增加 `recent_models: Vec::new()`；
- **不**加入 `validate_inner`：这是提示性历史，不是配置不变量，绝不因为一条过期记录阻塞 `save_ai`；
- **不**在 `sanitize_optional_selections_with_connect` 里裁剪：供应商临时停用/换 Key 时不能丢历史，读取时逐条校验即可；
- 设置页 `PUT /settings/api/models`（`settings/service/models/mod.rs:277-343`）的 `merge_settings` 以 `current` 为基底增量修改，未携带 `recentModels` 时原样保留；`save_ai` 会把该字段写回 `ai.json`。

### 5.2 状态层：`ModelsSnapshot`（`src/app/state/lx/chat.rs:42-96`）

```rust
/// 最近使用模型的保留条数（对齐 opencode `recentModels` 的 10 条上限）。
pub const MODEL_RECENT_LIMIT: usize = 10;

impl ModelsSnapshot {
    /// 无窗格选择时的兜底：`defaultModel` → `recentModels`（最近优先、逐条校验）→ 第一个可用模型。
    pub fn fallback_selection(&self) -> Option<ModelSelection> {
        if let Some(selection) = self.default_selection() {
            return Some(selection);
        }
        if let Some(selection) = self
            .settings
            .recent_models
            .iter()
            .find_map(|selection| self.validated_selection(Some(selection)))
        {
            return Some(selection);
        }
        self.first_available_selection()
    }

    /// 第一个可用模型：已启用 + 有 Key 的供应商下按 `BTreeMap` 顺序取第一个模型。
    fn first_available_selection(&self) -> Option<ModelSelection> {
        crate::app::state::lx::slash::ModelPicker::candidates(&self.settings)
            .into_iter()
            .next()
            .map(|candidate| candidate.selection)
    }

    /// 记录一次实际使用的模型：同 `provider/model` 去重后移到队首（覆盖最新 variant），截断到上限。
    /// 返回是否有变化（无变化不触发落盘）。
    pub fn remember_model(&mut self, selection: &ModelSelection) -> bool {
        let mut next = Vec::with_capacity(MODEL_RECENT_LIMIT);
        next.push(selection.clone());
        for existing in &self.settings.recent_models {
            if existing.provider == selection.provider && existing.model == selection.model {
                continue;
            }
            if next.len() >= MODEL_RECENT_LIMIT {
                break;
            }
            next.push(existing.clone());
        }
        if self.settings.recent_models == next {
            return false;
        }
        self.settings.recent_models = next;
        true
    }
}
```

改动 `effective_selection`（`chat.rs`）：

```rust
pub fn effective_selection(&self, models: &ModelsSnapshot) -> Option<ModelSelection> {
    // 窗格选择必须与 fallback 使用同一套可发送校验：供应商启用、模型存在、有 Key；
    // variant 失效只回落默认等级，不丢弃模型。失效窗格选择不得压住回退链。
    if let Some(selection) = &self.selection
        && let Some(selection) = models.validated_selection(Some(selection))
    {
        return Some(selection);
    }
    models
        .fallback_selection()
        .map(|selection| models.normalize_selection(selection))
}
```

`validated_selection` 只校验供应商启用、模型存在、API Key；variant 交给 `normalize_selection` 回落模型默认等级，避免“variant 被删就丢掉整个模型”。

`default_selection/title_selection/recap_selection` 保持原语义（只认配置项），避免标题/recap 因为“最近使用”而悄悄换模型。

### 5.3 持久化：`Effect::SaveDefaultModel` → `Effect::SaveModelChoice`

`src/app/effects.rs:286-289`：

```rust
/// 写回 `config/ai.json` 的模型偏好（`defaultModel` / `recentModels`；`None` 表示保持磁盘现值）。
SaveModelChoice {
    default_model: Option<crate::config::ai::ModelSelection>,
    recent: Option<Vec<crate::config::ai::ModelSelection>>,
},
```

`src/db/service.rs:446-467` 消费端：加载→按 `Some` 覆盖→`save_ai` 一次原子写。

`src/app/update/chat/slash.rs:519-532`：

```rust
pub fn persist_default_model(state: &mut AppState, selection: &ModelSelection) -> bool {
    let mut changed = false;
    if state.models.settings.default_model.as_ref() != Some(selection) {
        state.models.settings.default_model = Some(selection.clone());
        changed = true;
    }
    changed |= state.models.remember_model(selection);
    if !changed {
        return false;
    }
    state.effects.push(crate::app::effects::Effect::SaveModelChoice {
        default_model: Some(selection.clone()),
        recent: Some(state.models.settings.recent_models.clone()),
    });
    true
}
```

新增“只记最近使用、不改默认”的入口：

```rust
/// 记录一次实际使用的模型（发送/恢复会话）；不改 `defaultModel`。
pub fn remember_model_used(state: &mut AppState, selection: &ModelSelection) {
    if !state.models.remember_model(selection) {
        return;
    }
    state.effects.push(crate::app::effects::Effect::SaveModelChoice {
        default_model: None,
        recent: Some(state.models.settings.recent_models.clone()),
    });
}
```

### 5.4 记录时机（“上次使用”的定义）

| 时机 | 位置 | 动作 |
| --- | --- | --- |
| `/model` 采纳 | `update/chat/slash.rs` `finish_model_selection` 末尾（现有 `persist_default_model`） | 写 `defaultModel` + recent（一次 effect） |
| 发送一轮 | `update/chat/mod.rs` `try_send`：`FrozenModel::resolve` 成功后 | 钉住本轮实际使用的模型（含失效选择回退结果）；随后 `remember_model_used` |
| `/resume` 绑定会话模型 | `update/chat/session.rs` 绑定 `selection` 后 | 校验有效后 `remember_model_used` + 工作区记忆 + 同 session 同步 |
| steer 提升为下一轮 | `update/chat/steer.rs` | 钉住本轮实际使用的模型（`used_selection`）后 `remember_model_used` |
| AI Web 入口 | `try_send_ai_web` | 不参与（`ModelSelection` 是本地模型语义） |

`remember_model_used` 只做内存更新 + 异步落盘，不阻塞发送；写盘失败只 `tracing::warn!`（与现有 `SaveDefaultModel` 一致）。入口统一先用 `validated_selection` 过滤：供应商停用、无 Key、模型不存在或 variant 失效的旧会话模型不得写进 recent；variant 失效会规范化为模型默认等级。

与 opencode 的差异：opencode 的 `recent` 主要记录显式选模型（模型面板/CLI/Go upsell），本实现额外在发送、`/resume`、steer 时记录实际使用。这是 D1/D4“工作区沿用上次使用模型”的产品要求，不是实现错误；`/model` 仍写 `defaultModel`。

### 5.5 新建/切/建工作区与标签的状态刷新

`update/chat/mod.rs` 提供三个粒度：

```rust
/// 启动/配置加载后刷新所有窗格。
pub fn refresh_all_lx_status(state: &mut AppState);

/// 刷新当前工作区当前标签的 lx 窗格（切/建工作区、切标签后调用）。
/// 不遍历后台标签，避免把尚未展示的一次性 omitted_turns_notice 刷新掉。
pub fn refresh_active_tab_lx_status(state: &mut AppState);

/// default/recent 变化后，只刷新“无消息且无窗格选择”的空白 lx 窗格。
pub fn refresh_fallback_lx_status(state: &mut AppState);
```

调用点：

- `switch_workspace`：目标索引变化时刷新当前标签；同索引不刷新。
- `create_workspace_with_cwd`：激活后刷新当前标签。
- `open_worktree_checkout`：激活后刷新当前标签（在 `ensure_main_workspace` 插入前，索引仍正确）。
- `open_new_history_workspace`：`focus_panes` 后刷新当前标签（review 补的遗漏路径）。
- `create_tab` / 右键 `split_pane`：创建后先继承工作区模型，再按新 PaneId 刷新。
- `toggle_pane_view`：终端切回 lx 时刷新目标窗格。
- `switch_tab`：切换标签后刷新新标签，后台标签不再被工作区级刷新波及。
- `create_terminal_in_workspace`：新标签是 Terminal，无需 lx 状态；会继承工作区模型，切回 lx 时由 `toggle_pane_view` 刷新。

`refresh_status` 只读 `ModelsSnapshot`（无 IO）。`switch_workspace` 的既有测试在 models 为空的 `AppState::demo()` 下同样安全（`fallback_selection` 返回 `None`，状态行仍为 `no model`）。

### 5.6 回退链行为总表

| 场景 | 结果 |
| --- | --- |
| 工作区 A 里选过 Claude | A 的 `Workspace.lx_model = Claude`；已有窗格 `chat.selection = Claude`，切走再切回仍是 Claude |
| 在 A 新建标签/分割窗格 | 新窗格继承 `Workspace.lx_model = Claude`，不落到全局 default |
| B 从未用过模型，`defaultModel = GPT` | B 的新窗格显示 GPT（`defaultModel` 第二级） |
| B 从未用过模型，`defaultModel` 为空，最近使用 = `[Claude, GPT]` | B 显示 Claude |
| 最近使用全部失效（供应商被删/未启用/无 Key） | 取第一个可用模型；没有任何可用模型才 `no model` |
| 重启后打开 C（从未用过） | 先 `defaultModel`，无则 `recentModels[0]`，再无则第一个可用 |
| 设置页保存模型配置 | `recentModels` 原样保留（`merge_settings` 基于 current） |

### 5.7 工作区级记忆与新建窗格（review 修复）

- `Workspace` 新增进程内字段 `lx_model: Option<ModelSelection>`；不落库、不跨重启。
- 更新时机：
  - `/model` 采纳：`finish_model_selection` 在写入窗格 selection 后调用 `remember_workspace_model`。
  - 实际发送 / steer 提升：解析出的有效选择写入工作区记忆。
  - `/resume`：会话模型通过 `validated_selection` 后写入工作区记忆。
  - 失效会话模型不写 recent、不写工作区记忆（窗格仍保留原始 selection，供应商恢复后可再次生效）。
- 新窗格继承：`create_tab`、`split_pane`、`create_terminal_in_workspace` 先复制 `Workspace.lx_model` 到新窗格的 `chat.selection`，再刷新状态；新窗格不会停在 `no model`，也不会错误落到全局 default。
- 窗格级 `chat.selection` 仍是更高优先级的覆盖：已有会话、`/model` 手动选择不会被工作区记忆反向覆盖。
- 同一 session 开在多个窗格时，发送 ack 与 `/resume` 通过 `sync_session_selection` 把模型同步到其它窗格，避免 A 已换模型、B 仍显示旧模型。

---

## 6. 影响范围

| 文件 | 改动 |
| --- | --- |
| `src/ui/right_panel/todo.rs` | 几何常量与 helper、`priority_at`、`status_at` 边界、可见性文档注释 |
| `src/ui/right_panel/todo/item.rs` | `group_priority`、连接符字形、`row_view` / `render_item`、独立幽灵行不画徽标 |
| `src/ui/right_panel/todo/tests.rs`、`tests/grouping.rs` | 断言同步、点击端到端、N>3/组尾幽灵/窄面板/边界测试 |
| `docs/standards/tui-design-requirements.md` | Todo 聚合组列位与续行规则、`/model` recent 说明 |
| `src/config/ai/mod.rs`、`src/config/ai/tests.rs` | `recent_models` 字段与往返测试 |
| `src/app/state/lx/chat.rs`、`chat/tests.rs`、`chat/usage.rs`（如需） | 回退链、`validated_selection` variant 规范化、`effective_selection`、`remember_model` 与单测 |
| `src/app/state/workspace.rs`、`src/app/state.rs` | `Workspace.lx_model` 工作区记忆字段 |
| `src/app/effects.rs`、`src/db/service.rs` | `SaveModelChoice` |
| `src/app/update/chat/slash.rs`、`mod.rs`、`session.rs`、`steer.rs` | 记录时机、有效选择过滤、工作区记忆、同 session 同步 |
| `src/app/update/workspace.rs`、`menu.rs`、`pane.rs`、`worktree.rs`、`workspace_history/mod.rs` | 新建/切/建工作区与标签的状态刷新、新窗格继承工作区模型 |
| `docs/feature/todo-group-priority-and-ws-model/review.md` | 本轮审查记录与修复状态 |

不改：数据库 schema（`SCHEMA_VERSION` 保持 28）、TUI 文案（无新增用户可见字符串）、补偿/重试链路。

---

## 7. 风险与回滚

| 风险 | 缓解 |
| --- | --- |
| `ai.json` 新增字段被旧版本二进制覆盖丢失 | 旧版本 `extra` flatten 会原样保留未知字段（`AiSettings.extra`），不会写坏 |
| 最近使用列表让用户“莫名换了模型” | 只在 `defaultModel` 缺失时生效；`defaultModel` 仍由 `/model` 显式写入；`title/recap` 不受影响 |
| 工作区记忆让已有会话被意外覆盖 | 窗格级 `chat.selection` 优先级更高；工作区记忆只用于新建空白窗格 |
| 供应商停用/清 Key 后窗格选择仍显示旧模型 | `effective_selection` 与 fallback 共用 `validated_selection`，失效即回退 |
| 后台标签的一次性省略提示被切工作区清掉 | 只刷当前标签；后台标签在切标签时刷新 |
| 配置持久化并发丢更新 | 已知 P3 风险（`review.md` P3-6/P3-8）；当前单实例顺序 effect 正常，后续用文件锁/宽松解析加固 |
| 切工作区刷新状态行拖慢切换 | 纯内存遍历，无 IO；单测覆盖 |
| Todo 列位调整导致鼠标命中错位 | 渲染与命中共用同一组 helper（`row_priority_x` / `row_branch_x` / `row_line_x_for`），`priority_at` 走新几何 |
| 组内最高优先级来自别的行，点击改到别的条目 | 决策 D5 已确认；徽标配色沿用锚点状态，视觉归属清晰 |
| 聚合组续行宽度变窄影响折行高度 | 通过 `viewport_metrics` 同一口径计算；更新 grouping 折行测试 |

回滚方式：纯代码改动，`git revert` 对应提交即可；`ai.json` 多出的 `recentModels` 对旧代码无副作用（`extra` 保留）。

---

## 8. 验证计划

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`（重点：`ui::right_panel::todo`、`app::state::lx::chat`、`app::update::chat`、`app::update::workspace_history`、`config::ai`、`db::service`）
4. 手工（用户执行）：`cargo run` 检查聚合组三种形态（2/3 条、折行、编辑态、点击徽标）与多工作区模型切换/重启。
5. Review 修复重点回归：新标签/分割窗格继承工作区模型、历史列表新建工作区立即显示模型、停用供应商/清 Key 后回退到可用模型、同一 session 多窗格同步、组徽标点击端到端。

自动化结果与已知边界以 [review.md](./review.md) §6 与 [verification.md](./verification.md) 为准。
