# 文本折叠粘贴（Prompt 编辑器、Lx 输入框与聊天历史用户消息）：执行任务清单

> 状态：**已在工作区 `.worktrees/folded-long-paste`（分支 `feat/folded-long-paste`）全部完成并通过代码审查与验证**
>
> 关联文档：
> - 设计方案：`docs/feature/folded-long-paste/design.md`
> - 代码审查：`docs/feature/folded-long-paste/review.md`
> - 项目规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`

---

## Task 1：折叠 token 核心模块 `src/app/paste/folded.rs`

- [x] 定义 `FoldedKind::{Text, Code, Log}` 与 `FoldedToken { range, kind, line_count, body, id }`。
- [x] 实现 `<pasted-{text|code|log} lines="N">...</pasted-...>` 编解码（`encode_kind`、`encode`、`escape_body`、`unescape_body`），仅转义三类闭合标签序列。
- [x] 折叠态芯片文案 `FoldedToken::label()` 输出 `[Pasted text|code|log N lines]`（单行为 `1 line`，无首行预览后缀）。
- [x] 实现围栏感知扫描 `tokens(text)`，忽略代码围栏内的标签，并防止未闭合的起始标签字面量吞并后续合法 token。
- [x] 实现 `ExpandedPastes` 与 `Projection<'a>`（无 token 时 `Cow::Borrowed` 零分配；支持 `raw_to_display`、`display_to_raw_directed`、`display_range_to_raw`、`chip_at`、`copy_display_range`，移除展开态行首额外缩进）。
- [x] 实现 `mask`、`snap_range`、`clamp_cursor`、`prev_boundary`、`next_boundary` 等原子边界辅助函数。
- [x] 单元测试 `src/app/paste/folded/tests.rs` 覆盖三类标签编解码、闭合标签转义、围栏隔离、投影坐标边界及未闭合标签防护。

---

## Task 2：四项文本粘贴面板状态机 `src/app/paste/text.rs`

- [x] 定义 `TextPasteAction::{Raw, Text, Code, Log}`（固定顺序：`Paste raw`、`Paste text`、`Paste code`、`Paste log`）。
- [x] `text_paste_candidate(raw)`：每次非空白文本粘贴均生成候选（取消行数/字符数阈值限制）；超过 `MAX_FOLDED_BYTES`（256 KiB）时标记 `too_large = true`。
- [x] `TextPastePanel::new` 默认高亮第 1 项 `TextPasteAction::Raw`（索引 0）；超限时禁用后三项折叠操作，`disabled_mask() -> [bool; 4]` 返回 `[false, true, true, true]`。
- [x] 单元测试 `src/app/paste/text/tests.rs` 覆盖任意非空白粘贴触发、超限禁用掩码、默认首项 `Raw` 及四项循环切换。

---

## Task 3：Prompt 编辑器与 Lx 输入框集成

- [x] `Prompt`（`src/app/prompt/folded.rs`）与 `Lx`（`src/app/state/lx/folded.rs`）支持四项粘贴面板确认（`Raw` 插入原文，`Text`/`Code`/`Log` 插入对应折叠 token 并补齐空格，一步撤销）。
- [x] 粘贴入口 `src/event/paste.rs` 优先检测存在的文件/图片/目录路径（弹出 `Insert reference` / `Insert path`），非路径非空白文本弹出四项 `Paste` 面板。
- [x] 芯片与展开预览渲染（`src/ui/style.rs`、`src/ui/prompt.rs`、`src/ui/main_content/lx/input.rs`）：
  - `Text`：`Yellow` 加粗 + `Indexed(236)` 底；
  - `Code`：`Cyan` 加粗 + `Indexed(23)` 底；
  - `Log`：`LightGreen` 加粗 + `Indexed(22)` 底；
  - 展开态仅对芯片文本自身铺设对应深色底，不污染同行前后普通文字。
- [x] Lx 发送、入队、`prompt_history` 记录及落库保留 `<pasted-...>` 语义标签，`↑`/`↓` 历史召回保持折叠芯片显示。

---

## Task 4：Lx 聊天历史用户消息芯片渲染与独立展开

- [x] `src/ui/main_content/lx/history/{rows,paint,snapshot}.rs` 与 `src/ui/main_content/lx/history_hit.rs` 将用户消息中的 `<pasted-...>` 渲染为对应颜色的折叠芯片。
- [x] 按芯片行内精确列区间登记 `CardKind::UserPaste`，支持同行多芯片独立点击、点击同行普通文字不误触芯片，点击切换时固定视口（`pin_lx_viewport`）。

---

## Task 5：Code Review 缺陷修复（见 `docs/feature/folded-long-paste/review.md`）

- [x] **[P0-1]** 修复 Prompt 选区替换粘贴确认后未清除 `AppState.selection` 导致后续输入误删新芯片的问题。
- [x] **[P0-2]** 修复 `Projection::display_to_raw_directed`（`after = true`）在芯片左边界将前一段末尾误判为芯片末尾、以及 `display_range_to_raw` 扩展普通 `Plain` 段的问题。
- [x] **[P1-1]** 修复 Lx 会话自动标题生成与 `initial_session_title_for_meta` 未对折叠 token 做投影导致标题更新守卫失配及泄漏原始标签的问题。
- [x] **[P1-2]** 修复 Lx 聊天历史用户消息芯片点击热区缺失列范围（整行误触、同行多芯片无法点第二个）及切换未固定视口的问题。
- [x] **[P1-3]** 修复 `Ctrl/Cmd+A` 全选与键盘扩选未清除 `preview_selection`、以及 Prompt 窄屏软换行下复制折叠芯片重复拼接 token 的问题。
- [x] **[P1-4]** 修复 Prompt 块/模板操作与 `newline_below`、以及 Lx 斜杠/历史引用/技能扫描未统一遮蔽折叠 token 的问题。
- [x] **[P2-1]** 修复粘贴面板生命周期（空粘贴取消、视图切换取消、`save_prompt_now` 守卫、与 `slash`/`refer` 面板互斥、预览区同格微动单击收起与空行多击守卫）。
- [x] **[P2-2]** 修复 `folded::tokens` 遇到未闭合起始标签字面量时吞并后续合法 token 的问题。
