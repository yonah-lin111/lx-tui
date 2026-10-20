# `/resume` 复制会话 id 与代码块分割线：设计方案

> 状态：**用户已逐项确认边界**（见 §2），已实施，待验收。
>
> 任务范围：两处 lx 页交互微调——
> 1. `/resume` 二级面板内 `Ctrl/Cmd+C` 复制高亮会话 id。
> 2. agent 消息 markdown 代码块：复制按钮下方新增分割线，且语言为空时默认显示 `text`。

## 1. 背景与目标

### 1.1 `/resume` 复制会话 id

`/resume` 历史面板的每行只显示标题与相对时间，**会话 id 不再上桌**（见
`src/ui/main_content/lx/slash.rs::items` 注释），id 只用于内部绑定。用户需要把 id 交给
外部工具（`sqlite3` 查库、`git`、另一个 agent）时没有任何取用途径。

目标：面板打开时 `Ctrl/Cmd+C` 复制高亮行的 session id。

### 1.2 代码块分割线与默认语言

`src/ui/markdown_render.rs::render_code` 当前产出：**上横线 → 头部（`language: xx` + `[❐]`）
→ 正文 → 收尾横线**。头部与正文之间没有分隔，视觉上「工具条」与「代码」粘在一起；且围栏
没写语言时头部只剩一个孤零零的 `[❐]`，读不出「这是什么块」。

目标：头部与正文之间加一条分割线；无语言时头部显示 `language: text`。

成功标准：

1. 面板打开时 `Ctrl+C` 与 `Cmd+C` 都复制高亮行 session id，剪贴板内容为裸 id。
2. 面板打开时 `^C` **不再清空**输入框；面板关闭时 `^C` 仍清空输入框（行为不变）。
3. 面板底部提示出现 `[copy ^c]`。
4. 空列表 / 高亮越界时 `^C` 被吞掉：不清空、不报错、不弹 toast、不关面板。
5. 代码块结构变为「上横线 → 头部 → **新分割线** → 正文 → 收尾横线」，复制按钮仍复制正文。
6. 无语言代码块（``` 无语言 / 4 空格缩进块）头部显示 `language: text`，且**不做语法高亮**。
7. `ui/` 只读状态、无 IO；剪贴板写入只经既有 `Effect::CopyToClipboard`。

## 2. 已确认边界

用户逐题确认的决策（问题 → 结论）：

| # | 决策点 | 结论 |
| --- | --- | --- |
| 1 | 面板内 `^C` 冲突 | 面板打开期间 `^C` **只复制**、不清空输入框；面板关闭后恢复原语义。 |
| 2 | 剪贴板内容 | **仅**裸 session id 字符串，无前缀、无引号、无换行。 |
| 3 | 空列表 / 越界 | 吞键：不复制、不清空、不报错、不弹 toast、面板保留。 |
| 4 | 底部提示文案 | `[select ↵] [copy ^c] [edit ⇧↵] [close esc]`（`^c` 记法，插在 select 之后）。窄面板下接受尾部裁剪，不改面板布局逻辑。 |
| 5 | 新增分割线是否替换原横线 | **新增**一条，原有上下两条保留：上横线 → 头部 → 新线 → 正文 → 收尾横线。 |
| 6 | `text` 默认值生效范围 | **仅头部展示**；语法高亮仍按「无语言」处理（`text` 落到 `CLike` 会假高亮）。缩进代码块同样显示 `text`。 |
| 7 | `Cmd+C` 是否等效 | 等效。`SUPER` 分支补 `c\|C → LxInput::Clear`，与 prompt 侧 `copy_chord` 口径一致；提示仍写 `^c`。 |
| 8 | 复制反馈与出口 | 复用 `Effect::CopyToClipboard`（toast 落**屏幕顶部居中**，即 `anchor: None`）；不改 `Effect` 字段、不在 `app/` 里调 `event::`。 |
| 9 | 按键分流层 | 输入层不分裂：`c/C → LxInput::Clear` 不变；由 `apply_lx_input` 的 `/` 面板分支按面板种类解释该命令。 |
| 10 | 工程落点 | 单一 feature：`docs/feature/lx-resume-copy-and-code-rule/`，工作区 `.worktrees/lx-resume-copy-and-code-rule`，分支 `feat/lx-resume-copy-and-code-rule`（基于 `dev`），实现与测试通过后再问是否合并。 |

### 2.1 实现期补记的连带缺陷（问题 8 的延伸）

`src/ui/main_content/lx/history/code_copy.rs::collect` 以「遇到第一条非 `code_body` 行」
作为正文结束条件。**新增的分割线恰好会触发该条件**，导致每块复制出空字符串且不报错。
因此必须给 `MdLine` 增加 `code_rule` 标记（与既有 `code_copy` / `code_body` 同一套做法），
由 `collect` 跳过该行；不按字符猜 `───`（代码正文本身可能全是全角横线）。

## 3. 设计

### 3.1 面板内复制（决策 1/2/3/7/8/9）

- `src/input/mod.rs::lx_input_key`：`SUPER` 分支补 `KeyCode::Char('c' | 'C') => LxInput::Clear`，
  使 `Cmd+C` 与 `Ctrl+C` 落到同一命令。输入层**不感知面板状态**。
- `src/app/update/resume_edit.rs::copy_resume_session_id(state, pane)`：读高亮行
  `entry.session_id`，命中则 `push Effect::CopyToClipboard(id)` 并返回 true；无条目返回 false，
  且**不产生任何效应**。
- `src/app/update/pane.rs::apply_lx_input`：`/` 面板分支新增
  `LxInput::Clear if matches!(slash, SlashPanel::History(_))` → 调上面的函数并 `return`。
  放在 `_ if editable => {}` 之后、`EnterFolder` 分支之前；`Commands` 候选面板仍走编辑
  分支（`^C` 依旧清空输入框），`Model` 面板的 `Clear` 仍落到 `_ => return`（吞掉）。
- 剪贴板与 toast 完全复用 `db/service.rs` 的 `Effect::CopyToClipboard` 出箱路径。

### 3.2 底部提示（决策 4）

`src/ui/text.rs::LX_HISTORY_PANEL_FOOTER` 改为
`"[select ↵] [copy ^c] [edit ⇧↵] [close esc]"`。仅改常量，不动
`ui/main_content/lx/slash.rs::view` 的面板布局（窄宽度下尾部裁剪与现有 `[close esc]`
被裁时行为一致）。

### 3.3 代码块分割线与默认语言（决策 5/6）

- `src/ui/markdown_render.rs`：
  - `MdLine` 新增 `pub code_rule: bool`；`plain`/`spans`/`hanging` 三个构造器初始化为
    `false`，`code_rule_line` 置 `true`。
  - `wrap_md_line` 重建行对象时透传 `code_rule`（与 `code_body` 同处理）。
  - `render_code` 顺序改为：`code_rule_line` → 头部 → **`code_rule_line`** → 正文 →
    `code_rule_line`。
  - `code_header_line`：展示标签取 `lang.unwrap_or(text::LX_CHAT_CODE_LANG_DEFAULT)`，
    即无语言时显示 `language: text`；**高亮仍用原始 `Option<&str>`**，
    `render_code` 把两者分开传。
- `src/ui/text.rs` 新增 `pub const LX_CHAT_CODE_LANG_DEFAULT: &str = "text";`。
- `src/ui/main_content/lx/history/code_copy.rs::collect`：正文收集前先判 `line.code_rule`，
  命中则 `continue`（既不关闭收集，也不进复制内容）。

## 4. 影响面

| 位置 | 影响 |
| --- | --- |
| `ui/markdown_render.rs` | 代码块行数每块 +1；`MdLine` 增字段（仅有 1 处外部构造 `MdLine::spans/plain/hanging` 的 table.rs 走构造器，`..` 解构无）。 |
| `ui/main_content/lx/history.rs` | 每块 +1 行的行高随渲染同源，无需改代码。 |
| `ui/main_content/lx/history/code_copy.rs` | 必须跳过 `code_rule`，否则复制内容为空。 |
| `ui/overlay/ui_gallery/agent.rs` | 样例文档代码块同样多一条线，观感一致。 |
| `input/mod.rs` | `Cmd+C` 在 lx 输入框从「吞掉」变为「Clear」。 |

## 5. 测试策略

- `ui/markdown_render/tests.rs`：新结构（6 行）、`code_rule` 标记、`text` 默认标签、
  无语言不做语法高亮、缩进块也显示 `text`。
- `ui/main_content/lx/tests.rs`：新增分割线后 `[❐]` 复制内容仍为完整正文（含缩进、空行），
  且不含任何横线与 `language:`。
- `ui/main_content/lx/tests/slash_panels.rs`：底边含 `[copy ^c]`。
- `app/update/chat/tests/slash_panels.rs`：面板内 `Clear` 压 `CopyToClipboard(裸 id)`、
  不清空输入框；空面板无效应；面板关闭时 `Clear` 仍清空输入框。
- `input/tests.rs`：`Cmd+C` → `LxInput::Clear`；`Ctrl+C` 既有断言不变。
