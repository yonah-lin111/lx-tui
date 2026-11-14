# Prompt / Lx `@` 提及与粘贴面板交互对齐设计

状态：**已实施、通过评审修正并合并到 `dev`**（2026-11-14）。评审记录见 `review.md`。

关联任务清单：`docs/feature/prompt-mention-paste-parity/task.md`

参考实现：opencode TUI
- `packages/tui/src/component/prompt/autocomplete.tsx`（`onInput` / `dismissedValue` / `hide`）
- `packages/tui/src/prompt/display.ts`（`mentionTriggerIndex`）
- 桌面端 `packages/app/src/composer/editor/editor.tsx` 把 mention 渲染为不可编辑的原子块；本任务不改文本存储形态，不照搬这一点。

---

## 1. 目标与范围

修三个问题，两侧输入框（右侧 Prompt 与 lx 主输入框）统一到同一套交互时机：

| 编号 | 问题 | 目标行为 |
| --- | --- | --- |
| B1 | 粘贴普通文本后，文本粘贴面板（`Paste raw/Text/Code/Log`）约 0.8 秒后自己消失 | 任一粘贴提案（路径/文本）打开期间暂停自动保存，面板只能由用户确认/取消/外部点击关闭 |
| B2 | 使用过 `@` 命令后，再输入 `@` 不显示面板；再输入一个字符才显示 | 文本编辑当次按键就重新武装并重算面板，不再延迟一拍 |
| B3 | 光标落入已有 `@` 内容中（鼠标点击或方向键移动）会打开 `@` 面板 | 对齐 opencode TUI：面板只由文本编辑触发；纯移动/点击只关闭或收敛已打开的面板，不新开 |

范围：
- `src/app/prompt/`（右侧 Prompt 状态）
- `src/app/state/lx/` + `src/app/state/lx.rs`（lx 输入框状态）
- `src/app/update/`（自动保存与事件层顺序）
- 对应单元测试

非目标：
- 不改 `@` 的文本存储形态（不引入桌面端那种原子 mention 块）。
- 不改 `@` 语法、候选来源、扫描根、Tab 列表、面板视觉。
- 不新增 loading/empty UI；复用现有 `scanning…` / `no matches` 行。
- 不改 `/` 斜杠、Markdown 块、字母快捷面板的打开时机（仅让它们与新的 settle 分流共用同一收尾路径）。
- 不改数据库、schema、持久化格式。

---

## 2. 用户已确认决策

| 决策 | 内容 |
| --- | --- |
| D1 | B1 根因就是自动保存 tick 漏掉文本粘贴面板；任一粘贴提案打开时暂停自动保存，并停止安排自动保存唤醒 |
| D2 | 统一采用 opencode TUI 的时机模型：`@` 面板只由**文本编辑**触发 |
| D3 | 纯光标移动、鼠标点击只允许关闭/收敛已打开的 `@` 面板，不能新开 |
| D4 | Esc / 选中候选后的压制，只有**文本再次变化**才解除；移动光标不解除 |
| D5 | 在已有 `@` 路径中间**打字**仍会打开面板（opencode TUI 同款）；桌面端因 mention 是原子块才进不去 |
| D6 | 上述规则同时作用于右侧 Prompt 与 lx 输入框；不改 `@` 文本存储形态 |
| D7 | 临时草稿（`[tmp]`）不参与自动保存，B1 无需额外处理 |

---

## 3. 根因分析

### 3.1 B1：自动保存把文本粘贴面板收掉

`tick_prompt_autosave` 只挡了路径粘贴面板 `paste_panel`，没挡文本粘贴面板 `text_paste_panel`：

```rust
// src/app/update/interaction.rs:362
fn tick_prompt_autosave(state: &mut AppState, now: Instant) -> bool {
    if !prompt_item_editing(state)
        || state.prompt.paste_panel().is_some()
        || !state.prompt.autosave_due(now, PROMPT_AUTOSAVE_DELAY)
    {
        return false;
    }
    commit_editing_prompt(state);
    true
}
```

而提交入口会先取消文本粘贴面板：

```rust
// src/app/update/prompt/save.rs:26
pub fn commit_prompt_buffer(state: &mut AppState, workspace: usize) {
    state.prompt.paste_cancel();
    state.prompt.text_paste_cancel();
    ...
}
```

触发链：

1. 用户编辑已保存条目 → `is_saved = false`，`autosave` 观察开始计时（800ms）。
2. 用户粘贴普通文本 → `text_paste_candidate` 命中，打开 `text_paste_panel`；文本粘贴**不推进 revision**，旧计时继续。
3. 计时到点 → `tick_prompt_autosave` 看到 `paste_panel` 为 None，放行提交 → `commit_prompt_buffer` 调 `text_paste_cancel()` → 面板消失。

附带隐患：`src/app/update/interaction.rs:392` 的 `next_deadline` 不检查粘贴面板；路径面板打开时若已有 `autosave` 观察，会持续返回已过期 deadline，事件循环空转。

### 3.2 B2：`sync_mention_root` 与 `settle_panels` 只清了一个压制标记

事件层每个编辑器命令都会先同步根：

```rust
// src/app/update/editor.rs:359
pub(crate) fn sync_mention_root(state: &mut AppState) {
    ...
    state.prompt.set_mention_root(root);
}
```

`set_mention_root` 把 `Prompt.panel_suppressed` 镜像进 `MentionState.suppressed`：

```rust
// src/app/prompt.rs:741
pub fn set_mention_root(&mut self, root: Option<PathBuf>) {
    if self.panel_suppressed {
        self.mention.suppress();
    } else {
        self.mention.unsuppress();
    }
    self.mention.set_root(root, &self.text, self.cursor);
}
```

但编辑收尾只清 `panel_suppressed`，没有清 `mention.suppressed`：

```rust
// src/app/prompt.rs:932
fn settle_panels(&mut self) {
    ...
    self.panel_suppressed = false;
    ...
    self.mention.refresh(&masked, self.cursor); // 内部 if self.suppressed { reset; return; }
}
```

时序（实测）：

```text
confirm/Esc/paste 之后 panel_suppressed = true
按 '@'  -> sync_mention_root: mention.suppress()
        -> insert_char
        -> settle_panels: panel_suppressed = false，但 mention.suppressed 仍为 true
        -> mention.refresh 早退，面板不显示
再按 'a' -> sync_mention_root: 此时 panel_suppressed=false -> unsuppress
        -> mention.refresh 才显示
```

这就是“输入 `@` 不显示、再输入一个字符才显示”的直接原因。任何会把 `panel_suppressed` 置真的路径都会触发：`@` 候选确认、Esc、块/斜杠/快捷面板收束、粘贴后的压制。

### 3.3 B3：移动与根同步都会无条件重算面板

所有纯移动都调用同一个 `settle()`：

```rust
// src/app/prompt/navigation.rs:13
pub fn move_left(&mut self) {
    self.break_group();
    self.cursor = prev_pasted_boundary(&self.text, self.cursor);
    self.settle(); // -> settle_panels -> mention.refresh 无条件重算
}
```

鼠标落点同样走 `settle()`：

```rust
// src/app/prompt/viewport.rs:133
pub fn set_cursor_from_cell(&mut self, row: u16, col: u16) {
    self.break_group();
    self.cursor = self.viewport_offset((row, col), false);
    self.settle();
}
```

此外，每个按键前的 `sync_mention_root -> MentionState::set_root` 也无条件 `refresh`，即使根没变。因此把光标放进已有 `@src/app.rs` 中间时，`mention_trigger` 能解析出 `@src/app.r` 这样的查询并打开面板。

lx 现状同样是“方向键移进已有 mention 会打开”（实测），只有鼠标点击路径会 `mention.suppress()`。opencode TUI 的时机不同：

```ts
// packages/tui/src/component/prompt/autocomplete.tsx
onInput(value) {
  if (dismissedValue() === value) return
  setDismissedValue(undefined)
  ...
  const idx = mentionTriggerIndex(beforeCursor, renderer.widthMethod)
  if (idx !== undefined) { show("reference"); setStore("index", idx) }
}
```

- 只在内容变化 `onInput` 时计算触发；纯光标移动/点击不计算。
- Esc 后记录 `dismissedValue`，同一文本不再重开，文本一变才重新武装。
- 正在显示时，光标退到触发点之前或触发词中出现空白才关闭。

---

## 4. 统一交互模型

核心不变量：**`trigger` 是 `@` 会话是否存在的唯一真源；UI 不再从“文本+光标”自行推导会话。**

| 事件 | `@` 面板行为 |
| --- | --- |
| 文本编辑（插入/删除/换行/缩进/撤销重做/粘贴确认/进入目录） | 解除压制，`refresh` 全量重算；可新开面板 |
| 纯光标移动（←→↑↓/Home/End/行首行尾/按词移动） | 不解除压制；仅当 `trigger` 已存在时重算（用于收敛/关闭），不新开 |
| 鼠标点击/拖拽落点 | 同纯移动；不新开 |
| Esc / 选中候选 | 重置会话并压制；直到下一次文本编辑 |
| 根同步（工作区 cwd / worktree / 绑定切换） | 只更新根与缓存；仅当会话已存在或处于压制态时重算，不新开 |
| 后台快照同步（skills / MCP / refDir / subagent / worktree 列表） | 同上：只刷新已存在会话，不新开 |
| 扫描结果回投 | 会话仍在则刷新；会话已关闭则丢弃/保持关闭 |
| 粘贴普通文本/路径 | 维持现有压制语义，不因粘贴文本重新开 `@` 面板 |
| 粘贴提案打开 | 自动保存暂停；直到用户确认/取消/外部点击 |

`@` 输入框右侧 Prompt 已有 `MentionState.trigger`；lx 需要让 `LxMention::view` 也改为读取已存 `trigger`，不再每次用 `mention_trigger(text,cursor)` 现算，否则 UI 会绕过状态层把面板画出来。

---

## 5. 详细设计

### 5.1 粘贴提案暂停自动保存（B1）

新增统一的提案判断：

```rust
// src/app/prompt.rs（impl Prompt）
/// 路径或文本粘贴提案是否仍在等待用户选择；提案期间不得落库、不得安排自动保存。
pub fn paste_proposal_open(&self) -> bool {
    self.paste_panel().is_some() || self.text_paste_panel().is_some()
}
```

改动点：

1. `Prompt::autosave_deadline`（`src/app/prompt.rs:278`）在 `is_saved` 之外增加 `self.paste_proposal_open()` 早退；这样 `update::next_deadline` 自动停止安排唤醒，消除路径面板期间的空转。
2. `tick_prompt_autosave`（`src/app/update/interaction.rs:364`）改用：

```rust
if !prompt_item_editing(state)
    || state.prompt.paste_proposal_open()
    || !state.prompt.autosave_due(now, PROMPT_AUTOSAVE_DELAY)
{
    return false;
}
```

3. `save_prompt_now`（`src/app/update/prompt/save.rs:213`）与 `undo/redo`（`src/app/prompt.rs:506/521`）把重复的 `paste_panel().is_some() || text_paste_panel().is_some()` 换成 `paste_proposal_open()`，防止遗漏再次发生。
4. `commit_prompt_buffer` 保留“先取消提案再提交”的行为；它只由显式提交/切换目标/退出触发，不再被自动保存 tick 在提案期间调用。

行为：
- 提案打开期间 `is_saved` 保持 `false`，正文保持脏；面板确认后，下一次 tick 按既有 800ms 静置规则落库。
- 临时草稿本来 `prompt_item_editing == false`，行为不变。

### 5.2 Prompt：编辑与移动分流（B2 + B3）

`Prompt` 的收尾拆成两条路径：

```rust
// src/app/prompt.rs
fn settle(&mut self) {
    self.settle_panels(true);
}

fn settle_moved(&mut self) {
    self.settle_panels(false);
}

fn settle_panels(&mut self, edited: bool) {
    self.preview_selection = None;
    self.preview_drag = None;
    if edited {
        // 只有文本变化才解除 Esc/确认后的压制，并清掉供根同步镜像的标记。
        self.panel_suppressed = false;
    }
    self.scroll_cursor_into_view();
    let masked = crate::app::paste::mask_pasted_text(&self.text).into_owned();
    if edited {
        self.mention.unsuppress();
        self.mention.refresh(&masked, self.cursor);
    } else {
        self.mention.refresh_if_active(&masked, self.cursor);
    }
    if self.mention.panel().is_some() {
        self.slash = None;
        self.panel = None;
        self.refer = None;
        return;
    }
    self.refresh_refer_panel();
    if self.refer.is_some() {
        self.slash = None;
        self.panel = None;
        return;
    }
    self.refresh_slash_panel();
    if self.slash.is_some() {
        self.panel = None;
        return;
    }
    self.refresh_panel();
}
```

移动调用点改为 `settle_moved()`：

| 文件 | 方法 |
| --- | --- |
| `src/app/prompt/navigation.rs` | `move_left`、`move_right`、`move_up`、`move_down`、`move_home`、`move_end`、`move_line_start`、`move_line_end`、`move_word_backward`、`move_word_forward` |
| `src/app/prompt/viewport.rs` | `set_cursor_from_cell`、`set_cursor_offset` |

其余 `settle()` 调用（插入、删除、换行、列表续写、缩进、模板块改写、撤销恢复、`replace_range`、`mention_enter_folder`、粘贴改写等）保持编辑语义。

`MentionState` 增加“只收敛、不新开”的入口：

```rust
// src/app/prompt/mention.rs
/// 仅当当前已有 `@` 会话（含压制态）时重算；纯移动/后台同步不得新开面板。
pub(super) fn refresh_if_active(&mut self, text: &str, cursor: usize) {
    if self.trigger.is_some() || self.suppressed {
        self.refresh(text, cursor);
    }
}
```

`refresh` / `set_root` / `take_scan_request` 在状态层统一 `mask_pasted_text`：
调用方可以传原始文本（根同步、后台快照、操作根选择），折叠粘贴块正文不得参与
围栏/块绑定判断。这样不会出现“编辑路径用 masked、同步路径用 raw”的分裂。

`set_root` 改为不新开面板，也不在根/绑定切换时解除压制：

```rust
pub fn set_root(&mut self, root: Option<PathBuf>, text: &str, cursor: usize) {
    let previous = self.effective_root(text, cursor);
    let had_session = self.trigger.is_some() || self.suppressed;
    self.root = root;
    let current = self.effective_root(text, cursor);
    if current != previous {
        self.cache = None;
        self.pending = false;
        self.generation = self.generation.wrapping_add(1);
        // 压制态保持不变：根切换不是文本编辑，不得解除 Esc/确认后的压制。
    }
    if had_session {
        self.refresh(text, cursor);
    }
}
```

后台同步入口全部改为 `refresh_if_active`：
- `MentionState::set_worktree_repo`
- `MentionState::apply_worktree_entries`
- `MentionState::worktree_failed`
- `Prompt::set_mention_skills` / `Prompt::set_mention_mcp_servers` 中直接调用的 `refresh`

扫描请求改为只认已存在会话，避免“移动进已有 `@`”触发无意义扫描：

```rust
pub fn take_scan_request(&mut self, text: &str, cursor: usize) -> Option<(u64, PathBuf)> {
    if self.pending || self.suppressed || self.trigger.is_none() {
        return None;
    }
    let root = self.effective_root(text, cursor)?;
    if self.cache.is_some() {
        return None;
    }
    self.pending = true;
    Some((self.generation, root))
}
```

`apply_entries` 在“代匹配才写入”之后同样走 `refresh_if_active`：扫描结果只喂缓存，
不得把移动/根同步期间已关闭的会话重新打开；下一次文本编辑再按缓存出列表。

### 5.3 Lx：同一套时机模型（B2 + B3）

`Lx::refresh_panel` 拆分：

```rust
// src/app/state/lx/panels.rs
pub fn refresh_panel(&mut self) {
    self.refresh_panels(true);
}

/// 纯光标移动/点击后的收尾：只收敛现有面板，不新开 `@` 面板、不解除压制。
pub fn refresh_panel_moved(&mut self) {
    self.refresh_panels(false);
}

fn refresh_panels(&mut self, edited: bool) {
    // 原有 allow_panels / Shell / 二级斜杠面板守卫不变
    ...
    self.sync_mention_subagent_runs();
    let masked = crate::app::paste::mask_pasted_text(&self.input).into_owned();
    if edited {
        self.mention.unsuppress();
        self.mention.refresh(&masked, self.cursor);
    } else {
        self.mention.refresh_if_active(&masked, self.cursor);
    }
    if self.mention.is_open() || self.mention_visible() {
        self.panel = None;
        self.refer = None;
    } else if self.slash.is_some() || self.paste.is_some() || self.text_paste.is_some() {
        self.refer = None;
    } else {
        self.refresh_refer_panel();
        if self.refer.is_some() {
            self.panel = None;
        } else {
            self.panel = block_trigger(&masked, self.cursor).map(|trigger| {
                let items = block_commands(trigger.kind);
                BlockPanel::new(trigger, items, 0)
            });
        }
    }
    self.scroll_cursor_into_view();
}
```

移动调用点改为 `refresh_panel_moved()`：

| 文件 | 方法 |
| --- | --- |
| `src/app/state/lx/edit.rs` | `move_left`、`move_right`、`home`、`end`、`move_up`、`move_down`、`word_left`、`word_right` |
| `src/app/state/lx/selection.rs` | `doc_start`、`doc_end` |

`begin_text_selection_with_clicks` 现有的 `mention.suppress()` 保留（点击即会话结束）；`select_*` 只扩展选区、不重算面板，保持不变。

`LxMention` 增加：

```rust
/// 仅当当前已有 `@` 会话（含压制态）时重算；移动/后台同步不得新开面板。
pub fn refresh_if_active(&mut self, text: &str, cursor: usize) {
    if self.trigger.is_some() || self.suppressed {
        self.refresh(text, cursor);
    }
}
```

`view` 改为读取已存 `trigger`，与 Prompt 的 `view_open` 对齐（否则 UI 仍会按文本现算出面板）；
文本判定与 `refresh` 一样先 `mask_pasted_text`，且 `refresh` 内部也统一遮蔽，避免折叠 token 正文让 UI 与状态层判断不一致：

```rust
pub fn view(&self, text: &str, cursor: usize) -> LxMentionView {
    let masked = crate::app::paste::mask_pasted_text(text);
    if self.suppressed
        || self.trigger.is_none()
        || mention_trigger(&masked, cursor).is_none()
    {
        return LxMentionView::Hidden;
    }
    ...
}
```

后台快照同步改为 `refresh_if_active`：
- `set_root`（根变化清缓存/generation 后，仅已有会话才重算）
- `sync_referenced_folders`
- `set_skills`
- `set_mcp_servers`
- `set_disabled_mcp_servers`
- `set_subagent_roles`
- `sync_subagent_runs`

`take_scan_request` 同样只认已存在会话（不再现算文本，参数随之去掉）：

```rust
pub fn take_scan_request(&mut self) -> Option<(u64, PathBuf)> {
    if self.tab != MentionTab::Files
        || self.pending
        || self.suppressed
        || self.trigger.is_none()
        || self.cache.is_some()
    {
        return None;
    }
    let root = self.root.clone()?;
    self.pending = true;
    Some((self.generation, root))
}
```

### 5.4 与事件层的关系

事件层顺序保持“先同步根、再执行命令”：

```text
apply_editor(command)
  -> sync_mention_root        // set_root 只更新根/缓存，不再新开面板
  -> route_panel              // 面板按键优先
  -> 编辑/移动命令
       -> settle / settle_moved
```

因此：
- 按 `@`：`sync` 不新开 → `insert_char` 编辑收尾解除压制并 `refresh` → 当次按键出面板（有缓存直接列表，无缓存出 `scanning…`）。
- 点击/方向键落进已有 `@src/app.rs`：`sync` 不新开 → `set_cursor_*` 或移动走 `settle_moved` → 会话不存在 → 不弹。
- 在已有 `@` 路径中间打字：编辑走 `settle` → 解除压制并重算 → 弹（D5）。
- Esc 后移动：`panel_suppressed` 不清、`mention.suppressed` 不解除 → 不弹；下一次文本编辑才解除。

---

## 6. 不变量

1. 任一粘贴提案（`paste_panel` / `text_paste_panel`）打开期间，`tick_prompt_autosave` 不提交，`next_deadline` 不返回自动保存期限；提案正文不得落库。
2. `@` 面板的新开只能由文本编辑路径触发；移动、点击、根同步、后台快照同步不得新开。
3. `MentionState.trigger` / `LxMention.trigger` 是会话存在的唯一真源；UI 渲染与会话判断都读它。
4. Esc/确认后的压制只由文本编辑解除；移动不解除。
5. 移动路径不得改变 `revision`、正文、撤销栈。
6. 现有 `@` 语法、候选来源、扫描根、Tab 列表、面板尺寸与配色不变。

---

## 7. 测试计划

### 7.1 自动保存与粘贴提案

文件：`src/app/update/prompt/tests/mention_timing.rs`（新建子模块，`tests.rs` 内声明 `mod mention_timing;`）

- `autosave_defers_while_text_paste_proposal_open`
  - 打开条目编辑视图；`begin_text_paste` 打开文本粘贴面板。
  - `tick(now)` 与 `tick(now + 900ms)` 均返回 false；`effects` 为空；面板仍在；`next_deadline` 为 None。
  - 确认 `Paste raw` 后，继续 tick 到静置达标；断言产生 `Effect::SavePromptContent`。
- `autosave_does_not_close_path_paste_panel_with_pending_watch`
  - 先制造一次编辑让 `autosave` 观察存在，再打开路径粘贴面板；tick 不提交、面板不消失、`next_deadline` 为 None。

### 7.2 Prompt 提及时机

文件：`src/app/prompt/tests/mention_timing.rs`（新建子模块，`tests.rs` 内声明）

- `cursor_move_into_existing_mention_does_not_open_panel`
  - 种子：`@` + 扫描结果 + 确认得到 `@src/app.rs `。
  - `move_left()` 多次进入路径中间；断言 `mention()` 始终为 None。
- `typing_inside_existing_mention_opens_panel`
  - 接上例，在路径中间 `insert_char`；断言面板出现且 query 为当前位置之前的片段（D5）。
- `edit_after_mention_confirm_reopens_on_first_keystroke`
  - 确认候选后 `panel_suppressed = true`；模拟事件层先 `set_mention_root` 再 `insert_char('@')`；断言面板当次出现。
- `sync_root_does_not_open_closed_mention_panel`
  - 关闭面板态下调用 `set_mention_root`；断言不新开。
- 事件层顺序测试（`src/app/update/prompt/tests/mention_timing.rs`）
  - 用 `apply_editor(EditorCommand::InsertChar('@'))` 复现 `sync_mention_root -> insert_char` 顺序；断言第一下就出面板。

### 7.3 Lx 提及时机

文件：`src/app/state/lx/mention/tests.rs`

- `cursor_move_into_existing_mention_does_not_open_panel`
  - 缓存 `@src/app.rs` 并确认插入后 `move_left()`；断言 `mention().panel()` 为 None 且 `mention_visible()` 为 false。
- `escape_then_move_stays_closed_until_edit`
  - Esc 后 `move_left()` 多次仍不弹；`insert_char('a')` 后弹（query 正确）。
- `background_skill_sync_does_not_open_closed_panel`
  - 光标落在已有 `@main.rs` 内部、会话关闭态下 `set_mention_skills` / `set_mention_mcp_servers`；断言不弹。
- `root_change_with_folded_fence_token_keeps_open_panel`
  - 折叠粘贴块正文含代码围栏，块后输入 `@src` 后切根；断言会话保持打开。

### 7.4 回归

- 现有 `mention_panel_escape_closes_until_next_edit`、`mention_panel_typing_resets_selected_item_to_first`、`mention_panel_trigger_origin_change_resets_selected_item_to_first`、`panel_confirm_suppressed_until_next_edit` 必须继续通过。
- 现有 lx `@`/粘贴/斜杠/块面板测试必须继续通过。

### 7.5 人工验收（用户执行 `cargo run`）

1. 已保存 prompt 中先改几个字，再粘贴普通文本：面板保持，直到选择/Esc/点外部；不再约 0.8 秒消失。
2. 确认一个 `@` 候选后立刻再输入 `@`：第一下就出 `scanning…` 或列表。
3. 正文已有 `@src/app.rs `：鼠标点进路径中间、方向键移进路径中间都不弹；在同一位置删/打一个字符会弹。
4. Esc 关掉 `@` 面板后左右移动不弹；直接打字当次弹。
5. lx 输入框重复 2–4，行为一致。

---

## 8. 验收标准

- B1：文本/路径粘贴面板在自动保存计时到点时不会被关闭；提案期间无落库效应、无空转唤醒。
- B2：任何“确认/ Esc / 粘贴压制”之后的第一次文本编辑，`@` 面板当次响应，无延迟一拍。
- B3：点击或方向键把光标放入已有 `@` 内容不弹面板；移动只关闭/收敛已打开面板；在同一位置打字仍按编辑触发。
- lx 与 Prompt 对上述四条的可见行为一致。
- 不引入空目录、空文件、抽象层；不触碰数据库与 Git 状态。

---

## 9. 风险与回滚

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| 移动不再新开面板 | 用户若习惯“移进 `@` 就选文件”会感到变化 | 与 opencode TUI 对齐，且移动后打字仍会弹；设计已确认 |
| 后台快照不再新开面板 | skills/MCP 加载时不再“顺手”弹面板 | 会话关闭时本就不该弹；扫描结果仍会更新已打开面板 |
| 自动保存延后 | 提案打开期间不落库，面板长时间不关会延迟保存 | 面板只能由用户收束；切换目标/退出仍会先回滚再提交，不丢正文 |
| lx `view` 改读已存 trigger | UI 与会话状态强绑定 | 所有编辑/移动收尾都会刷新 trigger；补移动/后台同步测试 |

回滚：本次只改状态机与自动保存门禁，不涉及 schema；按文件回退即可。

---

## 10. 修订历史

| 轮次 | 内容 |
| --- | --- |
| 1 | 用户确认 B1 按“任一粘贴提案暂停自动保存”修复；临时草稿不处理 |
| 2 | 用户要求 `@` 交互统一参考 opencode；确认“只由文本编辑触发、移动/点击只收敛、Esc 后文本变化才重新武装”，并接受“在已有 `@` 路径中间打字仍会弹” |
| 3 | 实施后按两路只读评审修正：根/绑定切换不再解除压制、扫描结果只走 `refresh_if_active`、lx `view` 与 `refresh` 统一 mask 文本；补对应回归测试 |

---

## 11. 实施结果

### 11.1 代码改动

| 文件 | 改动 |
| --- | --- |
| `src/app/prompt.rs` | 新增 `paste_proposal_open()`；`autosave_deadline` 在提案期间返回 None；undo/redo 复用判断；`settle`/`settle_moved`/`settle_panels(edited)` 分流；skills/MCP 快照走 `refresh_if_active` |
| `src/app/prompt/mention.rs` | 新增 `refresh_if_active`；`set_root` 只更新根/缓存、不解除压制、不开面板；`take_scan_request` 只认已存 `trigger`；worktree 三个同步入口收敛；`apply_entries` 只喂缓存；`refresh` / `set_root` / `take_scan_request` 内部统一 `mask_pasted_text` |
| `src/app/prompt/navigation.rs`、`viewport.rs` | 纯移动与鼠标落点改走 `settle_moved()` |
| `src/app/state/lx/panels.rs` | `refresh_panel()` / `refresh_panel_moved()` 分流；`refresh_panels(edited)` |
| `src/app/state/lx/mention.rs` | 新增 `refresh_if_active`、`abandon_scan`；`view` 读已存 trigger 并先 mask；`refresh` 内部统一 mask；`set_root` 不清压制；`take_scan_request` 去掉参数并加压制/会话门禁；后台快照同步收敛；`apply_entries` 只喂缓存 |
| `src/app/state/lx/edit.rs`、`selection.rs` | 方向键/行首行尾/按词移动/文档首尾改走 `refresh_panel_moved()` |
| `src/app/update/interaction.rs`、`prompt/save.rs` | 自动保存 tick 与手动保存改用 `paste_proposal_open()` |
| `src/app/update/pane.rs`、`pane/popups.rs` | 切回 lx 用 `refresh_panel_moved()`；BTW 丢弃扫描结果时清原 composer `pending` |

### 11.2 测试

新增/补充：

- `src/app/prompt/tests/mention_timing.rs`：移动/鼠标落点/根同步不新开；路径中间打字重开；确认后第一次按键当次重开；Esc 后移动不解除；根切换与迟到扫描结果不重开。
- `src/app/update/prompt/tests/mention_timing.rs`：事件层 `sync_mention_root -> insert_char` 顺序下第一次 `@` 当次出面板；确认后方向键不重开。
- `src/app/update/prompt/tests.rs`：文本粘贴提案暂停自动保存且不安排唤醒；路径提案在已有观察下不被 tick 关闭。
- `src/app/state/lx/mention/tests.rs`：移动不新开；Esc 后移动不解除；后台 skills/MCP 同步不新开；根切换与迟到扫描结果不重开；折叠 token 含围栏时切根不关面板。
- `src/app/update/tests.rs`：BTW 扫描结果被路由丢弃后清 `pending`；切回 lx 视图不重开 Esc 面板。

验证结果（在 `.worktrees/prompt-mention-paste-parity`）：

- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：lib 5436 passed / 0 failed（6 ignored），全部集成测试套件通过。

### 11.3 未验证

- 真实终端下的鼠标点击落点、终端粘贴时序与视觉呈现需用户 `cargo run` 手工确认（验收清单见 `task.md`）。
- 未改动 `@` 文本存储形态与桌面端原子 mention 模型。


---

## 12. 评审修正（review.md）

| 级别 | 问题 | 修正 |
| --- | --- | --- |
| P1 | BTW 扫描结果被路由丢弃后 `pending` 永不清，永久 Loading | 新增 `LxMention::abandon_scan` / `Lx::abandon_mention_scan`；`apply_lx_mention_entries` 丢弃结果时按 qa 找回 composer 清在途标记 |
| P2 | 根同步/后台快照用原始文本刷新，折叠 token 里的围栏会误关面板 | `refresh` / `set_root` / `take_scan_request` 统一在状态层 `mask_pasted_text` |
| P2 | `toggle_pane_view` 用编辑语义收尾，切回 lx 重开已 Esc 面板 | 改用 `refresh_panel_moved()` |
| P2 | 两个新增测试在旧实现上同样通过 | 重写路径自动保存测试与 lx 后台同步测试；补 Prompt 侧测试 |
| P2 | 文档/注释仍写“根变化解除压制” | 同步 `task.md` 与字段注释 |
