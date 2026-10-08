# Prompt 按工作区隔离与钉住 — 设计

- 分支：`feature/prompt-per-workspace`（基于 `feature/workspace-context-menu` 堆叠，未合并）
- 状态：**已完成**

## 1. 背景与目标

1. **prompt 是全局唯一编辑器**：`AppState.prompt` 只有一份（文本、撤销历史、选区、@ 面板、`prompt_root` 绑定）。点击左栏工作区 item、菜单激活工作区、新建工作区或打开 worktree 激活时，草稿与上下文原地不动，无法为每个工作区各写一份草稿。
2. **缺少钉住能力**：需要一个钉住按钮，钉住后切换工作区 item 不改变右侧 prompt 面板的显示，便于在其它工作区操作时保留并继续编辑当前草稿。

## 2. 已确认决策（Grill Me 结论）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 切换工作区时 prompt 的切换粒度 | **整体按工作区隔离**：每个工作区各持一份编辑器状态（文本/撤销/选区/@ 面板/显式上下文根），切换即换草稿 |
| 2 | 钉住释放条件 | **仅三处**：①再点按钮；②显式 `Open prompt`（工作区/窗格菜单）；③被钉工作区被关闭。点击其它 ws item、菜单激活、新建/打开 worktree 激活均不释放；取消钉住后回到当前激活工作区草稿 |
| 3 | 钉住按钮位置与形态 | prompt 容器**底边框右端、折叠按钮左侧**；`[pin]`（未钉）/`[unpin]`（已钉），鼠标点击切换，样式同折叠按钮（accent）；折叠态不显示 |
| 4 | 数据归属 | 草稿挂在工作区上：`Workspace.prompt: Option<PromptDraft>`；**显示中的那份**在 `AppState.prompt` / `prompt_root`；钉住保存显示中的工作区索引，随拖拽/增删重映射 |
| 5 | 在途 @ 扫描结果路由 | 扫描事件携带编辑器标识（`PaneId`），结果按 id 写回所属草稿（可处于挂起态），杜绝跨草稿串根 |
| 6 | 钉住时面板语境 | 顶栏路径名、底边 `b:` 分支状态、`@` 扫描根回退全部取**显示中的工作区**（钉住目标），而非激活工作区 |
| 7 | 重启持久化 | 不做（与工作区一致，仅内存） |

## 3. 架构与数据流

### 3.1 状态层（`src/app/state.rs`、`src/app/update.rs`）

- `PromptDraft { editor: Prompt, root: Option<PathBuf> }`；`Workspace.prompt: Option<PromptDraft>`。
  - 不变量：任意时刻**恰有一份**草稿处于显示态——位于 `AppState.prompt` / `AppState.prompt_root`，其所属工作区槽位为 `None`；其余工作区各持 `Some`。
  - 草稿随 `Workspace` 结构体移动，拖拽重排/增删工作区时天然随行。
- `AppState.prompt_pinned: Option<usize>`：钉住显示中的工作区索引。
- `AppState::prompt_workspace() -> usize`：钉住目标，否则激活工作区。
- `swap_display_prompt(state, from, to)`（`update.rs` 私有，`worktree` 子模块可调）：
  取出 `to` 槽位草稿 → `mem::replace` 显示态 `prompt` / `prompt_root` → 旧显示草稿停回 `from` 槽位 → `sync_mention_root`。
  槽位缺失时兜底新建草稿（不 panic）。
- 切换入口全部接入：
  - `switch_workspace`：未钉住时 swap，再改 `active_workspace`；
  - `create_workspace`：未钉住时 swap 到新工作区；
  - `close_workspace`：显示中的草稿随工作区销毁，装载新激活工作区草稿；钉住索引移除或前移；被钉工作区被关即释放钉住；
  - `drag_workspace_to`：钉住索引与 `active_workspace` 一样经 `new_index` 重映射；
  - `update/worktree.rs`：插入主 checkout 时索引整体 +1（钉住索引同步 +1）；激活子项时未钉住则 swap。
- `toggle_prompt_pin(state)`：未钉 → 钉住当前显示工作区；已钉 → `release_prompt_pin`（swap 回激活工作区 + 清选区）。
- `open_prompt_for_workspace` / `open_prompt_for_pane`：先释放钉住，再切换/绑定目标根路径（显式指令覆盖钉住）。
- `sync_mention_root`：显式绑定 `prompt_root` 优先，回退**显示中工作区** cwd。

### 3.2 事件层（`src/event/mod.rs`）

- `AppEvent::MentionScanned` 增加 `prompt: PaneId`；`pump_mention_scan` 随请求一同取 `state.prompt.id()`。
- `update::apply_mention_entries(state, prompt, generation, entries)`：按 id 在显示态与挂起草稿中定位；找不到（工作区已关）丢弃。
- 左键命中钉住按钮 → `update::toggle_prompt_pin`。
- 切换草稿后 `state.prompt.id()` 变化，`current_geometry` 的 rects 随之变化，自动触发 `resize_panes` 调整新草稿尺寸。

### 3.3 渲染层（`src/ui/`）

- `text.rs`：`PROMPT_PIN_LABEL = "[pin]"`、`PROMPT_UNPIN_LABEL = "[unpin]"`。
- `ui/mod.rs`：
  - `prompt_pin_button(panel, pinned) -> Option<Rect>`（右缘贴折叠按钮左侧，间隔 1 列；空间不足或折叠态返回 None）与 `prompt_pin_at(...)`；
  - `render_collapse_buttons` 一并绘制钉住按钮（晚于容器内容）；
  - `prompt_path_name` / `workspace_path_name` / `render_prompt_branch_status` 改用显示中的工作区；底边分支状态右端避让钉住按钮。

## 4. 风险与边界

- `active_workspace` 的直接赋值与 `close`/`drag` 重排是最易漏接的路径：8 处全部接入并写回归测试（含钉住下的拖拽、关闭、新建、worktree 激活）。
- 在途扫描按 id 路由后，挂起草稿期间的结果仍会写入该草稿缓存，切回即用；已关闭则丢弃。
- 关闭工作区即丢弃其草稿（含被钉住者），不额外弹确认——与决策 2 一致。
- 折叠态不显示钉住按钮；已钉状态下折叠/展开保持钉住；钉住不改变键盘焦点归属。
- 钉住时点击其它 ws item 仍按“点击面板外”语义：prompt 失焦、选区清除，仅草稿显示不变。

## 5. 验收标准

- 未钉住：点击/激活任一工作区（含新建、打开 worktree）→ 面板切到该工作区草稿；切回后原草稿（文本/撤销/选区/@ 面板/根绑定）完整恢复。
- 钉住：切换工作区 item 面板不变；仅三条件释放；释放后立即显示激活工作区草稿。
- 每工作区上下文根独立：窗格 `Open prompt` 的绑定只影响该工作区草稿，来回切换保持。
- 在途扫描结果不串草稿；钉住按钮几何/命中/样式正确；`cargo fmt`、`clippy -D warnings`、`cargo test` 全绿。
