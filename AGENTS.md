# 项目级开发规范

编写或评审代码前，必须阅读并遵守：

- `docs/standards/project-directory-structure.md`
- `docs/standards/code-writing-standards.md`
- `docs/standards/tui-design-requirements.md`
- `docs/standards/html-assets.md`

## Agent 执行规则

- lx-tui 是 Rust 单二进制 TUI，渲染基线为 ratatui + crossterm（与 herdr 同栈），布局参照 herdr 并保持简化。
- 保持 `app/`（纯状态与更新）与 `ui/`（纯渲染）边界清晰；渲染只读状态，禁止在渲染中修改状态或执行 IO。
- 优先最小修改，不为未来假设创建空目录、空文件或抽象层。
- 新增、移动或拆分代码前，确认模块归属、依赖方向及已有可复用能力。
- 单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右。
- 浏览器前端资源（如设置页）放在 `assets/*.{html,css,js}`，HTML/CSS/JS 分文件，经 `include_str!` 编译进二进制并按独立路由（`/settings.css`、`/settings.js`）提供；禁止把 HTML/CSS/JS 重新内联进 `.rs`，禁止引入 Tailwind 等需构建步骤的样式方案，遵循 `docs/standards/html-assets.md`。
- 生产代码禁止 `unwrap()` / `expect()`；错误使用 `Result` 与 `?` 传播。
- 日志统一使用 `tracing`；禁止用 `println!` / `eprintln!` 输出调试信息，禁止绕过 ratatui 直接写 stdout。
- **lx 页的 toast 必须显示在 lx 页内**：lx 视图的反馈一律写 `pane.lx.toast`（`app::update::show_lx_toast` / `show_toast_at` / `show_lx_toast_for_session`），在窗格内容区里绘制（`ui::main_content::lx::toast`，宽度夹在窗格内、允许折行），禁止使用全局 `AppState.toast` 覆盖层。全局槽只有一个，且锚点容器放不下时会降级到屏幕顶部居中、横跨整个窗口——lx 的反馈会因此失去指代并挡住别的窗格（compact 失败提示曾这样横穿全屏）。终端视图与 prompt 栏等非 lx 区域仍用全局 toast。
- OS 专属代码只允许出现在 `src/platform/`，并按 `#[cfg(...)]` 门控。
- 终端退出/panic 恢复顺序不可交换：关闭鼠标上报 → 仍在原始模式下 drain 在途输入 → 离开备用屏并关闭括号粘贴 → 最后 `disable_raw_mode()`。先恢复 cooked 模式会让残留鼠标上报（`CSI <…M`）被终端回显、留在行缓冲并泄漏进 shell；改动 `src/tui/` 恢复路径必须保留并运行 `tui::tests` 的顺序回归测试。
- 完成修改后按影响范围执行验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- Agent 无法交互式验证 TUI 界面；涉及交互与外观的改动由用户运行 `cargo run` 自行确认。
- 以上文档是项目规范的唯一来源；若与上级指令冲突，以上级指令为准。

## 分支合并与数据库

- 合并前核对 `src/db/mod.rs` 的 `SCHEMA_VERSION` 与各分支新增迁移：多个分支并行加迁移时，后合并的一方必须顺延版本号（如 v12 已被占用则改为 v13），并保留旧库升级路径。禁止两个分支用同一版本号指向不同迁移——先启动的二进制会把库标成该版本，另一方的迁移会被静默跳过，造成缺列/缺表。
- 涉及整表重建（关闭外键、`DROP TABLE`）的迁移，先在库副本上验证 `user_version`、数据量与 `PRAGMA integrity_check`，再迁移真实开发库；迁移前关闭所有 lx 进程，并保留 `~/.lx-t-dev/db/lx.db.v*.bak*` 便于回滚。
- 删除 worktree 分支时，先停掉该目录下运行的 lx，再同步删除其隔离库 `~/.lx-t-dev/db/worktrees/<目录名>-<路径哈希>/`；旧隔离库留着会在重建同名 worktree 时被继续误用。
- 开发库按 linked worktree 隔离：主 checkout 固定使用 `~/.lx-t-dev/db/lx.db`，生产固定使用 `~/.lx-t/db/lx.db`；不要把隔离库复制回主库，也不要让未打补丁的旧二进制指向新库。
