# Prompt 编辑区滚动守卫 — 执行任务

- 规则：生产代码禁止 `unwrap()/expect()`；本任务无新增用户可见文案。

## 执行清单

### 0. 需求盘问与规格确认（已完成）
- [x] 盘问 1：修复口径选 A —— `max_scroll = (visual_rows + padding) − height`，可滚动性由滚动条几何派生
- [x] 盘问 2：滚动条可见性判据保持不变，正文恰好填满视口时仍显示滚动条并可滚进留白
- [x] 盘问 3：两条把 bug 当规格的测试改写为回归测试 + 新增不变量测试
- [x] 盘问 4：文档四处同步（规范条目、§5 后续修正、padding 公式订正、本目录）

### 1. 复现测试（先红后绿）
- [x] `src/app/prompt/tests.rs::short_text_without_scrollbar_cannot_scroll`（修复前 FAILED：滚轮仍移动视口）
- [x] `src/app/prompt/tests.rs::bottom_padding_counts_only_after_text_overflows`
- [x] `src/ui/tests.rs::prompt_scrollable_iff_scrollbar_visible`
- [x] `src/event/tests.rs::mouse_wheel_over_short_prompt_does_not_scroll_without_scrollbar`

### 2. 状态层（app）
- [x] `src/app/prompt/viewport.rs`：`max_scroll()` 改用单一公式并补文档注释
- [x] `src/app/update/editor.rs`：`scroll_prompt()` 返回 `bool`
- [x] `src/event/mouse/wheel.rs`：prompt 滚轮按实际位移决定重绘

### 3. 文档
- [x] `docs/standards/tui-design-requirements.md`：滚轮条目补「上限 = 视觉行 + 留白 − 视口高，滚动条不可见时恒为 0」
- [x] `docs/feature/prompt-pagination/design.md` §5：追加「后续修正（滚动守卫）」并限定留白可滚条件
- [x] `docs/feature/prompt-bottom-padding/design.md` §3/§4：订正公式、限定可滚条件
- [x] 本目录 `design.md` + `task.md`

### 4. 验证
- [x] `cargo test`（全量 2523 条通过）
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [ ] 用户运行 `cargo run` 人工确认观感
