# `lx` 底部状态/提示行与输入框灰线调整 — 执行任务

> 状态：**已完成开发与 Review 修复，待合入 `dev` 并清理 worktree**
> 设计依据：[design.md](design.md) · 审查记录：[review.md](review.md)

## 阶段 1：`bottom_bar` 布局重构（`3e5714cb`）

- [x] `src/ui/main_content/lx.rs`：新增 `BOTTOM_BAR_HEIGHT = 1` 与 `Layout::bottom_bar`，输入框与任务列表上移 1 行，`render` 统一绘制状态行或任务面板底部提示行；最小退化高度提升至 `5`（`ask` 激活无面板时保持 `4`）。
- [x] `src/ui/main_content/lx/status.rs`：`render` 改为接收 `bar: Rect` 内容行而非 `inner.bottom()` 窗格底边框，删除与 `─` 边框交错绘制假设。
- [x] `src/ui/main_content/lx/task.rs`：`draw_footer` 改绘于 `bar: Rect`，`task_panel_height` 复用底部留位，窗格底边框保持纯净。
- [x] 同步既有测试与文档：`status/tests.rs`、`task/tests{,/detail_slash,/history_takeover}.rs`、`lx/tests/bottom_bar.rs`、`viewport_anchor.rs`、`project-directory-structure.md`、`tui-design-requirements.md`。

## 阶段 2：输入框显式灰色分割线

- [x] `src/ui/style/lx.rs`：定义独立常量 `LX_INPUT_RULE_GRAY`（`#7d8590`）与 `style::lx_input_rule()`（无 `DIM`）。
- [x] `src/ui/main_content/lx/input.rs`：主输入框及复用 `input::draw` 的 BTW / Agents 编辑框上下 `─` 分割线切换为 `style::lx_input_rule()`，保留胶囊、`Shell mode`、`N queued` 原有样式。
- [x] `src/ui/style/tests.rs` & `src/ui/main_content/lx/input/tests.rs`：补充颜色/无 `DIM`/透明底与胶囊共存断言。

## 阶段 3：P0–P2 Review 修复与测试拆分（依据 [review.md](review.md)）

- [x] **P0-1**：`input::draw` 增加 `rect.width == 0 || rect.height < 3` 守卫、`visible` 钳制与 `put_clipped` 边界保护，并补充退化/越界回归测试。
- [x] **P1-1**：`lx.rs` 统一 `hides_bottom_bar`，提前 `build` 的 `lay.fallback` 拦截并在 `history_rect`/`history_wheel_region` 返回 `None`；补充有历史+`task` / `ask+task` 高 4 行退化回归测试。
- [x] **P1-2**：`wheel.rs` 拦截 `task.is_some() || ask.is_some()` 未命中滚轮，防止 `bottom_bar` 穿透滚动隐藏主输入框，并在 `lx_ask_task.rs` 补充测试。
- [x] **P1-3**：`status.rs` 修正窄宽携带 `omitted_turns` 时的 `>= 6` 预留校验与 `notice_reserve > 0` 绘制守卫，并在 `status/tests.rs` 补充 16–25 列回归测试。
- [x] **P1-4**：解除 `lx_input_rule()` 与 `github_dark` 耦合，清理 `style.rs` 与 `style/tests.rs` 跨色板依赖。
- [x] **P1-5**：`btw.rs` / `agents.rs` 对右侧胶囊启用 `single_line_label` 与逐字符显示宽度裁剪，在 `detail_slash.rs` 补充 ZWJ emoji + 控制字符测试。
- [x] **P1-6 & P2-1**：补齐 `tui-design-requirements.md` 与 `project-directory-structure.md`；增加 `Shell mode` 窄宽降级、宽字符 `CellDiffOption::Skip`、极矮编辑态 `resize` 前置及回归测试；将 `lx/tests.rs` 拆分为 `selection_copy.rs`、`mention_and_slash.rs`、`chat_bar_and_history.rs` 并清理跨模块“底边框”过期注释。

## 阶段 4：合入 `dev` 与 worktree 清理

- [ ] 在分支内提交全部未提交修复与文档，合并到 `dev` 并解决与 `compact-layout` 的合并冲突。
- [ ] 在 `dev` 运行全量 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与 `cargo test`。
- [ ] 更新本目录文档记录合入 commit 与全量验证结果，移除 `.worktrees/lx-status-above-border`、`feature/lx-status-above-border` 及对应隔离库 `~/.lx-t-dev/db/worktrees/lx-status-above-border-b03d203f/`。
