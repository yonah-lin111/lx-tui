//! 设置页 HTML：单文件自包含（内联 CSS/JS、无外部资源）、黑色主题。

/// 设置页正文；`token` 从 `location.search` 读取，所有请求附带。
pub fn html() -> &'static str {
    PAGE
}

/// 页面模板（英文界面）。
const PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>lx-tui settings</title>
<style>
:root {
  color-scheme: dark;
  --bg: #000;
  --panel: #0a0a0a;
  --panel-raised: #121212;
  --border: #272727;
  --border-strong: #3a3a3a;
  --text: #e6e6e6;
  --muted: #8f8f8f;
  --accent: #22d3ee;
  --accent-ink: #001418;
  --ok: #34d399;
  --err: #f87171;
}
* { box-sizing: border-box; }
html, body { height: 100%; }
body { margin: 0; font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; display: flex; overflow: hidden; background: var(--bg); color: var(--text); }
nav.side { width: 200px; flex-shrink: 0; border-right: 1px solid var(--border); padding: 16px 0; background: var(--panel); }
nav.side h1 { font-size: 13px; margin: 0 16px 12px; text-transform: uppercase; letter-spacing: .08em; color: var(--muted); }
nav.side button { display: block; width: 100%; text-align: left; padding: 8px 16px; border: 0; border-left: 2px solid transparent; background: none; font: inherit; cursor: pointer; color: var(--muted); }
nav.side button:hover { color: var(--text); background: #151515; }
nav.side button.active { background: #161616; color: var(--accent); border-left-color: var(--accent); font-weight: 600; }
main { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden; padding: 24px; max-width: 1440px; }
.view { display: flex; flex: 1 1 auto; min-height: 0; flex-direction: column; }
.view.hidden { display: none; }
#view-general { overflow-y: auto; }
h2 { font-size: 16px; margin: 0 0 16px; }
fieldset { border: 1px solid var(--border); border-radius: 6px; margin: 0 0 16px; padding: 12px; background: var(--panel); }
legend { font-size: 12px; color: var(--muted); padding: 0 6px; }
label { display: block; margin: 8px 0 4px; font-size: 12px; color: var(--muted); }
input, select, textarea { width: 100%; padding: 6px 8px; border: 1px solid var(--border-strong); border-radius: 4px; font: inherit; background: var(--panel-raised); color: var(--text); }
input:focus, select:focus, textarea:focus { outline: 1px solid var(--accent); border-color: var(--accent); }
textarea { min-height: 260px; resize: none; }
.grid { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px 16px; }
.actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin: 12px 0; }
button.primary { background: var(--accent); border: 0; color: var(--accent-ink); padding: 7px 14px; border-radius: 4px; cursor: pointer; font-weight: 600; }
button.ghost { background: none; border: 1px solid var(--border-strong); padding: 6px 12px; border-radius: 4px; cursor: pointer; color: var(--text); }
button.ghost:hover { border-color: var(--muted); }
button.danger { background: none; border: 1px solid var(--err); color: var(--err); padding: 6px 12px; border-radius: 4px; cursor: pointer; }
.status { font-size: 12px; color: var(--muted); margin-left: 8px; }
.status.error { color: var(--err); }
.status.ok { color: var(--ok); }
.commands-toolbar { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; flex-shrink: 0; }
.commands-toolbar label { margin: 0; }
.commands-toolbar select { width: auto; }
.commands-grid { display: grid; flex: 1 1 auto; min-height: 0; grid-template-columns: 220px minmax(0, 1fr); grid-template-rows: minmax(0, 1fr); gap: 16px; align-items: stretch; }
.commands-nav { display: flex; min-height: 0; flex-direction: column; gap: 2px; border-right: 1px solid var(--border); padding-right: 12px; }
.command-list { display: flex; min-height: 60px; flex: 1 1 auto; flex-direction: column; gap: 2px; overflow-y: auto; scrollbar-width: none; }
.command-list::-webkit-scrollbar { display: none; }
.nav-empty { margin: 8px 0; font-size: 12px; color: var(--muted); }
.nav-item { display: flex; width: 100%; align-items: center; gap: 6px; padding: 6px 8px; border: 0; border-radius: 4px; background: none; color: var(--muted); font: inherit; text-align: left; cursor: pointer; }
.nav-item:hover { background: #151515; color: var(--text); }
.nav-item.selected { background: #1c1c1c; color: #fff; }
.nav-item .nav-slash { color: #5c5c5c; }
.nav-item .nav-name { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.nav-item .tag { flex-shrink: 0; font-size: 11px; border: 1px solid var(--border-strong); border-radius: 3px; padding: 1px 6px; color: var(--muted); }
.nav-item.new { margin-top: 8px; flex-shrink: 0; border-top: 1px solid var(--border); border-radius: 0 0 4px 4px; padding-top: 10px; }
.editor-card { display: flex; min-height: 0; flex-direction: column; overflow: hidden; border: 1px solid var(--border); border-radius: 6px; background: var(--panel); padding: 12px; }
.editor-empty { display: flex; min-height: 280px; flex: 1 1 auto; flex-direction: column; align-items: center; justify-content: center; gap: 10px; border: 1px dashed var(--border-strong); border-radius: 6px; color: var(--muted); font-size: 12px; text-align: center; }
#editor { display: flex; flex: 1 1 auto; min-height: 0; flex-direction: column; }
#editor.hidden { display: none; }
.editor-head { display: flex; flex-shrink: 0; align-items: center; justify-content: space-between; border-bottom: 1px solid var(--border); margin-bottom: 12px; padding-bottom: 8px; }
.editor-head h3 { margin: 0; font-size: 14px; font-weight: 600; }
.editor-body { display: flex; flex: 1 1 auto; min-height: 0; flex-direction: column; overflow-y: auto; scrollbar-width: thin; scrollbar-color: var(--border-strong) transparent; }
.editor-body::-webkit-scrollbar { width: 8px; }
.editor-body::-webkit-scrollbar-thumb { background: var(--border-strong); border-radius: 4px; }
.editor-body > fieldset:first-of-type { flex-shrink: 0; }
.editor-body > .actions { flex-shrink: 0; }
.editor-markdown { display: flex; flex: 1 1 auto; flex-direction: column; }
#cmd-body { flex: 1 1 auto; }
.editor-card fieldset:last-of-type { margin-bottom: 0; }
@media (max-width: 900px) {
  .commands-grid { grid-template-columns: 1fr; grid-template-rows: max-content max-content; align-content: start; overflow-y: auto; }
  .commands-nav { border-right: 0; padding-right: 0; }
  .command-list { max-height: 38vh; }
}
.hidden { display: none; }
</style>
</head>
<body>
<nav class="side">
  <h1>lx-tui</h1>
  <button id="tab-general" class="active" onclick="showTab('general')">General</button>
  <button id="tab-commands" onclick="showTab('commands')">Commands</button>
</nav>
<main>
  <section id="view-general" class="view">
    <h2>General</h2>
    <p class="status" id="general-status">Loading…</p>
    <fieldset>
      <legend>Layout</legend>
      <div class="grid">
        <div><label for="sidebar_width">Sidebar width</label><input id="sidebar_width" type="number" min="1" max="200"></div>
        <div><label for="sidebar_min_width">Sidebar min width</label><input id="sidebar_min_width" type="number" min="1" max="200"></div>
        <div><label for="sidebar_max_width">Sidebar max width</label><input id="sidebar_max_width" type="number" min="1" max="200"></div>
        <div><label for="narrow_width">Narrow width</label><input id="narrow_width" type="number" min="1" max="500"></div>
        <div><label for="min_width">Min screen width</label><input id="min_width" type="number" min="1" max="500"></div>
        <div><label for="min_height">Min screen height</label><input id="min_height" type="number" min="1" max="500"></div>
        <div><label for="min_pane_width">Min pane width</label><input id="min_pane_width" type="number" min="1" max="500"></div>
        <div><label for="min_pane_height">Min pane height</label><input id="min_pane_height" type="number" min="1" max="500"></div>
      </div>
    </fieldset>
    <div class="actions">
      <button class="primary" onclick="saveConfig()">Save</button>
      <button class="ghost" onclick="loadState()">Reload</button>
      <span class="status" id="config-state"></span>
    </div>
  </section>

  <section id="view-commands" class="view hidden">
    <h2>Commands</h2>
    <div class="commands-toolbar">
      <label for="cmd-filter">Type</label>
      <select id="cmd-filter" onchange="renderCommands()">
        <option value="all">All</option>
        <option value="agentMD">Markdown insertion</option>
        <option value="agentBlock">Template block</option>
      </select>
      <button class="ghost" onclick="loadState()">Reload</button>
      <span class="status" id="commands-status">Loading…</span>
    </div>
    <div class="commands-grid">
      <nav class="commands-nav" aria-label="Custom commands">
        <div id="command-list" class="command-list"></div>
        <button id="new-command-row" type="button" class="nav-item new" onclick="newCommand()">
          <span class="nav-slash">+</span>
          <span class="nav-name">New command</span>
        </button>
      </nav>

      <section class="editor-card">
        <div id="editor-empty" class="editor-empty">
          <p id="editor-empty-text">Select a command to edit.</p>
          <button class="primary" onclick="newCommand()">New command</button>
        </div>

        <div id="editor" class="hidden">
          <div class="editor-head">
            <h3 id="editor-legend">Edit command</h3>
          </div>
          <div class="editor-body">
            <fieldset>
              <label for="cmd-name">Name</label>
              <input id="cmd-name" type="text" placeholder="my-command">
              <label for="cmd-scope">Scope</label>
              <select id="cmd-scope">
                <option value="user">User</option>
              </select>
              <label for="cmd-kind">Type</label>
              <select id="cmd-kind">
                <option value="agentMD">Markdown insertion</option>
                <option value="agentBlock">Template block</option>
              </select>
              <label for="cmd-md-scope">Visibility</label>
              <select id="cmd-md-scope">
                <option value="global">Global (everywhere)</option>
                <option value="template">Template only</option>
              </select>
              <label for="cmd-title">Block title (template block only)</label>
              <input id="cmd-title" type="text" placeholder="title">
              <label for="cmd-description">Description</label>
              <input id="cmd-description" type="text">
            </fieldset>
            <fieldset class="editor-markdown">
              <legend>Markdown body</legend>
              <textarea id="cmd-body"></textarea>
            </fieldset>
            <div class="actions">
              <button class="primary" onclick="saveCommand()">Save</button>
              <button class="ghost" onclick="hideEditor()">Cancel</button>
              <button class="danger" onclick="deleteCommand()">Delete</button>
              <span class="status" id="editor-status"></span>
            </div>
          </div>
        </div>
      </section>
    </div>
  </section>
</main>
<script>
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
  const payload = {
    description: document.getElementById('cmd-description').value,
    md_scope: mdScope,
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
    selected = {name, scope: 'user', kind, md_scope: mdScope, title: payload.title, description: payload.description, body};
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

loadState();
</script>
</body>
</html>
"#;
