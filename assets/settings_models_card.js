// ===== Model Card & Pricing / Modalities / Thinking Variants =====
// 从 settings_models.js 拆出的模型卡片渲染、单价（Pricing）、模态与思考等级编辑组件。
// 在 settings.html 中先于 settings_models.js 加载，与 settings_models.js / settings_connect.js 共享全局作用域。

const INPUT_MODALITIES = ['text', 'image', 'audio', 'video', 'pdf'];
const OUTPUT_MODALITIES = ['text', 'image', 'audio', 'video'];
const VARIANT_PRESETS = {
  openai: {low: {reasoningEffort: 'low'}, medium: {reasoningEffort: 'medium'}, high: {reasoningEffort: 'high'}},
  openrouter: {low: {reasoning: {effort: 'low'}}, medium: {reasoning: {effort: 'medium'}}, high: {reasoning: {effort: 'high'}}},
  anthropic: {low: {thinkingBudget: 4096}, high: {thinkingBudget: 16000}, max: {thinkingBudget: 32000}},
  reasoning_effort: {low: {reasoning_effort: 'low'}, medium: {reasoning_effort: 'medium'}, high: {reasoning_effort: 'high'}}
};

const expandedModels = new Set();

function normalizePricingField(value) {
  if (value === null || value === undefined || value === '') { return ''; }
  const text = String(value).trim();
  if (!text) { return ''; }
  const num = Number(text);
  return Number.isFinite(num) && num >= 0 ? text : '';
}

function normalizePricing(raw) {
  const src = raw && typeof raw === 'object' ? raw : {};
  const cacheRead = src.cacheRead !== undefined ? src.cacheRead : src.cache_read;
  const cacheWrite = src.cacheWrite !== undefined ? src.cacheWrite : src.cache_write;
  return {
    input: normalizePricingField(src.input),
    output: normalizePricingField(src.output),
    cacheRead: normalizePricingField(cacheRead),
    cacheWrite: normalizePricingField(cacheWrite)
  };
}

// 序列化模型单价：4 个字段全空时返回 null（通过三态语义显式清除后端旧单价），
// 任一字段非空时校验为非负有限数并组装 { input?, output?, cacheRead?, cacheWrite? }。
function serializePricing(pricing, modelId) {
  const src = pricing && typeof pricing === 'object' ? pricing : {};
  const fields = [
    ['input', 'Input price'],
    ['output', 'Output price'],
    ['cacheRead', 'Cache read price'],
    ['cacheWrite', 'Cache write price']
  ];
  const out = {};
  let hasAny = false;
  for (const [key, label] of fields) {
    const raw = String(src[key] === undefined || src[key] === null ? '' : src[key]).trim();
    if (!raw) { continue; }
    const num = Number(raw);
    if (!Number.isFinite(num) || num < 0) {
      throw new Error(label + ' must be a non-negative number for model ' + (modelId || 'model'));
    }
    out[key] = num;
    hasAny = true;
  }
  return hasAny ? out : null;
}

function normalizedModalities(value) {
  const input = value && Array.isArray(value.input) ? value.input.filter((item) => INPUT_MODALITIES.includes(item)) : ['text'];
  const output = value && Array.isArray(value.output) ? value.output.filter((item) => OUTPUT_MODALITIES.includes(item)) : ['text'];
  return {input: input, output: output};
}

function draftModels(models) {
  return (models || []).map((model) => ({
    id: model.id,
    name: model.name,
    transport: model.transport || null,
    context: model.limit ? String(model.limit.context) : '',
    output: model.limit ? String(model.limit.output) : '',
    pricing: normalizePricing(model.pricing),
    variant: model.variant || null,
    variantRows: Object.entries(model.variants || {}).map(([key, value]) => ({key: key, text: JSON.stringify(value)})),
    modalities: normalizedModalities(model.modalities)
  }));
}

function normalizeModels(raw) {
  return draftModels(raw);
}

function addModel() {
  const provider = currentProvider();
  if (!provider) { return; }
  const id = 'model-' + suffix();
  provider.models.push({
    id: id,
    name: 'New Model',
    transport: null,
    context: '',
    output: '',
    pricing: normalizePricing(null),
    variant: null,
    variantRows: [],
    modalities: {input: ['text'], output: ['text']}
  });
  markModelsDirty();
  renderModelList();
  renderDefaultModel();
}

function duplicateModel(provider, model) {
  const copy = JSON.parse(JSON.stringify(model));
  copy.id = (model.id || 'model') + '-copy-' + suffix();
  copy.name = (model.name || model.id) + ' copy';
  copy.pricing = normalizePricing(copy.pricing);
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
  if (modelsDraft.recapSummary && modelsDraft.recapSummary.provider === provider.id && modelsDraft.recapSummary.model === model.id) {
    modelsDraft.recapSummary = null;
    showToast('Recap model cleared', 'info');
  }
  markModelsDirty();
  renderModelList();
  renderDefaultModel();
}

function labeledInput(captionText, value, onInput, placeholder) {
  const wrap = element('div');
  wrap.appendChild(element('label', null, captionText));
  const input = document.createElement('input');
  input.type = 'text';
  input.value = value === null || value === undefined ? '' : value;
  if (placeholder) { input.placeholder = placeholder; }
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
  if (!model.pricing || typeof model.pricing !== 'object') {
    model.pricing = normalizePricing(null);
  }
  let key = provider.key + '\u0000' + model.id;
  const card = element('div', 'model-card');
  const head = element('div', 'model-head');
  const idField = labeledInput('Model ID', model.id, (value) => {
    const nextKey = provider.key + '\u0000' + value;
    if (expandedModels.has(key)) {
      expandedModels.delete(key);
      expandedModels.add(nextKey);
    }
    key = nextKey;
    model.id = value;
    markModelsDirty();
  });
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
  more.appendChild(pricingSection(model));
  more.appendChild(modalitySection(model));
  more.appendChild(thinkingSection(model));
  card.appendChild(more);
  return card;
}

function pricingSection(model) {
  if (!model.pricing || typeof model.pricing !== 'object') {
    model.pricing = normalizePricing(null);
  }
  const section = element('div', 'model-sub');
  section.appendChild(element('div', 'sub-title', 'Pricing (USD / 1M tokens)'));
  const grid = element('div', 'field-grid model-pricing-grid');
  grid.style.gridTemplateColumns = 'repeat(auto-fit, minmax(120px, 1fr))';
  grid.style.gap = '8px';
  const inputField = labeledInput('Input ($/1M)', model.pricing.input, (value) => {
    model.pricing.input = value;
    markModelsDirty();
  }, 'e.g. 3.00');
  const outputField = labeledInput('Output ($/1M)', model.pricing.output, (value) => {
    model.pricing.output = value;
    markModelsDirty();
  }, 'e.g. 15.00');
  const cacheReadField = labeledInput('Cache read ($/1M)', model.pricing.cacheRead, (value) => {
    model.pricing.cacheRead = value;
    markModelsDirty();
  }, 'e.g. 0.30');
  const cacheWriteField = labeledInput('Cache write ($/1M)', model.pricing.cacheWrite, (value) => {
    model.pricing.cacheWrite = value;
    markModelsDirty();
  }, 'e.g. 3.75');
  grid.appendChild(inputField.wrap);
  grid.appendChild(outputField.wrap);
  grid.appendChild(cacheReadField.wrap);
  grid.appendChild(cacheWriteField.wrap);
  section.appendChild(grid);
  return section;
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

function defaultVariantOptions(model) {
  return [{value: '', label: '(none)'}].concat(
    (model.variantRows || []).map((row) => ({value: row.key, label: row.key}))
  );
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
    options: defaultVariantOptions(model),
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
      if (!provider.models.some((model) => model.id === candidate.id)) {
        provider.models.push({
          id: candidate.id,
          name: candidate.id,
          transport: null,
          context: '',
          output: '',
          pricing: normalizePricing(candidate.pricing || null),
          variant: null,
          variantRows: [],
          modalities: {input: ['text'], output: ['text']}
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
