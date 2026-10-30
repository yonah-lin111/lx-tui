# lx MCP 与 Skills — 验证报告

> 验证对象：`feat/lx-mcp-skills`（worktree `.worktrees/lx-mcp-skills`，HEAD `8cf99e2`）
> 验证日期：2026-10-30
> 报告原则：按**用户可走通的路径**聚合验证项，每项同时覆盖配置、运行时、设置页/UI 与持久化中的多个实现点，不按函数逐条罗列。
> 自动化命令均在项目根目录执行；需要 TUI/浏览器观感的步骤标注为“人工确认”。

## 环境与基线

| 项目 | 值 |
|---|---|
| 分支 | `feat/lx-mcp-skills` |
| 基线 | `dev`（`SCHEMA_VERSION = 19`） |
| 本分支版本 | `SCHEMA_VERSION = 20`、`PROMPT_VERSION = lx-7` |
| 本机已导入 MCP | `~/.lx-t-dev/config/mcp.json`（来源 `~/.config/opencode/opencode.json`）：`codebase-memory-mcp`、`codegraph`、`context7`、`ssh-manager`，均为 stdio |
| 测试 MCP server | `tests/fixtures/mcp_echo.py`（initialize / tools / resources；支持 pid、工具列表文件与 sleep；仅用于确定性集成测试） |
| 质量门槛 | `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` |

---

## V1. 导入配置、MCP 设置页与 OAuth 登录闭环

**覆盖点**：`opencode.json` MCP 配置导入、MCP 配置 CRUD、用户级/项目级作用域、信任门控、脱敏、空密钥保留、测试连接、强制重启、OAuth 发现/动态注册/PKCE/回调/凭据落盘。

**输入内容**

`~/.config/opencode/opencode.json` 的 `mcp` 段（已按此转换写入 `~/.lx-t-dev/config/mcp.json`）：

```json
{
  "mcp": {
    "servers": {
      "codebase-memory-mcp": {
        "enabled": true,
        "transport": "stdio",
        "command": "/Users/yonah/.local/bin/codebase-memory-mcp",
        "args": []
      },
      "codegraph": {
        "enabled": true,
        "transport": "stdio",
        "command": "codegraph",
        "args": ["serve", "--mcp"]
      },
      "context7": {
        "enabled": true,
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "@upstash/context7-mcp@latest"]
      },
      "ssh-manager": {
        "enabled": true,
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "@zibdie/ssh-mcp-server"]
      }
    }
  }
}
```

导入一致性检查：

```bash
jq '.mcp | keys' ~/.config/opencode/opencode.json
jq '.mcp.servers | keys' ~/.lx-t-dev/config/mcp.json
stat -f '%Sp %N' ~/.lx-t-dev/config/mcp.json
```

另建一个临时 HTTP + `auth=oauth` 的 server（导入的 4 个均为 stdio）用于 OAuth 登录/登出与刷新失败验证。

**验证步骤**

1. 运行 `cargo run`，点击设置图标打开 `http://127.0.0.1:<port>/?token=<token>`，切到 **MCP**。
2. 确认列表出现 4 个导入的 server（`codebase-memory-mcp`、`codegraph`、`context7`、`ssh-manager`），状态点从 `◐` 变为 `●`；缩小窗口高度，把 MCP 表单滚到底部。
3. 对 `codegraph` 点 **Test connection**；对 `codebase-memory-mcp` 也执行一次；记录 `pgrep -af 'codegraph serve --mcp|codebase-memory-mcp'` 的 pid。
4. 对 `codegraph` 点 **Restart**，再次记录 pid；对临时 OAuth server 执行 Login → 等待 Ready → Logout。
5. 用带 token 的请求读取配置：`curl 'http://127.0.0.1:<port>/api/mcp?token=<token>'`。
6. 把临时 HTTP server 的 `bearer_token` 输入框留空再次保存，检查旧 token 是否保留；把 Auth 改为 `none` 再保存，检查 bearer 字段是否被清除。
7. 备份 `credentials.json`，把 `expires_at_ms` 改为过去时间、`refresh_token` 改为无效值，点 Restart，观察状态与 lx 页 toast。

**期望结果**

- `jq` 显示两边 server 名一致，均为 4 个；`~/.lx-t-dev/config/mcp.json` 的目录 `0700`、文件 `0600`；项目级文件位于 `.../config/mcp/projects/<目录名>-<路径哈希>.json`，未信任项目不可写（403），未知 scope 404。
- 4 个 server 均出现在 MCP tab 且变为 Ready；不再出现测试 fixture 的 `echo` server；MCP 表单在卡片内部滚动（`editor-body` 可滚到底部），页面本身不被裁切。MCP/Skills 编辑器不显示独立 Save 按钮，统一使用右上角操作条；任一 tab 修改后左侧出现黄点，右上角 Save 一次保存全部脏 tab，成功后黄点清空。
- `codegraph` 的测试连接返回 `codegraph_explore` 等工具；`codebase-memory-mcp` 返回 `search_graph`、`get_code_snippet` 等工具；`context7` 返回 `resolve-library-id` / `query-docs`；`ssh-manager` 返回连接管理工具。
- Restart 后只有 `codegraph` 的 pid 变化，其余 server pid 不变。
- OAuth 登录成功后 `~/.lx-t[-dev]/mcp/credentials.json` 为 `0600`，状态点变为 Ready；Logout 后回到未登录/不可用状态；token 过期且 refresh 被拒后进入 `NeedsLogin`，设置页显示需登录，lx 页提示在设置页登录。
- `GET /api/mcp` 不返回 `bearer_token` / `client_secret` 原文，只返回 `secret_flags`；空串保留旧值，切换认证方式时旧字段被清理。
- 自动化覆盖：`cargo test --lib settings::service::tests`、`cargo test --lib mcp::oauth`。

---

## V2. MCP 运行时：工具、resources、取消、进度与退出

**覆盖点**：导入配置的懒启动/并行启动、工具发现与过滤、`tools/list_changed` 下一快照生效、工具调用、resources 三工具、Esc 取消、启动/调用超时、失败不自动重试、启动进度、pane 内 toast、退出无残留子进程。

**输入内容**

已导入的 4 个 server（同 V1，全部 stdio）。用户依次输入：

```text
用 codegraph_explore 查看 src/mcp/manager.rs 的结构
用 context7 查 rmcp 的客户端 API 文档
用 codebase-memory 的 search_graph 找 McpManager
用 ssh-manager 列出当前连接
```

resources 用 `tests/fixtures/mcp_echo.py` 或任一声明 resources capability 的 server；确定性取消用 fixture 的 `sleep` 工具，或用 `codebase-memory-mcp` 的 `index_repository` 长任务。

**验证步骤**

1. 发送第一条消息，观察 lx 页输入框上方的活动标签。
2. 等工具卡片出现，依次展开检查参数/结果与标题。
3. 发送 context7、codebase-memory、ssh-manager 对应请求，确认各 server 的工具被独立路由。
4. 发送 resources 请求，检查 `list_resources` / `read_resource` 的卡片与结果。
5. 发起一个长任务（fixture `sleep` 或 `index_repository`），在工具 Running 时按一次 `Esc`。
6. 修改 fixture 的 `MCP_TOOLS_FILE`（或 `disabled_tools`）后再发一轮，检查下一轮工具列表。
7. 退出 lx，执行 `pgrep -af 'codegraph serve --mcp|codebase-memory-mcp|@upstash/context7|ssh-mcp-server'`。

**期望结果**

- lx 首页三个 agent 入口下方空一行显示 `loading mcp · loading skills`，装载完成显示 `4 mcp · 3 skills`；有失败时显示 `3 mcp 1 error`。输入框上方不再出现 MCP 启动进度。
- 工具卡片标题为 `■ codegraph · codegraph_explore · 183ms` 形式；参数 `├─` 连接符为灰色、参数正文为浅黄；结果 `└─` 折叠态正文灰色、展开态恢复默认前景；参数/结果的 `… N more lines` 提示为斜体。点击参数块只展开参数、点击结果块只展开结果，行为与命令卡片的独立分区一致；`└─` 展开续行不补 `│`，`├─` 展开续行保留 `│`。
- 新分割窗格与同一项目/工作区的其他窗格显示同一份 MCP/skill 计数；不同工作区按各自项目配置分别显示。
- codegraph 结果包含文件/符号结构；context7 返回文档片段；codebase-memory 返回图谱结果；ssh-manager 返回连接列表（无连接时为空列表或明确提示）。
- resources 返回 fixture 的 `echo://doc` 或目标 server 的资源；读取结果为资源正文；图片资源按工具图片规则归档。
- Esc 后卡片状态为 `cancelled`，本轮不再采样；客户端已向 server 发送 `notifications/cancelled`；未开始的同批工具记为 cancelled。
- `tools/list_changed` 只影响后续快照；已冻结的当前轮不变；下一轮读到新工具列表。
- 启动失败/需要登录时，对应项目的 lx 窗格内出现英文 toast，同一错误不会每轮重复刷屏。
- 退出后 `pgrep` 无上述 MCP 子进程 / npx 包装进程；`cargo test --lib mcp::manager::integration_tests` 已覆盖调用、取消、超时、指纹局部重启、`list_changed`、resources 与退出整组终止。

---

## V3. Skills：发现、设置页、`@` 面板与显式引用

**覆盖点**：四个发现根、信任门控、同名优先级、隐藏目录/深度/数量上限、`SKILL.md` 解析、mtime 缓存、目录注入、`@` 面板 skill 候选、`@[refer-skill]` 注入、附件 `skill` kind、历史回放摘要、`.agents` 只读且默认关闭、启停、预算/截断提示。

**输入内容**

用户级 skill：`~/.lx-t-dev/skills/release-checklist/SKILL.md`

```markdown
---
name: release-checklist
description: Steps to cut a release of lx-tui.
---

# Release checklist
1. bump version
2. update changelog
```

项目级 `.agents` skill：`<项目根>/.agents/skills/pdf/SKILL.md`（只读来源）。

另有一个实际用户 skill：`~/.lx-t-dev/skills/grill-me/skill.md`（小写文件名，用于大小写兼容回归）。

**验证步骤**

1. 在设置页 **Skills** tab 新建/编辑/保存 `release-checklist`，再禁用它；观察 `@` 面板。
2. 在输入框输入 `@release`，选中 skill 候选；检查插入的引用文本。
3. 发送消息，检查数据库附件 kind 与下一轮历史回放。
4. 编辑 `SKILL.md`（改变内容长度/描述），不重启 lx，再发一轮，检查目录是否刷新。
5. 在设置页尝试编辑 `.agents` 来源 skill，观察保存/删除按钮；检查 API 只提供 `.agents` 的启停开关（`PUT /api/skills/enabled`），不存在写入其正文的路由。
6. 构造超过 8000 字节的 skill 正文并引用，观察 pane 内 toast。
7. 在设置页打开 `grill-me`（小写 `skill.md`），修改 description 保存，再删除；检查目录里始终只有一个 skill 文件且保留原 casing。

**期望结果**

- 设置页按 `User · lx`、`User · .agents (read-only)`、`Project · lx` 分组；`lx` 来源可增删改，`.agents` 只读（保存/删除禁用）；写接口只落在 lx 自有根，不存在指向 `.agents` 正文的写路由，启停仍可通过 `/api/skills/enabled` 修改。
- `.agents` 来源 skill 默认 `enabled=false`，不进入系统提示目录与 `@` 面板；设置页手动勾选后写入 `config/skills.json` 的 `enabled`，重新扫描后加载。`lx` 来源仍按 `disabled` 列表启停。
- `@` 面板顶栏包含 `Files` / `refDir` / `Skills` / `MCP` 四个 Tab（lx 输入框与右侧 Prompt 编辑器一致），支持 `←`/`→` 左右键循环切换与鼠标点击切换：
  - `Files` Tab 仅展示工作区文件与目录；
  - `Skills` Tab 以 `◆ name` + `Skill · description` 展示已启用 skill，确认后插入 `@[refer-skill](<绝对路径>) `；
  - `MCP` Tab 以 `◆ name` + `MCP · transport` 展示已启用 MCP server，确认后插入 `@[refer-mcp](<server_name>) `；
  - `/` 面板只展示内置命令，不再出现 skill。
- 发送时正文以 `<skill>…</skill>` 只注入当轮；附件 kind 为 `skill`；历史只回放 `[skill <name> was loaded from <path> in this turn]`。
- `SKILL.md` 文件名大小写不敏感：`skill.md` 也能被发现；设置页读取/保存/删除沿用实际文件名，不会额外生成第二个 `SKILL.md`。
- 编辑 `SKILL.md` 后下一轮目录生效（mtime 签名变化，命中缓存时只做 stat）。
- 超过 8000 字节时注入被截断，pane 内提示 `skill "<name>" exceeds 8000 bytes; injection truncated`；读取失败不阻止发送。
- Skills 设置页的正文编辑区高度约 420px，编辑区内部可滚动；小窗口下不会被裁切。
- 自动化覆盖：`cargo test --lib skills::`、`cargo test --lib settings::service::tests::skills_api_lists_creates_disables_and_deletes`、`cargo test --lib skills::discover::tests::catalog_signature_tracks_added_and_edited_skills`。

---

## V4. 工具结果图片：归档、协议编码与跨轮回放

**覆盖点**：MCP 图片/`read` 图片归档、SQLite 只存路径、Anthropic/Responses/Gemini 原生图片、Chat Completions 后补 user 媒体消息、Gemini 1/2 兼容、模型不支持图片降级、最近 3 轮限制、缺图占位。

**输入内容**

- 一个会返回图片的 MCP 工具（或对本地 PNG 调用 `read`）。
- 依次发送 4 轮带工具图片的消息，再删除其中一轮的归档文件；随后切换到不支持图片输入的模型。

**验证步骤**

1. 调用图片工具，检查会话目录与数据库 `result_json`。
2. 逐协议抓取请求体（Anthropic / Responses / Gemini / Chat Completions）。
3. 进入下一轮，检查上一轮图片是否回放；删除归档文件后再进入下一轮。
4. 构造第 4 个含图轮次，检查最早轮次。
5. 切换到不支持图片的模型再发送。

**期望结果**

- 图片文件位于 `~/.lx-t[-dev]/session/<sid>/image/`；`result_json` / SQLite 只有路径与宽高，没有 base64。
- Anthropic/Responses/Gemini 的图片在工具结果原生块内；Chat Completions 在连续 tool 消息之后补一条 `role=user` 的 `Attached media from tool result:`；Gemini 1/2 走同样的后补消息路径。
- 只保留最近 3 个含工具图片的轮次；更早轮次追加 `[image from an earlier tool result omitted; call the tool again if needed]`。
- 归档文件缺失时追加 `[image no longer available]`，不中断回放。
- 不支持图片的模型收不到图片块，文本末尾追加 `[image omitted: this model does not support image input]`。
- 自动化覆盖：`cargo test --lib chat::transport::tools::wire_snapshot`、`cargo test --lib chat::context::tests::tools`、`cargo test --lib ui::main_content::lx::history::tool::mcp`。

---

## V5. 数据库迁移、会话图片目录与 GC

**覆盖点**：v19→v20 附件表重建、`skill` kind、图片目录搬迁、路径改写、旧库升级链、工具图片引用不被 30 天 GC 误删。

**输入内容**

真实开发库副本（验证时 `user_version=19`、附件 2 行）：

```bash
TMP=/path/to/tmp/lx-db-copy
cp ~/.lx-t-dev/db/lx.db "$TMP/lx.db"
cp ~/.lx-t-dev/db/lx.db-wal "$TMP/lx.db-wal" 2>/dev/null || true
cp ~/.lx-t-dev/db/lx.db-shm "$TMP/lx.db-shm" 2>/dev/null || true
```

**验证步骤**

1. 用当前分支代码打开副本（测试入口或 `Database::open`）。
2. 执行 `PRAGMA user_version`、附件行数、`PRAGMA foreign_key_check`、`PRAGMA integrity_check`，并检查附件表 CHECK 约束。
3. 构造旧的 `~/.lx-t-dev/images/<sid>/` 文件，启动后检查是否搬到 `session/<sid>/image/`，以及 `source_path` 是否同步改写。
4. 运行图片 GC，检查仍被 `result_json` 引用的工具图片是否保留。

**期望结果**

- `user_version=20`；附件行数迁移前后不变；`foreign_key_check` 无输出；`integrity_check=ok`；附件表 CHECK 含 `('file', 'folder', 'image', 'skill')`。
- 旧库从 v18/v19 连续升级可达 v20，不跳过迁移。
- 图片搬迁逐文件 rename，失败保留旧路径；重复启动幂等；只删除空旧目录。
- 30 天 GC 不会删除仍被附件或工具 `result_json` 引用的图片。

---

## V6. 提示词、子 agent 与质量回归

**覆盖点**：turn 起点冻结系统提示与工具、内置工具→MCP→resources 排序、主/子 agent 共享 MCP 与 skill 目录、explorer MCP 只读约束、英文模型文本、无图片 wire 回归、整体质量门槛。

**输入内容**

- 已导入的 `codegraph` server + 一个用户级 skill。
- 主会话任务：`让 explorer 用 codegraph_explore 调查 src/mcp/manager.rs 的结构，并说明可以调用哪些只读 MCP 工具`。

**验证步骤**

1. 发送主会话消息，抓取首轮请求的 `tools` 顺序与 system prompt 段顺序。
2. 让主 agent 调 `spawn_agent(role=explorer)`，检查子 agent 的提示与工具列表。
3. 运行全量质量门槛：

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
node --check assets/settings_mcp.js
node --check assets/settings_skills.js
```

**期望结果**

- 同一 turn 多次采样的 `ChatRequest.tools` 与系统提示字节一致；工具顺序为内置 → MCP 按名排序 → resources。
- 主/子 agent 各自在 turn 起点冻结快照，但共享同一项目的 MCP 连接；子 agent 的 skill 目录使用子 agent 版说明。
- explorer 工具面包含 `mcp__codegraph__codegraph_explore` 等已启用 MCP 工具，英文提示明确“只调用只读 MCP 工具，不创建/更新/删除”；explorer 不会调用 `write` / `edit` / `exec_command`。
- 模型可见文本与 TUI/设置页用户可见文案全部为英文；MCP token / `client_secret` 不出现在日志、toast、GET 响应或模型上下文。
- `cargo fmt` 无改动；`clippy` 0 warning；`cargo test` 3891 passed / 0 failed / 2 ignored；四份设置页 JS 通过语法检查。
- 无图片请求的四协议 wire 由 `chat::transport::tools::wire_snapshot::no_image_wire_bytes_are_stable` 逐字节锁定。

---

## 结论

- 自动化验证全部通过；`opencode.json` → `~/.lx-t-dev/config/mcp.json` 的 4 个 stdio server 已实际解析/校验通过，真实开发库副本迁移、MCP stdio 生命周期、OAuth mock、设置页 API、TUI 卡片/进度/取消均有测试或可复现命令覆盖。
- 未自动验证：设置页与 TUI 的实际视觉、鼠标点击区域和真实第三方 OAuth 服务器行为；需按 V1/V2/V3 的“人工确认”步骤运行 `cargo run` / 浏览器复核。
- 本分支已合并入 `dev`，`.worktrees/lx-mcp-skills` 工作区、隔离开发库与 `feat/lx-mcp-skills` 分支已移除。
