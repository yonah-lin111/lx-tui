// MCP 设置页：左列表右表单，配置/测试/重启/登录/登出。
// 复用 settings.js 的 api / toast / createSelect；所有写请求由服务端做 token 与同源校验。

let mcpState = null;
let mcpSelected = null;
let mcpNew = false;
let mcpToolNames = [];
let mcpDisabled = new Set();
let mcpBaseline = null;

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

function loadMcp() {
  const status = document.getElementById('mcp-status');
  if (status) { status.textContent = 'Loading…'; }
  return api('/api/mcp').then((data) => {
    mcpState = data;
    renderMcpList();
    if (status) { status.textContent = data.servers.length + ' server(s)'; }
    refreshActionBars();
  }).catch((error) => {
    if (status) { status.textContent = error.message; }
    showToast(error.message, 'error');
  });
}

function renderMcpList() {
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
    return;
  }
  for (const [label, servers] of groups) {
    const header = document.createElement('div');
    header.className = 'field-hint';
    header.textContent = label;
    list.appendChild(header);
    for (const server of servers) {
      const row = document.createElement('button');
      row.type = 'button';
      row.className = 'nav-item';
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
        tag.className = 'field-hint';
        tag.textContent = ' overrides user';
        row.appendChild(tag);
      }
      row.onclick = () => selectMcpServer(server);
      list.appendChild(row);
    }
  }
}

function mcpProjects() {
  if (!mcpState) { return []; }
  return mcpState.projects.filter((project) => project.trusted);
}

function selectMcpServer(server) {
  mcpNew = false;
  mcpSelected = server;
  showMcpEditor(server);
}

function newMcpServer() {
  mcpNew = true;
  mcpSelected = null;
  showMcpEditor(null);
}

function showMcpEditor(server) {
  const empty = document.getElementById('mcp-empty');
  const editor = document.getElementById('mcp-editor');
  empty.classList.add('hidden');
  editor.classList.remove('hidden');
  document.getElementById('mcp-legend').textContent = mcpNew ? 'New server' : ('Edit ' + server.name);

  const scopeHost = document.getElementById('mcp-scope-host');
  scopeHost.innerHTML = '';
  const scopeOptions = [{value: 'user', label: 'user'}].concat(mcpProjects().map((project) => ({value: project.scope, label: project.name})));
  const scopeValue = server ? server.scope : 'user';
  createSelect(scopeHost, {
    ariaLabel: 'Scope',
    options: scopeOptions,
    value: scopeValue,
    onChange: (next) => { scopeHost.dataset.value = next; }
  });
  scopeHost.dataset.value = scopeValue;

  const nameInput = document.getElementById('mcp-name');
  nameInput.value = server ? server.name : '';
  nameInput.disabled = !mcpNew;

  const config = server ? server.config : {enabled: true, transport: 'stdio', command: '', args: [], env: {}, env_vars: [], startup_timeout_sec: 30, tool_timeout_sec: 300};
  document.getElementById('mcp-enabled').checked = config.enabled !== false;

  fillTransportSelect(config.transport || 'stdio');
  fillAuthSelect(config.auth || 'none');

  document.getElementById('mcp-command').value = config.command || '';
  document.getElementById('mcp-args').value = (config.args || []).join('\n');
  document.getElementById('mcp-cwd').value = config.cwd || '';
  document.getElementById('mcp-env-vars').value = (config.env_vars || []).join('\n');
  document.getElementById('mcp-env').value = mapToLines(config.env || {});

  document.getElementById('mcp-url').value = config.url || '';
  const hasBearer = server && server.secret_flags && server.secret_flags.bearer_token;
  const hasSecret = server && server.secret_flags && server.secret_flags.client_secret;
  document.getElementById('mcp-bearer-token').value = '';
  document.getElementById('mcp-bearer-token').placeholder = hasBearer ? 'already set; leave empty to keep' : 'paste the token';
  document.getElementById('mcp-bearer-env').value = config.bearer_token_env_var || '';
  document.getElementById('mcp-headers').value = mapToLines(config.http_headers || {});
  document.getElementById('mcp-env-headers').value = mapToLines(config.env_http_headers || {});
  document.getElementById('mcp-scopes').value = (config.scopes || []).join(' ');
  document.getElementById('mcp-client-id').value = config.client_id || '';
  document.getElementById('mcp-client-secret').value = '';
  document.getElementById('mcp-client-secret').placeholder = hasSecret ? 'already set; leave empty to keep' : 'optional';

  mcpDisabled = new Set(config.disabled_tools || []);
  mcpToolNames = Array.from(mcpDisabled);
  renderMcpTools();
  updateMcpVisibility();
  mcpBaseline = JSON.parse(JSON.stringify(mcpSnapshot()));
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
  const config = {
    enabled: document.getElementById('mcp-enabled').checked,
    transport,
    startup_timeout_sec: 30,
    tool_timeout_sec: 300
  };
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
  return {
    name: document.getElementById('mcp-name').value.trim(),
    scope: selectValue('mcp-scope-host') || 'user',
    config: buildMcpConfig(),
    bearer_token: document.getElementById('mcp-bearer-token').value,
    client_secret: document.getElementById('mcp-client-secret').value
  };
}

function mcpDirty() {
  if (mcpBaseline === null || document.getElementById('mcp-editor').classList.contains('hidden')) {
    return false;
  }
  return JSON.stringify(mcpSnapshot()) !== JSON.stringify(mcpBaseline);
}

function resetMcpEditor() {
  if (mcpNew) {
    showMcpEditor(null);
  } else if (mcpSelected) {
    showMcpEditor(mcpSelected);
  }
}

async function saveMcpServer() {
  const name = document.getElementById('mcp-name').value.trim();
  const scope = selectValue('mcp-scope-host') || 'user';
  if (!name) {
    const error = new Error('server name is required');
    showToast(error.message, 'error');
    throw error;
  }
  const body = buildMcpConfig();
  try {
    await api('/api/mcp/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name), {
      method: 'PUT',
      body: JSON.stringify(body)
    });
    showToast('MCP server saved', 'ok');
    await loadMcp();
    const saved = mcpState.servers.find((server) => server.name === name && server.scope === scope);
    if (saved) {
      selectMcpServer(saved);
    } else {
      mcpBaseline = JSON.parse(JSON.stringify(mcpSnapshot()));
      refreshActionBars();
    }
  } catch (error) {
    showToast(error.message, 'error');
    throw error;
  }
}

function deleteMcpServer() {
  if (!mcpSelected) { return; }
  if (!window.confirm('Delete MCP server "' + mcpSelected.name + '"?')) { return; }
  api('/api/mcp/' + encodeURIComponent(mcpSelected.scope) + '/' + encodeURIComponent(mcpSelected.name), {
    method: 'DELETE'
  }).then(() => {
    showToast('MCP server deleted', 'ok');
    hideMcpEditor();
    return loadMcp();
  }).catch((error) => showToast(error.message, 'error'));
}

function hideMcpEditor() {
  document.getElementById('mcp-empty').classList.remove('hidden');
  document.getElementById('mcp-editor').classList.add('hidden');
  mcpSelected = null;
  mcpBaseline = null;
  refreshActionBars();
}

function mcpServerPath(suffix) {
  const name = document.getElementById('mcp-name').value.trim();
  const scope = selectValue('mcp-scope-host') || 'user';
  return '/api/mcp/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name) + suffix;
}

function mcpProbe() {
  api(mcpServerPath('/probe'), {method: 'POST', body: '{}'}).then((data) => {
    mcpToolNames = data.tools || [];
    renderMcpTools();
    showToast('connection ok: ' + mcpToolNames.length + ' tool(s)', 'ok');
  }).catch((error) => showToast(error.message, 'error'));
}

function mcpRestart() {
  api(mcpServerPath('/restart'), {method: 'POST', body: '{}'}).then(() => {
    showToast('restart requested', 'ok');
    setTimeout(loadMcp, 800);
  }).catch((error) => showToast(error.message, 'error'));
}

function mcpLogin() {
  // 先同步打开空白页，避免 fetch 回调里的 window.open 被浏览器拦截。
  const popup = window.open('', '_blank');
  api(mcpServerPath('/oauth/start'), {method: 'POST', body: '{}'}).then((data) => {
    if (popup) { popup.location = data.authorize_url; } else { window.open(data.authorize_url, '_blank', 'noopener'); }
    showToast('complete the login in the new tab', 'ok');
    let attempts = 0;
    const timer = setInterval(() => {
      attempts += 1;
      loadMcp();
      if (attempts >= 30) { clearInterval(timer); }
    }, 2000);
  }).catch((error) => {
    if (popup) { popup.close(); }
    showToast(error.message, 'error');
  });
}

function mcpLogout() {
  api(mcpServerPath('/oauth/logout'), {method: 'POST', body: '{}'}).then(() => {
    showToast('logged out', 'ok');
    return loadMcp();
  }).catch((error) => showToast(error.message, 'error'));
}

actionBars.mcp = createActionBar('actions-mcp', {
  isDirty: mcpDirty,
  save: saveMcpServer,
  reset: resetMcpEditor
});
