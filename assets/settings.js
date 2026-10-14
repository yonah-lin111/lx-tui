
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
      throw new Error(message);
    }
    return data;
  });
}

function showTab(name) {
  const general = name === 'general';
  document.getElementById('tab-general').classList.toggle('active', general);
  document.getElementById('tab-commands').classList.toggle('active', !general);
  document.getElementById('view-general').classList.toggle('hidden', !general);
  document.getElementById('view-commands').classList.toggle('hidden', general);
}

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
    document.getElementById('config-state').textContent = 'Saved';
    document.getElementById('config-state').className = 'status ok';
    loadState();
  }).catch((error) => {
    document.getElementById('config-state').textContent = error.message;
    document.getElementById('config-state').className = 'status error';
  });
}

const KIND_LABELS = {agentMD: 'Markdown', agentBlock: 'Template'};

function renderCommands() {
  const list = document.getElementById('command-list');
  list.innerHTML = '';
  const filter = document.getElementById('cmd-filter').value;
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
  document.getElementById('cmd-kind').value = command.kind || 'agentMD';
  document.getElementById('cmd-md-scope').value = command.md_scope || 'global';
  document.getElementById('cmd-block-type').value = command.block_type || 'template';
  document.getElementById('cmd-title').value = command.title || '';
  document.getElementById('cmd-description').value = command.description || '';
  document.getElementById('cmd-body').value = command.body || '';
  document.getElementById('editor-status').textContent = '';
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
  const kind = document.getElementById('cmd-kind').value;
  const mdScope = document.getElementById('cmd-md-scope').value;
  const body = document.getElementById('cmd-body').value;
  const blockType = document.getElementById('cmd-block-type').value;
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
    document.getElementById('editor-status').textContent = 'Saved';
    document.getElementById('editor-status').className = 'status ok';
    renderCommands();
    loadState();
  }).catch((error) => {
    document.getElementById('editor-status').textContent = error.message;
    document.getElementById('editor-status').className = 'status error';
  });
}

function deleteCommand() {
  if (!selected || !selected.name) { return; }
  api('/api/commands/user/' + encodeURIComponent(selected.kind) + '/' + encodeURIComponent(selected.name), {method: 'DELETE'}).then(() => {
    hideEditor();
    loadState();
  }).catch((error) => {
    document.getElementById('editor-status').textContent = error.message;
    document.getElementById('editor-status').className = 'status error';
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

loadState();
