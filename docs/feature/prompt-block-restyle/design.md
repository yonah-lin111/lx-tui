# Prompt 块结构与页列表重构 — 设计

- 状态：**已实施**（P1–P5 已在 `feat/prompt-block-restyle` 完成并验证通过）。
- 后续调整：`&&&` 状态不再染 edit 区外框，改为标题行页控件左侧的状态标签（`[todo]`/`[run]`/`[done]`，按状态取蓝/黄/绿）；§6 的边框染色仅作历史记录。
- 分支 / 工作区：`feat/prompt-block-restyle` @ `.worktrees/prompt-block-restyle`（基于 `dev` `c59c4bbd`）。
- 范围：`src/app/markdown/`、`src/app/prompt/`、`src/app/update/`、`src/ui/prompt*`、`src/ui/overlay/`、`src/ui/right_panel.rs`、`src/layout/prompt.rs`、`src/input/mod.rs`、`src/event/`、`assets/settings.js`。
- 取代：`docs/feature/prompt-block-focus/`（块聚焦）、`docs/feature/prompt-supplement-fold/`（补充块折叠）、`docs/feature/prompt-pagination/` 中的「页码菜单」与「标题行页名重命名」部分。旧文档保留，在顶部加「已被 prompt-block-restyle 取代」标注。
- 数据库：**不改 schema、不加迁移**，`SCHEMA_VERSION` 保持 20。

---

## 1. 背景与目标

现状：

1. `+++` 补充块只能嵌套在 `&&&` 内部（`blocks.rs::block_scan` 只在 `template` 打开时识别 `+++`）。
2. 一页可以有任意多个 `&&&`。
3. 块用整圈 box 边框（`┌─ ┐ │ └─ ┘`）绘制，占用左侧 2 列和右侧 1 列。
4. `+++` 默认折叠，有 `[▸]/[▾]` 按钮，配套一整套光标吸附和写入拦截逻辑。
5. 工具栏有块聚焦按钮 `[⊞]/[⊡]`，`Ctrl/Cmd+←/→` 在块之间切换。
6. 页切换靠标题行的 `‹ n/m ›` 和点击计数弹出的 `PageMenu` 浮层；页名在标题行内联重命名。

目标：

| # | 需求 | 本设计结论 |
|---|---|---|
| R1 | 一页只能有一个 `&&&`，可以有多个 `+++` | 只认第一个 `&&&`；入口隐藏 `&&&` 命令 |
| R2 | `+++` 不在 `&&&` 里；新建追加到最后一个 `+++` 后面，空一行并滚动到可见 | `+++` 改为顶层平级块，按固定落点追加 |
| R3 | 移除块边框，改用其他区分方式 | 左侧 1 列色条 `▎` 加分割细线 |
| R4 | `&&&` 状态色用作 edit 区边框 | 从分割线往下的容器边框跟随状态色 |
| R5 | 移除块聚焦按钮，快捷键改为切页 | `Ctrl/Cmd+←/→` 切换上一页/下一页 |
| R6 | 移除 `+++` 的折叠 | 折叠整套删除 |
| R7 | 页列表右侧加状态圆点，标题取 `&&&` 的 title | 新增 `PromptMode::Pages` 视图 |
| R8 | 页列表交互对齐 prompt list | 搜索、单击选中、双击打开、右键菜单 |
| R9 | `Ctrl/Cmd+L` 打开 prompt list，`Ctrl/Cmd+P` 打开 page list；再按一次或按 Esc 返回 | 切换矩阵见 §7 |
| R10 | 评估 settings 的影响 | 只改一条文案（§10） |

---

## 2. 已确认的决策（逐题对照）

| 题 | 决策 | 结论 |
|---|---|---|
| 1 | `+++` 结构 | `+++` 是顶层块，**只**能出现在 `&&&` 外面。`&&&` 内部出现的 `+++` 行当普通文本处理。旧数据不迁移 |
| 2 | 单 `&&&` 约束 | 入口：页面已有 `&&&` 时，斜杠面板隐藏所有生成 `&&&` 的命令。解析：只认第一个 `&&&`，后面的起止行一律当普通文本。不拦截输入或粘贴，也不弹提示 |
| 3 | `+++` 新建 | 任意位置都能触发（围栏内除外）。触发的命令行整行删除。落点依次判断：最后一个 `+++` 之后 → `&&&` 之后 → 文末，中间补空行。光标落到 `{title: }` 的 `}` 前，滚动到新块可见。整个操作记一步撤销 |
| 4 | 去边框 | 左侧 1 列色条 `▎`：`&&&` 按状态着色，`+++` 固定品红。右侧槽位改为 0 列。删除未使用的 `template_block_bg` / `supple_block_bg` |
| 5 | 块内分割线 | 保留，画成色条 `▎` 加满行 `─`，不画端点，颜色跟随所在块 |
| 6 | 边框染色 | 染色范围是分割线（`├─┤`）、内容区左右竖边和底边。tab 行和按钮行保持灰色。颜色取本页 `&&&` 的状态色（todo 蓝 / in_progress 黄 / done 绿）。没有 `&&&`、处于 Tree/Pages 视图或当前是 Todo tab 时保持灰色 |
| 7 | 切页快捷键 | `Ctrl/Cmd+←/→` 等价于 `switch_prompt_page(∓1)`，边界处原地不动、不循环。按词移动改用 `Alt+←/→` 和 `Alt+B/F`。块聚焦整套删除。`Pin` 改为以 `Save` 为锚点 |
| 9 | 页列表形态 | 新增 `PromptMode::Pages`，删除 `Overlay::PageMenu`。必须先有编辑目标才能进入 |
| 11 | 页右键菜单 | 页行菜单：`Open / Rename / Copy content / New page / Delete`。空白区菜单：`New page`。Rename 复用 `Overlay::Rename`。Delete 用菜单行内二次确认。删除标题行的行内重命名。右键不改变选中行 |
| 11+ | 鼠标 | 单击选中，双击打开该页（复用 `prompt_row_double_click`，窗口期 400ms） |
| 12 | 快捷键范围 | 只在 prompt 获得焦点时生效，按 §7 的切换矩阵执行。补全面板打开时先关面板再切换视图。未绑定编辑目标时 `Esc` 保持现状 |
| 13 | 页行内容 | `❯ 标题 … ●`。标题依次取独立 `{title:}` → 行内 `{title:}` → 模板名 → 页名 → 占位名。圆点：todo 空心灰、in_progress 黄实心、done 绿实心，没有 `&&&` 时不画。当前编辑页加粗。按显示标题搜索。Tree 视图的 `n|m|x` 计数不动 |
| 14 | 折叠 | 整套删除 |
| 15 | Pages 视图头部 | 工具栏：`[/]` 加钉住按钮。标题行：`条目名 · pages`，**隐藏 `‹ ›`**，计数显示选中行的序号，点击计数返回 Editor。编辑器状态原样保留，不做暂存 |
| 16 | 新建页位置 | 统一追加到末尾，DB 层不改 |
| 17 | 复制 | `[copy]` 只复制当前这一块。删除 `copy_template_content` 中跳过嵌套 `+++` 的分支 |
| 18 | 交付 | 在 worktree 中分 P1–P4 四个代码阶段实施，再加一个 P5 文档与 settings 文案阶段，每个阶段自行 commit。只迁移本次改动涉及的测试。不自行合并到 `dev` |
| 19 | settings | `md_scope=template` 的语义不变。文案改为 `Inside blocks (&&& / +++)` |
| 20 | 扫描器统一 | `block_scan` 作为唯一的块结构来源。`template_line_infos` 系列函数和 `ui::markdown::scan` 都改为从它投影。规则对 lx 输入框和 file_diff 高亮同样生效 |

---

## 3. 块协议与解析（P1）

### 3.1 新的块语法

```text
&&& refactorTemplate --start          ← 唯一有效的 &&&（第一个）
{title: 重构 prompt}

- Location: ...
&&& in_progress

+++ supplement --start                ← 顶层 +++（可多个）
{title: 补充 1}

## Supplementary Requirements
...

+++ supplement --end

+++ supplement --start                ← 新建追加在最后一个 +++ 之后
{title: }
...
+++ supplement --end
```

规则：

- **`&&&`**：文档中第一个合法的 `&&&` 起始行开启块，遇到第一个合法结束行时闭合。块闭合之后，再出现的 `&&&` 起始行**不再开启新块**，按普通文本处理。块未闭合时延伸到文末，与现有行为一致。
- **`+++`**：只在**不处于任何块内**时识别起始行。`+++` 块内部只识别同名的 `--end`，与现有行为一致。`+++` 未闭合时延伸到文末。
- **`&&&` 块内的 `+++` 行**：不取得块身份，作为 `&&&` 的普通正文行（Middle 角色）。
- **`+++` 块内的 `&&&` 行**：不取得块身份，作为 `+++` 的普通正文行。
- 代码围栏优先级最高。围栏内的任何标记都不识别，与现有行为一致。
- 旧的 `+++ log/logTemplate` 记录块语法仍然由 `SUPPLE_RESERVED` 排除，不作为补充块。

### 3.2 `block_scan` 改写（`src/app/markdown/blocks.rs`）

主循环改成单层状态机。`fill_roles` 不再需要按类型分两轮覆盖，因为块之间不会再重叠：

```rust
/// 当前打开的块：类型与起始行。
#[derive(Clone, Copy)]
enum Open {
    Template(usize),
    Supple(usize),
}

pub fn block_scan(text: &str) -> BlockScan {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut spans: Vec<BlockSpan> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut open: Option<Open> = None;
    // 已出现过 `&&&`（含未闭合）：之后的 `&&&` 起始行按普通文本处理（一页一个任务块）。
    let mut template_seen = false;

    for (index, line) in lines.iter().enumerate() {
        // 围栏处理与现状一致（略）
        match open {
            Some(Open::Template(start)) => {
                if let Some(end) = parse_template_end_line(line) {
                    spans.push(BlockSpan { kind: BlockKind::Template, start, end: index,
                                           status: end.status, closed: true });
                    open = None;
                }
            }
            Some(Open::Supple(start)) => {
                if parse_supple_end_line(line).is_some() {
                    spans.push(BlockSpan { kind: BlockKind::Supple, start, end: index,
                                           status: TemplateStatus::Todo, closed: true });
                    open = None;
                }
            }
            None => {
                if !template_seen && parse_template_start_line(line).is_some() {
                    template_seen = true;
                    open = Some(Open::Template(index));
                } else if parse_supple_start_line(line).is_some() {
                    open = Some(Open::Supple(index));
                }
            }
        }
    }
    // 未闭合块延伸到末行（与现状一致）
    // spans 按起始行排序后 fill_roles / mark_separators
}
```

要点：

- **`spans` 的顺序**：现状是按结束行压栈。改成平级后，块之间不会交叠，压栈顺序就是文档顺序。`block_spans` 的注释同步改为「文档顺序」。
- `fill_roles` 简化成一轮遍历，不再需要「`+++` 覆盖父块角色」的逻辑。
- `mark_separators` 不变。它只看相邻行是否属于同一种块，与嵌套关系无关。
- `BlockContext` 的三个枚举值语义不变：`Normal`、`Template`（`&&&` 正文）、`Supple`（`+++` 正文）。

### 3.3 扫描器统一（决策 20）

#### 3.3.1 `template.rs` 中的平行扫描器

| 函数 | 现状 | 改为 |
|---|---|---|
| `template_line_infos(text)` | 自己逐行扫描 `&&&` | 从 `block_scan(text).infos` 中筛出 `kind == Template` 的行，映射为 `TemplateLineInfo { role, status, separator }`（`BlockRole` 转 `TemplateLineRole`） |
| `parse_template_block_at_line(text, line)` | 自己向下找结束行 | 在 `block_scan(text).spans` 中找 `kind == Template && start == line` 的块，返回 `TemplateBlockRange { start, end }` |
| `template_block_start_at(text, offset)` | 基于 `template_line_infos` 向上回溯 | `block_at(text, offset)` 过滤 `kind == Template` 后取 `start` |
| `inside_template_block(text, offset)` | 自己扫描 | 已核实生产代码中没有调用方，只有 `app/markdown/tests.rs:1197` 一处测试使用。**直接删除**函数及该用例 |

`TemplateLineInfo` / `TemplateLineRole` 这两个类型保留，因为 `refer_shortcut.rs` 和 `ui/prompt.rs` 都在用。只改它们的生成方式。

#### 3.3.2 `ui::markdown::scan` 高亮扫描器（`src/ui/markdown.rs`）

现状 `ScanState { fence, template, supple }` 自己维护了一套块状态，`+++` 只在 `template == true` 时识别。改为：

```rust
pub fn scan(text: &str) -> Vec<Vec<Token>> {
    let blocks = crate::app::markdown::block_line_infos(text);
    let mut state = ScanState::default(); // 只剩 fence
    text.split('\n')
        .enumerate()
        .map(|(index, line)| scan_line(line, blocks.get(index).copied().flatten(), &mut state))
        .collect()
}
```

`scan_line` 根据传入的 `Option<BlockLineInfo>` 判断：

- `info.role == Start && kind == Template` → `scan_template_start`
- `info.role == End && kind == Template` → `scan_template_end`
- `info.role == Start && kind == Supple` → `scan_supple_start`
- `info.role == End && kind == Supple` → `scan_supple_end`
- `info.is_some()`（块内 Middle 行）→ 识别独立标题行和 `{wt:}` 行，规则与现在 `state.template` 分支一致
- 其他情况 → 普通 markdown

效果：第二个 `&&&` 起始行和 `&&&` 内的 `+++` 行都不会被标记为 `TemplateMarker` / `SuppleMarker`，按普通文本高亮。这个规则对 lx 输入框（`ui/main_content/lx/input.rs:47`）和 file_diff 卡片（`file_diff.rs:546`）同样生效。

> 依赖方向：`ui::markdown` → `app::markdown` 已经存在（`markdown::parse_worktree_line` 等），不新增反向依赖。

### 3.4 斜杠命令可见性（`src/app/markdown/slash.rs::slash_filter`）

`SlashContext` 新增一个字段：

```rust
pub struct SlashContext {
    pub context: BlockContext,
    pub worktree_available: bool,
    /// 本页已有 `&&&`（含未闭合）：隐藏一切生成 `&&&` 的命令。
    pub template_present: bool,
}
```

`Prompt::refresh_slash_panel`（`panel.rs:400`）在组装上下文时，用 `block_scan(&self.text).spans.iter().any(|s| s.kind == Template)` 填入这个字段。

新的可见性矩阵：

| 候选 | Normal | Template（`&&&` 内） | Supple（`+++` 内） |
|---|---|---|---|
| 内置模板命令 `SlashCommandId::ALL` | 只在 `!template_present` 时显示 | ✗ | ✗ |
| 自定义 `AgentBlock`（`&&&`） | 只在 `!template_present` 时显示 | ✗ | ✗ |
| 内置 `/supplement` | ✓ | ✓ | ✓ |
| 自定义 `SuppleBlock` | ✓ | ✓ | ✓ |
| `/gitWorkTree` | ✗ | 需要 `worktree_available` | 需要 `worktree_available` |
| `Raw` + `Global` | ✓ | ✓ | ✓ |
| `Raw` + `Template` | ✗ | ✓ | ✓ |

> `/supplement` 在 Normal 上下文中也可见，这是新增的。代码围栏内本来就不会触发斜杠面板（`slash_trigger`），不需要改。

### 3.5 `+++` 插入落点（`src/app/prompt/panel.rs::slash_confirm`）

现状：`self.splice_text(trigger.line_start, trigger.line_end, &content)` 原地替换命令行。改为：

```rust
_ => {
    let Some((content, cursor)) = markdown::slash_insertion(&item) else { return false; };
    self.slash = None;
    self.break_group();
    self.record(EditKind::Other);
    if matches!(item.insertion, markdown::SlashInsertion::SuppleBlock { .. }) {
        self.append_supple_block(trigger, &content, cursor);
    } else {
        self.splice_text(trigger.line_start, trigger.line_end, &content);
        self.cursor = trigger.line_start + cursor;
    }
    self.panel_suppressed = true;
    self.scroll_cursor_into_view();
    true
}
```

> 已核实：内置 `/supplement`（`SlashItem::builtin_supplement`，`slash.rs:491`）和自定义 `block: supple` 都使用 `SlashInsertion::SuppleBlock { command, title, body }`，只需匹配这一个变体。不再用 `content.starts_with("+++")` 嗅探字符串。`builtin_supplement` 的 doc 注释「仅在 `&&&` 正文出现」同步改为「任意位置可见」。

新增 `Prompt::append_supple_block`，放在 `src/app/prompt/block.rs`：

```rust
/// 删除斜杠命令行后，把补充块追加到固定落点；调用方已记录撤销快照。
///
/// 落点：最后一个 `+++` 结束行之后 → `&&&` 结束行之后 → 文末；
/// 与前文之间恰好隔一个空行，光标落在新块标题值（`{title: }` 的 `}` 前）。
pub(super) fn append_supple_block(&mut self, trigger: &SlashTrigger, content: &str, cursor: usize)
```

算法：

1. **删除命令行**：删掉 `trigger.line_start..trigger.line_end` 这段字节，再连同其后的一个换行一起删除（如果命令行是最后一行，就删除其前面的换行）。这样原位置不会留下空行。复用 `block_deletion_range` 的思路，但只吸收一侧的换行。
2. **在删除后的文本上**重新 `block_scan`，取锚点行：
   - `spans` 中最后一个 `kind == Supple` 的 `end`；
   - 没有的话，取 `kind == Template` 的 `end`；
   - 再没有的话，取文末。
3. **计算插入偏移**：锚点行之后的位置是 `line_end_at(anchor) `。
   - 锚点之后**还有内容**：插入文本 `"\n\n" + content`，插在锚点行行尾。如果锚点下一行已经是空行，就只插 `"\n" + content + "\n"`，保证与上下文各隔一个空行。实施时用测试覆盖这三种空行组合。
   - 锚点是文末：文本为空时直接插 `content`；文本非空时插 `"\n\n" + content`，并去掉文末多余的空行，保证中间恰好隔一个空行。
4. `self.cursor = 插入起点 + 前导换行长度 + cursor`。
5. **整个过程只有一个撤销快照**，由调用方在步骤 1 之前调用 `record`。

滚动规则（决策 3「滚到新块可见」）：

```rust
/// 让 `[start_line, end_line]` 尽量完整可见；块高于视口时保证起始行可见。
pub(super) fn scroll_lines_into_view(&mut self, start_line: usize, end_line: usize)
```

放在 `viewport.rs`。`slash_confirm` 在插入后调用 `scroll_lines_into_view(new_start, new_end)`，替代原来的 `scroll_cursor_into_view`。光标位于新块第二行，按这个规则必然可见。

> 删除折叠以后，`panel.rs:232` 的 `self.expanded_supplements.insert(...)` 一并删除。

### 3.6 复制（`src/app/markdown/template.rs::copy_template_content`）

- 删除 `inside_supple` 状态以及 `CopySubblock::Supple` 分支。平级之后，`&&&` 正文里不会再有被识别为块的 `+++`。
- `CopySubblock::Log` 分支保留，旧的 `+++ log` 记录块仍然要跳过。这样 `CopySubblock` 只剩一个变体，可以简化成 `fn is_log_subblock_start(line) -> bool`。
- `copy_text_at_cursor`（`prompt/template.rs:18`）的优先级判断「先 `+++` 后 `&&&`」保留。块不会重叠，这个顺序已经不影响结果，但保持现状可以少改代码。

### 3.7 worktree 绑定

`worktree.rs` 的 `block_end_line_at`、`worktree_line_spans` 等函数只依赖 `block_scan` 的 span 和 `closed`，与嵌套无关。改为平级后不需要修改，只需补一条用例：顶层 `+++` 的 `{wt:}` 绑定可以写入，也可以识别。

---

## 4. 删除块聚焦与折叠（P2）

### 4.1 折叠（决策 14），整套删除

| 位置 | 删除内容 |
|---|---|
| `src/app/prompt/fold.rs` 与 `fold/tests.rs` | 整个文件 |
| `src/app/prompt.rs` | `mod fold;`、字段 `expanded_supplements`、`fold_blocked`；`Snapshot` / `PromptHistory` 中的 `expanded_supplements`；`set_text`（294 行）的 clear；`restore_history`（343 行）、`restore_snapshot`（918 行）的赋值；`normalize_fold_cursor` 的两处调用（345、941 行）；`newline` 中的 `fold_newline_target`（388、869 行）；`replace_range` / splice 中的 `edit_hits_fold` 拦截与 `shift_folds`（846–860 行）；`take_fold_blocked` |
| `src/app/prompt/navigation.rs` | `snap_visible` 的 4 处调用（13、21、105、113 行），改为直接赋值 `self.cursor = at` |
| `src/app/prompt/viewport.rs` | `use super::fold`、`hidden` 向量、折叠过滤分支；模块文档中关于折叠的描述 |
| `src/app/prompt/panel.rs:232` | 新建补充块时的展开 |
| `src/app/update/editor.rs` | `report_fold_blocked` 函数及其 3 处调用 |
| `src/ui/prompt.rs` | `BlockButton::Fold`、`SUPPLE_BUTTONS` 中的 Fold、`block_button_label` 的折叠分支、`BlockButtonHit.folded`、`render` / `block_buttons` 中的 `folded` 集合、`paint_block_border` 的折叠分支 |
| `src/ui/text.rs` | `SUPPLE_BUTTON_FOLD_CLOSED/OPEN`、`SUPPLE_FOLD_BLOCKED`；tooltip 中与折叠按钮有关的文案 |
| `src/event/mouse/hit.rs:420` | `(Supple, Fold)` 分支 |
| `src/ui/tooltip.rs` / `src/app/tooltip.rs` | 补充块折叠按钮的 `TooltipTarget`（如果有） |

### 4.2 块聚焦（决策 7），整套删除

| 位置 | 删除 / 修改 |
|---|---|
| `src/app/prompt/block_focus.rs` | **整个文件删除**。其中 `title_value_offset` 被 slash 插入复用的话，迁到 `prompt/template.rs`。实施时用 codegraph 确认是否还有其他调用方 |
| `src/app/prompt.rs` | `mod block_focus;`、字段 `focus_block`、`Snapshot.focus_block`、`PromptHistory.focus_block`、`reconcile_block_focus` 的两处调用（346、935 行） |
| `src/app/prompt/viewport.rs` | `focused_block_range` 过滤与 `outside_focus` |
| `src/app/update/prompt.rs` | `toggle_prompt_block_focus`（238 行），以及 501、562 行的 `exit_block_focus` |
| `src/app/update/editor.rs` | `delete_template_block` 中的聚焦接续（538 行）、`restore_prompt_view` 中的 `exit_block_focus`（362 行）；`BlockPrev/BlockNext` 分派（97–110 行）改为切页（§4.3） |
| `src/app/update/pane.rs:965` | 窄屏自动退出块模式的逻辑 |
| `src/app/update/prompt/pages.rs:672` | `reload_editor_from_active_page` 中的 `exit_block_focus` |
| `src/app/state.rs` | `select_all_prompt` 中的块内分支（如果有，用 codegraph 确认） |
| `src/layout/prompt.rs` | `PromptToolbarButton::BlockFocus`、`PROMPT_BLOCK_FOCUS_WIDTH`、`PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR`；`PROMPT_PIN_MIN_WIDTH_EDITOR` 改为以 `Save` 为锚点重新计算（§4.4） |
| `src/ui/prompt.rs` / `header.rs` / `prompt_frame.rs` | `render_toolbar` / `render_header` 的 `block_focus`、`block_focus_available` 参数，以及 `prompt_frame.rs:165` 的可用性计算 |
| `src/ui/text.rs` | `PROMPT_BLOCK_FOCUS_ALL/ONE_LABEL`；tooltip 文案 `show all blocks` / `focus template block` |
| `src/app/tooltip.rs` / `src/ui/tooltip.rs:166` | `TooltipTarget::PromptBlockFocus` |
| `src/event/mouse/press.rs:382` | `BlockFocus` 按钮分支 |

### 4.3 `Ctrl/Cmd+←/→` 改为切页

`src/app/actions.rs`：

```rust
/// Ctrl/Cmd+←：编辑视图切到上一页（首页原地不动，不循环）。
PagePrev,
/// Ctrl/Cmd+→：编辑视图切到下一页（末页原地不动，不循环）。
PageNext,
```

替换掉 `BlockPrev { cmd }` / `BlockNext { cmd }`，不再需要 `cmd` 字段。

`src/input/mod.rs::editor_command`：

```rust
// Super 分支
KeyCode::Left => Some(EditorCommand::PagePrev),
KeyCode::Right => Some(EditorCommand::PageNext),
// Ctrl 分支
(true, false, KeyCode::Left) => Some(EditorCommand::PagePrev),
(true, false, KeyCode::Right) => Some(EditorCommand::PageNext),
```

`src/app/update/editor.rs::apply_editor`：

```rust
EditorCommand::PagePrev => { super::switch_prompt_page(state, -1); }
EditorCommand::PageNext => { super::switch_prompt_page(state, 1); }
```

> `switch_prompt_page` 会先 `commit_editing_prompt`、清空面板和选区、改写当前页，再 `reload_editor_from_active_page`，与点击 `‹ ›` 完全一样。它在 `apply_editor` 中的位置要放在 `route_panel` 之后。补全面板打开时，`route_panel` 不消费这两个命令，所以还要显式调用 `state.prompt.clear_panel()`。实施时用测试确认面板会被关闭。

`input/mod.rs` 的 doc 注释同步修改：去掉「Ctrl+←/→ 按词移动」，按词移动只保留 `Alt+B/F`、`Alt+←/→`。

### 4.4 工具栏布局（编辑视图）

删除 `[⊞]` 后，编辑视图右组从右到左是：`●`（Save）、`[○/◉]`（Pin）、`[refDir]`（Folders）。

```rust
/// 显示钉住按钮所需的最小内容区宽度（列）：编辑视图在保存点左侧、树/页视图在模式按钮右侧。
pub(crate) const PROMPT_PIN_MIN_WIDTH_EDITOR: u16 =
    PROMPT_SELECT_ALL_MIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_PIN_WIDTH;
```

`prompt_toolbar_button_rect` 中 `Pin` 的 editor 分支改为：

```rust
Rect::new(
    bar.right().saturating_sub(1 + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_PIN_WIDTH),
    bar.y, PROMPT_PIN_WIDTH, 1,
)
```

`prompt_toolbar_button_at` 的遍历数组中删掉 `BlockFocus`。`layout/tests.rs` 中与块聚焦相关的几何用例删除，Pin 的阈值和坐标用例改用新常量。

---

## 5. 块渲染（P3）

### 5.1 色条与槽位常量（`src/layout/prompt.rs`）

```rust
/// prompt 块左侧色条占位（`▎` + 0 列间隔）；右侧不再占位。
pub const PROMPT_BLOCK_GUTTER_LEFT: u16 = 1;
```

- 删除 `PROMPT_TEMPLATE_GUTTER_LEFT = 2` 和 `PROMPT_TEMPLATE_GUTTER_RIGHT = 1`，改名为 `PROMPT_BLOCK_GUTTER_LEFT = 1`，并删除 RIGHT。
- `viewport.rs::visual_rows` 中的 `margins` 改为 `PROMPT_BLOCK_GUTTER_LEFT`。
- `viewport.rs::line_gutter` 改用新常量。
- `ui/prompt.rs::block_gutter`、`block_buttons_start`、`block_text_end` 同步修改。`block_text_end` 中块行的右边界改为 `area.right()`，不再扣 GUTTER_RIGHT。

> 色条与正文之间不留空列。现状左槽 2 列中的第 2 列（`─` / 空格）用来分隔正文，现在由色字符 `▎` 本身的视觉宽度提供分隔（`▎` 是左 1/4 块）。如果实施后视觉上太挤，再改成 2 列（`▎` + 空格），只需改这一个常量。

### 5.2 文案常量（`src/ui/text.rs`）

```rust
/// prompt 块左侧色条字形（`&&&` 状态色 / `+++` 品红）。
pub const PROMPT_BLOCK_BAR: &str = "▎";
/// 块结构分割线（标题下 / 绑定行上的空行）。
pub const PROMPT_BLOCK_RULE: char = '─';
```

### 5.3 `ui/prompt.rs::render` 改写

现状每行调用 `paint_block_border(… role, folded, painted, buttons_start)`，按角色画 `┌ │ └ ┐ ┘` 和补线。改为：

```rust
/// 块行左侧色条：每个视觉行（含软换行续行）首列画 `▎`，色随块类型/状态。
fn paint_block_bar(buf: &mut Buffer, area: Rect, y: u16, info: BlockLineInfo) {
    if let Some(cell) = buf.cell_mut((area.x, y)) {
        cell.reset();
        cell.set_symbol(text::PROMPT_BLOCK_BAR);
        cell.set_style(block_color(info));
    }
}

/// 块颜色：`&&&` 按状态（`style::template_border`）、`+++` 固定品红（`style::supple_border`）。
fn block_color(info: BlockLineInfo) -> Style {
    match info.kind {
        BlockKind::Template => style::template_border(info.status),
        BlockKind::Supple => style::supple_border(),
    }
}
```

`paint_block_separator` 改为：x = `area.x` 处画 `▎`，`area.x+1 ..area.right()` 画 `─`，颜色都用 `block_color`，不再画 `├` / `┤`。

删除的内容：`block_row_role`（软换行降级只在画 `┌ └` 时需要）、`paint_block_border`、`render` 中的 `role` / `start_row` 计算。保留的内容：起始行仍然需要按钮组并用省略号截断，`start_row` 的判断改成直接看 `info.role == Start && first`。

### 5.4 按钮组

- `&&&`：`[status] [copy] [clean] [del]` 不变，右对齐到 `area.right()`。
- `+++`：`[copy] [clean] [del]`，删除 Fold。
- `SUPPLE_BUTTONS` 改为 3 个元素。`block_buttons_start` 的右边界从 `area.right() - 1`（原来要给右边框 `┐` 留位置）改为 `area.right()`。
- `template_buttons()` 改为从 `block_scan` 投影的 `template_line_infos` 获取信息。只有第一个 `&&&` 有按钮，这一点由 §3.3 自动保证。

### 5.5 未使用样式

`style.rs` 中的 `template_block_bg`（698 行）、`supple_block_bg`（985 行），以及 328 行引用它们的注释，全部删除。`style/tests.rs` 中相关的用例一并删除。

---

## 6. edit 区边框跟随状态色（P3，决策 6）

### 6.1 染色范围

```text
┌ Prompt ─ Todo ───────────────────┐   y = panel.y          灰（tab 行，Block 画）
│ [≡] [←] [→] [all]  [refDir] [○] ●│   y = panel.y+1        竖边灰
├─ item · page ─────────── ‹ 1/3 › ┤   y = divider          状态色（├ ─ ┤）
│ ▎&&& refactorTemplate --start    │   y ∈ (divider, bottom)  竖边状态色
└──────────────────────────────────┘   y = panel.bottom-1   状态色（└ ─ ┘，底边内容保持原样式）
```

### 6.2 颜色派生（只读，不写状态）

`ui/prompt_frame.rs`：

```rust
/// edit 区边框色：编辑视图 + Prompt tab + 本页存在 `&&&` 时取其状态色；否则 None（保持灰）。
pub(crate) fn edit_border_style(state: &AppState) -> Option<Style> {
    if state.right_tab != RightTab::Prompt
        || state.prompt_owner().prompt_panel.mode != PromptMode::Editor
    {
        return None;
    }
    crate::app::markdown::block_spans(state.prompt.text())
        .into_iter()
        .find(|span| span.kind == BlockKind::Template)
        .map(|span| style::template_border(span.status))
}
```

> 未闭合的 `&&&`，`status` 是 `Todo`（蓝），与页列表的圆点规则一致。

### 6.3 绘制顺序（`ui/right_panel.rs::render`）

1. `Block::bordered().border_style(style::border(focused))` 照常画整圈灰色外框。
2. `prompt_frame::render_body` 画工具栏、分割线（`render_divider` → `render_rule`）和正文。
3. **新增**：`render_body` 返回之后，如果 `edit_border_style(state)` 是 `Some(color)`，调用

```rust
/// 用 `color` 重绘分割线行至底边的外框字形（左右竖边、底边 `─` 与两角）；
/// 只改前景色，不改字形与单元格内容，底边已绘制的文字/控件不覆盖。
fn tint_edit_border(buf: &mut Buffer, panel: Rect, from_y: u16, color: Style)
```

- `from_y = prompt_divider_rect(panel).y`；
- 左右两列 `x = panel.x` 与 `x = panel.right()-1`，从 `from_y` 到 `panel.bottom()-1` 的每一格都 `set_style(color)`；
- 分割线行与底边行的内部 `x`，只给字形为 `─`、`├`、`┤`、`└`、`┘` 的单元格 `set_style(color)`。这样 `render_bottom` 画在底边上的文字、折叠按钮等都保持原来的样式。

这个函数放在 `ui/right_panel.rs`，因为外框是在这个文件里画的。颜色由 `prompt_frame::edit_border_style` 派生，渲染层只读状态。

> 分割线由 `render_rule` 画成 `style::border(focused)`，`tint` 只覆盖它的前景色，标题行之后叠加的 `条目名 · 页名 ‹ n/m ›` 文字由 `paint_title_row` 画，字形不是边框字符，所以不会被染色。

---

## 7. 视图模式与快捷键（P4，决策 9 / 12 / 15）

### 7.1 `PromptMode`（`src/app/prompt/tree/panel.rs`）

```rust
pub enum PromptMode {
    /// 树状管理器：文件夹与条目列表。
    Tree,
    /// 编辑视图：编辑选中条目正文。
    Editor,
    /// 页列表：当前编辑目标的页目录（编辑绑定保留，编辑缓冲原样驻留不渲染）。
    Pages,
}
```

`PromptPanel` 新增 Pages 视图专用状态（不放进 Tree 的 `selected/scroll/search`，避免互相污染）：

```rust
/// 页列表视图：选中行、滚动与搜索过滤（进入时选中当前页、清空搜索）。
#[derive(Debug, Default)]
pub struct PageListView {
    pub selected: usize,
    pub scroll: usize,
    pub search: TextInput,
}
// PromptPanel 内
pub pages_view: PageListView,
```

所有 `match panel.mode` 都要补上 `Pages` 分支。`prompt_tree_mode(state)` 语义保持「只是 Tree」。新增：

```rust
/// 显示中工作区是否处于页列表视图。
pub fn prompt_pages_mode(state: &AppState) -> bool
```

需要逐一核对的 `PromptMode` 引用点（来自 grep，实施前用 codegraph 再确认一遍）：
`ui/tooltip.rs:157`、`ui/prompt.rs:75`、`ui/text.rs:984`、`ui/prompt_frame.rs:34/67/185/206/255`、`app/update/editor.rs:346/357`、`app/update/prompt.rs:133/183/431/468/559/832/884`、`app/update/prompt/tree_view.rs:28`、`app/update/prompt/pages.rs:367/659`，以及 `event/mouse/{hover,drag,hit,wheel,press}.rs`、`event/{input,paste,overlay}.rs` 中所有的 `prompt_tree_mode`。

关键语义：

- `reload_editor_from_active_page`（`pages.rs:659`）要求 `mode == Editor`。从 Pages 视图打开某一页时，**先切到 Editor，再调用 `select_prompt_page`**，顺序见 §7.4。
- `rebind_after_page_delete`（`pages.rs:367`）要求 `mode == Editor`。在 Pages 视图中删页时，由 `perform_delete_page` 处理当前页变化：被删的是当前页，就按「下一页优先」改写 `active_page`，并把编辑缓冲重载为新的当前页正文。重载必须在 Pages 视图下也能执行，所以把 `reload_editor_from_active_page` 的条件放宽到 `mode != Tree`。

### 7.2 进入与返回

新文件 **`src/app/update/prompt/pages_view.rs`**，`app/update/prompt.rs` 已经有 959 行，不往里面加代码：

```rust
/// 进入页列表：需编辑视图且已绑定目标；先结算当前页，选中当前页、清空搜索。
pub fn show_prompt_pages(state: &mut AppState) -> bool {
    if prompt_tree_mode(state) || !prompt_editing(state) { return false; }
    commit_editing_prompt(state);
    finish_page_rename_noop; // 行内重命名已删除，此处无事
    state.prompt.clear_panel();
    state.selection = None;
    // mode = Pages；pages_view.selected = 当前页序号；search 清空；scroll 修正
    true
}

/// 页列表返回编辑视图：mode = Editor；编辑缓冲未动，光标/撤销/滚动原样。
pub fn hide_prompt_pages(state: &mut AppState) -> bool

/// 打开选中页：先回编辑视图，再切页（同页则仅返回）。
pub fn open_selected_page(state: &mut AppState) -> bool {
    let Some(page) = selected_page_id(state) else { return false; };
    hide_prompt_pages(state);
    select_prompt_page(state, page); // 同页返回 false，无副作用
    true
}

pub fn toggle_prompt_pages(state: &mut AppState) -> bool  // Pages ↔ Editor
pub fn move_page_selection(state: &mut AppState, delta: isize, visible: usize) -> bool
pub fn select_page_row(state: &mut AppState, row: usize) -> bool
pub fn apply_page_search(state: &mut AppState, command: &EditorCommand)
pub fn page_row_double_click(...)  // 复用 PROMPT_DOUBLE_CLICK_WINDOW，记点击用独立字段
```

> **编辑缓冲不需要暂存**（决策 15）：Pages 视图不清空 `editing`，也不调用 `stash_editor`。`state.prompt` 原样留在原处，只是不渲染。进入前已经 `commit_editing_prompt` 把正文写回页，所以 Pages 视图读到的页正文（标题和状态）就是最新的。

### 7.3 切换矩阵（决策 12）

| 当前 | `Ctrl/Cmd+L` | `Ctrl/Cmd+P` | `Esc` | `[≡]/[/]` 按钮 | 点击标题行计数 |
|---|---|---|---|---|---|
| Editor | `show_prompt_tree` | `show_prompt_pages`（没有编辑目标时无反应） | 保持现状（绑定目标 → Tree） | → Tree | → Pages |
| Tree | `toggle_prompt_panel_mode`（→ Editor） | 无反应 | → Editor | → Editor | 不显示 |
| Pages | `hide_prompt_pages` + `show_prompt_tree` | `hide_prompt_pages` | `hide_prompt_pages` | → Editor | → Editor |

> Pages 视图的模式按钮显示 `[/]`（与 Tree 相同，表示「回编辑」）。

输入层（`src/input/mod.rs`）：

- `editor_command` 新增两项，同时接受 `CONTROL` 和 `SUPER`，排除 `ALT`：
  ```rust
  KeyCode::Char('l' | 'L') => Some(EditorCommand::TogglePromptList),
  KeyCode::Char('p' | 'P') => Some(EditorCommand::TogglePageList),
  ```
  Ctrl 分支加在 `(true, false, KeyCode::Char(ch))` 的 match 中，Super 分支加在 `SUPER` 的 match 中。
- 只有 `focus == Prompt` 时才会进入 `editor_command`（`route` 的现有结构），终端和 lx 窗格不受影响。

update 层（`apply_editor` 的最前面，在 Tree/Pages 路由之前）：

```rust
match command {
    EditorCommand::TogglePromptList => { state.prompt.clear_panel(); return route_toggle_list(state); }
    EditorCommand::TogglePageList => { state.prompt.clear_panel(); return route_toggle_pages(state); }
    _ => {}
}
```

> 这两个命令必须在 `route_panel` 之前处理，保证「补全面板打开时先关面板再切换」。树视图的 `route_prompt_tree` 和页视图的 `route_prompt_pages` 收不到这两个命令，所以不会被当作搜索字符写入。

### 7.4 页视图按键路由（`route_prompt_pages`，放在 `editor.rs`，与 `route_prompt_tree` 对称）

| 命令 | 行为 |
|---|---|
| `Up` / `Down` | `move_page_selection(±1)` |
| `Newline`（Enter） | `open_selected_page` |
| `Escape` | `hide_prompt_pages` |
| `PagePrev` / `PageNext` | 吞掉（Pages 视图里不切页） |
| 其他 | `apply_page_search`（复用 `TextInput` 编辑语义，与 `apply_prompt_search` 同构） |

`Routed::Copy`（`event/input.rs:113`）：Pages 视图下 `Ctrl+C` 清空页搜索，与 Tree 视图一致。
`event/paste.rs:22`：Pages 视图下粘贴写入页搜索框，与 Tree 视图一致。

---

## 8. 页列表渲染与命中（P4，决策 13 / 15）

### 8.1 行模型（纯函数，`src/app/prompt/pages_list.rs` 新文件）

```rust
/// 页列表一行的派生数据：页 id、显示标题、`&&&` 状态（无 `&&&` 为 None）、是否当前页。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRow {
    pub page: PromptPageId,
    pub title: String,
    pub status: Option<TemplateStatus>,
    pub current: bool,
}

/// 页正文的显示标题与状态：独立 `{title:}` → 行内 `{title:}` → 模板名 → None（调用方回退页名）。
pub fn page_title_status(content: &str) -> (Option<String>, Option<TemplateStatus>)

/// 页列表全部行（按页顺序），按显示标题过滤（大小写不敏感子串，复用 `name_matches`）。
pub fn page_rows(pages: &[PromptPage], active: PromptPageId, query: &str) -> Vec<PageRow>
```

`page_title_status` 的实现：

```rust
let scan = markdown::block_scan(content);
let Some(span) = scan.spans.iter().find(|s| s.kind == BlockKind::Template) else {
    return (None, None);
};
let lines: Vec<&str> = content.split('\n').collect();
let start = lines.get(span.start).and_then(|l| markdown::parse_template_start_line(l));
let standalone = lines.get(span.start + 1)
    .and_then(|l| markdown::parse_title_line(l))
    .map(str::trim).filter(|t| !t.is_empty());
let inline = start.and_then(|s| s.title).map(str::trim).filter(|t| !t.is_empty());
let command = start.map(|s| s.command);
let title = standalone.or(inline).or(command).map(str::to_string);
(title, Some(span.status))
```

页名的回退在 `page_rows` 中处理：`title.unwrap_or(page.name)`，页名为空时用 `text::default_page_name(序号)`，与现有的占位逻辑一致。

> `name_matches` 目前是 `tree/panel.rs:486` 中的私有函数，提升为 `pub(crate)`，或者移到 `app/prompt/tree.rs` 让两处共用。

### 8.2 渲染（`src/ui/prompt/pages.rs` 新文件）

布局与 `ui/prompt/tree.rs::render` 完全对齐：

```text
y0  搜索框（复用 widgets::input 视口与硬件光标，占位文案 `search pages`）
y1  分割线（同 tree::paint_divider）
y2… 行：`❯ ` 或 `  ` + 标题（当前页 `style::strong()`，其余 `style::text()`）…… 状态圆点
```

- 选中行行首画 `❯`（`text::AGENT_SELECTED_MARKER`），悬停行铺 `style::selection()`。这些都复用 tree 的 `fill_row`、`paint_text`。实施时把这两个函数从 `tree.rs` 提成 `pub(super)` 供两处使用。
- 圆点贴在行的最右端（滚动条槽已由 `prompt_text_rect` 扣掉，文本区右缘就是可用的最右列）：
  - `Some(Todo)` → `text::PAGE_STATUS_OPEN = "○"`，`style::muted()`
  - `Some(InProgress)` → `text::PAGE_STATUS_DOT = "●"`，`style::template_status(InProgress)`（黄）
  - `Some(Done)` → `"●"`，`style::template_status(Done)`（绿）
  - `None` → 不画
- 标题的可用宽度是行宽减去 `❯ ` 的 2 列，再减去圆点和 1 列间隔，超出部分用 `…` 截断（`text::truncate_to_width`）。
- 空列表和无命中时显示：`text::PROMPT_PAGES_NO_MATCHES = "no matching pages"`。理论上不会出现空列表，因为每个编辑目标至少有一页。
- 滚动条：`prompt_frame::prompt_scrollbar_for` 增加 `PromptMode::Pages` 分支，复用 `prompt_tree_scrollbar_rect`，`total` 为过滤后的行数，`offset` 为 `pages_view.scroll`。

命中：

```rust
/// 页列表可见行命中（文本区坐标）；未命中或命中搜索框/分割线返回 None。
pub fn page_row_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> Option<usize>
```

### 8.3 头部（`ui/prompt.rs::render_toolbar`、`header.rs`、`prompt_frame.rs`）

- `render_toolbar` 的 `tree: bool` 参数改为 `mode: PromptMode`。Pages 视图的按钮集合是 `[/]` 加 `Pin`，`Pin` 的位置与 Tree 视图一致，但不显示 `[tmp]`。`layout::prompt_toolbar_button_rect` 的 `tree: bool` 同步改为 `mode`；`Pin` 在 Pages 视图下贴右侧（`bar.right() - 1 - PIN_WIDTH`）。
- `prompt_frame::title_source` 增加分支 `PromptMode::Pages => TitleSource::Pages { title, page }`。
- 标题行：左侧是 `条目名`，后面跟灰字 ` · pages`（`text::PROMPT_PAGES_LABEL = "pages"`），右侧是**只有计数**的 `n/m`，没有 `‹ ›`。`n` 为选中行在**未过滤**列表中的序号（从 1 起），`m` 为总页数。
  - `header::page_controls` 增加 `arrows: bool` 参数。Pages 视图传 `false`，只排布 counter；这时 `prev`/`next` 的 rect 宽度为 0，命中自然失效。
- 点击计数：Editor 视图 → `show_prompt_pages`；Pages 视图 → `hide_prompt_pages`（`event/mouse/press.rs:420` 处改写）。

### 8.4 删除行内重命名（决策 11）

| 位置 | 删除 |
|---|---|
| `app/prompt/tree/panel.rs` | `PageRename`、`page_rename` 字段及其 6 个方法 |
| `app/update/prompt/pages.rs` | `begin_page_rename`、`page_rename_active`、`apply_page_rename_key`、`cancel_page_rename`、`finish_page_rename`；`open_new_page` / `open_created_page` 中的 `start_page_rename`（新建页后**不再**自动进入命名） |
| `ui/prompt/header.rs` | `PageHeaderData.rename`、`rename_input_span`、`paint_rename_input`、`rename_cursor`；`PageControl::Name`（页名变成纯展示） |
| `ui/prompt_frame.rs` | `PromptPageHeader.rename` 与重命名硬件光标分支 |
| `input/mod.rs` | `page_rename_key` |
| `event/input.rs`、`event/mouse/press.rs:398` | 重命名路由与点击提交分支 |
| `app/update/{focus,right_panel,draft,update}.rs` | 调用 `finish/cancel_page_rename` 的地方，逐个删除 |

> 页名仍然在标题行展示（`条目名 · 页名`），只是不能点击编辑。改名统一用 Pages 视图右键 → Rename。

### 8.5 删除 `PageMenu` 浮层

| 位置 | 删除 |
|---|---|
| `ui/overlay/page_menu.rs` | 整个文件 |
| `ui/overlay.rs` | `mod page_menu` 与相关 re-export、`Overlay::PageMenu` 渲染分支 |
| `app/overlay.rs` | `OverlayKind::PageMenu`、`Overlay::PageMenu`、`PageMenu`、`PageMenuRow` |
| `app/update/prompt/pages.rs` | `activate_page_menu_row`、`move_page_menu_selection`、`set_page_menu_selection`、`open_prompt_page_menu` |
| `app/update/dialog.rs:180`、`event/overlay.rs`、`event/mouse/hover.rs` | 浮层的键盘和鼠标分支 |
| `input/mod.rs:294` | `OverlayKind::Menu | OverlayKind::PageMenu` 改为只有 `Menu` |
| `ui/tooltip.rs:45/87`、`app/tooltip.rs`、`ui/text.rs:1007` | `PageMenuAdd/Delete` tooltip、`MENU_PAGE_NEW/DELETE`、`MENU_TITLE_PAGES` |
| `ui/mod.rs:105` | 浮层判定 |
| `docs/standards/tui-design-requirements.md:29/35` | 把「页码菜单」从浮层清单和确认浮层示例中移除 |

---

## 9. 页右键菜单（P4，决策 11 / 16）

### 9.1 命令与目标（`src/app/overlay.rs`）

```rust
pub enum MenuCommand {
    // …现有…
    /// 页列表：打开该页（切页并回编辑视图）。
    OpenPage,
    /// 页列表：重命名该页（`Overlay::Rename`）。
    RenamePage,
    /// 页列表：复制该页全文。
    CopyPageContent,
    /// 页列表：末尾新建页并打开。
    NewPage,
    /// 页列表：删除该页（行内二次确认；仅剩一页时置灰）。
    DeletePage,
}

pub enum OverlayTarget {
    // …现有 PromptPage { workspace, owner, page } 复用为页行菜单目标…
    /// 页列表空白区域。
    PromptPages { workspace: usize, owner: PromptEditing },
}

pub enum RenameTarget {
    // …现有…
    PromptPage { workspace: usize, owner: PromptEditing, page: PromptPageId },
}
```

- `MenuCommand::requires_confirm` 中加入 `DeletePage`。
- **置灰**：`Menu` 目前没有 disabled 能力，需要用 grep 确认。推荐的做法是：只剩一页时 `open_page_menu` **不放入** `DeletePage`。这样不必给菜单新增 disabled 状态，与 `open_prompt_folder_menu` 按条件 push `NewSubFolder` 的做法一致。
  > 这与决策 11 中「置灰」的字面意思有偏差，但效果相同：用户都无法在只剩一页时删除。不新增菜单 disabled 能力，可以避免为一个菜单项改动菜单浮层的渲染和命中。**审核时请确认。**

### 9.2 打开（`app/update/prompt/pages_view.rs`）

```rust
/// 页行右键菜单：打开、重命名、复制、新建、删除（仅多页时）；不改动选中行。
pub fn open_page_menu(state: &mut AppState, page: PromptPageId, anchor: (u16, u16))

/// 页列表空白区域菜单：新建页。
pub fn open_pages_blank_menu(state: &mut AppState, anchor: (u16, u16))
```

`event/overlay.rs:495`（右键分发）增加 Pages 分支：

```rust
if update::prompt_panel_active(state) && update::prompt_pages_mode(state) {
    let area = paste::prompt_text_area(view);
    if area.contains((column, row).into()) {
        match ui::prompt::pages::page_row_at(area, &state.prompt_owner().prompt_panel, column, row)
            .and_then(|index| update::page_id_at_row(state, index))
        {
            Some(page) => update::open_page_menu(state, page, (column, row)),
            None => update::open_pages_blank_menu(state, (column, row)),
        }
        return true;
    }
}
```

### 9.3 执行（`app/update/menu.rs` 的命令分派）

| 命令 | 目标 | 行为 |
|---|---|---|
| `OpenPage` | `PromptPage { page, .. }` | `hide_prompt_pages` 后调用 `select_prompt_page(page)` |
| `RenamePage` | `PromptPage` | `Overlay::Rename { target: RenameTarget::PromptPage, input: 页名 }` |
| `CopyPageContent` | `PromptPage` | 取该页 `content`。如果是当前页，取 `state.prompt.text()`。进入 Pages 前已经 commit，两者相同，取页 content 即可。然后调用 `copy_with_toast`，toast 锚定 prompt 面板 |
| `NewPage` | `PromptPage` / `PromptPages` | 先 `hide_prompt_pages`，再走现有的 `open_new_page`。正式条目走 `Effect::CreatePage`，成功回调 `open_created_page` 切到新页；临时提示词直接追加。两条路径都**去掉 `start_page_rename`** |
| `DeletePage` | `PromptPage` | 行内确认后调用 `perform_delete_page(workspace, owner, page)`；**留在 Pages 视图**，`pages_view.selected` 钳到 `min(原行, 行数-1)` |

`commit_prompt_rename`（`prompt.rs` 约 776 行）增加 `RenameTarget::PromptPage` 分支：内容就是现在 `finish_page_rename` 中的 `with_panel(... page.name = name)`，再加上 `Effect::RenamePage`，trim 后为空则拒绝。这样可以复用浮层原有的「空名拒绝、浮层保持打开」语义。

`perform_delete_page` 的改动：

- 删除 `panel.page_rename()` 相关的清理（行内重命名已删除）。
- `reload_editor_from_active_page` 的条件放宽到 `mode != Tree`。被删的是当前页时，编辑缓冲会重载为新的当前页；被删的是非当前页时，`active_page` 不变，重载等价于用同一份正文 `set_text`。**要避免这次无谓的重载把撤销栈冲掉**：仅当 `page == 原 active` 时才调用 `reload_editor_from_active_page`。

删除旧流程：`delete_current_page`、`confirm_delete_page`、`ConfirmClose { target: OverlayTarget::PromptPage }` 的分支（`dialog.rs` 中的确认回调）。删除确认统一为菜单行内确认。

### 9.4 文案（`src/ui/text.rs`，英文）

```rust
pub const MENU_PAGE_OPEN: &str = "Open";
pub const MENU_PAGE_RENAME: &str = "Rename";      // 可直接复用 MENU_PROMPT_RENAME
pub const MENU_PAGE_COPY: &str = "Copy content";  // 可直接复用 MENU_PROMPT_COPY
pub const MENU_PAGE_NEW: &str = "New page";
pub const MENU_PAGE_DELETE: &str = "Delete";      // 可直接复用 MENU_PROMPT_DELETE
pub const PROMPT_PAGES_LABEL: &str = "pages";
pub const PROMPT_PAGES_SEARCH_PLACEHOLDER: &str = "search pages";
pub const PROMPT_PAGES_NO_MATCHES: &str = "no matching pages";
pub const PAGE_STATUS_OPEN: &str = "○";
pub const PAGE_STATUS_DOT: &str = "●";
```

能复用的文案就直接复用，不新增同值常量。`ui/overlay/menu.rs` 的命令到文案映射增加以上分支，`DeletePage` 的确认态文案复用现有删除确认文案。

### 9.5 鼠标（`event/mouse/press.rs`、`drag.rs`、`hover.rs`、`wheel.rs`）

- **按下**：Pages 视图在 `prompt_tree_mode` 分支旁边新增 `prompt_pages_mode` 分支，命中行就调用 `select_page_row`，并登记点击时刻。页列表**不支持拖动**，所以不走 `begin_prompt_drag`。
- **双击**：在按下时判定（Pages 没有拖拽，不必像 Tree 那样推迟到松开）。使用 `page_row_double_click(state, row, now)`，判定为真时调用 `open_selected_page`。点击记录放在一个新的 `AppState.page_row_click: Option<(usize, Instant)>`，**不与** `prompt_row_click` 共用，避免 Tree 和 Pages 之间切换时误判成双击。
- **悬停**：`hover.rs:308` 增加 Pages 分支，写入 `state.prompt_pages_hover`。这是一个新字段，与 `prompt_tree_hover` 对称。也可以复用 `prompt_tree_hover`，因为两者互斥。**推荐复用**，切换模式时清空即可，这样少一个字段。
- **滚轮**：`wheel.rs:385` 增加 Pages 分支，`scroll_page_list(direction * WHEEL_LINES, visible)`。

---

## 10. settings（决策 19）

- `assets/settings.js:512`：`{value: 'template', label: 'Template only'}` 改为 `{value: 'template', label: 'Inside blocks (&&& / +++)'}`。
- `src/settings/` 的 Rust 侧**不改**：`BlockType`、`MdScope`、frontmatter、`RESERVED_NAMES`、handlers 都不受影响。
- `slash.rs` 中 `SlashScope::Template` 的判断（`BlockContext::Template | BlockContext::Supple`）不改，平级 `+++` 自然被覆盖。
- 按 `docs/standards/html-assets.md` 的要求，资源经 `include_str!` 编译进二进制，不需要其他改动。

---

## 11. 文件拆分与行数

| 文件 | 现有行数 | 本次净变化（估计） | 处理 |
|---|---|---|---|
| `app/prompt.rs` | 966 | 删除折叠/聚焦字段和调用，约 −60 | 不拆 |
| `app/update/prompt.rs` | 959 | 删除聚焦，`commit_prompt_rename` 加分支，约 ±0 | 不拆，新逻辑放 `pages_view.rs` |
| `app/update/prompt/pages.rs` | 714 | 删除菜单和行内重命名，约 −250 | 不拆 |
| `app/update/prompt/pages_view.rs` | 新 | 约 +250 | 新文件 |
| `app/prompt/pages_list.rs` | 新 | 约 +120 | 新文件（纯函数） |
| `ui/prompt.rs` | 796 | 删除边框/折叠，约 −150 | 不拆 |
| `ui/prompt/pages.rs` | 新 | 约 +200 | 新文件 |
| `app/prompt/fold.rs`、`block_focus.rs`、`ui/overlay/page_menu.rs` | 211 / 201 / 291 | 全删 | — |

测试：

- 新增 `app/prompt/pages_list/tests.rs`、`ui/prompt/pages/tests.rs`、`app/update/prompt/pages_view/tests.rs`。
- `app/prompt/tests.rs`（3162 行）、`ui/prompt/tests.rs`（1896 行）**只删除和修改**本次受影响的用例，**不新增**用例到这两个文件。新用例放进对应的子模块测试文件。这两个文件存量超限的问题记为后续任务（task.md §9）。
- `app/markdown/tests.rs`（1545 行）：块扫描的新用例放进新建的 `app/markdown/blocks/tests.rs`，并用 `#[cfg(test)] mod tests;` 挂到 `blocks.rs`。

---

## 12. 风险与回退

| 风险 | 影响 | 缓解 |
|---|---|---|
| 旧数据中嵌套在 `&&&` 内的 `+++` 降级为普通文本 | 这些页显示为 `&&&` 正文，`+++` 的按钮消失 | 决策 1 已经接受。`&&&` 的 `[copy]` 现在会把这段文字一起复制进去（以前会跳过）。审核时请确认这个副作用可以接受 |
| 一页有多个 `&&&` 的旧数据 | 第二个及之后的 `&&&` 变成普通文本，页列表只统计第一个 | 决策 2 已经接受。Tree 视图的 `n|m|x` 跨页聚合会随之变少（`template_counts` 只数第一个） |
| 三套扫描器统一后，lx 输入框的高亮行为变化 | lx 输入框中嵌套的 `+++` 不再高亮 | 决策 20 已经接受 |
| `Ctrl+←/→` 不再按词移动 | 习惯用 Ctrl 按词跳的用户会意外切页 | 决策 7 已经接受。`Alt+←/→` 可以替代 |
| macOS 系统或终端拦截 `Ctrl/Cmd+←/→`、`Cmd+L/P` | 快捷键收不到 | 鼠标入口（`‹ ›`、计数、`[≡]`）都保留 |
| 删除非当前页时编辑缓冲被重载 | 撤销栈丢失 | §9.3：只有删除当前页时才重载 |
| 边框染色覆盖底边控件 | 底边文字变色 | §6.3：只给边框字形染色 |

回退：这次没有 DB 变更，回退就是不合并分支。各阶段的 commit 相互独立，可以按阶段 revert。

---

## 13. 实施差异

- `PromptMode` 枚举定义在 `src/layout/prompt.rs` 并由 `src/app/prompt/tree/panel.rs` 重新导出，同时实现 `From<bool>`（`true => Tree, false => Editor`），从而在不违反 `layout -> ratatui/std` 依赖方向的前提下让 `prompt_toolbar_button_rect` / `prompt_toolbar_button_at` 直接按 `PromptMode` 计算 `Editor / Tree / Pages` 三态工具栏布局。
- `edit_touches_title_line`（`src/app/markdown/template.rs`）保留原有对所有 `parse_template_start_line` 下一行 `{title:}` 的结构保护判定，与单 `&&&` 块扫描解耦。
- `right_panel::tint_edit_border` 在对分割线以下外框染色时，跳过主区与右栏之间的 `<>` 交换按钮所在单元格，保证交换按钮字形与样式不被覆盖。
