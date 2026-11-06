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

function emit(obj) {
  process.stdout.write(PREFIX + JSON.stringify(obj) + '\n');
}

async function main() {
  await useOrCreateTaskSpace(SPACE);
  await openOrReuseTab(HOME, { wait: true, timeout: 25 });

  const loggedIn = await js(
    "Boolean(localStorage.getItem('userToken')) && !location.pathname.startsWith('/sign_in')",
  );
  if (!loggedIn) {
    await openOrReuseTab(SIGN_IN, { wait: true, timeout: 20 });
    await handOffTaskSpace();
    emit({ event: 'login_required', url: SIGN_IN });
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
    emit({ event: 'done' });
    return;
  }

  if (ACTION === 'stop') {
    await js(`
      (() => {
        const labelled = Array.from(document.querySelectorAll('button, [role="button"], .ds-button')).find((el) => {
          const text = (el.getAttribute('aria-label') || '') + ' ' + (el.title || '');
          return /stop|停止/i.test(text);
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
    emit({ event: 'done' });
    return;
  }

  if (ACTION !== 'chat') {
    emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
    return;
  }

  // Channel A: clone the completion SSE stream and replay its JSON patches.
  await js(`
    (() => {
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
        initialCount: document.querySelectorAll('.ds-message').length,
        push(kind, delta) {
          if (!delta) return;
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
          return fragment && fragment.type === 'THINK' ? 'thinking' : 'text';
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
          const match = /^response\/fragments\/(-?\d+)\/([A-Za-z_]+)$/.exec(path);
          if (match) {
            const rawIndex = Number(match[1]);
            const index = rawIndex < 0 ? Math.max(0, this.fragments.length + rawIndex) : rawIndex;
            const field = match[2];
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
              const entry = {
                type: fragment.type || 'RESPONSE',
                content: typeof fragment.content === 'string' ? fragment.content : '',
              };
              this.fragments.push(entry);
              if (entry.content) {
                this.push(this.fragmentKind(entry), entry.content);
              }
            }
            return;
          }
          value.forEach((fragment, index) => {
            if (!fragment || typeof fragment !== 'object') return;
            const current = this.fragments[index] || { content: '' };
            if (fragment.type) current.type = fragment.type;
            if (typeof fragment.content !== 'string') {
              this.fragments[index] = current;
              return;
            }
            const previous = typeof current.content === 'string' ? current.content : '';
            const delta = fragment.content.startsWith(previous)
              ? fragment.content.slice(previous.length)
              : fragment.content;
            current.content = fragment.content;
            this.fragments[index] = current;
            if (delta) this.push(this.fragmentKind(current), delta);
          });
        },
        applyFragmentContent(index, op, value) {
          if (typeof value !== 'string') return;
          const fragment = this.fragments[index] || (this.fragments[index] = { content: '' });
          const previous = typeof fragment.content === 'string' ? fragment.content : '';
          let delta = '';
          if (op === 'APPEND') {
            delta = value;
          } else if (op === 'SET') {
            delta = value.startsWith(previous) ? value.slice(previous.length) : value;
          } else {
            delta = value;
          }
          fragment.content = op === 'APPEND' ? previous + value : value;
          this.fragments[index] = fragment;
          this.push(this.fragmentKind(fragment), delta);
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
          let buffer = '';
          let eventName = '';
          let dataLines = [];
          const flush = () => {
            const data = dataLines.join('\\n').trim();
            const name = eventName;
            dataLines = [];
            eventName = '';
            if (name === 'finish' || name === 'close') self.done = true;
            if (!data) return;
            if (data === '[DONE]') {
              self.done = true;
              return;
            }
            try {
              self.applyEvent(JSON.parse(data));
            } catch (error) {
              // Ignore keep-alive and non-patch frames.
            }
          };
          const pump = () => reader.read().then((chunk) => {
            if (chunk.done) {
              if (buffer.trim()) {
                if (buffer.startsWith('data:')) {
                  dataLines.push(buffer.slice(5).replace(/^ /, ''));
                } else if (buffer.startsWith('event:')) {
                  eventName = buffer.slice(6).trim();
                }
                buffer = '';
              }
              flush();
              if (self.sseActive) self.done = true;
              return;
            }
            buffer += decoder.decode(chunk.value, { stream: true });
            let index;
            while ((index = buffer.indexOf('\\n')) >= 0) {
              let line = buffer.slice(0, index);
              buffer = buffer.slice(index + 1);
              if (line.endsWith('\\r')) line = line.slice(0, -1);
              if (!line) {
                flush();
                continue;
              }
              if (line.startsWith('event:')) {
                eventName = line.slice(6).trim();
              } else if (line.startsWith('data:')) {
                dataLines.push(line.slice(5).replace(/^ /, ''));
              }
            }
            return pump();
          }).catch(() => {
            if (self.sseActive) self.done = true;
          });
          pump();
        },
        tickDom() {
          if (this.sseActive) return;
          const nodes = Array.prototype.slice.call(
            document.querySelectorAll('.ds-message'),
            this.initialCount,
          );
          let target = null;
          for (let index = nodes.length - 1; index >= 0; index -= 1) {
            if (nodes[index].querySelector('.ds-markdown, .ds-think-content')) {
              target = nodes[index];
              break;
            }
          }
          if (!target) return;
          const think = target.querySelector('.ds-think-content');
          const text = target.querySelector('.ds-markdown');
          this.pushDom('thinking', think ? (think.innerText || '') : '');
          this.pushDom('text', text ? (text.innerText || '') : '');
          const sample = (this.domThink || '') + '\u0000' + (this.domText || '');
          if (sample === this.domLastSample) {
            this.domStableTicks += 1;
          } else {
            this.domLastSample = sample;
            this.domStableTicks = 0;
          }
          // 兜底通道没有网络终态：正文已经出现且发送按钮恢复禁用态（生成结束）、内容稳定后收尾。
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
  await new Promise((resolve) => setTimeout(resolve, 400));

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
    for (const item of snapshot.queue) {
      emit({ event: item.kind, delta: item.delta });
    }
    if (snapshot.done) {
      emit({ event: 'done' });
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error('timed out waiting for DeepSeek to finish the reply');
}

try {
  await main();
} catch (error) {
  emit({ event: 'error', message: String((error && error.message) || error) });
}
