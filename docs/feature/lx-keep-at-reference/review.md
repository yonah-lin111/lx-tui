# lx 消息列表与置顶行保留 `@` 引用内容 — 代码审查与修复报告 (`review.md`)

- **特性目录**：`docs/feature/lx-keep-at-reference/`
- **Review 基线**：`feat/lx-keep-at-reference` @ `762e2606`（worktree `.worktrees/lx-keep-at-reference`）
- **修复提交**：`84162f74`（`fix(lx): resolve P0-P3 review findings for @ reference retention and sync feature docs`）
- **合并提交**：`ebd3d8e0`（`Merge branch 'feat/lx-keep-at-reference' into dev`）
- **关联文档**：
  - [design.md](design.md)
  - [task.md](task.md)
  - `docs/feature/lx-input-refer-and-quick-delete/{design,task}.md`
  - `docs/feature/lx-image-attachments-and-user-messages/task.md`
  - `docs/feature/lx-ai-web/design.md`
  - `docs/standards/tui-design-requirements.md`
- **Review 方式**：使用 `codegraph` 与 `codebase-memory-mcp` 追踪 `strip_references_for_display`、`BlockPaint::UserText`、`sticky_text`、`on_ai_web_session_loaded` 调用链，并委派 3 个并行 `explore` 子 Agent 分别审计「`history/` 渲染与缓存几何」「AI Web / Steer / Prepare 上下游数据流」「特性与规范文档一致性」。
- **结论**：共发现 **1 个 P0、2 个 P1、2 个 P2、1 个 P3**，已全部按优先级完成修复并通过回归测试。

---

## 0. 优先级汇总

| 级别 | 数量 | 关键问题概述 | 状态 |
| :--- | :---: | :--- | :---: |
| **P0** | 1 | AI Web `/resume` 合成云端附件 `@[refer-*]` 标记未转义文件名，气泡展示与 `/rewind` 重发遇到带括号文件名时解析截断并报 `FileNotFound` | 已修复 |
| **P1** | 2 | `snapshot.rs` 直接 `part.content.trim()` 吞掉多行文本首行前导缩进；`sticky.rs` 与 `snapshot.rs` 空正文附件回退及多文本段取样不一致 | 已修复 |
| **P2** | 2 | `cache/signature.rs` 残留已失效的 `marker_start`/`marker_end` 哈希且 `label` 仅哈希边缘；`prepare.rs` 注释及多处特性/规范文档存在事实漂移 | 已修复 |
| **P3** | 1 | `user_folding.rs` 既有测试夹具存在越界的 `marker_end: Some(37)` 偏移 | 已修复 |

---

## 1. P0（核心链路阻塞缺陷）

### [P0-1] `src/app/update/chat/web.rs:645-673` — AI Web `/resume` (`on_ai_web_session_loaded`) 合成云端附件引用标记时未调用 `encode_path` 转义文件名，导致保留 `@` 引用展示异常且 `/rewind` 重发报 `FileNotFound`

- **关键点**：
  - `on_ai_web_session_loaded` 在恢复 DeepSeek / Gemini / Grok 云端会话历史时，会将 `item.files` 中的附件名合成为 `@[refer-image]({name})` 或 `@[refer-file]({name})` 追加到用户消息 `raw_content` 中，用于消息气泡展示及 `/rewind` 回填输入框。
  - 原实现直接拼接 `format!("@[{tag}]({name})")`，未调用 `crate::app::markdown::encode_path(&name)` 转义括号与反斜杠，也未对纯空白 `item.text` 或文件名中的控制字符进行规范化。
- **错误原因**：
  - 改动前，消息列表气泡调用 `strip_references_for_display` 按 `marker_start..marker_end` 字节区间直接切掉合成标记，掩盖了合成标记本身的语法非法问题。
  - 改为保留 `@` 引用原文后，气泡直接渲染未转义的 `@[refer-*](...)` 文本；更严重的是，当用户执行 `/rewind` 将该消息回填到输入框并再次发送时，`prepare_ai_web_message` 调用 `parse_references`（内部通过 `find_closing_paren` 查找第一个未转义的 `)`），若云端附件文件名包含括号（例如常见的 `shot (1).png`），解析器会在 `shot (1` 处提前闭合，导致解析出的路径无法匹配 `remaining_cloud` 中的 `"shot (1).png"`，直接返回 `AiWebPrepareError::FileNotFound` 阻塞重发。
- **修复**：
  - 在 `on_ai_web_session_loaded` 中先对 `item.text` 规范化尾部空白（纯空白文本归一化为空串），对附件文件名清洗换行/NUL 后调用 `crate::app::markdown::encode_path` 生成合法的 `@[{tag}]({encoded_name})`。
- **验证**：
  - `src/app/update/chat/web/tests.rs::resumed_cloud_session_with_parentheses_in_filename_renders_and_rewinds_roundtrip`

---

## 2. P1（渲染正确性与边界一致性缺陷）

### [P1-1] `src/ui/main_content/lx/history/snapshot.rs:645-676` — 用户消息正文直接 `part.content.trim()` 会剥离多行文本首行的行内前导缩进

- **关键点**：
  - `snapshot.rs` 在构建用户消息 `BlockPaint::UserText` 时，原先将 `strip_references_for_display` 替换为 `let trimmed = part.content.trim();` 并把 `trimmed.to_string()` 作为 `display_text`。
- **错误原因**：
  - `str::trim()` 会同时剥离首尾换行与首行行首的水平空白（空格/Tab）。
  - 当用户发送包含首行缩进的多行代码片段或缩进列表（例如首行缩进 4 格、次行缩进 8 格）时，`part.content.trim()` 会抹掉首行的 4 格缩进，而次行保留 8 格缩进，导致首行与后续行的相对缩进对齐被破坏。
- **修复**：
  - 仅用 `part.content.trim().is_empty()` 判定是否为空白正文（决定是否触发附件数量预览回退）；非空正文使用 `part.content.trim_matches(['\r', '\n']).trim_end().to_string()`，仅剥离首尾空行与尾部空白，完整保留首行行内前导缩进与 `@` 引用原文。
- **验证**：
  - `src/ui/main_content/lx/history/tests/user_folding.rs::user_message_preserves_first_line_indentation_and_at_references`

### [P1-2] `src/ui/main_content/lx/history/sticky.rs:55-80` — 置顶悬浮行 `sticky_text` 未跳过空前导文本段且附件空标签回退不完整

- **关键点**：
  - `sticky_text` 原先只取 `message.text_parts().next()?`，且在无正文有附件时若非图片仅取 `attachments.first().map(|att| att.label.clone())`。
- **错误原因**：
  - `snapshot.rs` 会渲染消息中所有文本段，若多段用户消息的首个文本段为空白、第二段含正文与 `@` 引用，`sticky_text` 只看首段会误判为空；
  - 当非图片附件的 `label` 为空串时，`sticky_text` 最终折叠为空串返回 `None`，导致该轮用户消息在滚动时无法钉在置顶行，与 `snapshot.rs` 的 `folder_message_preview` / `file_message_preview` 兜底行为不一致。
- **修复**：
  - `sticky_text` 遍历 `message.text_parts()` 查找首个投影后非空的文本段；当所有文本段均为空且存在附件时，依次按图片数量预览、首个非空附件 `label`、文件夹数量预览、文件数量预览回退。
- **验证**：
  - `src/ui/main_content/lx/history/tests/sticky.rs::sticky_text_skips_empty_leading_text_part_and_falls_back_on_attachment_only_message`

---

## 3. P2（缓存指纹准确性与文档同步）

### [P2-1] `src/ui/main_content/lx/history/cache/signature.rs:134-150, 320-327` — 缓存指纹残留无用的 `marker_start`/`marker_end` 哈希且 `label` 哈希粒度不一致

- **关键点**：
  - `step_signature` 在 `message.role == "user"` 分支中仍对每个附件哈希 `marker_start` 与 `marker_end`，并对 `att.label` 使用 `write_edges(&att.label)`；而 `list_signature` 对 `att.label` 使用 `write_str` 却漏了 `att.position`。
- **错误原因**：
  - 早期 `snapshot.rs` 依赖 `marker_start..marker_end` 剥离用户正文中的引用标记，因此 marker 偏移会影响块文本；现在 `history/` 已不再剥离引用标记，全目录无任何代码读取 `marker_start` / `marker_end`，残留哈希徒增开销且误导维护者。
  - `write_edges` 仅哈希首尾各 64 字节，若超长路径仅中间段变化，会导致二级块缓存未失效。
- **修复**：
  - 移除 `step_signature` 中的 `marker_start` / `marker_end` 哈希，并将 `write_edges(&att.label)` 改为完整的 `write_str(&att.label)`；在 `list_signature` 中补齐 `hash.write_i64(att.position)`。

### [P2-2] `src/chat/prepare.rs:539-540` 与关联特性/规范文档 — 用户消息 `@` 引用展示语义描述漂移

- **关键点**：
  - `src/chat/prepare.rs` 中 `strip_references_for_display` 的文档注释仍称“移除展示用正文中的有效引用标记”，未说明消息列表与置顶行已改为保留 `@` 引用原文；
  - `docs/feature/lx-input-refer-and-quick-delete/{design,task}.md`、`docs/feature/lx-image-attachments-and-user-messages/task.md`、`docs/feature/lx-ai-web/design.md` 及 `docs/standards/tui-design-requirements.md` 中仍残留“用户消息气泡/置顶行剥离引用标记”的旧描述。
- **错误原因**：
  - 功能演进将“保留 `@[ref-content]/`”扩展为“保留所有 `@` 引用原文”，但旧文档与注释未同步刷新。
- **修复**：
  - 更新 `src/chat/prepare.rs` 中 `strip_references_for_display` 的文档注释；
  - 新建 `docs/feature/lx-keep-at-reference/{design,task,review}.md`；
  - 同步更新 `docs/feature/lx-input-refer-and-quick-delete/{design,task}.md`、`docs/feature/lx-image-attachments-and-user-messages/task.md`、`docs/feature/lx-ai-web/design.md` 与 `docs/standards/tui-design-requirements.md`。

---

## 4. P3（测试夹具整洁度）

### [P3-1] `src/ui/main_content/lx/history/tests/user_folding.rs:200-226` — `user_message_filters_ref_content_from_reference_tree_at_bottom` 存在越界的 `marker_end: Some(37)`

- **关键点**：
  - 测试文本 `"Check this: @[ref-content]/src/ui/"` 实际字节长度为 34，但夹具中硬编码了 `marker_start: Some(12), marker_end: Some(37)`（越界 3 字节）。
- **错误原因**：
  - 早期编写测试时手工估算字节偏移有误。
- **修复**：
  - 改为通过 `raw.find("@[ref-content]/src/ui/")` 与 `raw.len()` 动态计算精确字节区间 `Some(12)..Some(34)`。
