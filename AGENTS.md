# 项目级开发规范

编写或评审代码前，必须阅读并遵守：

- `docs/standards/project-directory-structure.md`
- `docs/standards/code-writing-standards.md`
- `docs/standards/tui-design-requirements.md`

## Agent 执行规则

- lx-tui 是 Rust 单二进制 TUI，渲染基线为 ratatui + crossterm（与 herdr 同栈），布局参照 herdr 并保持简化。
- 保持 `app/`（纯状态与更新）与 `ui/`（纯渲染）边界清晰；渲染只读状态，禁止在渲染中修改状态或执行 IO。
- 优先最小修改，不为未来假设创建空目录、空文件或抽象层。
- 新增、移动或拆分代码前，确认模块归属、依赖方向及已有可复用能力。
- 单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右。
- 生产代码禁止 `unwrap()` / `expect()`；错误使用 `Result` 与 `?` 传播。
- 日志统一使用 `tracing`；禁止用 `println!` / `eprintln!` 输出调试信息，禁止绕过 ratatui 直接写 stdout。
- OS 专属代码只允许出现在 `src/platform/`，并按 `#[cfg(...)]` 门控。
- 完成修改后按影响范围执行验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- Agent 无法交互式验证 TUI 界面；涉及交互与外观的改动由用户运行 `cargo run` 自行确认。
- 以上文档是项目规范的唯一来源；若与上级指令冲突，以上级指令为准。
