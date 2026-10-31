# 右侧面板 Todo 日程增强与多图表统计 — 技术设计

- 任务目录：`docs/feature/right-panel-todo-schedule/`
- 目标路径：`src/db/todo.rs`, `src/db/todo/date.rs`, `src/app/state/todo.rs`, `src/app/update/todo.rs`, `src/ui/right_panel/todo.rs`, `src/ui/right_panel/todo/{schedule.rs,charts.rs}`, `src/ui/style/todo.rs`, `src/ui/text/todo.rs`
- 状态：全部阶段及补充需求实现完成，并通过代码评审与 P0–P2 缺陷修复。

---

## 1. 背景与目标

原 `lx-tui` 的 Todo 面板为一个简易的全局平铺待办列表，所有条目混在一起，缺少时间维度与任务优先级管理。
参考 `lx-agent`（`/Users/yonah/projects/agent/lx-agent/src/renderer/src/features/schedule/`）成熟的日程/待办模型与工作流，为 `lx-tui` 的 Todo 面板进行全面增强：

1. **日期存储（Date Storage）**：
   - 待办由全局平铺改为**按日期（`YYYY-MM-DD`）组织**，每个日期拥有独立的有序待办清单；
   - 记录 `entry_date`（归属日期）、`completed_date`（完成日期）及 `priority`（`P0` / `P1` / `P2` / `P3` 优先级四档）。
2. **未完成顺延到今天（Rollover Uncompleted to Today）**：
   - 当用户查看“今天”的待办时，自动探测历史过去日期（`< 今天`）所有未完成的事项；
   - 在列表顶部展示紧凑的 **Rollover Banner** 提示条：显示历史未完成数量，提供 `[move to today]` / `[move]`（一键将它们移动到今天末尾）与 `[x]`（忽略本次提醒）。
3. **分割线日期跨度与纯英文一周快速点击感知带（Week Strip）**：
   - 工具栏下方的分割线内嵌当前日期信息：**左侧**展示当前选中年月日 `yyyy/mm/dd`（如 `2026/10/31`），**右侧**展示当前周起止号 `DD-DD`（如 `26-01`）；
   - 分割线正下方、列表区正上方为 **Week Strip**（1 行）：包含 `‹` 与 `›` 快速翻周按钮，中间均分周一至周日 7 个槽位，**去除日期号数字，仅显示纯英文星期**（`Mon`..`Sun`，窄宽度降级为 `M`..`S`），今日附带点标 `•`，选中日全槽高亮反显。
4. **一键智能排序（Sort by Priority）**：
   - 在顶部工具栏新增 `[sort]` 按钮（支持键盘 `s` 键）；
   - 排序规则严格对齐 `lx-agent`：**未完成在前 → 优先级 P0 > P1 > P2 > P3 → 原位置顺序 → ID 稳定兜底**。
5. **优先级徽标与可读性样式（Priority Chip）**：
   - 条目行内增加 `P0` / `P1` / `P2` / `P3` 徽标；
   - 支持鼠标单击徽标切换（`P0 → P1 → P2 → P3 → P0`）与键盘 `p` 键切换；
   - 各优先级具备专属深暗背景色与高对比度明亮前景色，且行选中/悬停底色绝不污染徽标可读性；已完成条目徽标自动压暗。
6. **多图表统计与范围标注（Multiple Stats Charts & Scope Label）**：
   - 底部统计区一次仅展示一个图表，通过分割线上的切图控件 `‹ scope · title n/3 ›` 或快捷键 `c` 进行主动切换；
   - 切图控件中间**明确注明統計范围是当天还是一周**，且切换器严格收敛在 `prev.x..=next.x` 区间内，绝不破坏两侧分割线 `─` 的 `style::border(focused)` 颜色一致性：
     1. **Chart 1: 三态与完成率（Status & Rate）**：标题为 `today · status 1/3`（非今日为 `day · status 1/3`），展示当前查看日的 open / doing / done 数量、完成率与三态柱图；
     2. **Chart 2: 优先级分布（Priority Breakdown）**：标题为 `today · priority 2/3`（非今日为 `day · priority 2/3`），展示当前查看日的 P0 / P1 / P2 / P3 分布与语义色柱图；
     3. **Chart 3: 近 7 日趋势（7-Day Trend）**：标题为 `week · 7d trend 3/3`，展示以当前查看日为终点、最近 7 天的每日“计划任务数（`░`）vs 完成任务数（`█`）”双柱对比。

---

## 2. 核心设计决策

| # | 决策项 | 结论 | 理由/细节 |
|---|--------|------|-----------|
| 1 | 数据组织形态 | 单日日程模式（按所选日期切片展示） | 对齐 `lx-agent`，界面清爽，切日与跨日顺延语义明确 |
| 2 | 顺延扫描范围 | 所有历史过去日期（`< 今天`）未完成待办 | 杜绝周末或跨天休假导致历史未完成任务被遗漏 |
| 3 | 顺延交互形态 | 列表顶部紧凑横幅（Rollover Banner） | 仅在查看今天且存在历史未完成时浮现，一键顺延或关闭，窄宽自动按阶梯降级文案 |
| 4 | 引入优先级 | 引入 P0 / P1 / P2 / P3 四档 | 排序对齐 `lx-agent`（未完成 > P0..P3），为图表提供多维度统计 |
| 5 | 工具栏排序按钮 | 新增 `[sort]` 按钮 | 点击或按 `s` 键对当前日期列表执行一键重排并落库持久化 |
| 6 | 优先级视觉与交互 | 独立徽标，深暗背景区分且高对比度前景 | P0 红、P1 琥珀黄、P2 蓝、P3 银灰；点击徽标或按 `p` 换档；每格独立 `reset` 避免选中行底色混入 |
| 7 | 分割线日期与周感知带 | 分割线左 `yyyy/mm/dd` + 右 `DD-DD`；感知带纯英文星期 | 顶部边框清晰展示完整年月日与周跨度，感知带只留 `Mon`..`Sun` 与今日 `•` 点标，视觉干净不拥挤 |
| 8 | 多图表类型、范围与边框 | 3 款图表；注明 `today`/`day`/`week`；两侧边框无色差 | 切图器严格限制在 `prev.x..=next.x` 内重置样式，两侧 `─` 保持 `style::border(focused)` 一致 |

---

## 3. 详细设计与关键数据结构

### 3.1 数据库模式变更（Migration v21）

数据库版本由 `SCHEMA_VERSION = 20` 推进至 `21`（`src/db/mod.rs`, `src/db/migration.rs`, `src/db/migration/schema.rs`）：

```sql
-- migration v21
ALTER TABLE todo ADD COLUMN entry_date TEXT NOT NULL DEFAULT '';
ALTER TABLE todo ADD COLUMN priority TEXT NOT NULL DEFAULT 'P1';
ALTER TABLE todo ADD COLUMN completed_date TEXT;

-- 将历史平铺数据全量迁移至当前本地日期，已完成项补齐 completed_date
UPDATE todo SET entry_date = ?1 WHERE entry_date = '';
UPDATE todo SET completed_date = entry_date WHERE status = 'done' AND completed_date IS NULL;

-- 废弃原先的全局 position 唯一索引，建立按日期维度的唯一与复合索引
DROP INDEX IF EXISTS idx_todo_position;
CREATE UNIQUE INDEX IF NOT EXISTS idx_todo_date_position ON todo(entry_date, position);
CREATE INDEX IF NOT EXISTS idx_todo_date_status ON todo(entry_date, status);
CREATE INDEX IF NOT EXISTS idx_todo_entry_date ON todo(entry_date);
```

#### 持久化不变量 (`src/db/todo.rs`)
- `save_todo`：插入新记录（`None` 分支）与跨日迁移（`Some(old_date)` 分支）结束后均调用 `renumber_date`，确保任意日期下的 `position` 恒为 `0..n` 密集递增；
- `rollover_todos_to_date`：采用与 `RENUMBER_OFFSET (1_000_000)` 不重叠的 `RESTORE_OFFSET (2_000_000) + offset` 临时位置区间，再对源日期与目标日期分别执行 `renumber_date`，杜绝唯一索引碰撞；
- `delete_todos_with_status_on`：按指定 `entry_date` 与 `status` 清理并重排该日期剩余位置。

---

### 3.2 状态层与行为层 (`src/app/state/todo.rs`, `src/app/update/todo.rs`)

- **`TodoPanel` 状态**：
  - 维护 `selected_date`、`today_date`、当前日期的 `items: Vec<TodoRecord>` 以及其余日期归档 `other_dates: BTreeMap<String, Vec<TodoRecord>>`；
  - `display_len()` 统一返回 `items.len() + ghost`（新建幽灵行占 1 槽位），供 `set_scroll`、`set_viewport_rows`、`scroll_to_edit` 与 `todo::scrollbar` 共用；
  - `set_today_date(today)` 在日期推进时自动重置 `rollover_dismissed = false`，若此前停留在旧今日则同步跟随到新今日；
  - `replace(items)` 对当前选中日期与归档日期统一先按 `position` 升序排序再归一为 `0..n`。
- **`Effect` 完备性**：
  - `Effect::DeleteTodosWithStatus { entry_date, status }`、`Effect::ReorderTodos { entry_date, ordered_ids }`、`Effect::RolloverTodos { ids, target_date }` 均在生成效应时冻结目标日期，不依赖执行期的动态选中日期。

---

### 3.3 界面布局与渲染 (`src/ui/right_panel/todo.rs`, `schedule.rs`, `charts.rs`)

#### 垂直布局划分
```text
┌─ Prompt ─ Todo ──────────────────────────────┐  ← 右侧容器顶边框 Tab 栏
│ [+]                  [edit] [del] [sort] ... │  ← 工具栏 (Toolbar)
├─ 2026/10/31 ───────────────────────── 26-01 ─┤  ← 表头分割线（左：所选年月日，右：本周日跨度）
│ ‹  Mon   Tue   Wed   Thu   Fri  [Sat •] Sun ›│  ← 一周感知带 (Week Strip, 纯英文星期 + 今日点标)
│ ! 2 past unfinished       [move to today] [x]│  ← 顺延横幅 (Rollover Banner, 存在时占列表首行)
│ ❯ [ ] P0 修复核心崩溃隐患                     │  ← 待办列表区 (Effective List Rect)
│   [~] P1 补充单元测试用例                     │
│   [x] P2 整理文档格式                         │
├─────────────────── ‹ today · status 1/3 › ───┤  ← 统计分割线 + 图表切换器（注明 today/day/week）
│ total 3 · doing 1 · done 1 33%               │  ← 统计内容区 (Stats Rect)
│        █                                     │
│   █    █    █                                │
│ open 1 doing 1 done 1                        │
└──────────────────────────────────────────────┘
```

#### 样式隔离与防串色机制
1. **顶部表头分割线日期标签（`render_divider_labels`）**：
   - 先对 `left_rect`（` yyyy/mm/dd `）与 `right_rect`（` DD-DD `）执行 `buf.set_style(rect, Style::reset())`，再写入 `style::border_title()`，剥离聚焦边框的 `BOLD` 修饰符。
2. **底部统计分割线切图器（`render_chart_switcher`）**：
   - `whole` 严格定义为 `Rect::new(prev_x, divider.y, width, 1)`（即 `prev.x..=next.x`），绝不外溢到左右两侧的 `─` 边框单元格；
   - 对 `whole` 执行 `buf.set_style(whole, Style::reset())` 后写入 `‹`/`›`（`style::accent()`）与中间标题（`style::muted_on_border()`），既保证左右两侧 `─` 边框颜色与修饰符 100% 一致，又保证 `‹`/`›` 箭头不继承分割线的 `DIM`。
3. **行内选中与幽灵行对齐（`render_rows`）**：
   - `row_selected` 与 `marker` 均在扣除幽灵行偏移后按列表下标 `record_index` 判定；
   - 未完成条目的优先级徽标每格独立 `cell.reset()` 后写入专属背景色与高对比度前景色，不被选中行底色覆盖。
