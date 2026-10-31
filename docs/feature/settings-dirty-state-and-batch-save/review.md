# 设置页（Settings）多层级未保存状态与一键保存 — 功能评审（Review）

> 审查对象：`feat/settings-dirty-state-and-batch-save`（工作区 `.worktrees/settings-dirty-state-and-batch-save`）。
> 关联文档：[design.md](design.md)、[task.md](task.md)。
> 状态：**评审完成，P0 ~ P2 全部缺陷已定位并修复**（2026-10-31）。

---

## 0. 评审结论

多层级草稿保存（`General` / `Commands` / `Models` / `MCP` / `Skills` 五大 Tab 与四个子列表小 Tab）、右侧未保存小黄点（`.nav-item.dirty::after`）、`Cancel` 单项撤销与右上角 `Reset` 整页还原、以及 `saveAllDirty()` 跨 Tab 串联保存的主体机制已跑通。

结合 `codegraph` 追踪前端脚本与后端 `src/settings/service/` 路由契约后，发现以下 **P0 ~ P2** 边界缺陷：

1. **P0（数据覆盖与保存报错阻断）**：
   - **P0-1**：`Commands` 切换命令类型为 `Template block (agentBlock)` 时未同步将 `Visibility (md_scope)` 锁定为 `template`，触发后端 `422: agentBlock requires template scope` 拒绝保存；且新建命令或跨类型修改命令时未校验目标 `(kind, name)` 是否已存在，会静默调用 `PUT`（后端 `overwrite = true`）覆盖已有的同名命令。
   - **P0-2**：`MCP` 与 `Skills` 在保存新建草稿（`+ New server` / `+ New skill`）时未检查同作用域下是否已存在同名条目，直接 `PUT` 会静默覆盖磁盘已有配置并冲掉该条目的内存草稿。
2. **P1（状态失步、异步竞态与刷新失效）**：
   - **P1-1**：`Skills` 页面点击 `Reload`（`loadSkills(true)`）时，`syncSkillBaselines` 仍复用了旧 `skillBaselines` 中已缓存的 `body` 与 `description`（`prevBase.bodyLoaded === true`），导致磁盘上修改过的 `SKILL.md` 正文无法通过 `Reload` 刷新。
   - **P1-2**：`Models` 页面在某 Provider 下点击 `Fetch models` 展开候选列表（`#fetch-candidates`）后，切换到其它 Provider、点击 `Cancel` 或点击右上角 `Reset` 时未调用 `hideFetchCandidates()`，导致上一个 Provider 的远端候选残留并在点击 `Add` 时写入错误的 Provider。
   - **P1-3**：`MCP` 页面的 `mcpProbe()` 在异步请求返回时直接覆写全局 `mcpToolNames`，若用户在探测期间切换到另一 Server（或处于未落盘的 `+ New server` 状态），会将工具列表串写到当前 Server 或请求不存在的路由。
3. **P2（高频重复计算与按钮状态一致性）**：
   - **P2-1**：`refreshActionBars()` 在每次输入事件中对每个 `actionBar` 重复调用 `anySettingsDirty()` 与 `controller.isDirty()`，导致单次按键触发多达 36 次 `sync*EditorToDraft()` 与 `JSON.stringify` 序列化比对。
   - **P2-2**：`Commands` 与 `MCP` 在新建模式（`+ New command` / `+ New server`）下未禁用编辑器头部的红色 `Delete` 按钮，与 `Skills` 新建模式禁用 `Delete` 的交互不一致。

以下按 **P0 → P1 → P2** 优先级逐项列出关键点、触发条件、错误原因与修复方案。

---

## 1. 核心数据流（Codegraph 梳理）

```text
用户输入 / 选择变化 (input / change / createSelect.choose)
  └─▶ refreshActionBars()
        ├─ updateCommandDirtyDots() ──▶ syncCommandEditorToDraft() ──▶ commandDrafts vs commandBaselines
        ├─ updateProviderDirtyDots() ─▶ computeModelsDirty()       ──▶ modelsDraft vs modelsBaseline
        ├─ updateMcpDirtyDots()     ──▶ syncMcpEditorToDraft()     ──▶ mcpDrafts vs mcpBaselines
        ├─ updateSkillDirtyDots()   ──▶ syncSkillEditorToDraft()   ──▶ skillDrafts vs skillBaselines
        └─ 汇总 dirtyByTab[key] & anyDirty ──▶ 刷新 5 个大 Tab 左侧黄点 + 操作条状态 + 子列表右侧黄点

右上角 Save 点击
  └─▶ saveAllDirty()
        ├─ saveGeneral()   ──▶ PUT /api/config                     ──▶ loadGeneral()
        ├─ saveCommand()   ──▶ POST .../rename + PUT /api/commands ──▶ reloadCommands(false)
        ├─ saveModels()    ──▶ PUT /api/models/settings            ──▶ loadModels()
        ├─ saveMcpServer() ──▶ PUT /api/mcp/<scope>/<name>         ──▶ loadMcp(false)
        └─ saveSkill()     ──▶ PUT /api/skills/<scope>/<name> + PUT /api/skills/enabled ──▶ loadSkills(false)
```

---

## 2. 问题清单与根因分析（P0 – P2）

### P0（阻断性缺陷：保存 422 报错、同名静默覆盖数据）

#### P0-1：`Commands` 切换为 `agentBlock` 保存报 422，且新建/跨类型改名未拦截同名覆盖

- **关键点**：`assets/settings.js`（`initCommandSelects`、`applyEditorValues`、`saveCommand`）、`src/settings/service/handlers.rs:501-525`（`put_command`）。
- **触发条件**：
  1. 点击 `+ New command`（或编辑已有 `agentMD` 命令），将 `Type` 切换为 `Template block`（`agentBlock`），不手动修改 `Visibility`（保持默认 `Global`），点击右上角 `Save`；
  2. 点击 `+ New command`，输入已存在的同类型命令名（如 `alpha`），或将已有命令切换 `Type` 到另一类型下已存在的同名命令，点击 `Save`。
- **错误原因**：
  1. 后端 `put_command`（`src/settings/service/handlers.rs:501`）严格校验 `if kind == CommandKind::AgentBlock && md_scope == MdScope::Global { return Response::error(422, "agentBlock requires template scope"); }`，而前端 `commandSelects.kind` 切换为 `agentBlock` 时未自动将 `md_scope` 设为 `'template'`，导致保存直接报错 422；
  2. 后端 `PUT /api/commands/user/<kind>/<name>` 调用 `commands::save(..., overwrite = true)`。只有同类型改名走了 `POST .../rename`（带冲突检测），而新建草稿 `saveNew` 和跨类型迁移直接走 `PUT`，未在前端检测 `commandBaselines.has(targetKey)`，会静默覆盖已有同名命令。
- **修复实现**：
  - 在 `initCommandSelects()` 的 `commandSelects.kind` `onChange` 与 `applyEditorValues()` 中：当 `kind === 'agentBlock'` 时自动将 `commandSelects.mdScope` 设为 `'template'` 并 `setDisabled(true)`；切回 `'agentMD'` 时恢复 `setDisabled(false)`；
  - 在 `saveCommand()` 中：对跨类型或改名的已有命令（`nextKey !== key`）以及新建命令（`saveNew`），若 `commandBaselines.has(targetKey)` 为真，立即定位到该项并抛出 `new Error('Command "' + name + '" already exists')`，阻止静默覆盖。

#### P0-2：`MCP` 与 `Skills` 新建草稿保存时未检测同作用域同名冲突，静默覆盖已有配置

- **关键点**：`assets/settings_mcp.js`（`saveMcpServer`）、`assets/settings_skills.js`（`saveSkill`）。
- **触发条件**：
  1. 在 `MCP` 页点击 `+ New server`，填写与当前 `scope` 下已有 Server 相同的 `name`，点击 `Save`；
  2. 在 `Skills` 页点击 `+ New skill`，填写与当前 `scope` 下已有 Skill 相同的 `name`，点击 `Save`。
- **错误原因**：
  - `PUT /api/mcp/<scope>/<name>` 与 `PUT /api/skills/<scope>/<name>` 均为幂等覆写接口；`saveMcpServer()` 与 `saveSkill()` 的 `saveNew` 分支未检查目标 `(scope, name)` 是否已存在于 `mcpBaselines` / `skillBaselines` 中，直接发送 `PUT` 覆盖原配置。
- **修复实现**：
  - `saveMcpServer()` 在 `saveNew` 分支先检查 `if (mcpBaselines.has(mcpServerKey(scope, name)))`，若存在则调用 `newMcpServer()` 定位并抛出 `new Error('MCP server "' + name + '" already exists')`；
  - `saveSkill()` 在 `saveNew` 分支先检查 `Array.from(skillBaselines.values()).some((item) => item.scope === scope && item.name === name)`，若存在则调用 `newSkill()` 定位并抛出 `new Error('Skill "' + name + '" already exists')`。

---

### P1（主要缺陷：Reload 不刷新正文、远端候选残留、异步探测串台）

#### P1-1：`Skills` 点击 `Reload` 按钮时不重新拉取已加载过的 `SKILL.md` 正文

- **关键点**：`assets/settings_skills.js`（`syncSkillBaselines`、`loadSkills`、`selectSkill`）。
- **触发条件**：在 `Skills` 页面点开某个 Skill（此时 `base.bodyLoaded = true`），外部或磁盘更新了该 `SKILL.md` 的正文或描述后，点击工具栏 `Reload`（`loadSkills(true)`）。
- **错误原因**：
  - `syncSkillBaselines(entries, discardDrafts)` 在 `discardDrafts === true` 时虽然清空了 `skillDrafts`，但仍复制了 `const previous = new Map(skillBaselines)` 并传入 `entryToSkillRecord(entry, previous.get(entry.path))`；
  - `entryToSkillRecord` 看到 `prevBase.bodyLoaded === true`，直接沿用了旧内存里的 `prevBase.description` 和 `prevBase.body`，导致 `Reload` 按钮对已打开过的 Skill 正文完全无效。
- **修复实现**：
  - 当 `discardDrafts === true` 时，`syncSkillBaselines` 不复用 `previous`（所有条目的 `bodyLoaded` 重置为 `false`）；
  - 若当前正选中某个已有 Skill（`!skillsNew && skillsSelected`），在 `loadSkills(true)` 完成后自动重新调用 `selectSkill(skillsSelected)` 拉取最新 `/api/skills/detail`。

#### P1-2：`Models` 切换 Provider、点击 `Cancel` 或 `Reset` 时未清理 `#fetch-candidates` 远端模型候选框

- **关键点**：`assets/settings_models.js`（`selectProvider`、`cancelProvider`、`resetModels`）。
- **触发条件**：在 Provider A 下点击 `Fetch models` 展示远端候选列表后，点击左侧 Provider B、或点击 `Cancel` / `Reset`。
- **错误原因**：
  - `selectProvider()`、`cancelProvider()`、`resetModels()` 均未调用 `hideFetchCandidates()`，导致 `#fetch-candidates` 跨 Provider 常驻，且其中 `Add` 按钮闭包仍绑定在旧 Provider 对象上。
- **修复实现**：
  - 在 `selectProvider()`、`cancelProvider()`、`resetModels()` 中统一调用 `hideFetchCandidates()`。

#### P1-3：`MCP` 的 `mcpProbe()` / `mcpRestart()` / `mcpLogin()` / `mcpLogout()` 在新建未保存状态或切换 Server 时发生串台

- **关键点**：`assets/settings_mcp.js`（`mcpProbe`、`mcpRestart`、`mcpLogin`、`mcpLogout`、`buildMcpConfig`）。
- **触发条件**：
  1. 点击 `+ New server` 尚未保存时直接点击 `Test connection` / `Restart` / `Login`；
  2. 在 Server A 点击 `Test connection`，在请求返回前切换到 Server B。
- **错误原因**：
  1. 后端 `/probe`、`/restart`、`/oauth/start` 均按磁盘已保存配置执行，未保存的新建草稿调用会直接返回 404；
  2. `mcpProbe()` 未捕获发起请求时的 `targetKey = mcpSelectedKey`，异步返回后直接改写全局 `mcpToolNames`，若用户已切到 Server B，会把 Server A 的工具列表污染到 Server B 上；
  3. `buildMcpConfig()` 仅从 `mcpSelected.config` 取 `startup_timeout_sec` / `tool_timeout_sec`，未优先从 `mcpBaselines.get(mcpSelectedKey)` 读取。
- **修复实现**：
  - 在 `mcpProbe()`、`mcpRestart()`、`mcpLogin()`、`mcpLogout()` 入口校验 `if (mcpNew || !mcpSelectedKey) { showToast('Save the server before testing or managing connection', 'info'); return; }`；
  - `mcpProbe()` 捕获 `const targetKey = mcpSelectedKey;`，返回时先将 `tools` 写入 `mcpBaselines.get(targetKey)` 与 `mcpDrafts.get(targetKey)` 的 `toolNames`，仅当 `!mcpNew && mcpSelectedKey === targetKey` 时才更新当前 DOM 的 `mcpToolNames` 与 `renderMcpTools()`；
  - `buildMcpConfig()` 优先从 `mcpBaselines.get(mcpSelectedKey)` 读取保留字段。

---

### P2（性能与交互细节一致性）

#### P2-1：`refreshActionBars()` 在每次输入事件中重复执行 36 次脏状态计算

- **关键点**：`assets/settings.js`（`createActionBar`、`refreshActionBars`）。
- **触发条件**：用户在任意输入框连续打字（触发 `document` 的 `input` 事件）。
- **错误原因**：
  - `refreshActionBars()` 先调用 4 个 `update*DirtyDots()`，随后对 5 个 `actionBar` 逐一调用 `bar.refresh()`；每个 `bar.refresh()` 内部又调用 `controller.isDirty()` 和 `anySettingsDirty()`（遍历 5 个 `bar.isDirty()`），最后更新 `#tab-*` 时再遍历调用 5 次 `isDirty()`，造成单次事件重复序列化比对 36 次。
- **修复实现**：
  - 在 `refreshActionBars()` 中先执行一次各视图的 `update*DirtyDots()`，随后一次性计算出 `dirtyMap`（每个 Tab 的布尔脏状态）和 `anyDirty`，将其作为参数传入 `bar.refresh(dirtyMap[key], anyDirty)`，消除所有冗余计算。

#### P2-2：`Commands` 与 `MCP` 在新建草稿模式下未禁用编辑器内的 `Delete` 按钮

- **关键点**：`assets/settings.js`（`showEditor`）、`assets/settings_mcp.js`（`applyMcpRecordToEditor`）。
- **触发条件**：点击 `+ New command` 或 `+ New server` 打开新建编辑器。
- **错误原因**：
  - `Skills` 在新建模式下禁用了 `#skill-editor .danger`（`Delete` 按钮），而 `Commands` 与 `MCP` 在新建模式下未禁用各自的 `Delete` 按钮，导致未落盘草稿同时出现作用相同的 `Cancel` 与 `Delete` 两个按钮。
- **修复实现**：
  - 在 `Commands` 的 `showEditor` 中设置 `const remove = document.querySelector('#editor .danger'); if (remove) { remove.disabled = !!draft; }`；
  - 在 `MCP` 的 `applyMcpRecordToEditor` 中设置 `const remove = document.querySelector('#mcp-editor .danger'); if (remove) { remove.disabled = !!isNew; }`。

---

## 3. 修复后验证清单

- [x] **P0-1**：`Commands` 切换 `agentBlock` 自动锁定 `md_scope = 'template'`；新建与跨类型改名冲突拦截测试通过。
- [x] **P0-2**：`MCP` 与 `Skills` 新建同名冲突拦截测试通过。
- [x] **P1-1**：`Skills` 点击 `Reload` 重置 `bodyLoaded` 并重新拉取当前选中 Skill 的 `/api/skills/detail` 测试通过。
- [x] **P1-2**：`Models` 切换 Provider / `Cancel` / `Reset` 自动隐藏 `#fetch-candidates` 测试通过。
- [x] **P1-3**：`MCP` 异步 `mcpProbe` 绑定 `targetKey` 防串台及未保存新建项拦截测试通过。
- [x] **P2-1 & P2-2**：`refreshActionBars()` 单次计算优化与新建模式禁用 `Delete` 按钮测试通过。
