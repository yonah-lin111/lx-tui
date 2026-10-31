# 右侧面板 Todo 日程增强与多图表统计 — 技术设计

- 任务目录：`docs/feature/right-panel-todo-schedule/`
- 目标路径：`src/db/todo.rs`, `src/app/state/todo.rs`, `src/app/update/todo.rs`, `src/ui/right_panel/todo.rs`, `src/ui/style/todo.rs`
- 状态：设计完成，等待用户审核确认后执行。

---

## 1. 背景与目标

当前 `lx-tui` 的 Todo 面板为一个简易的全局平铺待办列表，所有条目混在一起，缺少时间维度与任务优先级管理。
参考 `lx-agent`（`/Users/yonah/projects/agent/lx-agent/src/renderer/src/features/schedule/`）成熟的日程/待办模型与工作流，为 `lx-tui` 的 Todo 面板进行全面增强：

1. **日期存储（Date Storage）**：
   - 待办由全局平铺改为**按日期（YYYY-MM-DD）组织**，每个日期拥有独立的有序待办清单；
   - 记录 `entry_date`（归属日期）、`completed_date`（完成日期）及 `priority`（P0 / P1 / P2 / P3 优先级四档）。
2. **未完成顺延到今天（Rollover Uncompleted to Today）**：
   - 当用户查看“今天”的待办时，自动探测历史过去日期（`< 今天`）所有未完成的事项；
   - 在列表顶部展示紧凑的 **Rollover Banner** 提示条：显示历史未完成数量，提供 `[Roll to Today]`（一键将它们移动到今天）与 `[✕]`（忽略本次提醒）。
3. **一周快速点击感知带（Week Strip）**：
   - 位于工具栏下方、列表上方，展示周一至周日 7 天的快速切日栏；
   - 包含 `‹` 与 `›` 快速翻周按钮，标注日期与星期（Mon..Sun），当日显示特殊标记（如实心小圆点），选中的日期全宽反显/高亮，并附带微型完成度指示（如 `1/3` 或纯净圆点）。
4. **一键智能排序（Sort by Priority）**：
   - 在顶部工具栏新增 `[sort]` 按钮；
   - 排序规则严格对齐 `lx-agent`：**未完成在前 → 优先级 P0 > P1 > P2 > P3 → 原位置顺序 → ID 稳定兜底**。
5. **优先级徽标与可读性样式（Priority Chip）**：
   - 条目行内增加 `P0` / `P1` / `P2` / `P3` 徽标；
   - 支持鼠标单击切换（`P0 → P1 → P2 → P3 → P0`）与键盘 `p` 键切换；
   - 各优先级具备专属背景色与前景色，且背景绝不影响文本字体可读性。
6. **多图表统计（Multiple Stats Charts & Active Toggle）**：
   - 底部统计区一次仅展示一个图表，通过统计顶部的切图控件 `‹ 图表名 (1/3) ›` 或快捷键进行主动切换；
   - 提供 3 种高价值图表：
     1. **Chart 1: 三态与完成率（Status & Rate）**：展示当前查看日的 open / doing / done 数量与柱图，附带完成百分比；
     2. **Chart 2: 优先级分布（Priority Breakdown）**：展示 P0 / P1 / P2 / P3 柱图分布与占比；
     3. **Chart 3: 近 7 日趋势（7-Day Trend）**：展示以当前查看日为终点、最近 7 天的每日“计划任务数 vs 完成任务数”双柱对比。

---

## 2. Grill Me 确认的核心决策

| # | 决策项 | 结论 | 理由/细节 |
|---|--------|------|-----------|
| 1 | 数据组织形态 | 单日日程模式（按所选日期切片展示） | 对齐 `lx-agent`，界面清爽，切日与跨日顺延语义明确 |
| 2 | 顺延扫描范围 | 所有历史过去日期（`< 今天`）未完成待办 | 杜绝周末或跨天休假导致周五任务被遗漏在历史日期中 |
| 3 | 顺延交互形态 | 列表顶部紧凑横幅（Rollover Banner） | 仅在查看今天且存在历史未完成时浮现，一键顺延或关闭，不打扰视线 |
| 4 | 引入优先级 | 是，引入 P0 / P1 / P2 / P3 四档 | 排序对齐 `lx-agent`（未完成 > P0..P3），为图表提供多维度统计 |
| 5 | 工具栏排序按钮 | 新增 `[sort]` 按钮 | 点击对当前日期列表执行一键重排并落库持久化 |
| 6 | 优先级视觉与交互 | 独立徽标，背景区分且不影响字色 | P0 鲜红、P1 琥珀黄、P2 蓝、P3 弱灰；点击/按 `p` 换档；采用半透明/适度对比度底色 |
| 7 | 一周感知带位置 | 工具栏下方、列表上方 | 布局为：工具栏 → 周感知带 → (顺延横幅) → 待办列表 → 分割线/图表头 → 统计图表 |
| 8 | 多图表类型与切换 | 3 款图表；分割线切图控件 `‹ Title (n/3) ›` | 点击箭头或按 `c` 键主动切换：状态完成率、优先级分布、近 7 日趋势 |

---

## 3. 详细设计与关键数据结构

### 3.1 数据库模式变更（Migration v21）

当前数据库版本为 `SCHEMA_VERSION = 20`（`src/db/mod.rs`）。需要顺延至 `21`。

#### 表结构迁移逻辑
原 `v15` 的 `todo` 表缺少 `entry_date`、`priority` 与 `completed_date`，且在 `position` 上有全局唯一索引 `idx_todo_position`。
由于现在每个日期拥有独立的递增 `position`，唯一索引应改为 `(entry_date, position)`。

```sql
-- migration v21
ALTER TABLE todo ADD COLUMN entry_date TEXT NOT NULL DEFAULT '';
ALTER TABLE todo ADD COLUMN priority TEXT NOT NULL DEFAULT 'P1';
ALTER TABLE todo ADD COLUMN completed_date TEXT;

-- 将历史平铺数据全量迁移至当前本地日期
UPDATE todo SET entry_date = strftime('%Y-%m-%d', 'now', 'localtime') WHERE entry_date = '';
UPDATE todo SET completed_date = strftime('%Y-%m-%d', 'now', 'localtime') WHERE status = 'done' AND completed_date IS NULL;

-- 废弃原先的全局 position 唯一索引，建立日期维度的唯一与复合索引
DROP INDEX IF EXISTS idx_todo_position;
CREATE UNIQUE INDEX IF NOT EXISTS idx_todo_date_position ON todo(entry_date, position);
CREATE INDEX IF NOT EXISTS idx_todo_date_status ON todo(entry_date, status);
CREATE INDEX IF NOT EXISTS idx_todo_entry_date ON todo(entry_date);
```

#### 数据模型更新 (`src/db/todo.rs`)

```rust
/// 日程条目优先级（四档，P0 最高），库内以 TEXT 存储。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TodoPriority {
    P0,
    P1,
    P2,
    P3,
}

impl TodoPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "P0" => Self::P0,
            "P2" => Self::P2,
            "P3" => Self::P3,
            _ => Self::P1, // 默认 P1
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::P0 => Self::P1,
            Self::P1 => Self::P2,
            Self::P2 => Self::P3,
            Self::P3 => Self::P0,
        }
    }
}

/// 一条待办日程记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRecord {
    pub id: String,
    pub entry_date: String, // "YYYY-MM-DD"
    pub text: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
    pub completed_date: Option<String>,
    pub position: i64,
}
```

#### 数据库持久化接口扩展 (`src/db/todo.rs`)

```rust
impl Database {
    /// 按日期获取待办条目列表，按 position 升序。
    pub fn list_todos_by_date(&self, entry_date: &str) -> Result<Vec<TodoRecord>>;

    /// 查询所有早于指定日期（< today）且未完成（status != 'done'）的待办。
    pub fn list_past_incomplete_todos(&self, today: &str) -> Result<Vec<TodoRecord>>;

    /// 将一批条目顺延至目标日期（追加到目标日期的末尾，并清空原日期的 position 空隙）。
    pub fn rollover_todos_to_date(&self, ids: &[String], target_date: &str) -> Result<()>;

    /// 单日条目重排（写入目标顺序的 positions）。
    pub fn reorder_todos(&self, entry_date: &str, ordered_ids: &[String]) -> Result<()>;

    /// 查询日期区间内的单日统计（用于趋势图与周感知带小角标）。
    pub fn query_range_stats(&self, start_date: &str, end_date: &str) -> Result<Vec<DayStatsRecord>>;
}
```

---

### 3.2 状态层设计 (`src/app/state/todo.rs`)

```rust
/// 统计图表种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoChartKind {
    #[default]
    StatusAndRate, // 1/3: 三态与完成率
    Priority,      // 2/3: 优先级分布
    Trend7Day,     // 3/3: 近 7 日趋势
}

impl TodoChartKind {
    pub fn next(self) -> Self {
        match self {
            Self::StatusAndRate => Self::Priority,
            Self::Priority => Self::Trend7Day,
            Self::Trend7Day => Self::StatusAndRate,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::StatusAndRate => Self::Trend7Day,
            Self::Priority => Self::StatusAndRate,
            Self::Trend7Day => Self::Priority,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::StatusAndRate => "Status & Rate",
            Self::Priority => "Priority",
            Self::Trend7Day => "7-Day Trend",
        }
    }

    pub fn index_label(self) -> &'static str {
        match self {
            Self::StatusAndRate => "1/3",
            Self::Priority => "2/3",
            Self::Trend7Day => "3/3",
        }
    }
}

/// 周感知带中某一天的视图状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekDayState {
    pub date: String,        // "YYYY-MM-DD"
    pub day_of_month: u32,   // 1..31
    pub weekday_name: &'static str, // "Mon", "Tue"...
    pub is_today: bool,
    pub is_selected: bool,
    pub total_count: usize,
    pub completed_count: usize,
}

/// 顺延横幅状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverBannerState {
    pub count: usize,
    pub dismissed: bool,
}

/// Todo 面板状态。
pub struct TodoPanel {
    /// 当前选中的日期（"YYYY-MM-DD"）。
    selected_date: String,
    /// 今天的日期缓存（由系统环境读取）。
    today_date: String,
    /// 当前选中日期的待办列表（按 position 升序）。
    items: Vec<TodoRecord>,
    /// 选中行下标。
    selected: Option<usize>,
    /// 列表滚动偏移。
    scroll: usize,
    /// 行内编辑现场。
    editing: Option<TodoEdit>,
    /// 删除撤销槽。
    undo: Option<Vec<TodoRecord>>,
    /// 顺延提示横幅。
    rollover: Option<RolloverBannerState>,
    /// 当前激活的统计图表。
    active_chart: TodoChartKind,
    /// 7 天周感知带数据缓存。
    week_days: Vec<WeekDayState>,
    /// 近 7 日趋势统计缓存（当日及过去 6 天）。
    trend_stats: Vec<DayStatsRecord>,
    ...
}
```

---

### 3.3 交互与行为层 (`src/app/update/todo.rs`)

1. **日期切换**：
   - `select_date(state, "2026-10-31")`：切日、更新 `selected_date`、重载该日列表与撤销现场、刷新周感知带选中态与图表数据；
   - `shift_week(state, delta_weeks)`：按周移动基准日（`±7` 天）。
2. **一键排序**：
   - `sort_todos(state)`：对 `items` 按照 `lx-agent` 规则重排：
     ```rust
     items.sort_by(|a, b| {
         // 1. 未完成在前
         let a_done = a.status == TodoStatus::Done;
         let b_done = b.status == TodoStatus::Done;
         if a_done != b_done {
             return a_done.cmp(&b_done);
         }
         // 2. 优先级从高到低 (P0 < P1 < P2 < P3)
         if a.priority != b.priority {
             return a.priority.cmp(&b.priority);
         }
         // 3. 原 position 顺序
         if a.position != b.position {
             return a.position.cmp(&b.position);
         }
         // 4. id 稳定兜底
         a.id.cmp(&b.id)
     });
     ```
   - 登记 `Effect::ReorderTodos` 批量持久化。
3. **优先级变更**：
   - 鼠标点击某行优先级徽标或选中行按 `p` / `Shift+P`：
     - 调用 `cycle_priority(index)`，条目 `priority = priority.cycle()`；
     - 登记 `Effect::SaveTodo` 异步写库。
4. **一键顺延**：
   - 点击横幅中的 `[Roll to Today]`：
     - 调用 `execute_rollover(state)`，将所有历史未完成条目的 `entry_date` 修改为今天，追加进今天列表末尾；
     - 清除横幅，Toast 提示 `Moved N uncompleted todos to today`；
   - 点击横幅中的 `[✕]`：
     - 调用 `dismiss_rollover(state)`，仅在当前会话中标记 `dismissed = true`，隐藏横幅。
5. **图表切换**：
   - 点击切图小箭头 `‹` / `›` 或焦点在 Todo 时按 `c` 键：
     - `state.todo.toggle_chart_next()` / `toggle_chart_prev()`。

---

### 3.4 界面布局与渲染 (`src/ui/right_panel/todo.rs` & `style/todo.rs`)

#### 垂直布局划分
```text
┌──────────────────────────────────────────────┐
│ [≡] Prompt  [Todo]                           │  ← 右侧面板 Tab 栏
├──────────────────────────────────────────────┤
│ [+] [sort]              [edit] [del] [clear] │  ← 工具栏 (Toolbar)
│ ‹  Mon 26  Tue 27  [Wed 28]  Thu 29 ...  ›   │  ← 一周感知带 (Week Strip, 1行)
│ 💡 3 past todos pending [Roll to Today] [✕]  │  ← 顺延横幅 (Rollover Banner, 存在时1行)
│ ❯ [ ] P0 修复核心崩溃隐患                     │  ← 待办列表区 (List Rect)
│   [~] P1 补充单元测试用例                     │
│   [x] P2 整理文档格式                         │
├────── ‹ Status & Rate (1/3) › ───────────────┤  ← 分割线 + 图表切换器
│ total 3 · doing 1 · done 1 33%               │  ← 统计内容区 (Stats Rect)
│        █                                     │
│   █    █    █                                │
│  open doing done                             │
└──────────────────────────────────────────────┘
```

#### 优先级徽标配色与样式 (`src/ui/style/todo.rs`)
严格遵循 TUI 设计规范，前景色明晰，背景色采用低饱和度/暗底保证对比度，绝不影响文字可读性：
- **P0（最高）**：
  - 前景：`Color::LightRed` / `Indexed(196)`（鲜红加粗）
  - 背景：`Indexed(52)`（深暗红底）
- **P1（高）**：
  - 前景：`Color::LightYellow` / `Indexed(214)`（琥珀黄加粗）
  - 背景：`Indexed(58)`（深暗黄底）
- **P2（中）**：
  - 前景：`Color::LightCyan` / `Indexed(75)`（浅蓝）
  - 背景：`Indexed(23)`（深暗蓝底）
- **P3（低）**：
  - 前景：`Color::DarkGray` / `Indexed(245)`（中灰）
  - 背景：`Indexed(236)`（暗灰底）
- **已完成条目（Done）**：
  - 所有徽标前景自动压暗为 `style::muted()`，背景重置为透明，避免干扰已完成的视觉弱化。

#### 3 款图表渲染实现

1. **Chart 1: 三态与完成率（Status & Rate）**：
   - 顶部摘要：`total N · doing N · done N N%`；
   - 中部柱状：`open` / `doing` / `done` 3 根柱子，采用 `█` 字符，高度自适应伸缩；
   - 底部轴标签：`open`、`doing`、`done` 居中对齐。
2. **Chart 2: 优先级分布（Priority Breakdown）**：
   - 顶部摘要：`P0: N · P1: N · P2: N · P3: N`；
   - 中部柱状：对应 4 个优先级的柱子，分别用对应优先级的语义颜色渲染；
   - 底部轴标签：`P0`、`P1`、`P2`、`P3`。
3. **Chart 3: 近 7 日趋势（7-Day Trend）**：
   - 顶部摘要：`past 7 days · planned N · done N`；
   - 中部柱状：过去 7 天每天两个并排细柱（计划 `░` 蓝 / 完成 `█` 绿）；
   - 底部轴标签：`25` `26` `27` `28` `29` `30` `31`（日数字），当日标注特殊符号。

---

## 4. 关键代码接口设计

### 4.1 日期运算助手 (`src/platform/environment.rs` 或 `src/app/state/todo/date.rs`)
使用项目已具备的 civil calendar 算法（无第三方重依赖）：
- `today_local_date() -> String`: 返回本地今日 `"YYYY-MM-DD"`；
- `shift_date(date: &str, delta_days: i32) -> String`: 纯计算前进/后退天数；
- `week_dates_of(date: &str) -> [String; 7]`: 计算给定日期所在周（周一至周日）的 7 个日期字符串；
- `format_day_num(date: &str) -> u32`: 提取日数字（1..31）；
- `format_weekday_short(date: &str) -> &'static str`: 返回 `"Mon"`, `"Tue"` 等。

### 4.2 排序算法（纯函数）
```rust
pub fn sort_todo_records(mut items: Vec<TodoRecord>) -> Vec<TodoRecord> {
    items.sort_by(|a, b| {
        let a_done = a.status == TodoStatus::Done;
        let b_done = b.status == TodoStatus::Done;
        if a_done != b_done {
            return a_done.cmp(&b_done);
        }
        if a.priority != b.priority {
            return a.priority.cmp(&b.priority);
        }
        if a.position != b.position {
            return a.position.cmp(&b.position);
        }
        a.id.cmp(&b.id)
    });
    // 重新校准 position 为 0..n
    for (index, item) in items.iter_mut().enumerate() {
        item.position = index as i64;
    }
    items
}
```

---

## 5. 边界条件与风险防范

1. **旧数据库平滑迁移**：
   - v20 升级至 v21 时，所有历史已有的 todo 自动补齐 `entry_date = today`、`priority = P1`；
   - 原先全局唯一索引 `idx_todo_position` 安全平滑转为 `(entry_date, position)`。
2. **窄面板与极端尺寸降级**：
   - 面板宽度狭窄时，周感知带自动压缩为仅显示单字首字母（`M T W T F S S`）；
   - 垂直高度不足时，依序优雅降级：隐藏统计图表 → 隐藏顺延横幅 → 隐藏周感知带，确保主待办列表始终可读可用。
3. **纯状态与渲染分离不变量**：
   - 遵守项目规范：`ui/` 层纯只读，绝对不进行时间计算或数据库 I/O；所有时间基准在 `app/state` 初始化和事件轮询时写入。
4. **单文件行数红线控制**：
   - 生产代码单文件严格限制在 1000 行内（超出拆分子模块，如 `src/ui/right_panel/todo/` 下拆分 `week_strip.rs`, `chart.rs`, `banner.rs`；测试同步就近放置）。
