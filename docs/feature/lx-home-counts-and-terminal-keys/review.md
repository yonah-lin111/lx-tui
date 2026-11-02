# 功能代码评审报告（Review）：`lx` 首页 `AGENTS.md` / `MCP` 计数重构与终端快捷键修复

## 1. 评审范围与概览

- **评审工作区**：`.worktrees/lx-home-counts-and-terminal-keys`（分支 `feat/lx-home-counts-and-terminal-keys`）
- **对标参考**：
  - `/Users/yonah/projects/agent/opencode-dev`（首页 MCP 实时状态与快捷键绑定）
  - `/Users/yonah/projects/agent/openclaude-main`（Ink 终端按键解析）
  - `/Users/yonah/projects/agent/herdr-master`（Ghostty / Kitty `CSI u` 键盘编码）
  - 项目规范 `docs/standards/{project-directory-structure,code-writing-standards,tui-design-requirements}.md`

### 问题优先级汇总表（P0 -> P3）

| 编号 | 优先级 | 涉及文件与行号 | 关键点 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0-1** | **P0** | `src/input/encode.rs:10-25, 255-330` | Kitty 协议下 `Super+Z` 被无条件丢弃、`Ctrl+-` / `Ctrl+Z` 未统一编码为 `CSI u`，导致 `opencode` 在 `lx` 终端中无法使用 `Ctrl+Z` / `Super+Z` 撤销 | 已修复 |
| **P0-2** | **P0** | `src/event/inventory.rs:427-442`<br>`src/app/state.rs:285-290, 448-480` | `AGENTS.md` 首页计数作用域与真实对话注入不同源（误将 `repo_key` 同时作为 `project_root` 和 `cwd`），导致 linked worktree 与 monorepo 子目录计数错误 | 已修复 |
| **P1-1** | **P1** | `src/event/mod.rs:274-293, 352-373`<br>`src/event/inventory.rs`<br>`src/event/chat/mod.rs:592-605` | `AGENTS.md` 刷新触发条件仅依赖去重后的 `skill_roots` 与粗粒度 `persistence.pump`，同仓库打开 worktree 或 `GrantTrust` 可能漏刷，而在无关 DB 操作上又重复同步读盘 | 已修复 |
| **P1-2** | **P1** | `src/input/mod.rs:207-208, 456-457` | Ghostty Kitty 协议下 `Ctrl+Shift+Z` 发送小写 `'z' + CONTROL\|SHIFT`，在 `lx` 自身编辑器（Prompt 与 lx 输入框）中被误判为 `Undo` 而非 `Redo` | 已修复 |
| **P1-3** | **P1** | `src/event/inventory.rs:172-203, 249-314` | `update_mcp_status` 未按当前启用的 server 集合过滤，且 `initialize_mcp` 在重新打开已关闭工作区时跳过状态刷新，保留陈旧计数 | 已修复 |
| **P2-1** | **P2** | `src/input/encode.rs:237-253` | Kitty 协议下无修饰 `Esc` 仍发送裸 `0x1b`，未对齐 Ghostty 发送 `\x1b[27u` 消除 `Alt+<key>` 前缀歧义 | 已修复 |
| **P2-2** | **P2** | `src/input/encode.rs:280-315` | `control_byte` 缺少 `'~'` / `` '`' `` 映射，且兜底分支 `other as u8` 将非 C0 控制字符（如 Legacy 下 `Ctrl+.` 或非 ASCII）截断为普通字符写入 PTY | 已修复 |
| **P2-3** | **P2** | `src/input/encode.rs:11` | `encode_key` 的 `kitty` 判定使用了整个 `TermMode::KITTY_KEYBOARD_PROTOCOL`，未精确限定为 `DISAMBIGUATE_ESC_CODES \| REPORT_ALL_KEYS_AS_ESC` | 已修复 |
| **P2-4** | **P2** | `src/ui/main_content/lx/brand.rs:139-180` | 窄窗格下 `draw_inventory` 从左向右硬截断，优先保留了 `AGENTS.md` 却把右侧核心的 `mcp` / `skills` 裁掉，缺少窄宽自适应降级 | 已修复 |
| **P3-1** | **P3** | `src/ui/style.rs:911-914, 988-1018`<br>`docs/standards/tui-design-requirements.md:148`<br>`docs/feature/lx-home-counts-and-terminal-keys/{design,task}.md` | `chat_agents_md` 硬编码 `Color::Indexed(117)` 未复用品牌色常量；设计文档、任务清单及部分注释与最终实现不一致 | 已修复 |
| **P3-2** | **P3** | `src/event/inventory/tests.rs`<br>`src/ui/main_content/lx/tests.rs`<br>`src/input/encode/tests.rs`<br>`src/input/tests.rs` | 单元测试缺少 linked worktree / 子目录独立 `AGENTS.md` 计数、MCP 禁用状态过滤、窄窗格降级渲染、`Esc` Kitty 编码与 `Ctrl+Shift+Z` 重做覆盖 | 已修复 |

---

## 2. 逐项详细分析（关键点、错误原因与修复方案）

### P0-1：Kitty 键盘协议下 `Super+Z` 被丢弃、`Ctrl+-` / `Ctrl+Z` 未统一走 `CSI u`，导致 `opencode` 无法撤销

- **关键点**：
  - `src/input/encode.rs` 中的 `encode_key`、`encode_char`、`modifier_param`。
  - `opencode` 在 Ghostty 与 `lx` 内嵌终端中的撤销/重做快捷键编码一致性。
- **错误原因**：
  1. `opencode` 的输入框撤销默认绑定为 `"input.undo": "ctrl+-,super+z"`（当禁用 `terminal.suspend` 时包含 `ctrl+z`）；而在用户的 macOS 键盘修饰键映射中，物理 `Ctrl` 映射到了 `Command (Super)`，因此在 Ghostty 中按下物理 `Ctrl+Z` 实际产生 `Super+Z`（`\x1b[122;9u`）。
  2. 初版 `encode_key` 顶部写死了 `if key.modifiers.contains(KeyModifiers::SUPER) { return None; }`，且 `modifier_param` 仅计算了 `SHIFT (+1)`、`ALT (+2)`、`CONTROL (+4)`，缺少 `SUPER (+8)`，导致 `Super+Z`（撤销）、`Super+Shift+Z`（重做）、`Super+A`（全选）及 `Super+方向键` 全部被 `lx` 拦截丢弃。
  3. 初版 `encode_char` 在 Kitty `DISAMBIGUATE_ESC_CODES` 下仅对 `'i' | 'm' | '['` 走 `CSI u`，违反 Kitty 键盘协议规范与 Ghostty `key_encode.zig` 行为（开启 `DISAMBIGUATE_ESC_CODES` 时，所有带 `Ctrl` / `Alt` / `Super` 修饰的 ASCII 字符键均须发 `CSI <codepoint>;<mod> u`），导致 `Ctrl+-` 落入 `control_byte('-')` 退化为字面量 `'-'`（`0x2d`），`Ctrl+Z` 未编码为 `\x1b[122;5u`。
- **修复方案**：
  1. 在 `modifier_param` 中补齐 `SUPER (+8)`、`HYPER (+16)`、`META (+32)`；
  2. 在 `encode_key` 中，当子终端开启 Kitty 转义编码时，除纯 `Super+C`（无选区时不透传以免在 `claude` 中误输入 `'c'`）外，放行其余 `Super` 组合键；
  3. 在 `encode_char` 中，当开启 Kitty 转义编码或处于 `Ctrl+Shift+<char>` 时，所有带 `Ctrl` / `Alt` / `Super` 的按键统一编码为 `CSI u`（如 `Super+Z -> \x1b[122;9u`、`Ctrl+Z -> \x1b[122;5u`、`Ctrl+- -> \x1b[45;5u`）。

---

### P0-2：`AGENTS.md` 首页计数作用域与真实对话注入不同源，导致 linked worktree 与 monorepo 子目录计数错误

- **关键点**：
  - `src/event/inventory.rs` 的 `refresh_agents_md`；
  - `src/app/state.rs` 的 `agents_md_inventory` 与 `sync_lx_inventory`；
  - `src/event/chat/mod.rs:573-603` 的 `start_stream` 真实指令加载路径。
- **错误原因**：
  1. 在真实对话轮次 `start_stream` 中，`config_root` 取 `state.chat_session_key_for_pane(pane)`（即 `workspace.repo_key()`，指向主 checkout），`workspace_root` 取 `state.workspace_cwd_for_pane(pane)`（即 `workspace.cwd`，指向实际执行目录或 linked worktree），信任判定为 `is_trusted(cwd) || is_trusted(config_root)`，并调用 `load_instructions(user_root, config_root, workspace_root, workspace_trusted)`。
  2. 初版 `refresh_agents_md` 直接遍历去重后的 `skill_roots(state)`（仅包含 `workspace.repo_key()`），将同一个 `root` 同时作为 `project_root` 和 `cwd` 传入 `load_instructions(user_root, root, root, is_trusted(root))`，且 `AppState.agents_md_inventory` 以 `repo_key` 为键：
     - **Linked worktree 错误**：linked worktree 的 `repo_key()` 指向主仓库 checkout，而 `load_instructions` 的 `collect_search_dirs` 在真实对话中遇到 worktree 自身的 `.git` 文件即停止向上攀爬，只加载 worktree 自身的 `AGENTS.md`；初版首页却错误统计了主仓库的 `AGENTS.md`，且同一仓库的主 checkout 与多个 worktree 共享并互相覆盖同一个 `repo_key` 槽位；
     - **Monorepo 子目录错误**：当 `workspace.cwd` 位于仓库子目录（如 `repo/packages/pkg-a`）时，真实对话会扫描 `repo -> packages -> pkg-a` 整条目录链，而初版首页只扫描了 `repo` 根目录；
     - **信任判定偏差**：`persistence.is_trusted` 是精确路径匹配，初版只检查了 `is_trusted(repo_key)`，漏掉了 `is_trusted(cwd)`。
- **修复方案**：
  1. `AppState.agents_md_inventory` 改为按工作区实际执行目录 `workspace.cwd`（`Option<PathBuf>`）索引；`sync_lx_inventory` 对每个 `workspace` 按 `&workspace.cwd` 查找 `agents_md_inventory`；
  2. `refresh_agents_md` 遍历所有打开的工作区，分别提取 `config_root = workspace.repo_key().or(workspace.cwd.as_deref())` 与 `workspace_root = workspace.cwd.as_deref()`，信任判定对齐 `start_stream`：`workspace_root.is_some_and(|p| persistence.is_trusted(p)) || config_root.is_some_and(|p| persistence.is_trusted(p))`，调用 `load_instructions(user_root, config_root, workspace_root, trusted).active_count()`。

---

### P1-1：`AGENTS.md` 刷新触发条件不精确（同仓库打开 worktree / 授予信任漏刷，无关 DB 效应重复读盘）

- **关键点**：
  - `src/event/mod.rs` 事件主循环中的刷新触发条件；
  - `src/event/inventory.rs` 的作用域指纹检测；
  - `src/event/chat/mod.rs` 发送消息时的计数同步。
- **错误原因**：
  1. `src/event/mod.rs` 仅监测 `skill_roots(state)`（去重后的 `repo_key` 列表）变化；当用户在已打开主仓库的情况下新开一个同仓库的 linked worktree 时，`repo_key` 集合不变，若此时右侧 Prompt 被 pin 在原工作区（`prompt_workspace` 也不变），则 `skills_reload` 不会置位，新打开的 worktree 首页 `AGENTS.md` 永远停留在 `0`；
  2. `GrantTrust` 持久化效应在 `src/db/service.rs:237-240` 中执行后返回 `false`，当 Prompt 树早已加载完毕时，`persistence.pump(state)` 返回 `false`，导致授予信任后无法触发 `if persistence.pump(state) { inventory::refresh_agents_md(...) }`；
  3. 反之，当用户编辑 Prompt 或 Todo 触发 `persistence.pump(state) == true` 时，主循环又会在无任何工作区/信任变化的情况下同步重读磁盘上的 `AGENTS.md`。
- **修复方案**：
  1. 在 `src/event/inventory.rs` 新增 `agents_md_scopes(state, persistence) -> Vec<(Option<PathBuf>, Option<PathBuf>, bool)>`，采集每个打开工作区的 `(repo_key, cwd, trusted)` 状态指纹；
  2. 在 `src/event/mod.rs` 中维护 `last_agents_md_scopes`，每轮在 `persistence.pump(state)` 之后比较当前 `agents_md_scopes` 与 `last_agents_md_scopes`（或 `skills_reload` 置位时），一旦工作区集合、`cwd`、`repo_root` 或信任状态发生任何变化立即刷新 `refresh_agents_md`，并移除 `persistence.pump(state)` 分支下的盲目刷新；
  3. 在 `src/event/chat/mod.rs` 的 `start_stream` 中将 `loaded_instructions.active_count()` 同步写回 `state.agents_md_inventory`（以 `workspace_root` 为键），确保外部新增/删除 `AGENTS.md` 后在对话开始时自动同步。

---

### P1-2：Ghostty Kitty 协议下 `Ctrl+Shift+Z` 在 `lx` 编辑器中被误映射为 `Undo` 而非 `Redo`

- **关键点**：
  - `src/input/mod.rs` 的 `lx_input_key`（第 207–208 行）与 `editor_command`（第 456–457 行）。
- **错误原因**：
  1. `lx-tui` 宿主启动时启用了 `DISAMBIGUATE_ESCAPE_CODES`，Ghostty 按 Kitty 规范对 `Ctrl+Shift+Z` 发送 `\x1b[122;6u`（小写码点 `122 = 'z'` + `CONTROL | SHIFT` 修饰位），`crossterm` 将其解析为 `KeyEvent { code: KeyCode::Char('z'), modifiers: CONTROL | SHIFT }`；
  2. `lx_input_key` 与 `editor_command` 在 `(true, false, KeyCode::Char(ch))` 分支中写成：
     ```rust
     'z' | '-' => Some(...::Undo),
     'Z' | 'y' | 'Y' | '.' => Some(...::Redo),
     ```
     未像同文件 `todo_key` / `overlay_key` 那样先判断 `if shift`，导致小写 `'z'` 带 `SHIFT` 时总是命中第一支 `Undo`，使 `Ctrl+Shift+Z` 重做失效。
- **修复方案**：
  - 在 `lx_input_key` 与 `editor_command` 中将 `'z' | 'Z' if shift => Some(...::Redo)` 放在 `'z' | '-' => Some(...::Undo)` 之前，并补充针对 `KeyCode::Char('z') + CONTROL | SHIFT` 的单元测试。

---

### P1-3：MCP 状态聚合未按当前启用的 server 集合过滤，且重新打开工作区时保留陈旧计数

- **关键点**：
  - `src/event/inventory.rs` 的 `initialize_mcp`、`refresh_mcp`、`update_mcp_status`。
- **错误原因**：
  1. `update_mcp_status` 遍历 `statuses` 中所有属于该 `project` 的记录，并用 `let total = current.total.max(tracked);` 更新总数。若某个 server 在连接途中被禁用/删除，其残留状态或迟到的 `McpEvent::Status` 会把已禁用的 server 算进 `ready` 或 `errors`，并把 `total` 重新撑大；
  2. `initialize_mcp` 在 `state.mcp_inventory.contains_key(root)` 为真时直接 `continue`；当工作区关闭后修改了 MCP 配置再重新打开该工作区时，`mcp_inventory` 仍停留在该工作区关闭前的旧计数。
- **修复方案**：
  1. 在 `initialize_mcp` 中若 `state.mcp_inventory.contains_key(root)` 已存在，仍更新 `total = mentions.len()`（不重置已就绪计数，但修正 `total` 与 `loading`）；
  2. 在 `refresh_mcp` 中更新 `total = mentions.len()` 后，调用 `update_mcp_status` 时以当前 `current.total` 为准（仅当 `current.total == 0 && tracked > 0` 的未初始化场景才回退 `tracked`），且 `prune_mcp_statuses` 与 `refresh_mcp` 严格保证只统计当前启用的 server。

---

### P2-1：Kitty 协议下无修饰 `Esc` 仍发送裸 `0x1b`，未对齐 Ghostty 发送 `\x1b[27u`

- **关键点**：
  - `src/input/encode.rs` 的 `encode_esc`（第 237–253 行）。
- **错误原因**：
  - `encode_esc` 在 `kitty == true && !report_all && modifiers.is_empty()` 时返回了 `vec![0x1b]`。然而 Kitty `DISAMBIGUATE_ESC_CODES` 的核心设计正是将独立 `Esc` 键编码为 `\x1b[27u`，从而让 `opencode` / `codex` 等 TUI 无需等待 20ms 歧义超时即可立即区分单独按下的 `Esc` 与 `Alt+<key>`（`ESC <char>`）。Ghostty 在 `DISAMBIGUATE_ESC_CODES` 下对无修饰 `Esc` 发送 `\x1b[27u`。
- **修复方案**：
  - 在 `encode_esc` 中，当 `kitty || report_all` 为真时，统一返回 `csi_u(27, modifiers)`（无修饰即 `\x1b[27u`）。

---

### P2-2：`control_byte` 缺少 `'~'` / `` '`' `` 映射，且兜底分支将非 C0 控制字符截断为普通字节写入 PTY

- **关键点**：
  - `src/input/encode.rs` 的 `encode_char` 与 `control_byte`。
- **错误原因**：
  1. `control_byte` 未包含 `` '`' => 0x00 `` 与 `'~' => 0x1e`；
  2. `control_byte` 末尾兜底 `other => other as u8` 导致在 Legacy 模式下按下没有传统 C0 映射的组合键（如 `Ctrl+.`、`Ctrl+,` 或 `Ctrl+非ASCII`）时，直接输出字面量 `.` / `,` 或截断后的乱码字节；而在 Ghostty 中，无法映射为 C0 的 ASCII `Ctrl+<标点>` 会回退发送 `CSI u`（如 `Ctrl+. -> \x1b[46;5u`）。
- **修复方案**：
  - 将 `control_byte(c: char) -> Option<u8>` 改为返回 `Option<u8>`，补齐 `` '`' => Some(0x00) `` 与 `'~' => Some(0x1e)`；
  - 在 `encode_char` 的 Legacy `has_ctrl` 分支中：若 `control_byte(c)` 返回 `Some(b)` 则写入 `b`；若返回 `None` 且 `c.is_ascii()`，则回退返回 `csi_u(base_codepoint, modifiers)`（使 Legacy 模式下 `Ctrl+.` 也能正确发出 `\x1b[46;5u`）；非 ASCII 则不产生乱码字节。

---

### P2-3：`encode_key` 的 `kitty` 判定未精确限定为 `DISAMBIGUATE_ESC_CODES | REPORT_ALL_KEYS_AS_ESC`

- **关键点**：
  - `src/input/encode.rs:11`。
- **错误原因**：
  - `mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)` 包含了 `REPORT_EVENT_TYPES` 等辅助标志位；按 Kitty 键盘协议规范，只有开启 `DISAMBIGUATE_ESC_CODES`（位 1）或 `REPORT_ALL_KEYS_AS_ESC`（位 8）时才启用 `CSI u` 消歧编码。
- **修复方案**：
  - 改为 `let kitty = mode.intersects(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC);`。

---

### P2-4：窄窗格下 `draw_inventory` 缺少自适应降级，优先保留了 `AGENTS.md` 而裁掉了 `mcp` / `skills`

- **关键点**：
  - `src/ui/main_content/lx/brand.rs` 的 `draw_inventory`（第 139–180 行）。
- **错误原因**：
  - 当窗格较窄（例如 34 列分屏，完整行 `1 AGENTS.md · 4 mcp 1 error · 3 skills` 宽 38 列 > 34 列）时，`draw_inventory` 直接从左起逐段截断，导致右侧最关键的 `mcp` 错误状态与 `skills` 计数被截断丢弃。
- **修复方案**：
  - 参考 `opencode` 首页窄宽降级策略：当包含 `{agents_md} AGENTS.md · ` 的完整段总宽度超过 `rect.width` 时，自动省略左侧 `{agents_md} AGENTS.md · ` 段，仅渲染 `{mcp} mcp (errors) · {skills} skills` 并居中；若仍超宽再从左起逐段截断。

---

### P3-1：样式常量复用与文档/注释同步

- **关键点**：
  - `src/ui/style.rs`、`src/input/encode.rs`、`docs/standards/tui-design-requirements.md`、`docs/feature/lx-home-counts-and-terminal-keys/{design,task}.md`。
- **错误原因**：
  1. `src/ui/style.rs` 中 `chat_agents_md()` 硬编码了 `Color::Indexed(117)`，未与 `logo_pixel` / `lx_entry_gradient` 共用命名常量；
  2. `src/input/encode.rs` 顶部模块注释仍写「`Super/Cmd` 组合绝不写入 PTY」，与 Kitty 模式转发 `Super+Z` 等快捷键不一致；
  3. `docs/standards/tui-design-requirements.md:148` 中「不阻塞显示 `loading mcp`」表述有歧义；
  4. `design.md` 与 `task.md` 中部分章节尚未同步 P0–P2 的最新实现。
- **修复方案**：
  - 在 `src/ui/style.rs` 定义 `pub const LX_SKY_BLUE: Color = Color::Indexed(117);` 并在三处复用；
  - 更新 `src/input/encode.rs`、`docs/standards/tui-design-requirements.md`、`design.md` 与 `task.md`。

---

### P3-2：补齐边界场景单元测试

- **关键点**：
  - `src/event/inventory/tests.rs`、`src/ui/main_content/lx/tests.rs`、`src/input/encode/tests.rs`、`src/input/tests.rs`。
- **修复方案**：
  1. `src/event/inventory/tests.rs`：新增同仓库下主 checkout 与 linked worktree / 子目录拥有独立 `AGENTS.md` 计数的测试，以及禁用 MCP server 后 `refresh_mcp` 清理残留状态的测试；
  2. `src/ui/main_content/lx/tests.rs`：新增窄宽度下自动省略 `AGENTS.md` 段以完整保留 `mcp · skills` 的渲染测试；
  3. `src/input/encode/tests.rs`：更新并验证 Kitty 模式下无修饰 `Esc -> \x1b[27u`、Legacy 下 `Ctrl+. -> \x1b[46;5u`、`Ctrl+~ -> 0x1e`；
  4. `src/input/tests.rs`：新增 Ghostty Kitty 形态 `KeyCode::Char('z') + CONTROL | SHIFT` 触发 `EditorCommand::Redo` 与 `LxInput::Redo` 的回归测试。
