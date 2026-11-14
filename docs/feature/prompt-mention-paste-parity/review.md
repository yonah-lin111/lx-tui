# Prompt / Lx `@` 提及与粘贴面板交互评审

审查对象：`01cf8f4e`（`fix(prompt): align mention panel and paste proposal timing`）及当前工作区后续修正。
审查方式：逐条对照设计文档、事件层调用顺序与实际代码，先跑定向测试，再跑全量校验。
状态：**已按本文件修复并验证**。

## 结论

| 级别 | 数量 | 说明 |
| --- | --- | --- |
| P0 | 0 | 未发现崩溃、数据丢失或不可恢复状态 |
| P1 | 1 | BTW 扫描结果被路由丢弃后永久停在 Loading |
| P2 | 4 | 原始文本/遮蔽文本不一致、视图切换重开面板、两个空测试、文档与实现相反 |

---

## P1

### P1-1 被路由丢弃的 BTW 扫描结果不清 `pending`，该 composer 永久 Loading

**现象**
BTW（`/btw`）输入框发起 `@` 扫描后，在结果回投前切换到另一个 QA；旧结果按 `qa_id` 被丢弃。之后回到原 QA，Files Tab 一直显示 `scanning…`，不再发起新扫描。

**错误原因**
1. `src/app/update/pane/popups.rs::apply_lx_mention_entries` 在 `active != qa_id` 时直接 `return`，既不写缓存，也不清原 composer 的 `pending`。
2. `src/app/state/lx/mention.rs::take_scan_request` 用 `pending` 阻止重复发起。
3. 原 composer 的根没有变化，`set_root` 早退；`pending` 没有任何路径清零，形成死锁。

**证据**
```text
popups.rs:  if active != qa_id { return; }
mention.rs: if self.pending { return None; }
```

**修复**
- `LxMention::abandon_scan(generation, root)`：仅当代号与根都匹配时清 `pending`。
- `Lx::abandon_mention_scan` 暴露给事件层。
- `apply_lx_mention_entries` 丢弃结果时按 `qa_id` 找回原线程 composer 并调用 `abandon_scan`。

**验证**
`dropped_btw_scan_result_clears_composer_pending`：QA-1 发起扫描 → 切到 QA-2 → 回投旧结果 → QA-1 能重新发起扫描。

---

## P2

### P2-1 根同步/后台快照用原始文本刷新，折叠粘贴块里的代码围栏会误关已打开面板

**现象**
输入框里有折叠粘贴块（正文含以 ` ``` ` 开头的行），在块后输入 `@src` 打开面板；随后按方向键或切换根，面板消失，且移回触发词也不再恢复。

**错误原因**
- 编辑路径用 `mask_pasted_text` 后的文本刷新，移动/根同步/后台快照路径传入原始文本。
- `mention_trigger` 会用 `open_fence` 判断是否在围栏内；折叠 token 的原始正文可包含围栏。
- 根同步先按原始文本 `refresh`，把 `trigger` 清空；随后移动走 `refresh_if_active`，看到 `trigger == None` 不再重算，面板永久关闭。

**修复**
- `MentionState::refresh` / `set_root` / `take_scan_request` 统一在状态层遮蔽折叠粘贴块。
- `LxMention::refresh` 同样统一遮蔽，与 `view` 使用同一套文本。

**验证**
- `root_sync_with_folded_fence_token_keeps_open_panel`
- `root_change_with_folded_fence_token_keeps_open_panel`

### P2-2 `toggle_pane_view` 用编辑语义收尾，切回 lx 会重开已 Esc 的面板

**现象**
输入框停在 `@` 触发处，按 Esc 关闭面板，切到 Terminal 再切回 Lx，面板重新出现。

**错误原因**
`src/app/update/pane.rs::toggle_pane_view` 恢复 lx 视图时调用 `pane.lx.refresh_panel()`，它等价于 `refresh_panels(true)`：解除压制并全量重算。视图切换不是文本编辑。

**修复**
切回 lx 视图改用 `refresh_panel_moved()`，只收敛已有会话，不解除压制、不新开面板。

**验证**
`toggling_back_to_lx_does_not_reopen_escaped_mention_panel`。

### P2-3 两个新增测试在旧实现上同样通过，属于空测试

**现象**
- `autosave_does_not_close_path_paste_panel_with_pending_watch`：`paste_stage` 已推进 revision，旧 `autosave_deadline` 本来就会因 revision 不匹配返回 None。
- `background_skill_sync_does_not_open_closed_panel`：夹具文本末尾是空格，`mention_trigger` 本来就返回 None，旧的无条件 `refresh` 也不会开面板。

**修复**
- 路径提案测试先 `autosave_due` 记录当前 revision 的静置起点，再断言 `next_deadline == None`；旧实现会在这一步暴露已到期期限。
- lx 后台同步测试把光标放进 `@main.rs` 内部，旧实现会确实重开面板。
- 补 Prompt 侧同类测试 `background_skill_sync_does_not_open_closed_panel`。

### P2-4 文档与代码注释仍描述“根变化解除压制”

**现象**
`task.md` 写着“根变化后仍清缓存并 `suppressed = false`”；`MentionState::suppress` 与 `LxMention::suppressed` 注释仍写“直到绑定切换/根变化”。实现已改为只有文本编辑解除压制。

**修复**
同步 `task.md`、两处字段/方法注释；以设计 D4 与不变量为准。

---

## 已核查未发现问题

- 编辑/移动分流：`navigation.rs`、`viewport.rs`、`edit.rs`、`selection.rs` 的调用点分类正确；删除、缩进、列表续写、模板改写、撤销恢复仍走编辑语义。
- 压制生命周期：`set_mention_root` 的 `panel_suppressed -> mention.suppressed` 镜像未发现可达的“编辑后重新压制”路径。
- 扫描结果：`apply_entries` 的代号不匹配不会重开已关闭会话；除 P1-1 外未发现 `pending` 卡死路径。
- 自动保存：未发现脏缓冲丢失、面板关闭后不再保存或 `next_deadline` 空转。
- lx `view` 读已存 `trigger`：未发现可见但 trigger/root 明确过期的可达状态。

## 验证

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- 定向：`mention_timing`、`autosave`、`state::lx::mention`、`dropped_btw_scan_result`、`toggling_back_to_lx`。

未覆盖：真实终端鼠标落点、终端粘贴时序与视觉呈现，需用户 `cargo run` 手工确认。
