// ===== Models view =====
// 全局内存草稿：所有编辑（含删除/复制/启停）先落在 modelsDraft，
// Save all 一次 PUT 提交全部；Reload 重新拉取并丢弃草稿。
// 记录键与模型 id 一致（与 lx-agent 语义相同）；模态与思考等级只在展开区编辑。

const INPUT_MODALITIES = ['text', 'image', 'audio', 'video', 'pdf'];
const OUTPUT_MODALITIES = ['text', 'image', 'audio', 'video'];
const VARIANT_PRESETS = {
  openai: {low: {reasoningEffort: 'low'}, medium: {reasoningEffort: 'medium'}, high: {reasoningEffort: 'high'}},
  openrouter: {low: {reasoning: {effort: 'low'}}, medium: {reasoning: {effort: 'medium'}}, high: {reasoning: {effort: 'high'}}},
  anthropic: {low: {thinkingBudget: 4096}, high: {thinkingBudget: 16000}, max: {thinkingBudget: 32000}},
  reasoning_effort: {low: {reasoning_effort: 'low'}, medium: {reasoning_effort: 'medium'}, high: {reasoning_effort: 'high'}}
};

let modelsDraft = null;      // 全局编辑草稿
let selectedProviderKey = null;
let modelsDirty = false;
const modelSelects = {};
const expandedModels = new Set();

function suffix() {
  return Date.now().toString(36).slice(-6);
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) { node.className = className; }
  if (text !== undefined && text !== null) { node.textContent = text; }
  return node;
}

function currentProvider() {
  if (!modelsDraft) { return null; }
  return modelsDraft.providers.find((provider) => provider.key === selectedProviderKey) || null;
}

function markModelsDirty() {
  modelsDirty = true;
  updateModelsStatus();
}

function updateModelsStatus() {
  const count = ((modelsDraft && modelsDraft.providers) || []).length;
  document.getElementById('models-status').textContent = count + ' provider(s)' + (modelsDirty ? ' · unsaved changes' : '');
  document.getElementById('models-status').className = 'status ok';
}

function loadModels() {
  api('/api/models/settings').then((data) => {
    modelsDraft = snapshotToDraft(data);
    if (!modelsDraft.providers.some((provider) => provider.key === selectedProviderKey)) {
      selectedProviderKey = modelsDraft.providers.length ? modelsDraft.providers[0].key : null;
    }
    modelsDirty = false;
    renderModelsView();
    document.getElementById('models-revision').textContent = 'revision ' + String(data.revision || '').slice(0, 8);
  }).catch((error) => {
    document.getElementById('models-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('models-status').className = 'status error';
  });
}

// 409 后只刷新 revision，保留用户草稿（再次点击 Save all 即覆盖）。
function refreshRevision() {
  api('/api/models/settings').then((data) => {
    if (modelsDraft) { modelsDraft.revision = data.revision || ''; }
    document.getElementById('models-revision').textContent = 'revision ' + String(data.revision || '').slice(0, 8);
  }).catch((error) => {
    showToast(error.message, 'error');
  });
}

function snapshotToDraft(snapshot) {
  return {
    revision: snapshot.revision || '',
    streamIdleTimeoutMs: snapshot.streamIdleTimeoutMs,
    defaultModel: snapshot.defaultModel || null,
    providers: (snapshot.providers || []).map((provider) => ({
      key: provider.key,
      id: provider.id,
      type: provider.type,
      name: provider.name,
      baseUrl: provider.baseUrl,
      enabled: !!provider.enabled,
      hasApiKey: !!provider.hasApiKey,
      apiKeyText: '',
      clearApiKey: false,
      models: (provider.models || []).map((model) => ({
        id: model.id,
        name: model.name,
        transport: model.transport || null,
        context: model.limit ? String(model.limit.context) : '',
        output: model.limit ? String(model.limit.output) : '',
        variant: model.variant || null,
        variantRows: Object.entries(model.variants || {}).map(([key, value]) => ({key: key, text: JSON.stringify(value)})),
        modalities: normalizedModalities(model.modalities)
      }))
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
  const providers = (modelsDraft && modelsDraft.providers) || [];
  if (providers.length === 0) {
    list.appendChild(element('p', 'nav-empty', 'No providers configured.'));
  }
  for (const provider of providers) {
    const row = element('button', 'nav-item' + (provider.key === selectedProviderKey ? ' selected' : ''));
    row.type = 'button';
    const toggle = element('span', 'prov-toggle' + (provider.enabled ? ' on' : ''));
    toggle.setAttribute('aria-label', provider.enabled ? 'Enabled' : 'Off');
    toggle.onclick = (event) => {
      event.stopPropagation();
      provider.enabled = !provider.enabled;
      if (!provider.enabled && modelsDraft.defaultModel && modelsDraft.defaultModel.provider === provider.id) {
        modelsDraft.defaultModel = null;
        showToast('Default model cleared (its provider was turned off)', 'info');
      }
      markModelsDirty();
      renderProviders();
      renderDefaultModel();
    };
    const name = element('span', 'nav-name', provider.name || provider.id);
    const tag = element('span', 'tag', provider.type + (provider.hasApiKey ? '' : ' · no key'));
    row.appendChild(toggle);
    row.appendChild(name);
    row.appendChild(tag);
    row.onclick = () => selectProvider(provider.key);
    list.appendChild(row);
  }
}

function renderEditor() {
  const provider = currentProvider();
  document.getElementById('provider-empty').classList.toggle('hidden', !!provider);
  document.getElementById('provider-editor').classList.toggle('hidden', !provider);
  if (!provider) { return; }
  document.getElementById('provider-legend').textContent = 'Edit ' + (provider.name || provider.id);
  document.getElementById('prov-id').value = provider.id;
  document.getElementById('prov-name').value = provider.name;
  document.getElementById('prov-base-url').value = provider.baseUrl;
  document.getElementById('prov-api-key').value = provider.apiKeyText;
  document.getElementById('prov-clear-key').checked = provider.clearApiKey;
  document.getElementById('prov-key-state').textContent = provider.hasApiKey ? 'a key is stored' : 'no key stored';
  modelSelects.provType.setValue(provider.type);
  renderModelList();
}

function selectProvider(key) {
  selectedProviderKey = key;
  renderProviders();
  renderEditor();
}

function newProvider() {
  if (!modelsDraft) { return; }
  const id = 'provider-' + suffix();
  modelsDraft.providers.push({
    key: id, id: id, type: 'openai-compatible', name: 'New Provider', baseUrl: '',
    enabled: true, hasApiKey: false, apiKeyText: '', clearApiKey: false, models: []
  });
  selectedProviderKey = id;
  markModelsDirty();
  renderModelsView();
}

function duplicateProvider() {
  const provider = currentProvider();
  if (!provider || !modelsDraft) { return; }
  const id = provider.id + '-copy-' + suffix();
  const copy = JSON.parse(JSON.stringify(provider));
  copy.key = id;
  copy.id = id;
  copy.name = (provider.name || provider.id) + ' copy';
  copy.hasApiKey = false; // 复制项默认需重新输入 Key
  copy.apiKeyText = '';
  copy.clearApiKey = false;
  modelsDraft.providers.push(copy);
  selectedProviderKey = id;
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
  selectedProviderKey = modelsDraft.providers.length ? modelsDraft.providers[0].key : null;
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
  if (modelsDraft.defaultModel && modelsDraft.defaultModel.provider === provider.key && modelsDraft.defaultModel.model === model.id) {
    modelsDraft.defaultModel = null;
    showToast('Default model cleared', 'info');
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
  nameField.input.addEventListener('change', () => { renderProviders(); });
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
  const options = [{value: '', label: '(none)'}];
  for (const provider of modelsDraft.providers) {
    for (const model of provider.models) {
      options.push({
        value: provider.id + '\u0000' + model.id,
        label: (provider.name || provider.id) + ' / ' + (model.name || model.id) + (provider.enabled ? '' : ' (off)')
      });
    }
  }
  let value = modelsDraft.defaultModel ? modelsDraft.defaultModel.provider + '\u0000' + modelsDraft.defaultModel.model : '';
  if (value && !options.some((option) => option.value === value)) {
    modelsDraft.defaultModel = null;
    value = '';
    showToast('Default model cleared (its model was renamed or removed)', 'info');
  }
  modelSelects.defaultModel.setOptions(options, value);
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

function buildPayload() {
  const seenProviders = new Set();
  const providers = [];
  for (const provider of modelsDraft.providers) {
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
    const entry = {key: id, id: id, type: provider.type, name: (provider.name || '').trim() || id, baseUrl: baseUrl, models: models};
    if (provider.clearApiKey) {
      entry.clearApiKey = true;
    } else if ((provider.apiKeyText || '') !== '') {
      entry.apiKey = provider.apiKeyText;
    }
    providers.push(entry);
  }
  const enabledProviders = modelsDraft.providers.filter((provider) => provider.enabled).map((provider) => provider.id.trim());
  let defaultModel = null;
  if (modelsDraft.defaultModel) {
    const provider = modelsDraft.providers.find((item) => item.id === modelsDraft.defaultModel.provider);
    const model = provider && provider.models.find((item) => item.id.trim() === modelsDraft.defaultModel.model);
    if (provider && model) {
      defaultModel = {provider: provider.id, model: model.id.trim(), variant: model.variant || null};
    }
  }
  return {
    providers: providers,
    enabledProviders: enabledProviders,
    defaultModel: defaultModel,
    streamIdleTimeoutMs: modelsDraft.streamIdleTimeoutMs,
    revision: modelsDraft.revision
  };
}

function saveModels() {
  if (!modelsDraft) { return; }
  let payload;
  try {
    payload = buildPayload();
  } catch (error) {
    showToast(error.message, 'error');
    return;
  }
  api('/api/models/settings', {method: 'PUT', body: JSON.stringify(payload)}).then(() => {
    showToast('Saved', 'ok');
    modelsDirty = false;
    loadModels();
  }).catch((error) => {
    // 409：磁盘配置已被别处改动。只刷新 revision，保留草稿，再点一次即覆盖。
    if (error.status === 409) {
      showToast('Configuration changed elsewhere; revision refreshed — click Save all again', 'error');
      refreshRevision();
      return;
    }
    showToast(error.message, 'error');
  });
}

function fetchModels() {
  const provider = currentProvider();
  if (!provider) { return; }
  const body = {
    type: provider.type,
    baseUrl: provider.baseUrl.trim(),
    apiKey: provider.apiKeyText || undefined
  };
  if (provider.hasApiKey) { body.providerId = provider.key; }
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
    if (provider) { provider.apiKeyText = event.target.value; }
  });
  document.getElementById('prov-clear-key').addEventListener('change', (event) => {
    const provider = currentProvider();
    if (provider) { provider.clearApiKey = event.target.checked; }
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
  initProviderFields();
}

initModelsView();

