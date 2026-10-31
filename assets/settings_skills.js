// Skills 设置页：目录列表 + 详情编辑；.agents 来源只读。

const SKILL_NEW_DEFAULT = {
  name: '',
  scope: 'user',
  description: '',
  body: ''
};

let skillsState = null;
let skillsSelected = null;
let selectedSkillPath = null;
let skillsNew = false;
let skillScopeSelect = null;
const skillBaselines = new Map();
const skillDrafts = new Map();
let skillNewDraft = null;

function entryToSkillRecord(entry, prevBase) {
  return {
    path: entry.path,
    name: entry.name,
    scope: entry.scope,
    editable: !!entry.editable,
    enabled: !!entry.enabled,
    description: (prevBase && prevBase.bodyLoaded) ? prevBase.description : (entry.description || ''),
    body: (prevBase && prevBase.bodyLoaded) ? prevBase.body : '',
    bodyLoaded: !!(prevBase && prevBase.bodyLoaded)
  };
}

function isSkillRecordEqual(a, b) {
  if (!a || !b) { return false; }
  if (!!a.enabled !== !!b.enabled) { return false; }
  if (!b.editable || !a.bodyLoaded || !b.bodyLoaded) { return true; }
  return a.description === b.description && a.body === b.body;
}

function syncSkillBaselines(entries, discardDrafts) {
  if (discardDrafts) {
    skillDrafts.clear();
    skillNewDraft = null;
  }
  const previous = discardDrafts ? new Map() : new Map(skillBaselines);
  skillBaselines.clear();
  for (const entry of entries) {
    skillBaselines.set(entry.path, entryToSkillRecord(entry, previous.get(entry.path)));
  }
  for (const [path, draft] of Array.from(skillDrafts.entries())) {
    if (!skillBaselines.has(path) || isSkillRecordEqual(draft, skillBaselines.get(path))) {
      skillDrafts.delete(path);
    }
  }
  if (!skillsNew && selectedSkillPath) {
    skillsSelected = entries.find((entry) => entry.path === selectedSkillPath) || null;
    if (!skillsSelected) {
      selectedSkillPath = null;
      document.getElementById('skill-empty').classList.remove('hidden');
      document.getElementById('skill-editor').classList.add('hidden');
    } else if (discardDrafts) {
      selectSkill(skillsSelected);
    } else if (!skillDrafts.has(selectedSkillPath)) {
      applySkillRecordToEditor(skillBaselines.get(selectedSkillPath), false);
    }
  } else if (skillsNew && discardDrafts) {
    skillsNew = false;
    skillsSelected = null;
    selectedSkillPath = null;
    document.getElementById('skill-empty').classList.remove('hidden');
    document.getElementById('skill-editor').classList.add('hidden');
  }
}

function loadSkills(discardDrafts) {
  const status = document.getElementById('skills-status');
  if (status) { status.textContent = 'Loading…'; }
  return api('/api/skills').then((data) => {
    skillsState = data;
    syncSkillBaselines(data.entries || [], !!discardDrafts);
    renderSkillList();
    if (status) { status.textContent = data.entries.length + ' skill(s)'; }
    refreshActionBars();
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

function effectiveSkillEnabled(entry) {
  if (skillDrafts.has(entry.path)) {
    return !!skillDrafts.get(entry.path).enabled;
  }
  if (skillBaselines.has(entry.path)) {
    return !!skillBaselines.get(entry.path).enabled;
  }
  return !!entry.enabled;
}

function renderSkillList() {
  syncSkillEditorToDraft();
  const list = document.getElementById('skill-list');
  if (!list || !skillsState) { return; }
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
      const isSelected = !skillsNew && selectedSkillPath === entry.path;
      const isDirty = isSkillItemDirty(entry.path);
      const row = document.createElement('div');
      row.className = 'nav-item' + (isSelected ? ' selected' : '') + (isDirty ? ' dirty' : '');
      row.dataset.path = entry.path;
      const box = document.createElement('input');
      box.type = 'checkbox';
      box.checked = effectiveSkillEnabled(entry);
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
      row.onclick = () => selectSkill(entry);
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
  const newRow = document.getElementById('new-skill-row');
  if (newRow) {
    newRow.classList.toggle('selected', skillsNew);
    newRow.classList.toggle('dirty', isSkillNewDirty());
  }
}

function updateSkillDirtyDots() {
  syncSkillEditorToDraft();
  const list = document.getElementById('skill-list');
  if (list) {
    list.querySelectorAll('.nav-item').forEach((row) => {
      const path = row.dataset.path;
      if (path) { row.classList.toggle('dirty', isSkillItemDirty(path)); }
    });
  }
  const newRow = document.getElementById('new-skill-row');
  if (newRow) {
    newRow.classList.toggle('selected', skillsNew);
    newRow.classList.toggle('dirty', isSkillNewDirty());
  }
}

function skillScopeOptions() {
  const projects = (skillsState && skillsState.projects) || [];
  return [{value: 'user', label: 'User'}].concat(projects.filter((project) => project.trusted).map((project) => ({value: project.scope, label: project.name})));
}

function selectSkill(entry) {
  syncSkillEditorToDraft();
  skillsNew = false;
  skillsSelected = entry;
  selectedSkillPath = entry.path;
  if (!skillBaselines.has(entry.path)) {
    skillBaselines.set(entry.path, entryToSkillRecord(entry, null));
  }
  const base = skillBaselines.get(entry.path);
  const current = skillDrafts.get(entry.path) || base;
  applySkillRecordToEditor(current, false);
  renderSkillList();
  refreshActionBars();
  if (base.bodyLoaded) { return; }
  api('/api/skills/detail?path=' + encodeURIComponent(entry.path)).then((detail) => {
    base.description = detail.description;
    base.body = detail.body;
    base.scope = detail.scope || base.scope;
    base.bodyLoaded = true;
    if (skillDrafts.has(entry.path)) {
      const draft = skillDrafts.get(entry.path);
      if (!draft.bodyLoaded) {
        draft.description = detail.description;
        draft.body = detail.body;
        draft.scope = detail.scope || draft.scope;
        draft.bodyLoaded = true;
      }
      if (isSkillRecordEqual(draft, base)) {
        skillDrafts.delete(entry.path);
      }
    }
    if (!skillsNew && selectedSkillPath === entry.path) {
      applySkillRecordToEditor(skillDrafts.get(entry.path) || base, false);
    }
    renderSkillList();
    refreshActionBars();
  }).catch((error) => {
    showToast(error.message, 'error');
    refreshActionBars();
  });
}

function selectSkillByPath(path) {
  if (!skillsState || !Array.isArray(skillsState.entries)) { return; }
  const match = skillsState.entries.find((entry) => entry.path === path);
  if (match) { selectSkill(match); }
}

function newSkill() {
  syncSkillEditorToDraft();
  skillsNew = true;
  skillsSelected = null;
  selectedSkillPath = null;
  if (!skillNewDraft) {
    skillNewDraft = Object.assign({}, SKILL_NEW_DEFAULT);
  }
  applySkillRecordToEditor(skillNewDraft, true);
  renderSkillList();
  refreshActionBars();
}

function showSkillEditor(entry) {
  if (!entry) {
    applySkillRecordToEditor(skillNewDraft || Object.assign({}, SKILL_NEW_DEFAULT), true);
  } else {
    applySkillRecordToEditor(skillDrafts.get(entry.path) || skillBaselines.get(entry.path) || entryToSkillRecord(entry, null), false);
  }
}

function applySkillRecordToEditor(record, isNew) {
  document.getElementById('skill-empty').classList.add('hidden');
  document.getElementById('skill-editor').classList.remove('hidden');
  document.getElementById('skill-legend').textContent = isNew ? 'New skill' : ('Edit ' + record.name);
  const scopeHost = document.getElementById('skill-scope-host');
  const scopeValue = record.scope || 'user';
  scopeHost.innerHTML = '';
  skillScopeSelect = createSelect(scopeHost, {
    ariaLabel: 'Scope',
    options: skillScopeOptions(),
    value: scopeValue,
    disabled: !isNew,
    onChange: (next) => { scopeHost.dataset.value = next; }
  });
  scopeHost.dataset.value = scopeValue;

  const name = document.getElementById('skill-name');
  name.value = record.name || '';
  name.disabled = !isNew;
  document.getElementById('skill-path').value = isNew ? '' : (record.path || '');
  document.getElementById('skill-description').value = record.description || '';
  document.getElementById('skill-body').value = record.body || '';
  const enabledBox = document.getElementById('skill-enabled');
  enabledBox.checked = isNew ? true : !!record.enabled;
  enabledBox.disabled = isNew;
  const editable = isNew || (!!record.editable && !!record.bodyLoaded);
  document.getElementById('skill-description').disabled = !editable;
  document.getElementById('skill-body').disabled = !editable;
  const remove = document.querySelector('#skill-editor .danger');
  if (remove) { remove.disabled = isNew || !record.editable; }
  document.getElementById('skill-body').placeholder = '';
}

function skillPath(name, scope) {
  return '/api/skills/' + encodeURIComponent(scope) + '/' + encodeURIComponent(name);
}

function skillSnapshot() {
  return {
    name: document.getElementById('skill-name').value.trim(),
    scope: (document.getElementById('skill-scope-host').dataset.value) || 'user',
    description: document.getElementById('skill-description').value,
    body: document.getElementById('skill-body').value
  };
}

function syncSkillEditorToDraft() {
  const editor = document.getElementById('skill-editor');
  if (!editor || editor.classList.contains('hidden')) { return; }
  if (skillsNew) {
    skillNewDraft = skillSnapshot();
    return;
  }
  if (!selectedSkillPath || !skillBaselines.has(selectedSkillPath)) { return; }
  const base = skillBaselines.get(selectedSkillPath);
  const existing = skillDrafts.get(selectedSkillPath);
  const next = Object.assign({}, existing || base);
  next.enabled = document.getElementById('skill-enabled').checked;
  if (base.editable && base.bodyLoaded) {
    next.description = document.getElementById('skill-description').value;
    next.body = document.getElementById('skill-body').value;
    next.bodyLoaded = true;
  }
  if (isSkillRecordEqual(next, base)) {
    skillDrafts.delete(selectedSkillPath);
  } else {
    skillDrafts.set(selectedSkillPath, next);
  }
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

function resetSkillEditor() {
  skillDrafts.clear();
  skillNewDraft = null;
  if (skillsNew) {
    skillNewDraft = Object.assign({}, SKILL_NEW_DEFAULT);
    applySkillRecordToEditor(skillNewDraft, true);
  } else if (selectedSkillPath && skillBaselines.has(selectedSkillPath)) {
    applySkillRecordToEditor(skillBaselines.get(selectedSkillPath), false);
  }
  renderSkillList();
  refreshActionBars();
}

async function saveSkill() {
  syncSkillEditorToDraft();
  const dirtyPaths = Array.from(skillDrafts.keys()).filter(isSkillItemDirty);
  const saveNew = isSkillNewDirty();
  if (dirtyPaths.length === 0 && !saveNew) { return; }

  let createdTarget = null;
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
      const name = (skillNewDraft.name || '').trim();
      const scope = skillNewDraft.scope || 'user';
      if (!name) {
        newSkill();
        throw new Error('skill name is required');
      }
      const duplicate = Array.from(skillBaselines.values()).some((item) => item.scope === scope && item.name === name);
      if (duplicate) {
        newSkill();
        throw new Error('Skill "' + name + '" already exists');
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
      createdTarget = {name, scope, body: skillNewDraft.body, description: skillNewDraft.description};
      skillNewDraft = null;
    }

    showToast('skill saved', 'ok');
    await loadSkills(false);
    if (createdTarget && skillsState && Array.isArray(skillsState.entries)) {
      const saved = skillsState.entries.find((entry) => entry.name === createdTarget.name && entry.scope === createdTarget.scope);
      if (saved) {
        const base = skillBaselines.get(saved.path);
        if (base) {
          base.description = createdTarget.description;
          base.body = createdTarget.body;
          base.bodyLoaded = true;
        }
        if (skillsNew) {
          selectSkill(saved);
        }
      }
    }
    refreshActionBars();
  } catch (error) {
    await loadSkills(false);
    showToast(error.message, 'error');
    throw error;
  }
}

function deleteSkill() {
  if (skillsNew) {
    hideSkillEditor();
    return;
  }
  if (!skillsSelected) { return; }
  if (!window.confirm('Delete skill "' + skillsSelected.name + '"?')) { return; }
  const deletingPath = selectedSkillPath;
  api(skillPath(skillsSelected.name, skillsSelected.scope), {method: 'DELETE'}).then(() => {
    if (deletingPath) {
      skillDrafts.delete(deletingPath);
      skillBaselines.delete(deletingPath);
    }
    showToast('skill deleted', 'ok');
    hideSkillEditor();
    return loadSkills(false);
  }).catch((error) => showToast(error.message, 'error'));
}

function hideSkillEditor() {
  if (skillsNew) {
    skillNewDraft = null;
  } else if (selectedSkillPath) {
    skillDrafts.delete(selectedSkillPath);
  }
  document.getElementById('skill-empty').classList.remove('hidden');
  document.getElementById('skill-editor').classList.add('hidden');
  skillsNew = false;
  skillsSelected = null;
  selectedSkillPath = null;
  renderSkillList();
  refreshActionBars();
}

function setSkillEnabled(entry, enabled) {
  syncSkillEditorToDraft();
  if (!skillBaselines.has(entry.path)) {
    skillBaselines.set(entry.path, entryToSkillRecord(entry, null));
  }
  const base = skillBaselines.get(entry.path);
  const draft = skillDrafts.has(entry.path)
    ? Object.assign({}, skillDrafts.get(entry.path))
    : Object.assign({}, base);
  draft.enabled = !!enabled;
  if (isSkillRecordEqual(draft, base)) {
    skillDrafts.delete(entry.path);
  } else {
    skillDrafts.set(entry.path, draft);
  }
  if (!skillsNew && selectedSkillPath === entry.path) {
    document.getElementById('skill-enabled').checked = !!enabled;
  }
  renderSkillList();
  refreshActionBars();
}

function toggleSkillEnabled() {
  if (!skillsSelected) { return; }
  const enabled = document.getElementById('skill-enabled').checked;
  setSkillEnabled(skillsSelected, enabled);
}

actionBars.skills = createActionBar('actions-skills', {
  isDirty: skillDirty,
  save: saveSkill,
  reset: resetSkillEditor
});
