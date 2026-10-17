# Prompt 编辑器底部空占位减半 — 执行任务

- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案集中在 `ui/text.rs`（本任务无新增文案）。

## 执行清单

### 0. 需求盘问与规格确认（已完成）
- [x] 盘问 1：把占位比例从 `height / 2` 改为 `height / 4`（占位行数减半），语义不变
- [x] 盘问 2：不设最小保底，`height < 4` 时占位为 0
- [x] 盘问 3：就地补 `prompt-pagination/design.md` §5 的调整说明，并新建本任务文档

### 1. 状态层（app）
- [x] `src/app/prompt/viewport.rs`：`bottom_padding()` 改为 `height / 4`，同步文档注释
- [x] 确认 `max_scroll()` / `row_in_padding()` / `scroll_cursor_into_view()` 无需改动

### 2. 测试
- [x] `src/app/prompt/tests.rs`：更新三处旧期望值（高 2→1、高 4→2、高 5→2 已按新比例重算）
- [x] `src/app/prompt/tests.rs`：新增 `bottom_padding_is_quarter_of_viewport_height`
- [x] `src/app/prompt/tests.rs`：新增 `bottom_padding_does_not_extend_text_rows`
- [x] `src/app/update/tests.rs`：`scroll_prompt_moves_viewport_only` 改用 4 行视口
- [x] 复查 `src/ui/tests.rs::prompt_scrollbar_appears_only_on_overflow`：使用函数值，无需改

### 3. 文档
- [x] `docs/feature/prompt-pagination/design.md` §5：补“后续调整”说明指向本目录
- [x] 本目录 `design.md` + `task.md`

### 4. 验证
- [x] `cargo test`（全量通过）
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [ ] 用户运行 `cargo run` 人工确认观感
