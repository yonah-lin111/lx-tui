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
  return 'jpg';
}

function resolveGrokAssetUrl(raw) {
  const s = String(raw || '').trim();
  if (!s) return '';
  if (/^https?:\/\//i.test(s)) return s;
  return 'https://assets.grok.com/' + s.replace(/^\/+/, '');
}

function toGrokOriginalImageUrl(previewUrl) {
  const s = String(previewUrl || '').trim();
  if (!s) return '';
  return s.replace(/\/image\.([a-zA-Z0-9]+)(\?.*)?$/, '/original_image.$1$2');
}

function extractGrokGeneratedImageUrls(r) {
  if (!r || r.sender === 'human') return [];
  const previewUrls = [];
  const cards = Array.isArray(r.cardAttachmentsJson) ? r.cardAttachmentsJson : [];
  for (const raw of cards) {
    try {
      const card = typeof raw === 'string' ? JSON.parse(raw) : raw;
      if (
        card &&
        (card.cardType === 'generated_image_card' || card.type === 'render_generated_image') &&
        card.image_chunk &&
        card.image_chunk.imageUrl
      ) {
        const u = resolveGrokAssetUrl(card.image_chunk.imageUrl);
        if (u) previewUrls.push(u);
      }
    } catch (_) {}
  }
  const genUrls = Array.isArray(r.generatedImageUrls) ? r.generatedImageUrls : [];
  for (const u of genUrls) {
    const resolved = resolveGrokAssetUrl(u);
    if (resolved) previewUrls.push(resolved);
  }
  if (previewUrls.length === 0) {
    const meta = Array.isArray(r.fileAttachmentsMetadata) ? r.fileAttachmentsMetadata : [];
    for (const f of meta) {
      const uri = String((f && f.fileUri) || '');
      if (uri.includes('/generated/')) {
        const resolved = resolveGrokAssetUrl(uri);
        if (resolved) previewUrls.push(resolved);
      }
    }
  }
  const origByPreview = new Map();
  const assetMeta = Array.isArray(r.fileAttachmentAssetMetadata)
    ? r.fileAttachmentAssetMetadata
    : [];
  for (const asset of assetMeta) {
    if (!asset || !asset.key) continue;
    const prev = resolveGrokAssetUrl(asset.key);
    const origRaw = asset.auxKeys && asset.auxKeys['original-image'];
    if (prev && origRaw) {
      origByPreview.set(prev, resolveGrokAssetUrl(origRaw));
    }
  }
  const uniquePreviews = Array.from(new Set(previewUrls));
  return uniquePreviews.map((previewUrl) => ({
    previewUrl,
    originalUrl: origByPreview.get(previewUrl) || toGrokOriginalImageUrl(previewUrl),
  }));
}

async function downloadImageBuffer(frameId, startUrl) {
  if (!frameId || !startUrl) return null;
  try {
    const netRes = await cdp('Network.loadNetworkResource', {
      frameId,
      url: startUrl,
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
    if (buf.length > 0) {
      return { buf, contentType, finalUrl: startUrl };
    }
  } catch (_) {}
  return null;
}

async function saveGeneratedImages(siteTag, sessionPrefix, rawSessionId, turnNum, urls) {
  const rawList = Array.isArray(urls) ? urls.filter(Boolean) : [];
  if (rawList.length === 0) return [];
  const items = [];
  const seenPreview = new Set();
  for (const entry of rawList) {
    const previewUrl = typeof entry === 'string' ? entry : entry.previewUrl;
    if (!previewUrl || seenPreview.has(previewUrl)) continue;
    seenPreview.add(previewUrl);
    const originalUrl =
      (typeof entry === 'object' && entry.originalUrl) || toGrokOriginalImageUrl(previewUrl);
    items.push({ previewUrl, originalUrl });
  }
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
  for (let i = 0; i < items.length; i += 1) {
    const { previewUrl, originalUrl } = items[i];
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

// `/responses` 的 message 会把引用以 `<grok:render ...>` 原样返回；网页端实际
// 渲染为来源名链接。历史会话加载时按 cardAttachmentsJson 映射成 Markdown 链接。
function citationSourceLabel(url) {
  try {
    const host = new URL(url).hostname.replace(/^www\./, '');
    const parts = host.split('.').filter(Boolean);
    let name = host;
    if (parts.length >= 2) {
      const last = parts[parts.length - 1];
      const secondLast = parts[parts.length - 2];
      const compound =
        last.length === 2 &&
        ['com', 'co', 'org', 'net', 'gov', 'edu', 'ac'].includes(secondLast) &&
        parts.length >= 3;
      name = compound ? parts[parts.length - 3] : secondLast;
    }
    if (!name) return host;
    return name.charAt(0).toUpperCase() + name.slice(1);
  } catch (_) {
    return 'source';
  }
}

function parseCitationCards(cardAttachmentsJson) {
  const cards = new Map();
  const list = Array.isArray(cardAttachmentsJson) ? cardAttachmentsJson : [];
  for (const raw of list) {
    try {
      const card = typeof raw === 'string' ? JSON.parse(raw) : raw;
      if (card && card.id && card.url) {
        cards.set(String(card.id), card);
      }
    } catch (_) {}
  }
  return cards;
}

function citationMarkdown(url, label) {
  const raw = String(url || '').trim();
  if (!/^https?:\/\//i.test(raw)) return '';
  const safeUrl = raw
    .replace(/\(/g, '%28')
    .replace(/\)/g, '%29')
    .replace(/\s/g, '%20');
  const safeLabel = String(label || 'source').replace(/[\[\]]/g, '').trim() || 'source';
  return '[' + safeLabel + '](' + safeUrl + ')';
}

function citationFromAttrs(attrs, cards) {
  const id = /card_id\s*=\s*["']([^"']+)["']/i.exec(attrs);
  const card = id ? cards.get(id[1]) : null;
  if (!card || !card.url) return '';
  return citationMarkdown(card.url, citationSourceLabel(card.url));
}

function replaceGrokCitations(message, cardAttachmentsJson) {
  const cards = parseCitationCards(cardAttachmentsJson);
  let out = String(message || '');
  out = out.replace(
    /<grok:render\b([^>]*)>[\s\S]*?<\/grok:render>/gi,
    (_full, attrs, offset, source) => {
      const link = citationFromAttrs(attrs, cards);
      if (!link) return '';
      const before = source[offset - 1] || '';
      const gap = before && !/\s/.test(before) ? ' ' : '';
      return gap + link;
    }
  );
  // 自闭合引用同样按 card_id 映射，不能无条件删除而丢掉来源链接。
  out = out.replace(/<grok:render\b([^>]*)\/>/gi, (_full, attrs, offset, source) => {
    const link = citationFromAttrs(attrs, cards);
    if (!link) return '';
    const before = source[offset - 1] || '';
    const gap = before && !/\s/.test(before) ? ' ' : '';
    return gap + link;
  });
  // 截断会话可能只剩 <argument> 或未闭合的 <grok:render>；逐类清掉，
  // 避免把引用编号原样泄漏到 TUI。
  out = out
    .replace(/<argument\b[^>]*>[\s\S]*?<\/argument>/gi, '')
    .replace(/<argument\b[^>]*>/gi, '')
    .replace(/<\/argument>/gi, '')
    .replace(/<grok:render\b[^>]*>/gi, '')
    .replace(/<\/grok:render>/gi, '')
    .replace(/<\/?grok:[^>]*>?/gi, '')
    .replace(/\u2060/g, '')
    .trim();
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

async function emitLoginRequired() {
  await handOffTaskSpace().catch(() => {});
  await emit({ event: 'login_required', url: HOME });
}

async function grokLoginState() {
  // Grok 会给匿名访客自动建 userId，只看 cookie 会误判；必须要求 email 非空。
  // 返回值保留 HTTP 状态：401/200 匿名才是未登录，5xx/网络错误不能伪装成登录墙。
  const state = await js(String.raw`
    (async () => {
      try {
        const res = await fetch('/rest/auth/get-user', { credentials: 'include' });
        if (!res.ok) return { ok: false, status: res.status };
        const user = await res.json();
        const email = Boolean(user && user.email);
        return { ok: email, anonymous: !email, status: res.status };
      } catch (error) {
        return { ok: false, status: -1, message: String((error && error.message) || error) };
      }
    })()
  `).catch((error) => ({
    ok: false,
    status: -1,
    message: String((error && error.message) || error),
  }));
  return state && typeof state === 'object' ? state : { ok: false, status: -1 };
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
  const matches = (value) => {
    // 按钮标签可能是 "Build\nBeta"；只比较第一段模型名，菜单项仍按完整文本匹配。
    const first = String(value || '').trim().toLowerCase().split(/\s+/)[0];
    return first === want;
  };
  const buttonLabel = `
    (() => {
      const buttons = Array.from(document.querySelectorAll(${JSON.stringify(GROK_MODEL_BUTTON_SELECTOR)}));
      const btn = buttons.find((b) => b.closest('form')) || buttons[0];
      return btn ? (btn.innerText || '').trim() : '';
    })()
  `;

  // 已经是目标模型时不必开关菜单，避免无意义的 350ms 等待和误点其它同名按钮。
  const current = await js(buttonLabel).catch(() => '');
  if (matches(current)) return;

  // 按钮和菜单项共用同一个 15s deadline；菜单动画/慢网络不再一次查询就硬失败。
  const menuDeadline = Date.now() + 15000;
  let picked = null;
  while (Date.now() < menuDeadline) {
    picked = await js(`
      (() => {
        const buttons = Array.from(document.querySelectorAll(${JSON.stringify(GROK_MODEL_BUTTON_SELECTOR)}));
        const btn = buttons.find((b) => b.closest('form')) || buttons[0];
        if (!btn) return { button: false, found: false };
        if (btn.getAttribute('aria-expanded') !== 'true') btn.click();
        const want = ${JSON.stringify(want)};
        const items = Array.from(document.querySelectorAll('[role="menuitemradio"]'));
        const match = items.find((item) => (item.innerText || '').trim().toLowerCase() === want);
        if (!match) return { button: true, found: false };
        if (match.getAttribute('aria-checked') !== 'true') match.click();
        return { button: true, found: true, clicked: match.getAttribute('aria-checked') !== 'true' };
      })()
    `).catch(() => ({ button: false, found: false }));
    if (picked && picked.found) break;
    await new Promise((r) => setTimeout(r, 200));
  }
  if (!picked || !picked.found) {
    throw new Error('Grok model "' + MODEL + '" not found');
  }
  await new Promise((r) => setTimeout(r, 350));

  // 后置校验：按钮标签必须以目标模型开头（Build 会附带 "Beta"）。
  const label = await js(buttonLabel).catch(() => '');
  if (!matches(label)) {
    throw new Error('Grok model "' + MODEL + '" was not applied');
  }

  // 已选中时菜单不会自动收起，避免遮罩挡住后续输入。
  await js(`
    (() => {
      const buttons = Array.from(document.querySelectorAll(${JSON.stringify(GROK_MODEL_BUTTON_SELECTOR)}));
      const btn = buttons.find((b) => b.closest('form')) || buttons[0];
      if (btn && btn.getAttribute('aria-expanded') === 'true') btn.click();
      return true;
    })()
  `).catch(() => {});
}

async function grokAttachmentCount() {
  return await js(`
    (() => document.querySelectorAll(${JSON.stringify(GROK_ATTACHMENT_CHIP_SELECTOR)}).length)()
  `).catch(() => -1);
}

async function syncComposerAttachments() {
  // 只清理输入框 <form> 内的附件 chip，绝不触碰消息气泡里的附件。
  // 必须删到 0 个再上传；固定轮数会漏掉第 6 个及以后的残留附件。
  const clearDeadline = Date.now() + 10000;
  while (Date.now() < clearDeadline) {
    const count = await grokAttachmentCount();
    if (count === 0) break;
    if (count < 0) {
      throw new Error('Grok attachment list not accessible');
    }
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
    if (!removed) {
      throw new Error('Grok attachment remove button not found');
    }
    await new Promise((r) => setTimeout(r, 180));
  }
  const remaining = await grokAttachmentCount();
  if (remaining !== 0) {
    throw new Error('Grok could not clear existing attachments (chips=' + remaining + ')');
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
    `).catch(() => ({ chips: -1, inputFiles: -1 }));
    lastCount = state.chips;
    lastInputFiles = state.inputFiles;
    // 清理后应为 0；上传完成后必须正好是本轮文件数，不能把旧 chip 当成新文件。
    if (lastCount === FILES.length) return;
  }
  throw new Error(
    'Timed out waiting for Grok file upload (chips=' +
      lastCount +
      ', expected=' +
      FILES.length +
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
      await emit({ event: 'error', message: 'Failed to download original image from Grok' });
      return;
    }
    const ext = detectImageExt(fetched.buf, fetched.contentType, fetched.finalUrl || DOWNLOAD_URL);
    const fileName = (DOWNLOAD_STEM || 'grok-orig') + '.' + ext;
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
      await emitLoginRequired();
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
          let assistantTurn = 0;
          const messages = chain.map((r) => {
            if (r.sender !== 'human') assistantTurn += 1;
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
              turnNum: assistantTurn,
              created_at_ms: Date.parse(r.createTime || '') || 0,
              cardAttachmentsJson: Array.isArray(r.cardAttachmentsJson)
                ? r.cardAttachmentsJson
                : [],
              generatedImageUrls: Array.isArray(r.generatedImageUrls)
                ? r.generatedImageUrls
                : [],
              fileAttachmentsMetadata: meta,
              fileAttachmentAssetMetadata: Array.isArray(r.fileAttachmentAssetMetadata)
                ? r.fileAttachmentAssetMetadata
                : [],
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
      await emitLoginRequired();
      return;
    }
    if (!result || !result.ok) {
      await emit({
        event: 'error',
        message: 'Grok session load failed (' + httpDetail(result) + ')',
      });
      return;
    }
    const rawMessages = Array.isArray(result.messages) ? result.messages : [];
    const messages = [];
    for (const m of rawMessages) {
      let images = [];
      if (m.role === 'assistant') {
        const imageUrls = extractGrokGeneratedImageUrls({
          sender: 'assistant',
          cardAttachmentsJson: m.cardAttachmentsJson,
          generatedImageUrls: m.generatedImageUrls,
          fileAttachmentsMetadata: m.fileAttachmentsMetadata,
          fileAttachmentAssetMetadata: m.fileAttachmentAssetMetadata,
        });
        if (imageUrls.length > 0) {
          images = await saveGeneratedImages(
            'grok',
            'ai-web-grok-',
            SESSION_ID,
            m.turnNum || 1,
            imageUrls
          );
        }
      }
      messages.push({
        role: m.role,
        thinking: m.thinking,
        thinking_ms: m.thinking_ms,
        text:
          m.role === 'assistant'
            ? replaceGrokCitations(m.text, m.cardAttachmentsJson)
            : m.text,
        files: m.files,
        images,
        created_at_ms: m.created_at_ms,
      });
    }
    await emit({
      event: 'session_loaded',
      id: SESSION_ID,
      title: result.title || null,
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
      await emitLoginRequired();
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
      await emitLoginRequired();
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
