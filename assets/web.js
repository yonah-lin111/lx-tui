// ===== 统一 Web 控制台：共享核心（token / DOM / toast / 自绘 select）与视图路由 =====

const token = new URLSearchParams(location.search).get('token') || '';

function domEl(tag, className, text) {
  const node = document.createElement(tag);
  if (className) { node.className = className; }
  if (text !== undefined && text !== null) { node.textContent = String(text); }
  return node;
}

// 操作结果反馈走统一 toast-host；HTML 已提供宿主时直接复用，避免重复创建。
function toastHost() {
  let host = document.getElementById('toast-host');
  if (!host) {
    host = domEl('div', 'toast-host');
    host.id = 'toast-host';
    host.setAttribute('aria-live', 'polite');
    document.body.appendChild(host);
  }
  return host;
}

function showToast(message, kind) {
  const host = toastHost();
  const item = domEl('div', 'toast' + (kind ? ' ' + kind : ''), message);
  host.appendChild(item);
  setTimeout(() => {
    item.classList.add('leaving');
    setTimeout(() => item.remove(), 200);
  }, 3200);
}

// 菜单挂在 body 上用固定定位，避免被滚动容器裁剪；打开中的实例统一关闭。
const openSelects = new Set();

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
  button.setAttribute('aria-expanded', 'false');
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
    const menuWidth = Math.max(rect.width, 140);
    const clampedLeft = Math.max(8, Math.min(rect.left, window.innerWidth - menuWidth - 8));
    menu.style.left = clampedLeft + 'px';
    menu.style.width = menuWidth + 'px';
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
    // Settings 操作条依赖脏状态刷新；Usage 视图无该函数时跳过。
    if (typeof refreshActionBars === 'function') { refreshActionBars(); }
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
window.addEventListener('scroll', () => {
  openSelects.forEach((inst) => inst.reposition());
  if (typeof hideChartTooltip === 'function') { hideChartTooltip(); }
}, true);

// ===== 视图路由 =====

const VIEWS = ['home', 'settings', 'usage', 'todo'];
const VIEW_TITLES = {
  home: 'lx-tui',
  settings: 'lx-tui · Settings',
  usage: 'lx-tui · Usage',
  todo: 'lx-tui · Todo'
};
const VIEW_CSS = { settings: 'css-settings', usage: 'css-usage', todo: 'css-todo' };

function viewFromLocation() {
  if (location.hash === '#' || location.hash === '#/') { return 'home'; }
  const hash = location.hash.replace(/^#\/?/, '');
  if (VIEWS.includes(hash)) { return hash; }
  // 旧地址 /settings、/usage、/todo 直达对应视图。
  const path = location.pathname.replace(/\/+$/, '') || '/';
  if (path === '/settings') { return 'settings'; }
  if (path === '/usage') { return 'usage'; }
  if (path === '/todo') { return 'todo'; }
  return 'home';
}

function showView(view) {
  if (!VIEWS.includes(view)) { view = 'home'; }
  // 视图切换先收起挂在 body 上的浮层，避免它们脱离视图样式表后裸露。
  openSelects.forEach((inst) => inst.close());
  for (const name of VIEWS) {
    const section = document.getElementById('view-' + name);
    if (section) { section.classList.toggle('active', name === view); }
    const tab = document.querySelector('.web-tab[data-view="' + name + '"]');
    if (tab) {
      tab.classList.toggle('active', name === view);
      if (name === view) {
        tab.setAttribute('aria-current', 'page');
      } else {
        tab.removeAttribute('aria-current');
      }
    }
  }
  // 非当前视图的样式表用 media="not all" 挂起，不销毁节点、不重新请求。
  for (const [name, id] of Object.entries(VIEW_CSS)) {
    const link = document.getElementById(id);
    if (link) { link.media = name === view ? '' : 'not all'; }
  }
  if (view === 'settings') { initOnce('settings', window.initSettingsView); }
  if (view === 'usage') { initOnce('usage', window.initUsageView); }
  if (view === 'todo') { initOnce('todo', window.initTodoView); }
  // 离开 Usage 时收起图表 tooltip，避免它脱离视图样式表后裸露在别的视图上。
  if (view !== 'usage' && typeof hideChartTooltip === 'function') { hideChartTooltip(); }
  document.title = VIEW_TITLES[view];
}

// 首次进入才初始化；失败后允许下次重试，避免一次异常把视图永久卡死。
function initOnce(key, fn) {
  window.__webInit = window.__webInit || {};
  if (window.__webInit[key] === 'done' || window.__webInit[key] === 'running') { return; }
  if (typeof fn !== 'function') { return; }
  window.__webInit[key] = 'running';
  try {
    fn();
    window.__webInit[key] = 'done';
  } catch (error) {
    window.__webInit[key] = 'failed';
    showToast('Failed to initialise ' + key, 'error');
  }
}

function bootWeb() {
  const initial = viewFromLocation();
  const path = location.pathname.replace(/\/+$/, '') || '/';
  // 旧地址统一重写为 /?token=...#/<view>，之后切换只改 hash。
  if (path === '/settings' || path === '/usage' || path === '/todo') {
    const hash = initial === 'home' ? '#/' : '#/' + initial;
    history.replaceState(null, '', '/' + location.search + hash);
  }
  // 先接管路由：初始视图初始化失败时，顶栏切页仍然可用并会触发重试。
  window.addEventListener('hashchange', () => showView(viewFromLocation()));
  showView(initial);
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bootWeb);
} else {
  bootWeb();
}
