# `/resume` 复制会话 id 与代码块分割线：执行任务

- 状态：**已实施**（分支 `feat/lx-resume-copy-and-code-rule`，待用户验收与合并）。
- 前置：`design.md` 已由用户逐项确认（10 个决策，见其 §2）。
- 工作区：`.worktrees/lx-resume-copy-and-code-rule`，分支 `feat/lx-resume-copy-and-code-rule`（基于 `dev`）。

## 0. 开发门禁与现状核查

- [x] codegraph `explore 'resume history panel session id copy'`：确认 `CodeCopyButton` 的
      5 处调用面（`history.rs` / `code_copy.rs` / `event/tests.rs` / `main_content/lx.rs`）
      与 `ResumeSessionEdit` 的 4 处调用面。
- [x] codebase-memory `search_code LX_HISTORY_PANEL_FOOTER`：确认 5 处引用，其中
      `src/ui/text/tests.rs:410` 的 `shortcut_symbols_are_single_width_non_pua_and_used_consistently`
      会对新文案做「不出现 shift/ctrl/cmd/enter 全拼」强断言。
- [x] codebase-memory `search_code "fn render_code" --mode full`：确认现状为
      「横线 → 头部 → 正文 → 横线」4 段。
- [x] 复核 `collect` 的正文结束条件为「第一条非 `code_body` 行」，确认新增分割线会
      静默清空复制内容（design §2.1）。

## 1. `/resume` 面板复制快捷键

- [x] `src/input/mod.rs::lx_input_key`：`SUPER` 分支补 `c|C => LxInput::Clear`。
- [x] `src/app/update/resume_edit.rs`：新增 `copy_resume_session_id(state, pane) -> bool`，
      只压 `Effect::CopyToClipboard(裸 id)`，无条目返回 false 且无效应。
- [x] `src/app/update/pane.rs::apply_lx_input`：`/` 面板分支补
      `LxInput::Clear if History` → 复制并 return，位置在 `_ if editable` 之后、
      `EnterFolder` 之前。
- [x] `src/ui/text.rs::LX_HISTORY_PANEL_FOOTER` 改为含 `[copy ^c]`。

## 2. 代码块分割线与默认语言

- [x] `src/ui/markdown_render.rs::MdLine` 新增 `pub code_rule: bool`，三个构造器初始化
      `false`；`code_rule_line` 置 `true`；`wrap_md_line` 重建行时透传。
- [x] `render_code` 在头部与正文之间插入一条 `code_rule_line`。
- [x] `code_header_line` 展示标签空语言回落 `text`；高亮仍按 `None`。
- [x] `src/ui/text.rs` 新增 `LX_CHAT_CODE_LANG_DEFAULT = "text"`。
- [x] `src/ui/main_content/lx/history/code_copy.rs::collect` 跳过 `code_rule` 行。

## 3. 测试

- [x] `ui/markdown_render/tests.rs`：结构 / 标记 / `text` 默认 / 无高亮 / 缩进块。
- [x] `ui/main_content/lx/tests.rs`：新增分割线后复制正文完整、不含横线。
- [x] `ui/main_content/lx/tests/slash_panels.rs`：底边含 `[copy ^c]`。
- [x] `app/update/chat/tests/slash_panels.rs`：面板内复制裸 id、不清空输入框、空面板无效应、
      关闭面板恢复清空。
- [x] `input/tests.rs`：`Cmd+C` 映射。

## 4. 验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`（2531 lib + 全部集成测试通过）
