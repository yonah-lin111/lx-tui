# Review：BTW / Agents 列表-详情与 composer

> 审查基线：`961112f1`（含 feature + 首次光标/loading 修复）
> 审查方式：状态/输入路由、UI/几何/光标、测试/文档三路并行只读审查，逐项回到代码验证。
> 结论：**无 P0；P1 3 项、P2 9 项、P3 5 项，已按优先级修复。**

## P0

无。

## P1

| 编号 | 关键点 / 错误原因 | 位置 | 修复 |
| --- | --- | --- | --- |
| P1-1 | BTW 搜索后 `active` 与列表高亮分叉：`sync_selection_after_filter` 把窗口高亮设为 0，但渲染和 `enter_edit` 仍用 `selected_index()`；当前 Q 被过滤掉时列表无 `❯`，`Shift+Enter` 静默失败。 | `src/app/state/lx/btw.rs`、`src/app/update/chat/btw/input.rs`、`src/ui/main_content/lx/task/btw.rs` | 过滤仍有匹配时把 `active` 也落到第一项并补发 `LoadThread`；补“搜索后 Shift+Enter 进入第一项”测试。 |
| P1-2 | 搜索行硬件光标列重复叠加文本宽度：循环结束后 `x` 已是可见文本末尾，返回值又加 `view.cursor_col`；非空文本光标跑到文本末尾之后，横向滚动时可能越界。 | `src/ui/main_content/lx/task.rs::draw_search_row` | 单独保存文本起点 `text_x`，返回 `text_x + cursor_col` 并钳制在搜索框内；补非空/长文本滚动测试。 |
| P1-3 | Agents composer 从未按窗格 resize：`Lx.width` 一直是构造时的 `u16::MAX`，`visual_rows()` 不软换行，`height` 也无效；长单行被硬裁，多行草稿第 4 行起不可见，硬件光标可能越出窗格。 | `src/app/update/chat/viewport.rs::resolve_task_agents_visible` | 新增 `task_agent_composer_rect`，在 Agents 详情解析中按 composer 实际宽高调用 `resize`；补长草稿软换行/撑高测试。 |

## P2

| 编号 | 关键点 / 错误原因 | 位置 | 修复 |
| --- | --- | --- | --- |
| P2-1 | 切换 BTW Q、BTW/Agents 搜索后未清理 `lx_selection`；旧行列锚点会套到新线程/新 run 上，出现错误反显，复制内容串目标。 | `btw/mod.rs::switch_qa`、`btw/input.rs::edit_search`、`pane/task_page.rs::edit_agents_search` | 所有详情目标变化/搜索变更路径统一 `clear_lx_selection_for`；`Ctrl+X` 走同一同步路径。 |
| P2-2 | 搜索行滚轮守卫无条件执行：Processes 的“搜索行”正好是第一列表行，编辑态的 `rect.y+2` 落在 composer 正文行，滚轮被吞。 | `src/event/mouse/wheel.rs` | 仅当 tab 有搜索行且处于列表态时吞该行；补 Processes 首行滚轮测试。 |
| P2-3 | 编辑态仍暴露隐藏列表滚动条：`list_scrollbar` 不判断编辑态，5 行 composer 时轨道覆盖右侧，按下会拖动隐藏列表视口并吞掉 composer 点击。 | `src/ui/main_content/lx/task.rs::list_scrollbar` | 编辑态直接返回 `None`；补编辑态无列表滚动条测试。 |
| P2-4 | `Ctrl+X` 直接改搜索 `TextInput`，绕过 `edit_search`/`sync_agent_window`；剪切后高亮、窗口偏移与稳定选中不同步。 | `src/event/input.rs::cut_lx_selection` | 剪切后调用 BTW/Agents 的搜索同步 helper，并清旧选区。 |
| P2-5 | BTW `Accepted` 回投丢弃 `qa_id`，只按当前 `active` 找线程；ACK 晚于切 Q 时找不到线程，`pending_submit` 永久残留、草稿重复提交。 | `event/chat/btw.rs`、`btw/events.rs` | `AcceptedBtw` 增加 `qa_id`，按提交所属线程结算；补切 Q 后 ACK 测试。 |
| P2-6 | 同 tab 打开 agent 链接时 `TaskPage::select_tab` 提前返回，`agent_edit`/搜索保留；详情已切到新 agent，但发送和 `Ctrl+D` 仍作用于旧 agent。 | `app/update/chat/subagent.rs::open_agent_page`、`session.rs::open_subagents_page` | 打开链接/命令时显式清 `agent_edit` 与搜索，再定位目标；补测试。 |
| P2-7 | 目标 loading 文案恒为 `Working...`：`current_streaming_message` 只读 `RequestState::Streaming`，BTW 线程不写该状态。 | `src/ui/main_content/lx/chat_bar.rs` | `Idle` 时回退 `stream_assistant` / 最后一条 streaming assistant；`Committing/Finishing` 仍不回首轮阶段；补测试。 |
| P2-8 | Agents 状态筛选无匹配但搜索为空时显示 `no subagents yet`，让用户误以为没有子 agent。 | `src/ui/main_content/lx/task/agents.rs` | 增加 `no agents in this filter`；补筛选用例。 |
| P2-9 | `/btwList` 的 E2E/单测是空洞的：E2E 从未输入命令，单测只检查枚举显示名；文档门禁仍被测试字面量命中。 | `tests/btw_verify.rs`、`btw/tests.rs`、`task.md` | E2E 改为断言列表搜索占位；单测直接断言 `parse_btw_command("/btwList") == None`；文档改为“生产代码无引用，测试保留回归断言”。 |

## P3

| 编号 | 关键点 / 错误原因 | 位置 | 修复 |
| --- | --- | --- | --- |
| P3-1 | 双击只比较行号，不比较当前高亮；两次点击之间用 `↑/↓` 改过高亮时，会打开另一行。 | `src/event/mouse/press/lx.rs` | 双击命中先按点击行重新选中再进入编辑；搜索点击重置双击锚点。 |
| P3-2 | 裸 `/btw` 无条件 `active = None`，重开抢回最新 Q，违反 Q12；注释还声称保留当前 Q。 | `btw/mod.rs::try_submit_btw_command` | 裸 `/btw` 只在首次打开（`active=None`）落到最新；已有 active 保留。 |
| P3-3 | `Subagent` 标记无裁剪，窄窗格越过右边框；右侧 `To:` 胶囊只预留 8 列，可能覆盖标记尾部。 | `task/agents.rs`、`task/btw.rs` | 标记整体显示或整体隐藏；胶囊按真实标记宽度预留，补 9/10/11/12/20/32 列测试。 |
| P3-4 | `remove_composer` 无调用点，任务清单中的“agent 消失丢弃草稿”未落地。 | `app/state/lx/agent.rs` | 删除死 API；当前 run 列表只增不减，会话切换由 `clear_agents` 整体清空。 |
| P3-5 | 多处注释仍描述已删除的“内联追问 / `^a` 停止 / BTW 无列表”；`task/tests.rs` 继续堆 BTW/Agents 场景（审查时 1311 行）。 | 多处 | 已修正状态/事件/文案注释；BTW 场景迁入 `task/tests/btw_list.rs`，Agents 场景归入 `agent_list.rs`，根部降到 959 行。 |

## 验证

- `cargo fmt --all`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test --lib`：5316 passed / 0 failed / 6 ignored
- 新增核心回归：BTW 搜索 Shift+Enter、搜索光标非空/滚动、Agents composer 软换行、Agents ACK 切 Q、agent 链接退出 composer、Processes 首行滚轮、编辑态无列表滚动条、窄窗标记、筛选空态、BTW 目标工具阶段。

## 遗留

- `tests/btw_verify.rs` 为 ignored E2E，未在本次实跑；其断言已改为可验证列表态。
- 终端对 `Shift+Enter` 的识别、鼠标双击手感与 loading 观感仍需用户 `cargo run` 手工确认。
