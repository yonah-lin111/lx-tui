# Prompt 模板斜杠命令面板、Markdown 高亮对齐与模板块操作 — 设计

- 分支：`feature/prompt-template-commands`
- 状态：需求边界已确认（Grill Me 收口完成），待用户审核确认后执行

## 1. 背景与目标

参考 `lx-agent`（`LxMarkdownEditor.tsx`、`markdownSlashCommands/`、`markerDecorations.ts`、`editorTheme.ts`），在 `lx-tui` 的 Prompt 右栏编辑器中实现：

1. **`/` 斜杠模板命令面板**：
   - 支持 5 个核心模板命令：`/add`、`/bug`、`/common`、`/refactor`、`/style`（支持 `/addTemplate` 等别名）；
   - 插入标准多行 Markdown 模板块（`&&& <name>Template --start ... &&& <name>Template --end`）；
   - 插入后光标自动定位到首行标题占位符 `「title: 」` 的冒号之后。
2. **Markdown 语法高亮对齐**：
   - 支持 `&&&` 模板块高亮（起止行标记 `dim`、命令名按业务类型加粗分色、标题占位符 `Yellow` 下划线、正文保持 Markdown 解析）；
   - 支持 `@文件`（`@path/to/file`）高亮为 `Yellow` 下划线（对齐 `lx-agent` 色彩语义）。
3. **`&&&` 模板块操作按钮**：
   - 模板块起始行右缘右对齐显示功能按钮：`[copy]`（复制块内容）、`[clean]`（清理未填空项）、`[del]`（删除整块）、以及状态切换 `[todo]`/`[run]`/`[done]`；
   - 鼠标点击执行对应操作，变更录入撤销栈，支持 `Ctrl+Z` 一键复原。

## 2. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 命令标识与模板块格式 | 面板展示短名优先（`/add` 等），输入支持短名与长名双向别名匹配；插入模板严格对齐 `lx-agent` 的标准数据结构（`&&& addTemplate --start 「title: 」` ... `&&& addTemplate --end`），光标置于标题冒号后。 |
| 2 | `&&&` 模板块与 `@` 文件高亮配色 | 精细分词对齐 `lx-agent`：起止标记 `&&&`/`--start`/`--end` 用 `dim`；命令名按业务分色（`add` 绿 / `bug` 红 / `refactor` 紫 / `common` 蓝 / `style` 洋红，均加粗）；`「title: ...」` 与 `@文件` 均用 `Yellow` 下划线；块内正文正常按 Markdown 语法高亮。 |
| 3 | 斜杠命令触发位置与替换范围 | 严格行首触发（`^\s*/([a-zA-Z0-9_-]*)$`），光标在词尾；代码围栏内或已有模板块内坚决抑制；确认后整行替换为多行模板内容，单步可撤销；`@` 提及面板优先级高于 `/` 命令面板。 |
| 4 | 模板块右侧功能按钮与交互 | 模板首行右端右对齐显示 `[todo]`、`[copy]`、`[clean]`、`[del]`；长文本重叠时文本优先；鼠标左键点击执行，`[clean]` 与 `[del]` 单步可撤销，`[copy]` 触发系统剪贴板与 Toast 反馈。 |

## 3. 架构与数据结构设计

### 3.1 领域层（`src/app/markdown.rs`）

- **命令标识与触发**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum SlashCommandId {
      Add,
      Bug,
      Common,
      Refactor,
      Style,
  }

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct SlashTrigger {
      pub line_start: usize,
      pub line_end: usize,
      pub cursor: usize,
      pub query: String,
  }
  ```
- **命令面板状态机**：
  定义 `SlashPanel`（包含 `SlashTrigger`、候选列表、当前高亮 `active`、滚动锚点与显式视口），统一对齐 `BlockPanel` 与 `MentionPanel` 的视口管理。
- **模板内容生成与光标偏移**：
  实现 `slash_template_content(id: SlashCommandId) -> (&'static str, usize)`，返回与 `lx-agent` 严格一致的模板文本以及相对于起点的光标偏移量。
- **模板块解析与操作辅助函数**：
  - `parse_template_block_at_line(...) -> Option<TemplateBlockRange>`：解析模板起止行范围；
  - `clean_template_content(text: &str) -> String`：过滤未填写的空列表项；
  - `cycle_template_status(end_line: &str) -> String`：在 `todo`、`in_progress`、`done` 间轮转。

### 3.2 编辑器状态层（`src/app/prompt.rs`）

- `Prompt` 结构体持有 `slash_panel: Option<SlashPanel>`；
- `settle()` 阶段调用 `refresh_slash_panel()`；优先级规则：`mention` > `slash_panel` > `block_panel`；
- 面板操作：`slash_move(delta)`、`slash_confirm()`、`slash_escape()`、`slash_set_active(index)`、`slash_scroll(delta, base)`；
- 模板块操作：
  - `copy_template_block(start_line)`：提取纯净正文并返回；
  - `clean_template_block(start_line)`：就地清理并记录 `EditKind::Other` 单步快照；
  - `delete_template_block(start_line)`：删除整块并记录 `EditKind::Other` 单步快照；
  - `toggle_template_status(start_line)`：就地修改结束行状态并记录撤销快照。

### 3.3 路由与事件层（`src/app/update.rs` 与 `src/event/mod.rs`）

- 键盘路由：在 `route_panel` 中加入 `route_slash_panel` 分支（上下选择、回车确认、Esc 取消）；
- 鼠标命中：
  - 斜杠命令面板命中与悬停；
  - 模板首行按钮区域命中：计算 `[todo]`、`[copy]`、`[clean]`、`[del]` 矩形，分派至对应操作并触发反馈。

### 3.4 词法高亮层（`src/ui/markdown.rs` 与 `src/ui/style.rs`）

- **Token 种类扩展**：
  ```rust
  pub enum TokenKind {
      Marker,
      Heading,
      Strong,
      Emphasis,
      Strikethrough,
      InlineCode,
      CodeBlock,
      Quote,
      LinkText,
      Url,
      // 新增：
      TemplateMarker,      // &&&, --start, --end
      TemplateCommand(SlashCommandId), // add(绿), bug(红), refactor(紫), common(蓝), style(洋红)
      TemplateTitle,       // 「title: ...」
      FileMention,         // @path/to/file
  }
  ```
- **词法扫描**：
  - `scan_line` 扩展支持 `&&&` 模板行扫描（识别合法起始行与结束行）；
  - `scan_inline` 扩展识别 `@文件`（非空白 `@` 引导的路径字符，排查标点边界）。
- **ANSI 样式映射**（`src/ui/style.rs`）：
  - `template_marker()` → `muted()`；
  - `template_command(id)` → 对应 `Color::Green`、`Color::LightRed`、`Color::LightMagenta`、`Color::LightBlue`、`Color::Magenta` 加粗；
  - `template_title()` → `Yellow` + `UNDERLINED`；
  - `markdown_file_mention()` → `Yellow` + `UNDERLINED`。

### 3.5 渲染层（`src/ui/prompt.rs` 与 `src/ui/text.rs`）

- 复用 `ui/widgets/command_panel.rs` 渲染斜杠命令面板，标题为 `Templates`；
- 渲染模板开始行时，在行末右侧计算并绘制 `[todo]/[run]/[done] [copy] [clean] [del]` 操作按钮；
- 用户可见文案（命令描述、按钮文字、提示语）集中收敛至 `src/ui/text.rs`。

## 4. 边界与防护

1. **窄屏避让**：当 Prompt 内容区过窄时，按钮组自动隐藏或按优先级省略，确保文本正文绝对不被裁切遮挡。
2. **防误触抑制**：普通行中输入 `/`（如 URL、除号）严格不触发弹窗；未闭合围栏与已有模板块内部严格不触发。
3. **撤销原子性**：模板插入、整块删除、字段清理均作为单一快照落栈，`Ctrl+Z` 可单步完全回滚。
4. **ANSI 规范遵从**：所有视觉样式严禁使用背景涂抹或 RGB 杂色，严格遵从现有终端原生配色基线。
