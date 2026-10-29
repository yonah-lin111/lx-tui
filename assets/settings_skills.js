// Skills 设置页：目录列表 + 详情编辑；.agents 来源只读。

let skillsState = null;
let skillsSelected = null;
let skillsNew = false;

function loadSkills() {
  const status = document.getElementById('skills-status');
  if (status) { status.textContent = 'Loading…'; }
  return api('/api/skills').then((data) => {
    skillsState = data;
    renderSkillList();
    if (status) { status.textContent = data.entries.length + ' skill(s)'; }
  }).catch((error) => {
    if (status) { status.textContent = error.message; }
    showToast(error.message, 'error');
  });
}

function skillScopeLabel(scope) {
  if (scope === 'user') { return 'User'; }
  const project = (skillsState.projects || []).find((item) => item.scope === scope);
  return project ? project.name : scope;
}

function renderSkillList() {
  const list = document.getElementById('skill-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  const groups = new Map();
  for (const entry of skillsState.entries) {
    const key = skillScopeLabel(entry.scope) + ' · ' + (entry.source === 'agents' ? '.agents (read-only)' : 'lx');
    if (!groups.has(key)) { groups.set(key, []); }
    groups.get(key).push(entry);
  }
  for (const [label, entries] of groups) {
    const header = document.createElement('div');
    header.className = 'field-hint';
    header.textContent = label;
    list.appendChild(header);
    for (const entry of entries) {
      const row = document.createElement('div');
      row.className = 'nav-item';
      const box = document.createElement('input');
      box.type = 'checkbox';
      box.checked = entry.enabled;
      box.onclick = (event) => {
        event.stopPropagation();
        setSkillEnabled(entry, box.checked);
      };
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'nav-name';
      button.style.background = 'none';
      button.style.border = '0';
      button.style.color = 'inherit';
      button.style.textAlign = 'left';
      button.textContent = entry.name;
      if (entry.shadowed_by) { button.style.opacity = '0.5'; }
      button.onclick = () => selectSkill(entry);
      row.appendChild(box);
      row.appendChild(button);
      list.appendChild(row);
    }
  }
  if (skillsState.errors.length > 0) {
    const header = document.createElement('div');
    header.className = 'field-hint';
    header.textContent = 'Errors';
    list.appendChild(header);
    for (const error of skillsState.errors) {
      const row = document.createElement('div');
      row.className = 'field-hint';
      row.style.color = '#ff6b6b';
      row.textContent = '✕ ' + error.path;
      list.appendChild(row);
    }
  }
}

function skillScopeOptions() {
  return [{value: 'user', label: 'User'}].concat((skillsState.projects || []).filter((project) => project.trusted).map((project) => ({value: project.scope, label: project.name})));
}

function selectSkill(entry) {
  skillsNew = false;
  skillsSelected = entry;
  showSkillEditor(entry);
  api('/api/skills/detail?path=' + encodeURIComponent(entry.path)).then((detail) => {
    if (!skillsSelected || skillsSelected.path !== entry.path) { return; }
    document.getElementById('skill-description').value = detail.description;
    document.getElementById('skill-body').value = detail.body;
    document.getElementById('skill-scope-host').dataset.value = detail.scope;
  }).catch((error) => showToast(error.message, 'error'));
}

function newSkill() {
  skillsNew = true;
  skillsSelected = null;
  showSkillEditor(null);
}

function showSkillEditor(entry) {
  document.getElementById('skill-empty').classList.add('hidden');
  document.getElementById('skill-editor').classList.remove('hidden');
  document.getElementById('skill-legend').textContent = skillsNew ? 'New skill' : ('Edit ' + entry.name);
  const scopeHost = document.getElementById('skill-scope-host');
  const scopeValue = entry ? entry.scope : 'user';
  scopeHost.innerHTML = '';
  createSelect(scopeHost, {
    ariaLabel: 'Scope',
    options: skillScopeOptions(),
    value: scopeValue,
    onChange: (next) => { scopeHost.dataset.value = next; }
  });
  scopeHost.dataset.value = scopeValue;

  const name = document.getElementById('skill-name');
  name.value = entry ? entry.name : '';
  name.disabled = !skillsNew;
  document.getElementById('skill-path').value = entry ? entry.path : '';
  document.getElementById('skill-description').value = entry ? entry.description : '';
  document.getElementById('skill-body').value = '';
  document.getElementById('skill-enabled').checked = entry ? entry.enabled : true;
  const editable = skillsNew || (entry && entry.editable);
  document.getElementById('skill-description').disabled = !editable;
  document.getElementById('skill-body').disabled = !editable;
  const save = document.querySelector('#skill-editor .primary');
  if (save) { save.disabled = !editable; }
  const remove = document.querySelector('#skill-editor .danger');
  if (remove) { remove.disabled = skillsNew || !entry || !entry.editable; }
  document.getElementById('skill-body').placeholder = editable ? '' : '';
}

function skillPath(name, scope) {
  return '/api/skills/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name);
}

function saveSkill() {
  const name = document.getElementById('skill-name').value.trim();
  const scope = (document.getElementById('skill-scope-host').dataset.value) || 'user';
  if (!name) {
    showToast('skill name is required', 'error');
    return;
  }
  const body = {
    description: document.getElementById('skill-description').value,
    body: document.getElementById('skill-body').value
  };
  api(skillPath(name, scope), {method: 'PUT', body: JSON.stringify(body)}).then(() => {
    showToast('skill saved', 'ok');
    return loadSkills();
  }).catch((error) => showToast(error.message, 'error'));
}

function deleteSkill() {
  if (!skillsSelected) { return; }
  if (!window.confirm('Delete skill "' + skillsSelected.name + '"?')) { return; }
  api(skillPath(skillsSelected.name, skillsSelected.scope), {method: 'DELETE'}).then(() => {
    showToast('skill deleted', 'ok');
    hideSkillEditor();
    return loadSkills();
  }).catch((error) => showToast(error.message, 'error'));
}

function hideSkillEditor() {
  document.getElementById('skill-empty').classList.remove('hidden');
  document.getElementById('skill-editor').classList.add('hidden');
  skillsSelected = null;
}

function setSkillEnabled(entry, enabled) {
  api('/api/skills/enabled', {
    method: 'PUT',
    body: JSON.stringify({path: entry.path, enabled})
  }).then(() => {
    entry.enabled = enabled;
    showToast(enabled ? 'skill enabled' : 'skill disabled', 'ok');
  }).catch((error) => {
    showToast(error.message, 'error');
    entry.enabled = !enabled;
    renderSkillList();
  });
}

function toggleSkillEnabled() {
  if (!skillsSelected) { return; }
  const enabled = document.getElementById('skill-enabled').checked;
  setSkillEnabled(skillsSelected, enabled);
}
