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

async function grokCounts() {
  return await js(`
    (() => ({
      users: document.querySelectorAll('[data-testid="user-message"]').length,
      assistants: document.querySelectorAll('[data-testid="assistant-message"]').length,
    }))()
  `).catch(() => null);
}

async function grokActiveStop() {
  return await js(`
    (() => Array.from(document.querySelectorAll(${JSON.stringify(GROK_STOP_SELECTOR)}))
      .some((el) => !el.disabled && el.getAttribute('aria-disabled') !== 'true'))()
  `).catch(() => false);
}

async function waitGrokSubmitted(before) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    const counts = await grokCounts();
    if (
      counts &&
      before &&
      (counts.users > before.users || counts.assistants > before.assistants)
    ) {
      return true;
    }
    if (await grokActiveStop()) return true;
    await new Promise((r) => setTimeout(r, 100));
  }
  return false;
}

function normalizeGrokText(value) {
  return String(value || '')
    .replace(/\s+/g, ' ')
    .trim();
}

async function fillGrokPrompt() {
  const filled = await js(`
    (() => {
      const editor = document.querySelector(${JSON.stringify(GROK_EDITOR_SELECTOR)});
      if (!editor) return null;
      editor.focus();
      document.execCommand('selectAll', false, null);
      const prompt = ${JSON.stringify(PROMPT)};
      if (prompt) {
        document.execCommand('insertText', false, prompt);
      } else {
        // 纯附件消息必须清空旧草稿，不能把用户上一次输入一起提交。
        document.execCommand('delete', false, null);
      }
      editor.dispatchEvent(new Event('input', { bubbles: true }));
      return { text: editor.innerText || '' };
    })()
  `).catch(() => null);
  if (!filled) return false;
  const expected = normalizeGrokText(PROMPT);
  const actual = normalizeGrokText(filled.text);
  if (expected) {
    return actual.includes(expected.slice(0, Math.min(24, expected.length)));
  }
  return actual.length === 0;
}

async function submitGrokPrompt(timeoutMs) {
  const before = await grokCounts();
  if (!(await clickGrokSubmit(timeoutMs))) return false;
  return await waitGrokSubmitted(before);
}

async function main() {
  await activateTaskSpace();
  const current = await pageInfo().catch(() => null);
  const currentUrl = (current && current.url) || '';
  let onGrok = false;
  try {
    const parsed = new URL(currentUrl);
    onGrok = parsed.protocol === 'https:' && parsed.hostname === 'grok.com';
  } catch (_) {}
  if (!onGrok) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
  }
  await dismissCookieBanner();
  await installPermissionGuard();

  // 登录态由 API 判定，不依赖页面是否渲染完成；登录后再等 composer，避免
  // 把“页面仍在冷启动”误判成“未登录”，也避免把登录页里的普通 sign-in 链接当成登录墙。
  let loginState = { ok: false, status: -1 };
  const loginDeadline = Date.now() + 20000;
  while (Date.now() < loginDeadline) {
    loginState = await grokLoginState();
    if (loginState.ok) break;
    // 401 或 200 且无 email 是明确的未登录；5xx/网络错误继续重试到 deadline。
    if (loginState.status === 401 || loginState.status === 200) break;
    await new Promise((r) => setTimeout(r, 300));
  }
  if (!loginState.ok) {
    if (loginState.status === 401 || loginState.status === 200) {
      await emitLoginRequired();
    } else {
      await emit({
        event: 'error',
        message: 'Grok auth check failed (' + httpDetail(loginState) + ')',
      });
    }
    return;
  }

  // 非 chat 动作只用 REST/页面导航，不需要 ProseMirror；先分派，避免冷启动
  // composer 等待占满 CLI 的 60s 预算。
  if (ACTION !== 'chat') {
    await runNonChatAction(currentUrl);
    return;
  }

  // 冷启动/慢网络：ProseMirror 可能晚于登录态挂载。
  if (!(await ensureGrokComposer())) {
    await emit({ event: 'error', message: 'Grok composer not found' });
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
  await installDomMarkdown();
  await cdp('Network.enable').catch(() => {});
  const runId = String(Date.now()) + '-' + String(Math.random());
  await js(String.raw`
    (() => {
      // 页面上下文执行：不能引用 Node 侧函数。
      const stripGrokDirectives = (text, streaming) => {
        const cleaned = String(text || '')
          .replace(/<grok:render\b[^>]*>[\s\S]*?<\/grok:render>/gi, '')
          .replace(/<grok:render\b[^>]*\/>/gi, '')
          .replace(/<argument\b[^>]*>[\s\S]*?<\/argument>/gi, '')
          .replace(/<argument\b[^>]*>?/gi, '')
          .replace(/<\/argument>/gi, '')
          .replace(/<grok:render\b[^>]*>/gi, '')
          .replace(/<\/grok:render>/gi, '')
          .replace(/<\/?grok:[^>]*>?/gi, '')
          .replace(/<grok:render\b[^>]*$/gi, '')
          .replace(/\u2060/g, ' ')
          .replace(/^\n+/, '')
          .replace(/\n{3,}/g, '\n\n');
        return streaming ? cleaned : cleaned.trim();
      };
      const existingResponses = Array.from(document.querySelectorAll('[data-testid="assistant-message"]'));
      const baselineAssistants = existingResponses.length;
      window.__lxGrokStream = {
        runId: ${JSON.stringify(runId)},
        queue: [],
        done: false,
        error: null,
        started: false,
        wsSeen: false,
        wsActive: false,
        domStarted: false,
        pollStartedAt: 0,
        baselineAssistants,
        emptySince: 0,
        lastText: '',
        imageUrls: [],
        lastSample: '',
        stableTicks: 0,
        extractText(el, generating) {
          if (!el) return '';
          // 只读正文容器。助手消息头部有一个 sr-only 的 "Grok" 标题，
          // 正文尚未挂载时回退会误把标题当成回答输出。
          const body = el.querySelector('.response-content-markdown');
          if (!body) return '';
          const isStreaming = Boolean(generating);
          if (typeof window.__lxHtmlToMarkdown === 'function') {
            const raw = window.__lxHtmlToMarkdown(body, {
              streaming: isStreaming,
            });
            return stripGrokDirectives(raw, isStreaming);
          }
          const raw = (body.textContent || '').replace(/\r\n/g, '\n');
          return stripGrokDirectives(raw, isStreaming);
        },
        pushText(current) {
          if (!current || this.wsActive) return;
          const previous = this.lastText || '';
          if (current === previous) return;
          const emitDelta = (delta) => {
            if (!delta) return;
            this.started = true;
            this.domStarted = true;
            this.queue.push({ kind: 'text', delta });
          };
          if (!previous) {
            this.lastText = current;
            emitDelta(current);
            return;
          }
          if (current.startsWith(previous)) {
            const delta = current.slice(previous.length);
            this.lastText = current;
            emitDelta(delta);
            return;
          }
          // streamdown 重渲染：用尾部锚点在 current 中重新对齐，避免重复输出已流过的段落。
          for (const len of [48, 32, 20, 12, 8, 5]) {
            if (previous.length < len) continue;
            const anchor = previous.slice(previous.length - len);
            const pos = current.lastIndexOf(anchor);
            if (pos >= 0 && pos + len >= current.length - 1024) {
              const delta = current.slice(pos + len);
              if (delta.length > 0 || current.length >= previous.length) {
                this.lastText = current;
              }
              emitDelta(delta);
              return;
            }
          }
          let lcp = 0;
          while (lcp < current.length && lcp < previous.length && current[lcp] === previous[lcp]) {
            lcp += 1;
          }
          if (previous.length - lcp <= 160) {
            const remPrev = previous.slice(lcp).replace(/^[*_~\s\x60]+/, '');
            if (remPrev.length === 0) {
              const delta = current.slice(lcp);
              this.lastText = current;
              emitDelta(delta);
              return;
            }
            const idx = current.indexOf(remPrev, lcp);
            if (idx >= lcp && idx - lcp <= 24) {
              const delta = current.slice(idx + remPrev.length);
              this.lastText = current;
              emitDelta(delta);
              return;
            }
          }
          // 尾部收缩（例如引用角标重新排版）时不回退 lastText，等重渲染追平。
          if (current.length > previous.length) this.lastText = current;
        },
        pollDom() {
          const now = Date.now();
          if (!this.pollStartedAt) this.pollStartedAt = now;
          const messages = Array.from(document.querySelectorAll('[data-testid="assistant-message"]'));
          // 只认发送后新增的 assistant；历史晚挂载时 baseline 不变，旧回答不会被当成本轮输出。
          if (messages.length <= this.baselineAssistants) {
            return;
          }
          const target = messages[messages.length - 1];
          const stops = Array.from(document.querySelectorAll(${JSON.stringify(GROK_STOP_SELECTOR)}))
            .filter((el) => !el.disabled && el.getAttribute('aria-disabled') !== 'true');
          const generating = stops.length > 0;
          if (generating) this.started = true;
          const text = this.extractText(target, generating);
          const imageUrls = Array.from(
            new Set(
              Array.from(
                target.querySelectorAll(
                  'img[src*="/generated/"], .group\\/grok-image img, [data-testid] .group\\/image img'
                )
              )
                .map((img) => (img.currentSrc || img.src || '').trim())
                .filter((u) => /^https?:\/\//i.test(u))
            )
          );
          this.imageUrls = imageUrls;
          if (text) {
            this.emptySince = 0;
            const graceOver = now - this.pollStartedAt >= 1200 || !generating;
            if (!this.wsActive && (!this.wsSeen || !generating) && graceOver) {
              this.pushText(text);
            } else if (this.wsActive) {
              this.lastText = text;
            }
          } else if (imageUrls.length > 0) {
            this.emptySince = 0;
            this.started = true;
          } else if (!this.emptySince) {
            this.emptySince = Date.now();
          }

          // 新 assistant 已出现但一直没有正文或生成图（Build 画布/错误横幅/限流）：
          // 生成结束后 15s 仍为空就显式失败，不能空等 300s。
          // 文件上传/慢网络会先挂一个空占位节点，5s 会误杀正常回答。
          if (
            !text &&
            imageUrls.length === 0 &&
            !generating &&
            this.emptySince &&
            Date.now() - this.emptySince >= 15000
          ) {
            this.error = 'Grok response has no text';
            this.done = true;
            return;
          }

          const sample = text + '\n---\n' + imageUrls.join('\n');
          const hasOutput = text.length > 0 || imageUrls.length > 0;
          if ((this.started || hasOutput) && !generating) {
            if (sample === this.lastSample && hasOutput) {
              this.stableTicks += 1;
              // 2s 稳定窗口，覆盖 Build/联网引用重排，避免 600ms 就截断。
              if (this.stableTicks >= 30) this.done = true;
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
    if (FILES.length > 0 || KEEP_FILES.length > 0) {
      await emit({
        event: 'error',
        message: 'Grok /rewind cannot restore attachments; use /new or send without files',
      });
      return;
    }
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
    if (!(await fillGrokPrompt())) {
      await emit({ event: 'error', message: 'Grok edit composer not found or prompt not applied' });
      return;
    }
    await new Promise((r) => setTimeout(r, 300));
    if (!(await submitGrokPrompt(4000))) {
      await emit({ event: 'error', message: 'Failed to submit edited Grok prompt' });
      return;
    }
  } else {
    // 普通发送：清理输入区残留附件，再注入本轮文件；/rewind 明确不支持附件。
    await syncComposerAttachments();
    if (!(await fillGrokPrompt())) {
      await emit({ event: 'error', message: 'Grok prompt input not found or not applied' });
      return;
    }
    if (!(await submitGrokPrompt(6000))) {
      await emit({ event: 'error', message: 'Grok send button not available or message not submitted' });
      return;
    }
  }

  if (typeof drainEvents === 'function') {
    await drainEvents().catch(() => []);
  }
  let wsRawBuffer = '';
  let wsEmittedCleaned = '';
  let wsSeen = false;
  let wsDoneSeen = false;
  let wsAfterToolCard = false;
  let wsLastToolStep = -1;
  const cleanGrokWsBuffer = (raw, holdIncomplete) => {
    let out = String(raw || '')
      .replace(/<grok:render\b[^>]*>[\s\S]*?<\/grok:render>/gi, '')
      .replace(/<grok:render\b[^>]*\/>/gi, '')
      .replace(/<argument\b[^>]*>[\s\S]*?<\/argument>/gi, '')
      .replace(/\u2060/g, ' ');
    if (holdIncomplete) {
      const lastOpen = out.lastIndexOf('<');
      if (lastOpen >= 0) {
        const tail = out.slice(lastOpen).toLowerCase();
        if (
          '<grok:'.startsWith(tail) ||
          '</grok:'.startsWith(tail) ||
          '<argument'.startsWith(tail) ||
          '</argument'.startsWith(tail) ||
          tail.startsWith('<grok:') ||
          tail.startsWith('</grok:') ||
          tail.startsWith('<argument') ||
          tail.startsWith('</argument')
        ) {
          out = out.slice(0, lastOpen);
        }
      }
    } else {
      out = out
        .replace(/<argument\b[^>]*>?/gi, '')
        .replace(/<\/argument>/gi, '')
        .replace(/<\/?grok:[^>]*>?/gi, '');
    }
    return out;
  };

  const deadline = Date.now() + 300000;
  let lastHeartbeat = Date.now();
  let pollFailures = 0;
  while (Date.now() < deadline && !STREAM_ABORTED) {
    await ensureTaskSpaceControl();
    let wsDelta = '';
    if (typeof drainEvents === 'function') {
      const cdpEvents = await drainEvents().catch(() => []);
      if (Array.isArray(cdpEvents)) {
        for (const ev of cdpEvents) {
          if (!ev || ev.method !== 'Network.webSocketFrameReceived') continue;
          const payload =
            ev.params && ev.params.response && ev.params.response.payloadData;
          if (
            typeof payload !== 'string' ||
            (!payload.includes('"response.chunk"') &&
              !payload.includes('"response.done"') &&
              !payload.includes('"response.created"'))
          ) {
            continue;
          }
          try {
            const parsed = JSON.parse(payload);
            const evObj = parsed && parsed.event;
            if (!evObj) continue;
            if (evObj.type === 'response.created' || evObj.type === 'response.chunk') {
              wsSeen = true;
            }
            if (evObj.type === 'response.done') {
              wsDoneSeen = true;
              continue;
            }
            if (evObj.type !== 'response.chunk' || !evObj.chunk) continue;
            const chunk = evObj.chunk;
            const stepId =
              chunk.metadata && typeof chunk.metadata.step_id === 'number'
                ? chunk.metadata.step_id
                : 0;
            if (chunk.tool_usage_card) {
              if (stepId > wsLastToolStep) wsLastToolStep = stepId;
              if (wsRawBuffer.length > 0) wsAfterToolCard = true;
              continue;
            }
            if (stepId < wsLastToolStep) continue;
            if (
              chunk.text &&
              chunk.text.channel === 'CHANNEL_ASSISTANT_RESPONSE' &&
              typeof chunk.text.text === 'string' &&
              chunk.text.text
            ) {
              if (wsAfterToolCard) {
                if (wsRawBuffer && !wsRawBuffer.endsWith('\n\n')) {
                  wsRawBuffer += wsRawBuffer.endsWith('\n') ? '\n' : '\n\n';
                }
                wsAfterToolCard = false;
              }
              wsRawBuffer += chunk.text.text;
            } else if (
              chunk.render_citation &&
              typeof chunk.render_citation.url === 'string' &&
              chunk.render_citation.url
            ) {
              const citeUrl = chunk.render_citation.url.trim();
              if (/^https?:\/\//i.test(citeUrl)) {
                const label = citationSourceLabel(citeUrl);
                const md = citationMarkdown(citeUrl, label);
                if (md) {
                  wsRawBuffer += (wsRawBuffer && !/\s$/.test(wsRawBuffer) ? ' ' : '') + md;
                }
              }
            }
          } catch (_) {}
        }
      }
      if (wsRawBuffer) {
        const cleanedWs = cleanGrokWsBuffer(wsRawBuffer, !wsDoneSeen);
        if (cleanedWs.startsWith(wsEmittedCleaned)) {
          wsDelta = cleanedWs.slice(wsEmittedCleaned.length);
          wsEmittedCleaned = cleanedWs;
        } else {
          let lcp = 0;
          while (
            lcp < cleanedWs.length &&
            lcp < wsEmittedCleaned.length &&
            cleanedWs[lcp] === wsEmittedCleaned[lcp]
          ) {
            lcp += 1;
          }
          wsDelta = cleanedWs.slice(lcp);
          wsEmittedCleaned = cleanedWs;
        }
      }
    }
    const batch = await js(`
      (() => {
        const s = window.__lxGrokStream;
        if (!s || s.runId !== ${JSON.stringify(runId)}) {
          return { items: [], done: true, superseded: true };
        }
        if (${wsSeen ? 'true' : 'false'}) {
          s.wsSeen = true;
        }
        const wsChunk = ${JSON.stringify(wsDelta)};
        if (wsChunk && !s.domStarted) {
          s.wsActive = true;
          s.started = true;
          s.emptySince = 0;
          s.queue.push({ kind: 'text', delta: wsChunk });
        }
        s.pollDom();
        return {
          items: s.queue.splice(0, s.queue.length),
          done: s.done,
          error: s.error || null,
          imageUrls: s.imageUrls || [],
          turnNum: (s.baselineAssistants || 0) + 1,
          hadText: Boolean((s.lastText && s.lastText.trim()) || s.wsActive),
          superseded: false,
        };
      })()
    `).catch(() => null);

    if (!batch) {
      pollFailures += 1;
      if (pollFailures >= 10) {
        await emit({ event: 'error', message: 'Grok stream collector unavailable' });
        return;
      }
      await new Promise((r) => setTimeout(r, 100));
      continue;
    }
    pollFailures = 0;

    if (batch.superseded) {
      await emit({ event: 'error', message: 'Grok stream was superseded by another pane' });
      return;
    }

    if (batch.error) {
      await emit({ event: 'error', message: String(batch.error) });
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
      // 会话 URL 与侧栏标题可能晚于正文完成，慢网络下多等一会儿再回投。
      let sessionMeta = null;
      const metaDeadline = Date.now() + 5000;
      while (Date.now() < metaDeadline) {
        sessionMeta = await js(String.raw`
          (() => {
            const m = location.pathname.match(/\/c\/([0-9a-f-]{8,})/);
            if (!m) return null;
            const id = m[1];
            const link = Array.from(document.querySelectorAll('a[href^="/c/"]'))
              .find((a) => (a.getAttribute('href') || '').startsWith('/c/' + id));
            let title = link ? (link.innerText || '').trim().replace(/\s+/g, ' ') : '';
            if (/^new chat$/i.test(title)) title = '';
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
      const resolvedSid = (sessionMeta && sessionMeta.id) || SESSION_ID || 'temp';
      if (Array.isArray(batch.imageUrls) && batch.imageUrls.length > 0) {
        const saved = await saveGeneratedImages(
          'grok',
          'ai-web-grok-',
          resolvedSid,
          batch.turnNum || 1,
          batch.imageUrls,
          true
        );
        if (!batch.hadText && saved.length > 0) {
          await emit({
            event: 'text',
            delta:
              saved.length === 1
                ? 'Generated image'
                : 'Generated ' + saved.length + ' images',
          });
        }
        for (const img of saved) {
          await emit({
            event: 'image',
            label: img.label,
            path: img.path || '',
            original_url: img.original_url || '',
            preview_url: img.preview_url || '',
            file_stem: img.file_stem || '',
          });
        }
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
