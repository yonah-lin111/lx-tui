# 消息列表重复 `@` 引用去重任务清单

> 设计：`docs/feature/at-reference-dedupe/design.md`
> 状态：**已实施**。

- [x] `src/chat/prepare.rs`：新增 `push_unique_attachment` / `attachment_target`，文本引用按规范化目标去重。
- [x] `prepare_user_message` 所有文本引用 push 点改用统一入口（草稿图片保留原行为）。
- [x] `src/chat/prepare/web.rs`：`prepare_ai_web_message` 同步去重；云端文件重复引用不再报缺失。
- [x] `src/ui/main_content/lx/history/paint.rs`：附件树按 `(kind, source_path|label)` 去重（兜底旧数据）。
- [x] `src/chat/context/mod.rs`：skill 历史回放按 `source_path` 去重。
- [x] 测试：prepare 三条、web 一条、历史树一条、skill 回放一条。
- [x] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全部通过。

## 验收（用户 `cargo run`）

- [ ] 发送一条包含两次 `@Cargo.toml` 的消息：引用树只显示一行 `File: Cargo.toml`。
- [ ] 历史里已有的重复引用消息：展开后同样只显示一行。
- [ ] `@a/lib.rs` 与 `@b/lib.rs` 仍显示两行。
- [ ] 草稿图片 + 文本引用同一图片：仍显示两行（原有行为）。
