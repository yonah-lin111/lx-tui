# 工作区提示词管理、嵌套文件夹与工作区信任机制 — 设计

- 分支：`feature/workspace-prompt-management`
- 状态：需求边界已确认（Grill Me 全部收口完成），待用户审核确认后执行

## 1. 背景与目标

当前 `lx-tui` 的 Prompt 面板仅维护单份纯内存草稿（或按工作区内存隔离草稿），未实现持久化与条目管理。
本需求参考 `lx-agent` 与 `claude code` 的设计，补齐：
1. **工作区提示词管理（右栏 Prompt）**：
   - 以工作区（Workspace）为单元管理提示词条目；
   - 支持文件夹收纳，最大支持嵌套 2 层文件夹（Folder → SubFolder → Prompt）；
   - 提供右键上下文菜单（新建提示词、新建文件夹、重命名、删除、状态切换、复制内容等）；
   - 右栏全宽双模式切换（Tree View ↔ Editor View），通过 `[list]` / `[edit]` 或回车/Esc 顺畅流转。
2. **工作区信任机制（对齐 Claude Code 设计）**：
   - 判定时机：①终端启动 `lx` 进入应用；②新建工作区；③主内容区终端右键菜单点击“同步工作区到终端路径”；
   - 未信任的工作区弹窗征询用户；
   - 若用户选择“信任”：将该路径录入数据库 `workspace` 表，标记 `is_trusted = 1`，继续进入/打开；
   - 若用户选择“不信任”：执行原子性回退（启动时退出应用 exit 0，新建/同步时取消操作恢复原状并 Toast 告警）。
3. **数据库持久化（对齐 `lx-agent` 架构）**：
   - 采用 SQLite 嵌入式数据库（Rust `rusqlite`）；
   - 存储路径：生产环境 `~/.lx-t/db/lx.db`，开发环境 `~/.lx-t-dev/db/lx.db`；
   - 核心数据表使用 Workspace 领域语义：`workspace`、`workspace_folder`、`workspace_prompt`。

---

## 2. 已确认决策矩阵（Grill Me 收口完成）

| # | 决策项 | 备选方案 | 确认结论 | 状态 |
|---|--------|----------|----------|------|
| 1 | **右栏 Prompt 交互架构** | A. 顶部树/下方编辑（上下分栏）<br>B. 树视图与编辑视图模式切换<br>C. 左右分栏 | **B. 树视图与编辑视图模式切换（List/Tree View ↔ Editor View）**：TUI 宽度宝贵（默认 ~50 列），全宽切换、回车进入编辑、Esc/点击顶栏返回树最舒适 | **已确认** |
| 2 | **工作区启动不信任回退** | A. 直接退出程序（Exit 0/1）<br>B. 回退至 HOME 或上一信任工作区<br>C. 只读空闲工作区 | **A. 不进入应用，先在系统控制台完成信任页**（用户补充要求，对齐 Claude Code：未信任目录启动时先显示信任询问与路径，确认 `y` 才进入 TUI；拒绝输出提示并以 0 退出，全程不进入应用） | **已确认** |
| 3 | **新建/同步不信任回退** | A. 关闭弹窗并放弃创建/同步<br>B. 保留原工作区并 Toast 提示 | **A. 关闭弹窗并放弃创建/同步，保留原工作区状态并给出 Toast 警告** | **已确认** |
| 4 | **环境识别策略** | A. 编译期 `cfg!(debug_assertions)`<br>B. 环境变量 `LX_ENV` / `LX_TUI_ENV`<br>C. 组合策略：环境变量优先，缺省回退 debug 门控 | **C. 组合策略**：开发运行 `cargo run` 自动走 dev，生产打包或显式配置可覆盖 | **已确认** |
| 5 | **数据库表名与迁移方案** | A. 沿用 lx-agent 的 project 表名<br>B. 改用 workspace 表名（workspace, workspace_folder, workspace_prompt） | **B. 改用 workspace 语义**（workspace, workspace_folder, workspace_prompt），原生 PRAGMA user_version 迁移 | **已确认** |
| 6 | **右键菜单项与 2 层文件夹约束** | A. 动态隐藏非法操作<br>B. 禁用置灰非法操作 | **A. 动态隐藏非法操作**：第二层文件夹右键不展示“新建子文件夹”，物理杜绝嵌套越界；级联删除给出影响范围确认 | **已确认** |

---

## 3. 架构与数据结构设计

### 3.1 数据库持久层（`src/db/` 模块）

#### 3.1.1 存储路径与运行时环境
- 路径探测：
  ```rust
  pub fn database_dir(is_dev: bool) -> PathBuf {
      let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
      if is_dev {
          home.join(".lx-t-dev").join("db")
      } else {
          home.join(".lx-t").join("db")
      }
  }
  pub fn database_path(is_dev: bool) -> PathBuf {
      database_dir(is_dev).join("lx.db")
  }
  ```

#### 3.1.2 核心 Schema（按 workspace 命名重构）
```sql
CREATE TABLE IF NOT EXISTS workspace (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'filesystem',
    path TEXT UNIQUE,
    is_trusted INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_folder (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    parent_folder_id TEXT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (parent_folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_folder_workspace_id ON workspace_folder(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_folder_parent ON workspace_folder(parent_folder_id);

CREATE TABLE IF NOT EXISTS workspace_prompt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    folder_id TEXT,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'completed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_workspace_id ON workspace_prompt(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_prompt_folder_id ON workspace_prompt(folder_id);
```

### 3.2 工作区信任模型（`src/app/trust.rs`）

> 实施补充（用户要求）：启动信任改为**进入应用前在当前终端屏幕行内显示信任页**（不进入备用屏幕、不进入主界面，对齐 Claude Code）；应用内触发（新建工作区 / 终端同步）保持模态浮层。

1. **启动信任页（`event::startup_trust`，主事件循环之前、行内会话 `Tui::init_inline`）**：
   - 计算真实规范化路径 `canonicalize(cwd)`；查询 `workspace` 表中 `path == target_path` 且 `is_trusted == 1`；
   - 已信任：直接进入应用（不进入任何会话）；
   - 未信任：清屏后在当前终端屏幕**左上对齐**绘制信任页（对齐 Claude Code，无居中浮层）：
     全宽 `─` 分隔线在最上方，其下左对齐 LX 像素 logo，再往下 1 列缩进的内容——`Accessing workspace:` /
     accent 路径 / `Quick safety check:` 安全提示 / 能力说明（加入工作区 + 提示词管理） /
     `No, exit`（默认选中）与 `Yes, I trust this folder` 两个可选项（整行全宽高亮 + `▸` 标记）/
     键鼠页脚；`↑/↓` 或鼠标悬停切换、`Enter` 或左键点击确认、`Esc` 取消、`y`/`n` 快捷选择；
   - 选择 `Yes`：清屏归位、录入 DB 并进入应用（此后才进入备用屏）；选择 `No`：清屏后输出
     `workspace not trusted; exiting`、exit 0，**不进入应用**；
   - 数据库不可用降级放行（不阻断应用）。
2. **应用内信任浮层（新建工作区 / 终端同步触发）**：
   - `Overlay::TrustWorkspace(TrustWorkspace)`：标题 `trust workspace`，问题 `Do you trust the files in this folder?`，
     路径高亮（中间省略）与按钮 `[trust & open enter]` / `[don't trust esc]`；
   - `Enter` 信任并录入 DB 后续作；`Esc`/点击按钮之外拒绝。
3. **回退行为**：
   - 启动（控制台页）：拒绝输出 `workspace not trusted; exiting` 并以 0 退出，不进入应用；
   - 新建工作区：关闭浮层，放弃新建，保持当前聚焦工作区并 Toast 提示；
   - 终端同步 cwd：关闭浮层，保留原工作区 cwd 不动，Toast 提示“Workspace sync cancelled”。

#### 3.3.1 实施补充（用户验收反馈，已确认）

1. **列表模型**：树视图展示行 = 过滤后的树行；工具栏（树视图）右端常显
   `[temp prompt]` 按钮（右对齐）：点击提交当前编辑并打开临时草稿编辑视图；`[edit]` 在选中行非条目
   （含空树）时回退打开草稿编辑视图（修复“点了没反应”的死按钮）。`New prompt`/`New folder`/
   `New subfolder` 统一打开命名浮层（标题 `new prompt`/`new folder`，输入框不预填、占位提示默认名）：
   确认后创建并选中新节点，**新建提示词不再自动进入编辑视图**（停留在树视图，显式 `[edit]`/回车才编辑）。
   编辑视图分割线行内嵌当前编辑目标 title（条目名 / 临时草稿 `temp prompt`），超宽截断。
2. **临时草稿**：入口为工具栏 `[temp prompt]`；正文只存内存（`PromptPanel::draft`，随工作区
   隔离，不入库）；保存入口在编辑视图中执行——草稿编辑视图保存点打开“保存到文件夹”选择器：
   顶部 `name` 输入框预填默认名（可改）；列表为根层 `/` + 文件夹树形展示（`├─`/`└─` 末项
   连接符）+ 动作行 `+ New folder`；点击文件夹仅选中、`[save enter]`/回车才执行；点击动作行
   打开与列表一致的新建文件夹命名浮层（预填 `New folder`，确认后立即建库并返回选择器、
   刷新列表并选中新文件夹；取消返回选择器不创建，挂根层，2 层约束不变）；字符输入编辑
   顶部名称，提示词名去空白为空则拒绝；保存后以所填名称创建正式条目并
   绑定编辑器、清空草稿；保存路径经 `Effect::SaveDraft` 出箱。弹窗顶边框右上角 `[x]` 与
   点击弹窗外关闭，点击弹窗内部内容不关闭（通用模态交互，见 tui-design-requirements）。
3. **搜索框**：树视图顶部常显，交互与模态输入框一致（`widgets::input` 视口 + 硬件光标 + 占位）：
   字符/`Backspace`/`Delete` 编辑过滤词、`←`/`→`/`Home`/`End` 移动光标、`Ctrl+C` 清空；
   过滤为大小写不敏感子串（匹配文件夹与条目；命中文件夹展示整棵子树、命中条目展示祖先链，
   过滤时忽略折叠状态并绘制展开态三角，清空恢复）；命中过滤后选中落到首个结果；`Esc` 清空；
   `↑`/`↓` 移动结果、回车打开/折叠（文件夹折叠不再占用 `←`/`→`）。
4. **树形连接符**：子层行按末项判定绘制 `├─`（其后仍有同深度可见行）/ `└─`（末项），
   与侧栏工作区树的对齐一致；嵌套文件夹（第 2 层文件夹）内的条目再深一层，
   以祖先续接线 `│`（祖先文件夹为非末项）/空格对齐。搜索框下方加贯穿分割线（`├`/`┤` 衔接边框），
   列表在分割线下方独立滚动：滚轮按行、右缘滚动条（轨道点击/thumb 拖拽），搜索框与分割线固定。

### 3.3 Prompt 管理器状态模型（`src/app/prompt/` 与 `src/app/state.rs`）

1. **树节点数据结构**：
   ```rust
   pub enum PromptNode {
       Folder {
           id: String,
           parent_id: Option<String>,
           name: String,
           collapsed: bool,
           depth: usize, // 限制 0 或 1，最大嵌套 2 层
           children: Vec<PromptNode>,
       },
       Item {
           id: String,
           folder_id: Option<String>,
           name: String,
           status: PromptItemStatus,
           updated_at: String,
       },
   }
   ```
2. **右键上下文菜单扩展（`OverlayTarget`）**：
   - `OverlayTarget::PromptFolder { workspace: usize, folder_id: String, depth: usize }`
   - `OverlayTarget::PromptItem { workspace: usize, item_id: String }`
   - `OverlayTarget::PromptTreeBackground { workspace: usize }`
3. **菜单命令集合**：
   - 提示词条目菜单：`EditPrompt`, `ChangeStatus(todo/in_progress/completed)`, `RenamePrompt`, `CopyPromptContent`, `DeletePrompt`；
   - 文件夹菜单：`NewPrompt`, `NewSubFolder`（仅 `depth == 0` 时展示，`depth == 1` 禁用或不展示）, `RenameFolder`, `DeleteFolder`；
   - 空白区域菜单：`NewPrompt`, `NewFolder`。

---

## 4. 风险与边界控制

1. **TUI 空间狭窄**：默认 prompt 宽度只有 48~52 列，必须保证树形缩进（每层 2 列）与超长名称省略（`…`）优雅，禁止换行破坏行高。
2. **嵌套层级硬约束**：嵌套深度严格限制为 2 层（根层文件夹 + 子文件夹），超过 2 层时新建文件夹命令严格禁用。
3. **数据库并发与 IO 隔离**：遵循 `AGENTS.md` 规范，UI/App 状态层保持纯内存，数据库操作由后台或同步受控服务执行，禁止在 `render` 中碰 DB。
4. **终端恢复顺序**：若未信任退出，必须严格执行 `Tui::restore()` 顺序，禁止带脏状态崩溃。

---

## 5. 验收标准

1. `lx` 在未信任目录启动时先在当前终端行内显示信任页（不进入应用），选 `Yes` 存库并进入应用；选 `No` 清屏提示并干净退出（exit 0）。
2. 新建工作区与终端同步右键菜单同样执行信任校验；拒绝时干净回退，状态无残留。
3. 提示词管理正确按当前工作区隔离并持久化至 `~/.lx-t/db/lx.db` 或 `~/.lx-t-dev/db/lx.db`。
4. 提示词文件夹支持展开/折叠、最大嵌套 2 层、右键菜单功能健全（新建、重命名、删除、状态、复制）。
5. 选中提示词可在右栏编辑器中实时编辑与保存，切换工作区后无缝重载对应提示词。
6. 全量通过 `cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与单元/集成测试。
7. 树视图工具栏含 `[temp prompt]` 按钮；空树或选中文件夹时 `[edit]` 回退打开草稿编辑视图，不存在无响应按钮。
8. 草稿可编辑并在保存选择器中存入根层/现有文件夹/经命名浮层新建的文件夹（挂根层）：弹窗支持命名提示词、
   文件夹树形展示、点击仅选中而 `[save enter]` 才执行、`+ New folder` 打开命名浮层（确认建库并选中、
   取消返回）；保存后清空草稿并绑定新条目；树视图输入即过滤（命中/无命中占位/`Esc` 清空），
   子层连接符按末项判定为 `├─`/`└─`，嵌套文件夹条目以祖先续接线 `│`/空格再深一层。
