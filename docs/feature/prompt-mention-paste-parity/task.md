# Prompt / Lx `@` 提及与粘贴面板交互对齐任务清单

> 设计：`docs/feature/prompt-mention-paste-parity/design.md`
> 状态：**已实施并通过评审修正**（2026-11-14）。评审记录见 `review.md`。
> 执行顺序：Task 1 → Task 2 → Task 3 → Task 4 → Task 5。每个 Task 完成后只跑对应精确校验，最后统一 fmt/clippy/test。

---

## Task 1：粘贴提案暂停自动保存（B1）

### 1.1 新增统一判断

- [x] `src/app/prompt.rs`：在 `impl Prompt` 增加

```rust
/// 路径或文本粘贴提案是否仍在等待用户选择；提案期间不得落库、不得安排自动保存。
pub fn paste_proposal_open(&self) -> bool {
    self.paste_panel().is_some() || self.text_paste_panel().is_some()
}
```

### 1.2 自动保存门禁

- [x] `src/app/prompt.rs::Prompt::autosave_deadline`：在 `is_saved` 判断后增加 `|| self.paste_proposal_open()` 早退。
- [x] `src/app/update/interaction.rs::tick_prompt_autosave`：把 `state.prompt.paste_panel().is_some()` 换成 `state.prompt.paste_proposal_open()`。
- [x] 预期：`update::next_deadline` 因 `autosave_deadline == None` 自动停止返回期限；路径/文本提案期间事件循环不空转。

### 1.3 复用判断，消除重复

- [x] `src/app/update/prompt/save.rs::save_prompt_now`：重复的 `paste_panel().is_some() || text_paste_panel().is_some()` 改为 `state.prompt.paste_proposal_open()`。
- [x] `src/app/prompt.rs::undo` / `redo`：同上。
- [x] `src/app/update/prompt/save.rs::commit_prompt_buffer` 保留先取消再提交，不改语义。

### 1.4 精确校验

- [x] `cargo test autosave_defers_while_paste_proposal_open -- --nocapture`
- [x] `cargo test paste_proposal -- --nocapture`

---

## Task 2：Prompt 提及时机（B2 + B3）

### 2.1 收尾分流

- [x] `src/app/prompt.rs`：
  - `settle()` 改为 `self.settle_panels(true)`。
  - 新增 `fn settle_moved(&mut self) { self.settle_panels(false); }`。
  - `settle_panels` 增加 `edited: bool` 参数：
    - `edited == true`：清 `panel_suppressed`，`mention.unsuppress()`，`mention.refresh(...)`。
    - `edited == false`：不动 `panel_suppressed`，调用 `mention.refresh_if_active(...)`。
    - 其余 refer / slash / block 收敛逻辑保持原样。

### 2.2 移动调用点改为 `settle_moved()`

- [x] `src/app/prompt/navigation.rs`：
  - `move_left`、`move_right`
  - `move_up`（两个分支）、`move_down`（两个分支）
  - `move_home`（两个分支）、`move_end`（两个分支）
  - `move_line_start`、`move_line_end`
  - `move_word_backward`、`move_word_forward`
- [x] `src/app/prompt/viewport.rs`：
  - `set_cursor_from_cell`
  - `set_cursor_offset`
- [x] 不修改：删除类方法、`indent`/`outdent`、列表续写、模板改写、`replace_range`、`undo/redo` 恢复、`mention_enter_folder`、粘贴改写的 `settle()`。

### 2.3 MentionState 不新开

- [x] `src/app/prompt/mention.rs`：
  - 新增 `pub(super) fn refresh_if_active(&mut self, text: &str, cursor: usize)`：`trigger.is_some() || suppressed` 才 `refresh`。
  - `set_root`：在清理缓存/generation 前记录 `was_session = trigger.is_some() || suppressed`；根变化后清缓存与 `pending`，**压制态保持不变**；仅 `was_session` 为真时 `refresh`；其余情况不重算、不新开。
  - `set_worktree_repo`、`apply_worktree_entries`、`worktree_failed` 的 `refresh` 改为 `refresh_if_active`。
  - `take_scan_request`：去掉现算 `markdown::mention_trigger`，改为要求 `!self.suppressed && self.trigger.is_some()`；`pending` / `cache.is_some()` 早退保持不变。
- [x] `src/app/prompt.rs::set_mention_skills` / `set_mention_mcp_servers`：`mention.refresh(...)` 改为 `mention.refresh_if_active(...)`。
- [x] `Prompt::set_mention_root` 的 `panel_suppressed -> mention.suppress/unsuppress` 镜像保留；由于 `set_root` 不再新开，按键前的根同步不会绕过移动规则。

### 2.4 行为核对

- [x] 按 `@`：`sync_mention_root` 不新开 → 编辑 `settle(true)` 解除压制并重算 → 当次出面板。
- [x] Esc / 候选确认后：`panel_suppressed = true`；移动不清、不弹；下一次编辑清并弹。
- [x] 点击/方向键进入已有 `@`：`settle_moved(false)` + 无会话 → 不弹。
- [x] 在已有 `@` 中间打字：编辑 `settle(true)` → 弹。

### 2.5 测试

- [x] 新建 `src/app/prompt/tests/mention_timing.rs`，在 `src/app/prompt/tests.rs` 顶部声明 `mod mention_timing;`（与现有 `mod paste_folded;` 同风格）。
- [x] 覆盖：
  - `cursor_move_into_existing_mention_does_not_open_panel`
  - `typing_inside_existing_mention_opens_panel`
  - `edit_after_mention_confirm_reopens_on_first_keystroke`
  - `sync_root_does_not_open_closed_mention_panel`
- [x] 新建 `src/app/update/prompt/tests/mention_timing.rs`，在 `src/app/update/prompt/tests.rs` 顶部声明 `mod mention_timing;`；用 `apply_editor` 复现“先 sync 根、再插入 `@`”的事件顺序，断言第一下出面板。
- [x] 跑：
  - `cargo test mention_panel -- --nocapture`
  - `cargo test mention_timing -- --nocapture`
  - `cargo test apply_editor -- --nocapture`

---

## Task 3：Lx 提及时机（B2 + B3）

### 3.1 收尾分流

- [x] `src/app/state/lx/panels.rs`：
  - `refresh_panel()` 改为 `self.refresh_panels(true)`。
  - 新增 `pub fn refresh_panel_moved(&mut self) { self.refresh_panels(false); }`。
  - `refresh_panels(edited: bool)`：
    - 保留原有 `allow_panels` / Shell / 二级斜杠面板守卫。
    - `edited == true`：`mention.unsuppress()` + `mention.refresh(...)`。
    - `edited == false`：`mention.refresh_if_active(...)`。
    - 后续 `mention.is_open() || mention_visible()`、refer、block 分支保持原逻辑。

### 3.2 移动调用点改为 `refresh_panel_moved()`

- [x] `src/app/state/lx/edit.rs`：`move_left`、`move_right`、`home`、`end`、`move_up`、`move_down`、`word_left`、`word_right`。
- [x] `src/app/state/lx/selection.rs`：`doc_start`、`doc_end`。
- [x] 不修改：`insert_char`、`insert_str`、`newline`、`backspace`、`delete`、删除到行首/行尾、按词删除、`clear`、`undo/redo`、`delete_selected`、历史恢复等编辑路径。
- [x] `begin_text_selection_with_clicks` 的 `mention.suppress()` 保留。

### 3.3 LxMention 不新开

- [x] `src/app/state/lx/mention.rs`：
  - 新增 `pub fn refresh_if_active(&mut self, text: &str, cursor: usize)`：`trigger.is_some() || suppressed` 才 `refresh`。
  - `view`：在 `suppressed || mention_trigger(...).is_none()` 之外增加 `self.trigger.is_none()` 早退，防止 UI 绕过状态层现算触发。
  - `set_root`：根变化仍清缓存/`pending`/`generation`，但仅在已有会话或压制态时 `refresh`；不新开。
  - `sync_referenced_folders`、`set_skills`、`set_mcp_servers`、`set_disabled_mcp_servers`、`set_subagent_roles`、`sync_subagent_runs`：`refresh` 改为 `refresh_if_active`。
  - `take_scan_request`：增加 `self.suppressed` 与 `self.trigger.is_none()` 早退；去掉现算 `mention_trigger`（不再使用的 `text`/`cursor` 参数一并删除）。
- [x] `src/app/state/lx/panels.rs::sync_mention_subagent_runs` 依赖的 `sync_subagent_runs` 已按上条收敛。

### 3.4 测试

- [x] `src/app/state/lx/mention/tests.rs` 增加：
  - `cursor_move_into_existing_mention_does_not_open_panel`
  - `escape_then_move_stays_closed_until_edit`
  - `background_skill_sync_does_not_open_closed_panel`
- [x] 既有 `@` / Esc / 目录 / Tab / 粘贴测试全部保持通过。
- [x] 跑：
  - `cargo test state::lx::mention -- --nocapture`
  - `cargo test lx_mention -- --nocapture`

---

## Task 4：自动保存回归测试

- [x] 新建 `src/app/update/prompt/tests/mention_timing.rs`（若 Task 2 已建则复用同一文件），覆盖：
  - `autosave_defers_while_text_paste_proposal_open`
  - `autosave_does_not_close_path_paste_panel_with_pending_watch`
- [x] 断言点：
  - 提案期间 `tick` 返回 false、`effects` 为空、面板仍在、`next_deadline` 为 None。
  - 确认面板后下一轮静置 tick 产生 `Effect::SavePromptContent`。
- [x] 跑：`cargo test autosave -- --nocapture`

---

## Task 5：统一校验与文档收尾

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] 检查未新增空目录/空文件、未改数据库、未改 Git 状态。
- [x] 在设计文档补“实施结果”一节（修改点、测试结果、未验证项）。

---

## 验收清单（用户 `cargo run`）

- [x] 已保存 prompt 中“改字 → 立刻粘贴普通文本”：面板不再约 0.8 秒消失；选择/Esc/外部点击才关闭。
- [x] 确认 `@` 候选后立刻输入 `@`：第一下就出 `scanning…` 或候选列表。
- [x] 已有 `@src/app.rs `：鼠标点进/方向键移进路径中间不弹；同位置打字会弹。
- [x] Esc 后左右移动不弹；直接打字当次弹。
- [x] lx 输入框行为与 Prompt 一致。
- [x] `/` 斜杠、Markdown 块、字母快捷、路径/文本粘贴面板原有行为不回退。

---

## 风险与未验证

- 本环境无法交互式验证 TUI 外观与真实鼠标/终端粘贴时序；需用户 `cargo run` 确认。
- opencode TUI 的 `dismissedValue` 是“相同文本不重开”；本次用“文本编辑才解除压制”的等价布尔语义，未引入逐字符比较。
- 不改桌面端的原子 mention 模型；本任务只对齐 TUI 的触发时机。


---

## 评审修正（review.md）

- [x] P1：`LxMention::abandon_scan` / `Lx::abandon_mention_scan`；BTW 扫描结果被路由丢弃时清原 composer 的 `pending`。
- [x] P2：`MentionState` / `LxMention::refresh`、`set_root`、`take_scan_request` 统一在状态层遮蔽折叠粘贴块。
- [x] P2：`toggle_pane_view` 切回 lx 使用 `refresh_panel_moved()`，不解除压制、不新开面板。
- [x] P2：重写路径粘贴自动保存测试；lx 后台同步测试把光标放进 `@main.rs`；补 Prompt 侧同类测试。
- [x] P2：同步 `task.md` 与字段注释的压制生命周期描述。
- [x] 验证：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
