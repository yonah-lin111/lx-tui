# lx 欢迎页 AGENTS.md / MCP 计数重构与内嵌终端 Ghostty 键位对齐 — 设计文档

> 状态：**已在 `.worktrees/lx-home-counts-and-terminal-keys` 实现并通过验证**（2026-11-02）
> 参考实现：
> - `/Users/yonah/projects/agent/opencode-dev`（`packages/core/src/mcp/index.ts`、`packages/tui/src/feature-plugins/home/footer.tsx`、`packages/tui/src/feature-plugins/sidebar/mcp.tsx`、`packages/core/src/config/plugin/instruction.ts`）
> - `/Users/yonah/projects/agent/codex-main`（`codex-rs/tui/src/terminal_probe.rs`、`codex-rs/tui/src/tui/keyboard_modes.rs`、`codex-rs/tui/src/keymap.rs`）
> - `/Users/yonah/projects/agent/openclaude-main`（`src/ink/parse-keypress.ts`、`src/ink/terminal.ts`、`src/hooks/useTextInput.ts`）
> - `/Users/yonah/projects/agent/herdr-master`（`src/input/encode.rs`）
> - Ghostty 终端源码规范（`src/input/key_encode.zig`、`src/input/function_keys.zig`、`src/input/kitty.zig`）
> 执行任务清单见 [task.md](task.md)。

---

## 0. 现状核实（写设计前已逐项确认）

| 事实 | 现有代码位置 | 对本设计的影响 |
|---|---|---|
| `McpManager::snapshot_for` 已支持按每个 server 独立 `tokio::spawn` 并发连接，并在每个 server 结算时立即通过 `finish_start` 发送 `McpEvent::Status { project, server, status }` | `src/mcp/manager.rs:259-304, 371-425` | 底层连接已经是逐个异步结算的，之所以 UI 没有实时跳数字，根因在事件层与渲染层的 `loading` 阻塞门控 |
| `initialize_mcp` 将 `StartupCount.loading` 置为 `total > 0`，`update_mcp_status` 写死 `loading: loading \|\| current.loading`，直到全部 server 完成触发 `StartupProgress` 才解除 `loading` | `src/event/inventory.rs:191-200, 282-329` | 任一慢连接或超时连接会把整段压在 `loading mcp` 长达 30s，遮挡已就绪 server 的真实数量 |
| `refresh_mcp` 在配置变更时采用 `current.errors.min(total)` 估算，未从 `mcp_statuses` 实时派生 | `src/event/inventory.rs:250-278` | 当用户在设置页禁用或删除一个原先 `failed` 的 server 时，旧 `errors` 计数会残留；必须改为像 `opencode-dev` 一样直接从当前生效 server 的状态表实时聚合 |
| `brand::draw_inventory` 目前仅渲染 `{mcp} · {skills}`，且当 `count.loading == true` 时短路返回 `loading mcp` / `loading skills` | `src/ui/main_content/lx/brand.rs:138-194` | 需移除阻塞式 `loading` 分支，直接显示实时就绪数 `{ready} mcp` 与异常数 `{errors} error(s)`，并在 `mcp` 左侧新增 `{agents_md} AGENTS.md` |
| `AGENTS.md` 发现逻辑已在 `chat::agent::load_instructions` 中实现（全局 `~/.lx-t[-dev]` 首个非空候选文件 + 已信任工作区从 `project_root` 到 `cwd` 的非空候选文件链），但只在聊天 turn 起点调用，未接入欢迎页 `StartupCounts` | `src/chat/agent/instructions.rs:31-152`、`src/event/chat/mod.rs:592-604` | 需在 `instructions.rs` 提供轻量计数/发现复用，由事件层按工作区汇总生效文件数并经 `AppState::sync_lx_inventory` 同步到 `pane.lx.startup.agents_md` |
| `alacritty_terminal 0.26.0` 已完整实现 Kitty 键盘协议（`CSI ? u` 响应 `\x1b[?{mode}u`、`CSI > flags u` 压栈、`CSI < u` 出栈、备用屏自动切换模式栈），但 `EmulatorConfig::default()` 中 `kitty_keyboard` 默认为 `false` | `src/terminal/mod.rs:111-118`、`alacritty_terminal-0.26.0/src/term/mod.rs:350, 1275-1330` | 子进程（`opencode`、`codex`）启动时发送的 `\x1b[?u` 和 `\x1b[>5u` 被 `alacritty_terminal` 直接静默丢弃；必须将 `EmulatorConfig.kitty_keyboard` 设为 `true` |
| `input::encode::encode_key` 将 `KeyCode::Enter`、`Backspace`、`Tab`、`Esc` 硬编码为裸字节（`\r`、`0x7f`、`\t`、`0x1b`），完全丢弃 `SHIFT` / `ALT` / `CONTROL` 修饰键，且完全未根据 `TermMode::KITTY_KEYBOARD_PROTOCOL` 输出 `CSI u` | `src/input/encode.rs:10-35, 148-160` | 导致在终端窗格运行 `opencode`、`claude`、`codex` 时，`Shift+Enter` 与 `Alt+Enter` 变成直接回车发送，`Alt+Backspace` 无法整词删除，`Shift+Tab`（Kitty 模式）与 `Ctrl+Shift+<字母>` 全部失效 |

---

## 1. 已确认决策汇总（Grill Me 收口）

| 编号 | 决策维度 | 确认结论 |
|---|---|---|
| D1 | MCP 数字加载与显示逻辑重构 | 对齐 `opencode-dev`（`packages/core/src/mcp/index.ts` + `packages/tui/src/feature-plugins/home/footer.tsx`）的实时响应式计数模型：<br>1. 移除阻塞式的 `loading mcp` 文案，启动或重载时直接展示当前已连接就绪的数量 `{ready} mcp`；<br>2. 每个 MCP 服务在后台异步握手完成（`ServerStatus::Ready`）即实时 `+1` 刷新界面，不受慢服务阻塞；<br>3. 出现连接失败（`Failed`）或需要认证（`NeedsLogin`）时，实时在右侧追加红色的 `{errors} error` / `{errors} errors`；<br>4. 配置变更（`refresh_mcp`）时直接从当前已启用 server 的 `mcp_statuses` 实时聚合 `ready` 与 `errors`，消除残留错误计数。 |
| D2 | `mcp` 左侧新增 `AGENTS.md` 数量 | 1. **统计口径**：与实际注入给模型的 `chat::agent::load_instructions` 严格同源——统计当前工作区**实际生效（非空）的全局（`~/.lx-t[-dev]`）+ 项目级（已信任工作区根链路）指令文件总数**（即 `usize::from(global_source.is_some()) + project_sources.len()`）；默认初始化的空 `~/.lx-t[-dev]/AGENTS.md` 不计入。<br>2. **位置与文案**：置于 `mcp` 左侧，以 ` · ` 分隔，文案固定为 `{count} AGENTS.md`，整行格式为：`{agents_md} AGENTS.md · {mcp} mcp · {skills} skills`。<br>3. **配色**：使用 `style::chat_agents_md()`（品牌冰蓝 `Color::Indexed(117)`，与 `mcp` 的翡翠绿 `chat_tool_verb()`、`skills` 的浅黄 `chat_tool_param()` 形成清晰语义区分）。 |
| D3 | 内嵌终端运行 `opencode` / `claude` / `codex` 的快捷键与 `Shift+Enter` 换行修复 | 1. **开启内嵌终端 Kitty 协议握手**：在 `src/terminal/mod.rs` 中设置 `EmulatorConfig { kitty_keyboard: true, .. }`，使 `alacritty_terminal` 正常响应子 Agent 的 `CSI ? u` 查询并维护 `TermMode::KITTY_KEYBOARD_PROTOCOL` 栈。<br>2. **对齐 Ghostty 的“Kitty `CSI u` + Legacy 兜底”双层编码**（`src/input/encode.rs`）：<br>   - **Kitty 模式**（子进程已开启 `TermMode::KITTY_KEYBOARD_PROTOCOL`）：按子进程请求的 `TermMode` 位掩码编码修饰键序列（`Shift+Enter` → `\x1b[13;2u`、`Alt+Enter` → `\x1b[13;3u`、`Ctrl+Enter` → `\x1b[13;5u`、`Shift+Tab` → `\x1b[9;2u`、`Alt+Backspace` → `\x1b[127;3u`、`Ctrl+Backspace` → `\x1b[127;5u`、`Ctrl+<char>` / `Ctrl+Shift+<char>` / `Alt+<char>` → `\x1b[<codepoint>;<mod>u`）；普通无控制修饰的可打印字符（含 `Shift` 产出的大写与符号 `!@#`）保持发送原生 UTF-8 字节。<br>   - **Legacy 模式**（子进程未启用 Kitty 协议，如默认态下的 `claude` 或常规 shell）：对齐 Ghostty `src/input/function_keys.zig` 与 xterm `modifyOtherKeys` 规范——`Shift+Enter` → `\x1b[27;2;13~`、`Ctrl+Enter` → `\x1b[27;5;13~`、`Ctrl+Shift+Enter` → `\x1b[27;6;13~`、`Alt+Enter` → `\x1b\r`、`Shift+Tab`（含 `Tab+SHIFT`）→ `\x1b[Z`、`Alt+Backspace` → `\x1b\x7f`、`Ctrl+Backspace` → `\x08`、`Ctrl+Shift+<ASCII字母>` → `\x1b[<小写码点>;6u`。<br>3. **保留宿主级安全边界**：`Ctrl+Q`（退出 `lx`）、有终端选区时的 `Ctrl/Cmd+C`（复制选区）、Legacy 终端下的 `Super/Cmd` 键以及 Kitty 模式下的纯 `Super+C`（无选区时不透传以免在 `claude` 中误插入 `c`）由宿主 `lx-tui` 拦截；Kitty 模式下的其余 `Super/Cmd` 组合（如 `opencode` 的 `Super+Z` 撤销、`Super+Shift+Z` 重做、`Super+A` 全选、`Super+方向键`）按 `CSI u`（modifier `+8`）完整转发。 |

---

## 2. 模块结构与变更文件清单

所有改动严格遵守 `app/`（纯状态与更新）、`event/`（IO 与后台调度）、`ui/`（纯只读渲染）与 `input/` / `terminal/` 边界，且所有涉及文件均远低于 1000 行上限：

```text
src/
  chat/
    agent/
      instructions.rs         新增 count_active_instructions(user_root, project_root, cwd, workspace_trusted) -> usize 辅助函数（复用 load_instructions）
  app/
    state/
      lx.rs                   StartupCounts 新增 pub agents_md: usize 字段；更新注释
    state.rs                  AppState 新增 agents_md_inventory: BTreeMap<Option<PathBuf>, usize>；sync_lx_inventory 同步 agents_md、mcp、skills
    state/tests.rs            更新并补充 sync_lx_inventory 的 agents_md / mcp / skills 同步单测
  event/
    mod.rs                    启动、新打开工作区、信任变更及设置变更时调用 inventory::refresh_agents_md
    inventory.rs              重构 initialize_mcp / refresh_mcp / update_mcp_status（基于 mcp_statuses 实时计算 ready/errors，取消 loading 阻塞）；新增 refresh_agents_md
    inventory/tests.rs        新增 MCP 状态实时聚合、禁用失败服务清除错误计数、AGENTS.md 计数同步单测
  ui/
    text.rs                   新增 pub const LX_HOME_AGENTS_MD: &str = "AGENTS.md"
    style.rs                  新增 pub fn chat_agents_md() -> Style（品牌冰蓝 Color::Indexed(117)）
    main_content/
      lx/
        brand.rs              重构 draw_inventory 與 inventory_parts：左侧渲染 `{count} AGENTS.md`，中间 `{ready} mcp` (+ `{errors} error(s)`)，右侧 `{ready} skills`
        tests.rs              更新 home_shows_mcp_and_skill_counts 并验证 AGENTS.md 计数与实时 MCP 数字显示
  terminal/
    mod.rs                    Terminal::new 中设置 EmulatorConfig { kitty_keyboard: true, .. }
    tests.rs                  新增 Kitty 键盘协议探测（`CSI ? u` -> `\x1b[?0u`）与压栈/出栈（`CSI > 5 u` / `CSI < u`）模式位回归测试
  input/
    encode.rs                 重构 encode_key：支持 Kitty CSI u 模式编码与 Ghostty 对齐的 Legacy 修饰键编码（Shift+Enter、Alt+Enter、Ctrl+Enter、Shift+Tab、Alt/Ctrl+Backspace、Ctrl+Shift+<char> 等）
    encode/tests.rs           补充 Legacy 与 Kitty 模式下针对 opencode / claude / codex 的完整快捷键编码矩阵测试
```

---

## 3. 核心设计详解与关键代码

### 3.1 MCP 数字实时加载逻辑重构与 `AGENTS.md` 计数接入

#### 3.1.1 数据结构设计（`src/app/state/lx.rs` & `src/app/state.rs`）

在 `src/app/state/lx.rs` 中，扩展 `StartupCounts`：

```rust
/// 首页资源计数的一类（MCP server / skill）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupCount {
    /// 配置中的总数；0 表示未配置。
    pub total: usize,
    /// 已就绪数量。
    pub ready: usize,
    /// 失败或需要登录的数量。
    pub errors: usize,
    /// 后台启动/扫描是否仍在进行（供内部状态跟踪，UI 直接实时展示 ready 与 errors）。
    pub loading: bool,
}

/// 首页展示的 AGENTS.md、MCP 与 skill 启动计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupCounts {
    /// 当前工作区生效（非空）的全局 + 项目级 AGENTS.md 文件数量。
    pub agents_md: usize,
    pub mcp: StartupCount,
    pub skills: StartupCount,
}
```

在 `src/app/state.rs` 中，`AppState` 维护按工作区作用域根（`Option<PathBuf>`，与 `skill_inventory` 同构）索引的 `agents_md_inventory`：

```rust
/// 每个工作区根（`None` 表示无路径工作区）的生效 AGENTS.md 计数；由事件层刷新并经 `sync_lx_inventory` 同步到窗格。
pub agents_md_inventory: BTreeMap<Option<PathBuf>, usize>,
```

并在 `AppState::sync_lx_inventory` 中统一分发给每个窗格的 `pane.lx.startup`：

```rust
pub fn sync_lx_inventory(&mut self) -> bool {
    let roots: Vec<Option<PathBuf>> = self
        .workspaces
        .iter()
        .map(|workspace| workspace.repo_key().map(Path::to_path_buf))
        .collect();
    let counts: Vec<StartupCounts> = roots
        .into_iter()
        .map(|root| StartupCounts {
            agents_md: self.agents_md_inventory.get(&root).copied().unwrap_or(0),
            mcp: root
                .as_ref()
                .and_then(|root| self.mcp_inventory.get(root))
                .copied()
                .unwrap_or_default(),
            skills: self.skill_inventory.get(&root).copied().unwrap_or_default(),
        })
        .collect();
    let mut changed = false;
    for (workspace, counts) in self.workspaces.iter_mut().zip(counts) {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id)
                    && pane.lx.startup != counts
                {
                    pane.lx.startup = counts;
                    changed = true;
                }
            }
        }
    }
    changed
}
```

#### 3.1.2 `opencode-dev` 对齐的 MCP 实时计数与 `AGENTS.md` 计数刷新（`src/event/inventory.rs`）

参考 `opencode-dev` 的 `packages/tui/src/feature-plugins/home/footer.tsx:18-20` 与 `packages/tui/src/feature-plugins/sidebar/mcp.tsx:9-11`：
- `ready` 恒等于当前项目已启用 server 中状态为 `"ready"` 的数量；
- `errors` 恒等于当前项目已启用 server 中状态为 `"failed"` 或 `"needs_login"` 的数量；
- 配置变更（`SettingsEvent::McpChanged`）时，先通过 `prune_mcp_statuses` 剔除已禁用/已删除的 server，再调用 `recompute_mcp_inventory` 直接从 `mcp_statuses` 计算出精确的 `ready` 与 `errors`。

关键代码（`src/event/inventory.rs`）：

```rust
/// 启动（或首次打开工作区）时按配置初始化 MCP 计数与 `@` 面板候选并后台预热。
pub(crate) fn initialize_mcp(state: &mut AppState, persistence: &Persistence, roots: &[PathBuf]) {
    for root in roots {
        let effective = super::chat::mcp::effective_for_root(persistence, root);
        let mentions: Vec<crate::app::prompt::McpMentionEntry> = effective
            .iter()
            .filter(|server| server.config.enabled)
            .map(|server| {
                crate::app::prompt::McpMentionEntry::new(
                    server.name.clone(),
                    server.config.summary(),
                )
            })
            .collect();
        apply_mcp_mentions(state, root, &mentions);
        if state.mcp_inventory.contains_key(root) {
            continue;
        }
        let total = mentions.len();
        state.mcp_inventory.insert(
            root.clone(),
            StartupCount {
                total,
                ready: 0,
                errors: 0,
                loading: total > 0,
            },
        );
    }
    super::chat::mcp::prewarm_projects(persistence, roots);
}

/// 配置变化后刷新总数、状态聚合与 `@` 面板候选。
pub(crate) fn refresh_mcp(
    state: &mut AppState,
    persistence: &Persistence,
    roots: &[PathBuf],
    statuses: &std::collections::BTreeMap<(PathBuf, String), McpStatusView>,
) {
    for root in roots {
        let effective = super::chat::mcp::effective_for_root(persistence, root);
        let mentions: Vec<crate::app::prompt::McpMentionEntry> = effective
            .iter()
            .filter(|server| server.config.enabled)
            .map(|server| {
                crate::app::prompt::McpMentionEntry::new(
                    server.name.clone(),
                    server.config.summary(),
                )
            })
            .collect();
        apply_mcp_mentions(state, root, &mentions);
        let total = mentions.len();
        state.mcp_inventory.entry(root.clone()).or_default().total = total;
        update_mcp_status(state, root, statuses);
    }
    super::chat::mcp::prewarm_projects(persistence, roots);
}

/// 单个 server 状态或配置变化后，按项目从 `statuses` 实时重算 ready / error / loading。
pub(crate) fn update_mcp_status(
    state: &mut AppState,
    project: &Path,
    statuses: &std::collections::BTreeMap<(PathBuf, String), McpStatusView>,
) {
    let mut ready = 0;
    let mut errors = 0;
    let mut starting = 0;
    let mut tracked = 0;
    for ((root, _), status) in statuses {
        if root.as_path() != project {
            continue;
        }
        tracked += 1;
        match status.status.as_str() {
            "ready" => ready += 1,
            "failed" | "needs_login" => errors += 1,
            "starting" => starting += 1,
            _ => {}
        }
    }
    let current = state
        .mcp_inventory
        .get(project)
        .copied()
        .unwrap_or_default();
    let total = current.total.max(tracked);
    let settled = ready.saturating_add(errors);
    state.mcp_inventory.insert(
        project.to_path_buf(),
        StartupCount {
            total,
            ready,
            errors,
            loading: starting > 0 || (total > 0 && settled < total),
        },
    );
}

/// 刷新所有打开工作区的生效 `AGENTS.md` 文件数量（全局 `~/.lx-t[-dev]` + 已信任项目目录链）。
pub(crate) fn refresh_agents_md(state: &mut AppState, persistence: &Persistence) {
    let user_root = crate::config::user_data_root().ok();
    for root in skill_roots(state) {
        let trusted = root
            .as_deref()
            .is_some_and(|path| persistence.is_trusted(path));
        let loaded = crate::chat::agent::load_instructions(
            user_root.as_deref(),
            root.as_deref(),
            root.as_deref(),
            trusted,
        );
        let count = usize::from(loaded.global_source.is_some()) + loaded.project_sources.len();
        state.agents_md_inventory.insert(root, count);
    }
}
```

#### 3.1.3 欢迎页资源计数渲染（`src/ui/main_content/lx/brand.rs`、`src/ui/text.rs`、`src/ui/style.rs`）

- 在 `src/ui/text.rs` 新增：
  ```rust
  pub const LX_HOME_AGENTS_MD: &str = "AGENTS.md";
  ```
- 在 `src/ui/style.rs` 新增：
  ```rust
  /// 首页 `AGENTS.md` 资源计数：品牌冰蓝（与 `mcp` 翡翠绿、`skills` 浅黄区分）。
  pub fn chat_agents_md() -> Style {
      Style::default().fg(Color::Indexed(117))
  }
  ```
- 在 `src/ui/main_content/lx/brand.rs` 中重构 `draw_inventory` 与 `inventory_parts`：
  ```rust
  /// 首页资源计数：`1 AGENTS.md · 4 mcp · 3 skills`；失败追加 `1 error`。
  pub(super) fn draw_inventory(grid: &mut Grid, rect: Rect, counts: StartupCounts) {
      if rect.width == 0 {
          return;
      }
      let sep = (format!(" {} ", text::LX_ENTRY_SEPARATOR), style::muted());
      let agents_md = (
          format!("{} {}", counts.agents_md, text::LX_HOME_AGENTS_MD),
          style::chat_agents_md(),
      );
      let mcp = inventory_parts(counts.mcp, text::LX_HOME_MCP, style::chat_tool_verb());
      let skills = inventory_parts(
          counts.skills,
          text::LX_HOME_SKILLS,
          style::chat_tool_param(),
      );
      let mut segments: Vec<(String, Style)> = vec![agents_md, sep.clone()];
      segments.extend(mcp);
      segments.push(sep);
      segments.extend(skills);
      let width: usize = segments.iter().map(|(value, _)| value.width()).sum();
      let width = u16::try_from(width).unwrap_or(u16::MAX);
      // 超宽时从左边起画，后面的段由 Grid 自然裁掉。
      let mut x = if width > rect.width {
          rect.x
      } else {
          rect.x + rect.width.saturating_sub(width) / 2
      };
      let mut remaining = usize::from(rect.width);
      for (value, value_style) in segments {
          if remaining == 0 {
              break;
          }
          let value = text::truncate_to_width(&value, remaining);
          if value.is_empty() {
              break;
          }
          grid.put(x, rect.y, &value, value_style);
          let used = value.width().min(remaining);
          x = x.saturating_add(u16::try_from(used).unwrap_or(u16::MAX));
          remaining = remaining.saturating_sub(used);
      }
  }

  fn inventory_parts(count: StartupCount, label: &str, color: Style) -> Vec<(String, Style)> {
      let mut parts = vec![(format!("{} {label}", count.ready), color)];
      if count.errors > 0 {
          let label = if count.errors == 1 {
              text::LX_HOME_ERROR
          } else {
              text::LX_HOME_ERRORS
          };
          parts.push((" ".to_string(), style::muted()));
          parts.push((format!("{} {label}", count.errors), style::agent_fail()));
      }
      parts
  }
  ```

---

### 3.2 内嵌终端运行 `opencode` / `claude` / `codex` 的快捷键与 `Shift+Enter` 换行对齐 Ghostty

#### 3.2.1 根因与三大 Agent 协议矩阵

当用户在 `lx-tui` 的终端窗格中启动 `opencode`、`claude`（Claude Code）或 `codex` 时，三大 Agent 的终端握手与按键解析行为如下：

| Agent | 启动时发送的探测/启用序列 | `Shift+Enter` 换行期望序列 | `Alt+Enter` / `Ctrl+Enter` 换行序列 | `Shift+Tab` 期望序列 | `Alt+Backspace` / `Ctrl+Backspace` 删词序列 | 特殊解析约束 |
|---|---|---|---|---|---|---|
| **`opencode`** (`@opentui/core`) | 发送 `\x1b[?u` + `\x1b[c`；收到 `\x1b[?0u` 后发送 `\x1b[>5u`（`DISAMBIGUATE_ESC_CODES \| REPORT_ALTERNATE_KEYS`）；若未收到 `\x1b[?0u` 则退回发送 `\x1b[>4;1m` | Kitty: `\x1b[13;2u`<br>Legacy: `\x1b[27;2;13~` | Kitty: `\x1b[13;3u` / `\x1b[13;5u`<br>Legacy: `\x1b\r` / `\x1b[27;5;13~` | Kitty: `\x1b[9;2u`<br>Legacy: `\x1b[Z` | Kitty: `\x1b[127;3u` / `\x1b[127;5u`<br>Legacy: `\x1b\x7f` / `\x08` | 未收到 `\x1b[?0u` 时不开启 `parseKittyKeyboard` |
| **`codex`** (`crossterm 0.29`) | 发送 `\x1b[?u\x1b[c\x1b[6n...`；收到 `\x1b[?0u` 判定 `enhanced_keys_supported = true` 并发送 `\x1b[>5u` 或 `\x1b[>7u` | Kitty: `\x1b[13;2u`<br>Legacy: `\x1b[27;2;13~` | Kitty: `\x1b[13;3u`<br>Legacy: `\x1b\r` | Kitty: `\x1b[9;2u`<br>Legacy: `\x1b[Z` | Kitty: `\x1b[127;3u` / `\x1b[127;5u`<br>Legacy: `\x1b\x7f` / `\x08` | 裸 `\r` 会直接触发 `composer.submit` 提交消息 |
| **`claude`** (`openclaude` / Ink) | 白名单命中时发 `\x1b[>1u\x1b[>4;2m`，未命中时不发；但 `parseKeypress` **无条件常驻匹配** `CSI_U_RE` 与 `MODIFY_OTHER_KEYS_RE` | Kitty: `\x1b[13;2u`<br>Legacy: `\x1b[27;2;13~` | Kitty: `\x1b[13;3u`<br>Legacy: `\x1b\r` | Kitty: `\x1b[9;2u`<br>Legacy: `\x1b[Z` | Kitty: `\x1b[127;3u` / `\x1b[127;5u`<br>Legacy: `\x1b\x7f` / `\x08` | `CSI_U_RE` 为 `/^\x1b\[(\d+)(?:;(\d+))?u/`，**不支持 `:` 子参数**；且 `Ctrl+J` 必须收到 `0x0a` (`\n`) 才能换行 |

#### 3.2.2 开启 `alacritty_terminal` 的 Kitty 键盘协议支持（`src/terminal/mod.rs`）

在 `src/terminal/mod.rs:111-118` 的 `Terminal::new` 中开启 `kitty_keyboard: true`：

```rust
let term = Term::new(
    EmulatorConfig {
        scrolling_history: SCROLLBACK_LINES,
        kitty_keyboard: true,
        ..EmulatorConfig::default()
    },
    &size,
    listener,
);
```

开启后：
1. 子进程发送 `\x1b[?u` 时，`alacritty_terminal` 生成 `EmulatorEvent::PtyWrite("\x1b[?0u")`（或当前栈顶模式值），经 `Terminal::feed` 返回并在 `src/event/mod.rs:688-693` 自动写回子 PTY；
2. 子进程发送 `\x1b[>1u` / `\x1b[>5u` / `\x1b[>7u` 时，`alacritty_terminal` 自动压栈并置位 `TermMode::DISAMBIGUATE_ESC_CODES`、`TermMode::REPORT_ALTERNATE_KEYS`、`TermMode::REPORT_EVENT_TYPES` 等标志位；
3. 子进程退出或发送 `\x1b[<u` 或离开 `ALT_SCREEN` 时，`alacritty_terminal` 自动弹出并恢复主屏的 `TermMode`。

#### 3.2.3 对齐 Ghostty 的双模式按键编码器（`src/input/encode.rs`）

对齐 Ghostty（`src/input/key_encode.zig`、`src/input/function_keys.zig`、`src/input/kitty.zig`）与 `herdr`（`src/input/encode.rs`）：

1. **前置过滤**：
   - `KeyModifiers::SUPER`（macOS `Cmd`）继续返回 `None`，绝不泄漏进 PTY；
   - `KeyEventKind::Release` 继续在路由层丢弃（宿主 `crossterm` 仅开启 `DISAMBIGUATE_ESCAPE_CODES`，不订阅 `Release`）。
2. **`KeyCode::BackTab` 归一化**：
   - 宿主终端在 Kitty 模式下按下 `Shift+Tab` 可能由 `crossterm` 报告为 `KeyCode::BackTab`（带或不带 `SHIFT`）或 `KeyCode::Tab` + `KeyModifiers::SHIFT`。统一将 `KeyCode::BackTab` 归一化为 `KeyCode::Tab` + `modifiers | KeyModifiers::SHIFT` 处理。
3. **当子终端启用了 Kitty 协议（`mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)`）**：
   - **普通文本直接输出**：当 `KeyCode::Char(c)` 不含 `CONTROL` 与 `ALT`（即 `modifiers.is_empty()` 或 `modifiers == KeyModifiers::SHIFT`），且未开启 `TermMode::REPORT_ALL_KEYS_AS_ESC` 时，直接输出 `c` 的 UTF-8 字节（确保大写字母与 `!@#$%^&*()` 等上档符号原样输入）；
   - **无修饰基础键保持传统兼容编码**：当 `modifiers.is_empty()` 且未开启 `TermMode::REPORT_ALL_KEYS_AS_ESC` 时，`Enter` → `\r`、`Tab` → `\t`、`Backspace` → `0x7f`（`Esc` 在 `DISAMBIGUATE_ESC_CODES` 下或带修饰键时编码为 `\x1b[27u` / `\x1b[27;{mod}u`，无修饰时 `\x1b` 亦可被所有 TUI 识别，保持 `\x1b` 最兼容）；
   - **方向键 / 导航键 / F1–F12**：继续使用全终端通用的 xterm 修饰格式（`\x1b[1;{mod}A..D`、`\x1b[{code};{mod}~`）；
   - **带修饰键的 `Enter` / `Tab` / `Backspace` / `Esc` / `Char`**：编码为标准 `CSI u` 格式 `\x1b[{codepoint};{param}u`：
     - `Enter` = `13`（如 `Shift+Enter` → `\x1b[13;2u`、`Alt+Enter` → `\x1b[13;3u`、`Ctrl+Enter` → `\x1b[13;5u`）
     - `Tab` = `9`（如 `Shift+Tab` → `\x1b[9;2u`）
     - `Backspace` = `127`（如 `Alt+Backspace` → `\x1b[127;3u`、`Ctrl+Backspace` → `\x1b[127;5u`）
     - `Esc` = `27`（如 `Shift+Esc` → `\x1b[27;2u`、`Alt+Esc` → `\x1b[27;3u`）
     - `Char(c)`：基准码点取 `c.to_ascii_lowercase() as u32`（ASCII 字母）或 `c as u32`；注意：**对于单纯的 `Ctrl+J`（`KeyCode::Char('j')` + `KeyModifiers::CONTROL`），在未开启 `REPORT_ALL_KEYS_AS_ESC` 时保留经典的 `0x0a` (`\n`) 或在 Kitty 下发 `\x1b[106;5u`？**
       - 核查结论：在 `opencode` 与 `codex` 中，`0x0a` (`\n`) 和 `\x1b[106;5u` 都能触发换行；而在 `claude`（`openclaude`）中，如果开启了 `\x1b[>1u`（仅 `DISAMBIGUATE_ESC_CODES`），`claude` 的 `handleCtrl` 未绑定 `'j'`，收到 `\x1b[106;5u` 会被吞掉，只有收到 `0x0a` (`\n`) 才会换行！更重要的是，按照 Kitty 键盘协议官方规范（[Kitty Keyboard Protocol — Disambiguate escape codes](https://sw.kovidgoyal.net/kitty/keyboard-protocol/#disambiguate-escape-codes)），在仅开启 `DISAMBIGUATE_ESC_CODES` 时，`Ctrl+a..z` 的 legacy C0 控制码只有与传统键冲突的 `Ctrl+I` (Tab)、`Ctrl+M` (Enter)、`Ctrl+[` (Esc) 以及带 `Shift`/`Alt` 的组合键（如 `Ctrl+Shift+A`）才必须转成 `CSI u`，其余单纯的 `Ctrl+<a..z>`（不含 `Shift`/`Alt`）只有在开启 `REPORT_ALL_KEYS_AS_ESC` 时才转 `CSI u`！
       - 这意味着：**当仅含 `CONTROL`（无 `SHIFT`、无 `ALT`）且未开启 `REPORT_ALL_KEYS_AS_ESC` 时，普通 `Ctrl+A..Z`（除 `i`/`m`/`[` 外）继续发送标准 C0 控制字节（`Ctrl+C` = `0x03`, `Ctrl+J` = `0x0a`, `Ctrl+D` = `0x04`, `Ctrl+Z` = `0x1a` 等），而 `Ctrl+Shift+<char>`、`Ctrl+Alt+<char>`、`Ctrl+I`、`Ctrl+M` 则发送 `CSI u`（`\x1b[{cp};{mod}u`）！** 这样既 100% 兼容 `claude` 的 `Ctrl+J` 与 C0 控制键，又让 `Ctrl+Shift+<key>`、`Ctrl+M`、`Ctrl+I`、`Shift+Enter`、`Alt+Enter`、`Shift+Tab`、`Alt+Backspace` 在三大 Agent 中全部完美工作！
4. **当子终端处于 Legacy 模式（`!mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)`）**：
   - 对齐 Ghostty `src/input/function_keys.zig` 的默认表：
     - `Enter`：
       - 无修饰 → `\r`
       - `ALT`（无 `SHIFT`/`CONTROL`）→ `\x1b\r`
       - 含 `SHIFT` 或 `CONTROL`（如 `Shift+Enter`、`Ctrl+Enter`、`Ctrl+Shift+Enter`、`Alt+Shift+Enter`）→ `\x1b[27;{param};13~`（其中 `Shift+Enter` 为 `\x1b[27;2;13~`，`Ctrl+Enter` 为 `\x1b[27;5;13~`）
     - `Tab` / `BackTab`：
       - 无修饰 → `\t`
       - 含 `SHIFT`（`Shift+Tab` / `BackTab`）→ `\x1b[Z`
       - 仅 `ALT` → `\x1b\t`
       - 含 `CONTROL` → `\x1b[27;{param};9~`
     - `Backspace`：
       - 无修饰 → `0x7f`
       - `ALT` + `CONTROL` → `\x1b\x08`
       - 仅 `ALT` → `\x1b\x7f`
       - 仅 `CONTROL` → `\x08`
       - 含 `SHIFT` → `\x1b[27;{param};127~`
     - `Esc`：
       - 无修饰 → `\x1b`
       - 仅 `ALT` → `\x1b\x1b`
       - 含 `SHIFT` 或 `CONTROL` → `\x1b[27;{param};27~`
     - `Char(c)`：
       - 若同时含 `CONTROL` 与 `SHIFT`（如 `Ctrl+Shift+A`）：对齐 Ghostty `legacy()` 的 fixterms 分支，编码为 `\x1b[{lower_cp};{param}u`（如 `\x1b[97;6u`）；
       - 否则保持标准 Legacy 编码（含 `ALT` 前置 `0x1b`，含 `CONTROL` 走 `control_byte(c)`）。

#### 3.2.4 关键代码实现（`src/input/encode.rs`）

```rust
pub fn encode_key(key: KeyEvent, mode: TermMode) -> Option<Vec<u8>> {
    if key.modifiers.contains(KeyModifiers::SUPER) {
        return None;
    }
    let (code, modifiers) = match key.code {
        KeyCode::BackTab => (KeyCode::Tab, key.modifiers | KeyModifiers::SHIFT),
        other => (other, key.modifiers),
    };
    let kitty = mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL);
    let report_all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);

    match code {
        KeyCode::Char(c) => Some(encode_char(c, modifiers, kitty, report_all)),
        KeyCode::Enter => Some(encode_enter(modifiers, kitty, report_all)),
        KeyCode::Backspace => Some(encode_backspace(modifiers, kitty, report_all)),
        KeyCode::Tab => Some(encode_tab(modifiers, kitty, report_all)),
        KeyCode::Esc => Some(encode_esc(modifiers, kitty, report_all)),
        KeyCode::Up => Some(cursor_key(b'A', modifiers, mode)),
        KeyCode::Down => Some(cursor_key(b'B', modifiers, mode)),
        KeyCode::Right => Some(cursor_key(b'C', modifiers, mode)),
        KeyCode::Left => Some(cursor_key(b'D', modifiers, mode)),
        KeyCode::Home => Some(cursor_key(b'H', modifiers, mode)),
        KeyCode::End => Some(cursor_key(b'F', modifiers, mode)),
        KeyCode::Insert => Some(tilde_key(2, modifiers)),
        KeyCode::Delete => Some(tilde_key(3, modifiers)),
        KeyCode::PageUp => Some(tilde_key(5, modifiers)),
        KeyCode::PageDown => Some(tilde_key(6, modifiers)),
        KeyCode::F(number) => function_key(number, modifiers),
        _ => None,
    }
}

fn csi_u(codepoint: u32, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param == 1 {
        format!("\x1b[{codepoint}u").into_bytes()
    } else {
        format!("\x1b[{codepoint};{param}u").into_bytes()
    }
}

fn modify_other_key(codepoint: u32, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    format!("\x1b[27;{param};{codepoint}~").into_bytes()
}

fn encode_enter(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(13, modifiers)
        } else {
            vec![b'\r']
        };
    }
    if kitty {
        return csi_u(13, modifiers);
    }
    if modifiers == KeyModifiers::ALT {
        vec![0x1b, b'\r']
    } else {
        modify_other_key(13, modifiers)
    }
}

fn encode_tab(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(9, modifiers)
        } else {
            vec![b'\t']
        };
    }
    if kitty {
        return csi_u(9, modifiers);
    }
    if modifiers == KeyModifiers::SHIFT {
        b"\x1b[Z".to_vec()
    } else if modifiers == KeyModifiers::ALT {
        vec![0x1b, b'\t']
    } else {
        modify_other_key(9, modifiers)
    }
}

fn encode_backspace(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(127, modifiers)
        } else {
            vec![0x7f]
        };
    }
    if kitty {
        return csi_u(127, modifiers);
    }
    if modifiers == KeyModifiers::ALT {
        vec![0x1b, 0x7f]
    } else if modifiers == KeyModifiers::CONTROL {
        vec![0x08]
    } else if modifiers == (KeyModifiers::ALT | KeyModifiers::CONTROL) {
        vec![0x1b, 0x08]
    } else {
        modify_other_key(127, modifiers)
    }
}

fn encode_esc(modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    if modifiers.is_empty() {
        return if report_all {
            csi_u(27, modifiers)
        } else {
            vec![0x1b]
        };
    }
    if kitty {
        return csi_u(27, modifiers);
    }
    if modifiers == KeyModifiers::ALT {
        vec![0x1b, 0x1b]
    } else {
        modify_other_key(27, modifiers)
    }
}

fn encode_char(c: char, modifiers: KeyModifiers, kitty: bool, report_all: bool) -> Vec<u8> {
    let has_ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let has_alt = modifiers.contains(KeyModifiers::ALT);
    let has_shift = modifiers.contains(KeyModifiers::SHIFT);

    // 无 Ctrl/Alt 的普通或 Shift 可打印字符：除非开启 REPORT_ALL_KEYS_AS_ESC，一律发 UTF-8 文本。
    if !has_ctrl && !has_alt {
        if report_all {
            let base = if has_shift && c.is_ascii_uppercase() {
                c.to_ascii_lowercase() as u32
            } else {
                c as u32
            };
            return csi_u(base, modifiers);
        }
        let mut buffer = [0u8; 4];
        return c.encode_utf8(&mut buffer).as_bytes().to_vec();
    }

    let base_codepoint = if c.is_ascii_alphabetic() {
        c.to_ascii_lowercase() as u32
    } else {
        c as u32
    };

    // Ctrl+Shift+<char>：Legacy C0 无法表达 Shift，对齐 Ghostty 统一走 CSI u。
    if has_ctrl && has_shift {
        return csi_u(base_codepoint, modifiers);
    }

    // Kitty 协议激活时：
    // - 开启 REPORT_ALL_KEYS_AS_ESC：所有 Ctrl/Alt 组合均走 CSI u；
    // - 仅开启 DISAMBIGUATE_ESC_CODES：Alt 组合与存在歧义的 Ctrl+I / Ctrl+M / Ctrl+[ 走 CSI u，
    //   其余单纯 Ctrl+a..z 保持 C0 控制字节（确保 claude 的 Ctrl+J = 0x0a 换行及 shell 信号兼容）。
    if kitty {
        let ambiguous_ctrl = matches!(c.to_ascii_lowercase(), 'i' | 'm' | '[');
        if report_all || has_alt || ambiguous_ctrl {
            return csi_u(base_codepoint, modifiers);
        }
    }

    let mut bytes = Vec::new();
    if has_alt {
        bytes.push(0x1b);
    }
    if has_ctrl {
        bytes.push(control_byte(c));
    } else {
        let mut buffer = [0u8; 4];
        bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
    }
    bytes
}
```

---

## 4. 风险与边界分析

1. **多工作区与信任状态切换**：
   - 当未信任工作区点击信任（`Persistence` 授予信任）或工作区 Git 仓库根刷新（`GitRefreshed`）时，项目级 `AGENTS.md` 会从“未启用（0）”变为“已启用（N）”。因此 `inventory::refresh_agents_md` 在初始化、工作区列表变化、`persistence.pump(state)` 返回 `true`（信任状态落盘）以及 `skills_reload` 触发时均同步更新，开销仅为最多几个目录的轻量文件读取。
2. **窄屏截断**：
   - `brand::draw_inventory` 保持原有的居中与超宽截断保护（`text::truncate_to_width(&value, remaining)`），在极窄窗格下不会越界或 panic。
3. **子终端退出后的模式恢复**：
   - 当 `opencode` / `codex` / `claude` 退出并离开备用屏（或弹出 Kitty 键盘栈）时，`alacritty_terminal` 自动将 `TermMode::KITTY_KEYBOARD_PROTOCOL` 清零，终端立即回退到 Legacy 模式，普通 shell（`zsh` / `bash`）输入不受任何影响。
