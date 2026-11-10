# lx 助手正文 Markdown 块间可读性间距：任务清单

> 状态：**已完成**
> 日期：2026-11-10
> 设计依据：[design.md](./design.md)

---

## 1. 设计与边界确认（已完成）

- [x] 阅读项目规范文档（`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`）
- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索 `lx-tui` 的 Markdown 渲染与历史快照链路
- [x] 并行使用 3 个 `explore` subagent 调研 `lx-tui`、`codex-main`（`codex-rs/tui/src/markdown_render.rs`）与 `opencode-dev`（`@opentui/core` `Markdown.ts`）的块间距实现
- [x] 结合截图与数据库原始消息定位根因（`markdown_render.rs` 吞掉全部 `\n\n` 空行且缺少顶层块/列表切换间距状态机）
- [x] 使用 `grill-me` 逐项向用户确认 4 个关键边界（顶层块间距规则、列表内部间距规则、作用范围、代码块 `▂` 顶边留白）
- [x] 编写详细设计文档 [design.md](./design.md) 与本任务清单

---

## 2. 已执行实现任务

### T1：实现助手正文块间与列表间距状态机

- **目标文件**：`src/ui/markdown_render.rs`
- **具体改动**：
  - [x] 新增 `ListKind`（`Bullet` / `Ordered` / `Task`）与 `ListItem::kind(&self)`。
  - [x] 新增 `PrevBlock` 枚举（`Paragraph { in_list, depth }`、`Heading`、`ListItem { depth, kind }`、`CodeBlock`、`Table`、`Quote`、`Rule`）。
  - [x] 将 `ParserState::after_code_block: bool` 替换为 `prev_block: Option<PrevBlock>` 与 `saw_blank_line: bool`。
  - [x] 实现 `should_insert_blank_line(prev, saw_blank_line, curr)` 与 `transition_block(state, curr, code_chrome, lines)`：
    - 块首（`prev == None`）与块尾不补空行；
    - 源码中一个或多个连续空行合并记录为 `saw_blank_line = true`；
    - 仅当 `code_chrome == true`（即 `render_wrapped_code_blocks` 与 `IncrementalMarkdownState`）时插入块间空行；
    - 覆盖顶层块（段落、标题、列表、代码块、表格、引用块、分割线）与列表内部（松紧列表、父子嵌套、子列表退回外层、同级异类列表切换、列表续段）全部转换分支。
  - [x] 将 `blockquote` 简化为单行 `blockquote_line(line)`，由主循环统一驱动 `update_list_context` 与 `transition_block`。

### T2：补充单元测试与流式增量回归测试

- **目标文件**：
  - `src/ui/markdown_render/code_block/tests.rs`
  - `src/ui/markdown_render/incremental/tests.rs`
  - `src/ui/main_content/lx/history/tests/tool_geometry.rs`
  - `src/ui/main_content/lx/selection/tests.rs`
- **具体改动**：
  - [x] 新增顶层块间距测试：验证段落单 `\n` 紧邻、`\n\n` 空 1 行、多空行合并为 1 行、首尾无多余空行，以及标题/表格/引用/分割线与相邻块之间恒隔 1 行。
  - [x] 新增列表间距测试：验证紧凑列表（含软换行多行项）0 空行、松散列表 1 空行、父项→嵌套子列表 0 空行、子列表结束退回外层列表项 1 空行、同级无序/有序/任务列表切换 1 空行。
  - [x] 新增截图真实案例回归测试：验证「段落 → `## 核查结论` → 粗体段 → 无序列表 → 粗体段 → 多段统计」的完整空行分布。
  - [x] 新增非助手正文紧凑性测试：验证 `render_wrapped` / `render_wrapped_literal_rules` 保持紧凑无块间空行。
  - [x] 在 `incremental/tests.rs` 中验证流式逐字节输入与伪随机块序列下 `prefix + tail == render_wrapped_code_blocks` 恒成立。

### T3：同步项目设计规范文档

- **目标文件**：`docs/standards/tui-design-requirements.md`
- **具体改动**：
  - [x] 在 `docs/standards/tui-design-requirements.md` 的 Markdown 阅读渲染章节补充助手正文块间与列表间距规则说明，保持规范文档与代码实现一致。

---

## 3. 验证计划

- [x] `rtk cargo fmt --check`
- [x] `rtk cargo clippy --all-targets -- -D warnings`
- [x] `rtk cargo test markdown_render </dev/null`
- [x] `rtk cargo test ui::main_content::lx </dev/null`
