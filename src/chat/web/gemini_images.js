// Google Gemini web driver image helpers: preview download, original-resolution URL
// derivation, generated-image persistence, and the `download_image` action.
//
// Prepended after `gemini_helpers.js` and before `gemini_script.js` by
// `src/chat/web/mod.rs`; all three files share one top-level script scope.

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
  // 先剥离查询串：已带 `?alr=yes` 的 URL 直接跑尺寸后缀正则会把 `=yes` 误删。
  const base = s.replace(/\?.*$/, '').replace(/=[^=/]+$/, '');
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

async function saveGeneratedImages(
  siteTag,
  sessionPrefix,
  rawSessionId,
  turnNum,
  items,
  overwrite = false
) {
  const rawList = Array.isArray(items) ? items.filter(Boolean) : [];
  if (rawList.length === 0) return [];
  const safeTurn = Math.max(1, Math.trunc(Number(turnNum) || 1));
  const list = [];
  const seen = new Set();
  for (const entry of rawList) {
    const previewUrl = typeof entry === 'string' ? entry : entry.previewUrl || '';
    const dataUrl = typeof entry === 'object' && entry.dataUrl ? entry.dataUrl : '';
    if (!previewUrl || seen.has(previewUrl)) continue;
    seen.add(previewUrl);
    list.push({ previewUrl, dataUrl });
  }
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
    const { previewUrl, dataUrl } = list[i];
    const isHttp = /^https?:\/\//i.test(previewUrl);
    const originalUrl = isHttp
      ? toGeminiOriginalImageUrl(previewUrl)
      : 'gemini-session://' + cleanSid + '/' + String(safeTurn) + '/' + String(i + 1);
    const baseStem = siteTag + '-r' + String(safeTurn) + '-' + String(i + 1);
    const fileStem =
      siteTag + '-' + shortSid + '-r' + String(safeTurn) + '-' + String(i + 1) + '-orig';
    if (imageDir && !overwrite) {
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
    let buf = null;
    let contentType = '';
    if (dataUrl && dataUrl.startsWith('data:image/')) {
      const comma = dataUrl.indexOf(',');
      if (comma > 0) {
        buf = Buffer.from(dataUrl.slice(comma + 1), 'base64');
        contentType = dataUrl.slice(5, comma).split(';')[0] || 'image/webp';
      }
    } else if (isHttp) {
      const fetched = await downloadImageBuffer(frameId, previewUrl);
      buf = fetched && fetched.buf;
      contentType = (fetched && fetched.contentType) || '';
    }
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

async function runGeminiImageDownload(currentUrl) {
  const nodeFs = await import('node:fs');
  const nodePath = await import('node:path');
  if (!DOWNLOAD_DIR || (!DOWNLOAD_URL && !FALLBACK_URL)) {
    await emit({ event: 'error', message: 'Missing download target parameters' });
    return;
  }
  let resolvedDownloadUrl = DOWNLOAD_URL;
  let resolvedFallbackUrl = FALLBACK_URL;
  const sessionMatch = String(DOWNLOAD_URL || '').match(
    /^gemini-session:\/\/([a-zA-Z0-9_-]+)\/(\d+)\/(\d+)/
  );
  if (sessionMatch) {
    const [, sid, turnStr, idxStr] = sessionMatch;
    const turnIdx = Math.max(0, Number(turnStr) - 1);
    const imgIdx = Math.max(0, Number(idxStr) - 1);
    let effectiveSid = sid && sid !== 'temp' ? sid : '';
    if (!effectiveSid) {
      const m = String(currentUrl || '').match(/\/app\/([a-zA-Z0-9_-]{8,})/);
      if (m) effectiveSid = m[1];
    }
    const readHttpSrc = () =>
      js(String.raw`
        (() => {
          const responses = Array.from(document.querySelectorAll('model-response'));
          const mr = responses[${turnIdx}] || responses[responses.length - 1];
          if (!mr) return '';
          const imgs = Array.from(
            mr.querySelectorAll('generated-image img, single-image img, .generated-images img')
          );
          const img = imgs[${imgIdx}] || imgs[0];
          const src = img ? (img.currentSrc || img.src || '').trim() : '';
          if (/^https?:\/\//i.test(src)) return src;
          const allImgs = Array.from(
            document.querySelectorAll('model-response generated-image img, model-response single-image img, model-response .generated-images img')
          );
          const flatIdx = img ? allImgs.indexOf(img) : -1;
          const ggUrls = Array.from(new Set(
            performance.getEntriesByType('resource')
              .map((e) => e.name)
              .filter((n) => /^https:\/\/lh3\.googleusercontent\.com\/gg\//i.test(n))
              .map((n) => n.replace(/\?alr=yes$/i, ''))
          ));
          if (
            flatIdx >= 0 &&
            flatIdx < ggUrls.length &&
            ggUrls.length >= allImgs.length
          ) {
            return ggUrls[flatIdx];
          }
          return '';
        })()
      `).catch(() => '');
    // Reload the saved conversation URL so Gemini populates permanent FIFE gg/ URLs in resource timings.
    if (effectiveSid) {
      if (!currentUrl.includes(effectiveSid)) {
        await openOrReuseTab(HOME + '/' + effectiveSid, { wait: true, timeout: 25 });
      } else if (!(await readHttpSrc())) {
        await cdp('Page.reload', { ignoreCache: false }).catch(() => {});
        await new Promise((r) => setTimeout(r, 1500));
      }
      await installPermissionGuard();
      const waitDeadline = Date.now() + 12000;
      while (Date.now() < waitDeadline) {
        const foundUrl = await readHttpSrc();
        if (foundUrl) {
          resolvedFallbackUrl = foundUrl;
          resolvedDownloadUrl = toGeminiOriginalImageUrl(foundUrl);
          break;
        }
        await new Promise((r) => setTimeout(r, 250));
      }
    }
  }
  let frameId = '';
  try {
    const tree = await cdp('Page.getResourceTree');
    frameId = (tree && tree.frameTree && tree.frameTree.frame && tree.frameTree.frame.id) || '';
  } catch (_) {}
  let fetched = /^https?:\/\//i.test(resolvedDownloadUrl)
    ? await downloadImageBuffer(frameId, resolvedDownloadUrl)
    : null;
  if ((!fetched || !fetched.buf) && /^https?:\/\//i.test(resolvedFallbackUrl)) {
    fetched = await downloadImageBuffer(frameId, resolvedFallbackUrl);
  }
  if ((!fetched || !fetched.buf) && sessionMatch) {
    const turnIdx = Math.max(0, Number(sessionMatch[2]) - 1);
    const imgIdx = Math.max(0, Number(sessionMatch[3]) - 1);
    const dataUrl = await js(`
      (() => {
        const responses = Array.from(document.querySelectorAll('model-response'));
        const mr = responses[${turnIdx}] || responses[responses.length - 1];
        if (!mr) return '';
        const imgs = Array.from(
          mr.querySelectorAll('generated-image img, single-image img, .generated-images img')
        );
        const img = imgs[${imgIdx}] || imgs[0];
        if (!img || !img.complete || img.naturalWidth === 0) return '';
        try {
          const c = document.createElement('canvas');
          c.width = img.naturalWidth;
          c.height = img.naturalHeight;
          c.getContext('2d').drawImage(img, 0, 0);
          return c.toDataURL('image/png');
        } catch (_) {
          return '';
        }
      })()
    `).catch(() => '');
    if (dataUrl && dataUrl.startsWith('data:image/')) {
      const comma = dataUrl.indexOf(',');
      if (comma > 0) {
        fetched = {
          buf: Buffer.from(dataUrl.slice(comma + 1), 'base64'),
          contentType: 'image/png',
          finalUrl: '',
        };
      }
    }
  }
  if (!fetched || !fetched.buf || fetched.buf.length === 0) {
    await emit({ event: 'error', message: 'Failed to download original image from Gemini' });
    return;
  }
  const ext = detectImageExt(
    fetched.buf,
    fetched.contentType,
    fetched.finalUrl || resolvedDownloadUrl
  );
  const downloadStem =
    nodePath
      .basename(String(DOWNLOAD_STEM || 'gemini-orig'))
      .replace(/[^a-zA-Z0-9._-]/g, '_') || 'gemini-orig';
  const fileName = downloadStem + '.' + ext;
  try {
    nodeFs.mkdirSync(DOWNLOAD_DIR, { recursive: true });
    const target = nodePath.join(DOWNLOAD_DIR, fileName);
    nodeFs.writeFileSync(target, fetched.buf);
    await emit({
      event: 'image',
      label: fileName,
      path: target,
      original_url: resolvedDownloadUrl,
      preview_url: resolvedFallbackUrl,
      file_stem: downloadStem,
    });
    await emit({ event: 'done' });
  } catch (error) {
    await emit({
      event: 'error',
      message: 'Failed to save original image: ' + String((error && error.message) || error),
    });
  }
}
