# Todo 聚合组优先级展示 + 工作区模型记忆：任务清单

> 状态：**已合并到 `dev`（merge `e8b09999`，2026-11-13）**；P0/P1/P2 review 修复完成，P3 持久化/性能项留后续。交互外观与多工作区实测仍待用户 `cargo run` 验收。
> 日期：2026-11-12；合并 2026-11-13。
> 设计依据：[design.md](./design.md)；审查记录：[review.md](./review.md)。
> 检索工具：`codegraph`（lx-tui）、`codebase-memory-mcp`（project `lx-tui`）均已索引本仓库。

## 1. 已完成（设计阶段）

- [x] 用 codegraph / codebase-memory 定位 Todo 聚合组渲染、几何命中与模型选择链路。
- [x] 确认现有渲染格式 `❯ [ ]─┌─ P1 xxx` 与目标格式 `❯ [~] P1 ├─ xxx` 的列位差异。
- [x] 确认模型 `no model` 的两条根因：`effective_selection` 只有 `defaultModel` 一级兜底；切/建工作区不刷新 lx 状态行。
- [x] 核对 opencode `local.tsx` 的 `fallbackModel` 与 `model-preference.ts` 的 recent 持久化规则。
- [x] 用 `grill-me` 逐题确认 6 项边界（D1～D6，见 design §3）。
- [x] 输出 `design.md`（列位表、目标代码、影响范围、风险、验证计划）。
- [x] **用户审核 `design.md` / `task.md` 并明确批准实施。**

---

## 2. 审核门槛 G0（已通过：用户要求按文档执行）

批准后按以下顺序执行；任何一步发现设计与代码事实冲突，先停下回报，不擅自改边界。

1. D1～D4：模型回退链 `窗格选择 → defaultModel → recentModels → 第一个可用`，最近使用列表落 `ai.json`。
2. D5：聚合组徽标只画在锚点行、取组内最高优先级、点击改徽标来源条目。
3. D6：聚合组连接符列 `+9`、续行正文 `+12`。

---

## 3. Task 1：Todo 几何重构

**目标文件**：`src/ui/right_panel/todo.rs`、`src/ui/right_panel/todo/tests.rs`

- [x] `GROUP_BRANCH_WIDTH` 4 → 3。
- [x] 新增 `row_priority_x(list)`（`+6`）；`row_branch_x(list)` 改为 `row_priority_x + PRIORITY_WIDTH + 1`（`+9`）。
- [x] `row_text_x_for`：单条 `+9`、聚合组 `+12`（= `row_branch_x + GROUP_BRANCH_WIDTH`）。
- [x] `row_cont_x_for`：单条保持 `+6`（与 `P(n)` 左对齐），聚合组 `+12`。
- [x] 删除 `row_priority_x_for(list, grouped)` 与 `#[cfg(test)] row_priority_x(list)` 包装，更新全部调用点。
- [x] `priority_at`：只命中锚点行首行；列为 `row_priority_x(list)..+2`；返回值改为 `group_priority(...).display`。
- [x] 更新 `todo/tests.rs` 中所有几何断言与续行宽度断言。

**完成条件**：`cargo test --lib ui::right_panel::todo` 通过；单条条目列位（`[ ] P1 正文`、续行 `+6`）与现状完全一致。

---

## 4. Task 2：聚合组渲染与优先级聚合

**目标文件**：`src/ui/right_panel/todo/item.rs`、`src/ui/right_panel/todo/tests/grouping.rs`

- [x] 新增 `GroupPriority { display, priority }` 与 `group_priority(state, display, ghost, total)`：
  - 单条 → 自身；
  - 组 → 同 `group_id` 连续区间内取 `TodoPriority::min()`，并列取最靠前；跳过幽灵行。
- [x] `group_connector_glyph(offset, count, line_offset)` 改为 3 列字形（`┌─ ` / `├─ ` / `└─ ` / `│  ` / `   `），删除 `is_anchor` 参数。
- [x] `row_view`：锚点行 `priority` 取组级最高值（幽灵锚点同样处理）。
- [x] `render_item`：仅锚点行绘制徽标，X 用 `row_priority_x(list)`；徽标配色仍由锚点行状态决定。
- [x] 更新 `tests/grouping.rs`：
  - 2 条组：`❯ [ ] P1 ┌─ login api` / `         └─ login test`；
  - 3 条组：`         ┌─ schema` / `  [~] P0 ├─ migrate` / `         └─ docs`；
  - 折行续行：非末项 `         │  tail1`、末项 `            tail2`；
  - 命中：单条与组锚点共用 `+6` 徽标列，非锚点行不命中，组锚点返回最高优先级条目 display；
  - 连接符样式：`row_branch_x(list)` 处为 `┌`（不再 `+1`）。

**完成条件**：聚合组渲染与 design §4.1 的 ASCII 完全一致；点击锚点徽标循环切换最高优先级条目且显示立即变化。

---

## 5. Task 3：`recentModels` 配置字段

**目标文件**：`src/config/ai/mod.rs`、`src/config/ai/tests.rs`

- [x] `AiSettings` 新增 `recent_models: Vec<ModelSelection>`（`rename = "recentModels"`，`skip_serializing_if = "Vec::is_empty"`）。
- [x] `AiSettings::empty()` 补字段；不加入 `validate_inner`，不加入 `sanitize_optional_selections_with_connect`。
- [x] 单测：
  - 序列化/反序列化往返保留 `recentModels`；
  - 缺字段时为 `Vec::new()`；
  - 非法/过期条目不影响 `load_ai` 与 `validate()`；
  - 设置页 `merge_settings` 风格的最小回归：不带 `recentModels` 的保存不会清空（在 `settings/service/models` 既有测试或 `config/ai/tests.rs` 覆盖）。

---

## 6. Task 4：回退链与最近使用记录

**目标文件**：`src/app/state/lx/chat.rs`、`src/app/state/lx/chat/tests.rs`、`src/app/effects.rs`、`src/db/service.rs`

- [x] `MODEL_RECENT_LIMIT = 10`。
- [x] `ModelsSnapshot::fallback_selection()`：`defaultModel` → 逐条校验 `recentModels` → `ModelPicker::candidates().first()`。
- [x] `ModelsSnapshot::remember_model(&mut self, &ModelSelection) -> bool`：按 `provider/model` 去重前置、覆盖最新 variant、截断 10 条、无变化返回 false。
- [x] `effective_selection` 改用 `fallback_selection`；`default/title/recap_selection` 语义不变。
- [x] `Effect::SaveDefaultModel` → `Effect::SaveModelChoice { default_model: Option<_>, recent: Option<Vec<_>> }`；`db/service.rs` 一次加载/覆盖/原子写。
- [x] 单测：
  - `defaultModel` 有效 → 用它；
  - `defaultModel` 缺失/失效 → 取 recent 第一条有效；
  - recent 全部失效 → 取第一个可用模型；
  - 无启用供应商/无 Key → `None`；
  - `remember_model` 去重、前置、上限 10、variant 覆盖；
  - `effective_selection` 优先窗格选择。

**完成条件**：`no model` 只在“没有任何可用模型”时出现。

---

## 7. Task 5：记录时机与工作区刷新

**目标文件**：`src/app/update/chat/slash.rs`、`src/app/update/chat/mod.rs`、`src/app/update/chat/session.rs`、`src/app/update/chat/steer.rs`、`src/app/update/workspace.rs`、`src/app/update/tests.rs`

- [x] `persist_default_model`：同时更新内存 recent 并发出单个 `SaveModelChoice`（default + recent）。
- [x] 新增 `remember_model_used(state, selection)`（只记 recent，不改 default）。
- [x] `finish_model_selection`：走 `persist_default_model`（已有行为扩展）。
- [x] `try_send`：解析出可用模型后，钉住本轮实际使用的模型（含失效选择回退结果）；调用 `remember_model_used`。
- [x] `resume_session` 绑定模型后：校验有效后记录 recent/工作区记忆并同步同 session 窗格。
- [x] steer 提升轮次：钉住 `used_selection` 后调用 `remember_model_used`。
- [x] `switch_workspace` / `create_workspace_with_cwd` 末尾：调用 `refresh_active_tab_lx_status(state)`（只刷当前标签，避免消耗后台标签的一次性省略提示）。
- [x] 新建标签/右键分割/toggle 回 lx/worktree 直开/历史列表新建：创建或切回时刷新目标窗格。
- [x] `recentModels`/`defaultModel` 变化后：`refresh_fallback_lx_status` 刷新无消息、无窗格选择的空白窗格，保证状态行与实际发送一致。
- [x] 工作区级 `lx_model`：`/model`、发送、`/resume`、steer 更新；新窗格/标签先继承再刷新。
- [x] 同一 session 多窗格：发送 ack 与 `/resume` 后用 `sync_session_selection` 同步模型。
- [x] 单测：
  - 发送后 `chat.selection` 被钉住、recent 队首为该模型、`Effect::SaveModelChoice` 入队；
  - `/resume` 绑定模型后 recent 更新；
  - 切工作区后目标 lx 窗格 `status.model_name` 非空（用默认模型构造 `state.models`）；
  - 新建工作区同样刷新。
  - 新建标签/分割窗格继承 `Workspace.lx_model`，优先于全局 `defaultModel`；
  - 工作区历史新建并激活后状态行立即非空；
  - 停用供应商/清 Key 后窗格选择回退到可用模型；失效 variant 规范化而不是丢弃模型；
  - `/resume` 的失效模型不进入 recent；同一 session 多窗格同步。

---

## 8. Task 6：文档同步

- [x] `docs/standards/tui-design-requirements.md`
  - Todo 条目多行缩进段落：补充聚合组 3 列连接符（`+9`）、组徽标共用列（`+6`）、组内续行正文与首行正文对齐（`+12`）、组徽标取最高优先级且仅在锚点行。
  - `/model` 段落：补充“采纳后写回 `defaultModel` 并记入 `recentModels`（≤10，最近在前）；无窗格选择时按 `defaultModel → recentModels → 第一个可用模型` 兜底”。

---

## 9. 验证与验收

### 9.1 自动验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

重点用例：
- `ui::right_panel::todo::tests::grouping::*`
- `ui::right_panel::todo::tests::*`（几何/命中/编辑态）
- `app::state::lx::chat::tests::*`
- `app::update::chat::tests::*`、`app::update::tests::switch_workspace*`
- `config::ai::tests::*`
- `db::service` 相关 effect 测试

### 9.2 手工验收（用户执行 `cargo run`）

1. Todo 面板建 2 条/3 条同组待办：
   - 3 条组显示 `[~] P1 ├─` 在中间锚点行，首尾行只有 `┌─`/`└─`；
   - 徽标取组内最高优先级；
   - 点击徽标：切换最高优先级那条的 P 值，显示立即变化；
   - 长文本折行：`│` 与 `├` 同列，续行正文与首行正文对齐；
   - 行内编辑（双击）与 3 行视口正常。
2. 模型：
   - A 工作区 `/model` 选 Claude，切到从没选过模型的 B 工作区：状态行不再 `no model`，显示 `defaultModel`；
   - 只 `/resume` 一个 GPT 会话后切到 C（`defaultModel` 为空）：显示 GPT；
   - 在 B `/model` 换模型后切回 A：A 仍显示 Claude；
   - 在 A 的新标签/分割窗格：继承 A 的 Claude，不落到全局 default；
   - `ws list` 打开一个尚未打开的历史工作区：立即显示模型，不再 `no model`；
   - 清空当前模型供应商的 Key/停用供应商：状态行回退到其它可用模型，发送不再误报 `no model`；
   - 同一 session 在两个窗格打开：A 发送新模型后 B 跟随；
   - 重启应用：`ai.json` 的 `recentModels` 保留，新工作区按 recent 兜底；
   - 设置页保存模型配置后 `recentModels` 未被清空。

### 9.3 出口条件

- 自动验证全绿；
- 手工验收 9.2 全部通过；
- `docs/standards` 与实现一致（规范文档是唯一来源）。

---

## 10. 实施结果（2026-11-12；合并 2026-11-13）

- [x] Todo 几何、连接符、组内最高优先级、仅锚点行徽标、点击归属全部落地；单条条目列位保持不变。
- [x] `recentModels` 配置字段、四级回退链、`SaveModelChoice` 原子落盘、发送/恢复/steer 记录、切/建工作区状态刷新全部落地。
- [x] Review P1/P2 修复：历史新建刷新、工作区级模型记忆、窗格选择统一校验、失效 `/resume` 过滤、空白窗格状态同步、当前标签刷新、文档同步。
- [x] 自动化验证：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿（lib 5331 passed / 6 ignored；集成测试 63 passed / 2 ignored）。
- [x] 2026-11-13 合并到 `dev`（merge `e8b09999`），合并后主工作区全量验证通过；worktree 与分支已删除。
- [ ] 用户 `cargo run` 手工验收（design §8 与本文 §9.2），确认 Todo 外观/点击与多工作区模型行为。

实现说明：

- `/model` 采纳仍写回 `defaultModel`，并同时记入 `recentModels`；`/resume`、实际发送、steer 提升只记 `recentModels`，且入口统一用 `validated_selection` 过滤失效模型。
- 无任何显式选择时，`/model` 面板会把“第一个可用模型”标为当前项（回退链最后一级的必然结果）。
- 工作区记忆是 `Workspace.lx_model`（进程内），新窗格/标签优先继承；窗格级 selection 仍可覆盖。
- 仓库内 `codebase-memory` 索引项目名为 `lx-tui-todo-group-priority-and-ws-model`（worktree 已删除）；`codegraph` 索引位于主工作区 `.codegraph/`。

## 11. Review 修复清单

详见 [review.md](./review.md)。摘要：

- [x] P1-1 工作区历史新建并激活不刷新状态 → `open_new_history_workspace` 补刷。
- [x] P1-2 新标签/分割窗格不继承工作区模型 → `Workspace.lx_model` + 创建时继承。
- [x] P1-3 窗格选择与 fallback 校验口径不一致 → `effective_selection` 复用 `validated_selection`，variant 规范化。
- [x] P1-4 Todo 徽标点击缺端到端测试 → 两条点击链路测试。
- [x] P2-1 `/resume` 失效模型污染 recent → 统一过滤。
- [x] P2-2 失效 variant 丢模型 → 回落默认等级。
- [x] P2-3 recent/default 变化后空白窗格状态不同步 → `refresh_fallback_lx_status`。
- [x] P2-4 工作区刷新波及后台标签提示 → 只刷当前标签 + 切标签补刷。
- [x] P2-5 文档/验证报告偏差 → 三份文档同步。
- [x] P3-2/3/9/10/11/12：幽灵行死徽标、命中边界、空断言、同 session 同步、同索引刷新、行号漂移。
- [ ] P3-1/6/7/8：Todo 大组性能、配置无锁读改写、ack 前记 recent、悬空 default 阻塞 recent 落盘；留作后续持久化/性能加固。
