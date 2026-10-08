# 命令面板标题与 @ 面板目录导航 — 执行任务

- 分支：`feature/prompt-panel-nav`（工作区 `.worktrees/prompt-panel-nav`）
- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案收敛至 `ui/text.rs`；颜色从 `ui/style.rs` 取；文档同步 `docs/standards/tui-design-requirements.md`。

## 执行清单

### 1. 领域层（`app/markdown.rs`）

- [x] query 目录范围拆分（`mention_scope`）与范围过滤（直接子项 / 子树递归）
- [x] `MentionPanel::scope_name` 目录末段名
- [x] 单元测试：`app/markdown/tests.rs`

### 2. 状态层（`app/prompt*.rs`）

- [x] `MentionState::enter_folder`（目录 + 缓存后代校验；`@路径/` 写回）
- [x] `MentionState::refresh` 目录范围变化时高亮回首项
- [x] `Prompt::mention_enter_folder`（独立撤销步，回退走 `Undo`）
- [x] 单元测试：`app/prompt/tests.rs`

### 3. 输入与路由（`app/actions.rs`、`input/mod.rs`、`app/update.rs`）

- [x] `EnterFolder`（Shift+Enter）映射；回退复用既有 `Undo`（Ctrl/Cmd+Z）
- [x] 面板打开时接管 EnterFolder（文件/空目录 no-op），未打开时回落 `NewlineBelow`
- [x] 未消费回落 `NewlineBelow`，选区替换路径同步
- [x] 单元测试：`input/tests.rs`、`app/update/tests.rs`

### 4. 渲染层（`ui/`）

- [x] `command_panel` 视图 `title` / `right_title` / `footer` 与宽度计算
- [x] `ui/prompt.rs` 接线：`Commands` / `Files` / 目录名 / `[open ⇧↵] [back ^z]`
- [x] `ui/text.rs` 文案常量与符号兼容测试（单宽、非 PUA）
- [x] 单元测试：`ui/widgets/command_panel/tests.rs`、`ui/prompt/tests.rs`、`ui/text/tests.rs`

### 5. 集成测试

- [x] `tests/prompt_editor.rs`：进入目录 → 标题/目录名/底栏渲染 → Ctrl/Cmd+Z 撤销回退

### 6. 验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
