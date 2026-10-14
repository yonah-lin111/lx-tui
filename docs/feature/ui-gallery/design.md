# UI 组件查看弹窗（`[UI]`）：设计

- 分支：`ui-gallery`（工作区 `.worktrees/ui-gallery`）
- 状态：**待用户审核；本文记录已通过 Grill Me 盘问确认的边界与实现约束**

## 1. 目标与范围

在左侧栏底边框的 `[SET]` 按钮右侧新增一个按钮 `[UI]`。点击后打开一个尽可能大的居中模态弹窗，弹窗内部划分为左右两区：

- 左侧为组件树：顶层文件夹 `global` 与 `prompt`，文件夹可折叠展开；`global` 下含 `toast`、`modal`、`tooltip`、`button`、`menu`，`prompt` 下含 `editor`、`list`、`toolbar`。
- 右侧为选中组件的事例：以只读静态预览绘制该组件的外观样例，并在下方给出若干行用途/触发方式说明。

弹窗只做“查看”，不驱动真实业务逻辑；关闭后不影响任何现有面板状态。

## 2. 已确认决策（Grill Me 盘问结论）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 左侧树结构 | 顶层两个文件夹。`global` 下含 `toast / modal / tooltip / button / menu`；`prompt` 下含 `editor / list / toolbar`。文件夹可折叠展开。 |
| 2 | `prompt` 子项归属 | 右键菜单归 `global`（`menu`）；编辑器/树列表/工具栏归 `prompt`。 |
| 3 | 右侧事例形态 | **静态预览 + 文字说明**：只读绘制外观样例 + 几行用途说明；样例不可交互，不与真实组件状态耦合。 |
| 4 | 弹窗四周留白 | 四周各留 **2 列 / 2 行**（即占屏幕宽、高各减 4），居中。 |
| 5 | 左侧切换方式 | 左侧为竖排 tab/树列表；鼠标点击切换，键盘 `↑/↓` 移动高亮，`Esc` 关闭弹窗；`→` 展开文件夹、`←` 折叠文件夹（或移到父级）。不做 `Ctrl+数字` 等额外快捷键。 |
| 6 | 文件夹折叠 | 可折叠展开；文件夹行左侧 `▾`（展开）/`▸`（折叠）三角，回车或点击切换；默认全部展开。 |
| 7 | 分栏与左侧宽度 | 左侧固定 **24 列**，右侧占剩余全部宽度；弹窗过窄时按比例压缩左侧（保证右侧至少 1 列）。 |
| 8 | 左右分隔视觉 | 左右之间画一条竖直边框线 `│`，样式跟随弹窗边框（muted）。 |
| 9 | 命名与图标 | 全英文命名；文件夹行 `▾`/`▸` 三角（accent），子项行 `•` 图标，子项缩进 2 列，与 prompt list 树视图风格一致。 |
| 10 | `[UI]` 可见性 | 不依赖设置服务；侧栏展开且宽度足够时显示，侧栏折叠或过窄时隐藏。 |
| 11 | 按钮间距与降级 | `[UI]` 紧跟 `[SET]` 右侧隔 1 列；宽度不足时先隐藏 `[UI]`、保留 `[SET]`。 |

## 3. 已核实的实现基线

| 位置 | 现状及复用点 |
| --- | --- |
| `src/ui/controls.rs` | `settings_button` / `settings_button_at` / `render_settings_button`：侧栏底边框左端 `[SET]` 的几何、命中与逐格绘制，`[UI]` 完全对齐这一套并放在其右侧。 |
| `src/event/mouse/press.rs:122` | `[SET]` 的左键命中分支；`[UI]` 在其后追加同款分支。 |
| `src/ui/widgets/modal.rs` | 模态容器：居中、`Clear` 清底、圆角边框、标题与右上角 `[✕]`；`layout(screen, w, h)` 负责居中几何，`close_button_at` 负责关闭命中。 |
| `src/app/overlay.rs` | `Overlay` 枚举与 `OverlayKind`；新增 `Overlay::UiGallery` 与 `OverlayKind::UiGallery`。 |
| `src/ui/overlay.rs` | 浮层渲染分派；新增 `ui_gallery` 子模块与分支。 |
| `src/event/overlay.rs` | `handle_overlay_click` 的浮层点击分派；新增 `UiGallery` 分支（点条目切换/折叠、点外部或 `[✕]` 关闭）。 |
| `src/app/update/dialog.rs` | `apply_overlay_key` 的按键分派；新增 `ui_gallery` 键处理。 |
| `src/input/mod.rs` | `overlay_key` 按 `OverlayKind` 过滤按键；新增 `UiGallery` 分支（`Esc/Up/Down/Left/Right/Enter`）。 |
| `src/app/prompt/tree.rs` | 文件夹/条目的“扁平存储 + 可见行 + 折叠”模型，是左侧树的直接参照（但 UI 画廊数据为静态常量，不进 DB）。 |
| `src/ui/prompt/tree.rs` | 树行渲染：折叠三角、`├─`/`└─` 连接符、`•` 图标、缩进，右侧预览沿用其绘制手法。 |
| `src/app/tooltip.rs`、`src/ui/tooltip.rs`、`src/ui/text.rs` | `TooltipTarget` 枚举、`targets()` 注册与 `tooltip_hint` 映射；新增 `SidebarUi` 目标与 `open ui gallery` 提示。 |

## 4. 数据模型（`src/app/ui_gallery.rs`，纯数据）

UI 画廊内容为编译期静态定义，不写数据库、不依赖工作区；唯一状态是“当前高亮行”与“文件夹折叠集合”。

```rust
/// 组件树的静态节点种类。
pub enum UiGalleryNode { Folder, Component }

/// 一个组件（叶子）：稳定 id、展示名与右侧说明。
pub struct UiGalleryComponent {
    pub id: UiGalleryComponentId,
    pub label: &'static str,
    pub description: &'static [&'static str],
}

/// 顶层文件夹：稳定 id、展示名与下属组件。
pub struct UiGalleryFolder {
    pub id: UiGalleryFolderId,
    pub label: &'static str,
    pub components: &'static [UiGalleryComponent],
}
```

- `UiGallery` 持有 `folders: &'static [UiGalleryFolder]`（当前为常量 `GALLERY_FOLDERS`）、`collapsed: Vec<UiGalleryFolderId>`、`selected: usize`。
- `UiGallery::rows(&self) -> Vec<UiGalleryRow>`：先文件夹行后其展开的组件行，折叠时隐藏后代；与 `PromptTree::rows` 同构，`selected` 为可见行下标。
- `selected_row` / `selected_folder` / `selected_component`：把可见行映射回节点。
- `move_selection(delta)`：在可见行间钳制移动（不循环）。
- `toggle_folder(id)` / `activate()`（回车）：高亮在文件夹行时切换折叠、在组件行时无动作；鼠标点击文件夹行切换折叠、点击组件行仅选中。
- `left()`：高亮在组件行或折叠/顶层文件夹时移到其文件夹行；`right()`：高亮在文件夹行时展开。
- 组件内容（`label`、`description`）是用户可见文案，但作为组件“数据”保留在常量表中；右侧预览的固定文案（如样例的标题）放 `ui/text.rs`。

## 5. 左侧宽度与降级（`src/app/ui_gallery.rs` 纯函数 + `src/ui/overlay/ui_gallery.rs` 几何）

- 弹窗 `area`：`widgets::modal::layout(screen, screen.width - 4, screen.height - 4)`，放不下时该函数返回 `None`，此时浮层打开请求直接忽略（不进入 `UiGallery` 状态）。
- 左侧宽度：`left = 24.min(inner.width.saturating_sub(1))`，保证至少给右侧留 1 列；`inner.width <= 1` 时整体不可用（`shell` 为 `None`）。
- 竖直分隔线：左区右边界即 `inner.x + left` 列绘制 `│`，样式 `style::border(...)`（与弹窗边框一致，用 muted）。

## 6. 渲染（`src/ui/overlay/ui_gallery.rs`）

- 复用 `widgets::modal::render` 画外框、标题 `UI` 与右上角 `[✕]`。
- 左区：逐行绘制文件夹/组件行；高亮行以行首 `❯` 标记（同 Agents/树列表语义），鼠标悬停沿用 `selection`？——本期只做高亮 `❯` 标记（键盘/点击选中），不加悬停底色，避免与“静态预览”定位冲突。
- 右区：按选中组件的 `id` 分派到 `render_preview`，用只读段落/边框画出样例，下方渲染 `description` 行。预览只使用 `ui/style.rs` 的语义样式与 `ui/text.rs` 的文案常量，不读 `AppState` 领域数据。
- 底边框提示：左区末尾或弹窗底边显示 `[↑/↓] select  [enter] fold  [esc] close`。

## 7. 事件、依赖方向与失败语义

数据流：`[UI]` 命中（`ui/controls.rs` 几何）→ 鼠标路由（`event/mouse/press.rs`）→ `update::open_ui_gallery` 构造 `Overlay::UiGallery` → 浮层键盘/点击路由更新高亮与折叠 → `ui/overlay/ui_gallery.rs` 只读渲染。

- `app/` 不依赖 `ui/`：预览所需的“静态外观”由 `ui/` 侧按组件 id 绘制，`app/` 只保存 id 与选中/折叠状态。
- 弹窗打开期间底层点击不穿透（`handle_mouse_down_left` 在 `state.overlay.is_some()` 时早返回）。
- 失败语义：屏幕放不下弹窗（`modal::layout` 返回 `None`）时不进入浮层；`Esc`/`[✕]`/点击弹窗外关闭；无 IO、无 DB、无后台任务、无 panic 路径。
- 不在渲染中修改状态、不执行 IO。

## 8. 验收标准

1. 侧栏展开且够宽时，`[SET]` 右侧显示 `[UI]`；点击打开大弹窗，四周留 2 列/2 行空白；侧栏折叠或过窄时 `[UI]` 隐藏、`[SET]` 保留。
2. 弹窗左侧固定 24 列、右侧占剩余，中间一条竖直分隔线。
3. 左侧默认展开 `global`、`prompt`；`global` 含 `toast/modal/tooltip/button/menu`，`prompt` 含 `editor/list/toolbar`。
4. 鼠标点击或 `↑/↓` 移动高亮；文件夹行回车/点击切换折叠；点组件行切换右侧预览。
5. 右侧显示对应组件的静态外观样例与说明文字，不触发真实业务行为。
6. `Esc`、`[✕]` 或点击弹窗外关闭；关闭后底层布局与状态不变。
7. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通过；TUI 外观与交互由用户 `cargo run` 验收。

## 9. 实施影响

预计新增 `src/app/ui_gallery.rs`（+`src/app/ui_gallery/tests.rs`）与 `src/ui/overlay/ui_gallery.rs`（+`src/ui/overlay/ui_gallery/tests.rs`）；修改 `src/app/overlay.rs`、`src/app/update/{dialog.rs,menu.rs}`、`src/input/mod.rs`、`src/ui/{controls.rs,overlay.rs,text.rs,tooltip.rs}`、`src/app/tooltip.rs`、`src/event/{mouse/press.rs,overlay.rs}` 及对应 `tests.rs`。无新外部依赖、无浏览器资源、无 DB 迁移。
