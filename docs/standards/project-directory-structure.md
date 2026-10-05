# 项目目录结构

本文定义 lx-tui 的目录划分、模块职责与依赖方向。结构参照 herdr 的 TUI 架构做简化（不引入 server/client、插件、网站等额外层），交互层参考 codex（style / keymap / 纯数据终端模拟）。

## 总体目录

```text
lx-tui/
  Cargo.toml                    包与依赖声明
  Cargo.lock                    版本锁定，提交入库
  rust-toolchain.toml           固定 Rust 工具链版本
  clippy.toml                   Clippy 配置
  justfile                      统一命令入口（fmt / lint / test / run）
  AGENTS.md                     Agent 执行规则
  README.md                     项目说明
  docs/
    standards/                  项目规范（本文件所在）
    feature/                    功能设计与任务文档，按需创建
  src/
    main.rs                     启动编排：日志、终端初始化与恢复
    lib.rs                      crate 根；main.rs 保持薄入口，逻辑放模块
    layout.rs                   BSP 平铺模型（含折叠窄条）与几何计算，纯函数、不渲染
    app/                        应用状态与更新逻辑，纯数据、无 IO、可测试
      mod.rs
      state.rs                  唯一应用状态定义（含窗格种类与终端仿真状态）
      selection.rs              选区模型（终端窗格内文本选择）
      actions.rs                行为枚举
      update.rs                 行为到状态的转换、窗格尺寸同步与输出喂入
    ui/                         渲染层，只读状态
      mod.rs
      layout.rs                 屏幕区域划分（侧栏/标签栏/主区/右栏）
      style.rs                  语义化样式 Token（codex 风格 ANSI 配色）
      text.rs                   全部用户可见文案
      terminal.rs               终端网格到 Buffer 的渲染
      widgets/                  无业务语义的可复用组件，按需创建
    input/                      键盘路由与编码
      mod.rs                    键盘路由（仅 Ctrl+Q 退出，其余进焦点窗格）
      encode.rs                 按键到终端字节序列
    event/                      唯一事件循环：tokio select、PTY 消息与渲染调度
    terminal/                   终端仿真：alacritty_terminal 封装，纯内存、无 IO
    pty/                        PTY 会话：spawn / 读写 / 尺寸 / 终止
    tui/                        终端生命周期：原始模式、备用屏幕、恢复
    config/                     配置模型、默认值与校验
    platform/                   OS 专属实现（剪贴板、鼠标捕获与指针形状、默认 shell）
  tests/                        集成测试
  scripts/                      开发与维护脚本，按需创建
  assets/                       静态资源，按需创建
```

以上为目标结构。未实现的能力不提前创建目录或空文件，`按需创建` 的目录同理。

## 模块职责

1. `app/`：唯一状态来源；`state.rs` 只放数据结构（含窗格终端仿真状态），`update.rs` 只做状态转换；不得执行 IO、渲染或线程操作。
2. `layout.rs`（根级）：BSP 平铺模型与几何计算，`pane_rects` / `pane_inner_size` / 方向导航等均为纯函数，可直接用 `Rect` 单元测试。
3. `ui/`：接收 `&AppState` 绘制；`layout.rs` 负责屏幕区域划分，`terminal.rs` 负责把仿真网格写入 `Buffer`，均为纯渲染。
4. `event/`：唯一事件循环；tokio `select!` 收敛键盘、PTY 输出与定时事件为 `app` 行为调用与窗格写入，并做帧节流。
5. `input/`：终端事件到应用行为的映射与按键编码；`encode.rs` 为纯函数，按键判断不得散落在 `ui/` 组件中。
6. `terminal/`：终端仿真状态机（alacritty_terminal 封装）；字节进、画面出，不触碰 IO、不依赖其他模块状态。
7. `pty/`：真实进程会话；唯一允许持有 `portable-pty` 句柄与读线程的模块。
8. `tui/`：终端生命周期（原始模式、备用屏幕、括号粘贴、恢复与 panic 保护）；退出、panic 与终止信号时必须恢复终端状态。
9. `config/`：配置结构、默认值与校验；配置变更不直接改运行时状态，交由 `app` 行为处理。
10. `platform/`：OS 专属代码唯一落点，按 `#[cfg(...)]` 门控；核心模块不得出现裸 OS 分支。
11. `tests/`：跨模块集成测试；单元测试体放模块对应的 `tests.rs`，由主文件的 `#[cfg(test)] mod tests;` 引入，不集中存放。

## 依赖方向

```text
main -> 所有模块（只装配，不写业务）
event -> input, app, ui, tui, pty, terminal
ui -> app（只读状态）, layout（窗格几何）, terminal（只读网格）
input -> app（行为定义）, layout（方向导航）
app -> layout（平铺模型）, terminal（仿真状态）, config
pty -> layout（PaneId）
terminal -> 仅 alacritty_terminal / vte 与标准库（纯，无 IO）
tui -> 仅 crossterm / ratatui
layout -> 仅 ratatui 几何类型与标准库
config -> 仅标准库
platform -> 仅标准库与 OS 依赖
```

- `app/` 不得导入 `ui`、`input`、`event`、`pty`、`tui`。
- `ui/` 不得修改状态、不得执行 IO、不得依赖 `pty`、`tui`、`event`。
- `terminal/` 保持纯内存：不得使用 `std::io`、线程或 `pty`。
- `input/` 不得依赖具体终端实例；按键编码只依赖传入的 `TermMode`。
- 禁止跨模块深层导入实现文件；模块对外只暴露 `mod.rs` 中 `pub use` 的公开能力。
- 禁止因“可能复用”创建空的模块、目录或抽象层。
