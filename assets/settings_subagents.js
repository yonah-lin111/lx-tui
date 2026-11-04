// ===== Subagents 视图：并发上限、内置角色覆写/重置与自定义角色 CRUD =====

let subagentsState = null;
let subagentsSelectedName = null;
let subagentsNew = false;
let subagentBaseRoleSelect = null;
let subagentModelSelect = null;
let subagentEffortSelect = null;

let subagentsBaselineMaxConcurrent = 3;
const subagentBaselines = new Map();
const subagentDrafts = new Map();
let subagentNewDraft = null;

const NEW_SUBAGENT_ROLE_DEFAULT = {
  name: '',
  isBuiltin: false,
  baseRole: 'explorer',
  description: '',
  systemPrompt: '',
  modelKey: '',
  variant: '',
  nicknames: ''
};

function modelKeyOf(modelObj) {
  if (!modelObj || !modelObj.provider || !modelObj.model) { return ''; }
  return modelObj.provider + '::' + modelObj.model;
}

function parseModelKey(key, variant) {
  if (!key || !key.includes('::')) { return null; }
  const parts = key.split('::');
  const provider = (parts[0] || '').trim();
  const model = parts.slice(1).join('::').trim();
  if (!provider || !model) { return null; }
  const out = {provider: provider, model: model};
  const v = (variant || '').trim();
  if (v) { out.variant = v; }
  return out;
}

function parseNicknamesInput(text) {
  return (text || '')
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function builtinRecordFromState(name, overrideObj, defaultsObj) {
  const ov = overrideObj || {};
  const def = defaultsObj || {};
  const m = ov.model || null;
  return {
    name: name,
    isBuiltin: true,
    baseRole: name,
    description: (ov.description && ov.description.trim()) ? ov.description.trim() : (def.description || ''),
    systemPrompt: (ov.systemPrompt && ov.systemPrompt.trim()) ? ov.systemPrompt.trim() : (def.systemPrompt || ''),
    modelKey: modelKeyOf(m),
    variant: (m && m.variant) ? m.variant : '',
    nicknames: Array.isArray(ov.nicknameCandidates) ? ov.nicknameCandidates.join(', ') : ''
  };
}

function customRecordFromState(roleObj) {
  const m = roleObj.model || null;
  return {
    name: roleObj.name || '',
    isBuiltin: false,
    baseRole: roleObj.baseRole || 'explorer',
    description: roleObj.description || '',
    systemPrompt: roleObj.systemPrompt || '',
    modelKey: modelKeyOf(m),
    variant: (m && m.variant) ? m.variant : '',
    nicknames: Array.isArray(roleObj.nicknameCandidates) ? roleObj.nicknameCandidates.join(', ') : ''
  };
}

function normalizeSubagentDraft(rec) {
  return {
    name: (rec.name || '').trim(),
    isBuiltin: !!rec.isBuiltin,
    baseRole: rec.baseRole || 'explorer',
    description: (rec.description || '').trim(),
    systemPrompt: (rec.systemPrompt || '').trim(),
    modelKey: rec.modelKey || '',
    variant: rec.variant || '',
    nicknames: parseNicknamesInput(rec.nicknames).join(', ')
  };
}

function isSubagentRoleDirty(name) {
  const base = subagentBaselines.get(name);
  const draft = subagentDrafts.get(name);
  if (!base || !draft) { return false; }
  return JSON.stringify(normalizeSubagentDraft(base)) !== JSON.stringify(normalizeSubagentDraft(draft));
}

function isSubagentNewDirty() {
  if (!subagentNewDraft) { return false; }
  return JSON.stringify(normalizeSubagentDraft(subagentNewDraft)) !== JSON.stringify(normalizeSubagentDraft(NEW_SUBAGENT_ROLE_DEFAULT));
}

function currentMaxConcurrentValue() {
  const el = document.getElementById('subagents-max-concurrent');
  return el ? Number(el.value) : subagentsBaselineMaxConcurrent;
}

function subagentsDirty() {
  syncSubagentEditorToDraft();
  if (subagentsState === null) { return false; }
  if (currentMaxConcurrentValue() !== subagentsBaselineMaxConcurrent) { return true; }
  for (const name of subagentDrafts.keys()) {
    if (isSubagentRoleDirty(name)) { return true; }
  }
  return isSubagentNewDirty();
}

function ensureSubagentSelects() {
  if (!subagentBaseRoleSelect) {
    subagentBaseRoleSelect = createSelect(document.getElementById('subagent-base-role-host'), {
      options: [
        {value: 'explorer', label: 'explorer (read-only)'},
        {value: 'executor', label: 'executor (read & write)'}
      ],
      value: 'explorer',
      onChange: () => { refreshActionBars(); }
    });
  }
  if (!subagentModelSelect) {
    subagentModelSelect = createSelect(document.getElementById('subagent-model-host'), {
      options: [{value: '', label: 'Inherit parent model'}],
      value: '',
      onChange: (newKey) => {
        rebuildEffortOptions(newKey, '');
        refreshActionBars();
      }
    });
  }
  if (!subagentEffortSelect) {
    subagentEffortSelect = createSelect(document.getElementById('subagent-effort-host'), {
      options: [{value: '', label: 'Inherit / model default'}],
      value: '',
      onChange: () => { refreshActionBars(); }
    });
  }
}

function rebuildModelOptions(selectedKey, selectedVariant) {
  ensureSubagentSelects();
  const opts = [{value: '', label: 'Inherit parent model'}];
  const models = (subagentsState && Array.isArray(subagentsState.models)) ? subagentsState.models : [];
  for (const m of models) {
    const k = m.provider + '::' + m.model;
    opts.push({value: k, label: m.label || (m.provider + ' / ' + m.model)});
  }
  if (selectedKey && !opts.some((o) => o.value === selectedKey)) {
    opts.push({value: selectedKey, label: selectedKey.replace('::', ' / ')});
  }
  subagentModelSelect.setOptions(opts);
  subagentModelSelect.setValue(selectedKey || '');
  rebuildEffortOptions(selectedKey || '', selectedVariant || '');
}

function rebuildEffortOptions(modelKey, selectedVariant) {
  ensureSubagentSelects();
  const opts = [{value: '', label: 'Inherit / model default'}];
  const models = (subagentsState && Array.isArray(subagentsState.models)) ? subagentsState.models : [];
  const matched = models.find((m) => (m.provider + '::' + m.model) === modelKey);
  const variants = (matched && Array.isArray(matched.variants)) ? matched.variants : [];
  for (const v of variants) {
    opts.push({value: v, label: v});
  }
  if (selectedVariant && !opts.some((o) => o.value === selectedVariant)) {
    opts.push({value: selectedVariant, label: selectedVariant});
  }
  subagentEffortSelect.setOptions(opts);
  subagentEffortSelect.setValue(selectedVariant || '');
  subagentEffortSelect.setDisabled(opts.length <= 1);
}

function readSubagentEditorValues() {
  ensureSubagentSelects();
  return {
    name: document.getElementById('subagent-name').value.trim(),
    baseRole: subagentBaseRoleSelect.value() || 'explorer',
    modelKey: subagentModelSelect.value() || '',
    variant: subagentEffortSelect.value() || '',
    nicknames: document.getElementById('subagent-nicknames').value,
    description: document.getElementById('subagent-description').value,
    systemPrompt: document.getElementById('subagent-prompt').value
  };
}

function syncSubagentEditorToDraft() {
  const editor = document.getElementById('subagent-editor');
  if (!editor || editor.classList.contains('hidden')) { return; }
  const values = readSubagentEditorValues();
  if (subagentsNew) {
    subagentNewDraft = Object.assign({}, NEW_SUBAGENT_ROLE_DEFAULT, values, {isBuiltin: false});
    return;
  }
  if (!subagentsSelectedName) { return; }
  const base = subagentBaselines.get(subagentsSelectedName);
  if (!base) { return; }
  const updated = Object.assign({}, base, values, {
    name: base.isBuiltin ? base.name : values.name,
    baseRole: base.isBuiltin ? base.baseRole : values.baseRole,
    isBuiltin: base.isBuiltin
  });
  subagentDrafts.set(subagentsSelectedName, updated);
  if (!isSubagentRoleDirty(subagentsSelectedName)) {
    subagentDrafts.delete(subagentsSelectedName);
  }
}

async function loadSubagents(showReloadToast) {
  const statusEl = document.getElementById('subagents-status');
  statusEl.textContent = 'Loading…';
  try {
    const data = await api('/api/subagents');
    subagentsState = data;
    subagentsBaselineMaxConcurrent = Number(data.maxConcurrent || 3);
    const maxInput = document.getElementById('subagents-max-concurrent');
    if (maxInput) { maxInput.value = String(subagentsBaselineMaxConcurrent); }

    const previousName = subagentsSelectedName;
    subagentBaselines.clear();
    subagentDrafts.clear();
    subagentNewDraft = null;

    const builtins = data.builtins || {};
    const defaults = data.defaults || {};
    subagentBaselines.set('explorer', builtinRecordFromState('explorer', builtins.explorer, defaults.explorer));
    subagentBaselines.set('executor', builtinRecordFromState('executor', builtins.executor, defaults.executor));

    for (const r of (data.roles || [])) {
      const rec = customRecordFromState(r);
      subagentBaselines.set(rec.name, rec);
    }

    const totalCustom = (data.roles || []).length;
    statusEl.textContent = '2 built-in + ' + totalCustom + ' custom role(s)';

    const keepName = (previousName && subagentBaselines.has(previousName)) ? previousName : 'explorer';
    subagentsNew = false;
    subagentsSelectedName = null;
    selectSubagentRole(keepName, {skipSync: true});
    renderSubagentList();
    if (showReloadToast) { showToast('Subagents reloaded', 'info'); }
  } catch (error) {
    statusEl.textContent = 'Failed: ' + error.message;
    showToast(error.message, 'error');
  }
}

function renderSubagentList() {
  const list = document.getElementById('subagent-role-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  for (const [name, base] of subagentBaselines.entries()) {
    const row = element('button', 'nav-item' + (!subagentsNew && subagentsSelectedName === name ? ' selected' : ''));
    row.type = 'button';
    row.dataset.name = name;
    row.classList.toggle('dirty', isSubagentRoleDirty(name));
    const title = element('span', 'nav-name', name);
    const tag = element('span', 'tag', base.isBuiltin ? 'built-in · ' + base.baseRole : 'custom · ' + base.baseRole);
    row.appendChild(title);
    row.appendChild(tag);
    row.onclick = () => selectSubagentRole(name);
    list.appendChild(row);
  }
  const newRow = document.getElementById('new-subagent-role-row');
  if (newRow) {
    newRow.classList.toggle('selected', subagentsNew);
    newRow.classList.toggle('dirty', isSubagentNewDirty());
  }
  refreshActionBars();
}

function updateSubagentDirtyDots() {
  syncSubagentEditorToDraft();
  const list = document.getElementById('subagent-role-list');
  if (list) {
    list.querySelectorAll('.nav-item').forEach((row) => {
      const name = row.dataset.name;
      if (name) { row.classList.toggle('dirty', isSubagentRoleDirty(name)); }
    });
  }
  const newRow = document.getElementById('new-subagent-role-row');
  if (newRow) {
    newRow.classList.toggle('selected', subagentsNew);
    newRow.classList.toggle('dirty', isSubagentNewDirty());
  }
}

function selectSubagentRole(name, opts) {
  if (!(opts && opts.skipSync)) {
    syncSubagentEditorToDraft();
  }
  const base = subagentBaselines.get(name);
  if (!base) { return; }
  subagentsNew = false;
  subagentsSelectedName = name;
  const active = subagentDrafts.get(name) || base;
  showSubagentEditor(active);
  renderSubagentList();
}

function newSubagentRole() {
  syncSubagentEditorToDraft();
  subagentsNew = true;
  subagentsSelectedName = null;
  if (!subagentNewDraft) {
    subagentNewDraft = Object.assign({}, NEW_SUBAGENT_ROLE_DEFAULT);
  }
  showSubagentEditor(subagentNewDraft);
  renderSubagentList();
}

function showSubagentEditor(rec) {
  ensureSubagentSelects();
  document.getElementById('subagent-empty').classList.add('hidden');
  document.getElementById('subagent-editor').classList.remove('hidden');
  document.getElementById('subagent-legend').textContent = subagentsNew
    ? 'New custom role'
    : (rec.isBuiltin ? ('Built-in role: ' + rec.name) : ('Custom role: ' + rec.name));

  const nameInput = document.getElementById('subagent-name');
  nameInput.value = rec.name || '';
  nameInput.readOnly = !!rec.isBuiltin;

  subagentBaseRoleSelect.setValue(rec.baseRole || 'explorer');
  subagentBaseRoleSelect.setDisabled(!!rec.isBuiltin);

  rebuildModelOptions(rec.modelKey || '', rec.variant || '');

  document.getElementById('subagent-nicknames').value = rec.nicknames || '';
  document.getElementById('subagent-description').value = rec.description || '';
  document.getElementById('subagent-prompt').value = rec.systemPrompt || '';

  document.getElementById('subagent-reset-default').classList.toggle('hidden', !rec.isBuiltin);
  document.getElementById('subagent-delete-role').classList.toggle('hidden', !!rec.isBuiltin || subagentsNew);
}

function hideSubagentEditor() {
  if (subagentsNew) {
    subagentNewDraft = null;
    subagentsNew = false;
  } else if (subagentsSelectedName) {
    subagentDrafts.delete(subagentsSelectedName);
  }
  document.getElementById('subagent-editor').classList.add('hidden');
  document.getElementById('subagent-empty').classList.remove('hidden');
  subagentsSelectedName = null;
  renderSubagentList();
}

async function resetBuiltinRoleToDefault() {
  if (!subagentsSelectedName || (subagentsSelectedName !== 'explorer' && subagentsSelectedName !== 'executor')) {
    return;
  }
  try {
    await api('/api/subagents/builtins/' + encodeURIComponent(subagentsSelectedName), {method: 'DELETE'});
    showToast('Reset ' + subagentsSelectedName + ' to built-in defaults', 'ok');
    hideSubagentEditor();
    await loadSubagents(false);
  } catch (error) {
    showToast(error.message, 'error');
  }
}

async function deleteSubagentRole() {
  if (subagentsNew || !subagentsSelectedName) {
    hideSubagentEditor();
    return;
  }
  const base = subagentBaselines.get(subagentsSelectedName);
  if (!base || base.isBuiltin) { return; }
  try {
    await api('/api/subagents/roles/' + encodeURIComponent(subagentsSelectedName), {method: 'DELETE'});
    showToast('Deleted custom role "' + subagentsSelectedName + '"', 'ok');
    hideSubagentEditor();
    await loadSubagents(false);
  } catch (error) {
    showToast(error.message, 'error');
  }
}

function resetSubagents() {
  const maxInput = document.getElementById('subagents-max-concurrent');
  if (maxInput) { maxInput.value = String(subagentsBaselineMaxConcurrent); }
  const keepName = (!subagentsNew && subagentsSelectedName && subagentBaselines.has(subagentsSelectedName))
    ? subagentsSelectedName
    : 'explorer';
  subagentDrafts.clear();
  subagentNewDraft = null;
  subagentsNew = false;
  selectSubagentRole(keepName, {skipSync: true});
  renderSubagentList();
}

function recordToBuiltinOverride(rec, defaultsObj) {
  const def = defaultsObj || {};
  const desc = (rec.description || '').trim();
  const prompt = (rec.systemPrompt || '').trim();
  return {
    description: (desc && desc !== (def.description || '').trim()) ? desc : null,
    systemPrompt: (prompt && prompt !== (def.systemPrompt || '').trim()) ? prompt : null,
    model: parseModelKey(rec.modelKey, rec.variant),
    nicknameCandidates: parseNicknamesInput(rec.nicknames)
  };
}

function recordToCustomRole(rec) {
  return {
    name: (rec.name || '').trim(),
    baseRole: rec.baseRole || 'explorer',
    description: (rec.description || '').trim(),
    systemPrompt: (rec.systemPrompt || '').trim(),
    model: parseModelKey(rec.modelKey, rec.variant),
    nicknameCandidates: parseNicknamesInput(rec.nicknames)
  };
}

async function saveSubagents() {
  syncSubagentEditorToDraft();
  if (!subagentsDirty()) { return; }

  const defaults = (subagentsState && subagentsState.defaults) || {};
  const explorerRec = subagentDrafts.get('explorer') || subagentBaselines.get('explorer');
  const executorRec = subagentDrafts.get('executor') || subagentBaselines.get('executor');

  const roles = [];
  for (const [name, base] of subagentBaselines.entries()) {
    if (base.isBuiltin) { continue; }
    const effective = subagentDrafts.get(name) || base;
    roles.push(recordToCustomRole(effective));
  }
  let createdRoleName = null;
  if (isSubagentNewDirty()) {
    const newRole = recordToCustomRole(subagentNewDraft);
    roles.push(newRole);
    createdRoleName = newRole.name;
  }

  const payload = {
    maxConcurrent: currentMaxConcurrentValue(),
    builtins: {
      explorer: recordToBuiltinOverride(explorerRec, defaults.explorer),
      executor: recordToBuiltinOverride(executorRec, defaults.executor)
    },
    roles: roles
  };

  try {
    await api('/api/subagents', {
      method: 'PUT',
      body: JSON.stringify(payload)
    });
    if (createdRoleName) {
      subagentsNew = false;
      subagentNewDraft = null;
      subagentsSelectedName = createdRoleName;
    }
    showToast('Subagent settings saved', 'ok');
    await loadSubagents(false);
  } catch (error) {
    showToast(error.message, 'error');
    throw error;
  }
}

actionBars.subagents = createActionBar('actions-subagents', {
  isDirty: subagentsDirty,
  save: saveSubagents,
  reset: resetSubagents
});
