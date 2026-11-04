
const token = new URLSearchParams(location.search).get('token') || '';
let state = null;
let selected = null;
let draft = false;

function api(path, options) {
  const url = path + (path.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(token);
  return fetch(url, Object.assign({headers: {'Content-Type': 'application/json'}}, options || {})).then(async (response) => {
    const text = await response.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch (error) { data = null; }
    if (!response.ok) {
      const message = data && data.error ? data.error : ('HTTP ' + response.status);
      const failure = new Error(message);
      failure.status = response.status;
      throw failure;
    }
    return data;
  });
}

function showTab(name) {
  resetWebSearchReveals();
  const tabs = {general: 'tab-general', commands: 'tab-commands', model: 'tab-model', provider: 'tab-provider', mcp: 'tab-mcp', skills: 'tab-skills', subagents: 'tab-subagents'};
  const views = {general: 'view-general', commands: 'view-commands', model: 'view-model', provider: 'view-provider', mcp: 'view-mcp', skills: 'view-skills', subagents: 'view-subagents'};
  for (const key of Object.keys(tabs)) {
    const active = key === name;
    document.getElementById(tabs[key]).classList.toggle('active', active);
    document.getElementById(views[key]).classList.toggle('hidden', !active);
  }
  if (name === 'model' || name === 'provider') {
    if (typeof modelsDraft === 'undefined' || modelsDraft === null) {
      loadModels();
    } else if (name === 'model' && typeof renderDefaultModel === 'function') {
      renderDefaultModel();
    }
  }
  if (name === 'mcp' && (typeof mcpState === 'undefined' || mcpState === null)) { loadMcp(); }
  if (name === 'skills' && (typeof skillsState === 'undefined' || skillsState === null)) { loadSkills(); }
  if (name === 'subagents') {
    if (typeof subagentsState === 'undefined' || subagentsState === null) {
      loadSubagents();
    } else if (typeof subagentsDirty === 'function' && !subagentsDirty()) {
      // 干净状态下重新进入 subagents：刷新缓存的角色与模型目录。
      loadSubagents(false);
    }
  }
  refreshActionBars();
}

// ===== 页面级组件：toast 与自绘 select =====
// 操作结果反馈走 toast；常驻状态（数量、revision、key 状态）留在页面内。
// 所有 <select> 一律用 createSelect，避免系统原生弹层与黑灰主题分裂。

const toastHost = document.createElement('div');
toastHost.className = 'toast-host';
document.body.appendChild(toastHost);

function showToast(message, kind) {
  const item = document.createElement('div');
  item.className = 'toast' + (kind ? ' ' + kind : '');
  item.textContent = message;
  toastHost.appendChild(item);
  setTimeout(() => {
    item.classList.add('leaving');
    setTimeout(() => item.remove(), 200);
  }, 3200);
}

// 菜单挂在 body 上用固定定位，避免被滚动容器裁剪；打开中的实例统一关闭。
const openSelects = new Set();

function createSelect(host, config) {
  const optionsConfig = config || {};
  if (host._selectInstance) {
    openSelects.delete(host._selectInstance);
  }
  if (host._selectMenu && host._selectMenu.parentNode) {
    host._selectMenu.parentNode.removeChild(host._selectMenu);
  }
  host.innerHTML = '';
  host.classList.add('select-host');
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'select-button';
  button.disabled = !!optionsConfig.disabled;
  button.setAttribute('aria-haspopup', 'listbox');
  if (optionsConfig.ariaLabel) { button.setAttribute('aria-label', optionsConfig.ariaLabel); }
  const label = document.createElement('span');
  label.className = 'select-label';
  const caret = document.createElement('span');
  caret.className = 'select-caret';
  caret.textContent = '▾';
  button.appendChild(label);
  button.appendChild(caret);
  host.appendChild(button);

  const menu = document.createElement('div');
  menu.className = 'select-menu hidden';
  menu.setAttribute('role', 'listbox');
  document.body.appendChild(menu);

  let options = [];
  let current = optionsConfig.value === undefined ? '' : optionsConfig.value;
  let active = 0;
  let opened = false;

  const renderLabel = () => {
    const match = options.find((option) => option.value === current);
    label.textContent = match ? match.label : (optionsConfig.placeholder || '');
    label.classList.toggle('placeholder', !match);
  };

  const renderMenu = () => {
    while (menu.firstChild) { menu.removeChild(menu.firstChild); }
    options.forEach((option, index) => {
      const row = document.createElement('button');
      row.type = 'button';
      row.className = 'select-option' + (option.value === current ? ' selected' : '') + (index === active ? ' active' : '');
      row.setAttribute('role', 'option');
      row.textContent = option.label;
      row.onclick = () => { choose(option.value, true); };
      menu.appendChild(row);
    });
  };

  const reposition = () => {
    const rect = button.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const spaceAbove = rect.top - 8;
    const openUp = spaceBelow < 140 && spaceAbove > spaceBelow;
    menu.style.left = rect.left + 'px';
    menu.style.width = rect.width + 'px';
    menu.style.maxHeight = Math.min(240, Math.max(120, openUp ? spaceAbove : spaceBelow)) + 'px';
    if (openUp) {
      menu.style.top = 'auto';
      menu.style.bottom = (window.innerHeight - rect.top + 4) + 'px';
    } else {
      menu.style.bottom = 'auto';
      menu.style.top = (rect.bottom + 4) + 'px';
    }
  };

  const close = () => {
    opened = false;
    menu.classList.add('hidden');
    button.setAttribute('aria-expanded', 'false');
    openSelects.delete(instance);
  };

  const choose = (value, notify) => {
    const changed = value !== current;
    current = value;
    close();
    renderLabel();
    if (notify && changed && typeof optionsConfig.onChange === 'function') { optionsConfig.onChange(value); }
    refreshActionBars();
  };

  const open = () => {
    if (opened || button.disabled) { return; }
    openSelects.forEach((other) => { other.close(); });
    opened = true;
    menu.classList.remove('hidden');
    button.setAttribute('aria-expanded', 'true');
    active = Math.max(0, options.findIndex((option) => option.value === current));
    renderMenu();
    reposition();
    openSelects.add(instance);
  };

  button.addEventListener('click', () => { if (opened) { close(); } else { open(); } });
  button.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!opened) { open(); return; }
      const step = event.key === 'ArrowDown' ? 1 : -1;
      active = (active + step + options.length) % Math.max(1, options.length);
      renderMenu();
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      if (!opened) { open(); return; }
      if (options[active]) { choose(options[active].value, true); }
      return;
    }
    if (event.key === 'Escape') { close(); }
  });

  const instance = {
    el: host,
    menu,
    value: () => current,
    close,
    reposition,
    setOptions: (next, value) => {
      options = next || [];
      if (value !== undefined) { current = value; }
      if (!options.some((option) => option.value === current)) { current = ''; }
      renderLabel();
      renderMenu();
    },
    setValue: (value) => {
      current = value;
      renderLabel();
      if (opened) { renderMenu(); }
    },
    setDisabled: (disabled) => {
      button.disabled = !!disabled;
      if (disabled && opened) { close(); }
    },
  };
  host._selectMenu = menu;
  host._selectInstance = instance;
  instance.setOptions(optionsConfig.options || [], current);
  return instance;
}

document.addEventListener('click', (event) => {
  openSelects.forEach((instance) => {
    if (!instance.el.contains(event.target) && !instance.menu.contains(event.target)) { instance.close(); }
  });
});
window.addEventListener('resize', () => { openSelects.forEach((instance) => instance.reposition()); });
window.addEventListener('scroll', () => { openSelects.forEach((instance) => instance.reposition()); }, true);

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) { node.className = className; }
  if (text !== undefined && text !== null) { node.textContent = text; }
  return node;
}

// ===== 页面级组件：视图操作条（状态 + Reset + Save）=====
// 每个视图注册一个控制器（isDirty/save/reset），标题行右侧只有这一套按钮；
// 页面内不再有其它保存入口。状态圆点：绿=已保存，白=未保存，红=保存失败，灰脉冲=保存中。

const actionBars = {};
const TAB_KEYS = ['general', 'commands', 'model', 'provider', 'mcp', 'skills', 'subagents'];
let settingsSaving = false;
let settingsFailed = false;

function anySettingsDirty() {
  return Object.values(actionBars).some((bar) => bar.isDirty());
}

// 任意视图的 Save 都保存全部脏视图；tab 左侧黄点由同一份 isDirty 派生。
function saveAllDirty() {
  if (settingsSaving) { return Promise.resolve(); }
  const dirty = TAB_KEYS.filter((key) => actionBars[key] && actionBars[key].isDirty());
  if (dirty.length === 0) { return Promise.resolve(); }
  settingsSaving = true;
  settingsFailed = false;
  refreshActionBars();
  let chain = Promise.resolve();
  const seenSaves = new Set();
  for (const key of dirty) {
    const saveFn = actionBars[key].save;
    if (seenSaves.has(saveFn)) { continue; }
    seenSaves.add(saveFn);
    chain = chain.then(() => saveFn().catch((error) => {
      showTab((error && error.tab) || key);
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

function createActionBar(hostId, controller) {
  const host = document.getElementById(hostId);
  const status = element('span', 'action-status');
  const dot = element('span', 'status-dot clean');
  const text = element('span', null, 'Saved');
  status.appendChild(dot);
  status.appendChild(text);
  const reset = element('button', 'ghost', 'Reset');
  reset.type = 'button';
  const save = element('button', 'primary', 'Save');
  save.type = 'button';
  host.appendChild(status);
  host.appendChild(reset);
  host.appendChild(save);
  const refresh = (knownDirty, knownAnyDirty) => {
    const dirty = knownDirty !== undefined ? knownDirty : controller.isDirty();
    const anyDirty = knownAnyDirty !== undefined ? knownAnyDirty : anySettingsDirty();
    if (settingsSaving) {
      dot.className = 'status-dot saving';
      text.textContent = 'Saving…';
    } else if (settingsFailed) {
      dot.className = 'status-dot error';
      text.textContent = 'Save failed';
    } else if (anyDirty) {
      dot.className = 'status-dot dirty';
      text.textContent = 'Unsaved changes';
    } else {
      dot.className = 'status-dot clean';
      text.textContent = 'Saved';
    }
    save.disabled = settingsSaving || !anyDirty;
    reset.disabled = settingsSaving || !dirty;
  };
  save.onclick = () => {
    if (settingsSaving || !anySettingsDirty()) { return; }
    saveAllDirty().catch(() => {});
  };
  reset.onclick = () => {
    if (settingsSaving) { return; }
    controller.reset();
    settingsFailed = false;
    refreshActionBars();
    showToast('Reset to last saved state', 'info');
  };
  refresh();
  return {
    refresh: refresh,
    isDirty: () => controller.isDirty(),
    save: () => controller.save()
  };
}

function refreshActionBars() {
  if (typeof updateCommandDirtyDots === 'function') { updateCommandDirtyDots(); }
  if (typeof updateProviderDirtyDots === 'function') { updateProviderDirtyDots(); }
  if (typeof updateMcpDirtyDots === 'function') { updateMcpDirtyDots(); }
  if (typeof updateSkillDirtyDots === 'function') { updateSkillDirtyDots(); }
  if (typeof updateSubagentDirtyDots === 'function') { updateSubagentDirtyDots(); }
  const dirtyMap = {};
  let anyDirty = false;
  for (const [key, bar] of Object.entries(actionBars)) {
    const isDirty = !!bar.isDirty();
    dirtyMap[key] = isDirty;
    if (isDirty) { anyDirty = true; }
  }
  for (const [key, bar] of Object.entries(actionBars)) {
    bar.refresh(dirtyMap[key], anyDirty);
  }
  for (const key of TAB_KEYS) {
    const tab = document.getElementById('tab-' + key);
    if (tab) { tab.classList.toggle('dirty', !!dirtyMap[key]); }
  }
}

document.addEventListener('input', refreshActionBars);
document.addEventListener('change', refreshActionBars);

// ===== General view =====

const CONFIG_KEYS = ['sidebar_width','sidebar_min_width','sidebar_max_width','narrow_width','min_width','min_height','min_pane_width','min_pane_height'];
const WEB_SEARCH_KEYS = ['web_search_exa_api_key', 'web_search_tavily_api_key'];
let generalBaseline = null;

function generalValues() {
  const values = {};
  for (const key of CONFIG_KEYS) {
    values[key] = document.getElementById(key).value;
  }
  for (const key of WEB_SEARCH_KEYS) {
    const el = document.getElementById(key);
    values[key] = el ? el.value : '';
  }
  return values;
}

function generalDirty() {
  return generalBaseline !== null && JSON.stringify(generalValues()) !== JSON.stringify(generalBaseline);
}

function resetGeneral() {
  resetWebSearchReveals();
  if (generalBaseline === null) { return; }
  for (const key of CONFIG_KEYS) {
    document.getElementById(key).value = generalBaseline[key];
  }
  for (const key of WEB_SEARCH_KEYS) {
    const el = document.getElementById(key);
    if (el) { el.value = generalBaseline[key] || ''; }
  }
}

async function saveGeneral() {
  const config = {};
  for (const key of CONFIG_KEYS) {
    config[key] = Number(document.getElementById(key).value);
  }
  const web_search = {
    exa_api_key: (document.getElementById('web_search_exa_api_key').value || '').trim(),
    tavily_api_key: (document.getElementById('web_search_tavily_api_key').value || '').trim()
  };
  try {
    await api('/api/config', {method: 'PUT', body: JSON.stringify({config, web_search})});
    showToast('Settings saved', 'ok');
    await loadGeneral();
  } catch (error) {
    showToast(error.message, 'error');
    throw error;
  }
}

function applyGeneralData(data) {
  resetWebSearchReveals();
  for (const key of CONFIG_KEYS) {
    document.getElementById(key).value = data.config[key];
  }
  const webSearch = data.web_search || {};
  document.getElementById('web_search_exa_api_key').value = webSearch.exa_api_key || '';
  document.getElementById('web_search_tavily_api_key').value = webSearch.tavily_api_key || '';
  generalBaseline = generalValues();
  document.getElementById('general-status').textContent = 'Settings loaded';
  document.getElementById('general-status').className = 'status ok';
}

// 小眼睛：在 password / text 之间切换，只影响本次显示，不改动输入值。
function bindPasswordReveal(inputId, buttonId) {
  const input = document.getElementById(inputId);
  const button = document.getElementById(buttonId);
  if (!input || !button) { return; }
  button.addEventListener('click', () => {
    const show = input.type === 'password';
    input.type = show ? 'text' : 'password';
    button.classList.toggle('revealed', show);
    button.setAttribute('aria-pressed', show ? 'true' : 'false');
    button.setAttribute('aria-label', show ? 'Hide API key' : 'Show API key');
    input.focus();
  });
}

// 收起明文：数据重填、Reset 与切换 Tab 时强制回到 password，避免明文残留。
function resetWebSearchReveals() {
  for (const [inputId, buttonId] of [
    ['web_search_exa_api_key', 'exa-key-reveal'],
    ['web_search_tavily_api_key', 'tavily-key-reveal'],
    ['prov-api-key', 'prov-key-reveal']
  ]) {
    const input = document.getElementById(inputId);
    const button = document.getElementById(buttonId);
    if (input) { input.type = 'password'; }
    if (button) {
      button.classList.remove('revealed');
      button.setAttribute('aria-pressed', 'false');
      button.setAttribute('aria-label', 'Show API key');
    }
  }
}

bindPasswordReveal('web_search_exa_api_key', 'exa-key-reveal');
bindPasswordReveal('web_search_tavily_api_key', 'tavily-key-reveal');

function loadGeneral() {
  return api('/api/state').then((data) => {
    if (!state) { state = {}; }
    state.config = data.config;
    applyGeneralData(data);
    refreshActionBars();
  }).catch((error) => {
    document.getElementById('general-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('general-status').className = 'status error';
  });
}

function loadState() {
  return api('/api/state').then((data) => {
    state = data;
    applyGeneralData(data);
    syncCommandBaselines(data.commands || [], true);
    renderCommands();
  }).catch((error) => {
    renderCommands();
    document.getElementById('general-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('general-status').className = 'status error';
    document.getElementById('commands-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('commands-status').className = 'status error';
  });
}

// ===== Commands view =====

const KIND_LABELS = {agentMD: 'Markdown', agentBlock: 'Template'};
const NEW_COMMAND_DEFAULT = {
  name: '',
  kind: 'agentMD',
  md_scope: 'global',
  block_type: 'template',
  title: '',
  description: '',
  body: ''
};
const commandSelects = {};
let selectedCommandKey = null;
const commandBaselines = new Map();
const commandDrafts = new Map();
let newCommandDraft = null;

function commandKey(command) {
  return (command.kind || 'agentMD') + '\u0000' + (command.name || '');
}

function normalizeCommandValues(command) {
  const kind = command.kind || 'agentMD';
  return {
    name: (command.name || '').trim(),
    kind: kind,
    md_scope: kind === 'agentBlock' ? 'template' : (command.md_scope || 'global'),
    block_type: command.block_type || 'template',
    title: command.title || '',
    description: command.description || '',
    body: command.body || ''
  };
}

function syncCommandBaselines(commands, discardDrafts) {
  if (discardDrafts) {
    commandDrafts.clear();
    newCommandDraft = null;
  }
  commandBaselines.clear();
  for (const command of commands) {
    commandBaselines.set(commandKey(command), normalizeCommandValues(command));
  }
  for (const [key, draftValues] of Array.from(commandDrafts.entries())) {
    if (!commandBaselines.has(key) || JSON.stringify(draftValues) === JSON.stringify(commandBaselines.get(key))) {
      commandDrafts.delete(key);
    }
  }
  if (!draft && selectedCommandKey) {
    selected = commands.find((command) => commandKey(command) === selectedCommandKey) || null;
    if (!selected) {
      selectedCommandKey = null;
      document.getElementById('editor').classList.add('hidden');
      document.getElementById('editor-empty').classList.remove('hidden');
    } else if (discardDrafts || !commandDrafts.has(selectedCommandKey)) {
      applyEditorValues(commandBaselines.get(selectedCommandKey));
      document.getElementById('editor-legend').textContent = 'Edit ' + selected.name;
    }
  } else if (draft && discardDrafts) {
    draft = false;
    selected = null;
    selectedCommandKey = null;
    document.getElementById('editor').classList.add('hidden');
    document.getElementById('editor-empty').classList.remove('hidden');
  }
}

function initCommandSelects() {
  commandSelects.filter = createSelect(document.getElementById('cmd-filter-select'), {
    ariaLabel: 'Command type filter',
    value: 'all',
    options: [
      {value: 'all', label: 'All'},
      {value: 'agentMD', label: 'Markdown insertion'},
      {value: 'agentBlock', label: 'Template block'}
    ],
    onChange: () => renderCommands()
  });
  commandSelects.scope = createSelect(document.getElementById('cmd-scope-select'), {
    ariaLabel: 'Command scope',
    value: 'user',
    options: [{value: 'user', label: 'User'}]
  });
  commandSelects.kind = createSelect(document.getElementById('cmd-kind-select'), {
    ariaLabel: 'Command type',
    value: 'agentMD',
    options: [
      {value: 'agentMD', label: 'Markdown insertion'},
      {value: 'agentBlock', label: 'Template block'}
    ],
    onChange: (kind) => {
      if (kind === 'agentBlock') {
        commandSelects.mdScope.setValue('template');
      }
      commandSelects.mdScope.setDisabled(kind === 'agentBlock');
    }
  });
  commandSelects.mdScope = createSelect(document.getElementById('cmd-md-scope-select'), {
    ariaLabel: 'Command visibility',
    value: 'global',
    options: [
      {value: 'global', label: 'Global (everywhere)'},
      {value: 'template', label: 'Inside blocks (&&& / +++)'}
    ]
  });
  commandSelects.blockType = createSelect(document.getElementById('cmd-block-type-select'), {
    ariaLabel: 'Block type',
    value: 'template',
    options: [
      {value: 'template', label: 'Task block (&&&)'},
      {value: 'supple', label: 'Supplement block (+++)'}
    ]
  });
}

function renderCommands() {
  syncCommandEditorToDraft();
  const list = document.getElementById('command-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  const filter = commandSelects.filter.value();
  const all = (state && state.commands) || [];
  const commands = all.filter((command) => filter === 'all' || command.kind === filter);
  document.getElementById('commands-status').textContent = all.length + ' command(s)';
  document.getElementById('commands-status').className = 'status';
  document.getElementById('editor-empty-text').textContent =
    all.length === 0 ? 'No custom commands yet.' : 'Select a command to edit.';
  if (commands.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'nav-empty';
    empty.textContent = all.length === 0 ? 'No custom commands yet.' : 'No commands match this filter.';
    list.appendChild(empty);
  }
  for (const command of commands) {
    const key = commandKey(command);
    const isSelected = !draft && selectedCommandKey === key;
    const isDirty = isCommandItemDirty(key);
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'nav-item' + (isSelected ? ' selected' : '') + (isDirty ? ' dirty' : '');
    row.dataset.key = key;
    if (isSelected) { row.setAttribute('aria-current', 'true'); }
    const slash = document.createElement('span');
    slash.className = 'nav-slash';
    slash.textContent = '/';
    const name = document.createElement('span');
    name.className = 'nav-name';
    name.textContent = command.name;
    const tag = document.createElement('span');
    tag.className = 'tag';
    tag.textContent = (command.scope === 'project' ? 'Project' : 'User') + ' · ' + (KIND_LABELS[command.kind] || command.kind);
    row.appendChild(slash);
    row.appendChild(name);
    row.appendChild(tag);
    row.onclick = () => selectCommand(command);
    list.appendChild(row);
  }
  const newRow = document.getElementById('new-command-row');
  newRow.classList.toggle('selected', draft);
  newRow.classList.toggle('dirty', isNewCommandDirty());
  refreshActionBars();
}

function updateCommandDirtyDots() {
  syncCommandEditorToDraft();
  const list = document.getElementById('command-list');
  if (list) {
    list.querySelectorAll('.nav-item').forEach((row) => {
      const key = row.dataset.key;
      if (key) { row.classList.toggle('dirty', isCommandItemDirty(key)); }
    });
  }
  const newRow = document.getElementById('new-command-row');
  if (newRow) {
    newRow.classList.toggle('selected', draft);
    newRow.classList.toggle('dirty', isNewCommandDirty());
  }
}

function selectCommand(command) {
  syncCommandEditorToDraft();
  draft = false;
  selected = command;
  selectedCommandKey = commandKey(command);
  const values = commandDrafts.get(selectedCommandKey) ||
    commandBaselines.get(selectedCommandKey) ||
    normalizeCommandValues(command);
  showEditor(values, command.name);
  renderCommands();
}

function selectCommandByKey(key) {
  const all = (state && state.commands) || [];
  const match = all.find((command) => commandKey(command) === key);
  if (match) { selectCommand(match); }
}

function newCommand() {
  syncCommandEditorToDraft();
  draft = true;
  selected = null;
  selectedCommandKey = null;
  if (!newCommandDraft) {
    newCommandDraft = Object.assign({}, NEW_COMMAND_DEFAULT);
  }
  showEditor(newCommandDraft, '');
  renderCommands();
}

function editorValues() {
  const kind = commandSelects.kind.value();
  return {
    name: document.getElementById('cmd-name').value.trim(),
    kind: kind,
    md_scope: kind === 'agentBlock' ? 'template' : commandSelects.mdScope.value(),
    block_type: commandSelects.blockType.value(),
    title: document.getElementById('cmd-title').value,
    description: document.getElementById('cmd-description').value,
    body: document.getElementById('cmd-body').value
  };
}

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

function applyEditorValues(values) {
  const kind = values.kind || 'agentMD';
  const mdScope = kind === 'agentBlock' ? 'template' : (values.md_scope || 'global');
  document.getElementById('cmd-name').value = values.name;
  document.getElementById('cmd-title').value = values.title;
  document.getElementById('cmd-description').value = values.description;
  document.getElementById('cmd-body').value = values.body;
  commandSelects.kind.setValue(kind);
  commandSelects.mdScope.setValue(mdScope);
  commandSelects.mdScope.setDisabled(kind === 'agentBlock');
  commandSelects.blockType.setValue(values.block_type);
}

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

function showEditor(command, originalName) {
  document.getElementById('editor-empty').classList.add('hidden');
  document.getElementById('editor').classList.remove('hidden');
  const titleName = originalName !== undefined ? originalName : command.name;
  document.getElementById('editor-legend').textContent = titleName ? ('Edit ' + titleName) : 'New command';
  const remove = document.querySelector('#editor .danger');
  if (remove) { remove.disabled = !!draft; }
  const values = normalizeCommandValues(command);
  applyEditorValues(values);
}

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

// 命令列表单独刷新：不重放 General 输入，避免保存命令时冲掉未保存的常规设置。
function reloadCommands(discardDrafts) {
  return api('/api/commands').then((commands) => {
    if (!state) { state = {}; }
    state.commands = commands;
    syncCommandBaselines(commands || [], !!discardDrafts);
    renderCommands();
  }).catch((error) => {
    showToast(error.message, 'error');
  });
}

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
      const nextKey = commandKey({kind: values.kind, name});
      if (nextKey !== key && commandBaselines.has(nextKey)) {
        selectCommandByKey(key);
        throw new Error('Command "' + name + '" already exists');
      }
      const payload = {
        description: values.description,
        md_scope: values.kind === 'agentBlock' ? 'template' : values.md_scope,
        block_type: values.block_type,
        title: values.title,
        body: values.body
      };
      try {
        // 已存在的命令改名：先 rename（失败即中止，不动内容），再 PUT 新路径。
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
      commandBaselines.set(nextKey, normalizeCommandValues(savedItem));
      if (selectedCommandKey === key) {
        selectedCommandKey = nextKey;
        selected = savedItem;
        document.getElementById('editor-legend').textContent = 'Edit ' + name;
      }
    }

    if (saveNew) {
      const values = newCommandDraft;
      const name = values.name.trim();
      if (!name) {
        newCommand();
        throw new Error('Command name is required');
      }
      const createdKey = commandKey({kind: values.kind, name});
      if (commandBaselines.has(createdKey)) {
        newCommand();
        throw new Error('Command "' + name + '" already exists');
      }
      const payload = {
        description: values.description,
        md_scope: values.kind === 'agentBlock' ? 'template' : values.md_scope,
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
      commandBaselines.set(createdKey, normalizeCommandValues(created));
      newCommandDraft = null;
      if (draft) {
        draft = false;
        selected = created;
        selectedCommandKey = createdKey;
        showEditor(created, name);
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

function deleteCommand() {
  if (draft) {
    hideEditor();
    return;
  }
  if (!selected || !selected.name) { return; }
  const deletingKey = selectedCommandKey;
  api('/api/commands/user/' + encodeURIComponent(selected.kind) + '/' + encodeURIComponent(selected.name), {method: 'DELETE'}).then(() => {
    if (deletingKey) {
      commandDrafts.delete(deletingKey);
      commandBaselines.delete(deletingKey);
    }
    hideEditor();
    showToast('Deleted', 'ok');
    reloadCommands(false);
  }).catch((error) => {
    showToast(error.message, 'error');
  });
}

const SIDE_MIN_WIDTH = 140;
const SIDE_MAX_WIDTH = 360;
const LIST_MIN_WIDTH = 140;
const LIST_MAX_WIDTH = 560;
let cmdListWidth = 220;

function clampWidth(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function setupResizer(handle, read, write, min, max) {
  handle.addEventListener('pointerdown', (event) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = read();
    handle.setPointerCapture(event.pointerId);
    handle.classList.add('dragging');
    document.body.classList.add('resizing');
    const onMove = (moveEvent) => write(clampWidth(startWidth + moveEvent.clientX - startX, min, max));
    const stop = () => {
      handle.classList.remove('dragging');
      document.body.classList.remove('resizing');
      handle.removeEventListener('pointermove', onMove);
    };
    handle.addEventListener('pointermove', onMove);
    handle.addEventListener('pointerup', stop, {once: true});
    handle.addEventListener('pointercancel', stop, {once: true});
  });
}

setupResizer(
  document.getElementById('side-resizer'),
  () => document.querySelector('nav.side').getBoundingClientRect().width,
  (width) => { document.querySelector('nav.side').style.width = width + 'px'; },
  SIDE_MIN_WIDTH,
  SIDE_MAX_WIDTH
);
setupResizer(
  document.getElementById('cmd-resizer'),
  () => cmdListWidth,
  (width) => {
    cmdListWidth = width;
    document.querySelector('.commands-grid').style.setProperty('--cmd-list-width', width + 'px');
  },
  LIST_MIN_WIDTH,
  LIST_MAX_WIDTH
);
let modelsListWidth = 220;
setupResizer(
  document.getElementById('models-resizer'),
  () => modelsListWidth,
  (width) => {
    modelsListWidth = width;
    document.querySelector('.models-grid').style.setProperty('--models-list-width', width + 'px');
  },
  LIST_MIN_WIDTH,
  LIST_MAX_WIDTH
);
actionBars.general = createActionBar('actions-general', {
  isDirty: generalDirty,
  save: saveGeneral,
  reset: resetGeneral
});
actionBars.commands = createActionBar('actions-commands', {
  isDirty: commandsDirty,
  save: saveCommand,
  reset: resetCommandEditor
});
initCommandSelects();
loadState();
