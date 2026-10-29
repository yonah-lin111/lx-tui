# Prompt @ 命令面板 refDir Tab 与工具栏调整 — 执行任务清单

- **任务目录**：`docs/feature/prompt-mention-refdir-tab/`
- **设计文档**：`docs/feature/prompt-mention-refdir-tab/design.md`
- **工程约束**：严格遵守 `AGENTS.md` 及 `docs/standards/*` 规范；生产代码禁止 `unwrap()` / `expect()`；所有文案集中于 `src/ui/text.rs`；所有样式集中于 `src/ui/style.rs`；测试文件严格跟随被测模块目录结构；测试单文件不超过 1000 行。

---

## 阶段一：需求质询与方案设计（已完成）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索 `lx-tui` 的 `@` 命令面板、Prompt 工具栏、引用文件夹（`ReferencedFolders`）相关实现
- [x] **Grill Me 问题 1**：确认 `refDir` tab 候选内容（直接列出已配置的引用文件夹列表，选中后插入 `@[refer-folder](目录绝对路径)`）
- [x] **Grill Me 问题 2**：确认 `@` 面板左右键切换保留查询词、过滤搜索机制及空状态提示（展示 `(无匹配的引用文件夹)`，回车不触发误插入）
- [x] **Grill Me 问题 3**：确认顶部 Tab 左右循环切换、底栏动态快捷键提示（`←→ tab · ↑↓ select · ↵ insert · esc cancel`）及鼠标点击切换 Tab 支持
- [x] **Grill Me 问题 4**：确认改动范围聚焦于右栏 Prompt 编辑器（`src/ui/prompt/`），不扩散至 lx 对话输入框
- [x] 编写详细设计文档 `design.md` 与任务清单 `task.md`

---

## 阶段二：Prompt 工具栏 [dir] 按钮调整为 [refDir]

### 2.1 文案与布局层 (`src/ui/text.rs` & `src/layout/prompt.rs`)
- [x] **`src/ui/text.rs`**：
  - 将 `PROMPT_FOLDERS_LABEL` 从 `"[dir]"` 修改为 `"[refDir]"`
- [x] **`src/layout/prompt.rs`**：
  - 更新 `PROMPT_FOLDERS_WIDTH` 为 `8`（对应 `"[refDir]".len()`）
  - 校验并更新依赖 `PROMPT_FOLDERS_WIDTH` 的最小宽度常量 `PROMPT_FOLDERS_MIN_WIDTH_EDITOR`
- [x] **相关测试适配**：
  - 更新 `src/ui/prompt/tests.rs`、`src/layout/tests.rs`、`src/ui/text/tests.rs` 中涉及 `"[dir]"` 宽度的断言

---

## 阶段三：通用 CommandPanel 顶边框 Tab 绘制与命中支持

### 3.1 渲染组件扩展 (`src/ui/widgets/command_panel.rs`)
- [x] 定义 `PanelTab<'a>` 结构体（包含 `label: &'a str`、`active: bool`）
- [x] 在 `CommandPanelView<'a>` 中增加 `pub tabs: Option<&'a [PanelTab<'a>]>` 字段
- [x] 在 `PanelLayout` 中增加 `pub tabs: Vec<Rect>` 记录各 Tab 标签的全局坐标
- [x] 在 `layout()` 中为边框顶线上的 Tab 标签计算坐标与占用空间（考虑与右侧标题的间距）
- [x] 在 `render()` 中绘制 Tab 组：激活 Tab 使用 `style::accent()` 高亮，未激活 Tab 使用 `style::muted()`
- [x] 实现 `pub fn tab_at(layout: &PanelLayout, column: u16, row: u16) -> Option<usize>` 提供鼠标点击命中支持
- [x] 补充 `src/ui/widgets/command_panel` 相关单元测试

---

## 阶段四：Mention 领域模型与 Prompt 状态扩展

### 4.1 领域模型与按键逻辑 (`src/app/prompt/mention.rs` & `src/app/prompt.rs`)
- [x] 定义 `pub enum MentionTab { Files, RefDir }`，提供 `next()`, `prev()` 循环切换
- [x] 在 `MentionState` 中增加 `tab: MentionTab`、`ref_folders: Vec<ReferencedFolder>` 及 `ref_active: usize`
- [x] 实现 `MentionState::switch_tab(&mut self, delta: isize) -> bool`
- [x] 实现 `MentionState::set_tab(&mut self, tab: MentionTab) -> bool`
- [x] 实现 `MentionState::sync_referenced_folders(&mut self, folders: &[ReferencedFolder])`
- [x] 实现 `refDir` 模式下的过滤函数：基于当前 `@query` 对引用目录名称与路径进行不区分大小写模糊匹配
- [x] 改造 `confirm(&mut self, text_len: usize)`：
  - 当处于 `MentionTab::RefDir` 时，获取当前高亮的引用文件夹，生成 `@[refer-folder](<path>) ` 并替换 `@query` 范围
- [x] 在 `Prompt` 上暴露相关委派方法：
  - `pub fn mention_tab(&self) -> MentionTab`
  - `pub fn mention_switch_tab(&mut self, delta: isize) -> bool`
  - `pub fn mention_set_tab(&mut self, tab: MentionTab) -> bool`
  - `pub fn sync_referenced_folders(&mut self, folders: &[ReferencedFolder])`

---

## 阶段五：按键路由、事件流与 Prompt UI 组装

### 5.1 按键与鼠标路由 (`src/app/update/editor.rs` & `src/event/`)
- [x] **`src/app/update/editor.rs`**：
  - 在 `route_mention_panel` 中拦截 `EditorCommand::Left` 和 `EditorCommand::Right`，调用 `state.prompt.mention_switch_tab(-1 / 1)`
- [x] **`src/app/update/pane.rs` 或相关同步点**：
  - 确保在编辑器激活或引用文件夹变更时同步 `state.referenced.folders` 到 `state.prompt`
- [x] **`src/event/mouse/press.rs` & `src/event/mouse/hit.rs`**：
  - 增加 `mention_tab_at` 命中检测，点击顶栏 Tab 时切换当前活跃 Tab

### 5.2 渲染集成与文案 (`src/ui/prompt/panels.rs`)
- [x] **`src/ui/text.rs`**：
  - 增加相关常量：`MENTION_TAB_FILES = "Files"`、`MENTION_TAB_REF_DIR = "refDir"`、`MENTION_FOOTER_FILES`、`MENTION_FOOTER_REF_DIR`、`MENTION_REF_DIR_EMPTY = "(无匹配的引用文件夹)"`
- [x] **`src/ui/prompt/panels.rs`**：
  - 改造 `mention_panel_data`：
    - 根据当前 `tab` 生成对应的候选行：`Files` 走现有文件条目；`RefDir` 遍历已过滤的引用文件夹生成 `MentionRowText::File`（`is_directory: true`，显示名称与中间截断路径）
    - 无引用文件夹时生成一条特殊的置灰禁用项 `MENTION_REF_DIR_EMPTY`
    - 组装 `tabs = &[PanelTab { label: "Files", ... }, PanelTab { label: "refDir", ... }]`
    - 根据活跃 Tab 动态切换底栏 `footer`（`RefDir` 时显示不含 `enter folder` 的 footer）

---

## 阶段六：精确测试验证与回归

- [x] **单元测试验证**：
  - `src/ui/prompt/tests.rs`：测试 `@` 面板显示 Tab、切换 Tab、`refDir` 下的条目渲染与中间路径截断
  - `src/app/prompt/tests.rs`：测试左右键切换 Tab、过滤搜索保持查询词、回车插入 `@[refer-folder](xxx) `
  - `src/layout/tests.rs`：测试工具栏 `[refDir]` 宽度 8 及其右侧按钮落位
- [x] **代码质量与编译检查**：
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test` 针对受影响测试用例进行精确验证
