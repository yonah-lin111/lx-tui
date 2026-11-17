# 窄终端紧凑布局（Compact Layout）— 执行清单（v2）

- **状态**：已实现，待用户 `cargo run` 手动验收。
- **工程约束**：遵守 `AGENTS.md` 与 `docs/standards/*`；保持 `app/` 纯状态/更新、`ui/` 纯渲染；生产代码禁止 `unwrap()`/`expect()`；用户可见文案统一收敛到 `src/ui/text`。

## 阶段 1：阈值与布局模型

- [x] `Config.narrow_width` 默认值 80 → 120（对齐 opencode-dev）
- [x] `resolved_sidebar_width` 宽屏判断改为 `width > narrow_width`
- [x] `compute_with_panel` 参数改为 `Option<CompactPanel>`
- [x] `ViewLayout` 新增 `compact_left_button` / `compact_right_button` / `compact_status`
- [x] 紧凑模式 `view.tab_bar` 指向顶部栏中间 strip，复用原 tab 栏
- [x] 顶部栏 2 行/1 行高度规则
- [x] 左栏全屏 body / 右栏全屏 body / Chat body 三种几何
- [x] `src/ui/layout/tests.rs`：120/121 阈值、按钮位置、两行/一行、左右面板

## 阶段 2：状态机

- [x] `CompactRestore` 增加 `sidebar_collapsed`
- [x] `CompactPanel::{Left, Right}` 由焦点推导
- [x] `sync_compact_layout(state, width, config)`：进入快照三项并展开，退出恢复
- [x] 退出时焦点在 Prompt/Todo 保留右栏展开
- [x] 退出时侧栏恢复折叠且焦点在侧栏 → 交回主区
- [x] `toggle_compact_left` / `toggle_compact_right`
- [x] `back_to_compact_chat`
- [x] 紧凑期间禁止 `ToggleSidebar` / `TogglePrompt` / `TogglePanes`
- [x] 删除旧 switcher 状态、行模型、导航与全局 Agent 行
- [x] `src/app/state/compact/tests.rs` / `src/app/update/compact_layout/tests.rs`

## 阶段 3：顶部栏

- [x] 第一行 `[ws] + tab 条 + [panel]`
- [x] `[ws]`/`[panel]` 打开对应全屏栏时实心强调色
- [x] 第二行工作区状态点/名称 + 全局 Agent 摘要
- [x] 长工作区名截断，不挤掉左右按钮
- [x] 中间 tab 条复用原渲染；紧凑模式不显示 `[EXIT]`
- [x] `src/ui/widgets/compact_header/tests.rs`

## 阶段 4：事件与交互

- [x] 顶部栏左/右按钮点击 toggle
- [x] 中间 tab 条点击切换、`[+]`、滚动按钮、右键菜单、悬停
- [x] 点击 tab 从全屏栏返回 Chat
- [x] 左栏裸 `Esc` 返回 Chat；右栏 Esc 保留原语义
- [x] 状态行/顶部栏空白点击吞掉
- [x] 紧凑模式禁用折叠/钉住/交换/退出/分割线拖拽与 hover/tooltip
- [x] 全屏栏下窗格不进入几何，PTY 不 resize
- [x] `Resize` 事件置 dirty
- [x] `src/event/tests/compact.rs` 端到端覆盖

## 阶段 5：Toast 与文档

- [x] 紧凑全局 Toast 改底部单行横幅
- [x] 紧凑 Toast 点击关闭
- [x] `pane.lx.toast` 保持在 lx 窗格内
- [x] `assets/web.html` 文案 `Narrow layout width`
- [x] `docs/standards/tui-design-requirements.md` 紧凑布局章节更新为 v2
- [x] `docs/feature/compact-layout/design.md` 重写为 v2

## 验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --lib`（5814 通过，6 ignored；去掉旧 switcher 测试后数量减少）
- [x] 集成测试（`prompt_editor` / `prompt_short_ref` / `terminal_cursor` 已显式使用桌面阈值）
- [ ] 用户 `cargo run` 手动验收

## 手动验收清单

- [ ] 120x30：紧凑单列 + 2 行顶部栏
- [ ] 121x30：桌面三栏
- [ ] 40x12：1 行顶部栏，`[ws]`/`[panel]`/tab 均可点
- [ ] `[ws]` 打开全屏左栏，工作区/Agents/历史列表可点，Esc 返回
- [ ] `[panel]` 打开全屏右栏，Prompt/Todo 边框 tab 可切，再点按钮返回
- [ ] 中间 tab 点击切换、`[+]` 新建、多 tab 溢出滚动、右键菜单
- [ ] 左栏/右栏打开时拉宽缩窄，焦点与面板保持
- [ ] 复制终端内容，Toast 在底部单行显示
- [ ] 退出紧凑后侧栏/右栏折叠状态恢复

## 风险提醒

- 120 列以下大量桌面测试已切到显式 `desktop_config()`（`narrow_width = 80`）；新增桌面测试不得再依赖默认配置在 120 列下保持三栏。
- 紧凑模式复用真实侧栏/右栏，其内部控件仍按桌面规则渲染；新增侧栏/右栏控件时必须同时检查紧凑全屏宽度下的命中与截断。
- 全屏右栏的 Esc 不关闭整栏；不要把“返回 Chat”误加到 Prompt/Todo 的 Esc 分支。
