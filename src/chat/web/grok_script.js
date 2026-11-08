// xAI Grok web chat stream driver for `ego-browser nodejs`.
//
// Requires `grok_helpers.js` to be prepended ahead of this file.

async function waitGrokHistoryReady() {
  // 会话页是 SPA：composer 可能先于历史消息挂载。等消息数量稳定后再安装采集器，
  // 避免把随后渲染出来的旧回答当成这一轮的输出。
  const deadline = Date.now() + 8000;
  let previous = -1;
  let stable = 0;
  let sawUser = false;
  let sawAssistant = false;
  while (Date.now() < deadline) {
    const counts = await js(`
      (() => ({
        users: document.querySelectorAll('[data-testid="user-message"]').length,
        assistants: document.querySelectorAll('[data-testid="assistant-message"]').length,
      }))()
    `).catch(() => null);
    if (counts && counts.users > 0) sawUser = true;
    if (counts && counts.assistants > 0) sawAssistant = true;
    const total = counts ? counts.users + counts.assistants : -1;
    if (sawUser && sawAssistant && total > 0 && total === previous) {
      stable += 1;
      if (stable >= 2) return;
    } else {
      stable = 0;
      previous = total;
    }
    await new Promise((r) => setTimeout(r, 200));
  }
}

async function clickGrokSubmit(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const clicked = await js(`
      (() => {
        const btn = document.querySelector(${JSON.stringify(GROK_SUBMIT_SELECTOR)});
        if (!btn || btn.disabled || btn.getAttribute('aria-disabled') === 'true') return false;
        btn.click();
        return true;
      })()
    `).catch(() => false);
    if (clicked) return true;
    await new Promise((r) => setTimeout(r, 200));
  }
  return false;
}

async function main() {
  await activateTaskSpace();
  const current = await pageInfo().catch(() => null);
  const currentUrl = (current && current.url) || '';
  if (!currentUrl.startsWith('https://grok.com')) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
  }
  await dismissCookieBanner();
  await installPermissionGuard();

  // 登录态由 API 判定，不依赖页面是否渲染完成；登录后再等 composer，避免
  // 把“页面仍在冷启动”误判成“未登录”，也避免把登录页里的普通 sign-in 链接当成登录墙。
  let loggedIn = false;
  const loginDeadline = Date.now() + 20000;
  while (Date.now() < loginDeadline) {
    loggedIn = await grokLoggedIn();
    if (loggedIn) break;
    await new Promise((r) => setTimeout(r, 300));
  }
  if (!loggedIn) {
    await handOffTaskSpace();
    await emit({ event: 'login_required', url: HOME });
    return;
  }

  // 冷启动/慢网络：ProseMirror 可能晚于登录态挂载。
  if (!(await ensureGrokComposer())) {
    await emit({ event: 'error', message: 'Grok composer not found' });
    return;
  }

  if (ACTION !== 'chat') {
    await runNonChatAction(currentUrl);
    return;
  }

  if (FIRST_TURN && EDIT_INDEX < 0) {
    await ensureFreshChat();
    if (!(await ensureGrokComposer())) {
      await emit({ event: 'error', message: 'Grok composer not found after /new' });
      return;
    }
  } else {
    if (SESSION_ID && !currentUrl.includes(SESSION_ID)) {
      await openOrReuseTab(HOME + 'c/' + SESSION_ID, { wait: true, timeout: 25 });
      if (!(await ensureGrokComposer())) {
        await emit({ event: 'error', message: 'Grok composer not found for session' });
        return;
      }
    }
    await waitGrokHistoryReady();
  }

  // 发送前同步网页端的模型档位；失败直接报错，不静默沿用旧模型。
  try {
    await ensureGrokModel();
  } catch (error) {
    await emit({
      event: 'error',
      message: String((error && error.message) || error),
    });
    return;
  }

  // Set up DOM stream collector on window.__lxGrokStream. Grok 的 Edit 也是追加
  // 新轮次，因此普通续聊和 /rewind 的目标都恒为“发送前的下一条”回答。
  const runId = String(Date.now()) + '-' + String(Math.random());
  await js(String.raw`
    (() => {
      // 页面上下文执行：不能引用 Node 侧函数。
      const stripGrokDirectives = (text) =>
        String(text || '')
          .replace(/<grok:render\b[^>]*>[\s\S]*?<\/grok:render>/gi, '')
          .replace(/<grok:render\b[^>]*\/>/gi, '')
          .replace(/<grok:render\b[\s\S]*$/gi, '')
          .replace(/<\/?grok:[^>]*>?/gi, '')
          .replace(/\u2060/g, ' ')
          .trim();
      const existingResponses = Array.from(document.querySelectorAll('[data-testid="assistant-message"]'));
      const targetIndex = ${FIRST_TURN && EDIT_INDEX < 0 ? 0 : 'existingResponses.length'};
      window.__lxGrokStream = {
        runId: ${JSON.stringify(runId)},
        queue: [],
        done: false,
        started: false,
        targetIndex,
        lastText: '',
        lastSample: '',
        stableTicks: 0,
        extractText(el) {
          if (!el) return '';
          // 只读正文容器。助手消息头部有一个 sr-only 的 "Grok" 标题，
          // 正文尚未挂载时回退到 innerText 会把标题误当成回答输出。
          const body = el.querySelector('.response-content-markdown');
          if (!body) return '';
          return stripGrokDirectives((body.innerText || '').replace(/\r\n/g, '\n'));
        },
        pushText(current) {
          if (!current) return;
          const previous = this.lastText || '';
          if (current === previous) return;
          if (!previous) {
            this.lastText = current;
            this.started = true;
            this.queue.push({ kind: 'text', delta: current });
            return;
          }
          if (current.startsWith(previous)) {
            const delta = current.slice(previous.length);
            this.lastText = current;
            if (delta) {
              this.started = true;
              this.queue.push({ kind: 'text', delta });
            }
            return;
          }
          // streamdown 重渲染：用尾部锚点在 current 中重新对齐，避免重复输出已流过的段落。
          for (const len of [48, 32, 20, 12, 8, 5]) {
            if (previous.length < len) continue;
            const anchor = previous.slice(previous.length - len);
            const pos = current.lastIndexOf(anchor);
            if (pos >= 0 && pos + len >= current.length - 240) {
              const delta = current.slice(pos + len);
              if (delta.length > 0 || current.length >= previous.length) {
                this.lastText = current;
              }
              if (delta) {
                this.started = true;
                this.queue.push({ kind: 'text', delta });
              }
              return;
            }
          }
          // 尾部收缩（例如引用角标重新排版）时不回退 lastText，等重渲染追平。
          if (current.length > previous.length) this.lastText = current;
        },
        pollDom() {
          const messages = Array.from(document.querySelectorAll('[data-testid="assistant-message"]'));
          const target = messages[this.targetIndex];
          const stop = document.querySelector(${JSON.stringify(GROK_STOP_SELECTOR)});
          const generating = Boolean(stop);
          if (!target) {
            // 新回答尚未挂载：绝不复用上一轮已存在的回答。
            return;
          }
          if (generating) this.started = true;
          const text = this.extractText(target);
          if (text) this.pushText(text);

          const sample = text;
          if ((this.started || text.length > 0) && !generating) {
            if (sample === this.lastSample && text.length > 0) {
              this.stableTicks += 1;
              if (this.stableTicks >= 10) this.done = true;
            } else {
              this.lastSample = sample;
              this.stableTicks = 0;
            }
          } else {
            this.lastSample = sample;
            this.stableTicks = 0;
          }
        },
      };
      return true;
    })()
  `);

  // Submit either via inline edit (/rewind) or normal composer input.
  await ensureTaskSpaceControl();
  if (EDIT_INDEX >= 0) {
    const clicked = await js(`
      (() => {
        const users = Array.from(document.querySelectorAll('[data-testid="user-message"]'));
        const target = users[${EDIT_INDEX}];
        if (!target) return false;
        target.scrollIntoView({ block: 'center' });
        const edit = target.parentElement.querySelector('button[aria-label="Edit"]')
          || target.querySelector('button[aria-label="Edit"]');
        if (!edit) return false;
        edit.click();
        return true;
      })()
    `).catch(() => false);
    if (!clicked) {
      await emit({ event: 'error', message: 'Grok edit button not found for /rewind' });
      return;
    }
    await new Promise((r) => setTimeout(r, 500));
    const filled = await js(`
      (() => {
        const editor = document.querySelector(${JSON.stringify(GROK_EDITOR_SELECTOR)});
        if (!editor) return false;
        editor.focus();
        document.execCommand('selectAll', false, null);
        document.execCommand('insertText', false, ${JSON.stringify(PROMPT)});
        editor.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      })()
    `).catch(() => false);
    if (!filled) {
      await emit({ event: 'error', message: 'Grok edit composer not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 300));
    if (!(await clickGrokSubmit(4000))) {
      await emit({ event: 'error', message: 'Failed to submit edited Grok prompt' });
      return;
    }
  } else {
    // 普通发送：清理输入区残留附件，再注入本轮文件；/rewind 明确不支持附件。
    await syncComposerAttachments();
    const filled = await js(`
      (() => {
        const editor = document.querySelector(${JSON.stringify(GROK_EDITOR_SELECTOR)});
        if (!editor) return false;
        editor.focus();
        document.execCommand('selectAll', false, null);
        document.execCommand('insertText', false, ${JSON.stringify(PROMPT)});
        editor.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      })()
    `).catch(() => false);
    if (!filled) {
      await emit({ event: 'error', message: 'Grok prompt input not found' });
      return;
    }
    if (!(await clickGrokSubmit(6000))) {
      await emit({ event: 'error', message: 'Grok send button not available' });
      return;
    }
  }

  const deadline = Date.now() + 300000;
  let lastHeartbeat = Date.now();
  while (Date.now() < deadline && !STREAM_ABORTED) {
    await ensureTaskSpaceControl();
    const batch = await js(`
      (() => {
        const s = window.__lxGrokStream;
        if (!s || s.runId !== ${JSON.stringify(runId)}) {
          return { items: [], done: true, superseded: true };
        }
        s.pollDom();
        return { items: s.queue.splice(0, s.queue.length), done: s.done, superseded: false };
      })()
    `).catch(() => ({ items: [], done: false, superseded: false }));

    if (batch && batch.superseded) {
      return;
    }

    if (batch && Array.isArray(batch.items) && batch.items.length > 0) {
      lastHeartbeat = Date.now();
      const events = [];
      for (const item of batch.items) {
        if (item.kind === 'text' && item.delta) {
          events.push({ event: 'text', delta: item.delta });
        }
      }
      if (events.length > 0) {
        await emitMany(events);
      }
    } else if (Date.now() - lastHeartbeat >= 1000) {
      lastHeartbeat = Date.now();
      if (!(await checkStreamAlive())) {
        return;
      }
    }

    if (batch && batch.done) {
      // 会话 URL 与侧栏标题可能晚于正文完成，短暂重试后回投。
      let sessionMeta = null;
      const metaDeadline = Date.now() + 1800;
      while (Date.now() < metaDeadline) {
        sessionMeta = await js(String.raw`
          (() => {
            const m = location.pathname.match(/\/c\/([0-9a-f-]{8,})/);
            if (!m) return null;
            const id = m[1];
            const link = Array.from(document.querySelectorAll('a[href^="/c/"]'))
              .find((a) => (a.getAttribute('href') || '').startsWith('/c/' + id));
            let title = link ? (link.innerText || '').trim().replace(/\s+/g, ' ') : '';
            if (!title) {
              title = (document.title || '').replace(/\s*-\s*Grok$/i, '').trim();
            }
            return { id, title };
          })()
        `).catch(() => null);
        if (sessionMeta && sessionMeta.id && sessionMeta.title) {
          break;
        }
        await new Promise((r) => setTimeout(r, 250));
      }
      if (sessionMeta && sessionMeta.id) {
        await emit({
          event: 'session',
          id: sessionMeta.id,
          title: sessionMeta.title || null,
        });
      }
      await emit({ event: 'done' });
      return;
    }

    await new Promise((r) => setTimeout(r, 60));
  }

  if (STREAM_ABORTED) return;
  await emit({ event: 'error', message: 'Grok stream timed out' });
}

try {
  await main();
} catch (error) {
  const raw = String((error && error.message) || error || 'Grok web driver error');
  const firstLine = raw.split('\n').map((s) => s.trim()).find(Boolean) || raw;
  await emit({ event: 'error', message: firstLine });
}
