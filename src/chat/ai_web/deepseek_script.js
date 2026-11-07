// DeepSeek web chat driver for `ego-browser nodejs`.
//
// The Rust event layer feeds the action/prompt through the environment (and
// prepends assignments when ego-browser sanitizes process.env), then reads the
// `@@LX_AI_WEB@@` NDJSON lines emitted below.

const PREFIX = '@@LX_AI_WEB@@';
const SPACE = 'lx-tui-ai-web-deepseek';
const HOME = 'https://chat.deepseek.com';
const SIGN_IN = 'https://chat.deepseek.com/sign_in';
const PARAMS = typeof __LX_PARAMS === 'object' && __LX_PARAMS !== null ? __LX_PARAMS : {};
const ACTION = PARAMS.action || 'chat';
const PROMPT = PARAMS.prompt || '';
const FILES = Array.isArray(PARAMS.files) ? PARAMS.files : [];
const FIRST_TURN = Boolean(PARAMS.firstTurn);
const STREAM_URL = PARAMS.streamUrl || '';
const SESSION_ID = PARAMS.sessionId || '';
const TITLE = PARAMS.title || '';
const EDIT_INDEX = typeof PARAMS.editIndex === 'number' ? PARAMS.editIndex : -1;

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

async function ensureFreshChat() {
  const alreadyFresh = await js(`
    location.pathname === '/' && document.querySelectorAll('[data-virtual-list-item-key]').length === 0
  `).catch(() => false);
  if (alreadyFresh) return;

  const clicked = await js(`
    (() => {
      const candidates = Array.from(
        document.querySelectorAll('[tabindex="0"], button, a, [role="button"], div, span'),
      ).filter((el) => {
        const label = (el.innerText || '').trim();
        return label === '开启新对话' || label === 'New chat';
      });
      const clickable = candidates.find((el) => {
        const pk = Object.keys(el).find((k) => k.startsWith('__reactProps'));
        return pk && el[pk] && typeof el[pk].onClick === 'function';
      }) || candidates[candidates.length - 1];
      if (!clickable) return false;
      const pk = Object.keys(clickable).find((k) => k.startsWith('__reactProps'));
      if (pk && clickable[pk] && typeof clickable[pk].onClick === 'function') {
        clickable[pk].onClick({ stopPropagation() {}, preventDefault() {} });
      } else {
        clickable.click();
      }
      return true;
    })()
  `).catch(() => false);

  if (clicked) {
    await new Promise((r) => setTimeout(r, 250));
  }

  const nowFresh = await js(`
    location.pathname === '/' && document.querySelectorAll('[data-virtual-list-item-key]').length === 0
  `).catch(() => false);
  if (!nowFresh) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
  }
}

async function syncContainerAttachments(inEditBox) {
  const hadOldChips = await js(`
    (() => {
      const editArea = document.querySelector('textarea[name="user query"]');
      const scope = ${inEditBox ? '(editArea ? editArea.closest("[data-virtual-list-item-key]") : null)' : 'document'};
      if (!scope) return 0;
      const items = Array.from(scope.querySelectorAll('.ds-animated-size-item')).filter(
        (el) => !(${inEditBox ? 'false' : 'true'} && el.closest('[data-virtual-list-item-key]')),
      );
      let clicked = 0;
      for (const item of items) {
        const closeBtns = Array.from(item.querySelectorAll('[tabindex="0"]')).filter((el) =>
          el.querySelector('svg[viewBox="0 0 14 14"]'),
        );
        for (const btn of closeBtns) {
          const pk = Object.keys(btn).find((k) => k.startsWith('__reactProps'));
          if (pk && btn[pk] && typeof btn[pk].onClick === 'function') {
            btn[pk].onClick({ stopPropagation() {}, preventDefault() {} });
          } else {
            btn.click();
          }
          clicked += 1;
        }
      }
      return clicked;
    })()
  `).catch(() => 0);
  if (hadOldChips > 0) await new Promise((r) => setTimeout(r, 220));
  if (!FILES || FILES.length === 0) return;

  const doc = await cdp('DOM.getDocument', { depth: -1 });
  const rootId = doc && doc.root && doc.root.nodeId;
  if (!rootId) throw new Error('Failed to inspect DeepSeek DOM for file upload');
  const selector = inEditBox ? '[data-virtual-list-item-key] input[type="file"]' : 'input[type="file"]';
  const queried = await cdp('DOM.querySelectorAll', { nodeId: rootId, selector });
  const nodeIds = (queried && queried.nodeIds) || [];
  const targetNodeId = inEditBox ? nodeIds[0] : nodeIds[nodeIds.length - 1];
  if (!targetNodeId) throw new Error('DeepSeek file upload input not found');
  await cdp('DOM.setFileInputFiles', { nodeId: targetNodeId, files: FILES });

  const uploadDeadline = Date.now() + 30000;
  while (Date.now() < uploadDeadline) {
    await new Promise((r) => setTimeout(r, 150));
    const status = await js(`
      (() => {
        const editArea = document.querySelector('textarea[name="user query"]');
        const scope = ${inEditBox ? '(editArea ? editArea.closest("[data-virtual-list-item-key]") : null)' : 'document'};
        if (!scope) return { count: 0, uploading: true, error: null };
        const items = Array.from(scope.querySelectorAll('.ds-animated-size-item')).filter(
          (el) => !(${inEditBox ? 'false' : 'true'} && el.closest('[data-virtual-list-item-key]')),
        );
        if (items.length === 0) return { count: 0, uploading: true, error: null };
        let uploading = false;
        let error = null;
        for (const item of items) {
          const text = (item.innerText || '').trim();
          if (/失败|不支持|超出|过大|error|failed|unsupported|exceed/i.test(text)) {
            error = text.replace(/\\s+/g, ' ');
            break;
          }
          if (/上传中|解析中|uploading|parsing/i.test(text) || item.querySelector('svg[viewBox="0 0 20 20"][role="img"]')) {
            uploading = true;
          }
        }
        return { count: items.length, uploading, error };
      })()
    `).catch(() => ({ count: 0, uploading: true, error: null }));
    if (status.error) throw new Error('File upload rejected by DeepSeek: ' + status.error);
    if (status.count > 0 && !status.uploading) {
      await new Promise((r) => setTimeout(r, 100));
      return;
    }
  }
  throw new Error('Timed out waiting for DeepSeek file upload');
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
    await ensureFreshChat();
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

  if (ACTION === 'list_sessions') {
    const items = await js(String.raw`
      (async () => {
        const tokenObj = JSON.parse(localStorage.getItem('userToken') || '{}');
        const token = tokenObj.value || '';
        const resp = await fetch('/api/v0/chat_session/fetch_page?lte_cursor.pinned=false', {
          headers: { authorization: 'Bearer ' + token },
        }).then((r) => r.json());
        const sessions = (resp && resp.data && resp.data.biz_data && resp.data.biz_data.chat_sessions) || [];
        return sessions.map((s) => ({
          id: String(s.id || ''),
          title: String(s.title || 'New chat'),
          updated_at_ms: Math.round(Number(s.updated_at || 0) * 1000),
        })).filter((s) => s.id);
      })()
    `);
    await emit({ event: 'sessions', items: Array.isArray(items) ? items : [] });
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'load_session') {
    if (!SESSION_ID) {
      await emit({ event: 'error', message: 'missing session id' });
      return;
    }
    await openOrReuseTab(HOME + '/a/chat/s/' + SESSION_ID, { wait: true, timeout: 25 });
    const loaded = await js(`
      (async () => {
        const sessionId = ${JSON.stringify(SESSION_ID)};
        const tokenObj = JSON.parse(localStorage.getItem('userToken') || '{}');
        const token = tokenObj.value || '';
        const headers = { authorization: 'Bearer ' + token };
        const [historyResp, pageResp] = await Promise.all([
          fetch('/api/v0/chat/history_messages?chat_session_id=' + encodeURIComponent(sessionId), { headers })
            .then((r) => r.json())
            .catch(() => null),
          fetch('/api/v0/chat_session/fetch_page?lte_cursor.pinned=false', { headers })
            .then((r) => r.json())
            .catch(() => null),
        ]);
        const rawMessages = (historyResp && historyResp.data && historyResp.data.biz_data && historyResp.data.biz_data.chat_messages) || [];
        const sessions = (pageResp && pageResp.data && pageResp.data.biz_data && pageResp.data.biz_data.chat_sessions) || [];
        const foundSession = sessions.find((s) => String(s.id || '') === sessionId);
        const title = (foundSession && foundSession.title) || '';
        const byId = new Map();
        for (const msg of rawMessages) {
          if (msg && msg.message_id != null) {
            byId.set(msg.message_id, msg);
          }
        }
        let activeChain = rawMessages;
        if (rawMessages.length > 0 && rawMessages[rawMessages.length - 1].message_id != null) {
          const chain = [];
          let cur = rawMessages[rawMessages.length - 1];
          const visited = new Set();
          while (cur && !visited.has(cur.message_id)) {
            visited.add(cur.message_id);
            chain.push(cur);
            cur = cur.parent_id != null ? byId.get(cur.parent_id) : null;
          }
          chain.reverse();
          if (chain.length > 0) activeChain = chain;
        }
        const cleanText = (text) => String(text || '').replace(/\\s*\\[(?:citation|reference):\\d+\\]/g, '');
        const messages = activeChain.map((msg) => {
          const role = String(msg.role || '').toUpperCase() === 'USER' ? 'user' : 'assistant';
          const fragments = Array.isArray(msg.fragments) ? msg.fragments : [];
          let thinking = '';
          let thinkingMs = null;
          let text = '';
          const files = [];
          const pushFile = (f) => {
            if (!f) return;
            const name = typeof f === 'string' ? f : (f.file_name || f.name || '');
            if (name && !files.includes(String(name))) files.push(String(name));
          };
          if (role === 'user' && Array.isArray(msg.files)) {
            msg.files.forEach(pushFile);
          }
          for (const frag of fragments) {
            if (!frag || typeof frag !== 'object') continue;
            if (role === 'user' && Array.isArray(frag.files)) {
              frag.files.forEach(pushFile);
            }
            const ftype = String(frag.type || '').toUpperCase();
            if (role === 'user' && ftype === 'FILE') {
              pushFile(frag);
            } else if (role === 'user' && (ftype === 'REQUEST' || !ftype)) {
              text += String(frag.content || '');
            } else if (role === 'assistant') {
              if (ftype === 'THINK') {
                thinking += cleanText(frag.content);
                if (typeof frag.elapsed_secs === 'number' && Number.isFinite(frag.elapsed_secs)) {
                  thinkingMs = Math.max(1, Math.round(frag.elapsed_secs * 1000));
                }
              } else if (ftype === 'RESPONSE') {
                text += cleanText(frag.content);
              }
            }
          }
          return {
            role,
            thinking,
            thinking_ms: thinkingMs,
            text,
            files,
            created_at_ms: Math.round(Number(msg.inserted_at || 0) * 1000),
          };
        });
        return { id: sessionId, title, messages };
      })()
    `);
    if (!loaded || !loaded.id) {
      await emit({ event: 'error', message: 'failed to load DeepSeek session' });
      return;
    }
    await emit({
      event: 'session_loaded',
      id: loaded.id,
      title: loaded.title || null,
      messages: Array.isArray(loaded.messages) ? loaded.messages : [],
    });
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'rename_session') {
    if (TITLE) {
      await js(`
        (async () => {
          const match = location.pathname.match(/\\/s\\/([a-zA-Z0-9-]+)/);
          const sessionId = ${JSON.stringify(SESSION_ID)} || (match ? match[1] : '');
          if (!sessionId) return false;
          const tokenObj = JSON.parse(localStorage.getItem('userToken') || '{}');
          const token = tokenObj.value || '';
          await fetch('/api/v0/chat_session/update_title', {
            method: 'POST',
            headers: {
              authorization: 'Bearer ' + token,
              'content-type': 'application/json',
            },
            body: JSON.stringify({
              chat_session_id: sessionId,
              title: ${JSON.stringify(TITLE)},
            }),
          });
          return true;
        })()
      `);
    }
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'delete_session') {
    if (SESSION_ID) {
      await js(`
        (async () => {
          const sessionId = ${JSON.stringify(SESSION_ID)};
          const tokenObj = JSON.parse(localStorage.getItem('userToken') || '{}');
          const token = tokenObj.value || '';
          await fetch('/api/v0/chat_session/delete', {
            method: 'POST',
            headers: {
              authorization: 'Bearer ' + token,
              'content-type': 'application/json',
            },
            body: JSON.stringify({ chat_session_ids: [sessionId] }),
          });
          return true;
        })()
      `);
      if (currentUrl.includes(SESSION_ID)) {
        await openOrReuseTab(HOME, { wait: true, timeout: 20 });
      }
    }
    await emit({ event: 'done' });
    return;
  }

  if (ACTION !== 'chat') {
    await emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
    return;
  }

  if (FIRST_TURN && EDIT_INDEX < 0) {
    await ensureFreshChat();
  } else if (SESSION_ID && !currentUrl.includes(SESSION_ID)) {
    await openOrReuseTab(HOME + '/a/chat/s/' + SESSION_ID, { wait: true, timeout: 25 });
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
        attached: false,
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
          if (!xhr || xhr.__lxAttached || this.attached) return;
          xhr.__lxAttached = true;
          this.attached = true;
          this.sseActive = true;
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
          if (!response || !response.body || this.attached) return;
          const type = response.headers && response.headers.get
            ? (response.headers.get('content-type') || '')
            : '';
          if (type && !type.includes('text/event-stream')) return;
          this.attached = true;
          this.sseActive = true;
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
      const isStreamUrl = (u) => typeof u === 'string' && (
        u.includes('/api/v0/chat/completion')
        || u.includes('/api/v0/chat/edit_message')
        || u.includes('/api/v0/chat/regenerate')
      );
      if (!window.__lxXhrWrappedV2) {
        window.__lxXhrWrappedV2 = true;
        const origOpen = XMLHttpRequest.prototype.open;
        const origSend = XMLHttpRequest.prototype.send;
        XMLHttpRequest.prototype.open = function(method, url) {
          this.__lxUrl = typeof url === 'string' ? url : String(url || '');
          return origOpen.apply(this, arguments);
        };
        XMLHttpRequest.prototype.send = function() {
          if (isStreamUrl(this.__lxUrl)) {
            const current = window.__lxDeepSeekStream;
            if (current && !current.sseActive) {
              try {
                current.attachXhr(this);
              } catch (e) {}
            }
          }
          return origSend.apply(this, arguments);
        };
      }
      if (!window.__lxFetchWrappedV2) {
        window.__lxFetchWrappedV2 = true;
        const originalFetch = window.fetch.bind(window);
        window.fetch = function(input, init) {
          const url = typeof input === 'string' ? input : (input && input.url) || '';
          const result = originalFetch(input, init);
          if (isStreamUrl(url)) {
            result.then((response) => {
              const current = window.__lxDeepSeekStream;
              if (!current || current.sseActive) return;
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

  let submittedViaEdit = false;
  if (EDIT_INDEX >= 0) {
    const openedEdit = await js(`
      (() => {
        const items = Array.from(document.querySelectorAll('[data-virtual-list-item-key]'));
        const userItems = items.filter((el) => !el.querySelector('.ds-markdown, .ds-think-content'));
        const target = userItems[${EDIT_INDEX}] || userItems[userItems.length - 1];
        if (!target) return false;
        const btns = Array.from(target.querySelectorAll('.ds-button, button, [role="button"]'));
        const editBtn = btns[1] || btns[btns.length - 1];
        if (!editBtn) return false;
        const propKey = Object.keys(editBtn).find((k) => k.startsWith('__reactProps'));
        if (propKey && editBtn[propKey] && typeof editBtn[propKey].onClick === 'function') {
          editBtn[propKey].onClick({ stopPropagation() {}, preventDefault() {} });
        } else {
          editBtn.click();
        }
        return true;
      })()
    `).catch(() => false);
    if (openedEdit) {
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

    await js(`
      (() => {
        const textarea = document.querySelector('textarea[name="search"]') || document.querySelector('textarea');
        if (!textarea) return false;
        const keyboard = { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true };
        textarea.dispatchEvent(new KeyboardEvent('keydown', keyboard));
        textarea.dispatchEvent(new KeyboardEvent('keyup', keyboard));
        return true;
      })()
    `);
    await new Promise((resolve) => setTimeout(resolve, 90));

    const cleared = await js(`
      (() => {
        const textarea = document.querySelector('textarea[name="search"]') || document.querySelector('textarea');
        const textEmpty = !textarea || !textarea.value.trim();
        const composerChips = Array.from(document.querySelectorAll('.ds-animated-size-item'))
          .filter((el) => !el.closest('[data-virtual-list-item-key]'));
        return textEmpty && composerChips.length === 0 && ${PROMPT ? 'true' : 'false'};
      })()
    `);
    if (!cleared) {
      await js(`
        (() => {
          const button = document.querySelector('.ds-button--primary:not(.ds-button--disabled)');
          if (!button) return false;
          const pk = Object.keys(button).find((k) => k.startsWith('__reactProps'));
          if (pk && button[pk] && typeof button[pk].onClick === 'function') {
            button[pk].onClick({ stopPropagation() {}, preventDefault() {} });
          } else {
            button.click();
          }
          return true;
        })()
      `);
    }
  }

  const deadline = Date.now() + 300000;
  while (Date.now() < deadline) {
    const snapshot = await js(`
      (() => {
        const stream = window.__lxDeepSeekStream;
        if (!stream) return null;
        stream.tickDom();
        return { queue: stream.queue.splice(0, stream.queue.length), done: stream.done };
      })()
    `);
    if (!snapshot) throw new Error('DeepSeek stream collector was lost');
    if (snapshot.queue.length > 0) {
      await emitMany(snapshot.queue.map((item) => ({ event: item.kind, delta: item.delta })));
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
  await emit({ event: 'error', message: String((error && error.message) || error) });
}
