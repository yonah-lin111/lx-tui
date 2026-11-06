# lx `AI Web` 入口与 DeepSeek 网页端接入 — 执行任务清单

> 状态：**已在 `.worktrees/lx-ai-web`（`feature/lx-ai-web`）完成实现与测试，待用户确认合并**（2026-11-06）
> 设计依据：[design.md](design.md)
> 阶段划分：P0（开工门槛与基线核对）→ P1（`app/` 状态模型与纯内存会话更新）→ P2（`ui/` 欢迎页 4 入口布局、站点行与底边框渲染）→ P3（`chat/ai_web` 与 `event/` 的 `ego-browser` DeepSeek 流式驱动）→ P4（单元测试与精确校验）。

---

## P0. 开工门槛与基线核对

- [x] 用户审核确认 [design.md](design.md) 与本 `task.md` 后开始执行。
- [x] 遵守 `AGENTS.md` 与 `docs/standards/*.md` 全部规范：生产代码禁止 `unwrap()` / `expect()`，TUI 用户可见文案 100% 英文并集中在 `src/ui/text.rs`，保持 `app/` 与 `ui/` 纯函数边界，单文件 `< 1000` 行。

---

## P1. 状态模型与纯内存会话更新（`src/app/`）

- [x] **扩展 `LxEntry` 与 `AiWebSite`（`src/app/state/lx.rs`、`src/app/effects.rs`）**：
  - 在 `LxEntry` 中新增 `AiWeb` 变体，更新 `LxEntry::ALL` 为 4 项 `[Native, Hybrid, Automation, AiWeb]`。
  - 新增 `AiWebSite` 枚举（首期 `Deepseek`）并在 `Lx` 结构体中新增 `pub ai_web_site: AiWebSite`。
  - 在 `Effect` 中新增 `StartAiWebStream { key: RequestKey, site: AiWebSite, prompt: String }` 与 `ResetAiWebSession { site: AiWebSite }`。
- [x] **实现 `AI Web` 发送、状态栏刷新与 `/new` 重置（`src/app/update/chat/ai_web.rs`、`mod.rs`、`session.rs`、`slash.rs`、`pane.rs`）**：
  - 新增 `src/app/update/chat/ai_web.rs`：实现 `try_send_ai_web`（直接构造内存态 user + streaming assistant 消息并入队 `Effect::StartAiWebStream`）。
  - 在 `update::pane::set_lx_entry` 与新增的 `set_ai_web_site` 中调用 `refresh_lx_status`。
  - 在 `refresh_lx_status` 中为 `LxEntry::AiWeb` 设置底边框状态行 `model_name = Some("DeepSeek")`，隐藏 `variant` 与 Token 占用。
  - 在 `on_stream_finished` 中识别 `ai-web-` 内存会话，结算消息终态后直接将 `request` 置为 `RequestState::Idle`（不写 SQLite）。
  - 在 `SlashPanel::command_entries_for` 与 `apply_slash_command` 中按 `LxEntry::AiWeb` 裁剪斜杠命令（仅保留 `/new`，其余提示 `Only /new is available in AI Web`），并在 `new_session` 时入队 `Effect::ResetAiWebSession`。

---

## P2. 欢迎页 4 入口布局、站点选择行与底边框渲染（`src/ui/`）

- [x] **新增文案与配色（`src/ui/text.rs`、`src/ui/style.rs`）**：
  - 在 `src/ui/text.rs` 新增 `LX_ENTRY_AI_WEB` (`"AI Web"`)、`LX_ENTRY_AI_WEB_DESC`、`AI_WEB_SITE_DEEPSEEK` (`"DeepSeek"`)、`ai_web_site_label` 及错误提示常量。
  - 在 `src/ui/style.rs` 的 `lx_entry_gradient` 中补齐 `LxEntry::AiWeb => (LX_SKY_BLUE, LX_PINK)`。
- [x] **扩展欢迎页品牌区与动态入口几何（`src/ui/main_content/lx/brand.rs`、`src/ui/main_content/lx.rs`）**：
  - 在 `brand::icon_pixel` 中补齐 `LxEntry::AiWeb` 的调色板映射。
  - 在 `brand.rs` 新增 `draw_ai_web_sites` 与 `place_ai_web_sites`：当 `lx.entry == LxEntry::AiWeb` 时，在 `lay.inventory` 行居中绘制站点选择标签（`DeepSeek`）及选中下划线。
  - 在 `src/ui/main_content/lx.rs` 修复 `layout` 与 `place_entries` 的多入口间距/高度动态计算，并导出 `ai_web_site_hit` 命中函数。

---

## P3. `ego-browser` DeepSeek 流式驱动与事件循环集成（`src/chat/ai_web/`、`src/event/`）

- [x] **实现 `chat::ai_web` 模块与嵌入式脚本（`src/chat/ai_web/mod.rs`、`src/chat/ai_web/deepseek_script.js`）**：
  - 编写 `deepseek_script.js`：通过 `useOrCreateTaskSpace('lx-tui-ai-web-deepseek')` 打开/复用 `https://chat.deepseek.com`；检测未登录（无 `userToken` 或 `/sign_in`）时跳转 `https://chat.deepseek.com/sign_in` 并调用 `handOffTaskSpace()`，输出 `login_required` 事件；已登录时注入 SSE/DOM 双通道流采集器，填入 prompt 发送并实时输出 `@@LX_AI_WEB@@` 前缀的 NDJSON 行（`thinking` / `text` / `done` / `error`）。
  - 在 `src/chat/ai_web/mod.rs` 实现 NDJSON 行解析器 `parse_script_line`（映射为 `ChatDelta` / 登录提示 / 终态）及 `ego-browser` 可执行文件定位函数。
- [x] **接入事件层流管理与鼠标点击（`src/event/chat/mod.rs`、`src/event/chat/ai_web.rs`、`src/event/mouse/press.rs`）**：
  - 在 `src/event/mouse/press.rs` 中接入 `ai_web_site_hit` 左键点击切换。
  - 在 `src/event/chat/mod.rs` 与 `src/event/chat/ai_web.rs` 中处理 `Effect::StartAiWebStream` 与 `Effect::ResetAiWebSession`：异步拉起 `ego-browser nodejs`、按行读取 stdout 转换投递 `ChatNetEvent::Delta` / `Finished`、在检测到 `login_required` 时弹出窗格内 Toast，并支持通过 `streams.cancels` 取消子进程。

---

## P4. 单元测试与改后精确校验

- [x] **补充与更新单元测试**：
  - `src/ui/main_content/lx/tests.rs`：更新 4 入口布局测试，新增 `AiWeb` 入口渲染 `DeepSeek` 站点行、隐藏 `AGENTS.md · mcp · skills` 及 `ai_web_site_hit` 命中测试。
  - `src/chat/ai_web/mod.rs`（内联或子测试模块）：验证 `@@LX_AI_WEB@@` NDJSON 行协议解析（`thinking`、`text`、`login_required`、`done`、`error` 与非协议噪声行过滤）。
  - `src/app/update/chat/ai_web.rs`：验证 `LxEntry::AiWeb` 发送进入 `Streaming`、状态栏显示 `DeepSeek`、终态不触发 DB 写入效应，以及 `/new` 重置行为。
- [x] **执行精确校验命令（优先使用 `rtk`）**：
  - `rtk cargo fmt --check`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - `rtk cargo test ui::main_content::lx::`
  - `rtk cargo test chat::ai_web::`
  - `rtk cargo test app::update::`
