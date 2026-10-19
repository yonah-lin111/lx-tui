
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
  const tabs = {general: 'tab-general', commands: 'tab-commands', models: 'tab-models'};
  const views = {general: 'view-general', commands: 'view-commands', models: 'view-models'};
  for (const key of Object.keys(tabs)) {
    const active = key === name;
    document.getElementById(tabs[key]).classList.toggle('active', active);
    document.getElementById(views[key]).classList.toggle('hidden', !active);
  }
  if (name === 'models' && modelsDraft === null) { loadModels(); }
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
  host.classList.add('select-host');
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'select-button';
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
  };

  const open = () => {
    if (opened) { return; }
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
  };
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

// ===== General view =====

const CONFIG_KEYS = ['sidebar_width','sidebar_min_width','sidebar_max_width','narrow_width','min_width','min_height','min_pane_width','min_pane_height'];

function loadState() {
  api('/api/state').then((data) => {
    state = data;
    // 选中项跟随最新列表；被删除的选中项清空回占位。
    if (selected) {
      selected = (data.commands || []).find((command) => command.name === selected.name && command.kind === selected.kind) || null;
      if (!selected) { draft = false; hideEditor(); }
    }
    for (const key of CONFIG_KEYS) {
      document.getElementById(key).value = data.config[key];
    }
    document.getElementById('general-status').textContent = 'Settings loaded';
    document.getElementById('general-status').className = 'status ok';
    renderCommands();
  }).catch((error) => {
    renderCommands();
    document.getElementById('general-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('general-status').className = 'status error';
    document.getElementById('commands-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('commands-status').className = 'status error';
  });
}

function saveConfig() {
  const config = {};
  for (const key of CONFIG_KEYS) {
    config[key] = Number(document.getElementById(key).value);
  }
  api('/api/config', {method: 'PUT', body: JSON.stringify({config})}).then(() => {
    showToast('Settings saved', 'ok');
    loadState();
  }).catch((error) => {
    showToast(error.message, 'error');
  });
}

// ===== Commands view =====

const KIND_LABELS = {agentMD: 'Markdown', agentBlock: 'Template'};
const commandSelects = {};

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
    ]
  });
  commandSelects.mdScope = createSelect(document.getElementById('cmd-md-scope-select'), {
    ariaLabel: 'Command visibility',
    value: 'global',
    options: [
      {value: 'global', label: 'Global (everywhere)'},
      {value: 'template', label: 'Template only'}
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
    const isSelected = !draft && selected !== null && selected.name === command.name && selected.kind === command.kind;
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'nav-item' + (isSelected ? ' selected' : '');
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
  document.getElementById('new-command-row').classList.toggle('selected', draft);
}

function selectCommand(command) {
  draft = false;
  selected = command;
  showEditor(command);
  renderCommands();
}

function newCommand() {
  draft = true;
  selected = null;
  showEditor({name: '', scope: 'user', kind: 'agentMD', md_scope: 'global', title: '', description: '', body: ''});
  renderCommands();
}

function showEditor(command) {
  document.getElementById('editor-empty').classList.add('hidden');
  document.getElementById('editor').classList.remove('hidden');
  document.getElementById('editor-legend').textContent = command.name ? ('Edit ' + command.name) : 'New command';
  document.getElementById('cmd-name').value = command.name || '';
  commandSelects.kind.setValue(command.kind || 'agentMD');
  commandSelects.mdScope.setValue(command.md_scope || 'global');
  commandSelects.blockType.setValue(command.block_type || 'template');
  document.getElementById('cmd-title').value = command.title || '';
  document.getElementById('cmd-description').value = command.description || '';
  document.getElementById('cmd-body').value = command.body || '';
}

function hideEditor() {
  draft = false;
  selected = null;
  document.getElementById('editor').classList.add('hidden');
  document.getElementById('editor-empty').classList.remove('hidden');
  renderCommands();
}

function saveCommand() {
  const name = document.getElementById('cmd-name').value.trim();
  const kind = commandSelects.kind.value();
  const mdScope = commandSelects.mdScope.value();
  const body = document.getElementById('cmd-body').value;
  const blockType = commandSelects.blockType.value();
  const payload = {
    description: document.getElementById('cmd-description').value,
    md_scope: mdScope,
    block_type: blockType,
    title: document.getElementById('cmd-title').value,
    body: body
  };
  // 已存在的命令改名：先 rename（失败即中止，不动内容），再 PUT 新路径。
  const renameStep = !draft && selected !== null && selected.name && name !== selected.name
    ? api('/api/commands/user/' + encodeURIComponent(selected.kind) + '/' + encodeURIComponent(selected.name) + '/rename', {method: 'POST', body: JSON.stringify({to: name})}).then(() => {
        selected = Object.assign({}, selected, {name: name});
        renderCommands();
      })
    : Promise.resolve();
  renameStep.then(() => api('/api/commands/user/' + encodeURIComponent(kind) + '/' + encodeURIComponent(name), {method: 'PUT', body: JSON.stringify(payload)})).then(() => {
    draft = false;
    selected = {name, scope: 'user', kind, md_scope: mdScope, block_type: blockType, title: payload.title, description: payload.description, body};
    showEditor(selected);
    showToast('Saved', 'ok');
    renderCommands();
    loadState();
  }).catch((error) => {
    showToast(error.message, 'error');
  });
}

function deleteCommand() {
  if (!selected || !selected.name) { return; }
  api('/api/commands/user/' + encodeURIComponent(selected.kind) + '/' + encodeURIComponent(selected.name), {method: 'DELETE'}).then(() => {
    hideEditor();
    showToast('Deleted', 'ok');
    loadState();
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
initCommandSelects();
loadState();
