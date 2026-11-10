// ===== Usage Dashboard Controller =====
// 状态管理、筛选器、API 拉取、8 张汇总卡、3 个数据表格与分页控制。

const token = new URLSearchParams(window.location.search).get('token') || '';

const usageState = {
  rangePreset: '7d',
  customStart: '',
  customEnd: '',
  provider: '',
  model: '',
  workspaceId: '',
  sessionId: '',
  activeTab: 'logs',
  page: 1,
  pageSize: 50,
  autoRefreshMs: 0,
  autoRefreshTimer: null,
  loading: false,
  overview: null,
  logPage: null
};

const selects = {};
const openSelects = new Set();

function showToast(message, kind) {
  const host = document.getElementById('toast-host');
  if (!host) { return; }
  const item = domEl('div', 'toast' + (kind ? ' ' + kind : ''), message);
  host.appendChild(item);
  setTimeout(() => {
    item.classList.add('leaving');
    setTimeout(() => item.remove(), 200);
  }, 3200);
}

// ===== 自绘下拉选择器（与 settings.js 交互一致） =====

function createSelect(host, config) {
  if (!host) { return null; }
  const optionsConfig = config || {};
  if (host._selectInstance) {
    openSelects.delete(host._selectInstance);
  }
  if (host._selectMenu && host._selectMenu.parentNode) {
    host._selectMenu.parentNode.removeChild(host._selectMenu);
  }
  host.replaceChildren();
  host.classList.add('select-host');

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'select-button';
  button.disabled = !!optionsConfig.disabled;
  button.setAttribute('aria-haspopup', 'listbox');
  if (optionsConfig.ariaLabel) {
    button.setAttribute('aria-label', optionsConfig.ariaLabel);
  }
  const label = domEl('span', 'select-label');
  const caret = domEl('span', 'select-caret', '▾');
  button.appendChild(label);
  button.appendChild(caret);
  host.appendChild(button);

  const menu = domEl('div', 'select-menu hidden');
  menu.setAttribute('role', 'listbox');
  document.body.appendChild(menu);

  let options = [];
  let current = optionsConfig.value === undefined ? '' : optionsConfig.value;
  let active = 0;
  let opened = false;

  const renderLabel = () => {
    const match = options.find((opt) => opt.value === current);
    label.textContent = match ? match.label : (optionsConfig.placeholder || '');
    label.classList.toggle('placeholder', !match);
  };

  const renderMenu = () => {
    menu.replaceChildren();
    options.forEach((opt, index) => {
      const row = domEl(
        'button',
        'select-option' + (opt.value === current ? ' selected' : '') + (index === active ? ' active' : ''),
        opt.label
      );
      row.type = 'button';
      row.setAttribute('role', 'option');
      row.onclick = () => choose(opt.value, true);
      menu.appendChild(row);
    });
  };

  const reposition = () => {
    const rect = button.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom - 8;
    const spaceAbove = rect.top - 8;
    const openUp = spaceBelow < 140 && spaceAbove > spaceBelow;
    menu.style.left = rect.left + 'px';
    menu.style.width = Math.max(rect.width, 140) + 'px';
    menu.style.maxHeight = Math.min(240, Math.max(120, openUp ? spaceAbove : spaceBelow)) + 'px';
    if (openUp) {
      menu.style.top = 'auto';
      menu.style.bottom = (window.innerHeight - rect.top + 4) + 'px';
    } else {
      menu.style.bottom = 'auto';
      menu.style.top = (rect.bottom + 4) + 'px';
    }
  };

  const close = () => {
    opened = false;
    menu.classList.add('hidden');
    button.setAttribute('aria-expanded', 'false');
    openSelects.delete(instance);
  };

  const choose = (value, notify) => {
    const changed = value !== current;
    current = value;
    close();
    renderLabel();
    if (notify && changed && typeof optionsConfig.onChange === 'function') {
      optionsConfig.onChange(value);
    }
  };

  const open = () => {
    if (opened || button.disabled) { return; }
    openSelects.forEach((other) => other.close());
    opened = true;
    menu.classList.remove('hidden');
    button.setAttribute('aria-expanded', 'true');
    active = Math.max(0, options.findIndex((opt) => opt.value === current));
    renderMenu();
    reposition();
    openSelects.add(instance);
  };

  button.addEventListener('click', () => {
    if (opened) { close(); } else { open(); }
  });
  button.addEventListener('keydown', (evt) => {
    if (evt.key === 'ArrowDown' || evt.key === 'ArrowUp') {
      evt.preventDefault();
      if (!opened) { open(); return; }
      const step = evt.key === 'ArrowDown' ? 1 : -1;
      active = (active + step + options.length) % Math.max(1, options.length);
      renderMenu();
      return;
    }
    if (evt.key === 'Enter' || evt.key === ' ') {
      evt.preventDefault();
      if (!opened) { open(); return; }
      if (options[active]) { choose(options[active].value, true); }
      return;
    }
    if (evt.key === 'Escape') { close(); }
  });

  const instance = {
    el: host,
    menu: menu,
    value: () => current,
    close: close,
    reposition: reposition,
    setOptions: (next, value) => {
      options = next || [];
      if (value !== undefined) { current = value; }
      if (!options.some((opt) => opt.value === current)) { current = ''; }
      renderLabel();
      renderMenu();
    },
    setValue: (value) => {
      current = value;
      renderLabel();
      if (opened) { renderMenu(); }
    },
    setDisabled: (disabled) => {
      button.disabled = !!disabled;
      if (disabled && opened) { close(); }
    }
  };
  host._selectMenu = menu;
  host._selectInstance = instance;
  instance.setOptions(optionsConfig.options || [], current);
  return instance;
}

document.addEventListener('click', (evt) => {
  openSelects.forEach((inst) => {
    if (!inst.el.contains(evt.target) && !inst.menu.contains(evt.target)) {
      inst.close();
    }
  });
});
window.addEventListener('resize', () => openSelects.forEach((inst) => inst.reposition()));
window.addEventListener('scroll', () => openSelects.forEach((inst) => inst.reposition()), true);

// ===== API 与时间范围解析 =====

function apiGet(path, params) {
  const search = new URLSearchParams();
  if (token) { search.set('token', token); }
  if (params && typeof params === 'object') {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== null && v !== '') {
        search.set(k, String(v));
      }
    }
  }
  const url = path + '?' + search.toString();
  return fetch(url, {headers: {'Accept': 'application/json'}}).then(async (response) => {
    const text = await response.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch (_err) { data = null; }
    if (!response.ok) {
      const msg = data && data.error ? data.error : ('HTTP ' + response.status);
      throw new Error(msg);
    }
    return data;
  });
}

function resolveTimeRange(preset, customStart, customEnd) {
  const now = Date.now();
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  const endOfToday = new Date(now);
  endOfToday.setHours(23, 59, 59, 999);

  if (preset === 'all') {
    return {startTime: undefined, endTime: undefined, granularity: 'day'};
  }
  if (preset === 'today') {
    return {startTime: startOfToday.getTime(), endTime: endOfToday.getTime(), granularity: 'hour'};
  }
  if (preset === '7d' || preset === '30d') {
    const days = preset === '7d' ? 7 : 30;
    const start = new Date(startOfToday);
    start.setDate(start.getDate() - (days - 1));
    return {startTime: start.getTime(), endTime: endOfToday.getTime(), granularity: 'day'};
  }
  if (preset === 'custom') {
    let sKey = customStart || toLocalDateKey(now);
    let eKey = customEnd || sKey;
    if (sKey > eKey) {
      const tmp = sKey;
      sKey = eKey;
      eKey = tmp;
    }
    const sParts = sKey.split('-').map((p) => Number.parseInt(p, 10));
    const eParts = eKey.split('-').map((p) => Number.parseInt(p, 10));
    const startMs = new Date(sParts[0], sParts[1] - 1, sParts[2], 0, 0, 0, 0).getTime();
    const endMs = new Date(eParts[0], eParts[1] - 1, eParts[2], 23, 59, 59, 999).getTime();
    return {
      startTime: Number.isFinite(startMs) ? startMs : startOfToday.getTime(),
      endTime: Number.isFinite(endMs) ? endMs : endOfToday.getTime(),
      granularity: sKey === eKey ? 'hour' : 'day'
    };
  }
  return {startTime: undefined, endTime: undefined, granularity: 'day'};
}

function buildFilterParams(range) {
  return {
    startTime: range.startTime,
    endTime: range.endTime,
    granularity: range.granularity,
    provider: usageState.provider || undefined,
    model: usageState.model || undefined,
    workspaceId: usageState.workspaceId || undefined,
    sessionId: usageState.sessionId || undefined
  };
}

// ===== 数据拉取与级联筛选 =====

function setStatusIndicator(stateName, text) {
  const dot = document.getElementById('usage-status-dot');
  const label = document.getElementById('last-updated');
  if (dot) { dot.className = 'status-dot ' + stateName; }
  if (label) { label.textContent = text; }
}

function setErrorBanner(message) {
  const errEl = document.getElementById('usage-error');
  if (!errEl) { return; }
  if (!message) {
    errEl.textContent = '';
    errEl.classList.add('hidden');
  } else {
    errEl.textContent = message;
    errEl.classList.remove('hidden');
  }
}

async function loadUsageDashboard() {
  if (usageState.loading) { return; }
  usageState.loading = true;
  setStatusIndicator('loading', 'Refreshing…');
  const range = resolveTimeRange(usageState.rangePreset, usageState.customStart, usageState.customEnd);
  const baseParams = buildFilterParams(range);
  const logParams = Object.assign({}, baseParams, {
    page: usageState.page,
    pageSize: usageState.pageSize
  });

  try {
    const [overview, logPage] = await Promise.all([
      apiGet('/api/usage/overview', baseParams),
      apiGet('/api/usage/logs', logParams)
    ]);
    usageState.overview = overview || {};
    usageState.logPage = logPage || {rows: [], total: 0, page: 1, pageSize: usageState.pageSize};
    syncFilterOptions(usageState.overview.filters || {});
    renderSummaryCards(usageState.overview.summary || null);
    renderCharts(usageState.overview, range);
    renderTableSection();
    setErrorBanner('');
    const nowText = new Date().toLocaleTimeString('en-US', {hour12: false});
    setStatusIndicator('clean', 'Updated ' + nowText);
  } catch (error) {
    setStatusIndicator('error', 'Load failed');
    setErrorBanner('Failed to load usage analytics: ' + error.message);
    showToast(error.message, 'error');
  } finally {
    usageState.loading = false;
  }
}

async function loadUsageLogsPage(nextPage) {
  if (usageState.loading) { return; }
  usageState.page = Math.max(1, nextPage);
  usageState.loading = true;
  setStatusIndicator('loading', 'Loading page…');
  const range = resolveTimeRange(usageState.rangePreset, usageState.customStart, usageState.customEnd);
  const logParams = Object.assign({}, buildFilterParams(range), {
    page: usageState.page,
    pageSize: usageState.pageSize
  });

  try {
    const logPage = await apiGet('/api/usage/logs', logParams);
    usageState.logPage = logPage || {rows: [], total: 0, page: usageState.page, pageSize: usageState.pageSize};
    renderTableSection();
    const nowText = new Date().toLocaleTimeString('en-US', {hour12: false});
    setStatusIndicator('clean', 'Updated ' + nowText);
  } catch (error) {
    setStatusIndicator('error', 'Load failed');
    showToast(error.message, 'error');
  } finally {
    usageState.loading = false;
  }
}

function syncFilterOptions(filters) {
  const providers = Array.isArray(filters.providers) ? filters.providers : [];
  const models = Array.isArray(filters.models) ? filters.models : [];
  const workspaces = Array.isArray(filters.workspaces)
    ? filters.workspaces
    : (Array.isArray(filters.projects) ? filters.projects : []);
  const sessions = Array.isArray(filters.sessions) ? filters.sessions : [];

  if (usageState.provider && !providers.includes(usageState.provider)) {
    usageState.provider = '';
  }
  if (usageState.model && !models.includes(usageState.model)) {
    usageState.model = '';
  }
  if (usageState.workspaceId && !workspaces.some((w) => w.id === usageState.workspaceId)) {
    usageState.workspaceId = '';
  }
  if (usageState.sessionId && !sessions.some((s) => s.id === usageState.sessionId)) {
    usageState.sessionId = '';
  }

  selects.provider.setOptions(
    [{value: '', label: 'All providers'}].concat(providers.map((p) => ({value: p, label: p}))),
    usageState.provider
  );
  selects.model.setOptions(
    [{value: '', label: 'All models'}].concat(models.map((m) => ({value: m, label: m}))),
    usageState.model
  );
  selects.workspace.setOptions(
    [{value: '', label: 'All workspaces'}].concat(workspaces.map((w) => ({value: w.id, label: w.name || w.id}))),
    usageState.workspaceId
  );
  selects.session.setOptions(
    [{value: '', label: 'All sessions'}].concat(sessions.map((s) => ({value: s.id, label: s.name || s.id}))),
    usageState.sessionId
  );
}

// ===== 8 张汇总卡片渲染 =====

function renderSummaryCards(summary) {
  const host = document.getElementById('summary-cards');
  if (!host) { return; }
  host.replaceChildren();

  const s = summary || {};
  const inputTokens = Number(s.inputTokens || 0);
  const outputTokens = Number(s.outputTokens || 0);
  const cacheReadTokens = Number(s.cacheReadTokens || 0);
  const cacheWriteTokens = Number(s.cacheWriteTokens || 0);
  const totalTokens = Number(s.totalTokens !== undefined ? s.totalTokens : (inputTokens + outputTokens));
  const freshInputTokens = getFreshInputTokens(s);
  const requestCount = Number(s.requestCount || 0);
  const successCount = Number(s.successCount || 0);
  const errorCount = Number(s.errorCount || 0);
  const abortedCount = Number(s.abortedCount || 0);
  const pricedCount = Number(s.pricedRequestCount || 0);
  const cacheHitRate = calcCacheHitRate(inputTokens, cacheReadTokens);

  const shareOfInput = (part) => (inputTokens > 0 ? formatPercent((part / inputTokens) * 100, 1) + ' of input' : '-- of input');
  const avgOut = requestCount > 0 ? formatCompact(Math.round(outputTokens / requestCount)) : '--';

  const cards = [
    {
      label: 'Tokens Processed',
      value: formatCompact(totalTokens),
      sub: 'in: ' + formatCompact(inputTokens) + ' · out: ' + formatCompact(outputTokens)
    },
    {
      label: 'Fresh Input',
      value: formatCompact(freshInputTokens),
      sub: shareOfInput(freshInputTokens)
    },
    {
      label: 'Output Tokens',
      value: formatCompact(outputTokens),
      sub: 'avg ' + avgOut + ' / req'
    },
    {
      label: 'Cache Read',
      value: formatCompact(cacheReadTokens),
      sub: shareOfInput(cacheReadTokens)
    },
    {
      label: 'Cache Write',
      value: formatCompact(cacheWriteTokens),
      sub: shareOfInput(cacheWriteTokens)
    },
    {
      label: 'Cache Hit Rate',
      value: cacheHitRate === null ? '--' : formatPercent(cacheHitRate, 1),
      meter: cacheHitRate,
      sub: 'read ' + formatCompact(cacheReadTokens) + ' · write ' + formatCompact(cacheWriteTokens)
    },
    {
      label: 'Requests',
      value: formatNumber(requestCount),
      sub: 'ok ' + formatNumber(successCount) + ' · err ' + formatNumber(errorCount) + ' · abort ' + formatNumber(abortedCount) + ' · avg ' + formatDuration(s.avgDurationMs)
    },
    {
      label: 'Total Cost',
      value: formatUsd(s.totalCostUsd),
      sub: 'priced ' + formatNumber(pricedCount) + ' / ' + formatNumber(requestCount) + ' reqs'
    }
  ];

  for (const item of cards) {
    const card = domEl('div', 'summary-card');
    card.appendChild(domEl('div', 'summary-label', item.label));
    card.appendChild(domEl('div', 'summary-value', item.value));
    if (item.meter !== undefined) {
      const meter = domEl('div', 'cache-hit-meter');
      const fill = domEl('div', 'cache-hit-fill');
      fill.style.width = Math.max(0, Math.min(100, Number(item.meter || 0))) + '%';
      meter.appendChild(fill);
      card.appendChild(meter);
    }
    card.appendChild(domEl('div', 'summary-sub', item.sub));
    host.appendChild(card);
  }
}

// ===== 5 张图表联动渲染 =====

function renderCharts(overview, range) {
  const rawSeries = (overview && (overview.daily || overview.series)) || [];
  const filledSeries = fillUsageSeries(rawSeries, {
    granularity: range.granularity,
    startTime: range.startTime,
    endTime: range.endTime
  });
  const trendSub = document.getElementById('chart-trend-subtitle');
  if (trendSub) {
    trendSub.textContent = range.granularity === 'hour'
      ? 'Hourly stacked token volume and USD cost'
      : 'Daily stacked token volume and USD cost';
  }
  renderTrendChart(document.getElementById('chart-trend'), filledSeries, range.granularity);
  renderRequestsChart(document.getElementById('chart-requests'), filledSeries, range.granularity);
  renderModelDistributionChart(
    document.getElementById('chart-models'),
    (overview && overview.models) || [],
    document.getElementById('chart-models-subtitle')
  );
  renderProviderDistributionChart(
    document.getElementById('chart-providers'),
    (overview && overview.providers) || [],
    document.getElementById('chart-providers-subtitle')
  );
  renderTokenCompositionChart(
    document.getElementById('chart-composition'),
    overview ? overview.summary : null
  );
}

// ===== 底部 3 个 Tab 表格与分页 =====

function renderTableSection() {
  const overview = usageState.overview || {};
  const logPage = usageState.logPage || {rows: [], total: 0, page: 1, pageSize: usageState.pageSize};
  const models = Array.isArray(overview.models) ? overview.models : [];
  const providers = Array.isArray(overview.providers) ? overview.providers : [];

  const logsBtn = document.getElementById('tab-btn-logs');
  const modelsBtn = document.getElementById('tab-btn-models');
  const providersBtn = document.getElementById('tab-btn-providers');
  if (logsBtn) { logsBtn.textContent = 'Request Logs (' + formatNumber(logPage.total || 0) + ')'; }
  if (modelsBtn) { modelsBtn.textContent = 'Model Stats (' + formatNumber(models.length) + ')'; }
  if (providersBtn) { providersBtn.textContent = 'Provider Stats (' + formatNumber(providers.length) + ')'; }

  for (const [tabKey, btn] of [['logs', logsBtn], ['models', modelsBtn], ['providers', providersBtn]]) {
    if (!btn) { continue; }
    const active = usageState.activeTab === tabKey;
    btn.classList.toggle('active', active);
    btn.setAttribute('aria-selected', active ? 'true' : 'false');
  }

  const container = document.getElementById('table-container');
  const pagination = document.getElementById('logs-pagination');
  if (!container) { return; }

  if (usageState.activeTab === 'logs') {
    if (pagination) { pagination.classList.remove('hidden'); }
    renderLogsTable(container, logPage);
    renderPaginationBar(logPage);
  } else if (usageState.activeTab === 'models') {
    if (pagination) { pagination.classList.add('hidden'); }
    renderModelsTable(container, models);
  } else {
    if (pagination) { pagination.classList.add('hidden'); }
    renderProvidersTable(container, providers);
  }
}

function buildTable(headers) {
  const table = domEl('table', 'usage-table');
  const thead = domEl('thead');
  const tr = domEl('tr');
  for (const h of headers) {
    const th = domEl('th', h.numeric ? 'num' : '', h.label);
    tr.appendChild(th);
  }
  thead.appendChild(tr);
  table.appendChild(thead);
  const tbody = domEl('tbody');
  table.appendChild(tbody);
  return {table: table, tbody: tbody};
}

function appendEmptyRow(tbody, colSpan, message) {
  const tr = domEl('tr');
  const td = domEl('td', 'table-empty', message || 'No usage records in the selected range.');
  td.colSpan = colSpan;
  tr.appendChild(td);
  tbody.appendChild(tr);
}

function renderLogsTable(container, logPage) {
  container.replaceChildren();
  const headers = [
    {label: 'Time'},
    {label: 'Purpose'},
    {label: 'Provider'},
    {label: 'Model'},
    {label: 'Input', numeric: true},
    {label: 'Output', numeric: true},
    {label: 'Cache Read', numeric: true},
    {label: 'Cache Write', numeric: true},
    {label: 'Cost', numeric: true},
    {label: 'Duration', numeric: true},
    {label: 'Status', numeric: true}
  ];
  const {table, tbody} = buildTable(headers);
  const rows = (logPage && Array.isArray(logPage.rows)) ? logPage.rows : [];
  if (rows.length === 0) {
    appendEmptyRow(tbody, headers.length);
    container.appendChild(table);
    return;
  }

  for (const row of rows) {
    const tr = domEl('tr');
    const createdAt = row.createdAtMs !== undefined ? row.createdAtMs : row.createdAt;
    const tokens = row.tokens || {
      input: row.inputTokens || 0,
      output: row.outputTokens || 0,
      cacheRead: row.cacheReadTokens || 0,
      cacheWrite: row.cacheWriteTokens || 0
    };
    const cost = row.rates ? row.rates.totalCostUsd : row.totalCostUsd;
    const purpose = String(row.purpose || 'chat');
    const status = String(row.status || 'success');

    tr.appendChild(domEl('td', 'cell-muted', formatTimestamp(createdAt)));

    const purposeTd = domEl('td');
    purposeTd.appendChild(domEl('span', 'purpose-badge purpose-' + purpose, purpose));
    tr.appendChild(purposeTd);

    tr.appendChild(domEl('td', 'cell-truncate', row.provider || '--'));
    tr.appendChild(domEl('td', 'cell-truncate', row.model || '--'));
    tr.appendChild(domEl('td', 'num', formatNumber(tokens.input)));
    tr.appendChild(domEl('td', 'num', formatNumber(tokens.output)));
    tr.appendChild(domEl('td', 'num', formatNumber(tokens.cacheRead)));
    tr.appendChild(domEl('td', 'num', formatNumber(tokens.cacheWrite)));
    tr.appendChild(domEl('td', 'num', formatUsd(cost)));
    tr.appendChild(domEl('td', 'num', formatDuration(row.durationMs)));

    const statusTd = domEl('td', 'num');
    const badge = domEl('span', 'status-badge ' + status, status);
    if (row.errorMessage) {
      badge.title = String(row.errorMessage);
    }
    statusTd.appendChild(badge);
    tr.appendChild(statusTd);
    tbody.appendChild(tr);
  }
  container.appendChild(table);
}

function renderModelsTable(container, models) {
  container.replaceChildren();
  const headers = [
    {label: 'Model'},
    {label: 'Requests', numeric: true},
    {label: 'Input', numeric: true},
    {label: 'Output', numeric: true},
    {label: 'Cache Read', numeric: true},
    {label: 'Cache Write', numeric: true},
    {label: 'Total Tokens', numeric: true},
    {label: 'Cost', numeric: true},
    {label: 'Avg Cost', numeric: true}
  ];
  const {table, tbody} = buildTable(headers);
  if (!models.length) {
    appendEmptyRow(tbody, headers.length);
    container.appendChild(table);
    return;
  }
  for (const m of models) {
    const tr = domEl('tr');
    tr.appendChild(domEl('td', 'cell-truncate', m.model || '--'));
    tr.appendChild(domEl('td', 'num', formatNumber(m.requestCount)));
    tr.appendChild(domEl('td', 'num', formatNumber(m.inputTokens)));
    tr.appendChild(domEl('td', 'num', formatNumber(m.outputTokens)));
    tr.appendChild(domEl('td', 'num', formatNumber(m.cacheReadTokens)));
    tr.appendChild(domEl('td', 'num', formatNumber(m.cacheWriteTokens)));
    tr.appendChild(domEl('td', 'num', formatNumber(m.totalTokens)));
    tr.appendChild(domEl('td', 'num', formatUsd(m.totalCostUsd)));
    tr.appendChild(domEl('td', 'num', formatUsd(m.avgCostPerRequestUsd)));
    tbody.appendChild(tr);
  }
  container.appendChild(table);
}

function renderProvidersTable(container, providers) {
  container.replaceChildren();
  const headers = [
    {label: 'Provider'},
    {label: 'Requests', numeric: true},
    {label: 'Total Tokens', numeric: true},
    {label: 'Cost', numeric: true},
    {label: 'Success Rate', numeric: true},
    {label: 'Avg Duration', numeric: true}
  ];
  const {table, tbody} = buildTable(headers);
  if (!providers.length) {
    appendEmptyRow(tbody, headers.length);
    container.appendChild(table);
    return;
  }
  for (const p of providers) {
    const tr = domEl('tr');
    tr.appendChild(domEl('td', 'cell-truncate', p.provider || '--'));
    tr.appendChild(domEl('td', 'num', formatNumber(p.requestCount)));
    tr.appendChild(domEl('td', 'num', formatNumber(p.totalTokens)));
    tr.appendChild(domEl('td', 'num', formatUsd(p.totalCostUsd)));
    tr.appendChild(domEl('td', 'num', formatPercent(p.successRate, 1)));
    tr.appendChild(domEl('td', 'num', formatDuration(p.avgDurationMs)));
    tbody.appendChild(tr);
  }
  container.appendChild(table);
}

function renderPaginationBar(logPage) {
  const total = Number((logPage && logPage.total) || 0);
  const page = Number((logPage && logPage.page) || usageState.page || 1);
  const pageSize = Number((logPage && logPage.pageSize) || usageState.pageSize || 50);
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const info = document.getElementById('pagination-info');
  const prev = document.getElementById('page-prev');
  const next = document.getElementById('page-next');
  if (info) {
    info.textContent = 'Page ' + page + ' of ' + totalPages + ' · ' + formatNumber(total) + ' requests';
  }
  if (prev) { prev.disabled = page <= 1; }
  if (next) { next.disabled = page >= totalPages; }
}

// ===== 自动刷新与事件绑定 =====

function updateAutoRefresh(intervalMs) {
  usageState.autoRefreshMs = Number(intervalMs) || 0;
  if (usageState.autoRefreshTimer) {
    clearInterval(usageState.autoRefreshTimer);
    usageState.autoRefreshTimer = null;
  }
  if (usageState.autoRefreshMs > 0) {
    usageState.autoRefreshTimer = setInterval(() => {
      loadUsageDashboard();
    }, usageState.autoRefreshMs);
  }
}

function initUsageDashboard() {
  const now = Date.now();
  usageState.customStart = toLocalDateKey(now - 6 * 86400000);
  usageState.customEnd = toLocalDateKey(now);

  const startInput = document.getElementById('custom-start');
  const endInput = document.getElementById('custom-end');
  if (startInput) {
    startInput.value = usageState.customStart;
    startInput.addEventListener('change', () => {
      usageState.customStart = startInput.value;
      usageState.sessionId = '';
      usageState.page = 1;
      loadUsageDashboard();
    });
  }
  if (endInput) {
    endInput.value = usageState.customEnd;
    endInput.addEventListener('change', () => {
      usageState.customEnd = endInput.value;
      usageState.sessionId = '';
      usageState.page = 1;
      loadUsageDashboard();
    });
  }

  for (const btn of document.querySelectorAll('.range-btn')) {
    btn.addEventListener('click', () => {
      const preset = btn.dataset.range || '7d';
      usageState.rangePreset = preset;
      usageState.sessionId = '';
      usageState.page = 1;
      for (const other of document.querySelectorAll('.range-btn')) {
        other.classList.toggle('active', other === btn);
      }
      const customWrap = document.getElementById('custom-range-wrap');
      if (customWrap) {
        customWrap.classList.toggle('hidden', preset !== 'custom');
      }
      loadUsageDashboard();
    });
  }

  selects.provider = createSelect(document.getElementById('filter-provider'), {
    ariaLabel: 'Filter by provider',
    value: '',
    options: [{value: '', label: 'All providers'}],
    onChange: (value) => {
      usageState.provider = value;
      usageState.model = '';
      usageState.page = 1;
      loadUsageDashboard();
    }
  });
  selects.model = createSelect(document.getElementById('filter-model'), {
    ariaLabel: 'Filter by model',
    value: '',
    options: [{value: '', label: 'All models'}],
    onChange: (value) => {
      usageState.model = value;
      usageState.page = 1;
      loadUsageDashboard();
    }
  });
  selects.workspace = createSelect(document.getElementById('filter-workspace'), {
    ariaLabel: 'Filter by workspace',
    value: '',
    options: [{value: '', label: 'All workspaces'}],
    onChange: (value) => {
      usageState.workspaceId = value;
      usageState.sessionId = '';
      usageState.page = 1;
      loadUsageDashboard();
    }
  });
  selects.session = createSelect(document.getElementById('filter-session'), {
    ariaLabel: 'Filter by session',
    value: '',
    options: [{value: '', label: 'All sessions'}],
    onChange: (value) => {
      usageState.sessionId = value;
      usageState.page = 1;
      loadUsageDashboard();
    }
  });
  selects.autoRefresh = createSelect(document.getElementById('auto-refresh-filter'), {
    ariaLabel: 'Auto refresh interval',
    value: '0',
    options: [
      {value: '0', label: 'Off'},
      {value: '5000', label: '5s'},
      {value: '10000', label: '10s'},
      {value: '30000', label: '30s'}
    ],
    onChange: (value) => updateAutoRefresh(Number(value))
  });

  const refreshBtn = document.getElementById('refresh-btn');
  if (refreshBtn) {
    refreshBtn.addEventListener('click', () => loadUsageDashboard());
  }

  for (const tabBtn of document.querySelectorAll('.usage-tabs .provider-tab')) {
    tabBtn.addEventListener('click', () => {
      usageState.activeTab = tabBtn.dataset.tab || 'logs';
      renderTableSection();
    });
  }

  const prevBtn = document.getElementById('page-prev');
  const nextBtn = document.getElementById('page-next');
  if (prevBtn) {
    prevBtn.addEventListener('click', () => {
      if (usageState.page > 1) {
        loadUsageLogsPage(usageState.page - 1);
      }
    });
  }
  if (nextBtn) {
    nextBtn.addEventListener('click', () => {
      const total = Number((usageState.logPage && usageState.logPage.total) || 0);
      const totalPages = Math.max(1, Math.ceil(total / usageState.pageSize));
      if (usageState.page < totalPages) {
        loadUsageLogsPage(usageState.page + 1);
      }
    });
  }

  loadUsageDashboard();
}

initUsageDashboard();
