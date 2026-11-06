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
- **生产代码**单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右。**测试文件组织遵循主流 Rust 规范**：严格跟随被测模块的职责与目录层级拆分，生产代码拆出子模块时测试必须同步迁入对应的子模块（如 `<module>/<submodule>/tests.rs`），严禁在模块根部堆积数千行的单体测试；单测试文件建议控制在 1000 行以内，上限不超过 1500 行；跨场景超限按场景聚类拆分子测试文件，共用测试夹具抽取到 `test_support` 或 `fixtures` 模块（详见 `docs/standards/code-writing-standards.md`）。
- 浏览器前端资源（如设置页）放在 `assets/*.{html,css,js}`，HTML/CSS/JS 分文件，经 `include_str!` 编译进二进制并按独立路由（`/settings.css`、`/settings.js`）提供；禁止把 HTML/CSS/JS 重新内联进 `.rs`，禁止引入 Tailwind 等需构建步骤的样式方案，遵循 `docs/standards/html-assets.md`。
- 生产代码禁止 `unwrap()` / `expect()`；错误使用 `Result` 与 `?` 传播。
- 日志统一使用 `tracing`；禁止用 `println!` / `eprintln!` 输出调试信息，禁止绕过 ratatui 直接写 stdout。
- **lx 的模型提示词一律用英文编写**（系统提示、工具描述与 schema description、工具错误消息、环境快照、压缩提示、注入上下文的结构文本）；中文只用于代码注释与日志。**TUI 和浏览器设置页中所有用户可见文案一律英文**（toast、面板、工具卡片、状态行、空态与按钮），禁止中文文案泄漏到界面。系统提示结构对齐 codex `base_instructions`，默认回复语言跟随用户当前输入的语言（不预设、不强制单一语言）。
- **工具树展开/折叠**：各可折叠块独立，点击一块只切换该块，禁止整卡一起展开；`├─` 块展开后的续行保留 `│`，`└─` 块展开后的续行使用等宽空白、不补 `│`；`└─` 输出与汇报内容在折叠与展开态下统一采用思考块同款的压暗 markdown 格式（`markdown_render::render_wrapped` + `span_style.patch(style::muted())`），`search` 命中行与附属标签恒定保持 `style::muted()`，不在展开后恢复为 `style::text()`。该规则适用于所有工具卡片，细节见 `docs/standards/tui-design-requirements.md`。
- **agent 身份名固定为 `lx`，不可变**（`chat::prompt::AGENT_NAME` 唯一来源，提示首行由它生成）：不得由环境变量、命令行、设置项改写；改名等于换 agent，必须同时递增 `PROMPT_VERSION`。详见 `docs/standards/code-writing-standards.md`。
- **lx 页的 toast 必须显示在 lx 页内**：lx 视图的反馈一律写 `pane.lx.toast`（`app::update::show_lx_toast` / `show_toast_at` / `show_lx_toast_for_session`），在窗格内容区里绘制（`ui::main_content::lx::toast`，宽度夹在窗格内、允许折行），禁止使用全局 `AppState.toast` 覆盖层。全局槽只有一个，且锚点容器放不下时会降级到屏幕顶部居中、横跨整个窗口——lx 的反馈会因此失去指代并挡住别的窗格（compact 失败提示曾这样横穿全屏）。终端视图与 prompt 栏等非 lx 区域仍用全局 toast。
- **模态弹窗触发的 toast 必须在全局显示**：凡由居中模态弹窗（`Overlay::Connect`、`Overlay::WorktreeOpen`、`Overlay::TrustWorkspace`、`Overlay::NewWorkspace`、`Overlay::Rename`、`Overlay::ConfirmClose`、`Overlay::NewNode`、`Overlay::SaveDraft`、`Overlay::ReferencedFolder*`、`Overlay::ResumeSessionEdit` 等）触发的反馈（含操作成功/失败提示、复制反馈、打开浏览器、刷新模型等），必须在全局 `AppState.toast` 显示（`app::update::show_toast` 且 `anchor: None`，在屏幕顶部正中央绘制、晚于模态浮层渲染，不被模态压暗或遮挡），禁止写入固定容器（如 `pane.lx.toast` 或带 `Some(pane_id)` 容器锚点）。
- OS 专属代码只允许出现在 `src/platform/`，并按 `#[cfg(...)]` 门控。
- 终端退出/panic 恢复顺序不可交换：关闭鼠标上报 → 仍在原始模式下 drain 在途输入 → 离开备用屏并关闭括号粘贴 → 最后 `disable_raw_mode()`。先恢复 cooked 模式会让残留鼠标上报（`CSI <…M`）被终端回显、留在行缓冲并泄漏进 shell；改动 `src/tui/` 恢复路径必须保留并运行 `tui::tests` 的顺序回归测试。
- **lx 步骤标记与空行**：文字数据（正文 / 思考 / 消息级通知）用圆点 `●`/`○`，**只有工具调用用方块 `■`/`□`**；字形只在 `ui::text::agent::STEP_MARK_*` 定义，由 `StepMark::glyph(kind, loading_frame, animating)` 按 `StepKind` 选取，渲染层不得写死字符。同一条消息内任意两块之间（含合并文件工具组）一律空一行，空行只由 `history::snapshot` 累加，绘制路径不得另算。
- 完成修改后按影响范围执行验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- Agent 无法交互式验证 TUI 界面；涉及交互与外观的改动由用户运行 `cargo run` 自行确认。
- 以上文档是项目规范的唯一来源；若与上级指令冲突，以上级指令为准。

## 分支合并与数据库

- 合并前核对 `src/db/mod.rs` 的 `SCHEMA_VERSION` 与各分支新增迁移：多个分支并行加迁移时，后合并的一方必须顺延版本号（如 v12 已被占用则改为 v13），并保留旧库升级路径。禁止两个分支用同一版本号指向不同迁移——先启动的二进制会把库标成该版本，另一方的迁移会被静默跳过，造成缺列/缺表。
- 涉及整表重建（关闭外键、`DROP TABLE`）的迁移，先在库副本上验证 `user_version`、数据量与 `PRAGMA integrity_check`，再迁移真实开发库；迁移前关闭所有 lx 进程，并保留 `~/.lx-t-dev/db/lx.db.v*.bak*` 便于回滚。
- 删除 worktree 分支时，先停掉该目录下运行的 lx，再同步删除其隔离库 `~/.lx-t-dev/db/worktrees/<目录名>-<路径哈希>/`；旧隔离库留着会在重建同名 worktree 时被继续误用。
- 开发库按 linked worktree 隔离：主 checkout 固定使用 `~/.lx-t-dev/db/lx.db`，生产固定使用 `~/.lx-t/db/lx.db`；不要把隔离库复制回主库，也不要让未打补丁的旧二进制指向新库。
