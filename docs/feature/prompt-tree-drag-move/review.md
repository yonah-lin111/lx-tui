# prompt-tree-drag-move 评审

评审范围：`git diff dev...HEAD`（24 文件）+ 未提交的交互返工增量（10 文件）。
结论按可复现的失败路径分级，未核实到的猜测不入表。

分级口径：**P0** = 会写错数据或用户未操作却发生变更；**P1** = 明确的逻辑/体验缺陷，后果可逆；
**P2** = 健壮性、可读性、文档漂移。

## 修复状态（2026-10-25）

| 项 | 状态 | 落点 |
| --- | --- | --- |
| P0-1 | ✅ 已修 | `src/event/mouse/wheel.rs` 早退 + `mouse_wheel_during_prompt_drag_keeps_viewport_and_state`（反向验证：去掉守卫即失败于 `scroll 1 != 0`） |
| P1-1 | ✅ 已修 | `editor.rs`（`toggle_prompt_pin` / `release_prompt_pin`）、`workspace.rs`（`close_workspace`）+ 2 用例 |
| P1-2 | ✅ 已修 | `DbError::PromptMissing`、`move_prompt` 存在性校验 + `rows_affected` + 1 用例 |
| P1-3 | ✅ 已修 | `design.md` §1.2/§1.3/决策 6/§6.1/§6.2/§9、`task.md:21` |
| P2-1 | ✅ 已修 | `Persistence::db_result` + `move_reject_message` 三路分流 + 3 用例 |
| P2-2 | ✅ 已修 | `PromptDrag.pressed_at`，双击改用按下时刻 + 2 用例（正/反例） |
| P2-3 | ✅ 按规范澄清关闭 | 撤销本条：测试文件本就不设行数上限，规范补明（`AGENTS.md` + `code-writing-standards.md`）。`prompt/tests.rs` 2493 行、`event/tests.rs` 6057 行**保持原样** |
| P2-4 | ✅ 已修 | `tree.rs` 加 `!rows.is_empty()` + 1 用例 |
| P2-5 | ✅ 已修 | `effective_scroll` 三处统一 + 2 用例 |

收口验证：`cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` 干净；
`cargo test --lib` **3420 passed / 0 failed / 2 ignored**；`cargo test --test prompt_management` **3 passed**。

### P2-5 附带的行为变更（必须记录）

修 P2-5 时 `row_at` / `drop_hit_at` 的语义从"`scroll` 越界即未命中"改为"钳到末行"——
与渲染对齐。全量调用点（`drag.rs`、`hover.rs`、`press.rs`、`overlay.rs`、`event/tests.rs`）
已核实无反向依赖。**同时改了一条既有用例的断言**：`row_at_maps_viewport_and_respects_scroll_and_search_row`
末尾 `panel.scroll = 3` 的期望从 `None` 变为 `Some(2)`。改断言是为了适配新语义，不是为了让测试变绿。

---

## P0

### P0-1 拖动中滚轮可滚动列表，松手即静默改容器

- 位置：`src/event/mouse/wheel.rs:163`（`handle_mouse_wheel` 无 `prompt_drag` 守卫）、
  `src/event/mouse/wheel.rs:268`、`src/app/update/prompt/tree_view.rs:233`（`scroll_prompt_tree`）
- 关键代码：

```rust
// wheel.rs:268 —— 拖动期照常执行
if update::prompt_tree_mode(state) {
    let visible = update::prompt_tree_visible_rows(state);
    if update::scroll_prompt_tree(state, direction, visible) {
        *dirty = true;
    }
}
```

```rust
// drag.rs:89 —— 阈值与落点都用“绝对行号”
let crossed = match hit {
    PromptDropHit::Row(row) => row != drag.from_row,
    PromptDropHit::Root => true,
    PromptDropHit::Outside => false,
};
let armed = drag.dragging || crossed;
```

- 关键点：`from_row` / `drop_mark` / `drop_hit_at` 三者全部基于 `panel.scroll + 视口行`
  换算出**绝对行号**。滚轮改 `panel.scroll` 后，指针像素没动，`drop_hit_at` 返回的绝对行
  却整体平移到 `from_row + Δ`。
- 错误原因：
  1. `crossed` 判定失效——指针原地不动也会被算成"已跨行"，`dragging` 被**误置臂**；
     `drop_mark` 随之算出目标容器并高亮。
  2. 松手时 `finish_prompt_drag` 看到 `dragging == true`，走移动分支而不是点击分支。
- 复现（长列表，`rows.len() > 可视行`）：
  1. 在文件夹 A 的条目上按下不松手；2. 滚轮滚一格；3. 直接松手，不做任何位移。
  → 条目被静默移进指针此刻所在行的容器（若该行属于文件夹 B，就移到 B）。
  用户视角是"我只滚了一下就点了一下，东西自己跑了"。
- 修复：`handle_mouse_wheel` 开头加 `if state.prompt_drag.is_some() { return; }`。
  拖动期间禁止改变视口（与 design §1.3「不做边缘自动滚动」同一条约束）。
  补回归：按下 → 滚轮 → 松手，断言 `state.effects` 为空且 `prompt_drag.is_none()`。

---

## P1

### P1-1 拖动中切钉住 / 关工作区不清拖拽现场，高亮在另一个面板上乱亮

- 位置：`src/app/update/editor.rs:364`（`toggle_prompt_pin`）、
  `src/app/update/workspace.rs:353`（`close_workspace`）
- 关键代码：

```rust
// 已清理现场的三处（对照）
// right_panel.rs:17 / workspace.rs:310 / prompt.rs:240
state.prompt_drag = None; // 拖拽现场不跨面板/工作区存活
```

```rust
// editor.rs:364 —— 缺清理
pub fn toggle_prompt_pin(state: &mut AppState) {
    match state.prompt_pinned {
        None => state.prompt_pinned = Some(state.active_workspace),
        Some(_) => release_prompt_pin(state),
    }
}
```

- 关键点：拖动现场里的 `drop_mark` 是对**旧面板** `rows()` 校验过的绝对行号，
  而 `handle_mouse_drag` 的命中测试与渲染都读 `state.prompt_owner()`（**新面板**）。
- 错误原因：拖动中按切钉住快捷键，`prompt_workspace()` 换人后：
  1. `handle_mouse_drag` 用新面板把像素换成行号；
  2. `drag_prompt_to → prompt_drop_mark` 拿这个行号去查**旧面板** `prompt_drop_container`，
     校验通过则返回 `Row(新面板行号)`；
  3. 渲染用新面板画这个行号，高亮落在与指针无关的行上；
  4. 松手时 `finish_prompt_drag` 因 `drag.workspace != prompt_workspace()` 直接丢弃，
     **没有任何反馈**——高亮亮着却什么也不发生。
- 复现：钉住工作区 A → 在其列表按下条目 → 拖动 → 按切钉住键 → 指针继续移动。
  → 高亮跳到别的行，松手静默。`close_workspace` 同理（关掉前面的工作区会让索引整体前移）。
- 修复：`toggle_prompt_pin` 与 `close_workspace` 都补 `state.prompt_drag = None;`
  （与 `switch_workspace` / `select_right_tab` 同口径）；`release_prompt_pin` 也补一份，
  因为 `toggle` 的 `Some(_)` 分支走它。

### P1-2 `move_prompt` 对被删/不存在的提示词静默成功，且报错类型错位

- 位置：`src/db/mod.rs:521`
- 关键代码：

```rust
let workspace_external: Option<String> = self.conn.query_row(
    "SELECT workspace_id FROM workspace_prompt WHERE id = ?1", params![id], |row| row.get(0),
).optional()?;
// 提示词不存在 → 却被报成“文件夹缺失”
let workspace_external = workspace_external.ok_or(DbError::FolderMissing)?;
...
self.conn.execute(                      // 无 rows_affected 校验
    "UPDATE workspace_prompt SET folder_id = ?1, updated_at = datetime('now') WHERE id = ?2",
    params![folder_external, id],
)?;
Ok(())
```

- 关键点：`folder_id == None` 时**整段校验被跳过**，直接 `UPDATE ... WHERE id = ?`；
  `execute` 的返回值（影响行数）被丢弃。`pump` 以 `is_some()` 判定成功（`service.rs:607`）。
- 错误原因：
  1. 条目在另一个 lx 实例里已被删除时，本实例继续拖它 → `UPDATE` 命中 0 行 → 返回 `Ok` →
     `pump` 认为成功、不出 `TOAST_PROMPT_MOVE_FAILED`；`reload_tree` 后条目消失，
     用户看到"拖了一下，东西没了"，无法归因。
  2. 提示词不存在被报成 `FolderMissing`，日志/错误码会把人引到错误方向。
- 复现：两个 lx 进程开同一库；A 删除条目 X；B 把 X 拖进文件夹（或拖回根层）→ B 无任何提示。
- 修复：`rows_affected() == 0` 时返回 `DbError::PromptMissing`（若已有同类变体则复用，
  无则新增并补 `code()`/`Display`）；`folder_id == None` 分支也要取一次提示词做存在性校验。
  补测试：`move_prompt(不存在的 id, None)` 与 `(不存在的 id, Some(f))` 都必须 `Err`。

### P1-3 feature 文档与实现漂移

- 位置：`docs/feature/prompt-tree-drag-move/design.md:55`、`:69`、`:441`、`:542`；
  `docs/feature/prompt-tree-drag-move/task.md:21`
- 关键代码：

```rust
// 实际调用点（ui/prompt_frame.rs:185）——传整个现场
state.prompt_drag.filter(|drag| drag.dragging),
```

```rust
// design.md:441 仍是旧签名
state.prompt_drag.and_then(|drag| drag.dragging.then_some(drag.from_row)),
```

- 关键点：§11.7 只在文末补了修订说明，正文里**决策表第 6 条**、§1.3 非目标、§6.2 调用点、
  §11 小结四处仍是"仅被拖行铺底色 / 不画落点预览"的旧口径，且 §6.2 的代码快照已编译不过。
- 错误原因：`AGENTS.md` 规定这些文档是唯一来源。下次改这块时按 §6.2 抄代码会直接抄到过期签名；
  按决策表第 6 条实现会把刚落地的落点反显又删掉。
- 修复：决策表第 6 条与 §1.3、§6.2、§542 统一改为"源行压暗 + 目标容器行铺底色 +
  根层提示行"，§6.2 换成真实签名；`task.md:21` 的"质问 6"改写为最终口径并注明由 §11.7 取代。

---

## P2

### P2-1 DB 拒绝原因在 UI 层不可区分

- 位置：`src/db/service.rs:601-643`
- 关键代码：

```rust
let moved = self.with_db(|db| db.move_prompt(id, folder_id)).is_some();
...
text::TOAST_PROMPT_MOVE_FAILED,   // 两种情况共用一句
```

- 关键点：状态层已用 `TOAST_PROMPT_MOVE_DEPTH` / `TOAST_PROMPT_MOVE_NESTED` 给出精确原因，
  但 DB 层返回 `DepthExceeded` / `FolderHasChildren` 时只剩笼统的 `Failed to move; unchanged`。
- 错误原因：两层规则重复实现（design §10.3 已知），一旦漂移，用户只能看到最模糊的那句。
- 修复：`with_db` 返回的错误需要透传类型（现在被压成 Option），或至少按 `DbError::code()`
  分流到两条精确文案。改动量不小，先记录；短期可在 `pump` 里按 `code` 二分。

### P2-2 双击判定改到 Up 后阈值语义变了且无覆盖

- 位置：`src/app/update/prompt/tree_view.rs:129`、`drag.rs:159-165`
- 关键代码：

```rust
let now = Instant::now();                       // Up 时刻
if prompt_row_double_click(state, drag.from_row, now) { ... }
```

- 关键点：`PROMPT_DOUBLE_CLICK_WINDOW = 400ms` 从"Down→Down"变成"Up→Up"，
  实际两次 Down 的间隔可以到 400ms + 两次按压时长。
- 错误原因：慢速单击若按下超过窗口，第二次会被判成单击；反之长按后快速连点会被判成双击。
- 修复：把按下时刻也记进现场（`PromptDrag` 加 `pressed_at: Instant`），
  双击用按下时刻判定，保持原语义。补一条"慢速单击不触发编辑器"的用例。

### P2-3 测试文件超 1000 行上限 —— 已按规范澄清撤销

- 位置：`src/app/update/prompt/tests.rs`、`src/event/tests.rs`
- 原判断：按 `AGENTS.md:16` 的"单文件行数上限 1000"要求拆分。
- **该判断有误，条目撤销**：1000 行约束针对**生产代码**。测试的拆分依据是被测对象与场景
  归属，不是体积——一个模块的用例就该躺在它自己的 `tests.rs` 里，按行数切开只会让同一模块
  的用例散落多处，检索成本反而上升。
- 处理：在 `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 明确"测试文件不受
  1000 行限制"，并写明两种**应当**拆分的情形（跟随生产模块拆分、按无关测试簇归属），
  以及禁止"按体积切段"。上述两个文件保持原样，不做搬运。
- 原始风险仅剩一条：单文件过大时整文件重新解析的编译期成本上升，属可接受代价。

### P2-4 空树时根层提示行会覆盖空状态占位

- 位置：`src/ui/prompt/tree.rs:105-119`
- 关键代码：

```rust
if drop_mark == PromptDropMark::Root {
    let blank = rows.len().saturating_sub(scroll);
    if blank < usize::from(rows_area.height) {
        let y = rows_area.y + blank as u16;   // rows 为空时 blank == 0
```

- 关键点：`rows.is_empty()` 时 `blank == 0`，提示行落在 `rows_area.y`，
  正好是"no prompts yet"占位那一行，后画的提示会把它盖掉。
- 错误原因：当前不可达（`begin_prompt_drag` 需要有行才登记现场），但这是一条"改了门槛
  就会静默生效"的隐蔽路径，没有防护。
- 修复：条件加 `!rows.is_empty()`，与上一段空状态互斥。

### P2-5 `drop_hit_at` 与渲染的 `scroll` 钳制不一致

- 位置：`src/ui/prompt/tree.rs:44` vs `:224`/`:237`
- 关键代码：

```rust
// 渲染：钳制
let scroll = panel.scroll.min(rows.len().saturating_sub(1));
// 命中：直接用
let index = panel.scroll + usize::from(row - rows_y);
```

- 关键点：`panel.scroll > rows.len()-1` 时（行数骤减而无 `clamp_selection` 的瞬时窗口），
  渲染显示末行，命中却算出越界索引 → 判成 `Root`。
- 错误原因：同一份几何两份换算，钳制只做了一半。
- 修复：`row_at` / `drop_hit_at` 也用同一个钳制后的 `scroll`（抽一个 `fn effective_scroll(panel) -> usize`）。

---

## 已核实无问题

- 落点行号链路：状态层 `rows().iter().position()` 与渲染 `enumerate()` 都是**绝对下标**，
  搜索行/分割线由 `rows_area.y = area.y + 2` 统一扣除，`scroll` 两侧口径一致——不会错行。
- `Source` 与 `Target` 不可能同时命中同一行：拖到自己 / 同一容器都走 `Noop`，
  `prompt_drop_mark` 返回 `None`。
- 状态层与 DB 层的文件夹规则一致（先 `Nested`/子文件夹，后 `Depth`），
  子文件夹拖回根层走 `Ok(None)` 早退是刻意且正确的（层数不变）。
- 事件层余下的清理点齐全：`switch_workspace`、`select_right_tab`、`toggle_prompt_panel_mode`、
  新一次 `Down` 都会丢弃残留现场；`finish_prompt_drag` 另有 `workspace` 校验兜底。
- 命中矩形一致：`press.rs` 经 `pane_content_inner → paste::prompt_text_area →
  prompt_content_rect`，与 `drag.rs` 直接用 `prompt_content_rect` 是同一矩形。
- `workspace_folder.parent_folder_id` 与 `workspace_prompt.folder_id` 都存 `external_id`
  且全链路联查一致，`move_folder` 的 `children` 计数用 `dragged_external` 比对正确。
- `SCHEMA_VERSION` 仍为 14，本特性未新增迁移，`src/db/tests.rs` 与新代码自洽。
