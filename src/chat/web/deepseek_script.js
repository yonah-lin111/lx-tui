// DeepSeek web chat stream driver for `ego-browser nodejs`.
//
// Requires `deepseek_helpers.js` to be prepended ahead of this file.

async function main() {
  await activateTaskSpace();
  const current = await pageInfo().catch(() => null);
  const currentUrl = (current && current.url) || '';
  if (!currentUrl.startsWith(HOME)) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
  }

  const loggedIn = await js(
    "Boolean(localStorage.getItem('userToken')) && !location.pathname.startsWith('/sign_in')",
  );
  if (!loggedIn) {
    await openOrReuseTab(SIGN_IN, { wait: true, timeout: 20 });
    await handOffTaskSpace();
    await emit({ event: 'login_required', url: SIGN_IN });
    return;
  }

  if (ACTION !== 'chat') {
    await runNonChatAction(currentUrl);
    return;
  }

  if (FIRST_TURN && EDIT_INDEX < 0) {
    await ensureFreshChat();
  } else if (SESSION_ID && !currentUrl.includes(SESSION_ID)) {
    await openOrReuseTab(HOME + '/a/chat/s/' + SESSION_ID, { wait: true, timeout: 25 });
  }

  if (!(await ensureDeepSeekComposer())) {
    throw new Error('DeepSeek composer not found');
  }

  // Sync DeepThink ("深度思考") toggle state with THINKING ('deepthink' vs 'off').
  const toggledDeepThink = await js(`
    (() => {
      const wantDeepThink = ${JSON.stringify(THINKING)} !== 'off';
      const btn = Array.from(document.querySelectorAll('.ds-toggle-button')).find((el) => {
        const label = (el.innerText || '').trim();
        return label === '深度思考' || label === 'DeepThink';
      });
      if (!btn) return false;
      const isSelected = btn.className.includes('ds-toggle-button--selected');
      if (isSelected !== wantDeepThink) {
        btn.click();
        return true;
      }
      return false;
    })()
  `).catch(() => false);
  if (toggledDeepThink) {
    await new Promise((r) => setTimeout(r, 300));
  }

  await installDomMarkdown();

  // Channel A: intercept /api/v0/chat/completion over XMLHttpRequest & fetch.
  const runId = await js(String.raw`
    (() => {
      const existingKeys = Array.from(
        document.querySelectorAll('[data-virtual-list-item-key]'),
      ).map((el) => el.getAttribute('data-virtual-list-item-key') || '');
      const stream = {
        runId: String(Date.now()) + '-' + String(Math.random()),
        queue: [],
        done: false,
        sseActive: false,
        sseSeen: false,
        sseSeenAt: 0,
        sseText: false,
        sseThinking: false,
        domTextStarted: false,
        domThinkingStarted: false,
        submittedAt: Date.now(),
        activeXhr: null,
        deltaState: { op: 'SET', path: '' },
        fragments: [],
        thinkingContent: '',
        content: '',
        domThink: '',
        domText: '',
        domLastSample: '',
        domStableTicks: 0,
        seenKeys: new Set(existingKeys),
        targetKey: null,
        sseBuffer: '',
        sseEventName: '',
        sseDataLines: [],
        cleanPlain(text, holdIncomplete) {
          if (!text || typeof text !== 'string') return '';
          let out = text.replace(/\[(?:citation|reference):[^\]]*\]/g, '');
          if (holdIncomplete) {
            const tail = out.lastIndexOf('[');
            if (tail >= 0 && out.indexOf(']', tail) < 0) {
              const suffix = out.slice(tail + 1).toLowerCase();
              if (
                'reference:'.startsWith(suffix)
                || 'citation:'.startsWith(suffix)
                || suffix.startsWith('reference:')
                || suffix.startsWith('citation:')
              ) {
                out = out.slice(0, tail);
              }
            }
          }
          return out;
        },
        cleanText(text, holdIncomplete) {
          if (!text || typeof text !== 'string') return '';
          let out = '';
          let last = 0;
          const fence = new RegExp('\\x60{3}[\\s\\S]*?(?:\\x60{3}|$)', 'g');
          let match;
          while ((match = fence.exec(text)) !== null) {
            out += this.cleanPlain(text.slice(last, match.index), false);
            out += match[0];
            last = match.index + match[0].length;
          }
          out += this.cleanPlain(text.slice(last), holdIncomplete);
          return out;
        },
        push(kind, delta) {
          if (!kind || !delta) return;
          if (kind === 'text' && this.domTextStarted) return;
          if (kind === 'thinking' && this.domThinkingStarted) return;
          this.sseActive = true;
          if (kind === 'text') this.sseText = true;
          if (kind === 'thinking') this.sseThinking = true;
          this.queue.push({ kind: kind, delta: delta });
        },
        pushDom(kind, current) {
          if (kind === 'text' && this.sseText) return;
          if (kind === 'thinking' && this.sseThinking) return;
          const key = kind === 'thinking' ? 'domThink' : 'domText';
          const previous = this[key] || '';
          if (!current || current === previous) return;
          const emitDelta = (delta) => {
            if (!delta) return;
            if (kind === 'text') this.domTextStarted = true;
            if (kind === 'thinking') this.domThinkingStarted = true;
            this.queue.push({ kind: kind, delta: delta });
          };
          if (current.startsWith(previous)) {
            const delta = current.slice(previous.length);
            this[key] = current;
            emitDelta(delta);
            return;
          }
          for (const len of [48, 32, 20, 12, 8, 5]) {
            if (previous.length < len) continue;
            const anchor = previous.slice(previous.length - len);
            const pos = current.lastIndexOf(anchor);
            if (pos >= 0 && pos + len >= current.length - 240) {
              const delta = current.slice(pos + len);
              if (delta.length > 0 || current.length >= previous.length) {
                this[key] = current;
              }
              emitDelta(delta);
              return;
            }
          }
          const tailLen = Math.min(20, current.length);
          if (tailLen >= 5 && previous.includes(current.slice(current.length - tailLen))) {
            return;
          }
          let lcp = 0;
          while (lcp < current.length && lcp < previous.length && current[lcp] === previous[lcp]) {
            lcp += 1;
          }
          if (previous.length - lcp <= 40) {
            const remPrev = previous.slice(lcp).replace(/^[*_~\s\x60]+/, '');
            if (remPrev.length === 0) {
              const delta = current.slice(lcp);
              this[key] = current;
              emitDelta(delta);
              return;
            }
            const idx = current.indexOf(remPrev, lcp);
            if (idx >= lcp && idx - lcp <= 12) {
              const delta = current.slice(idx + remPrev.length);
              this[key] = current;
              emitDelta(delta);
              return;
            }
          }
          if (current.length > previous.length) {
            this[key] = current;
          }
        },
        fragmentKind(fragment) {
          if (!fragment) return null;
          const type = String(fragment.type || '').toUpperCase();
          if (type === 'THINK' || type === 'THINKING' || type === 'REASONING') return 'thinking';
          if (!type || type === 'RESPONSE' || type === 'TEXT' || type === 'TEMPLATE_RESPONSE') {
            return 'text';
          }
          return null;
        },
        parseDeltas(raw, state) {
          if (!raw || typeof raw !== 'object') return [];
          const target = state || this.deltaState;
          const op = raw.o !== undefined ? raw.o : target.op;
          let path = raw.p !== undefined ? raw.p : target.path;
          if (typeof path === 'string' && ! state) {
            if (path.startsWith('/')) path = path.slice(1);
            if (path && path !== 'response' && !path.startsWith('response/')) {
              path = 'response/' + path;
            }
          }
          target.op = op;
          target.path = path;
          if (op !== 'BATCH') {
            return [{ path: path || '', op: op || 'SET', value: raw.v }];
          }
          if (!Array.isArray(raw.v)) return [];
          const childState = { op: 'SET', path: '' };
          const out = [];
          for (const entry of raw.v) {
            const nested = this.parseDeltas(entry, childState);
            for (const item of nested) {
              let fullPath = (path ? path + '/' : '') + item.path;
              if (fullPath.startsWith('/')) fullPath = fullPath.slice(1);
              if (fullPath && fullPath !== 'response' && !fullPath.startsWith('response/')) {
                fullPath = 'response/' + fullPath;
              }
              out.push({
                path: fullPath,
                op: item.op,
                value: item.value,
              });
            }
          }
          return out;
        },
        applyEvent(event) {
          if (!event || typeof event !== 'object') return;
          if (Array.isArray(event)) {
            for (const item of event) this.applyEvent(item);
            return;
          }
          for (const patch of this.parseDeltas(event, this.deltaState)) {
            if (patch.value !== undefined) {
              this.applyValue(patch.path, patch.op, patch.value);
            }
          }
        },
        applyValue(path, op, value) {
          if (value && typeof value === 'object' && !Array.isArray(value) && value.response) {
            this.deltaState.path = 'response';
            this.deltaState.op = 'SET';
            this.applyValue('response', 'SET', value.response);
            return;
          }
          if (typeof path !== 'string') return;
          if (path === 'response') {
            if (!value || typeof value !== 'object') return;
            const hasFragments = Array.isArray(value.fragments) && value.fragments.length > 0;
            if (value.fragments !== undefined) this.applyFragments('SET', value.fragments);
            if (!hasFragments) {
              if (value.thinking_content !== undefined) {
                this.applyScalar('thinking', op, value.thinking_content);
              }
              if (value.content !== undefined) this.applyScalar('text', op, value.content);
            }
            if (value.status === 'FINISHED') this.done = true;
            return;
          }
          if (path === 'response/status' && value === 'FINISHED') {
            this.done = true;
            return;
          }
          if (path === 'response/fragments') {
            this.applyFragments(op, value);
            return;
          }
          const parts = path.split('/');
          if (parts.length === 4 && parts[0] === 'response' && parts[1] === 'fragments') {
            const rawIndex = Number(parts[2]);
            if (!Number.isNaN(rawIndex)) {
              const index = rawIndex < 0 ? Math.max(0, this.fragments.length + rawIndex) : rawIndex;
              const field = parts[3];
              const fragment = this.fragments[index] || (this.fragments[index] = { content: '' });
              if (field === 'content' || field === 'thinking_content') {
                if (field === 'thinking_content' && !fragment.type) fragment.type = 'THINK';
                this.applyFragmentContent(index, op, value);
              } else if (field === 'type') {
                fragment.type = value;
              } else {
                fragment[field] = value;
              }
              return;
            }
          }
          if (path === 'response/thinking_content') {
            this.applyScalar('thinking', op, value);
            return;
          }
          if (path === 'response/content') {
            this.applyScalar('text', op, value);
            return;
          }
          if (
            typeof value === 'string'
            && (path === '' || path === 'response' || path === 'response/fragments')
          ) {
            const index = this.fragments.length - 1;
            if (index >= 0) {
              this.applyFragmentContent(index, op === 'SET' ? 'SET' : 'APPEND', value);
              return;
            }
            this.applyScalar('text', op, value);
          }
        },
        applyScalar(kind, op, value) {
          if (typeof value !== 'string') return;
          const key = kind === 'thinking' ? 'thinkingContent' : 'content';
          const previous = this[key] || '';
          let delta = '';
          if (op === 'APPEND') {
            delta = value;
          } else if (op === 'SET') {
            delta = value.startsWith(previous) ? value.slice(previous.length) : value;
          } else {
            delta = value;
          }
          this[key] = op === 'APPEND' ? previous + value : value;
          this.push(kind, delta);
        },
        applyFragments(op, value) {
          if (!value) return;
          if (!Array.isArray(value)) value = [value];
          if (op === 'APPEND') {
            for (const fragment of value) {
              if (!fragment || typeof fragment !== 'object') continue;
              const index = this.fragments.length;
              const entry = {
                type: fragment.type || 'RESPONSE',
                content: '',
                emitted: '',
                sepEmitted: false,
              };
              this.fragments.push(entry);
              const text = typeof fragment.content === 'string'
                ? fragment.content
                : (typeof fragment.thinking_content === 'string' ? fragment.thinking_content : '');
              if (text) {
                this.applyFragmentContent(index, 'SET', text);
              }
            }
            return;
          }
          value.forEach((fragment, index) => {
            if (!fragment || typeof fragment !== 'object') return;
            const current = this.fragments[index] || {
              type: fragment.type || 'RESPONSE',
              content: '',
              emitted: '',
              sepEmitted: false,
            };
            if (fragment.type) current.type = fragment.type;
            this.fragments[index] = current;
            const text = typeof fragment.content === 'string'
              ? fragment.content
              : (typeof fragment.thinking_content === 'string' ? fragment.thinking_content : null);
            if (typeof text === 'string') {
              this.applyFragmentContent(index, 'SET', text);
            }
          });
          if (this.fragments.length > value.length) {
            this.fragments.length = value.length;
          }
        },
        applyFragmentContent(index, op, value) {
          if (typeof value !== 'string') return;
          const fragment = this.fragments[index] || (this.fragments[index] = {
            content: '',
            emitted: '',
            sepEmitted: false,
          });
          const previousRaw = typeof fragment.content === 'string' ? fragment.content : '';
          const nextRaw = op === 'APPEND' ? previousRaw + value : value;
          fragment.content = nextRaw;
          const kind = this.fragmentKind(fragment);
          if (!kind) return;
          const cleaned = kind === 'text' ? this.cleanText(nextRaw, true) : nextRaw;
          const previousEmitted = typeof fragment.emitted === 'string' ? fragment.emitted : '';
          let delta = '';
          if (cleaned.startsWith(previousEmitted)) {
            delta = cleaned.slice(previousEmitted.length);
          } else {
            let pos = 0;
            while (
              pos < cleaned.length
              && pos < previousEmitted.length
              && cleaned[pos] === previousEmitted[pos]
            ) {
              pos += 1;
            }
            delta = cleaned.slice(pos);
          }
          fragment.emitted = cleaned;
          if (!delta) return;
          if (kind === 'thinking' && !fragment.sepEmitted) {
            fragment.sepEmitted = true;
            const hasEarlierThink = this.fragments
              .slice(0, index)
              .some((item) => item && this.fragmentKind(item) === 'thinking' && item.emitted);
            if (hasEarlierThink) {
              delta = '\n\n' + delta;
            }
          }
          this.push(kind, delta);
        },
        flushSseFrame() {
          const data = this.sseDataLines.join('\n').trim();
          const name = this.sseEventName;
          this.sseDataLines = [];
          this.sseEventName = '';
          if (name === 'finish' || name === 'close') this.done = true;
          if (!data) return;
          if (data === '[DONE]') {
            this.done = true;
            return;
          }
          try {
            if (!name || name === 'delta' || name === 'message') {
              this.applyEvent(JSON.parse(data));
            }
          } catch (error) {
            // Ignore keep-alive and non-patch frames.
          }
        },
        feedSseChunk(chunk, isFinal) {
          if (chunk) this.sseBuffer += chunk;
          const trimLeadingSpace = (s) => (s.startsWith(' ') ? s.slice(1) : s);
          let index;
          while ((index = this.sseBuffer.indexOf('\n')) >= 0) {
            let line = this.sseBuffer.slice(0, index);
            this.sseBuffer = this.sseBuffer.slice(index + 1);
            if (line.endsWith('\r')) line = line.slice(0, -1);
            if (!line) {
              this.flushSseFrame();
              continue;
            }
            if (line.startsWith('event:')) {
              this.sseEventName = line.slice(6).trim();
            } else if (line.startsWith('data:')) {
              if (!this.sseSeen) {
                this.sseSeen = true;
                this.sseSeenAt = Date.now();
              }
              this.sseActive = true;
              this.sseDataLines.push(trimLeadingSpace(line.slice(5)));
            }
          }
          if (isFinal) {
            if (this.sseBuffer.trim()) {
              if (this.sseBuffer.startsWith('data:')) {
                if (!this.sseSeen) {
                  this.sseSeen = true;
                  this.sseSeenAt = Date.now();
                }
                this.sseActive = true;
                this.sseDataLines.push(trimLeadingSpace(this.sseBuffer.slice(5)));
              } else if (this.sseBuffer.startsWith('event:')) {
                this.sseEventName = this.sseBuffer.slice(6).trim();
              }
              this.sseBuffer = '';
            }
            this.flushSseFrame();
            if (this.sseActive) this.done = true;
          }
        },
        attachXhr(xhr) {
          if (!xhr || xhr.__lxAttached) return;
          xhr.__lxAttached = true;
          let consumed = 0;
          const self = this;
          const pull = (isFinal) => {
            try {
              if (xhr.status >= 400) return;
              const type = String(
                (xhr.getResponseHeader && xhr.getResponseHeader('content-type')) || '',
              ).toLowerCase();
              const full = typeof xhr.responseText === 'string' ? xhr.responseText : '';
              if (!full) {
                if (isFinal && self.activeXhr === xhr && self.sseActive) {
                  self.feedSseChunk('', true);
                }
                return;
              }
              // Non-SSE challenge / JSON responses must not lock out replayed SSE requests.
              const looksSse = /(?:^|\n)(?:event|data|:)/.test(full.slice(0, 256));
              if (type && !type.includes('text/event-stream') && !looksSse) return;
              if (!type && !looksSse) return;
              if (self.activeXhr && self.activeXhr !== xhr && (self.sseText || self.sseThinking)) {
                return;
              }
              self.activeXhr = xhr;
              if (full.length > consumed) {
                const delta = full.slice(consumed);
                consumed = full.length;
                self.feedSseChunk(delta, isFinal);
              } else if (isFinal) {
                self.feedSseChunk('', true);
              }
            } catch (e) {}
          };
          xhr.addEventListener('progress', () => pull(false));
          xhr.addEventListener('readystatechange', () => {
            if (xhr.readyState === 3) pull(false);
            if (xhr.readyState === 4) pull(true);
          });
          xhr.addEventListener('loadend', () => pull(true));
        },
        attach(response) {
          if (!response || !response.ok || !response.body) return;
          const type = response.headers && response.headers.get
            ? (response.headers.get('content-type') || '').toLowerCase()
            : '';
          if (type && !type.includes('text/event-stream')) return;
          if (!this.sseSeen) {
            this.sseSeen = true;
            this.sseSeenAt = Date.now();
          }
          const reader = response.body.getReader();
          const decoder = new TextDecoder();
          const self = this;
          const pump = () => reader.read().then((chunk) => {
            if (chunk.done) {
              self.feedSseChunk('', true);
              return;
            }
            self.feedSseChunk(decoder.decode(chunk.value, { stream: true }), false);
            return pump();
          }).catch(() => {
            if (self.sseActive) self.done = true;
          });
          pump();
        },
        readFiberMarkdown(el) {
          if (!el) return '';
          const key = Object.keys(el).find(
            (k) => k.startsWith('__reactFiber$') || k.startsWith('__reactInternalInstance$'),
          );
          let fiber = key ? el[key] : null;
          for (let depth = 0; fiber && depth < 20; depth += 1, fiber = fiber.return) {
            const props = fiber.memoizedProps;
            if (!props || typeof props !== 'object') continue;
            if (
              'citeProps' in props
              || 'cacheKey' in props
              || 'mergeConsecutiveCites' in props
            ) {
              if (typeof props.content === 'string') return props.content;
              if (typeof props.markdown === 'string') return props.markdown;
            }
          }
          return '';
        },
        tickDom() {
          if (this.sseText && (this.sseThinking || THINKING === 'off')) return;
          const items = Array.from(document.querySelectorAll('[data-virtual-list-item-key]'));
          let target = null;
          if (this.targetKey) {
            target = items.find(
              (el) => el.getAttribute('data-virtual-list-item-key') === this.targetKey,
            ) || null;
          } else {
            for (let index = items.length - 1; index >= 0; index -= 1) {
              const el = items[index];
              const key = el.getAttribute('data-virtual-list-item-key') || '';
              if (this.seenKeys.has(key)) continue;
              if (el.querySelector('.ds-markdown, .ds-think-content')) {
                this.targetKey = key;
                target = el;
                break;
              }
            }
          }
          if (!target) return;
          const idleButton = Boolean(
            document.querySelector('.ds-button--primary.ds-button--disabled'),
          );
          const stopIcon = Boolean(document.querySelector('.ds-icon-stop'));
          const isStreaming = stopIcon || !idleButton;
          const readMarkdownText = (el) => {
            if (!el) return '';
            const fiberMd = this.readFiberMarkdown(el);
            if (fiberMd) {
              return this.cleanText(fiberMd, isStreaming);
            }
            if (typeof window.__lxHtmlToMarkdown === 'function') {
              return this.cleanText(
                window.__lxHtmlToMarkdown(el, { streaming: isStreaming }),
                false,
              );
            }
            return '';
          };
          const now = Date.now();
          if (!this.pollStartedAt) this.pollStartedAt = now;
          const graceOver = this.sseSeen
            ? now - this.sseSeenAt >= 1200
            : now - this.pollStartedAt >= 1200;
          if (graceOver || !isStreaming) {
            const thinkEl =
              target.querySelector('.ds-think-content .ds-markdown')
              || target.querySelector('.ds-think-content');
            const textEl =
              target.querySelector('.ds-markdown.ds-assistant-message-main-content')
              || Array.from(target.querySelectorAll('.ds-markdown')).find(
                (el) => !el.closest('.ds-think-content'),
              )
              || null;
            if (!this.sseThinking) {
              this.pushDom('thinking', readMarkdownText(thinkEl));
            }
            if (!this.sseText) {
              this.pushDom('text', readMarkdownText(textEl));
            }
          }
          const sample = (this.domThink || '') + ':::' + (this.domText || '');
          if (sample === this.domLastSample) {
            this.domStableTicks += 1;
          } else {
            this.domLastSample = sample;
            this.domStableTicks = 0;
          }
          if (
            this.domStableTicks >= 6
            && idleButton
            && !stopIcon
            && (this.domThink || this.domText)
          ) {
            this.done = true;
          }
        },
      };
      window.__lxDeepSeekStream = stream;
      const urlOf = (input) => {
        if (typeof input === 'string') return input;
        if (input && typeof input.url === 'string') return input.url;
        if (input && typeof input.href === 'string') return input.href;
        try {
          return String(input || '');
        } catch (_) {
          return '';
        }
      };
      const isStreamUrl = (u) => typeof u === 'string' && (
        u.includes('/api/v0/chat/completion')
        || u.includes('/api/v0/chat/edit_message')
        || u.includes('/api/v0/chat/regenerate')
        || u.includes('/api/v0/chat/continue')
        || u.includes('/api/v0/chat/resume_stream')
      );
      if (!XMLHttpRequest.prototype.open.__lxWrapped) {
        const origOpen = XMLHttpRequest.prototype.open;
        const origSend = XMLHttpRequest.prototype.send;
        const wrappedOpen = function(method, url) {
          this.__lxUrl = urlOf(url);
          return origOpen.apply(this, arguments);
        };
        wrappedOpen.__lxWrapped = true;
        XMLHttpRequest.prototype.open = wrappedOpen;
        const wrappedSend = function() {
          if (isStreamUrl(this.__lxUrl)) {
            const current = window.__lxDeepSeekStream;
            if (current) {
              try {
                current.attachXhr(this);
              } catch (e) {}
            }
          }
          return origSend.apply(this, arguments);
        };
        wrappedSend.__lxWrapped = true;
        XMLHttpRequest.prototype.send = wrappedSend;
      }
      if (typeof window.fetch === 'function' && !window.fetch.__lxWrapped) {
        const originalFetch = window.fetch.bind(window);
        const wrappedFetch = function(input, init) {
          const url = urlOf(input);
          const result = originalFetch(input, init);
          if (isStreamUrl(url)) {
            result.then((response) => {
              const current = window.__lxDeepSeekStream;
              if (!current) return;
              try {
                current.attach(response.clone());
              } catch (error) {
                // A non-clonable body simply falls back to the DOM channel.
              }
            }).catch(() => {});
          }
          return result;
        };
        wrappedFetch.__lxWrapped = true;
        window.fetch = wrappedFetch;
      }
      return stream.runId;
    })()
  `);

  let submittedViaEdit = false;
  if (EDIT_INDEX >= 0) {
    const openedEdit = await js(`
      (() => {
        const items = Array.from(document.querySelectorAll('[data-virtual-list-item-key]'));
        const userItems = items.filter((el) => !el.querySelector('.ds-markdown, .ds-think-content'));
        const target = userItems[${EDIT_INDEX}];
        if (!target) return { status: 'missing_turn' };
        const btns = Array.from(target.querySelectorAll('.ds-button, button, [role="button"]'));
        const editBtn = btns[1] || btns[btns.length - 1];
        if (!editBtn) return { status: 'no_button' };
        const propKey = Object.keys(editBtn).find((k) => k.startsWith('__reactProps'));
        if (propKey && editBtn[propKey] && typeof editBtn[propKey].onClick === 'function') {
          editBtn[propKey].onClick({ stopPropagation() {}, preventDefault() {} });
        } else {
          editBtn.click();
        }
        return { status: 'clicked' };
      })()
    `).catch(() => ({ status: 'error' }));
    if (openedEdit && openedEdit.status === 'missing_turn') {
      await emit({ event: 'error', message: 'DeepSeek /rewind target turn not found' });
      return;
    }
    if (!openedEdit || openedEdit.status !== 'clicked') {
      await emit({ event: 'error', message: 'DeepSeek edit button not found for /rewind' });
      return;
    }
    await new Promise((r) => setTimeout(r, 120));
    await syncContainerAttachments(true);
    submittedViaEdit = await js(`
      (() => {
        const editArea = document.querySelector('textarea[name="user query"]');
        if (!editArea) return false;
        editArea.focus();
        const descriptor = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value');
        if (descriptor && descriptor.set) descriptor.set.call(editArea, ${JSON.stringify(PROMPT)});
        else editArea.value = ${JSON.stringify(PROMPT)};
        editArea.dispatchEvent(new Event('input', { bubbles: true }));
        editArea.dispatchEvent(new Event('change', { bubbles: true }));
        const container = editArea.closest('[data-virtual-list-item-key]') || document;
        const sendBtn = Array.from(container.querySelectorAll('.ds-button, button, [role="button"]'))
          .find((b) => {
            const label = (b.innerText || '').trim();
            return label === '发送' || label === 'Send';
          });
        if (!sendBtn) return false;
        const sendKey = Object.keys(sendBtn).find((k) => k.startsWith('__reactProps'));
        if (sendKey && sendBtn[sendKey] && typeof sendBtn[sendKey].onClick === 'function') {
          sendBtn[sendKey].onClick({ stopPropagation() {}, preventDefault() {} });
        } else {
          sendBtn.click();
        }
        return true;
      })()
    `).catch(() => false);
    if (!submittedViaEdit) {
      throw new Error('Failed to edit message in DeepSeek Web');
    }
  }

  if (!submittedViaEdit) {
    await syncContainerAttachments(false);
    const filled = await js(`
      (() => {
        const textarea = document.querySelector('textarea[name="search"]') || document.querySelector('textarea');
        if (!textarea) return false;
        textarea.focus();
        const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
        setter.call(textarea, ${JSON.stringify(PROMPT)});
        textarea.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      })()
    `);
    if (!filled) throw new Error('DeepSeek composer not found');

    const submitDeadline = Date.now() + 8000;
    let submitted = false;
    while (Date.now() < submitDeadline) {
      const cleared = await js(`
        (() => {
          const stream = window.__lxDeepSeekStream;
          if (stream && stream.sseActive) return true;
          const textarea = document.querySelector('textarea[name="search"]') || document.querySelector('textarea');
          const textEmpty = !textarea || !textarea.value.trim();
          const composerChips = Array.from(document.querySelectorAll('.ds-animated-size-item'))
            .filter((el) => !el.closest('[data-virtual-list-item-key]'));
          const stopIcon = Boolean(document.querySelector('.ds-icon-stop'));
          if (stopIcon) return true;
          if (${FILES.length > 0 ? 'true' : 'false'}) {
            return textEmpty && composerChips.length === 0;
          }
          return textEmpty && ${PROMPT ? 'true' : 'false'};
        })()
      `).catch(() => false);
      if (cleared) {
        submitted = true;
        break;
      }
      await js(`
        (() => {
          const button = document.querySelector('.ds-button--primary:not(.ds-button--disabled)');
          if (button && button.getAttribute('aria-disabled') !== 'true' && !button.querySelector('.ds-icon-stop')) {
            const pk = Object.keys(button).find((k) => k.startsWith('__reactProps'));
            if (pk && button[pk] && typeof button[pk].onClick === 'function') {
              button[pk].onClick({ stopPropagation() {}, preventDefault() {} });
            } else {
              button.click();
            }
            return true;
          }
          const textarea = document.querySelector('textarea[name="search"]') || document.querySelector('textarea');
          if (textarea) {
            const keyboard = { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true };
            textarea.dispatchEvent(new KeyboardEvent('keydown', keyboard));
            textarea.dispatchEvent(new KeyboardEvent('keyup', keyboard));
            return true;
          }
          return false;
        })()
      `).catch(() => false);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    if (!submitted) {
      throw new Error('DeepSeek failed to submit message');
    }
  }

  const deadline = Date.now() + 300000;
  let lastHeartbeat = Date.now();
  while (Date.now() < deadline && !STREAM_ABORTED) {
    await ensureTaskSpaceControl();
    const snapshot = await js(`
      (() => {
        const stream = window.__lxDeepSeekStream;
        if (!stream) return { superseded: true };
        if (stream.runId !== ${JSON.stringify(runId)}) return { superseded: true };
        stream.tickDom();
        return { queue: stream.queue.splice(0, stream.queue.length), done: stream.done, superseded: false };
      })()
    `);
    if (snapshot && snapshot.superseded) {
      await emit({ event: 'error', message: 'DeepSeek stream was superseded by another pane' });
      return;
    }
    if (!snapshot) throw new Error('DeepSeek stream collector was lost');
    if (snapshot.queue.length > 0) {
      lastHeartbeat = Date.now();
      await emitMany(snapshot.queue.map((item) => ({ event: item.kind, delta: item.delta })));
    } else if (Date.now() - lastHeartbeat >= 1000) {
      lastHeartbeat = Date.now();
      if (!(await checkStreamAlive())) {
        return;
      }
    }
    if (snapshot.done) {
      const sessionMeta = await js(String.raw`
        (() => {
          const match = location.pathname.match(/\/s\/([a-zA-Z0-9-]+)/);
          const id = match ? match[1] : '';
          if (!id) return null;
          const link = document.querySelector('a[href*="' + id + '"]');
          const title = link ? (link.innerText || '').trim() : '';
          return { id, title };
        })()
      `).catch(() => null);
      if (sessionMeta && sessionMeta.id) {
        await emit({ event: 'session', id: sessionMeta.id, title: sessionMeta.title || null });
      }
      await emit({ event: 'done' });
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 60));
  }
  throw new Error('timed out waiting for DeepSeek to finish the reply');
}

try {
  await main();
} catch (error) {
  const raw = String((error && error.message) || error || 'DeepSeek web driver error');
  const firstLine = raw.split('\n').map((s) => s.trim()).find(Boolean) || raw;
  await emit({ event: 'error', message: firstLine });
}
