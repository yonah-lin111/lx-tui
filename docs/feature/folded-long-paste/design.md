# 长文本折叠粘贴（prompt 编辑器与 lx 输入框）：设计方案

> 状态：**已在工作区 `.worktrees/folded-long-paste`（分支 `feat/folded-long-paste`）完成实现并通过验证，待用户确认合并**
>
> 关联文档：
> - 任务清单：`docs/feature/folded-long-paste/task.md`
> - 项目规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`

---

## 1. 背景与目标

### 1.1 现状

1. `src/app/paste.rs` 与 `src/app/prompt/paste.rs` 已有“路径粘贴 → 占位文本 → `Insert reference` / `Insert path` 面板”的完整链路，但它只处理**存在的文件/目录/图片路径**，不处理普通长文本。
2. `Prompt`（右栏提示词编辑器）与 `Lx`（lx 输入框）的正文都是纯 `String`，光标是字节偏移；没有富文本、extmark 或折叠区模型。
3. 终端粘贴长文本时，目前直接把整段文本塞进输入框：几十行日志会把输入框撑高，挤压历史区，且难以看出“这一大段是粘贴进来的”。

### 1.2 目标

1. 两个主输入框（prompt 编辑器正文、lx Chat 输入框）粘贴长文本时弹出命令面板，提供：
   - `Collapse paste`（默认）：插入自包含折叠 token；
   - `Paste full text`：按原文插入。
2. 折叠 token 必须是**可持久化的纯文本格式**，不依赖数据库新表、新列或旁路状态；保存、重启、撤销/重做、复制粘贴都不能丢正文。
3. 折叠态显示 `@[pasted:N](首行预览…)`；点击展开后直接显示原文；点击展开的原文（未拖动）收起；展开与折叠共用高亮背景。
4. 展开态支持拖拽选中复制，交互对齐 lx agent 工具卡片：未拖动 = 点击折叠，发生拖动 = 保留选区。
5. 光标把折叠块视为原子：左右移动整块跳过，Backspace/Delete 整块删除，不能把光标插进 token 中间。
6. 发给模型时一律解码成原文；模型永远看不到 `@[pasted:...]` 包装。

### 1.3 非目标

- 不做图片/文件粘贴的第二种表达方式；路径粘贴继续走现有 `Reference` / `Path` 面板，优先级高于长文本。
- 不新增数据库 schema、迁移或配置项。
- 不支持在围栏代码块（```）内部折叠。
- 不支持 lx Shell 模式、ask 手写答案框、prompt 树/页列表搜索框。
- 不在展开预览里执行 Markdown 语法高亮；预览是只读纯文本 + 高亮底色。
- 不持久化“展开/折叠”状态；重新打开页面一律折叠。
- 不为超长粘贴做分页/虚拟化；超过 256 KiB 只允许原文粘贴。

---

## 2. 格式规范（持久化的核心）

### 2.1 语法

```text
@[pasted:<N>](<正文>)
```

- `N`：原文的逻辑行数，十进制整数，最小 1。
- `正文`：原文，按下面的规则转义后原样放入括号内；允许换行。
- 结束符是**第一个未转义的 `)`**。
- 转义规则（只转义两个字符）：
  - `\` → `\\`
  - `)` → `\)`
- 其他字符（包括换行、`(`, `]`, `@`, `[pasted:`）原样保留。

### 2.2 编解码

```rust
/// 原文 → token 正文（只转义 `\` 与 `)`）。
pub fn escape_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ')' => out.push_str("\\)"),
            _ => out.push(ch),
        }
    }
    out
}

/// token 正文 → 原文。
pub fn unescape_body(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some(')') => out.push(')'),
                Some(other) => {
                    // 非本格式的转义原样保留，避免吞掉用户正文。
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// 原文 → 完整 token。
pub fn encode(content: &str) -> String {
    let line_count = content.split('\n').count().max(1);
    format!("@[pasted:{line_count}]({})", escape_body(content))
}
```

示例：

```text
原文：
foo(bar)
baz

token：
@[pasted:2](foo\(bar\)
baz)

折叠显示：
@[pasted:2](foo(bar)…)

展开显示：
foo(bar)
baz
```

### 2.3 解析

扫描规则：

1. 逐行扫描全文，维护围栏状态；**行首围栏标记只在 token 之外的文本上生效**。
2. 遇到 `@[pasted:` 时尝试解析：
   - 解析 `N`（至少一位数字）→ 必须紧跟 `](`；
   - 从正文起点向右找**第一个未转义的 `)`**；`\)` 与 `\\` 都跳过；
   - 成功则登记 `FoldedToken { range, line_count, body(解码后), preview }`，从结束 `)` 之后继续；
   - 失败（缺 `)`、`N` 非法、括号不完整、正文为空）则不登记，按普通文本继续扫描。
3. **围栏代码块内不解析**：扫描器自己维护 `fence: Option<(u8, usize)>`，只有 token 外的行首才能开/关围栏；在围栏内遇到 `@[pasted:` 直接跳过。
4. token 的正文内部不参与围栏状态切换：整段 token 是一个不透明区间，解析时直接跳过，正文里的 ``` 不会关闭外层围栏。
5. 解析失败的部分继续按普通字符扫描，不能让一个坏 token 阻断后续 token。

### 2.4 折叠态显示

`FoldedToken::label()`：

```text
@[pasted:<N>](<preview>)
```

- `preview` 取**第一条非空行**，去掉行尾空白，按显示宽度截断到 40 列；截断时追加 `…`。
- 如果没有非空行，`preview = "<N> lines"`。
- `preview` 是解码后的文本，因此 `)`、`\` 按原文显示；它只用于渲染，不参与重新解析。
- 折叠态显示的 label 与底层 token 不是同一串字节；底层 token 始终是完整转义正文。

### 2.5 身份与重复 token

- token 的展开状态不按字节偏移保存，而按 `(内容哈希, 同名出现序号)` 保存。
- 同一段文本被复制成两个 token 时，两个 token 的哈希相同、出现序号不同，可以独立展开/折叠。
- 文本编辑导致 token 前的内容增删时，哈希与序号不变，展开状态保持。
- token 被删除后，对应状态成为孤儿；下一次投影时自然忽略，不需要额外清理。
- 撤销/重做、载入提示词页、`set_text`、`import_full_text`、清空输入时，显式清空全部展开状态。

---

## 3. 交互规范

### 3.1 触发条件

普通文本粘贴（非路径）满足以下条件时弹面板：

| 条件 | 行为 |
| :--- | :--- |
| `行数 >= 3` 或 `字符数 > 150` | 弹长文本面板 |
| 不满足 | 按原文插入（现有行为） |
| 原文（trim 后）> 256 KiB | 弹面板，但 `Collapse paste` 置灰，默认高亮 `Paste full text` |
| 光标在围栏代码块内 | 不弹面板，原文插入 |
| 路径/文件/图片/文件夹粘贴 | 走现有路径面板，优先于长文本 |
| lx Shell 模式 / ask 编辑器 / prompt 搜索框 | 不弹面板，原文插入 |

- 行数按 `trim()` 后的 `split('\n').count()` 计算。
- 字符数按 `trim()` 后的 Unicode 标量数（`chars().count()`）计算；中文按 1 个字符计，不按 UTF-8 字节计。
- 粘贴文本先做换行归一：`\r\n`、`\r` → `\n`，再走现有 `prompt::text::sanitize`（Tab 展开、控制字符丢弃）；折叠 token 的正文与 `Paste full text` 都基于 sanitize 后的文本，保证 `insert_str` 的二次 sanitize 幂等。暂不剥离 ANSI 转义（终端粘贴一般已是可见文本；如后续需要，可复用 `chat::process::rtk::builtin::strip_ansi` 的纯函数能力，但不在本次范围）。
- 面板显示期间不改缓冲区；确认时才插入。因此 Esc/点击外部取消不需要回滚，也不会多出撤销步。

### 3.2 面板

- 标题：`Paste`
- 条目（固定顺序）：
  1. `Collapse paste`，预览列显示 `N lines`；默认高亮；
  2. `Paste full text`，预览列显示 `N lines`。
- 超限时 `Collapse paste` 灰色不可选，预览列显示 `too large to collapse`，默认高亮第二项。
- 底部提示沿用 `↑↓ select · ↵ confirm · esc cancel`。
- 键盘：`↑`/`↓` 切换（跳过禁用项），`Enter` 确认，`Esc` 取消。
- 鼠标：悬停高亮可用项，点击确认；点击面板外取消。
- 面板打开时其他编辑键被吞掉（与现有路径面板一致），避免锚点失效。

### 3.3 折叠态

- 在输入框内显示 label：`@[pasted:N](首行预览…)`。
- 样式：`style::pasted_chip()`（见 §6.1）；整段一个原子芯片。
- 点击芯片（左键、未拖动）→ 展开。
- 光标左右移动跳过整块；`Backspace`/`Delete` 在边界处整块删除；拖拽选区经过时吸附到整块边界。
- 芯片可以出现在行中间：`请看日志 @[pasted:6](…) 然后分析`。

### 3.4 展开态

- 芯片的 label 和 `@[pasted:N](...)` 包装全部消失，直接显示解码后的原文。
- 原文按输入框宽度软换行；整体向右缩进 2 列（渲染缩进，不写进缓冲区）。
- 整块铺 `style::pasted_bg()` 背景；文字使用正常前景色（不套 Markdown token 样式）。
- 多个芯片可以同时展开；输入框高度随内容增长，滚轮照常滚动输入框。
- 点击展开区任意单元格（未拖动）→ 收起该芯片。
- 拖拽选中：按下开始选区，拖动延伸；松开时若发生过拖动，保留选区供 `Ctrl/Cmd+C` 复制；若未拖动，视为单击并收起。
- 双击选词、三击选行沿用 `SelectionUnit` 规则，选区限定在展开区显示文本内。
- `Esc`：先清空展开区选区；没有选区时收起全部展开芯片；再没有展开芯片时走原有 Esc 语义。
- 展开/折叠不写缓冲区、不增加 revision、不触发自动保存、不进撤销栈。

### 3.5 复制语义

| 场景 | 复制内容 |
| :--- | :--- |
| 折叠芯片被选中（整块或部分） | 底层 token 原文（含 `@[pasted:N](...)` 包装），粘到另一个输入框仍可折叠 |
| 展开区拖拽选中 | 选中的**显示文本**（解码后的原文片段，不含缩进与包装） |
| 无选区 `Ctrl/Cmd+C` | 整个缓冲区原文，token 原样保留 |
| `Ctrl/Cmd+A` 后复制 | 整个缓冲区原文，token 原样保留（不展开为预览文本） |
| 提示词模板块复制按钮 | 块内 token 原样保留 |

- 展开区选区与普通文本选区是两套坐标：普通文本选区按原始字节，展开区选区按投影文本字节。一次拖拽不会同时跨越两者；从普通文本开始拖拽不进入展开区，从展开区开始拖拽不进入普通文本。
- 这一限制在实现中由“按下点落在哪个区域”决定，避免维护两套坐标的混合映射。

### 3.6 提交/保存

- lx 发送给模型前：`folded::expand_all(input)`，把所有 token 解码成原文；模型看不到 token。
- 排队消息、标题生成、prompt 历史记录同样使用展开后的文本。
- prompt 提示词页保存/自动保存：保存**原始 token 文本**（自包含，含完整正文），不展开；重新打开仍是折叠芯片。
- 提示词页重新打开：全部折叠；展开状态不落库。
- 复制到外部编辑器：token 是可读的 `@[pasted:N](正文)`，不会丢内容。

---

## 4. 数据与状态

### 4.1 新增模块

```text
src/app/paste/folded.rs        token 编解码、扫描、投影、原子编辑辅助
src/app/paste/folded/tests.rs
src/app/paste/text.rs          长文本粘贴面板状态机
src/app/paste/text/tests.rs
src/app/prompt/folded.rs       Prompt 的粘贴面板/展开状态/投影/原子编辑方法
src/app/prompt/folded/tests.rs
src/app/state/lx/folded.rs     Lx 的粘贴面板/展开状态/投影/原子编辑方法
src/app/state/lx/folded/tests.rs
```

`src/app/paste.rs` 增加：

```rust
mod folded;
mod text;

pub use folded::{
    ExpandedPastes, FoldedToken, Projection, encode as encode_pasted_text,
    expand_all as expand_pasted_text, mask as mask_pasted_text, tokens as folded_tokens,
};
pub use text::{TextPasteAction, TextPastePanel, TextPasteState};
```

### 4.2 核心类型

```rust
/// 一个已解析的折叠 token（字节区间指向原始缓冲区）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedToken {
    pub range: Range<usize>,
    pub line_count: usize,
    pub body: String,     // 已反转义
    pub preview: String,  // 折叠态首行预览（已截断）
    pub id: u64,          // body + line_count 的稳定哈希
}

/// 展开状态：按 (id, 同名出现序号) 记录，不按字节偏移。
#[derive(Debug, Default, Clone)]
pub struct ExpandedPastes {
    entries: Vec<(u64, usize)>,
}

/// 投影后的显示文本 + 显示区间到原始区间的映射。
pub struct Projection<'a> {
    text: Cow<'a, str>,
    segments: Vec<Segment>,
    tokens: Vec<FoldedToken>,
}

pub struct Segment {
    pub display: Range<usize>,
    pub raw: Range<usize>,
    pub kind: SegmentKind,
}

pub enum SegmentKind {
    Plain,
    Chip { token: usize, expanded: bool, occurrence: usize },
}
```

### 4.3 投影规则

```text
普通文本        → 原样进入显示文本；Segment::Plain（display 与 raw 等长）
折叠芯片        → 显示 label；Segment::Chip { expanded: false }
展开芯片        → 显示解码正文；Segment::Chip { expanded: true }
```

- 没有 token 时 `Projection::text` 直接借用原字符串（`Cow::Borrowed`），不产生额外分配。
- 有 token 时按段拼接；普通段可以按连续区间合并，不需要逐字符。
- `Projection` 每次渲染/编辑命令时重新计算；token 数很少，成本可接受。
- 显示文本的行结构、软换行、光标列全部基于 `Projection::text`。

### 4.4 坐标转换

```rust
impl Projection<'_> {
    /// 原始字节偏移 → 显示字节偏移；落在 token 内部时夹到芯片起点。
    pub fn raw_to_display(&self, raw: usize) -> usize;

    /// 显示字节偏移 → 原始字节偏移；落在芯片内部时返回芯片起点（点击由 hit 处理）。
    pub fn display_to_raw(&self, display: usize) -> usize;

    /// 显示区间 → 原始区间；整体覆盖 token 时扩展到 token 边界。
    pub fn display_range_to_raw(&self, range: Range<usize>) -> Range<usize>;

    /// 显示偏移命中哪个芯片（折叠或展开）。
    pub fn chip_at(&self, display: usize) -> Option<ChipHit>;

    /// 显示区间 → 所见即所得的复制文本（折叠芯片整块复制 token）。
    pub fn copy_display_range(&self, range: Range<usize>) -> String;
}
```

**所有编辑器几何/选区函数的坐标口径必须一次改清，禁止混用：**

| 函数族 | 坐标 | 说明 |
| :--- | :--- | :--- |
| `visual_rows` / `VisualRow.start/end` / `LxRow.start/end` | 显示 | 软换行、渲染、鼠标单元格都用它 |
| `cursor_cell` / `cursor_visual` | 原始入、显示出 | 先 `raw_to_display` |
| `set_cursor_from_cell` / `offset_at_cell` | 显示入、原始出 | 先 `display_to_raw`；命中芯片时夹边界 |
| `selection_bounds` / `unit_cell_range` | 显示入、原始出 | 供编辑/删除使用，token 整块吸附 |
| `selection_text` / `copy_text` | 显示入、复制文本出 | 用 `copy_display_range`，折叠芯片整块 token |
| `cell_range_for_byte_span` | 原始入、显示出 | 先 `raw_to_display` 再算单元格 |

- `selection_text_with` 不再逐视觉行切片 `self.text`；改为先求显示区间的字节范围，再 `copy_display_range`。这样软换行不会凭空插入换行，折叠芯片复制 token，展开芯片复制显示原文。
- `selection_bounds_with` 返回原始区间供 `backspace`/`delete` 使用；显示区间覆盖折叠芯片时返回整个 token 区间，保证原子删除。

### 4.5 原子编辑辅助

```rust
/// 把任意区间吸附到 token 边界：部分覆盖 token 时扩展为整块。
pub fn snap_range(text: &str, range: Range<usize>) -> Range<usize>;

/// 光标向左/右跨过 token：返回上一个/下一个合法边界。
pub fn prev_boundary(text: &str, offset: usize) -> usize;
pub fn next_boundary(text: &str, offset: usize) -> usize;

/// 把 token 区间替换为等长空白（保留换行），供 Markdown/触发词扫描使用。
pub fn mask(text: &str) -> Cow<'_, str>;
```

- `mask` 保留字节长度与换行位置，因此原始偏移、行号、`open_fence` 判定全部不变。
- token 正文里的 `&&&`、`+++`、```、`@`、`/` 在 mask 后变成空格，不会污染块扫描、提及触发、斜杠命令、围栏判定。

---

## 5. 编辑器集成

### 5.1 Prompt（右栏编辑器）

新增字段：

```rust
pub struct Prompt {
    // ...现有字段
    /// 长文本粘贴面板；与路径粘贴面板互斥。
    text_paste: TextPasteState,
    /// 已展开的折叠 token（按身份，不按偏移）。
    expanded_pastes: ExpandedPastes,
    /// 展开区选区（投影文本字节范围）；普通正文选区仍用原 selection。
    preview_selection: Option<(usize, usize)>,
}
```

关键方法：

```rust
impl Prompt {
    /// 当前长文本粘贴面板。
    pub fn text_paste_panel(&self) -> Option<&TextPastePanel>;

    /// 发起长文本粘贴提案；不修改缓冲区。
    pub fn begin_text_paste(&mut self, content: String, full_text: String, anchor: PasteAnchor) -> bool;

    /// 键盘移动高亮；返回是否消费。
    pub fn text_paste_move(&mut self, delta: isize) -> bool;

    /// 确认：Collapse 插入 token，Full 插入原文；一步撤销。
    pub fn text_paste_confirm(&mut self, action: TextPasteAction) -> bool;

    /// 取消：关闭面板，不动缓冲区。
    pub fn text_paste_cancel(&mut self) -> bool;

    /// 投影文本（渲染/光标/命中用）。
    pub fn projection(&self) -> Projection<'_>;

    /// 切换指定 token 的展开状态；不写缓冲区、不记撤销。
    pub fn toggle_paste(&mut self, id: u64, occurrence: usize) -> bool;

    /// 清空展开状态（undo/redo/set_text/history 恢复时调用）。
    pub fn clear_expanded_pastes(&mut self);

    /// 展开区选区（投影文本字节范围）。
    pub fn preview_selection(&self) -> Option<(usize, usize)>;
    pub fn set_preview_selection(&mut self, range: Option<(usize, usize)>);
}
```

渲染改动：

- `visual_rows()`：用 `folded::tokens()` + `Projection`；行的 `start/end` 改为**投影文本字节偏移**，`line` 仍为原始逻辑行号（用于块信息）。
- `cursor_cell()`：`raw_to_display(cursor)` 后再计算视觉行列。
- `set_cursor_from_cell()`：`display_to_raw()`；若命中芯片，返回芯片边界而不是内部。
- `paint_row()`：按投影显示文本绘制；普通段用原始 Markdown token 样式，芯片段用 `pasted_chip` / `pasted_bg`；展开段不套 Markdown。
- `render_panels()`：长文本面板优先于路径面板/提及/斜杠/块面板（互斥，按状态判断）。
- `markdown::scan()` 与 `block_scan()` 使用 `mask_pasted_text()` 的结果，避免 token 正文污染语法。

### 5.2 Lx（lx 输入框）

新增字段：

```rust
pub struct Lx {
    // ...现有字段
    text_paste: Option<LxTextPaste>,
    expanded_pastes: ExpandedPastes,
    preview_selection: Option<(usize, usize)>,
}

struct LxTextPaste {
    panel: TextPastePanel,
}
```

关键方法：

```rust
impl Lx {
    pub fn text_paste_panel(&self) -> Option<&TextPastePanel>;
    pub fn text_paste_move(&mut self, delta: isize) -> bool;
    pub fn text_paste_confirm(&mut self, action: TextPasteAction) -> bool;
    pub fn text_paste_cancel(&mut self) -> bool;

    pub fn projection(&self) -> Projection<'_>;
    pub fn toggle_paste(&mut self, id: u64, occurrence: usize) -> bool;
    pub fn clear_expanded_pastes(&mut self);

    /// 发送/排队/标题/历史用的展开文本。
    pub fn expanded_input(&self) -> String;

    /// 输入框单元格命中的折叠块（供鼠标按下判断）。
    pub fn paste_hit_at(&self, row: usize, col: u16) -> Option<folded::ChipHit>;

    /// 展开区选区。
    pub fn preview_selection(&self) -> Option<(usize, usize)>;
    pub fn set_preview_selection(&mut self, range: Option<(usize, usize)>);
}
```

编辑改动：

- `insert_str`、`insert_char`、`newline`、`backspace`、`delete`、`delete_to_line_start/end`、`delete_word_backward/forward`、`replace_range` 全部先 `snap_range` 到 token 边界。
- `move_left`/`move_right` 使用 `prev_boundary`/`next_boundary`。
- `move_up`/`move_down` 目标落在展开芯片内部时，跳到芯片起点/终点外侧。
- `home`/`end` 结果夹到 token 边界。
- 任何文本编辑后不主动清空展开状态；token 身份不变则保持展开。
- `undo`/`redo`、`import_full_text`、`clear`、会话切换时清空展开状态与预览选区。

提交改动：

- `try_send`：`let draft = target.lx.expanded_input().trim().to_string();`
- `queue_chat_while_shell_runs`：同样用 `expanded_input()`。
- `user_shell::try_submit_user_shell`：Shell 模式本身不生成 token，仍建议统一走 `expanded_input()` 防御。
- `prompt_history` 记录、标题生成、`QueuedSubmission::Chat` 的 draft 都用展开文本。
- `serialized_input()` 保持原样（供 shell 草稿/历史），新增 `expanded_input()` 专门给发送侧。

### 5.3 粘贴入口

`src/event/paste.rs`：

```rust
pub(super) fn handle_terminal_paste(state: &mut AppState, text: String) {
    state.prompt.text_paste_cancel();
    state.prompt.paste_cancel();
    // 树/页列表：搜索词语义，原样插入
    // 空文本：忽略
    // 1. 路径检测（现有逻辑）
    // 2. 长文本候选：
    if let Some(candidate) = text_paste_candidate(&text) {
        // 围栏内 / 受保护行：原样插入
        state.prompt.begin_text_paste(candidate.content, candidate.full_text, anchor);
        return;
    }
    // 3. 普通文本
    update::apply_pasted_text(state, text);
}

pub(super) fn handle_lx_paste(state: &mut AppState, text: String) -> bool {
    // 现有：空文本/非 lx/executing/shell/路径检测
    // 长文本候选：pane.lx.begin_text_paste(...)
    // 否则 insert_str
}
```

注意：长文本面板不先插入占位，因此 `begin_text_paste` 失败（受保护/围栏）时直接回退原文插入，不产生半截状态。

---

## 6. UI 渲染

### 6.1 样式

`src/ui/style.rs` 新增：

```rust
/// 折叠粘贴芯片：黄色加粗文字 + 深灰底。
pub fn pasted_chip() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD)
}

/// 折叠粘贴展开区：与芯片同一底色，正常前景。
pub fn pasted_bg() -> Style {
    Style::default().bg(Color::Indexed(236))
}
```

- 不使用 RGB；`Indexed(236)` 与现有 `chat_expanded_bg` 同色系。
- 芯片与展开区共用底色，满足“展开和折叠保持高亮背景”。
- 展开区缩进 2 列用渲染偏移，不写进缓冲区；缩进列也铺底色。

### 6.2 文案

`src/ui/text.rs` 新增：

```rust
pub const TEXT_PASTE_PANEL_TITLE: &str = "Paste";
pub const TEXT_PASTE_ACTION_COLLAPSE: &str = "Collapse paste";
pub const TEXT_PASTE_ACTION_FULL: &str = "Paste full text";
pub const TEXT_PASTE_TOO_LARGE: &str = "too large to collapse";
pub const TEXT_PASTE_LINES: fn(usize) -> String; // "N lines"
```

所有文案英文，集中在 `text.rs`。

### 6.3 Prompt 面板

`src/ui/prompt/panels.rs`：

- 新增 `TextPastePanelData { rows, active, disabled, anchor_row }`。
- `text_paste_view()` 复用 `CommandPanelView`，标题 `Paste`，底栏现有 `COMMAND_PANEL_FOOTER`。
- `render_panels()` 顺序：文本粘贴 → 路径粘贴 → 提及 → 字母快捷 → 斜杠 → 块命令。
- `text_paste_item_at()` / `text_paste_layout()` / `text_paste_panel_rect()`。

### 6.4 Lx 面板

`src/ui/main_content/lx/panels.rs`：

- 新增 `text_paste_panel_rows()`、`text_paste_view()`、`text_paste_layout()`、`text_paste_item_at()`、`render_text_paste_panel()`。
- 视觉与现有路径粘贴面板一致：无边框、贴输入框上沿、`❯` 选择列、标题 `Paste`。
- `render_paste_panel()` 改为按状态互斥渲染文本/路径面板。

### 6.5 芯片与展开区绘制

- 折叠态：按 `Projection` 的 `Chip { expanded: false }` 段绘制 label；字符逐个写入，宽字符后续列标记 `Skip`；整段 `style::pasted_chip()`。
- 展开态：按 `Chip { expanded: true }` 段绘制解码正文；每行行首额外 2 列缩进（同底色）；全部 `style::pasted_bg()`。
- 普通文本：现有 Markdown token 样式；token 正文被 mask，不会串色。
- 展开区选区：按投影文本字节范围换算成视觉行/列，复用反显绘制；与普通选区互斥。

### 6.6 命中

- 鼠标按下顺序（两个输入框一致）：
  1. 命令面板命中；
  2. 折叠芯片命中：折叠 → 展开；展开 → 开始展开区选区；
  3. 普通文本：原有光标/选区逻辑。
- 折叠芯片命中使用 `Projection::chip_at(display_offset)`，不按原始字节猜测。
- 展开区选区拖动：`drag_*` 路由检查 `preview_selection_drag`，只更新投影文本选区，不移动正文光标。

---

## 7. 关键流程

### 7.1 粘贴确认（折叠）

```text
Terminal Paste(text)
  └─ normalize CRLF
      └─ path? ── yes ──> 现有路径面板
      └─ no
          └─ fence? ── yes ──> 原文插入
          └─ no
              └─ long? ── no ──> 原文插入
              └─ yes
                  └─ 打开 TextPastePanel（不改缓冲区）
                      ├─ Esc / 点外部 → 关闭
                      ├─ Enter on Collapse
                      │   └─ token = encode(trimmed)
                      │       └─ replace_at(anchor, token + 空格)  // 一步撤销
                      └─ Enter on Full
                          └─ replace_at(anchor, normalized full text) // 一步撤销
```

### 7.2 渲染

```text
raw text + ExpandedPastes
  └─ folded::tokens(raw)           // 跳过围栏
      └─ Projection::new(raw, expanded)
          ├─ display text          // 折叠 label / 展开正文
          ├─ segments              // display ↔ raw
          └─ visual_rows(display)  // 软换行
              └─ paint_row(display char)
                  ├─ plain  → raw offset → markdown token style
                  └─ chip   → pasted_chip / pasted_bg
```

### 7.3 提交

```text
Lx input (raw token)
  └─ expanded_input()
      └─ folded::expand_all(raw)
          └─ 原文
              └─ try_send / queue / title / history
```

---

## 8. 边界与失败模式

| 场景 | 行为 |
| :--- | :--- |
| token 缺 `)` / `N` 非法 | 不识别，按普通文本显示与提交 |
| 正文含 `)` | 编码为 `\)`，解析正确 |
| 正文含 `\` | 编码为 `\\`，解析正确 |
| 正文含 `@[pasted:` 字面量 | 作为正文内容；外层 token 整体解析，内层不会被单独展开 |
| 正文含 ``` / `&&&` / `+++` | mask 后不影响块扫描；展开时按纯文本显示 |
| token 在围栏内 | 不识别、不折叠；原样显示 |
| token 被部分删除 | 原子编辑禁止；整体删除 |
| 两个相同 token | 按出现序号独立展开 |
| token 被撤销删除 | 撤销/重做清空展开状态，恢复后折叠 |
| 超过 256 KiB | 只允许原文粘贴，折叠项置灰 |
| 粘贴时选区触碰受保护行 | 不弹面板，按原文插入（或拒绝；实现取“原文插入”以不丢内容） |
| 面板打开时切换页面/会话 | 取消面板，不插入 |
| 提示词页重新打开 | token 保留，展开状态清空 |
| 外部编辑器改了 token 正文 | 重新解析；N 仅用于显示，提交以正文为准 |
| 复制折叠芯片 | 复制 token，跨输入框可还原 |
| 复制展开区选区 | 复制解码后的显示文本 |

---

## 9. 性能

- 无 token 时 `Projection` 借用原字符串，无额外分配；`mask` 也返回 `Cow::Borrowed`。
- 有 token 时每帧重建投影；256 KiB 上限下可接受。面板/渲染路径不做 IO。
- 展开状态是小型 `Vec<(u64, usize)>`，不复制正文。
- 展开区不铺 Markdown 语法分析，避免对几百行正文做二次扫描。
- 超过 256 KiB 禁止折叠，避免输入框每帧扫描 MB 级文本。

---

## 10. 测试策略

### 10.1 单元测试

`src/app/paste/folded/tests.rs`：

- 编码/解码：`\`、`)`、换行、CJK、emoji、空行、首尾空白。
- 解析：单个/多个 token、重复 token、嵌套字面量、缺少 `)`、非法 N、围栏内 token、token 正文含 ```。
- 预览：首行截断 40 列、空首行取第一个非空行、全空行显示 `N lines`。
- 投影：display 文本、segments、raw↔display、chip hit、copy range（折叠整块/展开片段）。
- 原子：`snap_range`、`prev_boundary`、`next_boundary`。
- mask：字节长度不变、换行保留、token 内触发词变空格。

`src/app/paste/text/tests.rs`：

- 阈值：2 行/80 字符不弹；3 行弹；151 字符弹。
- 超限：256 KiB + 1 只允许 full。
- 默认高亮、跳过禁用项、Esc/取消不动缓冲区。

### 10.2 Prompt 集成测试

`src/app/prompt/tests/paste_folded.rs`（新增子模块）：

- 折叠确认 → token 文本、光标位置、一步撤销、redo。
- 原文确认 → 原文插入。
- Esc/外部点击 → 缓冲区不变、撤销栈不变。
- 选区替换 → token 替换选区。
- 围栏内粘贴 → 不弹面板。
- 光标左右跨芯片、Backspace/Delete 整块删除。
- 展开/折叠不增加 revision、不进撤销栈。
- 展开区拖拽选区复制；单击收起。
- 保存/重新载入 → token 保留、展开状态清空。
- token 正文含 `&&&`/``` → 块扫描不受影响。

`src/ui/prompt/tests.rs`：

- 芯片 label 渲染、黄色加粗、`Indexed 236` 底。
- 展开区缩进 2 列、底色连续、普通 Markdown 样式不串入。
- 面板三行、禁用项灰色、命中。

### 10.3 Lx 集成测试

`src/app/state/lx/tests/paste_folded.rs`（新增子模块）：

- 同上编辑器行为。
- Shell 模式/`plain()` 不弹面板。
- `expanded_input()` 解码全部 token。
- `try_send`/队列/标题/历史使用展开文本。
- 路径粘贴优先于长文本。
- 多个芯片独立展开；切换会话清空。

`src/ui/main_content/lx/input/tests.rs`：

- 芯片/展开区渲染、缩进、底色。
- 展开区命中、选区反显。
- 面板渲染与命中。

`src/event/tests/focus_and_paste.rs`：

- 端到端粘贴路由、Esc、面板确认、鼠标点击芯片/展开区。

### 10.4 回归

- 现有路径粘贴面板全部测试必须保持通过。
- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

---

## 11. 已确认决策（Grill Me）

| 编号 | 决策 |
| :--- | :--- |
| D1 | 折叠占位是原子芯片，不允许光标进入内部改字 |
| D2 | 展开是只读预览，不修改缓冲区 |
| D3 | 阈值 `>=3 行 || >150 字符`；占位格式 `@[pasted:N](...)` |
| D4 | 面板默认 `Collapse paste`；Esc/点外部取消整次粘贴 |
| D5 | 展开为输入框内联只读预览，输入框临时变高 |
| D6 | 允许多个芯片同时展开 |
| D7 | 持久化采用自包含可读 token，正文直接存括号内，不用 base64 |
| D8 | 只转义 `\` 和 `)`；结束符为第一个未转义 `)` |
| D9 | 折叠态显示 `@[pasted:N](首行预览…)`；展开态显示原文 |
| D10 | 展开区支持拖拽选中复制，未拖动=点击折叠；双击选词/三击选行 |
| D11 | 复制折叠芯片复制 token；复制展开选区复制显示原文；无选区复制全文（token 原样） |
| D12 | 保存/提交：token 落库；提交给模型前解码 |
| D13 | 展开/折叠不算修改、不进撤销、不触发自动保存 |
| D14 | 保存后重开全部折叠，展开状态不持久化 |
| D15 | 视觉：共用 `Indexed 236` 底、芯片黄色加粗、预览缩进 2 列 |
| D16 | 单块上限 256 KiB，超限折叠项置灰 |
| D17 | 围栏代码块内不折叠 |
| D18 | 生效范围仅 prompt 正文与 lx Chat 输入框 |
| D19 | 路径/图片/文件夹粘贴优先于长文本面板 |

---

## 12. 待实现前确认的低风险项

以下不阻塞主体方案，实现时按推荐执行：

1. 粘贴文本是否剥离 ANSI：本次不剥离，仅归一换行。
2. 展开区双击选词/三击选行：按 `SelectionUnit` 实现；若成本过高，可先只支持拖拽字符选择，后续补齐。
3. 超限时是否显示 toast 解释：面板内已有 `too large to collapse` 灰字，不再额外 toast。
4. 芯片 label 的预览列宽固定 40；窄输入框由行绘制自然截断。
