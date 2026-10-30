# lx 输入框 `@` / `/` 命令面板样式重设计与 `@` 面板双 Tab

状态：已实现（合入 `dev`）。行为以本文件为准；实现见 `src/ui/widgets/command_panel.rs`、`src/ui/main_content/lx/{mention,slash,refer}.rs`、`src/app/state/lx/mention.rs`。

## 1. 目标

参考 codex 的 `@` / `/` 命令面板外观，统一 lx 输入框上方面板的视觉与状态表达：

- 移除面板背景色与四周边框，面板顶部只保留一条蓝色 `─` 横线；
- `@` 面板新增顶部 `Files` / `refDir` 双 Tab，支持键盘左右键与鼠标点击切换；
- 条目左侧常驻 `❯` 选择列：正在选择的项用默认字体色，已生效项（`/model` 当前模型、`/resume` 当前会话）用蓝色。

作用范围仅限 **lx 输入框上方弹出的命令面板**（`@` 提及、`/` 斜杠及其 `/model`、`/resume`、`/rewind` 二级面板、快捷引用与路径粘贴面板）。Prompt 右栏中跟随光标浮动的面板保持原有圆角边框与深色底。

## 2. 已确认的产品决策

| 编号 | 决策 | 具体行为 |
| --- | --- | --- |
| D1 | 无边框样式 | `CommandPanelView::borderless = true` 时：清底后不铺 `overlay_panel` 背景，不画左右/底边框，顶部整行绘制 `style::accent()` 的 `─`。 |
| D2 | 顶栏内容 | 左侧内嵌 `tabs`（优先）或 `title`，右侧内嵌 `right_title`（当前目录名 / 撤销范围说明）。右侧仅在左侧内容之后至少留 1 列 `─` 时绘制，超宽截断、仍放不下则省略，绝不覆盖左侧标题或 Tab。 |
| D3 | 条目标记 | 全体条目共用 `❯ ` 标记列（与输入框 `> ` 前缀左缘对齐）。无边框模式不铺任何行底色：`active` 行 `❯` 用默认字体色；`current` 行主名称用蓝色，且未同时为 `active` 时 `❯` 也用蓝色。二者重合时 `❯` 为默认色、名称为蓝色。 |
| D4 | `@` 四 Tab | 顶部固定 `Files` / `refDir` / `Skills` / `MCP`；每次新触发默认 `Files`；`←`/`→` 在面板可见（含 `scanning…` / `no matches`）时循环切换且不移动输入框光标，鼠标点击 Tab 同语义。 |
| D5 | `refDir` / `Skills` / `MCP` 数据 | `refDir` 复用项目引用文件夹列表（`state.referenced.folders`）插入 `@[refer-folder](绝对路径) `；`Skills` 展示已启用 skill 插入 `@[refer-skill](绝对路径) `；`MCP` 展示已启用 MCP server 插入 `@[refer-mcp](server_name) `；非 `Files` Tab 下 `Shift+Enter` 不动作。 |
| D6 | 高度口径 | 内容行先夹到 `MAX_POPUP_ROWS`(8)，再加顶部蓝色横线 1 行与底部提示块 2 行（1 行空行间隔 + 1 行快捷键提示），再与输入框上方可用行数取小；去掉了原底边框行。 |
| D7 | 底边快捷键提示 | 内容与提示之间**空一行**，提示本身**无边框、无分割线**，从左缘起 muted 单行绘制（如 `[tabs ←→] [select ↑↓] [insert ↵] …`），超宽按列截断。没有 footer 的面板不占这 2 行。 |
| D8 | 浮层拦截 | 面板打开期间，落在面板矩形内（顶栏横线、状态行、快捷键提示行、滚动条列）但未命中可点条目的点击一律由浮层吞掉，不穿透到底层欢迎页入口、历史卡片或文本选区。 |
| D9 | 鼠标路由 | `lx` 路径粘贴面板补齐悬停与点击路由（此前只有渲染）。 |

## 3. 实现要点

- `src/ui/widgets/command_panel.rs`：新增 `borderless` / `hide_active_marker` 字段与 `borderless_inner_rect`、`render_top_rule`、`render_borderless_footer`；`layout` 按 `borderless` 使用 `MIN_BORDERLESS_HEIGHT`、1 行顶线与可选 1 行 footer 作为框体预算；`panel_width` 在 `borderless` 下不再计入 4 列边框开销，但计入 footer 文案宽度。
- `src/app/state/lx/mention.rs`：`LxMention` 增加 `tab`、`referenced_folders`、`ref_active`、`ref_anchor`、`ref_viewport`、`trigger`；`view` / `is_open` / `confirm` / `enter_folder` / `move_active` / `set_active` / `scroll_viewport` / `escape` 全部按 Tab 分支。
- `src/app/state/lx/panels.rs` / `src/app/update/pane.rs`：`mention_tab`、`mention_switch_tab`、`mention_set_tab`、`sync_referenced_folders`（返回变更标记）、`hover_lx_mention`、`hover_lx_paste`、`select_lx_paste`、`switch_lx_mention_tab`。
- `src/ui/main_content/lx.rs`：新增 `paste_layout` / `paste_item_at` / `command_panel_contains`，`render_paste_panel` 复用同一份几何。
- `src/event/mouse/{press,hit,hover}.rs`：接入 Tab 命中、粘贴面板命中与浮层边界拦截。
- 文案统一为英文单宽；`MENTION_REF_DIR_EMPTY` 改为 `no matching folders`。

## 4. 验证

- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
- 新增测试覆盖：无边框顶部蓝线与无背景、`active`/`current` 的 `❯` 颜色与重合规则、窄宽度下 Tab 不被右侧状态挤丢、内容与提示之间空一行且提示行无边框、`refDir` 切换与插入、查询词变化重置滚轮视口、面板矩形内点击不穿透底层。
- 实机外观与手感由用户运行 `cargo run` 确认。
