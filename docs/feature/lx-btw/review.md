# `/btw` 功能评审与修复记录

> 评审对象：`feat/lx-btw`（worktree `/Users/yonah/projects/agent/lx-tui/.worktrees/lx-btw`）。
> 评审范围：`app/` 状态与更新、`event/` 数据流、`db/` 持久化、`chat/` 只读上下文、`ui/` 交互与 E2E。
> 结论：**无 P0；P1 已按下方顺序修复并复测，P2/P3 保留为后续项。**
>
> 验证基线：分支内 `cargo test --lib` **5272 passed / 5 ignored**、BTW 相关过滤 **50 passed**；合并后 `dev` 上 `cargo test --lib` **5300 passed / 6 ignored**；clippy/fmt 通过。真实 CLI+PTY E2E 在分支 session `sess-18e74a2bb0702e18-0`（`BTW verify 3`）与合并后 session `sess-18e74b8a3d756da0-0`（`BTW verify 4`）均通过；合并后线程 `btw-18e74b8d87e7df00-1` 最终目标 default。

## P0

无。未发现会导致主工作区数据损坏、跨会话泄漏或不可恢复崩溃的缺陷。

## P1（已修复）

### P1-1 切主会话/关闭 pane 后，BTW 最近一次 checkpoint 之后的正文丢失

- **位置**：`src/event/chat/btw.rs`（`BtwRun`、`handle_net_event`、`finish_run`）。
- **触发**：BTW 流式回答期间用 `/resume` 切走主会话或关闭源 pane；`LxBtw::clear()` 清空可见投影，后续 delta 找不到 thread，`finished` 时 `parts=None`。
- **错误原因**：流内容只存在 UI 投影里。短回答在 500ms 节流 checkpoint 前完成时，DB 里没有正文，`finish_with_optional_parts(None)` 也不会补写。
- **修复**：`BtwRun` 增加独立的 `LxChat` 缓冲，所有 `ChatDelta` 同时写入该缓冲；checkpoint 与结算在 UI parts 为空时改用缓冲 parts。关闭 pane 不再中断后台生成。
- **验证**：`finish_run` 的 parts 回退路径；E2E 仍能在切屏后加载 DB 正文。该路径暂无专门的“切会话中途”PTY 用例，见 P2-9。

### P1-2 另一实例启动恢复会把正在运行的 BTW 标记为 interrupted

- **位置**：`src/event/chat/btw.rs::recover_startup`、`src/db/chat/btw/recovery.rs`、`BtwRun`。
- **触发**：实例 A 的 BTW 在流式输出，实例 B 打开同一个 DB 并启动。
- **错误原因**：启动恢复直接全库更新 `source='btw'` 的 streaming/queued/running 行，没有使用主轮/子 agent 已有的跨进程 session guard。
- **修复**：BTW child session 与主轮共用 `try_lock_session`；`BtwRun` 持有 guard 直到结算；启动恢复只处理拿得到 guard 的 child session，并按 session 过滤更新消息、recipient、batch。
- **验证**：`btw_sessions_needing_recovery` + `recover_btw_startup_for` 单测/启动路径；E2E 通过。双实例并发场景仍未做专门集成测试。

### P1-3 回答者结算跨两个事务，第二次失败会把线程永久卡在 running

- **位置**：`src/db/chat/btw.rs::finish_btw_recipient`、`src/db/chat/mod.rs::finish_with_optional_parts_in_tx`。
- **触发**：第一次事务把 assistant 写成 complete 后，recipient/batch 更新遇到 SQLite busy 或进程中断。
- **错误原因**：`finish_with_optional_parts` 自己提交一次，随后 `finish_btw_recipient` 再开第二个事务；两段之间没有原子性，run 又被移除，后续同线程批次永久被 `active_batches` 挡住。
- **修复**：抽出接受外部 `Transaction` 的 `finish_with_optional_parts_in_tx`；BTW 的消息终态、usage、recipient、batch、thread 更新时间全部放进同一事务。
- **验证**：`db::chat::btw` 9 项测试通过；结算失败后的重试策略仍是 P2 风险。

### P1-4 认领回答者失败被当成“已被认领”，UI 永久 loading / recipient 永久 queued

- **位置**：`src/event/chat/btw.rs::start_recipient`、`src/db/chat/btw/recovery.rs::fail_btw_recipient`。
- **触发**：`start_btw_recipient` 返回 SQLite 错误；或恢复/其他执行流已结算该 recipient。
- **错误原因**：`.ok().flatten()` 把 `Err` 和 `None` 合并；pending 已从队列移除，却又没有回投事件，`pending_recipients` 永不归零。
- **修复**：区分 `Ok(Some)` / `Ok(None)` / `Err`；`Err` 时把 recipient 标 failed 并收敛批次，三种情况都回投失败事件清掉等待标记。
- **验证**：DB 收敛方法有覆盖；真实 SQLite 错误注入未单独做。

### P1-5 `/btw prompt` 会提交到上次浏览的旧 QA，而不是最新主 QA

- **位置**：`src/app/update/chat/btw/mod.rs::try_submit_btw_command`、`src/app/update/chat/btw/events.rs::on_qa_list_loaded`。
- **触发**：QA1 用过 BTW（`active=QA1`）→ 主会话新建 QA2 → 输入 `/btw 新问题`。
- **错误原因**：命令入口复用了“保留当前 active”的浏览语义；QA 列表回投后只有 `active.is_none()` 才选最后一条，新问题被送进 QA1。
- **修复**：带 prompt 的命令式 `/btw` 先清空 `active`（裸 `/btw` 保留浏览语义）；测试断言 Submit 的 `qa_id` 是最新 QA。
- **验证**：`btw_command_with_prompt_switches_to_the_latest_qa`。

### P1-6 BTW composer 的选区/滚动/复制/粘贴写到隐藏主输入框

- **位置**：`src/app/update/selection.rs`、`src/app/update/pane.rs`、`src/event/input.rs`、`src/event/mouse/wheel.rs`。
- **触发**：在 BTW 输入框点击/拖选、滚轮、Ctrl+C/X、粘贴。
- **错误原因**：`input_geom` 已按 `active_editor()` 计算 BTW composer 几何，但这些 update helper 仍直接操作 `pane.lx`，没有经过 `with_active_editor`；粘贴又被 `task.is_some()` 分支提前吞掉。
- **修复**：输入选区 begin/drag/end、take、输入视口滚动/滚动条、复制/剪切全部走 `with_active_editor`；粘贴在 task 分支前优先路由到 BTW composer；BTW composer 的滚轮在 list 区域滚 active editor；Scrollbar/copy 仍保留主内容区路径。
- **验证**：新增 `btw_composer_selection_and_scroll_stay_in_the_active_editor`、`lx_paste_targets_the_btw_composer_not_the_hidden_main_input`。

### P1-7 `default` 目标哨兵被当成 agent ID，显示 `To: default` 并误报 reset

- **位置**：`src/event/chat/btw.rs::resolve_target_views` / `target_view_for`。
- **触发**：重启后打开只由默认助手回答过的线程。
- **错误原因**：DB 的 `answerer_set_json` 用 `"default"` 表示默认助手，但加载时无条件生成 `agent_id=Some("default")`；后续无 `@` 追问在 `agents.runs` 找不到该 ID，错误触发 `BTW targets reset to lx`。
- **修复**：加载时把 `"default"` 映射为 `BtwTargetView::default_assistant()`（`agent_id=None`，显示 `lx`）；旧线程目标重新加载后不再误报。
- **验证**：`default_target_key_maps_to_the_default_assistant_not_an_agent_id`、`default_target_sentinel_renders_as_lx_not_an_agent_id`。

### P1-8 迟到的线程加载没有父会话/当前 QA 校验，会改回旧 active

- **位置**：`src/event/chat/btw.rs::BtwEvent::ThreadLoaded`、`src/app/update/chat/btw/events.rs::on_thread_loaded`。
- **触发**：LoadThread 后立刻切主会话/切 QA，旧回投到达。
- **错误原因**：事件只带 `pane/qa`，`ensure_thread` 还会改 active；旧快照能覆盖新会话视图。
- **修复**：`ThreadLoaded` 携带 `parent_session_id`；应用层校验父会话与 `active == qa_id`，非当前 QA 的迟到加载直接丢弃；保留 `submit_seq > 0` 的目标竞态保护。
- **验证**：`stale_thread_loaded_does_not_overwrite_another_session_or_qa`。

## P2（部分已修，未完成项保留）

### P2-1 每次提交只有一个 `pending_submit`，快速 Enter/切 QA/ACK 前 Stop 会丢事件（未修）

- `src/app/update/chat/btw/mod.rs` 覆盖 `pending_submit`；`Accepted`/`Failed` 不带 `qa_id` 返回路径，`on_accepted` 只找当前 active thread，切 QA 后 ACK 丢失。
- 影响：两个用户气泡乱序、原线程 loading 不消失、ACK 前 Stop 产生孤儿回答。
- 建议：事件携带并校验 `qa_id`，按线程维护 FIFO 在途提交集合，Stop 延迟到 ACK 后取消。

### P2-2 上一次目标部分缺失时静默缩小回答者集合（未修）

- `src/app/update/chat/btw/mod.rs::previous_targets` 只跳过缺失项；`missing && previous.is_empty()` 才提示回退。
- 设计 §10.3 要求“任一目标无效整批不提交，草稿与旧集合都保留”。当前 `[Turing, Hopper]` 缺 Hopper 会静默只发 Turing。
- 建议：`missing` 为真即拒绝并提示；若产品希望自动降级，必须显式展示删除项并二次确认。

### P2-3 同一主会话在多个 pane 打开时只更新第一个 pane（未修）

- `src/app/update/chat/btw/events.rs` 的 `on_recipient_started/on_delta/on_tool_state/on_finished` 命中第一个 `thread_by_child_session_mut` 后立即 `return`。
- 影响：第二个 pane 永远 streaming、loading 不消失。
- 建议：遍历所有 pane 并返回变更并集。

### P2-4 `/btwList` 面板没有指针优先级（未修）

- 面板绘制在对话区上方，但 `event/mouse/press/lx.rs` 的滚动条/代码复制/回到底部命中先于 picker；`wheel.rs` 也没有 picker 分支。
- 影响：点击面板右缘会命中底层对话滚动条，滚轮滚隐藏对话。
- 建议：press/wheel 先判 `picker_layout().rect`，命中即消费并只允许切 QA/滚动面板。

### P2-5 启动恢复把部分完成批次统一标 `interrupted`（未修）

- `src/db/chat/btw/recovery.rs` 对所有 queued/running recipient 标 interrupted，batch 直接 interrupted。
- 设计 §11.4 要求有成功有失败为 `partial`；需按 recovery 后 recipient 状态聚合 batch。

### P2-6 BTW 忽略 pane 禁用的 MCP server，且 MCP 配置根未冻结（未修）

- `src/event/chat/btw.rs::start_recipient` 传空 disabled 集合，并按当前 `project_root_for_pane` 取快照。
- 影响：用户禁用的 server 可能重新进入只读工具面；切换工作区后排队批次使用另一份配置。
- 建议：把 disabled 集合与 MCP config root 冻结进 `BtwSubmission`。

### P2-7 已完成 subagent 的报告正文没有注入 BTW 上下文（未修）

- `agent_references` 只拼 title/status/child_session，未注入 `report_json` 或最后工具结果。
- 影响：`@Turing` 的“结论”不可直接引用，模型只能重新读文件。
- 建议：注入有界 report 与最后一次完成工具结果，保持只读。

### P2-8 BTW 工具回放预算 2 KiB 且丢调用参数（未修）

- `src/chat/btw.rs` 将工具结果裁到 2 KiB（设计 §9.3 为 64 KiB），且只渲染 name/status/result，不保留 arguments。
- 影响：多目标追问无法知道“读了哪个文件/搜了什么”。
- 建议：沿用共享 64 KiB 限额并输出 `arguments_json`。

### P2-9 E2E 断言仍可能命中旧屏文本（未完全修）

- `tests/btw_verify.rs` 已修掉“fresh seed 却先断言 thread 必须存在”的矛盾，并把批次断言限定到目标线程；但 `assert_wait("42")` / `SIDE_OK` 仍是整屏 substring，复用有 BTW 历史的 session 会假通过。
- 建议：每次运行生成 nonce，提交后断言新 batch/recipient/assistant ID，再按 BTW 输出区取文本。

### P2-10 `src/event/chat/btw.rs` 仍超过 1000 行（未修）

- 当前约 1370 行；`src/db/chat/btw.rs` 已把恢复逻辑拆到 `src/db/chat/btw/recovery.rs`，事件层仍单文件。
- 建议：按 `submit/runtime/settle/tool/host/recovery` 拆分，测试跟随子模块；这是项目硬规范，不应继续在根文件加逻辑。

### P2-11 本轮顺手修复但未在 P1 单列

- `src/app/state.rs::has_streaming_lx` 纳入 BTW loading，底部扫描不再静止；`loading_label` 改为按 streaming assistant 判断工具/思考。
- BTW 焦点切换后 composer 硬编码 focused 导致光标常亮；`draw_bottom` 现在按 `focused && focus==Composer` 返回光标。
- 空 QA 时 `/btw prompt` 现在把正文回填主输入，不静默丢失。
- `ThreadLoaded` 空白 QA 列表路径不再清空 pending prompt。

### P2-12 设计承诺但首版未实现的功能（未修）

- `Send to main` 转交、异步图片/剪贴板 owner 校验、完整 MCP resource 桥未实现；task.md §11.2 已明确列为遗留。
- `list_agents` 与条件式 `web_search` 未进入实际工具面，design §10.1 已加现状标注。
- 这些不是运行时回归，但验收前必须明确是否属于本次范围。

## P3（未修，记录）

- `reject_host_request` 对 `Mutation` 返回 false，事件层会回落到主 mutation 路径；当前工具面没有写工具可达，但违反设计 §11.3 的双层拒绝。
- 多目标共享 `ChatStatus` / `step_usage`，任一 recipient 的 `ResponseFinished` 会重置另一路聚合；`BtwThreadState.chat.request` 不是多路状态的正确载体。
- `Shift+Tab` 在 BTW composer 仍映射为 `PrevStep` 后丢弃，`BtwFocus::previous` 无调用者。
- 窄/矮布局：目标放不下时整段丢弃 `To:`；面板高度 6..=8 未完全按窗格高度夹取。
- design §6.1/6.2 仍保留旧的多行 QA 列表/焦点描述；§10.1 的 `list_agents`、条件 `web_search` 在首版未接入；§13 的拆分计划未完成。设计文档已加“历史方案/现状”标注，详细实现以本节与 task §21 为准。

## 复测命令与结果

```bash
# 单元/集成基线
cargo test --lib                     # 5272 passed, 5 ignored
cargo test --lib btw                 # 50 passed
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# fresh CLI seed 后全链路（真实模型）
BTW_VERIFY_WS=/Users/yonah/.lx-t-dev/verify/lx-btw-cli-20261112 \
BTW_VERIFY_DB=/Users/yonah/.lx-t-dev/db/worktrees/lx-btw-df3a5393/lx.db \
BTW_VERIFY_TITLE='BTW verify 3' \
  cargo test --test btw_verify btw_chat_and_agent_mention_e2e -- --ignored --nocapture --test-threads=1

# 当前线程缺失目标回退
BTW_VERIFY_TITLE='BTW verify 3' ... \
  cargo test --test btw_verify btw_missing_previous_target_falls_back_e2e -- --ignored --nocapture --test-threads=1
```

结果：全链路 8.56s 通过；回退 7.34s 通过；`btw-18e74a342652ee60-1` 最终 `answerer_set_json=["default"]`，最后批次 `btw-1794484584713-2-1` completed、目标 default、徽标 `To: lx`。主库/主工作区会话未修改。
