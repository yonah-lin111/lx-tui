# 文本折叠粘贴（Prompt 编辑器、Lx 输入框与聊天历史用户消息）：设计方案

> 状态：**已完成实现、代码审查与修复验证，并合并入 `dev` 分支**
>
> 关联文档：
> - 任务清单：`docs/feature/folded-long-paste/task.md`
> - 代码审查：`docs/feature/folded-long-paste/review.md`
> - 项目规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`

---

## 1. 背景与目标

### 1.1 现状

1. `src/app/paste.rs` 与 `src/app/prompt/paste.rs` 已有“路径粘贴 → 占位文本 → `Insert reference` / `Insert path` 面板”的完整链路，但它只处理**存在的文件/目录/图片路径**，不处理普通文本。
2. `Prompt`（右栏提示词编辑器）与 `Lx`（lx 输入框）的正文都是纯 `String`，光标是字节偏移；没有富文本、extmark 或折叠区模型。
3. 终端粘贴文本/代码/日志时，若直接塞进输入框会把输入框撑高、挤压历史区，且无法区分粘贴内容的语义类型（普通文本、代码片段、运行日志）。

### 1.2 目标

1. 两个主输入框（Prompt 编辑器正文、Lx Chat 输入框）每次粘贴非空白且非路径的文本时弹出 `Paste` 命令面板（与文件/图片路径粘贴面板 `Insert reference` / `Insert path` 严格区分），固定提供四项：
   - `Paste raw`（默认高亮第 1 项）：按原文直接插入；
   - `Paste text`：插入文本折叠 token `<pasted-text lines="N">...</pasted-text>`；
   - `Paste code`：插入代码折叠 token `<pasted-code lines="N">...</pasted-code>`；
   - `Paste log`：插入日志折叠 token `<pasted-log lines="N">...</pasted-log>`。
2. 折叠 token 采用**可持久化的自包含 XML 语义标签格式**，不依赖数据库新表、新列或旁路状态；保存、重启、撤销/重做、`↑`/`↓` 历史召回、复制粘贴均保留完整语义与正文。
3. 折叠态芯片移除首行预览，直接按类型显示极简标签 `[Pasted text N lines]`、`[Pasted code N lines]`、`[Pasted log N lines]`（单行为 `1 line`），且三类芯片拥有各自独立的前景字体色与深色背景色；点击展开后内联只读显示原文（不额外插入 2 列行首缩进，仅对粘贴文本自身字符铺设对应类型的深色背景，不影响同行前后普通文字）；在展开区内未拖动松开则收起，拖动则选中文本复制。
4. 光标把折叠块视为原子：左右移动整块跳过，Backspace/Delete 整块删除，不能把光标插进 token 中间。
5. Lx 发送消息、入库、写入 `prompt_history` 及发给 LLM 的上下文均保留 `<pasted-{text|code|log} lines="N">...</pasted-...>` 语义标签（仅本地 Shell 命令执行时展开为纯文本）；Lx 聊天历史中的用户消息同样将 `<pasted-...>` 渲染为对应配色的折叠芯片，支持按芯片粒度独立点击展开/收起。

### 1.3 非目标

- 不做图片/文件粘贴的第二种表达方式；路径粘贴继续走现有 `Insert reference` / `Insert path` 面板，优先级高于文本粘贴面板。
- 不新增数据库 schema、迁移或配置项。
- 不支持在围栏代码块（```` ``` ````）内部折叠。
- 不支持 Lx Shell 模式、Ask 手写答案框、Prompt 树/页列表搜索框。
- 不在展开预览里执行 Markdown 语法高亮；预览是只读纯文本 + 对应类型深色背景。
- 不持久化“展开/折叠”视图状态；重新打开页面或会话一律默认折叠。
- 不为超长粘贴做分页/虚拟化；超过 256 KiB 只允许 `Paste raw`，后三项折叠操作置灰。

---

## 2. 格式规范（持久化的核心）

### 2.1 语法

```text
<pasted-text lines="N">正文</pasted-text>
<pasted-code lines="N">正文</pasted-code>
<pasted-log lines="N">正文</pasted-log>
```

- `FoldedKind`：`Text`（`pasted-text`）、`Code`（`pasted-code`）、`Log`（`pasted-log`）。
- `N`：原文的逻辑行数，十进制正整数，最小 1。
- `正文`：原文，仅对闭合标签序列 `</pasted-text>`、`</pasted-code>`、`</pasted-log>` 在 `<` 后插入反斜杠转义为 `<\/pasted-...>`；普通反斜杠与括号保持原样，确保发送给 LLM 时代码与正则无转义噪音。

### 2.2 编解码

```rust
const CLOSE_TAG_SUFFIXES: [&str; 3] = ["/pasted-text>", "/pasted-code>", "/pasted-log>"];

pub fn encode_kind(kind: FoldedKind, content: &str) -> String {
    let line_count = content.split('\n').count().max(1);
    let tag = kind.tag_name();
    format!(
        "<{tag} lines=\"{line_count}\">{}</{tag}>",
        escape_body(content)
    )
}
```

### 2.3 解析

扫描规则：

1. 逐行扫描全文，维护围栏状态；**行首围栏标记只在 token 之外的文本上生效**。
2. 遇到 `<pasted-` 时尝试匹配 `FoldedKind::{Text, Code, Log}` 的 `open_prefix`：
   - 解析 `N`（至少一位数字且 `> 0`）→ 必须紧跟 `">"`；
   - 查找对应类型的 `close_tag`（`</pasted-text>` / `</pasted-code>` / `</pasted-log>`）；
   - 校验未闭合防护：若截取的 `raw_body` 超出上限或内部包含未闭合的同类起始标签前缀 `<pasted-... lines="`，拒绝本次匹配，防止未闭合字面量吞并后续合法 token；
   - 成功则登记 `FoldedToken { range, kind, line_count, body(解码后), id }`，从闭合标签之后继续。
3. **围栏代码块内不解析**：扫描器自行维护 `fence: Option<(u8, usize)>`，只有 token 外的行首才能开/关围栏；token 正文内部换行后的 ```` ``` ```` 不会开启或关闭外层围栏。

### 2.4 折叠态芯片显示

`FoldedToken::label()`：

```text
[Pasted text N lines]
[Pasted code N lines]
[Pasted log N lines]
```

（当 `N == 1` 时显示 `[Pasted text 1 line]` / `[Pasted code 1 line]` / `[Pasted log 1 line]`，不含 `: 首行预览`。）

### 2.5 身份与重复 token

- token 的展开状态按 `(id: u64, occurrence: usize)` 保存，其中 `id` 为 `(kind, line_count, body)` 的稳定 FNV-1a 64 位哈希。
- 同一段文本被粘贴为两个相同 token 时，`occurrence` 不同，可独立展开/收起。
- 聊天历史用户消息中的展开态记录在 `LxChat.detail_expanded` 的 `(message_id, "paste:<id>:<occ>")` 槽位中。

---

## 3. 交互规范

### 3.1 触发条件

| 条件 | 行为 |
| :--- | :--- |
| 存在的文件/图片/文件夹路径粘贴 | 走现有路径粘贴面板（`Insert reference` / `Insert path`），优先级最高 |
| 非空白普通文本粘贴 | 每次粘贴均弹出 `Paste` 四项命令面板 |
| 原文（`trim()` 后）> 256 KiB | 弹面板，`Paste raw` 可选并默认高亮，`Paste text/code/log` 置灰并显示 `too large to collapse` |
| 光标在围栏代码块内 / Prompt 受保护行 | 不弹面板，按原文插入 |
| Lx Shell 模式 / Ask 手写框 / 搜索框 | 不弹面板，按原文插入 |

### 3.2 粘贴面板

- 标题：`Paste`
- 固定四项顺序：
  1. `Paste raw`（默认高亮第 1 项），右侧显示 `N lines`（或 `1 line`）；
  2. `Paste text`，右侧显示 `N lines`（超 256 KiB 时显示 `too large to collapse` 并置灰）；
  3. `Paste code`，右侧显示 `N lines`（超 256 KiB 时置灰）；
  4. `Paste log`，右侧显示 `N lines`（超 256 KiB 时置灰）。
- 键盘：`↑`/`↓` 在可用项间循环切换（在 Lx 中绝不触发 `prompt_history` 历史召回），`Enter` 确认，`Esc` 取消（不修改缓冲区）。
- 鼠标：悬停高亮可用项，点击确认；点击面板外关闭面板且不修改缓冲区。

### 3.3 折叠态与展开态配色

| 类型 | 折叠芯片 `style::pasted_chip(kind)` | 展开预览 `style::pasted_bg(kind)` |
| :--- | :--- | :--- |
| `FoldedKind::Text` | `Color::Yellow` + `BOLD` + `Color::Indexed(236)` 深灰底 | `Color::Indexed(236)` 深灰底 + 默认前景 |
| `FoldedKind::Code` | `Color::Cyan` + `BOLD` + `Color::Indexed(23)` 深青蓝底 | `Color::Indexed(23)` 深青蓝底 + 默认前景 |
| `FoldedKind::Log` | `Color::LightGreen` + `BOLD` + `Color::Indexed(22)` 深绿底 | `Color::Indexed(22)` 深绿底 + 默认前景 |

- 展开态不插入额外行首缩进，仅对芯片展开后的文本段自身铺设背景色，不影响同一行芯片前后的普通文字。
- 在展开区内按下未拖动松开视为单击收起该芯片；发生拖动则保留 `preview_selection` 供 `Ctrl/Cmd+C` 复制解码后的原文片段（支持双击选词、三击选行）。

### 3.4 复制、保存与发送语义

- **复制**：选中折叠芯片复制底层 `<pasted-...>` 原始 token；选中展开预览区复制解码后的原文片段；`Ctrl/Cmd+A` 全选时自动清理残留 `preview_selection`，复制全文保留原始 token。
- **保存与发送**：Prompt 保存、Lx 发送入库、发送给 LLM 的上下文、以及 `prompt_history`（`↑`/`↓` 召回）均保留 `<pasted-...>` 语义标签；会话初始标题与异步标题生成守卫统一将 `<pasted-...>` 投影为 `[Pasted ... N lines]`，避免泄漏 XML 标签或导致标题更新守卫失配。
- **Lx 聊天历史用户消息**：将 `<pasted-...>` 渲染为对应颜色的折叠芯片，按芯片行内精确列区间登记 `CardKind::UserPaste` 点击热区（支持同行多芯片独立点击、点击同行普通文字不误触芯片），点击芯片时固定视口（`pin_lx_viewport`）并切换展开态。
