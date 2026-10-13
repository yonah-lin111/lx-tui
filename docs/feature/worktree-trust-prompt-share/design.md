# worktree 信任继承与 prompt 共享 — 设计

- 分支：`feature/worktree-trust-prompt-share`
- 状态：**已完成**

## 1. 背景与目标

1. **worktree 的信任归属缺失**：左侧 ws 中打开 git worktree 条目（worktree 对话框、显式路径新建、启动目录、终端同步 cwd）时，信任检查要么缺失、要么按 worktree 目录本身判定与入库。用户期望：**只要主工作区（仓库主 checkout）被信任，其 worktree 目录默认信任、不重复入库**；主 checkout 未信任时，询问的是主 checkout 而不是 worktree 目录。
2. **右侧 prompt 重复创建**：prompt 树按工作区 cwd 存取，打开 worktree 条目会得到一棵空的独立树，在其中新建的提示词落到 worktree 路径名下。用户期望：**同一仓库的所有工作区条目共享主工作区的一棵提示词树**，不在库里重复创建。

## 2. 已确认决策（Grill Me 结论）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 信任检查/写库目标路径 | 一律解析到**仓库主 checkout（repo_root）**；先查 worktree 路径本身（兼容历史已信任数据，命中直接放行），未命中再查 repo_root：已信任直接放行且**不给 worktree 路径入库**；未信任弹浮层显示 repo_root、确认后**只给 repo_root 入库** |
| 2 | 规则覆盖入口 | 四条入口全覆盖：①worktree 对话框开条目（repo_root 同步已知）；②新建工作区弹窗显式路径（后台异步解析仓库根）；③启动信任门（进入主循环前同步解析）；④终端右键同步 cwd（后台异步解析）。未知路径一律由事件层后台跑 git，不阻塞 UI 线程 |
| 3 | prompt 共享键 | 任何带 git 元数据的工作区（主 checkout 或 linked worktree）prompt 键取 `repo_root`；非 git 工作区仍取 `cwd`；主 item 自身也按 repo_root 键。旧子目录键数据不迁移（dev 工具接受） |
| 4 | 共享深度 | **只共享数据**：同键面板共享同一棵提示词树并在变更后互相同步；各 item 保留自己的视图状态（选中/滚动/模式/过滤词/临时提示词）与编辑器草稿（沿用 prompt-per-workspace 语义） |
| 5 | 拒绝语义 | 拒绝信任 = 取消整个打开/创建操作（worktree 与自动补开的主 checkout 都不建）+ 复用 `Workspace creation cancelled` toast；已打开条目的纯切换不做信任检查、直接切换 |

## 3. 架构与数据流

### 3.1 信任归位（`src/app/trust.rs`、`src/app/effects.rs`、`src/db/service.rs`、`src/event/`）

- `Effect::CheckTrust` 增加 `root: Option<PathBuf>`：
  - `Some(root)`：调用方已解析出仓库主 checkout（worktree 对话框），服务端按 `is_trusted(path) || is_trusted(root)` 判定；
  - `None`：显式路径，服务端先做 `is_trusted(path)` 快速判定，未信任时**登记解析请求**（`AppState.trust_resolutions`），由事件层后台 `git::list_worktrees` 解析仓库根后回投 `AppEvent::TrustRootResolved`，主循环再以 `root = Some(...)` 重新入队检查。
- `TrustTrigger` 新增 `OpenWorktree { entry, repo_root, main_branch }`：信任通过后续作创建 worktree 工作区并自动补开主 checkout；拒绝时取消并提示。
- `commit_worktree_open`：条目已打开 → 直接切换（无检查）；新条目 → 关闭对话框并入队 `CheckTrust { path: entry.path, root: Some(repo_root), trigger: OpenWorktree{..} }`。
- 启动信任门（`event::trust`）：`startup_trust_target` 先查 cwd 是否已信任；未信任时解析 repo_root，repo_root 已信任则直接放行，否则信任页显示 repo_root 并只登记 repo_root。

### 3.2 prompt 共享（`src/app/state.rs`、`src/app/update/prompt.rs`、`src/db/service.rs`）

- `Workspace::prompt_key() -> Option<&Path>`：`git.repo_root` 优先，回退 `cwd`。
- `PromptPanel.loaded` 旁新增 `loaded_key: Option<PathBuf>`：记录已载入树对应的键；git 元数据晚到（cwd 键 → repo_root 键）时自动重载。
- 服务层：效应携带的 `workspace` 仍为来源 item 的 cwd（定位用）；DB 读写与树重载统一改用该 item 的 `prompt_key()`；`reload_tree` 改为**重载所有同键面板**（同仓库主 checkout 与 linked worktree 同步刷新）。
- 非重载变更的跨面板同步：`SavePromptContent` 成功后内容与待确认标记同步到所有同 id 面板；`change_prompt_status` 状态同步到所有同 id 面板；`apply_prompt_tree` 在条目被同键面板删除时解除该面板的编辑绑定。

## 4. 风险与边界

- 主 item 的 cwd 是主 checkout 子目录时，prompt 键变为 repo_root：旧子目录键下的提示词不再显示（不迁移）。
- 显式路径的仓库根解析是异步的：极端情况下解析结果回投时用户已发起下一轮操作，浮层目标可能滞后（与既有信任检查无取消机制一致）。
- 信任目标变化后，`grant` 只登记 repo_root；worktree 路径自身的历史信任行保留（兼容），不再新增。

## 5. 验收标准

- worktree 对话框选中 linked worktree：主 checkout 已信任 → 静默创建且库中无 worktree 路径行；未信任 → 浮层显示主 checkout，确认后只登记主 checkout；拒绝 → 不创建任何条目 + toast。
- 显式路径/启动目录/终端同步三条入口同样归位到 repo_root 判定与登记；启动于已信任仓库的 worktree 内不弹信任页。
- 同一仓库的主条目与 worktree 条目右侧 prompt 展示同一棵树：任一侧增删改/改名/状态/内容自动保存后，另一侧切回即为最新数据；库中提示词全部挂在 repo_root 名下。
- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
