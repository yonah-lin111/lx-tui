# lx `ask` 工具实施记录

> 分支：`feat/lx-ask-tool`（worktree `.worktrees/lx-ask-tool`，基线 `dev@f74e3f6`）
> 规范：`design.md`（16 节）、`task.md`（T0.1–T5）

## 1. 交付内容

| 层 | 文件 | 说明 |
|---|---|---|
| 工具 | `src/chat/tools/ask.rs` + `ask/tests.rs` | 参数模型、上限常量、`parse_arguments`（结构→数量→字符→重复→preview，整次拒绝）、`answered_result`；24 例单测 |
| 工具 | `types.rs` / `mod.rs` / `schema.rs` / `output.rs` | `TOOL_NAMES` 8 项、`ToolExecutionResult::Ask`、第 8 个 ToolSpec（数字引用 ask.rs 常量）、`format_for_model` / `to_result_json` |
| 卡片数据 | `tool_view.rs` / `steps.rs` / `event/chat/tool_calls.rs` | `AskDetailView`/`AskAnsweredView`，结算与重载两条路径同构 |
| 握手 | `chat/agent_loop/mod.rs` / `event/chat/host.rs` | `AgentHostRequest::AskUser` + `AgentHost::ask_user`（oneshot，丢弃=取消）+ `AgentHostEvent::AskUser` |
| 轮次 | `run_agent_turn` | ask 分支：非法参数→Failed 不打扰用户；取消→`finish_tool(Cancelled)` + `cancel_unstarted` + interrupted；`Failed{code}`→整轮失败 |
| 事件 | `event/chat/ask.rs` / `mod.rs` / `app/effects.rs` | `ChatStreams.asks`、归属校验、`Effect::AnswerAsk{key,call_id,answers}`、取消/结束/孤儿 sweep |
| 状态 | `app/state/lx/ask.rs` / `lx.rs` / `pane.rs` | `AskState`/`AskEditor`（独立缓冲，无 `DerefMut`）、`Lx.ask`、`Tab::has_pending_ask` |
| 输入 | `app/update/chat/ask.rs` / `pane.rs` | 键位（§7.1/§7.2）、单选/多选/手写规则、双击 Esc 取消、`apply_ask_paste`、`apply_editor_edit` 抽取复用 |
| 渲染 | `ui/main_content/lx/ask.rs` | 几何（70% 上限、最小 10 行、60 列分栏、矮窗格隐藏预览）、绘制、命中、滚轮分区（同一份 `AskLayout`） |
| 渲染 | `ui/main_content/lx/history/tool/ask.rs` | 卡片标题 `■/□ Ask`、`waiting for you` / `N questions`、每题树行、展开题干、**不展示 preview** |
| 渲染 | `ui/tab_bar.rs` / `ui/text*.rs` | 挂起 ask 的标签 +2 列并显示 `?`（宽度/绘制/命中同步） |
| 接线 | `event/mouse/{press,wheel}.rs`、`event/input.rs` | 面板内点击吞掉、滚轮分区分派、粘贴只进手写缓冲 |
| 尺寸 | `layout.rs` / `resize_panes` | `LX_ASK_*` 常量 + `lx_ask_editor_size`，折行宽度与渲染同源 |
| 提示词 | `chat/prompt.rs` | 第 8 个工具 + ask 段（对齐 codex default.md）+ `PROMPT_VERSION = "lx-3"` |

## 2. 验证记录

| 命令 | 结果 |
|---|---|
| `cargo test --lib chat::tools::ask` | 24 passed |
| `cargo test --lib chat::agent_loop` | 40 passed（含成功/取消/非法参数三例） |
| `cargo test --lib event::chat` | 79 → 加入阶段 3 后仍全绿（含 ask 9 例） |
| `cargo test --lib app::update` | 582 + 本阶段新增 |
| `cargo test --lib ui::main_content::lx::ask` | 12 passed |
| `cargo test --lib history::tool::ask` | 5 passed |
| `cargo test --lib ui::tab_bar` | 9 passed |
| `cargo test --lib event::tests::ask_` | 2 passed（点击选中、粘贴只进缓冲） |
| `cargo test`（全量） | **3291 lib + 16 个 target 全绿** |
| `cargo fmt` / `cargo clippy --all-targets -- -D warnings` | 零警告 |

未验证（Agent 无法交互）：真实 TUI 外观、鼠标手感、真实模型发起 ask 的端到端；需用户 `cargo run` 确认。

## 3. 与 design/task 的偏差

1. **单选鼠标单击 = 独占选中，不做「只高亮」**（design §7.3 已同步修改）。
   - 原设计只高亮不选中，导致纯鼠标用户永远交不出答案（键盘有 Space，鼠标没有）；现为「单击选中、Submit/Enter 提交」。
2. **任务顺序：T2.1 提前到 T1.3 之前**。T1.3 要在事件层写 `pane.lx.ask = AskState`，必须先有 `AskState`。
3. **修了一个既有崩溃**（阶段 1 测试撞出）：`Lx::delete_word_forward` 删除后把光标设到已删除的 `end`，缓冲区缩短后越界，下一次编辑在 `slash_trigger` 里 `&input[..cursor]` panic。改为光标不动（readline `M-d` 语义），1 行改动。
4. **手写缓冲初始宽度**：事件层用「输入框宽度 − 4」初始化；窗格 resize 后由 `layout::lx_ask_editor_size` 精确同步，渲染以缓冲区自身宽度为准（软换行与画框始终同源）。
5. **卡片运行态只写 `waiting for you`**：不解析流式参数、不猜题数（design §8.4 的取舍）。
6. **`AskHit`/`AskWheelZone` 只做状态迁移**，几何全部由 UI 层 `layout()` 给出，事件层不重算（design §7.3 的硬约束）。

## 4. 已知限制 / 后续

- 面板在极小窗格（`area.height < 12`）只显示题干 + 选项 + 底栏，预览隐藏并在底栏提示；无法滚动查看被隐藏的预览——这是设计取舍，不是缺陷。
- 历史卡片不保存/不展示 preview（`result_json` 自包含、不含 preview），与 design §9 一致。
- 标签栏 `?` 标记只在「该标签任一窗格挂着 ask」时出现；窗格切到终端视图同样计数（防御：面板仍挂着）。
- `/compact` 等本地命令在 ask 等待期间不会执行：输入被面板整块接管，这是 design §9.1 的预期。

## 5. 合并注意

- 无数据库迁移、无 `SCHEMA_VERSION` 变更。
- 与 `.worktrees/lx-history-perf`（未提交改动含 `history/tool.rs`）存在同文件冲突风险：对方合并前需先 rebase，`tool.rs` 的 ask 分支只有两处（`rows` / `draw_animated`）。
- 合并后建议实机走一遍：模型发起 ask → 单选/多选/手写/字符画预览 → 鼠标点选与滚轮 → 标签 `?` → Esc 双击取消 → 历史卡片与草稿状态。
