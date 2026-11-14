# 消息列表重复 `@` 引用去重设计

状态：**已实施**。

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

### 3.1 准备阶段按目标去重

`src/chat/prepare.rs`：

```rust
pub(crate) fn push_unique_attachment(workspace_root, attachments, attachment) {
    // 仅比较文本引用（marker_start 有值）；草稿附件与文本引用仍是两条独立展示行。
    if attachment.marker_start.is_some()
        && attachments.iter().any(|existing| {
            existing.marker_start.is_some()
                && attachment_target(workspace_root, existing)
                    == attachment_target(workspace_root, &attachment)
        })
    {
        return;
    }
    attachments.push(attachment);
}
```

目标身份 `attachment_target`：优先 `canonicalize(workspace_root.join(path))`，失败时退回去掉尾随 `/` 的原始路径/标签。因此：

- `@Cargo.toml` 与 `@[refer-file](Cargo.toml)` 视为同一目标；
- `@a/lib.rs` 与 `@b/lib.rs` 仍是两个目标；
- 草稿图片（`marker_start = None`）不参与比较，原有“草稿 + 文本引用两行”行为不变。

`prepare_ai_web_message` 同步使用；云端文件重复引用不再报 `FileNotFound`，而是复用已保留文件名并只留一行。

### 3.2 历史展示兜底

`UserRefLine::from_attachments` 按 `(kind, source_path|label)` 去重：上线前已写入的重复行在渲染时收敛，无需改库。

### 3.3 skill 历史回放去重

`chat::context::plan_turn` 用 `HashSet` 按 `source_path` 回放 skill 摘要行，重复附件只注入一次。

## 4. 非目标

- 不改用户正文：重复的 `@Cargo.toml` 文本保留原样（用户输入，不删除）。
- 不迁移旧库：重复附件行保留在 DB，只在展示与回放时收敛。
- 不改变图片去重语义（模型只发一份、草稿行仍展示）。

## 5. 测试

- `prepare_deduplicates_repeated_text_reference`：同一文件两次 `@` → 1 条附件。
- `prepare_deduplicates_same_target_across_syntaxes`：`@path` + `@[refer-file]` → 1 条。
- `prepare_keeps_same_named_files_in_different_dirs`：同名不同目录 → 2 条。
- `prepare_ai_web_deduplicates_repeated_reference`：网页路径 → 1 条附件、1 个上传路径。
- `user_message_dedupes_repeated_reference_tree_rows`：已存重复行只渲染一行。
- `replay_dedupes_repeated_skill_attachments`：skill 摘要只回放一次。

## 6. 验证

- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：lib 5423 passed / 0 failed，全部集成测试通过。
