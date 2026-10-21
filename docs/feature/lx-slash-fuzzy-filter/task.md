# `/resume`、`/model` 二级面板模糊匹配 + 状态行思考等级：执行任务

- 状态：**已实施**（分支 `feat/lx-slash-fuzzy-filter`，待用户验收与合并）。
- 前置：`design.md` 已由用户逐项确认（10 个决策）。
- 工作区：`.worktrees/lx-slash-fuzzy-filter`。

## 0. 开发门禁与现状核查

- [x] codegraph 定位 `SlashPanel`/`HistoryPanel`/`ModelPicker` 与全部构造点。
- [x] 确认 `apply_lx_input` 的 `/` 面板分支原为 `_ => return`（二级面板吞掉编辑键）。
- [x] 确认 prompt 历史只在 `on_turn_committed`（DB ack）记录 → 过滤词天然不入历史。
- [x] 确认 `ChatStatus` 原本没有 variant 字段；`status.rs` 只画「模型 · 上下文」。
- [x] 读 opencode `packages/tui/src/component/prompt/metadata.tsx:74`：思考等级为
      `theme.text.feedback.warning.base` + `BOLD`，`·` 用 muted。

## 1. 模糊过滤（`/resume`、`/model`）

- [x] `HistoryPanel` / `ModelPicker` 增加 `filter: String` 与 `filtered: Vec<usize>`。
- [x] `command_score` 提升为 `pub(crate)`，两个面板复用同一套分层打分。
- [x] `HistoryPanel::refilter`：标题 + session id，空查询恢复恒等视图。
- [x] `ModelPicker::refilter`：一级模型名 + provider 名；二级 variant 名。
- [x] `visible()` / `active_entry()` / `active_candidate()` / `active_stage_index()`：
      视图索引 ↔ 原始索引换算，采纳/复制/编辑一律走它们。
- [x] `item_count()` 改用 `filtered.len()`；`current_index()` 把真实索引换算成视图位置
      （已生效项被过滤掉时不画 `❯`）。
- [x] `ui/main_content/lx/slash.rs`：`items()` 与 `history_time_labels()` 按过滤视图产出；
      `no matches` 占位；`item_at()` 空态不可选。
- [x] `apply_lx_input`：二级面板放行编辑键并调用 `refresh_slash_filter`
      （`EnterFolder`、`Clear` 必须先于放行分支，否则模态打不开/复制失效）。
- [x] `/model` 进入二级时重建 `filtered` 并清空 `filter`（不沿用上一级过滤词）。
- [x] `text::LX_SLASH_NO_MATCHES = "no matches"`。

## 2. 状态行思考等级

- [x] `ChatStatus.variant: Option<String>`，由 `refresh_status` 从当前生效选择取。
- [x] `style::chat_variant()`：`Color::Yellow` + `BOLD`（对齐 opencode warning+BOLD）。
- [x] `status::layout` 改为**单一降级链**：模型名 → +上下文 → +variant → +两者，
      第一个放得下的组合即结果；结果里的 `model_width` 写入 `StatusLayout`。
- [x] `render` 直接用 `lay.model_width`，不再重算（重算会与降级链分叉）。
- [x] 删除已成死代码且会拖累字段的 `model_available_width`。

## 3. 测试

- [x] `slash/tests.rs`：空查询恒等、前缀优先、id 匹配、大小写、过滤为空、
      高亮钳制与退格恢复、`current_index` 被过滤掉时为 None、provider 名匹配、
      variant 名匹配、视图索引 ↔ 原始索引换算。
- [x] `update/chat/tests/slash_panels.rs`：打字过滤后 `↵` 采纳命中项、退格恢复、
      空态 `↵` 吞掉不加载会话、`^C` 复制过滤后项、`/model` 过滤与退格。
- [x] `ui/.../status/tests.rs`：`模型 · variant · 上下文` 顺序、加粗黄字与 muted 分隔符、
      无 variant 无空洞、窄宽先丢上下文再丢 variant。

## 3.5 收口调整（用户追加）

- [x] `/model` **第二级（思考等级）移除模糊匹配**：`refilter` 在 `Variants` 阶段恒为
      全量视图且 `filter` 保持空；键入字符只进输入框、不参与过滤。
- [x] `/model` 进入第二级 / `Esc` 从第二级退回第一级时重建 `filtered`
      （第一级保留过滤状态，退回不丢）。
- [x] **清空规则**（整框清空，不用快照）：
      - `Esc` 真正关闭 `/model` 第一级或 `/resume` → 清空；
      - `↵` **选中即完成**（无 variants 直接选中模型 / 选中思考等级 / 选中会话）→ 清空；
      - `↵` **进入第二级**（换层继续挑）→ 不清空，过滤词留给退回第一级时用；
      - `Esc` 从第二级退回第一级 → **不清空**，且按输入框现存过滤词重建第一级视图；
      - `⇧↵` 打开会话编辑模态 → **不清空**；
      - `^C` 复制会话 id → **不清空**；
      - `/` 命令候选面板 `Esc` → **不清空**（命令文本由采纳路径按行删除）。
- [x] 抽出 `clear_lx_input(state, pane)`（内部走 `Lx::clear()`，压一次 undo 快照）。

## 4. 验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`（16 个 test binary 全绿，lib 2831+）
