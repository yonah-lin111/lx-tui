# Prompt 顶部控件换位、页面标题展示与底部 Recent Pages 抽屉设计

- 状态：**已实现（待合并）**（2026-11-02）
- 适用范围：`src/input/`、`src/event/`、`src/layout/prompt.rs`、`src/db/`、`src/app/prompt/`、`src/app/update/`、`src/ui/prompt/`、`src/ui/prompt_frame.rs`、`src/ui/text.rs`、`src/ui/tooltip.rs`、`src/platform/environment.rs`。
- 架构约束：
  - 保持 `app/`（纯状态与更新）、`ui/`（纯渲染只读状态）、`layout/`（纯几何计算）与 `db/`（SQLite 持久化）边界清晰；
  - 所有用户可见界面文案与 Tooltip 统一使用英文并集中定义在 `src/ui/text.rs`；
  - 生产代码严禁 `unwrap()` / `expect()`，单文件行数不超过 1000 行（拆分子模块控制在 500 行左右）。

---

## 1. 背景与目标

针对 `src/ui/prompt/` 及其关联状态/布局模块，本次迭代包含以下三项核心改造：

1. **顶部两行表头控件换位（工具栏行 ↔ 分割线标题行）**：
   - 将原位于第 2 行（分割线标题行 `title_row`）右侧的**切换页面控件（`‹ n/m ›` / `n/m`）**上移至第 1 行（工具栏行），放置在 `[refDir]` 按钮与保存状态圆点 `●` 的**中间位置**（即右组从左到右为 `[refDir] ‹ n/m › ●`）。
   - 将原位于切换页面控件左侧的**模板块状态按钮（`[todo]` / `[run]` / `[done]`）**移动到原来切换页面控件所在的第 2 行（分割线标题行）最右侧（距右边框 `┤` 留 1 列 `─`）。
   - 在第 1 行工具栏左侧紧跟模式切换按钮（`[≡]` / `[/]`）之后新增 **`[rec]`** 按钮（在 `Tree`、`Editor`、`Pages` 三种模式下常显），并绑定快捷键 **`Ctrl/Cmd + r`**，用于开关底部 Recent Pages 抽屉。

2. **分割线标题行展示当前页面 Title（`prompt名称 · 当前页面title`）**：
   - 在 `PromptMode::Editor` 下，左侧标题行在提示词条目名称右侧添加当前页面的显示标题，使用 ` · ` 分隔（例如 ` API Design · Auth Flow `）。
   - 页面显示标题解析规则与 `Pages` 列表统一：优先提取当前页首个闭合 `&&&` 模板块的 `{title: xxx}` 标题，未定义时回退为 `page.name`（如 `Page 1`）。

3. **底部 Recent Pages 抽屉（支持全部视图模式打开、双行条目、独立滚动条与单击跳转）**：
   - 在 `Tree`、`Editor`、`Pages` 任一模式下，可通过点击顶部 `[rec]` 按钮或按快捷键 `Ctrl/Cmd + r` 在 Prompt 面板下方展开/收起类似抽屉的 `recent` 区域（不覆盖全屏，上方原视图按剩余高度自适应收缩）。
   - 抽屉内展示当前工作区所有已保存提示词（`PromptItem`）下的 Page，按页面最后修改时间 `updated_at` **全局倒序排列**（同一个 Prompt 若有多个最近修改的 Page，均各自作为独立条目按最新更新顺序显示，上限 20 条）。
   - 每个条目占 **2 行**：
     - 第 1 行：`prompt名称`（左对齐，当前正在编辑的页前置 `❯ ` 并加粗） + `[时间]`（右对齐，如 `[now]`、`[5m]`、`[2h]`、`[3d]`、`[10-15]`）。
     - 第 2 行：`最新的page名称`（左对齐，带 2 列缩进） + `状态小圆点`（右对齐，`Todo` 为灰色 `○`，`InProgress`/`Done` 为黄/绿 `●`，无模板块则留空）。
   - 抽屉支持独立垂直滚动条 `▐` 与鼠标滚轮滚动；鼠标悬停时双行同时高亮，左键单击任意条目即立即打开该 Prompt 的对应 Page 进入编辑视图（抽屉保持展开）。

---

## 2. 已确认的边界决策汇总（Grill Me 结论）

| 决策项 | 确认方案 |
| --- | --- |
| **1. `recent` 面板形态与触发入口** | 1. **入口按钮**：顶部工具栏左侧紧跟 `[≡]`/`[/]` 新增 `[rec]` 按钮（`Tree`/`Editor`/`Pages` 三种模式下均显示）。<br>2. **快捷键**：Prompt 聚焦时按 `Ctrl/Cmd + r` 切换抽屉展开/收起。<br>3. **展示形态**：在 Prompt 面板底部以**抽屉（Drawer）**形式出现，顶部带 `├─ recent ───┤` 分割线，上方 `Tree` / `Editor` / `Pages` 内容区高度相应收缩，与抽屉上下共存。 |
| **2. 抽屉高度与点击行为** | 1. **高度自适应与上限**：抽屉最大高度为 **7 行**（1 行 `├─ recent ───┤` 分割线 + 最多同时显示 **3 个双行条目** = 6 行内容），且不超过 `prompt_body_rect` 总高度的一半；若条目数为 1 或 2，则高度自适应为 3 行或 5 行；无记录时显示 1 行分割线 + 1 行空态 `no recent pages`（共 2 行）。<br>2. **点击跳转**：鼠标左键**单击**某条记录，立即提交当前未保存修改、切换到 `Editor` 模式并定位到对应的 `Prompt + Page`；跳转后**抽屉保持展开**。 |
| **3. 数据范围、排序、时间格式与状态点** | 1. **数据范围与排序**：取当前工作区所有已入库 `PromptItem` 的全部 `PromptPage`（不含 `tmp` 临时草稿），按每个 Page 的 `updated_at`（Unix 秒数）全局倒序排列（相同时按 `page.id` 倒序），最多取前 **20 条**。保存或新建 Page 时实时刷新内存与数据库的 `updated_at`。<br>2. **`[时间]` 格式**：极简英文相对时间：`[now]`（`< 60s`）、`[Nm]`（`1..=59m`）、`[Nh]`（`1..=23h`）、`[Nd]`（`1..=29d`）、`[MM-DD]`（`>= 30d`）。<br>3. **Page 名称与状态圆点**：复用 `page_title_status(&page.content)`，优先取 `{title: xxx}` 否则回退 `page.name`；有 `&&&` 模板块时绘制 `○`（Todo）或 `●`（InProgress/Done），无模板块不画圆点。 |
| **4. 顶部两行表头控件排布与页面 Title** | 1. **第 1 行（工具栏行）**：<br>   - `Editor`：左组 `[≡] [rec] [←] [→]`，右组 `[refDir] ‹ n/m › ●`（`‹ n/m ›` 位于 `[refDir]` 与 `●` 中间，间隔 1 列）。窄屏空间不足时按 `[refDir]` → `[rec]` → `[←] [→]` 顺序隐藏。<br>   - `Tree`：左组 `[/] [rec]`，右组 `[tmp]`。<br>   - `Pages`：左组 `[/] [rec]`，右组在与 `Editor` 对应的页码位置显示无箭头的 `n/m`（点击 `n/m` 切回 `Editor`）。<br>2. **第 2 行（分割线标题行 `title_row`）**：<br>   - `Editor`：左侧显示 ` prompt名称 · 当前页面title `（实时从当前编辑器缓冲 `state.prompt.text()` 提取 `{title:}` 或回退 `page.name`）；右侧原页控件位置（`area.right() - 2` 向左）放置模板块状态按钮 `[todo]`/`[run]`/`[done]`。<br>   - `Pages`：左侧显示 ` prompt名称 · pages `，右侧为纯分割线 `─`。 |
| **5. 焦点配合与双行悬停** | 1. **不抢占键盘焦点**：展开 `recent` 抽屉后，键盘焦点仍留在上方视图（`Editor` 正常打字、`Tree`/`Pages` 正常上下移动和搜索）。<br>2. **高亮与悬停**：抽屉中与当前正在编辑的 `(PromptItem.id, PromptPageId)` 匹配的条目在第 1 行左侧显示 `❯ ` 且标题加粗；鼠标悬停在某条目的第 1 行或第 2 行时，该条目的**两行同时铺选中底色**。 |

---

## 3. 核心架构与数据流

```text
┌────────────────────────────────────────────────────────────────────────────────────┐
│ 1. 顶部两行表头几何与渲染链路                                                      │
│   Row 1 (Toolbar):                                                                 │
│     Left : [≡]/[/] (Mode) -> [rec] (Recent) -> [←] (Undo) -> [→] (Redo)            │
│     Right: [refDir] (Folders) -> ‹ n/m › (PageControls) -> ● (SaveDot)             │
│   Row 2 (Divider / Title Row):                                                     │
│     Left : ├─ {prompt_name} · {active_page_title} ──                               │
│     Right: ─────────────────────────── [todo]/[run]/[done] ─┤                      │
├────────────────────────────────────────────────────────────────────────────────────┤
│ 2. Recent Pages 时间戳与派生数据流                                                 │
│   SQLite `workspace_prompt_page.updated_at` (strftime('%s', updated_at))           │
│     ──► db::PromptPageRecord { id, position, name, content, updated_at }           │
│     ──► app::prompt::PromptPage { id, name, content, updated_at }                  │
│     ──► 编辑保存 / 切换 / 新建页时刷新 `page.updated_at = current_unix_secs()`      │
│     ──► app::prompt::recent::recent_page_rows(panel, current_editor, now_secs)     │
│           └─ 收集所有 PromptItem 的全部 Page ──► 按 updated_at DESC 排序 (Top 20)   │
├────────────────────────────────────────────────────────────────────────────────────┤
│ 3. 底部 Recent 抽屉布局、滚轮与点击链路                                            │
│   Ctrl/Cmd + R 或点击工具栏 `[rec]`                                                │
│     ──► update::toggle_prompt_recent(state)                                        │
│           ├─ 切换 `prompt_panel.recent_open = !recent_open`                        │
│           └─ 同步重算上方编辑器视口高度 (`sync_prompt_Editor_size`)                │
│   layout::prompt_split_body(panel, recent_entry_count):                            │
│     ├─ 上方主视图区域 (`main_text_rect` + `main_scrollbar_rect`)                   │
│     └─ 下方抽屉区域 (`recent_divider_rect` + `recent_list_rect` + `scrollbar`)     │
│   鼠标命中 (`event::mouse`):                                                       │
│     ├─ Hover  ──► `state.prompt_recent_hover = Some(entry_index)` (双行同时高亮)   │
│     ├─ Wheel  ──► 指针在抽屉内时滚动 `prompt_panel.recent_scroll`                  │
│     └─ Click  ──► `update::open_recent_page(state, index)` ──► 切入对应 Prompt+Page│
└────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细技术设计与关键代码

### 4.1 数据库与内存模型：`PromptPage` 暴露 `updated_at` 时间戳

#### 关键点
1. **数据库层（`src/db/mod.rs`）**：
   - `workspace_prompt_page` 表已原生包含 `updated_at TEXT NOT NULL` 列（默认存入 `datetime('now')` UTC 时间），**无需升级 `SCHEMA_VERSION` 或做表迁移**。
   - 在 `PromptPageRecord` 中新增 `pub updated_at: i64`（Unix 秒数）。
   - 在 `Database::list_prompts` 查询 `workspace_prompt_page` 时，通过 SQLite 内置函数 ` COALESCE(CAST(strftime('%s', pg.updated_at) AS INTEGER), 0)` 直接读取 Unix 秒数，既高效又避免在 Rust 层重复解析日期字符串。
2. **平台时间辅助（`src/platform/environment.rs` & `src/platform/mod.rs`）**：
   - 新增纯函数 `pub fn current_unix_secs() -> i64` 与相对时间格式化函数 `pub fn format_relative_time(updated_secs: i64, now_secs: i64) -> String`：
     - `diff < 60` → `"[now]"`
     - `diff < 3600` → `format!("[{}m]", diff / 60)`
     - `diff < 86_400` → `format!("[{}h]", diff / 3600)`
     - `diff < 86_400 * 30` → `format!("[{}d]", diff / 86_400)`
     - `diff >= 86_400 * 30` → 调用现有 `civil_from_days` 计算出公历月日，返回 `format!("[{month:02}-{day:02}]")`。
3. **内存模型（`src/app/prompt/tree.rs`）**：
   - `PromptPage` 新增字段 `pub updated_at: i64`。
   - `PromptPage::saved(id, name, content)` 默认将 `updated_at` 设为 `0`，并提供 `PromptPage::saved_at(id, name, content, updated_at)`；当页面在内存中被编辑修改（`commit_editing_prompt` 内容实际发生变化、或 `cycle_page_template_status` 修改状态、或新建/Fork 页）时，将 `page.updated_at` 更新为 `crate::platform::current_unix_secs()`。

#### 关键代码

```rust
// src/db/mod.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptPageRecord {
    pub id: i64,
    pub position: i64,
    pub name: String,
    pub content: String,
    /// 最后修改时间（UTC Unix 秒数）。
    pub updated_at: i64,
}

// Database::list_prompts 中查询页记录：
let mut pages_statement = self.conn.prepare(
    r#"SELECT pg.prompt_id, pg.id, pg.position, pg.name, pg.content,
              COALESCE(CAST(strftime('%s', pg.updated_at) AS INTEGER), 0)
       FROM workspace_prompt_page pg
       JOIN workspace_prompt p ON p.id = pg.prompt_id
       WHERE p.workspace_id = ?1
       ORDER BY pg.prompt_id, pg.position"#,
)?;
```

```rust
// src/platform/environment.rs
/// 当前 UTC Unix 秒数；系统时钟异常时返回 0。
pub fn current_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// 格式化简短相对时间标签（如 `[now]`、`[5m]`、`[2h]`、`[3d]`、`[10-15]`）。
pub fn format_relative_time(updated_secs: i64, now_secs: i64) -> String {
    if updated_secs <= 0 {
        return "[now]".to_string();
    }
    let diff = now_secs.saturating_sub(updated_secs).max(0);
    if diff < 60 {
        "[now]".to_string()
    } else if diff < 3600 {
        format!("[{}m]", diff / 60)
    } else if diff < 86_400 {
        format!("[{}h]", diff / 3600)
    } else if diff < 86_400 * 30 {
        format!("[{}d]", diff / 86_400)
    } else {
        let days = updated_secs.div_euclid(86_400);
        let (_, month, day) = civil_from_days(days);
        format!("[{month:02}-{day:02}]")
    }
}
```

---

### 4.2 顶部两行表头重排：工具栏 `[rec]` + 页控件 `‹ n/m ›`、标题行 `· PageTitle` + 模板状态按钮

#### 关键点
1. **工具栏按钮与页控件几何（`src/layout/prompt.rs` & `src/ui/prompt/header.rs`）**：
   - `PromptToolbarButton` 新增 `Recent`（文案 `PROMPT_RECENT_LABEL = "[rec]"`，宽 5 列）。
   - **左组按钮坐标**：
     - `Mode` (`[≡]` / `[/]`)：`x = bar.x`，宽 3 列。
     - `Recent` (`[rec]`)：`x = bar.x + 4`，宽 5 列（当面板内宽能够容纳时显示）。
     - `Undo` (`[←]`)：`x = recent_right + 1`（若 `Recent` 隐藏则紧跟 `Mode`），宽 3 列。
     - `Redo` (`[→]`)：`x = undo.right() + 1`，宽 3 列。
   - **右组控件坐标（从右往左）**：
     - **Editor 模式**：
       - 最右 1 列：`Save` (`●` / `○`)，`x = bar.right() - 1`，宽 1 列。
       - 保存点左侧（隔 1 列）：页切换控件 `‹ n/m ›`（`next` 在 `save.x - 2`，`counter` 在 `next.x - 1 - counter_width`，`prev` 在 `counter.x - 2`）。
       - 页切换控件左侧（隔 1 列）：`Folders` (`[refDir]`，宽 8 列)，`x = prev.x - 1 - PROMPT_FOLDERS_WIDTH`。
     - **Pages 模式**：
       - 无 `Save` 圆点与 `[refDir]`，页码计数 `n/m`（无左右箭头）放置在工具栏右侧 `bar.right() - 1 - counter_width` 处，点击该 `n/m` 直接触发 `toggle_prompt_pages` 返回 `Editor`。
     - **Tree 模式**：
       - 左组 `[/] [rec]`，右组 `[tmp]`（`bar.right() - 1 - 5`）。
   - **窄屏裁剪优先级（工具栏行）**：
     - 最高优先级保留 `Mode`、`Save` 与页控件 `‹ n/m ›`；
     - 宽度不足时先隐藏 `[refDir]`，再隐藏 `[rec]`，再隐藏 `[←] [→]`，最后隐藏页控件。
2. **分割线标题行（`src/ui/prompt_frame.rs` & `src/ui/prompt/header.rs`）**：
   - 在 `prompt_frame.rs` 的 `prompt_page_header(state)` 中：
     - 当处于 `PromptMode::Editor` 时，从当前实时文本 `state.prompt.text()` 调用 `page_title_status` 提取块标题，若为 `None` 则取 `pages[index].name`（为空时兜底 `default_page_name(index + 1)`），赋给 `PromptPageHeader.name`。
     - 由此 `border_text` 会自动渲染出 ` {prompt_name} · {page_title} `（`prompt_name` 用 `border_title` 色，`· {page_title}` 用 `muted_on_border` 色）。
   - 模板块状态按钮 `[todo]` / `[run]` / `[done]` 的 `status_rect_in(area, status)`：
     - 直接右对齐到分割线标题行原来的页控件位置：`right = area.right().saturating_sub(2)`（即右边角 `┤` 左数留 1 列 `─`），`x = right.checked_sub(width)`。
     - 左侧标题区 `title_text_rect` 的右边界直接避让 `status_rect.x - 1`（无状态按钮时延伸到 `area.right() - 1`）。

#### 关键代码

```rust
// src/ui/prompt_frame.rs
pub(crate) fn prompt_page_header(state: &AppState) -> Option<PromptPageHeader> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    let (pages, active) = panel.editing_pages()?;
    match panel.mode {
        PromptMode::Tree => None,
        PromptMode::Editor => {
            let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
            let live_text = state.prompt.text();
            let (block_title, _) = crate::app::prompt::page_title_status(live_text);
            let page_title = block_title.unwrap_or_else(|| {
                let trimmed = pages.get(index).map(|p| p.name.trim()).unwrap_or("");
                if trimmed.is_empty() {
                    text::default_page_name(index + 1)
                } else {
                    trimmed.to_string()
                }
            });
            Some(PromptPageHeader {
                current: index + 1,
                total: pages.len(),
                name: page_title,
                arrows: true,
            })
        }
        PromptMode::Pages => {
            let rows = page_rows(pages, active, panel.pages_view.search.text());
            let current = rows
                .get(panel.pages_view.selected)
                .and_then(|row| pages.iter().position(|page| page.id == row.page))
                .map(|index| index + 1)
                .unwrap_or(0);
            Some(PromptPageHeader {
                current,
                total: pages.len(),
                name: text::PROMPT_PAGES_LABEL.to_string(),
                arrows: false,
            })
        }
    }
}
```

```rust
// src/ui/prompt/header.rs
/// 工具栏行页控件几何：
/// - `arrows == true`（Editor 模式）：位于保存圆点 `●` 左侧，排布 `‹ n/m ›`；
/// - `arrows == false`（Pages 模式）：位于工具栏右侧，排布 `n/m`。
pub fn toolbar_page_controls(
    panel: Rect,
    counter: &str,
    arrows: bool,
) -> Option<PromptPageControls> {
    let bar = crate::layout::prompt_toolbar_rect(panel)?;
    let counter_width = u16::try_from(UnicodeWidthStr::width(counter)).ok()?;
    let min_left = bar.x.saturating_add(4); // 至少留出左侧 [≡] + 1 列间隔
    if arrows {
        // 最右 1 列为保存点 `●` (bar.right() - 1)，再左 1 列空格，再左为 `›`
        let next = Rect::new(bar.right().checked_sub(3)?, bar.y, 1, 1);
        let counter_x = next.x.checked_sub(1)?.checked_sub(counter_width)?;
        let counter_rect = Rect::new(counter_x, bar.y, counter_width, 1);
        let prev = Rect::new(counter_rect.x.checked_sub(2)?, bar.y, 1, 1);
        (prev.x >= min_left).then_some(PromptPageControls {
            prev,
            counter: counter_rect,
            next,
        })
    } else {
        let right_edge = bar.right().checked_sub(1)?;
        let counter_x = right_edge.checked_sub(counter_width)?;
        let counter_rect = Rect::new(counter_x, bar.y, counter_width, 1);
        let prev = Rect::new(counter_x, bar.y, 0, 1);
        let next = Rect::new(right_edge, bar.y, 0, 1);
        (counter_x >= min_left).then_some(PromptPageControls {
            prev,
            counter: counter_rect,
            next,
        })
    }
}

/// 分割线标题行上的模板块状态按钮：置于原切换页控件所在的右侧位置（距右边框 `┤` 留 1 列 `─`）。
fn status_rect_in(area: Rect, status: TemplateStatus) -> Option<(Rect, Style)> {
    let label = text::template_status_label(status);
    let width = u16::try_from(UnicodeWidthStr::width(label)).ok()?;
    let right = area.right().saturating_sub(2);
    let x = right.checked_sub(width)?;
    // 左侧至少留出 `├─ ` + 2 列标题空间
    (x >= area.x.saturating_add(5)).then(|| {
        (
            Rect::new(x, area.y, width, 1),
            style::template_border(status),
        )
    })
}
```

---

### 4.3 底部 Recent Pages 抽屉：派生数据、几何分割、渲染与交互

#### 关键点
1. **Recent Pages 派生模型（新建 `src/app/prompt/recent.rs`）**：
   - `MAX_RECENT_PAGES: usize = 20`。
   - 定义 `RecentPageRow`：
     ```rust
     #[derive(Debug, Clone, PartialEq, Eq)]
     pub struct RecentPageRow {
         pub item_id: i64,
         pub page_id: PromptPageId,
         pub prompt_name: String,
         pub page_title: String,
         pub status: Option<TemplateStatus>,
         pub updated_at: i64,
         pub time_label: String,
         pub current: bool,
     }
     ```
   - `pub fn recent_page_rows(panel: &PromptPanel, live_editor: Option<(PromptEditing, PromptPageId, &str)>, now_secs: i64) -> Vec<RecentPageRow>`：
     - 遍历 `panel.tree.items()` 下的每个 `PromptItem` 及其 `pages`；
     - 若某页恰好是当前正在编辑的页（`live_editor` 匹配），用实时编辑缓冲文本计算 `page_title_status`，否则用 `page.content` 计算；
     - 按 `(row.updated_at, page_id)` **降序排序**，截断保留前 20 条。
2. **PromptPanel 抽屉状态（`src/app/prompt/tree/panel.rs` & `src/app/state.rs`）**：
   - `PromptPanel` 新增：
     - `pub recent_open: bool`（默认 `false`）
     - `pub recent_scroll: usize`（以**条目数**为单位的滚动偏移，每条占 2 行）
   - `AppState` 新增：
     - `pub prompt_recent_hover: Option<usize>`（鼠标悬停的 recent 条目索引）
3. **几何分割计算（`src/layout/prompt.rs`）**：
   - 定义 `PromptBodySplit`：
     ```rust
     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
     pub struct PromptBodySplit {
         /// 上方主视图区域（含最右 1 列滚动条槽）。
         pub main_body: Rect,
         /// 抽屉顶部分割线行（`├─ recent ───┤`，横跨面板外框 `panel.x..panel.right()`，高 1 行）。
         pub recent_divider: Option<Rect>,
         /// 抽屉内部区域（不含最右 1 列滚动条槽，高为 `2 * visible_entries` 或空态 1 行）。
         pub recent_text: Option<Rect>,
         /// 抽屉右侧滚动条槽（最右 1 列）。
         pub recent_scrollbar: Option<Rect>,
     }
     ```
   - 高度计算规则：
     - 当 `!recent_open` 时，`main_body = prompt_body_rect(inner)`，其余为 `None`。
     - 当 `recent_open` 时：
       - 若 `recent_count == 0`：抽屉需要 `1（分割线）+ 1（空状态行）= 2 行`；
       - 若 `recent_count > 0`：最多展示 `visible_entries = recent_count.min(3)` 条，抽屉高度 `drawer_h = 1 + (visible_entries as u16) * 2`（即 3、5 或 7 行）；
       - 同时限制 `drawer_h <= body.height / 2`（向下对齐到 `1 + 2 * k` 行，若連 1 条即 3 行都放不下则不展开抽屉），确保上方主视图永远至少保有一半高度。
   - **重要联动**：当 `recent_open` 切换或条目数变化导致上方 `main_body` 高度变化时，通过 `prompt_text_rect_with_drawer` 同步更新 `state.prompt.set_size(w, h)`，使编辑器的软换行与光标滚动自动适应收缩后的高度。
4. **抽屉渲染（新建 `src/ui/prompt/recent.rs`）**：
   - **分割线标题行**：调用 `render_rule(panel, divider.y, buf, focused)` 画 `├────┤`，并在 `panel.x + 2` 处用 `style::muted_on_border()` 覆盖绘制 ` recent `。
   - **双行条目绘制**：
     - 每个可见条目 `row` 占用 `y` 和 `y + 1` 两行：
     - 若 `hovered`（`prompt_recent_hover == Some(idx)`），对 `y` 和 `y + 1` 两行整行调用 `fill_row(buf, area.x, y, area.width)` 铺选中底色。
     - **第 1 行（`y`）**：
       - 左起 2 列：`row.current` 为真时画 `❯ `（`style::selection_marker()`），否则画 `"  "`；
       - 右端贴边绘制 `row.time_label`（如 `[5m]`，样式 `style::muted()`，悬停时为 `style::selection()`）；
       - 中间绘制 `row.prompt_name`（`row.current` 时 `style::strong()`，否则 `style::text()`），超宽时在时间标签前留 1 列间隔并截断加 `…`。
     - **第 2 行（`y + 1`）**：
       - 左起 2 列固定缩进 `"  "`；
       - 右端：若 `row.status` 为 `Some(status)`，贴右缘 1 列绘制状态小圆点（`Todo` 为 `○` + `style::muted()`；`InProgress`/`Done` 为 `●` + `style::template_border(status)`）；
       - 中间绘制 `row.page_title`（样式 `style::muted()`，让第 1 行 Prompt 名与第 2 行 Page 名主次分明），超宽时在圆点前截断加 `…`。
5. **快捷键与鼠标事件（`src/input/mod.rs`、`src/event/mouse/`、`src/app/update/prompt/`）**：
   - `src/input/mod.rs`：`EditorCommand` 新增 `ToggleRecentPages`，在 `editor_command` 中将 `Ctrl/Cmd + r`（不带 Shift/Alt）映射为 `EditorCommand::ToggleRecentPages`。
   - `src/event/mouse/press.rs`：
     - 点击工具栏 `PromptToolbarButton::Recent` → 调用 `update::toggle_prompt_recent(state)`；
     - 点击 `recent` 抽屉内条目 → 调用 `update::open_recent_page(state, entry_index)`（先提交当前未保存修改，将 `panel.editing` 绑定为 `PromptEditing::Item(row.item_id)`，将该 Item 的 `active_page` 设为 `row.page_id` 并持久化 `Effect::SetActivePage`，加载该页正文到编辑器，切换 `panel.mode = PromptMode::Editor`，保持 `recent_open = true`）。
   - `src/event/mouse/wheel.rs` & `drag.rs` & `motion.rs`：
     - 鼠标位于 `recent` 抽屉矩形内时，滚轮滚动 `panel.recent_scroll`；
     - 拖拽 `recent` 抽屉右侧滚动条时，按条目总数更新 `panel.recent_scroll`；
     - 鼠标移动时更新 `state.prompt_recent_hover`。

#### 关键代码

```rust
// src/ui/prompt/recent.rs
pub fn render(
    panel: Rect,
    divider: Rect,
    area: Rect,
    buf: &mut Buffer,
    rows: &[RecentPageRow],
    scroll: usize,
    focused: bool,
    hover: Option<usize>,
) {
    // 1. 绘制顶部 `├─ recent ─────────┤` 分割线
    super::render_rule(panel, divider.y, buf, focused);
    let label = text::PROMPT_RECENT_SECTION_LABEL; // " recent "
    let label_x = divider.x.saturating_add(2);
    if label_x.saturating_add(label.width() as u16) < divider.right() {
        buf.set_stringn(
            label_x,
            divider.y,
            label,
            label.width(),
            style::muted_on_border(),
        );
    }

    if area.width == 0 || area.height == 0 {
        return;
    }
    if rows.is_empty() {
        let empty = text::PROMPT_RECENT_EMPTY; // "no recent pages"
        let x = area.x.saturating_add(2);
        let max_w = usize::from(area.right().saturating_sub(x));
        buf.set_stringn(x, area.y, empty, max_w, style::muted());
        return;
    }

    let visible_entries = usize::from(area.height) / 2;
    let start = scroll.min(rows.len().saturating_sub(visible_entries.max(1)));
    for (offset, row) in rows.iter().enumerate().skip(start).take(visible_entries) {
        let y1 = area.y + (offset - start) as u16 * 2;
        let y2 = y1 + 1;
        let hovered = hover == Some(offset);
        paint_recent_entry(buf, area.x, y1, y2, area.width, row, hovered);
    }
}
```

---

## 5. 影响范围与风险控制

1. **数据库向后兼容性**：
   - 直接读取现有 `workspace_prompt_page.updated_at` 列并转换为 Unix 秒数，不改变表结构、不新增迁移版本，旧数据库 100% 无缝兼容。
2. **编辑器软换行与光标坐标同步**：
   - 抽屉展开/收起会改变上方编辑器的可用高度 `prompt_text_rect`。在每次 `toggle_prompt_recent`、打开/删除页面或窗口缩放时，统一通过 `sync_prompt_viewport_size(state)` 用扣除抽屉后的高度调用 `state.prompt.set_size(w, h)`，确保光标始终在可见视口内，不会出现在抽屉背后。
3. **窄屏与矮屏防呆**：
   - 当右侧面板高度过矮（例如总内高 `< 6` 行，扣除抽屉后上方不足 3 行）时，自动压制抽屉渲染，优先保障上方编辑器/列表可用；
   - 当右侧面板宽度较窄时，工具栏按 `[refDir]` → `[rec]` → `[←]/[→]` 渐进隐藏，绝不发生字符重叠或越界 panic。

---

## 6. 实现落点（已落地）

| 能力 | 实际落点 |
| --- | --- |
| 时间戳读取 | `PromptPageRecord.updated_at` + SQL `strftime('%s', pg.updated_at)`；`PromptPage.updated_at` / `saved_at` / `touch` |
| 相对时间 | `platform::current_unix_secs` / `format_relative_time`（`[now]`/`[Nm]`/`[Nh]`/`[Nd]`/`[MM-DD]`） |
| 页显示标题 | `app::prompt::page_display_title`（`{title:}` → 页名 → `Page N`），Pages 与标题行、抽屉共用 |
| Recent 派生 | `app::prompt::recent_page_rows`（跨条目按 `updated_at` 倒序，同刻按页 id 倒序，上限 20） |
| 抽屉几何 | `layout::prompt_body_split`（最多 3 条/7 行，不超过内容区一半，空间不足逐条减少或收起） |
| 抽屉渲染 | `ui::prompt::recent::render`（`RecentRender` 输入：分割线、双行条目、状态点、双行悬停） |
| 抽屉行为 | `update::toggle_prompt_recent` / `open_recent_page` / `scroll_prompt_recent` / `set_prompt_recent_scroll` |
| 工具栏页控件 | `layout::toolbar_page_controls`（Editor 带箭头、Pages 仅 `n/m`）；`render_header` 内绘制 |
| 标题行 | 左侧 `条目名 · 页名`；右侧状态标签 `[todo]/[run]/[done]`（`ui::prompt::header::paint_title_row`） |
| 视口同步 | `AppState.prompt_resize_pending` + `event::resize_panes` 按 `prompt_body_split().main` 更新编辑器尺寸 |
| 鼠标 | 工具栏 `[rec]` 点击、抽屉条目单击、双行悬停、抽屉独立滚动条点击/拖拽、滚轮不穿透 |
| 快捷键 | `EditorCommand::ToggleRecentPages` ← `Ctrl/Cmd+R`（Tree/Editor/Pages 均生效） |

### 测试覆盖

- `app::prompt::recent::tests`：跨条目排序、同刻次序、实时标题/状态、空名回退、20 条上限、仅统计已入库页。
- `app::update::prompt::recent::tests`：开关与滚动钳制、单击切换且抽屉保持展开、旧页正文先提交。
- `layout::tests`：抽屉高度上限/半屏压缩/空态行。
- `ui::prompt::recent::tests`：双行渲染、时间/状态点、双行悬停、空态与滚动窗口。
- `ui::prompt::tests` / `ui::prompt::pages::tests`：标题行无页控件、工具栏页控件命中、状态标签右对齐、窄屏降级。
- `event::tests::prompt_recent`：三种模式 Ctrl+R、`[rec]` 点击、条目单击跳转、悬停清除、滚轮不穿透。
- `db::tests`：`PromptPageRecord.updated_at` 读取与正文更新后递增。
- `tests/prompt_border_title.rs`：真实渲染下的两行表头、页控件位置、箭头置灰与标题截断。
