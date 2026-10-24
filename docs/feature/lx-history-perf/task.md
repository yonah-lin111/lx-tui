# lx 消息列表滚动性能优化任务清单

> 状态：**代码已完成，待用户 `cargo run` 确认交互手感**。
>
> 分支 `feat/lx-history-perf`，工作区 `.worktrees/lx-history-perf`。

## A. 预备与基线

- [x] A1. 用 codegraph / codebase-memory 复核索引覆盖：主库 `ready`，`parse_partial=0`、`skipped=0`；工作区另建索引 `lx-tui-history-perf`。
- [x] A2. 改动前基线测试：`cargo test` 全绿（exit 0）。
- [x] A3. 基线性能：未能在改动前采集（工作区已建分支）；**优化后**数字见文末「性能记录」。

## B. 缓存底座

- [x] B1. `history/cache.rs`：`RenderCache{frame, panes}`、`PaneCache{frame, snapshot, builds, blocks, bytes, limit}`、`BlockKey`、`BlockEntry`、32MB LRU。
  - 关键实现：`mark_used` 用**绘制路径**刷新 `last_frame`，`block()` 命中**不**刷新——否则一次流式追加会把全部缓存标成「本帧使用」，LRU 永远无法淘汰。
- [x] B2. `history/paint.rs`：`BlockPaint::{UserText, Markdown, Thinking, Static, Legacy}`，`rows()` 与绘制推进行数同源。
- [x] B3. `Fnv`（自写 FNV-1a 64）+ `write_edges`（首尾 64 字节）；`list_signature` + 各块指纹。
- [x] B4. 缓存单测：`history/cache/tests.rs`，13 条。

## C. 块绘制数据

- [x] C1. 用户正文：`wrap()` 结果进 `BlockPaint::UserText`。
- [x] C2. 助手 markdown / thinking：`markdown_render::render_wrapped` 结果进 `Markdown`/`Thinking`，绘制只遍历已排好的行。
- [ ] C3. 命令卡片 —— **未接入**，走 `BlockPaint::Legacy`。
- [ ] C4. 文件工具摘要 —— **未接入**，同上。
- [ ] C5. 文件差异卡片 —— **未接入**，同上。
- [ ] C6. 计划卡片 —— **未接入**，同上。
- [ ] C7. 本地命令卡片 —— **未接入**，同上。
- [ ] C8. compact / rewind / notice —— **未接入**，同上。

> 为什么 C3~C8 先不做：命令参数/输出、文件摘要都有行数上限（`DETAIL_MAX_ROWS = 24`），
> 本地命令折叠态只有预览 + 提示行；唯一没有上限的是**展开的文件差异**。
> 实测展开 2000 行差异卡（200×50 视口）滚动 P95 = 6.04ms，仍在 16ms 预算内，
> 且已由 `perf_lx_history_expanded_diff` 钉死。等出现「多个大差异卡同屏」的真实反馈
> 再给 `file_diff` 接 paint，属于有明显收益才做的优化，不是现在的欠账。

## D. 快照、布局、渲染拆分

- [x] D1. `history/snapshot.rs`：`HistorySnapshot{entries, messages, thinking, cards, total_rows}` + `SnapEntry{start, end, kind, blocks}` + `SnapBlock{start, rows, paint, key, source}`。
  - **快照只依赖「内容宽度 + 列表指纹」**，不含 area 高度/y/滚动：否则 `card_at` 用 `view.body` 会每帧多建一份。
- [x] D2. `history/layout.rs`：快照 → 屏幕矩形，返回类型与字段语义不变。
- [x] D3. `history/render.rs`：**三层裁剪**（条目 → 块 → 块内行窗口），返回值等于 `snapshot.total_rows`。
- [x] D4. `history.rs` 949 → 386 行；子文件 `cache.rs` 642、`snapshot.rs` 584、`paint.rs` 505，均 < 1000。

## E. 接入 UI 与事件循环

- [x] E1. 缓存透传 `ui::render_cached` → `panes` → `main_content` → `lx` → `history`。
  - **偏差**：`ui::render(frame, state, config)` 保留三参签名（内部建临时缓存）。设计里写「6 处测试调用」，
  - 实际 `tests/` 下有 8 个集成测试文件依赖三参签名，改签名要污染全部集成测试，收益为零。
- [x] E2. 事件循环持有 `ui_cache`：`retain_panes` → `resolve_*` → `begin_frame()` → `render_cached`。
- [x] E3. 合并 resolve：`resolve_lx_history` 一次 `history_view` 同时写 `viewport_top` 与
  `suppressed_prompt_header`；删除 `resolve_lx_sticky_prompt` / `resolve_lx_viewport_top`。
- [x] E4. 命中入口全部收 `&mut PaneCache`；`card_at` 改用快照 + 内容行换算，不再调 `history::layout`。
- [x] E5. 回归：几何测试断言未改。

## F. 工具输出高亮调整

- [x] F1~F7. `output_style(expanded)`；命令卡/本地命令/文件摘要/差异卡去掉 hover 参数；
  `fold_target_at` 只对 `CardKind::Thinking` 返回；删除 `tool_hovered_section`/`tool_is_hovered`/`shell_hovered_section`。
- [x] F8. 提亮断言改为「折叠恒 muted、展开恒 text」；thinking hover 测试不动。
- [x] F9. `clippy -D warnings` 通过。

## G. 测试与性能验证

- [x] G1. `history/tests/perf.rs`，两条 `#[ignore]`：`perf_lx_history`、`perf_lx_history_expanded_diff`。
- [x] G2. 缓存行为单测：只重排变化的块（`Arc::ptr_eq`）、宽度/折叠/工具状态失效、
  LRU 淘汰后行数一致、视口内块不被淘汰、hover/加载帧不触发重建。
- [x] G3. `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` / release 性能跑通。
- [ ] G4. **用户交互确认**：`cargo run` 用长会话验证滚轮、拖滚动条、鼠标划卡片、流式输出。

## H. 收尾

- [x] H1. 回写本文档与 `design.md` 的实现偏差。
- [x] H2. 偏差已记录（见 E1、C3~C8）。
- [x] H3. 改动按阶段提交在 `feat/lx-history-perf` 分支；未 push、未合并到 `dev`。

## 性能记录

环境：release，macOS aarch64，200×50 视口。

| 场景 | 规模 | 结果 |
|------|------|------|
| 冷启动建快照 | 1400 条消息 / 100,800 行 | 53.2ms（一次性） |
| 热缓存滚动（120 帧，每帧 +20 行） | 同上 | **P95 1.05ms** |
| 流式追加（120 帧，每帧追加一行） | 同上 | **P95 1.54ms** |
| 展开 2000 行文件差异滚动（Legacy 路径） | 2,106 行 | **P95 6.04ms** |
| 二级缓存占用 | 1400 条消息 | 13.58MB（上限 32MB） |

复跑：

```bash
cargo test --release --lib -- --ignored perf_lx_history --nocapture
```

## 遗留

1. `cache.rs`（642 行）与 `snapshot.rs`（584 行）略高于 500 行指引，可按指纹/构建职责再拆。
2. 工具卡片（C3~C8）仍是 `Legacy` 路径；展开的大差异已量到 6ms，未超预算。
3. 指纹对文件差异只覆盖 hunk 数与每 hunk 行数，等长改写且首尾 64 字节相同会漏判（当前无此写入路径）。
