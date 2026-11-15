// ===== Todo Dashboard Controller =====
// 只读看板：日期范围 + 状态/优先级/文本筛选，6 张汇总卡与 3 张表格。
// 所有顶层名字统一加 `todo` 前缀，避免与 usage.js / usage_charts.js / web.js 的全局作用域冲突。

const todoState = {
  rangePreset: '7d',
  customStart: '',
  customEnd: '',
  status: '',
  priority: '',
  search: '',
  activeTab: 'todos',
  page: 1,
  pageSize: 50,
  loading: false,
  requestSeq: 0,
  overview: null,
  itemPage: null,
  initialRendered: false
};

const todoSelects = {};
let todoSearchTimer = null;

// ===== 日期与筛选参数 =====

function todoDateKey(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return year + '-' + month + '-' + day;
}

function todoShiftDate(key, deltaDays) {
  const parts = key.split('-').map((part) => Number.parseInt(part, 10));
  const date = new Date(parts[0], parts[1] - 1, parts[2]);
  date.setDate(date.getDate() + deltaDays);
  return todoDateKey(date);
}

function resolveTodoRange(preset, customStart, customEnd) {
  const today = todoDateKey(new Date());
  if (preset === 'all') {
    return {startDate: undefined, endDate: undefined};
  }
  if (preset === 'today') {
    return {startDate: today, endDate: today};
  }
  if (preset === '7d' || preset === '30d') {
    const days = preset === '7d' ? 7 : 30;
    return {startDate: todoShiftDate(today, -(days - 1)), endDate: today};
  }
  if (preset === 'custom') {
    let start = customStart || todoShiftDate(today, -6);
    let end = customEnd || today;
    if (start > end) {
      const tmp = start;
      start = end;
      end = tmp;
    }
    return {startDate: start, endDate: end};
  }
  return {startDate: todoShiftDate(today, -6), endDate: today};
}

function todoFilterParams(range) {
  return {
    startDate: range.startDate,
    endDate: range.endDate,
    status: todoState.status || undefined,
    priority: todoState.priority || undefined,
    q: todoState.search.trim() || undefined
  };
}

// ===== 状态提示与数据拉取 =====

function setTodoStatus(stateName, text) {
  const dot = document.getElementById('todo-status-dot');
  const label = document.getElementById('todo-last-updated');
  const spinner = document.getElementById('todo-status-spinner');
  const loading = stateName === 'loading';
  const hasData = !!todoState.overview;
  // 已有数据时刷新只转 spinner：不改写“Updated HH:MM:SS”，否则快速请求会闪文案。
  if (dot && !(loading && hasData)) { dot.className = 'status-dot ' + stateName; }
  if (spinner) { spinner.classList.toggle('on', loading); }
  if (loading && hasData) { return; }
  if (label) { label.textContent = loading ? 'Loading…' : text; }
}

function setTodoError(message) {
  const errEl = document.getElementById('todo-error');
  if (!errEl) { return; }
  if (!message) {
    errEl.textContent = '';
    errEl.classList.add('hidden');
  } else {
    errEl.textContent = message;
    errEl.classList.remove('hidden');
  }
}

function isTodoViewActive() {
  const section = document.getElementById('view-todo');
  return !!section && section.classList.contains('active');
}

async function loadTodoDashboard() {
  const reqId = ++todoState.requestSeq;
  todoState.loading = true;
  setTodoStatus('loading');
  const range = resolveTodoRange(todoState.rangePreset, todoState.customStart, todoState.customEnd);
  const base = todoFilterParams(range);
  const itemParams = Object.assign({}, base, {
    page: todoState.page,
    pageSize: todoState.pageSize
  });

  try {
    const [overview, itemPage] = await Promise.all([
      apiGet('/api/todo/overview', base),
      apiGet('/api/todo/items', itemParams)
    ]);
    if (reqId !== todoState.requestSeq) { return; }
    todoState.overview = overview || {};
    todoState.itemPage = itemPage || {rows: [], total: 0, page: 1, pageSize: todoState.pageSize};
    const totalPages = Math.max(1, Math.ceil(Number(todoState.itemPage.total || 0) / todoState.pageSize));
    if (todoState.page > totalPages) {
      todoState.page = totalPages;
      return loadTodoDashboard();
    }
    renderTodoSummary(todoState.overview.summary || null);
    renderTodoCharts(todoState.overview || {}, range);
    renderTodoTableSection();
    if (!todoState.initialRendered) {
      todoState.initialRendered = true;
      todoFadeInInitial();
    }
    setTodoError('');
    const nowText = new Date().toLocaleTimeString('en-US', {hour12: false});
    setTodoStatus('clean', 'Updated ' + nowText);
  } catch (error) {
    if (reqId !== todoState.requestSeq) { return; }
    // 首次请求失败时把骨架换成真正的空态，避免一直闪“加载中”而只有 banner 说明失败。
    if (!todoState.overview) {
      renderTodoSummary(null);
      renderTodoCharts({}, range);
      renderTodoTableSection();
    }
    setTodoStatus('error', 'Load failed');
    setTodoError('Failed to load todos: ' + error.message);
    if (isTodoViewActive()) { showToast(error.message, 'error'); }
  } finally {
    if (reqId === todoState.requestSeq) {
      todoState.loading = false;
    }
  }
}

async function loadTodoItemsPage(nextPage) {
  const targetPage = Math.max(1, nextPage);
  const reqId = ++todoState.requestSeq;
  todoState.loading = true;
  setTodoStatus('loading');
  const range = resolveTodoRange(todoState.rangePreset, todoState.customStart, todoState.customEnd);
  const params = Object.assign({}, todoFilterParams(range), {
    page: targetPage,
    pageSize: todoState.pageSize
  });

  try {
    const itemPage = await apiGet('/api/todo/items', params);
    if (reqId !== todoState.requestSeq) { return; }
    todoState.itemPage = itemPage || {rows: [], total: 0, page: targetPage, pageSize: todoState.pageSize};
    todoState.page = Number(todoState.itemPage.page) || targetPage;
    const totalPages = Math.max(1, Math.ceil(Number(todoState.itemPage.total || 0) / todoState.pageSize));
    if (todoState.page > totalPages) {
      todoState.page = totalPages;
      return loadTodoItemsPage(totalPages);
    }
    renderTodoTableSection();
    setTodoError('');
    const nowText = new Date().toLocaleTimeString('en-US', {hour12: false});
    setTodoStatus('clean', 'Updated ' + nowText);
  } catch (error) {
    if (reqId !== todoState.requestSeq) { return; }
    setTodoStatus('error', 'Load failed');
    setTodoError('Failed to load todos: ' + error.message);
    if (isTodoViewActive()) { showToast(error.message, 'error'); }
  } finally {
    if (reqId === todoState.requestSeq) {
      todoState.loading = false;
    }
  }
}

// ===== 汇总卡片 =====

// 首次加载骨架：6 卡片 + 3 图表 + legend + 表格，尺寸与最终内容对齐，避免高度跳变。
function todoSkeletonCard() {
  const card = domEl('div', 'summary-card');
  card.appendChild(domEl('div', 'skeleton skeleton-line title'));
  card.appendChild(domEl('div', 'skeleton skeleton-line value'));
  card.appendChild(domEl('div', 'skeleton skeleton-line sub'));
  return card;
}

function todoSkeletonTable() {
  const wrap = domEl('div', 'skeleton-table');
  const columns = ['w-date', 'w-status', 'w-priority', 'w-text', 'w-group', 'w-num'];
  const row = (head) => {
    const tr = domEl('div', 'skeleton-table-row' + (head ? ' head' : ''));
    for (const column of columns) {
      tr.appendChild(domEl('div', 'skeleton ' + column));
    }
    return tr;
  };
  wrap.appendChild(row(true));
  for (let index = 0; index < 7; index += 1) {
    wrap.appendChild(row(false));
  }
  return wrap;
}

function renderTodoSkeleton() {
  const summary = document.getElementById('todo-summary-cards');
  if (summary) {
    summary.replaceChildren();
    for (let index = 0; index < 6; index += 1) {
      summary.appendChild(todoSkeletonCard());
    }
  }
  for (const id of ['todo-chart-trend', 'todo-chart-status', 'todo-chart-priority']) {
    const host = document.getElementById(id);
    if (host) { host.replaceChildren(domEl('div', 'skeleton chart-skeleton')); }
  }
  const legend = document.getElementById('todo-status-legend');
  if (legend) {
    const wrap = domEl('div', 'skeleton-legend');
    for (let index = 0; index < 3; index += 1) {
      wrap.appendChild(domEl('div', 'skeleton'));
    }
    legend.replaceChildren(wrap);
  }
  const table = document.getElementById('todo-table-container');
  if (table) { table.replaceChildren(todoSkeletonTable()); }
}

// 首次骨架切换为真实数据时统一淡入一次；后续刷新保留旧数据，不再触发。
function todoFadeInInitial() {
  for (const id of [
    'todo-summary-cards',
    'todo-chart-trend',
    'todo-chart-status',
    'todo-status-legend',
    'todo-chart-priority',
    'todo-table-container'
  ]) {
    const host = document.getElementById(id);
    if (host) { host.classList.add('data-enter'); }
  }
}

function todoFormatNumber(value) {
  return Number(value || 0).toLocaleString('en-US');
}

function todoFormatPercent(value, digits) {
  const number = Number(value);
  if (!Number.isFinite(number)) { return '--'; }
  return number.toFixed(digits) + '%';
}

function renderTodoSummary(summary) {
  const host = document.getElementById('todo-summary-cards');
  if (!host) { return; }
  host.replaceChildren();

  const s = summary || {};
  const total = Number(s.total || 0);
  const open = Number(s.open || 0);
  const inProgress = Number(s.inProgress || 0);
  const done = Number(s.done || 0);
  const overdue = Number(s.overdue || 0);
  const completion = total > 0 ? (done / total) * 100 : null;

  const cards = [
    {
      label: 'Total',
      value: todoFormatNumber(total),
      sub: 'in selected range'
    },
    {
      label: 'Open',
      value: todoFormatNumber(open),
      sub: 'not started'
    },
    {
      label: 'In Progress',
      value: todoFormatNumber(inProgress),
      sub: 'being worked on'
    },
    {
      label: 'Done',
      value: todoFormatNumber(done),
      sub: total > 0 ? todoFormatPercent((done / total) * 100, 1) + ' of total' : '-- of total'
    },
    {
      label: 'Completion',
      value: completion === null ? '--' : todoFormatPercent(completion, 1),
      sub: todoFormatNumber(done) + ' of ' + todoFormatNumber(total)
    },
    {
      label: 'Overdue',
      value: todoFormatNumber(overdue),
      sub: 'before today, still open'
    }
  ];

  for (const item of cards) {
    const card = domEl('div', 'summary-card');
    card.appendChild(domEl('div', 'summary-label', item.label));
    const valEl = domEl('div', 'summary-value', item.value);
    valEl.title = String(item.value);
    card.appendChild(valEl);
    const subEl = domEl('div', 'summary-sub', item.sub);
    subEl.title = String(item.sub);
    card.appendChild(subEl);
    host.appendChild(card);
  }
}

// ===== 表格与分页 =====

function todoBuildTable(headers) {
  const table = domEl('table', 'todo-table');
  const thead = domEl('thead');
  const tr = domEl('tr');
  for (const h of headers) {
    tr.appendChild(domEl('th', h.numeric ? 'num' : '', h.label));
  }
  thead.appendChild(tr);
  table.appendChild(thead);
  const tbody = domEl('tbody');
  table.appendChild(tbody);
  return {table: table, tbody: tbody};
}

function todoAppendEmptyRow(tbody, colSpan, message) {
  const tr = domEl('tr');
  const td = domEl('td', 'table-empty', message || 'No todos in the selected range.');
  td.colSpan = colSpan;
  tr.appendChild(td);
  tbody.appendChild(tr);
}

function todoStatusLabel(status) {
  if (status === 'in_progress') { return 'in progress'; }
  if (status === 'done') { return 'done'; }
  return 'todo';
}

function todoPriorityClass(priority) {
  return 'todo-badge todo-priority-' + String(priority || 'P1').toLowerCase();
}

function renderTodosTable(container, itemPage) {
  container.replaceChildren();
  const headers = [
    {label: 'Date'},
    {label: 'Status'},
    {label: 'Priority'},
    {label: 'Text'},
    {label: 'Group'},
    {label: 'Completed'}
  ];
  const {table, tbody} = todoBuildTable(headers);
  const rows = (itemPage && Array.isArray(itemPage.rows)) ? itemPage.rows : [];
  if (rows.length === 0) {
    todoAppendEmptyRow(tbody, headers.length);
    container.appendChild(table);
    return;
  }

  for (const row of rows) {
    const tr = domEl('tr');
    tr.appendChild(domEl('td', 'cell-muted', row.date || '--'));

    const statusTd = domEl('td');
    const status = String(row.status || 'todo');
    statusTd.appendChild(domEl('span', 'todo-badge todo-status-' + status, todoStatusLabel(status)));
    tr.appendChild(statusTd);

    const priorityTd = domEl('td');
    priorityTd.appendChild(domEl('span', todoPriorityClass(row.priority), row.priority || 'P1'));
    tr.appendChild(priorityTd);

    const textTd = domEl('td', 'cell-truncate', row.text || '--');
    textTd.title = String(row.text || '--');
    tr.appendChild(textTd);

    const groupLabel = row.groupLabel || '--';
    const groupTd = domEl('td', 'cell-truncate', groupLabel);
    groupTd.title = String(row.groupId || groupLabel);
    tr.appendChild(groupTd);

    tr.appendChild(domEl('td', 'cell-muted', row.completedDate || '--'));
    tbody.appendChild(tr);
  }
  container.appendChild(table);
}

function renderDailyTable(container, daily) {
  container.replaceChildren();
  const headers = [
    {label: 'Date'},
    {label: 'Planned', numeric: true},
    {label: 'Done', numeric: true},
    {label: 'Completion', numeric: true},
    {label: 'Remaining', numeric: true}
  ];
  const {table, tbody} = todoBuildTable(headers);
  if (!daily.length) {
    todoAppendEmptyRow(tbody, headers.length);
    container.appendChild(table);
    return;
  }
  for (const day of daily) {
    const tr = domEl('tr');
    const planned = Number(day.planned || 0);
    const done = Number(day.done || 0);
    tr.appendChild(domEl('td', 'cell-muted', day.date || '--'));
    tr.appendChild(domEl('td', 'num', todoFormatNumber(planned)));
    tr.appendChild(domEl('td', 'num', todoFormatNumber(done)));
    tr.appendChild(domEl(
      'td',
      'num',
      planned > 0 ? todoFormatPercent((done / planned) * 100, 1) : '--'
    ));
    tr.appendChild(domEl('td', 'num', todoFormatNumber(day.remaining)));
    tbody.appendChild(tr);
  }
  container.appendChild(table);
}

function renderGroupsTable(container, groups) {
  container.replaceChildren();
  const headers = [
    {label: 'Date'},
    {label: 'Group'},
    {label: 'Items', numeric: true},
    {label: 'Top Priority'},
    {label: 'Open', numeric: true},
    {label: 'Doing', numeric: true},
    {label: 'Done', numeric: true}
  ];
  const {table, tbody} = todoBuildTable(headers);
  if (!groups.length) {
    todoAppendEmptyRow(tbody, headers.length);
    container.appendChild(table);
    return;
  }
  for (const group of groups) {
    const tr = domEl('tr');
    tr.appendChild(domEl('td', 'cell-muted', group.date || '--'));
    const labelTd = domEl('td', 'cell-truncate', group.label || '--');
    labelTd.title = String(group.groupId || group.label || '--');
    tr.appendChild(labelTd);

    const priorityTd = domEl('td');
    priorityTd.appendChild(domEl('span', todoPriorityClass(group.topPriority), group.topPriority || 'P1'));
    tr.appendChild(priorityTd);

    tr.appendChild(domEl('td', 'num', todoFormatNumber(group.items)));
    tr.appendChild(domEl('td', 'num', todoFormatNumber(group.open)));
    tr.appendChild(domEl('td', 'num', todoFormatNumber(group.inProgress)));
    tr.appendChild(domEl('td', 'num', todoFormatNumber(group.done)));
    tbody.appendChild(tr);
  }
  container.appendChild(table);
}

function renderTodoPagination(itemPage) {
  const total = Number((itemPage && itemPage.total) || 0);
  const page = Number((itemPage && itemPage.page) || todoState.page || 1);
  const pageSize = Number((itemPage && itemPage.pageSize) || todoState.pageSize || 50);
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const info = document.getElementById('todo-pagination-info');
  const prev = document.getElementById('todo-page-prev');
  const next = document.getElementById('todo-page-next');
  if (info) {
    info.textContent = 'Page ' + page + ' of ' + totalPages + ' · ' + todoFormatNumber(total) + ' todos';
  }
  if (prev) { prev.disabled = page <= 1; }
  if (next) { next.disabled = page >= totalPages; }
}

function renderTodoTableSection() {
  const overview = todoState.overview || {};
  const itemPage = todoState.itemPage || {rows: [], total: 0, page: 1, pageSize: todoState.pageSize};
  const daily = Array.isArray(overview.daily) ? overview.daily : [];
  const groups = Array.isArray(overview.groups) ? overview.groups : [];

  const todosBtn = document.getElementById('tab-btn-todos');
  const dailyBtn = document.getElementById('tab-btn-daily');
  const groupsBtn = document.getElementById('tab-btn-groups');
  if (todosBtn) { todosBtn.textContent = 'Todos (' + todoFormatNumber(itemPage.total || 0) + ')'; }
  if (dailyBtn) { dailyBtn.textContent = 'Daily (' + todoFormatNumber(daily.length) + ')'; }
  if (groupsBtn) { groupsBtn.textContent = 'Groups (' + todoFormatNumber(groups.length) + ')'; }

  for (const [tabKey, btn] of [['todos', todosBtn], ['daily', dailyBtn], ['groups', groupsBtn]]) {
    if (!btn) { continue; }
    const active = todoState.activeTab === tabKey;
    btn.classList.toggle('active', active);
    btn.setAttribute('aria-selected', active ? 'true' : 'false');
  }

  const container = document.getElementById('todo-table-container');
  const pagination = document.getElementById('todo-pagination');
  if (!container) { return; }

  if (todoState.activeTab === 'todos') {
    if (pagination) { pagination.classList.remove('hidden'); }
    renderTodosTable(container, itemPage);
    renderTodoPagination(itemPage);
  } else if (todoState.activeTab === 'daily') {
    if (pagination) { pagination.classList.add('hidden'); }
    renderDailyTable(container, daily);
  } else {
    if (pagination) { pagination.classList.add('hidden'); }
    renderGroupsTable(container, groups);
  }
}

// ===== 初始化与事件绑定 =====

// Home 摘要卡：按条目文本搜索，并放宽到 All 范围，避免历史逾期条目被默认 7d 过滤。
function todoFocusFromHome(text) {
  todoState.search = String(text || '');
  todoState.rangePreset = 'all';
  todoState.status = '';
  todoState.priority = '';
  todoState.activeTab = 'todos';
  todoState.page = 1;
  const searchInput = document.getElementById('todo-search');
  if (searchInput) { searchInput.value = todoState.search; }
  // 旧的状态/优先级筛选会把目标条目藏掉，这里一并清空。
  if (todoSelects.status) { todoSelects.status.setValue(''); }
  if (todoSelects.priority) { todoSelects.priority.setValue(''); }
  for (const btn of document.querySelectorAll('#view-todo .range-btn')) {
    const isCurrent = btn.dataset.range === 'all';
    btn.classList.toggle('active', isCurrent);
    btn.setAttribute('aria-pressed', isCurrent ? 'true' : 'false');
  }
  const customWrap = document.getElementById('todo-custom-range');
  if (customWrap) { customWrap.classList.add('hidden'); }
  // 首次进入由 initOnce 带着当前状态加载；已初始化或首次加载仍在途时主动刷新，
  // 后者由 requestSeq 丢弃旧响应。
  if (window.__webInit && (window.__webInit.todo === 'done' || window.__webInit.todo === 'running')) {
    loadTodoDashboard();
  }
}

function initTodoDashboard() {
  const now = new Date();
  todoState.customStart = todoShiftDate(todoDateKey(now), -6);
  todoState.customEnd = todoDateKey(now);

  const startInput = document.getElementById('todo-custom-start');
  const endInput = document.getElementById('todo-custom-end');
  if (startInput) {
    startInput.value = todoState.customStart;
    startInput.addEventListener('change', () => {
      todoState.customStart = startInput.value;
      todoState.page = 1;
      loadTodoDashboard();
    });
  }
  if (endInput) {
    endInput.value = todoState.customEnd;
    endInput.addEventListener('change', () => {
      todoState.customEnd = endInput.value;
      todoState.page = 1;
      loadTodoDashboard();
    });
  }

  for (const btn of document.querySelectorAll('#view-todo .range-btn')) {
    btn.addEventListener('click', () => {
      const preset = btn.dataset.range || '7d';
      todoState.rangePreset = preset;
      todoState.page = 1;
      for (const other of document.querySelectorAll('#view-todo .range-btn')) {
        const isCurrent = other === btn;
        other.classList.toggle('active', isCurrent);
        other.setAttribute('aria-pressed', isCurrent ? 'true' : 'false');
      }
      const customWrap = document.getElementById('todo-custom-range');
      if (customWrap) {
        customWrap.classList.toggle('hidden', preset !== 'custom');
      }
      loadTodoDashboard();
    });
  }

  todoSelects.status = createSelect(document.getElementById('todo-filter-status'), {
    ariaLabel: 'Filter by status',
    value: '',
    options: [
      {value: '', label: 'All statuses'},
      {value: 'todo', label: 'Todo'},
      {value: 'in_progress', label: 'In Progress'},
      {value: 'done', label: 'Done'}
    ],
    onChange: (value) => {
      todoState.status = value;
      todoState.page = 1;
      loadTodoDashboard();
    }
  });
  todoSelects.priority = createSelect(document.getElementById('todo-filter-priority'), {
    ariaLabel: 'Filter by priority',
    value: '',
    options: [
      {value: '', label: 'All priorities'},
      {value: 'P0', label: 'P0'},
      {value: 'P1', label: 'P1'},
      {value: 'P2', label: 'P2'},
      {value: 'P3', label: 'P3'}
    ],
    onChange: (value) => {
      todoState.priority = value;
      todoState.page = 1;
      loadTodoDashboard();
    }
  });

  const searchInput = document.getElementById('todo-search');
  if (searchInput) {
    searchInput.addEventListener('input', () => {
      todoState.search = searchInput.value;
      if (todoSearchTimer) { clearTimeout(todoSearchTimer); }
      todoSearchTimer = setTimeout(() => {
        todoSearchTimer = null;
        todoState.page = 1;
        loadTodoDashboard();
      }, 250);
    });
  }

  const refreshBtn = document.getElementById('todo-refresh');
  if (refreshBtn) {
    refreshBtn.addEventListener('click', () => loadTodoDashboard());
  }

  for (const tabBtn of document.querySelectorAll('#view-todo .todo-tabs .provider-tab')) {
    tabBtn.addEventListener('click', () => {
      todoState.activeTab = tabBtn.dataset.tab || 'todos';
      renderTodoTableSection();
    });
  }

  const prevBtn = document.getElementById('todo-page-prev');
  const nextBtn = document.getElementById('todo-page-next');
  if (prevBtn) {
    prevBtn.addEventListener('click', () => {
      if (todoState.page > 1) {
        loadTodoItemsPage(todoState.page - 1);
      }
    });
  }
  if (nextBtn) {
    nextBtn.addEventListener('click', () => {
      const total = Number((todoState.itemPage && todoState.itemPage.total) || 0);
      const totalPages = Math.max(1, Math.ceil(total / todoState.pageSize));
      if (todoState.page < totalPages) {
        loadTodoItemsPage(todoState.page + 1);
      }
    });
  }

  renderTodoSkeleton();
  return loadTodoDashboard();
}

function initTodoView() {
  return initTodoDashboard();
}
