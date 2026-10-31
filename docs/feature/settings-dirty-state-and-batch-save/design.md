# 设置页（Settings）多层级未保存状态与一键保存 — 设计文档

> 状态：**已完成实现、评审（Review P0–P2 修复）与验证**（2026-10-31）
> 关联任务清单：[task.md](task.md)，功能评审报告：[review.md](review.md)
> 涉及范围：`assets/settings.html`、`assets/settings.css`、`assets/settings.js`、`assets/settings_models.js`、`assets/settings_mcp.js`、`assets/settings_skills.js`、`src/settings/service/tests.rs`

---

## 0. 背景与现状问题核实

在对现有前端代码进行完整排查后，定位出以下核心缺陷：

| 视图 / 模块 | 现状文件与位置 | 现存缺陷 |
|---|---|---|
| **Commands（命令）** | `assets/settings.js:469-540` | 仅维护单个 `selected` 与单个 `commandBaseline`。用户在命令 A 修改内容后未点保存，直接点击左侧列表的命令 B 或 `+ New command`，`showEditor()` 会直接用命令 B 的数据覆写表单并重置 `commandBaseline`，**命令 A 的未保存修改被静默丢弃**；左侧命令列表项无未保存小黄点。 |
| **MCP（服务）** | `assets/settings_mcp.js:91-158` | 仅维护单个 `mcpSelected` 与单个 `mcpBaseline`。切换左侧 Server 或点击 `+ New server` 时，`showMcpEditor()` 直接重置表单和 `mcpBaseline`，**未保存修改直接丢失**；左侧 Server 列表项既无选中高亮（`.selected`）也无未保存小黄点（`.dirty`）。 |
| **Skills（技能）** | `assets/settings_skills.js:86-168, 225-250` | 1. 仅维护单个 `skillsSelected` 与单个 `skillBaseline`，切换左侧 Skill 或点击 `+ New skill` 会冲掉未保存修改；<br>2. 左侧列表与详情页的 `Enabled` 勾选框绑定了 `setSkillEnabled()`，点击即刻发送 `PUT /api/skills/enabled` 落盘，不走草稿与小黄点，无法通过 `Reset` 还原。 |
| **Models（模型）** | `assets/settings_models.js:15-52, 122-139` | 虽有 `modelsDraft` 全局草稿，但：<br>1. `modelsDirty` 是单向布尔值，把字段改回原值后小黄点不会自动消失；<br>2. 左侧 Provider 列表项没有按单个 Provider 对比 `modelsBaseline` 显示子项小黄点。 |
| **General 与 Commands 耦合刷新** | `assets/settings.html:40,52`<br>`assets/settings.js:342-379` | `saveGeneral()` 保存成功后调用 `loadState()`，而 `loadState()` 会同时重刷 `General` 与 `Commands`；Commands 工具栏的 `Reload` 按钮也绑定了 `loadState()`，点击会顺带冲掉 `General` 的未保存修改。 |

---

## 1. 已确认的设计决策（Grill Me 收口）

| 编号 | 决策项 | 确认方案 |
|---|---|---|
| **D1** | **覆盖范围与脏状态判定** | 覆盖全部 5 个大 Tab（`General` / `Commands` / `Models` / `MCP` / `Skills`）及其中 4 个带子列表视图的全部左侧子项（小 Tab）。所有大 Tab 与小 Tab 的小黄点统一通过**「当前内存草稿值 vs 最近一次已保存基准值（baseline）深度比对」**实时派生；用户把内容改回原值时，小黄点自动熄灭。 |
| **D2** | **新建草稿（`+ New ...`）保留机制** | 点击 `+ New command / server / skill` 并编辑后，若未保存就切去查看其它已有项，新建草稿保存在独立的 `newDraft` 槽位中，底部 `+ New ...` 按钮右侧亮起小黄点；再次点击 `+ New ...` 可恢复刚才写了一半的内容，点击右上角 `Save` 时一并创建保存。 |
| **D3** | **`Cancel` 与 `Reset` 职责边界** | - **编辑器内 `Cancel` 按钮**：仅放弃**当前正在编辑的这一个小项**的未保存修改（若为 `+ New ...` 草稿则清空该新建草稿并收起编辑器；若为已有项则删除该项草稿并收起编辑器），同页面其它小项的草稿和小黄点完好保留。<br>- **右上角 `Reset` 按钮**：一次性撤销**当前大 Tab 下所有小项**（含新建草稿）的未保存修改，恢复到最近一次已保存基准，清空该大 Tab 下的所有小黄点。 |
| **D4** | **一键保存与部分失败恢复** | 点击任意视图右上角 `Save` 时，按顺序保存所有含未保存改动的视图与子项：<br>- **每成功保存一个小项，立即更新该小项的 `baseline` 并消除其小黄点**；<br>- 若某一项校验或请求失败，之前已保存成功的项保持已保存状态，失败项保留草稿与小黄点，自动切换到失败所在的大 Tab 并选中该失败子项，弹出明确说明失败位置的错误 Toast，状态栏显示红点 `Save failed`。 |
| **D5** | **Skills `Enabled` 勾选框一致性** | 移除点击即刻发请求的逻辑，将列表行与详情页中的 `Enabled` 勾选统一纳入 `Skills` 草稿与 baseline 比对体系：勾选变化后点亮对应 Skill 行与 `Skills` 大 Tab 的小黄点，支持 `Reset`/`Cancel` 还原，点击右上角 `Save` 时统一调用接口保存。 |
| **D6** | **小 Tab 小黄点视觉位置与样式** | 子列表项（`.nav-item`，含 `+ New ...` 行）的小黄点统一放置在**行右侧末尾**（CSS Grid 第 3 列 `grid-column: 3; grid-row: 1`，`6px` 圆点，颜色 `var(--warn)` `#fbbf24`），不改变左侧原有的 `/`、状态灯 `●`、复选框或 `+` 号布局。 |
| **D7** | **跨 Tab 数据刷新隔离** | 拆分 `loadGeneral()` 与 `reloadCommands()`：保存或点击 `Reload` 时严格限制在当前大 Tab 范围内，绝不覆盖其它大 Tab 的未保存草稿；单项保存触发本 Tab 列表刷新时，仅合并更新已保存项的 baseline，保留本 Tab 内可能剩余的未保存草稿（如部分失败场景）。 |

---

## 2. 架构与数据流设计

### 2.1 二级脏状态派生树（Two-Level Dirty State Tree）

```text
                       anySettingsDirty() (右上角 Save 按钮激活 & 全局状态点)
                                      │
        ┌──────────────┬──────────────┼──────────────┬──────────────┐
        ▼              ▼              ▼              ▼              ▼
  generalDirty   commandsDirty  isModelsDirty     mcpDirty      skillDirty    <-- 大 Tab 左侧小黄点 (#tab-*.dirty)
  (表单 vs 基准)       │              │              │              │
                 ┌─────┴─────┐  ┌─────┴─────┐  ┌─────┴─────┐  ┌─────┴─────┐
                 ▼           ▼  ▼           ▼  ▼           ▼  ▼           ▼
            commandDrafts  new  provider  default mcpDrafts new skillDrafts new <-- 小 Tab 右侧小黄点 (.nav-item.dirty)
             vs Baselines Draft  vs Base   Model  vs Base  Draft  vs Base  Draft
```

核心不变量（Invariants）：
1. **单一事实来源（Single Source of Truth）**：DOM 表单只是当前选中项（`selected` 或 `new`）的视图投射；每次发生 `input` / `change` / 自定义下拉 `onChange` 时，先执行 `sync*EditorToDraft()` 将当前表单值同步回内存草稿字典，再由草稿字典与基准字典比对派生所有小黄点与操作条状态。
2. **改回原值即干净（Auto-Clean on Revert）**：当某个子项的草稿与它的 baseline 深度相等时，立即从 `drafts` Map 中移除该 key（或将 `newDraft` 视作未脏），无需手动维护易失步的布尔标志位。

---

## 3. 详细模块设计与关键代码

### 3.1 样式层：子列表项小黄点（`assets/settings.css`）

在不修改现有 `.nav-item { display: grid; grid-template-columns: auto minmax(0, 1fr); ... }` 声明（保证 `src/settings/service/tests.rs:216` 断言完全兼容）的前提下，利用 CSS Grid 隐式第 3 列渲染右侧小黄点：

```css
/* 子列表项（Commands / Models / MCP / Skills 及 + New 按钮）右侧未保存小黄点 */
.nav-item.dirty::after {
  content: '';
  grid-column: 3;
  grid-row: 1;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warn);
  justify-self: end;
}
```

**布局说明**：
- 第 1 列（`auto`）：放置 `/`（Commands）、运行状态点 `●`（MCP）、启用复选框（Skills）、`+`（新建按钮）。
- 第 2 列（`minmax(0, 1fr)`）：第 1 行放置 `.nav-name`（超出自动省略号 `ellipsis`），第 2 行放置 `.tag`。
- 第 3 列（隐式 `auto`）：仅当 `.nav-item` 带有 `.dirty` 类时生成 `6px` 宽的圆点，垂直居中于第 1 行最右侧，与 `.nav-name` 保持 `column-gap: 6px` 间距，绝不遮挡文字或与左侧图标冲突。

---

### 3.2 页面级操作条与跨 Tab 保存定位（`assets/settings.js` & `assets/settings.html`）

#### 3.2.1 拆分 `General` 与 `Commands` 的 Reload 入口

在 `assets/settings.html` 中：
- `General` 视图的 `Reload` 按钮改为 `onclick="loadGeneral()"`；
- `Commands` 视图的 `Reload` 按钮改为 `onclick="reloadCommands(true)"`。

#### 3.2.2 `saveAllDirty()` 失败自动切 Tab

```javascript
// 任意视图的 Save 都按顺序保存全部脏视图；若某视图保存失败，自动切到该视图以便用户即时修正。
function saveAllDirty() {
  if (settingsSaving) { return Promise.resolve(); }
  const dirty = TAB_KEYS.filter((key) => actionBars[key] && actionBars[key].isDirty());
  if (dirty.length === 0) { return Promise.resolve(); }
  settingsSaving = true;
  settingsFailed = false;
  refreshActionBars();
  let chain = Promise.resolve();
  for (const key of dirty) {
    chain = chain.then(() => actionBars[key].save().catch((error) => {
      showTab(key);
      throw error;
    }));
  }
  return chain
    .then(() => { settingsFailed = false; })
    .catch((error) => { settingsFailed = true; throw error; })
    .finally(() => {
      settingsSaving = false;
      refreshActionBars();
    });
}
```

---

### 3.3 Commands 视图多草稿状态机（`assets/settings.js`）

#### 3.3.1 数据结构

```javascript
const NEW_COMMAND_DEFAULT = {
  name: '',
  kind: 'agentMD',
  md_scope: 'global',
  block_type: 'template',
  title: '',
  description: '',
  body: ''
};

let selectedCommandKey = null;          // 当前选中的已有命令键：`${kind}\u0000${name}`
const commandBaselines = new Map();     // key -> EditorValues（已落盘基准）
const commandDrafts = new Map();        // key -> EditorValues（仅存与 baseline 不同的修改草稿）
let newCommandDraft = null;             // EditorValues | null（+ New command 的草稿）

function commandKey(command) {
  return command.kind + '\u0000' + command.name;
}

function normalizeCommandValues(command) {
  return {
    name: (command.name || '').trim(),
    kind: command.kind || 'agentMD',
    md_scope: command.md_scope || 'global',
    block_type: command.block_type || 'template',
    title: command.title || '',
    description: command.description || '',
    body: command.body || ''
  };
}
```

#### 3.3.2 实时同步与脏状态判定

```javascript
function syncCommandEditorToDraft() {
  const editor = document.getElementById('editor');
  if (!editor || editor.classList.contains('hidden')) { return; }
  const current = editorValues();
  if (draft) {
    newCommandDraft = current;
    return;
  }
  if (!selectedCommandKey || !commandBaselines.has(selectedCommandKey)) { return; }
  const base = commandBaselines.get(selectedCommandKey);
  if (JSON.stringify(current) === JSON.stringify(base)) {
    commandDrafts.delete(selectedCommandKey);
  } else {
    commandDrafts.set(selectedCommandKey, current);
  }
}

function isCommandItemDirty(key) {
  if (!commandDrafts.has(key) || !commandBaselines.has(key)) { return false; }
  return JSON.stringify(commandDrafts.get(key)) !== JSON.stringify(commandBaselines.get(key));
}

function isNewCommandDirty() {
  return newCommandDraft !== null &&
    JSON.stringify(newCommandDraft) !== JSON.stringify(NEW_COMMAND_DEFAULT);
}

function commandsDirty() {
  syncCommandEditorToDraft();
  for (const key of commandDrafts.keys()) {
    if (isCommandItemDirty(key)) { return true; }
  }
  return isNewCommandDirty();
}
```

#### 3.3.3 切换、取消、重置与批量逐项保存

```javascript
function selectCommand(command) {
  syncCommandEditorToDraft();
  draft = false;
  selected = command;
  selectedCommandKey = commandKey(command);
  const values = commandDrafts.get(selectedCommandKey) ||
    commandBaselines.get(selectedCommandKey) ||
    normalizeCommandValues(command);
  showEditor(values, false);
  renderCommands();
}

function newCommand() {
  syncCommandEditorToDraft();
  draft = true;
  selected = null;
  selectedCommandKey = null;
  if (!newCommandDraft) {
    newCommandDraft = Object.assign({}, NEW_COMMAND_DEFAULT);
  }
  showEditor(newCommandDraft, true);
  renderCommands();
}

// Cancel：仅丢弃当前正在编辑的单个命令草稿，不影响其它脏命令
function hideEditor() {
  if (draft) {
    newCommandDraft = null;
  } else if (selectedCommandKey) {
    commandDrafts.delete(selectedCommandKey);
  }
  draft = false;
  selected = null;
  selectedCommandKey = null;
  document.getElementById('editor').classList.add('hidden');
  document.getElementById('editor-empty').classList.remove('hidden');
  renderCommands();
}

// Reset：清空 Commands 下所有草稿并还原当前打开的编辑器
function resetCommandEditor() {
  commandDrafts.clear();
  newCommandDraft = null;
  if (draft) {
    newCommandDraft = Object.assign({}, NEW_COMMAND_DEFAULT);
    applyEditorValues(newCommandDraft);
  } else if (selectedCommandKey && commandBaselines.has(selectedCommandKey)) {
    applyEditorValues(commandBaselines.get(selectedCommandKey));
  }
  renderCommands();
}

// Save：逐个保存所有脏命令（含新建命令草稿）；单项成功立即更新 baseline 并消黄点，失败定位到该项
async function saveCommand() {
  syncCommandEditorToDraft();
  const dirtyKeys = Array.from(commandDrafts.keys()).filter(isCommandItemDirty);
  const saveNew = isNewCommandDirty();
  if (dirtyKeys.length === 0 && !saveNew) { return; }

  try {
    for (const key of dirtyKeys) {
      const values = commandDrafts.get(key);
      const base = commandBaselines.get(key);
      const name = values.name.trim();
      if (!name) {
        selectCommandByKey(key);
        throw new Error('Command name is required');
      }
      const payload = {
        description: values.description,
        md_scope: values.md_scope,
        block_type: values.block_type,
        title: values.title,
        body: values.body
      };
      try {
        if (base.kind === values.kind && name !== base.name) {
          await api('/api/commands/user/' + encodeURIComponent(base.kind) + '/' + encodeURIComponent(base.name) + '/rename', {
            method: 'POST',
            body: JSON.stringify({to: name})
          });
        }
        await api('/api/commands/user/' + encodeURIComponent(values.kind) + '/' + encodeURIComponent(name), {
          method: 'PUT',
          body: JSON.stringify(payload)
        });
        if (base.kind !== values.kind) {
          await api('/api/commands/user/' + encodeURIComponent(base.kind) + '/' + encodeURIComponent(base.name), {
            method: 'DELETE'
          }).catch(() => {});
        }
      } catch (err) {
        selectCommandByKey(key);
        throw err;
      }
      commandDrafts.delete(key);
      commandBaselines.delete(key);
      const savedItem = Object.assign({}, values, {name, scope: 'user'});
      const nextKey = commandKey(savedItem);
      commandBaselines.set(nextKey, normalizeCommandValues(savedItem));
      if (selectedCommandKey === key) {
        selectedCommandKey = nextKey;
        selected = savedItem;
      }
    }

    if (saveNew) {
      const values = newCommandDraft;
      const name = values.name.trim();
      if (!name) {
        newCommand();
        throw new Error('Command name is required');
      }
      const payload = {
        description: values.description,
        md_scope: values.md_scope,
        block_type: values.block_type,
        title: values.title,
        body: values.body
      };
      try {
        await api('/api/commands/user/' + encodeURIComponent(values.kind) + '/' + encodeURIComponent(name), {
          method: 'PUT',
          body: JSON.stringify(payload)
        });
      } catch (err) {
        newCommand();
        throw err;
      }
      const created = Object.assign({}, values, {name, scope: 'user'});
      const createdKey = commandKey(created);
      commandBaselines.set(createdKey, normalizeCommandValues(created));
      newCommandDraft = null;
      if (draft) {
        draft = false;
        selected = created;
        selectedCommandKey = createdKey;
        showEditor(created, false);
      }
    }

    showToast('Saved', 'ok');
    await reloadCommands(false);
  } catch (error) {
    await reloadCommands(false);
    showToast(error.message, 'error');
    throw error;
  }
}
```

---

### 3.4 Models 视图按 Provider 精准比对与小黄点（`assets/settings_models.js`）

`Models` 后端接口 `PUT /api/models/settings` 原生接收全量 `providers` 与 `defaultModel` 原子保存。前端只需将单向布尔值 `modelsDirty` 升级为基于 `modelsDraft` 与 `modelsBaseline` 的实时深度比对，并为每个 Provider 列表项渲染 `.dirty` 小黄点：

```javascript
function isProviderDirty(provider) {
  if (!modelsBaseline || !Array.isArray(modelsBaseline.providers)) { return false; }
  const base = modelsBaseline.providers.find((item) => item.key === provider.key);
  if (!base) { return true; } // 新建或复制的 Provider
  return JSON.stringify(provider) !== JSON.stringify(base);
}

function computeModelsDirty() {
  if (!modelsDraft || !modelsBaseline) { return false; }
  return JSON.stringify({
    defaultModel: modelsDraft.defaultModel,
    providers: modelsDraft.providers
  }) !== JSON.stringify({
    defaultModel: modelsBaseline.defaultModel,
    providers: modelsBaseline.providers
  });
}

function markModelsDirty() {
  modelsDirty = computeModelsDirty();
  updateModelsStatus();
  updateProviderDirtyDots();
  refreshActionBars();
}

function updateProviderDirtyDots() {
  const list = document.getElementById('provider-list');
  if (!list || !modelsDraft) { return; }
  const rows = list.querySelectorAll('.nav-item');
  modelsDraft.providers.forEach((provider, index) => {
    if (rows[index]) {
      rows[index].classList.toggle('dirty', isProviderDirty(provider));
    }
  });
}
```

并在 `renderProviders()` 中构建每个 Provider 行时直接附上 `.dirty`：
```javascript
const dirty = isProviderDirty(provider);
const row = element(
  'button',
  'nav-item'
    + (provider.key === selectedProviderKey ? ' selected' : '')
    + (dirty ? ' dirty' : '')
);
```

同时在 Provider 编辑卡新增 `Cancel` 按钮（或在 `resetProvider` 行为上对齐）：若当前 Provider 有未保存修改，恢复或移除该 Provider 的单条草稿。

---

### 3.5 MCP 视图多 Server 草稿状态机（`assets/settings_mcp.js`）

#### 3.5.1 数据结构

```javascript
let mcpState = null;
let mcpSelectedKey = null;              // 当前选中的 Server key：`scope\u0000name`
let mcpNew = false;
const mcpBaselines = new Map();         // key -> McpDraftRecord
const mcpDrafts = new Map();            // key -> McpDraftRecord
let mcpNewDraft = null;                 // McpDraftRecord | null

function mcpServerKey(scope, name) {
  return scope + '\u0000' + name;
}
```

每条 `McpDraftRecord` 结构：
```javascript
function serverToDraftRecord(server) {
  const cfg = server ? JSON.parse(JSON.stringify(server.config || {})) : {
    enabled: true,
    transport: 'stdio',
    command: '',
    args: [],
    env: {},
    env_vars: [],
    startup_timeout_sec: 30,
    tool_timeout_sec: 300
  };
  const disabled = Array.isArray(cfg.disabled_tools) ? cfg.disabled_tools.slice().sort() : [];
  if (disabled.length > 0) { cfg.disabled_tools = disabled; } else { delete cfg.disabled_tools; }
  return {
    name: server ? server.name : '',
    scope: server ? server.scope : 'user',
    config: cfg,
    bearer_token: '',
    client_secret: '',
    toolNames: disabled.slice(),
    secret_flags: (server && server.secret_flags) ? Object.assign({}, server.secret_flags) : {}
  };
}
```

#### 3.5.2 脏比对与列表渲染

```javascript
function comparableMcpRecord(record) {
  return {
    name: record.name,
    scope: record.scope,
    config: record.config,
    bearer_token: record.bearer_token || '',
    client_secret: record.client_secret || ''
  };
}

function isMcpItemDirty(key) {
  if (!mcpDrafts.has(key) || !mcpBaselines.has(key)) { return false; }
  return JSON.stringify(comparableMcpRecord(mcpDrafts.get(key))) !==
    JSON.stringify(comparableMcpRecord(mcpBaselines.get(key)));
}

function isMcpNewDirty() {
  if (!mcpNewDraft) { return false; }
  const def = serverToDraftRecord(null);
  return JSON.stringify(comparableMcpRecord(mcpNewDraft)) !==
    JSON.stringify(comparableMcpRecord(def));
}

function mcpDirty() {
  syncMcpEditorToDraft();
  for (const key of mcpDrafts.keys()) {
    if (isMcpItemDirty(key)) { return true; }
  }
  return isMcpNewDirty();
}
```

在 `renderMcpList()` 中，为每个 `row` 添加选中态与脏黄点，并更新 `#new-mcp-row`：
```javascript
const key = mcpServerKey(server.scope, server.name);
const isSelected = !mcpNew && mcpSelectedKey === key;
const isDirty = isMcpItemDirty(key);
row.className = 'nav-item' + (isSelected ? ' selected' : '') + (isDirty ? ' dirty' : '');
```
```javascript
const newRow = document.getElementById('new-mcp-row');
if (newRow) {
  newRow.classList.toggle('selected', mcpNew);
  newRow.classList.toggle('dirty', isMcpNewDirty());
}
```

#### 3.5.3 批量逐项保存 `saveMcpServer()`

- 先调用 `syncMcpEditorToDraft()` 将当前表单写入草稿。
- 依次遍历 `mcpDrafts` 中所有 `isMcpItemDirty(key)` 的项，以及 `isMcpNewDirty()` 的新建项：
  - 发送 `PUT /api/mcp/${encodeURIComponent(record.scope)}/${encodeURIComponent(record.name)}`；
  - 每成功一项，立即将 `record`（清空明文密钥输入框、`secret_flags` 同步更新）写入 `mcpBaselines`，并从 `mcpDrafts` 移除（或将 `mcpNewDraft` 置 `null`）；
  - 若某一项校验或请求失败，自动在编辑器中打开该项并抛出错误，保留该项及后续未保存项的草稿与小黄点。
- 循环结束后调用 `loadMcp(false)` 刷新运行状态快照，同时保留任何未保存的草稿。

---

### 3.6 Skills 视图多 Skill 草稿与 Enabled 统一保存（`assets/settings_skills.js`）

#### 3.6.1 数据结构与懒加载正文对齐

由於 `/api/skills` 列表只返回 `name, description, path, scope, source, enabled, editable`，而 `body` 在点击某个 Skill 时经 `/api/skills/detail?path=...` 异步加载，每条 Skill 的基准与草稿设计如下：

```javascript
const SKILL_NEW_DEFAULT = {
  name: '',
  scope: 'user',
  description: '',
  body: ''
};

let skillsState = null;
let selectedSkillPath = null;
let skillsNew = false;
const skillBaselines = new Map();       // path -> SkillRecord
const skillDrafts = new Map();          // path -> SkillRecord
let skillNewDraft = null;               // { name, scope, description, body } | null
```

其中 `SkillRecord` 结构：
```javascript
{
  path: entry.path,
  name: entry.name,
  scope: entry.scope,
  editable: !!entry.editable,
  enabled: !!entry.enabled,
  description: entry.description || '',
  body: '',
  bodyLoaded: false
}
```

- **列表加载时（`loadSkills`）**：用返回的 `entries` 更新 `skillBaselines`（若旧 baseline 已经 `bodyLoaded: true`，保留已加载的 `body` 基准；若 `discardDrafts === true` 则清空 `skillDrafts` 与 `skillNewDraft`）。
- **点击列表勾选框或详情勾选框时**：
  ```javascript
  function stageSkillEnabled(path, enabled) {
    const base = skillBaselines.get(path);
    if (!base) { return; }
    const draft = skillDrafts.get(path)
      ? Object.assign({}, skillDrafts.get(path))
      : Object.assign({}, base);
    draft.enabled = !!enabled;
    if (isSkillRecordEqual(draft, base)) {
      skillDrafts.delete(path);
    } else {
      skillDrafts.set(path, draft);
    }
    if ( !skillsNew && selectedSkillPath === path) {
      document.getElementById('skill-enabled').checked = !!enabled;
    }
    renderSkillList();
    refreshActionBars();
  }
  ```
- **详情异步加载返回时（`/api/skills/detail`）**：
  - 将 `detail.description` 与 `detail.body` 写入 `base.description`、`base.body` 并置 `base.bodyLoaded = true`；
  - 若 `skillDrafts.has(path)`（例如用户先点了 `Enabled` 勾选框再点开详情），在不覆盖用户未保存编辑的前提下补齐 `draft.body = detail.body; draft.bodyLoaded = true`。

#### 3.6.2 脏状态判定

```javascript
function isSkillRecordEqual(a, b) {
  if (a.enabled !== b.enabled) { return false; }
  if (!b.editable || !a.bodyLoaded || !b.bodyLoaded) { return true; }
  return a.description === b.description && a.body === b.body;
}

function isSkillItemDirty(path) {
  if (!skillDrafts.has(path) || !skillBaselines.has(path)) { return false; }
  return !isSkillRecordEqual(skillDrafts.get(path), skillBaselines.get(path));
}

function isSkillNewDirty() {
  return skillNewDraft !== null &&
    JSON.stringify(skillNewDraft) !== JSON.stringify(SKILL_NEW_DEFAULT);
}

function skillDirty() {
  syncSkillEditorToDraft();
  for (const path of skillDrafts.keys()) {
    if (isSkillItemDirty(path)) { return true; }
  }
  return isSkillNewDirty();
}
```

#### 3.6.3 批量逐项保存 `saveSkill()`

```javascript
async function saveSkill() {
  syncSkillEditorToDraft();
  const dirtyPaths = Array.from(skillDrafts.keys()).filter(isSkillItemDirty);
  const saveNew = isSkillNewDirty();
  if (dirtyPaths.length === 0 && !saveNew) { return; }

  try {
    for (const path of dirtyPaths) {
      const draft = skillDrafts.get(path);
      const base = skillBaselines.get(path);
      try {
        if (base.editable && draft.bodyLoaded && (draft.description !== base.description || draft.body !== base.body)) {
          await api(skillPath(draft.name, draft.scope), {
            method: 'PUT',
            body: JSON.stringify({description: draft.description, body: draft.body})
          });
          base.description = draft.description;
          base.body = draft.body;
        }
        if (draft.enabled !== base.enabled) {
          await api('/api/skills/enabled', {
            method: 'PUT',
            body: JSON.stringify({path: draft.path, enabled: draft.enabled})
          });
          base.enabled = draft.enabled;
        }
      } catch (err) {
        selectSkillByPath(path);
        throw err;
      }
      skillDrafts.delete(path);
    }

    if (saveNew) {
      const name = skillNewDraft.name.trim();
      const scope = skillNewDraft.scope || 'user';
      if (!name) {
        newSkill();
        throw new Error('skill name is required');
      }
      try {
        await api(skillPath(name, scope), {
          method: 'PUT',
          body: JSON.stringify({description: skillNewDraft.description, body: skillNewDraft.body})
        });
      } catch (err) {
        newSkill();
        throw err;
      }
      skillNewDraft = null;
      skillsNew = false;
    }

    showToast('skill saved', 'ok');
    await loadSkills(false);
  } catch (error) {
    await loadSkills(false);
    showToast(error.message, 'error');
    throw error;
  }
}
```

---

## 4. 边界条件与回归防护（含 Review P0–P2 收口）

1. **命令重命名（Rename）、类型切换（`agentBlock` 锁定 `template`）与同名冲突拦截（P0-1 / P0-2）**：
   - 保留 `'/rename'` 与 `JSON.stringify({to: name})` 的调用结构，兼容已有测试 `page_editor_supports_rename_flow`。
   - 当命令类型切为 `agentBlock` 时，自动将 `md_scope` 设为 `'template'` 并禁用 `Visibility` 下拉框，避免后端返回 `422: agentBlock requires template scope`。
   - 当已有命令跨类型迁移或改名（`nextKey !== key`）、以及在 `Commands` / `MCP` / `Skills` 中保存 `+ New ...` 新建草稿时，前端先校验目标键是否已存在于各自 `Baselines` 中，若冲突则定位到该项并报错，防止静默调用幂等 `PUT` 覆盖磁盘已有项。
2. **Skills `Reload` 强制失效正文缓存与 MCP `mcpProbe` 异步防串台（P1-1 / P1-2 / P1-3）**：
   - `loadSkills(true)`（点击 `Reload`）时不复用旧 `skillBaselines` 的 `bodyLoaded` 缓存，且若当前正选中某 Skill，自动重新请求 `/api/skills/detail` 刷新正文。
   - `Models` 在 `selectProvider` / `newProvider` / `duplicateProvider` / `cancelProvider` / `resetModels` 时统一调用 `hideFetchCandidates()` 清理 `#fetch-candidates` 候选框。
   - `mcpProbe()` 捕获发起时的 `targetKey = mcpSelectedKey` 并拦截未落盘的新建草稿，防止异步返回时把工具列表串写到其它 Server。
3. **输入期间列表小黄点实时刷新与单次脏计算优化（P2-1 / P2-2）**：
   - 在右侧表单输入文字触发 `document` 的 `input` / `change` 事件时，调用各视图的 `sync*EditorToDraft()` 与列表节点 `.classList.toggle('dirty', ...)` 原地更新类名，**不重建整个左侧列表 DOM**；`refreshActionBars()` 汇总一次 `dirtyMap` 与 `anyDirty` 并分发给各操作条，消除重复序列化比对。
   - `Commands`、`MCP`、`Skills` 在新建模式下统一禁用编辑器头部的红色 `Delete` 按钮。
4. **测试断言兼容与新增验证（`src/settings/service/tests.rs`）**：
   - 保留 `src/settings/service/tests.rs` 中对 `.nav-item`、`.models-grid`、`#mcp-editor`、`#skill-editor`、`loadModels`/`saveModels`、`loadMcp`/`saveMcpServer`、`loadSkills`/`saveSkill` 等全部既有断言；
   - 新增对 `.nav-item.dirty::after`、多草稿状态管理函数及 P0–P2 边界防护的测试。
   - 使用 `node --check` 校验四份 `.js` 脚本语法零错误。
