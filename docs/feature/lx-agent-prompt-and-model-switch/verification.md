# lx Agent 系统提示词与模型切换 — Agent 自测验证报告

> 关联功能目录：`docs/feature/lx-agent-prompt-and-model-switch/`
> 测试环境：当前 worktree 运行 `cargo run`（开发环境 `~/.lx-t-dev`）
> 校验方式：用户向 lx Agent 发送提示词，完成后由外部 Agent 检查数据库 session 记录判断结果。

---

### Item 1：全局与项目级 `AGENTS.md` 注入自测（暗号校验）

- **输入内容**：
  - 测试输入：`ping-agents-md and ping-agents-project`
- **验证步骤**：
  1. 在 `cargo run` 启动的 `lx-tui` 中，确保当前工作区为本工程；
  2. 在输入框输入 `ping-agents-md and ping-agents-project` 并按回车发送；
  3. 观察并等待 Agent 回复完成。
- **期望结果**：
  - Agent 回复中严格包含全局暗号 `PONG: AGENTS_MD_INJECTED_SUCCESSFULLY` 与项目暗号 `PONG: PROJECT_AGENTS_MD_INJECTED_SUCCESSFULLY`。

---

### Item 2：会话中途模型切换与 `<system-update>` 接管自测

- **输入内容**：
  - 第 1 轮输入：`Hello! Who are you?`
  - 第 2 步操作：输入 `/model` 切换为另一模型（例如从当前模型切为另一家 provider/model 或思考等级）；
  - 第 2 轮输入：`What is your model now, and did you receive any system update?`
- **验证步骤**：
  1. 发送第 1 轮消息后，按 `/` 键输入 `/model` 并选定新模型（时间线出现 `-- Model switched --` 条目）；
  2. 发送第 2 轮提示词询问模型身份与更新；
  3. 观察 Agent 的回答并点击时间线上的 `-- Model switched --` 卡片展开查看。
- **期望结果**：
  - 展开卡片以压暗 Markdown 展示前后模型跃迁详情与注入提示词；
  - Agent 回复明确声明切换后的新模型身份，且明确指出 `[SYSTEM_UPDATE_RECEIVED: I have seamlessly taken over this conversation]`。

---

### Item 3：会话持久化与 `/resume` 恢复自测

- **输入内容**：
  - 第 1 步操作：输入 `/new` 新建会话；
  - 第 2 步操作：输入 `/resume` 并回车选择刚才完成切换的旧会话。
- **验证步骤**：
  1. 发送 `/new` 解绑会话；
  2. 发送 `/resume`，在列表中选中上一个完成模型切换的会话并按回车加载；
  3. 检查时间线上是否完好保留 `-- Model switched --` 卡片，且再次点击能够正常展开/折叠。
- **期望结果**：
  - 历史消息与 `-- Model switched --` 条目完整复原，位置与切换时间无漂移，点击展开显示完整注入细节。
