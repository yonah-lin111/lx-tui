# lx `@` 提及 `Subagents` Tab、`lx-guide` 自省 Skill 与 `<subagent_reference>` — Review 与修复清单 (`review.md`)

- **特性目录**：`docs/feature/lx-mention-subagents-tab/`
- **Review 基线**：`feat/lx-mention-subagents-tab` @ `a293fb3c`（worktree `.worktrees/lx-mention-subagents-tab`；分支基点 `42e70535`，合并前 `dev` 为 `fac77938`）
- **修复提交**：`236145a7`（`fix(lx): close subagent mention and lx-guide review findings`）
- **合并提交**：`b8d8533c`（`Merge branch 'feat/lx-mention-subagents-tab' into dev`）
- **关联文档**：
  - [design.md](design.md)
  - [task.md](task.md)
  - 项目规范：`docs/standards/{project-directory-structure,code-writing-standards,tui-design-requirements,html-assets,web-deepseek}.md`
- **Review 范围**：
  - `@` 面板第 5 个 `Subagents` Tab 的候选排序、键盘/鼠标交互、窄窗格可达性与 4-Tab 隔离
  - `<subagent_reference>` 的角色/实例解析、字段安全、预算与 TUI/headless 注入链路
  - 内嵌 `lx-guide` 系统 Skill 的安装、发现、只读语义、设置页/CLI 管理与文档准确性
  - 多窗格 `@` 快照（skills / MCP / subagent roles）生命周期与状态同步
- **Review 方式**：`codegraph` + `codebase-memory-mcp` 交叉检索调用链，另委派 3 个并行子 Agent 分别审计「提及 UI 与多窗格状态」「`<subagent_reference>` 解析与注入」「`lx-guide` / Skills 管理」。
- **结论**：发现 **1 个 P0、5 个 P1、12 个 P2、8 个 P3**，已全部按优先级修复并通过精准回归。

---

## 0. 优先级汇总

| 级别 | 数量 | 判定标准 | 状态 |
| :--- | :---: | :--- | :---: |
| **P0** | 1 | 会话内已结束子 Agent 的引用丢失身份与报告，主模型被误导重新派发 | 已修复 |
| **P1** | 5 | `@` 快照缺失/过期、新窗格丢失角色、自省 skill 触发词被截断、安装非原子、设置页技能路由写错目录 | 已修复 |
| **P2** | 12 | 注入转义与预算、工具面口径、headless 缺口、合成轮误注入、昵称歧义、窄窗格不可达、性能与设置页语义 | 已修复 |
| **P3** | 8 | 状态归一化、去重键、图标区分、`unreachable!()`、文档与事实漂移 | 已修复 |

---

## 1. P0（关键路径缺陷）

### [P0-1] `src/event/chat/subagent/snapshot.rs:198-291` — 实例在发送前结束会被静默降级为角色规格，丢失 `agent_id` / `task` / `latest_report`

- **关键点**：
  - 用户从 `@` 面板插入 `@[refer-subagent](Turing:explorer)` 时 Turing 为 `running`；输入问题期间 Turing 完成，`agents.runs` 中状态变为 `completed`（或 `failed` / `interrupted` / `cancelled`），但输入框里的 marker 不会变。
  - 原实现只按 `AgentFilter::Active` 在 `runs` 中查找实例，终态实例匹配失败后回退到 `resolve_role("explorer")`，生成 `kind: role`。
  - 结果：`agent_id`、`nickname`、真实 `status`、`agent_path`、`title`、`task`、`latest_report` 全部丢失；guidance 变成“调用 `spawn_agent` 重新派一个 explorer”，而不是回答报告或 `resume_agent` / `send_input` 原实例，可能重复派发。若后缀是已删除的自定义角色，则一个块都不生成，marker 以纯文本留在 prompt 中。
- **错误原因**：把 UI 展示筛选器 `AgentFilter::Active` 复用为“实例是否存在/可引用”的语义判断；没有“实例存在但已结束”这一分支。
- **修复**：
  - `find_mentioned_run` 先找活动实例，找不到时按 `runs.iter().rev()` 查找最近的非活动实例；实例存在即输出 `kind: finished_instance`，保留 `agent_id`、`status`、`title`、`task`、`latest_report`。
  - `finished_instance` 的 guidance 要求先基于 `latest_report` 回答；后续工作使用 `resume_agent` / `send_input` 复用原实例，禁止 spawn 重复子 Agent。
  - 仅当 `runs` 中完全不存在该实例时才回退到角色规格。
- **验证**：
  - `src/event/chat/tests/subagent_roles.rs::subagent_reference_blocks_preserve_finished_instance_when_completed`

---

## 2. P1（核心功能与数据安全）

### [P1-1] `src/app/state/lx/panels.rs:136-175,263-281`、`src/event/chat/subagent/{settle,runner,launch,mod}.rs`、`src/app/update/chat/subagent.rs` — `subagent_runs` 快照在 settle/队列/恢复/关闭后未同步，存活实例从 `Subagents` Tab 消失

- **关键点**：
  - `on_subagent_finished` 在生命周期状态机尚未跑完时就同步了 mention 快照（此时 `completed` / `cancelled` 会先被移出 Active 列表）。
  - 随后 `settle.rs` 把可恢复实例改回 `idle`（158-161、211-214、243-246），`runner.rs::sync_agent_ui_queue` 又会投影为 `idle` / `blocked` / `running`，`launch_child_turn` 恢复实例为 `running`，`close_agent` 把 `idle` 改成 `cancelled`——这些 `run.status` 写入点都没有再次同步 mention。
  - 用户停留在 `Subagents` Tab 时，列表可能缺少仍存活的 idle 子 Agent，或长期显示过期的 `running`；只有再次编辑输入才会在 `refresh_panel` 中顺带纠正。
- **错误原因**：`subagent_runs` 是独立快照，但同步点只在 `on_subagent_started` / `on_subagent_finished` / `set_subagent_list`，远少于 `run.status` 的实际写入点，且同步发生在状态机中间态。
- **修复**：
  - 在 `settle` 状态流转后、`sync_agent_ui_queue` 末尾、`launch_child_turn` 恢复/新建分支、`close_agent` 的 `idle -> cancelled` 分支补 `sync_mention_subagent_runs`。
  - `mention_switch_tab` / `mention_set_tab` / `mention_force_tab` 切换前先同步，保证打开/切到 `Subagents` 时立即反映最新状态。
  - `on_subagent_started` 的同步移到 `restore_selected_task_agent`（树序排序）之后。
- **验证**：
  - `src/app/state/lx/mention/tests.rs::switching_to_subagents_tab_refreshes_runs_without_typing`

### [P1-2] `src/app/state/pane.rs:88-153`、`src/app/update/workspace.rs:91-199,346` — 新建标签页/终端标签/同路径工作区拿不到已加载的自定义角色与 `@` 快照

- **关键点**：
  - `create_tab`、`create_terminal_in_workspace` 都走 `Pane::new()` → `Lx::new()` → `LxMention::default()`，其中 `subagent_roles` 只有 3 个内置角色；`skills` / `mcp_servers` 为空列表。
  - 之后没有“新窗格创建”事件触发刷新；`agents_md_scopes` / `skill_roots` 对相同路径去重，因此同路径新工作区也不会重扫。
  - `Tab::split_pane` 只复制源窗格快照；若源窗格本身就是新标签页，复制到的仍是默认 3 个角色。
- **错误原因**：快照按窗格保存，但初始化值来自配置默认值而非工作区生效配置；事件层没有“窗格创建后补齐”入口，设计假设分割窗格是唯一的新窗格来源。
- **修复**：
  - 新增 `Pane::copy_mention_inventory_from(&mut self, source: &Pane)`，统一复制 `referenced_folders` / `skills` / `mcp_servers` / `subagent_roles`。
  - `Tab::split_pane`、`create_tab`、`create_terminal_in_workspace`、同路径 `create_workspace_with_cwd` 均从源窗格/同路径工作区根窗格复制。
- **验证**：
  - `src/app/update/tests.rs::create_tab_inherits_mention_inventory_from_source_pane`

### [P1-3] `src/skills/assets/lx-guide/SKILL.md:2` — description 495 字符被 `## Skills` 目录截断到 200，后半段触发词全部丢失

- **关键点**：`MAX_CATALOG_DESCRIPTION_CHARS = 200`（`src/skills/mod.rs:61`，`src/skills/render.rs:52` 截断）。原 description 中的 `subagent roles`、`skills`、`MCP servers`、`settings`、`keybindings`、`troubleshooting` 以及“用户问 you/lx/this agent 时使用”均在截断点之后。用户问“subagent 角色怎么配”“skill 放哪”“MCP 怎么加”时，模型看不到这些关键词，`lx-guide` 很可能不会被选中，内置自省价值失效。
- **错误原因**：描述按“详尽文档摘要”写，而非按“目录触发词”写；没有资产长度约束测试。
- **修复**：压缩到 ≤ 195 字符并前置高信号触发词；`install_system_skills_is_idempotent_and_discovered_as_readonly_system_skill` 增加 `description.chars().count() <= MAX_CATALOG_DESCRIPTION_CHARS` 断言。

### [P1-4] `src/skills/system.rs:47-70`、`src/{event/chat/skills,settings/service/skills,cli/skill/exec,chat/agent/exec/mod}.rs` — 系统 skill 安装非原子、无锁、每次扫描整目录删除重建

- **关键点**：
  - 原实现先 `remove_dir_all(.system)` 再逐个 `fs::write`，期间并发读者（`build_job`、`GET /api/skills`、`catalog`）会看到 `.system` 不存在或 references 缺失，`lx-guide` 短暂消失，模型 `read` 引用文件可能失败。
  - 两个不同版本的二进制（升级后旧 TUI 仍在运行）会互相删除重建，形成写盘风暴；交错写可能留下撕裂文件而 marker 仍匹配，之后不会自愈。
  - `.system` 被替换成文件/符号链接时安装永久失败；四处调用点全部 `let _ =` 静默吞错。
- **错误原因**：把“内容寻址的幂等安装”实现成“整目录删除重建”，并放在每次扫描/请求的热路径；无文件锁、无临时文件 + rename。
- **修复**：
  - 新增进程内 `INSTALL_LOCK` 串行化安装。
  - `.system` 为文件/符号链接时先 `remove_file`；不再删除 `.system` 整目录，改为 `create_dir_all` + 逐文件 `platform::write_file_atomic`（同目录临时文件 + rename）原子替换，保留 `.system` 下其它条目。
  - 四类调用点（`event/chat/skills.rs`、`settings/service/skills.rs`、`cli/skill/exec.rs`、`chat/agent/exec/mod.rs`）统一 `tracing::warn!` 记录安装失败。
- **验证**：
  - `src/skills/discover/tests.rs::install_system_skills_is_idempotent_and_discovered_as_readonly_system_skill`（含 sentinel 保留、reference 恢复与指纹幂等断言）

### [P1-5] `assets/settings_skills.js:22,376,454`、`src/settings/service/skills.rs:144-160` — 设置页按 frontmatter `name` 而非目录名路由，PUT/DELETE 会写错/删错目录

- **关键点**：
  - `SkillEntry.name` 来自 frontmatter，目录名只用于缺省值；设置页却把 `entry.name` 当作服务端路径参数，服务端再把该参数当目录名（`write_skill(root, name, ...)` / `delete_skill(root, name)`）。
  - 目录 `foo/SKILL.md` 中 `name: bar` 时，编辑后 `PUT /api/skills/user/bar` 会创建/覆盖 `skills/bar`，原 `foo` 不变并被遮蔽，用户看到“保存成功”但实际未生效；Delete 会删错 `bar`。该缺陷会放大“同名覆盖内置 `lx-guide`”场景。
- **错误原因**：路由标识选错，把模型可见的 `name` 当成文件系统主键；`entry_json` 未暴露目录名。
- **修复**：`entry_json` 增加 `dir`（`SKILL.md` 父目录名）；设置页记录 `dir`，PUT/DELETE 使用 `dir || name`，保证只影响真实目录。
- **备注**：这是 `dev` 已有的设置页缺陷，但与本次“用户同名覆盖内置 skill”直接相关，故纳入修复。

---

## 3. P2（功能边界、注入安全、性能与文档）

### [P2-1] `src/event/chat/subagent/snapshot.rs:240-291,547-619` — `<subagent_reference>` 动态字段未转义、未单行化、无总量上限

- **关键点**：自定义角色的 `description` / `systemPrompt` 可含换行；`systemPrompt` 最长 16,000 字符且原样从第 0 列输出。只要包含 `</subagent_reference>` 就能提前闭合块，把后续文本变成主 agent 的自由指令；项目级角色来自仓库 `.lx/agents/*.json`，属于仓库可控的间接提示注入面。`latest_report` / `task` 虽被单行化，但同样未转义。
- **错误原因**：把伪 XML 当作可靠边界，所有动态字段直接 `format!` 插值；没有控制字符过滤、闭合标签转义与预算控制。
- **修复**：所有标量字段单行化 + 剥离控制字符（`system_prompt` 保留换行）+ 大小写不敏感转义 `</subagent_reference`；`task` / `description` 600 字符、`latest_report` 2400 字符、`system_prompt` 4000 字符，超限追加 `[truncated]`；单轮最多 8 个块。

### [P2-2] `src/event/chat/subagent/snapshot.rs:318-323` — 角色块 `tools` 只报叶子工具面，`executor` 实际可能是 15 项的 `ExecutorCoordinator`

- **关键点**：默认 `max_depth=2` 时，`executor` 子 Agent 会获得 7 个实现工具 + 8 个控制工具；原块只列 7 个，与 `lx-guide/references/tools.md` 的 `ExecutorCoordinator (15)` 矛盾。`allow_mcp=true` 时动态追加的 `mcp__*` 也未体现。
- **错误原因**：生成块时没有父深度上下文，直接使用 `ToolSurface::from_role`。
- **修复**：`max_depth > 1` 时在 `tools` 行追加 8 个控制工具说明；`capabilities` 单独列 `allow_mcp` / `allow_skills`，不再声称是最终工具面。

### [P2-3] `src/chat/agent/exec/mod.rs:129-145` — `lx-tui exec` 完全不注入 `<subagent_reference>`

- **关键点**：headless 路径只调用 `skill_reference_blocks`，`@[refer-subagent](explorer)` 作为纯文本留在 prompt 中，模型拿不到角色规格或实例详情。
- **错误原因**：新 block 生成只接在 TUI 的 `start_stream`，没有接入 exec。
- **修复**：exec 同样调用 `subagent_reference_blocks(&prepared.raw_text, &subagents_config, &[])`（实例列表为空，仅角色规格）。

### [P2-4] `src/event/chat/mod.rs:583-620` — 子 Agent 报告合成的 user 消息会触发 `@[refer-*]` 块提取

- **关键点**：自动报告轮把报告 envelope 以 `role=user` 写入历史；`start_stream` 从“最后一条 user 消息”提取草稿。报告正文若包含 `@[refer-subagent](...)`（例如子 Agent 读过本项目文档），父 Agent 的自动报告轮会注入对应块，即使用户从未点名。
- **错误原因**：`latest_user_draft_and_attachments` 未区分真实用户输入与运行时合成消息。
- **修复**：新增 `is_synthetic_report_turn`；合成报告轮跳过 skill 与 subagent 引用块提取。

### [P2-5] `src/config/subagents.rs:660-675`、`src/event/chat/subagent/snapshot.rs:167-175` — 昵称含 `:` 破坏 `<nickname>:<role>` 解析；裸角色名可能被同名实例抢先命中

- **关键点**：昵称校验只禁止逗号和（新增前）部分字符，允许 `:`；昵称 `Bot:1` 插入 `Bot:1:explorer` 后 `split_once(':')` 得到 `Bot` 与 `1:explorer`，实例和角色都匹配失败。反向问题：裸 `@[refer-subagent](explorer)` 先按实例匹配，若某实例昵称恰为 `explorer`，会注入错误的实例块。
- **错误原因**：编码分隔符无保留约束，且解析顺序把实例放在角色之前。
- **修复**：昵称额外禁止 `:` / `(` / `)`；解析改用 `rsplit_once(':')`；裸 target 且能 `resolve_role` 时直接按角色解析，含 `:` 才查实例。

### [P2-6] `src/ui/widgets/command_panel.rs:122-330,475-480`、`src/ui/main_content/lx/mention.rs:309-313` — 窄窗格丢弃第 5 个 Tab 且鼠标不可达

- **关键点**：完整 5 Tab 约需 54 列；80 列终端左右分屏后输入框约 36 列，原布局从索引 0 开始、放不下就 `break`，`MCP` 与 `Subagents` 被丢弃且无 overflow 提示；`tab_at` 只能命中实际渲染的 Tab，`Subagents` 完全点不到。底部提示还只看 `tabs[0].active`。
- **错误原因**：Tab 头无溢出策略，布局与命中测试未随第 5 个 Tab 升级。
- **修复**：`TabSlot` 增加原始 `index`；布局按活跃 Tab 计算滑动窗口起点，保证活跃 Tab 始终可见；`tab_at` 返回原始下标，鼠标映射正确；底部提示改用 `lx.mention_tab() == MentionTab::Files`。
- **验证**：`src/ui/main_content/lx/tests/slash_panels.rs::mention_subagents_tab_stays_visible_in_narrow_pane`

### [P2-7] `src/app/state/lx/mention.rs:66-80,281-313` — `filtered_subagents` 每帧深拷贝完整 `ResolvedRoleSpec`（含最多 16k 系统提示词）

- **关键点**：UI 的 `panel_data` 在 `layout`、`render`、`item_at` 中都会调用 `filtered_subagents`，原实现把每个角色整体 `.cloned()`；自定义角色多时每帧多次分配大字段，造成明显抖动。
- **错误原因**：展示层需要 owned 值，直接克隆了完整配置对象。
- **修复**：`SubagentMentionItem<'a>` 改为借用 `&SubagentRunMentionEntry` / `&ResolvedRoleSpec`，渲染与计数不再深拷贝。

### [P2-8] `assets/settings_skills.js:404-408` — 新建技能的重名检查把 system/只读条目也算重复，无法从 UI 覆盖内置 `lx-guide`

- **关键点**：内置 `lx-guide` 的 `scope` 是 `user`，新建 `user/lx-guide` 会被前端判为重复并拒绝；但服务端 `PUT` 实际允许创建并遮蔽内置技能，设计与实现、UI 与服务端不一致。
- **错误原因**：查重使用“作用域 + 名字”，未区分可编辑与只读/被遮蔽条目。
- **修复**：查重只针对同作用域且 `item.editable` 的条目，允许用户创建覆盖内置 skill 的同名技能。

### [P2-9] `src/settings/service/skills.rs:276-289` — 删除不存在的用户 `lx-guide` 返回含糊的 `422 read failed`

- **关键点**：无同名用户技能时，DELETE 命中 `read_dir` 的 ENOENT，返回 `422 read failed: No such file or directory`，而不是“内置只读技能”。
- **错误原因**：删除路径缺少“名字由 System/Agents 提供”的语义判断。
- **修复**：可写目录不存在时先发现同名 `System` / `Agents` 条目，命中则返回 `403 built-in or .agents skill is read-only`。

### [P2-10] `src/skills/assets/lx-guide/references/{commands-and-mentions,subagents}.md` — 自省手册存在多处事实漂移

- **关键点**：
  - 声称 `/` 命令只识别“无空格单行”，但 `/rename <title>`（别名 `/title`）在提交阶段允许空格标题。
  - “project-scoped roles and skills stay inside the workspace” 不成立：项目 lx skills 在 `<data_root>/skills/projects/<scope>/`，项目 MCP 在 `<data_root>/config/mcp/projects/<scope>.json`；只有 `.agents/skills`、`.lx/agents`、`.agents/roles` 在 workspace。
  - CLI 示例 `lx-tui skill` / `lx-tui settings` 裸命令会报缺少子命令，应为 `skill list` / `settings show`。
  - `spawn_agent` / `send_input` 的参数面完全缺失。
- **错误原因**：把面板触发条件当成命令提交条件，混用两类存储模型，且把 subagent 工具当成“控制语义”而非 schema 记录。
- **修复**：修正 `/rename` 说明、补充五级 skill 存储路径与优先级表、修正 CLI 示例并补充 `$<skill-name>`、补全 `spawn_agent` / `send_input` 参数表与 `finished_instance` 续作说明。

### [P2-11] `src/app/state/lx/mention.rs:437-442` — `view()` 的全空 `Hidden` 判定会把 `Subagents` 一起隐藏

- **关键点**：当 `root=None`、`referenced_folders` / `skills` / `mcp_servers` 全空且当前为 `Files` 时返回 `Hidden`；而 `switch_tab_for_entry` 要求 `view != Hidden`。新标签页的 skills/MCP 恰好为空时，用户无法从 `Files` 切到始终有内容的 `Subagents`。
- **错误原因**：Hidden 判定未考虑新增的 subagent 候选。
- **修复**：`subagent_roles` 与 `subagent_runs` 均空才隐藏。

### [P2-12] `src/app/state/lx/mention.rs:479-491` — 任意 Tab 的 `@` 输入都会请求工作区文件扫描

- **关键点**：`take_scan_request` 不检查当前 Tab，只要存在 `@` 触发词就请求扫描；`Subagents` / `Skills` / `MCP` 不需要文件候选。
- **错误原因**：新增 Tab 沿用了 `Files` 的扫描触发逻辑。
- **修复**：仅在 `MentionTab::Files` 发起扫描；切回 `Files` 时下一轮自然再请求。

---

## 4. P3（状态细节、整洁度与文档精度）

| 编号 | 位置 | 关键点 / 错误原因 | 修复 |
| :--- | :--- | :--- | :--- |
| **P3-1** | `src/app/prompt/mention.rs:614` | `MentionState::confirm` 对 `Files \| Subagents` 使用 `unreachable!()`，未来改动可能触发 panic | 改为 `return None` |
| **P3-2** | `src/app/state/lx/panels.rs:136-160` | `run.status` 原样写入 `Active · <status>`，DB 旧别名 `pending` / `complete` 会直接显示 | 经 `AgentStatus::parse(...).map(as_str).unwrap_or(raw)` 归一化 |
| **P3-3** | `src/app/update/chat/subagent.rs:50-58` | spawn 时先同步 mention 再排序树序，`@` 列表顺序可能与 Agents 面板不一致 | 同步移到 `restore_selected_task_agent` 之后 |
| **P3-4** | `src/event/chat/subagent/snapshot.rs:152-182` | 去重键大小写敏感，同一实例用昵称与 `agent_id` 双引用会生成两个块 | `raw_target` 小写化去重 + `seen_agent_ids` 按 `agent_id` 去重 |
| **P3-5** | `src/event/chat/subagent/snapshot.rs:563-619` | `task` / `report` 共用 600 字符且无截断提示，长报告只剩开头却不自知 | `latest_report` 放宽到 2400 并追加 `...[truncated]`；`system_prompt` 单独 4000 |
| **P3-6** | `src/ui/mention.rs:124-132`、`src/ui/text.rs:151-152` | `Subagents` 行复用 skill 的 `◆` 图标与配色，与 `Skills` / `MCP` 视觉同类 | 新增 `MENTION_SUBAGENT_ICON = "●"`，subagent 行使用独立图标与普通文本配色 |
| **P3-7** | `src/skills/system.rs:52-54` | `.system` 被替换为文件/符号链接时 `remove_dir_all` 永久失败且被静默吞掉 | 先 `remove_file`，并对安装错误 `tracing::warn!` |
| **P3-8** | `docs/feature/lx-mention-subagents-tab/{design,task}.md`、`references/tools.md` | `design.md` 仍保留 §3.5 简化版伪代码（无 tools/system_prompt）；`task.md` 内嵌文件清单漏 `references/tools.md`；`tools.md` 关于 `*** End Patch` 尾随内容与 `!` 回放截断字节数的描述不准确 | 重写 §3.5/§3.6 并新增 §3.7；`task.md` 补 T7 与 4 个内嵌文件；修正 `tools.md` 事实 |

---

## 5. 复审与合并后验证

### 5.1 合并前（worktree）

- `rtk cargo fmt --check`：通过
- `rtk cargo clippy --all-targets -- -D warnings`：通过（0 warnings）
- 受影响模块精准回归（`skills::discover`、`event::chat::tests::subagent_roles`、`settings::service::tests`、`cli::tests::config_domains_tests`、`app::state::lx::mention`、`chat::prepare`、`ui::main_content::lx::tests`、`app::update::tests`）：**461 passed, 0 failed**
- `rtk cargo test --lib`：**4918 passed, 1 failed, 4 ignored**
- 唯一失败项 `db::chat::subagent::tests::dev_database_copy_migrates_to_current_version_and_passes_integrity_check` 报 `UnsupportedSchema(25)`：分支基点为 `dev@42e70535`（`SCHEMA_VERSION = 24`），开发库已被最新 `dev` 迁移到 v25；在 `dev` 上运行同一测试通过，确认与环境版本错位有关，与本特性代码无关。

### 5.2 合并后（dev）

- 合并方式：`git merge --no-ff feat/lx-mention-subagents-tab`；11 个双方均修改的文件（含 `src/event/chat/mod.rs`、`src/app/state/{pane,lx}.rs`、`src/ui/text.rs`、`src/app/update/{tests,workspace,chat/subagent}.rs`）全部自动合并，无冲突。
- `rtk cargo fmt --check`：通过
- `rtk cargo clippy --all-targets -- -D warnings`：通过（0 warnings）
- `node --check assets/settings_skills.js`：通过
- `rtk cargo test`：**5047 passed, 4 ignored, 0 failed**（16 个测试套件；`SCHEMA_VERSION = 25`，原 `UnsupportedSchema(25)` 问题随合并消失）

---

## 6. 剩余风险与未验证

- **交互式外观**：窄窗格 Tab 滑动、`●` subagent 图标、设置页 `system (built-in)` 行的观感需用户 `cargo run` 确认。
- **并发安装**：`INSTALL_LOCK` 只覆盖进程内并发；多进程同时安装不同版本仍可能互相覆盖，但逐文件原子写保证读者不会看到半文件，且下次启动会按指纹自愈。跨进程文件锁未引入（当前单机单实例为主要场景）。
- **`.system` 用户自建技能**：安装不再删除 `.system` 整目录，但发现器仍跳过隐藏目录，用户放入 `.system` 的自定义技能不会出现在目录中（与既有行为一致）。
- **角色 `system_prompt` 注入**：已限制长度并转义闭合标签，但仍以 user 文本形式进入父上下文；若需彻底隔离，后续可改为 JSON 字段或独立不可信数据围栏。
