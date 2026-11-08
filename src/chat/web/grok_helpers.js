// xAI Grok web driver shared helpers, model switcher, attachment sync, and session actions.
//
// Prepended before `grok_script.js` by `src/chat/web/mod.rs`.

const PREFIX = '@@LX_AI_WEB@@';
const SPACE = 'lx-tui-ai-web-grok';
const HOME = 'https://grok.com/';
const PARAMS = typeof __LX_PARAMS === 'object' && __LX_PARAMS !== null ? __LX_PARAMS : {};
const ACTION = PARAMS.action || 'chat';
const MODEL = PARAMS.model || 'Fast';
const THINKING = PARAMS.thinking || '';
const PROMPT = PARAMS.prompt || '';
const FILES = Array.isArray(PARAMS.files) ? PARAMS.files : [];
const KEEP_FILES = Array.isArray(PARAMS.keepFiles) ? PARAMS.keepFiles : [];
const FIRST_TURN = Boolean(PARAMS.firstTurn);
const STREAM_URL = PARAMS.streamUrl || '';
const SESSION_ID = PARAMS.sessionId || '';
const TITLE = PARAMS.title || '';
const EDIT_INDEX = typeof PARAMS.editIndex === 'number' ? PARAMS.editIndex : -1;

const GROK_EDITOR_SELECTOR = '[contenteditable="true"].ProseMirror, [contenteditable="true"]';
const GROK_SUBMIT_SELECTOR = 'button[data-testid="chat-submit"]';
const GROK_STOP_SELECTOR = 'button[aria-label="停止模型响应"], button[aria-label*="Stop model"]';
const GROK_MODEL_BUTTON_SELECTOR = 'button[aria-label^="型号"], button[aria-label^="Model"]';
const GROK_ATTACHMENT_LIST_SELECTOR =
  'form [role="list"][aria-label*="对话附件"], ' +
  'form [role="list"][aria-label*="attachments"], ' +
  'form [role="list"][aria-label*="Attachments"]';
const GROK_ATTACHMENT_CHIP_SELECTOR =
  'form [role="list"][aria-label*="对话附件"] [role="listitem"], ' +
  'form [role="list"][aria-label*="attachments"] [role="listitem"], ' +
  'form [role="list"][aria-label*="Attachments"] [role="listitem"]';

let STREAM_ABORTED = false;

async function emitMany(objs) {
  if (!objs || objs.length === 0 || STREAM_ABORTED) return;
  const payload = objs.map((obj) => PREFIX + JSON.stringify(obj) + '\n').join('');
  if (STREAM_URL) {
    try {
      await fetch(STREAM_URL, { method: 'POST', body: payload });
      return;
    } catch (_) {
      STREAM_ABORTED = true;
      return;
    }
  }
  process.stdout.write(payload);
}

async function emit(obj) {
  await emitMany([obj]);
}

async function checkStreamAlive() {
  if (!STREAM_URL || STREAM_ABORTED) return !STREAM_ABORTED;
  try {
    await fetch(STREAM_URL, { method: 'POST', body: '' });
    return true;
  } catch (_) {
    STREAM_ABORTED = true;
    return false;
  }
}

async function reclaimSpaceTarget(target) {
  if (!target || !target.ownership || target.ownership === 'agent') return false;
  if (target.ownership === 'agentDelegatedToUser') {
    await takeOverTaskSpace(target.id).catch(async () => {
      await claimTaskSpace(target.id).catch(() => {});
    });
  } else {
    await claimTaskSpace(target.id).catch(async () => {
      await takeOverTaskSpace(target.id).catch(() => {});
    });
  }
  return true;
}

async function activateTaskSpace() {
  const spaces = await listTaskSpaces().catch(() => []);
  const target = Array.isArray(spaces)
    ? spaces.find((s) => s && (s.name === SPACE || s.taskId === SPACE))
    : null;
  if (target) {
    await reclaimSpaceTarget(target);
  }
  await useOrCreateTaskSpace(SPACE);
}

async function ensureTaskSpaceControl() {
  const spaces = await listTaskSpaces().catch(() => []);
  const target = Array.isArray(spaces)
    ? spaces.find((s) => s && (s.name === SPACE || s.taskId === SPACE))
    : null;
  if (target && target.ownership && target.ownership !== 'agent') {
    await reclaimSpaceTarget(target);
  }
  await useOrCreateTaskSpace(SPACE).catch(() => {});
}

async function installPermissionGuard() {
  const guardSource = String.raw`
    (() => {
      try {
        if (typeof Notification !== 'undefined') {
          Notification.requestPermission = () => Promise.resolve('denied');
          try {
            Object.defineProperty(Notification, 'permission', {
              get: () => 'denied',
              configurable: true,
            });
          } catch (_) {}
        }
        if (navigator.permissions && typeof navigator.permissions.query === 'function' && !navigator.permissions.__lxWrapped) {
          const origQuery = navigator.permissions.query.bind(navigator.permissions);
          navigator.permissions.query = (desc) => {
            if (desc && desc.name === 'notifications') {
              return Promise.resolve({ state: 'denied', onchange: null });
            }
            return origQuery(desc);
          };
          navigator.permissions.__lxWrapped = true;
        }
      } catch (_) {}
      return true;
    })()
  `;
  await cdp('Page.enable').catch(() => {});
  await cdp('Page.addScriptToEvaluateOnNewDocument', { source: guardSource }).catch(() => {});
  await js(guardSource).catch(() => false);
}

async function dismissCookieBanner() {
  // OneTrust 拒绝按钮：只点击，不用 display:none 绕过（不篡改用户偏好）。
  await js(String.raw`
    (() => {
      const reject = document.querySelector('.ot-pc-refuse-all-handler');
      if (!reject) return false;
      const style = getComputedStyle(reject);
      if (style.display === 'none' || style.visibility === 'hidden') return false;
      reject.click();
      return true;
    })()
  `).catch(() => false);
}

async function grokLoggedIn() {
  // Grok 会给匿名访客自动建 userId，只看 cookie 会误判；必须要求 email 非空。
  const state = await js(String.raw`
    (async () => {
      try {
        const res = await fetch('/rest/auth/get-user', { credentials: 'include' });
        if (!res.ok) return { ok: false, status: res.status };
        const user = await res.json();
        return { ok: Boolean(user && user.email), user };
      } catch (_) {
        return { ok: false };
      }
    })()
  `).catch(() => ({ ok: false }));
  return Boolean(state && state.ok);
}

async function ensureGrokComposer() {
  // tiptap/React 冷启动或慢网络：标签页 load 可能先于 ProseMirror 挂载。
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    const ready = await js(
      `Boolean(document.querySelector(${JSON.stringify(GROK_EDITOR_SELECTOR)}))`
    ).catch(() => false);
    if (ready) return true;
    await new Promise((r) => setTimeout(r, 200));
  }
  return false;
}

async function ensureGrokModel() {
  const want = String(MODEL || 'Fast').trim().toLowerCase();

  // 模型按钮可能晚于 ProseMirror 挂载：按截止时间轮询，禁止单次查询误报。
  const menuDeadline = Date.now() + 15000;
  let menuOpen = false;
  while (!menuOpen && Date.now() < menuDeadline) {
    menuOpen = await js(`
      (() => {
        const btn = document.querySelector(${JSON.stringify(GROK_MODEL_BUTTON_SELECTOR)});
        if (!btn) return false;
        if (btn.getAttribute('aria-expanded') !== 'true') btn.click();
        return true;
      })()
    `).catch(() => false);
    if (!menuOpen) {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  if (!menuOpen) {
    throw new Error('Grok model menu not found');
  }
  await new Promise((r) => setTimeout(r, 350));

  const picked = await js(`
    (() => {
      const want = ${JSON.stringify(want)};
      const items = Array.from(document.querySelectorAll('[role="menuitemradio"]'));
      const match = items.find((item) => (item.innerText || '').trim().toLowerCase() === want);
      if (!match) return { found: false };
      if (match.getAttribute('aria-checked') !== 'true') {
        match.click();
        return { found: true, clicked: true };
      }
      return { found: true, clicked: false };
    })()
  `).catch(() => ({ found: false }));
  if (!picked || !picked.found) {
    throw new Error('Grok model "' + MODEL + '" not found');
  }
  await new Promise((r) => setTimeout(r, 350));

  // 后置校验：按钮标签必须变成目标模型。
  const label = await js(`
    (() => {
      const btn = document.querySelector(${JSON.stringify(GROK_MODEL_BUTTON_SELECTOR)});
      return btn ? (btn.innerText || '').trim() : '';
    })()
  `).catch(() => '');
  if (String(label).trim().toLowerCase() !== want) {
    throw new Error('Grok model "' + MODEL + '" was not applied');
  }

  // 已选中时菜单不会自动收起，避免遮罩挡住后续输入。
  await js(`
    (() => {
      const btn = document.querySelector(${JSON.stringify(GROK_MODEL_BUTTON_SELECTOR)});
      if (btn && btn.getAttribute('aria-expanded') === 'true') btn.click();
      return true;
    })()
  `).catch(() => {});
}

async function syncComposerAttachments() {
  // 只清理输入框 <form> 内的附件 chip，绝不触碰消息气泡里的附件。
  for (let round = 0; round < 5; round += 1) {
    const removed = await js(`
      (() => {
        const lists = Array.from(
          document.querySelectorAll(${JSON.stringify(GROK_ATTACHMENT_LIST_SELECTOR)})
        );
        for (const list of lists) {
          const item = list.querySelector('[role="listitem"]');
          if (!item) continue;
          const remove = item.querySelector(
            'button[aria-label="移除此附件"], button[aria-label*="Remove attachment"], button[aria-label*="Remove"]'
          );
          if (remove) {
            remove.click();
            return true;
          }
        }
        return false;
      })()
    `).catch(() => false);
    if (!removed) break;
    await new Promise((r) => setTimeout(r, 180));
  }

  if (FILES.length === 0) return;

  // 隐藏 file input 可能晚于输入框挂载；最多等 10s，避免冷启动误报。
  let nodeId = 0;
  const inputDeadline = Date.now() + 10000;
  while (!nodeId && Date.now() < inputDeadline) {
    const doc = await cdp('DOM.getDocument', { depth: -1 }).catch(() => null);
    const rootId = doc && doc.root && doc.root.nodeId;
    if (rootId) {
      const queried = await cdp('DOM.querySelectorAll', {
        nodeId: rootId,
        selector: '[data-testid="drop-ui"] input[type="file"]',
      }).catch(() => null);
      nodeId = queried && queried.nodeIds && queried.nodeIds[0];
    }
    if (!nodeId) {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  if (!nodeId) {
    throw new Error('Grok file input not found');
  }
  await cdp('DOM.setFileInputFiles', { nodeId, files: FILES });

  let lastCount = 0;
  let lastInputFiles = 0;
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 200));
    const state = await js(`
      (() => {
        const chips = document.querySelectorAll(${JSON.stringify(GROK_ATTACHMENT_CHIP_SELECTOR)}).length;
        const input = document.querySelector('[data-testid="drop-ui"] input[type="file"]');
        const inputFiles = input && input.files ? input.files.length : 0;
        return { chips, inputFiles };
      })()
    `).catch(() => ({ chips: 0, inputFiles: 0 }));
    lastCount = state.chips;
    lastInputFiles = state.inputFiles;
    if (lastCount >= FILES.length) return;
  }
  throw new Error(
    'Timed out waiting for Grok file upload (chips=' +
      lastCount +
      ', inputFiles=' +
      lastInputFiles +
      ')'
  );
}

async function ensureFreshChat() {
  const isFresh = async () =>
    Boolean(
      await js(`
        (location.pathname === '/' && !document.querySelector('[data-testid="user-message"]'))
      `).catch(() => false)
    );

  if (await isFresh()) return;
  await openOrReuseTab(HOME, { wait: true, timeout: 25 });
}

function httpDetail(result) {
  if (result && typeof result.status === 'number' && result.status > 0) {
    return String(result.status);
  }
  if (result && result.message) return String(result.message);
  return 'network error';
}

async function runNonChatAction(currentUrl) {
  if (ACTION === 'new') {
    await ensureFreshChat();
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'stop') {
    await js(`
      (() => {
        const stopBtn = Array.from(
          document.querySelectorAll(${JSON.stringify(GROK_STOP_SELECTOR)})
        ).find((el) => !el.disabled && el.getAttribute('aria-disabled') !== 'true');
        if (!stopBtn) return false;
        stopBtn.click();
        return true;
      })()
    `).catch(() => false);
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'list_sessions') {
    const result = await js(`
      (async () => {
        try {
          const res = await fetch('/rest/app-chat/conversations?pageSize=60&excludeProjects=true', {
            credentials: 'include',
          });
          if (res.status === 401) return { status: 401, ok: false };
          if (!res.ok) return { status: res.status, ok: false };
          const data = await res.json();
          const conversations = Array.isArray(data && data.conversations) ? data.conversations : [];
          const items = conversations
            .map((c) => ({
              id: String((c && c.conversationId) || ''),
              title: String((c && c.title) || 'New chat'),
              updated_at_ms:
                Date.parse((c && (c.modifyTime || c.createTime)) || '') || 0,
            }))
            .filter((s) => s.id);
          return { status: res.status, ok: true, items };
        } catch (error) {
          return { status: -1, ok: false, message: String((error && error.message) || error) };
        }
      })()
    `).catch(() => ({ status: -1, ok: false, message: 'Grok session list request failed' }));
    if (result && result.status === 401) {
      await emit({ event: 'login_required', url: HOME });
      return;
    }
    if (!result || !result.ok) {
      await emit({
        event: 'error',
        message: 'Grok session list failed (' + httpDetail(result) + ')',
      });
      return;
    }
    await emit({ event: 'sessions', items: Array.isArray(result.items) ? result.items : [] });
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'load_session') {
    if (!SESSION_ID) {
      await emit({ event: 'error', message: 'missing session id' });
      return;
    }
    const result = await js(`
      (async () => {
        try {
          const res = await fetch(
            '/rest/app-chat/conversations/' + encodeURIComponent(${JSON.stringify(SESSION_ID)}) + '/responses',
            { credentials: 'include' }
          );
          if (res.status === 401) return { status: 401, ok: false };
          if (!res.ok) return { status: res.status, ok: false };
          const data = await res.json();
          const all = Array.isArray(data && data.responses) ? data.responses : [];
          const byId = new Map(all.map((r) => [r.responseId, r]));
          const chain = [];
          const visited = new Set();
          let node = all[all.length - 1];
          while (node && !visited.has(node.responseId)) {
            visited.add(node.responseId);
            chain.push(node);
            node = node.parentResponseId ? byId.get(node.parentResponseId) : null;
          }
          chain.reverse();
          const messages = chain.map((r) => {
            const meta = Array.isArray(r.fileAttachmentsMetadata) ? r.fileAttachmentsMetadata : [];
            const raw = Array.isArray(r.fileAttachments) ? r.fileAttachments : [];
            const files =
              r.sender === 'human'
                ? (meta.length > 0
                    ? meta.map((f) => String((f && f.fileName) || ''))
                    : raw.map((f) =>
                        typeof f === 'string'
                          ? f
                          : String((f && (f.fileName || f.name)) || '')
                      )
                  )
                    .map((name) => name.trim())
                    .filter(Boolean)
                : [];
            return {
              role: r.sender === 'human' ? 'user' : 'assistant',
              thinking: '',
              thinking_ms: null,
              text: String(r.message || ''),
              files,
              created_at_ms: Date.parse(r.createTime || '') || 0,
            };
          });
          // responses 接口不含标题；从会话列表补一次，避免 /resume、show 丢标题。
          let title = '';
          try {
            const listRes = await fetch(
              '/rest/app-chat/conversations?pageSize=60&excludeProjects=true',
              { credentials: 'include' }
            );
            if (listRes.ok) {
              const listData = await listRes.json();
              const conversations = Array.isArray(listData && listData.conversations)
                ? listData.conversations
                : [];
              const conversation = conversations.find(
                (c) => c && c.conversationId === ${JSON.stringify(SESSION_ID)}
              );
              if (conversation) title = String(conversation.title || '');
            }
          } catch (_) {}
          return { status: res.status, ok: true, title, messages };
        } catch (error) {
          return { status: -1, ok: false, message: String((error && error.message) || error) };
        }
      })()
    `).catch(() => ({ status: -1, ok: false, message: 'Grok session load request failed' }));
    if (result && result.status === 401) {
      await emit({ event: 'login_required', url: HOME });
      return;
    }
    if (!result || !result.ok) {
      await emit({
        event: 'error',
        message: 'Grok session load failed (' + httpDetail(result) + ')',
      });
      return;
    }
    await emit({
      event: 'session_loaded',
      id: SESSION_ID,
      title: result.title || null,
      messages: Array.isArray(result.messages) ? result.messages : [],
    });
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'rename_session') {
    if (!TITLE) {
      await emit({ event: 'error', message: 'missing session title' });
      return;
    }
    if (!SESSION_ID) {
      await emit({ event: 'error', message: 'missing session id' });
      return;
    }
    const result = await js(`
      (async () => {
        try {
          const res = await fetch(
            '/rest/app-chat/conversations/' + encodeURIComponent(${JSON.stringify(SESSION_ID)}),
            {
              method: 'PUT',
              credentials: 'include',
              headers: { 'content-type': 'application/json' },
              body: JSON.stringify({ title: ${JSON.stringify(TITLE)} }),
            }
          );
          return { status: res.status, ok: res.ok };
        } catch (error) {
          return { status: -1, ok: false, message: String((error && error.message) || error) };
        }
      })()
    `).catch(() => ({ status: -1, ok: false, message: 'Grok rename request failed' }));
    if (result && result.status === 401) {
      await emit({ event: 'login_required', url: HOME });
      return;
    }
    if (!result || !result.ok) {
      await emit({
        event: 'error',
        message: 'Grok rename failed (' + httpDetail(result) + ')',
      });
      return;
    }
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'delete_session') {
    if (!SESSION_ID) {
      await emit({ event: 'error', message: 'missing session id' });
      return;
    }
    const result = await js(`
      (async () => {
        try {
          const res = await fetch(
            '/rest/app-chat/conversations/' + encodeURIComponent(${JSON.stringify(SESSION_ID)}),
            { method: 'DELETE', credentials: 'include' }
          );
          return { status: res.status, ok: res.ok };
        } catch (error) {
          return { status: -1, ok: false, message: String((error && error.message) || error) };
        }
      })()
    `).catch(() => ({ status: -1, ok: false, message: 'Grok delete request failed' }));
    if (result && result.status === 401) {
      await emit({ event: 'login_required', url: HOME });
      return;
    }
    if (!result || !result.ok) {
      await emit({
        event: 'error',
        message: 'Grok delete failed (' + httpDetail(result) + ')',
      });
      return;
    }
    if (currentUrl.includes(SESSION_ID)) {
      await openOrReuseTab(HOME, { wait: true, timeout: 20 });
    }
    await emit({ event: 'done' });
    return;
  }

  await emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
}
