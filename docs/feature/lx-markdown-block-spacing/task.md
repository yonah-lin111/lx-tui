# lx 助手正文 Markdown 块间距与超链接/生图交互：任务清单

> 状态：**已完成并合并至 `dev`（Phase 1 + Phase 2 + Review 修复 P0～P3）**
> 日期：2026-11-10
> 设计依据：[design.md](./design.md) · 审查报告：[review.md](./review.md)

---

## 1. Phase 1：助手正文块间与列表间距（已完成）

- [x] 阅读项目规范文档并使用 `codegraph` / `codebase-memory-mcp` 梳理渲染链路
- [x] 并行使用 3 个 `explore` subagent 调研 `lx-tui`、`codex-main`、`opencode-dev`
- [x] 使用 `grill-me` 确认 4 个块间距边界（Q1～Q4）
- [x] T1：在 `src/ui/markdown_render.rs` 实现 `PrevBlock`、`should_insert_blank_line` 与 `transition_block`
- [x] T2：在 `code_block/tests.rs` 与 `incremental/tests.rs` 补充单元测试与流式等价性测试
- [x] T3：同步更新 `docs/standards/tui-design-requirements.md` 并在 `.worktrees/lx-markdown-block-spacing` 提交代码（`5947f68`）

---

## 2. Phase 2：Markdown 超链接渲染优化与 `Ctrl + 点击` 打开（已完成）

### T4：Markdown 超链接隐藏 `(url)` 与行内链接列区间追踪

- **目标文件**：
  - `src/ui/markdown_render/inline.rs`
  - `src/ui/markdown_render.rs`
  - `src/ui/markdown_render/table.rs`
  - `src/ui/markdown_render/code_block.rs`
- **具体改动**：
  - [x] 在 `inline.rs` 实现 `is_web_url(url)`、`inline_spans_with_links(text)` 与 `heading_spans_and_links`：
    - `[文字](http(s)://...)` 且 `文字` 非空时仅显示带下划线的 `文字`（`style::md_read_link_text()`），隐藏 ` (url)` 并记录 `link: Some(url)`；
    - `[](http(s)://...)`、`<http(s)://...>` 与裸 `http(s)://...`（自动剥离句尾标点并截断中文后文）显示 URL 本身（`style::md_read_link_text()`）并记录 `link: Some(url)`；
    - 非 `http(s)://` 链接（如 `[说明](docs/readme.md)`）保留显示 `说明 (docs/readme.md)`，`link: None`。
  - [x] 在 `MdLine` 新增 `links: Vec<MdLinkSpan>`（记录行内 `start_col..end_col` 与 `url`），并在 `wrap_md_line_inner` 软换行、标题行与 `table::table` 单元格排版中保持每行 `links` 映射。

### T5：网格链接热区、`Ctrl + 点击` 打开浏览器与 AI Web 生图点击门控

- **目标文件**：
  - `src/ui/main_content/lx/grid.rs`
  - `src/ui/main_content/lx/history/paint.rs`
  - `src/ui/main_content/lx/history/tool.rs`
  - `src/ui/main_content/lx/selection.rs`
  - `src/app/state/lx/chat.rs`
  - `src/app/update/chat/viewport.rs`
  - `src/app/update/chat/subagent.rs`
  - `src/event/mouse/press.rs`
  - `src/event/mouse/press/lx.rs`
- **具体改动**：
  - [x] 在 `Grid` 新增 `GridLinkSpan`、`set_link_span` 与 `link_at`；在 `draw_markdown`、`draw_thinking` 与 `tool::put_segments` 绘制时写入链接热区。
  - [x] 在 `CardTarget` 新增 `OpenUrl { url: String }`，在 `toggle_lx_card` 与 `toggle_agent_card` 中触发 `Effect::OpenBrowser { url }`。
  - [x] 在 `selection::press_content_hit` 中传入 `mouse.modifiers`：
    - 统一识别 `KeyModifiers::CONTROL`（`Ctrl + 点击`）返回 `Some(CardTarget::OpenUrl { url })`；
    - 将 AI Web 生图行的 `CardTarget::Attachment`（预览文件名）与已下载态 `CardTarget::DownloadOriginalImage`（`[✓ Downloaded]`）门控为仅在 `Ctrl + 点击` 时触发，未下载态 `[↓ Download original]` 保持普通左键点击即可触发下载；
    - 在 `event::mouse::press::lx` 中支持 `Ctrl + Left` 与 macOS `Ctrl + Right` 按下即打开，并拦截 Toast、命令面板与 Ask 面板遮挡。

### T6：测试、Review 与规范文档更新

- **目标文件**：
  - `src/ui/markdown_render/tests.rs`
  - `src/ui/markdown_render/code_block/tests.rs`
  - `src/ui/main_content/lx/history/tests/turn_tail.rs`
  - `src/event/tests/mouse_click.rs`
  - `docs/standards/tui-design-requirements.md`
  - `docs/feature/lx-markdown-block-spacing/review.md`
- **具体改动**：
  - [x] 更新并新增 Markdown 链接渲染测试：验证 `[文字](https://...)` 仅渲染 `文字` 且携带链接元数据、软换行跨行保留链接热区、裸 URL 剥离句尾标点与中文后文、非网页相对路径保留 `(path)`、标题行超链接与表格短标签超链接收缩不崩溃。
  - [x] 新增 `Ctrl + 点击` 测试：验证普通左键或 `Cmd` 点击链接或 AI Web 预览图文件名不触发外部打开，按住 `Ctrl + 点击`（含 `Left` 与 macOS `Right`）链接触发 `CardTarget::OpenUrl` / `Effect::OpenBrowser`、按住 `Ctrl + 点击` AI Web 预览图或 `[✓ Downloaded]` 触发定位、普通左键点击 `[↓ Download original]` 仍正常触发下载。
  - [x] 编写 `review.md` 并完成 P0～P3 全部 14 项修复。
  - [x] 合并 `feat/lx-markdown-block-spacing` 至主工作区 `dev` 分支，解决 `src/event/mouse/press.rs` 与 `src/event/tests/mouse_click.rs` 合并冲突，并清理 `.worktrees/lx-markdown-block-spacing` 工作区、分支及隔离数据库。
