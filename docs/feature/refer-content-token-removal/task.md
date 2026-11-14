# 移除 `@[ref-content]/` 快捷引用标记任务清单

> 设计：`docs/feature/refer-content-token-removal/design.md`
> 状态：**已实施、通过评审修正并合并到 `dev`**（2026-11-14）。用户已确认“全链路删除 + 插入候选原文”。评审记录见 `review.md`。
> 执行顺序：Task 1 → 5；每个 Task 完成后跑对应精确测试，最后统一 fmt/clippy/test。

---

## Task 1：快捷面板改插原文

- [x] `src/app/markdown/refer_shortcut.rs`
  - [x] `refer_insertion` 改为 `format!("{} ", candidate.text.trim_end())`。
  - [x] 删除 `Source::RefContent` 变体、区间收集、提及区间过滤与匹配分支。
  - [x] 更新模块注释：确认时插入候选原文。
- [x] `src/app/markdown/refer_shortcut/tests.rs`
  - [x] `insertion_uses_ref_content_with_trailing_space` 改为断言原文插入。
  - [x] 删除 ref-content 候选相关断言；保留 `@ref/` 与 `@[refer-*]` 去重、排序用例。
- [x] 精确校验：`cargo test markdown::refer_shortcut`

---

## Task 2：删除 `ref_content` 模块与调用点

- [x] 删除 `src/app/markdown/ref_content.rs` 与 `src/app/markdown/ref_content/`。
- [x] `src/app/markdown.rs`：移除 `mod ref_content; pub use ref_content::*;`。
- [x] 新增 `src/app/markdown/mention_paths.rs`：
  - [x] 迁入 `extract_mention_paths`，删除 ref-content 分支。
  - [x] 继续跳过 `@ref/`、`@[refer-`，围栏内不解析。
  - [x] 单测覆盖普通 `@`、跳过前缀、围栏。
- [x] `src/app/prompt.rs`、`src/app/state/lx/edit.rs`：删除 `ref_content_deletion_range` 退格分支。
- [x] 精确校验：`cargo test ref_content`、`cargo test mention_paths`、`cargo test backspace`

---

## Task 3：清理 `prepare` 与历史提取

- [x] `src/chat/prepare.rs`：删除第 3.5 步与 `resolve_ref_content_target`。
- [x] `src/chat/prepare/web.rs`：删除第 3 步与该导入。
- [x] `src/app/state/lx/chat.rs`：更新注释（只提取普通 `@`）。
- [x] `src/app/state/lx/chat/steps/tests.rs`：夹具改用普通 `@docs/readme.md`。
- [x] `src/chat/prepare/tests.rs`：删除两个 ref-content 用例。
- [x] 精确校验：`cargo test prepare`

---

## Task 4：清理 UI 高亮与附件树过滤

- [x] `src/ui/markdown.rs`：删除 `TokenKind::RefContent` 与高亮分支。
- [x] `src/ui/style.rs`：删除 `markdown_ref_content`。
- [x] `src/ui/right_panel/prompt/mod.rs`：删除对应映射。
- [x] `src/ui/main_content/lx/history/paint.rs`：删除 ref-content 标签过滤。
- [x] `src/ui/markdown/tests.rs`：删除 ref-content 高亮用例。
- [x] `src/ui/main_content/lx/history/tests/user_folding.rs`：删除过滤用例。
- [x] 精确校验：`cargo test ui::markdown`、`cargo test user_message`

---

## Task 5：更新其余测试与统一校验

- [x] `src/app/prompt/tests.rs`：更新快捷面板插入断言，删除 ref-content 退格用例。
- [x] `src/app/update/tests.rs`：更新回车确认与 lx 快捷插入断言。
- [x] `src/app/state/lx/refer/tests.rs`：更新插入与整块删除断言。
- [x] `tests/prompt_short_ref.rs`：更新插入断言；删除 ref-content 高亮与删除用例。
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] 确认代码中不再有 `REF_CONTENT_PREFIX` / `ref_content` / `RefContent` 残留（历史 docs 除外）。

---

## 验收清单（用户 `cargo run`）

- [x] 正文先写 `@docs/feature/x/design.md`，另起一行输入 `design` 确认：插入完整 `@docs/feature/x/design.md `，不是 `@[ref-content]/...`。
- [x] 对 `@[refer-folder](/abs/path)`、`@[refer-image](...)`、`@[refer-skill](...)` 重复确认：插入各自原标记。
- [x] 对 `@ref/...` 候选重复确认：插入 `@ref/...`，高亮仍为柔紫。
- [x] 插入的 `@路径` 发送后正常挂载文件/目录与图片。
- [x] 快捷引用不再有独立高亮；`@ref/` 高亮保持。
- [x] `/`、块面板、`@` 提及面板无回退。

---

## 风险与未验证

- 旧草稿/历史消息里的 `@[ref-content]/...` 变为普通文本，不再挂载文件（已确认不迁移）。
- 旧消息附件树可能显示一条 ref-content 标签（不再过滤）。
- 本环境无法交互验证 TUI 渲染，需用户 `cargo run` 确认。


---

## 评审修正（review.md）

- [x] P2：`refer_candidates` 跳过 `@ref/` 开头的 mention 区间，尾句点不再产生双候选。
- [x] P2：展示层把旧 `@[ref-content]` 标签还原为 `source_path`，不暴露内部 token。
- [x] P2：更新快捷引用测试名、失败消息与恒真断言；目录候选改为精确整段断言。
- [x] P2：补旧 token 0 附件、完整引用 + 短引用解析、尾句点候选、旧标签展示测试。
- [x] P2：`extract_mention_paths` 复用 `mention_token_ranges`，删除第三套扫描逻辑。
- [x] 接受项：单独确认悬空 `@ref/` 不会额外解析；合法短引用仍要求同消息完整引用（D2/D3）。
- [x] 验证：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`cargo test --test prompt_short_ref`。
---

## 合并记录（2026-11-14）

- 已合并到 `dev`：`f939665e`（`merge: prompt mention/paste parity and @ reference cleanup`）。
- 冲突处理：与 `dev` 的 BTW/Agents composer 改动合并时，保留 `MentionScanOwner` 归属路由与 Agents composer 的 `@` 支持，同时接入本次“只有文本编辑才重开面板”的时机模型；`panels.rs` 的编辑/移动分流同时覆盖主输入框与 detail composer。
- 工作树 `.worktrees/prompt-mention-paste-parity` 与分支 `feat/prompt-mention-paste-parity` 已移除。
- 合并后验证：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（lib 5461 passed / 0 failed / 6 ignored，全部集成测试通过）。
