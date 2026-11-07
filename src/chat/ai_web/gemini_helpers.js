// Google Gemini web driver shared helpers, model/thinking switcher, attachment sync, and session actions.
//
// Prepended before `gemini_script.js` by `src/event/chat/ai_web.rs`.

const PREFIX = '@@LX_AI_WEB@@';
const SPACE = 'lx-tui-ai-web-gemini';
const HOME = 'https://gemini.google.com/app';
const SIGN_IN = 'https://accounts.google.com/ServiceLogin?continue=https://gemini.google.com/app';
const PARAMS = typeof __LX_PARAMS === 'object' && __LX_PARAMS !== null ? __LX_PARAMS : {};
const ACTION = PARAMS.action || 'chat';
const MODEL = PARAMS.model || '3.8 Flash';
const THINKING = PARAMS.thinking || 'low';
const PROMPT = PARAMS.prompt || '';
const FILES = Array.isArray(PARAMS.files) ? PARAMS.files : [];
const KEEP_FILES = Array.isArray(PARAMS.keepFiles) ? PARAMS.keepFiles : [];
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
    } catch (_) {}
  }
  process.stdout.write(payload);
}

async function emit(obj) {
  await emitMany([obj]);
}

async function installPermissionGuard() {
  await js(String.raw`
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
  `).catch(() => false);
}

async function ensureFreshChat() {
  const alreadyFresh = await js(`
    (location.pathname === '/app' || location.pathname === '/app/')
    && document.querySelectorAll('user-query, model-response').length === 0
  `).catch(() => false);
  if (alreadyFresh) return;

  const clicked = await js(String.raw`
    (() => {
      const btn = document.querySelector(
        'a[aria-label*="发起新对话"], button[aria-label*="发起新对话"], a[aria-label*="New chat"], button[aria-label*="New chat"], [data-test-id="new-chat-button"]'
      );
      if (btn && !btn.disabled && btn.getAttribute('aria-disabled') !== 'true') {
        btn.click();
        return true;
      }
      return false;
    })()
  `).catch(() => false);

  if (clicked) {
    await new Promise((r) => setTimeout(r, 350));
  }

  const nowFresh = await js(`
    (location.pathname === '/app' || location.pathname === '/app/')
    && document.querySelectorAll('user-query, model-response').length === 0
  `).catch(() => false);
  if (!nowFresh) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
    await installPermissionGuard();
  }
}

async function ensureGeminiModelAndThinking() {
  const targetModel = String(MODEL || '3.8 Flash').trim();
  const targetThinking = String(THINKING || 'low').trim().toLowerCase();
  const thinkingLabels = {
    low: ['低', 'low'],
    medium: ['中', 'medium'],
    high: ['高', 'high'],
  }[targetThinking] || ['低', 'low'];

  // Step 1: Open mode menu and select the requested model if not already active.
  const menuOpened = await js(String.raw`
    (() => {
      const btn = document.querySelector('[data-test-id="bard-mode-menu-button"], button.input-area-switch');
      if (!btn) return false;
      btn.click();
      return true;
    })()
  `).catch(() => false);
  if (!menuOpened) return;
  await new Promise((r) => setTimeout(r, 280));

  const modelSwitched = await js(`
    (() => {
      const want = ${JSON.stringify(targetModel)}.toLowerCase();
      const items = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitemradio"]'));
      const match = items.find((el) => (el.innerText || '').toLowerCase().includes(want));
      if (!match) return { found: false, clicked: false };
      const checked = match.getAttribute('aria-checked') === 'true' || Boolean(match.querySelector('.mat-icon, [ data-mat-icon-name="check" ]'));
      if (!checked) {
        match.click();
        return { found: true, clicked: true };
      }
      return { found: true, clicked: false };
    })()
  `).catch(() => ({ found: false, clicked: false }));

  if (modelSwitched && modelSwitched.clicked) {
    await new Promise((r) => setTimeout(r, 320));
    await js(String.raw`
      (() => {
        const btn = document.querySelector('[data-test-id="bard-mode-menu-button"], button.input-area-switch');
        if (btn) btn.click();
      })()
    `).catch(() => {});
    await new Promise((r) => setTimeout(r, 280));
  }

  // Step 2: Open "思考等级 / Thinking level" submenu and pick the requested level.
  const subMenuOpened = await js(String.raw`
    (() => {
      const items = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitem"]'));
      const sub = items.find((el) => {
        const t = (el.innerText || '').trim().toLowerCase();
        return t.includes('思考等级') || t.includes('thinking level') || t.includes('reasoning');
      });
      if (!sub) return false;
      sub.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true }));
      sub.click();
      return true;
    })()
  `).catch(() => false);

  if (subMenuOpened) {
    await new Promise((r) => setTimeout(r, 250));
    await js(`
      (() => {
        const labels = ${JSON.stringify(thinkingLabels)};
        const radios = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitemradio"]'));
        const target = radios.find((el) => {
          const text = (el.innerText || '').trim().toLowerCase();
          return labels.some((kw) => text === kw || text.startsWith(kw + ' ') || text.includes(kw));
        });
        if (target) {
          target.click();
          return true;
        }
        return false;
      })()
    `).catch(() => false);
    await new Promise((r) => setTimeout(r, 180));
  }

  // Close any remaining menu backdrop.
  await js(String.raw`
    (() => {
      const backdrop = document.querySelector('.cdk-overlay-backdrop-showing');
      if (backdrop) backdrop.click();
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    })()
  `).catch(() => {});
}

async function syncContainerAttachments(inEditBox) {
  // Remove existing composer attachment previews not in KEEP_FILES.
  await js(`
    (() => {
      const keep = ${inEditBox ? JSON.stringify(KEEP_FILES) : '[]'}.slice();
      const chips = Array.from(
        document.querySelectorAll('uploader-file-preview, .uploader-file-preview-container, [data-test-id*="file-preview"]')
      );
      for (const chip of chips) {
        const text = (chip.innerText || chip.getAttribute('aria-label') || '').trim();
        const keepIdx = keep.findIndex((name) => name && text.includes(name));
        if (${inEditBox ? 'true' : 'false'} && keepIdx !== -1) {
          keep.splice(keepIdx, 1);
          continue;
        }
        const removeBtn = chip.querySelector(
          'button[aria-label*="移除"], button[aria-label*="删除"], button[aria-label*="Remove"], button[aria-label*="Delete"], button'
        );
        if (removeBtn) removeBtn.click();
      }
      return true;
    })()
  `).catch(() => false);

  if (!FILES || FILES.length === 0) return;

  // Trigger upload menu if no <input type="file"> exists yet.
  const hasFileInput = await js(`Boolean(document.querySelector('input[type="file"]'))`).catch(() => false);
  if (!hasFileInput) {
    await js(String.raw`
      (() => {
        const upBtn = document.querySelector(
          'button[aria-label*="上传和工具"], button[aria-label*="上传"], button[aria-label*="Upload"], uploader button'
        );
        if (upBtn) upBtn.click();
      })()
    `).catch(() => {});
    await new Promise((r) => setTimeout(r, 250));
    await js(String.raw`
      (() => {
        const backdrop = document.querySelector('.cdk-overlay-backdrop-showing');
        if (backdrop) backdrop.click();
      })()
    `).catch(() => {});
  }

  const doc = await cdp('DOM.getDocument', { depth: -1 });
  const rootId = doc && doc.root && doc.root.nodeId;
  if (!rootId) throw new Error('Failed to inspect Gemini DOM for file upload');
  const queried = await cdp('DOM.querySelectorAll', { nodeId: rootId, selector: 'input[type="file"]' });
  const nodeIds = (queried && queried.nodeIds) || [];
  const targetNodeId = nodeIds[nodeIds.length - 1];
  if (!targetNodeId) throw new Error('Gemini file upload input not found');
  await cdp('DOM.setFileInputFiles', { nodeId: targetNodeId, files: FILES });

  const uploadDeadline = Date.now() + 25000;
  while (Date.now() < uploadDeadline) {
    await new Promise((r) => setTimeout(r, 200));
    const status = await js(String.raw`
      (() => {
        const busy = Boolean(
          document.querySelector('uploader mat-progress-spinner, uploader-file-preview mat-progress-spinner, [aria-busy="true"] uploader-file-preview')
        );
        return { busy };
      })()
    `).catch(() => ({ busy: false }));
    if (!status.busy) {
      await new Promise((r) => setTimeout(r, 150));
      return;
    }
  }
}

async function ensureSidebarExpanded() {
  await js(String.raw`
    (() => {
      const links = document.querySelectorAll('a[href*="/app/"]');
      if (links.length > 0) return true;
      const toggle = document.querySelector(
        'button[aria-label*="主菜单"], button[aria-label*="Main menu"], [data-test-id="side-nav-menu-button"]'
      );
      if (toggle) {
        toggle.click();
        return true;
      }
      return false;
    })()
  `).catch(() => false);
  await new Promise((r) => setTimeout(r, 300));
}

async function runNonChatAction(currentUrl) {
  if (ACTION === 'new') {
    await ensureFreshChat();
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'stop') {
    await js(String.raw`
      (() => {
        const stopBtn = Array.from(
          document.querySelectorAll('button[aria-label*="停止"], button[aria-label*="Stop"], button.stop-button, .blue-circle.stop-icon')
        ).find((el) => !el.disabled);
        if (!stopBtn) return false;
        const clickable = stopBtn.closest('button') || stopBtn;
        clickable.click();
        return true;
      })()
    `).catch(() => false);
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'list_sessions') {
    await ensureSidebarExpanded();
    const items = await js(String.raw`
      (() => {
        const seen = new Set();
        const out = [];
        const now = Date.now();
        const anchors = Array.from(document.querySelectorAll('a[href*="/app/"]'));
        anchors.forEach((a, idx) => {
          const href = a.getAttribute('href') || '';
          const m = href.match(/\/app\/([a-zA-Z0-9_-]{8,})/);
          if (!m) return;
          const id = m[1];
          if (seen.has(id)) return;
          seen.add(id);
          const rawTitle = (a.getAttribute('aria-label') || a.innerText || '').trim().replace(/\s+/g, ' ');
          out.push({
            id,
            title: rawTitle || 'Gemini chat',
            updated_at_ms: now - idx * 60000,
          });
        });
        return out;
      })()
    `).catch(() => []);
    await emit({ event: 'sessions', items: Array.isArray(items) ? items : [] });
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'load_session') {
    if (!SESSION_ID) {
      await emit({ event: 'error', message: 'missing session id' });
      return;
    }
    await openOrReuseTab(HOME + '/' + SESSION_ID, { wait: true, timeout: 25 });
    await installPermissionGuard();
    const waitDeadline = Date.now() + 10000;
    while (Date.now() < waitDeadline) {
      const count = await js(`document.querySelectorAll('user-query, model-response').length`).catch(() => 0);
      if (count > 0) break;
      await new Promise((r) => setTimeout(r, 200));
    }
    const loaded = await js(`
      (() => {
        const sessionId = ${JSON.stringify(SESSION_ID)};
        const sideLink = Array.from(document.querySelectorAll('a[href*="/app/"]')).find((a) =>
          (a.getAttribute('href') || '').includes('/app/' + sessionId)
        );
        const title = sideLink
          ? (sideLink.getAttribute('aria-label') || sideLink.innerText || '').trim().replace(/\\s+/g, ' ')
          : '';
        const nodes = Array.from(document.querySelectorAll('user-query, model-response'));
        const messages = [];
        nodes.forEach((node, idx) => {
          const tag = node.tagName.toLowerCase();
          if (tag === 'user-query') {
            const clone = node.cloneNode(true);
            clone.querySelectorAll('button, mat-icon, .cdk-visually-hidden').forEach((el) => el.remove());
            const text = (clone.innerText || '').trim();
            const files = Array.from(node.querySelectorAll('[aria-label], .file-preview, img[alt]'))
              .map((el) => (el.getAttribute('alt') || el.getAttribute('aria-label') || '').trim())
              .filter((n) => n && n !== '修改' && n !== '复制' && !n.includes('提示'));
            messages.push({
              role: 'user',
              thinking: '',
              thinking_ms: null,
              text,
              files,
              created_at_ms: Date.now() - (nodes.length - idx) * 1000,
            });
          } else if (tag === 'model-response') {
            const thoughtEl = node.querySelector('model-thoughts, thinking-overlay, .thoughts-content');
            let thinking = '';
            if (thoughtEl) {
              const tClone = thoughtEl.cloneNode(true);
              tClone.querySelectorAll('button, .cdk-visually-hidden').forEach((el) => el.remove());
              thinking = (tClone.innerText || '').trim();
            }
            const msgEl = node.querySelector('message-content') || node;
            const mClone = msgEl.cloneNode(true);
            mClone.querySelectorAll(
              '.cdk-visually-hidden, source-footnote, sources-carousel-inline, sources-list, button, mat-icon'
            ).forEach((el) => el.remove());
            const text = (mClone.innerText || '').trim();
            messages.push({
              role: 'assistant',
              thinking,
              thinking_ms: thinking ? 1000 : null,
              text,
              files: [],
              created_at_ms: Date.now() - (nodes.length - idx) * 1000,
            });
          }
        });
        return { id: sessionId, title, messages };
      })()
    `).catch(() => null);
    if (!loaded || !loaded.id) {
      await emit({ event: 'error', message: 'failed to load Gemini session' });
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
      await ensureSidebarExpanded();
      const menuOpened = await js(`
        (() => {
          const match = location.pathname.match(/\\/app\\/([a-zA-Z0-9_-]+)/);
          const sessionId = ${JSON.stringify(SESSION_ID)} || (match ? match[1] : '');
          if (!sessionId) return false;
          const link = Array.from(document.querySelectorAll('a[href*="/app/"]')).find((a) =>
            (a.getAttribute('href') || '').includes('/app/' + sessionId)
          );
          const row = link ? link.parentElement : null;
          const moreBtn = row ? row.querySelector('button[aria-label*="更多选项"], button[aria-label*="More options"], button') : null;
          if (!moreBtn) return false;
          moreBtn.click();
          return true;
        })()
      `).catch(() => false);
      if (menuOpened) {
        await new Promise((r) => setTimeout(r, 250));
        await js(String.raw`
          (() => {
            const items = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitem"], .cdk-overlay-container button'));
            const renameBtn = items.find((el) => {
              const t = (el.innerText || el.getAttribute('aria-label') || '').trim().toLowerCase();
              return t.includes('重命名') || t.includes('rename');
            });
            if (renameBtn) renameBtn.click();
          })()
        `).catch(() => {});
        await new Promise((r) => setTimeout(r, 280));
        await js(`
          (() => {
            const input = document.querySelector('.cdk-overlay-container input, mat-dialog-container input');
            if (!input) return false;
            input.focus();
            const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set;
            if (setter) setter.call(input, ${JSON.stringify(TITLE)});
            else input.value = ${JSON.stringify(TITLE)};
            input.dispatchEvent(new Event('input', { bubbles: true }));
            const buttons = Array.from(document.querySelectorAll('.cdk-overlay-container button, mat-dialog-container button'));
            const saveBtn = buttons.find((b) => {
              const t = (b.innerText || '').trim().toLowerCase();
              return t === '重命名' || t === '保存' || t === 'rename' || t === 'save';
            });
            if (saveBtn) saveBtn.click();
            return true;
          })()
        `).catch(() => false);
      }
    }
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'delete_session') {
    if (SESSION_ID) {
      await ensureSidebarExpanded();
      const menuOpened = await js(`
        (() => {
          const sessionId = ${JSON.stringify(SESSION_ID)};
          const link = Array.from(document.querySelectorAll('a[href*="/app/"]')).find((a) =>
            (a.getAttribute('href') || '').includes('/app/' + sessionId)
          );
          const row = link ? link.parentElement : null;
          const moreBtn = row ? row.querySelector('button[aria-label*="更多选项"], button[aria-label*="More options"], button') : null;
          if (!moreBtn) return false;
          moreBtn.click();
          return true;
        })()
      `).catch(() => false);
      if (menuOpened) {
        await new Promise((r) => setTimeout(r, 250));
        await js(String.raw`
          (() => {
            const items = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitem"], .cdk-overlay-container button'));
            const delBtn = items.find((el) => {
              const t = (el.innerText || el.getAttribute('aria-label') || '').trim().toLowerCase();
              return t.includes('删除') || t.includes('delete');
            });
            if (delBtn) delBtn.click();
          })()
        `).catch(() => {});
        await new Promise((r) => setTimeout(r, 280));
        await js(String.raw`
          (() => {
            const buttons = Array.from(document.querySelectorAll('.cdk-overlay-container button, mat-dialog-container button'));
            const confirmBtn = buttons.find((b) => {
              const t = (b.innerText || '').trim().toLowerCase();
              return t === '删除' || t === 'delete' || b.getAttribute('data-test-id') === 'confirm-button';
            });
            if (confirmBtn) confirmBtn.click();
          })()
        `).catch(() => {});
        await new Promise((r) => setTimeout(r, 250));
      }
      if (currentUrl.includes(SESSION_ID)) {
        await openOrReuseTab(HOME, { wait: true, timeout: 20 });
      }
    }
    await emit({ event: 'done' });
    return;
  }

  await emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
}
