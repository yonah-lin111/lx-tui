// ===== Home Todo 摘要卡 =====
// 只读：仅请求 /api/todo/home；点击条目跳到 Todo 并按文本搜索。
// 依赖 web.js 的 domEl、usage.js 的 apiGet 与 todo.js 的 todoFocusFromHome。

const homeState = { loading: false, loaded: false, data: null };

function homeFormatDate(value) {
  if (!value || value.length !== 10) { return value || ''; }
  const parts = value.split('-').map((part) => Number.parseInt(part, 10));
  const date = new Date(parts[0], parts[1] - 1, parts[2]);
  if (Number.isNaN(date.getTime())) { return value; }
  return date.toLocaleDateString('en-US', {weekday: 'short', month: 'short', day: 'numeric'});
}

function homeSetNote(text, isError) {
  const note = document.getElementById('home-todo-note');
  if (!note) { return; }
  note.textContent = text || '';
  note.classList.toggle('error', !!isError);
  note.classList.toggle('hidden', !text);
}

function homeRenderStats(summary) {
  const stats = summary || {};
  const today = Number(stats.todayOpen || 0);
  const progress = Number(stats.inProgress || 0);
  const overdue = Number(stats.overdue || 0);
  const todayEl = document.getElementById('home-stat-today');
  const progressEl = document.getElementById('home-stat-progress');
  const overdueEl = document.getElementById('home-stat-overdue');
  if (todayEl) { todayEl.textContent = String(today); }
  if (progressEl) { progressEl.textContent = String(progress); }
  if (overdueEl) { overdueEl.textContent = String(overdue); }
  const block = document.getElementById('home-stat-overdue-block');
  if (block) { block.classList.toggle('alert', overdue > 0); }
}

function homeOpenTodo(item) {
  if (location.hash !== '#/todo') { location.hash = '#/todo'; }
  if (typeof todoFocusFromHome === 'function') { todoFocusFromHome(item.text || ''); }
}

function homeRenderItem(item, today) {
  const row = domEl('button', 'home-todo-row');
  row.type = 'button';
  row.title = item.text || '';
  const priority = String(item.priority || 'P3');
  row.appendChild(domEl('span', 'home-todo-priority ' + priority.toLowerCase(), priority));
  row.appendChild(domEl('span', 'home-todo-text', item.text || ''));
  const overdue = !!item.date && item.date < today;
  const meta = [];
  if (overdue) {
    const sameYear = item.date.slice(0, 4) === today.slice(0, 4);
    meta.push(sameYear ? item.date.slice(5) : item.date, 'overdue');
  } else if (item.status === 'in_progress') {
    meta.push('today', 'in progress');
  } else {
    meta.push('today');
  }
  row.appendChild(domEl('span', 'home-todo-meta' + (overdue ? ' overdue' : ''), meta.join(' · ')));
  row.addEventListener('click', () => homeOpenTodo(item));
  return row;
}

function homeRender(data) {
  const today = (data && data.date) || '';
  const dateEl = document.getElementById('home-todo-date');
  if (dateEl) { dateEl.textContent = homeFormatDate(today); }
  homeRenderStats(data && data.summary);
  const list = document.getElementById('home-todo-list');
  if (!list) { return; }
  list.replaceChildren();
  const items = data && Array.isArray(data.items) ? data.items : [];
  for (const item of items) {
    const li = document.createElement('li');
    li.appendChild(homeRenderItem(item, today));
    list.appendChild(li);
  }
  homeSetNote(items.length ? '' : 'No open todos.', false);
}

// 每次回到 Home 都刷新；首次加载显示行内 Loading…，失败不影响其它入口。
async function loadHomeSummary() {
  if (homeState.loading) { return; }
  homeState.loading = true;
  if (!homeState.loaded) { homeSetNote('Loading…', false); }
  try {
    const data = await apiGet('/api/todo/home');
    homeState.data = data || {};
    homeState.loaded = true;
    homeRender(homeState.data);
  } catch (_error) {
    homeSetNote(homeState.loaded ? 'Could not refresh todos.' : 'Todo unavailable.', true);
  } finally {
    homeState.loading = false;
  }
}
