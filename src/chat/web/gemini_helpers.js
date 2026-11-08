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

async function ensureFreshChat() {
  const isFresh = async () =>
    Boolean(
      await js(`
        (location.pathname === '/app' || location.pathname === '/app/')
        && document.querySelectorAll('user-query, model-response').length === 0
        && Boolean(document.querySelector('.ql-editor[contenteditable="true"], rich-textarea'))
      `).catch(() => false),
    );

  if (await isFresh()) return;

  const clicked = await js(String.raw`
    (() => {
      const btn = document.querySelector(
        'a[aria-label*="发起新对话"], button[aria-label*="发起新对话"], a[aria-label*="New chat"], button[aria-label*="New chat"], [data-test-id="new-chat-button"], a[href="/app"]'
      );
      if (btn && !btn.disabled && btn.getAttribute('aria-disabled') !== 'true') {
        btn.click();
        return true;
      }
      return false;
    })()
  `).catch(() => false);

  if (clicked) {
    const clickDeadline = Date.now() + 2500;
    while (Date.now() < clickDeadline) {
      await new Promise((r) => setTimeout(r, 150));
      if (await isFresh()) return;
    }
  }

  await js(`location.assign(${JSON.stringify(HOME)})`).catch(() => {});
  const navDeadline = Date.now() + 15000;
  while (Date.now() < navDeadline) {
    await new Promise((r) => setTimeout(r, 200));
    if (await isFresh()) {
      await installPermissionGuard();
      return;
    }
  }
  await installPermissionGuard();
}

async function ensureGeminiModelAndThinking() {
  const targetModel = String(MODEL || '3.8 Flash').trim().toLowerCase();
  const targetThinking = String(THINKING || 'low').trim().toLowerCase();
  const thinkingLabels = {
    low: ['低', 'low'],
    medium: ['中', 'medium'],
    high: ['高', 'high'],
  }[targetThinking] || ['低', 'low'];

  // Step 1: Open mode menu if not already expanded and select the requested model.
  const menuOpened = await js(String.raw`
    (() => {
      const btn = document.querySelector('[data-test-id="bard-mode-menu-button"], button.input-area-switch');
      if (!btn) return false;
      if (btn.getAttribute('aria-expanded') !== 'true') {
        btn.click();
      }
      return true;
    })()
  `).catch(() => false);
  if (!menuOpened) return;
  await new Promise((r) => setTimeout(r, 260));

  const modelSwitched = await js(`
    (() => {
      const want = ${JSON.stringify(targetModel)};
      const items = Array.from(
        document.querySelectorAll('gem-menu-item[role="menuitemradio"], [role="menuitemradio"]')
      ).filter((el) => !el.closest('.submenu-container'));
      const match = items.find((el) => {
        const lbl = (el.querySelector('.label')?.innerText || el.innerText || '').trim().toLowerCase();
        return lbl === want || lbl.includes(want);
      });
      if (!match) return { found: false, clicked: false };
      if (match.getAttribute('aria-checked') !== 'true') {
        match.click();
        return { found: true, clicked: true };
      }
      return { found: true, clicked: false };
    })()
  `).catch(() => ({ found: false, clicked: false }));

  if (modelSwitched && modelSwitched.clicked) {
    await new Promise((r) => setTimeout(r, 300));
    await js(String.raw`
      (() => {
        const btn = document.querySelector('[data-test-id="bard-mode-menu-button"], button.input-area-switch');
        if (btn && btn.getAttribute('aria-expanded') !== 'true') {
          btn.click();
        }
      })()
    `).catch(() => {});
    await new Promise((r) => setTimeout(r, 260));
  }

  // Step 2: Check if target thinking level is already selected; if not, open submenu and click it.
  const thinkingAlreadyChecked = await js(`
    (() => {
      const labels = ${JSON.stringify(thinkingLabels)};
      const radios = Array.from(
        document.querySelectorAll('.submenu-container gem-menu-item[role="menuitemradio"], gem-menu-item[role="menuitemradio"]')
      );
      const target = radios.find((el) => {
        const lbl = (el.querySelector('.label')?.innerText || el.innerText || '').trim().toLowerCase();
        return labels.some((kw) => lbl === kw || lbl.startsWith(kw + ' '));
      });
      return Boolean(target && target.getAttribute('aria-checked') === 'true');
    })()
  `).catch(() => false);

  if (!thinkingAlreadyChecked) {
    await js(String.raw`
      (() => {
        const items = Array.from(document.querySelectorAll('gem-menu-item[role="menuitem"], [role="menuitem"]'));
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
    await new Promise((r) => setTimeout(r, 240));
    await js(`
      (() => {
        const labels = ${JSON.stringify(thinkingLabels)};
        const radios = Array.from(
          document.querySelectorAll('.submenu-container gem-menu-item[role="menuitemradio"], gem-menu-item[role="menuitemradio"]')
        );
        const target = radios.find((el) => {
          const lbl = (el.querySelector('.label')?.innerText || el.innerText || '').trim().toLowerCase();
          return labels.some((kw) => lbl === kw || lbl.startsWith(kw + ' '));
        });
        if (target && target.getAttribute('aria-checked') !== 'true') {
          target.click();
          return true;
        }
        return false;
      })()
    `).catch(() => false);
    await new Promise((r) => setTimeout(r, 180));
  }

  // Close mode menu if still open.
  await js(String.raw`
    (() => {
      const btn = document.querySelector('[data-test-id="bard-mode-menu-button"], button.input-area-switch');
      if (btn && btn.getAttribute('aria-expanded') === 'true') {
        btn.click();
      }
      const backdrop = document.querySelector('.cdk-overlay-backdrop-showing');
      if (backdrop) backdrop.click();
    })()
  `).catch(() => {});
}

async function syncContainerAttachments(inEditBox) {
  // Remove existing composer attachment previews not in KEEP_FILES.
  const cleanupResult = await js(`
    (() => {
      const keep = ${inEditBox ? JSON.stringify(KEEP_FILES) : '[]'}.slice();
      const chips = Array.from(
        document.querySelectorAll('uploader-file-preview, [data-test-id*="file-preview"]')
      );
      let closedCount = 0;
      let keptCount = 0;
      for (const chip of chips) {
        const img = chip.querySelector('img[alt]');
        const text = (
          (img && img.getAttribute('alt')) ||
          chip.innerText ||
          chip.getAttribute('aria-label') ||
          ''
        ).trim();
        const keepIdx = keep.findIndex((name) => {
          if (!name) return false;
          const base = name.replace(/\\.[^.]+$/, '');
          return text.includes(name) || (base && text.includes(base));
        });
        if (${inEditBox ? 'true' : 'false'} && keepIdx !== -1) {
          keep.splice(keepIdx, 1);
          keptCount += 1;
          continue;
        }
        const removeBtn = chip.querySelector(
          'button[aria-label*="关闭"], button[aria-label*="移除"], button[aria-label*="删除"], button[aria-label^="close "], button[aria-label*="Remove"], button[aria-label*="Delete"], .gem-attachment-close-button button, button'
        );
        if (removeBtn) {
          removeBtn.click();
          closedCount += 1;
        }
      }
      return { closedCount, keptCount };
    })()
  `).catch(() => ({ closedCount: 0, keptCount: 0 }));

  if (cleanupResult && cleanupResult.closedCount > 0) {
    await new Promise((r) => setTimeout(r, 250));
  }

  if (!FILES || FILES.length === 0) return;

  // Open the upload drawer and poll until <input type="file"> is mounted in DOM.
  // Angular lazy-loads <uploader> (~1.5s on a fresh chat), so only click the
  // toggle button when aria-expanded !== 'true' to avoid closing it mid-load.
  const openDeadline = Date.now() + 8000;
  let lastClickAt = 0;
  let inputReady = false;
  while (Date.now() < openDeadline) {
    const count = await js(`document.querySelectorAll('input[type="file"]').length`).catch(() => 0);
    if (count > 0) {
      inputReady = true;
      break;
    }
    const now = Date.now();
    await js(`
      (() => {
        const upBtn = document.querySelector(
          'button[aria-label*="上传和工具"], button[aria-label*="上传"], button[aria-label*="Upload"], simplified-input-menu button'
        );
        if (!upBtn) return false;
        const expanded = upBtn.getAttribute('aria-expanded') === 'true';
        if (!expanded || ${now - lastClickAt > 2500 ? 'true' : 'false'}) {
          upBtn.click();
          return true;
        }
        return false;
      })()
    `)
      .then((clicked) => {
        if (clicked) lastClickAt = Date.now();
      })
      .catch(() => {});
    await new Promise((r) => setTimeout(r, 150));
  }
  if (!inputReady) {
    throw new Error('Gemini file upload input not found');
  }

  const mediaFiles = FILES.filter((f) =>
    /\.(png|jpe?g|webp|gif|bmp|svg|ico|avif|heic|heif|tiff?|mp4|mov|webm|mkv|avi|m4v)$/i.test(String(f))
  );
  const docFiles = FILES.filter(
    (f) => !/\.(png|jpe?g|webp|gif|bmp|svg|ico|avif|heic|heif|tiff?|mp4|mov|webm|mkv|avi|m4v)$/i.test(String(f))
  );

  async function setFilesOnSelector(selectorExpr, fileList) {
    if (!fileList || fileList.length === 0) return;
    const evalRes = await cdp('Runtime.evaluate', { expression: selectorExpr }).catch(() => null);
    const objectId = evalRes && evalRes.result && evalRes.result.objectId;
    if (objectId) {
      await cdp('DOM.setFileInputFiles', { objectId, files: fileList });
      return;
    }
    const doc = await cdp('DOM.getDocument', { depth: -1 });
    const rootId = doc && doc.root && doc.root.nodeId;
    if (!rootId) throw new Error('Failed to inspect Gemini DOM for file upload');
    const queried = await cdp('DOM.querySelectorAll', { nodeId: rootId, selector: 'input[type="file"]' });
    const nodeIds = (queried && queried.nodeIds) || [];
    const targetNodeId = nodeIds[nodeIds.length - 1];
    if (!targetNodeId) throw new Error('Gemini file upload input not found');
    await cdp('DOM.setFileInputFiles', { nodeId: targetNodeId, files: fileList });
  }

  if (mediaFiles.length > 0) {
    await setFilesOnSelector(
      `document.querySelector('input[type="file"][accept*="image"]') || document.querySelector('input[type="file"]')`,
      mediaFiles
    );
  }
  if (docFiles.length > 0) {
    await setFilesOnSelector(
      `document.querySelector('images-files-uploader input[type="file"]') || document.querySelector('input[type="file"]:not([accept*="image"])') || document.querySelector('input[type="file"]')`,
      docFiles
    );
  }

  const expectedCount = ((cleanupResult && cleanupResult.keptCount) || 0) + FILES.length;
  const uploadDeadline = Date.now() + 30000;
  let uploadedReady = false;
  while (Date.now() < uploadDeadline) {
    await new Promise((r) => setTimeout(r, 200));
    const status = await js(`
      (() => {
        const chips = Array.from(document.querySelectorAll('uploader-file-preview'));
        const busy = Boolean(
          document.querySelector(
            'uploader mat-progress-spinner, uploader-file-preview mat-progress-spinner, uploader-file-preview [role="progressbar"], [aria-busy="true"] uploader-file-preview'
          )
        );
        return { count: chips.length, busy };
      })()
    `).catch(() => ({ count: 0, busy: true }));
    if (status.count >= expectedCount && !status.busy) {
      uploadedReady = true;
      await new Promise((r) => setTimeout(r, 200));
      break;
    }
  }

  // Close the upload drawer if it is still expanded.
  await js(String.raw`
    (() => {
      const upBtn = document.querySelector('button[aria-label*="上传和工具"], button[aria-label*="Upload"]');
      if (upBtn && upBtn.getAttribute('aria-expanded') === 'true') {
        upBtn.click();
      }
    })()
  `).catch(() => {});

  if (!uploadedReady) {
    throw new Error('Timed out waiting for Gemini file upload');
  }
}

async function ensureSidebarExpanded() {
  await js(String.raw`
    (() => {
      const links = document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]');
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
        const anchors = Array.from(
          document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
        );
        anchors.forEach((a, idx) => {
          const href = a.getAttribute('href') || '';
          const m = href.match(/^\/app\/([a-zA-Z0-9_-]{8,})/);
          if (!m) return;
          const id = m[1];
          if (seen.has(id)) return;
          seen.add(id);
          const rawTitle = (a.innerText || a.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ');
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
        const sideLink = Array.from(
          document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
        ).find((a) => (a.getAttribute('href') || '').startsWith('/app/' + sessionId));
        let title = sideLink
          ? (sideLink.innerText || sideLink.getAttribute('aria-label') || '').trim().replace(/\\s+/g, ' ')
          : '';
        if (!title) {
          const docTitle = (document.title || '').replace(/\\s*-\\s*Google Gemini$/i, '').trim();
          if (docTitle && docTitle !== 'Google Gemini' && docTitle !== 'Gemini') {
            title = docTitle;
          }
        }
        const nodes = Array.from(document.querySelectorAll('user-query, model-response'));
        const messages = [];
        nodes.forEach((node, idx) => {
          const tag = node.tagName.toLowerCase();
          if (tag === 'user-query') {
            const hidden = [];
            node.querySelectorAll('button, mat-icon, .cdk-visually-hidden').forEach((el) => {
              hidden.push([el, el.style.display]);
              el.style.display = 'none';
            });
            const text = (node.innerText || '').replace(/\\r\\n/g, '\\n').trim();
            for (const [el, prev] of hidden) el.style.display = prev;
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
              const hiddenT = [];
              thoughtEl.querySelectorAll('button, mat-icon, .cdk-visually-hidden').forEach((el) => {
                hiddenT.push([el, el.style.display]);
                el.style.display = 'none';
              });
              thinking = (thoughtEl.innerText || '').replace(/\\r\\n/g, '\\n').trim();
              for (const [el, prev] of hiddenT) el.style.display = prev;
            }
            const msgEl = node.querySelector('message-content .markdown, message-content') || node;
            const hiddenM = [];
            msgEl.querySelectorAll(
              '.cdk-visually-hidden, source-footnote, sources-carousel-inline, sources-list, button, mat-icon'
            ).forEach((el) => {
              hiddenM.push([el, el.style.display]);
              el.style.display = 'none';
            });
            const tempCodeNodes = [];
            const bt = String.fromCharCode(96);
            msgEl.querySelectorAll('code').forEach((c) => {
              if (!c.closest('pre, code-block')) {
                const open = document.createTextNode(bt);
                const close = document.createTextNode(bt);
                c.insertBefore(open, c.firstChild);
                c.appendChild(close);
                tempCodeNodes.push(open, close);
              }
            });
            const text = (msgEl.innerText || '').replace(/\\r\\n/g, '\\n').trim();
            for (const tn of tempCodeNodes) tn.remove();
            for (const [el, prev] of hiddenM) el.style.display = prev;
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
          const link = Array.from(
            document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
          ).find((a) => (a.getAttribute('href') || '').startsWith('/app/' + sessionId));
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
            if (typeof input.select === 'function') input.select();
            const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set;
            if (setter) setter.call(input, ${JSON.stringify(TITLE)});
            else input.value = ${JSON.stringify(TITLE)};
            input.dispatchEvent(new InputEvent('input', { bubbles: true, data: ${JSON.stringify(TITLE)}, inputType: 'insertText' }));
            input.dispatchEvent(new Event('change', { bubbles: true }));
            return true;
          })()
        `).catch(() => false);
        await new Promise((r) => setTimeout(r, 180));
        await js(String.raw`
          (() => {
            const buttons = Array.from(document.querySelectorAll('.cdk-overlay-container button, mat-dialog-container button'));
            const saveBtn = buttons.find((b) => {
              const t = (b.innerText || '').trim().toLowerCase();
              return t === '重命名' || t === '保存' || t === 'rename' || t === 'save';
            });
            if (saveBtn && !saveBtn.disabled) saveBtn.click();
          })()
        `).catch(() => {});
        await new Promise((r) => setTimeout(r, 400));
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
          const link = Array.from(
            document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
          ).find((a) => (a.getAttribute('href') || '').startsWith('/app/' + sessionId));
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
