# lx 欢迎页 AGENTS.md / MCP 计数重构与内嵌终端 Ghostty 键位对齐 — 执行任务清单

> 状态：**已完成并合入 `dev` 分支，工作区与特性分支已清理**（2026-11-02）
> 设计依据：[design.md](design.md) | 评审记录：[review.md](review.md)
> 阶段划分：P0（开工门槛与基线核对）→ P1（MCP 实时计数重构与 `AGENTS.md` 计数状态接入）→ P2（`lx` 欢迎页 UI 渲染更新）→ P3（内嵌终端 Kitty 协议握手与 Ghostty 键位编码修复）→ P4（定向与全量验证）。

---

## P0. 开工门槛与基线核对

- [x] 用户审核确认 [design.md](design.md) 与本 `task.md` 后开始执行。
- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 复核拟改文件的最新符号与调用链，严格遵守 `AGENTS.md` 与 `docs/standards/*.md` 全部规范（生产代码禁止 `unwrap()` / `expect()`，TUI 文案 100% 英文，单文件 `< 1000` 行）。

---

## P1. MCP 实时计数重构与 `AGENTS.md` 计数状态接入（`src/app/`、`src/event/`）

- [x] **扩展 `StartupCounts` 与 `AppState`（`src/app/state/lx.rs`、`src/app/state.rs`）**：
  - 在 `StartupCounts` 中新增 `pub agents_md: usize` 字段。
  - 在 `AppState` 中新增 `pub agents_md_inventory: BTreeMap<Option<PathBuf>, usize>` 字段（按工作区实际执行目录 `workspace.cwd` 索引，隔离主 checkout、monorepo 子目录与 linked worktree）。
  - 在 `AppState::sync_lx_inventory` 中将 `agents_md_inventory`、`mcp_inventory`、`skill_inventory` 同步到所有窗格的 `pane.lx.startup`。
- [x] **重构 MCP 实时状态聚合与新增 `refresh_agents_md`（`src/event/inventory.rs`、`src/event/mod.rs`、`src/event/chat/mod.rs`）**：
  - 修改 `refresh_mcp(state, persistence, roots, mcp_statuses)`：移除旧的 `current.errors.min(total)` 估算逻辑，清理已关闭工作区的残留 `mcp_inventory`，更新 `total` 后直接调用 `update_mcp_status(state, root, mcp_statuses)`，按 `opencode-dev` 逻辑从当前已启用 server 的实时状态表派生 `ready` 与 `errors`。
  - 修改 `update_mcp_status`：移除 `loading: loading || current.loading` 锁死逻辑，按 `starting > 0 || (total > 0 && settled < total)` 准确反映后台状态。
  - 新增 `agents_md_scopes` 与 `refresh_agents_md(state: &mut AppState, persistence: &Persistence)`：与 `chat::start_stream` 同源复用 `crate::chat::agent::load_instructions(user_root, config_root, workspace_root, trusted).active_count()` 统计各工作区生效（非空）的全局 `~/.lx-t[-dev]` 与项目级 `AGENTS.md` 文件数量并写入 `state.agents_md_inventory`。
  - 在 `src/event/mod.rs` 基于 `agents_md_scopes` 指纹变化与 `skills_reload` 精准触发 `inventory::refresh_agents_md`，并在 `src/event/chat/mod.rs` 的 `start_stream` 中同步最新计数。

---

## P2. `lx` 欢迎页 UI 渲染重构（`src/ui/`）

- [x] **新增文案与样式 token（`src/ui/text.rs`、`src/ui/style.rs`）**：
  - 在 `src/ui/text.rs` 新增 `pub const LX_HOME_AGENTS_MD: &str = "AGENTS.md";`。
  - 在 `src/ui/style.rs` 提取命名常量 `pub const LX_SKY_BLUE: Color = Color::Indexed(117);` 并新增 `pub fn chat_agents_md() -> Style`（复用 `LX_SKY_BLUE`）。
- [x] **重构 `brand::draw_inventory` 与 `inventory_parts`（`src/ui/main_content/lx/brand.rs`）**：
  - 移除 `inventory_parts` 中 `if count.loading` 阻塞渲染 `loading mcp` / `loading skills` 的分支，始终实时渲染 `{count.ready} {label}`，并在 `count.errors > 0` 时追加红色的 `{count.errors} error(s)`。
  - 在 `mcp` 段左侧拼接 `{counts.agents_md} AGENTS.md`（使用 `style::chat_agents_md()`）与分隔符 ` · `（使用 `style::muted()`），使整行呈现为 `{agents_md} AGENTS.md · {mcp} mcp · {skills} skills`；当窄窗格宽度不足以容纳全段时，优先省略左侧 `AGENTS.md` 段以完整保留核心 `mcp · skills`。

---

## P3. 内嵌终端 Kitty 键盘协议握手与 Ghostty 键位编码对齐（`src/terminal/`、`src/input/`）

- [x] **启用 `alacritty_terminal` Kitty 键盘协议支持（`src/terminal/mod.rs`）**：
  - 在 `Terminal::new` 构造 `EmulatorConfig` 时设置 `kitty_keyboard: true`，使内嵌终端能够自动响应子进程（`opencode`、`codex`）的 `CSI ? u` 探测并维护 `TermMode::KITTY_KEYBOARD_PROTOCOL` 模式栈。
- [x] **重构 `input::encode::encode_key` 与修复 `lx` 编辑器 `Ctrl+Shift+Z` 重做（`src/input/encode.rs`、`src/input/mod.rs`）**：
  - 将 `KeyCode::BackTab` 归一化为 `(KeyCode::Tab, modifiers | KeyModifiers::SHIFT)`。
  - 在 `modifier_param` 补齐 `SUPER (+8)`、`HYPER (+16)`、`META (+32)`。
  - 在 `src/input/mod.rs` 的 `lx_input_key` 与 `editor_command` 中修复 Ghostty Kitty 协议下 `KeyCode::Char('z') + CONTROL | SHIFT` 的 `Redo` 映射。
  - 实现 `encode_enter`、`encode_tab`、`encode_backspace`、`encode_esc`、`encode_char`：
    - **Kitty 模式**（`mode.intersects(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC)`）：
      - `Shift+Enter` → `\x1b[13;2u`、`Alt+Enter` → `\x1b[13;3u`、`Ctrl+Enter` → `\x1b[13;5u`、`Esc` → `\x1b[27u`；
      - `Shift+Tab` → `\x1b[9;2u`、`Alt+Backspace` → `\x1b[127;3u`、`Ctrl+Backspace` → `\x1b[127;5u`；
      - 除纯 `Super+C` 外，所有带 `Ctrl` / `Alt` / `Super` 修饰的按键统一编码为 `\x1b[<codepoint>;<mod>u`（含 `Super+Z` → `\x1b[122;9u`、`Super+Shift+Z` → `\x1b[122;10u`、`Ctrl+Z` → `\x1b[122;5u`、`Ctrl+-` → `\x1b[45;5u`、`Ctrl+.` → `\x1b[46;5u`）；
      - 无控制修饰的可打印字符（含 `Shift` 产出的大写与符号）保持输出 UTF-8 字节。
    - **Legacy 模式**（未开启 Kitty 协议，对齐 Ghostty `function_keys.zig`）：
      - `Shift+Enter` → `\x1b[27;2;13~`、`Ctrl+Enter` → `\x1b[27;5;13~`、`Alt+Enter` → `\x1b\r`；
      - `Shift+Tab` → `\x1b[Z`、`Alt+Backspace` → `\x1b\x7f`、`Ctrl+Backspace` → `\x08`；
      - `Ctrl+Shift+<char>` 及无传统 C0 映射的 ASCII 符号（如 `Ctrl+.`）→ `\x1b[<lower_cp>;<mod>u`。

---

## P4. 单元测试更新与改后精确校验

- [x] **补充与更新单元测试**：
  - `src/ui/main_content/lx/tests.rs`：更新 `home_shows_mcp_and_skill_counts`，验证 `{agents_md} AGENTS.md · {ready} mcp · {ready} skills`、后台连接中实时显示 `{ready} mcp`（不再被 `loading mcp` 遮挡）、以及失败追加 `{errors} error(s)`。
  - `src/app/state/tests.rs` & `src/event/inventory/tests.rs`：验证 `sync_lx_inventory` 同步 `agents_md`、MCP 单个服务 `Ready` 实时递增、禁用失败服务立即清除 `errors` 计数、以及授予信任后生效 `AGENTS.md` 计数更新。
  - `src/terminal/tests.rs`：验证 `Terminal::feed(b"\x1b[?u")` 返回 `b"\x1b[?0u"`，以及 `feed(b"\x1b[>5u")` 置位 `TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALTERNATE_KEYS`、退出备用屏或 `feed(b"\x1b[<u")` 后恢复。
  - `src/input/encode/tests.rs`：覆盖 Legacy 与 Kitty `TermMode` 下 `Shift+Enter`、`Alt+Enter`、`Ctrl+Enter`、`Ctrl+J`、`Shift+Tab`（含 `Tab+SHIFT` 与 `BackTab`）、`Alt+Backspace`、`Ctrl+Backspace`、`Ctrl+Shift+<char>`、以及 Shift 符号字符（`!@#`）的精确字节编码测试。
- [x] **执行定向校验命令（优先使用 `rtk`）**：
  - `rtk cargo test ui::main_content::lx::`
  - `rtk cargo test app::state::`
  - `rtk cargo test event::inventory::`
  - `rtk cargo test terminal::`
  - `rtk cargo test input::encode::`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - `rtk cargo fmt --check`
