# 设置页（Settings）多层级未保存状态与一键保存 — 执行任务清单

> 状态：**已完成实现与验证**（2026-10-31）
> 设计依据：[design.md](design.md)
> 执行约束：用户审核确认本文档与 `design.md` 后再开始修改代码；完成后执行 `node --check` 与 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 定向及全量校验。

---

## 阶段 1：样式与通用操作条升级（`assets/settings.css`、`assets/settings.html`、`assets/settings.js`）

- [x] **T1.1 子列表项（小 Tab）右侧小黄点样式（`assets/settings.css`）**
  - 在不改动 `.nav-item { display: grid; grid-template-columns: auto minmax(0, 1fr); ... }` 原有规则的前提下，新增 `.nav-item.dirty::after` 样式（`grid-column: 3; grid-row: 1; width: 6px; height: 6px; border-radius: 50%; background: var(--warn); justify-self: end;`）。
  - 确保 `Commands`、`Models`、`MCP`、`Skills` 四个视图的普通列表项及底部 `+ New ...` 按钮在附带 `.dirty` 类时均在右侧显示对齐的小黄点，且不遮挡长名称省略号。

- [x] **T1.2 拆分 `General` 与 `Commands` 的 Reload 隔离（`assets/settings.html`、`assets/settings.js`）**
  - 将 `assets/settings.html` 中 `General` 的 Reload 按钮改为 `onclick="loadGeneral()"`，`Commands` 的 Reload 按钮改为 `onclick="reloadCommands(true)"`。
  - 在 `assets/settings.js` 中新增独立 `loadGeneral()`：`saveGeneral()` 成功后仅调用 `loadGeneral()`，不再重刷 `Commands` 状态，防止冲掉 `Commands` 未保存草稿。

- [x] **T1.3 `saveAllDirty()` 跨 Tab 保存失败自动定位（`assets/settings.js`）**
  - 按 `TAB_KEYS` 顺序串联保存所有脏视图；当某个视图 `save()` 抛错时，自动调用 `showTab(key)` 切换到该失败视图，并将 `settingsFailed` 置为 `true` 展示红点 `Save failed`。
  - 在全局 `input` / `change` 事件监听中触发当前活跃视图的草稿同步与子列表小黄点原地刷新。

---

## 阶段 2：Commands 视图多命令草稿与批量保存（`assets/settings.js`）

- [x] **T2.1 建立 `commandBaselines`、`commandDrafts` 与 `newCommandDraft`**
  - 用 `commandKey(cmd)`（`kind + '\u0000' + name`）标识每条已存在命令。
  - 实现 `syncCommandEditorToDraft()`：当表单值与 `commandBaselines.get(key)` 一致时自动移除 `commandDrafts` 中的记录（小黄点自动消失）；不一致时更新草稿。
  - `+ New command` 草稿独立存储在 `newCommandDraft`，与 `NEW_COMMAND_DEFAULT` 对比计算 `isNewCommandDirty()`。

- [x] **T2.2 切换、取消、重置与原地小黄点更新**
  - `selectCommand(command)` 与 `newCommand()` 切换前先执行 `syncCommandEditorToDraft()`，再回填目标项的草稿或基准。
  - `hideEditor()`（编辑器 `Cancel` 按钮）仅丢弃当前选中命令（或当前新建命令）的草稿，保留其它命令的未保存修改与小黄点。
  - `resetCommandEditor()`（右上角 `Reset` 按钮）清空 `commandDrafts` 与 `newCommandDraft` 全部未保存修改，并恢复当前打开的编辑器内容。
  - 实现 `updateCommandDirtyDots()`，在表单输入时原地更新 `#command-list` 各行与 `#new-command-row` 的 `.dirty` 类名，不重建列表 DOM。

- [x] **T2.3 逐项落盘与失败恢复的 `saveCommand()`**
  - 遍历所有脏命令及新建草稿依次提交：支持同类型改名（`POST .../rename` + `PUT`）、跨类型迁移（`PUT` 新类型 + `DELETE` 旧类型）与常规更新（`PUT`）。
  - 每成功保存一项立即更新 `commandBaselines` 并移除 `commandDrafts` 对应项；若某项校验或请求失败，自动在编辑器中定位到该命令并抛出错误，已成功项不回滚。

---

## 阶段 3：Models 视图按 Provider 精准比对与小黄点（`assets/settings_models.js`）

- [x] **T3.1 基于 `modelsBaseline` 的深度脏比对**
  - 实现 `isProviderDirty(provider)`（按 `provider.key` 在 `modelsBaseline.providers` 中查找基准并比对；新建/复制的 Provider 直接视为 `dirty`）。
  - 实现 `computeModelsDirty()`（比对 `defaultModel` 与 `providers` 全量结构），替换原有的单向布尔值置位逻辑，使字段改回原值后Provider 小黄点与 `Models` 大 Tab 小黄点自动熄灭。

- [x] **T3.2 Provider 列表小黄点渲染与实时刷新**
  - 在 `renderProviders()` 中为每个 Provider 行根据 `isProviderDirty(provider)` 附加 `.dirty` 类。
  - 实现 `updateProviderDirtyDots()` 并在 `markModelsDirty()` 中调用，确保右侧修改任意 Provider 字段或模型卡片时左侧对应 Provider 的小黄点实时响应。

---

## 阶段 4：MCP 视图多 Server 草稿与批量保存（`assets/settings_mcp.js`）

- [x] **T4.1 建立 `mcpBaselines`、`mcpDrafts` 与 `mcpNewDraft`**
  - 以 `mcpServerKey(scope, name)` 为键维护每个 Server 的基准与修改草稿，新建 Server 独立存储在 `mcpNewDraft`。
  - 实现 `syncMcpEditorToDraft()`、`isMcpItemDirty(key)`、`isMcpNewDirty()` 与 `mcpDirty()`。

- [x] **T4.2 列表选中态、小黄点及 `Cancel` / `Reset` 隔离**
  - 在 `renderMcpList()` 中补齐 `.selected` 与 `.dirty` 类名渲染，同步更新 `#new-mcp-row` 的 `.selected` 与 `.dirty` 状态；输入时通过 `updateMcpDirtyDots()` 原地更新类名。
  - `hideMcpEditor()`（`Cancel`）只丢弃当前 Server（或新建 Server）的草稿；`resetMcpEditor()`（右上角 `Reset`）清空全部 MCP 草稿并还原当前表单。

- [x] **T4.3 逐项保存 `saveMcpServer()`**
  - 依次提交 `mcpDrafts` 中的所有脏 Server 与 `mcpNewDraft`；每项成功后立即更新 `mcpBaselines` 并清除该项草稿；若某项失败则自动选中该失败项并报错，保留未完成项草稿。

---

## 阶段 5：Skills 视图多 Skill 草稿与 Enabled 统一保存（`assets/settings_skills.js`）

- [x] **T5.1 建立 `skillBaselines`、`skillDrafts` 与 `skillNewDraft`（兼容正文懒加载）**
  - 以 `entry.path` 为键维护每个已存在 Skill 的基准与草稿，兼容 `/api/skills` 列表摘要与 `/api/skills/detail` 异步正文加载。
  - 将左侧列表复选框与详情页 `Enabled` 复选框改为调用 `stageSkillEnabled(path, enabled)` 更新内存草稿并刷新小黄点，移除立即发请求落盘的旧逻辑。

- [x] **T5.2 列表选中态、小黄点及 `Cancel` / `Reset` 隔离**
  - 在 `renderSkillList()` 中为每个 `.nav-item` 行及 `#new-skill-row` 绑定 `.selected` 与 `.dirty` 类名；输入时通过 `updateSkillDirtyDots()` 原地更新类名。
  - `hideSkillEditor()`（`Cancel`）只撤销当前 Skill（或新建 Skill）的未保存修改（含未保存的 `Enabled` 变更）；`resetSkillEditor()`（右上角 `Reset`）清空全部 Skill 草稿并恢复所有勾选框与表单基准。

- [x] **T5.3 逐项保存 `saveSkill()`**
  - 遍历 `skillDrafts` 中所有脏项：若可编辑正文/描述变化则请求 `PUT /api/skills/<scope>/<name>`，若 `enabled` 变化则请求 `PUT /api/skills/enabled`；若 `skillNewDraft` 脏则创建新 Skill。
  - 每成功一项立即更新 `skillBaselines` 并消除小黄点；遇到失败自动定位到失败 Skill 并提示错误。

---

## 阶段 6：测试更新与代码质量校验（`src/settings/service/tests.rs`）

- [x] **T6.1 语法与静态资源回归测试**
  - 运行 `node --check assets/settings.js assets/settings_models.js assets/settings_mcp.js assets/settings_skills.js` 验证全部脚本语法。
  - 在 `src/settings/service/tests.rs` 中补充对 `.nav-item.dirty::after` 及各视图多草稿状态管理入口的断言。
  - 运行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test settings::service` 完成校验。
