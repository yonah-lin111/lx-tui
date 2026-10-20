# Prompt 编辑区滚动守卫 — 设计

- 分支：`fix/prompt-scroll-guard`（worktree `.worktrees/prompt-scroll-guard`）
- 状态：需求边界已确认（Grill Me 收口完成），实现与测试完成

## 1. 背景与目标

**Bug**：prompt 编辑区没有出现滚动条时也支持滚动。

**期望**：出现滚动条时才能滚动。

根因在 `src/app/prompt/viewport.rs`：

```rust
fn max_scroll(&self) -> usize {
    self.text_max_scroll() + self.bottom_padding()   // 旧
}
```

`text_max_scroll()` 在正文不溢出时为 0，但 `bottom_padding()`（视口高度 / 4）仍会给出 1 行以上的额外距离，于是出现「短正文 + 无滚动条 + 滚轮能把正文顶出视口」的现象。滚动条可见性走的是另一条判据 `visual_rows().len() + bottom_padding() > 视口高度`，两套口径不一致正是 bug 来源。

## 2. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 修复口径 | `max_scroll()` 改为 `(visual_rows().len() + bottom_padding()).saturating_sub(height)`，即滚动条的最大可拖偏移；「滚动条不可见 ⇔ 上限为 0 ⇔ 滚轮/拖拽/轨道点击一律无效」成为恒等式 |
| 2 | 滚动条可见性 | 不变：`total = 视觉行 + 留白`，`visible = 槽高`；正文恰好填满视口时因留白仍超出而显示滚动条、可滚进留白，属于「有滚动条所以能滚」，与期望不冲突 |
| 3 | 测试 | 把两条把 bug 当规格的旧测试改写为回归测试，并新增「滚动条可见性 ⇔ 可滚动性」不变量测试 |
| 4 | 文档 | 规范文档补守卫句、`prompt-pagination/design.md` §5 追加后续修正、订正 `prompt-bottom-padding/design.md` 公式、新建本目录 |

## 3. 实现要点

- `src/app/prompt/viewport.rs`：`max_scroll()` 改用单一公式；`text_max_scroll()` 保留（拖选自动滚动的边界仍用它，不进入留白）。
- `bottom_padding()` 语义不变（仍为 `height / 4`），只是不再无条件贡献滚动距离。
- `src/app/update/editor.rs`：`scroll_prompt()` 由无返回值改为返回 `bool`（视口是否实际移动），与 `scroll_prompt_selection()` 同口径。
- `src/event/mouse/wheel.rs`：prompt 编辑器滚轮只在真正移动时置 `*dirty = true`，无滚动条时既不滚动也不重绘。

## 4. 影响与不变量

- 滚动条可见 ⇔ `max_scroll() > 0`；两者由同一公式派生，不会再漂移。
- 滚动条轨道点击（`offset_from_track_row`）、thumb 拖拽（`set_prompt_scroll`）、滚轮（`scroll_prompt`）全部经 `scroll_to`/`scroll_by` 走 `max_scroll()` 钳制，因此一个修复覆盖三条入口。
- 底部留白特性保留：正文溢出时仍可滚进 `height / 4` 的留白；短正文仍从顶部显示、不可滚动。
- 拖选自动滚动继续用 `text_max_scroll()` 作 cap，不会把选区拖进留白（原有行为不变）。
- 渲染层未改动，`ui/` 仍只读状态。

## 5. 验证

- `src/app/prompt/tests.rs`
  - `short_text_without_scrollbar_cannot_scroll`（回归）：2 行正文 / 5 行视口，`scroll_by` 与 `scroll_to` 都不移动。
  - `bottom_padding_counts_only_after_text_overflows`：短正文上限为 0 且 `row_in_padding(0)` 为假；12 行正文 / 8 行视口时上限 4 + 2 = 6，留白区判定正确。
  - 原有 `scroll_by_moves_viewport_without_cursor_and_reattaches_on_edit`、`click_focus_keeps_bottom_padding_scroll` 未改，继续守住「溢出时可进留白」。
- `src/ui/tests.rs`
  - `prompt_scrollable_iff_scrollbar_visible`（不变量）：按真实布局几何遍历多种内容行数，断言 `max_scroll == total - visible`、`滚动条可见 == max_scroll > 0`、`scroll_by` 可用性与之一致。
- `src/event/tests.rs`
  - `mouse_wheel_over_short_prompt_does_not_scroll_without_scrollbar`（回归）：滚轮后 `scroll() == 0` 且 `dirty` 保持 false。
- 全量 `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt` 通过；TUI 实际观感由用户 `cargo run` 验收。
