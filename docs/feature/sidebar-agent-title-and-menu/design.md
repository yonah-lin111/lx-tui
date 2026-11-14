# 侧栏 Agents 条目第二行标题与右键菜单 — 设计

> 状态：**已实现（V2 最终形态）**
> 关联任务：[task.md](task.md) ｜ 审查记录：[review.md](review.md)
> 依据：用户需求 + Grill Me 收口结论 + 实现期补齐的 Recent 标题/Open prompt/状态色/移除项需求。

## 1. 背景与目标

侧栏 `Agents` 分区原本每个条目只有一行。最终形态为每个条目固定两行：第一行显示 Agent 身份、运行状态与归属位置，第二行显示标题或 `(no title)`。

```text
❯ ● lx-native            [T1 P1]
  └─ Auth Flow
  ○ claude               [T1 P2]
  └─ (no title)
```

标题只允许两种来源：

1. Prompt 面板 Recent 列表中的页（保存页标题快照并绑定 `(item_id, page_id)`）；
2. 用户在标题弹窗中直接输入的自定义文本。

标题编辑只改侧栏显示，**不写 DB、不修改 `LxChat::session_title`、不修改输入框右上角标题胶囊、不影响 `/resume` 列表、不向终端写 OSC 标题**。重启后全部丢失。

右键菜单：

```text
┌─ lx-native ───────────┐
│ Edit title            │
│ Open prompt           │  ← 仅绑定仍可解析时显示
│ Unbind recent         │  ← 有绑定；自定义标题显示 Remove title
│ Close                 │  ← 行内二次确认
└───────────────────────┘
```

## 2. 最终决策

| 编号 | 决策维度 | 最终结论 |
|---|---|---|
| D1 | 移除语义 | 菜单不使用 `Delete`，统一 `Close`：只关闭，不删除 DB 内容 |
| D2 | lx `Close` | 等价 `/new`：停流、解绑当前会话、回空白欢迎页；DB 历史保留；窗格与同窗格终端 Agent 保留 |
| D3 | 终端 Agent `Close` | 关闭所属窗格并结束进程；同窗格 lx 会话随窗格关闭，但 DB 历史仍可在 `/resume` |
| D4 | 标题编辑范围 | lx 与终端 Agent 都提供 `Edit title`，全部只改侧栏显示 |
| D5 | 持久化 | 纯内存覆盖；不写 DB、不产生 Effect、不改真实会话/终端标题 |
| D6 | 默认标题 | V2 起**没有默认标题来源**：未设置覆盖时显示 `(no title)`；`session_title` 与终端 OSC 标题都不参与 |
| D7 | 空标题/清空 | 第二行始终存在；空输入提交 = 清除覆盖并显示 `(no title)` |
| D8 | 覆盖生命周期 | 切 Tab、切工作区、窗格重排后保留；lx 会话重置/切换、终端 Agent 退出/种类变化、窗格关闭时清除；重启丢失 |
| D9 | `Close` 确认 | 菜单行内二次确认：`Close` → `Confirm close`，切换选中项复位 |
| D10 | `[T1 P1]` 编号 | `T` = 当前工作区内 Tab 位置（1 基）；`P` = 窗格在 `layout.pane_ids()` 树序中的位置（1 基）；同一窗格 lx/终端两行一致；标签右对齐 |
| D11 | 终端唯一窗格 | 终端 Agent 是标签页唯一窗格时菜单不显示 `Close`；关闭整个 Tab 走标签栏菜单 |
| D12 | 标题编辑交互 | 独立 `AgentTitleDialog`：单行输入 + Recent 页快照列表；↑/↓ 选择并绑定；直接编辑转自定义；Enter 提交；Esc 取消 |
| D13 | 生成中关闭 lx | 允许；`new_session` 内部先 `stop_stream`，已落库消息保留 |
| D14 | 覆盖与真实标题 | 覆盖只影响侧栏；清除覆盖后不回退任何真实标题，直接 `(no title)` |
| D15 | 移除自动标题来源 | `lx.chat.session_title` 与 `PaneAgentSnapshot.title` 不再参与侧栏第二行 |
| D16 | Recent 选择绑定 | 选中 Recent 行时用页标题回填输入框，并绑定 `AgentPromptRef { item_id, page_id }` |
| D17 | 单输入框交互 | ↑/↓ 浏览 Recent 并回填/绑定；直接编辑输入框转为自定义并解绑；Enter 提交；Esc 取消；鼠标点行等价 ↑/↓；`Save` / `Clear` / `Cancel` 与键盘一致 |
| D18 | 标题快照 | 绑定时保存选中时的页标题文本；页后续改名不自动跟随 |
| D19 | Open prompt | 仅当绑定页仍存在且已入库时，在 `Edit title` 后插入 `Open prompt`；点击释放 Prompt 钉住、结算当前编辑、打开精确 item/page、展开并聚焦 Prompt 面板 |
| D20 | 生命周期沿用 | 绑定随覆盖一起清除/丢失，不单独持久化 |
| D21 | 弹窗状态色 | Recent 行页标题状态色与 Prompt 抽屉一致：todo 亮白、in_progress 黄、done 绿、无状态压暗；选中项保留状态前景并加粗 |
| D22 | 移除标题入口 | 有标题覆盖时菜单在 `Open prompt` 后追加移除项：带绑定显示 `Unbind recent`，自定义标题显示 `Remove title`；行内二次确认（`Confirm unbind` / `Confirm remove title`）后清空绑定与标题；绑定页失效仍可移除；无覆盖不显示 |
| D23 | 侧栏标题状态色 | Agents 第二行在绑定可解析时按绑定页实时模板状态着色，与 Recent 一致；悬停只叠加底色，不覆盖状态前景色；自定义/失效绑定压暗 |

## 3. 数据模型

### 3.1 条目与位置

`SidebarAgentEntry` 增加 `pane_index`：

```rust
pub struct SidebarAgentEntry {
    pub tab_index: usize,
    /// 窗格在 layout.pane_ids() 树序中的位置（0 基存储；显示时 +1）。
    pub pane_index: usize,
    pub pane_id: PaneId,
    pub kind: SidebarAgentKind,
    pub state: AgentState,
}
```

`AppState::agent_entries()` 按 Tab 顺序与 `layout.pane_ids()` 树序枚举；同一窗格先 lx 后终端 Agent，两条记录共享 `pane_index`。

### 3.2 标题覆盖

```rust
pub struct AgentPromptRef {
    pub item_id: i64,
    pub page_id: PromptPageId,
}

pub struct AgentTitleOverride {
    /// 选中时的显示快照。
    pub text: String,
    /// Some = 来自 Recent 页；None = 自定义输入。
    pub prompt: Option<AgentPromptRef>,
}
```

存放位置：

- lx：`LxChat::agent_title_override: Option<AgentTitleOverride>`；
- 终端：`Pane::terminal_agent_title: Option<(AgentKind, AgentTitleOverride)>`。

不使用全局 `HashMap`，避免工作区索引变化、PaneId 复用与跨重启残留。

### 3.3 生命周期

| 事件 | 结果 |
|---|---|
| 切 Tab / 切工作区 / 窗格重排 | 覆盖保留 |
| lx `/new`、删除当前会话、rewind 删空会话 | `reset_session_view` 清空 |
| lx `/resume` 切到另一会话 | `on_session_loaded_full` 的 `session_changed` 分支清空 |
| 同会话重载 | 保留 |
| AI Web 站点切换 | `reset_session_view` 清空 |
| 跨 AI Web / 本地入口切换 | 清空 lx 覆盖（包括没有 `session_id` 的 loading/shell 临时态） |
| 终端 Agent 退出、换 `AgentKind` | `update_pane_agent` 清空 |
| 终端 Agent 同种类 state/OSC 更新 | 保留 |
| 窗格关闭 | `Pane` 销毁，覆盖自然消失 |
| 进程重启 | 全部丢失 |

**关键不变量**：写入覆盖要求目标仍可接受标题（lx 会话视图存活；终端 kind 匹配）；**清除无条件生效**，避免条目消失后旧覆盖漂移到下一个会话。

### 3.4 标题解析

`Pane::agent_display_title(kind)` 只读 `AgentTitleOverride.text`，不读 `session_title` 或 OSC `title`。无覆盖/空文本返回 `None`，UI 显示 `(no title)`。

`AppState::agent_title_status(workspace, pane, kind)` 按条目所属工作区解析绑定页：

- 绑定页正是当前显示的 Prompt 编辑页时，读取编辑器实时正文；
- 否则读取内存页正文；
- 无绑定、绑定页删除、无模板块时返回 `None`。

状态色统一走 `style::template_page_title(Option<TemplateStatus>)`：todo 亮白、in_progress 黄、done 绿、无状态压暗。

## 4. 菜单模型

### 4.1 目标与命令

```rust
OverlayTarget::Agent {
    workspace: usize,
    tab: usize,
    pane: PaneId,
    kind: SidebarAgentKind,
    /// 打开菜单时是否带 Recent 绑定；固化移除项文案。
    bound: bool,
}

MenuCommand::{EditAgentTitle, OpenPrompt, RemoveAgentTitle, CloseAgent}
```

`EditAgentTitle` 不进入 `requires_confirm`；`RemoveAgentTitle` 与 `CloseAgent` 进入行内二次确认。

### 4.2 命令顺序

1. `EditAgentTitle`（恒有）
2. `OpenPrompt`（绑定页仍存在且已入库）
3. `RemoveAgentTitle`（存在标题覆盖）
4. `CloseAgent`（lx 恒有；终端仅当 Tab 内窗格数 > 1）

菜单标题使用 Agent 身份名（`text::sidebar_agent_label(kind)`），不使用可能被改写的标题文本。

### 4.3 移除项文案

移除项文案在打开菜单时固化：

- `bound = true` → `Unbind recent` / 确认态 `Confirm unbind`；
- `bound = false` → `Remove title` / 确认态 `Confirm remove title`。

菜单存续期间即使覆盖被外部清空，标签与菜单宽度也保持不变，避免指针落点漂移。

### 4.4 Close

- lx：复用 `update::chat::new_session`，先停流再重置视图，不产生 `DeleteChatSession`；
- 终端：复用 `close_pane`，保留最后窗格保护；
- 执行前再次校验目标仍存在；会话切换或终端换 kind 时直接关闭旧菜单/标题弹窗。

## 5. 标题弹窗

### 5.1 结构

`Overlay::AgentTitle(AgentTitleDialog)`：

```text
┌─ Edit title ─────────────────────────────┐
│ > title or pick recent                   │
│ ── recent ─────────────────────────────  │
│ ❯ Auth Flow                      2m      │
│   Backend Prompt                         │
│   Deploy Plan                    5m      │
│   Release Prompt                         │
│              ↵ save  ^c clear  esc cancel│
└──────────────────────────────────────────┘
```

- 输入框预填当前标题；已绑定时高亮对应 Recent 行。
- Recent 行快照在打开时生成；打开期间不随底部抽屉刷新，避免模态期间重排。
- 列表最多显示 5 条，超过时通过 ↑/↓ 滚动窗口；选中项始终可见。
- 空态显示 `no recent pages`。

### 5.2 交互

| 操作 | 行为 |
|---|---|
| ↑/↓ | 移动 Recent 高亮，回填页标题并建立绑定 |
| 直接输入/删除/粘贴/剪切 | 转为自定义标题，清除绑定与高亮 |
| Enter / `Save` | 提交；空输入清除覆盖 |
| `Clear` | 清空输入与绑定，弹窗保持打开 |
| Esc / `Cancel` / 点击弹窗外 | 取消，不提交 |
| 鼠标点击 Recent 行 | 等价 ↑/↓ |
| 点击输入框 | 进入文本选择/编辑 |

提交前校验目标仍存在且接受该 Agent 标题；目标失效时保留弹窗，不静默丢弃输入。

### 5.3 绑定有效性

- 绑定时保存 `item_id/page_id` 与页标题快照；
- 提交时再次确认页存在，否则存为自定义标题（无绑定）；
- 绑定页删除后：标题文本保留、`Open prompt` 隐藏、移除项仍为 `Unbind recent`；
- 只有绑定页存在且为 Saved 页时才允许 `Open prompt`。

## 6. 渲染与命中

### 6.1 两行条目

```text
第一行：marker(2) + dot(2) + name + padding + [T1 P1]
第二行：TREE_LEAF("  └─ ") + title + padding
```

- 第一行名称先按显示宽度截断，`[T1 P1]` 固定右对齐；
- 第二行前缀复用 `text::agent::TREE_LEAF`，`└` 与第一行状态圆点同列；
- 标题按显示宽度截断，CJK/emoji 不越界；
- 前缀恒为 muted；标题按绑定页状态着色；无标题 `(no title)` 为 muted；
- 悬停由 `ListItem.style` 铺满两行，标题前景保留状态色。

### 6.2 槽位模型

每个条目固定 2 行，滚动状态以条目为单位：

- `agent_visible_items(list) = (height / 2).max(1)`（滚动计算用）；
- `agent_content_rect(list)`：高度向下取偶；**高度 < 2 时返回 0 行**，避免“实际不渲染但命中层可点”的幽灵条目；
- 渲染、滚动条、命中统一使用 `agent_content_rect`；
- 点击/右键第一行或第二行都命中同一条目；底部留空行与滚动条列不命中；
- 滚动条 `visible` 使用条目数，不使用行数。

### 6.3 命中

- `agent_item_index_at`：`slot = agent_scroll + (row - content.y) / 2`；
- 左键 `agent_target_at` → `focus_agent_pane_view`；
- 右键 `agent_item_index_at` → `open_agent_menu`；
- 悬停/滚轮复用同一命中函数，滚动后按新偏移重算。

## 7. 事件接入

- 右键：工作区列表之后、右栏/主区之前命中 Agents 条目；
- 左键：沿用 `handle_sidebar_press`，两行同命中；
- 滚轮：只滚动 Agents 列表，不穿透；
- 标题弹窗：`OverlayKind::AgentTitle` 接管 Esc/Enter/↑/↓ 与文本编辑键；鼠标点击输入框、Recent 行、按钮、弹窗外；
- 会话切换 / 终端换 kind：关闭指向该窗格的 Agent 菜单与标题弹窗。

## 8. 不变量

1. **绝不写 DB**：标题编辑、移除、解绑不产生 Effect；`Open prompt` 的 `SetActivePage` 等属于 Prompt 自身行为。
2. **绝不改真实标题**：`LxChat::session_title`、标题胶囊、`/resume` 列表、终端 OSC 标题保持原样。
3. **绝不向终端写内容**：不因标题功能向 PTY 写标题、发信号。
4. **条目身份**：同一窗格 lx 与终端两行必须可区分；菜单目标与覆盖都带 `SidebarAgentKind`。
5. **条目消失即覆盖失效**：会话重置/切换、终端退出/换 kind、窗格关闭清除；清除路径不得被存活判断阻断。
6. **菜单/弹窗目标失效即关闭**：会话切换、终端换 kind 后旧浮层不得作用于新目标。
7. **最后窗格保护**：终端唯一窗格不提供 `Close`。
8. **二次确认**：`CloseAgent` 与 `RemoveAgentTitle` 必须行内确认。
9. **窄侧栏**：名称先截断，`[T1 P1]` 保留；第二行标题不越界。
10. **渲染只读**：`ui/` 只读 `AppState`，不修改覆盖、不执行关闭。

## 9. 文件清单

```text
src/app/overlay.rs                 OverlayTarget::Agent { bound }、AgentTitleDialog、
                                   MenuCommand::{EditAgentTitle, RemoveAgentTitle, CloseAgent}
src/app/state/pane.rs              SidebarAgentEntry.pane_index、AgentPromptRef、
                                   AgentTitleOverride、accepts_agent_title、
                                   agent_title_override / agent_display_title /
                                   set_agent_title_override
src/app/state/lx/chat.rs           LxChat.agent_title_override 初始化与 reset 清理
src/app/state.rs                   agent_entries 枚举 pane_index、agent_title_status(workspace, ...)
src/app/update/pane.rs             set_lx_entry 跨入口清空标题；update_pane_agent 清空/关浮层
src/app/update/chat/session.rs     会话切换清空覆盖并关闭 Agent 浮层
src/app/update/agent_title.rs      打开弹窗、Recent 选择、提交、移除、Open prompt、浮层失效
src/app/update/menu.rs             open_agent_menu / close_agent / 命令分派
src/app/update/dialog.rs           粘贴/剪切转自定义的逻辑
src/app/update/prompt/recent.rs    open_prompt_page 校验、当前页展开、Recent 行
src/ui/left_panel/mod.rs           两行渲染、[T1 P1]、状态色、槽位命中/滚动
src/ui/overlay/agent_title.rs      弹窗几何、命中、渲染、小屏保护
src/ui/overlay/menu.rs             Agent 菜单标题与命令文案
src/ui/style.rs                    template_page_title
src/ui/text/workspace.rs           AGENT_NO_TITLE、菜单/确认/弹窗文案
src/input/mod.rs                   AgentTitle 键位映射
src/event/overlay.rs               右键、弹窗鼠标、外部关闭
```

测试按模块归属：

```text
src/app/state/pane/tests.rs            标题解析、覆盖生命周期、清除语义
src/app/state/tests.rs                 跨工作区状态解析
src/app/update/menu/tests.rs           菜单命令、确认、关闭、弹窗、Open prompt、目标失效
src/app/update/prompt/recent/tests.rs  当前页展开、无效页拒绝、Recent 行
src/app/update/chat/web/tests.rs       跨 AI Web 入口清空临时标题
src/ui/left_panel/tests.rs             两行渲染、[T1 P1]、状态色、实时正文、命中/滚动
src/ui/overlay/agent_title/tests.rs    几何、命中、渲染、状态色、小屏
src/ui/overlay/tests.rs                菜单文案固化
src/ui/style/tests.rs                  页标题状态色映射
src/input/tests.rs / tests_agent_title.rs
src/event/tests/agent_menu.rs          右键、左键、滚轮、弹窗鼠标
```

## 10. 测试与验收

### 10.1 状态与生命周期

- 未设置覆盖时 lx / 终端都显示 `(no title)`，不读取 session/OSC 标题；
- 自定义标题、绑定标题、绑定快照；
- 无会话拒绝写入、无会话仍可清除；
- 终端 kind 变化/退出清空，同 kind 更新保留；
- 会话重置/切换、AI Web 跨入口清空；
- 跨工作区保留覆盖，状态按所属工作区解析。

### 10.2 菜单与弹窗

- 无标题：`[EditAgentTitle, CloseAgent]`；
- 自定义标题：`[EditAgentTitle, RemoveAgentTitle, CloseAgent]`；
- 有效绑定：`[EditAgentTitle, OpenPrompt, RemoveAgentTitle, CloseAgent]`；
- 终端唯一窗格无 `Close`；
- 移除项文案在菜单打开时固化；
- `Edit title` 打开弹窗并预填；
- ↑/↓、直接编辑、粘贴/剪切、Enter、Esc、Clear、Save、鼠标行点击；
- 绑定页删除后提交为自定义、菜单无 `Open prompt`；
- 会话切换/终端换 kind 关闭旧浮层；
- `Open prompt` 折叠时展开、钉住时释放并打开精确页。

### 10.3 渲染与命中

- 第一行 `[T1 P1]`，同窗格两行一致；
- 第二行 `"  └─ "`，无标题 `(no title)`；
- CJK 截断、悬停两行底色、状态色；
- 两行点击/右键同命中；
- 底部留空行与滚动条列不命中；
- 高度 < 2 行无命中槽位；
- 小屏弹窗不 panic、按钮不与输入行重叠。

### 10.4 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

## 11. 验收标准

- 每个 Agents 条目固定两行，第二行为标题或 `(no title)`。
- 第一行显示 `[T1 P1]`，T/P 为 1 基位置，同窗格两行一致。
- 标题只来自 Recent 或自定义输入；不出现会话标题/终端 OSC 标题。
- 右键菜单按顺序显示 `Edit title` / 可选 `Open prompt` / 可选移除项 / 可选 `Close`。
- `Close` 与移除项均需行内二次确认；`Close` 只关闭不删 DB。
- 绑定有效时 `Open prompt` 打开精确页并聚焦 Prompt 面板。
- 标题状态色与 Prompt Recent 一致；悬停保留状态前景。
- 所有标题修改只在内存；重启丢失；真实标题、`/resume`、标题胶囊、OSC 不变。
- 终端 Agent 是唯一窗格时不显示 `Close`。
- 两行条目的左键、右键、悬停、滚轮、滚动条、窄侧栏、小屏全部正确。
- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test --lib` 通过。

## 12. V1 历史（已被 V2 取代）

以下是实现过程中被 V2 取代的 V1 约定，仅作历史记录，不再作为实现依据：

- V1/D6：lx 默认读 `session_title`、终端默认读 OSC title；V2/D15 改为无默认标题来源。
- V1/D7：清空覆盖后回落默认标题；V2 改为 `(no title)`。
- V1/D12：复用 `Overlay::Rename` 单行输入；V2 改为 `AgentTitleDialog` + Recent 列表。
- V1/D14：覆盖清除后回落实时默认标题；V2 取消该回退。
- V1 `pane_index` 设计为 1 基存储；实现统一为 0 基存储、显示 +1。
