use super::*;

fn parse(json: &str) -> Option<AiWebStreamEvent> {
    parse_script_line(&format!("{PROTOCOL_PREFIX}{json}"))
}

#[test]
fn parses_thinking_and_text_deltas() {
    assert_eq!(
        parse(r#"{"event":"thinking","delta":"hmm"}"#),
        Some(AiWebStreamEvent::Thinking("hmm".into()))
    );
    assert_eq!(
        parse(r#"{"event":"text","delta":"hello"}"#),
        Some(AiWebStreamEvent::Text("hello".into()))
    );
}

#[test]
fn parses_empty_delta() {
    assert_eq!(
        parse(r#"{"event":"text","delta":""}"#),
        Some(AiWebStreamEvent::Text(String::new()))
    );
    assert_eq!(
        parse(r#"{"event":"thinking","delta":""}"#),
        Some(AiWebStreamEvent::Thinking(String::new()))
    );
}

#[test]
fn parses_login_required_with_url() {
    assert_eq!(
        parse(r#"{"event":"login_required","url":"https://chat.deepseek.com/sign_in"}"#),
        Some(AiWebStreamEvent::LoginRequired {
            url: "https://chat.deepseek.com/sign_in".into()
        })
    );
}

#[test]
fn parses_done_and_error() {
    assert_eq!(parse(r#"{"event":"done"}"#), Some(AiWebStreamEvent::Done));
    assert_eq!(
        parse(r#"{"event":"error","message":"user is controlling"}"#),
        Some(AiWebStreamEvent::Error("user is controlling".into()))
    );
}

#[test]
fn ignores_invalid_json_and_unknown_events() {
    assert_eq!(parse("not json"), None);
    assert_eq!(parse(r#"{"event":"unknown"}"#), None);
    assert_eq!(parse(r#"{"delta":"missing event"}"#), None);
}

#[test]
fn ignores_non_protocol_noise() {
    assert_eq!(
        parse_script_line("[ego-browser:notice] Ego Lite update is available"),
        None
    );
    assert_eq!(parse_script_line(""), None);
    assert_eq!(parse_script_line("console output"), None);
}

#[test]
fn parses_session_events() {
    assert_eq!(
        parse(r#"{"event":"session","id":"s-1","title":"Hello"}"#),
        Some(AiWebStreamEvent::Session {
            id: "s-1".into(),
            title: Some("Hello".into()),
        })
    );
    assert_eq!(
        parse(
            r#"{"event":"sessions","items":[{"id":"s-1","title":"Chat 1","updated_at_ms":1000}]}"#
        ),
        Some(AiWebStreamEvent::Sessions(vec![AiWebSessionSummary {
            id: "s-1".into(),
            title: "Chat 1".into(),
            updated_at_ms: 1000,
        }]))
    );
    assert_eq!(
        parse(
            r#"{"event":"image","label":"grok-r1-1.jpg","path":"/tmp/grok-r1-1.jpg","original_url":"https://assets.grok.com/u/original_image.jpg","preview_url":"https://assets.grok.com/u/image.jpg","file_stem":"grok-s1-r1-1-orig"}"#
        ),
        Some(AiWebStreamEvent::GeneratedImage {
            label: "grok-r1-1.jpg".into(),
            path: "/tmp/grok-r1-1.jpg".into(),
            original_url: "https://assets.grok.com/u/original_image.jpg".into(),
            preview_url: "https://assets.grok.com/u/image.jpg".into(),
            file_stem: "grok-s1-r1-1-orig".into(),
        })
    );
    assert_eq!(
        parse(
            r#"{"event":"session_loaded","id":"s-1","title":"Chat 1","messages":[{"role":"user","thinking":"","thinking_ms":null,"text":"hi","created_at_ms":1000}]}"#
        ),
        Some(AiWebStreamEvent::SessionLoaded {
            id: "s-1".into(),
            title: Some("Chat 1".into()),
            messages: vec![AiWebHistoryMessage {
                role: "user".into(),
                thinking: String::new(),
                thinking_ms: None,
                text: "hi".into(),
                files: Vec::new(),
                images: Vec::new(),
                created_at_ms: 1000,
            }],
        })
    );
}

#[test]
fn scripts_for_site_returns_deepseek_gemini_and_grok_driver_bundles() {
    let (ds_helpers, ds_main) = scripts_for_site(crate::app::state::AiWebSite::Deepseek);
    assert!(ds_helpers.contains("lx-tui-ai-web-deepseek"));
    assert!(ds_helpers.contains("takeOverTaskSpace"));
    assert!(ds_helpers.contains("claimTaskSpace"));
    assert!(ds_main.contains("ds-toggle-button"));
    assert!(ds_main.contains("ds-markdown.ds-assistant-message-main-content"));
    assert!(ds_main.contains("readMarkdownText"));
    assert!(ds_main.contains("readFiberMarkdown"));
    assert!(ds_main.contains("window.__lxHtmlToMarkdown"));
    assert!(
        !ds_main.contains("this.cleanText(el.innerText"),
        "DeepSeek DOM fallback must not strip Markdown via innerText"
    );
    assert!(ds_main.contains("await ensureTaskSpaceControl()"));
    assert!(ds_helpers.contains("citation|reference"));
    assert!(ds_helpers.contains("[^\\\\]]*"));

    let (gm_helpers, gm_main) = scripts_for_site(crate::app::state::AiWebSite::Gemini);
    assert!(gm_helpers.contains("lx-tui-ai-web-gemini"));
    assert!(gm_helpers.contains("bard-mode-menu-button"));
    assert!(gm_helpers.contains("takeOverTaskSpace"));
    assert!(gm_helpers.contains("claimTaskSpace"));
    assert!(gm_helpers.contains("window.__lxHtmlToMarkdown"));
    assert!(
        !gm_helpers.contains("async function saveGeneratedImages"),
        "Gemini image helpers must live in gemini_images.js"
    );
    assert!(GEMINI_IMAGES_JS.contains("toGeminiOriginalImageUrl"));
    assert!(GEMINI_IMAGES_JS.contains("async function saveGeneratedImages"));
    assert!(GEMINI_IMAGES_JS.contains("async function runGeminiImageDownload"));
    assert!(GEMINI_IMAGES_JS.contains(r"s.replace(/\?.*$/, '').replace(/=[^=/]+$/, '')"));
    assert!(gm_main.contains("__lxGeminiStream"));
    assert!(gm_main.contains("window.__lxHtmlToMarkdown"));
    assert!(
        !gm_main.contains("el.innerText"),
        "Gemini stream collector must not strip Markdown via innerText"
    );
    assert!(gm_main.contains("await activateTaskSpace()"));
    assert!(gm_main.contains("await ensureTaskSpaceControl()"));
    assert!(gm_main.contains("imageWaitTicks < 250"));
    assert!(gm_main.contains("!isGenerating && this.stableTicks >= 14"));

    let (gk_helpers, gk_main) = scripts_for_site(crate::app::state::AiWebSite::Grok);
    assert!(gk_helpers.contains("lx-tui-ai-web-grok"));
    assert!(gk_helpers.contains("chat-submit"));
    assert!(gk_helpers.contains("takeOverTaskSpace"));
    assert!(gk_helpers.contains("claimTaskSpace"));
    assert!(gk_helpers.contains("replaceGrokCitations"));
    assert!(gk_helpers.contains("cardAttachmentsJson"));
    assert!(gk_helpers.contains("grok:render"));
    assert!(gk_helpers.contains("grokAttachmentCount"));
    assert!(gk_helpers.contains("emitLoginRequired"));
    assert!(gk_helpers.contains("citationMarkdown"));
    assert!(gk_main.contains("__lxGrokStream"));
    assert!(gk_main.contains("window.__lxHtmlToMarkdown"));
    assert!(gk_main.contains("CHANNEL_ASSISTANT_RESPONSE"));
    assert!(gk_main.contains("Network.webSocketFrameReceived"));
    assert!(
        !gk_main.contains("body.innerText"),
        "Grok stream collector must not strip Markdown via innerText"
    );
    assert!(gk_main.contains("ensureGrokComposer"));
    assert!(gk_main.contains("stripGrokDirectives"));
    assert!(gk_main.contains("baselineAssistants"));
    assert!(gk_main.contains("fillGrokPrompt"));
    assert!(gk_main.contains("submitGrokPrompt"));
    assert!(gk_main.contains("Grok response has no text"));
    assert!(gk_main.contains("Grok stream was superseded by another pane"));
    assert!(gk_main.contains("await activateTaskSpace()"));
    assert!(gk_main.contains("await ensureTaskSpaceControl()"));
}

#[test]
fn grok_script_source_escapes_prompt_and_keeps_thinking_empty() {
    let evil = "quote\" backtick` dollar${x}\nline</script>";
    let source = script_source(
        crate::app::state::AiWebSite::Grok,
        ScriptParams {
            action: "chat",
            model: Some("Build"),
            thinking: Some(""),
            prompt: Some(evil),
            ..Default::default()
        },
    );
    assert!(source.contains(&serde_json::to_string(evil).expect("json prompt")));
    assert!(
        source.contains("thinking: \"\","),
        "Grok must inject empty thinking"
    );
    assert!(
        source.contains("model: \"Build\","),
        "Grok Build model must be injected"
    );
}

#[test]
fn diag_line_filter_preserves_primary_error_and_ignores_stack_or_exit_trailer() {
    let mut last_diag: Option<&str> = None;
    let lines = [
        "[ego-browser:notice] attached",
        "Error: listTabs: Control of this task space is with the user",
        "This is a hard stop, not an obstacle to route around",
        "at buildEgoError (file:////[eval1]:461:19)",
        "ego's nodejs process exited with code 1.",
    ];
    for line in lines {
        if should_record_diag_line(line, last_diag) {
            last_diag = Some(line);
        }
    }
    assert_eq!(
        last_diag,
        Some("Error: listTabs: Control of this task space is with the user")
    );
}

#[test]
fn script_source_escapes_user_controlled_values() {
    let files = vec![
        "/tmp/a\"b.png".to_string(),
        "/tmp/back`tick.pdf".to_string(),
    ];
    let keep = vec!["keep\nname.pdf".to_string()];
    let evil = "quote\" backtick` dollar${x}\nline</script>\u{2028}end";
    let source = script_source(
        crate::app::state::AiWebSite::Gemini,
        ScriptParams {
            action: "chat",
            model: Some(evil),
            thinking: Some(evil),
            prompt: Some(evil),
            files: &files,
            keep_files: &keep,
            session_id: Some(evil),
            title: Some(evil),
            edit_user_index: Some(3),
            ..Default::default()
        },
    );

    // 所有用户可控值都必须以 serde_json 字符串字面量出现，换行/引号/反斜杠不得裸奔。
    assert!(source.contains(&serde_json::to_string(evil).expect("json string")));
    assert!(source.contains(&serde_json::to_string(&files).expect("json files")));
    assert!(source.contains(&serde_json::to_string(&keep).expect("json keep files")));
    let prompt_line = source
        .lines()
        .find(|line| line.trim_start().starts_with("prompt: "))
        .expect("prompt field");
    assert!(prompt_line.contains("\\nline</script>"), "{prompt_line}");
    assert!(prompt_line.trim_end().ends_with("\","), "{prompt_line}");
    assert!(source.contains("editIndex: 3,"));
    assert!(source.starts_with("const __LX_PARAMS = {\n"));
}

#[test]
fn gemini_script_source_concatenates_image_helpers_before_main_script() {
    let source = script_source(
        crate::app::state::AiWebSite::Gemini,
        ScriptParams {
            action: "download_image",
            ..Default::default()
        },
    );
    let dom_md_at = source
        .find("function lxHtmlToMarkdown(")
        .expect("shared dom markdown serializer");
    let helpers_at = source
        .find("async function runNonChatAction")
        .expect("gemini helpers");
    let images_at = source
        .find("async function saveGeneratedImages")
        .expect("gemini image helpers");
    let main_at = source
        .find("async function main()")
        .expect("gemini main script");
    assert!(
        dom_md_at < helpers_at && helpers_at < images_at && images_at < main_at,
        "dom_markdown must precede helpers, image helpers, and main script"
    );
    assert!(source.contains("await runGeminiImageDownload(currentUrl);"));
}

#[test]
fn dom_markdown_serializer_converts_deepseek_gemini_grok_html_and_preserves_streaming_prefix() {
    let harness = r#"
function mkText(value) {
  return {
    nodeType: 3,
    nodeValue: String(value),
    textContent: String(value),
    parentNode: null,
    nextSibling: null,
  };
}

function mkEl(tag, attrs, children) {
  const attrMap = attrs || {};
  const clsSet = new Set(
    String(attrMap.class || '')
      .split(/\s+/)
      .filter(Boolean),
  );
  const el = {
    nodeType: 1,
    tagName: String(tag).toUpperCase(),
    className: String(attrMap.class || ''),
    classList: {
      contains: (c) => clsSet.has(c),
    },
    attributes: attrMap,
    parentNode: null,
    nextSibling: null,
    firstChild: null,
    children: [],
    checked: Boolean(attrMap.checked),
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(attrMap, name) ? String(attrMap[name]) : null;
    },
    hasAttribute(name) {
      return Object.prototype.hasOwnProperty.call(attrMap, name);
    },
    get textContent() {
      return this.childNodes.map((c) => c.textContent || '').join('');
    },
    childNodes: [],
    closest(selector) {
      const parts = selector.split(',').map((s) => s.trim()).filter(Boolean);
      let cur = this;
      while (cur && cur.nodeType === 1) {
        if (parts.some((p) => matchSingle(cur, p))) return cur;
        cur = cur.parentNode;
      }
      return null;
    },
    querySelectorAll(selector) {
      const results = [];
      const parts = selector.split(',').map((s) => s.trim()).filter(Boolean);
      const walk = (node) => {
        for (const child of node.children || []) {
          if (parts.some((p) => matchSelector(child, p, this))) {
            results.push(child);
          }
          walk(child);
        }
      };
      walk(this);
      return results;
    },
    querySelector(selector) {
      return this.querySelectorAll(selector)[0] || null;
    },
  };
  const kids = (children || []).map((c) => (typeof c === 'string' ? mkText(c) : c));
  for (let i = 0; i < kids.length; i += 1) {
    kids[i].parentNode = el;
    kids[i].nextSibling = kids[i + 1] || null;
  }
  el.childNodes = kids;
  el.children = kids.filter((k) => k.nodeType === 1);
  el.firstChild = kids[0] || null;
  return el;
}

function matchSingle(el, sel) {
  if (!el || el.nodeType !== 1) return false;
  if (sel.startsWith('.')) return el.classList.contains(sel.slice(1));
  const attrMatch = sel.match(/^([a-z0-9_-]+)?\[([a-z0-9_-]+)="([^"]+)"\]$/i);
  if (attrMatch) {
    const [, tagPart, attrName, attrVal] = attrMatch;
    if (tagPart && el.tagName.toLowerCase() !== tagPart.toLowerCase()) return false;
    return el.getAttribute(attrName) === attrVal;
  }
  if (sel.includes('.')) {
    const [tagPart, ...classes] = sel.split('.');
    if (tagPart && el.tagName.toLowerCase() !== tagPart.toLowerCase()) return false;
    return classes.every((c) => el.classList.contains(c));
  }
  return el.tagName.toLowerCase() === sel.toLowerCase();
}

function matchSelector(el, sel, scopeRoot) {
  if (sel.startsWith(':scope > ')) {
    const rest = sel.slice(':scope > '.length).trim();
    if (!rest.includes('>')) {
      return el.parentNode === scopeRoot && matchSingle(el, rest);
    }
  }
  if (sel.includes('>')) {
    const segs = sel.split('>').map((s) => s.trim());
    if (!matchSingle(el, segs[segs.length - 1])) return false;
    let cur = el.parentNode;
    for (let i = segs.length - 2; i >= 0; i -= 1) {
      if (segs[i] === ':scope') return cur === scopeRoot;
      if (!matchSingle(cur, segs[i])) return false;
      cur = cur ? cur.parentNode : null;
    }
    return true;
  }
  if (sel.includes(' ')) {
    const segs = sel.split(/\s+/);
    if (!matchSingle(el, segs[segs.length - 1])) return false;
    let cur = el.parentNode;
    let idx = segs.length - 2;
    while (cur && idx >= 0) {
      if (matchSingle(cur, segs[idx])) idx -= 1;
      cur = cur.parentNode;
    }
    return idx < 0;
  }
  return matchSingle(el, sel);
}

// 1. Screenshot reproduction tree (Gemini / DeepSeek / Grok DOM with headings, multi-p list items, citations, code blocks, tables)
const screenshotDom = mkEl('div', { class: 'markdown' }, [
  mkEl('p', {}, [
    '这两项因素不仅直接冲击生育意愿，而且在现实中形成了',
    mkEl('strong', {}, ['负向循环']),
    '。',
    mkEl('source-footnote', {}, ['[1]']),
    mkEl('span', { class: 'ds-markdown-cite' }, ['-13']),
  ]),
  mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突']),
  mkEl('ul', {}, [
    mkEl('li', {}, [
      mkEl('p', {}, [mkEl('b', {}, ['实际到手收入承压'])]),
      mkEl('p', {}, ['低收入群体与初入职场的青年人群体抗风险能力最弱。']),
    ]),
    mkEl('li', {}, [
      mkEl('p', {}, [mkEl('strong', {}, ['隔代抚养支持链条受阻'])]),
      mkEl('p', {}, ['中国绝大多数双职工家庭的育儿模式高度依赖祖辈照料。']),
    ]),
  ]),
  // DeepSeek code block with banner + Copy button
  mkEl('div', { class: 'md-code-block' }, [
    mkEl('div', { class: 'md-code-block-banner-wrap' }, [
      mkEl('div', { class: 'md-code-block-infostring' }, ['rust']),
      mkEl('div', { class: 'ds-markdown-code-copy-button' }, ['Copy']),
    ]),
    mkEl('pre', {}, ['fn main() {\n    println!("ok");\n}\n']),
  ]),
  // Gemini code-block with decoration header + buttons
  mkEl('code-block', {}, [
    mkEl('div', { class: 'code-block-decoration' }, [
      mkEl('span', {}, ['Python']),
      mkEl('div', { class: 'buttons' }, [mkEl('button', {}, ['复制'])]),
    ]),
    mkEl('pre', {}, [
      mkEl('code', { 'data-test-id': 'code-content' }, ['print("gemini")\n']),
    ]),
  ]),
  // Grok .not-prose code block + non-code .not-prose placeholder card
  mkEl('div', { class: 'not-prose' }, [
    mkEl('div', { class: 'chat-code-block not-prose', 'data-testid': 'code-block' }, [
      mkEl('div', { 'data-code-card': 'true' }, [
        mkEl('span', { class: 'font-mono' }, ['Bash']),
        mkEl('button', {}, ['复制']),
        mkEl('pre', { class: 'shiki', 'aria-label': 'Bash 代码' }, [
          mkEl('code', {}, [
            mkEl('span', { class: 'line' }, ['echo 1']),
            mkEl('span', { class: 'line' }, ['echo 2']),
          ]),
        ]),
      ]),
    ]),
  ]),
  mkEl('div', { class: 'not-prose' }, ['Generating image...']),
  // Table with footer button
  mkEl('table-block', {}, [
    mkEl('table', {}, [
      mkEl('thead', {}, [
        mkEl('tr', {}, [mkEl('th', {}, ['项目']), mkEl('th', {}, ['影响'])]),
      ]),
      mkEl('tbody', {}, [
        mkEl('tr', {}, [
          mkEl('td', {}, [mkEl('strong', {}, ['社保'])]),
          mkEl('td', {}, ['现金流承压']),
        ]),
      ]),
    ]),
    mkEl('div', { class: 'table-footer' }, [mkEl('button', {}, ['More'])]),
  ]),
]);

const fullMd = lxHtmlToMarkdown(screenshotDom, { streaming: false });

// 2. Streaming prefix monotonicity check across progressive DOM snapshots
const snapshots = [
  mkEl('div', {}, [mkEl('h3', {}, ['一、 强制提高'])]),
  mkEl('div', {}, [mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突'])]),
  mkEl('div', {}, [
    mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突']),
    mkEl('ul', {}, [
      mkEl('li', {}, [mkEl('p', {}, [mkEl('b', {}, ['实际到手'])])]),
    ]),
  ]),
  mkEl('div', {}, [
    mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突']),
    mkEl('ul', {}, [
      mkEl('li', {}, [mkEl('p', {}, [mkEl('b', {}, ['实际到手收入承压'])])]),
    ]),
  ]),
  mkEl('div', {}, [
    mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突']),
    mkEl('ul', {}, [
      mkEl('li', {}, [
        mkEl('p', {}, [mkEl('b', {}, ['实际到手收入承压'])]),
        mkEl('p', {}, ['低收入群体']),
      ]),
    ]),
  ]),
  mkEl('div', {}, [
    mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突']),
    mkEl('ul', {}, [
      mkEl('li', {}, [
        mkEl('p', {}, [mkEl('b', {}, ['实际到手收入承压'])]),
        mkEl('p', {}, ['低收入群体']),
      ]),
    ]),
    mkEl('pre', { class: 'language-rust' }, ['fn main() {']),
  ]),
  mkEl('div', {}, [
    mkEl('h3', {}, ['一、 强制提高社保与生育意愿的直接冲突']),
    mkEl('ul', {}, [
      mkEl('li', {}, [
        mkEl('p', {}, [mkEl('b', {}, ['实际到手收入承压'])]),
        mkEl('p', {}, ['低收入群体']),
      ]),
    ]),
    mkEl('pre', { class: 'language-rust' }, ['fn main() {\n    println!("hi");\n}\n']),
  ]),
];

const streamed = snapshots.map((dom) => lxHtmlToMarkdown(dom, { streaming: true }));
streamed.push(lxHtmlToMarkdown(snapshots[snapshots.length - 1], { streaming: false }));

for (let i = 1; i < streamed.length; i += 1) {
  if (!streamed[i].startsWith(streamed[i - 1])) {
    throw new Error(
      `Step ${i} is not a prefix extension:\nPREV: ${JSON.stringify(streamed[i - 1])}\nCURR: ${JSON.stringify(streamed[i])}`,
    );
  }
}

process.stdout.write(JSON.stringify({ fullMd, finalStreamed: streamed[streamed.length - 1] }));
"#;

    let script = format!("{DOM_MARKDOWN_JS}\n{harness}");
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(&script)
        .output()
        .expect("node must be available to run dom_markdown test");
    assert!(
        output.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid json from node");
    let full_md = parsed["fullMd"].as_str().expect("fullMd string");

    assert!(
        full_md.contains("### 一、 强制提高社保与生育意愿的直接冲突"),
        "{full_md}"
    );
    assert!(full_md.contains("**负向循环**"), "{full_md}");
    assert!(
        full_md
            .contains("- **实际到手收入承压**\n  低收入群体与初入职场的青年人群体抗风险能力最弱。"),
        "{full_md}"
    );
    assert!(
        full_md.contains(
            "- **隔代抚养支持链条受阻**\n  中国绝大多数双职工家庭的育儿模式高度依赖祖辈照料。"
        ),
        "{full_md}"
    );
    assert!(
        full_md.contains("```rust\nfn main() {\n    println!(\"ok\");\n}\n```"),
        "{full_md}"
    );
    assert!(
        full_md.contains("```python\nprint(\"gemini\")\n```"),
        "{full_md}"
    );
    assert!(
        full_md.contains("```bash\necho 1\necho 2\n```"),
        "{full_md}"
    );
    assert!(
        full_md.contains("| 项目 | 影响 |\n| --- | --- |\n| **社保** | 现金流承压 |"),
        "{full_md}"
    );
    assert!(
        !full_md.contains("Copy"),
        "must strip Copy button: {full_md}"
    );
    assert!(
        !full_md.contains("复制"),
        "must strip 复制 button: {full_md}"
    );
    assert!(
        !full_md.contains("-13"),
        "must strip .ds-markdown-cite: {full_md}"
    );
    assert!(
        !full_md.contains("[1]"),
        "must strip source-footnote: {full_md}"
    );
    assert!(
        !full_md.contains("Generating image..."),
        "must strip non-code .not-prose placeholder: {full_md}"
    );

    // Verify that `markdown_render::render_wrapped_code_blocks` renders the extracted Markdown
    // with proper heading, strong, bullet, code block, and table styles!
    let rendered = crate::ui::markdown_render::render_wrapped_code_blocks(full_md, 80);
    assert!(
        rendered.iter().any(|line| {
            line.text() == "一、 强制提高社保与生育意愿的直接冲突"
                && line.spans[0].1 == crate::ui::style::md_read_heading_level(3)
        }),
        "heading must render with h3 style: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| {
            line.text().starts_with("• ")
                && line.spans.iter().any(|(text, s)| {
                    text == "实际到手收入承压" && *s == crate::ui::style::md_read_strong()
                })
        }),
        "list item must render bullet and bold title: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| {
            line.text()
                .contains("低收入群体与初入职场的青年人群体抗风险能力最弱")
                && line.indent == 2
        }),
        "list continuation paragraph must stay indented at column 2: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.code == Some(crate::ui::markdown_render::CodeLineKind::Body)),
        "code block must render with CodeLineKind::Body: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| line.table.is_some()),
        "table must render with TableLineRef: {rendered:?}"
    );
}

#[test]
fn assembled_web_driver_scripts_for_all_sites_pass_js_syntax_validation() {
    for site in [
        crate::app::state::AiWebSite::Deepseek,
        crate::app::state::AiWebSite::Gemini,
        crate::app::state::AiWebSite::Grok,
    ] {
        let source = script_source(
            site,
            ScriptParams {
                action: "chat",
                model: Some("Fast"),
                thinking: Some("deepthink"),
                prompt: Some("test markdown"),
                first_turn: true,
                ..Default::default()
            },
        );
        // Wrap in async function because top-level await is used inside ego-browser nodejs runner.
        let wrapped = format!("async function __check() {{\n{source}\n}}");
        let output = std::process::Command::new("node")
            .arg("-e")
            .arg(&wrapped)
            .output()
            .expect("node must be available");
        assert!(
            output.status.success(),
            "JS syntax error for {site:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn deepseek_stream_collector_survives_non_sse_xhr_and_preserves_markdown_fences() {
    let (_ds_helpers, ds_main) = scripts_for_site(crate::app::state::AiWebSite::Deepseek);
    // Extract the `const runId = await js(String.raw`...`);` payload from deepseek_script.js
    let start_marker = "const runId = await js(String.raw`";
    let start = ds_main
        .find(start_marker)
        .expect("deepseek stream setup start")
        + start_marker.len();
    let end = ds_main[start..]
        .find("\n  `);")
        .expect("deepseek stream setup end")
        + start;
    let stream_iife = &ds_main[start..end];

    let harness = format!(
        r####"
const THINKING = 'deepthink';
global.window = global;
global.document = {{
  querySelectorAll: () => [],
  querySelector: () => null,
}};
function MockXHR() {{
  this.status = 200;
  this.readyState = 0;
  this.responseText = '';
  this._headers = {{}};
  this._listeners = {{}};
}}
MockXHR.prototype.open = function(method, url) {{}};
MockXHR.prototype.send = function() {{}};
MockXHR.prototype.getResponseHeader = function(name) {{
  return this._headers[String(name).toLowerCase()] || null;
}};
MockXHR.prototype.addEventListener = function(ev, fn) {{
  (this._listeners[ev] = this._listeners[ev] || []).push(fn);
}};
MockXHR.prototype._emit = function(ev) {{
  for (const fn of this._listeners[ev] || []) fn();
}};
global.XMLHttpRequest = MockXHR;
global.fetch = function() {{ return Promise.resolve({{ ok: true }}); }};

{DOM_MARKDOWN_JS}
window.__lxHtmlToMarkdown = lxHtmlToMarkdown;
const runId = {stream_iife};
const stream = window.__lxDeepSeekStream;
if (!stream || stream.runId !== runId) throw new Error('stream not initialized');

// 1. First XHR returns JSON business challenge -> must NOT lock out second XHR
const xhr1 = new XMLHttpRequest();
xhr1._headers['content-type'] = 'application/json';
xhr1.open('POST', '/api/v0/chat/completion');
xhr1.send();
xhr1.readyState = 4;
xhr1.responseText = '{{"code":0,"challenge":"abc"}}';
xhr1._emit('readystatechange');
xhr1._emit('loadend');

if (stream.sseText || stream.sseThinking) {{
  throw new Error('JSON challenge XHR should not activate SSE');
}}

// 2. Replayed XHR returns real SSE stream with Markdown & code fence containing [citation:1]
const xhr2 = new XMLHttpRequest();
xhr2._headers['content-type'] = 'text/event-stream; charset=utf-8';
xhr2.open('POST', '/api/v0/chat/completion');
xhr2.send();
xhr2.readyState = 3;
const frames = [
  'data: {{"o":"SET","v":{{"response":{{"fragments":[{{"id":1,"type":"THINK","content":"**思考中**"}}]}}}}}}\n\n',
];
xhr2.responseText += frames[0];
xhr2._emit('progress');

// Simulate DeepThink > 1.2s before text arrives: tickDom() must NOT pre-empt SSE text
stream.sseSeenAt = Date.now() - 5000;
stream.pollStartedAt = Date.now() - 5000;
stream.tickDom();
if (stream.domTextStarted || stream.domThinkingStarted) {{
  throw new Error('tickDom must not pre-empt active SSE stream during DeepThink');
}}

const moreFrames = [
  'data: {{"p":"response/fragments","o":"APPEND","v":[{{"id":2,"type":"RESPONSE","content":"### 标题\\n\\n- **要点**[citation:2]\\n```rust\\nlet s = \\"[citation:1]\\";\\n```\\n区间 [0, 1["}}]}}\n\n',
  'event: finish\ndata: {{}}\n\n',
];
for (const frame of moreFrames) {{
  xhr2.responseText += frame;
  xhr2._emit('progress');
}}
xhr2.readyState = 4;
xhr2._emit('readystatechange');

process.stdout.write(JSON.stringify({{
  done: stream.done,
  queue: stream.queue,
}}));
"####
    );

    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(&harness)
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid json from node");
    assert_eq!(parsed["done"], serde_json::Value::Bool(true));
    let queue = parsed["queue"].as_array().expect("queue array");
    let thinking: String = queue
        .iter()
        .filter(|item| item["kind"] == "thinking")
        .filter_map(|item| item["delta"].as_str())
        .collect();
    let text: String = queue
        .iter()
        .filter(|item| item["kind"] == "text")
        .filter_map(|item| item["delta"].as_str())
        .collect();
    assert_eq!(thinking, "**思考中**");
    assert_eq!(
        text,
        "### 标题\n\n- **要点**\n```rust\nlet s = \"[citation:1]\";\n```\n区间 [0, 1["
    );
}
