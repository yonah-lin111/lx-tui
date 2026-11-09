// Google Gemini web driver shared helpers, model/thinking switcher, attachment sync, and session actions.
//
// Prepended before `gemini_script.js` by `src/event/chat/web.rs`.

const PREFIX = '@@LX_AI_WEB@@';
const SPACE = 'lx-tui-ai-web-gemini';
const HOME = 'https://gemini.google.com/app';
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
const SESSIONS_ROOT = PARAMS.sessionsRoot || '';
const DOWNLOAD_URL = PARAMS.downloadUrl || '';
const FALLBACK_URL = PARAMS.fallbackUrl || '';
const DOWNLOAD_DIR = PARAMS.downloadDir || '';
const DOWNLOAD_STEM = PARAMS.downloadStem || '';
const EDIT_INDEX = typeof PARAMS.editIndex === 'number' ? PARAMS.editIndex : -1;

function detectImageExt(buf, contentType, url) {
  if (buf && buf.length >= 12) {
    if (buf[0] === 0xff && buf[1] === 0xd8) return 'jpg';
    if (buf[0] === 0x89 && buf[1] === 0x50 && buf[2] === 0x4e && buf[3] === 0x47) return 'png';
    if (
      buf.slice(0, 4).toString('ascii') === 'RIFF' &&
      buf.slice(8, 12).toString('ascii') === 'WEBP'
    ) {
      return 'webp';
    }
    if (buf.slice(0, 3).toString('ascii') === 'GIF') return 'gif';
  }
  const ct = String(contentType || '').toLowerCase();
  if (ct.includes('jpeg') || ct.includes('jpg')) return 'jpg';
  if (ct.includes('webp')) return 'webp';
  if (ct.includes('gif')) return 'gif';
  if (ct.includes('png')) return 'png';
  const m = String(url || '').match(/\.(jpe?g|png|webp|gif)(?:[?#]|$)/i);
  if (m) return m[1].toLowerCase().replace('jpeg', 'jpg');
  return 'png';
}

function toGeminiOriginalImageUrl(previewUrl) {
  const s = String(previewUrl || '').trim();
  if (!s) return '';
  const base = s.replace(/=[^=/]+$/, '');
  return base + '=s0-d-I?alr=yes';
}

async function downloadImageBuffer(frameId, startUrl) {
  if (!frameId || !startUrl) return null;
  let curUrl = startUrl;
  for (let hop = 0; hop < 5; hop += 1) {
    try {
      const netRes = await cdp('Network.loadNetworkResource', {
        frameId,
        url: curUrl,
        options: { disableCache: false, includeCredentials: true },
      });
      const resource = netRes && netRes.resource;
      if (!resource || resource.httpStatusCode !== 200 || !resource.stream) {
        return null;
      }
      const headers = resource.headers || {};
      const contentType = headers['content-type'] || headers['Content-Type'] || '';
      const chunks = [];
      while (true) {
        const part = await cdp('IO.read', { handle: resource.stream, size: 65536 });
        if (part && part.data) {
          chunks.push(
            part.base64Encoded
              ? Buffer.from(part.data, 'base64')
              : Buffer.from(part.data, 'utf8')
          );
        }
        if (!part || part.eof) break;
      }
      await cdp('IO.close', { handle: resource.stream }).catch(() => {});
      const buf = Buffer.concat(chunks);
      const asText = buf.length < 4096 ? buf.toString('utf8').trim() : '';
      if (/^https?:\/\//i.test(asText)) {
        curUrl = asText;
        continue;
      }
      if (buf.length > 0) {
        return { buf, contentType, finalUrl: curUrl };
      }
      return null;
    } catch (_) {
      return null;
    }
  }
  return null;
}

async function saveGeneratedImages(siteTag, sessionPrefix, rawSessionId, turnNum, urls) {
  const list = Array.isArray(urls) ? Array.from(new Set(urls.filter(Boolean))) : [];
  if (list.length === 0) return [];
  const nodeFs = await import('node:fs');
  const nodePath = await import('node:path');
  const cleanSid = String(rawSessionId || 'temp').replace(/[^a-zA-Z0-9_-]/g, '_') || 'temp';
  const shortSid = cleanSid.slice(0, 8);
  const fullSessionId = sessionPrefix + cleanSid;
  const imageDir = SESSIONS_ROOT ? nodePath.join(SESSIONS_ROOT, fullSessionId, 'image') : '';
  if (imageDir) {
    try {
      nodeFs.mkdirSync(imageDir, { recursive: true, mode: 0o700 });
    } catch (_) {}
  }
  let frameId = '';
  try {
    const tree = await cdp('Page.getResourceTree');
    frameId = (tree && tree.frameTree && tree.frameTree.frame && tree.frameTree.frame.id) || '';
  } catch (_) {}

  const out = [];
  for (let i = 0; i < list.length; i += 1) {
    const previewUrl = list[i];
    const originalUrl = toGeminiOriginalImageUrl(previewUrl);
    const baseStem = siteTag + '-r' + String(turnNum) + '-' + String(i + 1);
    const fileStem = siteTag + '-' + shortSid + '-r' + String(turnNum) + '-' + String(i + 1) + '-orig';
    if (imageDir) {
      let cached = null;
      for (const ext of ['jpg', 'png', 'webp', 'gif']) {
        const cand = nodePath.join(imageDir, baseStem + '.' + ext);
        try {
          const st = nodeFs.statSync(cand);
          if (st.isFile() && st.size > 0) {
            cached = {
              label: baseStem + '.' + ext,
              path: cand,
              original_url: originalUrl,
              preview_url: previewUrl,
              file_stem: fileStem,
            };
            break;
          }
        } catch (_) {}
      }
      if (cached) {
        out.push(cached);
        continue;
      }
    }
    const fetched = await downloadImageBuffer(frameId, previewUrl);
    const buf = fetched && fetched.buf;
    const contentType = (fetched && fetched.contentType) || '';
    const ext = detectImageExt(buf, contentType, previewUrl);
    const label = baseStem + '.' + ext;
    let savedPath = '';
    if (imageDir && buf && buf.length > 0) {
      const target = nodePath.join(imageDir, label);
      try {
        nodeFs.writeFileSync(target, buf, { mode: 0o600 });
        savedPath = target;
      } catch (_) {}
    }
    out.push({
      label,
      path: savedPath || null,
      original_url: originalUrl,
      preview_url: previewUrl,
      file_stem: fileStem,
    });
  }
  return out;
}

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
  // Gemini 是 Angular 应用：冷启动或页面尚未渲染完成时，输入框可能先于模式菜单按钮出现。
  // 单次查询会把「还没渲染」误判成「菜单不存在」，因此按截止时间轮询等待按钮挂载。
  const menuDeadline = Date.now() + 15000;
  let menuOpened = false;
  while (!menuOpened && Date.now() < menuDeadline) {
    menuOpened = await js(String.raw`
      (() => {
        const btn = document.querySelector('[data-test-id="bard-mode-menu-button"], button.input-area-switch');
        if (!btn) return false;
        if (btn.getAttribute('aria-expanded') !== 'true') {
          btn.click();
        }
        return true;
      })()
    `).catch(() => false);
    if (!menuOpened) {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  if (!menuOpened) {
    throw new Error('Gemini model menu not found');
  }
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
  if (!modelSwitched || !modelSwitched.found) {
    throw new Error('Gemini model "' + MODEL + '" not found in the mode menu');
  }

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
    const modelApplied = await js(`
      (() => {
        const want = ${JSON.stringify(targetModel)};
        const items = Array.from(
          document.querySelectorAll('gem-menu-item[role="menuitemradio"], [role="menuitemradio"]')
        ).filter((el) => !el.closest('.submenu-container'));
        const match = items.find((el) => {
          const lbl = (el.querySelector('.label')?.innerText || el.innerText || '').trim().toLowerCase();
          return lbl === want || lbl.includes(want);
        });
        return Boolean(match && match.getAttribute('aria-checked') === 'true');
      })()
    `).catch(() => false);
    if (!modelApplied) {
      throw new Error('Gemini model "' + MODEL + '" was not applied');
    }
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
    const submenuOpened = await js(String.raw`
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
    if (!submenuOpened) {
      throw new Error('Gemini thinking level menu not found');
    }
    await new Promise((r) => setTimeout(r, 240));
    const thinkingPick = await js(`
      (() => {
        const labels = ${JSON.stringify(thinkingLabels)};
        const radios = Array.from(
          document.querySelectorAll('.submenu-container gem-menu-item[role="menuitemradio"], gem-menu-item[role="menuitemradio"]')
        );
        const target = radios.find((el) => {
          const lbl = (el.querySelector('.label')?.innerText || el.innerText || '').trim().toLowerCase();
          return labels.some((kw) => lbl === kw || lbl.startsWith(kw + ' '));
        });
        if (!target) return { found: false, clicked: false };
        if (target.getAttribute('aria-checked') !== 'true') {
          target.click();
          return { found: true, clicked: true };
        }
        return { found: true, clicked: false };
      })()
    `).catch(() => ({ found: false, clicked: false }));
    if (!thinkingPick || !thinkingPick.found) {
      throw new Error('Gemini thinking level "' + THINKING + '" not found');
    }
    if (thinkingPick.clicked) {
      // 选择后面板会立即关闭，无法再从菜单读取 aria-checked；此处只保证目标项存在且点击成功。
      await new Promise((r) => setTimeout(r, 180));
    }
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

async function syncContainerAttachments() {
  // 普通发送前清空输入框里残留的附件 chip；`/rewind` 的附件由编辑流程单独校验。
  const closedCount = await js(`
    (() => {
      const chips = Array.from(
        document.querySelectorAll('uploader-file-preview, [data-test-id*="file-preview"]')
      );
      let closedCount = 0;
      for (const chip of chips) {
        const removeBtn = chip.querySelector(
          'button[aria-label*="关闭"], button[aria-label*="移除"], button[aria-label*="删除"], button[aria-label^="close "], button[aria-label*="Remove"], button[aria-label*="Delete"], .gem-attachment-close-button button'
        );
        if (removeBtn) {
          removeBtn.click();
          closedCount += 1;
        }
      }
      return closedCount;
    })()
  `).catch(() => 0);

  if (closedCount > 0) {
    await new Promise((r) => setTimeout(r, 250));
  }

  if (!FILES || FILES.length === 0) return;

  // Open the upload drawer and poll until <input type="file"> is mounted in DOM.
  // Angular lazy-loads <uploader> (~1.5s on a fresh chat); never click while
  // aria-expanded === 'true', otherwise we close the drawer mid-load.
  const openDeadline = Date.now() + 8000;
  let inputReady = false;
  while (Date.now() < openDeadline) {
    const count = await js(`document.querySelectorAll('input[type="file"]').length`).catch(() => 0);
    if (count > 0) {
      inputReady = true;
      break;
    }
    await js(String.raw`
      (() => {
        const upBtn = document.querySelector(
          'button[aria-label*="上传和工具"], button[aria-label*="上传"], button[aria-label*="Upload"], simplified-input-menu button'
        );
        if (!upBtn) return false;
        if (upBtn.getAttribute('aria-expanded') === 'true') return false;
        const now = Date.now();
        if (now - (window.__lxUploadClickAt || 0) < 1200) return false;
        window.__lxUploadClickAt = now;
        upBtn.click();
        return true;
      })()
    `).catch(() => {});
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

  const inputCount = await js(`document.querySelectorAll('input[type="file"]').length`).catch(() => 0);
  if (inputCount <= 1) {
    // 单个 input 接收全部文件：分两次注入会互相覆盖 FileList。
    await setFilesOnSelector(`document.querySelector('input[type="file"]')`, FILES);
  } else {
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
  }

  const expectedCount = FILES.length;
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
  if (ACTION === 'download_image') {
    const nodeFs = await import('node:fs');
    const nodePath = await import('node:path');
    if (!DOWNLOAD_DIR || (!DOWNLOAD_URL && !FALLBACK_URL)) {
      await emit({ event: 'error', message: 'Missing download target parameters' });
      return;
    }
    let frameId = '';
    try {
      const tree = await cdp('Page.getResourceTree');
      frameId = (tree && tree.frameTree && tree.frameTree.frame && tree.frameTree.frame.id) || '';
    } catch (_) {}
    let fetched = DOWNLOAD_URL ? await downloadImageBuffer(frameId, DOWNLOAD_URL) : null;
    if ((!fetched || !fetched.buf) && FALLBACK_URL) {
      fetched = await downloadImageBuffer(frameId, FALLBACK_URL);
    }
    if (!fetched || !fetched.buf || fetched.buf.length === 0) {
      await emit({ event: 'error', message: 'Failed to download original image from Gemini' });
      return;
    }
    const ext = detectImageExt(fetched.buf, fetched.contentType, fetched.finalUrl || DOWNLOAD_URL);
    const fileName = (DOWNLOAD_STEM || 'gemini-orig') + '.' + ext;
    try {
      nodeFs.mkdirSync(DOWNLOAD_DIR, { recursive: true });
      const target = nodePath.join(DOWNLOAD_DIR, fileName);
      nodeFs.writeFileSync(target, fetched.buf);
      await emit({
        event: 'image',
        label: fileName,
        path: target,
        original_url: DOWNLOAD_URL,
        preview_url: FALLBACK_URL,
        file_stem: DOWNLOAD_STEM,
      });
      await emit({ event: 'done' });
    } catch (error) {
      await emit({
        event: 'error',
        message: 'Failed to save original image: ' + String((error && error.message) || error),
      });
    }
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
    const imgDeadline = Date.now() + 3000;
    while (Date.now() < imgDeadline) {
      const pendingImg = await js(String.raw`
        (() => {
          const containers = Array.from(
            document.querySelectorAll('model-response generated-image, model-response single-image, model-response .generated-images')
          );
          if (containers.length === 0) return false;
          return containers.some((c) => {
            const img = c.querySelector('img');
            const src = img ? (img.currentSrc || img.src || '').trim() : '';
            return !/^https?:\/\//i.test(src);
          });
        })()
      `).catch(() => false);
      if (!pendingImg) break;
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
        let assistantTurn = 0;
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
              images: [],
              created_at_ms: Date.now() - (nodes.length - idx) * 1000,
            });
          } else if (tag === 'model-response') {
            assistantTurn += 1;
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
            const imageUrls = Array.from(
              node.querySelectorAll('generated-image img, single-image img, .generated-images img')
            )
              .map((img) => (img.currentSrc || img.src || '').trim())
              .filter((u) => /^https?:\\/\\//i.test(u));
            messages.push({
              role: 'assistant',
              thinking,
              thinking_ms: thinking ? 1000 : null,
              text,
              files: [],
              turnNum: assistantTurn,
              imageUrls,
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
    const rawMessages = Array.isArray(loaded.messages) ? loaded.messages : [];
    const messages = [];
    for (const m of rawMessages) {
      let images = [];
      if (m.role === 'assistant' && Array.isArray(m.imageUrls) && m.imageUrls.length > 0) {
        images = await saveGeneratedImages(
          'gemini',
          'ai-web-gm-',
          loaded.id,
          m.turnNum || 1,
          m.imageUrls
        );
      }
      messages.push({
        role: m.role,
        thinking: m.thinking || '',
        thinking_ms: m.thinking_ms || null,
        text: m.text || '',
        files: Array.isArray(m.files) ? m.files : [],
        images,
        created_at_ms: m.created_at_ms || 0,
      });
    }
    await emit({
      event: 'session_loaded',
      id: loaded.id,
      title: loaded.title || null,
      messages,
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
    await ensureSidebarExpanded();
    const menuOpened = await js(`
      (() => {
        const sessionId = ${JSON.stringify(SESSION_ID)};
        const link = Array.from(
          document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
        ).find((a) => (a.getAttribute('href') || '').startsWith('/app/' + sessionId));
        const row = link ? link.parentElement : null;
        const moreBtn = row ? row.querySelector('button[aria-label*="更多选项"], button[aria-label*="More options"]') : null;
        if (!moreBtn) return false;
        moreBtn.click();
        return true;
      })()
    `).catch(() => false);
    if (!menuOpened) {
      await emit({ event: 'error', message: 'Gemini session not found in sidebar' });
      return;
    }
    await new Promise((r) => setTimeout(r, 250));
    const renameClicked = await js(String.raw`
      (() => {
        const items = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitem"], .cdk-overlay-container button'));
        const renameBtn = items.find((el) => {
          const t = (el.innerText || el.getAttribute('aria-label') || '').trim().toLowerCase();
          return t.includes('重命名') || t.includes('rename');
        });
        if (!renameBtn) return false;
        renameBtn.click();
        return true;
      })()
    `).catch(() => false);
    if (!renameClicked) {
      await emit({ event: 'error', message: 'Gemini rename menu item not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 280));
    const inputFilled = await js(`
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
    if (!inputFilled) {
      await emit({ event: 'error', message: 'Gemini rename dialog not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 180));
    const saved = await js(String.raw`
      (() => {
        const buttons = Array.from(document.querySelectorAll('.cdk-overlay-container button, mat-dialog-container button'));
        const saveBtn = buttons.find((b) => {
          const t = (b.innerText || '').trim().toLowerCase();
          return t === '重命名' || t === '保存' || t === 'rename' || t === 'save';
        });
        if (!saveBtn || saveBtn.disabled) return false;
        saveBtn.click();
        return true;
      })()
    `).catch(() => false);
    if (!saved) {
      await emit({ event: 'error', message: 'Gemini rename save button not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 400));
    await emit({ event: 'done' });
    return;
  }

  if (ACTION === 'delete_session') {
    if (!SESSION_ID) {
      await emit({ event: 'error', message: 'missing session id' });
      return;
    }
    await ensureSidebarExpanded();
    const menuOpened = await js(`
      (() => {
        const sessionId = ${JSON.stringify(SESSION_ID)};
        const link = Array.from(
          document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
        ).find((a) => (a.getAttribute('href') || '').startsWith('/app/' + sessionId));
        const row = link ? link.parentElement : null;
        const moreBtn = row ? row.querySelector('button[aria-label*="更多选项"], button[aria-label*="More options"]') : null;
        if (!moreBtn) return false;
        moreBtn.click();
        return true;
      })()
    `).catch(() => false);
    if (!menuOpened) {
      await emit({ event: 'error', message: 'Gemini session not found in sidebar' });
      return;
    }
    await new Promise((r) => setTimeout(r, 250));
    const deleteClicked = await js(String.raw`
      (() => {
        const items = Array.from(document.querySelectorAll('.cdk-overlay-container [role="menuitem"], .cdk-overlay-container button'));
        const delBtn = items.find((el) => {
          const t = (el.innerText || el.getAttribute('aria-label') || '').trim().toLowerCase();
          return t.includes('删除') || t.includes('delete');
        });
        if (!delBtn) return false;
        delBtn.click();
        return true;
      })()
    `).catch(() => false);
    if (!deleteClicked) {
      await emit({ event: 'error', message: 'Gemini delete menu item not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 280));
    const confirmed = await js(String.raw`
      (() => {
        const buttons = Array.from(document.querySelectorAll('.cdk-overlay-container button, mat-dialog-container button'));
        const confirmBtn = buttons.find((b) => {
          const t = (b.innerText || '').trim().toLowerCase();
          return t === '删除' || t === 'delete' || b.getAttribute('data-test-id') === 'confirm-button';
        });
        if (!confirmBtn) return false;
        confirmBtn.click();
        return true;
      })()
    `).catch(() => false);
    if (!confirmed) {
      await emit({ event: 'error', message: 'Gemini delete confirmation not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 250));
    if (currentUrl.includes(SESSION_ID)) {
      await openOrReuseTab(HOME, { wait: true, timeout: 20 });
    }
    await emit({ event: 'done' });
    return;
  }

  await emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
}
