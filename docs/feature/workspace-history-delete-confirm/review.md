# 工作区历史删除二次确认、批量清理失效项与跨平台路径粘贴 — 代码评审报告 (`review.md`)

- **评审范围**：`.worktrees/workspace-history-delete-confirm`（分支 `feature/workspace-history-delete-confirm`，对比 `dev`）。
- **评审方式**：结合 `codegraph`、`codebase-memory-mcp` 与 3 个并行 `general` 子 Agent（Track 1：状态机/右键菜单/级联与批量删除；Track 2：UI 布局/渲染/鼠标滚轮/滚动条/键盘事件；Track 3：跨平台路径粘贴归一化与工作区/引用目录链路）进行全链路代码审查。
- **优先级定义**：
  - **P0**：核心功能失效 / 数据持久化缺陷（已删除条目在重启后复活或破坏核心功能承诺）
  - **P1**：破坏性操作安全边界 / 视口可达性 / 跨平台核心能力失效
  - **P2**：边界条件与幂等性缺陷 / 交互状态残留 / 宽字符渲染截断
  - **P3**：代码与测试文件组织规范 / 回归测试覆盖补齐

---

## 问题分级总览（按 P0 → P3 排序）

| 级别 | 编号 | 模块 / 文件 | 关键点摘要 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | **P0-1** | `src/db/service.rs`<br>`src/app/update/workspace_history/delete.rs` | 删除磁盘上已不存在的 Git linked worktree（单条 `Delete` 或批量 `Delete missing`）后，因目录与 `.git` 文件已消失，`service.rs` 跳过 `db.hide_workspace(&path)`，导致重启后被主仓库 `git worktree list`（`prunable`）重新扫描复活。 | 已修复 |
| **P1** | **P1-1** | `src/app/state/workspace_history.rs`<br>`src/app/update/workspace_history/delete.rs` | `ConfirmClose` 确认弹窗缺乏“所见即所删”一致性保障：`delete_missing_workspace_history_entries` 提交时二次调用 `refresh_existence()` 存在 TOCTOU 越界删除风险；三处级联 worktree 判定条件写法不一且 `delete_workspace_history_entry` 漏判 `is_linked`；`MissingWorkspaces` 列表未复用 `display_label` 展示分支名。 | 已修复 |
| **P1** | **P1-2** | `src/app/update/dialog.rs`<br>`src/ui/overlay/dialog.rs`<br>`src/event/input.rs`<br>`src/event/overlay.rs`<br>`src/event/mouse/wheel.rs`<br>`src/event/mouse/drag.rs` | `ConfirmClose` 待删列表滚动上限按固定常量 `CONFIRM_DELETE_LIST_MAX_ROWS (5)` 计算，而非小屏或长问题折行下压缩后的真实可见行数 `layout.visible_rows`，导致小屏下列表尾部条目永远无法滚到。 | 已修复 |
| **P1** | **P1-3** | `src/platform/paste_path.rs`<br>`src/platform/mod.rs`<br>`src/app/state/workspace.rs`<br>`src/app/update/workspace.rs`<br>`src/event/commands.rs`<br>`src/ui/text/workspace.rs` | 跨平台 `~` 家目录展开在原生 Windows 上失效（`home_dir()` 仅读取 `HOME` 环境变量，未回退 `USERPROFILE` / `HOMEDRIVE`+`HOMEPATH`），且 `~` 展开逻辑重复散落在 `app` 与 `event` 层、未收敛至 `src/platform/`。 | 已修复 |
| **P2** | **P2-1** | `src/platform/paste_path.rs`<br>`src/app/update/dialog.rs`<br>`src/app/update/workspace.rs`<br>`src/event/commands.rs` | `normalize_pasted_path_input` 对转义尾随空格（`\ `）与引号包裹尾随空格（`'/path/foo '`）不幂等：粘贴时剥离转义/引号后，提交阶段二次归一化与 `.trim()` 将真实尾随空格当作拖拽噪声剥离，导致目录匹配失败或误命中同名无空格目录。 | 已修复 |
| **P2** | **P2-2** | `src/event/mouse/press.rs`<br>`src/event/mouse/drag.rs`<br>`src/app/update/selection.rs` | 鼠标左键按下与释放清理遗漏 `ConfirmClose.scroll_drag` 与 `overlay_input_drag` 互斥复位：窗口外丢失 `MouseUp` 后再次按下或释放时，残留标志会导致输入框拖选或滚动条拖拽失灵。 | 已修复 |
| **P2** | **P2-3** | `src/ui/overlay/dialog.rs` | `render_confirm` 中问题折行与待删列表按终端列宽（`UnicodeWidthStr::width`）计算预算，却调用按字符数截断的 `text::ellipsize` 而非 `text::ellipsize_width`，导致含中文/宽字符的工作区名或路径超出宽度时丢失省略号 `…` 被硬裁剪。 | 已修复 |
| **P2** | **P2-4** | `src/app/update/dialog.rs`<br>`src/platform/paste_path.rs` | 路径弹窗粘贴与 `file://` URI 边界缺陷：输入框仅含空白时粘贴未先清空前置空白、粘贴空引号 `''` 回退插入字面量；`commit_new_workspace` 首次确认产生两步冗余 Undo；`parse_file_uri` 未拦截 `%00` 空字节且 `localhost` 大小写敏感。 | 已修复 |
| **P3** | **P3-1** | `src/ui/overlay/tests.rs`<br>`src/ui/overlay/dialog/tests.rs`<br>`src/app/update/tests.rs`<br>`src/app/update/dialog/tests.rs` | `src/ui/overlay/tests.rs` 达 1974 行（超出 1500 行单测试文件上限），需按职责拆分子模块测试 `src/ui/overlay/dialog/tests.rs`，并将新增的路径弹窗测试迁入 `src/app/update/dialog/tests.rs`，补齐 P0–P2 全场景回归测试。 | 已修复 |

---

## 详细问题分析与错误原因

### P0-1：删除磁盘已丢失的 Git linked worktree 后未持久化隐藏，重启应用后条目复活

- **涉及位置**：
  - `src/db/service.rs:628-632`（`Effect::DeleteWorkspaceHistory` 处理分支）
  - `src/app/update/workspace_history/delete.rs:27-37, 61-67`
  - `src/db/workspace.rs:286-310, 387-427`（`list_workspace_history_entries`）
- **关键点**：
  1. 当某个 Git linked worktree 目录在磁盘上被外部删除（如 `rm -rf`）但尚未在主仓库执行 `git worktree prune` 时，主仓库的 `git worktree list --porcelain` 仍会输出该 worktree（标记为 `prunable`），因此 `list_workspace_history_entries` 会将其作为 `exists = false, git.is_linked = true` 的失效子项加载到 `ws 列表`；
  2. 用户通过 `Delete missing`（或单条 `Delete`）确认删除该失效 worktree 时，`delete.rs` 虽然将其路径加入了内存 `state.workspace_history.hidden_worktrees`，但数据库层 `src/db/service.rs:630` 仅在 `path.join(".git").is_file()` 为真时才调用 `db.hide_workspace(&path)`；
  3. 由于该 worktree 目录在磁盘上已不存在，`path.join(".git").is_file()` 恒为 `false`，导致 SQLite `workspace_hidden` 表未记录该路径。
- **错误原因**：
  将“属于需要防自动扫描复活的 linked worktree”错误等同于“磁盘上当前存在 `.git` 文件”，忽略了已被外部删除目录的 prunable linked worktree 磁盘上已无 `.git` 文件。结果当前会话因内存 `hidden_worktrees` 看似已删，一旦重启应用（内存 `hidden_worktrees` 清空），主仓库 `git worktree list` 扫描会立即把刚清理掉的失效 worktree 重新塞回 `ws 列表`。
- **修复措施**：
  在 `src/db/service.rs` 处理 `Effect::DeleteWorkspaceHistory { path }` 时，将持久化隐藏条件扩展为 `if path.join(".git").is_file() || state.workspace_history.hidden_worktrees.contains(&path)`，确保无论是存活还是已丢失目录的 linked worktree，删除后均写入 `workspace_hidden` 表持久化隐藏（后续若显式 `RecordWorkspace` 重新打开仍会自动解除隐藏）。

---

### P1-1：`ConfirmClose` 确认弹窗缺乏“所见即所删”一致性保障与级联谓词统一

- **涉及位置**：
  - `src/app/state/workspace_history.rs:227-293`（`delete_worktree_count`、`target_delete_entries`）
  - `src/app/update/workspace_history/delete.rs:11-71`（`delete_workspace_history_entry`、`delete_missing_workspace_history_entries`）
- **关键点**：
  1. **批量清理 TOCTOU**：用户在菜单点击 `Confirm delete missing` 打开 `ConfirmClose` 弹窗时已调用 `refresh_existence()`，弹窗据此渲染待删列表（所有 `!entry.exists` 项）。但当用户输入 `confirm delete` 提交时，`delete_missing_workspace_history_entries` 内部再次调用 `state.workspace_history.refresh_existence()` 重新扫描磁盘。若在弹窗打开期间有其它工作区目录被外部移动/卸载，这些**从未在确认列表中展示过**的工作区会被一并静默删除并杀死其运行中的 Agent/Shell；
  2. **三处级联 worktree 谓词不一致**：
     - `delete_worktree_count` 使用 `other_git.repo_root == git.repo_root && other_git.is_linked`；
     - `target_delete_entries` 使用 `git.repo_root == entry.path && git.is_linked`；
     - `delete_workspace_history_entry` 使用 `git.repo_root == entry.path`（漏判 `git.is_linked`）；
  3. **`MissingWorkspaces` 列表项名称未复用 `display_label`**：单条删除子 worktree 时使用 `entry.display_label(is_grouped_child)`（未重命名时展示分支名），而 `MissingWorkspaces` 直接取 `entry.name.clone()`，导致未重命名的失效子 worktree 在弹窗列表中全部显示为相同的父仓库名。
- **错误原因**：
  确认弹窗的预览列表、问题计数与提交删除各自独立遍历 `entries` 且在提交时二次刷新存在性，违反破坏性确认“所见即所删”原则。
- **修复措施**：
  1. 在 `WorkspaceHistoryPanel` 中抽取统一的级联子项与分组子项判定方法（统一以 `!git.is_linked` 的主仓库及 `other_git.repo_root == entry.path && other_git.is_linked` 为唯一判据），让 `delete_worktree_count`、`target_delete_entries` 与 `delete_workspace_history_entry` 共用同一套数据源；
  2. `target_delete_entries` 在 `MissingWorkspaces` 分支同样通过 `entry.display_label(self.is_grouped_child(entry))` 生成展示名称；
  3. `delete_missing_workspace_history_entries` 不再在提交时扩大 `!entry.exists` 集合（仅对已处于 `!entry.exists` 快照中且此刻磁盘上仍满足 `!entry.path.is_dir()` 的条目执行删除；若某目录在弹窗期间已恢复存在则安全跳过，绝不越界删除未出现在确认列表中的新条目）。

---

### P1-2：小屏终端下 `ConfirmClose` 列表滚动上限按固定 5 行计算，导致列表尾部不可达

- **涉及位置**：
  - `src/app/update/dialog.rs:416-455`（`scroll_confirm_close`、`set_confirm_close_scroll`）
  - `src/ui/overlay/dialog.rs:188-214`（`confirm_close_layout`）
  - `src/event/input.rs:102-118`、`src/event/overlay.rs:146-163`、`src/event/mouse/wheel.rs:43-50`、`src/event/mouse/drag.rs:138-150`
- **关键点**：
  1. `confirm_close_layout` 在终端高度较小（如 10~13 行）或问题文案折行较多时，会将列表实际可见行数 `visible_rows` 压缩为 `1..4` 行；
  2. 但 `scroll_confirm_close` 与 `set_confirm_close_scroll` 始终用固定常量 `visible = total.min(CONFIRM_DELETE_LIST_MAX_ROWS)`（即 5）计算 `max_scroll = total.saturating_sub(visible)`；
  3. 当待删条目数为 4、小屏实际 `visible_rows = 2` 时，状态层算出的 `max_scroll = 4 - 4 = 0`，导致滚轮、键盘 `Up`/`Down` 与滚动条拖拽全部被钳死在 `0`，后 2 条待删工作区永远无法滚入视野。
- **错误原因**：
  状态更新层硬编码了 UI 最大行数上限 `5`，忽略了终端窗口高度受限时 `confirm_close_layout` 动态收缩后的真实视口高度 `layout.visible_rows`。
- **修复措施**：
  为 `scroll_confirm_close_with_rows(state, delta, visible_rows)` 与 `set_confirm_close_scroll(state, offset, visible_rows)` 传入真实可见行数 `visible_rows`（保留 `scroll_confirm_close(state, delta)` 默认按 `CONFIRM_DELETE_LIST_MAX_ROWS` 供无屏幕几何的纯状态单测调用）；在 `event/input.rs`、`event/overlay.rs`、`event/mouse/wheel.rs`、`event/mouse/drag.rs` 中统一传入 `layout.visible_rows`，并在步进前先将 `confirm.scroll` 夹取到 `max_scroll`。

---

### P1-3：跨平台 `~` 家目录展开在原生 Windows 上失效，且平台逻辑散落在 `app` 与 `event` 层

- **涉及位置**：
  - `src/app/state/workspace.rs:231-235`（`home_dir`）
  - `src/app/update/workspace.rs:62-72`（`parse_workspace_path`）
  - `src/event/commands.rs:235-251`（`validate_referenced_path`）
  - `src/platform/paste_path.rs`
- **关键点**：
  1. `home_dir()` 仅读取 `std::env::var_os("HOME")`；在原生 Windows 环境（通常只设置 `USERPROFILE` 或 `HOMEDRIVE` + `HOMEPATH`）下恒返回 `None`，导致 `~/Desktop` 或 `~\Desktop` 在 Windows 上直接报错；
  2. `~` 展开代码在 `src/app/update/workspace.rs` 与 `src/event/commands.rs` 中重复手写，且 `rest.starts_with('\\')` 未按平台门控（在 Unix 上会把合法的相对目录名 `~\foo` 误当作 `$HOME/foo`）；
  3. `src/app/update/workspace.rs:67` 直接硬编码字符串 `"Cannot resolve home directory"`，未收敛到 `src/ui/text/workspace.rs`。
- **错误原因**：
  未遵循 `AGENTS.md`“OS 专属代码只允许出现在 `src/platform/` 并按 `#[cfg(...)]` 门控”的规范，在业务层直接用 Unix 环境变量假设处理跨平台家目录展开。
- **修复措施**：
  1. 在 `src/platform/paste_path.rs`（并由 `src/platform/mod.rs` 及 `src/app/state/workspace.rs` 导出复用）实现跨平台 `home_dir()`（依次读取 `HOME`、`USERPROFILE`、`HOMEDRIVE` + `HOMEPATH`）与 `expand_tilde_path(trimmed: &str) -> Option<Result<PathBuf, ()>>`（Unix 匹配 `~` 与 `~/...`，Windows 额外匹配 `~\...`）；
  2. `parse_workspace_path`、`validate_referenced_path` 与 `resolve_pasted_path` 统一复用该平台层函数；
  3. 将 `"Cannot resolve home directory"` 收敛为 `ui::text::TOAST_HOME_DIR_NOT_FOUND`。

---

### P2-1：`normalize_pasted_path_input` 对转义/引号尾随空格不幂等，二次归一化剥离合法目录尾空格

- **涉及位置**：
  - `src/platform/paste_path.rs:36-85`
  - `src/app/update/dialog.rs:258-262, 490`
  - `src/app/update/workspace.rs:44, 58-59`
  - `src/event/commands.rs:231-232`
- **关键点**：
  1. 当用户粘贴真实以空格结尾的目录（如 `/tmp/foo\ ` 或 `'/tmp/foo '`）时，粘贴入框首次调用 `normalize_pasted_path_input` 会还原出 `"/tmp/foo "`；
  2. 但按回车提交时，`commit_new_workspace` 与 `parse_workspace_path` / `validate_referenced_path` 会对输入框文本**再次**调用 `normalize_pasted_path_input` 并紧跟 `.trim()`；
  3. 第二次调用时，由于转义反斜杠或外层引号已在粘贴阶段剥离，末尾空格变成了“未转义空格”，被 `trim_trailing_unescaped_spaces` 和 `.trim()` 彻底抹掉，变成 `"/tmp/foo"`，导致找不到目录或静默打开错误的同名无空格目录。
- **错误原因**：
  `normalize_pasted_path_input` 在剥离尾随未转义空格前未检查原路径（仅去前导空白与尾随 `\r\n`）是否已在磁盘上真实存在，且下游调用方对归一化结果多余地调用了 `.trim()`。
- **修复措施**：
  1. 在 `normalize_pasted_path_input` 中，去掉前导空白与尾随 `\r`/`\n` 后，若该字符串不含引号/`file://` 且作为路径在磁盘上已真实存在（`Path::new(line).exists()`），直接原样返回，保证对真实存在且带尾随空格的路径多次调用完全幂等；
  2. 移除 `commit_new_workspace`、`parse_workspace_path` 与 `validate_referenced_path` 中对 `normalize_pasted_path_input` 返回值的多余 `.trim()` 调用。

---

### P2-2：鼠标左键按下与释放遗漏 `ConfirmClose.scroll_drag` 与 `overlay_input_drag` 互斥复位

- **涉及位置**：
  - `src/event/mouse/press.rs:56-66`
  - `src/event/mouse/drag.rs:531-562`
  - `src/app/update/selection.rs:334-399`
- **关键点**：
  1. `handle_mouse_down_left` 顶部清理了各面板残留的 `*_scroll_drag`（防止指针移出终端窗口丢失 `MouseUp` 后残留），但遗漏了 `Overlay::ConfirmClose` 的 `scroll_drag` 与 `state.overlay_input_drag`；
  2. `handle_mouse_up_left` 在清理 `confirm.scroll_drag` 后直接 `return`，未同步复位 `state.overlay_input_drag = false`；
  3. `stop_active_selection_drags` 同样遗漏了 `confirm.scroll_drag = None`。
- **错误原因**：
  新增 `ConfirmClose.scroll_drag` 时只接入了 `handle_mouse_drag` 和 `handle_mouse_up_left` 的单路径，未对齐全局手势生命周期的幂等清理。
- **修复措施**：
  在 `handle_mouse_down_left`、`handle_mouse_up_left` 与 `stop_active_selection_drags` 中统一复位 `confirm.scroll_drag = None` 与 `state.overlay_input_drag = false`。

---

### P2-3：`render_confirm` 使用按字符数截断的 `text::ellipsize` 导致宽字符行截断失真

- **涉及位置**：
  - `src/ui/overlay/dialog.rs:546, 583, 590`
- **关键点**：
  `render_confirm` 在计算待删列表行 `  • <name> (<path>)` 宽度时使用了 `UnicodeWidthStr::width`（按终端显示列宽），但在截断超长名称、超长路径及超长折行问题时却调用了按 `chars().count()` 截断的 `text::ellipsize`。当工作区名称或路径包含中文/CJK 宽字符时，字符数远小于显示列宽，`text::ellipsize` 不会截断，最终被 `Paragraph` 在右边界无省略号硬裁。
- **错误原因**：
  误用仅面向纯 ASCII 常量文案的 `text::ellipsize`，而未调用同模块已有的列宽感知版本 `text::ellipsize_width`。
- **修复措施**：
  将 `src/ui/overlay/dialog.rs` 中问题折行与待删列表行的 3 处 `text::ellipsize` 全部替换为 `text::ellipsize_width`。

---

### P2-4：路径弹窗粘贴替换、首次确认 Undo 栈与 `file://` URI 边界缺陷

- **涉及位置**：
  - `src/app/update/dialog.rs:258-276, 483-502`
  - `src/platform/paste_path.rs:219-269`
- **关键点**：
  1. `apply_overlay_paste`：当路径输入框内容为纯空格 `"   "` 时，`whole_replace` 为 `true`，但直接调用 `insert_single_line_paste` 只在光标处插入而未清除原有空格；当粘贴 `''` 或 `""` 时，归一化结果为空串，却因 `if normalized.is_empty() { text } else { &normalized }` 回退插入了字面量 `''`；
  2. `commit_new_workspace`：首次回车确认时先调用 `replace_new_workspace_input(state, &normalized)`，解析成功后再调用 `replace_new_workspace_input(state, &resolved)`，一次回车向输入框推入 2 条 Undo 记录；
  3. `parse_file_uri` / `percent_decode`：未检查百分号解码后是否包含 `'\0'` 空字节（如 `file:///tmp/a%00b`），且 `localhost` 主机名剥离大小写敏感。
- **错误原因**：
  1. `whole_replace` 未先执行 `input.select_all()` 覆盖旧空白；
  2. `commit_new_workspace` 在已知后续会回填 `resolved` 的情况下提前回填了中间态 `normalized`；
  3. `\0` 检查仅在 `percent_decode` 之前执行了一次。
- **修复措施**：
  1. 在 `apply_overlay_paste` 的 `whole_replace` 分支中：若 `normalized` 为空则直接忽略返回 `false`；若输入框非空（含纯空白）先调用 `input.select_all()` 再插入 `&normalized`；
  2. 在 `commit_new_workspace` 中合并回填逻辑：解析成功且需首次确认时仅调用一次 `replace_new_workspace_input(state, &resolved)`，解析失败时若 `normalized` 非空且与原文不同才回填 `&normalized`，确保单次 `Enter` 最多产生 1 步 Undo；
  3. 在 `percent_decode` 中拒绝包含 `'\0'` 的解码结果，并在 `parse_file_uri` 中支持大小写不敏感的 `localhost` 主机名剥离及 Windows `file://C:/...` 双斜杠盘符容错。

---

### P3-1：拆分超限测试文件并补齐 P0–P2 全链路回归测试

- **涉及位置**：
  - `src/ui/overlay/tests.rs`（1974 行 → 拆出 `src/ui/overlay/dialog/tests.rs`）
  - `src/app/update/tests.rs`（将新增路径测试迁入 `src/app/update/dialog/tests.rs`）
  - `src/app/update/workspace_history/delete/tests.rs`
  - `src/event/tests/workspace_history.rs`
  - `src/platform/paste_path/tests.rs`
- **关键点与修复措施**：
  1. 遵循 `AGENTS.md`“单测试文件建议控制在 1000 行以内，上限不超过 1500 行；严格跟随被测模块的职责与目录层级拆分”规范，将 `src/ui/overlay/tests.rs` 中属于 `dialog.rs` 的弹窗测试拆入 `src/ui/overlay/dialog/tests.rs`，使两个测试文件均控制在 ~1000 行左右；并将本分支新增在 `src/app/update/tests.rs` 的路径解析测试迁入 `src/app/update/dialog/tests.rs`；
  2. 移除 `src/ui/overlay/dialog.rs` 中仅被旧测试使用、不含列表几何的冗余公共函数 `confirm_close_shell`，统一使用 `confirm_close_layout`；
  3. 新增覆盖 P0-1、P1-1、P1-2、P1-3、P2-1、P2-2、P2-3、P2-4 的单元测试与集成测试。
