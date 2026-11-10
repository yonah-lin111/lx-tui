// Shared DOM-to-Markdown serializer for AI Web drivers (DeepSeek, Gemini, Grok).
//
// `lxHtmlToMarkdown` must remain 100% self-contained (no external closures) so
// `lxHtmlToMarkdown.toString()` can be injected into browser page contexts.

function lxHtmlToMarkdown(root, options) {
  if (!root) return '';
  const opts = options || {};
  const streaming = Boolean(opts.streaming);

  const cleanText = (value) =>
    String(value || '')
      .replace(/\r\n?/g, '\n')
      .replace(/[\u200b\ufeff\u00ad\u2060]/g, '')
      .replace(/\u00a0/g, ' ');

  const isCodeBlockContainer = (el) => {
    if (!el || el.nodeType !== 1) return false;
    const tag = el.tagName.toLowerCase();
    if (tag === 'code-block' || tag === 'pre') return true;
    const cls = el.classList;
    if (
      cls &&
      (cls.contains('md-code-block') ||
        cls.contains('chat-code-block') ||
        cls.contains('code-block'))
    ) {
      return true;
    }
    const testId = el.getAttribute && el.getAttribute('data-testid');
    if (testId === 'code-block') return true;
    return Boolean(el.getAttribute && el.getAttribute('data-code-card') === 'true');
  };

  const isMathContainer = (el) => {
    if (!el || el.nodeType !== 1) return false;
    const tag = el.tagName.toLowerCase();
    if (tag === 'math') return true;
    const cls = el.classList;
    return Boolean(cls && (cls.contains('katex') || cls.contains('katex-display')));
  };

  const shouldSkipElement = (el) => {
    if (!el || el.nodeType !== 1) return false;
    if (isCodeBlockContainer(el) || isMathContainer(el)) return false;
    const tag = el.tagName.toLowerCase();
    if (
      tag === 'script' ||
      tag === 'style' ||
      tag === 'noscript' ||
      tag === 'template' ||
      tag === 'button' ||
      tag === 'svg' ||
      tag === 'mat-icon' ||
      tag === 'gem-icon' ||
      tag === 'gem-icon-button' ||
      tag === 'source-footnote' ||
      tag === 'sources-list' ||
      tag === 'sources-carousel-inline' ||
      tag === 'overview-carousel' ||
      tag === 'message-actions' ||
      tag === 'response-status' ||
      tag === 'generated-image' ||
      tag === 'single-image' ||
      tag === 'grok:render' ||
      tag === 'argument'
    ) {
      return true;
    }
    if (el.getAttribute) {
      if (el.getAttribute('data-no-copy') === 'true') return true;
      if (
        el.getAttribute('aria-hidden') === 'true' &&
        !(el.querySelector && el.querySelector('pre, code'))
      ) {
        return true;
      }
    }
    const cls = el.classList;
    if (cls) {
      if (
        cls.contains('cdk-visually-hidden') ||
        cls.contains('sr-only') ||
        cls.contains('ds-markdown-cite') ||
        cls.contains('ds-markdown-code-copy-button') ||
        cls.contains('md-code-block-banner-wrap') ||
        cls.contains('md-code-block-banner') ||
        cls.contains('md-code-block-footer') ||
        cls.contains('md-code-block-action') ||
        cls.contains('code-block-decoration') ||
        cls.contains('table-footer') ||
        cls.contains('generated-images') ||
        cls.contains('t-copy-label-swap')
      ) {
        return true;
      }
      if (cls.contains('not-prose')) {
        if (
          el.querySelector &&
          el.querySelector('pre, code, [data-testid="code-block"], .chat-code-block')
        ) {
          return false;
        }
        return true;
      }
    }
    return false;
  };

  const hasMeaningfulContent = (node) => {
    if (!node) return false;
    if (node.nodeType === 3) {
      return cleanText(node.nodeValue || '').trim().length > 0;
    }
    if (node.nodeType !== 1) return false;
    if (shouldSkipElement(node)) return false;
    const tag = node.tagName.toLowerCase();
    if (tag === 'br' || tag === 'hr') return true;
    if (tag === 'img') {
      const src = ((node.getAttribute && node.getAttribute('src')) || '').trim();
      return /^https?:\/\//i.test(src);
    }
    if (isMathContainer(node)) {
      const ann =
        node.querySelector && node.querySelector('annotation[encoding="application/x-tex"]');
      const tex = ann
        ? cleanText(ann.textContent || '').trim()
        : cleanText(node.textContent || '').trim();
      return tex.length > 0;
    }
    if (isCodeBlockContainer(node)) {
      const pre =
        tag === 'pre'
          ? node
          : node.querySelector &&
            node.querySelector('pre, code[data-test-id="code-content"], code');
      return Boolean(pre && cleanText(pre.textContent || '').length > 0);
    }
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (hasMeaningfulContent(child)) return true;
    }
    return false;
  };

  const isTail = (node) => {
    let cur = node;
    while (cur && cur !== root) {
      for (let sib = cur.nextSibling; sib; sib = sib.nextSibling) {
        if (hasMeaningfulContent(sib)) return false;
      }
      cur = cur.parentNode;
    }
    return true;
  };

  const extractCodeLanguage = (el, pre, codeEl) => {
    let raw = '';
    if (el.querySelector) {
      const dsLang = el.querySelector('.md-code-block-infostring, .md-code-block-language');
      if (dsLang) raw = cleanText(dsLang.textContent || '').trim();
      if (!raw) {
        const gmLang = el.querySelector('.code-block-decoration > span, .code-block-decoration span');
        if (gmLang && !(gmLang.closest && gmLang.closest('button, .buttons'))) {
          raw = cleanText(gmLang.textContent || '').trim();
        }
      }
      if (!raw) {
        const gkLang = el.querySelector('span.font-mono');
        if (gkLang && !(gkLang.closest && gkLang.closest('button, pre, code'))) {
          raw = cleanText(gkLang.textContent || '').trim();
        }
      }
    }
    if (!raw && pre && pre.getAttribute) {
      const aria = cleanText(pre.getAttribute('aria-label') || '').trim();
      if (aria) {
        raw = aria.replace(/\s*(?:代码|code)$/i, '').trim();
      }
    }
    if (!raw) {
      for (const candidate of [codeEl, pre, el]) {
        if (!candidate) continue;
        const dataLang =
          candidate.getAttribute &&
          (candidate.getAttribute('data-language') || candidate.getAttribute('data-lang'));
        if (dataLang) {
          raw = cleanText(dataLang).trim();
          break;
        }
        const cls = typeof candidate.className === 'string' ? candidate.className : '';
        const match = cls.match(/(?:^|\s)language-([\w+#.-]+)/i);
        if (match && match[1]) {
          raw = match[1];
          break;
        }
      }
    }
    const token = raw.split(/\s+/)[0] || '';
    if (/^(?:copy|复制|已复制|copied|run|运行|download|下载|code|代码|text|plaintext)$/i.test(token)) {
      return '';
    }
    return token.toLowerCase();
  };

  const extractCodeBody = (codeEl) => {
    if (!codeEl) return '';
    const lineSpans =
      codeEl.querySelectorAll &&
      Array.from(codeEl.querySelectorAll(':scope > span.line'));
    if (lineSpans && lineSpans.length > 0) {
      const hasDirectNewlines = Array.from(codeEl.childNodes).some(
        (n) => n.nodeType === 3 && (n.nodeValue || '').includes('\n'),
      );
      if (!hasDirectNewlines) {
        return lineSpans.map((s) => cleanText(s.textContent || '')).join('\n');
      }
    }
    return cleanText(codeEl.textContent || '');
  };

  const serializeCodeBlock = (el) => {
    const tag = el.tagName.toLowerCase();
    const pre = tag === 'pre' ? el : el.querySelector && el.querySelector('pre');
    const codeEl =
      (pre && pre.querySelector && pre.querySelector('code')) ||
      (el.querySelector && el.querySelector('code[data-test-id="code-content"]')) ||
      pre;
    const rawCode = extractCodeBody(codeEl);
    const code = rawCode.replace(/\n+$/, '');
    const tail = streaming && isTail(el);
    if (tail && !code) return '';
    const lang = extractCodeLanguage(el, pre, codeEl);
    let maxTicks = /^(?:markdown|md|mdx)$/i.test(lang) ? 3 : 2;
    const tickMatches = code.match(/`+/g);
    if (tickMatches) {
      for (const m of tickMatches) {
        if (m.length > maxTicks) maxTicks = m.length;
      }
    }
    const fence = '`'.repeat(maxTicks + 1);
    if (tail) {
      return fence + lang + '\n' + code;
    }
    return fence + lang + '\n' + code + '\n' + fence;
  };

  const serializeMath = (el) => {
    const ann =
      el.querySelector && el.querySelector('annotation[encoding="application/x-tex"]');
    const tex = ann ? cleanText(ann.textContent || '').trim() : cleanText(el.textContent || '').trim();
    if (!tex) return '';
    const isDisplay =
      (el.classList && el.classList.contains('katex-display')) ||
      (el.getAttribute && el.getAttribute('display') === 'block');
    if (isDisplay) {
      return streaming && isTail(el) ? '$$\n' + tex : '$$\n' + tex + '\n$$';
    }
    return streaming && isTail(el) ? '$' + tex : '$' + tex + '$';
  };

  const isBlockElement = (el) => {
    if (!el || el.nodeType !== 1) return false;
    if (shouldSkipElement(el)) return false;
    if (isCodeBlockContainer(el)) return true;
    const tag = el.tagName.toLowerCase();
    return (
      tag === 'p' ||
      tag === 'h1' ||
      tag === 'h2' ||
      tag === 'h3' ||
      tag === 'h4' ||
      tag === 'h5' ||
      tag === 'h6' ||
      tag === 'ul' ||
      tag === 'ol' ||
      tag === 'li' ||
      tag === 'blockquote' ||
      tag === 'table' ||
      tag === 'hr' ||
      tag === 'pre'
    );
  };

  const isTransparentContainer = (el) => {
    if (!el || el.nodeType !== 1) return false;
    if (shouldSkipElement(el) || isCodeBlockContainer(el) || isMathContainer(el)) return false;
    if (isBlockElement(el)) return false;
    const tag = el.tagName.toLowerCase();
    if (
      tag === 'div' ||
      tag === 'section' ||
      tag === 'article' ||
      tag === 'main' ||
      tag === 'response-element' ||
      tag === 'table-block' ||
      tag === 'message-content' ||
      tag === 'model-thoughts' ||
      tag === 'thinking-overlay'
    ) {
      return true;
    }
    if (el.children && el.children.length > 0) {
      for (let i = 0; i < el.children.length; i += 1) {
        const child = el.children[i];
        if (isBlockElement(child) || isTransparentContainer(child)) {
          return true;
        }
      }
    }
    return false;
  };

  const wrapInline = (el, delim, flag, ctx) => {
    if (ctx[flag]) return serializeInlineNodes(el.childNodes, ctx);
    const inner = serializeInlineNodes(el.childNodes, { ...ctx, [flag]: true });
    if (!inner) return '';
    const leadMatch = inner.match(/^[ \t\n]+/);
    const lead = leadMatch ? leadMatch[0] : '';
    const rest = inner.slice(lead.length);
    if (!rest) return lead;
    const trailMatch = rest.match(/[ \t\n]+$/);
    const trail = trailMatch ? trailMatch[0] : '';
    const core = trail ? rest.slice(0, rest.length - trail.length) : rest;
    if (!core) return inner;
    if (streaming && isTail(el)) {
      return lead + delim + core;
    }
    return lead + delim + core + delim + trail;
  };

  const serializeInlineNode = (node, ctx) => {
    if (!node) return '';
    if (node.nodeType === 3) {
      const text = cleanText(node.nodeValue || '');
      return ctx.inCode ? text : text.replace(/[ \t]*\n[ \t]*/g, ' ');
    }
    if (node.nodeType !== 1) return '';
    const el = node;
    if (shouldSkipElement(el)) return '';
    if (isMathContainer(el)) return serializeMath(el);
    if (isCodeBlockContainer(el)) return serializeCodeBlock(el);

    const tag = el.tagName.toLowerCase();
    if (tag === 'br') return ctx.inTable ? '<br>' : '\n';
    if (tag === 'hr') return '\n\n---\n\n';
    if (tag === 'strong' || tag === 'b') {
      return ctx.inLink ? serializeInlineNodes(el.childNodes, ctx) : wrapInline(el, '**', 'inStrong', ctx);
    }
    if (tag === 'em' || tag === 'i') {
      return ctx.inLink ? serializeInlineNodes(el.childNodes, ctx) : wrapInline(el, '*', 'inEm', ctx);
    }
    if (tag === 'del' || tag === 's' || tag === 'strike') {
      return ctx.inLink ? serializeInlineNodes(el.childNodes, ctx) : wrapInline(el, '~~', 'inStrike', ctx);
    }
    if (tag === 'code') {
      const code = cleanText(el.textContent || '').replace(/\n+/g, ' ');
      if (!code) return '';
      if (ctx.inLink) return code;
      const tick = code.includes('`') ? '``' : '`';
      const pad = code.startsWith('`') || code.endsWith('`') ? ' ' : '';
      if (streaming && isTail(el)) {
        return tick + pad + code;
      }
      return tick + pad + code + pad + tick;
    }
    if (tag === 'a') {
      const rawHref = (el.getAttribute && el.getAttribute('href')) || '';
      const href = rawHref.trim();
      let label = serializeInlineNodes(el.childNodes, { ...ctx, inLink: true }).trim();
      if (!label && el.getAttribute) {
        const aria = cleanText(el.getAttribute('aria-label') || '').trim();
        if (aria) label = aria.replace(/\s*\([^)]*\)\s*$/, '').trim() || aria;
      }
      if (!href || /^javascript:/i.test(href)) return label;
      if (!label) label = href;
      const safeLabel = label.replace(/\[/g, '\\[').replace(/\]/g, '\\]');
      const safeHref = href.replace(/\(/g, '%28').replace(/\)/g, '%29');
      if (streaming && isTail(el)) {
        return '[' + safeLabel;
      }
      return '[' + safeLabel + '](' + safeHref + ')';
    }
    if (tag === 'img') {
      const src = ((el.getAttribute && el.getAttribute('src')) || '').trim();
      if (!/^https?:\/\//i.test(src)) return '';
      const alt = cleanText((el.getAttribute && el.getAttribute('alt')) || 'image')
        .replace(/\[/g, '')
        .replace(/\]/g, '')
        .trim() || 'image';
      return '![' + alt + '](' + src + ')';
    }
    return serializeInlineNodes(el.childNodes, ctx);
  };

  const serializeInlineNodes = (nodes, ctx) => {
    let out = '';
    for (let i = 0; i < nodes.length; i += 1) {
      out += serializeInlineNode(nodes[i], ctx);
    }
    return out;
  };

  const serializeTable = (tableEl, ctx) => {
    const allRows = Array.from(tableEl.querySelectorAll('tr')).filter(
      (tr) => tr.closest('table') === tableEl,
    );
    if (allRows.length === 0) return '';
    const tableIsTail = streaming && isTail(tableEl);
    if (tableIsTail && isTail(allRows[0])) {
      return '';
    }
    const formatRow = (tr) =>
      Array.from(tr.children)
        .filter((c) => c.tagName && /^(th|td)$/i.test(c.tagName))
        .map((cell) =>
          serializeInlineNodes(cell.childNodes, { ...ctx, inTable: true })
            .replace(/\s*\n\s*/g, '<br>')
            .replace(/[ \t]{2,}/g, ' ')
            .replace(/\|/g, '｜')
            .trim(),
        );
    const headerCells = formatRow(allRows[0]);
    const colCount = Math.max(1, headerCells.length);
    const padRow = (cells) => {
      const row = cells.slice(0, colCount);
      while (row.length < colCount) row.push('');
      return '| ' + row.join(' | ') + ' |';
    };
    const lines = [
      padRow(headerCells),
      '| ' + Array(colCount).fill('---').join(' | ') + ' |',
    ];
    for (let i = 1; i < allRows.length; i += 1) {
      const tr = allRows[i];
      if (tableIsTail && isTail(tr)) break;
      lines.push(padRow(formatRow(tr)));
    }
    return lines.join('\n');
  };

  const serializeListItem = (li, marker, indent, ctx) => {
    const contIndent = ' '.repeat(indent.length + marker.length);
    const segments = [];
    let inlineBuf = [];
    const flushInline = () => {
      if (inlineBuf.length === 0) return;
      const text = serializeInlineNodes(inlineBuf, ctx).replace(/^[ \t]+/, '').replace(/[ \t]+$/, '');
      inlineBuf = [];
      if (text) segments.push({ kind: 'block', text });
    };

    for (let child = li.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 1 && shouldSkipElement(child)) continue;
      if (
        child.nodeType === 1 &&
        child.tagName.toLowerCase() === 'input' &&
        child.getAttribute('type') === 'checkbox'
      ) {
        continue;
      }
      if (child.nodeType === 1) {
        const cTag = child.tagName.toLowerCase();
        if (cTag === 'ul' || cTag === 'ol') {
          flushInline();
          const nested = serializeList(child, cTag === 'ol', {
            ...ctx,
            listDepth: (ctx.listDepth || 0) + 1,
          });
          if (nested) segments.push({ kind: 'list', text: nested });
          continue;
        }
        if (isBlockElement(child) || isTransparentContainer(child)) {
          flushInline();
          const blockText = serializeBlock(child, { ...ctx, inListItem: true });
          if (blockText) segments.push({ kind: 'block', text: blockText });
          continue;
        }
      }
      inlineBuf.push(child);
    }
    flushInline();

    if (segments.length === 0) return '';
    let out = '';
    for (let i = 0; i < segments.length; i += 1) {
      const seg = segments[i];
      const cleaned = seg.text.replace(/^[☑□✅]\s*/, '');
      const isFenceBlock = /^`{3,}/.test(cleaned);
      const requiresOwnLine = isFenceBlock || /^(?:\$\$\n|\| |> )/.test(cleaned);
      if (i === 0 && seg.kind !== 'list' && !requiresOwnLine) {
        const lines = cleaned.split('\n');
        out +=
          indent +
          marker +
          lines[0] +
          (lines.length > 1
            ? '\n' + lines.slice(1).map((l) => (l ? contIndent + l : '')).join('\n')
            : '');
      } else if (seg.kind === 'list') {
        out += (out ? '\n' : indent + marker.trimEnd() + '\n') + seg.text;
      } else if (isFenceBlock) {
        const lines = cleaned.split('\n');
        const formatted = lines
          .map((l, idx) =>
            idx === 0 || (idx === lines.length - 1 && /^`{3,}$/.test(l)) ? contIndent + l : l,
          )
          .join('\n');
        out += (out ? '\n' : indent + marker.trimEnd() + '\n') + formatted;
      } else {
        const lines = cleaned.split('\n');
        const indented = lines.map((l) => (l ? contIndent + l : '')).join('\n');
        out += (out ? '\n' : indent + marker.trimEnd() + '\n') + indented;
      }
    }
    return out;
  };

  const serializeList = (listEl, ordered, ctx) => {
    const depth = ctx.listDepth || 0;
    const indent = '  '.repeat(depth);
    const start = ordered ? Number(listEl.getAttribute('start')) || 1 : 1;
    const items = Array.from(listEl.children).filter(
      (c) => c.tagName && c.tagName.toLowerCase() === 'li',
    );
    const out = [];
    for (let i = 0; i < items.length; i += 1) {
      const li = items[i];
      if (shouldSkipElement(li)) continue;
      const box =
        li.querySelector &&
        li.querySelector(':scope > input[type="checkbox"], :scope > p > input[type="checkbox"]');
      const isTask =
        Boolean(box) || (li.classList && li.classList.contains('ds-markdown-task-list-item'));
      let marker;
      if (isTask) {
        const checked = box
          ? Boolean(box.checked || box.hasAttribute('checked'))
          : /^[☑✅]/.test(cleanText(li.textContent || '').trim());
        marker = checked ? '- [x] ' : '- [ ] ';
      } else if (ordered) {
        marker = String(start + i) + '. ';
      } else {
        marker = '- ';
      }
      const rendered = serializeListItem(li, marker, indent, ctx);
      if (rendered) out.push(rendered);
    }
    return out.join('\n');
  };

  const serializeBlock = (node, ctx) => {
    if (!node) return '';
    if (node.nodeType === 3) {
      return cleanText(node.nodeValue || '').replace(/[ \t]*\n[ \t]*/g, ' ').trim();
    }
    if (node.nodeType !== 1) return '';
    const el = node;
    if (shouldSkipElement(el)) return '';
    if (isMathContainer(el)) return serializeMath(el);
    if (isCodeBlockContainer(el)) return serializeCodeBlock(el);

    const tag = el.tagName.toLowerCase();
    if (/^h[1-6]$/.test(tag)) {
      if (ctx.inListItem) {
        return wrapInline(el, '**', 'inStrong', ctx).trim();
      }
      const level = Number(tag[1]);
      const body = serializeInlineNodes(el.childNodes, ctx).replace(/\s+/g, ' ').trim();
      return body ? '#'.repeat(level) + ' ' + body : '';
    }
    if (tag === 'p') {
      return serializeInlineNodes(el.childNodes, ctx).replace(/^[ \t]+/, '').replace(/[ \t]+$/, '');
    }
    if (tag === 'ul' || tag === 'ol') {
      return serializeList(el, tag === 'ol', ctx);
    }
    if (tag === 'li') {
      return serializeListItem(el, '- ', '  '.repeat(ctx.listDepth || 0), ctx);
    }
    if (tag === 'blockquote') {
      const inner = serializeChildrenAsBlocks(el, ctx);
      if (!inner) return '';
      return inner
        .split('\n')
        .map((line) => (line ? '> ' + line : '>'))
        .join('\n');
    }
    if (tag === 'table') {
      return serializeTable(el, ctx);
    }
    if (tag === 'hr') {
      return '---';
    }
    if (isTransparentContainer(el)) {
      return serializeChildrenAsBlocks(el, ctx);
    }
    return serializeInlineNode(el, ctx).trim();
  };

  const serializeChildrenAsBlocks = (container, ctx) => {
    const blocks = [];
    let inlineRun = [];
    const flushInlineRun = () => {
      if (inlineRun.length === 0) return;
      const text = serializeInlineNodes(inlineRun, ctx).replace(/^[ \t]+/, '').replace(/[ \t]+$/, '');
      inlineRun = [];
      if (text) blocks.push(text);
    };

    for (let child = container.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 1 && shouldSkipElement(child)) continue;
      if (
        child.nodeType === 1 &&
        (isBlockElement(child) || isTransparentContainer(child))
      ) {
        flushInlineRun();
        const block = serializeBlock(child, ctx);
        if (block) blocks.push(block);
      } else {
        inlineRun.push(child);
      }
    }
    flushInlineRun();
    return blocks.join(ctx.inListItem ? '\n' : '\n\n');
  };

  const result = serializeBlock(root, {
    listDepth: 0,
    inListItem: false,
    inTable: false,
    inStrong: false,
    inEm: false,
    inStrike: false,
    inLink: false,
    inCode: false,
  });

  const normalized = result
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/^\n+/, '');
  return streaming ? normalized : normalized.replace(/\n+$/, '');
}

const LX_HTML_TO_MARKDOWN_SOURCE = lxHtmlToMarkdown.toString();

async function installDomMarkdown() {
  await js(`window.__lxHtmlToMarkdown = ${LX_HTML_TO_MARKDOWN_SOURCE};`).catch(() => {});
}
