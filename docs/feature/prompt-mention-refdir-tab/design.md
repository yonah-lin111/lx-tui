# Prompt @ 命令面板 refDir Tab 与工具栏调整：设计

## 1. 目标与范围

为 Prompt 编辑视图的 `@` 文件提及命令面板添加顶栏 Tab 支持，支持在当前工作区文件（`Files`）与已配置的引用文件夹（`refDir`）之间无缝切换；同时将 Prompt 顶栏工具栏原 `[dir]` 按钮文字调整为 `[refDir]`。

### 1.1 背景与现状

1. **Prompt 工具栏 `[dir]` 按钮**：
   - 位于 Prompt 编辑器顶部工具栏，点击后打开项目级引用文件夹菜单（`ReferencedFolderMenu`），供用户管理、新增磁盘目录。
   - 现存标识为 `text::PROMPT_FOLDERS_LABEL = "[dir]"`，宽度为 5 列。
2. **`@` 命令面板（Mention Panel）**：
   - 在 Prompt 编辑器中键入 `@` 触发（`markdown::mention_trigger`），默认异步扫描工作区根目录并展示文件/文件夹列表。
   - 浮层由 `ui::widgets::command_panel` 渲染，当前顶边框仅固定显示标题 `Files`（或 `Files-[main]` / `Files-[wt]`）。
   - 键盘上下键移动高亮，回车插入 `@相对路径`，`Shift+Enter` 进入子目录，`Esc` 关闭。
   - 目前缺少直接在 `@` 面板中引用已配置的外部引用文件夹的能力。

### 1.2 核心改动

1. **Prompt 工具栏按钮调整**：
   - 将 `PROMPT_FOLDERS_LABEL` 从 `"[dir]"` 修改为 `"[refDir]"`。
   - 按钮显示宽度从 5 列扩展为 8 列（`[refDir]` 占用 8 列），更新 `PROMPT_FOLDERS_WIDTH` 与编辑栏最小宽度计算。
2. **`@` 面板顶部新增 Tab**：
   - 面板顶边框渲染两个可切换 Tab：`Files`（工作区文件，默认激活）与 `refDir`（引用文件夹）。
   - 支持键盘 `Left` / `Right` 左右方向键循环切换 Tab，同时保留输入缓冲区的查询关键词（Query），保证来回切换内容完整不丢失。
   - 支持鼠标点击顶边框 Tab 标签直接切换。
3. **`refDir` Tab 列表与插入行为**：
   - 数据源直接来自当前工作区已配置的引用文件夹列表（`state.referenced.folders`）。
   - 当 `@` 后面有输入查询词时，使用该查询词对引用目录的“文件夹名称”和“绝对路径”执行大小写不敏感的包含过滤。
   - 条目行样式对齐现存 `MentionRow`（双行）：
     - 第一行：目录图标 `📁 ` + 目录名 `/`
     - 第二行：`└─ ` + 中间截断的绝对路径
   - 空状态防误触：若尚未添加引用目录或无过滤匹配项，展示置灰提示 `(无匹配的引用文件夹)`，回车不触发替换。
   - 选中并回车确认后，将触发区间 `@query` 替换为 `@[refer-folder](<绝对路径>) `（携带尾随空格）。
4. **动态底栏提示（Footer）**：
   - `Files` Tab：`←→ tab · ↑↓ select · ↵ confirm · ⇧↵ enter · esc cancel`
   - `refDir` Tab：`←→ tab · ↑↓ select · ↵ insert · esc cancel`（隐藏不适用的 `⇧↵ enter`）。

---

## 2. 详细技术方案

### 2.1 架构分层与数据流

遵循项目级规范：
- `app/` 负责纯状态管理与更新，不执行 IO，不引入渲染代码；
- `ui/` 负责纯只读渲染与坐标计算，不修改状态；
- 事件层负责按键与鼠标事件的分发。

```
[用户键入 @ 或文字]
       │
       ▼
[Editor / Prompt] ────────── 更新光标与文本
       │
       ├─► 保持 query 过滤词不变
       ├─► MentionState.refresh(query)
       │       ├─► Files Tab: 过滤工作区文件 entries
       │       └─► RefDir Tab: 过滤 referenced.folders
       ▼
[键盘 Left / Right] ────────► MentionState.switch_tab() (循环切换 Files ◄► RefDir)
       │
       ▼
[键盘 Enter] ───────────────► MentionState.confirm()
       │                          ├─► Files: 插入 @relative/path
       │                          └─► RefDir: 插入 @[refer-folder](abs/path) 
       ▼
[UI command_panel 渲染] ────► 顶边框渲染 [Files] [refDir] Tab + 动态底栏提示
```

### 2.2 核心模块变更

#### 1. 文本与布局常量 (`src/ui/text.rs` & `src/layout/prompt.rs`)

- `src/ui/text.rs`:
  ```rust
  pub const PROMPT_FOLDERS_LABEL: &str = "[refDir]";
  pub const MENTION_TAB_FILES: &str = "Files";
  pub const MENTION_TAB_REF_DIR: &str = "refDir";
  pub const MENTION_FOOTER_FILES: &str = "←→ tab · ↑↓ select · ↵ confirm · ⇧↵ enter · esc cancel";
  pub const MENTION_FOOTER_REF_DIR: &str = "←→ tab · ↑↓ select · ↵ insert · esc cancel";
  pub const MENTION_REF_DIR_EMPTY: &str = "(无匹配的引用文件夹)";
  ```
- `src/layout/prompt.rs`:
  - 将 `PROMPT_FOLDERS_WIDTH` 从 `5` 调整为 `8`。
  - `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 同步适配。

#### 2. 通用面板组件 Tab 绘制与命中 (`src/ui/widgets/command_panel.rs`)

扩展 `CommandPanelView`，支持在边框顶部渲染 Tab：
```rust
pub struct PanelTab<'a> {
    pub label: &'a str,
    pub active: bool,
}

pub struct CommandPanelView<'a> {
    // ...
    pub tabs: Option<&'a [PanelTab<'a>]>,
    // 保持现存 title 字段向后兼容
}
```
- 在 `layout()` 中计算各 Tab 标签的局部矩形区域，保存在 `PanelLayout` 中：
  ```rust
  pub struct PanelLayout {
      // ...
      pub tabs: Vec<Rect>,
  }
  ```
- 在 `render()` 中：
  - 若 `tabs` 存在，在边框顶线绘制 Tab（激活项使用 `style::accent()` 加粗/高亮，未激活项使用 `style::muted()`）；
  - 保留右侧 `right_title`（如 `Files-[main]`）。
- 提供 `tab_at(layout: &PanelLayout, column: u16, row: u16) -> Option<usize>` 用于鼠标点击命中。

#### 3. Mention 领域模型扩展 (`src/app/prompt/mention.rs` & `src/app/prompt.rs`)

在 `MentionState` 中增加 Tab 状态与引用文件夹快照同步：
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MentionTab {
    #[default]
    Files,
    RefDir,
}

impl MentionTab {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::RefDir,
            Self::RefDir => Self::Files,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Self::Files => Self::RefDir,
            Self::RefDir => Self::Files,
        }
    }
}
```

`MentionState` 增加字段：
```rust
pub struct MentionState {
    // ... 原有字段保持
    tab: MentionTab,
    /// 引用文件夹原始快照
    referenced_folders: Vec<ReferencedFolder>,
    /// refDir 选中的高亮索引
    ref_dir_active: usize,
}
```

核心方法：
- `sync_referenced_folders(&mut self, folders: &[ReferencedFolder])`：同步当前项目的引用文件夹。
- `tab(&self) -> MentionTab`：获取当前活跃 Tab。
- `switch_tab(&mut self, delta: isize)`：左右切换 Tab，高亮索引重置为 0，保留输入字符与查询词。
- `set_tab(&mut self, tab: MentionTab)`：支持鼠标点击直接设值。
- `filtered_ref_folders(&self, text: &str, cursor: usize) -> Vec<&ReferencedFolder>`：按光标处 `@query` 执行大小写不敏感过滤。
- `confirm(&mut self, text_len: usize) -> Option<(Range<usize>, String)>`：
  - 若 `tab == MentionTab::Files`：走原有逻辑插入 `@relative/path`。
  - 若 `tab == MentionTab::RefDir`：获取当前选中的引用文件夹，生成 `@[refer-folder](绝对路径) `，替换 `@query` 触发范围。
- `enter_folder()`：在 `RefDir` 模式下直接返回 `None`（不穿透进入内部文件）。

#### 4. 按键路由与事件处理 (`src/app/update/editor.rs` & `src/event/mouse/press.rs`)

- `src/app/update/editor.rs`:
  在 `route_mention_panel` 中增加 `Left` / `Right` 分发：
  ```rust
  fn route_mention_panel(state: &mut AppState, command: &EditorCommand) -> bool {
      match command {
          EditorCommand::Up => state.prompt.mention_move(-1),
          EditorCommand::Down => state.prompt.mention_move(1),
          EditorCommand::Left => state.prompt.mention_switch_tab(-1),
          EditorCommand::Right => state.prompt.mention_switch_tab(1),
          EditorCommand::Newline => state.prompt.mention_confirm(),
          EditorCommand::Escape => state.prompt.mention_escape(),
          EditorCommand::EnterFolder => {
              if !state.prompt.mention_active() {
                  return false;
              }
              state.prompt.mention_enter_folder();
              true
          }
          _ => false,
      }
  }
  ```
- `src/event/mouse/press.rs`:
  在鼠标点击处理中，检测是否点击了 mention 面板顶部的 Tab：
  ```rust
  if let Some(tab_index) = mention_tab_at(state, view, mouse.column, mouse.row) {
      update::switch_mention_tab_to(state, tab_index);
      *dirty = true;
      return;
  }
  ```

#### 5. Prompt UI 渲染与组装 (`src/ui/prompt/panels.rs`)

- 更新 `mention_panel_data`：
  - 当 `tab == MentionTab::Files` 时，生成 Files 的 `CommandItem`；
  - 当 `tab == MentionTab::RefDir` 时，生成引用文件夹的 `CommandItem`（带 `📁 ` 图标与中间截断路径）；空状态时生成 `(无匹配的引用文件夹)`。
  - 组装 `tabs = [PanelTab { label: "Files", active: ... }, PanelTab { label: "refDir", active: ... }]`。
  - 动态切换底栏 `footer`。

---

## 3. 边界条件与健壮性

1. **来回切换 Tab 状态保持**：
   - 用户输入 `@abc` 切到 `refDir`，再按左键切回 `Files`，文本区内容 `@abc` 始终不受影响，文件扫描与过滤缓存正常复用。
2. **空状态与无匹配保护**：
   - 若用户尚未配置任何引用文件夹，或输入的关键词在 `refDir` 下无匹配项，回车不会执行任何插入，面板保持开启或可通过 `Esc` 退出。
3. **路径含空格与特殊字符**：
   - 插入使用 `crate::app::markdown::format_reference(ReferKind::Folder, &folder.path)`，确保格式统一且合法。
4. **窗口与尺寸适应**：
   - 面板宽度自适应条目文本与 Tab 宽度，如果终端宽度受限，引用路径自动进行居中省略截断（`ellipsize_middle`），避免字符溢出破坏边框。
