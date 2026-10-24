# lx 消息列表滚动性能优化任务清单

> 状态：**未开始**。用户审核 `design.md` 并确认后，按本清单顺序执行。
>
> 完成一项勾一项；每项都要写明落点与验证方式。禁止跳过前一项直接改后面的模块。

## A. 预备与基线

- [ ] A1. 用 codegraph / codebase-memory 复核将修改文件的索引覆盖情况；索引落后时先刷新，或用 Read 核对源码。
  - 重点文件：`history.rs`、`history/{rows,body,tool,user_shell,history_hit}.rs`、`tool/{command,file_summary,file_diff,plan}.rs`、`ui/{mod,panes,main_content}.rs`、`event/mouse/*`、`app/update/chat/viewport.rs`。
  - 记录 `index_status` 的 `parse_partial` / `skipped` 列表，逐项对照。

- [ ] A2. 在改动前跑一次基线测试，确认起点全绿。
  - `cargo test`
  - 记录 `history/tests.rs`、`history/tests/tool_geometry.rs`、`app/update/chat/tests/viewport_anchor.rs` 的用例数。

- [ ] A3. 记录基线性能（可选但推荐）。
  - 在临时分支或本地加一个一次性计时测试，构造 1000 条消息测一次 `layout + render`，把数字写进本文件末尾“基线记录”。
  - 测完删除临时测试，不提交。

## B. 缓存底座

- [ ] B1. 新建 `src/ui/main_content/lx/history/cache.rs`。
  - `RenderCache { frame, panes: HashMap<PaneId, PaneCache> }`。
  - `begin_frame` / `pane` / `retain_panes` / `mark_used` / `bytes`。
  - `PaneCache { frame, snapshot: Option<(u64, Arc<HistorySnapshot>)>, blocks, bytes }`。
  - `BlockKey { message_index: usize, step_index: usize, width: u16, sig: u64 }`。
  - `BlockEntry { paint: Arc<BlockPaint>, bytes, last_frame }`。
  - 32MB LRU：只淘汰 `last_frame < current_frame` 的项。
  - 关键点：`PaneCache` 不持有 `Lx` 借用，只持 `Arc` 数据；所有方法用 `Option`/`?`，禁止 `unwrap()`。

- [ ] B2. 定义 `BlockPaint` 及 `rows()`。
  - 变体至少覆盖：`UserText`、`Markdown`、`Thinking`、`Tool`、`Shell`、`Notice`、`Stats`、`Compact`、`Rewind`。
  - 每个变体的 `rows()` 必须等于对应现有 `*::rows()` 的返回值（先写对照测试）。
  - 动态字形、hover 样式、选中底色**不进** `BlockPaint`。

- [ ] B3. 实现指纹工具。
  - `Fnv` 小实现（不引第三方依赖）；`write_edges`（首尾各 64 字节）。
  - `list_signature(lx, width)`、`part_signature(part, width, expanded)`、`tool_signature(...)`、`shell_signature(...)`。
  - 关键点：指纹只读状态，不扫描正文全文；折叠状态、宽度、状态码必须进入指纹。

- [ ] B4. 缓存单测（先于业务接入写，保证底座正确）。
  - `cache.rs` 内 `#[cfg(test)] mod tests`：
    - 同 key 两次取用只调用一次构造闭包；
    - 不同 sig / width / 折叠状态产生不同 key；
    - LRU 超限后旧项被淘汰，本帧使用项保留；
    - 淘汰后重新构造的 `BlockPaint::rows()` 与淘汰前一致。
  - `cargo test cache::` 通过。

## C. 块绘制数据（按模块接入缓存）

> 每项完成标准：同一份数据同时供 `rows()` 和 `draw()` 使用，二者不再各自构造一遍。

- [ ] C1. 用户正文与统计行：`history/rows.rs`。
  - `cached_wrap(content, width) -> Arc<Vec<String>>`，接入 `text_rows`（用户分支）与统计行绘制。
  - 保留 `wrap()` 作为无缓存底层实现，缓存层调用它。

- [ ] C2. 助手 markdown 正文：`history/rows.rs` + `history/body.rs`。
  - `cached_md(content, width) -> Arc<Vec<MdLine>>`，接入 `markdown_rows`、`draw_body`、`draw_thinking`。
  - 关键点：`draw_body` 不再调用 `markdown_render::render_wrapped`，只遍历缓存行。
  - 行窗口裁剪：`skip = scroll.saturating_sub(block_start)`，只遍历视口内的行，同时正确推进绝对行号。

- [ ] C3. 命令卡片：`history/tool/command.rs`。
  - `blocks()` 改为经由缓存返回 `Arc<CommandPaint> { title: Vec<SegmentText>, body: Vec<BodyRow> }`。
  - `rows()` 用 `title_rows() + body.len()`；`draw()` 直接消费 `body`。
  - 参数区/输出区折叠状态、`output_tail.len()`/首尾字节、`dropped_bytes`、进程状态进入指纹。
  - `hovered` 参数全部删除（见 F 组）。

- [ ] C4. 文件工具摘要：`history/tool/file_summary.rs`。
  - `detail()` 结果缓存为 `Arc<Vec<Vec<Segment>>>`；`detail_rows`、`draw` 共用。
  - 折叠额度、展开状态、`arguments_json` 长度/首尾字节进入指纹。

- [ ] C5. 文件差异卡片：`history/tool/file_diff.rs`。
  - `rows()` 与 `draw()` 共用 `Arc<Vec<Row>>`；单卡与合并组分别缓存。
  - 合并组的 `expanded: &[bool]` 必须进入指纹（逐调用独立）。
  - 展开底色在绘制时补 `chat_expanded_bg`，不要烘进缓存行。

- [ ] C6. 计划卡片：`history/tool/plan.rs`。
  - 行数据按 `plan.revision + steps + 宽度 + 展开` 缓存。

- [ ] C7. 本地命令卡片：`history/user_shell.rs`。
  - `sections()` 结果缓存为 `Arc<ShellPaint> { params, output }`；`rows()`/`draw()` 共用。
  - `output_tail`、`dropped_bytes`、进程状态、折叠状态进入指纹。

- [ ] C8. compact / rewind / notice：`history/{compact,rewind,notice}.rs`。
  - 行数小，但为了快照 O(1) 取行数，仍缓存为 `BlockPaint`；若实现成本高，可只缓存 `rows` 与文案，正文在绘制时现算，但必须保证 `rows()` 与绘制推进行数一致。

## D. 快照、布局、渲染拆分

- [ ] D1. 新建 `history/snapshot.rs`。
  - `HistorySnapshot { entries, messages, thinking, cards, total_rows }`。
  - `build(area, lx, cache) -> HistorySnapshot`：沿用现有 `layout` 的遍历顺序与空行规则。
  - `PaneCache::snapshot(area, lx) -> Arc<HistorySnapshot>`：列表指纹不变时直接返回 `Arc`。
  - 关键点：末尾呼吸行、消息间空行、块间空行、失败/中断通知、统计行、compact/rewind 的位置逐行对齐现状。

- [ ] D2. 新建 `history/layout.rs`，搬移 `layout()`。
  - 返回类型保持 `HistoryLayout` 不变；从快照映射屏幕矩形。
  - `thinking` / `cards` / `messages` / `total_rows` 字段语义不变。
  - 现有 `history/tests.rs` 与 `history/tests/tool_geometry.rs` 不改断言即可通过。

- [ ] D3. 新建 `history/render.rs`，搬移 `render()`。
  - 只遍历与视口相交的 `entries` 与 `blocks`；整条在视口外时 `continue`，不构造任何行。
  - 块内同样按 `scroll` 做行窗口裁剪。
  - 返回值改为 `snapshot.total_rows`（与 `layout` 同源），并保留“绘制推进行数 == 总行数”的测试。
  - 动态内容在 `paint.draw(...)` 时计算：方块字形、thinking hover、用户选中底色。

- [ ] D4. 精简 `history.rs`。
  - 只保留类型、`StepMark`/`StepKind`、`view`、`scrollbar`、`notice`、folds 辅助。
  - 确认单文件 ≤1000 行，子文件 ~500 行。
  - `pub(crate) use` 重新导出 `content_rect`、`text_rows`、`tool_rows`、`visible_row`、`wrap` 等现有符号，外部调用尽量不改。

## E. 接入 UI 与事件循环

- [ ] E1. `ui::render` 增加 `cache: &mut RenderCache`。
  - `src/ui/mod.rs` → `panes::render_panes` → `main_content::render` → `lx::render` → `build_chat` → `history::view/render` 逐层透传。
  - 更新 6 处测试调用：`event/tests.rs`（2）、`event/tests/lx_toast.rs`、`ui/toast/tests.rs`、`ui/tooltip/tests.rs`，分别传 `&mut RenderCache::default()`。

- [ ] E2. 事件循环持有缓存。
  - `src/event/mod.rs`：`let mut ui_cache = ui::RenderCache::default();`
  - 每次绘制前 `ui_cache.begin_frame()`。
  - 绘制闭包传 `&mut ui_cache`。
  - 窗格集合变化时 `retain_panes`。

- [ ] E3. 合并 resolve。
  - `app/update/chat/viewport.rs`：新增 `resolve_lx_history(state, rects, cache)`，一次 `history_view` 同时写 `viewport_top` 与 `suppressed_prompt_header`。
  - 删除/保留旧函数的决定：**删除** `resolve_lx_sticky_prompt` 与 `resolve_lx_viewport_top`，更新 `app/update/chat/mod.rs` 导出与 `event/mod.rs` 调用。
  - `app/update/chat/tests/history.rs` 三处调用改为新函数。

- [ ] E4. 命中入口接入缓存。
  - `history_hit.rs`：`history_view`、`card_at`、`card_target_at`、`fold_target_at`、`chat_hit`、`history_rows`、`history_scrollbar`、`sticky_prompt_jump`、`history_center_target` 全部接收 `&mut PaneCache`。
  - `card_at` 用快照 + 内容行换算，不再调用 `history::layout`。
  - `event/mouse/{hover,wheel,press,drag,hit}.rs` 与 `event/mod.rs` 传同一个 `ui_cache`。
  - 关键点：鼠标事件与绘制共用同一份快照；事件到达时若状态已变，列表指纹会触发一次重建，保证命中与即将绘制的画面一致。

- [ ] E5. 回归：`cargo test` 全绿，几何测试断言不变。

## F. 工具输出高亮调整

- [ ] F1. `tool.rs`：`output_style(expanded)` 去掉 `hovered`。
- [ ] F2. `tool/command.rs`：`finished_output`、`guide_row`、`blocks`/`rows`、`draw` 去掉 hover 参数。
- [ ] F3. `user_shell.rs`：`sections`、`finished_output`、`draw` 去掉 hover 参数。
- [ ] F4. `tool/file_summary.rs`：`detail` 去掉 hover；剩余行数提示恒用 `style::agent_step_detail()`。
- [ ] F5. `tool/file_diff.rs`：`layout`/`draw` 去掉 hover；提示行不再提亮。
- [ ] F6. `history_hit.rs`：`fold_target_at` 只对 `CardKind::Thinking` 返回目标。
- [ ] F7. 删除失效 API：`LxChat::{tool_hovered_section, tool_is_hovered, shell_hovered_section}`；`history.rs` 中对应的传参；合并卡的 hover 查找。
- [ ] F8. 更新测试：
  - `tool/file_diff/tests.rs::file_diff_hover_region_is_the_whole_folded_block_but_only_the_hint_lifts` 改为“hover 前后样式完全一致”。
  - 命令/本地命令 hover 提亮测试改为“折叠恒 muted，展开恒 text”。
  - 删除只验证 hover 提亮的断言；点击/命中测试保留。
  - thinking hover 测试保持不变。
- [ ] F9. `cargo clippy --all-targets -- -D warnings`：确认没有残留未使用参数/函数/字段。

## G. 测试与性能验证

- [ ] G1. 新增 `history/tests/perf.rs`（`#[ignore]`）。
  - 构造 1000 条消息、约 10 万渲染行、200×50 视口。
  - 场景一：热缓存连续滚动 120 帧（每帧 +20 行），测 `layout + render`，断言 P95 ≤16ms。
  - 场景二：流式追加最后一条消息 120 帧，断言 P95 ≤16ms。
  - 场景三：冷启动构建一次，记录耗时（只记录，不设硬断言）。
  - 断言缓存字节数 ≤32MB。
  - `cargo test --release -- --ignored perf_lx_history` 手动跑一次，把结果写入本文件末尾。

- [ ] G2. 缓存行为单测补齐。
  - 连续两次 layout 只排版一次；
  - 追加正文只重排该消息；
  - 宽度、折叠状态、进程输出变化分别失效；
  - LRU 淘汰后重新构建，`HistoryLayout` 与未淘汰时 `assert_eq!`；
  - 列表指纹对 id/顺序/状态/展开状态敏感。

- [ ] G3. 全量验证。
  - `cargo fmt`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`
  - `cargo test --release -- --ignored perf_lx_history`

- [ ] G4. 用户交互确认。
  - 用户运行 `cargo run`，用 1000 条消息的会话验证：滚轮、拖滚动条、鼠标划过工具卡片、流式输出。
  - 确认工具输出折叠态不再随 hover 变亮，展开态文字正常。

## H. 收尾

- [ ] H1. 回写本文档：勾选完成项、记录性能数字、列出未验证部分。
- [ ] H2. 如实现中发现设计与现状冲突，先回写 `design.md` 的“决策/风险”段落，再继续改代码。
- [ ] H3. 检查 `cargo fmt` 后无大范围无关 diff；不修改 Git 状态（不 commit/push，除非用户明确要求）。

## 基线记录

> 实现前填写：构造方式、消息数、总行数、冷启动耗时、当前滚动帧耗时。

- 基线：待填
- 优化后：待填
