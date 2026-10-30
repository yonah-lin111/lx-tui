# lx MCP 与 Skills — 执行任务

> 状态：**已完成并完成评审修复（未合并）**。依据 [design.md](design.md)，代码评审见 [review.md](review.md)，验证步骤见 [verification.md](verification.md)。
> 阶段顺序：P1 → P2 → P3 → P4 → P5 → P6 → Review 修复（P0–P2）。每阶段以对应的定向测试和验收条件收口，再进入下一阶段。
>
> **当前进度**（2026-10-30，worktree `.worktrees/lx-mcp-skills`，分支 `feat/lx-mcp-skills`）：
> - 已提交：`d4416ea`、`0bc0f19`、`b097cbf`（OAuth/设置页/取消/TUI 卡片与进度/缓存/集成测试）、`b9b9168`（`SKILL.md` 大小写兼容）、`15aa057`（设置页滚动/MCP 卡片格式与斜体提示）、`91ad3d4`（启动即装载、首页计数、skill 移入 @ 面板、工具树独立折叠）、`4f77408`（MCP 卡片点击面边界测试）、`9ecd262`（首页计数窄窗格裁切）、`017bdc7`（skill 缓存清理）、`2feb3ee`（首页计数下移、多窗格项目级计数、`.agents` 默认关闭、设置页统一保存与 tab 黄点）、`d02e04d`（配置变更清理已移除 MCP server 计数）、`8cf99e2`（首页计数上方间距改为 1 行）。
> - 已完成：P1–P5 全部功能、P6 集成用例与真实开发库副本迁移复核，以及 [review.md](review.md) 中 P0–P2 全部评审问题的代码修复与回归测试。
> - MCP：stdio/HTTP 客户端、OAuth 登录/刷新重连、启动进度、工具与 resources 三工具统一 `notifications/cancelled` + 超时保护、JSON-RPC 业务错误与传输断开区分处理、resources 图片归档、`tools/list_changed` 即时刷新 `snapshot_cache`、失败/登录 pane 内英文 toast。
> - Skills：mtime 失效缓存、多工作区与 `GitRefreshed` 仓库根变化自动重扫、`.agents` 默认关闭（`enabled` 白名单）、目录预算与注入截断/读取失败 pane 内英文 toast。
> - 设置页：`/api/mcp`、`/api/skills`、OAuth callback、`settings_mcp.js`、`settings_skills.js`，右上角统一跨 tab 保存、脏 tab 黄点、自定义超时保留、只读 `.agents` 保护与下拉菜单 DOM 回收。
> - TUI：MCP 类型化卡片（运行中/成功/失败/中断统一结构，参数与结果独立折叠，`^o` 快捷键切换卡片展开）、首页 agent 模式下方空 1 行显示 `mcp · skills` 计数、pane 内 toast、explorer MCP 只读约束。
> - 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（3898 lib tests + 集成测试）与 `node --check` 全部通过；真实 v19 开发库副本迁移到 v20 后 `integrity_check=ok`、`foreign_key_check` 为空、附件行数 2→2。
> - **尚未合并**（用户要求先完成实现，不要合并）。

## 0. 开工门槛

- [x] 用户确认设计与任务清单；按反馈先修订文档。实现工作区按设计决策 32 创建，创建后分别初始化 codegraph 与 codebase memory 索引。
- [x] 重读 `AGENTS.md` 与四份 `docs/standards/` 规范；用 codegraph 和 codebase memory 复核拟改模块的数据流、调用方与当前代码，不按设计中的旧行号直接修改。
- [x] **修正设计基线**：实现按当前基线落地 `SCHEMA_VERSION = 20`、`PROMPT_VERSION = "lx-7"`、内置工具 16 项；v19 已被子 agent 表占用，附件 `skill` kind 使用 v20。
- [x] 核对当前未提交改动、真实数据库路径和运行中的 lx；不覆盖用户改动，不擅自修改 Git 状态。记录 P1 前精确基线与需要维护的旧库版本。（主 checkout 仅有未跟踪的 `docs/feature/lx-mcp-skills/`；本次迁移仅在开发库副本执行）
- [x] 确认 `rmcp` crates.io 稳定版、客户端 feature 与项目 `reqwest 0.13` 的兼容性；实现阶段优先查 Context7，再核对 crate 实际 API。新增依赖后用 `cargo tree -d` 检查重复依赖。（锁定 `rmcp 3.8.0`，其 `reqwest 0.13.2` 与项目 0.13 同版本）

## P1. 基础契约、图片目录与数据库迁移

- [x] 在 `platform::path_text` 抽出项目 scope 命名与哈希的唯一实现，`db` 改为复用；新增 `config::mcp`、`config::skills` 的类型、校验、作用域路径、合并和原子读写。项目文件校验 `project_root`，未信任项目不生效；私有目录 `0700`、配置文件 `0600`。
- [x] 新增拥有所有权的 `ToolDecl` 与 `ChatRequest.tools: Arc<Vec<_>>`，将四个 transport 的工具声明改为读取冻结列表；内置工具原有顺序、各 `ToolSurface` 白名单保持不变。执行层只接受本轮声明过的名字，空列表用于摘要请求。
- [x] 把会话图片目录改为 `session/<sid>/image/`，提供字节归档入口；删除、touch、30 天 GC 同步适配。引用查询纳入工具 `result_json` 中的图片路径，避免 GC 误删。
- [x] 在**数据库副本**上先验证迁移：版本、附件行数、全部索引、外键和 `PRAGMA integrity_check`。整表重建前停止目标数据库的 lx 进程并制作可恢复备份；迁移为附件增加 `skill` kind，保留旧库升级链。（2026-10-30 开发库副本 v19→v20：附件 2 行不变、`integrity_check=ok`、`foreign_key_check` 为空、`skill` kind 已生效；未操作真实开发库）
- [x] 图片搬迁放在 schema 升级之后的启动补偿流程：逐文件 rename，成功才改对应 `source_path`；失败保留旧路径；重复运行、搬迁后事务失败、多个 worktree 隔离库共享旧图片目录均能补写。只删除空旧目录。
- [x] 定向验证：配置校验/合并/权限、`turn_tools` 顺序与越权拦截、四 transport 无图片时的工具声明、附件迁移副本、图片搬迁/崩溃补偿/GC。P1 不启动真实 MCP server。

## P2. MCP 客户端与连接管理

- [x] 锁定 P0 核实的 `rmcp` 版本及 feature；`rmcp` 调用只放在 `src/mcp/`。实现 stdio、Streamable HTTP、分页工具发现、resources capability 与一次性 probe。
- [x] 实现 `platform::mcp_process`：白名单环境 + 显式透传 + 显式覆盖、项目 cwd、独立进程组、stderr 限长日志、退出整组终止；OS 分支只在 `platform/`。
- [x] 实现工具名清洗/64 字符截断/碰撞反查、Schema 清洗和 5000 字节降级；单个坏工具跳过并记录警告。实现 `enabled_tools`/`disabled_tools` 与稳定排序。
- [x] 实现 `McpManager`：按项目根与 server 名缓存、并行懒启动、指纹变化局部重连、失败不自动重试、`tools/list_changed` 只影响下一快照、启动进度、超时、取消和 shutdown；旧快照调用已移除工具返回明确失败。（含 `starting` 在途去重、`StartupProgress` 事件、`tools/list_changed` 原地替换、2s 优雅关闭后整组 SIGKILL）
- [x] 实现 resources 三工具，只在有 resources capability 时注册；全 server 列表排序，单 server 透传分页，`read_resource` 复用结果转换。
- [x] 实现 OAuth：独立 `credentials.json` 原子 `0600` 保存、revision 参与指纹、10 分钟一次性 state、动态注册/PKCE、刷新失败进入 `NeedsLogin`、登出失效；token 与 `client_secret` 不进日志和错误文本。
- [x] 定向验证：配置指纹、命名碰撞、Schema 边界、stdio 环境隔离、测试 server 的懒启动/并行/超时/取消/重连/退出无残留、resources 分页、OAuth state 与刷新失败。（新增 `tests/fixtures/mcp_echo.py` 集成测试：调用/取消/超时/退出、指纹只重启目标 server、`tools/list_changed`、resources 列表/读取；OAuth mock 覆盖发现/注册/换 token）

## P3. 工具结果图片与跨轮回放

- [x] 实现 MCP 内容转换：`structuredContent` 优先且保留图片，text/resource 拼接，audio 与非图片资源给英文占位；`isError` 转工具失败；文本上限 64 KiB，图片最多 4 张，归档失败计入 omitted。
- [x] `ToolExecutionResult`、`ModelItem::ToolResult`、`HistoryBlock::ToolResult` 增加图片元数据/字节通路；工具执行时归档，SQLite 只存路径与尺寸。`read` 图片走同一工具结果通路，移除额外伪造的 user 图片消息。
- [x] 按设计 §5.2 分别实现 Anthropic、Responses、Gemini 原生工具图片；Chat Completions 在连续 tool 结果后只补一条 user 媒体消息；Gemini 1/2 走兼容补消息路径。模型不支持图片时在事件层统一转文本占位。
- [x] 历史只保留最近 3 个含工具图片的轮次；旧轮次和缺失文件给英文占位，token 估算每张 +1600（主会话与子 agent 共用同一规则）；用户图片回放规则不变。压缩摘要只含图片描述，不带字节。
- [x] **先固定无图片 wire 回归样本**，再改五类协议路径；逐协议验证无图片字节一致、有图片形状、连续调用顺序、缺图、超限与跨轮回放。（`transport/tools` 新增四协议逐字节快照；缺图回放追加 `[image no longer available]`；Gemini 动态工具参数经 `trim_for_gemini` 裁剪）

## P4. Skills 发现、注入与引用

- [x] 实现四个发现根、信任门控、最深 6 层/每根 2000 目录上限、隐藏目录跳过、符号链接环检测、同名优先级与 shadowed、`.agents` 默认禁用（`enabled` 白名单）与 `lx` 按 canonical 路径禁用；扫描在后台，按目录和 `SKILL.md` mtime 失效缓存；多工作区及 `GitRefreshed` 仓库根归位后自动补扫。
- [x] 实现 `SKILL.md` 有界解析（256 KiB、frontmatter 标量与块标量、description 必填、name 校验、大小写不敏感文件名兼容）；错误进入目录错误列表，不阻断其他 skill。编辑仅允许 lx 自有根，服务端生成 frontmatter。
- [x] 实现英文 skill 目录渲染：8000 字符预算、description 截断/整行省略、主/子 agent 使用说明不同；与环境快照一起在 turn 起点冻结。调整提示组合顺序并递增到**当前基线之后**的版本（`PROMPT_VERSION = "lx-7"`），守住 12/16 KiB 与单条 system 2000 UTF-16 码元限制。
- [x] `@` 提及面板加入已启用 skill 候选（排在文件候选之前），选中插入 `@[refer-skill](<绝对路径>)`；`/` 面板只保留内置命令；扩展引用解析、显示、快速删除。发送时全文只注入当轮，单个 8000 字节截断并给窗格内英文 toast；读取失败不阻止发送，历史只回放摘要行。
- [x] 定向验证：解析失败、四根优先级/信任/禁用/环、目录预算、提示顺序与英文约束、面板过滤与导航、引用注入/截断/失败和附件 kind。

## P5. 主/子 agent 接线、设置页与 TUI

- [x] 事件层持有全局 `McpManager`；主会话和子 agent 各自在 turn 起点取 MCP 与 skill 冻结快照，后续采样复用。内置工具先列、MCP 按名排序、resources 最后；三种角色都可用启用的 MCP 工具，explorer 仅在英文提示中约束只读。
- [x] agent loop 为 MCP 调用提供真实取消路径：Esc 发 MCP cancelled 通知，工具票据按 `Cancelled` 结算并取消未开始项；调用超时和 server 断开按设计错误语义处理。子 agent 复用项目连接，但图片写入自己的 session 目录。
- [x] 设置服务新增 MCP/Skills API 与 OAuth callback；scope 只从 TUI 推送的可信项目表解析，未知 404、未信任拒绝、`.agents` 写入 403；所有写请求保持 token/同源校验，callback 在 token 校验前以 state 验证。GET 脱敏，空密钥字段表示保留原值；设置页 probe 使用独立连接。
- [x] `assets/settings.html`、`settings_mcp.js`、`settings_skills.js` 与已有脚本/样式接线；静态资源用 `include_str!` 独立路由，复用现有布局、确认框和 toast。覆盖配置/测试/重启/登录/登出、skill 增删改/启停与只读来源。
- [x] MCP 工具卡片新增类型化详情与折叠/展开布局；工具标记取 `StepMark::glyph`，块间空行只由 `history::snapshot` 计算。启动进度进 lx 活动标签，MCP/skill 反馈统一写 `pane.lx.toast`，用户可见文案集中 `ui::text`。
- [x] 定向验证：主/子 turn 冻结、权限与取消、设置路由安全和脱敏、静态资源、MCP 卡片行数/点击区域、进度与窗格内 toast。浏览器和 TUI 的实际交互由用户运行 `cargo run` 确认。

## P6. 集成验收与交付

- [x] 用临时配置、临时数据库和本地测试 server 覆盖：配置变化仅重启目标 server；工具发现变化只在下一 turn 生效；主/子 agent 调用、resources、OAuth 失败恢复；图片跨轮回放、删除源文件后仍可用、30 天 GC 不误删。（本地 stdio fixture 已覆盖配置指纹局部重启、`tools/list_changed`、resources、取消/超时/退出；OAuth 用 mock 服务覆盖登录链路）
- [x] 对照设计 §11 的 10 条不变量与 §12 测试矩阵逐项核验；核对生产文件 ≤1000 行、测试随子模块下沉、`app/` 与 `ui/` 边界、OS 代码归属、英文模型文本和敏感字段不泄漏。
- [x] 完成功能评审（[review.md](review.md)），按 P0 → P1 → P2 顺序修复全部发现的问题并补充回归测试。
- [x] 只对受影响模块先跑定向测试；代码完成后执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`，记录实际命令和结果。涉及终端恢复路径时额外运行 `tui::tests` 顺序回归。（三条命令全部通过，3898 lib tests + 集成测试全绿）
- [x] 合并前再次核对 schema 版本与各分支迁移占用，确认旧库到新版本连续可达；在副本上复核 `user_version`、行数、`foreign_key_check`、`integrity_check`，保留可回滚备份。（`dev` 仍为 19，本分支 20 不冲突；开发库副本验证通过，按要求暂不合并）
- [x] 交付说明已实现范围、验证证据、未验证的 TUI/浏览器观感及剩余风险；未经明确授权不修改 Git 状态。（未合并、未修改主 checkout 的 Git 状态）
