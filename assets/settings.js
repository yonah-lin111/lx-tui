
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
      const failure = new Error(message);
      failure.status = response.status;
      throw failure;
    }
    return data;
  });
}

function showTab(name) {
  const tabs = {general: 'tab-general', commands: 'tab-commands', models: 'tab-models'};
  const views = {general: 'view-general', commands: 'view-commands', models: 'view-models'};
  for (const key of Object.keys(tabs)) {
    const active = key === name;
    document.getElementById(tabs[key]).classList.toggle('active', active);
    document.getElementById(views[key]).classList.toggle('hidden', !active);
  }
  if (name === 'models' && modelsState === null) { loadModels(); }
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
  document.getElementById('cmd-block-type').value = command.block_type || 'template';
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
  const blockType = document.getElementById('cmd-block-type').value;
  const payload = {
    description: document.getElementById('cmd-description').value,
    md_scope: mdScope,
    block_type: blockType,
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
    selected = {name, scope: 'user', kind, md_scope: mdScope, block_type: blockType, title: payload.title, description: payload.description, body};
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

const SIDE_MIN_WIDTH = 140;
const SIDE_MAX_WIDTH = 360;
const LIST_MIN_WIDTH = 140;
const LIST_MAX_WIDTH = 560;
let cmdListWidth = 220;

function clampWidth(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function setupResizer(handle, read, write, min, max) {
  handle.addEventListener('pointerdown', (event) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = read();
    handle.setPointerCapture(event.pointerId);
    handle.classList.add('dragging');
    document.body.classList.add('resizing');
    const onMove = (moveEvent) => write(clampWidth(startWidth + moveEvent.clientX - startX, min, max));
    const stop = () => {
      handle.classList.remove('dragging');
      document.body.classList.remove('resizing');
      handle.removeEventListener('pointermove', onMove);
    };
    handle.addEventListener('pointermove', onMove);
    handle.addEventListener('pointerup', stop, {once: true});
    handle.addEventListener('pointercancel', stop, {once: true});
  });
}

setupResizer(
  document.getElementById('side-resizer'),
  () => document.querySelector('nav.side').getBoundingClientRect().width,
  (width) => { document.querySelector('nav.side').style.width = width + 'px'; },
  SIDE_MIN_WIDTH,
  SIDE_MAX_WIDTH
);
setupResizer(
  document.getElementById('cmd-resizer'),
  () => cmdListWidth,
  (width) => {
    cmdListWidth = width;
    document.querySelector('.commands-grid').style.setProperty('--cmd-list-width', width + 'px');
  },
  LIST_MIN_WIDTH,
  LIST_MAX_WIDTH
);


// ===== Models view =====
// 供应商与模型配置：GET/PUT /api/models/settings、POST /api/models/fetch。
// 动态数据一律用 textContent 渲染；API key 输入框默认遮蔽，后端不回传明文。

let modelsState = null;   // 服务端快照（providers/enabledProviders/defaultModel/revision）
let providerDraft = null; // 当前编辑中的 provider（含 models 数组）
let providerDraftId = null; // 编辑中的 provider key（null 表示新增）
let keyTouched = false;   // 本次是否输入/清除过 key

function loadModels() {
  api('/api/models/settings').then((data) => {
    modelsState = data;
    // 保留编辑草稿的选中项；被删除则清空。
    if (providerDraftId !== null && !(data.providers || []).some((p) => p.key === providerDraftId)) {
      hideProviderEditor();
    }
    renderProviders();
    renderDefaultModel();
    document.getElementById('models-status').textContent = (data.providers || []).length + ' provider(s)';
    document.getElementById('models-status').className = 'status ok';
    document.getElementById('models-revision').textContent = 'revision ' + (data.revision || '').slice(0, 8);
  }).catch((error) => {
    document.getElementById('models-status').textContent = 'Failed to load: ' + error.message;
    document.getElementById('models-status').className = 'status error';
  });
}

function renderProviders() {
  const list = document.getElementById('provider-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  const providers = (modelsState && modelsState.providers) || [];
  if (providers.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'nav-empty';
    empty.textContent = 'No providers configured.';
    list.appendChild(empty);
  }
  for (const provider of providers) {
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'nav-item' + (provider.key === providerDraftId ? ' selected' : '');
    const name = document.createElement('span');
    name.className = 'nav-name';
    name.textContent = provider.name || provider.id;
    const tag = document.createElement('span');
    tag.className = 'tag';
    tag.textContent = provider.type + (provider.enabled ? '' : ' · disabled') + (provider.hasApiKey ? '' : ' · no key');
    row.appendChild(name);
    row.appendChild(tag);
    row.onclick = () => selectProvider(provider.key);
    list.appendChild(row);
  }
}

function selectProvider(key) {
  const provider = (modelsState.providers || []).find((p) => p.key === key);
  if (!provider) { return; }
  providerDraftId = key;
  providerDraft = {
    key: provider.key,
    id: provider.id,
    type: provider.type,
    name: provider.name,
    baseUrl: provider.baseUrl,
    enabled: provider.enabled,
    hasApiKey: provider.hasApiKey,
    models: provider.models.map((model) => ({
      key: model.key, id: model.id, name: model.name, transport: model.transport,
      context: model.limit ? model.limit.context : '', output: model.limit ? model.limit.output : '',
      variant: model.variant || '', variantsJson: JSON.stringify(model.variants || {})
    }))
  };
  keyTouched = false;
  showProviderEditor();
  renderProviders();
}

function newProvider() {
  providerDraftId = null;
  const id = 'provider-' + Date.now().toString(36).slice(-6);
  providerDraft = {key: id, id: id, type: 'openai-compatible', name: 'New Provider',
    baseUrl: '', enabled: true, hasApiKey: false, models: []};
  keyTouched = false;
  showProviderEditor();
  hideFetchCandidates();
}

function duplicateProvider() {
  if (!providerDraft) { return; }
  const copy = JSON.parse(JSON.stringify(providerDraft));
  copy.key = providerDraft.id + '-copy-' + Date.now().toString(36).slice(-4);
  copy.id = copy.key;
  copy.name = providerDraft.name + ' copy';
  copy.hasApiKey = false; // 复制项默认需重新输入 Key
  providerDraftId = null;
  providerDraft = copy;
  keyTouched = false;
  showProviderEditor();
  renderProviders();
}

function deleteProvider() {
  if (providerDraftId === null) { return; }
  const key = providerDraftId;
  // 删除默认供应商前要求先选择有效替代。
  const isDefault = modelsState.defaultModel && modelsState.defaultModel.provider === key;
  if (isDefault) {
    document.getElementById('provider-status').textContent = 'Choose another default model before deleting this provider.';
    document.getElementById('provider-status').className = 'status error';
    return;
  }
  modelsState.providers = modelsState.providers.filter((p) => p.key !== key);
  modelsState.enabledProviders = modelsState.enabledProviders.filter((id) => id !== key);
  hideProviderEditor();
  renderProviders();
  renderDefaultModel();
}

function showProviderEditor() {
  document.getElementById('provider-empty').classList.add('hidden');
  document.getElementById('provider-editor').classList.remove('hidden');
  document.getElementById('provider-legend').textContent = providerDraftId ? ('Edit ' + providerDraft.name) : 'New provider';
  document.getElementById('prov-id').value = providerDraft.id;
  document.getElementById('prov-name').value = providerDraft.name;
  document.getElementById('prov-type').value = providerDraft.type;
  document.getElementById('prov-enabled').value = providerDraft.enabled ? 'true' : 'false';
  document.getElementById('prov-base-url').value = providerDraft.baseUrl;
  const keyInput = document.getElementById('prov-api-key');
  keyInput.value = '';
  document.getElementById('prov-clear-key').checked = false;
  document.getElementById('prov-key-state').textContent = providerDraft.hasApiKey ? 'a key is stored' : 'no key stored';
  renderModelList();
}

function hideProviderEditor() {
  providerDraft = null;
  providerDraftId = null;
  document.getElementById('provider-editor').classList.add('hidden');
  document.getElementById('provider-empty').classList.remove('hidden');
}

function renderModelList() {
  const list = document.getElementById('model-list');
  while (list.firstChild) { list.removeChild(list.firstChild); }
  if (!providerDraft || providerDraft.models.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'nav-empty';
    empty.textContent = 'No models yet.';
    list.appendChild(empty);
    return;
  }
  providerDraft.models.forEach((model, index) => {
    list.appendChild(modelRow(model, index));
  });
}

function modelRow(model, index) {
  const row = document.createElement('div');
  row.className = 'model-row';
  const fields = [
    ['id', model.id, 'Model ID'],
    ['name', model.name, 'Display name'],
    ['context', model.context, 'Context'],
    ['output', model.output, 'Output'],
    ['variant', model.variant, 'Default variant'],
    ['variantsJson', model.variantsJson === undefined ? '{}' : model.variantsJson, 'Variants (JSON)']
  ];
  for (const [field, value, label] of fields) {
    const wrap = document.createElement('div');
    const caption = document.createElement('label');
    caption.textContent = label;
    const input = document.createElement('input');
    input.type = 'text';
    input.value = value === null || value === undefined ? '' : value;
    input.oninput = () => { model[field] = field === 'context' || field === 'output' ? input.value.trim() : input.value; };
    wrap.appendChild(caption);
    wrap.appendChild(input);
    row.appendChild(wrap);
  }
  const transportWrap = document.createElement('div');
  const transportLabel = document.createElement('label');
  transportLabel.textContent = 'Transport';
  const transportSelect = document.createElement('select');
  for (const option of ['', 'openai', 'openai-compatible', 'openai-responses', 'anthropic', 'google']) {
    const opt = document.createElement('option');
    opt.value = option;
    opt.textContent = option === '' ? '(inherit)' : option;
    if ((model.transport || '') === option) { opt.selected = true; }
    transportSelect.appendChild(opt);
  }
  transportSelect.onchange = () => { model.transport = transportSelect.value || null; };
  transportWrap.appendChild(transportLabel);
  transportWrap.appendChild(transportSelect);
  row.appendChild(transportWrap);

  const actions = document.createElement('div');
  actions.className = 'model-actions';
  const dup = document.createElement('button');
  dup.type = 'button';
  dup.className = 'ghost';
  dup.textContent = 'Duplicate';
  dup.onclick = () => {
    const copy = JSON.parse(JSON.stringify(model));
    copy.key = model.id + '-copy-' + Date.now().toString(36).slice(-4);
    copy.id = copy.key;
    copy.name = model.name + ' copy';
    providerDraft.models.splice(index + 1, 0, copy);
    renderModelList();
  };
  const del = document.createElement('button');
  del.type = 'button';
  del.className = 'danger';
  del.textContent = 'Remove';
  del.onclick = () => { providerDraft.models.splice(index, 1); renderModelList(); renderDefaultModel(); };
  actions.appendChild(dup);
  actions.appendChild(del);
  row.appendChild(actions);
  return row;
}

function addModel() {
  if (!providerDraft) { return; }
  const id = 'model-' + Date.now().toString(36).slice(-6);
  providerDraft.models.push({key: id, id: id, name: 'New Model', transport: null, context: '', output: '', variant: '', variantsJson: '{}'});
  renderModelList();
}

function renderDefaultModel() {
  const select = document.getElementById('default-model');
  while (select.firstChild) { select.removeChild(select.firstChild); }
  const none = document.createElement('option');
  none.value = '';
  none.textContent = '(none)';
  select.appendChild(none);
  const providers = (modelsState && modelsState.providers) || [];
  const enabled = (modelsState && modelsState.enabledProviders) || [];
  for (const provider of providers) {
    for (const model of provider.models || []) {
      const option = document.createElement('option');
      option.value = provider.key + '\u0000' + model.key;
      option.textContent = provider.name + ' / ' + model.name + (enabled.includes(provider.key) ? '' : ' (disabled)');
      const current = modelsState.defaultModel;
      if (current && current.provider === provider.key && current.model === model.key) { option.selected = true; }
      select.appendChild(option);
    }
  }
}

function markDefaultDirty() {
  document.getElementById('provider-status').textContent = '';
}

function collectPayload() {
  // 用编辑中的 provider 草稿替换服务端快照中的对应项。
  const providers = (modelsState.providers || []).map((p) => ({...p, models: p.models.map((m) => ({...m}))}));
  const draft = buildProviderPayload();
  const index = providers.findIndex((p) => p.key === (providerDraftId !== null ? providerDraftId : draft.key));
  if (index >= 0) { providers[index] = draft; } else { providers.push(draft); }
  const defaultValue = document.getElementById('default-model').value;
  let defaultModel = null;
  if (defaultValue) {
    const [provider, model] = defaultValue.split('\u0000');
    const target = providers.find((p) => p.key === provider);
    const modelDef = target && target.models.find((m) => m.key === model);
    defaultModel = {provider: provider, model: model, variant: modelDef && modelDef.variant ? modelDef.variant : null};
  }
  const enabledProviders = document.getElementById('prov-enabled').value === 'true'
    ? Array.from(new Set([...(modelsState.enabledProviders || []), providerDraft.key]))
    : (modelsState.enabledProviders || []).filter((id) => id !== providerDraft.key);
  return {
    providers: providers,
    enabledProviders: enabledProviders,
    defaultModel: defaultModel,
    streamIdleTimeoutMs: modelsState.streamIdleTimeoutMs,
    revision: modelsState.revision
  };
}

function buildProviderPayload() {
  // variants 是「等级名 -> 供应商参数对象」映射（如 {"high":{"reasoningEffort":"high"}}）。
  // 以 JSON 文本编辑：既保留从 lx-agent 带入的参数，也允许手工配置思考等级。
  const variantsOf = (model) => {
    const raw = (model.variantsJson || '').trim();
    if (raw === '') { return {}; }
    let parsed;
    try { parsed = JSON.parse(raw); } catch (error) {
      throw new Error('Invalid variants JSON for ' + (model.id || 'model') + ': ' + error.message);
    }
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new Error('Variants must be a JSON object for ' + (model.id || 'model'));
    }
    return parsed;
  };
  const provider = {
    key: providerDraft.key,
    id: providerDraft.id,
    type: document.getElementById('prov-type').value,
    name: document.getElementById('prov-name').value.trim() || providerDraft.id,
    baseUrl: document.getElementById('prov-base-url').value.trim(),
    enabled: document.getElementById('prov-enabled').value === 'true',
    models: providerDraft.models
      // 记录键与模型 ID 一致（与 lx-agent 语义相同）：ID 即发往供应商的模型名。
      .filter((model) => (model.id || '').trim() !== '')
      .map((model) => ({
        key: model.id.trim(),
        id: model.id.trim(),
        name: (model.name || '').trim() || model.id.trim(),
        transport: model.transport || null,
        limit: model.context !== '' && model.output !== ''
          ? {context: Number(model.context), output: Number(model.output)}
          : null,
        variant: model.variant || null,
        variants: variantsOf(model)
      }))
  };
  const keyInput = document.getElementById('prov-api-key').value;
  if (document.getElementById('prov-clear-key').checked) {
    provider.clearApiKey = true;
  } else if (keyInput !== '') {
    provider.apiKey = keyInput;
  }
  // 留空且未清除：省略 apiKey（后端保持旧值）。
  return provider;
}

function saveModels() {
  if (!providerDraft) { return; }
  let payload;
  try {
    payload = collectPayload();
  } catch (error) {
    document.getElementById('provider-status').textContent = error.message;
    document.getElementById('provider-status').className = 'status error';
    return;
  }
  api('/api/models/settings', {method: 'PUT', body: JSON.stringify(payload)}).then((result) => {
    document.getElementById('provider-status').textContent = 'Saved';
    document.getElementById('provider-status').className = 'status ok';
    keyTouched = false;
    loadModels();
    void result;
  }).catch((error) => {
    // 409：磁盘配置已被别处改动。自动拉取最新快照刷新 revision，保留当前编辑，
    // 用户再次点击即可保存，无需整页重载。
    if (error.status === 409) {
      document.getElementById('provider-status').textContent = 'Configuration changed elsewhere; refreshed, click Save again';
      document.getElementById('provider-status').className = 'status error';
      loadModels();
      return;
    }
    document.getElementById('provider-status').textContent = error.message;
    document.getElementById('provider-status').className = 'status error';
  });
}

function fetchModels() {
  if (!providerDraft) { return; }
  const status = document.getElementById('fetch-status');
  status.textContent = 'Fetching…';
  status.className = 'status';
  const body = {
    type: document.getElementById('prov-type').value,
    baseUrl: document.getElementById('prov-base-url').value.trim(),
    apiKey: document.getElementById('prov-api-key').value || undefined
  };
  if (providerDraftId !== null) { body.providerId = providerDraftId; }
  api('/api/models/fetch', {method: 'POST', body: JSON.stringify(body)}).then((result) => {
    status.textContent = result.models.length + ' model(s) found';
    status.className = 'status ok';
    renderFetchCandidates(result.models);
  }).catch((error) => {
    status.textContent = error.message;
    status.className = 'status error';
    hideFetchCandidates();
  });
}

function renderFetchCandidates(models) {
  const box = document.getElementById('fetch-candidates');
  while (box.firstChild) { box.removeChild(box.firstChild); }
  box.classList.remove('hidden');
  if (models.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'nav-empty';
    empty.textContent = 'No models returned.';
    box.appendChild(empty);
    return;
  }
  for (const candidate of models) {
    const row = document.createElement('div');
    row.className = 'fetch-row';
    const label = document.createElement('span');
    label.textContent = candidate.id + (candidate.ownedBy ? ' · ' + candidate.ownedBy : '');
    const add = document.createElement('button');
    add.type = 'button';
    add.className = 'ghost';
    add.textContent = 'Add';
    add.onclick = () => {
      // 明确选择加入；不覆盖已编辑模型。
      if (!providerDraft.models.some((model) => model.id === candidate.id)) {
        providerDraft.models.push({key: candidate.id, id: candidate.id, name: candidate.id,
          transport: null, context: '', output: '', variant: '', variantsJson: '{}'});
        renderModelList();
      }
      add.disabled = true;
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


loadState();
