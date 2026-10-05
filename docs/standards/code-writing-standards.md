# 代码编写规范

本文定义 lx-tui 的 Rust 代码、错误处理、并发、依赖与验证规则。

## Rust 与注释

- 代码以 `cargo fmt` 输出为准，不允许未格式化提交。
- Clippy 必须全绿：`cargo clippy --all-targets -- -D warnings`；`#[allow]` 必须附注释说明原因，禁止用整文件 `#![allow]` 压制。
- 命名遵循 Rust 规范：类型与枚举 `UpperCamelCase`，函数、变量与模块 `snake_case`，常量 `SCREAMING_SNAKE_CASE`。
- 标识符、配置键与状态值使用英文稳定值；中文只用于注释和用户可见文案；禁止写入 Unicode 替换字符 `U+FFFD`。
- 公共项使用 `///` 文档注释说明职责与错误条件；内部逻辑使用简体中文单行注释，保持简洁。
- 所有用户可见文案集中在 `src/ui/text.rs`，禁止在组件或逻辑中硬编码字符串。

## 错误处理与日志

- 生产代码禁止 `unwrap()` / `expect()`；测试代码允许。
- 错误使用 `Result` 与 `?` 向上传播；`main.rs` 顶层统一记录并退出。
- 禁止吞错误；有意忽略错误时必须显式写出并附注释说明原因。
- 日志统一使用 `tracing`；禁止 `println!` / `eprintln!` 输出调试信息，禁止任何绕过 ratatui 直接写 stdout 的行为。
- 高频路径（输入、渲染、PTY 读取）禁止日志刷屏。

## 并发与事件

- 状态变更只在事件循环内发生；后台任务通过 channel 发送消息，禁止多线程直接修改 `AppState`。
- 渲染是纯函数：只接收 `&AppState`，禁止在渲染路径中修改状态、执行 IO 或阻塞。
- 文件、进程、网络等阻塞操作必须放入后台任务，不得阻塞事件循环。
- 持锁范围最小化；禁止跨 `.await` 持有同步锁。

## 模块拆分

- 单文件超过 1000 行必须按职责拆分，拆分子文件控制在 500 行左右；不得只压缩排版。
- `app/` 按 state / actions / update 职责拆分；`ui/` 按区域与组件拆分。
- 一个模块出现两个以上无关职责时拆分；不为“整齐”过度拆分。

## 依赖与平台

- 新增依赖必须有明确理由，先确认标准库与现有依赖无法覆盖；禁止为小功能引入重依赖。
- OS API 与平台行为必须在 `src/platform/` 内按 `#[cfg(...)]` 门控，核心模块不得出现 `cfg(target_os)` 分支。

## 测试与验证

- 单元测试体与实现分离：主文件只保留 `#[cfg(test)] mod tests;` 声明，测试放同层 `tests.rs`（`mod.rs` 模块）或同名子目录 `tests.rs`（普通模块，如 `src/app/state.rs` → `src/app/state/tests.rs`）；集成测试放 `tests/`。
- `app/` 状态必须能在无终端、无 PTY 环境下构造并测试；`layout.rs` 与 `ui/layout.rs` 等纯函数直接覆盖边界尺寸。
- 修复 bug 必须先补复现测试，再改代码。
- 修改完成后按影响范围执行：

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

受影响范围优先，提交前全量。TUI 交互与外观由用户运行 `cargo run` 自行确认。
