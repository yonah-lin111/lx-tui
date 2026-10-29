# lx 命令面板样式重设计 — 任务清单

状态：已实现（合入 `dev`）。

## 实现项

- [x] `CommandPanelView` 新增 `borderless` / `hide_active_marker`，面板支持无背景、无四周边框、顶部单条蓝色横线。
- [x] 无边框面板内容区仅扣除顶部分割线 1 行；`panel_width` 不再计入边框开销。
- [x] 条目常驻 `❯ ` 标记列：`active` 用默认字体色，`current` 主名称蓝色、非 `active` 时 `❯` 蓝色，重合时 `❯` 默认色 + 名称蓝色。
- [x] 顶栏左侧 `tabs`/`title` 优先排布，右侧 `right_title` 保留 1 列间隔、超宽截断或省略。
- [x] 底部独立一行显示快捷键提示（无边框、无分割线，muted 单行、超宽截断），`@`、`/`、`/model`、`/resume`、`/rewind`、快捷引用与路径粘贴面板均已接入。
- [x] `LxMention` 增加 `Files` / `refDir` Tab 状态、引用文件夹快照、`ref_active` / `ref_anchor` / `ref_viewport` 与触发区间。
- [x] `@` 面板 `←`/`→` 方向键循环切换 Tab（面板可见时），鼠标点击 Tab 同语义。
- [x] `refDir` 过滤、导航、滚轮视口与 `@[refer-folder](路径) ` 插入。
- [x] 引用文件夹快照变化（`sync_referenced_folders` 返回 `bool`）触发重绘。
- [x] `lx` 路径粘贴面板补齐 `paste_layout` / `paste_item_at` 与悬停/点击路由。
- [x] `command_panel_contains` 拦截面板矩形内的非条目点击，禁止穿透底层。
- [x] `MENTION_REF_DIR_EMPTY` 统一为英文 `no matching folders`；移除废弃的底边页脚/`@Command` 标题常量。
- [x] 同步 `docs/standards/tui-design-requirements.md` 与 `docs/feature/lx-at-mention/design.md`。

## 评审与修复

- [x] `docs/feature/lx-command-panel-style/review.md`：P0-1、P0-2、P1-1、P1-2、P1-3、P2 全部修复并补回归测试。
