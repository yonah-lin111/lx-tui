# 工作区提示词管理、嵌套文件夹与工作区信任机制 — 执行任务

- 分支：`feature/workspace-prompt-management`
- 规则：生产代码禁止 `unwrap()/expect()`；错误使用 `Result` 与 `?`；用户可见文案统一至 `ui/text.rs`；样式只从 `ui/style.rs` 取；变更前后运行测试。

## 执行清单

### 1. 数据库与持久化基础设施（`src/db/`）
- [x] 引入 `rusqlite` 依赖与环境路径管理（生产 `~/.lx-t/db/lx.db`，开发 `~/.lx-t-dev/db/lx.db`）
- [x] 实现 SQLite 初始化与 Schema 迁移版本控制（`workspace`, `workspace_folder`, `workspace_prompt`）
- [x] 实现 Workspace 信任状态查询与插入 API（`get_workspace_trust`, `set_workspace_trusted`）
- [x] 实现提示词与文件夹 CRUD API（支持最大 2 层文件夹深度约束校验）
- [x] 单元测试：`src/db/tests.rs` 验证表迁移、层级限制、级联删除与事务一致性

### 2. 工作区信任机制（`src/app/` 与 `src/ui/overlay.rs`）
- [x] 定义 `OverlayKind::TrustWorkspace` 与 `Overlay::TrustWorkspace` 状态结构体
- [x] `lx` 启动时增加工作区信任门：未信任不进入应用，先在系统控制台显示信任页（`[y/N]`），拒绝则输出提示并退出；已信任直接进入
- [x] 新建工作区弹窗确认后增加信任检测：未信任触发信任浮层；拒绝则取消创建
- [x] 终端右键菜单同步按钮触发前增加信任检测：未信任触发信任浮层；拒绝则取消同步
- [x] `ui/overlay.rs` 渲染信任模态框（标题、路径高亮、`[Trust & Open]`、`[Don't Trust]` 按钮）
- [x] 单元测试：`src/app/overlay/tests.rs` 与 `src/app/update/tests.rs` 覆盖三种触发路径与拒绝回退行为

### 3. 提示词管理领域与状态模型（`src/app/prompt/`）
- [x] 定义 `PromptTree`、`PromptFolder`、`PromptItem` 纯内存模型与树形构建算法
- [x] 限制文件夹嵌套最大深度（`depth <= 1`，即最多 2 层）
- [x] 在 `AppState` 中维护当前工作区的提示词树状态、当前选中提示词 id、树滚动位置
- [x] 支持右栏在「提示词树状管理器视图」与「提示词编辑视图」之间切换（或分栏联动）
- [x] 单元测试：`src/app/prompt/tests.rs` 验证树形折叠、新增限制、节点定位

### 4. 浮层菜单与交互路由（`src/app/overlay.rs` 与 `src/app/update.rs`）
- [x] 扩展 `OverlayTarget` 支持提示词条目、文件夹与空白区域目标
- [x] 扩展 `MenuCommand`（`NewPrompt`, `NewFolder`, `RenameFolder`, `DeleteFolder`, `ChangeStatus`, `CopyContent`, `DeletePrompt`）
- [x] 实现右键点击树节点弹出对应菜单，左键点击执行对应命令
- [x] 实现重命名弹窗复用现有 `Rename` 浮层；删除弹窗复用 `ConfirmClose` 风格
- [x] 单元测试：`src/app/update/tests.rs` 覆盖右键菜单命令分派与数据同步

### 5. 渲染层支持（`src/ui/prompt.rs` 与 `src/ui/widgets/`）
- [x] 在 `ui/prompt.rs` 中实现提示词树渲染：文件夹图标/折叠展开三角、缩进线、条目状态标签（`todo`/`run`/`done`）
- [x] 在 `ui/text.rs` 增加信任弹窗文案、树节点状态文案与右键菜单文案
- [x] 适配宽度受限场景下的文本截断与省略号
- [x] 单元测试：`src/ui/prompt/tests.rs` 与 `src/ui/text/tests.rs` 确保无非法字符与对齐无误

### 6. 集成测试与规范校验
- [x] 端到端集成测试：新建未信任工作区 → 确认信任 → 新建文件夹 → 嵌套子文件夹 → 尝试建第 3 层（阻止） → 新建提示词 → 编辑内容并持久化 → 切换工作区隔离
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`

## 完成记录

- 实施分支：`feature/workspace-prompt-management`（工作区 `.worktrees/workspace-prompt-management`）。
- 架构落地：`src/db/`（rusqlite bundled + `PRAGMA user_version` 迁移 + 信任/文件夹/提示词 CRUD）与
  `src/db/service.rs` 同步受控服务；app 层新增 `Effect` 出箱、`PromptTree/PromptPanel` 模型与
  `update::{prompt,trust}` 子模块，DB 读写全部收口在事件循环内的服务执行。
- 实施差异（相对 design.md）：
  1. `OverlayTarget` 的提示词目标使用 DB 整数 id（`i64`）而非 external_id 字符串，保持枚举 `Copy`
     并避免全量调用点改动；external_id 仍按设计存库。
  2. 三层触发（启动/新建/同步）以“信任检查效应 + 决策续作”实现：启动检查在
     `Persistence::with_database` 内完成；新建工作区与终端同步先入 `effect::CheckTrust` 队列，
     由服务查库后选择续作或弹信任浮层。
  3. 信任浮层按键：`Enter` = 信任并续作、`Esc`/点击按钮之外 = 拒绝；启动拒绝恢复终端后输出
     `ui/text.rs::TRUST_REFUSED_MESSAGE` 并以 0 退出。
  4. DB 路径按环境变量 `LX_TUI_ENV`/`LX_ENV` 优先、缺省 `cfg!(debug_assertions)` 判定；
     额外支持 `LX_TUI_DB_PATH` 覆盖，供测试与集成测试隔离。
  5. 提示词树默认编辑视图（保持既有右栏行为），工具栏最左新增模式按钮 `[list]`/`[edit]`；
     因按钮组加宽，`[select all]` 的显示阈值提升到内容区 35 列。
  6. 隐藏第二层折叠/展开后仍保留折叠状态（重载按文件夹 id 继承）；删除文件夹在内存与服务层
     均级联，第三层新建在菜单（不展示）与 DB（`DepthExceeded`）双重阻止。
  7. 既有测试适配：新建工作区显式路径改为断言信任检查效应；同步路径测试先以已信任结果续作；
     三个真实 PTY 集成测试经 `tests/common` 隔离数据库并预信任工作目录；侧栏/prompt 标题断言
     改为与工作目录名无关的稳定形式（长目录名截断与窄面板标题重叠）。
- 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿
  （864 lib + 31 集成，含新增 `tests/prompt_management.rs` 端到端用例）。

## 补充修订（用户要求）

- 启动信任行为调整：`lx` 在未信任目录启动时**不进入应用**，先在系统控制台显示信任页
  （标题 / 路径 / `Do you trust the files in this folder? [y/N]`，参考 Claude Code）；
  `y`/`yes` 放行并入库，其余输入输出 `workspace not trusted; exiting` 后以 0 退出。
  应用内部触发（显式路径新建工作区、终端右键同步）保持 `Overlay::TrustWorkspace` 弹窗；
  启动信任门由 `db::service::Persistence::ensure_startup_trust` 在 `Tui::init` 之前执行。
- 新增真实 PTY 集成测试 `tests/trust_prompt.rs`：未信任目录启动先显示控制台信任页，
  `y` 进入应用、`n` 不进入应用并退出；`tests/prompt_management.rs` 改为覆盖应用内信任弹窗
  （新建未信任工作区 → 弹窗确认 → 创建并入库）。
- `Database::open` 增加 5 秒 `busy_timeout`，提升多实例并发下的健壮性。
