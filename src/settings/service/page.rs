//! 设置页 HTML：单文件自包含（内联 CSS/JS、无外部资源）。

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
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body { margin: 0; font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; display: flex; min-height: 100vh; background: #fbfbfb; color: #222; }
nav { width: 200px; border-right: 1px solid #ddd; padding: 16px 0; background: #f4f4f4; }
nav h1 { font-size: 13px; margin: 0 16px 12px; text-transform: uppercase; letter-spacing: .08em; color: #666; }
nav button { display: block; width: 100%; text-align: left; padding: 8px 16px; border: 0; background: none; font: inherit; cursor: pointer; color: #333; }
nav button.active { background: #e2e2e2; color: #000; font-weight: 600; }
main { flex: 1; padding: 24px; max-width: 860px; }
h2 { font-size: 16px; margin: 0 0 16px; }
fieldset { border: 1px solid #ddd; border-radius: 6px; margin: 0 0 16px; padding: 12px; }
legend { font-size: 12px; color: #666; padding: 0 6px; }
label { display: block; margin: 8px 0 4px; font-size: 12px; color: #555; }
input, select, textarea { width: 100%; padding: 6px 8px; border: 1px solid #ccc; border-radius: 4px; font: inherit; background: #fff; color: inherit; }
textarea { min-height: 180px; resize: vertical; }
.grid { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px 16px; }
.actions { display: flex; gap: 8px; align-items: center; margin: 12px 0; }
button.primary { background: #0a7d4f; border: 0; color: #fff; padding: 7px 14px; border-radius: 4px; cursor: pointer; }
button.ghost { background: none; border: 1px solid #bbb; padding: 6px 12px; border-radius: 4px; cursor: pointer; color: inherit; }
button.danger { background: none; border: 1px solid #c33; color: #c33; padding: 6px 12px; border-radius: 4px; cursor: pointer; }
.row { display: flex; justify-content: space-between; gap: 12px; align-items: center; border-bottom: 1px solid #eee; padding: 8px 0; }
.row:last-child { border-bottom: 0; }
.tag { font-size: 11px; border: 1px solid #bbb; border-radius: 3px; padding: 1px 6px; color: #555; margin-left: 6px; }
.status { font-size: 12px; color: #666; margin-left: 8px; }
.status.error { color: #c33; }
.status.ok { color: #0a7d4f; }
.preview { border: 1px dashed #ccc; border-radius: 4px; padding: 10px; min-height: 120px; background: #fff; overflow-wrap: anywhere; }
.preview h1, .preview h2, .preview h3 { margin: 8px 0 4px; }
.preview pre { background: #f3f3f3; padding: 8px; border-radius: 4px; overflow: auto; }
.preview code { background: #f3f3f3; padding: 0 3px; border-radius: 3px; }
.hidden { display: none; }
.columns { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
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
    <p class="status" id="commands-status">Loading…</p>
    <div class="actions">
      <button class="primary" onclick="newCommand()">New command</button>
      <button class="ghost" onclick="loadState()">Reload</button>
      <label for="cmd-filter" style="margin:0 0 0 12px">Type</label>
      <select id="cmd-filter" onchange="renderCommands()" style="width:auto">
        <option value="all">All</option>
        <option value="agentMD">Markdown insertion</option>
        <option value="agentBlock">Template block</option>
      </select>
    </div>
    <div id="command-list"></div>

    <div id="editor" class="hidden">
      <fieldset>
        <legend id="editor-legend">Edit command</legend>
        <div class="columns">
          <div>
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
            <label for="cmd-body">Markdown body</label>
            <textarea id="cmd-body" oninput="renderPreview()"></textarea>
          </div>
          <div>
            <label>Preview</label>
            <div id="cmd-preview" class="preview"></div>
          </div>
        </div>
      </fieldset>
      <div class="actions">
        <button class="primary" onclick="saveCommand()">Save</button>
        <button class="ghost" onclick="hideEditor()">Cancel</button>
        <button class="danger" onclick="deleteCommand()">Delete</button>
        <span class="status" id="editor-status"></span>
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
