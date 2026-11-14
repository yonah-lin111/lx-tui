# 侧栏 Agents 条目第二行标题与右键菜单 — 执行任务

> 状态：**已实现、通过审查修复并合入 dev（V2）**
> 设计依据：[design.md](design.md) ｜ 审查记录：[review.md](review.md)
> 规则：生产代码禁止 `unwrap()` / `expect()`；用户可见文案全部收敛到 `ui/text/`；`app/` 管状态与更新、`ui/` 只读渲染、`event/` 只分派；全部标题修改只走内存，不写 DB、不改会话/终端真实标题。

## P0. 基线核对

- [x] 用户确认需求与设计。
- [x] 工作树 `git status --short` 干净，无未提交用户改动。
- [x] 复读 `docs/standards/` 下的目录、代码与 TUI 规范。
- [x] 用 Codegraph / Codebase Memory 核对 `agent_entries`、`AgentListItem`、`agent_item_index_at`、`LxChat::reset_session_view`、`on_session_loaded_full`、`update_pane_agent`、`MenuCommand`、`OverlayTarget`、`open_right_click_menu` 等符号。

## P1. 状态层

### 1.1 条目序号

- [x] `SidebarAgentEntry` 增加 `pane_index`，按 `layout.pane_ids()` 树序 0 基存储。
- [x] `agent_entries()` 同时为 lx 与终端条目写入同一 `pane_index`。
- [x] `agent_entry_count()` 保持只计数，不引入标题解析。
- [x] `[T1 P1]` 显示时统一 +1，同一窗格两行一致。

### 1.2 标题覆盖

- [x] `AgentPromptRef { item_id, page_id }`。
- [x] `AgentTitleOverride { text, prompt }`；`text` 为选择时快照。
- [x] lx 覆盖存 `LxChat.agent_title_override`。
- [x] 终端覆盖存 `Pane.terminal_agent_title: Option<(AgentKind, AgentTitleOverride)>`。
- [x] `agent_display_title` 只读覆盖，不读 `session_title` / OSC 标题。
- [x] `accepts_agent_title`：lx 要求会话视图存活，终端要求 kind 匹配。
- [x] 写入要求目标可接受标题；**清除无条件生效**。

### 1.3 生命周期

- [x] `/new`、删除当前会话、rewind 删空会话：`reset_session_view` 清空。
- [x] `/resume` 切到另一会话：`on_session_loaded_full` 的 `session_changed` 分支清空。
- [x] 同会话重载保留。
- [x] AI Web 站点切换：`reset_session_view` 清空。
- [x] 跨 AI Web / 本地入口切换：即使没有 `session_id` 也清空残留标题。
- [x] 终端 Agent 退出/换 kind：`update_pane_agent` 清空。
- [x] 同种类 state/OSC 更新保留。
- [x] 窗格关闭自然销毁；重启丢失。
- [x] 切 Tab/工作区/窗格重排保留。

### 1.4 状态测试

- [x] lx/终端未设置覆盖时 显示 `(no title)`，不读 session/OSC。
- [x] 自定义、绑定、清除语义。
- [x] 无会话拒绝写入、无会话仍可清除。
- [x] 终端 kind 变化/退出清空，同 kind 更新保留。
- [x] 会话重置/切换清空。
- [x] 跨工作区覆盖保留；`agent_title_status(workspace, ...)` 按所属工作区解析。
- [x] 正在编辑页的实时状态色。

## P2. 菜单与更新

### 2.1 菜单模型

- [x] `OverlayTarget::Agent { workspace, tab, pane, kind, bound }`。
- [x] `MenuCommand::{EditAgentTitle, RemoveAgentTitle, CloseAgent}`；`OpenPrompt` 复用既有命令。
- [x] `CloseAgent` 与 `RemoveAgentTitle` 进入 `requires_confirm`。
- [x] `bound` 在打开菜单时固化，移除项文案不随实时状态翻转。

### 2.2 打开菜单

- [x] `open_agent_menu` 防御越界并结束滚动条拖拽。
- [x] 命令顺序：`Edit title` → 可选 `Open prompt` → 可选移除项 → 可选 `Close`。
- [x] lx 恒有 `Close`；终端仅在 Tab 内窗格数 > 1 时有 `Close`。
- [x] 终端唯一窗格不显示 `Close`。
- [x] 菜单目标记录完整身份与绑定快照。

### 2.3 执行

- [x] `Edit title` 打开 `AgentTitleDialog`，预填当前标题与绑定。
- [x] `Open prompt` 释放 Prompt 钉住、打开精确页、展开并聚焦 Prompt 面板。
- [x] 移除项清空绑定与标题；绑定页失效仍可移除。
- [x] `Close` 执行前再次校验目标仍存在。
- [x] lx `Close` 等价 `/new`，不产生 `DeleteChatSession`。
- [x] 终端 `Close` 复用 `close_pane`。
- [x] 会话切换/终端换 kind 时关闭旧菜单与标题弹窗。
- [x] 菜单/弹窗目标失效时不落到新会话。

### 2.4 菜单测试

- [x] 无标题/自定义/有效绑定的命令集合。
- [x] 唯一窗格无 `Close`。
- [x] `Close` 行内二次确认与切换复位。
- [x] lx Close 后 `session_id=None`、消息清空、无 Delete Effect。
- [x] 终端 Close 移除目标窗格。
- [x] 目标消失/会话切换不误关其他 Agent。
- [x] 移除项文案在覆盖变化后保持稳定。
- [x] 终端 `Edit title` → 提交闭环；不产生 Effect、不改 OSC 快照。

## P3. 标题弹窗

### 3.1 模型与几何

- [x] `AgentTitleDialog`、`OverlayKind::AgentTitle`、`Overlay::AgentTitle`。
- [x] 打开时快照目标工作区 Recent 页。
- [x] 高度随条目数增长并夹取；最多显示 5 条。
- [x] 输入行、`recent` 分割线、双行列表、底部 `Save/Clear/Cancel`。
- [x] 空态 `no recent pages`。
- [x] 小屏下不画越界分割线/空态；无独立按钮行时不命中按钮。

### 3.2 交互

- [x] 预填当前标题；已绑定高亮对应 Recent 行。
- [x] ↑/↓ 回填页标题并建立绑定。
- [x] 直接输入/删除/粘贴/剪切转自定义并解绑。
- [x] 全选后粘贴相同文本不误解绑。
- [x] Enter 提交；空输入清除覆盖。
- [x] Esc / Cancel / 点击弹窗外取消。
- [x] `Clear` 清空输入与绑定且保持弹窗。
- [x] 鼠标点击 Recent 行等价 ↑/↓。
- [x] 提交前校验目标；目标失效保留弹窗。

### 3.3 Open prompt 与绑定

- [x] 绑定仅在页仍存在且为 Saved 时提供 `Open prompt`。
- [x] `open_prompt_page` 先校验 Saved 与页存在，再短路当前页。
- [x] 当前页是绑定页且 Prompt 折叠时自动展开。
- [x] 页删除后提交为自定义标题，菜单不提供 `Open prompt`。
- [x] 移除项仍为 `Unbind recent`。

## P4. UI 渲染与命中

- [x] `AgentListItem` 增加 `pane_index`、`title`、`status`。
- [x] 两行条目：第一行 marker/dot/name/location；第二行 `TREE_LEAF` + 标题。
- [x] `[T1 P1]` 右对齐，名称先截断。
- [x] 标题按显示宽度截断，CJK/emoji 不越界。
- [x] 标题状态色统一走 `style::template_page_title`。
- [x] 悬停两行铺底色，标题状态前景不被覆盖。
- [x] `agent_visible_items` / `agent_content_rect` 统一渲染、滚动条与命中。
- [x] 高度 < 2 行时内容区为 0，不产生幽灵命中。
- [x] 两行点击/右键同命中；底部留空行与滚动条列不命中。
- [x] 奇数高度向下取整，滚动条按条目数计算。

## P5. 事件

- [x] 右键 Agents 条目打开 Agent 菜单，不误开工作区/Tab/Pane 菜单。
- [x] 右键第二行同样命中。
- [x] 左键两行聚焦对应 Pane 与目标视图。
- [x] 滚轮滚动后悬停/右键按新偏移命中。
- [x] 标题弹窗接管 Esc/Enter/↑/↓ 与文本编辑键。
- [x] 点击输入框、Recent 行、按钮、弹窗外行为与键盘一致。
- [x] 会话切换/终端换 kind 关闭旧浮层。

## P6. 审查修复

### P1（必须修复）

- [x] `Open prompt` 当前页分支展开折叠的 Prompt 面板。
- [x] lx 标题清除无条件生效；跨 AI Web 入口清空临时态标题。
- [x] 会话切换/终端换 kind 关闭 Agent 菜单与标题弹窗。
- [x] 提交标题前校验目标；失效时保留弹窗。

### P2（应当修复）

- [x] 全选后粘贴相同文本保持 Recent 绑定。
- [x] `open_prompt_page` 拒绝 Temp / 不存在页，先校验再短路。
- [x] `agent_title_status` 增加 `workspace` 参数，按所属工作区解析。
- [x] `style::template_page_title` 统一三处状态色映射。
- [x] 移除项文案在菜单打开时固化。
- [x] 小屏弹窗分割线/空态/按钮行保护。

### P3（已完成 / 记录）

- [x] Agents 区高度 < 2 行时无命中槽位。
- [x] 移除未使用的弹窗几何导出。
- [x] 输入层“所有文本浮层”测试补 `AgentTitle`。
- [x] 补 Esc/↑/实时状态色/无 Effect/跨工作区/终端提交闭环等测试。
- [x] 更新设计、任务、审查、TUI 标准与代码注释。
- [ ] 标题状态每帧全量扫描绑定页正文：记录为性能债，后续按内容 revision 缓存或轻量早退扫描。
- [ ] `src/app/state/lx/chat.rs` 超过 1000 行：既有结构债，不在本功能内拆分。

## P7. 验证与收尾

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --lib`
- [x] 生产代码无新增 `unwrap()` / `expect()`。
- [x] 无 `println!` / `eprintln!` 调试输出。
- [x] 用户可见文案全英文。
- [x] 标题编辑不写 DB、不产生 Effect、不改真实标题。
- [x] `review.md` 记录 P0–P3、根因、修复与残余风险。

## 验收清单

- [x] 每个 Agents 条目固定两行；第二行为标题或 `(no title)`。
- [x] 第一行显示 `[T1 P1]`，T/P 为 1 基位置；同窗格两行一致。
- [x] 右键菜单按顺序显示 `Edit title` / 可选 `Open prompt` / 可选移除项 / 可选 `Close`。
- [x] `Close` 行内二次确认；lx 只关会话视图，终端只关窗格；不删 DB。
- [x] 移除项行内二次确认；绑定标题显示 `Unbind recent`，自定义标题显示 `Remove title`。
- [x] 标题弹窗支持 Recent 选择与自定义输入；空输入清除覆盖。
- [x] `Open prompt` 打开精确页并展开/聚焦 Prompt 面板。
- [x] 所有标题修改只存在内存；重启丢失；会话/终端真实标题不变。
- [x] 终端 Agent 是唯一窗格时不显示 `Close`。
- [x] 两行条目的左键、右键、悬停、滚轮、滚动条、窄侧栏、小屏全部正确。
- [x] 受影响的 fmt / clippy / 单元测试全部通过。
- [ ] 真实终端人工验收（`cargo run`）：弹窗高度、Recent 选中底色、按钮间距、小屏退化。
