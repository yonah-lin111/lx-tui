// MCP 设置页：左列表右表单，配置/测试/重启/登录/登出。
// 复用 settings.js 的 api / toast / createSelect；所有写请求由服务端做 token 与同源校验。

let mcpState = null;
let mcpSelected = null;
let mcpSelectedKey = null;
let mcpNew = false;
let mcpToolNames = [];
let mcpDisabled = new Set();
const mcpBaselines = new Map();
const mcpDrafts = new Map();
let mcpNewDraft = null;

const mcpStatusMarks = {
  ready: '●',
  starting: '◐',
  failed: '✕',
  needs_login: '🔑',
  disabled: '○',
  unknown: '○'
};

function mcpServerKey(scope, name) {
  return scope + '\u0000' + name;
}

function normalizeMcpConfig(raw, prev) {
  const cfg = raw || {};
  const basePrev = prev || cfg;
  const transport = cfg.transport || 'stdio';
  const config = {
    enabled: cfg.enabled !== false,
    transport,
    startup_timeout_sec: (basePrev && basePrev.startup_timeout_sec) ? basePrev.startup_timeout_sec : 30,
    tool_timeout_sec: (basePrev && basePrev.tool_timeout_sec) ? basePrev.tool_timeout_sec : 300
  };
  if (basePrev && Array.isArray(basePrev.enabled_tools)) {
    config.enabled_tools = basePrev.enabled_tools;
  }
  const disabled = Array.from(new Set(cfg.disabled_tools || [])).sort();
  if (disabled.length > 0) { config.disabled_tools = disabled; }
  if (transport === 'stdio') {
    config.command = (cfg.command || '').trim();
    config.args = parseLines((cfg.args || []).join('\n'));
    const cwd = (cfg.cwd || '').trim();
    if (cwd) { config.cwd = cwd; }
    config.env = parseMap(mapToLines(cfg.env || {}));
    config.env_vars = parseLines((cfg.env_vars || []).join('\n'));
    return config;
  }
  config.url = (cfg.url || '').trim();
  config.auth = cfg.auth || 'none';
  if (cfg.bearer_token) { config.bearer_token = cfg.bearer_token; }
  const tokenEnv = (cfg.bearer_token_env_var || '').trim();
  if (tokenEnv) { config.bearer_token_env_var = tokenEnv; }
  config.http_headers = parseMap(mapToLines(cfg.http_headers || {}));
  config.env_http_headers = parseMap(mapToLines(cfg.env_http_headers || {}));
  config.scopes = parseLines((cfg.scopes || []).join(' ').replace(/\s+/g, '\n'));
  const clientId = (cfg.client_id || '').trim();
  if (clientId) { config.client_id = clientId; }
  if (cfg.client_secret) { config.client_secret = cfg.client_secret; }
  return config;
}

function serverToMcpRecord(server) {
  const rawConfig = server ? server.config : null;
  const config = normalizeMcpConfig(rawConfig, rawConfig);
  const disabled = Array.isArray(config.disabled_tools) ? config.disabled_tools.slice() : [];
  return {
    name: server ? server.name : '',
    scope: server ? server.scope : 'user',
    config: config,
    bearer_token: '',
    client_secret: '',
    toolNames: disabled,
    secret_flags: (server && server.secret_flags) ? Object.assign({}, server.secret_flags) : {}
  };
}

function comparableMcpRecord(record) {
  return {
    name: record.name,
    scope: record.scope,
    config: record.config,
    bearer_token: record.bearer_token || '',
    client_secret: record.client_secret || ''
  };
}

function syncMcpBaselines(servers, discardDrafts) {
  if (discardDrafts) {
    mcpDrafts.clear();
    mcpNewDraft = null;
  }
  const previousTools = new Map();
  for (const [key, base] of mcpBaselines.entries()) {
    if (Array.isArray(base.toolNames) && base.toolNames.length > 0) {
      previousTools.set(key, base.toolNames);
    }
  }
  mcpBaselines.clear();
  for (const server of servers) {
    const key = mcpServerKey(server.scope, server.name);
    const record = serverToMcpRecord(server);
    if (previousTools.has(key) && record.toolNames.length === 0) {
      record.toolNames = previousTools.get(key);
    }
    mcpBaselines.set(key, record);
  }
  for (const [key, draftRecord] of Array.from(mcpDrafts.entries())) {
    if (!mcpBaselines.has(key) ||
        JSON.stringify(comparableMcpRecord(draftRecord)) === JSON.stringify(comparableMcpRecord(mcpBaselines.get(key)))) {
      mcpDrafts.delete(key);
    }
  }
  if (!mcpNew && mcpSelectedKey) {
    mcpSelected = servers.find((server) => mcpServerKey(server.scope, server.name) === mcpSelectedKey) || null;
    if (!mcpSelected) {
      mcpSelectedKey = null;
      document.getElementById('mcp-empty').classList.remove('hidden');
      document.getElementById('mcp-editor').classList.add('hidden');
    } else if (discardDrafts || !mcpDrafts.has(mcpSelectedKey)) {
      applyMcpRecordToEditor(mcpBaselines.get(mcpSelectedKey), false);
    }
  } else if (mcpNew && discardDrafts) {
    mcpNew = false;
    mcpSelected = null;
    mcpSelectedKey = null;
    document.getElementById('mcp-empty').classList.remove('hidden');
    document.getElementById('mcp-editor').classList.add('hidden');
  }
}

function loadMcp(discardDrafts) {
  const status = document.getElementById('mcp-status');
  if (status) { status.textContent = 'Loading…'; }
  return api('/api/mcp').then((data) => {
    mcpState = data;
    syncMcpBaselines(data.servers || [], !!discardDrafts);
    renderMcpList();
    if (status) { status.textContent = data.servers.length + ' server(s)'; }
    refreshActionBars();
  }).catch((error) => {
    if (status) { status.textContent = error.message; }
    showToast(error.message, 'error');
  });
}

function renderMcpList() {
  syncMcpEditorToDraft();
  const list = document.getElementById('mcp-list');
  if (!list || !mcpState) { return; }
  while (list.firstChild) { list.removeChild(list.firstChild); }
  const groups = new Map();
  for (const server of mcpState.servers) {
    const key = server.scope === 'user' ? 'User' : ('Project · ' + server.project_root);
    if (!groups.has(key)) { groups.set(key, []); }
    groups.get(key).push(server);
  }
  if (groups.size === 0) {
    const empty = document.createElement('p');
    empty.className = 'field-hint';
    empty.textContent = 'No MCP servers configured.';
    list.appendChild(empty);
  }
  for (const [label, servers] of groups) {
    const header = document.createElement('div');
    header.className = 'field-hint';
    header.textContent = label;
    list.appendChild(header);
    for (const server of servers) {
      const key = mcpServerKey(server.scope, server.name);
      const isSelected = !mcpNew && mcpSelectedKey === key;
      const isDirty = isMcpItemDirty(key);
      const row = document.createElement('button');
      row.type = 'button';
      row.className = 'nav-item' + (isSelected ? ' selected' : '') + (isDirty ? ' dirty' : '');
      row.dataset.key = key;
      const mark = document.createElement('span');
      mark.textContent = mcpStatusMarks[server.status] || '○';
      mark.style.color = server.status === 'failed' ? '#ff6b6b' : (server.status === 'ready' ? '#4ade80' : (server.status === 'needs_login' ? '#fbbf24' : 'inherit'));
      mark.style.marginRight = '6px';
      const name = document.createElement('span');
      name.className = 'nav-name';
      name.textContent = server.name;
      if (server.shadowed_user) { name.style.opacity = '0.5'; }
      row.appendChild(mark);
      row.appendChild(name);
      if (server.overrides_user) {
        const tag = document.createElement('span');
        tag.className = 'tag';
        tag.textContent = 'overrides user';
        row.appendChild(tag);
      }
      row.onclick = () => selectMcpServer(server);
      list.appendChild(row);
    }
  }
  const newRow = document.getElementById('new-mcp-row');
  if (newRow) {
    newRow.classList.toggle('selected', mcpNew);
    newRow.classList.toggle('dirty', isMcpNewDirty());
  }
}

function updateMcpDirtyDots() {
  syncMcpEditorToDraft();
  const list = document.getElementById('mcp-list');
  if (list) {
    list.querySelectorAll('.nav-item').forEach((row) => {
      const key = row.dataset.key;
      if (key) { row.classList.toggle('dirty', isMcpItemDirty(key)); }
    });
  }
  const newRow = document.getElementById('new-mcp-row');
  if (newRow) {
    newRow.classList.toggle('selected', mcpNew);
    newRow.classList.toggle('dirty', isMcpNewDirty());
  }
}

function mcpProjects() {
  if (!mcpState) { return []; }
  return mcpState.projects.filter((project) => project.trusted);
}

function selectMcpServer(server) {
  syncMcpEditorToDraft();
  mcpNew = false;
  mcpSelected = server;
  mcpSelectedKey = mcpServerKey(server.scope, server.name);
  const record = mcpDrafts.get(mcpSelectedKey) ||
    mcpBaselines.get(mcpSelectedKey) ||
    serverToMcpRecord(server);
  applyMcpRecordToEditor(record, false);
  renderMcpList();
  refreshActionBars();
}

function selectMcpServerByKey(key) {
  if (!mcpState || !Array.isArray(mcpState.servers)) { return; }
  const match = mcpState.servers.find((server) => mcpServerKey(server.scope, server.name) === key);
  if (match) { selectMcpServer(match); }
}

function newMcpServer() {
  syncMcpEditorToDraft();
  mcpNew = true;
  mcpSelected = null;
  mcpSelectedKey = null;
  if (!mcpNewDraft) {
    mcpNewDraft = serverToMcpRecord(null);
  }
  applyMcpRecordToEditor(mcpNewDraft, true);
  renderMcpList();
  refreshActionBars();
}

function showMcpEditor(server) {
  const record = server ? serverToMcpRecord(server) : serverToMcpRecord(null);
  applyMcpRecordToEditor(record, !server);
}

function applyMcpRecordToEditor(record, isNew) {
  const empty = document.getElementById('mcp-empty');
  const editor = document.getElementById('mcp-editor');
  empty.classList.add('hidden');
  editor.classList.remove('hidden');
  document.getElementById('mcp-legend').textContent = isNew ? 'New server' : ('Edit ' + record.name);
  const remove = document.querySelector('#mcp-editor .danger');
  if (remove) { remove.disabled = !!isNew; }

  const scopeHost = document.getElementById('mcp-scope-host');
  scopeHost.innerHTML = '';
  const scopeOptions = [{value: 'user', label: 'user'}].concat(mcpProjects().map((project) => ({value: project.scope, label: project.name})));
  const scopeValue = record.scope || 'user';
  createSelect(scopeHost, {
    ariaLabel: 'Scope',
    options: scopeOptions,
    value: scopeValue,
    disabled: !isNew,
    onChange: (next) => { scopeHost.dataset.value = next; }
  });
  scopeHost.dataset.value = scopeValue;

  const nameInput = document.getElementById('mcp-name');
  nameInput.value = record.name || '';
  nameInput.disabled = !isNew;

  const config = record.config || {enabled: true, transport: 'stdio', command: '', args: [], env: {}, env_vars: [], startup_timeout_sec: 30, tool_timeout_sec: 300};
  document.getElementById('mcp-enabled').checked = config.enabled !== false;

  fillTransportSelect(config.transport || 'stdio');
  fillAuthSelect(config.auth || 'none');

  document.getElementById('mcp-command').value = config.command || '';
  document.getElementById('mcp-args').value = (config.args || []).join('\n');
  document.getElementById('mcp-cwd').value = config.cwd || '';
  document.getElementById('mcp-env-vars').value = (config.env_vars || []).join('\n');
  document.getElementById('mcp-env').value = mapToLines(config.env || {});

  document.getElementById('mcp-url').value = config.url || '';
  const hasBearer = !!(record.secret_flags && record.secret_flags.bearer_token);
  const hasSecret = !!(record.secret_flags && record.secret_flags.client_secret);
  document.getElementById('mcp-bearer-token').value = record.bearer_token || '';
  document.getElementById('mcp-bearer-token').placeholder = hasBearer ? 'already set; leave empty to keep' : 'paste the token';
  document.getElementById('mcp-bearer-env').value = config.bearer_token_env_var || '';
  document.getElementById('mcp-headers').value = mapToLines(config.http_headers || {});
  document.getElementById('mcp-env-headers').value = mapToLines(config.env_http_headers || {});
  document.getElementById('mcp-scopes').value = (config.scopes || []).join(' ');
  document.getElementById('mcp-client-id').value = config.client_id || '';
  document.getElementById('mcp-client-secret').value = record.client_secret || '';
  document.getElementById('mcp-client-secret').placeholder = hasSecret ? 'already set; leave empty to keep' : 'optional';

  mcpDisabled = new Set(config.disabled_tools || []);
  const knownTools = Array.isArray(record.toolNames) ? record.toolNames : [];
  mcpToolNames = Array.from(new Set(knownTools.concat(Array.from(mcpDisabled))));
  renderMcpTools();
  updateMcpVisibility();
}

function fillTransportSelect(value) {
  const host = document.getElementById('mcp-transport-host');
  host.innerHTML = '';
  createSelect(host, {
    ariaLabel: 'Transport',
    options: [{value: 'stdio', label: 'stdio'}, {value: 'http', label: 'http'}],
    value,
    onChange: (next) => { host.dataset.value = next; updateMcpVisibility(); }
  });
  host.dataset.value = value;
}

function fillAuthSelect(value) {
  const host = document.getElementById('mcp-auth-host');
  host.innerHTML = '';
  createSelect(host, {
    ariaLabel: 'Auth',
    options: [
      {value: 'none', label: 'none'},
      {value: 'bearer', label: 'bearer'},
      {value: 'oauth', label: 'oauth'}
    ],
    value,
    onChange: (next) => { host.dataset.value = next; updateMcpVisibility(); }
  });
  host.dataset.value = value;
}

function selectValue(id) {
  const host = document.getElementById(id);
  return host ? host.dataset.value : '';
}

function updateMcpVisibility() {
  const transport = selectValue('mcp-transport-host') || 'stdio';
  document.getElementById('mcp-stdio-fields').classList.toggle('hidden', transport !== 'stdio');
  document.getElementById('mcp-http-fields').classList.toggle('hidden', transport !== 'http');
  const auth = selectValue('mcp-auth-host') || 'none';
  document.getElementById('mcp-bearer-token-row').classList.toggle('hidden', auth !== 'bearer');
  document.getElementById('mcp-bearer-env-row').classList.toggle('hidden', auth !== 'bearer');
  document.getElementById('mcp-oauth-fields').classList.toggle('hidden', auth !== 'oauth');
}

function mapToLines(map) {
  return Object.keys(map).map((key) => key + '=' + map[key]).join('\n');
}

function parseLines(text) {
  return text.split('\n').map((line) => line.trim()).filter((line) => line.length > 0);
}

function parseMap(text) {
  const out = {};
  for (const line of parseLines(text)) {
    const index = line.indexOf('=');
    if (index <= 0) { continue; }
    out[line.slice(0, index).trim()] = line.slice(index + 1).trim();
  }
  return out;
}

function renderMcpTools() {
  const host = document.getElementById('mcp-tools');
  while (host.firstChild) { host.removeChild(host.firstChild); }
  if (mcpToolNames.length === 0) {
    const hint = document.createElement('p');
    hint.className = 'field-hint';
    hint.textContent = 'Test the connection to list tools.';
    host.appendChild(hint);
    return;
  }
  for (const tool of mcpToolNames) {
    const label = document.createElement('label');
    label.className = 'field-hint';
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = !mcpDisabled.has(tool);
    box.onchange = () => {
      if (box.checked) { mcpDisabled.delete(tool); } else { mcpDisabled.add(tool); }
      refreshActionBars();
    };
    label.appendChild(box);
    label.appendChild(document.createTextNode(' ' + tool));
    host.appendChild(label);
  }
}

function buildMcpConfig() {
  const transport = selectValue('mcp-transport-host') || 'stdio';
  const baseRecord = (!mcpNew && mcpSelectedKey && mcpBaselines.has(mcpSelectedKey))
    ? mcpBaselines.get(mcpSelectedKey)
    : null;
  const prev = (baseRecord && baseRecord.config) ||
    ((!mcpNew && mcpSelected && mcpSelected.config) ? mcpSelected.config : null);
  const config = {
    enabled: document.getElementById('mcp-enabled').checked,
    transport,
    startup_timeout_sec: (prev && prev.startup_timeout_sec) ? prev.startup_timeout_sec : 30,
    tool_timeout_sec: (prev && prev.tool_timeout_sec) ? prev.tool_timeout_sec : 300
  };
  if (prev && Array.isArray(prev.enabled_tools)) {
    config.enabled_tools = prev.enabled_tools;
  }
  const disabled = Array.from(mcpDisabled).sort();
  if (disabled.length > 0) { config.disabled_tools = disabled; }
  if (transport === 'stdio') {
    config.command = document.getElementById('mcp-command').value.trim();
    config.args = parseLines(document.getElementById('mcp-args').value);
    const cwd = document.getElementById('mcp-cwd').value.trim();
    if (cwd) { config.cwd = cwd; }
    config.env = parseMap(document.getElementById('mcp-env').value);
    config.env_vars = parseLines(document.getElementById('mcp-env-vars').value);
    return config;
  }
  config.url = document.getElementById('mcp-url').value.trim();
  config.auth = selectValue('mcp-auth-host') || 'none';
  const token = document.getElementById('mcp-bearer-token').value;
  if (token) { config.bearer_token = token; }
  const tokenEnv = document.getElementById('mcp-bearer-env').value.trim();
  if (tokenEnv) { config.bearer_token_env_var = tokenEnv; }
  config.http_headers = parseMap(document.getElementById('mcp-headers').value);
  config.env_http_headers = parseMap(document.getElementById('mcp-env-headers').value);
  config.scopes = parseLines(document.getElementById('mcp-scopes').value.replace(/\s+/g, '\n'));
  const clientId = document.getElementById('mcp-client-id').value.trim();
  if (clientId) { config.client_id = clientId; }
  const clientSecret = document.getElementById('mcp-client-secret').value;
  if (clientSecret) { config.client_secret = clientSecret; }
  return config;
}

function mcpSnapshot() {
  const base = (!mcpNew && mcpSelectedKey && mcpBaselines.has(mcpSelectedKey))
    ? mcpBaselines.get(mcpSelectedKey)
    : null;
  return {
    name: document.getElementById('mcp-name').value.trim(),
    scope: selectValue('mcp-scope-host') || 'user',
    config: buildMcpConfig(),
    bearer_token: document.getElementById('mcp-bearer-token').value,
    client_secret: document.getElementById('mcp-client-secret').value,
    toolNames: mcpToolNames.slice(),
    secret_flags: base ? Object.assign({}, base.secret_flags) : {}
  };
}

function syncMcpEditorToDraft() {
  const editor = document.getElementById('mcp-editor');
  if (!editor || editor.classList.contains('hidden')) { return; }
  const current = mcpSnapshot();
  if (mcpNew) {
    mcpNewDraft = current;
    return;
  }
  if (!mcpSelectedKey || !mcpBaselines.has(mcpSelectedKey)) { return; }
  const base = mcpBaselines.get(mcpSelectedKey);
  base.toolNames = current.toolNames.slice();
  if (JSON.stringify(comparableMcpRecord(current)) === JSON.stringify(comparableMcpRecord(base))) {
    mcpDrafts.delete(mcpSelectedKey);
  } else {
    mcpDrafts.set(mcpSelectedKey, current);
  }
}

function isMcpItemDirty(key) {
  if (!mcpDrafts.has(key) || !mcpBaselines.has(key)) { return false; }
  return JSON.stringify(comparableMcpRecord(mcpDrafts.get(key))) !==
    JSON.stringify(comparableMcpRecord(mcpBaselines.get(key)));
}

function isMcpNewDirty() {
  if (!mcpNewDraft) { return false; }
  const def = serverToMcpRecord(null);
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

function resetMcpEditor() {
  mcpDrafts.clear();
  mcpNewDraft = null;
  if (mcpNew) {
    mcpNewDraft = serverToMcpRecord(null);
    applyMcpRecordToEditor(mcpNewDraft, true);
  } else if (mcpSelectedKey && mcpBaselines.has(mcpSelectedKey)) {
    applyMcpRecordToEditor(mcpBaselines.get(mcpSelectedKey), false);
  }
  renderMcpList();
  refreshActionBars();
}

async function saveMcpServer() {
  syncMcpEditorToDraft();
  const dirtyKeys = Array.from(mcpDrafts.keys()).filter(isMcpItemDirty);
  const saveNew = isMcpNewDirty();
  if (dirtyKeys.length === 0 && !saveNew) { return; }

  try {
    for (const key of dirtyKeys) {
      const record = mcpDrafts.get(key);
      const name = (record.name || '').trim();
      const scope = record.scope || 'user';
      if (!name) {
        selectMcpServerByKey(key);
        throw new Error('server name is required');
      }
      try {
        await api('/api/mcp/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name), {
          method: 'PUT',
          body: JSON.stringify(record.config)
        });
      } catch (err) {
        selectMcpServerByKey(key);
        throw err;
      }
      const nextFlags = Object.assign({}, record.secret_flags);
      if (record.bearer_token) { nextFlags.bearer_token = true; }
      if (record.client_secret) { nextFlags.client_secret = true; }
      const savedConfig = JSON.parse(JSON.stringify(record.config));
      delete savedConfig.bearer_token;
      delete savedConfig.client_secret;
      mcpBaselines.set(key, {
        name,
        scope,
        config: savedConfig,
        bearer_token: '',
        client_secret: '',
        toolNames: record.toolNames.slice(),
        secret_flags: nextFlags
      });
      mcpDrafts.delete(key);
    }

    if (saveNew) {
      const record = mcpNewDraft;
      const name = (record.name || '').trim();
      const scope = record.scope || 'user';
      if (!name) {
        newMcpServer();
        throw new Error('server name is required');
      }
      const createdKey = mcpServerKey(scope, name);
      if (mcpBaselines.has(createdKey)) {
        newMcpServer();
        throw new Error('MCP server "' + name + '" already exists');
      }
      try {
        await api('/api/mcp/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name), {
          method: 'PUT',
          body: JSON.stringify(record.config)
        });
      } catch (err) {
        newMcpServer();
        throw err;
      }
      const nextFlags = {};
      if (record.bearer_token) { nextFlags.bearer_token = true; }
      if (record.client_secret) { nextFlags.client_secret = true; }
      const savedConfig = JSON.parse(JSON.stringify(record.config));
      delete savedConfig.bearer_token;
      delete savedConfig.client_secret;
      mcpBaselines.set(createdKey, {
        name,
        scope,
        config: savedConfig,
        bearer_token: '',
        client_secret: '',
        toolNames: record.toolNames.slice(),
        secret_flags: nextFlags
      });
      mcpNewDraft = null;
      if (mcpNew) {
        mcpNew = false;
        mcpSelectedKey = createdKey;
      }
    }

    showToast('MCP server saved', 'ok');
    await loadMcp(false);
  } catch (error) {
    await loadMcp(false);
    showToast(error.message, 'error');
    throw error;
  }
}

function deleteMcpServer() {
  if (mcpNew) {
    hideMcpEditor();
    return;
  }
  if (!mcpSelected) { return; }
  if (!window.confirm('Delete MCP server "' + mcpSelected.name + '"?')) { return; }
  const deletingKey = mcpSelectedKey;
  api('/api/mcp/' + encodeURIComponent(mcpSelected.scope) + '/' + encodeURIComponent(mcpSelected.name), {
    method: 'DELETE'
  }).then(() => {
    if (deletingKey) {
      mcpDrafts.delete(deletingKey);
      mcpBaselines.delete(deletingKey);
    }
    showToast('MCP server deleted', 'ok');
    hideMcpEditor();
    return loadMcp(false);
  }).catch((error) => showToast(error.message, 'error'));
}

function hideMcpEditor() {
  if (mcpNew) {
    mcpNewDraft = null;
  } else if (mcpSelectedKey) {
    mcpDrafts.delete(mcpSelectedKey);
  }
  document.getElementById('mcp-empty').classList.remove('hidden');
  document.getElementById('mcp-editor').classList.add('hidden');
  mcpNew = false;
  mcpSelected = null;
  mcpSelectedKey = null;
  renderMcpList();
  refreshActionBars();
}

function mcpServerPath(suffix) {
  const name = document.getElementById('mcp-name').value.trim();
  const scope = selectValue('mcp-scope-host') || 'user';
  return '/api/mcp/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name) + suffix;
}

function mcpProbe() {
  if (mcpNew || !mcpSelectedKey) {
    showToast('Save the server before testing connection', 'info');
    return;
  }
  const targetKey = mcpSelectedKey;
  api(mcpServerPath('/probe'), {method: 'POST', body: '{}'}).then((data) => {
    const tools = data.tools || [];
    if (mcpBaselines.has(targetKey)) {
      mcpBaselines.get(targetKey).toolNames = tools.slice();
    }
    if (mcpDrafts.has(targetKey)) {
      mcpDrafts.get(targetKey).toolNames = tools.slice();
    }
    if (!mcpNew && mcpSelectedKey === targetKey) {
      mcpToolNames = tools;
      renderMcpTools();
    }
    showToast('connection ok: ' + tools.length + ' tool(s)', 'ok');
  }).catch((error) => showToast(error.message, 'error'));
}

function mcpRestart() {
  if (mcpNew || !mcpSelectedKey) {
    showToast('Save the server before restarting', 'info');
    return;
  }
  api(mcpServerPath('/restart'), {method: 'POST', body: '{}'}).then(() => {
    showToast('restart requested', 'ok');
    setTimeout(() => loadMcp(false), 800);
  }).catch((error) => showToast(error.message, 'error'));
}

function mcpLogin() {
  if (mcpNew || !mcpSelectedKey) {
    showToast('Save the server before logging in', 'info');
    return;
  }
  // 先同步打开空白页，避免 fetch 回调里的 window.open 被浏览器拦截。
  const popup = window.open('', '_blank');
  api(mcpServerPath('/oauth/start'), {method: 'POST', body: '{}'}).then((data) => {
    if (popup) { popup.location = data.authorize_url; } else { window.open(data.authorize_url, '_blank', 'noopener'); }
    showToast('complete the login in the new tab', 'ok');
    let attempts = 0;
    const timer = setInterval(() => {
      attempts += 1;
      loadMcp(false);
      if (attempts >= 30) { clearInterval(timer); }
    }, 2000);
  }).catch((error) => {
    if (popup) { popup.close(); }
    showToast(error.message, 'error');
  });
}

function mcpLogout() {
  if (mcpNew || !mcpSelectedKey) {
    showToast('Save the server before logging out', 'info');
    return;
  }
  api(mcpServerPath('/oauth/logout'), {method: 'POST', body: '{}'}).then(() => {
    showToast('logged out', 'ok');
    return loadMcp(false);
  }).catch((error) => showToast(error.message, 'error'));
}

actionBars.mcp = createActionBar('actions-mcp', {
  isDirty: mcpDirty,
  save: saveMcpServer,
  reset: resetMcpEditor
});
