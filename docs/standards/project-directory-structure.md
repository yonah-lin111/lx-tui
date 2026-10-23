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
      state/lx/                 lx 页状态：编辑器、聊天会话（chat.rs 会话/请求状态机）、
                                聊天步骤时间线（chat/steps.rs：part/工具/status/摘要/类型化详情）、
                                进程/计划卡片视图（chat/tool_view.rs：CommandView /
                                ProcessInteractionView / PlanSnapshotView）、
                                文件差异详情（chat/file_diff.rs：从落库 result_json 解码的
                                类型化 FileDiffDetailView，坏文件局部降级）、
                                流事件→步骤状态转换（chat/stream.rs）、`/` 命令面板（slash.rs）、
                                输入模式判定（shell.rs：仅首个非空白 `!`/`！` 进入 shell）、
                                本地命令卡片与队列视图（chat/shell.rs）
      update/                   行为到状态的转换（含 update/chat：发送/流事件/终态；
                                update/chat/session.rs：会话生命周期与 `/` 命令；
                                update/chat/view.rs：DB 记录→步骤时间线投影；
                                update/chat/process.rs：卡片展开/键盘选择、进程事件更新、
                                计划提交与重载恢复；
                                update/chat/user_shell.rs：本地命令提交/回执/FIFO 队列
                                与停止（不占用模型 RequestState）；
                                update/chat/viewport.rs：滚动/视口记忆/卡片折叠
                                （toggle_lx_card；无 hover 态））
    ui/                         渲染层，只读状态
      mod.rs
      layout.rs                 屏幕区域划分（侧栏/标签栏/主区/右栏）
      style.rs                  语义化样式 Token（codex 风格 ANSI 配色）
      text.rs                   全部用户可见文案
      markdown.rs               编辑器 markdown 高亮（保留标记上色）
      markdown_render.rs        面向阅读的 markdown 渲染器（折叠标记；UI 页 agent 回答与 lx 聊天共用）
      markdown_render/table.rs  markdown 表格渲染（列宽收缩、单元格行内渲染与 `<br>` 换行）
      main_content.rs           主内容渲染：按视图分派 lx 页与终端网格（含 lx 子模块）
      main_content/lx/          lx 页渲染：欢迎页（brand.rs 品牌区）、网格（grid.rs）、命令面板（slash.rs）、历史（history.rs（置顶用户消息行在 history/sticky.rs；本轮统计行在 history/stats.rs；卡片命中类型 CardKind/CardHit 在 history/cards.rs）；history/tool.rs 是工具步骤分派与共用行能力：方块/动词/路径/行范围/增删计数与状态，样式 token 与文案均取自 style / text::agent；history/tool/file_summary.rs 是普通工具的第二行摘要组装；history/tool/file_diff.rs 是 edit/write 的文件差异卡片——每文件 3 行预览与完整展开、连续同种调用的合并卡、增删行底色与轻量语法高亮，rows/draw 共用同一布局）；history/user_shell.rs 是本地命令卡片：与工具步骤共用直角树体（`│`/`└─`），标题 `■ Running` / `■ You run command · exit 0 · 1.2s` 不带连接符；参数区与输出区各自独立折叠（参数恒为浅黄、不响应 hover（`… N more lines` 同色）；输出折叠 dim、悬停或展开为默认前景（只有真的能展开才响应 hover），不铺灰底；`└─` 只收口参数区），运行中只给 `open task panel · live output` 一行引导，rows/draw/regions 共用同一布局）、聊天历史命中与视口入口（history_hit.rs：置顶行/回到底部、工具卡片点击面与折叠内容 hover 区（`CardHit { card, fold }`）、滚动条与行数口径）、消息底部通知（notice.rs）、状态（status.rs，覆盖绘制在窗格底边框）、聊天栏（chat_bar.rs，`Working... (2.7s · ↑1.6k ↓320 tok)` + Esc 待命提示 + 回到底部；提示类反馈已改走 main content 的 toast）、提及面板
      widgets/                  无业务语义的可复用组件（菜单、模态容器等），按需创建
    input/                      键盘路由与编码
      mod.rs                    键盘路由（仅 Ctrl+Q 退出，其余进焦点窗格）
      encode.rs                 按键到终端字节序列
    event/                      唯一事件循环：tokio select、PTY 消息与渲染调度
      chat/mod.rs               聊天轮次编排：取走网络效应、装配上下文、启动 agent turn、
                                工具调用 BeginTool/FinishTool 的 DB→状态→回执握手与按 key 过滤
      chat/host.rs              agent loop ↔ 事件层的请求枚举与 `AgentHost` 生产实现
      chat/tool_calls.rs        BeginTool/FinishTool 握手：pending/running/结果落库门槛与结算
      chat/timeline.rs          步骤 position、parts 快照、checkpoint 与草稿提取
      chat/process_events.rs    进程事件 → UI 更新 + 受控输出快照落库（每秒合并、终态立即）
      chat/user_shell.rs        本地命令编排：DB 预登记 → spawn → 输出/终态落库 →
                                队列分派、停止与下一次采样前的增量注入握手
      chat/process/rtk/         rtk 集成：外部二进制检测/命令包装（mod.rs）与
                                内置输出过滤（builtin.rs）；agent 与本地命令共用
    terminal/                   终端仿真：alacritty_terminal 封装，纯内存、无 IO
    pty/                        PTY 会话：spawn / 读写 / 尺寸 / 终止
    git/                        git 查询：worktree 列表的执行与解析（唯一执行 git 命令的模块）
    db/                         SQLite 持久层：工作区信任、提示词/文件夹、聊天会话 CRUD（唯一使用 rusqlite 的模块）
      migration.rs              Schema 迁移（v4~v9）：建表、表重建、数据回填与 `user_version` 推进
      chat.rs                   聊天会话/消息/内容块读写与按序加载
      chat/tools.rs             工具调用 CRUD、时间线位置分配、按会话加载与启动恢复
      chat/process.rs           会话进程元数据（agent_process）：登记、输出快照合并、
                                终态提交（revision 单调）、按会话批量加载与启动恢复
      service.rs                同步持久化服务：执行 app 层效应出箱、按需载入提示词树
    tui/                        终端生命周期：原始模式、备用屏幕、恢复
    config/                     配置模型、默认值与校验
      ai.rs                     模型配置（config/ai.json）：类型、校验与原子保存
    chat/                       流式聊天协议适配：统一事件、SSE 帧解析、thinking 标签、五种 transport、请求执行与上下文装配；
                                lx agent 身份提示（prompt.rs：唯一稳定系统指令文本）；
                                transport/tools/：七工具 wire 形状（Chat/Responses/Anthropic/Google）、
                                历史 call/result 归并与跨帧工具参数缓冲；
                                文件工具执行层（chat/tools/：read/search/edit/write 与 Codex 风格补丁引擎；
                                进程/计划工具的参数规范化与结果 JSON）；
                                会话进程管理器（chat/process/：pipe 与 PTY spawn、双流 drain、
                                有界输出缓冲与安全文本化、stdin、等待窗口、停止与 reap）；
                                计划契约（chat/plan.rs：全量替换、最多一个 in_progress、快照 JSON）
      events.rs                 统一纯数据事件（ChatDelta/PartKind/StopReason）
      sse.rs                    可增量喂入的 SSE 帧解析器
      thinking.rs               <think>/<thinking> 跨 chunk 标签状态机
      transport.rs              按 transport 构造请求与解码器（openai/openai-responses/anthropic/google）
      client.rs                 reqwest 流式执行、超时与取消
      context/                  上下文装配（完整轮次分组、text/工具步骤合并为有序 items、整轮预算裁剪、
                                单条结果 64 KiB 回放上限；turns 与 items 两视图同源）
      tools/mod.rs              文件工具执行入口：参数校验分发、上限常量、结构化结果
      tools/types.rs            工具上下文/结果/变更/错误码/ToolSpec 契约
      tools/schema.rs           七工具稳定英文名、描述与严格 JSON Schema（additionalProperties=false）
      tools/args.rs             参数白名单校验（不信任模型遵守 schema）
      tools/path.rs             路径解析（相对路径以冻结 cwd 为基准）与 IO 错误归一化
      tools/text.rs             文本编解码：BOM/换行保留、行切分与行 diff 计数
      tools/diff.rs             结构化行差异：hunks/真实行号/精确增删计数、上下文半径 1、
                                超限输入的线性简化脚本与生成预算（纯计算，不读写文件）
      tools/read.rs             read：带行号内容、目录列表、截断与 next_offset
      tools/search.rs           search：ignore 遍历、正则/字面量、include glob、上限与超时
      tools/patch.rs            Codex 风格补丁纯解析模型（Add/Update/Delete/Move）
      tools/edit.rs             edit：补丁规划（路径校验）与逐个提交，多文件部分成功如实报告
      tools/write.rs            write：创建或完整覆盖、父目录创建、原子替换
      tools/lock.rs             按规范化路径的短时写锁（不做全局串行）
      tools/output.rs           format_for_model / to_summary / to_result_json 三合一呈现
      tools/output/file.rs      edit/write/失败结果的 JSON 形状、file_totals 与差异正文预算裁切
                                （512 KiB / 10,000 行，超限先裁正文再省略文件摘要）
      agent_loop/mod.rs         agent loop 纯编排：多步采样、ordinal 串行工具调度、取消与限额
      agent_loop/executor.rs    生产执行器 `AgentTools`：文件走 spawn_blocking、进程走会话
                                ProcessManager、计划走校验 + 修订号；本轮停止集合
                                （不依赖 app/ui/event，文件 IO 经注入的 ToolExecutor）
    settings/                   设置与命令文件持久化，及本地回环设置 Web 服务
      commands.rs               自定义命令 Markdown 文件的读写
      service.rs                回环 HTTP 服务：静态设置页、JSON API 与事件投递（路由 async，禁止在运行时内再建运行时）
      service/models.rs         模型设置 API：GET 脱敏 / PUT 原子保存 / POST 远端发现（发现直接 await，不 block_on）
    platform/                   OS 专属实现（剪贴板、鼠标捕获与指针形状、默认 shell、进程 cwd、打开浏览器、文件/目录权限、原子文件替换）
  tests/                        集成测试
  scripts/                      开发与维护脚本，按需创建
  assets/                       浏览器前端资源（HTML/CSS/JS），经 include_str! 嵌入二进制
    settings.html               设置页结构
    settings.css                设置页样式
    settings.js                 设置页脚本
```

以上为目标结构。未实现的能力不提前创建目录或空文件，`按需创建` 的目录同理。

## 模块职责

1. `app/`：唯一状态来源；`state.rs`/`state/` 只放数据结构（含窗格终端仿真状态与 lx 聊天会话状态），`update/` 只做状态转换；不得执行 IO、渲染或线程操作。`update/chat/` 实现 lx 聊天状态机（发送/流事件/终态/`/` 命令），只产生 `Effect` 与纯状态更新；展示历史唯一顺序来源是 `ChatMessageView::steps`（part 与工具步骤共用同一 `position`），由 `update/chat/view.rs` 从 DB 的 parts 与 tools 按 position 合并。
2. `layout.rs`（根级）：BSP 平铺模型与几何计算，`pane_rects` / `pane_inner_size` / 方向导航等均为纯函数，可直接用 `Rect` 单元测试。
3. `ui/`：接收 `&AppState`（及其只读派生）绘制；`layout.rs` 负责屏幕区域划分，`main_content.rs` 按窗格视图分派 lx 页与终端网格并写入 `Buffer`，均为纯渲染；lx 页的聊天历史（`main_content/lx/history.rs`，工具步骤行在 `main_content/lx/history/tool.rs`，单行摘要、样式 token 取自 `ui::style`）、消息底部通知（`main_content/lx/notice.rs`）、状态（`main_content/lx/status.rs`，覆盖绘制在窗格底边框上）按职责拆分，命中几何与绘制共享纯布局函数（工具卡片行数由同一 `tool::rows` 单一口径给出：普通文件工具 2 行、进程卡片存活 3 行/终态 2 行、带差异正文的文件卡片按预览/展开可变）。聊天历史的命中函数集中在 `main_content/lx/history_hit.rs`（置顶跳转、回到底部、工具卡片点击面 `card_target_at` 与折叠内容 hover 区 `fold_target_at`、滚动条与行数），`lx.rs` 只保留页面布局与绘制。
4. `event/`：唯一事件循环；tokio `select!` 收敛键盘、PTY 输出与定时事件为 `app` 行为调用与窗格写入，并做帧节流。`event/chat/` 是 lx agent 轮次的唯一编排者：启动 agent turn、按 `RequestKey` 过滤过期事件、并处理工具调用的 BeginTool/FinishTool 握手（DB pending/running/结果落库 → 状态 → 回执）；可见窗格的 pending 只在该帧绘制完成后才推进 running 并回执 ticket。
5. `input/`：终端事件到应用行为的映射与按键编码；`encode.rs` 为纯函数，按键判断不得散落在 `ui/` 组件中。
6. `terminal/`：终端仿真状态机（alacritty_terminal 封装）；字节进、画面出，不触碰 IO、不依赖其他模块状态。
7. `pty/`：真实进程会话；唯一允许持有 `portable-pty` 句柄与读线程的模块；窗格 shell 按所属工作区 cwd 启动。
8. `git/`：唯一执行 git 命令的模块（`git worktree list --porcelain` 的查询与解析）；由事件层在后台线程调用，app 只接收转换后的纯数据。
9. `db/`：唯一使用 rusqlite 的模块：SQLite 初始化与 `PRAGMA user_version` 迁移（`migration.rs`，含 v4~v13：v9 为 part 完成时刻与工具调用表重建、v11 为会话进程元数据表、v12 为撤销 journal 与文件快照表、v13 为会话模型列可空与消息 `kind`/`user_shell_json`）；路径生产 `~/.lx-t/db/lx.db`、开发 `~/.lx-t-dev/db/lx.db`，开发环境在 linked worktree 内隔离为 `db/worktrees/<目录名>-<路径哈希>/lx.db`（worktree 根由入口解析，db 层不依赖 git）；工作区信任读写、提示词与文件夹 CRUD（含 2 层嵌套约束与级联删除）、聊天会话/消息/内容块 CRUD 与按序加载（`chat/mod.rs`）、工具调用状态机与时间线位置分配（`chat/tools.rs`）、会话进程元数据（`chat/process.rs`，工具结果体保持不可变）、本地命令消息事务与 revision 单调校验（`chat/user_shell.rs`）；`service.rs` 是事件循环持有的同步受控服务，执行 app 层 `Effect` 出箱并惰性载入提示词树，启动恢复由事件层按 session guard 执行；app 层保持纯内存，禁止直接触碰 rusqlite。
10. `tui/`：终端生命周期（原始模式、备用屏幕、括号粘贴、恢复与 panic 保护）与光标去重后端（`QuietCursor`：按物理状态去重下发 Hide/Show/MoveTo）；退出、panic 与终止信号时必须恢复终端状态。
11. `config/`：配置结构、默认值与校验；`ai.rs` 承载模型配置（`config/ai.json`）的类型、校验与原子保存；配置只读写本项目数据根（`~/.lx-t[-dev]/config/ai.json`），不读取 lx-agent 的 `~/.lx`；配置变更不直接改运行时状态，交由 `app` 行为处理。
12. `settings/`：设置与命令文件持久化（`settings.json`、自定义命令 Markdown 文件）及本地回环设置 Web 服务；`service.rs` 只监听 `127.0.0.1` 并带每次运行随机 token，静态资源（`page.rs` 以 `include_str!` 加载 `assets/`，经独立路由提供）与 JSON API 共用一套存储函数；`service/models.rs` 提供模型设置的脱敏 GET、原子 PUT 与远端发现 POST，状态变更经事件投递回 `event` 循环。
13. `chat/`：流式聊天协议适配，唯一使用 `reqwest` 的模块：统一纯数据事件（`events.rs`）、可增量 SSE 帧解析（`sse.rs`）、thinking 标签状态机（`thinking.rs`）、`lx` 身份提示（`prompt.rs`）、五种 transport 的请求/解码（`transport/`，其中 `transport/tools/` 统一七工具 wire 形状、历史 call/result 归并与跨帧参数缓冲）、请求执行与取消（`client.rs`，含“已产生工具输入即不重试”的重试闸门）与上下文装配（`context/`：完整轮次分组、`parts`/`tools` 按 position 合并为有序 `ModelItem`、整轮预算裁剪与单条结果 64 KiB 回放上限，`turns` 与 `items` 两视图同源）；`chat/agent_loop/` 是 agent loop 纯编排（多步采样、按 ordinal 串行调度工具、取消与步骤/调用上限），只依赖纯数据事件与 `db::chat` 记录类型，持久化与状态经注入的 `AgentHost` 由事件层回复；`chat/tools/` 是文件工具执行层（`read`/`search`/`edit`/`write` 与 Codex 风格补丁引擎）：相对路径以调用方冻结的 `ToolContext::cwd` 为基准、同步阻塞（由调用方 `spawn_blocking`）、只返回结构化结果，`format_for_model`/`to_summary`/`to_result_json` 是唯一三种呈现口径；工具层不触碰 Git 索引/分支/暂存区/提交，写文件经 `platform::write_file_atomic` 原子替换并按规范化路径加短时写锁。不依赖 `app`/`ui`/`event`。
14. `platform/`：OS 专属代码唯一落点，按 `#[cfg(...)]` 门控（含文件/目录权限收紧、原子文件替换 `write_file_atomic`/`rename_over`）；核心模块不得出现裸 OS 分支。
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
chat -> db（仅记录类型转换）, config（模型快照）, platform（原子写）, serde/reqwest 与标准库
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
