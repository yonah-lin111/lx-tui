// ===== Usage Dashboard: Native SVG Charts & Formatters =====
// 纯原生 SVG 图表渲染与格式化工具；零外部依赖，零 DOM 字符串注入。

const SVG_NS = ['http', '://www.w3.org/2000/svg'].join('');

const CHART_COLORS = {
  freshInput: '#60a5fa',
  cacheRead: '#a78bfa',
  cacheWrite: '#fbbf24',
  output: '#3ecf8e',
  cost: '#f472b6',
  requests: '#38bdf8',
  grid: '#262626',
  axis: '#9a9a9a',
  crosshair: '#4a4a4a'
};

const PROVIDER_PALETTE = [
  '#60a5fa', '#3ecf8e', '#a78bfa', '#fbbf24',
  '#f472b6', '#38bdf8', '#fb923c', '#94a3b8'
];

function domEl(tag, className, text) {
  const node = document.createElement(tag);
  if (className) { node.className = className; }
  if (text !== undefined && text !== null) { node.textContent = String(text); }
  return node;
}

function svgEl(tag, attrs, text) {
  const node = document.createElementNS(SVG_NS, tag);
  if (attrs) {
    for (const [key, val] of Object.entries(attrs)) {
      if (val !== undefined && val !== null) {
        node.setAttribute(key, String(val));
      }
    }
  }
  if (text !== undefined && text !== null) {
    node.textContent = String(text);
  }
  return node;
}

// ===== 数值与时间格式化工具 =====

function formatNumber(value) {
  const num = Number(value);
  if (!Number.isFinite(num)) { return '0'; }
  return Math.round(num).toLocaleString('en-US');
}

function formatCompact(value) {
  const num = Number(value);
  if (!Number.isFinite(num)) { return '0'; }
  const abs = Math.abs(num);
  if (abs >= 1e9) { return (num / 1e9).toFixed(1) + 'B'; }
  if (abs >= 1e6) { return (num / 1e6).toFixed(1) + 'M'; }
  if (abs >= 1e3) { return (num / 1e3).toFixed(1) + 'k'; }
  return String(Math.round(num));
}

function formatUsd(value, fractionDigits) {
  if (value === null || value === undefined || !Number.isFinite(Number(value))) {
    return '--';
  }
  const num = Number(value);
  if (num === 0) { return '$0.00'; }
  const defaultDigits = fractionDigits !== undefined ? fractionDigits : 4;
  const digits = Math.abs(num) < 0.0001 ? 6 : defaultDigits;
  return '$' + num.toFixed(digits);
}

function formatUsdShort(value) {
  const num = Number(value);
  if (!Number.isFinite(num) || num <= 0) { return '$0'; }
  if (num < 0.01) { return '$' + num.toFixed(4); }
  if (num < 1) { return '$' + num.toFixed(3); }
  return '$' + num.toFixed(2);
}

function formatPercent(value, digits) {
  if (value === null || value === undefined || !Number.isFinite(Number(value))) {
    return '--';
  }
  const d = digits !== undefined ? digits : 1;
  return Number(value).toFixed(d) + '%';
}

function formatDuration(ms) {
  if (ms === null || ms === undefined || !Number.isFinite(Number(ms)) || Number(ms) <= 0) {
    return '--';
  }
  const num = Number(ms);
  if (num < 1000) { return Math.round(num) + 'ms'; }
  const sec = num / 1000;
  if (sec < 60) { return (sec < 10 ? sec.toFixed(1) : Math.round(sec)) + 's'; }
  const min = Math.floor(sec / 60);
  return min + 'm ' + Math.round(sec % 60) + 's';
}

function formatTimestamp(ms) {
  if (!ms || !Number.isFinite(Number(ms))) { return '--'; }
  const d = new Date(Number(ms));
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  const hours = String(d.getHours()).padStart(2, '0');
  const minutes = String(d.getMinutes()).padStart(2, '0');
  const seconds = String(d.getSeconds()).padStart(2, '0');
  return month + '-' + day + ' ' + hours + ':' + minutes + ':' + seconds;
}

function getFreshInputTokens(point) {
  if (!point) { return 0; }
  const input = Number(point.inputTokens || 0);
  const cacheRead = Number(point.cacheReadTokens || 0);
  const cacheWrite = Number(point.cacheWriteTokens || 0);
  return Math.max(0, input - cacheRead - cacheWrite);
}

function calcCacheHitRate(inputTokens, cacheReadTokens) {
  const input = Number(inputTokens);
  if (!Number.isFinite(input) || input <= 0) { return null; }
  const rate = (Number(cacheReadTokens || 0) / input) * 100;
  return Math.min(100, Math.max(0, rate));
}

function toLocalDateKey(timestamp) {
  const d = new Date(timestamp);
  const year = d.getFullYear();
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return year + '-' + month + '-' + day;
}

function toLocalHourKey(timestamp) {
  const d = new Date(timestamp);
  const hours = String(d.getHours()).padStart(2, '0');
  return toLocalDateKey(timestamp) + ' ' + hours + ':00';
}

function formatBucketLabel(dateStr, granularity) {
  const text = String(dateStr || '');
  if (granularity === 'hour' || text.length > 10) {
    return text.slice(11) || text;
  }
  return text.slice(5) || text;
}

function createEmptyBucket(dateKey) {
  return {
    date: dateKey,
    requestCount: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    totalCostUsd: null
  };
}

function parseBucketTimestamp(dateKey, granularity) {
  if (!dateKey) { return null; }
  const iso = granularity === 'hour' ? dateKey.replace(' ', 'T') + ':00' : dateKey + 'T00:00:00';
  const ts = new Date(iso).getTime();
  return Number.isFinite(ts) ? ts : null;
}

// 补齐连续小时桶（最多 24 个）或日期桶（最多 366 天），确保趋势图 X 轴连续。
function fillUsageSeries(points, rangeOrGranularity, startTimeArg, endTimeArg) {
  const list = Array.isArray(points) ? points : [];
  let granularity = 'day';
  let startTime = startTimeArg;
  let endTime = endTimeArg;
  if (rangeOrGranularity && typeof rangeOrGranularity === 'object') {
    granularity = rangeOrGranularity.granularity || 'day';
    startTime = rangeOrGranularity.startTime;
    endTime = rangeOrGranularity.endTime;
  } else if (typeof rangeOrGranularity === 'string') {
    granularity = rangeOrGranularity;
  }

  const pointMap = new Map(list.map((item) => [item.date, item]));
  const fallbackStart = list.length > 0 ? parseBucketTimestamp(list[0].date, granularity) : null;
  const rangeStart = startTime !== undefined && startTime !== null ? startTime : fallbackStart;
  if (rangeStart === null || !Number.isFinite(rangeStart)) { return []; }

  const result = [];
  if (granularity === 'hour') {
    const cursor = new Date(rangeStart);
    cursor.setMinutes(0, 0, 0);
    const endHour = new Date(endTime !== undefined && endTime !== null ? endTime : Date.now());
    endHour.setMinutes(0, 0, 0);
    for (let i = 0; i < 24 && cursor.getTime() <= endHour.getTime(); i += 1) {
      const key = toLocalHourKey(cursor.getTime());
      result.push(pointMap.get(key) || createEmptyBucket(key));
      cursor.setHours(cursor.getHours() + 1);
    }
    return result;
  }

  const cursor = new Date(rangeStart);
  cursor.setHours(0, 0, 0, 0);
  const fallbackEnd = list.length > 0 ? parseBucketTimestamp(list[list.length - 1].date, granularity) : Date.now();
  const endDate = new Date(
    endTime !== undefined && endTime !== null
      ? endTime
      : Math.max(Date.now(), fallbackEnd || Date.now())
  );
  endDate.setHours(0, 0, 0, 0);
  const maxSpanStart = new Date(endDate);
  maxSpanStart.setDate(maxSpanStart.getDate() - 365);
  if (cursor.getTime() < maxSpanStart.getTime()) {
    cursor.setTime(maxSpanStart.getTime());
  }
  for (let i = 0; i < 366 && cursor.getTime() <= endDate.getTime(); i += 1) {
    const key = toLocalDateKey(cursor.getTime());
    result.push(pointMap.get(key) || createEmptyBucket(key));
    cursor.setDate(cursor.getDate() + 1);
  }
  return result;
}

// ===== 悬浮 Tooltip 控制器 =====

function showChartTooltip(evt, title, rows) {
  const tip = document.getElementById('chart-tooltip');
  if (!tip) { return; }
  tip.replaceChildren();
  tip.appendChild(domEl('div', 'tooltip-title', title));
  for (const row of rows || []) {
    const line = domEl('div', 'tooltip-row');
    const left = domEl('span', 'tooltip-label-wrap');
    if (row.color) {
      const dot = domEl('span', 'tooltip-dot');
      dot.style.background = row.color;
      left.appendChild(dot);
    }
    left.appendChild(domEl('span', null, row.label));
    const right = domEl('span', 'tooltip-value', row.value);
    line.appendChild(left);
    line.appendChild(right);
    tip.appendChild(line);
  }
  tip.hidden = false;
  tip.classList.remove('hidden');
  positionChartTooltip(evt);
}

function positionChartTooltip(evt) {
  const tip = document.getElementById('chart-tooltip');
  if (!tip || tip.hidden) { return; }
  const rect = tip.getBoundingClientRect();
  const offset = 14;
  let x = (evt.clientX || 0) + offset;
  let y = (evt.clientY || 0) + offset;
  if (x + rect.width + 12 > window.innerWidth) {
    x = Math.max(8, (evt.clientX || 0) - rect.width - offset);
  }
  if (y + rect.height + 12 > window.innerHeight) {
    y = Math.max(8, (evt.clientY || 0) - rect.height - offset);
  }
  tip.style.left = x + 'px';
  tip.style.top = y + 'px';
}

function hideChartTooltip() {
  const tip = document.getElementById('chart-tooltip');
  if (!tip) { return; }
  tip.hidden = true;
  tip.classList.add('hidden');
}

function renderEmptyChart(host, message) {
  hideChartTooltip();
  host.replaceChildren();
  host.appendChild(domEl('div', 'chart-empty', message || 'No usage data in the selected range.'));
}

// ===== Chart 1: Token & Cost Trend (双 Y 轴 4 层堆叠面积 + Cost 虚线折线) =====

function renderTrendChart(host, series, granularity) {
  if (!host) { return; }
  const list = Array.isArray(series) ? series : [];
  const hasData = list.some((p) => (p.requestCount || 0) > 0 || (p.inputTokens || 0) > 0 || (p.outputTokens || 0) > 0);
  if (!list.length || !hasData) {
    renderEmptyChart(host);
    return;
  }
  host.replaceChildren();

  const W = 920;
  const H = 260;
  const padL = 52;
  const padR = 60;
  const padT = 16;
  const padB = 30;
  const plotW = W - padL - padR;
  const plotH = H - padT - padB;

  const computed = list.map((p) => {
    const fresh = getFreshInputTokens(p);
    const cacheRead = Number(p.cacheReadTokens || 0);
    const cacheWrite = Number(p.cacheWriteTokens || 0);
    const output = Number(p.outputTokens || 0);
    const y1 = fresh;
    const y2 = y1 + cacheRead;
    const y3 = y2 + cacheWrite;
    const y4 = y3 + output;
    const cost = p.totalCostUsd !== null && p.totalCostUsd !== undefined ? Number(p.totalCostUsd) : null;
    return {
      raw: p,
      label: formatBucketLabel(p.date, granularity),
      fresh: fresh,
      cacheRead: cacheRead,
      cacheWrite: cacheWrite,
      output: output,
      totalTokens: y4,
      y0: 0,
      y1: y1,
      y2: y2,
      y3: y3,
      y4: y4,
      cost: cost
    };
  });

  const maxTokens = Math.max(4, ...computed.map((d) => d.y4));
  const hasAnyCost = computed.some((d) => d.cost !== null);
  const maxCost = Math.max(0.0001, ...computed.map((d) => d.cost || 0));
  const n = computed.length;
  const xAt = (idx) => (n <= 1 ? padL + plotW / 2 : padL + (idx / (n - 1)) * plotW);
  const yTok = (val) => padT + plotH - (Math.max(0, val) / maxTokens) * plotH;
  const yCost = (val) => padT + plotH - (Math.max(0, val) / maxCost) * plotH;

  const svg = svgEl('svg', {viewBox: '0 0 ' + W + ' ' + H, class: 'chart-svg'});
  const defs = svgEl('defs');
  const gradSpecs = [
    ['gradFresh', CHART_COLORS.freshInput],
    ['gradCacheRead', CHART_COLORS.cacheRead],
    ['gradCacheWrite', CHART_COLORS.cacheWrite],
    ['gradOutput', CHART_COLORS.output]
  ];
  for (const [id, color] of gradSpecs) {
    const lg = svgEl('linearGradient', {id: id, x1: '0', y1: '0', x2: '0', y2: '1'});
    lg.appendChild(svgEl('stop', {offset: '5%', 'stop-color': color, 'stop-opacity': '0.32'}));
    lg.appendChild(svgEl('stop', {offset: '95%', 'stop-color': color, 'stop-opacity': '0.02'}));
    defs.appendChild(lg);
  }
  svg.appendChild(defs);

  // 水平网格线与左右 Y 轴刻度
  const ticks = 4;
  for (let i = 0; i <= ticks; i += 1) {
    const ratio = i / ticks;
    const y = padT + plotH - ratio * plotH;
    svg.appendChild(svgEl('line', {
      x1: padL, y1: y, x2: W - padR, y2: y,
      stroke: CHART_COLORS.grid, 'stroke-dasharray': i === 0 ? undefined : '3 3'
    }));
    svg.appendChild(svgEl('text', {
      x: padL - 8, y: y + 4,
      fill: CHART_COLORS.axis, 'font-size': '11', 'text-anchor': 'end'
    }, formatCompact(maxTokens * ratio)));
    if (hasAnyCost) {
      svg.appendChild(svgEl('text', {
        x: W - padR + 8, y: y + 4,
        fill: CHART_COLORS.axis, 'font-size': '11', 'text-anchor': 'start'
      }, formatUsdShort(maxCost * ratio)));
    }
  }

  // X 轴标签（最多约 8 个均匀分布刻度）
  const step = Math.max(1, Math.ceil(n / 8));
  for (let i = 0; i < n; i += step) {
    svg.appendChild(svgEl('text', {
      x: xAt(i), y: H - 8,
      fill: CHART_COLORS.axis, 'font-size': '11', 'text-anchor': 'middle'
    }, computed[i].label));
  }

  // 堆叠面积与顶边折线
  const layers = [
    {low: 'y0', high: 'y1', stroke: CHART_COLORS.freshInput, fill: 'url(#gradFresh)'},
    {low: 'y1', high: 'y2', stroke: CHART_COLORS.cacheRead, fill: 'url(#gradCacheRead)'},
    {low: 'y2', high: 'y3', stroke: CHART_COLORS.cacheWrite, fill: 'url(#gradCacheWrite)'},
    {low: 'y3', high: 'y4', stroke: CHART_COLORS.output, fill: 'url(#gradOutput)'}
  ];
  for (const layer of layers) {
    if (n === 1) {
      const cx = xAt(0);
      const yTop = yTok(computed[0][layer.high]);
      const yBot = yTok(computed[0][layer.low]);
      svg.appendChild(svgEl('rect', {
        x: cx - 18, y: yTop, width: 36, height: Math.max(0, yBot - yTop),
        fill: layer.fill, stroke: layer.stroke, 'stroke-width': '1.5'
      }));
      continue;
    }
    const topPts = computed.map((d, idx) => xAt(idx).toFixed(1) + ',' + yTok(d[layer.high]).toFixed(1));
    const botPts = computed.slice().reverse().map((d, revIdx) => {
      const idx = n - 1 - revIdx;
      return xAt(idx).toFixed(1) + ',' + yTok(d[layer.low]).toFixed(1);
    });
    const areaD = 'M' + topPts.join(' L') + ' L' + botPts.join(' L') + ' Z';
    const lineD = 'M' + topPts.join(' L');
    svg.appendChild(svgEl('path', {d: areaD, fill: layer.fill}));
    svg.appendChild(svgEl('path', {d: lineD, fill: 'none', stroke: layer.stroke, 'stroke-width': '1.8'}));
  }

  // 右 Y 轴成本虚线折线
  const hasAnyCost = computed.some((d) => d.cost !== null && d.cost > 0);
  if (hasAnyCost) {
    const costPts = computed.map((d, idx) => xAt(idx).toFixed(1) + ',' + yCost(d.cost || 0).toFixed(1));
    if (n > 1) {
      svg.appendChild(svgEl('path', {
        d: 'M' + costPts.join(' L'),
        fill: 'none',
        stroke: CHART_COLORS.cost,
        'stroke-width': '2',
        'stroke-dasharray': '5 4'
      }));
    } else {
      svg.appendChild(svgEl('circle', {
        cx: xAt(0), cy: yCost(computed[0].cost || 0), r: '4', fill: CHART_COLORS.cost
      }));
    }
  }

  // 垂直悬停准星线与透明交互带
  const crosshair = svgEl('line', {
    x1: padL, y1: padT, x2: padL, y2: padT + plotH,
    stroke: CHART_COLORS.crosshair, 'stroke-dasharray': '3 3', visibility: 'hidden'
  });
  svg.appendChild(crosshair);

  const bandW = n <= 1 ? plotW : plotW / n;
  for (let i = 0; i < n; i += 1) {
    const cx = xAt(i);
    const bx = n <= 1 ? padL : Math.max(padL, cx - bandW / 2);
    const bw = n <= 1 ? plotW : Math.min(bandW, W - padR - bx);
    const hit = svgEl('rect', {
      x: bx, y: padT, width: Math.max(4, bw), height: plotH,
      fill: 'transparent', style: 'cursor: crosshair'
    });
    const d = computed[i];
    hit.addEventListener('mousemove', (evt) => {
      crosshair.setAttribute('x1', String(cx));
      crosshair.setAttribute('x2', String(cx));
      crosshair.setAttribute('visibility', 'visible');
      showChartTooltip(evt, d.raw.date, [
        {label: 'Fresh Input', value: formatNumber(d.fresh), color: CHART_COLORS.freshInput},
        {label: 'Cache Read', value: formatNumber(d.cacheRead), color: CHART_COLORS.cacheRead},
        {label: 'Cache Write', value: formatNumber(d.cacheWrite), color: CHART_COLORS.cacheWrite},
        {label: 'Output', value: formatNumber(d.output), color: CHART_COLORS.output},
        {label: 'Total Tokens', value: formatNumber(d.totalTokens)},
        {label: 'Cost', value: formatUsd(d.cost), color: CHART_COLORS.cost}
      ]);
    });
    hit.addEventListener('mouseleave', () => {
      crosshair.setAttribute('visibility', 'hidden');
      hideChartTooltip();
    });
    svg.appendChild(hit);
  }

  host.appendChild(svg);
}

// ===== Chart 2: Requests (请求数柱状图) =====

function renderRequestsChart(host, series, granularity) {
  if (!host) { return; }
  const list = Array.isArray(series) ? series : [];
  const hasData = list.some((p) => (p.requestCount || 0) > 0);
  if (!list.length || !hasData) {
    renderEmptyChart(host);
    return;
  }
  host.replaceChildren();

  const W = 440;
  const H = 230;
  const padL = 42;
  const padR = 14;
  const padT = 14;
  const padB = 28;
  const plotW = W - padL - padR;
  const plotH = H - padT - padB;

  const rawMaxReq = Math.max(1, ...list.map((p) => Number(p.requestCount || 0)));
  const maxReq = Math.max(4, Math.ceil(rawMaxReq / 4) * 4);
  const n = list.length;
  const slotW = plotW / n;
  const barW = Math.max(3, Math.min(28, slotW * 0.68));

  const svg = svgEl('svg', {viewBox: '0 0 ' + W + ' ' + H, class: 'chart-svg'});
  const ticks = 4;
  for (let i = 0; i <= ticks; i += 1) {
    const ratio = i / ticks;
    const y = padT + plotH - ratio * plotH;
    svg.appendChild(svgEl('line', {
      x1: padL, y1: y, x2: W - padR, y2: y,
      stroke: CHART_COLORS.grid, 'stroke-dasharray': i === 0 ? undefined : '3 3'
    }));
    svg.appendChild(svgEl('text', {
      x: padL - 6, y: y + 4,
      fill: CHART_COLORS.axis, 'font-size': '10', 'text-anchor': 'end'
    }, formatCompact(Math.round(maxReq * ratio))));
  }

  const step = Math.max(1, Math.ceil(n / 6));
  for (let i = 0; i < n; i += step) {
    const cx = padL + (i + 0.5) * slotW;
    svg.appendChild(svgEl('text', {
      x: cx, y: H - 8,
      fill: CHART_COLORS.axis, 'font-size': '10', 'text-anchor': 'middle'
    }, formatBucketLabel(list[i].date, granularity)));
  }

  for (let i = 0; i < n; i += 1) {
    const p = list[i];
    const reqs = Number(p.requestCount || 0);
    const bh = (reqs / maxReq) * plotH;
    const bx = padL + (i + 0.5) * slotW - barW / 2;
    const by = padT + plotH - bh;
    const bar = svgEl('rect', {
      x: bx.toFixed(1),
      y: by.toFixed(1),
      width: barW.toFixed(1),
      height: Math.max(0, bh).toFixed(1),
      fill: CHART_COLORS.requests
    });
    const hit = svgEl('rect', {
      x: (padL + i * slotW).toFixed(1),
      y: padT,
      width: slotW.toFixed(1),
      height: plotH,
      fill: 'transparent'
    });
    hit.addEventListener('mousemove', (evt) => {
      bar.setAttribute('opacity', '0.82');
      showChartTooltip(evt, p.date, [
        {label: 'Requests', value: formatNumber(reqs), color: CHART_COLORS.requests},
        {label: 'Tokens', value: formatNumber((p.inputTokens || 0) + (p.outputTokens || 0))}
      ]);
    });
    hit.addEventListener('mouseleave', () => {
      bar.removeAttribute('opacity');
      hideChartTooltip();
    });
    svg.appendChild(bar);
    svg.appendChild(hit);
  }

  host.appendChild(svg);
}

// ===== Chart 3: Model Distribution (Top 8 横向条形图，Cost 优先 / Token 兜底) =====

function renderModelDistributionChart(host, models, subtitleEl) {
  if (!host) { return; }
  const subEl = subtitleEl || document.getElementById('chart-models-subtitle');
  const visible = (Array.isArray(models) ? models : []).filter(
    (m) => Number(m.totalTokens || 0) > 0 || Number(m.requestCount || 0) > 0
  );
  const hasPricing = visible.some((m) => m.totalCostUsd !== null && m.totalCostUsd !== undefined && Number(m.totalCostUsd) > 0);
  if (subEl) {
    subEl.textContent = hasPricing ? 'Top 8 by cost (USD)' : 'No pricing configured — showing token volume';
  }
  if (!visible.length) {
    renderEmptyChart(host);
    return;
  }
  host.replaceChildren();

  const sorted = visible.slice().sort((a, b) => {
    if (hasPricing) {
      return Number(b.totalCostUsd || 0) - Number(a.totalCostUsd || 0);
    }
    return Number(b.totalTokens || 0) - Number(a.totalTokens || 0);
  }).slice(0, 8);

  const W = 440;
  const H = 230;
  const padL = 128;
  const padR = 64;
  const padT = 10;
  const padB = 14;
  const plotW = W - padL - padR;
  const plotH = H - padT - padB;
  const rowH = plotH / Math.max(1, sorted.length);
  const barH = Math.min(18, Math.max(8, rowH * 0.62));
  const maxVal = Math.max(
    0.000001,
    ...sorted.map((m) => (hasPricing ? Number(m.totalCostUsd || 0) : Number(m.totalTokens || 0)))
  );
  const barColor = hasPricing ? CHART_COLORS.cost : CHART_COLORS.freshInput;

  const svg = svgEl('svg', {viewBox: '0 0 ' + W + ' ' + H, class: 'chart-svg'});
  sorted.forEach((m, idx) => {
    const val = hasPricing ? Number(m.totalCostUsd || 0) : Number(m.totalTokens || 0);
    const cy = padT + (idx + 0.5) * rowH;
    const bw = Math.max(2, (val / maxVal) * plotW);
    const nameText = String(m.model || 'unknown');
    const shortName = nameText.length > 17 ? nameText.slice(0, 16) + '…' : nameText;

    svg.appendChild(svgEl('text', {
      x: padL - 8, y: cy + 4,
      fill: '#e8e8e8', 'font-size': '11', 'text-anchor': 'end'
    }, shortName));

    const bar = svgEl('rect', {
      x: padL, y: (cy - barH / 2).toFixed(1),
      width: bw.toFixed(1), height: barH.toFixed(1),
      fill: barColor
    });
    const valLabel = hasPricing ? formatUsd(m.totalCostUsd) : formatCompact(m.totalTokens);
    svg.appendChild(svgEl('text', {
      x: (padL + bw + 6).toFixed(1), y: cy + 4,
      fill: CHART_COLORS.axis, 'font-size': '10', 'text-anchor': 'start'
    }, valLabel));

    const hit = svgEl('rect', {
      x: 0, y: (padT + idx * rowH).toFixed(1),
      width: W, height: rowH.toFixed(1),
      fill: 'transparent'
    });
    hit.addEventListener('mousemove', (evt) => {
      bar.setAttribute('opacity', '0.82');
      showChartTooltip(evt, nameText, [
        {label: 'Requests', value: formatNumber(m.requestCount)},
        {label: 'Total Tokens', value: formatNumber(m.totalTokens), color: CHART_COLORS.freshInput},
        {label: 'Cost', value: formatUsd(m.totalCostUsd), color: CHART_COLORS.cost},
        {label: 'Avg Cost / Req', value: formatUsd(m.avgCostPerRequestUsd)}
      ]);
    });
    hit.addEventListener('mouseleave', () => {
      bar.removeAttribute('opacity');
      hideChartTooltip();
    });
    svg.appendChild(bar);
    svg.appendChild(hit);
  });

  host.appendChild(svg);
}

// ===== 环形图公共绘制辅助 =====

function donutArcPath(cx, cy, rOuter, rInner, startAngle, endAngle) {
  const clampSpan = Math.min(Math.PI * 2 - 0.0001, Math.max(0.0001, endAngle - startAngle));
  const end = startAngle + clampSpan;
  const largeArc = clampSpan > Math.PI ? 1 : 0;
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

function renderDonutWithLegend(host, slices, centerPrimary, centerSecondary) {
  host.replaceChildren();
  const layout = domEl('div', 'donut-layout');
  const svgWrap = domEl('div', 'donut-svg-wrap');
  const svg = svgEl('svg', {viewBox: '0 0 180 180', class: 'chart-svg'});
  const cx = 90;
  const cy = 90;
  const rOuter = 74;
  const rInner = 46;

  const total = slices.reduce((acc, s) => acc + Math.max(0, Number(s.value || 0)), 0);
  let angle = -Math.PI / 2;
  const nonZero = slices.filter((s) => Number(s.value || 0) > 0);

  for (const slice of nonZero) {
    const val = Number(slice.value || 0);
    const sweep = total > 0 ? (val / total) * Math.PI * 2 : 0;
    const gap = nonZero.length > 1 ? Math.min(0.025, sweep * 0.1) : 0;
    const d = donutArcPath(cx, cy, rOuter, rInner, angle + gap / 2, angle + sweep - gap / 2);
    const path = svgEl('path', {d: d, fill: slice.color});
    path.addEventListener('mousemove', (evt) => {
      path.setAttribute('opacity', '0.82');
      showChartTooltip(evt, slice.name, slice.tooltipRows);
    });
    path.addEventListener('mouseleave', () => {
      path.removeAttribute('opacity');
      hideChartTooltip();
    });
    svg.appendChild(path);
    angle += sweep;
  }

  svg.appendChild(svgEl('text', {
    x: cx, y: cy - 2,
    fill: '#e8e8e8', 'font-size': '15', 'font-weight': '600', 'text-anchor': 'middle'
  }, centerPrimary));
  svg.appendChild(svgEl('text', {
    x: cx, y: cy + 15,
    fill: CHART_COLORS.axis, 'font-size': '10', 'text-anchor': 'middle'
  }, centerSecondary));

  svgWrap.appendChild(svg);
  layout.appendChild(svgWrap);

  const legend = domEl('div', 'donut-legend');
  for (const slice of slices) {
    const row = domEl('div', 'donut-legend-row');
    const swatch = domEl('span', 'legend-swatch');
    swatch.style.background = slice.color;
    const name = domEl('span', 'donut-legend-name', slice.name);
    name.title = String(slice.name);
    const val = domEl('span', 'donut-legend-val', slice.legendRight);
    row.appendChild(swatch);
    row.appendChild(name);
    row.appendChild(val);
    row.addEventListener('mousemove', (evt) => showChartTooltip(evt, slice.name, slice.tooltipRows));
    row.addEventListener('mouseleave', hideChartTooltip);
    legend.appendChild(row);
  }
  layout.appendChild(legend);
  host.appendChild(layout);
}

// ===== Chart 4: Provider Distribution (Top 8 供应商环形图) =====

function renderProviderDistributionChart(host, providers, subtitleEl) {
  if (!host) { return; }
  const subEl = subtitleEl || document.getElementById('chart-providers-subtitle');
  const list = (Array.isArray(providers) ? providers : []).filter((p) => Number(p.requestCount || 0) > 0);
  if (!list.length) {
    if (subEl) { subEl.textContent = 'Top 8 by cost (USD)'; }
    renderEmptyChart(host);
    return;
  }

  const hasPricing = list.some((p) => p.totalCostUsd !== null && p.totalCostUsd !== undefined && Number(p.totalCostUsd) > 0);
  if (subEl) {
    subEl.textContent = hasPricing ? 'Top 8 by cost (USD)' : 'No pricing configured — showing token volume';
  }

  const sorted = list.slice().sort((a, b) => {
    if (hasPricing) {
      return Number(b.totalCostUsd || 0) - Number(a.totalCostUsd || 0);
    }
    return Number(b.totalTokens || 0) - Number(a.totalTokens || 0);
  }).slice(0, 8);

  const totalMetric = sorted.reduce((acc, p) => acc + (hasPricing ? Number(p.totalCostUsd || 0) : Number(p.totalTokens || 0)), 0);
  if (totalMetric <= 0) {
    renderEmptyChart(host);
    return;
  }

  const slices = sorted.map((p, idx) => {
    const metricVal = hasPricing ? Number(p.totalCostUsd || 0) : Number(p.totalTokens || 0);
    const share = (metricVal / totalMetric) * 100;
    const color = PROVIDER_PALETTE[idx % PROVIDER_PALETTE.length];
    return {
      name: String(p.provider || 'unknown'),
      value: metricVal,
      color: color,
      legendRight: (hasPricing ? formatUsd(metricVal) : formatCompact(metricVal)) + ' (' + formatPercent(share, 1) + ')',
      tooltipRows: [
        {label: 'Cost', value: formatUsd(p.totalCostUsd), color: color},
        {label: 'Total Tokens', value: formatNumber(p.totalTokens)},
        {label: 'Requests', value: formatNumber(p.requestCount)},
        {label: 'Success Rate', value: formatPercent(p.successRate)},
        {label: 'Avg Duration', value: formatDuration(p.avgDurationMs)}
      ]
    };
  });

  renderDonutWithLegend(
    host,
    slices,
    hasPricing ? formatUsd(totalMetric) : formatCompact(totalMetric),
    hasPricing ? 'total cost' : 'tokens'
  );
}

// ===== Chart 5: Token Composition (4 类 Token 构成环形图) =====

function renderTokenCompositionChart(host, summary) {
  if (!host) { return; }
  if (!summary) {
    renderEmptyChart(host);
    return;
  }
  const fresh = getFreshInputTokens(summary);
  const output = Number(summary.outputTokens || 0);
  const cacheRead = Number(summary.cacheReadTokens || 0);
  const cacheWrite = Number(summary.cacheWriteTokens || 0);
  const total = fresh + output + cacheRead + cacheWrite;
  if (total <= 0) {
    renderEmptyChart(host);
    return;
  }

  const rawItems = [
    {name: 'Fresh Input', value: fresh, color: CHART_COLORS.freshInput},
    {name: 'Output', value: output, color: CHART_COLORS.output},
    {name: 'Cache Read', value: cacheRead, color: CHART_COLORS.cacheRead},
    {name: 'Cache Write', value: cacheWrite, color: CHART_COLORS.cacheWrite}
  ];
  const slices = rawItems.map((item) => {
    const share = (item.value / total) * 100;
    return {
      name: item.name,
      value: item.value,
      color: item.color,
      legendRight: formatCompact(item.value) + ' (' + formatPercent(share, 1) + ')',
      tooltipRows: [
        {label: 'Tokens', value: formatNumber(item.value), color: item.color},
        {label: 'Share', value: formatPercent(share, 1)}
      ]
    };
  });

  renderDonutWithLegend(host, slices, formatCompact(total), 'tokens');
}
