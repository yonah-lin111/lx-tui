# 消息列表重复 `@` 引用去重评审

审查对象：`5c12d02c`（`fix(chat): dedupe repeated @ references in user messages`）及当前工作区后续修正。
审查方式：沿 marker 生命周检查 prepare → DB → 标题/Web prompt → rewind → 历史展示 → 模型装配。
状态：**已按本文件修复并验证**。

## 结论

| 级别 | 数量 | 说明 |
| --- | --- | --- |
| P0 | 0 | 未发现崩溃或不可恢复数据损坏 |
| P1 | 2 | prepare 层丢 marker 导致标题/Web prompt/rewind 漏处理；去重忽略 kind 导致图片静默丢失 |
| P2 | 4 | 当前轮 skill 未去重、展示键过度折叠、云端重复引用误报、测试与文档失真 |

**核心结论：去重层选错了。** `marker_start/marker_end` 是标题剥离、Web prompt 清洗与 `/rewind` 图片回填的载荷，prepare 阶段直接丢弃重复附件会连带丢失后一个出现位置。修复方向是：**引用元数据保留全部 marker；展示层按目标收敛；图片与 skill 在内容层去重。**

---

## P1

### P1-1 prepare 层丢弃重复附件，连带丢失后一个 marker

**现象**
正文：
```text
@Cargo.toml 这个说什么

@Cargo.toml 有什么功能
```
prepare 只保留 `marker 0..12`，第二条 `29..41` 没有附件承载。后续：
- 新会话标题/预览用 `strip_references_from_inputs` 只按附件 marker 切文本 → 留下 `@Cargo.toml 有什么功能`；
- AI Web 的 `web_prompt` 同样只按 `inputs` 的 marker 剥离 → 实际发给网页的 prompt 残留重复 `@` 文本；
- 若两条都是图片引用，`/rewind` 只改写第一条缺失原图的 marker，重发第二条仍报 `FileNotFound`。

**错误原因**
`strip_marker_ranges` 只删除传入区间，不会扫描剩余文本；系统没有“已识别但被合并”的区间记录。一个附件只承载一个 marker，是标题/rewind 的隐含契约。

**修复**
- 撤销 prepare 阶段的附件去重：每个文本出现位置都保留 `MessageAttachmentInput` 与 marker。
- 图片仍由 `add_unique_image` 按规范化路径/字节去重后发给模型；Web 上传路径 `files` 仍按路径去重。
- 展示层 `UserRefLine::from_attachments` 按目标收敛重复行。
- `/rewind` 恢复每条图片附件的 marker，重复引用都能被改写。

**验证**
- `prepare_keeps_repeated_text_reference_markers`：2 条附件、marker `0`/`29`，标题剥离后不含 `@Cargo.toml`。
- `prepare_keeps_same_target_across_syntaxes`：`@src.rs` + `@[refer-file](src.rs)` 两条 marker 都保留。
- `prepare_ai_web_keeps_repeated_reference_markers_but_uploads_once`：2 条 marker、1 个上传路径、`web_prompt == "看 以及"`。

### P1-2 去重键忽略 `kind`，显式 File 引用会吞掉后面的 Image 引用

**现象**
正文同时出现 `@[refer-file](photo.png)` 与 `@photo.png` 时：
- 显式引用阶段先入队 `kind = File`；
- 普通提及阶段解析出图片、已加入 `unique_images`，但 `push_unique_attachment` 只看目标路径，把 Image 附件丢弃；
- DB 与上下文装配只按 `kind == Image` 收图，最终图片一份都没发给模型。

**错误原因**
显式 `@[refer-file]` 不校验扩展名，固定按 File 入队；去重键只有目标路径，没有 kind/元数据合并策略。

**修复**
P1-1 撤销 prepare 去重后，两种引用各自保留；模型图片仍由 `unique_images` 去重。

**验证**
`prepare_keeps_image_kind_when_explicit_file_reference_comes_first`：File + Image 两条附件，`model_images.len() == 1`。

---

## P2

### P2-1 当前轮 `@[refer-skill]` 注入完全没有去重

**现象**
同一条消息写两次 `@[refer-skill](/abs/demo/SKILL.md)`，当轮注入两份完整正文。

**错误原因**
本次只改了历史回放 `chat::context::plan_turn`；当前轮走 `event::chat::skills::reference_blocks`，它对每个引用各加载一次，没有 seen 集合。`chat/agent/exec` 的 `$skill` 路径同样没有去重。

**修复**
- `reference_blocks` 按 canonical path 去重。
- `skill_reference_blocks`（`$name`）按 skill 路径去重。

**验证**
`duplicate_skill_reference_injects_once`：重复引用只产生一个注入块。

### P2-2 展示去重键使用存储后的 `source_path`，会过度折叠

**现象**
- 两个内容相同、basename 相同的不同原图（`a/shot.png`、`b/shot.png`）归档后共用同一个 `source_path`，历史树只显示一行。
- `exec -i shot.png "see @shot.png"` 的草稿附件与文本引用归档到同一路径，历史树只剩一行，违反“草稿图片 + 文本引用保留两行”的验收项。

**错误原因**
去重键 `(kind, source_path|label)` 混用了“原始目标”和“归档后路径”；图片的 `source_path` 在入库时被改写，不能代表原图身份。且草稿附件（`marker_start == None`）与文本引用共用同一个键。

**修复**
`UserRefLine::from_attachments`：
- 键增加 `marker_start.is_some()`，草稿与文本引用分开；
- 图片用原始 `label`，其余用 `source_path`；
- 旧内部 token 标签显示为 `source_path`。

**验证**
`user_ref_line_dedupes_text_refs_but_keeps_draft_and_distinct_images`：重复文本引用收敛，草稿图、文本图、不同原图各自成行。

### P2-3 云端文件重复引用原本会误报 `FileNotFound`

**现象**
网页端同一云端附件引用两次：第一次从 `remaining_cloud` 移除并保留，第二次找不到匹配，直接 `Err(FileNotFound)`。

**错误原因**
`remaining_cloud` 是一次性消费队列；没有检查“已保留”集合。

**修复**
保留 `keep_files` 已包含该文件的分支：仍生成第二条附件记录（展示层收敛），不重复上传，也不报错。

**验证**
`prepare_ai_web_keeps_duplicate_cloud_reference_without_error`：2 条附件、1 个 keep file、0 个上传路径、`web_prompt == "看 和"`。

### P2-4 测试与文档按旧方案断言，无法防回归

**现象**
- web 去重测试只断言附件/`files` 数量，不断言 `web_prompt`，P1-1 本可被一条断言发现。
- `prepare_deduplicates_same_target_across_syntaxes` 只断言 `source_path`，两种写法文本相同，无法证明保留了哪一条 marker。
- 历史树测试只断言总行数，没有反向断言“不同原图仍是两行”。
- 设计/任务文档仍写“prepare 层按目标去重”。

**修复**
- 重写 prepare 测试为 marker 保留语义，并直接断言标题剥离结果。
- 新增图片 kind、重复图片 marker、云端重复引用、展示键边界测试。
- 更新 `design.md` / `task.md`，记录“去重层从 prepare 移到展示/内容层”。

---

## 已核查未发现问题

- 图片内容去重：`add_unique_image` 同时按 canonical path 与字节比较，重复图片只发一份。
- Web `files` 上传路径：仍按 `source_path` 去重，不会重复上传。
- 短引用 `matches`：基于全部附件，撤销 prepare 去重后只可能更完整，不会解析到错误目标。
- 附件顺序：`insert_attachments` 用 `enumerate` 重新编号，marker 排序后 position 连续。

## 已知取舍

- **DB 仍保留重复附件行**：这是 marker 完整性的代价，也是相对旧行为的回退。重复行只用于展示与元数据；模型侧图片按字节去重、skill 按路径去重、Web 上传路径去重。
- 不做旧库迁移：旧消息的重复行由展示层收敛。
- 展示层不做 IO，无法 canonicalize；同一目标的绝对/相对路径别名可能显示两行，但不会重复传给模型。

## 验证

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- 定向：`prepare_keeps_*`、`prepare_ai_web_keeps_*`、`user_ref_line_*`、`user_message_dedupes_*`、`duplicate_skill_reference`。
