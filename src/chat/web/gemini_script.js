// Google Gemini web chat stream driver for `ego-browser nodejs`.
//
// Requires `gemini_helpers.js` to be prepended ahead of this file.

async function main() {
  await activateTaskSpace();
  const current = await pageInfo().catch(() => null);
  const currentUrl = (current && current.url) || '';
  if (!currentUrl.startsWith('https://gemini.google.com')) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
  }
  await installPermissionGuard();

  let loggedIn = false;
  const loginDeadline = Date.now() + 8000;
  while (Date.now() < loginDeadline) {
    const state = await js(String.raw`
      (() => {
        if (location.hostname !== 'gemini.google.com') return { ready: false, signIn: false };
        const signInLink = Boolean(
          document.querySelector('a[href*="accounts.google.com/ServiceLogin"], a[aria-label*="登录"], a[aria-label*="Sign in"]')
        );
        const hasComposer = Boolean(
          document.querySelector('.ql-editor[contenteditable="true"], rich-textarea, [data-test-id="bard-mode-menu-button"]')
        );
        return { ready: !signInLink && hasComposer, signIn: signInLink && !hasComposer };
      })()
    `).catch(() => ({ ready: false, signIn: false }));
    if (state && state.ready) {
      loggedIn = true;
      break;
    }
    if (state && state.signIn) {
      break;
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  if (!loggedIn) {
    await handOffTaskSpace();
    await emit({ event: 'login_required', url: HOME });
    return;
  }

  if (ACTION !== 'chat') {
    await runNonChatAction(currentUrl);
    return;
  }

  if (FIRST_TURN && EDIT_INDEX < 0) {
    await ensureFreshChat();
  } else if (SESSION_ID && !currentUrl.includes(SESSION_ID)) {
    await openOrReuseTab(HOME + '/' + SESSION_ID, { wait: true, timeout: 25 });
    await installPermissionGuard();
  }

  // Synchronize selected Gemini model and thinking level in the web UI.
  await ensureGeminiModelAndThinking();

  // Set up DOM stream collector on window.__lxGeminiStream.
  const runId = String(Date.now()) + '-' + String(Math.random());
  await js(`
    (() => {
      const existingResponses = Array.from(document.querySelectorAll('model-response'));
      const targetIndex = ${
        EDIT_INDEX >= 0
          ? EDIT_INDEX
          : FIRST_TURN
            ? 0
            : 'existingResponses.length'
      };
      const initialMr = ${EDIT_INDEX >= 0 ? 'existingResponses[targetIndex]' : 'null'};
      window.__lxGeminiStream = {
        runId: ${JSON.stringify(runId)},
        queue: [],
        done: false,
        started: false,
        isEdit: ${EDIT_INDEX >= 0 ? 'true' : 'false'},
        editResetSeen: false,
        initialThinkText: '',
        initialTargetText: '',
        targetIndex,
        domThink: '',
        domText: '',
        lastSample: '',
        stableTicks: 0,
        pushDom(kind, current) {
          const key = kind === 'thinking' ? 'domThink' : 'domText';
          const previous = this[key] || '';
          if (!current || current === previous) return;
          if (kind === 'thinking') {
            let delta = '';
            if (!previous) {
              delta = current;
            } else if (current.length > previous.length && current.startsWith(previous)) {
              delta = current.slice(previous.length);
            } else if (!previous.includes(current)) {
              delta = '\\n' + current;
            }
            this[key] = current;
            if (delta) {
              this.started = true;
              this.queue.push({ kind, delta });
            }
            return;
          }
          if (!previous) {
            this[key] = current;
            this.started = true;
            this.queue.push({ kind, delta: current });
            return;
          }
          if (current.startsWith(previous)) {
            const delta = current.slice(previous.length);
            this[key] = current;
            if (delta) {
              this.started = true;
              this.queue.push({ kind, delta });
            }
            return;
          }
          // Align the tail of previous near the end of current so mid-document
          // citation animations do not re-emit already streamed paragraphs.
          for (const len of [48, 32, 20, 12, 8, 5]) {
            if (previous.length < len) continue;
            const anchor = previous.slice(previous.length - len);
            const pos = current.lastIndexOf(anchor);
            if (pos >= 0 && pos + len >= current.length - 240) {
              const delta = current.slice(pos + len);
              if (delta.length > 0 || current.length >= previous.length) {
                this[key] = current;
              }
              if (delta) {
                this.started = true;
                this.queue.push({ kind, delta });
              }
              return;
            }
          }
          // If the tail of current was already emitted in previous (e.g. the final
          // paragraph temporarily shrank while attaching citations), emit nothing and
          // preserve previous until the re-animation catches up.
          const tailLen = Math.min(20, current.length);
          if (tailLen >= 5 && previous.includes(current.slice(current.length - tailLen))) {
            return;
          }
          // Handle inline markdown delimiters stripped near the streaming cursor.
          let lcp = 0;
          while (lcp < current.length && lcp < previous.length && current[lcp] === previous[lcp]) {
            lcp += 1;
          }
          if (previous.length - lcp <= 40) {
            const remPrev = previous.slice(lcp).replace(/^[*_~\\s\\x60]+/, '');
            if (remPrev.length > 0) {
              const idx = current.indexOf(remPrev, lcp);
              if (idx >= lcp && idx - lcp <= 10) {
                const delta = current.slice(idx + remPrev.length);
                this[key] = current;
                if (delta) {
                  this.started = true;
                  this.queue.push({ kind, delta });
                }
                return;
              }
            }
          }
          if (current.length > previous.length) {
            this[key] = current;
          }
        },
        extractCleanText(root, selectorList, stripSelectors) {
          if (!root) return '';
          const el = selectorList ? root.querySelector(selectorList) : root;
          if (!el) return '';
          const hidden = [];
          if (stripSelectors) {
            el.querySelectorAll(stripSelectors).forEach((n) => {
              hidden.push([n, n.style.display]);
              n.style.display = 'none';
            });
          }
          const tempCodeNodes = [];
          if (selectorList && selectorList.includes('message-content')) {
            const bt = String.fromCharCode(96);
            el.querySelectorAll('code').forEach((c) => {
              if (!c.closest('pre, code-block')) {
                const open = document.createTextNode(bt);
                const close = document.createTextNode(bt);
                c.insertBefore(open, c.firstChild);
                c.appendChild(close);
                tempCodeNodes.push(open, close);
              }
            });
          }
          const text = (el.innerText || '').replace(/\\r\\n/g, '\\n').trim();
          for (const tn of tempCodeNodes) {
            tn.remove();
          }
          for (const [n, prev] of hidden) {
            n.style.display = prev;
          }
          return text;
        },
        pollDom() {
          const responses = Array.from(document.querySelectorAll('model-response'));
          const mr = responses[this.targetIndex];
          const stopBtn = document.querySelector(
            'button[aria-label*="停止"], button[aria-label*="Stop"], button.stop-button, .blue-circle.stop-icon'
          );
          const announcerBusy = mr
            ? Boolean(mr.querySelector('[aria-busy="true"]'))
            : false;
          const isGenerating = Boolean(stopBtn) || announcerBusy;
          if (this.isEdit && !this.editResetSeen) {
            if (!mr) {
              this.editResetSeen = true;
              return;
            }
            const curThink = this.extractCleanText(
              mr,
              'model-thoughts .thoughts-content, model-thoughts, thinking-overlay .thoughts-content, thinking-overlay',
              'button, mat-icon, .cdk-visually-hidden'
            );
            const curBody = this.extractCleanText(
              mr,
              'message-content .markdown, message-content',
              '.cdk-visually-hidden, source-footnote, sources-carousel-inline, sources-list, button, mat-icon'
            );
            if (
              announcerBusy ||
              curBody !== this.initialTargetText ||
              curThink !== this.initialThinkText
            ) {
              this.editResetSeen = true;
            } else {
              return;
            }
          }
          if (isGenerating) {
            this.started = true;
          }
          if (!mr) {
            return;
          }
          const thinkText = this.extractCleanText(
            mr,
            'model-thoughts .thoughts-content, model-thoughts, thinking-overlay .thoughts-content, thinking-overlay',
            'button, mat-icon, .cdk-visually-hidden'
          );
          const bodyText = this.extractCleanText(
            mr,
            'message-content .markdown, message-content',
            '.cdk-visually-hidden, source-footnote, sources-carousel-inline, sources-list, button, mat-icon'
          );
          if (thinkText) this.pushDom('thinking', thinkText);
          if (bodyText) this.pushDom('text', bodyText);

          const sample = thinkText + '\\n---\\n' + bodyText;
          const caughtUp = bodyText.length >= (this.domText || '').length;
          if ((this.started || bodyText.length > 0 || thinkText.length > 0) && !isGenerating && caughtUp) {
            if (sample === this.lastSample && (bodyText.length > 0 || thinkText.length > 0)) {
              this.stableTicks += 1;
              if (this.stableTicks >= 10) {
                this.done = true;
              }
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
      if (initialMr) {
        window.__lxGeminiStream.initialThinkText = window.__lxGeminiStream.extractCleanText(
          initialMr,
          'model-thoughts .thoughts-content, model-thoughts, thinking-overlay .thoughts-content, thinking-overlay',
          'button, mat-icon, .cdk-visually-hidden'
        );
        window.__lxGeminiStream.initialTargetText = window.__lxGeminiStream.extractCleanText(
          initialMr,
          'message-content .markdown, message-content',
          '.cdk-visually-hidden, source-footnote, sources-carousel-inline, sources-list, button, mat-icon'
        );
      }
      return true;
    })()
  `);

  // Submit either via inline edit (/rewind) or normal composer input.
  await ensureTaskSpaceControl();
  let submittedViaEdit = false;
  if (EDIT_INDEX >= 0) {
    const editCheck = await js(`
      (() => {
        const queries = Array.from(document.querySelectorAll('user-query'));
        const target = queries[${EDIT_INDEX}];
        if (!target) return { status: 'missing_turn' };
        const existingFiles = target.querySelectorAll(
          '.file-preview-container button, .file-preview-container a, gem-media-attachment, gem-attachment'
        ).length;
        const wantedFiles = ${FILES.length + KEEP_FILES.length};
        if (${EDIT_INDEX === 0 ? 'true' : 'false'} && existingFiles !== wantedFiles) {
          return { status: 'attachments_changed_turn0' };
        }
        target.scrollIntoView({ block: 'center' });
        const editBtn = target.querySelector(
          'button[aria-label="修改"], button[aria-label*="修改"], button[aria-label*="Edit"]'
        );
        if (!editBtn) return { status: 'no_button' };
        editBtn.click();
        return { status: 'clicked' };
      })()
    `).catch(() => ({ status: 'error' }));

    if (editCheck && editCheck.status === 'clicked') {
      await new Promise((r) => setTimeout(r, 350));
      const filledEdit = await js(`
        (() => {
          const queries = Array.from(document.querySelectorAll('user-query'));
          const target = queries[${EDIT_INDEX}] || document;
          const desired = ${JSON.stringify(PROMPT)};
          const editable = target.querySelector('.ql-editor[contenteditable="true"], [contenteditable="true"]');
          if (editable) {
            const currentText = (editable.innerText || '').replace(/\\u200b/g, '').trim();
            const nextText = currentText === desired.trim() ? desired + '\\u200b' : desired;
            editable.focus();
            document.execCommand('selectAll', false, null);
            document.execCommand('insertText', false, nextText);
            editable.dispatchEvent(new Event('input', { bubbles: true }));
            return true;
          }
          const textarea = target.querySelector('textarea') || document.querySelector('user-query textarea');
          if (textarea) {
            const currentText = (textarea.value || '').replace(/\\u200b/g, '').trim();
            const nextText = currentText === desired.trim() ? desired + '\\u200b' : desired;
            textarea.focus();
            const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value')?.set;
            if (setter) setter.call(textarea, nextText);
            else textarea.value = nextText;
            textarea.dispatchEvent(new Event('input', { bubbles: true }));
            return true;
          }
          return false;
        })()
      `).catch(() => false);

      if (filledEdit) {
        const updateDeadline = Date.now() + 4000;
        while (Date.now() < updateDeadline) {
          await new Promise((r) => setTimeout(r, 150));
          const clickedUpdate = await js(`
            (() => {
              const queries = Array.from(document.querySelectorAll('user-query'));
              const target = queries[${EDIT_INDEX}] || document;
              const buttons = Array.from(target.querySelectorAll('button'));
              const submitBtn = buttons.find((b) => {
                const t = (b.innerText || b.getAttribute('aria-label') || '').trim().toLowerCase();
                return t === '更新' || t === 'update' || t.includes('更新') || t.includes('update');
              });
              if (!submitBtn || submitBtn.disabled || submitBtn.getAttribute('aria-disabled') === 'true') {
                return false;
              }
              submitBtn.click();
              return true;
            })()
          `).catch(() => false);
          if (clickedUpdate) {
            submittedViaEdit = true;
            break;
          }
        }
      }
      if (!submittedViaEdit) {
        await emit({ event: 'error', message: 'Failed to submit edited Gemini prompt' });
        return;
      }
    } else if (editCheck && editCheck.status === 'attachments_changed_turn0') {
      await ensureFreshChat();
      await js(`
        (() => {
          if (window.__lxGeminiStream) {
            window.__lxGeminiStream.isEdit = false;
            window.__lxGeminiStream.targetIndex = 0;
          }
        })()
      `).catch(() => {});
    } else if (editCheck && editCheck.status === 'missing_turn') {
      if (EDIT_INDEX === 0) {
        await ensureFreshChat();
      }
      await js(`
        (() => {
          if (window.__lxGeminiStream) {
            const existing = document.querySelectorAll('model-response').length;
            window.__lxGeminiStream.isEdit = false;
            window.__lxGeminiStream.targetIndex = ${EDIT_INDEX === 0 ? 0 : 'existing'};
          }
        })()
      `).catch(() => {});
    } else {
      await emit({ event: 'error', message: 'Gemini edit button not found for /rewind' });
      return;
    }
  }
  if (!submittedViaEdit) {
    await syncContainerAttachments(false);
    const filled = await js(`
      (() => {
        const editor = document.querySelector('.ql-editor[contenteditable="true"], rich-textarea [contenteditable="true"]');
        if (editor) {
          editor.focus();
          document.execCommand('selectAll', false, null);
          if (${JSON.stringify(PROMPT)}) {
            document.execCommand('insertText', false, ${JSON.stringify(PROMPT)});
          } else {
            document.execCommand('delete', false, null);
          }
          editor.dispatchEvent(new Event('input', { bubbles: true }));
          return true;
        }
        const textarea = document.querySelector('textarea');
        if (textarea) {
          textarea.focus();
          const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value')?.set;
          if (setter) setter.call(textarea, ${JSON.stringify(PROMPT)});
          else textarea.value = ${JSON.stringify(PROMPT)};
          textarea.dispatchEvent(new Event('input', { bubbles: true }));
          return true;
        }
        return false;
      })()
    `).catch(() => false);
    if (!filled) {
      await emit({ event: 'error', message: 'Gemini prompt input not found' });
      return;
    }
    const sendDeadline = Date.now() + 6000;
    let sent = false;
    while (Date.now() < sendDeadline) {
      const attempt = await js(String.raw`
        (() => {
          const sendBtn = Array.from(
            document.querySelectorAll('gem-icon-button.send-button button, button.send-button, button[aria-label*="发送"], button[aria-label*="Send"]')
          ).find((b) => !b.disabled && b.getAttribute('aria-disabled') !== 'true');
          if (sendBtn) {
            sendBtn.click();
            return true;
          }
          return false;
        })()
      `).catch(() => false);
      await new Promise((r) => setTimeout(r, 200));
      const cleared = await js(String.raw`
        (() => {
          const editor = document.querySelector('.ql-editor[contenteditable="true"], rich-textarea [contenteditable="true"]');
          const textEmpty = !editor || editor.classList.contains('ql-blank') || !(editor.innerText || '').trim();
          const chips = document.querySelectorAll('uploader-file-preview').length;
          const stopBtn = Boolean(
            document.querySelector('button[aria-label*="停止"], button[aria-label*="Stop"], button.stop-button, .blue-circle.stop-icon')
          );
          return stopBtn || (textEmpty && chips === 0);
        })()
      `).catch(() => false);
      if (cleared) {
        sent = true;
        break;
      }
      if (attempt) {
        sent = true;
      }
    }
    if (!sent) {
      await emit({ event: 'error', message: 'Gemini send button not available' });
      return;
    }
  }

  const deadline = Date.now() + 300000;
  let lastHeartbeat = Date.now();
  while (Date.now() < deadline && !STREAM_ABORTED) {
    await ensureTaskSpaceControl();
    const batch = await js(`
      (() => {
        const s = window.__lxGeminiStream;
        if (!s || s.runId !== ${JSON.stringify(runId)}) return { items: [], done: true, superseded: true };
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
        if (item.kind === 'thinking' && item.delta) {
          events.push({ event: 'thinking', delta: item.delta });
        } else if (item.kind === 'text' && item.delta) {
          events.push({ event: 'text', delta: item.delta });
        }
      }
      await emitMany(events);
    } else if (Date.now() - lastHeartbeat >= 1000) {
      lastHeartbeat = Date.now();
      if (!(await checkStreamAlive())) {
        return;
      }
    }

    if (batch && batch.done) {
      let sessionMeta = null;
      const metaDeadline = Date.now() + 1800;
      while (Date.now() < metaDeadline) {
        sessionMeta = await js(String.raw`
          (() => {
            const m = location.pathname.match(/\/app\/([a-zA-Z0-9_-]{8,})/);
            if (!m) return null;
            const id = m[1];
            const link = Array.from(
              document.querySelectorAll('a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]')
            ).find((a) => (a.getAttribute('href') || '').startsWith('/app/' + id));
            let title = link
              ? (link.innerText || link.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ')
              : '';
            if (!title) {
              const docTitle = (document.title || '').replace(/\s*-\s*Google Gemini$/i, '').trim();
              if (docTitle && docTitle !== 'Google Gemini' && docTitle !== 'Gemini') {
                title = docTitle;
              }
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

  await emit({ event: 'error', message: 'Gemini stream timed out' });
}

try {
  await main();
} catch (error) {
  const raw = String((error && error.message) || error || 'Gemini web driver error');
  const firstLine = raw.split('\n').map((s) => s.trim()).find(Boolean) || raw;
  await emit({ event: 'error', message: firstLine });
}
