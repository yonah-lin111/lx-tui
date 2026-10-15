# UI 组件查看弹窗（`[UI]`）：执行任务

- 分支：`ui-gallery`（工作区 `.worktrees/ui-gallery`）
- 状态：**已实现并自测通过；等待用户审核与合并决定**

## 0. 开发门禁与现状核查

- [x] 用户审核 `design.md` 边界（Grill Me 11 点全部确认）。
- [x] 用 codegraph 与 codebase-memory 复核将编辑的符号与调用链；确认各文件行数。
- [x] 读取项目四份规范；只做只读 Git 查询。
- [x] 确认依赖方向：`app` 纯状态、`ui` 纯渲染、`input` 按键映射、`event` 事件分派。

## 1. 纯数据模型（`src/app/ui_gallery.rs`）

- [x] 定义 `UiGalleryNode` / `UiGalleryComponent` / `UiGalleryFolder` 与静态常量 `GALLERY_FOLDERS`。
- [x] `global`：`toast` / `modal` / `tooltip` / `button` / `menu` / `fullscreen`；`prompt`：`editor` / `list` / `toolbar`。
- [x] `UiGallery` 状态：`collapsed` 集合与 `selected` 可见行；默认全部展开。
- [x] `rows()` 生成文件夹行 + 展开的组件行；`selected_folder/component`、`move_selection`、`toggle_folder`、`activate`、`left/right`。
- [x] 精确测试：行序列、折叠隐藏后代、选择钳制、左右移动、回车切换折叠。

## 2. 浮层接线

- [x] `OverlayKind::UiGallery` + `Overlay::UiGallery(UiGallery)` 与 `kind()` 映射。
- [x] `ui/overlay.rs` 渲染分支改为调 `fullscreen::render`（不再用 `widgets::modal::render`）。
- [x] `update::open_ui_gallery`、`set_ui_gallery_selection`、`activate_ui_gallery_row` 与关闭复用 `close_overlay`。
- [x] `input::overlay_key` 新增 `UiGallery`：`Esc/Up/Down/Left/Right/Enter`。
- [x] `app::update::dialog::apply_overlay_key` 分派到 `apply_ui_gallery_key`。
- [x] 精确测试：`kind()`、打开/关闭、按键到状态迁移、鼠标选中/折叠。

## 3. `[UI]` 按钮（`src/ui/controls.rs`）

- [x] 新增 `ui_gallery_button(view, state)`：`[SET]` 右侧隔 1 列，宽度 `[UI]` 文案长度。
- [x] `ui_gallery_button_at` 命中；宽度不足时 `[UI]` 隐藏、`[SET]` 保留；不依赖设置服务。
- [x] `render_ui_gallery_button` 逐格 reset + 强调色绘制（同 `[SET]`）。
- [x] `ui/mod.rs` 渲染序列加入；`event/mouse/press.rs` 命中分支打开浮层。
- [x] `TooltipTarget::SidebarUiGallery` + `targets()` 注册 + `tooltip_hint` 文案。
- [x] 精确测试：相邻关系、宽度降级、命中、渲染文案、服务无关性、折叠隐藏。

## 4. 全屏覆盖与渲染（`src/ui/widgets/fullscreen.rs` + `src/ui/overlay/ui_gallery.rs`）

- [x] 新增公共全屏覆盖组件（最外层蓝色圆角边框、无底色；标题/提示内嵌边框线）：`fullscreen::layout/render/close_button_rect/close_button_at`。
- [x] `ui_gallery_shell(screen)`：整屏覆盖；左侧 24 列、右侧占剩余、中间竖直分隔线。
- [x] 左区行渲染：文件夹 `▾/▸`、子项 `•`、缩进 2 列、高亮 `❯`。
- [x] 右区按组件 id 分派 `appearance_size` + `render_appearance`：8 个组件各自的**真实外观**，**水平居中**于右区 + 左对齐说明（去 Preview 标题、无统一包裹框）。
- [x] 点击命中：`ui_gallery_row_at` 映射到可见行并切换选中/折叠；`[✕]` 或 `Esc` 关闭。
- [x] 精确测试：全屏几何、清底无背景、行命中、预览分派不 panic、窄屏降级、渲染文本。

## 5. 收尾验证与交付

- [x] 用户可见文案集中在 `ui/text.rs`（组件展示名与描述字符串作为 `UiGallery` 数据保留在 `ui_gallery.rs`）。
- [x] 无生产 `unwrap/expect`、无 UI IO。
- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（1358 lib + 集成测试全绿）。
- [ ] 汇总实现与测试结果；TUI 由用户 `cargo run` 验收。
- [ ] 待用户确认后合并 `ui-gallery` 到 `dev`。

## 验收记录模板

| 项目 | 结果/证据 |
| --- | --- |
| 文档审核 | 已确认（Grill Me 11 点） |
| `[UI]` 按钮几何与降级 | 已实现 + 单元测试 |
| 全屏覆盖公共组件（蓝色外框） | 已实现 + 单元测试 |
| 右侧真实外观预览（水平居中） | 已实现 + 单元测试 |
| 左侧树与折叠 | 已实现 + 单元测试 |
| 右侧静态预览 | 已实现 + 单元测试 |
| 键鼠交互与关闭 | 已实现 + 单元测试 |
| 精确测试 | 38 项 ui_gallery 相关用例通过 |
| `cargo fmt` / Clippy / `cargo test` | 通过（1358 lib tests） |
| TUI 人工验收 | 待用户运行 |

