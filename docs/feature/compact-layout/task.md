# 窄终端紧凑布局（Compact Layout）— 执行清单（v2）

- **状态**：已完成实现、Review（P0–P3）修复、`dev@d5bf1251` 合入冲突与 `P0-2` 对齐，并通过全量自动化测试验证；按用户授权合入 `dev` 并清理 worktree。
- **工程约束**：遵守 `AGENTS.md` 与 `docs/standards/*`；保持 `app/` 纯状态/更新、`ui/` 纯渲染；生产代码禁止 `unwrap()`/`expect()`；用户可见文案统一收敛到 `src/ui/text`。

## 阶段 1：阈值与布局模型

- [x] `Config.narrow_width` 固定默认 160，去掉 `settings.json` / Web UI / `settings` CLI 持久化配置（忽略旧配置中的 `narrow_width`）
- [x] `resolved_sidebar_width` 宽屏判断改为 `width > narrow_width`
- [x] `compute_with_panel` 参数改为 `Option<CompactPanel>`
- [x] `ViewLayout` 新增 `compact_left_button` / `compact_right_button`
- [x] 紧凑模式 `view.tab_bar` 指向顶部栏中间 strip，复用原 tab 栏
- [x] 顶部栏固定单行高度规则
- [x] 左栏全屏 body / 右栏全屏 body / Chat body 三种几何
- [x] `src/ui/layout/tests.rs`：160/161 阈值、按钮位置、单行顶部栏、极窄防重叠、左右面板

## 阶段 2：状态机

- [x] `CompactRestore` 增加 `sidebar_collapsed`
- [x] `CompactPanel::{Left, Right}` 由焦点推导
- [x] `sync_compact_layout(state, width, config)`：进入快照三项并展开，退出恢复
- [x] 退出时焦点在 Prompt/Todo 保留右栏展开；`prompt_left && panes_focused()` 保留主区展开
- [x] 退出时侧栏恢复折叠且焦点在侧栏 → 若右列主区也折叠则转交右栏，否则交回主区
- [x] `toggle_compact_left` / `toggle_compact_right` / `back_to_compact_chat`
- [x] 紧凑期间禁止 `ToggleSidebar` / `TogglePrompt` / `TogglePanes`
- [x] 删除旧 switcher 状态、行模型、导航与全局 Agent 行
- [x] `src/app/state/compact/tests.rs` / `src/app/update/compact_layout/tests.rs`

## 阶段 3：顶部栏

- [x] 单行 `[ws] + tab 条 + [panel]`（无第二行工作区/Agent 状态摘要）
- [x] `[ws]`/`[panel]` 打开对应全屏栏时实心强调色
- [x] 中间 tab 条复用原渲染；紧凑模式不显示 `[EXIT]`
- [x] `src/ui/widgets/compact_header/tests.rs`

## 阶段 4：事件与交互

- [x] 顶部栏左/右按钮点击 toggle
- [x] 中间 tab 条点击切换、`[+]`、滚动按钮、右键菜单、悬停
- [x] 点击 tab 从全屏栏返回 Chat
- [x] 左栏裸 `Esc` 返回 Chat；右栏 Esc 保留原语义
- [x] 顶部栏间隙/空白点击吞掉
- [x] 紧凑模式禁用外框折叠/钉住/交换/退出/分割线拖拽与 hover/tooltip（保留左栏内部分区 `Agents` 折叠按钮）
- [x] 全屏栏下窗格不进入几何，PTY 不 resize
- [x] `Resize` 事件置 dirty
- [x] `src/event/tests/compact.rs` 端到端覆盖

## 阶段 5：Toast、设置移除与规范同步

- [x] 紧凑全局 Toast 取消底部横幅，复用桌面带边框浮框与顶部位置
- [x] 紧凑 Toast 点击关闭
- [x] `pane.lx.toast` 保持在 lx 窗格内
- [x] 从 `settings.json` / `assets/web.html` / `assets/settings.js` / `/api/config` / `settings` CLI 移除 `narrow_width` 持久化与配置项
- [x] `docs/standards/tui-design-requirements.md` 全文对齐 v2（含第 103 行降级规则）
- [x] `docs/feature/compact-layout/design.md` 重写并校准为 v2

## 阶段 6：Review 修复（P0–P3）

- [x] `P0-1`：`session::current_geometry` 恢复 `(!view.compact && state.prompt_left && state.panes_collapsed)`，修复桌面折叠主区 PTY 被缩成 2 列
- [x] `P1-1`：`sync_compact_layout` 退出恢复 `panes_collapsed` / 折叠侧栏交焦点保护，以及 `ui/mod.rs`、`right_panel/mod.rs`、`footer.rs` 补齐 `!view.compact`
- [x] `P1-2`：`has_visible_selection` 按紧凑唯一可见容器隔离；`open_prompt_for_pane` / `open_prompt_page` / `back_to_compact_chat` / `toggle_compact_right` 补清选区
- [x] `P1-3`：`controls::collapse_buttons` 与 `ui/mod.rs` 在全屏左栏保留 `Agents` 分区折叠按钮
- [x] `P1-4`：`ui::tooltip::target_at` / `targets` 与 `hover.rs` 完整接入 `!view.compact` 门控
- [x] `P1-5`：`docs/standards/tui-design-requirements.md:103` 移除旧 80 列冲突表述
- [x] `P2-1`：`session::sync_layout_geometry` 按 `state.all_pane_ids()` 保留隐藏 `lx` 缓存，并在屏幕/模式变化时钳制工作区滚动、reveal 激活标签及复位指针形状
- [x] `P2-2`：跨 160/161 阈值清理在途拖拽/悬停与 `drag.rs` `!view.compact` 闸门；`commit_worktree_open` 打开已有 worktree 回 Chat；`focus_prompt` 提交 Todo 编辑
- [x] `P2-3`：`event/mod.rs` 仅在右栏可见且当前页为 `RightTab::Todo` 时调用 `resolve_todo_viewport`
- [x] `P2-4`：`right_panel::render_bottom` 紧凑模式下不预留隐藏按钮空白
- [x] `P2-5`：`compact_layout` / `compact_header` / `tab_bar` 极窄宽度防重叠保护
- [x] `P2-6`：校准 `design.md`、`task.md`、过期注释与 CLI/Settings/Compact 回归测试
- [x] `P3-1`：`compact_header` 改用 `text::TAB_SEPARATOR`

## 合并前必做清单（`dev@d5bf1251` — `P0-2`）

- [x] 合并 `dev@d5bf1251`，解决 `src/app/state.rs`、`src/app/update/selection/{mod.rs,tests.rs}` 与 `src/event/tests/overlay.rs` 冲突
- [x] 将 `dev` 新增的 `80~120` 列桌面测试（`src/event/tests/{agent_menu,mouse_drag,overlay,workspace_history}.rs`）切换为 `desktop_config()`（`enter_in_new_agent_reveals_newly_created_tab_when_tab_bar_overflows` 固定 `90x24` 桌面并额外补齐紧凑全屏左栏 `New agent` 回归测试），并为 `autoscroll`/`wheel`/`drag`/`workspace_list_rect` 补齐面板宽度守卫
- [x] 合并后全量执行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`

## 验证

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test` 全量通过（合并 `dev@d5bf1251` 后：lib `5901 passed / 6 ignored` + 集成测试 `64 passed / 2 ignored`，含 PTY 真实交互与 Web 资源契约测试）
- [ ] 用户 `cargo run` 手动验收（用户已授权在全量自动化校验通过后直接合入 `dev` 并清理 worktree；如需复核外观交互可在 `dev` 运行 `cargo run`）

## 手动验收清单

- [ ] 160x30：紧凑单列 + 单行顶部栏
- [ ] 161x30：桌面三栏
- [ ] 40x12：1 行顶部栏，`[ws]`/`[panel]`/tab 均可点
- [ ] `[ws]` 打开全屏左栏，工作区/Agents（含 `[▼]`/`[▲]`）/历史列表与 `[WEB]` 可点，Esc 返回（历史模式双 Esc）
- [ ] `[panel]` 打开全屏右栏，Prompt/Todo 边框 tab 可切、底边框完整，再点按钮返回
- [ ] 中间 tab 点击切换、`[+]` 新建、多 tab 溢出滚动、右键菜单
- [ ] 左栏/右栏打开时拉宽缩窄，焦点与面板保持、激活标签可见
- [ ] 复制终端内容，Toast 复用桌面全局带边框浮框（顶部），不在底部
- [ ] 退出紧凑后侧栏/右栏折叠状态恢复

## 风险提醒

- `P0-2`：合入 `dev@d5bf1251` 时如不把新增桌面事件测试切至 `desktop_config()`，测试将因默认 160 列触发紧凑而大量失败。
- 160 列以下大量桌面测试已切到显式 `desktop_config()`（`narrow_width = 80`）；新增桌面测试不得再依赖默认配置在 160 列及以下保持三栏。
- 紧凑模式复用真实侧栏/右栏，其内部控件仍按桌面规则渲染；新增侧栏/右栏控件时必须同时检查紧凑全屏宽度下的命中与截断。
- 全屏右栏的 Esc 不关闭整栏；不要把“返回 Chat”误加到 Prompt/Todo 的 Esc 分支。
