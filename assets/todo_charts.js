// ===== Todo Dashboard: Native SVG / DOM Charts =====
// 纯原生图表渲染：每日计划/完成趋势、状态环形图、优先级横条。
// 顶层名字统一 `todo` / `TODO_` 前缀；本文件先于 todo.js 加载，由 renderTodoCharts 调用。

const TODO_CHART_COLORS = {
  planned: '#60a5fa',
  done: '#3ecf8e',
  open: '#9a9a9a',
  inProgress: '#60a5fa',
  grid: '#262626',
  axis: '#9a9a9a',
  p0: '#ef6b6b',
  p1: '#fbbf24',
  p2: '#94a3b8',
  p3: '#64748b'
};

const TODO_CHART_MONTHS = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'
];

const TODO_CHART_DAY_MS = 86400000;
const TODO_CHART_DAILY_MAX_DAYS = 62;
const TODO_CHART_WEEKLY_MAX_DAYS = 400;
const TODO_CHART_TOOLTIP_CLASS = 'todo-chart-tooltip';
const TODO_CHART_SVG_NS = ['http', '://www.w3.org/2000/svg'].join('');

let todoChartTooltipNode = null;
let todoChartTooltipBound = false;

// ===== DOM / SVG 基础 =====

function todoSvgEl(tag, attrs, text) {
  const node = document.createElementNS(TODO_CHART_SVG_NS, tag);
  if (attrs) {
    for (const [key, value] of Object.entries(attrs)) {
      if (value !== undefined && value !== null) {
        node.setAttribute(key, String(value));
      }
    }
  }
  if (text !== undefined && text !== null) {
    node.textContent = String(text);
  }
  return node;
}

function todoChartNumber(value) {
  const num = Number(value);
  if (!Number.isFinite(num)) { return '0'; }
  return Math.round(num).toLocaleString('en-US');
}

function todoChartPercent(value, digits) {
  if (value === null || value === undefined || !Number.isFinite(Number(value))) {
    return '--';
  }
  const places = digits === undefined ? 1 : digits;
  return Number(value).toFixed(places) + '%';
}

function todoChartToCount(value) {
  const num = Number(value);
  return Number.isFinite(num) && num > 0 ? num : 0;
}

// ===== 本地日期工具（禁止 new Date('YYYY-MM-DD')，避免 UTC 偏移） =====

function todoChartParseDateKey(key) {
  const text = String(key || '');
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
  if (!match) { return null; }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day);
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  ) {
    return null;
  }
  return date;
}

function todoChartDateKey(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return year + '-' + month + '-' + day;
}

function todoChartAddDays(date, days) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

function todoChartDaySpan(startDate, endDate) {
  const start = Date.UTC(startDate.getFullYear(), startDate.getMonth(), startDate.getDate());
  const end = Date.UTC(endDate.getFullYear(), endDate.getMonth(), endDate.getDate());
  return Math.floor((end - start) / TODO_CHART_DAY_MS) + 1;
}

function todoChartMonday(date) {
  return todoChartAddDays(date, -((date.getDay() + 6) % 7));
}

// ===== 桶标签与粒度 =====

function todoChartMonthDay(date) {
  return TODO_CHART_MONTHS[date.getMonth()] + ' ' + date.getDate();
}

function todoChartLongDate(date) {
  return TODO_CHART_MONTHS[date.getMonth()] + ' ' + date.getDate() + ', ' + date.getFullYear();
}

function todoChartBucketLabel(bucketStart, granularity) {
  if (granularity === 'monthly') {
    return TODO_CHART_MONTHS[bucketStart.getMonth()] + ' ' + bucketStart.getFullYear();
  }
  if (granularity === 'weekly') {
    const bucketEnd = todoChartAddDays(bucketStart, 6);
    if (bucketStart.getFullYear() !== bucketEnd.getFullYear()) {
      return todoChartMonthDay(bucketStart) + ', ' + bucketStart.getFullYear() +
        ' – ' + todoChartMonthDay(bucketEnd) + ', ' + bucketEnd.getFullYear();
    }
    if (bucketStart.getMonth() !== bucketEnd.getMonth()) {
      return todoChartMonthDay(bucketStart) + ' – ' + todoChartMonthDay(bucketEnd);
    }
    return todoChartMonthDay(bucketStart) + ' – ' + bucketEnd.getDate();
  }
  return todoChartMonthDay(bucketStart);
}

function todoChartGranularity(spanDays) {
  if (spanDays <= TODO_CHART_DAILY_MAX_DAYS) { return 'daily'; }
  if (spanDays <= TODO_CHART_WEEKLY_MAX_DAYS) { return 'weekly'; }
  return 'monthly';
}

// 按粒度补齐连续桶：daily 逐日、weekly 周一对齐、monthly 自然月。
function todoChartBuildBuckets(dailyRows, startDate, endDate, granularity) {
  const buckets = [];
  const bucketMap = new Map();

  const keyForDate = (date) => {
    if (granularity === 'monthly') {
      return date.getFullYear() + '-' + String(date.getMonth() + 1).padStart(2, '0');
    }
    if (granularity === 'weekly') {
      return todoChartDateKey(todoChartMonday(date));
    }
    return todoChartDateKey(date);
  };

  const bucketForDate = (date) => {
    const key = keyForDate(date);
    let bucket = bucketMap.get(key);
    if (bucket) { return bucket; }
    let bucketStart = date;
    if (granularity === 'weekly') {
      bucketStart = todoChartMonday(date);
    } else if (granularity === 'monthly') {
      bucketStart = new Date(date.getFullYear(), date.getMonth(), 1);
    }
    bucket = {
      label: todoChartBucketLabel(bucketStart, granularity),
      planned: 0,
      done: 0
    };
    bucketMap.set(key, bucket);
    buckets.push(bucket);
    return bucket;
  };

  for (let cursor = startDate; cursor <= endDate; cursor = todoChartAddDays(cursor, 1)) {
    bucketForDate(cursor);
  }

  for (const row of dailyRows) {
    const rowDate = todoChartParseDateKey(row && row.date);
    if (!rowDate || rowDate < startDate || rowDate > endDate) { continue; }
    const bucket = bucketForDate(rowDate);
    bucket.planned += todoChartToCount(row.planned);
    bucket.done += todoChartToCount(row.done);
  }

  return buckets;
}

// ===== 悬浮 Tooltip =====

function todoChartEnsureTooltip() {
  if (todoChartTooltipNode && todoChartTooltipNode.isConnected) {
    return todoChartTooltipNode;
  }
  todoChartTooltipNode = document.querySelector('.' + TODO_CHART_TOOLTIP_CLASS);
  if (!todoChartTooltipNode) {
    todoChartTooltipNode = document.createElement('div');
    todoChartTooltipNode.className = TODO_CHART_TOOLTIP_CLASS;
    todoChartTooltipNode.setAttribute('role', 'tooltip');
    todoChartTooltipNode.hidden = true;
    document.body.appendChild(todoChartTooltipNode);
  }
  if (!todoChartTooltipBound) {
    const view = document.getElementById('view-todo');
    if (view) {
      todoChartTooltipBound = true;
      view.addEventListener('mouseleave', todoChartHideTooltip);
      // scroll 不冒泡但可捕获，覆盖 .todo-main 内部的滚动。
      view.addEventListener('scroll', todoChartHideTooltip, true);
    }
  }
  return todoChartTooltipNode;
}

function todoChartPositionTooltip(evt) {
  const tip = todoChartTooltipNode;
  if (!tip || tip.hidden) { return; }
  const rect = tip.getBoundingClientRect();
  const offset = 14;
  const pointerX = Number(evt.clientX || 0);
  const pointerY = Number(evt.clientY || 0);
  let x = pointerX + offset;
  let y = pointerY + offset;
  if (x + rect.width + 12 > window.innerWidth) {
    x = pointerX - rect.width - offset;
  }
  if (y + rect.height + 12 > window.innerHeight) {
    y = pointerY - rect.height - offset;
  }
  x = Math.min(Math.max(8, x), Math.max(8, window.innerWidth - rect.width - 8));
  y = Math.min(Math.max(8, y), Math.max(8, window.innerHeight - rect.height - 8));
  tip.style.left = x + 'px';
  tip.style.top = y + 'px';
}

function todoChartShowTooltip(evt, title, rows) {
  const tip = todoChartEnsureTooltip();
  tip.replaceChildren();
  tip.appendChild(domEl('div', 'todo-tooltip-title', title));
  for (const row of rows || []) {
    const line = domEl('div', 'todo-tooltip-row');
    const left = domEl('span', 'todo-tooltip-label');
    if (row.color) {
      const dot = domEl('span', 'todo-tooltip-dot');
      dot.style.background = row.color;
      left.appendChild(dot);
    }
    left.appendChild(domEl('span', null, row.label));
    line.appendChild(left);
    line.appendChild(domEl('span', 'todo-tooltip-value', row.value));
    tip.appendChild(line);
  }
  tip.hidden = false;
  todoChartPositionTooltip(evt);
}

function todoChartHideTooltip() {
  if (todoChartTooltipNode) {
    todoChartTooltipNode.hidden = true;
  }
}

function todoChartRenderEmpty(host, message) {
  todoChartHideTooltip();
  host.replaceChildren();
  host.appendChild(domEl('div', 'chart-empty', message || 'No todos in the selected range.'));
}

// ===== 趋势柱状图：Planned vs Done =====

function todoChartRenderTrend(host, buckets, granularity, startDate, endDate) {
  const list = Array.isArray(buckets) ? buckets : [];
  if (!list.length) {
    todoChartRenderEmpty(host, 'No todos in the selected range.');
    return;
  }
  host.replaceChildren();

  const W = 920;
  const H = 260;
  const padL = 44;
  const padR = 16;
  const padT = 16;
  const padB = 30;
  const plotW = W - padL - padR;
  const plotH = H - padT - padB;
  const n = list.length;
  const slotW = plotW / n;

  const rawMax = Math.max(1, ...list.map((bucket) => Math.max(bucket.planned, bucket.done)));
  const tickStep = Math.max(1, Math.ceil(rawMax / 4));
  const maxValue = tickStep * 4;
  const noun = granularity === 'weekly' ? 'week' : granularity === 'monthly' ? 'month' : 'day';

  const svg = todoSvgEl('svg', {
    viewBox: '0 0 ' + W + ' ' + H,
    class: 'chart-svg',
    role: 'img',
    'aria-label': 'Planned vs done todos per ' + noun + ' from ' +
      todoChartLongDate(startDate) + ' to ' + todoChartLongDate(endDate)
  });

  const ticks = 4;
  for (let i = 0; i <= ticks; i += 1) {
    const ratio = i / ticks;
    const y = padT + plotH - ratio * plotH;
    svg.appendChild(todoSvgEl('line', {
      x1: padL,
      y1: y,
      x2: W - padR,
      y2: y,
      stroke: TODO_CHART_COLORS.grid,
      'stroke-dasharray': i === 0 ? undefined : '3 3'
    }));
    svg.appendChild(todoSvgEl('text', {
      x: padL - 8,
      y: y + 4,
      fill: TODO_CHART_COLORS.axis,
      'font-size': '11',
      'text-anchor': 'end'
    }, todoChartNumber(tickStep * i)));
  }

  const labelStep = Math.max(1, Math.ceil(n / 8));
  for (let i = 0; i < n; i += labelStep) {
    svg.appendChild(todoSvgEl('text', {
      x: padL + (i + 0.5) * slotW,
      y: H - 8,
      fill: TODO_CHART_COLORS.axis,
      'font-size': '11',
      'text-anchor': 'middle'
    }, list[i].label));
  }

  const barW = Math.max(2, Math.min(16, slotW * 0.3));
  const gap = Math.max(1, Math.min(4, slotW * 0.08));
  const groupW = barW * 2 + gap;

  for (let i = 0; i < n; i += 1) {
    const bucket = list[i];
    const cx = padL + (i + 0.5) * slotW;
    const groupLeft = cx - groupW / 2;
    const plannedH = (bucket.planned / maxValue) * plotH;
    const doneH = (bucket.done / maxValue) * plotH;

    const plannedBar = todoSvgEl('rect', {
      x: groupLeft.toFixed(1),
      y: (padT + plotH - plannedH).toFixed(1),
      width: barW.toFixed(1),
      height: Math.max(0, plannedH).toFixed(1),
      fill: TODO_CHART_COLORS.planned
    });
    const doneBar = todoSvgEl('rect', {
      x: (groupLeft + barW + gap).toFixed(1),
      y: (padT + plotH - doneH).toFixed(1),
      width: barW.toFixed(1),
      height: Math.max(0, doneH).toFixed(1),
      fill: TODO_CHART_COLORS.done
    });
    const hit = todoSvgEl('rect', {
      x: (padL + i * slotW).toFixed(1),
      y: padT,
      width: slotW.toFixed(1),
      height: plotH,
      fill: 'transparent',
      style: 'cursor: crosshair'
    });

    hit.addEventListener('mousemove', (evt) => {
      plannedBar.setAttribute('opacity', '0.82');
      doneBar.setAttribute('opacity', '0.82');
      todoChartShowTooltip(evt, bucket.label, [
        {label: 'Planned', value: todoChartNumber(bucket.planned), color: TODO_CHART_COLORS.planned},
        {label: 'Done', value: todoChartNumber(bucket.done), color: TODO_CHART_COLORS.done},
        {label: 'Remaining', value: todoChartNumber(bucket.planned - bucket.done)}
      ]);
    });
    hit.addEventListener('mouseleave', () => {
      plannedBar.removeAttribute('opacity');
      doneBar.removeAttribute('opacity');
      todoChartHideTooltip();
    });

    svg.appendChild(plannedBar);
    svg.appendChild(doneBar);
    svg.appendChild(hit);
  }

  host.appendChild(svg);
}

// ===== 环形图弧线 =====

function todoChartDonutArc(cx, cy, rOuter, rInner, startAngle, endAngle) {
  const span = Math.min(Math.PI * 2 - 0.0001, Math.max(0.0001, endAngle - startAngle));
  const end = startAngle + span;
  const largeArc = span > Math.PI ? 1 : 0;
  const x1 = cx + rOuter * Math.cos(startAngle);
  const y1 = cy + rOuter * Math.sin(startAngle);
  const x2 = cx + rOuter * Math.cos(end);
  const y2 = cy + rOuter * Math.sin(end);
  const x3 = cx + rInner * Math.cos(end);
  const y3 = cy + rInner * Math.sin(end);
  const x4 = cx + rInner * Math.cos(startAngle);
  const y4 = cy + rInner * Math.sin(startAngle);
  return [
    'M', x1.toFixed(2), y1.toFixed(2),
    'A', rOuter, rOuter, 0, largeArc, 1, x2.toFixed(2), y2.toFixed(2),
    'L', x3.toFixed(2), y3.toFixed(2),
    'A', rInner, rInner, 0, largeArc, 0, x4.toFixed(2), y4.toFixed(2),
    'Z'
  ].join(' ');
}

// ===== 状态环形图 =====

function todoChartRenderStatus(summary) {
  const host = document.getElementById('todo-chart-status');
  const legendHost = document.getElementById('todo-status-legend');
  if (!host) { return; }

  const source = summary || {};
  const open = todoChartToCount(source.open);
  const inProgress = todoChartToCount(source.inProgress);
  const done = todoChartToCount(source.done);
  const total = open + inProgress + done;
  if (legendHost) { legendHost.replaceChildren(); }

  if (total <= 0) {
    todoChartRenderEmpty(host, 'No todos in the selected range.');
    return;
  }

  host.replaceChildren();
  const slices = [
    {name: 'Open', value: open, color: TODO_CHART_COLORS.open},
    {name: 'In Progress', value: inProgress, color: TODO_CHART_COLORS.inProgress},
    {name: 'Done', value: done, color: TODO_CHART_COLORS.done}
  ];
  const completion = (done / total) * 100;

  const size = 180;
  const cx = size / 2;
  const cy = size / 2;
  const rOuter = 74;
  const rInner = 46;
  const svg = todoSvgEl('svg', {
    viewBox: '0 0 ' + size + ' ' + size,
    class: 'chart-svg',
    role: 'img',
    'aria-label': 'Status distribution: ' +
      todoChartNumber(open) + ' open, ' +
      todoChartNumber(inProgress) + ' in progress, ' +
      todoChartNumber(done) + ' done'
  });

  let angle = -Math.PI / 2;
  const nonZero = slices.filter((slice) => slice.value > 0);
  for (const slice of nonZero) {
    const sweep = (slice.value / total) * Math.PI * 2;
    const gap = nonZero.length > 1 ? Math.min(0.025, sweep * 0.1) : 0;
    const path = todoSvgEl('path', {
      d: todoChartDonutArc(cx, cy, rOuter, rInner, angle + gap / 2, angle + sweep - gap / 2),
      fill: slice.color
    });
    path.addEventListener('mousemove', (evt) => {
      path.setAttribute('opacity', '0.82');
      todoChartShowTooltip(evt, slice.name, [
        {label: 'Todos', value: todoChartNumber(slice.value), color: slice.color},
        {label: 'Share', value: todoChartPercent((slice.value / total) * 100, 1)}
      ]);
    });
    path.addEventListener('mouseleave', () => {
      path.removeAttribute('opacity');
      todoChartHideTooltip();
    });
    svg.appendChild(path);
    angle += sweep;
  }

  svg.appendChild(todoSvgEl('text', {
    x: cx,
    y: cy - 2,
    fill: '#e8e8e8',
    'font-size': '15',
    'font-weight': '600',
    'text-anchor': 'middle'
  }, todoChartPercent(completion, 1)));
  svg.appendChild(todoSvgEl('text', {
    x: cx,
    y: cy + 15,
    fill: TODO_CHART_COLORS.axis,
    'font-size': '10',
    'text-anchor': 'middle'
  }, 'complete'));

  const wrap = domEl('div', 'todo-donut-svg-wrap');
  wrap.appendChild(svg);
  host.appendChild(wrap);

  if (!legendHost) { return; }
  for (const slice of slices) {
    const share = (slice.value / total) * 100;
    const row = domEl('div', 'donut-legend-row');
    row.setAttribute(
      'aria-label',
      slice.name + ': ' + todoChartNumber(slice.value) + ' todos, ' + todoChartPercent(share, 1)
    );
    const swatch = domEl('span', 'legend-swatch');
    swatch.style.background = slice.color;
    const name = domEl('span', 'donut-legend-name', slice.name);
    name.title = slice.name;
    const value = domEl(
      'span',
      'donut-legend-val',
      todoChartNumber(slice.value) + ' · ' + todoChartPercent(share, 1)
    );
    row.appendChild(swatch);
    row.appendChild(name);
    row.appendChild(value);
    row.addEventListener('mousemove', (evt) => {
      todoChartShowTooltip(evt, slice.name, [
        {label: 'Todos', value: todoChartNumber(slice.value), color: slice.color},
        {label: 'Share', value: todoChartPercent(share, 1)}
      ]);
    });
    row.addEventListener('mouseleave', todoChartHideTooltip);
    legendHost.appendChild(row);
  }
}

// ===== 优先级横条：P0–P3 =====

function todoChartRenderPriority(priority) {
  const host = document.getElementById('todo-chart-priority');
  if (!host) { return; }

  const source = priority || {};
  const rows = [
    {label: 'P0', value: todoChartToCount(source.p0), color: TODO_CHART_COLORS.p0},
    {label: 'P1', value: todoChartToCount(source.p1), color: TODO_CHART_COLORS.p1},
    {label: 'P2', value: todoChartToCount(source.p2), color: TODO_CHART_COLORS.p2},
    {label: 'P3', value: todoChartToCount(source.p3), color: TODO_CHART_COLORS.p3}
  ];
  const total = rows.reduce((sum, row) => sum + row.value, 0);

  host.replaceChildren();
  const list = domEl('div', 'todo-priority-bars');
  for (const row of rows) {
    const share = total > 0 ? (row.value / total) * 100 : 0;
    const item = domEl('div', 'todo-priority-row');
    item.setAttribute(
      'aria-label',
      row.label + ': ' + todoChartNumber(row.value) + ' todos, ' + todoChartPercent(share, 1)
    );
    const label = domEl('span', 'todo-priority-label', row.label);
    label.style.color = row.color;
    const track = domEl('span', 'todo-priority-track');
    const fill = domEl('span', 'todo-priority-fill');
    fill.style.width = Math.max(0, Math.min(100, share)) + '%';
    fill.style.background = row.color;
    track.appendChild(fill);
    const value = domEl(
      'span',
      'todo-priority-value',
      todoChartNumber(row.value) + ' · ' + todoChartPercent(share, 1)
    );
    item.appendChild(label);
    item.appendChild(track);
    item.appendChild(value);
    item.addEventListener('mousemove', (evt) => {
      todoChartShowTooltip(evt, row.label, [
        {label: 'Todos', value: todoChartNumber(row.value), color: row.color},
        {label: 'Share', value: todoChartPercent(share, 1)}
      ]);
    });
    item.addEventListener('mouseleave', todoChartHideTooltip);
    list.appendChild(item);
  }
  host.appendChild(list);
}

// ===== 入口：随筛选结果重绘三张图 =====

function renderTodoCharts(overview, range) {
  const data = overview || {};
  const rangeInfo = range || {};
  todoChartHideTooltip();

  const dailyRows = (Array.isArray(data.daily) ? data.daily : []).filter(
    (row) => row && todoChartParseDateKey(row.date)
  );

  let earliest = null;
  let latest = null;
  for (const row of dailyRows) {
    const date = todoChartParseDateKey(row.date);
    if (!earliest || date < earliest) { earliest = date; }
    if (!latest || date > latest) { latest = date; }
  }

  let startDate = todoChartParseDateKey(rangeInfo.startDate) || earliest;
  let endDate = todoChartParseDateKey(rangeInfo.endDate) || latest;
  if (startDate && endDate && startDate > endDate) {
    const swap = startDate;
    startDate = endDate;
    endDate = swap;
  }

  let granularity = 'daily';
  if (startDate && endDate) {
    granularity = todoChartGranularity(todoChartDaySpan(startDate, endDate));
  }

  const subtitle = document.getElementById('todo-trend-subtitle');
  if (subtitle) {
    subtitle.textContent = 'Planned vs done · ' + granularity;
  }

  const trendHost = document.getElementById('todo-chart-trend');
  if (trendHost) {
    if (!dailyRows.length || !startDate || !endDate) {
      todoChartRenderEmpty(trendHost, 'No todos in the selected range.');
    } else {
      const buckets = todoChartBuildBuckets(dailyRows, startDate, endDate, granularity);
      todoChartRenderTrend(trendHost, buckets, granularity, startDate, endDate);
    }
  }

  todoChartRenderStatus(data.summary || {});
  todoChartRenderPriority(data.priority || {});
}
