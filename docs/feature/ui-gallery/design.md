# UI 组件查看弹窗（`[UI]`）：设计

- 分支：`ui-gallery`（工作区 `.worktrees/ui-gallery`）
- 状态：**待用户审核；本文记录已通过 Grill Me 盘问确认的边界与实现约束**

## 1. 目标与范围

在左侧栏底边框的 `[SET]` 按钮右侧新增一个按钮 `[UI]`。点击后用**全屏覆盖**（不是模态弹窗）展示组件目录：整屏覆盖、最外层一条蓝色边框、无自绘背景色，内部划分为左右两区：

- 左侧为组件树：顶层文件夹 `global` 与 `prompt`，文件夹可折叠展开；`global` 下含 `toast`、`modal`、`tooltip`、`button`、`menu`、`fullscreen`（本全屏覆盖组件本身），`prompt` 下含 `editor`、`list`、`toolbar`。
- 右侧为选中组件的真实外观预览：直接按组件自身样式与尺寸渲染（不额外包裹样例框）并**水平居中**，其下留 1 行空白后渲染 muted 用途说明（每行同样水平居中）。

查看器只做“查看”，不驱动真实业务逻辑；关闭后不影响任何现有面板状态。

## 2. 已确认决策（Grill Me 盘问结论）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 左侧树结构 | 顶层两个文件夹。`global` 下含 `toast / modal / tooltip / button / menu / fullscreen`；`prompt` 下含 `editor / list / toolbar`。文件夹可折叠展开。 |
| 2 | `prompt` 子项归属 | 右键菜单归 `global`（`menu`）；编辑器/树列表/工具栏归 `prompt`。 |
| 3 | 展示形态（增补） | **不使用模态弹窗**：抽成无边框、无背景色的**全屏覆盖公共组件**（`ui/widgets/fullscreen.rs`），直接整屏覆盖；原模态内容迁入。 |
| 4 | 右侧预览（增补） | 按组件**真实外观**渲染，**不加统一包裹框**（原 `Preview` 标题与拉满宽度的大边框去掉）；组件自带边框时贴合真实尺寸，无边框组件（如 `button`）即纯标签。 |
| 5 | 右侧标题与说明（增补） | 去掉 `Preview` 标题；先渲染组件真实外观，其下空 1 行后渲染 muted 说明文字。 |
| 6 | 左侧切换方式 | 左侧为竖排树；鼠标点击切换，键盘 `↑/↓` 移动高亮，`→` 展开、`←` 折叠/回父级，`Esc` 关闭。 |
| 7 | 文件夹折叠 | 可折叠展开；文件夹行左侧 `▾`（展开）/`▸`（折叠）三角，回车或点击切换；默认全部展开。 |
| 8 | 分栏与左侧宽度 | 左侧固定 **24 列**，右侧占剩余宽度；整屏过窄时按比例压缩左侧（保证右侧至少 1 列）。 |
| 9 | 左右分隔视觉 | 左右之间画一条竖直边框线 `│`，样式跟随失焦态边框色。 |
| 10 | 命名与图标 | 全英文命名；文件夹行 `▾`/`▸` 三角（accent），子项行 `•` 图标，子项缩进 2 列，与 prompt list 树视图风格一致。 |
| 11 | `[UI]` 可见性 | 不依赖设置服务；侧栏展开且宽度足够时显示，侧栏折叠或过窄时隐藏。 |
| 12 | 按钮间距与降级 | `[UI]` 紧跟 `[SET]` 右侧隔 1 列；宽度不足时先隐藏 `[UI]`、保留 `[SET]`。 |
| 13 | 外框（增补） | 全屏覆盖组件最外层加一条**蓝色边框**（`style::accent()`，Cyan 圆角），仍不铺底色。 |
| 14 | 右侧对齐（增补） | 右侧预览的组件与说明文字均**水平居中**于右区（说明逐行居中，同 `text-align: center`）。 |

## 3. 已核实的实现基线

| 位置 | 现状及复用点 |
| --- | --- |
| `src/ui/controls.rs` | `settings_button` / `settings_button_at` / `render_settings_button`：侧栏底边框左端 `[SET]` 的几何、命中与逐格绘制，`[UI]` 完全对齐这一套并放在其右侧。 |
| `src/event/mouse/press.rs:122` | `[SET]` 的左键命中分支；`[UI]` 在其后追加同款分支。 |
| `src/ui/widgets/fullscreen.rs`（新增） | **全屏覆盖公共组件**：最外层蓝色圆角边框、无底色，`Clear` 清为默认底；标题内嵌顶边框左侧、提示内嵌底边框左侧（同 prompt 面板 `Prompt` / `b:` 的做法）、右上角 `[✕]` 覆盖在顶边框右端；边框内整块为内容区。几何/清底/命中与文案由调用方提供，无业务语义。 |
| `src/ui/widgets/modal.rs` | 模态容器（保留给既有浮层）：`close_button_rect` 被 toast/modal 预览复用，保证样例关闭按钮几何与真实组件一致。 |
| `src/ui/toast.rs` | 真实 toast：圆角边框、内容自适应宽度、右上角 `[✕]`；预览按其外观与尺寸重建。 |
| `src/app/overlay.rs` | `Overlay` 枚举与 `OverlayKind`；新增 `Overlay::UiGallery` 与 `OverlayKind::UiGallery`。 |
| `src/ui/overlay.rs` | 浮层渲染分派；新增 `ui_gallery` 子模块与分支。 |
| `src/event/overlay.rs` | `handle_overlay_click` 的浮层点击分派；新增 `UiGallery` 分支（点条目切换/折叠、点 `[✕]` 关闭；全屏覆盖无“浮层之外”）。 |
| `src/app/update/{dialog,ui_gallery}.rs` | `apply_overlay_key` 分派到 `apply_ui_gallery_key`。 |
| `src/input/mod.rs` | `overlay_key` 按 `OverlayKind` 过滤按键；新增 `UiGallery` 分支（`Esc/Up/Down/Left/Right/Enter`）。 |
| `src/app/prompt/tree.rs` | 文件夹/条目的“扁平存储 + 可见行 + 折叠”模型，是左侧树的直接参照（但 UI 画廊数据为静态常量，不进 DB）。 |
| `src/ui/prompt/tree.rs` | 树行渲染：折叠三角、`├─`/`└─` 连接符、`•` 图标、缩进，左侧树沿用其绘制手法。 |
| `src/app/tooltip.rs`、`src/ui/tooltip.rs`、`src/ui/text.rs` | `TooltipTarget` 枚举、`targets()` 注册与 `tooltip_hint` 映射；新增 `SidebarUiGallery` 目标与 `open ui gallery` 提示。 |

## 4. 数据模型（`src/app/ui_gallery.rs`，纯数据）

UI 画廊内容为编译期静态定义，不写数据库、不依赖工作区；唯一状态是“当前高亮行”与“文件夹折叠集合”。

```rust
/// 组件树的可见行种类。
pub enum UiGalleryNode { Folder(UiGalleryFolderId), Component(UiGalleryComponentId) }

/// 一个组件（叶子）：种类标识与右侧说明段落。
pub struct UiGalleryComponent {
    pub id: UiGalleryComponentId,
    pub description: &'static [&'static str],
}

/// 顶层文件夹：种类标识与下属组件。
pub struct UiGalleryFolder {
    pub id: UiGalleryFolderId,
    pub components: &'static [UiGalleryComponent],
}
```

- `UiGallery` 持有 `folders: &'static [UiGalleryFolder]`（当前为常量 `GALLERY_FOLDERS`）、`collapsed: Vec<UiGalleryFolderId>`、`selected: usize`。
- `UiGallery::rows(&self) -> Vec<UiGalleryRow>`：先文件夹行后其展开的组件行，折叠时隐藏后代；与 `PromptTree::rows` 同构，`selected` 为可见行下标。
- `selected_node` / `selected_component`：把可见行映射回节点/组件。
- `move_selection(delta)`：在可见行间钳制移动（不循环）；`set_selected(index)` 供鼠标点击。
- `toggle_folder(id)`：切换折叠；折叠若隐藏高亮所在的后代组件，则高亮回落到该文件夹行。
- `activate()`（回车/点击）：文件夹行切换折叠，组件行无动作。
- `move_left()`：组件行或折叠/顶层文件夹行移到所属文件夹行，已展开文件夹则折叠；`move_right()`：展开当前折叠文件夹。
- 组件展示名（`UiGalleryComponentId::label` / `UiGalleryFolderId::label`）与说明段落均取自 `ui/text.rs`（用户可见文案集中处）。

## 5. 全屏覆盖与分区（`src/ui/widgets/fullscreen.rs` + `src/ui/overlay/ui_gallery.rs`）

- 全屏覆盖 `area` 即整个 `screen`（无四周留白）；最外层绘制一条蓝色圆角边框（`style::accent()`）。`fullscreen::layout` 在宽 < 10 或高 < 3 时返回 `None`，此时不进入 `UiGallery` 状态（`ui_gallery_layout` 返回 `None`）。
- 标题与提示**内嵌在边框线上**（不额外占用内容行）：标题 `UI` 在顶边框左侧（`border_title` 淡蓝）、提示 `[↑/↓] select  [enter] fold  [esc] close` 在底边框左侧（muted）、`[✕]` 覆盖在顶边框右端（accent）。边框内区 `content` 整块为内容区。
- 左侧宽度 `left = 24.min(content.width - 2)`，保证右侧至少 1 列；`content.width < 3` 时不可用。
- 竖直分隔线：左区右边界列绘制 `│`，样式 `style::border(false)`；无水平分隔线（保持整洁）。
- `Clear` 整屏重置为终端默认底色，**不铺** `overlay_panel` 底色，透明终端下不出现背景块。

## 6. 渲染（`src/ui/overlay/ui_gallery.rs`）

- 调 `fullscreen::render` 画顶栏标题、`[✕]` 与底栏提示。
- 左区：逐行绘制文件夹/组件行；高亮行以行首 `❯` 标记（同 Agents/树列表语义）。
- 右区：按选中组件 `id` 先算自然尺寸 `appearance_size`，再在右区**水平居中**；`render_appearance` **按真实外观**绘制（复用组件自身样式与尺寸），不额外包裹样例框；其下空 1 行后逐行**水平居中**渲染 `description`（muted）。各组件样例：
  - `toast` / `tooltip`：圆角边框 + 面板底色 + 右上角 `[✕]`，宽度贴合正文（正文最长行 + 4）、高度 3 行。
  - `modal`：圆角强调色边框 + 标题 + `[✕]` + 问题行 + 底部按钮行（复用 `modal::button_row`）。
  - `menu`：圆角强调色边框 + 标题 + 条目行，首项 `overlay_selection` 高亮。
  - `button`：纯强调色标签行，**无边框、无底色**（组件本身就是标签）。
  - `fullscreen`：本全屏覆盖组件自身——蓝色圆角边框 + 标题/提示内嵌边框线 + 右上角 `[✕]`。
  - `prompt editor` / `prompt list`：复用真实 prompt 面板外观——圆角边框 + `Prompt Manager` 标题 + 工具栏/搜索行 + 分割线 + 内容行。
  - `prompt toolbar`：真实工具栏行——强调色按钮 + 右对齐 `[◉] [*] ●`。
- 预览只使用 `ui/style.rs` 的语义样式与 `ui/text.rs` 的文案常量，不读 `AppState` 领域数据。

## 7. 事件、依赖方向与失败语义

数据流：`[UI]` 命中（`ui/controls.rs` 几何）→ 鼠标路由（`event/mouse/press.rs`）→ `update::open_ui_gallery` 构造 `Overlay::UiGallery` → 覆盖键盘/点击路由更新高亮与折叠 → `ui/overlay/ui_gallery.rs` 只读渲染。

- `app/` 不依赖 `ui/`：真实外观由 `ui/` 侧按组件 id 绘制，`app/` 只保存 id 与选中/折叠状态。
- 打开期间底层点击不穿透（`handle_mouse_down_left` 在 `state.overlay.is_some()` 时早返回）。
- 关闭：`Esc` 或右上角 `[✕]`（全屏覆盖没有“浮层之外”，点击覆盖内部空白不关闭）。
- 失败语义：屏幕过小（`fullscreen::layout` 返回 `None`）时不进入覆盖；无 IO、无 DB、无后台任务、无 panic 路径。
- 不在渲染中修改状态、不执行 IO。

## 8. 验收标准

1. 侧栏展开且够宽时，`[SET]` 右侧显示 `[UI]`；点击后**整屏覆盖**（最外层蓝色边框、无四周留白、无背景色）；侧栏折叠或过窄时 `[UI]` 隐藏、`[SET]` 保留。
2. 覆盖内部左侧固定 24 列、右侧占剩余，中间一条竖直分隔线；顶栏标题 + `[✕]`、底栏快捷键提示。
3. 左侧默认展开 `global`、`prompt`；`global` 含 `toast/modal/tooltip/button/menu/fullscreen`，`prompt` 含 `editor/list/toolbar`。
4. 鼠标点击或 `↑/↓` 移动高亮；文件夹行回车/点击切换折叠；点组件行切换右侧预览。
5. 右侧显示对应组件的**真实外观**（无统一包裹大框）并**水平居中**，其下为逐行水平居中的说明文字，不触发真实业务行为。
6. `Esc` 或 `[✕]` 关闭；关闭后底层布局与状态不变。
7. 标题 `UI` 与快捷键提示分别内嵌在顶边框左侧与底边框左侧（参考 prompt 面板 `ws:` / `b:` 的边框内嵌做法）。
8. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通过；TUI 外观与交互由用户 `cargo run` 验收。

## 9. 实施影响

新增 `src/ui/widgets/fullscreen.rs`（+`tests.rs`）、`src/app/ui_gallery.rs`（+`tests.rs`）与 `src/ui/overlay/ui_gallery.rs`（+`tests.rs`）；修改 `src/app/{overlay.rs,mod.rs,tooltip.rs}`、`src/app/update/{dialog.rs,update.rs,ui_gallery.rs}`、`src/input/mod.rs`、`src/ui/{controls.rs,mod.rs,overlay.rs,text.rs,tooltip.rs,widgets/mod.rs}`、`src/event/{mouse/press.rs,overlay.rs}` 及对应 `tests.rs`。无新外部依赖、无浏览器资源、无 DB 迁移。
