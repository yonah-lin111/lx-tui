# worktree 信任继承与 prompt 共享 — 执行任务

## 1. 信任归位 repo_root

- [x] `TrustTrigger` 新增 `OpenWorktree`；`TrustResolution` 请求模型
- [x] `Effect::CheckTrust` 增加 `root: Option<PathBuf>`；服务端 `is_trusted(path) || is_trusted(root)` 判定
- [x] `commit_worktree_open`：已打开直接切换；新条目经信任检查续作创建
- [x] 显式路径与终端同步 cwd：未信任时登记仓库根解析请求
- [x] 事件层：后台解析仓库根 → `AppEvent::TrustRootResolved` 回投 → 重新入队信任检查
- [x] 启动信任门：解析 repo_root 后判定与登记

## 2. prompt 按 repo_root 键共享

- [x] `Workspace::prompt_key()`（git → repo_root，回退 cwd）
- [x] `PromptPanel.loaded_key`：键变化自动重载
- [x] 服务层效应与树重载改用 prompt 键；`reload_tree` 重载所有同键面板
- [x] 内容自动保存与状态变更跨同键面板同步

## 3. 测试与验证

- [x] 信任：worktree 继承主 checkout 信任（静默放行、不入库 worktree 路径）
- [x] 信任：主 checkout 未信任 → 浮层目标为主 checkout，确认只登记主 checkout
- [x] 信任：拒绝 → 不创建条目 + toast；已打开条目切换无检查
- [x] 启动门：worktree 目录启动且主 checkout 已信任 → 无信任页
- [x] prompt：worktree 条目与主条目共享同一棵树，互相变更后同步
- [x] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`
