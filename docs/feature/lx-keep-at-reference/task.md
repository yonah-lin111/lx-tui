# lx 消息列表与置顶行保留 `@` 引用内容开发任务 (`task.md`)

状态：已完成代码实现、Review 审查与 P0-P3 缺陷修复，待用户确认合并。
对应设计：`docs/feature/lx-keep-at-reference/design.md`。
审查报告：`docs/feature/lx-keep-at-reference/review.md`。

---

## 阶段 1：需求澄清与工作区初始化

- [x] 1.1 使用 `grill-me` 逐项确认 4 个关键产品与视觉边界（引用保留范围、底部附件树、置顶悬浮行、气泡内文本样式）
- [x] 1.2 在 `.worktrees/lx-keep-at-reference` 创建独立工作区与分支 `feat/lx-keep-at-reference`
- [x] 1.3 对新建工作区完成 `codebase-memory-mcp` 与 `codegraph` 索引初始化

---

## 阶段 2：核心渲染层改造（`ui/main_content/lx/history`）

- [x] 2.1 改造 `src/ui/main_content/lx/history/snapshot.rs`
  - [x] 移除用户消息气泡构建时对 `crate::chat::strip_references_for_display` 的调用，完整保留 `@` 引用原文
  - [x] 保留空正文带附件时的图片/文件夹/文件数量预览回退逻辑
  - [x] 保留底部 `UserRefLine::from_attachments` 附件树渲染逻辑不变
- [x] 2.2 改造 `src/ui/main_content/lx/history/sticky.rs`
  - [x] 移除 `sticky_text` 对 `crate::chat::strip_references_for_display` 的调用，使置顶悬浮行同步保留 `@` 引用原文
- [x] 2.3 编写基础单元测试
  - [x] `src/ui/main_content/lx/history/tests/user_folding.rs`：验证含多个 `@` 引用及仅含单个 `@` 引用的用户消息渲染与附件树
  - [x] `src/ui/main_content/lx/history/tests/sticky.rs`：验证置顶行保留 `@` 引用原文

---

## 阶段 3：全链路代码审查与 P0-P3 缺陷修复（Review & Fixes）

- [x] 3.1 结合 `codegraph`、`codebase-memory-mcp` 与 3 个并行 `explore` 子 Agent 审计历史渲染、AI Web、缓存指纹与特性文档，编写 `docs/feature/lx-keep-at-reference/review.md`
- [x] 3.2 修复 **[P0-1]**：`src/app/update/chat/web.rs` 的 `on_ai_web_session_loaded` 合成云端附件引用标记时调用 `encode_path` 转义文件名并规范化空白，修复含括号文件名在气泡展示与 `/rewind` 重发时截断报 `FileNotFound` 的问题
- [x] 3.3 修复 **[P1-1]**：`src/ui/main_content/lx/history/snapshot.rs` 非空用户消息使用 `trim_matches(['\r', '\n']).trim_end()` 替代 `trim()`，保留多行代码/列表首行的行内前导缩进
- [x] 3.4 修复 **[P1-2]**：`src/ui/main_content/lx/history/sticky.rs` 的 `sticky_text` 支持跳过空前导文本段，并在附件 `label` 为空时兜底回退至文件夹/文件数量预览
- [x] 3.5 修复 **[P2-1]**：`src/ui/main_content/lx/history/cache/signature.rs` 移除用户消息附件已失效的 `marker_start` / `marker_end` 哈希，统一使用完整 `write_str(&att.label)` 并在 `list_signature` 中补齐 `att.position`
- [x] 3.6 修复 **[P2-2]**：更新 `src/chat/prepare.rs` 文档注释，并同步更新关联特性文档（`lx-input-refer-and-quick-delete`、`lx-image-attachments-and-user-messages`、`lx-ai-web`）与 `docs/standards/tui-design-requirements.md`
- [x] 3.7 修复 **[P3-1]**：修正 `src/ui/main_content/lx/history/tests/user_folding.rs` 中 `user_message_filters_ref_content_from_reference_tree_at_bottom` 越界的 `marker_end` 测试夹具数据
- [x] 3.8 执行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 及受影响模块单元测试全部通过
