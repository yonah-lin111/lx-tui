# 边框标签值颜色：聚焦不再变蓝 — 执行任务

- 任务目录：`docs/feature/border-label-color/`
- 规则：严格遵守 `AGENTS.md`；禁止 `unwrap()` / `expect()`（测试除外）；样式统一从 `ui/style.rs` 取；测试驱动。
- 需求原文：移除 lx 页面中 `ws:xxx` 的 `xxx` 在聚焦时变蓝；prompt 的 `ws`、git 分支与工作区也一并移除；保留原本的默认颜色不变。

## 执行清单

### 1. 需求与边界对齐（Grill Me 阶段）
- [x] 质问 1：lx 窗格 `ws:` 值聚焦与失焦完全一致 → 采纳推荐
- [x] 质问 2：prompt 底边前缀是否一并改 → 前缀不动
- [x] 质问 3：prompt 底边值改为"显式默认前景 + dim，不受面板焦点影响" → 确认
- [x] 质问 4：窗格 `agent:` 值是否一并改 → 用户追加"一起改"
- [x] 质问 5：保留 dim 一档 → 采纳推荐
- [x] 质问 6：同步更新规范文档 → "更新"
- [x] 质问 7：前缀失焦继承 dim、聚焦变亮的行为保持不变 → 采纳推荐
- [x] 质问 8：改动范围 = 四处值、前缀全不动 → 采纳推荐
- [x] 完成 `design.md` 定稿

### 2. 复现测试（先红）
- [x] `ui::tests::lx_pane_ws_value_ignores_focus`：聚焦窗格的 `ws:` 值必须为 `Reset + DIM`
- [x] `ui::tests::pane_agent_value_ignores_focus`：聚焦窗格的 `agent:` 值同上
- [x] `ui::tests::prompt_bottom_values_stay_default_while_focused`：prompt 聚焦时 `b:`/`wt:`/`ws:` 的值不被染色
- [x] 三条测试先跑出预期失败（`left: (Cyan, BOLD | DIM)`），确认精确复现

### 3. 实现
- [x] `src/ui/style.rs`：删除 `border_value(focused)`，新增 `border_label_value()`（显式 `Color::Reset` + `DIM` + `remove_modifier(BOLD)`）
- [x] `src/ui/panes.rs`：`render_pane_workspace_label` / `render_pane_agent_label` 的值改用新 token，移除不再影响取色的 `focused` 参数
- [x] `src/ui/prompt_frame.rs`：`render_workspace_label` / `branch_status_spans` 的值改用新 token
- [x] 前缀 `ws:` / `agent:` / `b:` / `wt:` 保持 `accent()` 不变

### 4. 测试与文档
- [x] `ui::style::tests::border_label_value_resets_inherited_accent`：token 级断言（叠加到 accent 单元格后仍是默认灰、无 BOLD）
- [x] 更新 3 个断言旧行为的既有测试（不降低覆盖面）
- [x] `docs/standards/tui-design-requirements.md`：窗格 `ws:`、prompt `ws:`、`b:`/`wt:` 三处描述
- [x] `docs/feature/workspace-agent-status/design.md`：`agent:` 值描述

### 5. 验证
- [x] `cargo test --lib`（2517 passed）
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`（全量）

## 第二轮：前缀与页名也不再随焦点变化

用户追加要求：prompt 容器聚焦时 git 信息与 `ws` 的**前缀**不要高亮加粗；prompt 左上角 title 的**页名**聚焦不要高亮；并把 main content 的 `agent:` / `ws:` 前缀调整为与 prompt git 前缀一致。

### 1. 边界对齐（Grill Me 阶段）
- [x] 质问 1：前缀去掉加粗、保留青色 → 采纳推荐（`Cyan + DIM` + 显式去 BOLD）
- [x] 质问 2：prompt 底边框与窗格顶边前缀统一 → 采纳推荐
- [x] 质问 3：页名去掉继承的粗体、保留默认灰 + dim → 采纳推荐
- [x] 质问 4：需求 2 与需求 1 合并为同一处改动（五处前缀共用一个 token）→ 采纳推荐
- [x] 质问 5：取方案甲（两状态都是淡青，聚焦不变亮变粗）→ 采纳推荐
- [x] 质问 6：`muted_on_border()` 补 `remove_modifier(BOLD)`，一处修好全部 5 个调用点 → 采纳推荐

### 2. 复现测试（先红）
- [x] `ui::tests::prompt_bottom_prefix_ignores_focus`（新增）：聚焦时 `b:` 前缀 `(Cyan, BOLD | DIM)` → 期望 `(Cyan, DIM)`
- [x] `ui::tests::prompt_page_name_ignores_focus`（新增）：聚焦时页名 `(Reset, BOLD | DIM)` → 期望 `(Reset, DIM)`
- [x] `lx_pane_ws_value_ignores_focus` / `pane_agent_value_ignores_focus`（扩展前缀断言）：`(Cyan, BOLD)` → 期望 `(Cyan, DIM)`
- [x] `prompt_bottom_values_stay_default_while_focused`（扩展前缀断言）：同上
- [x] 五条测试先跑出预期失败，确认精确复现

### 3. 实现
- [x] `src/ui/style.rs`：新增 `border_label_prefix()`（`Cyan + DIM` + 显式去 BOLD）
- [x] `src/ui/style.rs`：`muted_on_border()` 补 `.remove_modifier(BOLD)`
- [x] `src/ui/panes.rs`：`ws:` / `agent:` 前缀改用新 token
- [x] `src/ui/prompt_frame.rs`：`b:` / `wt:` / `ws:` 前缀改用新 token

### 4. 测试与文档
- [x] `ui::style::tests::border_label_prefix_is_soft_cyan_without_bold`（新增）
- [x] `ui::style::tests::muted_on_border_strips_inherited_bold`（新增）
- [x] 更新 2 处断言旧前缀样式的既有测试
- [x] `docs/standards/tui-design-requirements.md`：窗格 `ws:`、`agent:`、prompt `ws:`、`b:`/`wt:`、页名共 5 处描述
- [x] `docs/feature/workspace-agent-status/design.md`：`agent:` 前缀描述

### 5. 验证
- [x] `cargo test --lib`（2517 passed）
- [x] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿

## 完成记录

- 实施分支：`fix/border-label-color`（工作区 `.worktrees/border-label-color`）。
- 根因两种：① 窗格顶边值走 `border_value(focused)`，聚焦即 `Cyan + BOLD + DIM`；② prompt 底边值用 `muted()`（只加 DIM、不设前景），叠加写入边框单元格时继承了聚焦边框的 `Cyan + BOLD`。
- 未改动项：所有前缀仍 `accent()`；失焦时前缀随边框继承 DIM 的现有行为保留；布局、宽度计算、截断与隐藏规则不变。
- TUI 实际观感需用户运行 `cargo run` 人工验收。
