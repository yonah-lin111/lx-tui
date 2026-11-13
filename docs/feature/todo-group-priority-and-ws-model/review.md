# Todo 聚合组优先级展示 + 工作区模型记忆：Review

- 审查范围：`dda53729..HEAD`，即 `c84347b4`（Todo 聚合组）、`3bc0bd52`（工作区模型/回退链）、`3567f598`（新建窗格状态补刷）。
- 审查方式：三路并行只读审查（Todo、模型/持久化、窗格/文档）+ 主线复核；对照 `design.md`、`task.md`、`opencode-dev` 的 `local.tsx` / `model-preference.ts`。
- 结论：**无 P0**；P1 4 项、P2 7 项、P3 14 项。P1/P2 已在本分支修复；P3 中影响正确性的低风险项已修，持久化并发加固与 Todo 大组性能留作后续（见每项状态）。

## 0. 关键结论

1. **Todo 聚合组主干正确**：组范围、幽灵行偏移、锚点行、`+6/+9/+12` 几何、最高优先级并列取最靠前，经代码推演与测试均自洽；没有发现会写错条目的生产缺陷。真正缺的是“点徽标 → 改归属条目”的端到端测试。
2. **`no model` 不是回退链本身的问题**：`LxChat.status` 是缓存快照，而 `Pane::new()` 默认 lx 视图、状态为空；新建/激活路径漏刷状态才会显示 `no model`。`3567f598` 修了主要路径，但仍漏了“工作区历史列表新建并激活”这一条。
3. **“工作区记忆”此前实际是“窗格记忆”**：新标签/分割窗格不会继承本工作区最近使用的模型，只会落到全局 `defaultModel → recentModels → 首个可用`，与 D1 和 opencode 的 location memory 语义不一致。
4. **回退链与窗格选择的校验口径不一致**：fallback 校验“启用 + 有 Key”，窗格选择只校验 provider/model 存在；供应商停用或 Key 被清后，失效选择会压住回退链，发送时反而提示 `no model`。
5. **文档落后于实现**：函数名、签名、覆盖路径、重启回退顺序和测试数量均有偏差；`verification.md` 把“只刷目标工作区”写成“其它窗格提示都保留”，实际同工作区其它标签不成立。

## 1. P0

无。未发现会导致数据损坏、崩溃、错误覆盖用户数据或不可恢复状态的问题。

## 2. P1

### P1-1 工作区历史新建并激活工作区不刷新 lx 状态

- **位置**：`src/app/update/workspace_history/mod.rs` `open_new_history_workspace`。
- **现象**：`ws list` 打开一个尚未打开但已信任的历史工作区，新工作区默认 lx 欢迎页底边框停在 `no model`。
- **根因**：该路径直接 `Workspace::single_terminal` + 激活，末尾只有 `clear_selection` / `exit_workspace_history` / `focus_panes`，没有调用状态刷新；`3bc0bd52` 只覆盖了 `create_workspace_with_cwd` 和 worktree 直开。
- **修复**：`focus_panes` 后调用 `refresh_active_tab_lx_status`。
- **测试**：新增 `opening_new_history_entry_initializes_lx_status`。
- **状态**：已修复。

### P1-2 新标签/分割窗格不继承本工作区最近使用的模型

- **位置**：`src/app/update/workspace.rs` `create_tab`、`src/app/update/menu.rs` `split_pane`、`src/app/state/workspace.rs`。
- **现象**：工作区 A 的窗格用 `/resume` 使用 GPT，工作区 B 后来 `/model` 选 Claude；回到 A 新建标签/分割窗格，新窗格显示 Claude（全局 `defaultModel`），不是 A 的 GPT。
- **根因**：D1 要求工作区级记忆，实现只有窗格级 `chat.selection` + 全局 fallback；新窗格 `selection=None`，自然落到全局链。`design.md` 的“各 lx 窗格的 `chat.selection`”只能记住已有窗格，不能覆盖新窗格。
- **修复**：`Workspace` 新增进程内 `lx_model`；在 `/model`、发送、`/resume`、steer 时更新；`create_tab`、`split_pane`、`create_terminal_in_workspace` 创建新窗格时先继承 `lx_model`，再刷新状态。仍保留窗格级 selection 作为会话/窗格覆盖。
- **测试**：`create_tab_inherits_workspace_model_over_default`、`split_pane_inherits_workspace_model_over_default`、`model_selection_updates_workspace_model_memory`、`try_send...`/`session_loaded...` 的工作区记忆断言。
- **状态**：已修复。

### P1-3 窗格 selection 的可用性判定宽于 fallback

- **位置**：`src/app/state/lx/chat.rs` `effective_selection` vs `validated_selection`。
- **现象**：`/model` 选 Claude 后在设置页停用该供应商或清空 Key，状态行仍显示 Claude；按 Enter 时 `FrozenModel::resolve` 失败并提示 `no model`，即使其它模型可用。
- **根因**：窗格分支只检查 `providers` + `models` 是否存在，没有检查 `enabled_providers` 和 API Key；与 fallback 的 `validated_selection` 两套口径。
- **修复**：`effective_selection` 的窗格分支改用 `models.validated_selection`；失效时走 fallback。`validated_selection` 同时把失效 variant 规范化为模型默认等级，而不是丢弃模型。发送/steer 钉住的是本轮实际使用的模型（含失效选择回退结果），避免流启动重新解析时与 DB 冻结模型不一致。
- **测试**：`effective_selection_falls_back_when_pane_provider_disabled_or_key_missing`、`fallback_selection_normalizes_stale_recent_variant`。
- **状态**：已修复。

### P1-4 Todo “点徽标 → 改归属条目”缺少端到端测试

- **位置**：`src/ui/right_panel/todo/tests/grouping.rs`、`src/app/update/todo.rs` `click_todo_priority`。
- **现象**：原测试只验证 `priority_at` 返回值；若 `priority_at` 改回返回被点行，或幽灵行移位写错，测试仍全绿，但点击会改错条目。
- **根因**：命中层与更新层的隐式契约没有测试守。
- **修复**：新增 `clicking_group_badge_cycles_and_selects_badge_owner`、`clicking_group_badge_with_ghost_commits_then_updates_owner`，把 `priority_at` 的真实返回值喂给 `click_todo_priority`，断言锚点未被误改、归属条目被循环、选中正确。
- **状态**：已修复。

## 3. P2

### P2-1 `/resume` 把失效会话模型写入 recent

- **位置**：`src/app/update/chat/session.rs` `on_session_loaded_full`、`src/app/update/chat/slash.rs` `remember_model_used`。
- **现象**：恢复一个供应商已删除/停用/无 Key/variant 已移除的旧会话，失效模型被推到 `recentModels` 队首；连续恢复 10 个失效会话可挤掉全部有效历史。
- **根因**：`remember_model` 是纯列表操作，调用方误以为传入值一定可用；`/resume` 直接使用 DB 快照。
- **修复**：`remember_model_used` 先用 `validated_selection` 过滤/规范化；`on_session_loaded_full` 只对有效选择更新 recent 和工作区记忆。窗格仍保留 DB 原始 selection，以便供应商恢复后再次生效。
- **测试**：`session_loaded_skips_invalid_model_in_recent`；原 `session_loaded_records_recent_model_without_touching_default` 改为在 `default_model=None` 下断言，避免“写成同一个值”的空断言。
- **状态**：已修复。

### P2-2 失效 variant 导致整个模型被跳过

- **位置**：`src/app/state/lx/chat.rs` `validated_selection`。
- **现象**：`recentModels=[p/m(high)]`，之后 `high` 被移除，新工作区不会显示 `p/m` 的默认等级，而会跳到下一条或首个可用模型。
- **根因**：校验阶段把 `variant_ok` 当硬条件，没有复用 `normalize_selection`。
- **修复**：`validated_selection` 只校验 provider/模型/启用/Key，variant 交给 `normalize_selection` 回落默认。
- **状态**：已修复（见 P1-3 测试）。

### P2-3 recent/default 变化后空白窗格状态行不同步

- **位置**：`src/app/update/chat/slash.rs` `remember_model_used` / `persist_default_model`。
- **现象**：`defaultModel` 为空时，pane1 空白页显示首个可用 Claude；pane2 `/resume` GPT 后 recent=[GPT]；pane1 仍显示 Claude，但实际发送会解析为 GPT。
- **根因**：fallback 依赖全局可变状态，但更新只刷新触发动作的窗格。
- **修复**：新增 `refresh_fallback_lx_status`，在 default/recent 实际变化后只刷新“无消息且无窗格选择”的 lx 窗格；这类窗格不可能持有一次性 `omitted_turns_notice`，不会误清提示。
- **状态**：已修复。

### P2-4 工作区刷新遍历所有标签，可能清掉后台标签的一次性省略提示

- **位置**：`src/app/update/chat/mod.rs` 原 `refresh_active_workspace_lx_status`、`src/app/state.rs` `active_pane_ids`。
- **现象**：后台标签的窗格已有 `status.omitted_turns` 但尚未展示，切工作区再切回来提示消失。
- **根因**：刷新粒度为“工作区全部标签”，而 `refresh_status` 会用 `take()` 重建 `ChatStatus`，一次刷新把 notice 转入 status，下一次刷新就清空。
- **修复**：改为 `refresh_active_tab_lx_status`，只刷当前标签；`switch_tab` 切到新标签时补刷。`switch_workspace` 同索引时跳过状态刷新，避免无意义消费。
- **测试**：现有 `switch_and_create_workspace_refresh_active_lx_status_model` 继续通过；新增历史新建路径测试。
- **状态**：已修复。

### P2-5 文档/验证报告与实现不一致

- **位置**：`design.md`、`task.md`、`verification.md`。
- **问题**：
  - 文档函数名 `refresh_workspace_lx_status(state, workspace)` 不存在；实际是 `refresh_active_tab_lx_status(state)`。
  - 创建入口写成 `create_workspace`，实际是 `create_workspace_with_cwd`。
  - `design.md` §5.6 重启回退顺序写成 `recentModels[0] → defaultModel`，与 D4/实现相反。
  - `3567f598` 的四个补刷路径没有写进 design/task，只在 verification 追补。
  - `verification.md` “其它窗格提示保留”只对其它工作区成立。
  - 测试数量 5313 已过期（当前 5331）。
- **修复**：本次同步更新三份文档，函数名改为符号引用，补充工作区记忆、新建窗格、历史路径、`omitted_turns` 作用域和 review 链接；删除易漂移的行号。
- **状态**：已修复。

### P2-6 组徽标归属条目在视口外时，点击会滚动到归属条目

- **位置**：`src/ui/right_panel/todo/item.rs` `group_priority`、`src/app/update/todo.rs` `click_todo_priority`。
- **现象**：大组锚点可见、最高优先级成员在视口外；点击徽标后 `select` 滚到隐藏条目，锚点被移出视野。
- **判断**：这是 D5“点击切换决定徽标的条目并选中它”的直接结果，属于**产品行为确认**而非实现错误。点击后滚动到真正被修改的条目，能让用户看到改动对象；若只改不滚，用户看不到反馈。
- **处理**：不修改代码；在 `design.md` §4.6 明确“点击会选中并滚入归属条目”。
- **状态**：已确认/文档化。

### P2-7 `verification.md` 的已知边界写得过宽

- **位置**：`verification.md` 已知边界。
- **问题**：“其它窗格 `omitted_turns_notice` 保留”只覆盖其它工作区；“上述创建/切回路径立即刷新”不包含历史新建；未披露“新窗格继承工作区模型”的新语义。
- **修复**：改为按“当前标签/其它工作区/历史路径/工作区记忆”分别陈述，并补 `review.md` 链接。
- **状态**：已修复。

## 4. P3

| 编号 | 问题 | 根因 | 处理 |
| --- | --- | --- | --- |
| P3-1 | `group_priority` 每行重算 role 并二次扫描组，大组/滚动有额外 O(G) | helper 从 display 反推 role，未复用 `VisibleItem.role` | 未改代码；组通常很小，留作性能优化，需时把 role 作为参数传入 |
| P3-2 | 独立新建幽灵行绘制不可点击的 P1 徽标 | `priority_at` 对 Single 幽灵返回 None，但渲染仍画 badge | 已修：`render_item` 对独立幽灵行不画徽标；新增测试 |
| P3-3 | `status_at` 缺列表边界校验，极窄面板可能命中边框列 | 历史实现只校验列区间 | 已修：补 `list.contains`；新增测试 |
| P3-4 | app 层 `TodoGroupRole` 与 UI 层 `DisplayGroupRole` 在幽灵行下锚点口径不同 | app 层不含幽灵槽 | 仅测试使用，无生产影响；在 `group.rs` 注释明确展示层以 `DisplayGroupRole` 为准 |
| P3-5 | Todo 测试缺组尾幽灵、N>3、最高在末位、窄面板 | 测试按列位表补，未按状态空间补 | 已补 `group_priority_handles_n_gt_3_with_highest_at_last`、`grouped_narrow_panel_renders_without_panic` 等 |
| P3-6 | `SaveModelChoice` 与设置页 PUT 都是无锁读改写，可能丢更新 | `save_ai` 只保证单次原子 rename，没有跨进程锁；recent 不在 revision | **未修**：涉及设置服务、connect 写入等多个调用点，留作持久化加固；当前单实例正常使用风险低 |
| P3-7 | 发送/steer 在 `BeginChatTurn` ack 前就记录 recent | 记录挂在“尝试发送”而非 ack | **未修**：正常路径 ack 很快；失败轮次会误记一次 recent，影响有限。留作把选择暂存到 ack 后消费的后续改动 |
| P3-8 | 磁盘 `defaultModel` 悬空导致 `load_ai` 失败，recent 写不进去 | `SaveModelChoice` 复用严格校验的 `load_ai`/`save_ai` | **未修**：与 P3-6 同属配置持久化加固；需要宽松解析或清理悬空 default |
| P3-9 | `/resume` 测试的“defaultModel 不被改写”是空断言 | 测试前 default 已等于传入 selection | 已修：测试前置 `None`，并补失效模型用例 |
| P3-10 | 同一 session 多窗格模型不同步 | 窗格级 selection 无同步路径 | 已修：`sync_session_selection` 在发送 ack 和 `/resume` 后同步同 session 的其它窗格；新增测试 |
| P3-11 | `switch_workspace` 同索引也刷新，白跑并可能清提示 | 只做越界判断 | 已修：`from == index` 时跳过状态刷新 |
| P3-12 | 文档行号大面积漂移 | 文档写死了实现行号 | 已修：改为符号/模块引用 |
| P3-13 | 测试缺 worktree 激活索引偏移、同工作区多标签 notice、空白窗格同步 | 覆盖不足 | 部分已补（历史新建、空白窗格逻辑间接覆盖、同 session 同步）；worktree 索引偏移经代码核对正确，未单独造重型测试 |
| P3-14 | `resolve_prompt_title_model` 仍只认 default，与“no model 只在无可用模型时出现”的全局措辞冲突 | 标题/recap 有意不走 recent | 文档收窄为“状态行/发送”；标题/recap 语义不变 |

## 5. 与 opencode 的语义对照

| 维度 | opencode | 本实现 | 结论 |
| --- | --- | --- | --- |
| 已有会话模型 | session durable selection | DB `agent_session.model_*` → `chat.selection` | 一致 |
| 新会话模型 | `newSessionModelByLocationAgent`（location+agent，内存）→ agent.model → fallback | `Workspace.lx_model`（内存）→ 窗格 selection → `defaultModel` → recent → 首个可用 | 方向一致；本实现把“最近实际使用”也写入工作区记忆，比 opencode 的“新建页显式选择”更宽 |
| 全局 fallback | CLI → location 配置 → recent → 首个可用 | `defaultModel` → recent（逐条校验）→ 首个可用 | 一致（本实现无 CLI 模型参数） |
| recent 写入时机 | 主要是显式选择（模型面板/CLI/Go upsell） | `/model`、发送、`/resume`、steer | **有意差异**：D1/D4 要求“上次使用”也进入全局回退；已写入文档 |
| recent 持久化 | `model.json` | `ai.json` 的 `recentModels` | 字段形态一致，文件不同 |
| 并发写 | Flock 串行化 | 无跨进程锁 | 差异，见 P3-6 |

## 6. 修复验证

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test --lib`：**5331 passed / 0 failed / 6 ignored**。
- `cargo test`：全量通过（lib 5331 passed / 6 ignored；集成 63 passed / 2 ignored；0 failed）。
- 新增/更新测试：
  - 模型：`effective_selection_falls_back_when_pane_provider_disabled_or_key_missing`、`fallback_selection_normalizes_stale_recent_variant`、`session_loaded_skips_invalid_model_in_recent`、`session_model_syncs_to_other_panes_viewing_same_session`。
  - 工作区：`create_tab_inherits_workspace_model_over_default`、`split_pane_inherits_workspace_model_over_default`、`model_selection_updates_workspace_model_memory`、`opening_new_history_entry_initializes_lx_status`。
  - Todo：`clicking_group_badge_cycles_and_selects_badge_owner`、`clicking_group_badge_with_ghost_commits_then_updates_owner`、`group_priority_handles_n_gt_3_with_highest_at_last`、`grouped_narrow_panel_renders_without_panic`、`standalone_new_ghost_row_does_not_draw_dead_priority_badge`、`status_hit_does_not_escape_list_bounds`。
