// ===== Sandbox view =====
let sandboxState = null;
let sandboxBaseline = null;
let sandboxCapabilities = null;
let sandboxRules = null;
let sandboxWarning = null;
let sandboxModeSelect = null;
let sandboxEscalationSelect = null;

function initSandboxSelects() {
  sandboxModeSelect = createSelect(document.getElementById('sandbox-mode-host'), {
    ariaLabel: 'Default sandbox mode',
    value: 'workspace-write',
    options: [
      {value: 'read-only', label: 'Read only'},
      {value: 'workspace-write', label: 'Workspace write'},
      {value: 'danger-full-access', label: 'Danger full access'}
    ]
  });
  sandboxEscalationSelect = createSelect(document.getElementById('sandbox-escalation-host'), {
    ariaLabel: 'Escalation requests',
    value: 'ask',
    options: [
      {value: 'ask', label: 'Ask'},
      {value: 'never', label: 'Never'}
    ]
  });
}

function sandboxValues() {
  return {
    mode: sandboxModeSelect ? sandboxModeSelect.value() : 'workspace-write',
    network_access: document.getElementById('sandbox-network').checked,
    escalation: sandboxEscalationSelect ? sandboxEscalationSelect.value() : 'ask',
    writable_roots: document.getElementById('sandbox-roots').value
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.length > 0)
  };
}

function sandboxDirty() {
  if (!sandboxBaseline) { return false; }
  return JSON.stringify(sandboxValues()) !== JSON.stringify(sandboxBaseline);
}

function renderSandbox() {
  if (!sandboxState) { return; }
  if (sandboxModeSelect) { sandboxModeSelect.setValue(sandboxState.mode); }
  document.getElementById('sandbox-network').checked = !!sandboxState.network_access;
  if (sandboxEscalationSelect) { sandboxEscalationSelect.setValue(sandboxState.escalation); }
  document.getElementById('sandbox-roots').value = (sandboxState.writable_roots || []).join('\n');
  const status = document.getElementById('sandbox-status');
  if (status) {
    let text = sandboxCapabilities
      ? 'Process sandbox: ' + sandboxCapabilities.process_sandbox
      : '';
    if (sandboxCapabilities && sandboxCapabilities.reason) {
      text += (text ? ' — ' : '') + sandboxCapabilities.reason;
    }
    if (sandboxWarning) {
      text += (text ? ' · Warning: ' : 'Warning: ') + sandboxWarning;
    }
    status.textContent = text;
  }
  const rules = document.getElementById('sandbox-rules');
  if (rules) {
    if (sandboxRules) {
      const loaded = sandboxRules.loaded_files || 0;
      const total = sandboxRules.total_files || 0;
      rules.textContent = 'Rules: ' + sandboxRules.directory + ' (' + loaded + '/' + total + ' files loaded)';
      if (sandboxRules.errors && sandboxRules.errors.length > 0) {
        rules.textContent += ' — ' + sandboxRules.errors.join('; ');
      }
    } else {
      rules.textContent = 'Rules: unavailable';
    }
  }
}

async function openSandboxRules() {
  try {
    await api('/api/sandbox/open-rules', {method: 'POST'});
    showToast('Opened rules folder', 'ok');
  } catch (error) {
    showToast(error.message, 'error');
  }
}

function resetSandbox() {
  if (!sandboxBaseline) { return; }
  sandboxState = JSON.parse(JSON.stringify(sandboxBaseline));
  renderSandbox();
  if (typeof refreshActionBars === 'function') { refreshActionBars(); }
}

async function loadSandbox(force) {
  if (sandboxState && !force) { return; }
  const host = document.getElementById('sandbox-status');
  if (host) { host.textContent = 'Loading…'; }
  try {
    const payload = await api('/api/sandbox');
    sandboxState = payload.config || {};
    sandboxBaseline = JSON.parse(JSON.stringify(sandboxState));
    sandboxCapabilities = payload.capabilities || null;
    sandboxRules = payload.rules || null;
    sandboxWarning = payload.warning || null;
    renderSandbox();
    if (typeof refreshActionBars === 'function') { refreshActionBars(); }
    if (sandboxWarning) {
      showToast(sandboxWarning, 'error');
    }
  } catch (error) {
    if (host) { host.textContent = error.message; }
    showToast(error.message, 'error');
  }
}

async function saveSandbox() {
  try {
    const payload = await api('/api/sandbox', {
      method: 'PUT',
      body: JSON.stringify(sandboxValues())
    });
    sandboxState = payload.config || sandboxValues();
    sandboxBaseline = JSON.parse(JSON.stringify(sandboxState));
    sandboxCapabilities = payload.capabilities || sandboxCapabilities;
    sandboxRules = payload.rules || sandboxRules;
    sandboxWarning = payload.warning || null;
    renderSandbox();
    if (typeof refreshActionBars === 'function') { refreshActionBars(); }
    showToast('Sandbox settings saved', 'ok');
  } catch (error) {
    showToast(error.message, 'error');
    throw error;
  }
}

function updateSandboxDirtyDots() {}

initSandboxSelects();

actionBars.sandbox = createActionBar('actions-sandbox', {
  isDirty: sandboxDirty,
  save: saveSandbox,
  reset: resetSandbox
});
