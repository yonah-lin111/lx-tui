# Prompt 编辑器底部空占位减半 — 设计

- 分支：`dev`（小改动，不单独开分支）
- 状态：需求边界已确认，实现与测试完成

## 1. 背景与目标

prompt 编辑器允许把视口滚到正文之后，留出视觉呼吸空间。`docs/feature/prompt-pagination/design.md` §5 的实现口径是“视口高度除以 2”，实际使用时留白过大：正文写完后向下滚动，最多可出现约半屏空白。

目标：把这段底部虚拟空占位的行数调整为当前的一半，即改用视口高度除以 4（向下取整），其余行为完全不变。

## 2. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | “当前的一半”如何落地 | `Prompt::bottom_padding()` 从 `height / 2` 改为 `height / 4`，向下取整；不改变“只影响滚动上限与滚动条几何”的语义 |
| 2 | 矮视口是否保底 | 不设最小保底：纯 `height / 4`，`height < 4` 时占位为 0；矮视口不再有额外留白，也不额外冒出滚动条 |
| 3 | 文档 | 在 `prompt-pagination/design.md` §5 就地补一条“后续调整”说明，并新建本任务目录记录决策与验证；不抹掉原始“除以 2”记录 |

## 3. 实现要点

- `src/app/prompt/viewport.rs`：`bottom_padding()` 返回 `usize::from(self.height) / 4`，文档注释同步为“四分之一”，并说明高度不足 4 行取整为 0。
- 占位仍只进入 `max_scroll()`（`text_max_scroll() + bottom_padding()`）与 `src/ui/prompt_frame.rs` 的滚动条 `total`，不写入 `visual_rows()`，因此光标、点击、选区与 `row_in_padding` 语义不变。
- `row_in_padding` 仍以“滚动偏移 + 视口行 >= 视觉行数”判断，点击留白只聚焦不移动光标的行为自动跟随新的滚动上限。

## 4. 影响与不变量

- 空文档、短文本打开时仍从顶部显示，不自动滚到留白。
- 滚轮与滚动条可进入文末留白；方向键与编辑命令仍限于真实正文。
- `scroll_cursor_into_view` 的钳制上限自动缩小，编辑时不再把留白立即收回的行为保留。
- 用户可见文案无变化，不新增 `ui/text.rs` 常量。

## 5. 验证

- `src/app/prompt/tests.rs`：
  - 更新 `scroll_by_moves_viewport_without_cursor_and_reattaches_on_edit`、`click_focus_keeps_bottom_padding_scroll`、`scroll_by_can_enter_bottom_padding_for_short_text` 的期望值；
  - 新增 `bottom_padding_is_quarter_of_viewport_height`（1/3/4/5/8/20 行的高度→留白映射）；
  - 新增 `bottom_padding_does_not_extend_text_rows`（占位不混入视觉行，全部计入滚动上限）。
- `src/app/update/tests.rs::scroll_prompt_moves_viewport_only` 改用 4 行视口，断言滚动上限来自“真实文本上限 + 1 行留白”。
- `src/ui/tests.rs::prompt_scrollbar_appears_only_on_overflow` 用函数值断言，随实现自动跟随，无需改期望常量。
- 全量 `cargo test` 通过；TUI 实际观感由用户 `cargo run` 验收。
