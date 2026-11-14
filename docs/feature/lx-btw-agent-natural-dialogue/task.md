# BTW / Agents 自然对话与子 agent `@` 输入 — 实施任务拆解 (`task.md`)

- **关联设计**：[`design.md`](design.md)
- **执行前提**：用户审核确认 `design.md` 与本 `task.md` 后才开始改代码。
- **当前状态**：T1–T6 全部完成，2026-11-14 审查修复后合并到 `dev`（merge `bd8668d0`）。
- **审查报告**：[`review.md`](review.md)。
- **合并记录**：`dev@f354b4af` + `feat/btw-agent-natural-dialogue@9d3df292` → `bd8668d0`；无冲突（仅 `src/chat/tools/tests.rs` 自动合并）；`cargo fmt`、`clippy -D warnings`、`cargo test`（5433 passed）通过。

---

## 任务总览

| 任务 ID | 任务名称 | 涉及核心文件 | 预计改动后行数（生产 ≤1000） | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **T1** | BTW composer 胶囊删除目标段，清理 `target_label` / `effective_targets` / `LX_BTW_TARGETS_PREFIX` | `src/ui/main_content/lx/task/btw.rs`<br>`src/app/state/lx/btw.rs`<br>`src/ui/text/btw.rs` | `task/btw.rs`: ~298 行<br>`state/lx/btw.rs`: ~412 行 | 已完成 |
| **T2** | Agents composer 胶囊删除 `To: ` 前缀 | `src/ui/main_content/lx/task/agents.rs` | ~513 行 | 已完成 |
| **T3** | BTW 统计行标签改为模型显示名 | `src/event/chat/btw.rs` | 保持 ~1370 行（存量超红线，不新增） | 已完成 |
| **T4** | Agents 追问 composer 开启 `@`：能力开关、4 Tab、朴素插入、上下文同步、扫描归属、键位 | `src/app/state/lx.rs`<br>`src/app/state/lx/panels.rs`<br>`src/app/state/lx/mention.rs`<br>`src/app/prompt/mention.rs`<br>`src/app/update/pane/task_page.rs`<br>`src/app/update/pane/popups.rs`<br>`src/event/mod.rs`<br>`src/ui/main_content/lx/mention.rs` | `state/lx.rs`: ~900 行<br>`state/lx/panels.rs`: ~540 行<br>`state/lx/mention.rs`: ~860 行<br>`task_page.rs`: ~780 行<br>`popups.rs`: ~300 行 | 已完成 |
| **T5** | 测试更新与新增（渲染、插入、键位、扫描路由） | 见各任务“测试”节 | 单测试文件 ≤1500 行 | 已完成 |
| **T6** | 精准校验：`cargo fmt` / `clippy -D warnings` / 受影响模块测试 | — | — | 已完成 |

> 顺序要求：T1/T2/T3 互相独立，可并行；T4 内部按 4.1→4.6 顺序落地；T5 跟随各任务；T6 最后执行。

---

## T1：BTW composer 胶囊删除目标段

### 目标

BTW 编辑态顶边框右上角只剩 `QA n · 标题`；目标信息（`To: ...`）从 UI 彻底移除，但**提交语义、目标解析、DB 字段保持不变**。

### 改动点

1. `src/ui/main_content/lx/task/btw.rs:160-219` `draw_qa_badge`
   - 删除 `targets` 计算与 `target_label()` 调用；
   - 删除 `targets` 与标题争抢宽度的分支；
   - `room = max - width(prefix)`，标题按 `QA_BADGE_MAX_COLS` 截断；
   - 框内绘制逻辑（右侧内缩 1 列、避开左侧 `BTW` 标记）保持不变。

   ```rust
   let max = rect.width.saturating_sub(12).min(QA_BADGE_MAX_COLS);
   let room = max.saturating_sub(width(prefix.as_str()));
   if room < 4 {
       return;
   }
   let title = text::truncate_to_width(title, usize::from(room));
   let badge = format!(" {prefix}{title} ");
   ```

2. `src/app/state/lx/btw.rs`
   - 删除 `BtwThreadState::target_label()`（93-105 行）；
   - 删除 `BtwThreadState::effective_targets()`（120-127 行）；
   - 保留 `targets` 字段与 `BtwTargetView::default_assistant()`。

3. `src/ui/text/btw.rs:32`
   - 删除 `LX_BTW_TARGETS_PREFIX`。

4. 测试清理：`src/app/update/chat/btw/tests.rs:627-633`
   - 删除 `default_target_sentinel_renders_as_lx_not_an_agent_id`。

### 验收

- 宽 80 的 BTW 编辑态顶边框：`─ BTW ──────── QA 1 · 你好呀 ─`；
- 任何宽度下都无 `To:`；
- `cargo test --lib app::update::chat::btw` 通过。

---

## T2：Agents composer 胶囊删除 `To: `

### 改动点

`src/ui/main_content/lx/task/agents.rs:273-292`：

```rust
fn draw_agent_badge(grid: &mut Grid, rect: Rect, run: &SubagentRun, marker_w: u16) {
    let name = run
        .nickname
        .as_deref()
        .filter(|nickname| !nickname.is_empty())
        .unwrap_or_else(|| run.role.as_str());
    let label = format!(" {name} [{}] ", run.role.as_str());
    // 宽度守卫不变
}
```

测试更新：`src/ui/main_content/lx/task/tests/agent_list.rs:842-870`

```rust
assert!(
    lines.iter().any(|line| line.contains("Turing [executor]")),
    "composer 右侧目标：{lines:?}"
);
assert!(
    !lines.iter().any(|line| line.contains("To:")),
    "不再出现 To: 前缀：{lines:?}"
);
```

### 验收

- `Subagent`（左）+ `Turing [executor]`（右）；
- 窄窗格 `agent_composer_marker_and_badge_stay_inside_narrow_rect` 仍通过。

---

## T3：BTW 统计行改为模型显示名

### 改动点

`src/event/chat/btw.rs`（`start_recipient`，665-701 行）：

```rust
// 统计行展示回答者模型，而不是回答者身份。
let model_label = state
    .models
    .model_display_name(&start.model.provider, &start.model.model);
let mut buffer = crate::app::state::LxChat::new();
buffer
    .messages
    .push(crate::app::update::new_btw_assistant_message(
        &start.assistant_message_id,
        &model_label,
    ));
let run = BtwRun {
    // ... 既有字段
    label: model_label,
};
```

- 不改 `new_btw_assistant_message` 签名；
- 不改 `BtwRun` 字段；
- 不改 `stats_line` 渲染；
- 不改 DB 写入（assistant 行本来就有 provider/model）。

### 测试

- `src/app/update/chat/view/tests.rs` 已有 `ModelsSnapshot` 用法；补一条 `model_display_name` 缺失配置时回退 `model_id` 的断言（如尚无覆盖）。
- 手工验收：BTW 回答完成后统计行首段是模型名；重启恢复同一线程后首段不变。

### 验收

- live：`deepseek-v4.1-flash · 5.4k in · 166 out · ...`；
- reload：同一条线程仍为同一模型名；
- 主对话与子 agent 详情统计行不变。

---

## T4：Agents 追问 composer 开启 `@`

### T4.1 能力开关与 Tab 集合

**文件**：`src/app/state/lx.rs`

1. 字段：

   ```rust
   /// Agents 追问 composer：只开 `@`（4 Tab、朴素插入），其余编辑浮层保持关闭。
   mention_minimal: bool,
   ```

   `build()` 初始化为 `false`。

2. `agent_composer()`（266-272 行）：

   ```rust
   pub fn agent_composer() -> Self {
       let mut lx = Self::build(false);
       lx.no_shell = true;
       lx.allow_panels = false;
       lx.mention_minimal = true;
       lx
   }
   ```

3. 新增：

   ```rust
   /// `@` 面板可用 Tab：AI Web 仅 Files；Agents 追问不含会话态 Subagents；其余 5 项。
   pub fn mention_tabs(&self) -> &'static [crate::app::prompt::MentionTab] {
       use crate::app::prompt::MentionTab;
       if self.entry == LxEntry::AiWeb {
           &[MentionTab::Files]
       } else if self.mention_minimal {
           &MentionTab::PROMPT_ALL
       } else {
           &MentionTab::ALL
       }
   }
   ```

### T4.2 `refresh_panel` 只刷新 `@`

**文件**：`src/app/state/lx/panels.rs:15-71`

`if !self.allow_panels { ... }` 分支内：

```rust
if self.mention_minimal {
    let masked = crate::app::paste::mask_pasted_text(&self.input).into_owned();
    self.mention.unsuppress();
    self.mention.refresh(&masked, self.cursor);
} else {
    self.mention.suppress();
}
```

其余浮层：

```rust
self.panel = None;
self.slash = None;
self.paste = None;
self.text_paste = None;
self.refer = None;
```

### T4.3 朴素插入

**文件**：`src/app/state/lx/mention.rs`（`confirm`，630-689 行）
**文件**：`src/app/state/lx/panels.rs`（`mention_confirm`，271-284 行）

1. `LxMention::confirm(text_len: usize, plain: bool)`：

   - 二级 Tab：
     - `RefDir`：`@<folder.path.display()>`（绝对路径）；
     - `Skills`：`@<skill.name>`；
     - `Mcp`：`@<server.name>`；
     - `Subagents`：`@<reference_target()>`（`mention_minimal` 下不可达，兜底）；
   - `plain == false` 时保持原 `format_reference(...)` 分支；
   - Files 分支继续用 `mention_insertion(&entry)`（本来就是 `@path ` / `@dir/ `）；
   - 统一追加尾随空格，`reset_session()` + `suppressed = true` 保持。

2. `Lx::mention_confirm`：

   ```rust
   let plain = self.mention_minimal;
   let Some((range, insertion)) = self.mention.confirm(self.input.len(), plain) else {
       return false;
   };
   ```

### T4.4 Tab 循环按集合

**文件**：`src/app/state/lx/mention.rs`

- 把 `switch_tab_for_entry(delta, text, cursor, ai_web)` 收敛为 `switch_tab(delta, text, cursor, tabs)`，按集合下标取模循环；
- 把 `set_tab_for_entry(tab, text, cursor, ai_web)` 收敛为 `set_tab(tab, text, cursor, tabs)`，`!tabs.contains(&tab)` 直接拒绝；
- `ai_web: bool` 由 `mention_tabs()` 返回的集合取代（AI Web 集合为 `[Files]`），不再保留双入口。

**文件**：`src/app/state/lx/panels.rs`

```rust
pub fn mention_switch_tab(&mut self, delta: isize) -> bool {
    self.sync_mention_subagent_runs();
    let tabs = self.mention_tabs();
    self.mention.switch_tab(delta, &self.input, self.cursor, tabs)
}

pub fn mention_set_tab(&mut self, tab: crate::app::prompt::MentionTab) -> bool {
    self.sync_mention_subagent_runs();
    let tabs = self.mention_tabs();
    self.mention.set_tab(tab, &self.input, self.cursor, tabs)
}
```

### T4.5 顶栏渲染读 `mention_tabs`

**文件**：`src/ui/main_content/lx/mention.rs:128-158`

- 新增 `tab_label(MentionTab) -> &'static str`；
- 新增 `panel_tabs(&Lx) -> Vec<PanelTab<'static>>`；
- `panel_data` 内删除 AiWeb 特判与 5 项硬编码，统一 `let tabs = panel_tabs(lx);`。

### T4.6 上下文同步

**文件**：`src/app/state/lx/panels.rs`

新增 `sync_agent_composer_context()`（见 design.md 3.4.6），并在：

1. `enter_task_agent_edit`（`src/app/update/pane/task_page.rs:538-574`）：`page.agent_edit = Some(agent_id)` 之后调用；
2. `sync_lx_mention_root`（`src/app/update/pane/popups.rs:19-30`）：末尾追加 `let composer = pane.lx.sync_agent_composer_context();` 并并入返回值；
3. `set_mention_skills`、`set_mention_mcp_servers`、`sync_referenced_folders`（`panels.rs:113-129 / 343-349`）：更新自身后调用。

### T4.7 扫描归属

**文件**：`src/app/update/pane/popups.rs`

- 新增 `MentionScanOwner { Main, Btw(String), Agent(String) }`；
- `take_lx_mention_scan_requests` 返回 `Vec<(PaneId, MentionScanOwner, u64, PathBuf)>`；
- Agents 分支：`agents_editing()` + `agent_edit_id()` + `composer.take_mention_scan_request()`；
- `apply_lx_mention_entries` 增加 Agent 分支，校验 `agent_edit_id()` 仍是同一 agent。

**文件**：`src/event/mod.rs`

- `LxMentionScanned { pane, owner, generation, root, entries }`；
- `pump_lx_mention_scan` 与 dispatch 跟随改名。

**跟随修改的测试夹具**：

- `src/app/update/tests.rs:5111-5210`：`apply_lx_mention_entries(..., None, ...)` → `MentionScanOwner::Main`；
- `src/event/tests/test_support.rs:196`：同上。

### T4.8 键位

**文件**：`src/app/update/pane/task_page.rs:237-272`

- `apply_agent_edit_input` 第一行调用 `apply_agent_mention_input`；
- 新增 `apply_agent_mention_input`，镜像 `chat/btw/input.rs:263-303`（见 design.md 3.4.8）。

### T4 验收

- Agents 详情 composer 敲 `@` 弹出面板，顶栏 4 项；
- `←/→` 在 4 项内循环，不会出现 `Subagents`；
- Enter/Tab 采纳后输入框是 `@src/main.rs `（无 marker）；
- Enter（面板已关）发送给子 agent 的文本与输入框一致；
- Esc 两级退出；
- `/`、粘贴路径、md 块面板仍不弹出。

---

## T5：测试清单

| 文件 | 新增/修改 | 断言重点 |
| :--- | :--- | :--- |
| `src/ui/main_content/lx/task/tests/btw_list.rs` | 新增 | BTW 编辑态胶囊含 `QA 1 · 标题`、不含 `To:`；窄窗格整体不画 |
| `src/ui/main_content/lx/task/tests/agent_list.rs` | 修改 | 含 `Turing [executor]`、不含 `To:`；窄窗格不越界 |
| `src/app/state/lx/mention/tests.rs` | 新增 | `mention_tabs()` 4 项且循环跳过 `Subagents`；`/` 与快捷引用保持关闭；四类朴素插入文本 |
| `src/app/update/chat/tests/task_page.rs` | 新增 | 面板打开时 Enter 不产生发送 effect；采纳后草稿正确；Esc 两级退出；agent 切换后迟到结果被丢弃 |
| `src/app/update/tests.rs` | 修改 | 主输入框扫描回投改用 `MentionScanOwner::Main` |
| `src/app/update/chat/btw/tests.rs` | 删除 | 移除 `target_label` 相关断言 |
| `src/app/state/lx/chat/tests.rs` | 新增 | `model_display_name` 命中配置名、缺失时回退 `model_id` |
| `src/event/tests/test_support.rs` | 修改 | `MentionScanOwner::Main` |

单测试文件红线：≤1500 行；超限按场景拆分（如 `mention/tests/agent_composer.rs`）。

实际落地：`mention/tests.rs` 890 行、`task_page.rs` 1116 行（均在红线内），未新增测试文件。

---

## T6：精准校验

按受影响范围执行（不做全量浪费）：

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib ui::main_content::lx::task
cargo test --lib app::state::lx
cargo test --lib app::update::pane
cargo test --lib app::update::chat::btw
cargo test --lib event::chat::tests
```

实际执行结果（2026-11-13，本 worktree）：

- `cargo fmt`：通过；
- `cargo clippy --all-targets -- -D warnings`：通过；
- `cargo test`：5433 passed / 0 failed（含全部集成测试；审查修复后）；
- 受影响模块：`ui::main_content::lx::task` 93 passed、`app::state::lx` 246 passed、`app::update` 986 passed、`app::update::chat::btw` 18 passed、`event::chat::tests` 120 passed。

交互观感仍需用户 `cargo run` 复核：

1. BTW 编辑态胶囊无 `To:`；
2. BTW 回答统计行显示模型名，重启后不变；
3. Agents composer 胶囊为 `Turing [executor]`；
4. Agents composer 的 `@` 四 Tab、朴素插入、Enter/Esc 手感。

---

## 验收清单（全部满足才算完成）

- [x] BTW composer 右上角不再出现 `To:`，目标语义仍正常（@agent 回答归属正确）。
- [x] Agents composer 右上角不再出现 `To:`，显示 `xxx[role]`。
- [x] BTW 统计行首段是模型显示名；live 与 reload 一致。
- [x] Agents 追问 composer 支持 `@`，且只有 Files/RefDir/Skills/MCP 四个 Tab。
- [x] `@` 选中后写入朴素文本，发送给子 agent 的文本与输入框一致。
- [x] `/`、markdown 块面板、路径粘贴参照面板在 Agents composer 中保持关闭。
- [x] 迟到扫描结果不会污染已切换的 agent。
- [x] `cargo fmt`、`clippy -D warnings`、受影响模块测试全部通过。
