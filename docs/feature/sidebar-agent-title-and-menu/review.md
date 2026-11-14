# 侧栏 Agents 标题与右键菜单 — 审查记录

> 设计：[design.md](design.md) ｜ 任务：[task.md](task.md)
> 分支：`feat/sidebar-agent-title-and-menu`（已合入 `dev`）｜ 工作树：`.worktrees/sidebar-agent-title-and-menu`（已移除）
> 审查范围：`dev..HEAD` 的 7 个提交（`6ba04d88`、`179389d8`、`f836e21c`、`7284e1d6`、`95e567e5`、`a97a64f0`、`edca616d`）
> 审查方式：Codegraph / Codebase Memory 结构核对、逐文件静态审查、专项测试与全量 `cargo test --lib`、3 个并行只读审查子任务。

## 1. 关键结论

- **主路径成立**：Agents 每个条目固定两行；标题只来自 Recent 页绑定或自定义输入，不写 DB、不改 `session_title`、不改终端 OSC 标题、不影响 `/resume` 与标题胶囊。
- **生命周期基本闭环**：`/new`、删除会话、会话切换、AI Web 站点切换、终端 Agent 退出/种类变化、窗格关闭都会清空覆盖；重启丢失。
- **菜单与弹窗闭环**：`Edit title` → 标题弹窗；有效绑定追加 `Open prompt`；有覆盖追加 `Unbind recent` / `Remove title`；`Close` 与移除项均需行内二次确认。
- **主要风险不在功能主路径，而在状态身份竞态与文档漂移**：菜单/弹窗打开期间会话被异步切换，动作可能落到新会话；V1 设计段落与最终 V2 实现互相矛盾。
- **未发现 P0**（无崩溃、数据丢失、越权写库或安全边界破坏）。

## 2. 问题清单（按 P0 → P3）

### P0 — 阻断

无。

### P1 — 必须修复

#### P1-1（已修复）`Open prompt` 在绑定页已是当前页时不会展开折叠的 Prompt 面板

- **位置**：`src/app/update/prompt/recent.rs` 当前页短路分支。
- **现象**：Prompt 面板折叠、绑定页恰好是当前编辑页时，点击 `Open prompt` 只切换焦点和视图，不展开面板；焦点落到不可见的编辑器。
- **错误原因**：当前页分支只设置 `PromptMode::Editor`、`focus_prompt` 并关闭抽屉，没有执行非当前页分支里的 `state.prompt_collapsed = false`。
- **修复**：当前页分支同样展开面板；补“折叠 + 当前页”回归测试。

#### P1-2（已修复）Lx 标题清除被 `has_active_session()` 绑死，临时态可留下无法清除的覆盖

- **位置**：`src/app/state/pane.rs::set_agent_title_override`、`src/app/update/pane.rs::set_lx_entry`。
- **现象**：`has_active_session()` 包含 `loading_session` / `Committing` / shell 等无稳定 `session_id` 的临时态，允许写入标题；条目消失后清除分支同样被该判断拒绝。加载失败或跨入口清除 `loading_session` 后，旧标题可能留到下一次未重置的会话。
- **错误原因**：同一个“目标仍可写”判断同时用于设置与清除；`set_lx_entry` 跨 AI Web 时只在 `incompatible` 时重置，没有独立清理标题。
- **修复**：设置要求目标可接受标题，清除无条件生效；跨 AI Web 入口切换直接清空 lx 标题覆盖；补无会话清除与跨入口清理测试。

#### P1-3（已修复）菜单/标题弹窗打开期间会话切换，动作会落到新会话

- **位置**：`src/app/overlay.rs::OverlayTarget::Agent`、`src/app/update/menu.rs::close_agent`、`src/app/update/chat/session.rs::on_session_loaded_full`、`src/app/update/pane.rs::update_pane_agent`。
- **现象**：在会话 A 上打开 `Close` 菜单或标题弹窗，异步 `/resume` 到 B 完成后，目标校验只比较 `tab/pane/kind`，仍会通过；再次确认 `Close` 会关闭 B（生成中还会停流），标题也会写到 B。
- **错误原因**：`OverlayTarget::Agent` 没有会话身份；菜单/弹窗没有在会话切换时失效。
- **修复**：会话实际切换、终端 Agent 种类变化时关闭指向该窗格的 Agent 菜单/标题弹窗；补 A→B 切换后菜单与弹窗均已失效的测试。

#### P1-4（已修复）`commit_agent_title` 先吞掉弹窗，目标失效时静默丢弃输入

- **位置**：`src/app/update/agent_title.rs::commit_agent_title`。
- **现象**：弹窗打开后窗格关闭、lx 会话结束或终端 Agent 换 kind，按 Enter 仍会关闭弹窗，但标题未保存且无任何反馈。
- **错误原因**：函数开头直接 `state.overlay.take()`，之后才查找目标；失败分支没有恢复或提示。
- **修复**：提交前校验目标仍存在且接受该 Agent 标题；失败时保留弹窗，让用户可取消或改目标；补目标失效时弹窗保留的测试。

### P2 — 应当修复

#### P2-1（已修复）全选后粘贴与当前标题相同的文本会解除 Recent 绑定

- **位置**：`src/app/update/dialog.rs::apply_overlay_paste`。
- **现象**：绑定 Recent 后全选并粘贴同标题，文本未变，但 `mark_custom()` 被调用，`Open prompt` 消失，移除项从 `Unbind recent` 变为 `Remove title`。
- **错误原因**：只判断 `insert_str` 是否执行，没有比较粘贴前后文本；键盘编辑路径已经做了前后比较。
- **修复**：粘贴前后文本不同才 `mark_custom()`；补相同文本粘贴测试。

#### P2-2（已修复）`open_prompt_page` 在存在性检查前短路当前页，且不校验已入库页

- **位置**：`src/app/update/prompt/recent.rs::open_prompt_page`。
- **现象**：当前页分支只比较 `editing` 与 `active_page`，不验证 item/page 仍存在，也不拒绝 `Temp` 页；与函数注释“只接受已入库页”不符。
- **错误原因**：重构时保留了旧 `open_recent_page` 的语义注释，但校验没有迁移到公共入口。
- **修复**：入口先拒绝非 Saved 页并确认 item/page 存在，再做当前页短路；补不存在页与 Temp 页测试。

#### P2-3（已修复）`agent_title_status` 用活动工作区的 Prompt 面板解析任意 pane 的绑定

- **位置**：`src/app/state.rs::agent_title_status`。
- **现象**：当前 UI 只查询活动工作区条目，所以未显现；但 API 一旦用于非活动条目，会把 A 工作区的 item/page id 拿到 B 工作区的树里解析，状态色错误。
- **错误原因**：签名没有 workspace，内部固定使用 `self.active_workspace`；与其他标题 API（都显式带 workspace）不一致。
- **修复**：签名增加 `workspace`，按条目所属工作区解析；UI 传入 `state.active_workspace`；补跨工作区状态解析测试。

#### P2-4（已修复）页标题状态色映射在三处重复，Todo 特例容易漏改

- **位置**：`src/ui/left_panel/mod.rs`、`src/ui/overlay/agent_title.rs`、`src/ui/right_panel/prompt/recent.rs`。
- **现象**：三处都手写 `Todo → style::text()`、其余 `template_status`、`None → muted`；`style::template_status(Todo)` 本身返回 muted，依赖调用点覆盖。
- **错误原因**：缺少“页标题状态色”的单一语义函数。
- **修复**：在 `ui/style.rs` 增加 `template_page_title(Option<TemplateStatus>)`，三处统一引用；补样式映射测试。

#### P2-5（已修复）移除项文案随实时绑定状态翻转，菜单宽度会自行变化

- **位置**：`src/app/overlay.rs::OverlayTarget::Agent`、`src/ui/overlay/menu.rs::menu_labels`。
- **现象**：菜单打开期间终端 Agent 退出或覆盖被清空，`Unbind recent` 会变成 `Remove title`，菜单宽度收缩；指针停在原右边界时可能点到菜单外，确认文案也会跳变。
- **错误原因**：目标只存 `pane/kind`，文案在渲染时重新查询实时状态。
- **修复**：打开菜单时把“是否带绑定”固化进 `OverlayTarget::Agent`，菜单存续期间文案稳定；补菜单打开后覆盖变化仍保持标签的测试。

#### P2-6（已修复）小屏下标题弹窗的分割线/空态/按钮行越界并与输入行重叠

- **位置**：`src/ui/overlay/agent_title.rs`。
- **现象**：终端高度 3–4 行时，`modal::layout` 的 `inner.height` 可低至 1；分割线画到内框之外，空态文字写出列表区，按钮行被夹到输入行，鼠标点按钮实际落在输入框。
- **错误原因**：渲染与命中没有对 `inner.height` / `list_area.height` 做最小行数保护；同类弹窗已有 `shell.inner.height > 1` 的保护。
- **修复**：分割线、空态、按钮行分别加高度守卫；按钮命中在无按钮行时返回 None；补小屏渲染与命中测试。

### P3 — 建议修复 / 记录

#### P3-1（已修复）Agents 区高 1 行时存在“不可见但可点”的条目

- **位置**：`src/ui/left_panel/mod.rs::agent_visible_items / agent_content_rect / agent_item_index_at`。
- **现象**：`agent_visible_items` 用 `.max(1)` 保证滚动计算，内容区高 1 行时 ratatui 实际渲染 0 条，但命中层仍把该行映射到第 0 条。
- **修复**：内容区高度不足 2 行时按 0 行处理，命中与滚动条均返回 None；保留滚轮分区判定。

#### P3-2（已修复）`ui/overlay.rs` 导出未使用的弹窗几何函数

- **位置**：`src/ui/overlay.rs` 的 `pub use agent_title::{agent_title_max_rows, agent_title_visible_start, ...}`。
- **修复**：仅保留事件层与测试实际使用的导出。

#### P3-3（已修复）输入层“所有文本浮层”回归测试漏掉 `AgentTitle`

- **位置**：`src/input/tests.rs::all_text_overlays_route_word_ops_and_cut`。
- **修复**：列表补 `OverlayKind::AgentTitle`。

#### P3-4（部分修复）测试缺口

- **缺口**：Esc/Cancel 取消、无选中时按 ↑、侧栏实时编辑状态色、标题编辑无 Effect、跨工作区标题保留、终端 Edit→提交闭环、滚动条按偶数内容高度计算。
- **修复**：至少补 Esc、↑、实时状态色、无 Effect、跨工作区与终端提交闭环；其余记录为后续。

#### P3-5（已修复）文档与注释漂移

- **修复前位置**：`design.md` 第 1–11 节仍按 V1 写默认会话/OSC 标题、Rename 浮层、`pane_index` 1 基；`task.md` 全部未勾选；`review.md` 原为实现日志；`src/ui/left_panel/mod.rs` 的 `title` 注释仍写“其次会话标题 / 终端 OSC 标题”；`docs/standards/tui-design-requirements.md` 的 Agents 行仍写 `[tab N]`。
- **修复**：按最终 V2 重写设计/任务/审查文档，更新标准与代码注释。

#### P3-6（保留）标题状态每帧全量扫描绑定页正文

- **位置**：`src/app/state.rs::agent_title_status` → `page_title_status` → `markdown::block_scan`。
- **风险**：绑定到数千行 Prompt 时，侧栏每次重绘都分配并扫描整页；流式输出期间重绘频繁。
- **处理**：本次保留，记录为性能债；后续可按内容 revision 缓存首个模板块状态，或提供轻量早退扫描。

#### P3-7（保留）`src/app/state/lx/chat.rs` 超过 1000 行

- **现状**：`dev` 基线已 1057 行，本分支仅新增 5 行字段/清理。
- **处理**：属于既有结构债，不在本功能内拆分；本次不扩大该文件。

## 3. 修复与验证

| 编号 | 修复 | 回归测试 |
|---|---|---|
| P1-1 | 当前页分支补 `prompt_collapsed = false` | `recent::tests::opening_current_page_expands_collapsed_prompt` |
| P1-2 | 清除无条件生效；跨 AI Web 入口清空临时态标题 | `pane::tests::lx_override_write_requires_live_session_but_clear_is_unconditional`、`web::tests::switching_to_ai_web_clears_stale_agent_title_without_session` |
| P1-3 | 会话切换/终端换 kind 关闭 Agent 菜单与标题弹窗 | `menu::tests::session_switch_closes_agent_menu_and_title_dialog`、`terminal_kind_change_closes_agent_menu` |
| P1-4 | 提交前校验目标；失效保留弹窗 | `menu::tests::commit_after_target_lost_keeps_dialog_and_does_not_write_title` |
| P2-1 | 粘贴前后文本不同才转自定义 | `menu::tests::pasting_same_bound_title_keeps_recent_binding` |
| P2-2 | 入口先校验 Saved/页存在，再短路 | `recent::tests::open_prompt_page_rejects_missing_and_temp_pages` |
| P2-3 | `agent_title_status(workspace, ...)` 按所属工作区解析 | `state::tests::agent_title_status_resolves_the_entry_workspace` |
| P2-4 | 新增 `style::template_page_title` 并替换三处映射 | `style::tests::template_page_title_maps_all_statuses_consistently` |
| P2-5 | `OverlayTarget::Agent.bound` 固化移除项文案 | `ui::overlay::tests::agent_remove_title_label_stays_frozen_after_binding_cleared` |
| P2-6 | 小屏分割线/空态/按钮行守卫 | `ui::overlay::agent_title::tests::tiny_shell_has_no_button_row_to_overlap_input` |
| P3-1 | 高度 < 2 行时内容区为 0 | `left_panel::tests::one_row_agents_area_has_no_hit_slot` |
| P3-2 | 移除未使用导出 | 编译与 clippy |
| P3-3 | 文本浮层测试补 `AgentTitle` | `input::tests::all_text_overlays_route_word_ops_and_cut` |
| P3-4 | 补 Esc、无选中时 ↑、实时状态色、无 Effect、跨工作区、终端提交闭环 | `menu::tests::escape_cancels_title_dialog_without_committing`、`up_from_no_recent_selection_binds_last_row`、`terminal_agent_edit_title_commits_without_effects_or_snapshot_changes`、`left_panel::tests::sidebar_title_status_follows_live_edited_page`、`state::tests::agent_title_status_resolves_the_entry_workspace` |
| P3-5 | 重写设计/任务/审查文档、更新 TUI 标准与注释 | 文档检查 |

### 3.1 验证结果

功能分支（合并前）：

```text
cargo fmt                                             通过
cargo clippy --all-targets -- -D warnings             通过
cargo test --lib                                      5523 passed, 0 failed, 6 ignored
```

合并 `dev` 后（`bc5d6f3a` 之后的 15 个提交 + 本功能 7 个提交）：

```text
cargo fmt                                             通过
cargo clippy --all-targets -- -D warnings             通过
cargo test --lib                                      5549 passed, 0 failed, 6 ignored
```

专项：

```text
cargo test --lib app::state::pane::                   13 passed
cargo test --lib app::state::tests::agent_title_status 1 passed
cargo test --lib app::update::menu::                  31 passed
cargo test --lib app::update::prompt::recent::        12 passed
cargo test --lib app::update::chat::web::             25 passed
cargo test --lib ui::left_panel::                     38 passed
cargo test --lib ui::overlay::agent_title::           10 passed
cargo test --lib ui::style::                           21 passed
cargo test --lib event::tests::agent_menu::           13 passed
cargo test --lib input::tests::all_text_overlays_route_word_ops_and_cut 1 passed
```

### 3.2 未修复项

- P3-6：标题状态每帧全量扫描绑定页正文，保留为性能债。
- P3-7：`src/app/state/lx/chat.rs` 超过 1000 行，属于既有结构债，本次不拆分。
- 真实终端人工验收仍建议在合并后的 `dev` 上执行一次 `cargo run`。

### 3.3 合并记录

- 合并 `dev` 时仅 `docs/standards/tui-design-requirements.md` 冲突：`dev` 的侧栏底栏已统一为 `[WEB]`，功能分支文档仍写 `[SET]` / `[USE]`。
- 解决方式：保留 `dev` 的 `[WEB]` 底栏描述，同时合入本功能的 Agents 两行条目、标题状态色、右键菜单与标题弹窗规则。
- `SCHEMA_VERSION` 两侧均为 29，无迁移冲突；合并后全量测试通过。

## 4. 残余风险

- 终端同一 `AgentKind` 的进程快速替换（旧 Claude 退出、新 Claude 启动且轮询未观察到 `None`）无法区分，标题可能被新进程继承；检测快照没有进程身份，当前只能覆盖“种类变化 / 退出”路径。
- 绑定页被删除后保留标题文本快照、隐藏 `Open prompt`、保留 `Unbind recent`，属有意降级。
- 标题只存内存，重启丢失，属需求约定。
- 真实终端观感（弹窗高度、Recent 选中底色、按钮间距、小屏退化）仍需用户 `cargo run` 人工验收。
