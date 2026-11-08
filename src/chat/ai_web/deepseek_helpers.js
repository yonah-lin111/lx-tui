// DeepSeek web driver shared helpers, attachment sync, and session management actions.
//
// Prepended before `deepseek_script.js` by `src/event/chat/ai_web.rs`.

const PREFIX = '@@LX_AI_WEB@@';
const SPACE = 'lx-tui-ai-web-deepseek';
const HOME = 'https://chat.deepseek.com';
const SIGN_IN = 'https://chat.deepseek.com/sign_in';
const PARAMS = typeof __LX_PARAMS === 'object' && __LX_PARAMS !== null ? __LX_PARAMS : {};
const ACTION = PARAMS.action || 'chat';
const MODEL = PARAMS.model || 'DeepSeek';
const THINKING = PARAMS.thinking || 'deepthink';
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
        return label === '开启新对话' || label === '新对话' || label === 'New chat';
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
  const cleanupResult = await js(`
    (() => {
      const editArea = document.querySelector('textarea[name="user query"]');
      const scope = ${inEditBox ? '(editArea ? editArea.closest("[data-virtual-list-item-key]") : null)' : 'document'};
      if (!scope) return { closedCount: 0, keptCount: 0 };
      const items = Array.from(scope.querySelectorAll('.ds-animated-size-item')).filter(
        (el) => !(${inEditBox ? 'false' : 'true'} && el.closest('[data-virtual-list-item-key]')),
      );
      const remainingKeep = ${inEditBox ? JSON.stringify(KEEP_FILES) : '[]'}.slice();
      let closedCount = 0;
      let keptCount = 0;
      for (const item of items) {
        const img = item.querySelector('img[alt]');
        const name = img
          ? (img.getAttribute('alt') || '').trim()
          : (((item.innerText || '').split('\\n')[0] || '').trim());
        const keepIdx = remainingKeep.indexOf(name);
        if (${inEditBox ? 'true' : 'false'} && name && keepIdx !== -1) {
          remainingKeep.splice(keepIdx, 1);
          keptCount += 1;
          continue;
        }
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
          closedCount += 1;
        }
      }
      return { closedCount, keptCount };
    })()
  `).catch(() => ({ closedCount: 0, keptCount: 0 }));

  const keptCount = (cleanupResult && cleanupResult.keptCount) || 0;
  if (cleanupResult && cleanupResult.closedCount > 0) {
    const clearDeadline = Date.now() + 3000;
    while (Date.now() < clearDeadline) {
      await new Promise((r) => setTimeout(r, 80));
      const remaining = await js(`
        (() => {
          const editArea = document.querySelector('textarea[name="user query"]');
          const scope = ${inEditBox ? '(editArea ? editArea.closest("[data-virtual-list-item-key]") : null)' : 'document'};
          if (!scope) return 0;
          return Array.from(scope.querySelectorAll('.ds-animated-size-item')).filter(
            (el) => !(${inEditBox ? 'false' : 'true'} && el.closest('[data-virtual-list-item-key]')),
          ).length;
        })()
      `).catch(() => 0);
      if (remaining <= keptCount) break;
    }
  }

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

  const expectedCount = keptCount + FILES.length;
  const knownNames = [
    ...KEEP_FILES,
    ...FILES.map((p) => String(p).split(/[\\\\/]/).pop() || ''),
  ].filter(Boolean);

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
        if (items.length < ${expectedCount}) return { count: items.length, uploading: true, error: null };
        const known = ${JSON.stringify(knownNames)};
        let uploading = false;
        let error = null;
        for (const item of items) {
          let statusText = (item.innerText || '').trim();
          for (const name of known) {
            if (name) statusText = statusText.split(name).join(' ');
          }
          const fk = Object.keys(item).find(
            (k) => k.startsWith('__reactFiber') || k.startsWith('__reactInternalInstance')
          );
          const childProps =
            fk &&
            item[fk] &&
            item[fk].memoizedProps &&
            item[fk].memoizedProps.children &&
            item[fk].memoizedProps.children.props;
          const fileObj = childProps && childProps.file;
          const snapObj = childProps && childProps.snapshot;
          if (
            snapObj &&
            ((snapObj.fileErrorState && snapObj.fileErrorState.hasError) ||
              (snapObj.fileUploadInfo && snapObj.fileUploadInfo.failed))
          ) {
            error = String((fileObj && fileObj.errorCode) || statusText || 'upload failed').trim();
            break;
          }
          if (fileObj && fileObj.status && /FAIL|ERROR|REJECT/i.test(String(fileObj.status))) {
            error = String(fileObj.errorCode || fileObj.status);
            break;
          }
          if (/上传失败|解析失败|不支持|超出|过大|upload failed|parse failed|unsupported|exceed/i.test(statusText)) {
            error = statusText.replace(/\\s+/g, ' ').trim();
            break;
          }
          if (
            (snapObj && snapObj.fileUploadInfo && snapObj.fileUploadInfo.isUploading) ||
            (fileObj && fileObj.status !== 'SUCCESS') ||
            /上传中|解析中|uploading|parsing/i.test(statusText) ||
            item.querySelector('svg[viewBox="0 0 20 20"][role="img"]')
          ) {
            uploading = true;
          }
        }
        return { count: items.length, uploading, error };
      })()
    `).catch(() => ({ count: 0, uploading: true, error: null }));
    if (status.error) throw new Error('File upload rejected by DeepSeek: ' + status.error);
    if (status.count >= expectedCount && !status.uploading) {
      await new Promise((r) => setTimeout(r, 100));
      return;
    }
  }
  throw new Error('Timed out waiting for DeepSeek file upload');
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
        const res = await fetch('/api/v0/chat_session/fetch_page?lte_cursor.pinned=false', {
          headers: { authorization: 'Bearer ' + token },
        });
        if (!res.ok) throw new Error('HTTP ' + res.status);
        const resp = await res.json();
        const biz = resp && resp.data && resp.data.biz_data;
        if (!biz || !Array.isArray(biz.chat_sessions)) {
          throw new Error('Invalid DeepSeek session list response');
        }
        return biz.chat_sessions.map((s) => ({
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
        const [historyHttp, pageHttp] = await Promise.all([
          fetch('/api/v0/chat/history_messages?chat_session_id=' + encodeURIComponent(sessionId), { headers }),
          fetch('/api/v0/chat_session/fetch_page?lte_cursor.pinned=false', { headers }).catch(() => null),
        ]);
        if (!historyHttp || !historyHttp.ok) {
          throw new Error('Failed to fetch DeepSeek session history');
        }
        const historyResp = await historyHttp.json();
        const historyBiz = historyResp && historyResp.data && historyResp.data.biz_data;
        if (!historyBiz || !Array.isArray(historyBiz.chat_messages)) {
          throw new Error('Invalid DeepSeek session history payload');
        }
        const pageResp = pageHttp && pageHttp.ok ? await pageHttp.json().catch(() => null) : null;
        const rawMessages = historyBiz.chat_messages;
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
          const res = await fetch('/api/v0/chat_session/update_title', {
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
          return res.ok;
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
          const res = await fetch('/api/v0/chat_session/delete', {
            method: 'POST',
            headers: {
              authorization: 'Bearer ' + token,
              'content-type': 'application/json',
            },
            body: JSON.stringify({ chat_session_ids: [sessionId] }),
          });
          return res.ok;
        })()
      `);
      if (currentUrl.includes(SESSION_ID)) {
        await openOrReuseTab(HOME, { wait: true, timeout: 20 });
      }
    }
    await emit({ event: 'done' });
    return;
  }

  await emit({ event: 'error', message: 'unknown AI Web action: ' + ACTION });
}
