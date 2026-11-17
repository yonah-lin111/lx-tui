# `lx` 底部状态/提示行与输入框灰线调整 — 代码审查（Review）

> 状态：**P0–P2 全部审查发现已修复并完成分支内验证，待合入 `dev` 后补充合入记录**
> 审查范围：`feature/lx-status-above-border`（commit `3e5714cb` 与后续工作区改动）
> 依据规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`

---

## 审查发现汇总（按 P0 → P2 排序）

| 级别 | 编号 | 核心问题 | 关键文件 | 状态 |
|---|---|---|---|---|
| P0 | P0-1 | `input::draw` 缺少最小矩形与 `visible` 上限守卫，`rect.bottom() - 1` 存在整数下溢与越界写风险 | `src/ui/main_content/lx/input.rs` | 已修复 |
| P1 | P1-1 | `build` 绕过 `lay.fallback`、`ask && task` 高度阈值不一致且 `history_rect`/`history_wheel_region` 未拦截退化态 | `src/ui/main_content/lx.rs` | 已修复 |
| P1 | P1-2 | `task`/`ask` 激活时在 `bottom_bar` 或边缘行滚轮穿透滚动隐藏的主输入框 | `src/event/mouse/wheel.rs` | 已修复 |
| P1 | P1-3 | `status::render` 在 16–22 列窄宽携带 `omitted_turns` 时整行空白或覆盖模型名 | `src/ui/main_content/lx/status.rs` | 已修复 |
| P1 | P1-4 | `lx_input_rule()` 跨色板复用 `github_dark::FG_MUTED` 且 `style.rs` 存在过时注释 | `src/ui/style.rs`、`src/ui/style/lx.rs` | 已修复 |
| P1 | P1-5 | `btw`/`agents` 编辑框右侧胶囊未先净化且 `UnicodeWidthStr` 与 `Grid::put` 口径不一致 | `src/ui/main_content/lx/task/{btw,agents}.rs` | 已修复 |
| P1 | P1-6 | `docs/standards/tui-design-requirements.md` 残留“底边框运行指标”、`BlockPanel` 圆角及色板例外交叉引用错位 | `docs/standards/tui-design-requirements.md` | 已修复 |
| P2 | P2-1 | `Shell mode · N queued` 窄宽整段消失、宽字符占位少 `CellDiffOption::Skip`、极矮编辑态跳过 `resize`、`lx/tests.rs`（2411 行）超限及跨模块“底边框”过期注释 | `input.rs`、`status.rs`、`task.rs`、`viewport.rs`、`lx/tests.rs` 等 | 已修复 |

---

## 详细审查点与修复根因

### P0-1：`input::draw` 缺少 `rect.height < 3`、`rect.width == 0` 与 `visible` 上限守卫
- **错误原因**：直接计算 `rect.bottom() - 1`、`rect.y + 1`，循环 `0..visible` 未按 `rect.height.saturating_sub(2)` 钳制，且前缀使用不裁剪的 `grid.put`。
- **修复**：入口校验 `rect.width == 0 || rect.height < 3` 早退；循环上界取 `visible.min(rect.height.saturating_sub(2))`；前缀与占位符全部改走 `grid.put_clipped` 并守卫 `content_w > 0`。

### P1-1：聊天模式下 `build` 绕过 `lay.fallback` 且 `ask && task` 高度阈值不一致
- **错误原因**：`build` 先判断 `has_history || loading_session` 再判 `lay.fallback`；`layout` 的 `min_height` 只看了 `ask_active` 未叠加 `&& lx.task.is_none()`；`history_rect`/`history_wheel_region` 在退化态未返回 `None`。
- **修复**：统一抽取 `hides_bottom_bar = ask_active && lx.task.is_none()`；在 `build` 最前及 `history_rect`/`history_wheel_region` 中优先拦截 `lay.fallback`；同步调整 `app/update/chat/tests/history.rs` 探测循环跳过退化高度。

### P1-2：`task`/`ask` 激活时 `bottom_bar` 与边缘行滚轮穿透隐藏的主输入框
- **错误原因**：`task::draw_footer` 移入 `bottom_bar` 后不在 `task_output..task_list` 命中区间内，高度极矮无 `task_output` 时 `chat_bar` 同样掉空，导致滚轮直达 `scroll_lx_input`。
- **修复**：在 `wheel.rs` 消费 `target.lx.task.is_some() || target.lx.ask.is_some()` 的其余滚轮事件。

### P1-3：`status::render` 在窄宽携带 `omitted_turns` 时整行空白或覆盖模型名
- **错误原因**：`notice_reserve` 未保证扣减后仍余最小状态宽度 6 列，17–22 列会触发 `layout` 退化；`notice_reserve == 0` 且无 `activity` 时 `notice_left` 误取 `lay.label.x`，<= 16 列时直接从行首覆盖模型名。
- **修复**：仅当 `usable.saturating_sub(notice_need) >= 6` 时预留 `notice_reserve`，且仅在 `notice_reserve > 0` 并校验真实左段尾列 `x + GAP` 之后才绘制。

### P1-4：`lx_input_rule()` 跨色板耦合 `github_dark::FG_MUTED`
- **错误原因**：UI 控件分割线直接依赖阅读侧 Markdown 色板模块，违背色板独立约定。
- **修复**：将 `lx_input_rule()` 迁入 `src/ui/style/lx.rs` 并定义独立常量 `LX_INPUT_RULE_GRAY`（`#7d8590`），解除与 `github_dark` 的引用与测试绑定，并清理 `style.rs` 过期注释。

### P1-5：`btw` / `agents` 徽章宽度与控制字符净化脱节
- **错误原因**：两处右侧胶囊按 `UnicodeWidthStr::width` 测宽而 `Grid::put` 按 `char.width()` 写格，且 `btw` 缺失 `single_line_label` 净化，遇到 ZWJ emoji 或控制字符会覆盖顶边最右 `─`。
- **修复**：先过 `single_line_label`，再按逐字符显示宽度测宽并用 `grid.put_clipped` 绘制。

### P1-6 & P2-1：规范文档一致性、宽字符 diff、窄宽降级与测试文件拆分
- 补齐 `docs/standards/tui-design-requirements.md`（状态行、`BlockPanel`、Markdown 色板与 UI Chrome RGB 例外条款）及 `docs/standards/project-directory-structure.md`。
- `(shell == true, queued > 0)` 窄宽先保留 `Shell mode`；`status.rs` / `task.rs` 宽字符第 2 列设置 `CellDiffOption::Skip`；`viewport.rs` 将 `btw`/`agents` 编辑框 `resize` 提前到 `task_output_inner?` 之前；将 `lx/tests.rs` 拆分为 `selection_copy.rs`（299 行）、`mention_and_slash.rs`（473 行）、`chat_bar_and_history.rs`（781 行），`lx/tests.rs` 降至 868 行。
