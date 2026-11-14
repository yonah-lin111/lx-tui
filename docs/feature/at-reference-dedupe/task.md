# 消息列表重复 `@` 引用去重任务清单

> 设计：`docs/feature/at-reference-dedupe/design.md`
> 评审：`docs/feature/at-reference-dedupe/review.md`
> 状态：**已实施并通过评审修正**。

- [x] 撤销 prepare 阶段附件去重：`prepare_user_message` / `prepare_ai_web_message` 每个文本出现位置都保留 marker。
- [x] 图片内容仍按规范化路径/字节去重（`add_unique_image`），Web 上传路径仍按 `source_path` 去重。
- [x] `src/ui/main_content/lx/history/paint.rs`：引用树按 `(kind, marker 有无, 目标)` 去重；图片用 `label`，其余用 `source_path`。
- [x] 旧 `@[ref-content]` 附件 label 展示为 `source_path`，不暴露内部 token。
- [x] `src/event/chat/skills.rs`：当前轮 `@[refer-skill]` 按 canonical path 只注入一次。
- [x] `src/chat/agent/exec/mod.rs`：`$skill` 注入按 skill 路径去重。
- [x] `src/chat/context/mod.rs`：历史 skill 回放按 `source_path` 去重。
- [x] Web 云端文件重复引用不报 `FileNotFound`，不重复上传。
- [x] 测试：marker 保留、File/Image kind、重复图片、同名不同目录、Web prompt 剥离、云端重复、历史树去重、草稿/不同原图边界、当前轮/历史 skill。
- [x] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全部通过。

## 验收（用户 `cargo run`）

- [ ] 发送一条包含两次 `@Cargo.toml` 的消息：引用树只显示一行 `File: Cargo.toml`。
- [ ] 历史里已有的重复引用消息：展开后同样只显示一行。
- [ ] `@a/lib.rs` 与 `@b/lib.rs` 仍显示两行。
- [ ] 草稿图片 + 文本引用同一图片：仍显示两行。
- [ ] 同一条消息重复 `@[refer-skill]`：只注入一次全文。
- [ ] AI Web 输入框不残留重复 `@` 文本。

## 已知取舍

- DB 仍保留重复附件行：这是 marker 完整性的代价；重复内容不在展示与模型装配中重复。
- 不做旧库迁移：旧消息重复行由展示层收敛。
