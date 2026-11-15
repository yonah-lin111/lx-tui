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

  const menu = domEl('div', 'select-menu hidden' + (optionsConfig.menuClass ? ' ' + optionsConfig.menuClass : ''));
  menu.setAttribute('role', 'listbox');
  document.body.appendChild(menu);
  bindSegmentGroup(menu, SELECT_MENU_CONFIG);

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
    menu.querySelectorAll(':scope > .select-option').forEach((row) => row.remove());
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
    syncSegmentPill(menu, SELECT_MENU_CONFIG, false);
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
    syncSegmentPill(menu, SELECT_MENU_CONFIG, true);
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
window.addEventListener('resize', () => {
  openSelects.forEach((inst) => inst.reposition());
  syncSegmentPills(true);
});
window.addEventListener('scroll', () => {
  openSelects.forEach((inst) => inst.reposition());
  if (typeof hideChartTooltip === 'function') { hideChartTooltip(); }
}, true);

// ===== 主题切换：顶部 select 修改外观并持久化到 settings.json =====
// 服务端已在首屏按持久化值注入 <html data-theme>，这里只负责后续切换与跨浏览器同步。
const WEB_THEME_OPTIONS = [
  {value: 'default', label: 'Theme: Default'},
  {value: 'glass', label: 'Theme: Glass'}
];
let webThemeSelect = null;
let webThemeCurrent = document.documentElement.dataset.theme === 'glass' ? 'glass' : 'default';
let webThemeTouched = false;

function applyWebTheme(theme) {
  const next = theme === 'glass' ? 'glass' : 'default';
  webThemeCurrent = next;
  document.documentElement.dataset.theme = next;
  if (webThemeSelect) { webThemeSelect.setValue(next); }
  settleViewTransitions();
  initSegmentPills();
  syncSegmentPills(true);
}

async function loadWebTheme() {
  const search = new URLSearchParams();
  if (token) { search.set('token', token); }
  try {
    // 服务端没有独立的 GET /api/config；`/api/state` 已包含完整的 config。
    const response = await fetch('/api/state?' + search.toString(), {
      headers: {'Accept': 'application/json'}
    });
    if (!response.ok) { return; }
    const data = await response.json();
    const theme = data && data.config ? data.config.web_theme : '';
    // 用户已经手动切过主题时，忽略这次迟到的初始读取，避免把选择回滚。
    if (webThemeTouched) { return; }
    if (theme === 'default' || theme === 'glass') { applyWebTheme(theme); }
  } catch (_error) {
    // 首屏主题已由服务端注入；拉取失败时保持当前外观，不打断页面。
  }
}

async function saveWebTheme(theme) {
  const search = new URLSearchParams();
  if (token) { search.set('token', token); }
  const response = await fetch('/api/config?' + search.toString(), {
    method: 'PUT',
    headers: {'Accept': 'application/json', 'Content-Type': 'application/json'},
    body: JSON.stringify({config: {web_theme: theme}})
  });
  if (response.ok) { return; }
  let message = 'HTTP ' + response.status;
  try {
    const data = await response.json();
    if (data && data.error) { message = data.error; }
  } catch (_error) {
    // 保留状态码文案。
  }
  throw new Error(message);
}

function initWebTheme() {
  const host = document.getElementById('web-theme-select');
  if (!host) { return; }
  webThemeSelect = createSelect(host, {
    ariaLabel: 'Theme',
    value: webThemeCurrent,
    options: WEB_THEME_OPTIONS,
    menuClass: 'web-theme-menu',
    onChange: (value) => {
      const previous = webThemeCurrent;
      webThemeTouched = true;
      applyWebTheme(value);
      saveWebTheme(value).catch((error) => {
        applyWebTheme(previous);
        showToast('Failed to save theme: ' + error.message, 'error');
      });
    }
  });
  loadWebTheme();
}

// ===== 视图路由 =====

const VIEWS = ['home', 'settings', 'usage', 'todo'];
const VIEW_TITLES = {
  home: 'lx-tui',
  settings: 'lx-tui · Settings',
  usage: 'lx-tui · Usage',
  todo: 'lx-tui · Todo'
};
const VIEW_CSS = { settings: 'css-settings', usage: 'css-usage', todo: 'css-todo' };

// ===== 分段指示块：顶栏 tab 与视图内小 tab 共用 hover / 选中逻辑 =====
// 顶栏的 pill 已在 HTML 中；range/provider 分组在启动时注入 .seg-pill。
const SELECT_MENU_CONFIG = {selector: '.select-menu', item: '.select-option', active: '.select-option.active', vertical: true};
const SEGMENT_GROUPS = [
  {selector: '.web-tabs', item: '.web-tab', active: '.web-tab.active'},
  {selector: '.range-group', item: '.range-btn', active: '.range-btn.active'},
  {selector: '.provider-tabs', item: '.provider-tab', active: '.provider-tab.active'},
  {selector: 'nav.side', item: 'button', active: 'button.active', vertical: true},
  {selector: '.command-list', item: '.nav-item', active: '.nav-item.selected', vertical: true, fallback: false},
  {selector: '.provider-list', item: '.nav-item', active: '.nav-item.selected', vertical: true, fallback: false},
  SELECT_MENU_CONFIG
];
const segmentPreview = new WeakMap();
const segmentPillTimers = new WeakMap();
let glassEnterTimer = null;

function segmentItemOf(group, config, target) {
  if (!(target instanceof Element)) { return null; }
  const item = target.closest(config.item);
  return item && item.parentElement === group ? item : null;
}

function ensureSegmentPill(group) {
  let pill = group.querySelector(':scope > .web-tab-pill, :scope > .seg-pill');
  if (!pill) {
    pill = document.createElement('span');
    pill.className = 'seg-pill';
    pill.setAttribute('aria-hidden', 'true');
    group.prepend(pill);
  }
  return pill;
}

// hover / focus 优先，其次才是 active；离开分组后自动回落到选中项。
function segmentTarget(group, config) {
  const preview = segmentPreview.get(group);
  if (preview && preview.hover && preview.hover.isConnected) { return preview.hover; }
  if (preview && preview.focus && preview.focus.isConnected) { return preview.focus; }
  const active = group.querySelector(config.active);
  if (active) { return active; }
  return config.fallback === false ? null : group.querySelector(config.item);
}

function syncSegmentPill(group, config, immediate) {
  const pill = ensureSegmentPill(group);
  const target = segmentTarget(group, config);
  if (!target) {
    pill.style.opacity = '0';
    delete pill.dataset.segmentSized;
    return;
  }
  const groupRect = group.getBoundingClientRect();
  const rect = target.getBoundingClientRect();
  if (!rect.width || !groupRect.width) {
    // 隐藏视图量不到布局：等可见后再直接落位，避免从 0 宽度滑入。
    pill.style.opacity = '0';
    delete pill.dataset.segmentSized;
    return;
  }
  const instant = immediate || !pill.dataset.segmentSized;
  if (instant) {
    pill.classList.add('no-transition');
    const timer = segmentPillTimers.get(pill);
    if (timer) { clearTimeout(timer); }
    segmentPillTimers.set(pill, setTimeout(() => {
      pill.classList.remove('no-transition');
      segmentPillTimers.delete(pill);
    }, 24));
  }
  pill.dataset.segmentSized = '1';
  pill.style.opacity = '1';
  if (config.vertical) {
    // offsetTop/Height 不受父级 scale 动画影响，菜单/列表入场时也能量到真实位置。
    pill.style.height = target.offsetHeight + 'px';
    pill.style.transform = 'translate3d(0, ' + target.offsetTop + 'px, 0)';
  } else {
    const x = rect.left - groupRect.left - group.clientLeft + group.scrollLeft;
    pill.style.width = rect.width + 'px';
    pill.style.transform = 'translate3d(' + x + 'px, 0, 0)';
  }
}

function setSegmentPreview(group, config, key, item) {
  const state = segmentPreview.get(group) || {hover: null, focus: null};
  if (state[key] === item) { return; }
  state[key] = item;
  segmentPreview.set(group, state);
  syncSegmentPill(group, config, false);
}

function clearSegmentPreview(group, config, key) {
  const state = segmentPreview.get(group);
  if (!state || !state[key]) { return; }
  state[key] = null;
  syncSegmentPill(group, config, false);
}

function bindSegmentGroup(group, config) {
  if (group.dataset.segmentBound === '1') { return; }
  group.dataset.segmentBound = '1';
  ensureSegmentPill(group);
  group.addEventListener('pointerover', (event) => {
    // 悬停分组内的标题/空白同样是一次预览变更：没有条目就清掉，
    // 否则指示块会停在上一个条目上，看起来像一直选中。
    setSegmentPreview(group, config, 'hover', segmentItemOf(group, config, event.target));
  });
  group.addEventListener('pointerleave', () => clearSegmentPreview(group, config, 'hover'));
  group.addEventListener('focusin', (event) => {
    const item = segmentItemOf(group, config, event.target);
    if (item) { setSegmentPreview(group, config, 'focus', item); }
  });
  group.addEventListener('focusout', (event) => {
    if (event.relatedTarget && group.contains(event.relatedTarget)) { return; }
    clearSegmentPreview(group, config, 'focus');
  });
  // active 类由各视图脚本切换；childList 负责列表/下拉选项整批重绘后的重新挂载。
  new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === 'childList') {
        syncSegmentPill(group, config, false);
        return;
      }
      if (record.target instanceof Element && record.target.matches(config.item)) {
        syncSegmentPill(group, config, false);
        return;
      }
    }
  }).observe(group, {subtree: true, childList: true, attributes: true, attributeFilter: ['class']});
  // Settings 的 provider tabs 位于懒加载子视图里：从 display:none 变为可见时
  // 分组尺寸才会出现，靠 ResizeObserver 补一次直接落位。
  if (typeof ResizeObserver === 'function') {
    new ResizeObserver(() => syncSegmentPill(group, config, false)).observe(group);
  }
}

function initSegmentPills() {
  for (const config of SEGMENT_GROUPS) {
    for (const group of document.querySelectorAll(config.selector)) {
      bindSegmentGroup(group, config);
    }
  }
}

function syncSegmentPills(immediate) {
  for (const config of SEGMENT_GROUPS) {
    for (const group of document.querySelectorAll(config.selector)) {
      syncSegmentPill(group, config, immediate);
    }
  }
}

// 切换主题/缩放时清掉残留的进场类，避免旧视图被固定在动画终态。
function settleViewTransitions() {
  if (glassEnterTimer) {
    clearTimeout(glassEnterTimer);
    glassEnterTimer = null;
  }
  for (const name of VIEWS) {
    const section = document.getElementById('view-' + name);
    if (!section) { continue; }
    section.onanimationend = null;
    section.classList.remove('glass-enter');
  }
}

// ===== 全局加载遮罩：只盖内容区，顶部导航始终可点 =====
const pageLoadingState = { seq: 0, startedAt: 0, hideTimer: null };

function resetPageLoading() {
  pageLoadingState.seq += 1;
  if (pageLoadingState.hideTimer) {
    clearTimeout(pageLoadingState.hideTimer);
    pageLoadingState.hideTimer = null;
  }
  const node = document.getElementById('page-loading');
  if (node) {
    node.classList.add('hidden');
    node.classList.remove('leaving');
  }
}

function showPageLoading(label) {
  const node = document.getElementById('page-loading');
  if (!node) { return 0; }
  if (pageLoadingState.hideTimer) {
    clearTimeout(pageLoadingState.hideTimer);
    pageLoadingState.hideTimer = null;
  }
  const labelNode = document.getElementById('page-loading-label');
  if (labelNode) { labelNode.textContent = label || 'Loading…'; }
  node.classList.remove('hidden', 'leaving');
  pageLoadingState.seq += 1;
  pageLoadingState.startedAt = Date.now();
  return pageLoadingState.seq;
}

// 至少展示 180ms，避免快请求闪一下反而像故障；过期请求不得关闭新遮罩。
function hidePageLoading(seq) {
  const node = document.getElementById('page-loading');
  if (!node || (seq && seq !== pageLoadingState.seq)) { return; }
  const wait = Math.max(0, 180 - (Date.now() - pageLoadingState.startedAt));
  if (pageLoadingState.hideTimer) { clearTimeout(pageLoadingState.hideTimer); }
  pageLoadingState.hideTimer = setTimeout(() => {
    pageLoadingState.hideTimer = null;
    if (seq !== pageLoadingState.seq) { return; }
    node.classList.add('leaving');
    setTimeout(() => {
      if (seq !== pageLoadingState.seq) { return; }
      node.classList.add('hidden');
      node.classList.remove('leaving');
    }, 220);
  }, wait);
}

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
  // 切页先收起上一页尚未结束的加载遮罩与浮层，避免遮罩归属错乱。
  resetPageLoading();
  openSelects.forEach((inst) => inst.close());
  const current = VIEWS.find((name) => {
    const section = document.getElementById('view-' + name);
    return section && section.classList.contains('active');
  });
  const reduceMotion = window.matchMedia
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const animateGlass = document.documentElement.dataset.theme === 'glass'
    && current && current !== view && !reduceMotion;
  settleViewTransitions();
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
  if (animateGlass) {
    const entering = document.getElementById('view-' + view);
    if (entering) {
      const done = () => {
        entering.classList.remove('glass-enter');
        entering.onanimationend = null;
        glassEnterTimer = null;
      };
      entering.classList.add('glass-enter');
      entering.onanimationend = (event) => {
        if (event.target !== entering) { return; }
        done();
      };
      // 后台标签页可能不派发 animationend；兜底清理，避免 will-change 长期驻留。
      glassEnterTimer = setTimeout(done, 260);
    }
  }
  // 非当前视图的样式表用 media="not all" 挂起，不销毁节点、不重新请求。
  for (const [name, id] of Object.entries(VIEW_CSS)) {
    const link = document.getElementById(id);
    if (link) { link.media = name === view ? '' : 'not all'; }
  }
  // 等目标视图的样式表恢复后再量指示块，避免按无样式布局量出错误宽度。
  syncSegmentPills(false);
  if (view === 'settings') { initOnce('settings', 'Loading settings…', window.initSettingsView); }
  if (view === 'usage') { initOnce('usage', 'Loading usage…', window.initUsageView); }
  if (view === 'todo') { initOnce('todo', 'Loading todos…', window.initTodoView); }
  // 离开 Usage 时收起图表 tooltip，避免它脱离视图样式表后裸露在别的视图上。
  if (view !== 'usage' && typeof hideChartTooltip === 'function') { hideChartTooltip(); }
  document.title = VIEW_TITLES[view];
}

// 首次进入才初始化；失败后允许下次重试，避免一次异常把视图永久卡死。
// 初始化函数返回 Promise 时，全局遮罩持续到它 settle，保证懒加载视图不闪空容器。
function initOnce(key, label, fn) {
  window.__webInit = window.__webInit || {};
  if (window.__webInit[key] === 'done' || window.__webInit[key] === 'running') { return; }
  if (typeof fn !== 'function') { return; }
  window.__webInit[key] = 'running';
  let result;
  try {
    result = fn();
  } catch (error) {
    window.__webInit[key] = 'failed';
    showToast('Failed to initialise ' + key, 'error');
    return;
  }
  if (result && typeof result.then === 'function') {
    const seq = showPageLoading(label);
    result.then(
      () => { window.__webInit[key] = 'done'; },
      () => { window.__webInit[key] = 'failed'; }
    ).finally(() => hidePageLoading(seq));
    return;
  }
  window.__webInit[key] = 'done';
}

function bootWeb() {
  const initial = viewFromLocation();
  initSegmentPills();
  initWebTheme();
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
