# lx 消息列表滚动对齐 opencode — Review

- **分支**：`lx-scroll-parity`
- **Worktree**：`.worktrees/lx-scroll-parity`
- **基线**：`ab357047`（`dev`）
- **首次实现提交**：`ed76d856`（`fix(lx): align history scrolling with opencode`）
- **Review 日期**：2026-11-12
- **审查范围**：`ab357047..ed76d856` 的全部生产代码、测试与 feature 文档；重点覆盖缓存/快照、主历史与子 agent 详情锚点、滚轮/命中/选区、性能夹具。
- **方法**：3 个只读 subagent 并行审查（缓存与快照、锚点与滚动、详情 UI 与文档），所有结论回到 HEAD 代码与定向测试复核。
- **修复状态**：P0/P1/P2 与可修 P3 已全部落到工作区；未提交 Git。

---

## 0. 关键点

本次功能把 lx 历史从「每帧全量重排 + 绝对行号」推进到「跨帧快照 + 单帧指纹 + 块级锚点」：

1. `RenderCache` 在事件循环的一帧内复用列表指纹，`finish_frame` 关闭记忆，避免鼠标事件复用旧几何。
2. 历史快照的 `messages` 改为 `Arc`，去掉每帧整表克隆；置顶行查找改二分。
3. 块级锚点在快照重建前后按稳定条目 ID + 块来源补偿，主历史与子 agent 详情共用。
4. 缓存淘汰改为单次收集/排序；屏幕外流式合并在末条滚出视口后延迟 80ms 合并。
5. 新增 300 消息 / 600 工具 / ≥3 万行性能夹具，并记录 debug `opt-level=1` 数据。

方向正确，但审查发现：**1 个可 panic 的 ready 快照/实时消息错配（P0）**、一批会让性能目标或阅读位置失效的 P1，以及测试/文档未真正钉住验收口径的 P2。以下全部按优先级记录并修复。

---

## 1. 问题清单

### P0-1 ready 快照与实时消息列表混用，`sticky_candidate` 可越界 panic

**位置**

- `src/ui/main_content/lx/history/sticky.rs:44-56`
- `src/ui/main_content/lx/history.rs:423-451`
- `src/ui/main_content/lx/history/cache.rs:336-365`
- `src/ui/main_content/lx/history_hit.rs:185-207`、`223-265`

**现象**

`snapshot_ready_source` 只校验 owner/width，不校验内容；`view_source_inner` 把旧快照的 `messages` 行区间和**当前** `source.messages` 一起交给 `sticky_candidate`。当会话被 `/resume`、rewind 重载或切换为更短历史时，快照仍是旧的长列表，`rows` 下标可能大于当前 `messages.len()`；`sticky_candidate` 的 `messages[*candidate]` 会 panic。长度相同但内容替换时则会钉到错误 prompt、跳错位置。

**错误原因**

ready 快照可以用来解释「屏幕上已经画出的几何」，但不能和实时消息列表按下标交叉索引。事件循环有 16ms 绘制门控，`chat_hit` / `sticky_prompt_jump` / hover 可以在下一次重绘前读到旧快照；`view_source_inner` 没有长度防御，`sticky_candidate` 也没有 `get` 保护。

**修复**

- `view_source_inner` 在 `snapshot.messages.len() != source.messages.len()` 时跳过 sticky（ready 路径），不生成置顶行。
- `sticky_candidate` 全程改用 `rows.get()` / `messages.get()`，扫描上界同时受 `rows.len()` 与 `messages.len()` 限制。
- 补回归：渲染长会话后截断 `chat.messages`，不重绘直接调用 `history_rows` / `sticky_prompt_jump` / `chat_hit`，断言不 panic 且不返回错误置顶行。

**回归测试**

- `ready_hit_does_not_panic_when_live_messages_shrink`

---

### P1-1 可见流式末条白算一次 non-stream 全量指纹

**位置**

- `src/ui/main_content/lx/history/cache.rs:319-327`、`446-487`

**现象**

`offscreen_stream_throttled` 先计算 `non_stream_signature_for`，之后才用旧快照的末条起始行判断它是否真的滚出视口。末条仍在视口内时，这次 non-stream 扫描必然被丢弃，随后照常重建；读态流式的常见帧固定做 2 次全量列表哈希（full + non-stream），与「每帧单指纹」的目标不符。`snapshot_source_fresh` 在末条可见时也会在重建后计算基线。

**错误原因**

最便宜的可见性判断放在了最贵的扫描之后；基线计算没有按「末条是否 streaming 且整块在视口下方」门控。

**修复**

- 抽出 `streaming_tail_offscreen(width, source)`：要求末条 `status == "streaming"`、宽度/视口已知、follow=false，且旧快照末条 `start >= scroll + viewport_rows`。
- `offscreen_stream_throttled` 先做该判断，再读取 `snapshot_non_stream` 并计算 non-stream。
- `snapshot_source_fresh` 只在 `streaming_tail_offscreen` 为真时建立 `snapshot_non_stream`；末条可见时保持 `None`，等首次滚出视口后正常重建一次再补基线。
- 更新文档：贴底/末条可见为 1 次全量指纹；屏幕外合并的判定帧需要 full + non-stream 两次。

**回归测试**

- `visible_streaming_tail_skips_non_stream_signature`：末条可见时追加内容，断言 `builds +1`、`signature_evaluations +1`。

---

### P1-2 子 agent 详情缓存 owner 用对象地址，run 排序后锚点直接失效

**位置**

- `src/ui/main_content/lx/history/source.rs:88-113`
- `src/app/state/lx/agent.rs:407-448`
- `src/app/update/chat/subagent.rs:56`、`76`、`317`、`769`
- `src/app/update/chat/viewport.rs:286-296`

**现象**

`HistorySource::owner()` 返回 `&LxChat` 地址。`sort_runs_tree_order` 用 `mem::take(&mut self.runs)` 重建 `Vec`，所有 `SubagentRun`（含 `LxChat`）地址改变；下一次 resolve 的 `snapshot_cached` owner 不匹配，直接返回 `None`，不捕获锚点。若同一帧上方内容变高/变矮，读态会跳位；分配器复用地址时 `snapshot_ready_source` 还可能把另一个 run 的快照当成当前 run 的几何。

**错误原因**

把「对象地址」当成「逻辑数据源身份」。run 列表会在启动、列表回投、结束和选中恢复时重排，地址不是稳定 ID。

**修复**

- `HistorySource` 新增 `owner: usize` 字段。
- 主历史：有 `session_id` 时用 session id 的稳定哈希；未绑定会话的草稿退回 `LxChat` 地址。
- 子 agent 详情：新增 `HistorySource::from_run(run)`，用 `agent_id + child_session_id` 的稳定哈希。
- `task/agents.rs`、`history_hit.rs`、`selection.rs` 的详情路径全部改用 `from_run`，禁止混用指针 owner。
- 补测试：两个 run 重排后 `HistorySource::owner()` 不变；选中 run 的详情锚点仍能补偿。

**回归测试**

- `history_source_owner_is_stable_across_run_reordering`
- 现有 `agents_detail_anchor_compensates_when_above_grows` 在 owner 修复后仍通过。

---

### P1-3 主历史 `history_scroll` 不写回钳制值，锚点缺失时长期越界

**位置**

- `src/app/update/chat/viewport.rs:113-202`
- `src/ui/main_content/lx/history.rs:400-407`
- 对比详情分支 `src/app/update/chat/viewport.rs:360-362`

**现象**

`view.scroll` 会把顶部行钳到 `total - body.height`，但 `resolve_lx_history` 只在满足锚点补偿且 `next != base_scroll` 时写 `history_scroll`。宽度变化导致 `snapshot_cached` 返回 `None`、锚点块/消息被删除、或 `delta == 0` 时，`history_scroll` 保留越界旧值，`viewport_top` 却是钳制后的值。之后追加内容会表现为「静默贴底」或延迟一帧才停；下一帧锚点也按越界旧行捕获。

**错误原因**

主历史缺少详情分支已有的 `next_scroll.min(max)` 兜底；视图钳制没有回写状态。

**修复**

- 在最终 `view` 计算后，若 `!follow_bottom`，把 `history_scroll` 同步为 `min(history_scroll, total - body.height)`（等价于 `view.scroll`），变化时置 `changed`。
- 保持 `follow_bottom` 不变；补偿仍不主动恢复跟随。

**回归测试**

- 阅读态把 `history_scroll` 设为超过上限，resolve 后断言 `history_scroll == viewport_top == max_top`；
- 删除锚点消息后 resolve，断言同样钳制且 follow=false。

---

### P1-4 子 agent 详情的 detail-only 变更不置 `rows_dirty`

**位置**

- `src/app/update/chat/subagent.rs:230-259`
- `src/app/update/chat/viewport.rs:284`
- `src/ui/main_content/lx/task/agents.rs:44-55`
- `src/app/update/chat/subagent.rs:933`
- `src/ui/main_content/lx/selection.rs:78-91`

**现象**

`on_subagent_tool_settled` 只在 `settle_tool_result` 返回 true 时置 `run.rows_dirty`；同一终态 call 的迟到 detail 分支把 `changed` 置 true，却不置 dirty。`resolve_task_agents_visible` 只在 `rows_dirty || total_rows == 0` 时量行；绘制路径却因签名（含 tool detail）变化重建快照。结果是绘制/滚动条使用新 `view.total_rows`，而 `run.total_rows`/`visible` 仍是旧值；滚动条拖拽、PageDown、jump bottom 与选区几何都按旧上限钳制，详情锚点也不会触发。

**错误原因**

「内容变化」与「需要重新量行」两个信号分离，只有前者被置位。

**修复**

- 在 detail 更新完成后统一 `if changed { run.rows_dirty = true; }`，覆盖 `settle_tool_result` 与 detail-only 两条路径。
- 补回归：先结算中断，再用同一终态 + 更大的 detail 调用 `on_subagent_tool_settled`，断言 dirty、resolve 后 total 等于绘制快照，滚动可到新末行。

**回归测试**

- `detail_only_settle_marks_rows_dirty`

---

### P1-5 详情区宽度变化时 `should_follow` 旧短路把阅读态改回贴底

**位置**

- `src/app/update/chat/viewport.rs:287-296`、`349-362`
- `src/ui/main_content/lx/task/agents.rs:114-135`
- `src/ui/main_content/lx/history/cache.rs:354-365`
- `src/event/mod.rs:483-485`

**现象**

窗格宽度变化触发 `invalidate_task_agent_rows`；`anchor_before` 因宽度不符返回 `None`，`anchored=false`；`should_follow` 仍含 `(!anchored && run.scroll >= max)`。变宽后 total/max 变小，用户正在旧底部附近阅读时该条件为真，`run.follow` 被写回 true，详情跳到底部并恢复跟随。设计状态表明确要求「窗口宽度变化 → follow 不变」。

**错误原因**

用「scroll 到旧 max」推断「用户想跟随」，没有区分用户主动滚到底与几何收缩造成的钳制。`scroll_agents_detail` / `set_task_agents_scroll` / `jump_lx_history_bottom` 已经在用户主动到底时显式置 follow=true，resolve 里的旧 max 短路是多余且有害的。

**修复**

- 详情 `should_follow` 改为 `run.follow || total <= visible + 1`；删除 `(!anchored && run.scroll >= max)`。
- 保留补偿后的 clamp；不因 clamp 自动恢复 follow。
- 更新 design/task 的详情语义说明：详情与主历史一致，补偿不主动恢复跟随；用户滚到底/点击回到底部时才恢复。

**回归测试**

- `resizing_while_reading_does_not_restore_follow`：follow=false、scroll 接近旧 max，`rows_dirty=true` 后用更宽 rect resolve，断言 follow 仍 false 且 scroll 不超过新 max。

---

### P1-6 尾部块消失时回退 `entry.start`，视口向上跳整条消息

**位置**

- `src/ui/main_content/lx/history/anchor.rs:83-96`
- `src/ui/main_content/lx/history/snapshot.rs:856-877`
- `src/ui/main_content/lx/history/source.rs:77`
- `src/app/update/chat/viewport.rs:163-170`

**现象**

`anchor_row` 在块身份和块序号都找不到时，对 Message 条目一律返回 `entry.start`。`Stats` 是消息尾块；`subagents_running` 变真时所有 assistant 的 Stats 同帧消失。用户正读 Stats 行时，锚点块消失，补偿量约为负的整条消息高度，视口向上跳到消息开头。`RecapRunning`、`Attachments`、`Notice` 等尾部块同理。

**错误原因**

把「正文卡片被折叠/删除」的回退规则套到了一行 footer 上。尾部块的正确近似位置是消息末尾，而不是消息开头。

**修复**

- `anchor_row` 对 `BlockSource::Stats`、`Recap`、`RecapRunning`、`Attachments`、`Notice` 的缺失回退到 `entry.end.saturating_sub(1)`；其余消息内块维持 `entry.start`。
- 补测试：锚定 Stats 后令 `subagents_running=true` 重建快照，断言新行在条目末尾附近，resolve 后视口不向上跳。

**回归测试**

- `tail_block_disappearance_falls_back_near_message_end`

---

### P1-7 置顶摘要先投影整段用户消息，512 字符上限没有限制成本

**位置**

- `src/ui/main_content/lx/history/sticky.rs:60-114`
- `src/ui/main_content/lx/history_hit.rs:185-207`
- `src/ui/main_content/lx/history.rs:423-451`
- `src/event/mouse/wheel.rs:569-577`

**现象**

`sticky_text` 对每个 text part 先 `Projection::new(part.content.trim(), ...).text().to_string()`，之后才 `chars().take(512)`。1 MiB 的 prompt 在每次滚轮/命中 `history_rows` 时都会全量扫描并复制；一次 `view` 内可能执行两次。`SAMPLE_CHARS` 实际没有封住成本。

**错误原因**

截断发生在投影/复制之后；`Projection` 的成本与输入长度线性相关。

**修复**

- `sticky_text` 在投影前先截取前 `SAMPLE_CHARS` 个字符（保留 `@` 原文），再做 `Projection`；空头部继续扫下一个 part。
- 保持最终 512 字符输出上限与空白折叠行为不变。
- 补测试：1 MiB 用户 prompt 的摘要长度仍 ≤512，且不复制整段正文。

**回归测试**

- 现有 `sticky_text_collapses_sanitizes_and_marks_empty` / `sticky_text_is_truncated_with_ellipsis_to_width`
  继续覆盖输出上限；投影前截样由实现与 code review 确认。

---

### P1-8 行数会变化的卡片折叠没有统一 pin 视口

**位置**

- `src/app/update/chat/viewport.rs:498-596`
- `src/app/update/chat/model_switch.rs:24-29`

**现象**

`Thinking`、工具卡、命令卡、compact 都先 `pin_lx_viewport` 再切换；但主历史的 `CardTarget::UserMessage`、`CardTarget::Report`、`CardTarget::ModelSwitch` 直接改展开状态。跟随底部时展开这些卡片，`follow_bottom` 仍为 true，下一帧直接跳到新底部，用户点击的卡片被顶走。

**错误原因**

行数会变化的折叠入口没有统一走 pin；三个分支是历史遗留，但本功能把「展开不顶走阅读位置」列为验收项，必须在本次收口。

**修复**

- `CardTarget::UserMessage`：确认消息存在后先 `pin_lx_viewport`。
- `toggle_report_card`：确认 report 存在后先 pin，再切换/追加批次。
- `toggle_model_switch`：确认 switch 存在后先 pin，再 toggle。
- 补测试：follow=true 时依次展开 user/report/model switch，断言 `follow=false`、`history_scroll == 旧 viewport_top`。

**回归测试**

- `toggling_long_user_message_while_following_pins_viewport`
- `toggling_report_and_model_switch_while_following_pins_viewport`

---

### P2-1 选区几何用 ready 快照，`render_chunk` 却重建非 ready 快照

**位置**

- `src/ui/main_content/lx/selection.rs:93-107`、`478-515`、`637-678`
- `src/ui/main_content/lx/history/render.rs:32-70`
- `src/ui/main_content/lx/history/cache.rs:131-137`

**现象**

`content_geom` 已改为 `view_source_plain_ready`，返回旧快照的 `scroll/total_rows`；`copy_text` / `probe_row_cell` 的 `render_chunk` 仍调用 `history::render_source`（`snapshot_source`）。事件路径在 `finish_frame` 后已关闭指纹记忆，内容一旦变化就会重建新快照，但复制仍用旧几何行号读新内容，可能复制到错位文本；链接/表格探测也会混用新旧几何。

**错误原因**

ready 迁移只覆盖几何入口，没有覆盖同一路径的渲染辅助函数。

**修复**

- 新增 `history::render_source_ready`，内部走 `snapshot_ready_source`；`render_source` 保持绘制路径语义。
- `selection::render_chunk` 改走 `render_source_ready`。
- 补测试：建立几何后修改上方内容，不重绘直接 `copy_text`，断言缓存不重建且复制旧屏幕内容。

**回归测试**

- `copy_text_uses_ready_snapshot_after_live_content_change`

---

### P2-2 主历史 code-copy / thinking 命中用旧快照下标配当前消息列表

**位置**

- `src/ui/main_content/lx/history_hit.rs:268-333`、`363-435`
- `src/ui/main_content/lx/history.rs:244-265`

**现象**

`chat_hit` 已改用 ready 快照，但 `code_copy_at_source` 用旧快照的 `message_index/step_index` 从**当前** `source.messages` 取正文；消息插入/删除/会话切换后可能复制另一条消息的代码。`card_at_source` 对 `CardKind::Thinking` 也只按旧下标返回目标，可能折叠错误的 thinking 块。

**错误原因**

快照的几何下标被当成稳定身份；只有 `SnapEntry.id` 是稳定的。

**修复**

- `code_copy_at_source` 通过 `SnapEntry.id` 解析当前 message；ID 不匹配时按 ID 查找，找不到则拒绝命中，绝不回退旧下标。
- `card_at_source` 对 Thinking 目标用所在条目的 `SnapEntry.id` 解析当前 message index；ID 找不到则跳过该卡片。
- 补测试：渲染后插入一条消息，再命中旧代码块，断言按 ID 复制旧屏幕代码，不误取新消息；
  thinking 的 ID 解析走同一 helper 路径，现有 thinking 命中测试保持通过。

**回归测试**

- `stale_code_copy_hit_resolves_message_by_id`（thinking 的 ID 解析由同一 helper 路径覆盖，现有 thinking 命中测试保持通过）

---

### P2-3 详情 pin 测试用不存在的卡片，断言是空转

**位置**

- `src/ui/main_content/lx/task/tests/agent_list.rs:415-449`
- `src/ui/main_content/lx/history/snapshot.rs:642-653`

**现象**

测试对 `m0`（只有 3 行）直接发 `CardTarget::UserMessage`。快照只在折行 >5 行时登记 UserMessage 点击面，真实 UI 不会产生该 target；`toggle_user_message_expanded` 无条件返回 true 但行数不变，所以测试无论 pin/补偿是否正确都通过。若卡片真在视口上方变高，正确补偿应是 `pinned + growth`，而不是保持 `pinned`。

**错误原因**

测试构造的 target 与实际命中几何脱节，且夹具没有行数变化。

**修复**

- 把可折叠用户消息放到**视口下方**（>5 行），先通过真实 resolve 建立 total/visible，再 toggle，断言 `rows_dirty`、`follow=false`、total 增长且 scroll 保持 pinned。
- 视口上方内容增长由现有 `agents_detail_anchor_compensates_when_above_grows` 覆盖，
  断言 scroll 被补偿 `+growth`。

**回归测试**

- 重写 `toggling_detail_card_while_following_pins_viewport`
- `agents_detail_anchor_compensates_when_above_grows`

---

### P2-4 性能夹具不是文档声称的场景，断言过弱

**位置**

- `src/ui/main_content/lx/history/tests/perf.rs:130-303`
- `src/ui/main_content/lx/history/tests/perf.rs:473-474`
- `docs/feature/lx-scroll-parity/design.md:525-528`
- `docs/feature/lx-scroll-parity/task.md:516-517`

**现象**

- `realistic_history` 所有消息 `status="complete"`，末条不是 streaming，`skip_streaming_tail` 分支从未执行。
- 流式断言用 `builds <= cold + FRAMES`、`evals 增量 <= FRAMES`，零重建也能通过。
- 只测 `view×2 + render`，没有 `resolve_lx_history` 的锚点捕获/补偿，也没有 `history_rows` 滚轮路径。
- 文档命令 `--ignored perf_lx_history` 不匹配 `perf_lx_long_thinking_and_streaming`；并行跑重型 perf 也会互相争 CPU。
- 所有断言只看 P95，没有「无 >50ms 长任务」的 max 检查。

**错误原因**

夹具与断言是「缓存层近似」，不是文档写的「事件循环一帧」。P0/P4 表里的 4ms/8ms/12ms 目标也没有编码为断言。

**修复**

- 末条设为 `status="streaming"`；流式阶段改为精确 `builds == cold + FRAMES`、`evals 增量 == FRAMES`。
- 增加 `assert_no_long_tasks`（max ≤50ms），覆盖滚动/流式。
- 文档命令改为 `--ignored perf_lx_ --test-threads=1`，同时覆盖 thinking 流式用例。
- P0 表注明「观测目标；硬门禁仍是 16ms」，避免把记录值误读为回归断言。
- 记录「输入到画面 <50ms 未自动化」为人工验收项。

**回归测试**

- `perf_lx_history_realistic`（debug/release）
- `perf_lx_history` / `perf_lx_history_expanded_diff` / `perf_lx_long_thinking_and_streaming`

---

### P2-5 共享缓存预算被终端窗格稀释

**位置**

- `src/ui/main_content/lx/history/cache.rs:169-182`
- `src/ui/main_content/panes.rs:68-79`

**现象**

`render_panes` 对每个 Terminal 窗格都调用 `cache.pane(*id)`，即使它当前是终端视图；`PaneCache` 因此为终端窗格也创建条目，`share_budget` 按 `panes.len()` 平摊 32MB。3 个终端窗格 + 1 个 lx 窗格时 lx 只拿到 8MB，与「32MB 由所有 lx 窗格共享」不符。

**错误原因**

预算分母统计了从未使用历史缓存的窗格。

**修复**

- `share_budget` 只统计有历史缓存（`snapshot` / `blocks` / `md_streams`）的窗格；无历史缓存的窗格不参与分母。
- 新窗格首次建立快照后，下一次 `begin_frame` 自动重新平摊。
- 调整现有预算测试并补「终端窗格不稀释」用例。

**回归测试**

- `budget_is_shared_across_history_panes`
- `terminal_panes_do_not_dilute_history_budget`

---

### P2-6 feature 文档仍写「待评审/未实现」，P2 文件列表与实现相反

**位置**

- `docs/feature/lx-scroll-parity/design.md:3`、`8`
- `docs/feature/lx-scroll-parity/task.md:3`、`239-247`、`245`、`533-544`

**现象**

- 文档状态是「待评审」「评审通过前不修改生产代码」，但 HEAD 已有生产实现和 DoD 勾选。
- task.md P2 要求 `src/app/state/lx/chat.rs` 增加 `HistoryAnchor`/`LxChat.history_anchor`，与 design「不落状态」相反；实际锚点在 `ui/main_content/lx/history/anchor.rs`。
- task.md 把新增测试写成 `src/app/update/chat/tests/history.rs`，实际是 `viewport_anchor.rs`。
- 文档中的补偿基准、详情 `needs_measure` 条件、pin 顺序、`anchor_at` 签名与 HEAD 不一致。

**修复**

- 状态改为「已实现（ed76d856）；待人工验收 / 待合并」。
- 按实际文件/函数重写 P2 文件列表与步骤，删除状态字段描述。
- 同步补偿基准 `base_scroll + delta`、`needs_measure = rows_dirty || total_rows == 0`、toggle 后 pin、`anchor_at(snapshot, scroll, width)`。
- 补审查记录链接与本 review 的修复状态。

---

### P3-1 主历史解析路径没有消费 `fresh=false`

**位置**：`cache.rs:282-310`、`history.rs:465-475`、`viewport.rs:147-199`

**说明**：主历史只用 `snapshot_source`，合并帧仍把旧 `view.total_rows` 写入 `viewport_top`。当前合并条件保证末条在视口下方，滚动/钳制通常 no-op；属于契约缺口而非已复现 bug。记录为已知限制，不扩大本次 API 改动。

### P3-2 `evict` 每次超限都排序全表

**位置**：`cache.rs:590-616`

**说明**：设计比较的是「一次淘汰 k 块」的 O(n log n) 与旧 O(n·k)；k=1 时排序比一次 min 扫描贵。实测 1400 条/缓存未超 32MB，未触发；多窗格极端情况下可改为 BTreeMap 或按需 min。记录为已知限制。

### P3-3 读态每帧仍做一次锚点 `find_entry` 线性扫描

**位置**：`anchor.rs:162-207`、`viewport.rs:154-162`

**说明**：内容未变时快照复用，但 `resolve_lx_history` 仍每帧捕获/解析锚点，`find_entry` 对 entries 线性查找并克隆 ID。当前 1400 条规模开销很小；如需进一步优化，可比较快照 Arc 是否变化后跳过锚点。记录为已知限制。

### P3-4 空详情行数口径不一致

**位置**：`task/agents.rs:40-55`、`95-110`、`selection.rs:67-74`

**说明**：空消息流 draw 返回 1 行，`detail_rows` total=0，选区 total=1。修复：无历史时统一按 1 行占位；有 notices/compact 但无消息时走真实快照。

### P3-5 文件差异缓存指纹不含显示宽度

**位置**：`cache/tool_signature.rs:245-257`

**说明**：文件差异行只哈希 `text.len()` 与 kind；同字节长度但 ASCII/CJK 显示宽度不同会引发不同折行行数，却命中旧 `Legacy rows`。修复：文件差异行加入 `unicode_width` 宽度；命令输出保持首尾+长度以控制 500KB 成本。

### P3-6 锚点测试实际锚到下一条消息

**位置**：`history/tests/anchor.rs:18-41`

**说明**：每条消息只有 1 行，`messages[1].start + 1` 落在 message1 与 message2 之间的 spacer，`entry_at` 实际返回 message2。测试没有覆盖块内 offset。修复：使用 3 行块、非零 offset，并断言 `AnchorEntry::Message { index: 1, .. }` 与 `offset > 0`。

### P3-7 工具卡折叠缓存没有失效测试

**位置**：`cache/tests.rs:616-674`

**说明**：只测同状态复用，没有测 `CardSection` 切换后 `Legacy rows`/`CardSpan` 更新。补测试：折叠→展开→折叠，断言 rows/cards 同步变化。

### P3-8 `BlockSource` 不是完全稳定身份

**位置**：`paint.rs:29-74`、`anchor.rs:83-96`

**说明**：`BlockSource` 编码旧 message_index、step_index、group 与 expanded 状态；消息前插后 source 失配，只能回退块序号。当前生产路径以 append 为主，记录为已知限制；本次先修尾部块回退，不做全量稳定 ID 改造。

### P3-9 文档代码片段与实现漂移

**说明**：`design.md` 的 `view.scroll + delta`、`task.md` 的 `anchor_at(..., source, ...)`、详情 pin 顺序等按实际实现更新。

---

## 2. 修复执行清单

| 优先级 | 问题 | 状态 |
|---|---|---|
| P0-1 | ready 快照/实时消息错配 panic | ✅ 已修复 + 回归测试 |
| P1-1 | 可见流式末条白算 non-stream 指纹 | ✅ 已修复 + 回归测试 |
| P1-2 | 详情缓存 owner 地址不稳定 | ✅ 已修复 + 回归测试 |
| P1-3 | 主历史 scroll 不写回钳制值 | ✅ 已修复 + 回归测试 |
| P1-4 | 详情 detail-only 变更不 dirty | ✅ 已修复 + 回归测试 |
| P1-5 | 详情宽度变化恢复 follow | ✅ 已修复 + 回归测试 |
| P1-6 | 尾部块消失回退消息开头 | ✅ 已修复 + 回归测试 |
| P1-7 | 置顶摘要全量投影 | ✅ 已修复（投影前截样） |
| P1-8 | user/report/model-switch 未 pin | ✅ 已修复 + 回归测试 |
| P2-1 | 选区 render_chunk 非 ready | ✅ 已修复 + 回归测试 |
| P2-2 | code-copy/thinking 旧下标 | ✅ code-copy 按 ID 校验/thinking 按 ID 解析 |
| P2-3 | 详情 pin 测试空转 | ✅ 重写为真实行数变化 |
| P2-4 | 性能夹具与断言弱 | ✅ 末条 streaming、精确计数、max 断言、命令修正 |
| P2-5 | 终端窗格稀释缓存预算 | ✅ 已修复 + 预算测试 |
| P2-6 | feature 文档漂移 | ✅ 已同步 design/task |
| P3-1 | 主历史 fresh=false | 记录限制（当前合并条件保证 no-op） |
| P3-2 | evict 排序复杂度 | 记录限制（未超预算触发） |
| P3-3 | 锚点每帧线性扫描 | 记录限制（规模小，后续可跳过无变化帧） |
| P3-4 | 空详情行数口径 | ✅ 统一为 1 行占位 |
| P3-5 | 文件差异指纹显示宽度 | ✅ 已修复 + 回归测试 |
| P3-6 | 锚点测试无效 | ✅ 改为块内 offset |
| P3-7 | 工具卡折叠缓存测试 | ✅ 已补折叠失效测试 |
| P3-8 | BlockSource 稳定身份 | 记录限制（append-only 生产路径可接受） |
| P3-9 | 文档代码片段 | ✅ 已同步实现 |

---

## 3. 修复与验证

修复完成后的工作区验证（未提交）：

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings   # 全绿
cargo test                                  # lib 5250 passed, 0 failed, 6 ignored；integration 全绿
cargo test --lib -- --ignored perf_lx_ --test-threads=1
cargo test --release --lib -- --ignored perf_lx_ --test-threads=1
```

性能复测（单线程）：

| 构建 | 夹具 | 冷构建 | 滚动 P95 | 流式 P95 | expanded diff P95 |
|---|---|---|---|---|---|
| dev (opt1) | 1400 / 102.9k | 355ms | 1.83ms | 2.89ms | 8.23ms |
| dev (opt1) | 300 / 31.8k | 222ms | 0.37ms | 0.72ms | — |
| release | 1400 / 102.9k | 230ms | 1.92ms | 2.83ms | 6.82ms |
| release | 300 / 31.8k | 130ms | 0.33ms | 0.64ms | — |

`realistic` 夹具实际 `total_rows=31800`、`big_result=569999B`、
`builds=121`、`evals=241`；贴底流式精确每帧 1 次指纹 + 1 次重建。滚动/流式采样
max 均 ≤50ms。人工验收仍按 `design.md` §7.2：长会话滚轮、读态流式、回底、
上方卡片展开、子 agent 详情。

## 4. 风险与未覆盖

- `HistorySource` 增加 owner 字段会改变缓存失效时机；切换会话/选中 run 会清空快照，首次绘制多一次全量构建，属预期。
- 详情 `should_follow` 去掉旧 max 短路后，内容收缩钳到 max 不再自动恢复跟随；用户主动滚到底或点「回到底部」仍会恢复。该行为与主历史一致。
- 空详情/只有 notices 的子 agent run 仍需人工确认；本次只统一行数口径。
- 性能断言仍受机器负载影响；已加 `--test-threads=1`，但不承诺跨机器绝对毫秒。
- TUI 交互与观感仍需用户 `cargo run` 实机确认。
