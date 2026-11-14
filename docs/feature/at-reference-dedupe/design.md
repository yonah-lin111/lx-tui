# 消息列表重复 `@` 引用去重设计

状态：**已实施并通过评审修正**。评审记录见 `review.md`。

## 1. 现象与证据

一条用户消息正文里出现两次 `@Cargo.toml`：

```text
@Cargo.toml 这个说什么

@Cargo.toml 有什么功能
```

消息下方的引用树出现两行 `File: Cargo.toml`。

开发库证据（worktree 隔离库）：

```text
message_id                position  label        marker_start  marker_end
msg-18e7ccde82cb69d8-1    0         Cargo.toml   0             12
msg-18e7ccde82cb69d8-1    1         Cargo.toml   29            41
```

即：同一目标按文本出现次数各写了一条附件。

## 2. 根因

1. `chat::prepare::prepare_user_message` 对每个 `@path` / `@[refer-*]` 命中各推一条 `MessageAttachmentInput`，没有按目标去重。
2. `UserRefLine::from_attachments` 原样渲染 `msg.attachments`，历史消息里的重复行全部显示。
3. `chat::context::plan_turn` 对每条 `Skill` 附件回放一行摘要；重复 skill 引用会重复回放。

## 3. 方案

### 3.1 去重层：展示与内容，而不是 prepare

`marker_start` / `marker_end` 是三类下游逻辑的载荷：

- 新会话标题/预览：`strip_references_from_inputs` 按 marker 切文本；
- AI Web：`prepare_ai_web_message` 按 marker 生成 `web_prompt`；
- `/rewind`：按图片附件 marker 把缺失原图改写成受管归档路径。

如果在 prepare 阶段直接丢掉重复附件，后一个出现位置就失去 marker，上述三条都会漏处理。因此：

- **prepare 保留每个文本出现位置的附件与 marker**；
- **UI 引用树按目标收敛重复行**；
- **图片内容按规范化路径/字节去重**（`add_unique_image`，模型只收一份）；
- **skill 注入按路径去重**（当前轮 `reference_blocks` 与历史回放）。

这保证“重复引用不重复展示、不重复传给模型”，同时不破坏标题、Web prompt 与 rewind。

`src/ui/main_content/lx/history/paint.rs::UserRefLine::from_attachments`：

```rust
let key = (
    att.kind.as_str(),
    att.marker_start.is_some(),   // 草稿附件与文本引用分开
    display_target(att).trim_end_matches('/').to_string(),
);
seen.insert(key)
```

- 图片用原始 `label`：入库时 `source_path` 会被改写成归档路径，内容相同、basename 相同的不同原图会共用该路径，不能作为身份。
- 其余类型用 `source_path`（引用原文/规范化路径），缺失时回退 `label`。
- 旧版快捷引用 label 以 `@[` 开头：展示为 `source_path`，不向用户暴露内部 token。
- UI 不做 IO，无法 canonicalize：同一目标的绝对路径/相对路径两种写法可能仍显示两行；相同写法的重复引用会收敛。模型侧图片/skill 仍按规范化路径去重。

### 3.2 云端文件重复引用

`prepare_ai_web_message` 的 `remaining_cloud` 是一次性消费队列。第一次命中后文件名进入 `keep_files`；第二次命中时复用 `keep_files` 中的名字并生成第二条附件记录（展示层收敛），不再返回 `FileNotFound`，也不会重复上传。

### 3.3 skill 去重

- 历史回放：`chat::context::plan_turn` 用 `HashSet` 按 `source_path` 回放摘要行。
- 当前轮：`event::chat::skills::reference_blocks` 按 canonical path 只注入一次全文。
- `$name` 形式：`chat/agent/exec` 的 `skill_reference_blocks` 按 skill 路径去重。

## 4. 非目标

- 不改用户正文：重复的 `@Cargo.toml` 文本保留原样（用户输入，不删除）。
- 不迁移旧库：重复附件行保留在 DB，只在展示与内容装配时收敛。
- 不改变图片去重语义（模型只发一份、草稿行仍展示）。
- 不引入附件级 `merged_ranges` 或新增 DB 列。

## 5. 测试

- `prepare_keeps_repeated_text_reference_markers`：同一文件两次 `@` → 2 条 marker，标题剥离覆盖两处。
- `prepare_keeps_same_target_across_syntaxes`：`@path` + `@[refer-file]` → 2 条 marker，按正文顺序排列。
- `prepare_keeps_image_kind_when_explicit_file_reference_comes_first`：File + Image 不互相吞并，模型图片 1 份。
- `prepare_keeps_duplicate_image_markers_but_sends_one_image`：2 条图片 marker、1 份模型图片。
- `prepare_keeps_same_named_files_in_different_dirs`：同名不同目录 → 2 条。
- `prepare_ai_web_keeps_repeated_reference_markers_but_uploads_once`：2 条 marker、1 个上传路径、`web_prompt` 已剥离两处。
- `prepare_ai_web_keeps_duplicate_cloud_reference_without_error`：云端重复引用不报错、不重复 keep。
- `user_message_dedupes_repeated_reference_tree_rows`：已存重复行只渲染一行。
- `user_ref_line_dedupes_text_refs_but_keeps_draft_and_distinct_images`：草稿/文本/不同原图边界。
- `replay_dedupes_repeated_skill_attachments`、`duplicate_skill_reference_injects_once`：历史与当前轮 skill 只注入一次。

## 6. 验证

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：全量通过。

## 7. 评审修正

| 级别 | 问题 | 修正 |
| --- | --- | --- |
| P1 | prepare 丢重复附件连带丢 marker，标题/Web prompt/rewind 漏处理 | 撤销 prepare 去重，保留全部 marker |
| P1 | 去重忽略 kind，显式 File 吞掉后面的 Image | 同上；补 File + Image 回归测试 |
| P2 | 当前轮 skill 未去重 | `reference_blocks` / `skill_reference_blocks` 按路径去重 |
| P2 | 展示键用归档 `source_path`，过度折叠 | 图片用 `label`，键加入 marker 有无 |
| P2 | 云端重复引用误报 `FileNotFound` | 复用 `keep_files`，不重复上传 |
| P2 | 测试只看数量、文档按旧方案 | 重写断言，更新设计/任务文档 |
