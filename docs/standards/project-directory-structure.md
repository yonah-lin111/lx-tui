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
      overlay.rs                浮层模型（右键菜单、重命名输入与关闭确认）
      selection.rs              选区模型（终端窗格内文本选择）
      actions.rs                行为枚举
      state/lx/                 lx 页状态：编辑器、聊天会话（chat.rs）、`/` 命令面板（slash.rs）
      update/                   行为到状态的转换（含 update/chat：发送/流事件/终态/命令）
    ui/                         渲染层，只读状态
      mod.rs
      layout.rs                 屏幕区域划分（侧栏/标签栏/主区/右栏）
      style.rs                  语义化样式 Token（codex 风格 ANSI 配色）
      text.rs                   全部用户可见文案
      markdown.rs               编辑器 markdown 高亮（保留标记上色）
      markdown_render.rs        面向阅读的 markdown 渲染器（折叠标记；UI 页 agent 回答与 lx 聊天共用）
      markdown_render/table.rs  markdown 表格渲染（列宽收缩、单元格行内渲染与 `<br>` 换行）
      main_content.rs           主内容渲染：按视图分派 lx 页与终端网格（含 lx 子模块）
      main_content/lx/          lx 页渲染：欢迎页（brand.rs 品牌区）、网格（grid.rs）、命令面板（slash.rs）、历史（history.rs）、状态（status.rs，覆盖绘制在窗格底边框）、聊天栏（chat_bar.rs，loading + 回到底部）、提及面板
      widgets/                  无业务语义的可复用组件（菜单、模态容器等），按需创建
    input/                      键盘路由与编码
      mod.rs                    键盘路由（仅 Ctrl+Q 退出，其余进焦点窗格）
      encode.rs                 按键到终端字节序列
    event/                      唯一事件循环：tokio select、PTY 消息与渲染调度
      chat.rs                   聊天网络流编排：取走网络效应、装配上下文、回投结果
    terminal/                   终端仿真：alacritty_terminal 封装，纯内存、无 IO
    pty/                        PTY 会话：spawn / 读写 / 尺寸 / 终止
    git/                        git 查询：worktree 列表的执行与解析（唯一执行 git 命令的模块）
    db/                         SQLite 持久层：工作区信任、提示词/文件夹、聊天会话 CRUD（唯一使用 rusqlite 的模块）
      chat.rs                   聊天会话/消息/内容块读写（迁移只在 mod.rs）
      service.rs                同步持久化服务：执行 app 层效应出箱、按需载入提示词树
    tui/                        终端生命周期：原始模式、备用屏幕、恢复
    config/                     配置模型、默认值与校验
      ai.rs                     模型配置（config/ai.json）：类型、校验与原子保存
    chat/                       流式聊天协议适配：统一事件、SSE 帧解析、thinking 标签、五种 transport、请求执行与上下文装配
      events.rs                 统一纯数据事件（ChatDelta/PartKind/StopReason）
      sse.rs                    可增量喂入的 SSE 帧解析器
      thinking.rs               <think>/<thinking> 跨 chunk 标签状态机
      transport.rs              按 transport 构造请求与解码器（openai/openai-responses/anthropic/google）
      client.rs                 reqwest 流式执行、超时与取消
      context.rs                上下文装配（完整轮次、窗口裁剪）
    settings/                   设置与命令文件持久化，及本地回环设置 Web 服务
      commands.rs               自定义命令 Markdown 文件的读写
      service.rs                回环 HTTP 服务：静态设置页、JSON API 与事件投递（路由 async，禁止在运行时内再建运行时）
      service/models.rs         模型设置 API：GET 脱敏 / PUT 原子保存 / POST 远端发现（发现直接 await，不 block_on）
    platform/                   OS 专属实现（剪贴板、鼠标捕获与指针形状、默认 shell、进程 cwd、打开浏览器、文件/目录权限）
  tests/                        集成测试
  scripts/                      开发与维护脚本，按需创建
  assets/                       浏览器前端资源（HTML/CSS/JS），经 include_str! 嵌入二进制
    settings.html               设置页结构
    settings.css                设置页样式
    settings.js                 设置页脚本
```

以上为目标结构。未实现的能力不提前创建目录或空文件，`按需创建` 的目录同理。

## 模块职责

1. `app/`：唯一状态来源；`state.rs`/`state/` 只放数据结构（含窗格终端仿真状态与 lx 聊天会话状态），`update/` 只做状态转换；不得执行 IO、渲染或线程操作。`update/chat.rs` 实现 lx 聊天状态机（发送/流事件/终态/`/` 命令），只产生 `Effect` 与纯状态更新。
2. `layout.rs`（根级）：BSP 平铺模型与几何计算，`pane_rects` / `pane_inner_size` / 方向导航等均为纯函数，可直接用 `Rect` 单元测试。
3. `ui/`：接收 `&AppState`（及其只读派生）绘制；`layout.rs` 负责屏幕区域划分，`main_content.rs` 按窗格视图分派 lx 页与终端网格并写入 `Buffer`，均为纯渲染；lx 页的聊天历史（`main_content/lx/history.rs`）、状态（`main_content/lx/status.rs`，覆盖绘制在窗格底边框上）按职责拆分，命中几何与绘制共享纯布局函数。
4. `event/`：唯一事件循环；tokio `select!` 收敛键盘、PTY 输出与定时事件为 `app` 行为调用与窗格写入，并做帧节流。
5. `input/`：终端事件到应用行为的映射与按键编码；`encode.rs` 为纯函数，按键判断不得散落在 `ui/` 组件中。
6. `terminal/`：终端仿真状态机（alacritty_terminal 封装）；字节进、画面出，不触碰 IO、不依赖其他模块状态。
7. `pty/`：真实进程会话；唯一允许持有 `portable-pty` 句柄与读线程的模块；窗格 shell 按所属工作区 cwd 启动。
8. `git/`：唯一执行 git 命令的模块（`git worktree list --porcelain` 的查询与解析）；由事件层在后台线程调用，app 只接收转换后的纯数据。
9. `db/`：唯一使用 rusqlite 的模块：SQLite 初始化与 `PRAGMA user_version` 迁移、工作区信任读写、提示词与文件夹 CRUD（含 2 层嵌套约束与级联删除）、聊天会话/消息/内容块 CRUD（`chat.rs`，含 v7 五表迁移）；`service.rs` 是事件循环持有的同步受控服务，执行 app 层 `Effect` 出箱并惰性载入提示词树；app 层保持纯内存，禁止直接触碰 rusqlite。
10. `tui/`：终端生命周期（原始模式、备用屏幕、括号粘贴、恢复与 panic 保护）与光标去重后端（`QuietCursor`：按物理状态去重下发 Hide/Show/MoveTo）；退出、panic 与终止信号时必须恢复终端状态。
11. `config/`：配置结构、默认值与校验；`ai.rs` 承载模型配置（`config/ai.json`）的类型、校验与原子保存；配置只读写本项目数据根（`~/.lx-t[-dev]/config/ai.json`），不读取 lx-agent 的 `~/.lx`；配置变更不直接改运行时状态，交由 `app` 行为处理。
12. `settings/`：设置与命令文件持久化（`settings.json`、自定义命令 Markdown 文件）及本地回环设置 Web 服务；`service.rs` 只监听 `127.0.0.1` 并带每次运行随机 token，静态资源（`page.rs` 以 `include_str!` 加载 `assets/`，经独立路由提供）与 JSON API 共用一套存储函数；`service/models.rs` 提供模型设置的脱敏 GET、原子 PUT 与远端发现 POST，状态变更经事件投递回 `event` 循环。
13. `chat/`：流式聊天协议适配，唯一使用 `reqwest` 的模块：统一纯数据事件（`events.rs`）、可增量 SSE 帧解析（`sse.rs`）、thinking 标签状态机（`thinking.rs`）、五种 transport 的请求/解码（`transport/`）、请求执行与取消（`client.rs`）与上下文装配（`context.rs`）；不依赖 `app`/`ui`，DB 依赖仅限记录类型转换。
14. `platform/`：OS 专属代码唯一落点，按 `#[cfg(...)]` 门控（含文件/目录权限收紧）；核心模块不得出现裸 OS 分支。
15. `tests/`：跨模块集成测试；单元测试体放模块对应的 `tests.rs`，由主文件的 `#[cfg(test)] mod tests;` 引入，不集中存放。
16. `assets/`：浏览器前端资源，HTML/CSS/JS 分文件维护，经 `include_str!` 编译进二进制并按独立路由提供，遵循 `html-assets.md`。

## 依赖方向

```text
main -> 所有模块（只装配，不写业务）
event -> input, app, ui, tui, pty, terminal, git, db, settings, chat
ui -> app（只读状态）, layout（窗格几何）, terminal（只读网格）
input -> app（行为定义）, layout（方向导航）
app -> layout（平铺模型）, terminal（仿真状态）, config, db（仅记录类型转换，无 IO）, chat（事件类型）
pty -> layout（PaneId）
git -> 仅标准库
db -> 仅 rusqlite 与标准库
chat -> db（仅记录类型转换）, config（模型快照）, serde/reqwest 与标准库
settings -> config, commands 存储与标准库 / tokio（不含 app、ui、event 实现）
terminal -> 仅 alacritty_terminal / vte 与标准库（纯，无 IO）
tui -> 仅 crossterm / ratatui
layout -> 仅 ratatui 几何类型与标准库
config -> 仅标准库（serde/serde_json）, platform（原子保存权限）
platform -> 仅标准库与 OS 依赖
```

- `app/` 不得导入 `ui`、`input`、`event`、`pty`、`tui`、`git`。
- `ui/` 不得修改状态、不得执行 IO、不得依赖 `pty`、`tui`、`event`。
- `terminal/` 保持纯内存：不得使用 `std::io`、线程或 `pty`。
- `input/` 不得依赖具体终端实例；按键编码只依赖传入的 `TermMode`。
- 禁止跨模块深层导入实现文件；模块对外只暴露 `mod.rs` 中 `pub use` 的公开能力。
- 禁止因“可能复用”创建空的模块、目录或抽象层。
