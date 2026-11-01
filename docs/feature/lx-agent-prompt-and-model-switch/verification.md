# lx Agent 系统提示词（AGENTS.md）注入与模型切换 — 验证报告

> 关联功能目录：`docs/feature/lx-agent-prompt-and-model-switch/`
> 验证基线：工作区 `.worktrees/lx-agent-prompt-and-model-switch`（分支 `feat/lx-agent-prompt-and-model-switch`，commit `10f68041`）
> 验证状态：**全部通过**（代码规范 0 警告、自动化测试全部通过）

---

### Item 1：全局与项目级 `AGENTS.md` 发现与系统提示词双层注入

- **输入内容**：
  - 全局用户根目录：`~/.lx-t[-dev]/AGENTS.md` 写入规则 `Global rule`；
  - 项目工作区根目录：`crates/core`（子目录）工作区，项目根目录含 `AGENTS.md`（`Root rule`），子目录含 `agent.md`（`Subdir rule`）；
  - 测试命令：`rtk cargo test chat::agent::tests::instructions` 与 `rtk cargo test chat::prompt::`。
- **验证步骤**：
  1. 调用 `ensure_global_agents_md` 验证全局候选文件不存在时自动创建 `~/.lx-t[-dev]/AGENTS.md`，已有 `agent.md` 时不覆盖；
  2. 调用 `load_instructions` 扫描目录链，并验证 `PromptParts` 拼接与 `PROMPT_VERSION = "lx-8"`；
  3. 执行 `rtk cargo test chat::agent::tests::instructions` 与 `rtk cargo test chat::prompt::`。
- **期望结果**：
  - 全局与项目级指令以 `\n\n--- project-doc ---\n\n` 分隔合并在 `<INSTRUCTIONS>` 标签内；
  - 优先级遵循 `AGENTS.override.md` > `AGENTS.md` > `agent.md`，共享 32 KiB 预算；未信任工作区拦截项目层；
  - 系统提示词按固定顺序组装：`# Your Model` → `environment` → `# AGENTS.md instructions` → `## Skills` → `## MCP servers` → `background` → `compact`。全部用例通过。

---

### Item 2：会话中途模型切换时间线持久化（v22）、连续切换合并与 `/resume` 恢复

- **输入内容**：
  - 会话包含消息，执行 `/model` 连续切换：从 `anthropic/claude-sonnet-4` 切至 `openai/gpt-5`，再切至 `9router/deepseek-v4.1-flash (max)`；
  - 测试命令：`rtk cargo test db::chat::model_switch`、`rtk cargo test db::migration::`、`rtk cargo test db::service::resume_tests::resume_restores_persisted_model_switches`。
- **验证步骤**：
  1. 验证 SQLite Schema 升至 v22，`agent_model_switch` 表与外键创建正确；
  2. 验证多表联合 `next_session_sequence` 保证消息与切换记录单调递增；
  3. 在测试会话中提交切换，使用 `new_session` 清空内存后通过 `/resume` 重载会话；
  4. 验证 `/rewind` 撤销轮次时级联清理该轮之后的切换记录；
  5. 运行上述单元测试与服务层集成测试。
- **期望结果**：
  - 空白未发消息会话切模型不产生切换记录；
  - 已有消息会话原子写入 `agent_model_switch` 并分配正确 `sequence`，连续切换在时间线合并为 `x2` 且首行显示初始至最终目标模型的跃迁；
  - `/resume` 成功还原模型切换时间线条目；真实开发库副本迁移验证 integrity check 通过。全部用例通过。

---

### Item 3：跨模型 `<system-update>` 提示词注入、思考标签清洗与图片降级

- **输入内容**：
  - 历史轮次包含带有图片的用户输入或工具结果；
  - 会话中途切换到无视觉模型（`supports_image_input == false`）；
  - 测试命令：`rtk cargo test chat::context::tests::model_switch` 与 `rtk cargo test event::chat::subagent::tests::replay_persisted_items`。
- **验证步骤**：
  1. 组装含模型切换记录的 `AssemblyInput`，验证新模型在草稿消息前收到 `<system-update>`；
  2. 验证跨模型回放历史时剥离旧模型 reasoning 原文与签名；
  3. 验证子 Agent 与主会话在目标模型缺少视觉能力时，将历史图片降级为英文文本占位且当前轮含图直接报错；
  4. 运行上下文装配与子 Agent 重放测试。
- **期望结果**：
  - 切换记录按 sequence 顺序在下一轮用户输入前转换为 `<system-update>` 块，转义闭合标签；
  - 历史图片安全降级为 `ERROR: Cannot read ... (this model does not support image input)`，历史轮次不阻塞当前推理；
  - 无论是否切换模型，旧模型的 `reasoning_metadata` 均不跨模型回传。全部用例通过。

---

### Item 4：TUI 历史渲染：`-- Model switched --` 折叠/展开与思考块压暗 Markdown 样式

- **输入内容**：
  - 包含单次及连续模型切换条目的窗格历史；
  - 包含反引号代码（`inline_code`）的模型身份与提示词正文；
  - 测试命令：`rtk cargo test ui::main_content::lx::history::tests::model_switch` 与 `rtk cargo test app::update::chat::tests::model_switch`。
- **验证步骤**：
  1. 测量折叠态渲染行数与文本内容（标题行 `style::accent()`、描述行 `style::muted()`）；
  2. 触发 `toggle_lx_card` 切换展开状态，测量展开态行数；
  3. 检查展开态中反引号代码与文本样式；
  4. 运行 TUI 几何与状态机测试。
- **期望结果**：
  - 折叠态严格占用 2 行内容（加末尾空行共 3 行）；
  - 点击整块可切换展开/折叠；
  - 展开态正文采用思考块同款的压暗 Markdown 渲染（`md_read_inline_code().patch(style::muted())`），无样式错位；
  - `compact` 展开态与 `model_switch` 展开态视觉风格完全统一。全部用例通过。
