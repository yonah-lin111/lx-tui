// ===== Models view =====
// 全局内存草稿：所有编辑（含删除/复制/启停）先落在 modelsDraft，
// Save（视图操作条）一次 PUT 提交全部；Reload / Reset 丢弃草稿回到最近快照。
// 记录键与模型 id 一致（与 lx-agent 语义相同）；
// 模型卡片、单价（Pricing）、模态（INPUT_MODALITIES / OUTPUT_MODALITIES）与
// 思考等级（Thinking variants / VARIANT_PRESETS / reasoningEffort）组件已拆分至 settings_models_card.js。

const TITLE_LANGUAGE_OPTIONS = [
  {value: 'auto', label: 'Auto (follow input)'},
  {value: 'zh-CN', label: 'Simplified Chinese'},
  {value: 'en', label: 'English'},
  {value: 'zh-TW', label: 'Traditional Chinese'},
  {value: 'ja', label: 'Japanese'},
  {value: 'ko', label: 'Korean'}
];

let modelsDraft = null;      // 全局编辑草稿
let modelsBaseline = null;   // 最近一次加载/保存的快照（Reset 回填基准）
let selectedProviderKey = null;
let activeProviderTab = null; // 'quick' | 'custom'，由 settings_connect.js 初始化
let selectedPresetId = null;
const modelSelects = {};

function suffix() {
  return Date.now().toString(36).slice(-6);
}

function currentProvider() {
  if (!modelsDraft) { return null; }
  if (activeProviderTab === 'quick') { return currentPreset(); }
  return modelsDraft.providers.find((provider) => provider.key === selectedProviderKey) || null;
}

function modelsSnapshot() {
  return modelsDraft ? JSON.stringify(modelsDraft) : '';
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
    recapEnabled: modelsDraft.recapEnabled,
    recapSummary: modelsDraft.recapSummary,
    titleLanguage: modelsDraft.titleLanguage
  }) !== JSON.stringify({
    defaultModel: modelsBaseline.defaultModel,
    titleSummary: modelsBaseline.titleSummary,
    recapEnabled: modelsBaseline.recapEnabled,
    recapSummary: modelsBaseline.recapSummary,
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

// computeModelsDirty 已移除：模型页脏状态由 isModelSettingsDirty / isProviderSettingsDirty 分别驱动。
function updateProviderDirtyDots() {
  const list = document.getElementById('provider-list');
  if (!list || !modelsDraft) { return; }
  for (const row of list.querySelectorAll('.nav-item')) {
    const key = row.dataset.key || '';
    let dirty = false;
    if (activeProviderTab === 'quick') {
      const preset = (modelsDraft.quickConnect || []).find((item) => item.id === key);
      dirty = !!preset && isPresetDirty(preset);
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

function snapshotToDraft(snapshot) {
  return {
    revision: snapshot.revision || '',
    streamIdleTimeoutMs: snapshot.streamIdleTimeoutMs,
    defaultModel: snapshot.defaultModel || null,
    titleSummary: snapshot.titleSummary || null,
    recapEnabled: snapshot.recapEnabled !== false,
    recapSummary: snapshot.recapSummary || null,
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
    quickConnect: (snapshot.quickConnect || []).map(draftPresetSnapshot)
  };
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
    list.appendChild(domEl('p', 'nav-empty', query ? 'No matching providers.' : 'No providers configured.'));
  }
  for (const provider of providers) {
    const dirty = isProviderDirty(provider);
    const row = domEl('button', 'nav-item' + (provider.key === selectedProviderKey ? ' selected' : '') + (dirty ? ' dirty' : ''));
    row.type = 'button';
    row.dataset.key = provider.key;
    const name = domEl('span', 'nav-name', provider.name || provider.id);
    const tag = domEl('span', 'tag', provider.type + (provider.apiKey ? '' : ' · no key') + (provider.enabled ? '' : ' · off'));
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
  if (modelsDraft.recapSummary && modelsDraft.recapSummary.provider === provider.id) {
    modelsDraft.recapSummary = null;
    showToast('Recap model cleared', 'info');
  }
  selectedProviderKey = null;
  markModelsDirty();
  renderModelsView();
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
  let recapValue = modelsDraft.recapSummary ? modelsDraft.recapSummary.provider + '\u0000' + modelsDraft.recapSummary.model : '';
  if (recapValue && !options.some((option) => option.value === recapValue)) {
    modelsDraft.recapSummary = null;
    recapValue = '';
    showToast('Recap model cleared (its model was renamed or removed)', 'info');
  }
  if (modelSelects.recapModel) {
    modelSelects.recapModel.setOptions(options, recapValue);
  }
  const recapToggle = document.getElementById('recap-enabled');
  if (recapToggle) {
    recapToggle.checked = modelsDraft.recapEnabled !== false;
  }
  if (modelSelects.titleLanguage) {
    const lang = modelsDraft.titleLanguage || 'auto';
    modelSelects.titleLanguage.setOptions(TITLE_LANGUAGE_OPTIONS, lang);
  }
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
          pricing: serializePricing(model.pricing, modelId),
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
      if (activeProviderTab !== 'custom') {
        switchProviderTab('custom');
      } else {
        renderModelsView();
      }
      throw error;
    }
  }
  const enabledProviders = modelsDraft.providers.filter((provider) => provider.enabled).map((provider) => provider.id.trim());
  // 选择可以指向 Custom 供应商，也可以指向已连接且启用的快捷连接供应商。
  const defaultModel = resolveDraftSelection(modelsDraft.defaultModel);
  const titleSummary = resolveDraftSelection(modelsDraft.titleSummary);
  const recapSummary = resolveDraftSelection(modelsDraft.recapSummary);
  return {
    providers: providers,
    enabledProviders: enabledProviders,
    defaultModel: defaultModel,
    titleSummary: titleSummary,
    recapEnabled: modelsDraft.recapEnabled !== false,
    recapSummary: recapSummary,
    titleLanguage: modelsDraft.titleLanguage || 'auto',
    streamIdleTimeoutMs: modelsDraft.streamIdleTimeoutMs,
    revision: modelsDraft.revision
  };
}

function modelsPayload() {
  return buildPayload();
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
    apiKey: provider.apiKey || undefined,
    formAnswers: provider.formAnswers || {}
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
  modelSelects.recapModel = createSelect(document.getElementById('recap-model-host'), {
    ariaLabel: 'Recap model',
    placeholder: '(none)',
    value: '',
    options: [{value: '', label: '(none)'}],
    onChange: (value) => {
      if (!modelsDraft) { return; }
      if (!value) {
        modelsDraft.recapSummary = null;
        markModelsDirty();
        return;
      }
      const parts = value.split('\u0000');
      modelsDraft.recapSummary = {provider: parts[0], model: parts[1], variant: null};
      markModelsDirty();
    }
  });
  const recapToggle = document.getElementById('recap-enabled');
  if (recapToggle) {
    recapToggle.onchange = () => {
      if (!modelsDraft) { return; }
      modelsDraft.recapEnabled = recapToggle.checked;
      markModelsDirty();
    };
  }
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
