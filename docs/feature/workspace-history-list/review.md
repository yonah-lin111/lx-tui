# 工作区历史列表 — Review 与修复记录 (`review.md`)

- **审查范围**：`feature/workspace-history-list` 相对 `dev` 的全部改动（工作区历史列表、Git worktree 树、新建/重命名/删除、全局排序与拖拽）。
- **审查基线**：`69ae1df6`（review 前）。
- **审查方式**：3 路并行审查（DB/持久化、App 状态机、UI/事件/输入）+ 人工复核代码与测试；所有结论均以当前源码为准。
- **结论**：**0 个 P0；4 个 P1、11 个 P2 已全部修复；9 个 P3 中 7 个修复、2 个保留并说明取舍**。修复后 `cargo clippy --all-targets -- -D warnings` 通过，全量测试 4492 passed。

| 级别 | 数量 | 状态 |
| --- | --- | --- |
| P0（阻断/数据损坏） | 0 | — |
| P1（安全/数据完整性/主流程） | 4 | 已修复 |
| P2（重要边界/一致性） | 11 | 已修复 |
| P3（体验/防御/测试） | 9 | 7 修复，2 保留并说明 |

---

## P0（阻断）

无。

---

## P1（安全 / 数据完整性 / 主流程）

### P1-1 路径文本不一致导致删除静默失效、重命名/排序插入重复工作区行

- **位置**：`src/db/workspace.rs:28`、`:491`；`src/app/update/workspace.rs:87`、`:235`。
- **根因**：`workspace.path` 是文本主键；旧库/终端 cwd 可能保存 `/tmp/x`、`/tmp/x/`、`/private/tmp/x`、`a/../a` 等指向同一目录的不同文本。历史列表父项又使用 `git worktree list` 的 canonical `repo_root`，删除/重命名按 canonical 文本做 SQL 等值匹配时命中 0 行；`rename_workspace` 则会 UPSERT 出第二行，信任与 prompt/session 分裂到两行。
- **影响**：删除后条目重新出现；重命名后出现重复历史项；同一目录的 CLI/Agent 数据归属分裂。
- **修复**：
  - 新增 `workspace_row_path`：先精确匹配，未命中再按 `canonicalize` 等价匹配已有行；`is_trusted`、`trust_workspace`、`ensure_workspace`、`record_workspace_open`、`rename_workspace`、`workspace_external_id`、`save_workspace_order` 全部走该入口。
  - `delete_workspace` 清理同一目录的全部等价行（含旧库重复行），事务内删除。
  - `create_workspace_with_cwd` / `sync_workspace_cwd` 写入前 canonicalize。
- **验证**：`workspace_rows_resolve_canonical_equivalent_paths`（DB 层）；历史打开已有工作区用 `same_workspace_path` 兼容文本差异。

### P1-2 历史列表打开未信任工作区完全绕过信任检查

- **位置**：`src/app/trust.rs:27`、`src/app/update/workspace_history.rs:565`、`src/app/update/trust.rs:41`。
- **根因**：历史打开直接 `Workspace::single_terminal` + `RecordWorkspace`；`record_workspace_open` 的 `ON CONFLICT` 只刷新 `updated_at`，不会授予信任。live worktree 自动发现/拖拽补建的行 `is_trusted = 0`，打开后项目 `AGENTS.md`、项目 MCP、项目 skills 全部静默不加载。
- **影响**：与 Worktree 对话框的信任门不一致；未信任仓库可在无提示下运行 Agent。
- **修复**：
  - `WorkspaceHistoryEntry` 增加 `trusted` 快照（路径或仓库根已信任）；`list_workspace_history_entries` 从 `workspace.is_trusted` 计算。
  - 新增 `TrustTrigger::OpenWorkspaceHistory { path, name }`；未信任条目先发 `Effect::CheckTrust`，授予后 `open_workspace_history_trusted` 按路径重新定位并续作；拒绝弹 `Workspace open cancelled`。
  - 已打开的同路径工作区仍直接切换，不重复弹信任。
- **验证**：`untrusted_history_entry_requests_trust_instead_of_opening`；DB `trusted` 断言；事件测试先 `trust_workspace` 再双击打开。

### P1-3 打开已打开的历史工作区后焦点仍留在侧栏，键盘输入被吞

- **位置**：`src/app/update/workspace_history.rs:660`；`src/app/update/workspace.rs` 的 `switch_workspace`。
- **根因**：`switch_workspace` 只在 `prompt_focused()` 时交回主区；历史列表的 Enter/双击先把焦点放到 Sidebar，因此“已打开”分支退出历史后焦点仍在 Sidebar，Active 模式路由把所有按键吞掉。新建分支原本已调用 `focus_panes`，两条路径不一致。
- **影响**：打开已打开工作区后必须再点一次窗格才能输入。
- **修复**：`open_existing_history_workspace` 在 `ensure_main_workspace` + 退出历史后无条件 `focus_panes`；新建分支保持一致。
- **验证**：`opening_already_open_history_entry_returns_focus_to_panes`。

### P1-4 删除仓库父项会级联删除全部 worktree，但确认弹窗只显示父项名字

- **位置**：`src/app/update/workspace_history.rs:721`、`src/app/state/workspace_history.rs:227`、`src/ui/overlay/dialog.rs:284`、`src/ui/text.rs:484`。
- **根因**：父项删除会把同 `repo_root` 的所有 linked worktree 一并加入待删路径，但 `confirm_delete_workspace_question` 只接收父项名称；用户看不到数据破坏范围。
- **影响**：确认“删除 lx-tui”实际删掉整个仓库组的历史、会话、prompt，并关闭其他 worktree 窗口、杀掉运行中的 Agent。
- **修复**：`WorkspaceHistoryPanel::delete_worktree_count` 统计同组 linked worktree 数量；父项最终确认文案改为 `delete workspace "<name>", N linked worktree(s), and all their data?`；普通条目/子项文案不变。
- **验证**：`workspace_history_group_delete_question_lists_worktrees`。

---

## P2（重要边界 / 一致性）

### P2-1 侧栏折叠后仍能命中/滚动/键盘操作不可见列表

- **位置**：`src/ui/sidebar.rs:236`、`:250`、`:336`、`:536`；`src/event/input.rs:54`。
- **根因**：折叠态侧栏仍保留 `x+1..x+2` 的内部几何；`workspace_slot_at` / `agents_list_rect` / 滚动条命中没有 `sidebar_collapsed` 守卫；历史键盘分支只检查 `sidebar_focused`。
- **影响**：折叠后点击空白会静默切换隐藏工作区/Agents；滚轮滚动隐藏列表；历史搜索框不可见却仍接收按键。
- **修复**：`workspace_list_rect`、`agents_list_rect`、`workspace_section_at`、`workspace_scrollbar`、`workspace_list_rows` 与历史键盘分支统一加折叠守卫；历史命中本身已有 `history_section_rect` 守卫。
- **验证**：`workspace_list_blank_hidden_when_sidebar_collapsed` 覆盖 `x+1/x+2`。

### P2-2 历史滚动偏移只夹到 `len-1`，视口变高/条目减少后出现空白和滚动条错位

- **位置**：`src/ui/sidebar/history.rs:146`、`:324`、`:364`；`src/app/state/workspace_history.rs:427`；`src/event/mod.rs:462`。
- **根因**：渲染/命中使用 `min(scroll, len-1)`，正确上限应是 `len - viewport`；Agents 折叠/终端变高/删除/重载后没有主动夹取。
- **修复**：`WorkspaceHistoryPanel::clamp` 返回是否有变化并按 `total - visible` 夹取；渲染、`history_slot_at`、滚动条、`drop_index` 全部使用视口上限；事件循环每帧在历史模式下调用 `clamp` 并置 dirty。
- **验证**：`clamp_limits_history_scroll_to_viewport`。

### P2-3 进入历史/新建/清空搜索后选中项可能在视口外

- **位置**：`src/app/update/workspace_history.rs:68`、`:429`、`:836`、`:916`、`:938`；`src/event/mod.rs:462`。
- **根因**：只设置 `selected`，没有视口高度可用；`clamp` 不滚动选中项。
- **修复**：`WorkspaceHistoryPanel` 增加 `reveal_selected`；进入历史、重载恢复选中、新建条目、删除、清空/编辑搜索到空时置位；事件循环拿到可见行数后调用 `scroll_selected_into_view` 并清除标志。
- **验证**：选中揭示逻辑由事件循环测试与 clamp 测试覆盖。

### P2-4 `NewWorkspace` 内联确认在 Ctrl+X 改变输入后不复位

- **位置**：`src/app/update/dialog.rs:419`。
- **根因**：`edit_text_input` 与 `apply_overlay_paste` 会复位 `new_workspace_confirming`，但 `cut_overlay_selected_text` 漏了 `Overlay::NewWorkspace` 分支。
- **影响**：用户确认 `[Confirm]` 后剪切输入框文本，按钮仍是 Confirm，再回车会直接信任并创建已变化的路径。
- **修复**：剪切成功后检测 `Overlay::NewWorkspace` 并复位确认态。
- **验证**：`cut_in_new_workspace_resets_inline_confirmation`。

### P2-5 Git 仓库子目录被误判为 linked worktree 子项

- **位置**：`src/db/workspace.rs:268`、`:592`。
- **根因**：`git worktree list` 在仓库子目录中也能成功；原逻辑把任何 `path_to_repo` 命中的记录都当 worktree，且 `is_linked: true` 硬编码。
- **影响**：`<repo>/src` 被缩进为分支子项；双击会强制补开主 checkout；父项删除会把子目录工作区当作 worktree 一并删除。
- **修复**：只有记录路径 canonical 等于主 checkout 或某个 live worktree 根时才归组；已删除 worktree 仅当位于 `<root>/.worktrees/<one>` 或 `.git/worktrees/*/gitdir` 管理路径中才归组；其余保持顶层。
- **验证**：`repo_subdirectory_stays_top_level_in_history`；原有 Git 树测试仍通过。

### P2-6 删除仍存在于磁盘的 linked worktree 后，重启会被 Git 自动扫描复活

- **位置**：`src/db/migration/schema.rs:19`、`src/db/migration.rs:274`、`src/db/workspace.rs:537`、`:218`。
- **根因**：`hidden_worktrees` 只在内存；重启后 `git worktree list` 会重新发现仍存在的 worktree。
- **修复**：v24 新增 `workspace_hidden(path PRIMARY KEY, hidden_at)`；删除仍存在的 linked worktree（`.git` 为文件）时持久化隐藏；`list_workspace_history_entries` 过滤隐藏路径；`RecordWorkspace` 显式登记时解除隐藏。
- **验证**：`hidden_workspace_paths_persist_until_unhidden`、`deleting_last_open_workspace_keeps_hidden_trusted_row`、`record_workspace_unhides_previously_hidden_path`。

### P2-7 通过 Worktree 对话框重新打开被删除的 worktree 后仍被隐藏

- **位置**：`src/db/service.rs` 的 `Effect::RecordWorkspace`（约 `:569`）。
- **根因**：原逻辑只在历史列表打开/新建/`create_workspace_with_cwd` 清理 `hidden_worktrees`，`open_worktree_checkout` 没有。
- **修复**：把“解除隐藏”收敛到 `Effect::RecordWorkspace` 执行路径（内存 + DB `unhide_workspace`），所有显式登记入口统一生效。
- **验证**：`record_workspace_unhides_previously_hidden_path`。

### P2-8 不在历史条目中的活跃工作区每次 reload 被挤到末尾，拖拽位置无法持久化

- **位置**：`src/app/update/workspace_history.rs:254`；`src/app/update/workspace.rs:235`。
- **根因**：`sync_active_order_to_history` 对未匹配项使用 `usize::MAX` 排序；`sync_history_entries_from_active` 只回写已匹配路径，导致 Sync cwd 后工作区永远被排到末尾。
- **修复**：改为稳定槽位合并——已匹配项按历史顺序填入原槽位，未匹配项保持当前位置；`sync_workspace_cwd` 显式登记 `RecordWorkspace`，使显式同步也进入历史。
- **验证**：`history_sync_keeps_unmatched_active_workspace_in_its_slot`；`sync_ws_path_preserves_manual_name` 等原有测试通过。

### P2-9 拖拽轻微抖动即触发整表写回

- **位置**：`src/app/state.rs:291`、`src/app/state/workspace_history.rs:94`、`src/app/update/workspace_list.rs`、`src/app/update/workspace_history.rs:169`。
- **根因**：`dragging` 在“指针移动”时置位，松开即持久化；即使目标块与自身重叠、顺序没变也会 `SaveWorkspaceOrder` + reload，可能重排活跃列表。
- **修复**：新增 `workspace_drag_moved` / `WorkspaceHistoryPanel::drag_moved`，只在 `order` 真正变化时置位；松开仅后者触发持久化，`dragging` 继续负责视觉反显。
- **验证**：`history_drag_inside_same_block_does_not_persist_order`；活跃列表 `workspace_drag_highlight_waits_for_pointer_move` 仍通过。

### P2-10 工作区重命名/删除 DB 写失败静默

- **位置**：`src/db/service.rs:583`、`:630`、`:725`；`src/ui/text.rs`。
- **根因**：`with_db` 失败只 `tracing::warn!`；重命名下次 reload 会“变回去”，删除条目会重新出现，用户无感知。
- **修复**：重命名/删除写失败时弹 `Workspace update failed` Danger Toast（无数据库的只读降级不打扰）。
- **验证**：无故障注入测试；由代码路径覆盖，正常测试通过。

### P2-11 删除最后一个打开工作区的 DB 行后，后续保存会以未信任默认名复活

- **位置**：`src/app/update/workspace_history.rs:721`；`src/db/service.rs:590`。
- **根因**：最后一个工作区无法 `close_workspace`，但删除效应仍删除 `workspace` 行；之后任意 prompt/session 保存调用 `ensure_workspace` 会插入 `is_trusted = 0`、目录末段名的新行，历史条目也重新出现。
- **修复**：服务层在执行删除前记录该路径是否仍打开；删除后若仍打开，则重新 `trust_workspace` 并写入 `workspace_hidden`，保留一条“已信任 + 隐藏”行供终端/后续保存使用；内存侧清空该工作区的 prompt tree/referenced。
- **验证**：`deleting_last_open_workspace_keeps_hidden_trusted_row`；`deleting_only_open_workspace_keeps_it_open_and_queues_delete`。

---

## P3（体验 / 防御 / 测试）

### P3-1 删除时用分页接口当全量接口，只中断前 200 个会话

- **位置**：`src/db/service.rs:604`。
- **根因**：`list_chat_sessions` 内部 `limit.clamp(1, SESSION_PAGE_SIZE * 4)`，传 `usize::MAX` 实际只取 200 条。
- **修复**：按 `SESSION_PAGE_SIZE` 循环取全量，直到不足一页；逐页中断子 Agent 并解绑会话。
- **验证**：编译与既有删除测试通过。

### P3-2 主工作区命名自愈是启发式，可能误伤与子 worktree 同名的合法手动名称

- **位置**：`src/db/workspace.rs` 的 `is_polluted`。
- **根因**：无法区分旧版污染名与用户手动改名；原逻辑还扫描仓库下任意历史记录，扩大误伤面。
- **处理**：已收窄为“仅匹配 live linked worktree 目录名”，移除任意历史记录扫描；**仍保留启发式**，因为旧库没有 `name_is_manual` 审计信息。若要彻底消除误伤，需要一次性迁移标记或版本化命名来源，属于后续独立改动。
- **状态**：保留并在此说明，不阻塞本分支。

### P3-3 双击以可见行号判定，滚动/拖拽后可能打开错误条目

- **位置**：`src/app/update/workspace_history.rs:514`、`src/event/mouse/hit.rs`。
- **根因**：原 `row_click` 只记录可见行号；滚动、过滤、重排后同一行号可能对应另一条目；拖拽后 400ms 内再点也可能误判。
- **修复**：`row_click` 改为 `(PathBuf, Instant)`，按条目路径比对；滚动、设置偏移、重载、真正拖拽、折叠、搜索编辑都会清空点击序列。
- **保留差异**：仍按“第二次按下”判定，而非 prompt tree 的 MouseUp；身份校验与失效清理已消除误开风险，不额外改动事件生命周期。

### P3-4 丢失 MouseUp 后历史拖拽/搜索拖选状态残留，下一次手势被旧状态劫持

- **位置**：`src/event/mouse/press.rs:56`。
- **根因**：按下入口只清理 `prompt_drag`；指针移出窗口丢 Up 后，`search_drag`/`scroll_drag`/`drag` 会保留。
- **修复**：每次左键按下先清空历史列表的全部手势状态（含 `drag_moved`）。
- **验证**：既有鼠标事件测试通过。

### P3-5 折叠箭头命中列与绘制列不一致

- **位置**：`src/ui/sidebar.rs:413`、`src/ui/sidebar/history.rs:333`。
- **根因**：Active 只接受 `list.x`，History 接受前 4 列；实际箭头在 2 列选中标记之后（`list.x + 2`）。
- **影响**：点箭头不折叠、点标记反而折叠。
- **修复**：两侧统一只接受 `list.x + 2`；History 搜索态禁用折叠。
- **验证**：`workspace_group_toggle_hit_targets_parent_chevron_cell_only` 与 History 搜索态测试。

### P3-6 历史拖拽落点在列表末尾空白时返回 None

- **位置**：`src/ui/sidebar/history.rs:364`。
- **根因**：`(slot < total).then_some(slot)`，条目少于视口时末尾空白槽位越界。
- **修复**：落点钳到 `total - 1`，与 Active 列表一致。
- **验证**：`drop_index_clamps_blank_area_to_last_item`。

### P3-7 搜索过滤期间折叠箭头显示展开但点击无视觉反馈

- **位置**：`src/ui/sidebar/history.rs:333`、`src/app/state/workspace_history.rs` 的 `visible_rows`。
- **根因**：搜索命中子项时箭头强制显示 `▾`，但点击仍修改底层 `collapsed_repos`，可见行不变。
- **修复**：搜索过滤期间折叠箭头命中直接返回 None，点击落到行选择。
- **验证**：`group_toggle_is_disabled_while_searching`。

### P3-8 宽字符名称按字符数截断，去重序号/`*` 可能被裁掉

- **位置**：`src/ui/sidebar/history.rs:219`、`src/ui/text.rs:1272`。
- **根因**：`text::ellipsize` 以字符数而非显示列计算；CJK/emoji 名称会突破列预算。
- **修复**：新增 `text::ellipsize_width` 按显示列截断，历史列表使用；编号/标记宽度仍预留。
- **验证**：`ellipsize_width_counts_display_columns`。

### P3-9 测试盲区

- **新增覆盖**：canonical 路径、v23/v24 迁移回填、隐藏路径持久化/解除、仓库子目录、未信任打开、焦点回主区、父项删除文案、最后打开工作区隐藏行、稳定槽位、拖拽抖动不写库、折叠侧栏、滚动夹取、空白拖拽吸附、搜索态折叠、Ctrl+X 复位、宽字符截断。
- **仍可加强**：DB 写失败故障注入、真实鼠标 Up/拖拽序列的事件级双击测试、200+ 会话删除循环的规模测试。

---

## 未修复 / 明确取舍

1. **P3-2 命名自愈启发式**：保留，已收窄误伤面；彻底修复需要旧库命名来源审计/一次性迁移，另行评估。
2. **P3-3 双击在第二次按下判定**：保留；身份校验 + 滚动/拖拽/重载失效已覆盖实际误开场景。若后续要求与 prompt tree 完全一致，可改为 MouseUp 判定。
3. **旧库重复路径行不批量迁移**：只在 trust/rename/delete/save-order 时按 canonical 等价合并处理；避免启动时对不存在路径做不可逆 canonical 化。
4. **`workspace_hidden` 语义**：删除的 linked worktree 会一直隐藏，直到通过历史列表/Worktree 对话框/`[+]` 显式重新登记；这是“删除即隐藏”的产品语义。

---

## 验证

```text
rtk cargo fmt
rtk cargo clippy --all-targets -- -D warnings   # 通过
rtk cargo test --lib -- --skip exec_turn_declares_main_tools_without_ask
# 4492 passed, 3 ignored, 1 filtered out
```

- 跳过的 `chat::transport::tools::tests::exec_turn_declares_main_tools_without_ask` 是分支落后 `dev` 的既有失败（`dev` 的 `580eb0de` 已修复工具数量断言），与本次功能无关。
- 新增/更新的关键测试文件：
  - `src/db/workspace.rs` tests（canonical、隐藏、子目录、Git 树、position）
  - `src/db/service/tests.rs`（最后打开工作区、解除隐藏）
  - `src/app/update/workspace_history/tests.rs`（信任、焦点、拖拽、稳定槽位、删除）
  - `src/app/update/tests.rs`（Ctrl+X 确认态）
  - `src/ui/sidebar/history.rs` tests（折叠列、空白拖拽、搜索态）
  - `src/ui/overlay/tests.rs`（父项删除文案）
  - `src/ui/text/tests.rs`（宽字符截断）
