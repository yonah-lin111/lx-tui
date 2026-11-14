# 侧栏 Agents 标题与右键菜单 — 实现记录

> 设计：[design.md](design.md) ｜ 任务：[task.md](task.md)
> 分支：`feat/sidebar-agent-title-and-menu` ｜ 工作树：`.worktrees/sidebar-agent-title-and-menu`

## 1. 落地范围

| 决策 | 实现 |
|---|---|
| D1 `Delete` → `Close` | 菜单命令 `MenuCommand::CloseAgent`，文案 `Close`，只关闭不删内容 |
| D2 lx Close | 复用 `update::new_session`：停流、解绑会话、回欢迎页；DB 历史保留 |
| D3 终端 Close | 复用 `close_pane`：关闭目标窗格并结束进程 |
| D4 两端都可编辑标题 | `EditAgentTitle` 对 lx / 终端均提供 |
| D5 纯内存 | `LxChat.agent_title_override` 与 `Pane.terminal_agent_title`，不写 DB、不改真实标题 |
| D6 默认标题 | lx 取 `chat.session_title`；终端取 `PaneAgentSnapshot.title` |
| D7 空标题 | 第二行显示 `(no title)`；空输入清除覆盖 |
| D8 覆盖生命周期 | 会话重置/切换、Agent 退出/种类变化、窗格关闭时清除；重启丢失 |
| D9 行内二次确认 | `CloseAgent` 进入 `requires_confirm`；`Confirm close` |
| D10 `[T1 P1]` | `text::agent_location(tab_index, pane_index)`，两者 0 基入参、显示 +1 |
| D11 唯一窗格保护 | 终端 Agent 是 tab 唯一窗格时菜单只有 `Edit title` |
| D12 编辑交互 | 复用 `Overlay::Rename` 居中单行输入 |
| D13 生成中可关闭 | `new_session` 内部先 `stop_stream` |
| D14 覆盖优先 | `Pane::agent_display_title` 覆盖优先，清除后回落实时默认标题 |

## 2. 生产代码变更

```text
src/app/overlay.rs               OverlayTarget::Agent / MenuCommand::{EditAgentTitle, CloseAgent} /
                                 RenameTarget::AgentTitle / CloseAgent 行内确认
src/app/state.rs                 agent_entries 枚举 pane_index
src/app/state/pane.rs            SidebarAgentEntry.pane_index、Pane.terminal_agent_title、
                                 agent_display_title / set_agent_title_override
src/app/state/lx/chat.rs         LxChat.agent_title_override 初始化与 reset 清理
src/app/update/pane.rs           update_pane_agent 在 AgentKind 变化/退出时清空终端覆盖
src/app/update/chat/session.rs   on_session_loaded_full 会话切换清空 lx 覆盖
src/app/update/menu.rs           open_agent_menu / close_agent / activate_menu 分支
src/app/update/dialog.rs         commit_rename 支持 AgentTitle 空值清除
src/ui/left_panel/mod.rs         两行条目、[T1 P1]、条目槽位滚动/命中模型
src/ui/overlay/menu.rs           Agent 菜单标题与命令文案
src/ui/overlay/dialog.rs         Agent 重命名浮层标题
src/ui/text/workspace.rs         agent_location(tab, pane)、AGENT_NO_TITLE、菜单/重命名文案
src/ui/main_content/tests.rs     Pane 字面量补 terminal_agent_title
src/event/overlay.rs             右键 Agents 条目打开 Agent 菜单
```

测试新增/修改：

```text
src/app/state/pane/tests.rs           10 项：标题解析、覆盖优先级、生命周期、kind 切换清理
src/app/update/menu/tests.rs          13 项：菜单命令集合、二次确认、关闭分派、重命名提交
src/app/update/chat/tests/session.rs  1 项：同会话重载保留覆盖、切会话清空覆盖
src/ui/left_panel/tests.rs            两行渲染、[T1 P1]、命中、悬停、奇数高度、窄侧栏、CJK 截断
src/ui/text/tests.rs                  agent_location 格式与新增英文文案
src/event/tests/agent_menu.rs         6 项：两行右键、唯一窗格无 Close、左键跳转、滚动后命中
src/event/tests/mouse_click.rs        双条目点击行偏移改为 +2，并覆盖标题行点击
```

## 3. 与设计的偏差

1. **`pane_index` 存储改为 0 基**：设计写的是 1 基存储。实现改为与 `tab_index` 一致的下标语义（0 基），由 `agent_location` 统一 +1 显示。
   - 原因：避免 `agent_location(tab_index, pane_index)` 一个 0 基、一个 1 基的隐性约定；测试与调用方直接传条目字段即可。
   - 显示结果不变：首窗格仍为 `[T1 P1]`。
2. **新增 `RENAME_AGENT_TITLE`**：设计只要求复用 Rename 浮层，未指定浮层标题；实现为 `Edit title`。
3. **`set_lx_entry` 的注释修正**：设计假设所有入口切换都会 `reset_session_view`。实际 Native/Hybrid/Automation 互切保留同一会话，因此侧栏覆盖会保留；AI Web 跨越或站点切换仍会重置并清空。该行为符合 D8（只有会话关闭/切换才清除）。
4. **UI 测试文件行数**：`src/ui/left_panel/tests.rs` 从 819 行增至 1140 行，低于 1500 行上限，未再拆分。

## 4. 验证结果

```text
cargo fmt                                             通过
cargo clippy --all-targets -- -D warnings             通过
cargo test --lib                                      5471 passed, 0 failed, 6 ignored
```

专项：

```text
cargo test --lib app::state::pane::                   10 passed
cargo test --lib app::update::menu::                  13 passed
cargo test --lib ui::left_panel::                     34 passed
cargo test --lib ui::text::                           47 passed
cargo test --lib event::tests::agent_menu::            6 passed
cargo test --lib event::tests::mouse_click::          23 passed
cargo test --lib event::tests::mouse_drag::            9 passed
```

## 5. 未验证与风险

- 未运行 `cargo run` 做人工视觉确认：右对齐标签、两行悬停背景、菜单确认态需用户在真实终端确认。
- 终端 Agent 的唯一窗格保护在“菜单打开后窗格被外部关闭”的竞态下仍由 `close_pane` 的最后窗格保护兜底，不会误关。
- 侧栏标题覆盖只在内存，重启后回到默认标题；这是需求约定，不是缺陷。

---

## 6. V2 补充：Recent 标题与 Open prompt

> 用户追加需求：标题只能来自 Prompt 面板 Recent 列表或自定义输入；右键菜单增加打开该提示词的入口。

### 6.1 落地范围

| 决策 | 实现 |
|---|---|
| D15 移除自动标题来源 | `Pane::agent_display_title` 只读 `AgentTitleOverride`；`session_title` / OSC `title` 不再参与 |
| D16 Recent 选择绑定页 | `AgentTitleDialog` 快照目标工作区 Recent 行；选择后回填页标题并绑定 `(item_id, page_id)` |
| D17 单输入框交互 | ↑/↓ 回填并绑定；直接编辑转自定义并解除绑定；Enter 提交；Esc 取消 |
| D18 标题快照 | `AgentTitleOverride.text` 保存选中时页标题；后续页改名不自动跟随 |
| D19 Open prompt | 绑定有效时菜单插入 `OpenPrompt`；打开精确页并聚焦 Prompt 面板 |
| D20 生命周期沿用 | 覆盖随会话重置/切换、Agent 退出/种类变化、窗格关闭清除；重启丢失 |
| D21 状态色对齐 | 弹窗 Recent 页标题复用 Prompt 抽屉状态色（todo 亮白、in_progress 黄、done 绿、无状态压暗） |

### 6.2 生产代码变更

```text
src/app/state/pane.rs             AgentPromptRef / AgentTitleOverride
                                  Pane::agent_title_override / agent_display_title / set_agent_title_override
src/app/state/lx/chat.rs          LxChat.agent_title_override 改为 Option<AgentTitleOverride>
src/app/state.rs                  重导出 AgentPromptRef / AgentTitleOverride
src/app/overlay.rs                AgentTitleDialog、OverlayKind::AgentTitle、Overlay::AgentTitle
src/app/update/agent_title.rs     打开弹窗、Recent 选择、编辑转自定义、提交、Open prompt
src/app/update/menu.rs            EditAgentTitle 打开新弹窗；绑定有效时追加 OpenPrompt
src/app/update/dialog.rs          AgentTitle 分支路由；移除 RenameTarget::AgentTitle 提交
src/app/update/prompt/recent.rs   open_prompt_page(item_id, page_id)、prompt_recent_rows_for_workspace
src/ui/overlay/agent_title.rs     弹窗几何、命中与渲染
src/ui/overlay.rs                 dispatch 与导出
src/input/mod.rs                  AgentTitle 键位映射
src/event/overlay.rs              鼠标点击、输入框命中与外部关闭
src/ui/text/workspace.rs          AGENT_TITLE_PLACEHOLDER
```

### 6.3 与设计的偏差

1. **不复用 `Overlay::Rename`**：设计 V1/D12 要求复用 Rename 浮层，但 Recent 列表与行选择无法塞进单行 Rename；新增独立 `AgentTitleDialog`。
2. **复用 `MenuCommand::OpenPrompt`**：未新增命令；通过 `OverlayTarget::Agent` 分支分派到 `open_agent_prompt`，菜单文案保持 `Open prompt`。
3. **Recent 行快照**：弹窗打开时生成 `Vec<RecentPageRow>`，打开期间不随底部抽屉刷新；避免模态期间列表重排导致选择错位。
4. **按工作区取 Recent**：新增 `prompt_recent_rows_for_workspace`；Prompt 面板被钉在别的工作区时，Agents 弹窗仍取条目所属工作区的 Recent。
5. **提交时再次校验绑定**：弹窗打开后页被删除，Enter 会将文本存为自定义标题而不是保留失效绑定；菜单不会提供 `Open prompt`。
6. **空输入不回落**：提交空输入直接清除覆盖，第二行显示 `(no title)`；这是“只允许 Recent 或自定义”的直接结果。

### 6.4 验证结果

```text
cargo fmt                                             通过
cargo clippy --all-targets -- -D warnings             通过
cargo test --lib                                      5498 passed, 0 failed, 6 ignored
```

专项（V2）：

```text
cargo test --lib app::state::pane::                   11 passed
cargo test --lib app::update::menu::                  23 passed
cargo test --lib app::update::prompt::recent::        10 passed
cargo test --lib ui::overlay::agent_title::            7 passed
cargo test --lib input::tests_agent_title::            1 passed
cargo test --lib event::tests::agent_menu::           12 passed
```

### 6.5 未验证与风险

- 未运行 `cargo run` 做人工视觉确认：弹窗高度、Recent 选中底色、按钮间距需真实终端复核。
- 弹窗列表最多显示 5 条，超过时通过 ↑/↓ 自动滚动；未接入滚轮/滚动条。
- 绑定页被删除后标题文本仍保留为快照，`Open prompt` 隐藏；这是有意的降级行为。
- 所有条目初始显示 `(no title)`，不再自动出现会话/终端标题；这是 V2 需求约定。
