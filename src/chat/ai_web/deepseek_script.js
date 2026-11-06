// DeepSeek web chat driver for `ego-browser nodejs`.
//
// The Rust event layer feeds the action/prompt through the environment (and
// prepends assignments when ego-browser sanitizes process.env), then reads the
// `@@LX_AI_WEB@@` NDJSON lines emitted below.

const PREFIX = '@@LX_AI_WEB@@';
const SPACE = 'lx-tui-ai-web-deepseek';
const HOME = 'https://chat.deepseek.com';
const SIGN_IN = 'https://chat.deepseek.com/sign_in';
const ACTION = process.env.LX_AI_WEB_ACTION || 'chat';
const PROMPT = process.env.LX_AI_WEB_PROMPT || '';
const FIRST_TURN = process.env.LX_AI_WEB_FIRST_TURN === '1';
const STREAM_URL = process.env.LX_AI_WEB_STREAM_URL || '';

async function emitMany(objs) {
  if (!objs || objs.length === 0) return;
  const payload = objs.map((obj) => PREFIX + JSON.stringify(obj) + '\n').join('');
  if (STREAM_URL) {
    try {
      await fetch(STREAM_URL, { method: 'POST', body: payload });
      return;
    } catch (e) {}
  }
  process.stdout.write(payload);
}

async function emit(obj) {
  await emitMany([obj]);
}

async function main() {
  await useOrCreateTaskSpace(SPACE);
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

  if (ACTION === 'new') {
    await js(`
      (() => {
        if (location.pathname === '/' && document.querySelectorAll('.ds-message').length === 0) {
          return false;
        }
        const target = Array.from(document.querySelectorAll('button, a, div, span')).find((el) => {
          const label = (el.innerText || '').trim();
          return label === '开启新对话' || label === 'New chat';
        });
        if (!target) return false;
        (target.closest('button, a') || target).click();
        return true;
      })()
    `);
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'stop') {
    await js(String.raw`
      (() => {
        const labelled = Array.from(document.querySelectorAll('button, [role="button"], .ds-button')).find((el) => {
          const text = (el.getAttribute('aria-label') || '') + ' ' + (el.title || '');
          return text.toLowerCase().includes('stop') || text.includes('停止');
        });
        const button = document.querySelector('.ds-button--primary .ds-icon-stop')?.closest('button, [role="button"], .ds-button')
          || labelled
          || document.querySelector('.ds-icon-stop')?.closest('button, [role="button"], .ds-button')
          || document.querySelector('.ds-button--primary:not(.ds-button--disabled)');
        if (!button) return false;
        button.click();
        return true;
      })()
    `);
    await emit({ event: 'done' });
    return;
  }

  if (ACTION !== 'chat') {
    await emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
    return;
  }

  if (FIRST_TURN) {
    const clickedNew = await js(String.raw`
      (() => {
        if (location.pathname === '/' && document.querySelectorAll('.ds-message').length === 0) {
          return false;
        }
        const target = Array.from(document.querySelectorAll('button, a, div, span')).find((el) => {
          const label = (el.innerText || '').trim();
          return label === '开启新对话' || label === 'New chat';
        });
        if (!target) return false;
        (target.closest('button, a, [tabindex="0"]') || target).click();
        return true;
      })()
    `);
    if (clickedNew) {
      await new Promise((resolve) => setTimeout(resolve, 350));
    }
  }

  // Ensure DeepThink ("深度思考") is enabled so reasoning deltas are streamed.
  await js(String.raw`
    (() => {
      const btn = Array.from(document.querySelectorAll('.ds-toggle-button')).find((el) => {
        const label = (el.innerText || '').trim();
        return label === '深度思考' || label === 'DeepThink';
      });
      if (btn) {
        if (!btn.className.includes('ds-toggle-button--selected')) {
          btn.click();
        }
        return true;
      }
      return false;
    })()
  `);

  // Channel A: intercept /api/v0/chat/completion over XMLHttpRequest & fetch.
  await js(String.raw`
    (() => {
      const existingKeys = Array.from(
        document.querySelectorAll('[data-virtual-list-item-key]'),
      ).map((el) => el.getAttribute('data-virtual-list-item-key') || '');
      const stream = {
        queue: [],
        done: false,
        sseActive: false,
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
        cleanText(text, holdIncomplete) {
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
        push(kind, delta) {
          if (!kind || !delta) return;
          this.sseActive = true;
          this.queue.push({ kind: kind, delta: delta });
        },
        pushDom(kind, current) {
          const key = kind === 'thinking' ? 'domThink' : 'domText';
          const previous = this[key] || '';
          if (current === previous) return;
          let delta = '';
          if (current.length > previous.length && current.startsWith(previous)) {
            delta = current.slice(previous.length);
          } else {
            let index = 0;
            while (
              index < current.length
              && index < previous.length
              && current[index] === previous[index]
            ) {
              index += 1;
            }
            delta = current.slice(index);
          }
          this[key] = current;
          if (delta) this.queue.push({ kind: kind, delta: delta });
        },
        fragmentKind(fragment) {
          if (!fragment) return null;
          if (fragment.type === 'THINK') return 'thinking';
          if (!fragment.type || fragment.type === 'RESPONSE') return 'text';
          return null;
        },
        parseDeltas(raw, state) {
          if (!raw || typeof raw !== 'object') return [];
          const target = state || this.deltaState;
          const op = raw.o !== undefined ? raw.o : target.op;
          const path = raw.p !== undefined ? raw.p : target.path;
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
              out.push({
                path: (path ? path + '/' : '') + item.path,
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
            this.applyValue('response', op, value.response);
            return;
          }
          if (typeof path !== 'string') return;
          if (path === 'response') {
            if (!value || typeof value !== 'object') return;
            if (value.fragments !== undefined) this.applyFragments('SET', value.fragments);
            if (value.thinking_content !== undefined) {
              this.applyScalar('thinking', op, value.thinking_content);
            }
            if (value.content !== undefined) this.applyScalar('text', op, value.content);
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
              if (field === 'content') {
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
          if (!Array.isArray(value)) return;
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
              if (typeof fragment.content === 'string' && fragment.content) {
                this.applyFragmentContent(index, 'SET', fragment.content);
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
            if (typeof fragment.content === 'string') {
              this.applyFragmentContent(index, 'SET', fragment.content);
            }
          });
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
              .some((item) => item && item.type === 'THINK' && item.emitted);
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
            if (!name || name === 'delta') {
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
              this.sseDataLines.push(trimLeadingSpace(line.slice(5)));
            }
          }
          if (isFinal) {
            if (this.sseBuffer.trim()) {
              if (this.sseBuffer.startsWith('data:')) {
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
          if (!xhr) return;
          let consumed = 0;
          const self = this;
          const pull = (isFinal) => {
            try {
              const full = typeof xhr.responseText === 'string' ? xhr.responseText : '';
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
          if (!response || !response.body) return;
          const type = response.headers && response.headers.get
            ? (response.headers.get('content-type') || '')
            : '';
          if (type && !type.includes('text/event-stream')) return;
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
        tickDom() {
          if (this.sseActive) return;
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
          const think = target.querySelector('.ds-think-content');
          const text = target.querySelector('.ds-markdown');
          this.pushDom('thinking', think ? (think.innerText || '') : '');
          this.pushDom('text', text ? (text.innerText || '') : '');
          const sample = (this.domThink || '') + ':::' + (this.domText || '');
          if (sample === this.domLastSample) {
            this.domStableTicks += 1;
          } else {
            this.domLastSample = sample;
            this.domStableTicks = 0;
          }
          const idleButton = Boolean(
            document.querySelector('.ds-button--primary.ds-button--disabled'),
          );
          if (
            this.domStableTicks >= 6
            && idleButton
            && !document.querySelector('.ds-icon-stop')
            && (this.domThink || this.domText)
          ) {
            this.done = true;
          }
        },
      };
      window.__lxDeepSeekStream = stream;
      if (!window.__lxXhrWrapped) {
        window.__lxXhrWrapped = true;
        const origOpen = XMLHttpRequest.prototype.open;
        const origSend = XMLHttpRequest.prototype.send;
        XMLHttpRequest.prototype.open = function(method, url) {
          this.__lxUrl = typeof url === 'string' ? url : String(url || '');
          return origOpen.apply(this, arguments);
        };
        XMLHttpRequest.prototype.send = function() {
          if (this.__lxUrl && this.__lxUrl.includes('/api/v0/chat/completion')) {
            const current = window.__lxDeepSeekStream;
            if (current) {
              try {
                current.attachXhr(this);
              } catch (e) {}
            }
          }
          return origSend.apply(this, arguments);
        };
      }
      if (!window.__lxFetchWrapped) {
        window.__lxFetchWrapped = true;
        const originalFetch = window.fetch.bind(window);
        window.fetch = function(input, init) {
          const url = typeof input === 'string' ? input : (input && input.url) || '';
          const result = originalFetch(input, init);
          if (url.includes('/api/v0/chat/completion')) {
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
      }
      return true;
    })()
  `);

  const filled = await js(`
    (() => {
      const textarea = document.querySelector('textarea[name="search"]')
        || document.querySelector('textarea');
      if (!textarea) return false;
      textarea.focus();
      const setter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        'value',
      ).set;
      setter.call(textarea, ${JSON.stringify(PROMPT)});
      textarea.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    })()
  `);
  if (!filled) throw new Error('DeepSeek composer not found');

  await js(`
    (() => {
      const textarea = document.querySelector('textarea[name="search"]')
        || document.querySelector('textarea');
      if (!textarea) return false;
      const keyboard = {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        which: 13,
        bubbles: true,
        cancelable: true,
      };
      textarea.dispatchEvent(new KeyboardEvent('keydown', keyboard));
      textarea.dispatchEvent(new KeyboardEvent('keyup', keyboard));
      return true;
    })()
  `);
  await new Promise((resolve) => setTimeout(resolve, 80));

  const cleared = await js(`
    (() => {
      const textarea = document.querySelector('textarea[name="search"]')
        || document.querySelector('textarea');
      return !textarea || !textarea.value.trim();
    })()
  `);
  if (!cleared) {
    await js(`
      (() => {
        const button = document.querySelector('.ds-button--primary:not(.ds-button--disabled)');
        if (!button) return false;
        button.click();
        return true;
      })()
    `);
  }

  const deadline = Date.now() + 300000;
  while (Date.now() < deadline) {
    const snapshot = await js(`
      (() => {
        const stream = window.__lxDeepSeekStream;
        if (!stream) return null;
        stream.tickDom();
        return {
          queue: stream.queue.splice(0, stream.queue.length),
          done: stream.done,
        };
      })()
    `);
    if (!snapshot) throw new Error('DeepSeek stream collector was lost');
    if (snapshot.queue.length > 0) {
      await emitMany(
        snapshot.queue.map((item) => ({ event: item.kind, delta: item.delta })),
      );
    }
    if (snapshot.done) {
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
  await emit({ event: 'error', message: String((error && error.message) || error) });
}
