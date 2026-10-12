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
body { margin: 0; font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; display: flex; min-height: 100vh; background: var(--bg); color: var(--text); }
nav { width: 200px; border-right: 1px solid var(--border); padding: 16px 0; background: var(--panel); }
nav h1 { font-size: 13px; margin: 0 16px 12px; text-transform: uppercase; letter-spacing: .08em; color: var(--muted); }
nav button { display: block; width: 100%; text-align: left; padding: 8px 16px; border: 0; border-left: 2px solid transparent; background: none; font: inherit; cursor: pointer; color: var(--muted); }
nav button:hover { color: var(--text); background: #151515; }
nav button.active { background: #161616; color: var(--accent); border-left-color: var(--accent); font-weight: 600; }
main { flex: 1; padding: 24px; max-width: 1440px; }
h2 { font-size: 16px; margin: 0 0 16px; }
fieldset { border: 1px solid var(--border); border-radius: 6px; margin: 0 0 16px; padding: 12px; background: var(--panel); }
legend { font-size: 12px; color: var(--muted); padding: 0 6px; }
label { display: block; margin: 8px 0 4px; font-size: 12px; color: var(--muted); }
input, select, textarea { width: 100%; padding: 6px 8px; border: 1px solid var(--border-strong); border-radius: 4px; font: inherit; background: var(--panel-raised); color: var(--text); }
input:focus, select:focus, textarea:focus { outline: 1px solid var(--accent); border-color: var(--accent); }
textarea { min-height: 220px; resize: vertical; }
.grid { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px 16px; }
.actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin: 12px 0; }
button.primary { background: var(--accent); border: 0; color: var(--accent-ink); padding: 7px 14px; border-radius: 4px; cursor: pointer; font-weight: 600; }
button.ghost { background: none; border: 1px solid var(--border-strong); padding: 6px 12px; border-radius: 4px; cursor: pointer; color: var(--text); }
button.ghost:hover { border-color: var(--muted); }
button.danger { background: none; border: 1px solid var(--err); color: var(--err); padding: 6px 12px; border-radius: 4px; cursor: pointer; }
.row { display: flex; justify-content: space-between; gap: 12px; align-items: center; border-bottom: 1px solid #1d1d1d; padding: 8px 0; }
.row:last-child { border-bottom: 0; }
.row .name { overflow-wrap: anywhere; }
.tag { font-size: 11px; border: 1px solid var(--border-strong); border-radius: 3px; padding: 1px 6px; color: var(--muted); margin-left: 6px; white-space: nowrap; }
.status { font-size: 12px; color: var(--muted); margin-left: 8px; }
.status.error { color: var(--err); }
.status.ok { color: var(--ok); }
.preview { border: 1px dashed var(--border-strong); border-radius: 4px; padding: 10px; min-height: 220px; background: var(--panel-raised); overflow-wrap: anywhere; }
.preview h1, .preview h2, .preview h3 { margin: 8px 0 4px; }
.preview pre { background: #161616; padding: 8px; border-radius: 4px; overflow: auto; }
.preview code { background: #161616; padding: 0 3px; border-radius: 3px; }
.preview a { color: var(--accent); }
.hidden { display: none; }
.commands-grid { display: grid; grid-template-columns: minmax(220px, 280px) minmax(0, 1fr); gap: 16px; align-items: start; }
.commands-grid .empty-hint { color: var(--muted); padding: 12px 0; }
#editor { grid-column: 2 / 4; display: grid; grid-template-columns: minmax(280px, 1fr) minmax(280px, 1fr); gap: 16px; align-items: start; }
#editor.hidden { display: none; }
#editor .editor-left { min-width: 0; }
#editor .footer { grid-column: 1 / -1; }
@media (max-width: 1100px) {
  .commands-grid { grid-template-columns: 1fr; }
  #editor { grid-template-columns: 1fr; }
}
</style>
</head>
<body>
<nav>
  <h1>lx-tui</h1>
  <button id="tab-general" class="active" onclick="showTab('general')">General</button>
  <button id="tab-commands" onclick="showTab('commands')">Commands</button>
</nav>
<main>
  <section id="view-general">
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

  <section id="view-commands" class="hidden">
    <h2>Commands</h2>
    <div class="commands-grid">
      <div class="commands-list">
        <div class="actions">
          <button class="primary" onclick="newCommand()">New command</button>
          <button class="ghost" onclick="loadState()">Reload</button>
        </div>
        <label for="cmd-filter">Type</label>
        <select id="cmd-filter" onchange="renderCommands()">
          <option value="all">All</option>
          <option value="agentMD">Markdown insertion</option>
          <option value="agentBlock">Template block</option>
        </select>
        <p class="status" id="commands-status">Loading…</p>
        <div id="command-list"></div>
      </div>

      <div id="editor" class="hidden">
        <div class="editor-left">
          <fieldset>
            <legend id="editor-legend">Edit command</legend>
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
          <fieldset>
            <legend>Markdown body</legend>
            <textarea id="cmd-body" oninput="renderPreview()"></textarea>
          </fieldset>
        </div>
        <fieldset>
          <legend>Preview</legend>
          <div id="cmd-preview" class="preview"></div>
        </fieldset>
        <div class="actions footer">
          <button class="primary" onclick="saveCommand()">Save</button>
          <button class="ghost" onclick="hideEditor()">Cancel</button>
          <button class="danger" onclick="deleteCommand()">Delete</button>
          <span class="status" id="editor-status"></span>
        </div>
      </div>
    </div>
  </section>
</main>
<script>
const token = new URLSearchParams(location.search).get('token') || '';
let state = null;
let editing = null;

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
    for (const key of CONFIG_KEYS) {
      document.getElementById(key).value = data.config[key];
    }
    document.getElementById('general-status').textContent = 'Settings loaded';
    document.getElementById('general-status').className = 'status ok';
    renderCommands();
  }).catch((error) => {
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

function renderCommands() {
  const list = document.getElementById('command-list');
  list.innerHTML = '';
  const filter = document.getElementById('cmd-filter').value;
  const commands = ((state && state.commands) || []).filter((command) => filter === 'all' || command.kind === filter);
  document.getElementById('commands-status').textContent = commands.length + ' command(s)';
  document.getElementById('commands-status').className = 'status';
  for (const command of commands) {
    const row = document.createElement('div');
    row.className = 'row';
    const left = document.createElement('div');
    left.className = 'name';
    left.textContent = command.name;
    const tag = document.createElement('span');
    tag.className = 'tag';
    tag.textContent = (command.scope === 'project' ? 'Project' : 'User') + ' · ' + command.kind;
    left.appendChild(tag);
    const right = document.createElement('div');
    const edit = document.createElement('button');
    edit.className = 'ghost';
    edit.textContent = 'Edit';
    edit.onclick = () => showEditor(command);
    right.appendChild(edit);
    row.appendChild(left);
    row.appendChild(right);
    list.appendChild(row);
  }
  if (commands.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'status';
    empty.textContent = 'No custom commands yet.';
    list.appendChild(empty);
  }
}

function newCommand() {
  document.getElementById('cmd-filter').value = 'all';
  showEditor({name: '', scope: 'user', kind: 'agentMD', md_scope: 'global', title: '', description: '', body: ''});
  document.getElementById('cmd-name').disabled = false;
}

function showEditor(command) {
  editing = command;
  document.getElementById('editor').classList.remove('hidden');
  document.getElementById('editor-legend').textContent = command.name ? ('Edit ' + command.name) : 'New command';
  document.getElementById('cmd-name').value = command.name || '';
  document.getElementById('cmd-name').disabled = Boolean(command.name);
  document.getElementById('cmd-kind').value = command.kind || 'agentMD';
  document.getElementById('cmd-md-scope').value = command.md_scope || 'global';
  document.getElementById('cmd-title').value = command.title || '';
  document.getElementById('cmd-description').value = command.description || '';
  document.getElementById('cmd-body').value = command.body || '';
  document.getElementById('editor-status').textContent = '';
  renderPreview();
}

function hideEditor() {
  editing = null;
  document.getElementById('editor').classList.add('hidden');
}

function saveCommand() {
  if (!editing) { return; }
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
  api('/api/commands/user/' + encodeURIComponent(kind) + '/' + encodeURIComponent(name), {method: 'PUT', body: JSON.stringify(payload)}).then(() => {
    document.getElementById('editor-status').textContent = 'Saved';
    document.getElementById('editor-status').className = 'status ok';
    loadState();
    editing = {name, scope: 'user', kind, md_scope: mdScope, title: payload.title, description: payload.description, body};
    document.getElementById('cmd-name').disabled = true;
    document.getElementById('editor-legend').textContent = 'Edit ' + name;
  }).catch((error) => {
    document.getElementById('editor-status').textContent = error.message;
    document.getElementById('editor-status').className = 'status error';
  });
}

function deleteCommand() {
  if (!editing || !editing.name) { return; }
  api('/api/commands/user/' + encodeURIComponent(editing.kind) + '/' + encodeURIComponent(editing.name), {method: 'DELETE'}).then(() => {
    hideEditor();
    loadState();
  }).catch((error) => {
    document.getElementById('editor-status').textContent = error.message;
    document.getElementById('editor-status').className = 'status error';
  });
}

function escapeHtml(text) {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function markdownToHtml(source) {
  const lines = source.split('\n');
  const out = [];
  let inCode = false;
  let inList = false;
  for (const raw of lines) {
    const line = raw;
    if (line.trim().startsWith('```')) {
      if (inCode) { out.push('</pre>'); inCode = false; } else { out.push('<pre>'); inCode = true; }
      continue;
    }
    if (inCode) { out.push(escapeHtml(line)); continue; }
    if (/^\s*[-*]\s+/.test(line)) {
      if (!inList) { out.push('<ul>'); inList = true; }
      out.push('<li>' + inline(line.replace(/^\s*[-*]\s+/, '')) + '</li>');
      continue;
    }
    if (inList) { out.push('</ul>'); inList = false; }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = heading[1].length;
      out.push('<h' + level + '>' + inline(heading[2]) + '</h' + level + '>');
      continue;
    }
    if (line.trim() === '') { continue; }
    out.push('<p>' + inline(line) + '</p>');
  }
  if (inList) { out.push('</ul>'); }
  if (inCode) { out.push('</pre>'); }
  return out.join('\n');
}

function inline(text) {
  let html = escapeHtml(text);
  html = html.replace(/`([^`]+)`/g, '<code>$1</code>');
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" rel="noreferrer">$1</a>');
  return html;
}

function renderPreview() {
  document.getElementById('cmd-preview').innerHTML = markdownToHtml(document.getElementById('cmd-body').value);
}

loadState();
</script>
</body>
</html>
"#;
