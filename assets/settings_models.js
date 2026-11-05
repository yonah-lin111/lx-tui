// ===== Models view =====
// 全局内存草稿：所有编辑（含删除/复制/启停）先落在 modelsDraft，
// Save（视图操作条）一次 PUT 提交全部；Reload / Reset 丢弃草稿回到最近快照。
// 记录键与模型 id 一致（与 lx-agent 语义相同）；模态与思考等级只在展开区编辑。

const INPUT_MODALITIES = ['text', 'image', 'audio', 'video', 'pdf'];
const OUTPUT_MODALITIES = ['text', 'image', 'audio', 'video'];
const TITLE_LANGUAGE_OPTIONS = [
  {value: 'auto', label: 'Auto (follow input)'},
  {value: 'zh-CN', label: 'Simplified Chinese'},
  {value: 'en', label: 'English'},
  {value: 'zh-TW', label: 'Traditional Chinese'},
  {value: 'ja', label: 'Japanese'},
  {value: 'ko', label: 'Korean'}
];
const VARIANT_PRESETS = {
  openai: {low: {reasoningEffort: 'low'}, medium: {reasoningEffort: 'medium'}, high: {reasoningEffort: 'high'}},
  openrouter: {low: {reasoning: {effort: 'low'}}, medium: {reasoning: {effort: 'medium'}}, high: {reasoning: {effort: 'high'}}},
  anthropic: {low: {thinkingBudget: 4096}, high: {thinkingBudget: 16000}, max: {thinkingBudget: 32000}},
  reasoning_effort: {low: {reasoning_effort: 'low'}, medium: {reasoning_effort: 'medium'}, high: {reasoning_effort: 'high'}}
};

let modelsDraft = null;      // 全局编辑草稿
let modelsBaseline = null;   // 最近一次加载/保存的快照（Reset 回填基准）
let selectedProviderKey = null;
let activeProviderTab = 'quick'; // 'quick' | 'custom'
let selectedPresetId = null;
const modelSelects = {};
const expandedModels = new Set();

function suffix() {
  return Date.now().toString(36).slice(-6);
}

// 当前编辑对象：Quick Connect 预设或自定义供应商，字段形状一致。
function currentPreset() {
  if (!modelsDraft) { return null; }
  return (modelsDraft.quickConnect || []).find((preset) => preset.id === selectedPresetId) || null;
}

function currentProvider() {
  if (!modelsDraft) { return null; }
  if (activeProviderTab === 'quick') { return currentPreset(); }
  return modelsDraft.providers.find((provider) => provider.key === selectedProviderKey) || null;
}

function isProviderDirty(provider) {
  if (!modelsBaseline || !Array.isArray(modelsBaseline.providers)) { return false; }
  const base = modelsBaseline.providers.find((item) => item.key === provider.key);
  if (!base) { return true; }
  return JSON.stringify(provider) !== JSON.stringify(base);
}

function isModelSettingsDirty() {
  if (!modelsDraft || !modelsBaseline) { return false; }
  return JSON.stringify({
    defaultModel: modelsDraft.defaultModel,
    titleSummary: modelsDraft.titleSummary,
    titleLanguage: modelsDraft.titleLanguage
  }) !== JSON.stringify({
    defaultModel: modelsBaseline.defaultModel,
    titleSummary: modelsBaseline.titleSummary,
    titleLanguage: modelsBaseline.titleLanguage
  });
}

function isProviderSettingsDirty() {
  if (!modelsDraft || !modelsBaseline) { return false; }
  if (JSON.stringify(modelsDraft.providers) !== JSON.stringify(modelsBaseline.providers)) { return true; }
  // 快捷连接草稿与基线逐项比较（后端会把保存后的连接状态写回 GET）。
  return (modelsDraft.quickConnect || []).some((preset) => {
    const base = (modelsBaseline.quickConnect || []).find((item) => item.id === preset.id);
    return !base || JSON.stringify(preset) !== JSON.stringify(base);
  });
}

function computeModelsDirty() {
  return isModelSettingsDirty() || isProviderSettingsDirty();
}

function updateProviderDirtyDots() {
  const list = document.getElementById('provider-list');
  if (!list || !modelsDraft) { return; }
  for (const row of list.querySelectorAll('.nav-item')) {
    const key = row.dataset.key || '';
    let dirty = false;
    if (activeProviderTab === 'quick') {
      const preset = (modelsDraft.quickConnect || []).find((item) => item.id === key);
      const base = ((modelsBaseline && modelsBaseline.quickConnect) || []).find((item) => item.id === key);
      dirty = !!preset && (!base || JSON.stringify(preset) !== JSON.stringify(base));
    } else {
      const provider = modelsDraft.providers.find((item) => item.key === key);
      dirty = !!provider && isProviderDirty(provider);
    }
    row.classList.toggle('dirty', dirty);
  }
}

function markModelsDirty() {
  updateModelsStatus();
  updateProviderDirtyDots();
  refreshActionBars();
}

function updateModelsStatus() {
  const count = ((modelsDraft && modelsDraft.providers) || []).length;
  const text = count + ' provider(s)';
  const statusEl = document.getElementById('models-status');
  if (statusEl) {
    statusEl.textContent = text;
    statusEl.className = 'status ok';
  }
  const modelStatusEl = document.getElementById('model-settings-status');
  if (modelStatusEl) {
    modelStatusEl.textContent = text;
    modelStatusEl.className = 'status ok';
  }
}

// Reset：把草稿回填为最近一次加载/保存的快照。
function resetModels() {
  if (modelsBaseline === null) { return; }
  hideFetchCandidates();
  modelsDraft = JSON.parse(JSON.stringify(modelsBaseline));
  if (!modelsDraft.providers.some((provider) => provider.key === selectedProviderKey)) {
    selectedProviderKey = null;
  }
  if (!(modelsDraft.quickConnect || []).some((preset) => preset.id === selectedPresetId)) {
    selectedPresetId = null;
  }
  renderModelsView();
}

function loadModels() {
  return api('/api/models/settings').then((data) => {
    modelsDraft = snapshotToDraft(data);
    modelsBaseline = JSON.parse(JSON.stringify(modelsDraft));
    // 默认不选中任何 provider：直接显示空状态。
    if (!modelsDraft.providers.some((provider) => provider.key === selectedProviderKey)) {
      selectedProviderKey = null;
    }
    if (!(modelsDraft.quickConnect || []).some((preset) => preset.id === selectedPresetId)) {
      selectedPresetId = null;
    }
    // 最小化 mock（没有 quickConnect 数据）时直接落到 Custom，保证可编辑。
    if (!(modelsDraft.quickConnect || []).length && modelsDraft.providers.length) {
      switchProviderTab('custom');
    }
    renderModelsView();
    const revEl = document.getElementById('models-revision');
    if (revEl) { revEl.textContent = 'revision ' + String(data.revision || '').slice(0, 8); }
    refreshActionBars();
  }).catch((error) => {
    const msg = 'Failed to load: ' + error.message;
    const statusEl = document.getElementById('models-status');
    if (statusEl) { statusEl.textContent = msg; statusEl.className = 'status error'; }
    const modelStatusEl = document.getElementById('model-settings-status');
    if (modelStatusEl) { modelStatusEl.textContent = msg; modelStatusEl.className = 'status error'; }
  });
}

// 409 后只刷新 revision，保留用户草稿（再次点击 Save 即覆盖）。
function refreshRevision() {
  return api('/api/models/settings').then((data) => {
    if (modelsDraft) { modelsDraft.revision = data.revision || ''; }
    const revEl = document.getElementById('models-revision');
    if (revEl) { revEl.textContent = 'revision ' + String(data.revision || '').slice(0, 8); }
  }).catch((error) => {
    showToast(error.message, 'error');
  });
}

function draftModels(models) {
  return (models || []).map((model) => ({
    id: model.id,
    name: model.name,
    transport: model.transport || null,
    context: model.limit ? String(model.limit.context) : '',
    output: model.limit ? String(model.limit.output) : '',
    variant: model.variant || null,
    variantRows: Object.entries(model.variants || {}).map(([key, value]) => ({key: key, text: JSON.stringify(value)})),
    modalities: normalizedModalities(model.modalities)
  }));
}

function snapshotToDraft(snapshot) {
  return {
    revision: snapshot.revision || '',
    streamIdleTimeoutMs: snapshot.streamIdleTimeoutMs,
    defaultModel: snapshot.defaultModel || null,
    titleSummary: snapshot.titleSummary || null,
    titleLanguage: snapshot.titleLanguage || 'auto',
    providers: (snapshot.providers || []).map((provider) => ({
      key: provider.key,
      id: provider.id,
      type: provider.type,
      name: provider.name,
      baseUrl: provider.baseUrl,
      enabled: !!provider.enabled,
      apiKey: provider.apiKey || '',
      models: draftModels(provider.models)
    })),
    quickConnect: (snapshot.quickConnect || []).map((preset) => ({
      key: preset.id, id: preset.id, type: preset.type, name: preset.name,
      note: preset.note || '', defaultBaseUrl: preset.defaultBaseUrl || '',
      baseUrl: preset.baseUrl || '', enabled: !!preset.enabled, connected: !!preset.connected,
      connectionType: preset.connectionType || null,
      connectionLabel: preset.connectionLabel || 'Not connected',
      apiKey: preset.apiKey || '', formAnswers: Object.assign({}, preset.formAnswers || {}),
      envVars: preset.envVars || [], methods: preset.methods || [],
      models: draftModels(preset.models), defaultModels: draftModels(preset.defaultModels)
    }))
  };
}

function normalizedModalities(value) {
  const input = value && Array.isArray(value.input) ? value.input.filter((item) => INPUT_MODALITIES.includes(item)) : ['text'];
  const output = value && Array.isArray(value.output) ? value.output.filter((item) => OUTPUT_MODALITIES.includes(item)) : ['text'];
  return {input: input, output: output};
}

function renderModelsView() {
  renderProviders();
  renderEditor();
  renderDefaultModel();
  updateModelsStatus();
}

function renderProviders() {
  const list = document.getElementById('provider-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  if (activeProviderTab === 'quick') {
    renderPresetList();
    return;
  }
  const query = providerSearch.trim().toLowerCase();
  const providers = ((modelsDraft && modelsDraft.providers) || []).filter((provider) => {
    if (!query) { return true; }
    return String(provider.name || '').toLowerCase().includes(query)
      || String(provider.id || '').toLowerCase().includes(query);
  });
  if (providers.length === 0) {
    list.appendChild(element('p', 'nav-empty', query ? 'No matching providers.' : 'No providers configured.'));
  }
  for (const provider of providers) {
    const dirty = isProviderDirty(provider);
    const row = element('button', 'nav-item' + (provider.key === selectedProviderKey ? ' selected' : '') + (dirty ? ' dirty' : ''));
    row.type = 'button';
    row.dataset.key = provider.key;
    const name = element('span', 'nav-name', provider.name || provider.id);
    const tag = element('span', 'tag', provider.type + (provider.apiKey ? '' : ' · no key') + (provider.enabled ? '' : ' · off'));
    row.appendChild(name);
    row.appendChild(tag);
    row.onclick = () => selectProvider(provider.key);
    list.appendChild(row);
  }
}

function renderEditor() {
  if (activeProviderTab === 'quick') {
    renderPresetEditor();
    return;
  }
  hidePresetEditor();
  const provider = currentProvider();
  document.getElementById('provider-empty').classList.toggle('hidden', !!provider);
  document.getElementById('provider-editor').classList.toggle('hidden', !provider);
  if (!provider) { return; }
  document.getElementById('provider-legend').textContent = 'Edit ' + (provider.name || provider.id);
  document.getElementById('prov-id').value = provider.id;
  document.getElementById('prov-name').value = provider.name;
  document.getElementById('prov-base-url').value = provider.baseUrl;
  document.getElementById('prov-api-key').value = provider.apiKey;
  document.getElementById('prov-enabled').checked = provider.enabled;
  // 每次切换 provider 都收回明文显示，避免上一条的可见状态带到下一条。
  document.getElementById('prov-api-key').type = 'password';
  document.getElementById('prov-key-reveal').classList.remove('revealed');
  document.getElementById('prov-key-reveal').setAttribute('aria-pressed', 'false');
  document.getElementById('prov-key-reveal').setAttribute('aria-label', 'Show API key');
  modelSelects.provType.setValue(provider.type);
  renderModelList();
}

function selectProvider(key) {
  hideFetchCandidates();
  if (activeProviderTab !== 'custom') { switchProviderTab('custom'); }
  selectedProviderKey = key;
  renderProviders();
  renderEditor();
}

function newProvider() {
  if (!modelsDraft) { return; }
  if (activeProviderTab !== 'custom') { switchProviderTab('custom'); }
  hideFetchCandidates();
  const id = 'provider-' + suffix();
  modelsDraft.providers.push({
    key: id, id: id, type: 'openai-compatible', name: 'New Provider', baseUrl: '',
    enabled: true, apiKey: '', models: []
  });
  selectedProviderKey = id;
  markModelsDirty();
  renderModelsView();
}

function duplicateProvider() {
  const provider = currentProvider();
  if (!provider || !modelsDraft) { return; }
  hideFetchCandidates();
  const id = provider.id + '-copy-' + suffix();
  const copy = JSON.parse(JSON.stringify(provider));
  copy.key = id;
  copy.id = id;
  copy.name = (provider.name || provider.id) + ' copy';
  copy.apiKey = ''; // 复制项默认需重新输入 Key
  modelsDraft.providers.push(copy);
  selectedProviderKey = id;
  markModelsDirty();
  renderModelsView();
}

function cancelProvider() {
  if (!modelsDraft || !selectedProviderKey) { return; }
  hideFetchCandidates();
  const base = modelsBaseline && Array.isArray(modelsBaseline.providers)
    ? modelsBaseline.providers.find((item) => item.key === selectedProviderKey)
    : null;
  const index = modelsDraft.providers.findIndex((item) => item.key === selectedProviderKey);
  if (index >= 0) {
    if (base) {
      modelsDraft.providers[index] = JSON.parse(JSON.stringify(base));
    } else {
      modelsDraft.providers.splice(index, 1);
    }
  }
  selectedProviderKey = null;
  markModelsDirty();
  renderModelsView();
}

function deleteProvider() {
  const provider = currentProvider();
  if (!provider || !modelsDraft) { return; }
  modelsDraft.providers = modelsDraft.providers.filter((item) => item.key !== provider.key);
  if (modelsDraft.defaultModel && modelsDraft.defaultModel.provider === provider.id) {
    modelsDraft.defaultModel = null;
    showToast('Default model cleared', 'info');
  }
  if (modelsDraft.titleSummary && modelsDraft.titleSummary.provider === provider.id) {
    modelsDraft.titleSummary = null;
    showToast('Title model cleared', 'info');
  }
  selectedProviderKey = null;
  markModelsDirty();
  renderModelsView();
}

function addModel() {
  const provider = currentProvider();
  if (!provider) { return; }
  const id = 'model-' + suffix();
  provider.models.push({
    id: id, name: 'New Model', transport: null, context: '', output: '',
    variant: null, variantRows: [], modalities: {input: ['text'], output: ['text']}
  });
  markModelsDirty();
  renderModelList();
  renderDefaultModel();
}

function duplicateModel(provider, model) {
  const copy = JSON.parse(JSON.stringify(model));
  copy.id = (model.id || 'model') + '-copy-' + suffix();
  copy.name = (model.name || model.id) + ' copy';
  const index = provider.models.indexOf(model);
  provider.models.splice(index + 1, 0, copy);
  markModelsDirty();
  renderModelList();
  renderDefaultModel();
}

function removeModel(provider, model) {
  const index = provider.models.indexOf(model);
  if (index >= 0) { provider.models.splice(index, 1); }
  if (modelsDraft.defaultModel && modelsDraft.defaultModel.provider === provider.id && modelsDraft.defaultModel.model === model.id) {
    modelsDraft.defaultModel = null;
    showToast('Default model cleared', 'info');
  }
  if (modelsDraft.titleSummary && modelsDraft.titleSummary.provider === provider.id && modelsDraft.titleSummary.model === model.id) {
    modelsDraft.titleSummary = null;
    showToast('Title model cleared', 'info');
  }
  markModelsDirty();
  renderModelList();
  renderDefaultModel();
}

function labeledInput(captionText, value, onInput) {
  const wrap = element('div');
  wrap.appendChild(element('label', null, captionText));
  const input = document.createElement('input');
  input.type = 'text';
  input.value = value === null || value === undefined ? '' : value;
  input.oninput = () => onInput(input.value);
  wrap.appendChild(input);
  return {wrap: wrap, input: input};
}

function renderModelList() {
  const list = document.getElementById('model-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  const provider = currentProvider();
  if (!provider || provider.models.length === 0) {
    list.appendChild(element('p', 'nav-empty', 'No models yet.'));
    return;
  }
  for (const model of provider.models) {
    list.appendChild(modelCard(provider, model));
  }
}

function modelCard(provider, model) {
  const key = provider.key + '\u0000' + model.id;
  const card = element('div', 'model-card');
  const head = element('div', 'model-head');
  const idField = labeledInput('Model ID', model.id, (value) => { model.id = value; markModelsDirty(); });
  idField.input.addEventListener('change', () => { renderDefaultModel(); });
  const nameField = labeledInput('Display name', model.name, (value) => { model.name = value; markModelsDirty(); });
  nameField.input.addEventListener('change', () => { renderProviders(); renderDefaultModel(); });
  const tools = element('div', 'model-tools');
  const expanded = expandedModels.has(key);
  const advanced = element('button', 'ghost', expanded ? 'Hide' : 'Advanced');
  advanced.type = 'button';
  const more = element('div', 'model-more' + (expanded ? '' : ' hidden'));
  advanced.onclick = () => {
    if (expandedModels.has(key)) { expandedModels.delete(key); } else { expandedModels.add(key); }
    const isOpen = expandedModels.has(key);
    advanced.textContent = isOpen ? 'Hide' : 'Advanced';
    more.classList.toggle('hidden', !isOpen);
  };
  const dup = element('button', 'ghost', 'Duplicate');
  dup.type = 'button';
  dup.onclick = () => duplicateModel(provider, model);
  const remove = element('button', 'danger', 'Remove');
  remove.type = 'button';
  remove.onclick = () => removeModel(provider, model);
  tools.appendChild(advanced);
  tools.appendChild(dup);
  tools.appendChild(remove);
  head.appendChild(idField.wrap);
  head.appendChild(nameField.wrap);
  head.appendChild(tools);
  card.appendChild(head);

  const grid = element('div', 'model-grid-3');
  const contextField = labeledInput('Context', model.context, (value) => { model.context = value; markModelsDirty(); });
  const outputField = labeledInput('Output', model.output, (value) => { model.output = value; markModelsDirty(); });
  const transportWrap = element('div');
  transportWrap.appendChild(element('label', null, 'Transport'));
  const transportHost = element('div', 'select-host');
  transportWrap.appendChild(transportHost);
  grid.appendChild(contextField.wrap);
  grid.appendChild(outputField.wrap);
  grid.appendChild(transportWrap);
  more.appendChild(grid);
  createSelect(transportHost, {
    ariaLabel: 'Model transport',
    value: model.transport || '',
    options: [
      {value: '', label: '(inherit)'},
      {value: 'openai', label: 'openai'},
      {value: 'openai-compatible', label: 'openai-compatible'},
      {value: 'openai-responses', label: 'openai-responses'},
      {value: 'anthropic', label: 'anthropic'},
      {value: 'google', label: 'google'}
    ],
    onChange: (value) => { model.transport = value || null; markModelsDirty(); }
  });
  more.appendChild(modalitySection(model));
  more.appendChild(thinkingSection(model));
  card.appendChild(more);
  return card;
}

function modalitySection(model) {
  const section = element('div', 'model-sub');
  section.appendChild(element('div', 'sub-title', 'Modalities'));
  const grid = element('div', 'check-grid');
  grid.appendChild(modalityGroup('Input', 'input', INPUT_MODALITIES, model));
  grid.appendChild(modalityGroup('Output', 'output', OUTPUT_MODALITIES, model));
  section.appendChild(grid);
  return section;
}

function modalityGroup(title, key, values, model) {
  const wrap = element('div');
  wrap.appendChild(element('div', 'check-title', title));
  const row = element('div', 'check-row');
  for (const value of values) {
    const item = element('label', 'check-item');
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = model.modalities[key].includes(value);
    box.onchange = () => {
      const list = model.modalities[key];
      const index = list.indexOf(value);
      if (box.checked && index < 0) { list.push(value); }
      if (!box.checked && index >= 0) { list.splice(index, 1); }
      markModelsDirty();
    };
    item.appendChild(box);
    item.appendChild(document.createTextNode(' ' + value));
    row.appendChild(item);
  }
  wrap.appendChild(row);
  return wrap;
}

function thinkingSection(model) {
  const section = element('div', 'model-sub');
  const head = element('div', 'sub-head');
  head.appendChild(element('div', 'sub-title', 'Thinking variants'));
  const presetHost = element('div', 'select-host preset-host');
  head.appendChild(presetHost);
  section.appendChild(head);
  const body = element('div', 'variant-body');
  section.appendChild(body);
  const presetSelect = createSelect(presetHost, {
    ariaLabel: 'Add thinking preset',
    placeholder: 'Add preset…',
    value: '',
    options: [
      {value: 'openai', label: 'OpenAI reasoning effort'},
      {value: 'openrouter', label: 'OpenRouter reasoning'},
      {value: 'anthropic', label: 'Anthropic thinking budget'},
      {value: 'reasoning_effort', label: 'reasoning_effort'},
      {value: 'custom', label: 'Custom variant'}
    ],
    onChange: (value) => {
      applyVariantPreset(model, value);
      presetSelect.setValue('');
      markModelsDirty();
      renderVariants(body, model);
    }
  });
  renderVariants(body, model);
  return section;
}

function renderVariants(body, model) {
  while (body.firstChild) { body.removeChild(body.firstChild); }
  if (model.variantRows.length === 0) {
    body.appendChild(element('div', 'nav-empty', 'No thinking variants configured.'));
    return;
  }
  const defaultRow = element('div', 'variant-default');
  defaultRow.appendChild(element('div', 'check-title', 'Default variant'));
  const defaultHost = element('div', 'select-host');
  defaultRow.appendChild(defaultHost);
  body.appendChild(defaultRow);
  createSelect(defaultHost, {
    ariaLabel: 'Default thinking variant',
    value: model.variant || '',
    options: [{value: '', label: '(none)'}].concat(model.variantRows.map((row) => ({value: row.key, label: row.key}))),
    onChange: (value) => { model.variant = value || null; markModelsDirty(); }
  });
  for (const row of model.variantRows) {
    body.appendChild(variantRow(model, row, body));
  }
}

function variantRow(model, row, body) {
  const wrap = element('div', 'variant-row');
  const keyInput = document.createElement('input');
  keyInput.type = 'text';
  keyInput.value = row.key;
  keyInput.setAttribute('aria-label', 'Variant name');
  keyInput.oninput = () => { row.key = keyInput.value; markModelsDirty(); };
  keyInput.onchange = () => {
    const key = keyInput.value.trim();
    const duplicate = model.variantRows.some((other) => other !== row && other.key === key);
    if (!key || duplicate) {
      showToast(duplicate ? ('Duplicate variant name ' + key) : 'Variant name is required', 'error');
      keyInput.value = row.key;
      return;
    }
    if (model.variant === row.key) { model.variant = key; }
    row.key = key;
    keyInput.value = key;
    markModelsDirty();
    renderVariants(body, model);
  };
  const jsonInput = document.createElement('input');
  jsonInput.type = 'text';
  jsonInput.className = 'variant-json';
  jsonInput.value = row.text;
  jsonInput.setAttribute('aria-label', 'Variant JSON');
  jsonInput.oninput = () => { row.text = jsonInput.value; markModelsDirty(); };
  const remove = element('button', 'danger', 'Remove');
  remove.type = 'button';
  remove.onclick = () => {
    const index = model.variantRows.indexOf(row);
    if (index >= 0) { model.variantRows.splice(index, 1); }
    if (model.variant === row.key) {
      model.variant = model.variantRows.length ? model.variantRows[0].key : null;
    }
    markModelsDirty();
    renderVariants(body, model);
  };
  wrap.appendChild(keyInput);
  wrap.appendChild(jsonInput);
  wrap.appendChild(remove);
  return wrap;
}

function applyVariantPreset(model, kind) {
  const named = VARIANT_PRESETS[kind];
  let defaultKey = 'high';
  let preset = named;
  if (!named) {
    let index = model.variantRows.length + 1;
    let key = 'variant-' + index;
    while (model.variantRows.some((row) => row.key === key)) { index += 1; key = 'variant-' + index; }
    preset = {};
    preset[key] = {reasoningEffort: 'high'};
    defaultKey = key;
  }
  for (const [key, config] of Object.entries(preset)) {
    const existing = model.variantRows.find((row) => row.key === key);
    if (existing) { existing.text = JSON.stringify(config); } else { model.variantRows.push({key: key, text: JSON.stringify(config)}); }
  }
  if (!model.variant) { model.variant = defaultKey; }
}

function renderDefaultModel() {
  if (!modelsDraft) { return; }
  // Custom 与快捷连接共用同一份选项；快捷连接附 `(Quick Connect)` 标识。
  const options = [{value: '', label: '(none)'}].concat(defaultModelOptions());
  let value = modelsDraft.defaultModel ? modelsDraft.defaultModel.provider + '\u0000' + modelsDraft.defaultModel.model : '';
  if (value && !options.some((option) => option.value === value)) {
    modelsDraft.defaultModel = null;
    value = '';
    showToast('Default model cleared (its model was renamed or removed)', 'info');
  }
  if (modelSelects.defaultModel) {
    modelSelects.defaultModel.setOptions(options, value);
  }
  let titleValue = modelsDraft.titleSummary ? modelsDraft.titleSummary.provider + '\u0000' + modelsDraft.titleSummary.model : '';
  if (titleValue && !options.some((option) => option.value === titleValue)) {
    modelsDraft.titleSummary = null;
    titleValue = '';
    showToast('Title model cleared (its model was renamed or removed)', 'info');
  }
  if (modelSelects.titleModel) {
    modelSelects.titleModel.setOptions(options, titleValue);
  }
  if (modelSelects.titleLanguage) {
    const lang = modelsDraft.titleLanguage || 'auto';
    modelSelects.titleLanguage.setOptions(TITLE_LANGUAGE_OPTIONS, lang);
  }
}

function buildVariants(model) {
  const variants = {};
  for (const row of model.variantRows) {
    const key = (row.key || '').trim();
    if (!key) { throw new Error('Thinking variant name is required for model ' + (model.id || 'model')); }
    if (Object.prototype.hasOwnProperty.call(variants, key)) {
      throw new Error('Duplicate thinking variant ' + key + ' for model ' + (model.id || 'model'));
    }
    let parsed;
    try {
      parsed = JSON.parse((row.text || '').trim() || '{}');
    } catch (error) {
      throw new Error('Invalid variants JSON for ' + (model.id || 'model') + ' / ' + key + ': ' + error.message);
    }
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new Error('Variant ' + key + ' must be a JSON object');
    }
    variants[key] = parsed;
  }
  return variants;
}

function limitOf(model, modelId) {
  const context = (model.context || '').trim();
  const output = (model.output || '').trim();
  if (context === '' && output === '') { return null; }
  if (context === '' || output === '') {
    throw new Error('Context and Output must both be set for model ' + modelId);
  }
  const contextValue = Number(context);
  const outputValue = Number(output);
  if (!Number.isInteger(contextValue) || contextValue <= 0 || !Number.isInteger(outputValue) || outputValue <= 0) {
    throw new Error('Context/Output must be positive integers for model ' + modelId);
  }
  return {context: contextValue, output: outputValue};
}

function modalitiesOf(model) {
  const input = model.modalities.input.filter((value) => INPUT_MODALITIES.includes(value));
  const output = model.modalities.output.filter((value) => OUTPUT_MODALITIES.includes(value));
  if (input.length === 0 && output.length === 0) { return null; }
  return {input: input, output: output};
}

function resolveSelectionVariant(selection, model) {
  if (selection && selection.variant && Array.isArray(model.variantRows)) {
    if (model.variantRows.some((row) => (row.key || '').trim() === selection.variant)) {
      return selection.variant;
    }
  }
  return model.variant || null;
}

function buildPayload() {
  const seenProviders = new Set();
  const providers = [];
  for (const provider of modelsDraft.providers) {
    try {
      const id = (provider.id || '').trim();
      if (!id) { throw new Error('Provider ID is required'); }
      if (seenProviders.has(id)) { throw new Error('Duplicate provider ID: ' + id); }
      seenProviders.add(id);
      const baseUrl = (provider.baseUrl || '').trim();
      if (!baseUrl) { throw new Error('Base URL is required for provider ' + id); }
      const seenModels = new Set();
      const models = [];
      for (const model of provider.models) {
        const modelId = (model.id || '').trim();
        if (!modelId) { continue; }
        if (seenModels.has(modelId)) { throw new Error('Duplicate model ID ' + modelId + ' on provider ' + id); }
        seenModels.add(modelId);
        const variants = buildVariants(model);
        const variant = model.variant || null;
        if (variant && !Object.prototype.hasOwnProperty.call(variants, variant)) {
          throw new Error('Default thinking variant ' + variant + ' is missing on model ' + modelId);
        }
        models.push({
          id: modelId,
          name: (model.name || '').trim() || modelId,
          transport: model.transport || null,
          limit: limitOf(model, modelId),
          variant: variant,
          variants: variants,
          modalities: modalitiesOf(model)
        });
      }
      const entry = {
        key: id,
        id: id,
        type: provider.type,
        name: (provider.name || '').trim() || id,
        baseUrl: baseUrl,
        // apiKey 三态：非空替换、空字符串清除、字段缺失保留（这里始终显式发送）。
        apiKey: provider.apiKey || '',
        models: models
      };
      providers.push(entry);
    } catch (error) {
      error.tab = 'provider';
      selectedProviderKey = provider.key;
      renderModelsView();
      throw error;
    }
  }
  const enabledProviders = modelsDraft.providers.filter((provider) => provider.enabled).map((provider) => provider.id.trim());
  // 选择可以指向 Custom 供应商，也可以指向已连接且启用的快捷连接供应商。
  const defaultModel = resolveDraftSelection(modelsDraft.defaultModel);
  const titleSummary = resolveDraftSelection(modelsDraft.titleSummary);
  return {
    providers: providers,
    enabledProviders: enabledProviders,
    defaultModel: defaultModel,
    titleSummary: titleSummary,
    titleLanguage: modelsDraft.titleLanguage || 'auto',
    streamIdleTimeoutMs: modelsDraft.streamIdleTimeoutMs,
    revision: modelsDraft.revision
  };
}

// 自定义供应商/模型选择是否真的有改动；仅预设草稿变化时跳过 `ai.json` 提交。
function hasCustomModelChanges() {
  if (!modelsDraft || !modelsBaseline) { return false; }
  if (isModelSettingsDirty()) { return true; }
  return modelsDraft.providers.some((provider) => isProviderDirty(provider));
}

async function saveModels() {
  if (!modelsDraft) { return; }
  try {
    // 快捷连接草稿必须先落盘：`ai.json` 的 defaultModel 校验依赖最新 connect.json。
    if (typeof savePresetDrafts === 'function') { await savePresetDrafts(); }
    if (hasCustomModelChanges()) {
      // connect.json 已变化，先刷新 revision 再提交模型设置，避免误报 409。
      await refreshRevision();
      let payload;
      try {
        payload = buildPayload();
      } catch (error) {
        showToast(error.message, 'error');
        throw error;
      }
      await api('/api/models/settings', {method: 'PUT', body: JSON.stringify(payload)});
    }
    showToast('Saved', 'ok');
    await loadModels();
  } catch (error) {
    // 409：磁盘配置已被别处改动。只刷新 revision，保留草稿，再点一次即覆盖。
    if (error.status === 409) {
      showToast('Configuration changed elsewhere; revision refreshed — click Save again', 'error');
      await refreshRevision();
      throw error;
    }
    showToast(error.message, 'error');
    throw error;
  }
}

function fetchModels() {
  const provider = currentProvider();
  if (!provider) { return; }
  const body = {
    type: provider.type,
    baseUrl: provider.baseUrl.trim(),
    apiKey: provider.apiKey || undefined
  };
  // 快捷连接预设总是带 providerId：OAuth 连接的 Key 不在页面上，需由后端物化。
  if (activeProviderTab === 'quick' || provider.apiKey) { body.providerId = provider.key; }
  api('/api/models/fetch', {method: 'POST', body: JSON.stringify(body)}).then((result) => {
    showToast(result.models.length + ' model(s) found', 'ok');
    renderFetchCandidates(result.models);
  }).catch((error) => {
    showToast(error.message, 'error');
    hideFetchCandidates();
  });
}

function renderFetchCandidates(models) {
  const box = document.getElementById('fetch-candidates');
  while (box.firstChild) { box.removeChild(box.firstChild); }
  box.classList.remove('hidden');
  const provider = currentProvider();
  if (!provider || models.length === 0) {
    box.appendChild(element('p', 'nav-empty', 'No models returned.'));
    return;
  }
  for (const candidate of models) {
    const row = element('div', 'fetch-row');
    const label = element('span', null, candidate.id + (candidate.ownedBy ? ' · ' + candidate.ownedBy : ''));
    const add = element('button', 'ghost', 'Add');
    add.type = 'button';
    add.onclick = () => {
      // 明确选择加入；不覆盖已编辑的同 id 模型。
      if (!provider.models.some((model) => model.id === candidate.id)) {
        provider.models.push({
          id: candidate.id, name: candidate.id, transport: null, context: '', output: '',
          variant: null, variantRows: [], modalities: {input: ['text'], output: ['text']}
        });
        markModelsDirty();
        renderModelList();
        renderDefaultModel();
      }
      add.textContent = 'Added';
    };
    row.appendChild(label);
    row.appendChild(add);
    box.appendChild(row);
  }
}

function hideFetchCandidates() {
  const box = document.getElementById('fetch-candidates');
  while (box.firstChild) { box.removeChild(box.firstChild); }
  box.classList.add('hidden');
}

function initProviderFields() {
  // 小眼睛：在 password / text 之间切换，只影响本次显示，不改动草稿值。
  const reveal = document.getElementById('prov-key-reveal');
  const keyInput = document.getElementById('prov-api-key');
  reveal.addEventListener('click', () => {
    const show = keyInput.type === 'password';
    keyInput.type = show ? 'text' : 'password';
    reveal.classList.toggle('revealed', show);
    reveal.setAttribute('aria-pressed', show ? 'true' : 'false');
    reveal.setAttribute('aria-label', show ? 'Hide API key' : 'Show API key');
    keyInput.focus();
  });
  document.getElementById('prov-name').addEventListener('input', (event) => {
    const provider = currentProvider();
    if (provider) { provider.name = event.target.value; markModelsDirty(); }
  });
  document.getElementById('prov-base-url').addEventListener('input', (event) => {
    const provider = currentProvider();
    if (provider) { provider.baseUrl = event.target.value; markModelsDirty(); }
  });
  document.getElementById('prov-api-key').addEventListener('input', (event) => {
    const provider = currentProvider();
    if (provider) { provider.apiKey = event.target.value; markModelsDirty(); }
  });
  document.getElementById('prov-enabled').addEventListener('change', (event) => {
    const provider = currentProvider();
    if (!provider) { return; }
    provider.enabled = event.target.checked;
    // 关掉默认模型或标题模型所属 provider 时清空对应选择，避免后端校验 422。
    if (!provider.enabled && modelsDraft.defaultModel && modelsDraft.defaultModel.provider === provider.id) {
      modelsDraft.defaultModel = null;
      showToast('Default model cleared (its provider was turned off)', 'info');
    }
    if (!provider.enabled && modelsDraft.titleSummary && modelsDraft.titleSummary.provider === provider.id) {
      modelsDraft.titleSummary = null;
      showToast('Title model cleared (its provider was turned off)', 'info');
    }
    markModelsDirty();
    renderProviders();
    renderDefaultModel();
  });
}

function initModelsView() {
  modelSelects.provType = createSelect(document.getElementById('prov-type-host'), {
    ariaLabel: 'Provider transport type',
    value: 'openai-compatible',
    options: [
      {value: 'openai', label: 'openai'},
      {value: 'openai-compatible', label: 'openai-compatible'},
      {value: 'anthropic', label: 'anthropic'},
      {value: 'google', label: 'google'}
    ],
    onChange: (value) => {
      // Quick Connect 预设的传输协议由预设固定，不随编辑器下拉改变。
      if (activeProviderTab !== 'custom') { return; }
      const provider = currentProvider();
      if (provider) { provider.type = value; markModelsDirty(); }
    }
  });
  modelSelects.defaultModel = createSelect(document.getElementById('default-model-host'), {
    ariaLabel: 'Default model',
    placeholder: '(none)',
    value: '',
    options: [{value: '', label: '(none)'}],
    onChange: (value) => {
      if (!modelsDraft) { return; }
      if (!value) {
        modelsDraft.defaultModel = null;
        markModelsDirty();
        return;
      }
      const parts = value.split('\u0000');
      modelsDraft.defaultModel = {provider: parts[0], model: parts[1], variant: null};
      markModelsDirty();
    }
  });
  modelSelects.titleModel = createSelect(document.getElementById('title-model-host'), {
    ariaLabel: 'Title model',
    placeholder: '(none)',
    value: '',
    options: [{value: '', label: '(none)'}],
    onChange: (value) => {
      if (!modelsDraft) { return; }
      if (!value) {
        modelsDraft.titleSummary = null;
        markModelsDirty();
        return;
      }
      const parts = value.split('\u0000');
      modelsDraft.titleSummary = {provider: parts[0], model: parts[1], variant: null};
      markModelsDirty();
    }
  });
  modelSelects.titleLanguage = createSelect(document.getElementById('title-language-host'), {
    ariaLabel: 'Title language',
    value: 'auto',
    options: TITLE_LANGUAGE_OPTIONS,
    onChange: (value) => {
      if (!modelsDraft) { return; }
      modelsDraft.titleLanguage = value || 'auto';
      markModelsDirty();
    }
  });
  initProviderFields();
}

// 顶部操作条复用 settings.js 的 createActionBar（脏状态显示 "Unsaved changes"）。
actionBars.model = createActionBar('actions-model', {
  isDirty: isModelSettingsDirty,
  save: saveModels,
  reset: resetModels
});
actionBars.provider = createActionBar('actions-provider', {
  isDirty: isProviderSettingsDirty,
  save: saveModels,
  reset: resetModels
});
initModelsView();

