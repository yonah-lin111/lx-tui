# Prompt 按工作区隔离与钉住 — 执行任务

- 分支：`feature/prompt-per-workspace`（基于 `feature/workspace-context-menu` 堆叠）
- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案收敛至 `ui/text.rs`；样式统一从 `ui/style.rs` 取；测试驱动，按受影响范围严格验证。

## 执行清单

### 1. 文档与需求对齐（Grill Me 阶段）

- [x] 对齐并确认切换粒度（整体按工作区隔离：文本/撤销/选区/@ 面板/上下文根随工作区）
- [x] 对齐并确认钉住释放条件（仅按钮、显式 Open prompt、被钉工作区关闭三处）
- [x] 对齐并确认钉住按钮形态（prompt 底边框折叠按钮左侧，`[pin]`/`[unpin]`，鼠标点击）
- [x] 完成 `design.md` 决策表与数据流编写

### 2. 状态层（`src/app/`）

- [x] `state.rs`：新增 `PromptDraft` 与 `Workspace.prompt: Option<PromptDraft>`；`single_terminal` 初始持有草稿；`demo()` 初始工作区槽位为 None
- [x] `state.rs`：新增 `prompt_pinned: Option<usize>` 与 `prompt_workspace()`，更新字段文档
- [x] `update.rs`：实现 `swap_display_prompt`、`toggle_prompt_pin`、`release_prompt_pin`
- [x] `update.rs`：`switch_workspace`、`create_workspace`、`close_workspace`、`drag_workspace_to` 接入显示切换/索引重映射
- [x] `update/worktree.rs`：插入主 checkout 与激活子项两处接入
- [x] `update.rs`：`sync_mention_root` 回退显示中工作区；`open_prompt_for_workspace/pane` 先释放钉住
- [x] 状态单元测试：切换换草稿、钉住冻结、释放条件、关闭/拖拽/新建下的索引与不变量

### 3. 事件层（`src/event/`）

- [x] `MentionScanned` 携带 `prompt: PaneId`；`pump_mention_scan` 发送编辑器标识
- [x] `update::apply_mention_entries` 按 id 路由到显示态或挂起草稿，找不到丢弃
- [x] 左键命中钉住按钮 → `toggle_prompt_pin`
- [x] 事件单元测试：在途扫描跨草稿路由、挂起草稿回填、钉住点击

### 4. 渲染层（`src/ui/`）

- [x] `text.rs`：新增 `PROMPT_PIN_LABEL` / `PROMPT_UNPIN_LABEL`
- [x] `mod.rs`：`prompt_pin_button` / `prompt_pin_at` 几何与命中，绘制晚于容器内容
- [x] `mod.rs`：`prompt_path_name`、`workspace_path_name`、`render_prompt_branch_status` 取显示中工作区；分支状态右端避让钉住按钮
- [x] 渲染单元测试：按钮几何/标签/命中、钉住时顶栏与底边状态来源

### 5. 验证与提交

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿
- [x] 提交并询问合并
