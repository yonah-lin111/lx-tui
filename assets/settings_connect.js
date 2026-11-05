// ===== Quick Connect view =====
// 预设供应商的列表、说明横幅、表单字段、OAuth 授权与连接状态保存。
// 数据来自 GET /api/models/settings 的 quickConnect；本文件在 settings_models.js
// 之后加载，与它共享同一条全局作用域（activeProviderTab / modelsDraft）。

let providerSearch = '';
let presetOAuth = null; // { attemptId, url, userCode, instructions, mode, timer }

function presetList() {
  return (modelsDraft && modelsDraft.quickConnect) || [];
}

function presetBaseline(id) {
  if (!modelsBaseline || !Array.isArray(modelsBaseline.quickConnect)) { return null; }
  return modelsBaseline.quickConnect.find((preset) => preset.id === id) || null;
}

function isPresetDirty(preset) {
  const base = presetBaseline(preset.id);
  return !base || JSON.stringify(preset) !== JSON.stringify(base);
}

function switchProviderTab(tab) {
  activeProviderTab = tab === 'custom' ? 'custom' : 'quick';
  dismissPresetOAuth(false);
  hideFetchCandidates();
  const quickTab = document.getElementById('prov-tab-quick');
  const customTab = document.getElementById('prov-tab-custom');
  const quick = activeProviderTab === 'quick';
  quickTab.classList.toggle('active', quick);
  customTab.classList.toggle('active', !quick);
  quickTab.setAttribute('aria-selected', quick ? 'true' : 'false');
  customTab.setAttribute('aria-selected', quick ? 'false' : 'true');
  document.getElementById('new-provider-row').classList.toggle('hidden', quick);
  renderProviders();
  renderEditor();
  refreshActionBars();
}

function onProviderSearchInput(value) {
  providerSearch = value || '';
  renderProviders();
}

function presetMatchesSearch(preset) {
  const query = providerSearch.trim().toLowerCase();
  if (!query) { return true; }
  return String(preset.name || '').toLowerCase().includes(query)
    || String(preset.id || '').toLowerCase().includes(query);
}

function renderPresetList() {
  const list = document.getElementById('provider-list');
  const items = presetList().filter(presetMatchesSearch);
  items.sort((left, right) => {
    const connected = (left.connected ? 0 : 1) - (right.connected ? 0 : 1);
    if (connected !== 0) { return connected; }
    return String(left.name || left.id).localeCompare(String(right.name || right.id), undefined, {sensitivity: 'base'});
  });
  if (items.length === 0) {
    list.appendChild(element('p', 'nav-empty', 'No matching providers.'));
    return;
  }
  for (const preset of items) {
    const row = element('button', 'nav-item' + (preset.id === selectedPresetId ? ' selected' : '') + (isPresetDirty(preset) ? ' dirty' : ''));
    row.type = 'button';
    row.dataset.key = preset.id;
    row.appendChild(element('span', 'nav-name', preset.name || preset.id));
    const status = element('span', 'nav-status' + (preset.connected ? ' connected' : ''), preset.connectionLabel || 'Not connected');
    row.appendChild(status);
    row.onclick = () => selectPreset(preset.id);
    list.appendChild(row);
  }
}

function selectPreset(id) {
  if (id !== selectedPresetId) { dismissPresetOAuth(false); }
  selectedPresetId = id;
  hideFetchCandidates();
  renderProviders();
  renderEditor();
}

function presetMethods(preset, type) {
  return (preset.methods || []).filter((method) => method.type === type);
}

function presetFieldActive(field, answers) {
  if (!field.whenEq) { return true; }
  return answers[field.whenEq[0]] === field.whenEq[1];
}

function presetMethodsSummary(preset) {
  const labels = [];
  const oauth = presetMethods(preset, 'oauth');
  if (oauth.length) {
    labels.push('Sign-in: ' + oauth.map((method) => method.label).join(', '));
  }
  if (presetMethods(preset, 'apiKey').length) { labels.push('API key'); }
  if ((preset.envVars || []).length) {
    labels.push('Env: ' + preset.envVars.map((name) => '$' + name).join(', '));
  }
  return labels.join(' · ') || 'API key';
}

function presetHasForm(preset) {
  return (preset.methods || []).some((method) => (method.form || []).length > 0);
}

function renderPresetFormFields(preset) {
  const host = document.getElementById('preset-form-fields');
  while (host.firstChild) { host.removeChild(host.firstChild); }
  const answers = preset.formAnswers || (preset.formAnswers = {});
  const seen = new Set();
  for (const method of preset.methods || []) {
    for (const field of method.form || []) {
      if (seen.has(field.key)) { continue; }
      seen.add(field.key);
      if (!presetFieldActive(field, answers)) { continue; }
      host.appendChild(presetField(preset, field, answers));
    }
  }
}

function presetField(preset, field, answers) {
  const wrap = element('div');
  wrap.appendChild(element('label', null, field.label + (field.required ? ' *' : '')));
  const options = Array.isArray(field.options) ? field.options : [];
  if (options.length) {
    const selectHost = element('div', 'select-host');
    wrap.appendChild(selectHost);
    createSelect(selectHost, {
      ariaLabel: field.label,
      value: answers[field.key] || '',
      options: [{value: '', label: field.placeholder || '(select)'}].concat(options.map((option) => ({
        value: option.value,
        label: option.label + (option.description ? ' — ' + option.description : '')
      }))),
      onChange: (value) => {
        answers[field.key] = value;
        markModelsDirty();
        renderPresetFormFields(preset);
      }
    });
    return wrap;
  }
  const input = document.createElement('input');
  input.type = 'text';
  input.value = answers[field.key] || '';
  input.placeholder = field.placeholder || '';
  input.oninput = () => { answers[field.key] = input.value; markModelsDirty(); };
  wrap.appendChild(input);
  return wrap;
}

function renderPresetEditor() {
  const preset = currentPreset();
  document.getElementById('provider-empty').classList.toggle('hidden', !!preset);
  document.getElementById('provider-editor').classList.toggle('hidden', !preset);
  document.getElementById('preset-banner').classList.toggle('hidden', !preset);
  document.getElementById('custom-editor-actions').classList.toggle('hidden', true);
  document.getElementById('preset-editor-actions').classList.toggle('hidden', !preset);
  document.getElementById('preset-oauth-card').classList.toggle('hidden', !preset || !presetMethods(preset, 'oauth').length);
  document.getElementById('preset-fields-card').classList.toggle('hidden', !preset || !presetHasForm(preset));
  if (!preset) { return; }
  document.getElementById('provider-legend').textContent = preset.name || preset.id;
  const status = document.getElementById('preset-status-badge');
  status.textContent = preset.connectionLabel || 'Not connected';
  status.classList.toggle('connected', !!preset.connected);
  document.getElementById('preset-banner-desc').textContent = preset.note || 'Built-in quick connect preset.';
  document.getElementById('preset-banner-methods').textContent = presetMethodsSummary(preset);
  const nameInput = document.getElementById('prov-name');
  nameInput.value = preset.name;
  nameInput.readOnly = true;
  document.getElementById('prov-id').value = preset.id;
  document.getElementById('prov-base-url').value = preset.baseUrl;
  document.getElementById('prov-api-key').value = preset.apiKey;
  document.getElementById('prov-enabled').checked = !!preset.enabled;
  document.getElementById('prov-api-key').type = 'password';
  document.getElementById('prov-key-reveal').classList.remove('revealed');
  document.getElementById('prov-key-reveal').setAttribute('aria-pressed', 'false');
  document.getElementById('prov-key-reveal').setAttribute('aria-label', 'Show API key');
  const typeButton = document.querySelector('#prov-type-host .select-button');
  if (typeButton) { typeButton.disabled = true; }
  modelSelects.provType.setValue(preset.type);
  renderPresetFormFields(preset);
  renderPresetOAuth(preset);
  renderModelList();
}

// Custom 编辑区重新接管：隐藏预设专属区块并恢复名称输入。
function hidePresetEditor() {
  document.getElementById('preset-banner').classList.add('hidden');
  document.getElementById('preset-editor-actions').classList.add('hidden');
  document.getElementById('custom-editor-actions').classList.remove('hidden');
  document.getElementById('preset-oauth-card').classList.add('hidden');
  document.getElementById('preset-fields-card').classList.add('hidden');
  document.getElementById('prov-name').readOnly = false;
  const typeButton = document.querySelector('#prov-type-host .select-button');
  if (typeButton) { typeButton.disabled = false; }
  dismissPresetOAuth(false);
}

function renderPresetOAuth(preset) {
  const host = document.getElementById('preset-oauth-methods');
  while (host.firstChild) { host.removeChild(host.firstChild); }
  for (const method of presetMethods(preset, 'oauth')) {
    const button = element('button', 'primary', 'Sign in with ' + (method.label || 'OAuth'));
    button.type = 'button';
    button.onclick = () => startPresetOAuth(method.id);
    host.appendChild(button);
  }
  renderPresetOAuthPending();
}

function renderPresetOAuthPending() {
  const box = document.getElementById('preset-oauth-pending');
  while (box.firstChild) { box.removeChild(box.firstChild); }
  if (!presetOAuth) {
    box.classList.add('hidden');
    return;
  }
  box.classList.remove('hidden');
  box.appendChild(element('div', null, presetOAuth.instructions || 'Continue in your browser.'));
  const row = element('div', 'preset-oauth-row');
  const open = element('button', 'ghost', 'Open Browser');
  open.type = 'button';
  open.onclick = () => {
    if (presetOAuth && presetOAuth.url) {
      window.open(presetOAuth.url, '_blank', 'noopener,noreferrer');
    }
  };
  row.appendChild(open);
  if (presetOAuth.userCode) {
    row.appendChild(element('span', 'preset-oauth-code', presetOAuth.userCode));
    const copyCode = element('button', 'ghost', 'Copy Code');
    copyCode.type = 'button';
    copyCode.onclick = () => copyPresetOAuth(presetOAuth.userCode);
    row.appendChild(copyCode);
  }
  const copyLink = element('button', 'ghost', 'Copy Link');
  copyLink.type = 'button';
  copyLink.onclick = () => copyPresetOAuth(presetOAuth.url);
  row.appendChild(copyLink);
  const cancel = element('button', 'danger', 'Cancel');
  cancel.type = 'button';
  cancel.onclick = () => cancelPresetOAuth();
  row.appendChild(cancel);
  box.appendChild(row);
  if (presetOAuth.mode === 'code') {
    const inline = element('div', 'preset-oauth-inline');
    const input = document.createElement('input');
    input.type = 'text';
    input.placeholder = 'Paste authorization code';
    inline.appendChild(input);
    const submit = element('button', 'primary', 'Submit Code');
    submit.type = 'button';
    submit.onclick = () => submitPresetOAuthCode(input.value);
    inline.appendChild(submit);
    box.appendChild(inline);
  }
  box.appendChild(element('div', 'preset-oauth-url', presetOAuth.url || ''));
}

function startPresetOAuth(methodId) {
  const preset = currentPreset();
  if (!preset) { return; }
  dismissPresetOAuth(false);
  const body = {
    providerId: preset.id,
    methodId: methodId,
    formAnswers: preset.formAnswers || {}
  };
  api('/api/models/connect/oauth/start', {method: 'POST', body: JSON.stringify(body)}).then((data) => {
    presetOAuth = {
      attemptId: data.attemptId,
      url: data.url || '',
      userCode: data.userCode || '',
      instructions: data.instructions || '',
      mode: data.mode || 'auto',
      timer: null
    };
    renderPresetOAuthPending();
    startPresetOAuthPolling();
  }).catch((error) => showToast(error.message, 'error'));
}

function stopPresetOAuthPolling() {
  if (presetOAuth && presetOAuth.timer) {
    clearInterval(presetOAuth.timer);
    presetOAuth.timer = null;
  }
}

function startPresetOAuthPolling() {
  if (!presetOAuth) { return; }
  stopPresetOAuthPolling();
  presetOAuth.timer = setInterval(pollPresetOAuth, 1000);
  pollPresetOAuth();
}

function pollPresetOAuth() {
  if (!presetOAuth) { return; }
  const attemptId = presetOAuth.attemptId;
  api('/api/models/connect/oauth/status?attemptId=' + encodeURIComponent(attemptId)).then((data) => {
    // 期间用户可能取消或切换了供应商：忽略过期响应。
    if (!presetOAuth || presetOAuth.attemptId !== attemptId) { return; }
    if (data.status === 'complete') {
      const preset = currentPreset();
      const name = preset ? (preset.name || preset.id) : 'provider';
      stopPresetOAuthPolling();
      presetOAuth = null;
      showToast('Connected ' + name, 'ok');
      loadModels();
    } else if (data.status === 'failed') {
      stopPresetOAuthPolling();
      presetOAuth = null;
      renderPresetOAuthPending();
      showToast(data.error || 'OAuth sign-in failed', 'error');
    }
  }).catch(() => {});
}

function dismissPresetOAuth(notify) {
  if (!presetOAuth) { return; }
  stopPresetOAuthPolling();
  const attemptId = presetOAuth.attemptId;
  presetOAuth = null;
  renderPresetOAuthPending();
  api('/api/models/connect/oauth?attemptId=' + encodeURIComponent(attemptId), {method: 'DELETE'}).catch(() => {});
  if (notify) { showToast('Sign-in cancelled', 'info'); }
}

function cancelPresetOAuth() {
  dismissPresetOAuth(true);
}

function submitPresetOAuthCode(value) {
  if (!presetOAuth) { return; }
  const code = String(value || '').trim();
  if (!code) {
    showToast('Authorization code is required', 'error');
    return;
  }
  const body = {attemptId: presetOAuth.attemptId, code: code};
  api('/api/models/connect/oauth/complete', {method: 'POST', body: JSON.stringify(body)})
    .then(() => showToast('Code submitted', 'ok'))
    .catch((error) => showToast(error.message, 'error'));
}

function copyPresetOAuth(text) {
  if (!text) { return; }
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text)
      .then(() => showToast('Copied', 'ok'))
      .catch(() => showToast('Copy failed', 'error'));
  } else {
    showToast('Clipboard unavailable', 'error');
  }
}

function resetPresetProvider() {
  const preset = currentPreset();
  if (!preset) { return; }
  dismissPresetOAuth(false);
  // 含 ${...} 的模板地址必须由后端按表单答案展开，不能作为覆盖值回写。
  preset.baseUrl = preset.defaultBaseUrl && preset.defaultBaseUrl.indexOf('${') < 0 ? preset.defaultBaseUrl : '';
  preset.formAnswers = {};
  preset.models = JSON.parse(JSON.stringify(preset.defaultModels || []));
  markModelsDirty();
  renderModelsView();
  showToast('Reset to preset defaults (save to apply)', 'info');
}

function disconnectPresetProvider() {
  const preset = currentPreset();
  if (!preset || !preset.connected) { return; }
  dismissPresetOAuth(false);
  api('/api/models/connect/disconnect', {
    method: 'POST',
    body: JSON.stringify({providerId: preset.id})
  }).then(() => {
    showToast('Disconnected ' + (preset.name || preset.id), 'ok');
    return loadModels();
  }).catch((error) => showToast(error.message, 'error'));
}

function cancelPresetProvider() {
  const preset = currentPreset();
  if (!preset) { return; }
  dismissPresetOAuth(false);
  const base = presetBaseline(preset.id);
  if (base) {
    const index = presetList().indexOf(preset);
    if (index >= 0) { modelsDraft.quickConnect[index] = JSON.parse(JSON.stringify(base)); }
  }
  selectedPresetId = null;
  markModelsDirty();
  renderModelsView();
}

function buildPresetModels(preset) {
  const seen = new Set();
  const models = [];
  for (const model of preset.models) {
    const id = (model.id || '').trim();
    if (!id) { continue; }
    if (seen.has(id)) { throw new Error('Duplicate model ID ' + id + ' on preset ' + preset.id); }
    seen.add(id);
    const variants = buildVariants(model);
    const variant = model.variant || null;
    if (variant && !Object.prototype.hasOwnProperty.call(variants, variant)) {
      throw new Error('Default thinking variant ' + variant + ' is missing on model ' + id);
    }
    models.push({
      id: id,
      name: (model.name || '').trim() || id,
      transport: model.transport || null,
      limit: limitOf(model, id),
      variant: variant,
      variants: variants,
      modalities: modalitiesOf(model)
    });
  }
  return models;
}

// 顶部 Save：只提交有改动的快捷连接项；保存后由 loadModels 重新对齐基线。
async function savePresetDrafts() {
  if (!modelsDraft) { return; }
  const dirty = presetList().filter((preset) => isPresetDirty(preset));
  if (dirty.length === 0) { return; }
  const providers = dirty.map((preset) => ({
    id: preset.id,
    enabled: !!preset.enabled,
    apiKey: preset.apiKey || '',
    formAnswers: preset.formAnswers || {},
    baseUrl: (preset.baseUrl || '').trim(),
    models: buildPresetModels(preset)
  }));
  await api('/api/models/connect', {method: 'PUT', body: JSON.stringify({providers: providers})});
}

// Model 页下拉：已启用的 Custom 供应商 + 已连接且启用的快捷连接供应商。
function defaultModelOptions() {
  const options = [];
  for (const provider of (modelsDraft && modelsDraft.providers) || []) {
    if (!provider.enabled) { continue; }
    for (const model of provider.models) {
      options.push({
        value: provider.id + '\u0000' + model.id,
        label: (provider.name || provider.id) + ' / ' + (model.name || model.id)
      });
    }
  }
  for (const preset of presetList()) {
    if (!preset.connected || !preset.enabled) { continue; }
    for (const model of preset.models) {
      options.push({
        value: preset.id + '\u0000' + model.id,
        label: (preset.name || preset.id) + ' / ' + (model.name || model.id) + ' (Quick Connect)'
      });
    }
  }
  return options;
}

function resolveDraftSelection(selection) {
  if (!selection) { return null; }
  const provider = (modelsDraft.providers || []).find((item) => item.enabled && item.id === selection.provider)
    || presetList().find((item) => item.connected && item.enabled && item.id === selection.provider);
  if (!provider) { return null; }
  const model = provider.models.find((item) => (item.id || '').trim() === selection.model);
  if (!model) { return null; }
  return {
    provider: provider.id,
    model: model.id.trim(),
    variant: resolveSelectionVariant(selection, model)
  };
}

function initPresetView() {
  const search = document.getElementById('provider-search');
  if (search) {
    search.addEventListener('input', () => onProviderSearchInput(search.value));
  }
  switchProviderTab('quick');
}

initPresetView();
