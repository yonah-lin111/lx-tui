# `lx` 底部状态/提示行上移至窗格底边框上方与输入框灰线调整 — 设计

> 状态：**已完成实现与 P0–P2 Review 修复，待合入 `dev` 后收尾记录**
> 关联文档：[task.md](task.md) · [review.md](review.md)
> 分支：`feature/lx-status-above-border`（基于 `dev@d5bf1251`）

## 1. 背景与目标

此前 `lx` 页将聊天状态（模型名 · 上下文占用 · 运行指标）与后台任务面板提示行（`task::draw_footer`）直接覆写到窗格底边框（`╰──╯`）上，导致：

1. 窗格底边框被状态文本或按键提示打断，难以与上下窗格边框保持一致的干净轮廓；
2. 输入框底部分割线 `─` 使用终端 `Modifier::DIM`（`style::muted()`），在部分终端主题下密集横线依然偏亮抢眼。

本次改动在现有 `app/` 纯状态、`ui/` 纯渲染、`event/` 分派边界内完成两项收口：

1. **独立 `bottom_bar` 行**：将 `status::render` 与 `task::draw_footer` 从窗格底边框移入内容区最末 1 行（`area.bottom() - 1`），紧贴输入框/任务列表底分割线下方、与输入框左右同宽齐平；窗格底边框 `╰─…─╯` 在任何状态下保持纯净。
2. **显式灰色输入框分割线**：在 `ui/style/lx.rs` 新增独立 UI token `style::lx_input_rule()`（`#7d8590`，不叠加 `DIM`、不复用 `github_dark` 阅读色板），统一作用于主输入框及复用 `input::draw` 的 BTW / Agents 编辑框上下 `─` 分割线，同时保留 `chat_provider()`、`Shell mode`、`N queued`、`session_title_badge` 与 `btw_qa_badge` 原有样式。

## 2. 布局与渲染设计

### 2.1 `lx::layout` 与 `bottom_bar` 几何

- `BOTTOM_BAR_HEIGHT = 1`；`hides_bottom_bar = ask_active && lx.task.is_none()`。
- 非 `hides_bottom_bar` 时：
  - `min_height = INPUT_MIN_BOX + BOTTOM_BAR_HEIGHT + 1`（5 行），`< 5` 行或宽度不足时统一置 `lay.fallback = true`。
  - `bottom_bar` 固定落在 `Rect::new(content_x, area.bottom() - 1, content_w, 1)`，与输入框/任务列表左右对齐；`bottom_anchor = area.bottom() - 1`，输入框、`ask` 或 `task_list` 均从 `bottom_anchor` 向上排布。
- `hides_bottom_bar == true`（`ask` 激活且 `task` 未打开）时：
  - `min_height = INPUT_MIN_BOX + 1`（4 行），`bottom_bar = None`，`bottom_anchor = area.bottom()`，由 `ask` 面板自带底栏占用底行。
- `build` 优先判定 `if lay.fallback`（先于 `history::has_history(lx) || lx.chat.loading_session`），确保极矮窗口（`< 5` 行）在有历史、打开 `task` 或 `ask && task` 共存时同样统一退化为居中 `lx` 字标；`history_rect` 与 `history_wheel_region` 同步在 `lay.fallback` 时返回 `None`。

### 2.2 状态行与任务提示行渲染

- `status::render(buf, bar, lx)`：
  - 改为接收内容区 `bar: Rect`（`lay.bottom_bar`），取消底边框预铺 `─` 依赖。
  - 右侧 `omitted_turns` 提示仅在扣除 `notice_w + GAP` 后仍余 `>= 6` 列时预留 `notice_reserve`，且仅当 `notice_reserve > 0` 并在真实左段末尾 `x + GAP` 之后才绘制，防止 16–22 列窄宽下整行空白或覆盖行首模型名。
  - `put_model` 与 `put` 在写宽字符第 2 列占位空格时设置 `CellDiffOption::Skip`，避免帧差 diff 擦掉宽字符右半格。
- `task::draw_footer(buf, bar, page, lx)`：
  - 同源接收 `bar: Rect`，按从右往左整项丢弃策略绘制键位提示；`put_buf` 同样对宽字符第 2 列占位设置 `CellDiffOption::Skip`。

### 2.3 输入框分割线与编辑态复用

- `style::lx_input_rule()` 定义于 `src/ui/style/lx.rs`（`LX_INPUT_RULE_GRAY = Color::Rgb(0x7d, 0x85, 0x90)`，无 `Modifier::DIM`），独立于 `github_dark` 阅读色板。
- `input::draw` 增加 `rect.width == 0 || rect.height < 3` 守卫、`visible` 按 `rect.height.saturating_sub(2)` 钳制、前缀改用 `grid.put_clipped` 及 `content_w > 0` 光标门控；底分割线右侧提示 `(shell, queued)` 在窄宽下先保留 `Shell mode`、再窄才整体隐藏。
- `task::btw::draw_qa_badge` 与 `task::agents::draw_agent_badge` 统一先走 `text::chat::single_line_label` 净化，再按逐字符显示宽度测宽并以 `grid.put_clipped` 绘制，保证与 `Grid` 写入一致，不覆盖顶边最右 `─`。

### 2.4 交互与滚轮隔离

- `src/event/mouse/wheel.rs` 在 `task` 与 `ask` 滚轮分支之后、历史与主输入框滚轮分支之前，拦截 `target.lx.task.is_some() || target.lx.ask.is_some()` 下的未命中滚轮（如 `bottom_bar` 提示行、左右 `MARGIN` 与无输出区时的间隙），防止穿透滚动被遮挡的主输入框草稿。
- `src/app/update/chat/viewport.rs` 将 `resolve_task_btw_visible` 与 `resolve_task_agents_visible` 中的 `composer.editor_mut().resize` 提前到 `task_output_inner?` 短路之前，使 5–6 行极矮窗格下编辑态输入框仍同步软换行与视口高度。

## 3. 主要修改文件清单

- 核心渲染与样式：`src/ui/main_content/lx.rs`、`src/ui/main_content/lx/input.rs`、`src/ui/main_content/lx/status.rs`、`src/ui/main_content/lx/task.rs`、`src/ui/main_content/lx/task/{btw,agents}.rs`、`src/ui/main_content/lx/{chat_bar,grid}.rs`、`src/ui/style.rs`、`src/ui/style/lx.rs`
- 状态/更新/事件/文案：`src/event/mouse/wheel.rs`、`src/app/update/chat/viewport.rs`、`src/app/state/lx/{agent,chat}.rs`、`src/app/update/{chat/subagent,pane,workspace}.rs`、`src/ui/{text,text/btw}.rs`
- 测试与拆分：`src/ui/main_content/lx/tests.rs` 及 `tests/{bottom_bar,chat_bar_and_history,mention_and_slash,selection_copy}.rs`、`src/ui/main_content/lx/input/tests.rs`、`src/ui/main_content/lx/status/tests.rs`、`src/ui/main_content/lx/task/tests/{tests,detail_slash,history_takeover}.rs`、`src/ui/style/tests.rs`、`src/event/tests/lx_ask_task.rs`、`src/app/update/chat/tests/{history,viewport_anchor}.rs`
- 规范文档：`docs/standards/tui-design-requirements.md`、`docs/standards/project-directory-structure.md`
