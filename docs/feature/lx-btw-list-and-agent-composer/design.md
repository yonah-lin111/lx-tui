# BTW / Agents 列表-详情导航与统一 composer

> 状态：**已实现并合并到 `dev`（合并提交 `b54a0031`）**；终端交互观感仍建议用户运行 `cargo run` 复核。
>
> 创建：2026-11-12；实施：2026-11-12。
>
> 本文是本次任务的设计基线。生产代码已按本文实现；实现偏差见文末“实施记录”。
>
> 关联文档：
> - `docs/feature/lx-btw/design.md`：BTW 线程、QA 归属与只读回答的既有设计。
> - `docs/feature/lx-btw/task.md`：既有 `/btwList` 与 BTW composer 的实施记录。
> - `docs/standards/tui-design-requirements.md`：lx 页整体布局与输入交互规范。
> - `docs/standards/lx-message-block-display.md`：BTW 与 Agents 详情共用的消息块规范。

## 1. 目标与范围

把 `TaskPage` 的 `BTW` 与 `Agents` 两个页签统一成同一套 **列表态 → 详情编辑态** 导航：

1. **列表态**：
   - 底部容器高度固定 8 行，与 `Processes` 页签一致。
   - 顶边框 1 行、tab 行 1 行、搜索行 1 行、列表 5 行。
   - `BTW` 展示主会话的 Q 列表（原 `/btwList` 的条目）；`Agents` 展示子 agent 列表。
   - 搜索框常驻焦点，直接输入即过滤。
   - `↑/↓` 移动高亮并立即预览上方对话；`Shift+Enter` 进入编辑态；单击选中、双击进入。
2. **编辑态**：
   - 隐藏顶边框、tab 行、搜索行与列表，底部只剩 composer。
   - composer 高度按草稿软换行在 3..=5 行之间，上方对话区自动吃满剩余高度。
   - composer 顶边框左侧标记 `BTW` 或 `Subagent`；右侧显示 QA 归属 / 目标。
   - 发送后留在编辑态并清空草稿；`Esc` 退回列表态。
3. **BTW 与 Agents 详情渲染完全同源**：
   - 同一套 `HistorySource` 绘制、量行、锚点重排、滚动条、空态/加载态、卡片折叠、选区与复制。
4. **删除 `/btwList` 命令及其二级选择面板**，由常驻列表取代。
5. **Agents 删除单行内联追问**（`agent_reply`），改用同一套 composer。
6. **删除 `Ctrl+A` 停止全部子 agent** 的能力。

不在范围：

- 不改主会话输入框、历史区、右侧面板、弹窗、toast。
- 不改 BTW 的数据库模型、QA 归属、回答者调度与只读权限。
- 不改 Agents 的 `SendSubagentUserInput` 后端语义。
- 不改 `Processes` 页签的列表结构与高度。

## 2. 已确认决策（用户逐项确认）

| 编号 | 决策 |
| --- | --- |
| Q1 | 列表 `↑/↓` 移动高亮时，上方对话区立即切换到该 Q / agent 的线程预览；`Shift+Enter` 只负责进入编辑态。 |
| Q2 | 搜索框常驻焦点；直接打字过滤；`↑/↓` 被拦截为列表导航；`Shift+Enter` 进入；`Esc` 先清搜索、再关闭面板。 |
| Q3 | 列表态固定 8 行：顶边框 1 + tab 行 1 + 搜索行 1 + 列表 5；BTW 与 Agents 相同，`Processes` 保持 6 行列表。 |
| Q4 | 编辑态底部只有 composer（3..=5 行）；tab 行、顶部分隔线、搜索行与列表全部隐藏；上方对话区占满剩余高度。 |
| Q5 | 编辑态 `Esc` 始终退回列表态并保留草稿；列表态再按 `Esc` 才关闭面板。搜索词非空时先清空搜索词。 |
| Q6 | `/btw prompt` 直接进入最新主 Q 的编辑态并提交；裸 `/btw` 进入列表态。首次打开高亮最新主 Q，再次打开保留当前浏览的 Q（与 Q12 一致）。 |
| Q7 | Agents 也改用“列表 + `Shift+Enter` 进入详情 + 独立 composer”；移除 `agent_reply` 单行内联输入。 |
| Q8 | Agents composer 复用纯文本编辑核心（软换行、选区、撤销重做、多行粘贴、硬件光标），但不提供 `@` 文件/子 agent 提及与 `/` 命令面板；BTW 保留 `@` 提及与粘贴，删除 `/btwList` 后不再有 BTW 专属 `/` 命令。 |
| Q9 | 搜索范围：BTW 匹配主 Q 标题；Agents 匹配昵称、角色、任务标题与 `agent_id`；先应用 `all/active/done` 筛选，再套搜索。无结果文案 `no matching questions` / `no matching agents`。 |
| Q10 | 切换 tab 时强制退出编辑态并清空搜索词；列表高亮保留；未发送草稿按 Q / agent 保留。关闭整个面板后重新打开回到列表态，搜索词清空，高亮保留。 |
| Q11 | 取消 `BtwFocus` 与 `Tab` 切焦点；composer 恒聚焦；`PgUp/PgDn` 每页 10 行、滚轮每格 3 行滚动上方对话，与 Agents 详情一致。 |
| Q12 | 主会话新增 Q 不自动抢走高亮；仅当首次打开、高亮为空或原 Q 被删除时回退到最新 Q。 |
| Q13 | composer 标记：BTW 左侧 `BTW`、右侧保留 `QA n · 标题 · To: <回答者>`；Agents 左侧 `Subagent`、右侧 `To: <nickname> [<role>]`。只在编辑态出现；窄窗格时标记整体隐藏，不截半截。 |
| Q14 | 列表 `↑/↓` 到两端即停，不关闭面板；退出统一用 `Esc`。 |
| Q15 | 列表行单击选中并预览，双击进入编辑态；点击搜索框只定位文本光标；编辑态点击上方对话不切焦点。 |
| Q16 | 高亮到尚无 BTW 线程的 Q 时显示 `No side conversation yet`，不提前写库；历史加载中显示与 Agents 相同的呼吸 loading。 |
| Q17 | BTW 行左侧 `QA n · 标题`；右侧有线程时 `● side thread`，最近批次为 `queued/running/failed/interrupted` 时改显示状态词并着色；Agents 行维持现有树形列，只增加搜索行。 |
| Q18 | 列表加载中显示 `Loading main questions...` / `Loading agents...`；空列表显示 `No main questions yet` / `no subagents yet`；状态筛选无匹配但存在 agent 时显示 `no agents in this filter`。此时 `Shift+Enter` 无动作，对话区显示 `No side conversation yet` 或 `no subagents yet`。 |
| Q19 | Agents 草稿按 `agent_id` 各存一份；切换高亮、回列表、切 tab 都不丢；发送成功清空；会话切换/`clear_agents` 时整体丢弃；不跨重启恢复。 |
| Q20 | 搜索词为空时 `←/→` 轮转 tab；搜索词非空时 `←/→` 移动搜索框光标。 |
| Q21 | 列表态 `Tab`：Agents 轮转状态筛选；BTW 不动作；`Shift+Tab` 两页都不动作。 |
| Q22 | 列表态普通 `Enter` 不进入、不关闭、不换行；只有 `Shift+Enter` 进入选中项。 |
| Q23 | BTW Q 列表保持时间正序（旧 → 新），首次打开高亮最新并滚到底部。 |
| Q24 | BTW 列表行选中样式跟随 Agents：行首 `❯` + 选中正文色 / 未选中 muted，不铺整行选中底色。 |
| Q25 | 编辑态 `Ctrl+D` 停止当前目标（BTW 当前线程；Agents 当前 agent），执行后仍留在编辑态且保留草稿。 |
| Q26 | 编辑态 `←/→` 只移动文本框光标，不切换 tab；要切 tab 必须先 `Esc` 回列表。 |
| Q27 | Agents 发送成功后留在编辑态并清空输入；不退回列表。 |
| Q28 | 移除 `Ctrl+A` 停止全部子 agent；不再保留该快捷键。列表搜索框按既有编辑器语义：`Ctrl+A` 移到行首，`Ctrl+Shift+A` / `Cmd+Shift+A` 全选；编辑态与主编辑器一致。 |
| Q29 | 高亮切换到不同 Q 时，该 Q 对话重置到最底部并恢复 `follow`；切回也不恢复旧阅读位置。 |
| Q30 | 搜索过滤到 0 条时，上方对话区保留最后选中的内容；只有列表显示无匹配。仍有匹配但当前 active 被过滤掉时，active、高亮与详情统一落到第一项（确保 `Shift+Enter` 可用）；清空搜索后保持该第一项。 |

## 3. 当前实现事实（含关键代码）

### 3.1 面板几何

`src/ui/main_content/lx/task.rs`：

- `LIST_MAX = 6`（第 48 行）——列表固定 6 行。
- `list_height()` = `LIST_MAX + TAB_ROW + TOP_BORDER` = 8 行（第 62-67 行）。
- `task_panel_height(lx)`：BTW 页签按 composer 行数在 6..=8 行之间生长；其他 tab 固定 8 行（第 70-85 行）。
- `draw_list`：画顶边框、tab 行、Agents 筛选胶囊；BTW 直接调用 `btw::draw_bottom`（第 194-289 行）。
- `list_rect`：扣掉顶边框与 tab 行（第 292-299 行）。
- `TaskHit` 只有 `Tab` / `Close` / `Agent` / `AgentFilter`（第 435-445 行）。
- `hit` 的 Agents 列表行命中按 `rect.height - TOP_BORDER - TAB_ROW` 计算可见行（第 617-668 行）。

`src/ui/main_content/lx/task/btw.rs`：

- `draw_bottom` 接收 `list_rect`，在里面画 loading 空行与 composer（第 29-82 行）。
- `composer_rect_in` 从 `list_rect` 底部按 `COMPOSER_MAX_ROWS=3` 计算 composer（第 118-137 行）。
- `draw_qa_badge` 在 composer 顶边框右上角画 `QA n · 标题 · To: ...`（第 143-199 行）。
- `/btwList` 二级面板整套：`picker_region`、`picker_labels`、`picker_items`、`picker_view`、`picker_layout`、`picker_item_at`、`render_picker`（第 201-350 行）。
- `draw_conversation` 与 `detail_rows` 使用 `HistorySource::from_chat` 绘制 BTW 对话（第 352-415 行）。

`src/ui/main_content/lx/task/agents.rs`：

- `draw_detail`、`detail_rows`、`anchor_before`、`anchor_row` 使用 `HistorySource::from_run`（第 25-163 行）。
- 列表绘制与窗口起点（第 261-463 行）。

### 3.2 BTW 状态与选择器

`src/app/state/lx/btw.rs`：

```rust
pub enum BtwFocus { Composer, Conversation }        // 第 15-21 行
pub struct BtwThreadState { ... pub composer: BtwComposer, ... }
pub struct LxBtw {
    threads: BTreeMap<String, BtwThreadState>,
    pub qa: Vec<BtwQaSummary>,
    pub active: Option<String>,
    pub focus: BtwFocus,
    pub pending_prompt: Option<String>,
    pub picking: Option<usize>,          // `/btwList` 高亮
    pub pending_pick: bool,              // `/btwList` loading 占位
    pub picker_loading: bool,
}
```

`open_btw_list` / `is_btw_list_command` / `switch_qa` 位于 `src/app/update/chat/btw/mod.rs` 第 77-210 行。

### 3.3 Agents 内联追问

`src/app/state/lx/task.rs` 第 175-178 行：

```rust
pub agent_reply: Option<crate::app::overlay::TextInput>,
pub agent_reply_target: Option<(String, String)>,
```

`src/app/update/pane/task_page.rs`：

- `apply_agent_reply_input`（第 62-130 行）拦截全部按键编辑单行 `TextInput`。
- `submit_agent_reply`（第 148-213 行）发送后清空 `agent_reply`，退回列表。
- `open_agent_reply`（第 216-258 行）拒绝 cancelled agent。
- `LxInput::Enter` 在 Agents 页打开内联追问（第 498-507 行）。
- `LxInput::LineStart` 在 Agents 页停止全部子 agent（第 528-536 行）。

### 3.4 详情渲染重复

BTW 和 Agents 各自维护一套：

- 绘制：`btw::draw_conversation` vs `agents::draw_detail`。
- 量行：`btw::detail_rows` vs `agents::detail_rows`。
- 空态：BTW 在对话区不画 loading；Agents 画呼吸 loading。
- 锚点：Agents 有 `anchor_before` / `anchor_row`，BTW 没有。
- 视口解析：`resolve_task_btw_visible` 不做块级锚点补偿；`resolve_task_agents_visible` 做。

这导致 BTW 在流式输出、工具展开、窗口 resize 时可能出现阅读位置跳动，且空态表现与 Agents 不一致。

### 3.5 输入路由

`src/app/state/lx.rs`：

```rust
pub fn active_editor(&self) -> &Lx {
    if self.is_btw_tab()
        && let Some(composer) = self.btw.active_composer()
    {
        return composer;
    }
    self
}
```

问题：

- BTW 进入列表态后，`active_editor` 仍返回隐藏的 composer；粘贴、剪切、复制、提及与引用面板会继续落到不可见输入框。
- Agents 没有独立 composer，`active_editor` 永远返回主输入框。

相关调用点：

- `src/ui/main_content/lx/panels.rs::input_rect` / `input_geom`
- `src/event/paste.rs`
- `src/event/input.rs::copy_lx_selection` / `cut_lx_selection`
- `src/app/update/pane/popups.rs`
- `src/app/update/chat/viewport.rs`

### 3.6 鼠标与滚轮

- `src/event/mouse/press/lx.rs`：`task::hit` 只处理 tab/close/agent/filter；`/btwList` 另走 `task_btw_picker_hit`（第 254-267 行）。
- `src/event/mouse/wheel.rs`：
  - 输出区（`task_rects.output`）滚轮按 tab 分派 Agents / BTW / Processes（第 505-534 行）。
  - 列表区（`task_rects.list`）BTW 分支错误地滚动隐藏 composer（第 535-557 行）。
- `src/event/mouse/hit.rs` 与 `drag.rs` 的列表滚动条统一走 `task_list_scrollbar` / `set_task_list_scroll`。

### 3.7 测试与文案

- `/btwList` 相关测试：`src/app/update/chat/btw/tests.rs`、`src/ui/main_content/lx/task/tests.rs`、`tests/btw_verify.rs`。
- 内联追问相关测试：`src/app/update/chat/tests/task_page.rs`、`src/app/update/chat/tests/subagent.rs`、`src/event/tests/*`。
- 文案：`src/ui/text/btw.rs`、`src/ui/text.rs`。
- 历史文档：`docs/feature/lx-btw/design.md` 与 `task.md` 仍把 `/btwList` 描述为当前入口；本次需在新文档中注明其已被取代，不重写历史章节。

## 4. 目标交互

### 4.1 列表态布局（BTW 与 Agents 共用）

```text
┌─ lx ─────────────────────────────────────────────────────────────┐
│  ↑ 上方对话：当前高亮 Q / agent 的完整消息列表（复用历史渲染）      │
│                                                                   │
│ ───────────────────────────────────────────────────────────────── │
│ Processes · Agents · BTW                                   [esc]  │
│ > search questions…                                               │
│ ❯ QA 3 · 修复登录                                      running    │
│   QA 2 · 端口冲突                              ● side thread      │
│   QA 1 · 初始化项目                              ● side thread      │
│   ...                                                             │
└─ [↑↓ select] [⇧↵ enter] [←→ tabs] [esc close] ────────────────────┘
```

- 顶边框与 tab 行仍由 `task.rs` 绘制。
- 搜索行固定在 tab 行下方；列表滚动条只覆盖下方 5 行。
- `❯` 只标记当前高亮；不铺整行底色。
- 高亮行切换时，上方对话立即切换并贴底（Q1/Q29）。

### 4.2 编辑态布局（BTW 与 Agents 共用）

```text
┌─ lx ─────────────────────────────────────────────────────────────┐
│  ↑ 上方对话：当前 Q / agent 的完整消息列表                         │
│                                                                   │
│ BTW ─────────────────────────────────────── QA 3 · 修复登录 · To: lx │
│ > 继续追问这个实现…                                                │
│ ───────────────────────────────────────────────────────────────── │
└─ [↵ send] [⇧↵ newline] [^d stop] [esc back] ──────────────────────┘
```

- 不画顶边框、tab 行、搜索行、列表。
- composer 自身保留上下两条 `─` 边框。
- 左侧标记 `BTW` / `Subagent`；右侧胶囊显示归属。
- 高度 = 内容行数（1..=3）+ 2 条边框 = 3..=5。

### 4.3 列表行格式

BTW：

```text
❯ QA 3 · 修复登录                                      running
  QA 2 · 端口冲突                              ● side thread
  QA 1 · 初始化项目                              ● side thread
```

- 左侧固定 `QA n · `；标题按剩余宽度截断（`…`）。
- 右侧状态优先级：`queued` / `running` / `failed` / `interrupted` / `partial` / `cancelled` 显示 `btw_status_label`；否则有线程显示 `● side thread`；无线程留空。
- 状态颜色：运行中橙黄、失败红、中断/取消黄、`● side thread` muted。

Agents：

```text
❯ ■ Turing [explorer]  running  2m 59s  修复登录回调
  └─ ■ Curie [executor]  done  1m 02s  补充测试
```

- 保持现有树形前缀、状态图标、身份、状态、时长、标题列。
- 搜索过滤后父节点缺失时不画悬空连接符（现有 `tree_prefix` 逻辑）。
- 右侧新增搜索行不改变列宽计算方式。

### 4.4 键位

#### BTW 列表态

| 键 | 行为 |
| --- | --- |
| 字符 / Backspace / Delete | 编辑搜索词 |
| `←/→` | 搜索词为空：轮转 tab；非空：移动搜索光标 |
| `↑/↓` | 移动 Q 高亮并即时预览；到两端停住 |
| `Shift+Enter` | 进入当前高亮 Q 的编辑态（`LxInput::EnterFolder`） |
| `Ctrl+Enter` | 不动作（`LxInput::Newline`；列表态只有 `Shift+Enter` 一个入口） |
| `Enter` | 不动作 |
| `Tab` / `Shift+Tab` | 不动作 |
| `PgUp/PgDn` | 上方对话滚动 10 行 |
| 滚轮 | 列表区滚列表；对话区滚对话 |
| `Ctrl+D` | 停止当前 Q 的活动回答与排队 |
| `Esc` | 搜索词非空：清空；否则关闭面板 |

#### Agents 列表态

| 键 | 行为 |
| --- | --- |
| 字符 / Backspace / Delete | 编辑搜索词 |
| `←/→` | 搜索词为空：轮转 tab；非空：移动搜索光标 |
| `↑/↓` | 移动 agent 高亮并即时预览；到两端停住 |
| `Shift+Enter` | 进入当前 agent 的编辑态（`LxInput::EnterFolder`） |
| `Ctrl+Enter` | 不动作 |
| `Enter` | 不动作 |
| `Tab` | 轮转 `all → active → done` |
| `PgUp/PgDn` | 上方对话滚动 10 行 |
| 滚轮 | 列表区滚列表；对话区滚对话 |
| `Ctrl+D` | 停止当前 agent |
| `Esc` | 搜索词非空：清空；否则关闭面板 |

> `Ctrl+A` 不再停止全部子 agent；列表搜索框按既有编辑器语义处理（行首 / 全选）。

#### 编辑态（两页一致）

| 键 | 行为 |
| --- | --- |
| 字符 / 编辑键 | 编辑 composer |
| `Enter` | 发送 |
| `Shift+Enter` / `Ctrl+Enter` | 换行 |
| `←/→` / `↑/↓` | 文本光标 / 软换行移动 |
| `PgUp/PgDn` | 上方对话滚动 10 行 |
| 滚轮 | 上方对话滚动 3 行/格 |
| `Ctrl+D` | 停止当前目标（BTW 线程 / agent） |
| `Esc` | 先关闭打开的浮层（`@`、粘贴、引用、`/`）；否则退回列表态 |
| `Tab` / `Shift+Tab` | 无浮层时不动作 |

### 4.5 底边提示

- BTW 列表：`[↑↓ select] [⇧↵ enter] [←→ tabs] [esc close]`
- BTW 编辑：`[↵ send] [⇧↵ newline] [^d stop] [esc back]`
- Agents 列表：`[↑↓ select] [⇧↵ enter] [tab filter] [^d stop] [←→ tabs] [esc close]`
- Agents 编辑：`[↵ send] [⇧↵ newline] [^d stop] [esc back]`

窄窗格从右往左整项丢弃，不截半条提示。

## 5. 状态模型

### 5.1 派生模式

不新增全局枚举，按 tab 派生：

```rust
impl Lx {
    pub fn btw_editing(&self) -> bool { self.is_btw_tab() && self.btw.editing }
    pub fn agents_editing(&self) -> bool { self.is_agents_tab() && self.task.as_ref().is_some_and(|p| p.agent_edit.is_some()) }
}
```

`TaskPage` 增加：

```rust
pub struct TaskPage {
    // ...现有字段...
    /// Agents 列表搜索框；列表态常驻焦点。
    pub agents_search: TextInput,
    /// 正在编辑的 agent_id；None 为列表态。
    pub agent_edit: Option<String>,
    /// Agents 列表请求在途（打开 Agents 页/切到该页时置位，列表回投后清除）。
    pub agents_loading: bool,
}
```

删除：

```rust
pub agent_reply: Option<TextInput>,
pub agent_reply_target: Option<(String, String)>,
```

`TaskPage` 仍保持 `Debug + Clone + Default + PartialEq + Eq`（`TextInput` 满足）。

### 5.2 LxBtw

`src/app/state/lx/btw.rs`：

```rust
pub struct LxBtw {
    threads: BTreeMap<String, BtwThreadState>,
    pub qa: Vec<BtwQaSummary>,
    pub active: Option<String>,
    /// 是否处于编辑态；false 为列表态。
    pub editing: bool,
    /// 列表搜索框。
    pub search: TextInput,
    /// Q 列表窗口（高亮/锚点/显式视口）。
    pub qa_window: PanelWindow,
    pub pending_prompt: Option<String>,
}
```

删除：

- `BtwFocus`
- `focus`
- `picking`
- `pending_pick`
- `picker_loading`

保留：

- `active` 为稳定 `qa_id`，不随搜索过滤变化。
- `pending_prompt` 仍用于 `/btw prompt` 在 QA 列表到达前暂存正文。

新增方法：

```rust
impl LxBtw {
    pub fn filtered_qa_indices(&self) -> Vec<usize>;
    pub fn selected_index(&self) -> Option<usize>;       // active 在过滤结果中的位置
    pub fn move_selection(&mut self, delta: isize) -> bool;
    pub fn select_qa(&mut self, qa_id: &str) -> bool;    // 已有，保留
    pub fn enter_edit(&mut self) -> bool;
    pub fn leave_edit(&mut self) -> bool;
    pub fn clear_search(&mut self) -> bool;
    pub fn sync_selection_after_filter(&mut self);
}
```

过滤规则：

```rust
let query = self.search.text().trim().to_lowercase();
self.qa.iter().enumerate()
    .filter(|(_, q)| query.is_empty() || q.title.to_lowercase().contains(&query))
    .map(|(i, _)| i)
    .collect()
```

空标题不匹配非空查询。

### 5.3 Agents

`src/app/state/lx/agent.rs`：

```rust
pub struct AgentsState {
    pub runs: Vec<SubagentRun>,
    pub cards_dirty: bool,
    /// 稳定选中的 agent_id：面板关闭/搜索过滤后仍保留。
    pub selected_id: Option<String>,
    /// agent_id -> 未发送草稿；只在实际进入编辑时惰性创建。
    pub composers: BTreeMap<String, DetailComposer>,
}
```

保留现有 `filtered_indices(filter)` 给非面板路径使用（如 `mention_runs`、状态恢复），新增：

```rust
pub fn visible_indices(&self, filter: AgentFilter, query: &str) -> Vec<usize>;
```

匹配字段：

- `run.display_identity()`
- `run.role.as_str()`
- `run.title`
- `run.agent_id`

大小写不敏感子串。

`TaskPage` 的 `selected_agent_index`、`len`、`clamp`、`move_active_len` 与 UI/命中/滚动全部改用 `visible_indices(self.agent_filter, self.agents_search.text())`，保证渲染、命中、翻页、滚动条同源。

稳定选中：

- `AgentsState.selected_id` 保存稳定 `agent_id`，不随面板关闭、搜索过滤或可见列表变化。
- `TaskPage.agents_window.active` 仍是“当前可见列表中的位置”。
- 每次有效选择同时更新 `selected_id` 与 `agents_window.active`。
- 打开/切回 Agents 页时，用 `selected_id` 在可见列表里恢复 `agents_window.active`。
- 搜索变化时：
  1. 用 `selected_id` 在新可见列表里找位置；找到就更新 `agents_window.active`。
  2. 找不到则 `agents_window.active = 0`，但保留 `selected_id` 供详情区继续显示。
- 清空搜索后按 `selected_id` 恢复高亮位置。
- 详情区使用“可见选中项，否则按 `selected_id` 回退”；停止/发送等动作只作用于可见选中项，避免误操作被搜索隐藏的 agent。

### 5.4 DetailComposer

把现有 `BtwComposer` 泛化为共享类型：

`src/app/state/lx/composer.rs`（新增）：

```rust
pub struct DetailComposer(Box<Lx>);

impl DetailComposer {
    pub fn new_btw() -> Self;      // Lx::composer()
    pub fn new_agent() -> Self;    // Lx::agent_composer()
    pub fn editor(&self) -> &Lx;
    pub fn editor_mut(&mut self) -> &mut Lx;
    pub fn text(&self) -> &str;
    pub fn is_empty(&self) -> bool;
    pub fn clear(&mut self);
    pub fn revision(&self) -> u64;
}
```

`Lx` 新增：

```rust
pub fn agent_composer() -> Self {
    let mut lx = Self::build(false);
    lx.no_shell = true;
    lx.allow_panels = false; // 新增私有字段
    lx
}
```

`refresh_panel` 开头：

```rust
if !self.allow_panels {
    self.panel = None;
    self.slash = None;
    self.paste = None;
    self.text_paste = None;
    self.refer = None;
    self.mention.suppress();
    self.scroll_cursor_into_view();
    return;
}
```

BTW 线程的 `composer` 字段类型改为 `DetailComposer`，行为不变。

### 5.5 编辑态归属

- BTW：`LxBtw.editing` 为真时，`active_thread().composer` 是可见 composer。
- Agents：`TaskPage.agent_edit = Some(agent_id)` 时，`agents.composers[agent_id]` 是可见 composer。
- 发送成功后：
  - BTW：清空 composer，`editing` 保持 true。
  - Agents：清空并移除该 composer（下次进入重新创建），`agent_edit` 保持 Some。
- `Esc` 回列表：
  - BTW：`editing = false`，草稿保留。
  - Agents：`agent_edit = None`，草稿保留在 map。
- 切 tab / 关闭面板：两页都退出编辑态；搜索词清空；高亮保留。
- agent 从 `runs` 移除时，删除对应 composer。

## 6. 详情渲染统一

### 6.1 新增共享模块

`src/ui/main_content/lx/task/detail.rs`（新增）承载两页共用逻辑：

```rust
pub(crate) struct DetailView<'a> {
    pub source: HistorySource<'a>,
    pub scroll: usize,
    pub follow: bool,
}

pub(crate) fn draw(
    grid: &mut Grid,
    rect: Rect,
    view: DetailView<'_>,
    empty: EmptyState<'_>,
    cache: &mut PaneCache,
) -> usize;

pub(crate) fn measure(
    rect: Rect,
    view: DetailView<'_>,
    cache: &mut PaneCache,
) -> (usize, bool);

pub(crate) fn anchor_before(...) -> Option<HistoryAnchor>;
pub(crate) fn anchor_row(...) -> Option<usize>;
pub(crate) fn scrollbar(...) -> Option<ScrollbarLayout>;

pub(crate) enum EmptyState<'a> {
    Placeholder(&'a str),
    Loading { label: &'a str, frame: usize },
}
```

调用方：

- Agents：`HistorySource::from_run(run)`，空态 `loaded ? "no subagents yet" : loading`。
- BTW：`HistorySource::from_chat(&thread.chat, &thread.compact)`，空态 `loaded ? "No side conversation yet" : "loading side conversation…"`。

### 6.2 空态与加载

- 未加载且无消息：与 Agents 相同的呼吸圆点 + `loading side conversation…`（BTW）/ `loading agent transcript…`（Agents）。
- 已加载且空：`No side conversation yet` / `no subagents yet`。
- 搜索无结果但详情保留：不进入空态，继续画最后选中内容（Q30）。
- 编辑态目标的 composer 上方复用 chat bar：BTW 看 `thread.is_loading()`，Agents 看 `run.is_running()`；列表态不显示。文案从目标自身消息流派生，BTW/Agents 没有 `RequestState::Streaming` 时回退到 `stream_assistant` / 最后一条 streaming assistant；扫描动画只用默认前景色（base dim / 窗口 bold），不跑主聊天四色渐变。

### 6.3 锚点与视口

- 把 `agents_anchor_before` / `agents_anchor_row` 的逻辑提取到 `detail.rs`，BTW 也使用。
- `resolve_task_btw_visible` 改为与 `resolve_task_agents_visible` 相同的“变化前锚点 → 重建快照 → 变化后锚点行 → 调整 `scroll`”流程。
- 两页 `rows_dirty` 语义保持一致：消息/工具/折叠/宽度变化置脏；resize 显式置脏。
- `visible` / `total_rows` / `follow` / `scroll` 继续由事件循环在绘制前写入。

### 6.4 卡片、选区与复制

现有 `history_hit` 已能按 `TaskTab::Btw` / `Agents` 选择数据源；本次只做两件事：

1. `task_output_rect` 在编辑态与列表态都返回同一对话区矩形。
2. `content_geom` / `press_content_hit` / `task_detail_code_copy_at` / `task_*_scrollbar` 全部改用共享 `DetailView` 构造，避免 BTW 与 Agents 的 `top` 口径分叉。

### 6.5 两个选中口径

Agents 必须区分两个查询：

- **可见选中项**：用于列表高亮、`Ctrl+D` 停止、`Shift+Enter` 进入编辑、发送目标。它必须落在当前 `visible_indices` 内；搜索无结果时为 None，动作 no-op。
- **详情回退项**：用于上方对话绘制、量行、滚动条、`is_at_bottom`。它先取可见选中项，取不到时按 `AgentsState.selected_id` 稳定 ID 回退（Q30）。

BTW 也保持同一口径：`active` 是稳定 `qa_id`，列表高亮位置由 `qa_window.active` 表示；搜索无结果时不改变 `active`，因此详情继续显示。

对应新增：

```rust
impl TaskPage {
    pub fn selected_agent_index(&self, agents: &AgentsState) -> Option<usize>; // 可见选中
    pub fn detail_agent_index(&self, agents: &AgentsState) -> Option<usize>;   // 详情回退
}
```


## 7. 底部面板实现

### 7.1 常量

`src/ui/main_content/lx/task.rs`：

```rust
pub(super) const TOP_BORDER: u16 = 1;
pub(super) const TAB_ROW: u16 = 1;
pub(super) const SEARCH_ROW: u16 = 1;
pub(crate) const LIST_MAX: usize = 6;
/// 有搜索行的列表（BTW / Agents）可见 5 行。
pub(crate) const LIST_MAX_SEARCH: usize = LIST_MAX - 1;
```

- 删除 `COMPOSER_GAP`。
- `list_height()` 保持 8。
- 新增 `list_rows_rect(rect, tab) -> Rect`：
  - Processes：`y = rect.y + TOP_BORDER + TAB_ROW`，height = 6。
  - Agents/Btw：`y = rect.y + TOP_BORDER + TAB_ROW + SEARCH_ROW`，height = 5。
- 新增 `search_rect(rect) -> Rect`：`y = rect.y + TOP_BORDER + TAB_ROW`，height = 1。

### 7.2 面板高度

```rust
pub(super) fn task_panel_height(lx: &Lx) -> u16 {
    match lx.task.as_ref().map(|p| p.tab) {
        Some(TaskTab::Btw) if lx.btw.editing => composer_height(lx.btw.active_thread().map(|t| t.composer.editor())),
        Some(TaskTab::Agents) if lx.task.as_ref().and_then(|p| p.agent_edit.as_deref()).is_some() => {
            composer_height(lx.active_agent_composer().map(|c| c.editor()))
        }
        _ => list_height(),
    }
}

fn composer_height(editor: Option<&Lx>) -> u16 {
    let rows = editor.map(|e| e.visual_rows().len()).unwrap_or(1).clamp(1, 3);
    u16::try_from(rows).unwrap_or(1) + 2
}
```

### 7.3 绘制管线

`draw_list` 改成：

```text
if tab == Btw && btw.editing:
    return btw::draw_edit(grid, rect, lx, focused)
if tab == Agents && page.agent_edit.is_some():
    return agents::draw_edit(grid, rect, lx, focused)

draw_top_border(rect)
draw_tab_row(rect)
if tab == Agents: draw_filter_pills(rect)
if tab != Processes: draw_search_row(rect, ...)
let rows = list_rows_rect(rect, tab)
draw_rows(...)
draw_list_scrollbar(rows)
```

要点：

- 编辑态不画任何额外的顶边框/tab/搜索/列表；composer 的 `input::draw` 自己画上下边框。
- 列表态 scrollbar 只覆盖 `rows`，不覆盖搜索行。
- `draw_list` 返回搜索框或 composer 的硬件光标。

### 7.4 搜索行

```rust
fn draw_search_row(grid: &mut Grid, rect: Rect, text: &TextInput, placeholder: &str) -> Option<(u16,u16)> {
    let mut x = grid.put(rect.x, rect.y, ">", style::accent());
    x = grid.put(x, rect.y, " ", style::text());
    let width = usize::from(rect.right().saturating_sub(x));
    let input = widgets::input::view(text.text(), text.cursor(), width);
    if text.text().is_empty() {
        grid.put(x, rect.y, placeholder, style::muted());
    } else {
        for span in widgets::input::build_styled_spans(input.visible, input.start_char, text.selection()) {
            x = grid.put(x, rect.y, span.content.as_ref(), span.style);
        }
    }
    Some((rect.y, x + input.cursor_col))
}
```

复用 `src/ui/left_panel/history.rs::paint_search_row` 与 `src/ui/right_panel/prompt/tree.rs::paint_search_row` 的既有做法，不新造单行输入视口。

### 7.5 列表绘制

BTW `task/btw.rs` 新增：

```rust
pub(crate) fn draw_list(grid: &mut Grid, list: Rect, lx: &Lx, focused: bool) -> Option<(u16,u16)>;
```

- 先画搜索行；再画 5 行 Q。
- `❯` 标记、标题、右侧状态。
- 搜索无结果时在列表首行画 `no matching questions`。
- 列表为空时 `No main questions yet` / `Loading main questions...`。
- 滚动条由 `task.rs` 统一画。

Agents `task/agents.rs` 的 `draw_list` 保持现有列逻辑，但接收的列表区域改为搜索行下方 5 行；搜索无结果时显示 `no matching agents`。

### 7.6 编辑态绘制

BTW：

```rust
pub(crate) fn draw_edit(grid: &mut Grid, rect: Rect, lx: &Lx, focused: bool) -> Option<(u16,u16)> {
    let thread = lx.btw.active_thread()?;
    let composer = Rect::new(rect.x, rect.y, rect.width, rect.height); // task_list 已是 composer 高度
    let editor = thread.composer.editor();
    let visible = usize::from(composer.height.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let scroll = editor.scroll().min(total.saturating_sub(visible));
    let dims = input::draw(grid, composer, editor, focused, scroll, visible);
    draw_composer_mark(grid, composer, text::LX_BTW_INPUT_MARK);
    draw_qa_badge(grid, composer, lx);
    dims
}
```

Agents：

```rust
pub(crate) fn draw_edit(grid: &mut Grid, rect: Rect, lx: &Lx, focused: bool) -> Option<(u16,u16)> {
    let (page, run, composer) = ...; // page.agent_edit + agents.composers
    let dims = input::draw(grid, rect, composer.editor(), focused, scroll, visible);
    draw_composer_mark(grid, rect, text::LX_TASK_AGENT_INPUT_MARK);
    draw_agent_badge(grid, rect, run);
    dims
}
```

`draw_composer_mark` 在顶边框左侧覆盖 `─` 写入 ` BTW ` / ` Subagent `，使用 accent + bold；窄窗格时标记整体隐藏；右侧胶囊沿用现有 `style::btw_qa_badge()` 或 `style::session_title_badge`。

### 7.7 几何入口

- `btw::composer_rect(rect, lx)`：只在 `lx.btw.editing` 时返回 composer 矩形；列表态返回 None。
- 新增 `agents::composer_rect(rect, lx)`：只在 `agent_edit` 有值时返回。
- `panels::input_rect` / `input_geom`：
  - BTW 编辑态 -> BTW composer。
  - Agents 编辑态 -> Agent composer。
  - 列表态 -> None。
- `task_btw_composer_rect` 增加编辑态判断；新增 `task_agent_composer_rect`（如需要）。
- `chat_bar::bar_rect` 使用 `task_list`（编辑态为 composer 高度），保持 `↓ jump to bottom` 在 composer 上方。

## 8. 输入路由

### 8.1 总入口

`apply_task_page_input` 改为：

```rust
if tab == Btw {
    return btw::apply_input(state, pane, command); // 内部按 editing 分流
}
if tab == Agents {
    if page.agent_edit.is_some() {
        return agents_edit::apply_input(state, pane, command);
    }
    return agents_list::apply_input(state, pane, command);
}
// Processes 保持现有逻辑
```

建议拆分：

- `src/app/update/pane/task_agents.rs`：Agents 列表/编辑路由、草稿、发送、停止。
- `src/app/update/chat/btw/input.rs`：BTW 列表/编辑路由。
- `src/app/update/pane/task_page.rs`：共享 ESC/PgUp/PgDn/tab 切换与进程逻辑。

### 8.2 列表态搜索编辑

`LxInput` -> `TextInput` 映射：

| LxInput | TextInput |
| --- | --- |
| `Char(ch)` | `insert_char(ch)` |
| `Backspace` | `backspace()` |
| `Delete` | `delete()` |
| `Left/Right` | 搜索词为空时轮转 tab；否则 `move_left/right()` |
| `Home/LineStart` | `Home` / `Ctrl+A` -> `move_home()` |
| `End/LineEnd` | `move_end()` |
| `WordLeft/WordRight` | `move_word_left/right()` |
| `DeleteWordBackward/Forward` | 对应删除 |
| `SelectAll` | `select_all()`（`Ctrl+Shift+A` / `Cmd+Shift+A`） |
| `Undo/Redo` | `undo/redo` |
| `Paste`（事件层） | 单行化后 `insert_str` |
| 其他 | 吞掉 |

每次文本变化后：

1. 重算可见列表。
2. 稳定选中 ID 仍在可见列表时恢复其位置；被过滤掉且列表非空时，高亮与详情落到第一项（BTW 同步更新 `active` 并发 `LoadThread`）。
3. `viewport = None`。
4. 若高亮变化，触发预览切换（BTW `switch_qa` / Agents `select_task_agent`）。
5. 清空 `lx_selection`，避免搜索文本与对话选区串用；剪切搜索文本走同一同步路径。

### 8.3 编辑态

BTW 编辑态复用现有 composer 编辑逻辑：

- 删除 `picker_open` / `slash_open` / `BtwFocus` 分支。
- 保留 `@` 提及、粘贴、引用、md 块面板的浮层优先处理。
- `Enter` 发送；`Shift+Enter` 换行；`Ctrl+D` 停止；`Esc` 先关浮层，再 `leave_edit`。
- `↑/↓` 归 composer；`PgUp/PgDn` 归对话。
- 删除 `←/→` 空草稿切 tab 分支（Q26）。

Agents 编辑态：

- 复用同一套纯文本编辑分派；只处理 `@`/slash 之外的面板（实际上 `allow_panels=false`，不会打开）。
- `Enter` 发送 `SendSubagentUserInput`。
- 发送成功：清空并移除 composer，`agent_edit` 保持。
- cancelled agent：进入时 toast `Cannot send input to a closed subagent`，不进入编辑态。
- `Ctrl+D` 停止当前 agent。
- `Esc` 回列表。

### 8.4 Esc 优先级

统一顺序：

1. 打开的浮层（`@`、粘贴、引用、md 块、`/`）优先关闭。
2. 编辑态：退出到列表态，保留草稿。
3. 列表态：搜索词非空 -> 清空；否则关闭任务面板。

### 8.5 上下移动与预览

BTW：

```rust
pub fn select_qa(state, pane, qa_id) -> bool {
    // 1. lx.btw.select_qa(qa_id)
    // 2. 若切换到不同 Q：thread.follow = true; thread.scroll = usize::MAX; rows_dirty = true
    // 3. push Effect::Btw(LoadThread) （已 loaded 的幂等，事件回投按 active 校验）
    // 4. 清理对话选区
}
```

Agents：

- 沿用 `ensure_selected_agent_loaded`。
- 选择变化时 `run.follow = true; run.scroll = usize::MAX`（现有 `reset_active_agent_follow` 扩展为按 id 稳定定位）。
- 列表命中、键盘、鼠标全部走同一个 `select_task_agent_id`。

### 8.6 粘贴 / 复制 / 剪切

`src/event/paste.rs`：

- BTW 编辑态 -> `handle_lx_paste`（现有，保留路径面板、粘贴芯片与 `@`）。
- BTW 列表态 -> 单行化写入 `lx.btw.search`。
- Agents 编辑态 -> **直接 `editor.insert_str(&text)`**，保留换行，但不打开路径粘贴面板/芯片（`allow_panels=false`）；这是纯文本消息。
- Agents 列表态 -> 单行化写入 `page.agents_search`。
- Processes -> 保持吞掉。

`src/event/input.rs`：

- `copy_lx_selection` / `cut_lx_selection`：
  - 列表态优先复制/剪切搜索框选区。
  - 编辑态优先 composer 选区。
  - 其次才是对话选区。
- `Ctrl+A` 删除 stop-all 分支；`LineStart` 与 `SelectAll` 按编辑器既有语义分派。

## 9. 删除 `/btwList`

### 9.1 命令层

`src/app/state/lx/slash.rs`：

- 删除 `SlashCommand::BtwList` 变体、`name/aliases/text/description` 分支。
- `ALL` 长度保持 12（该命令本来就不在 `ALL`）。
- 删除 `LX_BTW_LIST_COMMAND_DESC` 引用。

`src/app/update/chat/slash.rs`：

- 删除 `SlashCommand::BtwList => false` 的保留分支。

### 9.2 BTW composer 内的 `/` 面板

`src/app/update/chat/btw/input.rs`：

- 删除 `btw_command_entries`、`refresh_composer_slash`、`accept_btw_composer_command`、`select_btw_slash`。
- 删除 `slash_open` 分支。
- 删除 `is_btw_list_command` 的 Enter 分支。
- 删除 `apply_picker_input` 与 `picker_open` 优先分支。

`src/ui/main_content/lx.rs`：

- 删除 `render_picker` 调用。
- 删除 `task_btw_picker_hit`。
- 删除 `event/mouse/press/lx.rs` 的 `/btwList` 命中分支。

### 9.3 状态层

`LxBtw` 删除：

- `picking`
- `pending_pick`
- `picker_loading`
- `open_picker` / `picker_open` / `close_picker` / `move_picker` / `picked_qa`

`on_qa_list_loaded` 删除所有 picker 处理。

### 9.4 `/btw` 语义

- 裸 `/btw`：打开 BTW tab，列表态并清搜索；首次打开（`active=None`）由 QA 列表回投高亮最新 Q，已有 `active` 时保留当前浏览的 Q（Q12）。
- `/btw prompt`：打开 BTW tab，`active=None`、进入编辑态等待列表回投，随后选最新 Q 并提交 prompt。
- 无 QA：列表显示空态；`/btw prompt` 的正文回填主输入并 warning，不再保留隐藏 composer。

## 10. 事件回投

### 10.1 QA 列表

`on_qa_list_loaded` 新流程：

```text
1. 校验 parent_session_id。
2. 替换 lx.btw.qa。
3. 若 active 为空：选最后一个 Q（最新）。
4. 若 active 已不在新列表：回退到最新；若列表空：active=None，editing=false。
5. 同步 qa_window / search 过滤后的高亮。
6. 若 pending_prompt 存在：
   - 选最新 Q；
   - editing = true；
   - 确保线程视图；
   - 写入 composer；
   - submit_current。
7. 否则按 active 发 LoadThread（幂等）。
8. 同步 `@` 根与实例。
```

### 10.2 线程加载

`on_thread_loaded` 保留现有父会话 + `active == qa_id` 校验：

- 迟到回投不覆盖新选中项。
- `loaded=true` 后不再重复加载。
- 若当前在编辑态且线程刚建立，保持 composer 可见。
- `Accepted` 回投必须携带 `qa_id` 并按该线程定位；ACK 可能晚于用户切换 QA，不能只匹配当前 `active`，否则 `pending_submit` 永久残留。

### 10.3 Agents 列表更新

- 新 run 追加不抢高亮（Q12）。
- 若 `AgentsState.selected_id` 对应 run 消失：清空 `agent_edit` 与 composer，回退第一项。
- `TaskPage.agents_loading`：
  - `open_agent_page` / `select_task_tab(Agents)` / `cycle_task_tab` 切入 Agents 且列表为空时置 true，并确保登记一次 `Effect::ListSubagents`；
  - `set_subagent_list` 成功后置 false；
  - 列表非空或已有缓存时不显示 loading，直接画列表。
- 删除 `Ctrl+A` stop-all 后，`stop_all_subagents` 函数与文案一并删除。

## 11. 鼠标与滚轮

### 11.1 命中

`TaskHit` 扩展：

```rust
pub(crate) enum TaskHit {
    Tab(usize),
    Close,
    Qa(usize),        // BTW 过滤后位置映射回 qa 真下标
    Agent(usize),
    AgentFilter(AgentFilter),
    Search,           // 搜索行；按下后由调用方按列定位光标
}
```

`hit`：

- 先判 `[close esc]`、tab、筛选胶囊。
- 编辑态：不返回任何 tab/搜索/列表命中（tab 已隐藏）。
- 列表态：搜索行返回 `Search`；列表行按 `list_rows_rect` 与过滤结果映射。

`src/event/mouse/press/lx.rs`：

- `Search` -> 计算 `char_index`，设置搜索框光标/选区。
- `Qa(index)` -> 单击选中；双击进入编辑态。
- `Agent(index)` -> 同上。
- Tab/Close/Filter 保持。

双击判定：

- 新增 `AppState.task_list_click: Option<(PaneId, u16, u16, Instant, usize)>` 或复用现有 `selection_click` 但加 pane 标识；建议新增专用字段，避免与对话选区双击串用。
- 双击命中时先按点击行重新选中，再进入编辑态；避免两次点击之间用 `↑/↓` 改过高亮后打开错误项。
- 与 Todo / 工作区列表使用同一套单击/双击窗口。

### 11.2 滚轮

`src/event/mouse/wheel.rs`：

- 对话区（`output.y..list.y`）：
  - Agents -> `scroll_agents_detail`
  - Btw -> `scroll_btw_detail`
  - Processes -> `scroll_task_output`
- 列表区（`list.y..list.bottom()`）：
  - Processes -> `scroll_task_list`（6 行）
  - Agents 列表态 -> `scroll_agents_list`（5 行）
  - Agents 编辑态 -> `scroll_lx_input`（composer）
  - Btw 列表态 -> `scroll_btw_list`（5 行）
  - Btw 编辑态 -> `scroll_lx_input`（composer）
- 仅 **列表态且该 tab 真的绘制搜索行** 时吞掉搜索行滚轮；Processes 首行与编辑态 composer 行继续走列表/composer 滚轮。
- 编辑态不生成列表滚动条几何，也不接受列表滚动条命中/拖拽。

### 11.3 滚动条

- `task_list_scrollbar` / `set_task_list_scroll` 按 tab 与搜索行高度：
  - Processes：6 行。
  - Agents：`visible_indices(filter, query).len()` + 5 行可见。
  - Btw：`filtered_qa_indices().len()` + 5 行可见。
- 拖拽/轨道点击与键盘选择共用同一份可见列表。
- 编辑态 `list_scrollbar` / `task_list_scrollbar` 返回 None，避免隐藏列表吞掉 composer 边缘点击。

## 12. 文件修改清单

| 文件 | 修改 |
| --- | --- |
| `src/app/state/lx/composer.rs`（新增） | `DetailComposer`，BTW / Agent 两种构造。 |
| `src/app/state/lx/btw.rs` | 删除 `BtwFocus`、picker 字段/方法；新增 `editing`、`search`、`qa_window` 与过滤/选择方法。 |
| `src/app/state/lx/agent.rs` | `AgentsState.composers`；新增 `visible_indices(filter, query)`；agent composer 查找/清理。 |
| `src/app/state/lx/task.rs` | 删除 `agent_reply` / `agent_reply_target`；新增 `agents_search`、`agent_edit`、`agents_loading`；`len/clamp/selected_agent_index/move_active` 改用可见列表与搜索行高度。 |
| `src/app/state/lx.rs` | `active_editor` / `with_active_editor` 支持 Agents composer 与 BTW 编辑态；新增 `agent_composer()`、`allow_panels`；`is_btw_tab` 相邻新增 `is_agents_tab` / `btw_editing` / `agents_editing`。 |
| `src/app/state/lx/slash.rs` | 删除 `SlashCommand::BtwList`。 |
| `src/app/update/chat/btw/mod.rs` | 删除 `open_btw_list` / `is_btw_list_command`；`switch_qa` 改为选择 + 预览；新增 enter/leave edit。 |
| `src/app/update/chat/btw/input.rs` | 删除 picker/slash；新增列表态搜索/导航/Shift+Enter；编辑态复用 composer。 |
| `src/app/update/chat/btw/events.rs` | `on_qa_list_loaded` 去 picker，接入编辑态与 pending prompt。 |
| `src/app/update/chat/mod.rs` | 删除 `open_btw_list` / `select_btw_slash` / `select_btw_qa`（或改为列表选择）的导出。 |
| `src/app/update/pane/task_page.rs` | 删除 `agent_reply` 全套与 `Ctrl+A` stop-all；新增 Agents 列表/编辑路由、共享 Esc/PgUp/PgDn。 |
| `src/app/update/pane/task_agents.rs`（新增，建议） | Agents 列表搜索、选择、编辑、发送、停止。 |
| `src/app/state.rs` | 新增 `task_list_click`（列表单击/双击判定），并在切 pane/关闭面板时清理。 |
| `src/app/update/chat/task_page.rs` | 列表滚动/滚动条按 tab 与搜索行分派；新增 BTW/Agents 列表滚动函数。 |
| `src/app/update/chat/viewport.rs` | BTW 使用锚点；Agents/BTW composer 尺寸解析；列表搜索变化后的视口钳制。 |
| `src/app/effects.rs` | 不改协议；仅沿用 `LoadThread` / `Submit` / `Stop`。 |
| `src/ui/main_content/lx/task.rs` | 新增搜索行/列表行几何、`LIST_MAX_SEARCH`、`TaskHit::{Qa,Search}`、编辑态绘制分派、footer 分模式。 |
| `src/ui/main_content/lx/task/detail.rs`（新增） | 共享详情绘制/量行/锚点/滚动条/空态。 |
| `src/ui/main_content/lx/task/btw.rs` | 删除 picker；新增列表绘制、编辑态 composer、标记；删除 gap loading。 |
| `src/ui/main_content/lx/task/agents.rs` | 列表接收搜索行下方区域；新增编辑态 composer 绘制/标记；详情改用 `detail.rs`。 |
| `src/ui/main_content/lx.rs` | layout / render / task_output / task_list / composer rect / picker 删除 / active editor 几何。 |
| `src/ui/main_content/lx/panels.rs` | `input_rect` / `input_geom` 支持 BTW/Agent 编辑态，列表态返回 None。 |
| `src/ui/main_content/lx/chat_bar.rs` | 删除 `draw_inline_loading`（BTW 底部 gap loading 移除后无调用者）。 |
| `src/ui/main_content/lx/history_hit.rs` | 用共享 `DetailView` 构造；BTW 锚点；列表态无 composer 影响。 |
| `src/ui/main_content/lx/selection.rs` | BTW/Agent 编辑态与列表态的命中分支；搜索框选区不进入对话选区。 |
| `src/event/input.rs` | 复制/剪切搜索框选区；删除 BTW focus 特判；`Ctrl+A` 不再停止全部。 |
| `src/event/paste.rs` | 粘贴按列表/编辑态路由到搜索框或 composer。 |
| `src/event/mouse/press/lx.rs` | 新 TaskHit；双击进入；picker 删除。 |
| `src/event/mouse/wheel.rs` | 列表/对话/composer 滚轮分派。 |
| `src/event/mouse/hit.rs`、`drag.rs` | 列表滚动条按 tab 与搜索行高度。 |
| `src/ui/text.rs`、`src/ui/text/btw.rs` | 新增搜索/标记/提示文案；删除 `/btwList` 与内联追问文案。 |
| `tests/btw_verify.rs` | 删除 `/btwList` 步骤，改为列表态搜索 + Shift+Enter。 |
| `docs/feature/lx-btw/design.md`、`task.md` | 增加“§15/§13.2 已被本功能取代”的注记，不改写历史内容。 |

## 13. 测试矩阵

### 13.1 几何与绘制

| 用例 | 断言 |
| --- | --- |
| BTW 列表态高度 | `task_panel_height == 8`；顶边框 1 + tab 1 + 搜索 1 + 列表 5。 |
| Agents 列表态高度 | 同上；Processes 仍为 8（列表 6）。 |
| BTW 编辑态高度 | 1 行草稿 3 行；3 行草稿 5 行；隐藏 tab/搜索/列表。 |
| Agents 编辑态高度 | 同上。 |
| 编辑态对话区 | 输出矩形比列表态多 3 行（顶边框 + tab + 搜索）。 |
| 搜索行 | 空显占位；有词显示光标、选区与水平滚动。 |
| 列表滚动条 | 超过 5 条才出现；只覆盖列表 5 行。 |
| 标记 | 编辑态顶边框左侧 `BTW` / `Subagent`；右侧归属/目标。 |

### 13.2 BTW 列表

| 用例 | 断言 |
| --- | --- |
| 打开列表 | 高亮最新 Q 并滚到底部；对话显示该 Q。 |
| `↑/↓` | 高亮变化、对话切换、贴底；两端不关闭。 |
| `Shift+Enter` | 进入编辑态；tab/搜索/列表消失。 |
| `Enter` | 不动作。 |
| 搜索标题 | 大小写不敏感子串；无结果列表显示 `no matching questions`，对话保留最后内容。 |
| 清空搜索 | 原选中 Q 恢复高亮。 |
| 新 Q 到达 | 不抢高亮；原高亮仍存在。 |
| 原 Q 删除 | 回退最新或空态。 |
| 无线程 Q | 显示 `No side conversation yet`；不写库。 |
| 线程加载 | 显示呼吸 loading；加载完成后显示消息。 |
| 行状态 | running/failed/interrupted 着色；completed 显示 `● side thread`。 |
| 草稿 | 进入编辑输入一半，`Esc` 回列表再进入，草稿仍在。 |
| `/btwList` | 命令面板不再出现；主输入与 BTW composer 都不识别。 |

### 13.3 Agents 列表

| 用例 | 断言 |
| --- | --- |
| 打开列表 | 高亮当前/首项；对话显示该 run。 |
| 搜索昵称/角色/标题/id | 过滤正确；先 filter 后 search。 |
| `Tab` | all → active → done，搜索词保留。 |
| 无结果 | 列表 `no matching agents`；对话保留最后内容。 |
| `Shift+Enter` | 进入编辑态；隐藏 tab/搜索/列表。 |
| 双击 | 进入编辑态；单击只选中。 |
| cancelled agent | 不进入编辑态，toast `Cannot send input to a closed subagent`。 |
| 发送 | 留在编辑态、清空输入；调用 `SendSubagentUserInput`。 |
| 草稿 | 按 agent_id 保留；切 tab/回列表不丢。 |
| `Ctrl+A` | 不再停止全部 agent；按编辑器既有语义移到行首；`Ctrl+Shift+A` / `Cmd+Shift+A` 全选。 |
| `Ctrl+D` | 停止当前 agent，仍在编辑态。 |

### 13.4 详情一致性

| 用例 | 断言 |
| --- | --- |
| BTW / Agents 空态 | 同一位置、同一 loading 动画。 |
| 流式输出 | 视口保持阅读位置，不跳顶。 |
| 工具卡片展开 | 展开前后锚点补偿一致。 |
| 窗口 resize | `rows_dirty` 失效后重新量行，滚动条一致。 |
| 代码块复制 | 两页都能命中 `[❐]`。 |
| 文本选区 | 两页都能拖选/复制。 |
| 滚轮/翻页 | 3 行/格、10 行/页一致。 |

### 13.5 事件与路由

| 用例 | 断言 |
| --- | --- |
| 滞后 `LoadThread` | 切到新 Q 后旧回投被丢弃。 |
| `/btw prompt` 首问 | QA 列表到达后进入最新 Q 编辑态并提交。 |
| 无 QA 时 `/btw prompt` | 正文回填主输入、warning、列表空态。 |
| 切 tab | 退出编辑态、清空搜索、保留高亮。 |
| 关闭面板 | 列表态重新打开；搜索清空；高亮保留。 |
| 粘贴 | 列表态进搜索框；编辑态进 composer；不写隐藏主输入。 |
| 复制/剪切 | 优先搜索框选区，其次 composer 选区，再对话选区。 |

## 14. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| `active_editor` 语义扩大到 Agents 后，粘贴/提及路由可能串到主输入。 | 先加 `btw_editing` / `agents_editing` 派生函数；所有调用点显式分支；列表态 `input_rect` 返回 None；补粘贴/复制回归测试。 |
| 搜索过滤后索引与真实 `runs` / `qa` 下标错位。 | 所有列表操作统一走 `visible_indices` / `filtered_qa_indices`；动作使用真实下标；稳定选中用 `qa_id` / `agent_id`。 |
| 每按一次 `↑/↓` 触发一次 DB 线程加载。 | 已有 `loaded` 幂等；`on_thread_loaded` 按 active 校验丢弃滞后回投；必要时在事件层将同一 QA 的重复 `LoadThread` 合并（不改变 UI 语义）。 |
| BTW 视图改为锚点重排后，流式滚动行为变化。 | 复用 Agents 已验证的 `anchor_before` / `anchor_row`；保留 `follow` 快捷路径；补流式与展开用例。 |
| 任务面板高度在列表/编辑态切换时跳动。 | 列表态固定 8；编辑态按草稿行数 3..=5；用同一 `composer_height` 计算；测试覆盖 1..3 行草稿。 |
| 删除 `Ctrl+A` 停止全部会让习惯旧快捷键的用户误以为失效。 | 底边提示同时删除 `stop all`；toast/文档不承诺该能力；新文档记录这是用户明确要求。 |
| `/btwList` 删除后旧测试与 E2E 大量失败。 | 任务清单单列测试迁移；先改生产代码再批量更新测试；E2E `tests/btw_verify.rs` 改为新入口。 |
| 生产文件接近 1000 行上限。 | `task.rs`、`task_page.rs`、`btw.rs` 改动前先量行数；新增逻辑优先拆到 `task/detail.rs`、`task_agents.rs`、`btw/list.rs` 等职责文件；测试按场景拆文件。 |

## 15. 不做的事

- 不新增数据库迁移。
- 不把 BTW 线程列表持久化到本地 UI 状态；Q 列表仍来自 `LoadQaList`。
- 不为 Agents 搜索增加后端过滤；仍是对已回投的 `runs` 做内存过滤。
- 不改 `Processes` 页签的搜索/高度/退出行为。
- 不改主输入框的 `Ctrl+A`、`Tab` 等编辑语义。
- 不保留 `/btwList` 的隐藏别名或兼容入口。

## 16. 验收标准

1. BTW 与 Agents 列表态高度均为 8 行，搜索行常驻，列表 5 行，滚动条只覆盖列表。
2. `↑/↓` 即时预览且贴底；两端不关闭面板。
3. `Shift+Enter` 与双击都能进入编辑态；编辑态无 tab/顶分隔线/搜索/列表。
4. 编辑态 composer 标记、归属胶囊、3..=5 行高度正确；发送后留在编辑态；`Esc` 回列表。
5. BTW `@` 提及与粘贴仍可用；Agents composer 无 `@`/`/` 面板。
6. `/btwList` 在生产代码与文案中完全消失；测试保留“不再识别”的回归断言；`/btw` 与 `/btw prompt` 行为符合 Q6/Q12。
7. BTW 与 Agents 详情在空态、加载、流式、折叠、选区、复制、滚轮、翻页上表现一致。
8. `Ctrl+A` 不再停止全部 agent；`Ctrl+D` 仍停止当前目标。
9. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响测试全部通过。
10. 用户运行 `cargo run` 人工确认列表态/编辑态外观、鼠标与 `Shift+Enter` 可被终端识别。

## 17. 实施记录（2026-11-12）

实际落地与本文的差异，均为实现时收敛后的结果：

- 稳定选中放在 `AgentsState.selected_id`，`TaskPage.agents_window.active` 只表示当前可见列表位置；详情统一走 `TaskPage::detail_agent_index`（可见项优先，搜索无命中时回退稳定 ID）。
- `DetailComposer` 定为 `src/app/state/lx/composer.rs`，BTW 构造保留 `@`/粘贴，Agents 构造通过 `Lx::agent_composer()` 关闭全部浮层。
- 共享详情绘制放在 `src/ui/main_content/lx/task/detail.rs`；BTW 与 Agents 共用量行、空态、滚动条与阅读锚点。
- 列表态搜索行由 `task::draw_search_row` 统一绘制；BTW/Agents 列表正文均为 5 行，Processes 仍为 6 行。
- `Shift+Enter` 在输入层映射为 `LxInput::EnterFolder`，列表态只认它进入编辑；`Ctrl+Enter`（`LxInput::Newline`）不进入。
- `/btwList`、`BtwFocus`、`picker_*`、`agent_reply` 全部删除；`Ctrl+A` 不再发送 `CancelSubagents`，列表搜索框按行首语义处理。
- `Ctrl+D` 在 BTW/Agents 编辑态停留原页；BTW/Agents 发送成功后都留在编辑态并清空草稿。
- `Esc` 优先级：浮层（仅 BTW）→ 编辑态退回列表 → 列表清搜索 → 关闭面板。
- 鼠标单击行只选中/预览，同一行 400ms 内第二次单击进入编辑；搜索行点击按列定位光标。
- 新增测试覆盖：BTW 搜索/稳定选中/Shift+Enter/回车不变/空列表；Agents 搜索/Shift+Enter/草稿/取消态；Agents composer 复制剪切；8 行列表与 5 行正文；BTW/Agents 详情 loading 与布局。
- 验证：`cargo fmt --all`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 均通过（功能分支 5316 单测；合并后 `dev` 5347 单测，0 失败，6 ignored）。

## 18. Review 修复记录（2026-11-13）

只读审查发现的问题与修复见 [review.md](review.md)，设计层面的收敛：

- BTW 搜索仍有匹配时，`active`、列表高亮与详情统一落到第一项；0 匹配时继续保留最后详情（Q30）。
- 搜索行硬件光标以文本起点为基准；Agents composer 与 BTW 一样按窗格宽高 resize。
- 搜索行滚轮只在列表态吞掉；编辑态不暴露隐藏列表滚动条。
- BTW `Accepted` 回投按 `qa_id` 结算，不依赖 ACK 到达时的 `active`。
- 目标 loading 文案允许从 `stream_assistant` / streaming assistant 回退，但仍只在编辑态显示。
- Agents 筛选无匹配使用独立空态文案；`Subagent` 标记窄窗整体隐藏。
- 裸 `/btw` 重开保留当前 Q；双击先重选点击行。
