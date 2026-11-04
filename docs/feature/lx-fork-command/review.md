# `/fork` 会话分支功能评审报告 (Review)

本文档针对工作区在分支 `lx-fork-command` 上实现的 `/fork` 会话分支功能、数据库事务层、应用状态机、TUI 渲染层以及对应的 `design.md` 与 `task.md` 文档进行全面审查，梳理关键点、错误原因，并按 P0 ~ P3 排序规划修复。

---

## 一、审查概述与基线

- **功能目标**：在 lx-tui 中提供 `/fork` 命令，弹出二级选择面板，列出历史用户消息（最新在上，首项固定为 `Full session`，显示相对时间），选中后分支新会话并回填原文。
- **当前状态**：核心 DB 事务复制、图片复制、应用状态机、Effects 分派与 TUI 渲染已初版完成。全量测试通过（4426 passed）。
- **审查发现**：
  - **P0（致命/崩溃/数据损毁）**：0 项
  - **P1（硬上限违规/严重功能缺陷/UI破坏）**：3 项
  - **P2（逻辑边界缺陷/交互陷阱/状态不一致）**：4 项
  - **P3（代码异味/文档脱节/测试补齐）**：4 项

---

## 二、缺陷与问题清单（按 P0 ~ P3 排序）

### P0：阻塞级致命缺陷（0 项）
- 无内存安全隐患、Panic、无控制事务回滚或数据表死锁。

---

### P1：高优先级（硬限制违规 / 功能严重缺陷 / 关键特性破坏）

#### P1-1: `src/app/update/chat/session.rs` 超过 1000 行硬上限（当前 1017 行）
- **关键点**：`AGENTS.md` 与 `docs/standards/code-writing-standards.md` 明确规定：“生产代码单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右。”
- **错误原因**：本次分支向 `session.rs` 追加了 slash 分派分支与加载/回填逻辑（+41 行），使原 976 行的文件突破 1000 行红线。`session.rs` 内部同时混杂了后台任务页面（Task page）、Slash 命令分派、会话生命周期等职责。
- **修复方案**：
  1. 将 Task page 职责（`open_task_page`、`close_task_page`、`select_task_tab`、`on_process_listed` 等约 190 行）拆出为 `src/app/update/chat/task_page.rs`；
  2. 将 Slash 命令交互与分派（`apply_slash_enter`、`apply_slash_command`、`open_model_picker`、`finish_model_selection` 等约 330 行）拆出为 `src/app/update/chat/slash.rs`；
  3. `session.rs` 专注会话生命周期（`new_session`、`resume_session`、`on_session_loaded`、`rename`、`delete`），控制在 500 行左右；
  4. 同步更新 `mod.rs` 导出。

#### P1-2: `load_chat_session_into_pane` 失败时误判为成功，在源会话中回填并误报 `Forked session`
- **关键点**：`src/db/service/chat.rs:409-424` 与 `src/db/service/chat.rs:582-586`。
- **错误原因**：
  - `load_chat_session_into_pane` 的返回值语义为“状态是否变更/是否有 UI 重绘”，当装载失败（如会话已被删除或并发被锁）时，内部弹出了 `LX_CHAT_SESSION_BUSY` 提示，但返回了 `true`！
  - `ForkChatSession` 仅在 `!load_chat_session_into_pane` 时判定失败：
    ```rust
    if !self.load_chat_session_into_pane(...) {
        update::on_fork_failed(...);
        return true;
    }
    update::on_fork_completed(...);
    ```
  - 这导致即使新分支未成功装载，依然调用 `on_fork_completed`，在未切换绑定的**源会话**中回填了提示词，并弹出 `Forked session` 成功 Toast。用户若按下 Enter，将本应发往分支的消息重复发送至源会话。违反 `design.md` §3.4 / Q11。
- **修复方案**：
  - 在 `ForkChatSession` 处增加对窗格当前绑定会话是否已成功变更为 `forked.session_id` 的显式校验；
  - 若绑定未变更，调用 `on_fork_failed`，避免向源会话回填与误报成功；
  - 补充加载失败时的回归测试。

#### P1-3: 较长用户消息导致整张 Fork 面板的时间列完全消失
- **关键点**：`src/ui/widgets/command_panel.rs:656-661, 750-766, 786-792` 与 `src/app/update/chat/fork.rs:134-159`。
- **错误原因**：
  - `fork_label` 按 120 个字符截断，在 80 列常见面板下宽度已远超可用宽度。
  - `command_panel.rs` 在布局计算时取**全量条目的最大 label 宽度**进行统一补齐（`label_width`）。
  - 在渲染单行时，`used = marker + label_width`。当 `used + tag.width() >= available` 时，`pad = available.saturating_sub(used).saturating_sub(tag.width())` 为 0。
  - 而绘制 tag 的条件为 `if pad > 0`！
  - 导致只要有一条长消息存在，所有条目（包括 `Full session` 和短消息）的 `pad` 都为 0，右侧时间列（如 `5m ago`）被全部整列吞掉！
  - 直接违反了用户提出的“面板需要显示时间”与 `design.md:875-876` 的验收标准。
- **修复方案**：
  1. 在 `command_panel.rs` 中，计算 `label_width` 时或单行排版时必须为 `tag` 预留最小保护空间，并将 tag 的渲染条件由 `pad > 0` 修正为当剩余空间能容纳 tag 时绘制（或消除误裁）；
  2. 在 `fork.rs` 的 `fork_label` 中，使用显示列宽截断（建议截断到 50~60 显示列宽），为右侧时间列留出充足空间；
  3. 增加长文本、宽字符存在时时间列依然正常渲染的 UI 回归测试。

---

### P2：中优先级（逻辑边界 / 交互体验 / 数据一致性）

#### P2-1: 子 agent 汇报 Envelope 消息混入 Fork 节点列表
- **关键点**：`src/app/update/chat/fork.rs:22-28`。
- **错误原因**：后台子 agent 汇总消息插入时角色也是 `role: "user"`、`kind: "chat"`（见 `subagent.rs:699-726`），但其携带非空的 `reports`。`fork.rs` 仅判断 `role == "user" && kind == "chat"`，导致内部汇报消息（`[local runtime] subagent reports: ...`）出现在 fork 面板中，若选中会回填机器生成的内部结构文本。
- **修复方案**：在 `fork.rs` 中增加条件 `message.reports.is_empty()`，过滤掉子 agent 汇报消息。

#### P2-2: `next_deadline` 未调度面板时钟唤醒，空闲时相对时间冻结
- **关键点**：`src/app/update/interaction.rs:316-339, 357-378` 与 `src/event/mod.rs:194`。
- **错误原因**：虽然 `tick_slash_panel_clock` 会在 tick 时更新 `now_ms`，但 `next_deadline` 未对 `SlashPanel::Fork` 与 `SlashPanel::History` 调度定期唤醒。如果用户停留在面板不移动键盘鼠标，事件循环会陷入长达 3600 秒的 `IDLE_WAIT`，导致相对时间无法“自然推进”，违背 `design.md:333, 758` 的承诺。
- **修复方案**：在 `next_deadline` 中检测若有窗格正打开 `SlashPanel::Fork` 或 `SlashPanel::History`，返回例如 5~10 秒的唤醒超时（相对时间粒度为分钟，无需每毫秒唤醒）。

#### P2-3: 过滤输入时默认高亮仍停在 `Full session`，直接回车易误克隆全量
- **关键点**：`src/app/state/lx/slash.rs:449-477` 与 `src/app/update/chat/tests/fork.rs:92-98`。
- **错误原因**：`refilter` 重新过滤时重置 `self.window = PanelWindow::default()`，使 active 索引恒为 0。而 `filtered[0]` 恒为 `Full session`。用户输入关键词搜索特定消息后若直接按回车，本意是 fork 匹配到的第一条消息，却直接执行了 `Full session`（克隆全会话）。
- **修复方案**：当过滤词非空且匹配到至少 1 条具体消息时（`filtered.len() > 1`），将初始 active 索引设为 1（首条匹配消息）；无过滤词时保持在 0。

#### P2-4: Windows 路径 JSON 转义导致 `result_json` 中图片路径重写失败与图片复制 entry 错误处理
- **关键点**：`src/db/chat/fork.rs:535`、`src/platform/clipboard_image.rs:229-244`。
- **错误原因**：
  - Windows 系统中文件路径分隔符为 `\`，存入 SQLite JSON 字段时被转义为 `\\`；简单的 `String::replace(from, to)` 无法匹配到转义后的路径。
  - `clipboard_image.rs` 在遍历图片目录时使用了 `entries.flatten()`，若单个 entry 读取失败被吞掉，上层仍返回 `Ok(true)` 并启用全量路径重写，导致 DB 指向实际未复制成功的悬空路径。
- **修复方案**：
  - 在 `fork.rs` 中不仅替换原生路径，同时替换转义路径（`replace(&from.replace('\\', "\\\\"), &to.replace('\\', "\\\\"))`）；
  - 在 `clipboard_image.rs` 中显式处理 entry 错误，避免静默产生悬空引用。

---

### P3：低优先级（文档更新 / 测试补齐 / 代码整洁）

#### P3-1: `design.md` 与 `task.md` 过期与缺失项修正
- **关键点**：
  - `task.md:332` 勾选了 `fork_escape_closes_and_clears`，但实际未实现该测试；
  - `design.md` 存在重复的 `ForkEntry` 结构体且其中一份缺少 `created_at_ms`；
  - `task.md` 与 `design.md` 的文案清单遗漏了 `LX_FORK_EMPTY_MESSAGE`；
  - 文档行号引用已发生漂移。
- **修复方案**：全面同步修复 `design.md` 与 `task.md`，补齐真实的测试用例。

#### P3-2: 测试辅助函数 `buffer_lines` 重复定义
- **关键点**：`src/ui/main_content/lx/tests/fork_panels.rs` 与 `slash_panels.rs` 重复定义了完全相同的 `buffer_lines` 函数，违反 `docs/standards/code-writing-standards.md:60`。
- **修复方案**：将公用测试辅助函数抽取或规范复用。

#### P3-3: `fork_label` 截断按 Unicode 码点数而非显示列宽
- **关键点**：`src/app/update/chat/fork.rs:134-159`。
- **错误原因**：使用 `chars().count()` 截断，CJK 字符占双列，emoji 可能占双列或由多个码点组成。
- **修复方案**：改用 `ui::text::truncate_to_width` 按显示列宽精确截断。

#### P3-4: 补齐各修复项的测试用例
- **修复方案**：
  - 补充 `fork_escape_closes_and_clears` 测试；
  - 补充长文本、宽字符下时间标签稳定显示的 UI 测试；
  - 补充过滤时默认选中首条匹配项而非 `Full session` 的测试；
  - 补充子 agent 汇报消息被排除在 fork 列表外的测试；
  - 补充会话装载失败时正确返回 `on_fork_failed` 且不污染源会话的测试。

---

## 三、修复执行顺序规划

按照 P1 -> P2 -> P3 的顺序严格执行：

1. **第 1 阶段（P1 修复）**：
   - [x] P1-1: 拆分 `src/app/update/chat/session.rs`（拆出 `task_page.rs` 与 `slash.rs`），确保各文件严格控制在 500 行左右，低于 1000 行限制。
   - [x] P1-2: 修复 `src/db/service/chat.rs` 中的会话装载校验逻辑，避免向源会话回填与误报成功。
   - [x] P1-3: 修复 `src/ui/widgets/command_panel.rs` 的 tag 渲染保护与 `src/app/update/chat/fork.rs` 的显示宽度截断，确保长文本下时间列不消失。
2. **第 2 阶段（P2 修复）**：
   - [x] P2-1: `src/app/update/chat/fork.rs` 排除 `reports` 非空的子 agent 汇报消息。
   - [x] P2-2: `src/app/update/interaction.rs` 在 `next_deadline` 中增加打开面板时的时钟唤醒。
   - [x] P2-3: `src/app/state/lx/slash.rs` 修正过滤时默认高亮为第一条匹配消息。
   - [x] P2-4: 增强图片路径 JSON 转义改写与复制错误防护。
3. **第 3 阶段（P3 修复与文档同步）**：
   - [x] P3-1: 更新 `design.md` 与 `task.md`，修复结构体定义、文案清单、测试清单。
   - [x] P3-2 & P3-3: 消除重复测试辅助函数，优化 `fork_label`。
   - [x] P3-4: 补充缺失测试并跑通全部测试验证。
