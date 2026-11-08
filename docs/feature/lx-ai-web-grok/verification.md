# Grok 接入 Web 入口 — 验证报告 (`verification.md`)

- **执行日期**：2026-11-08
- **工作区**：`.worktrees/lx-ai-web-grok`（分支 `feat/lx-ai-web-grok`，基线 `d6e8bada`）
- **相关提交**：`a8fda29c`（Grok 接入）、`7b15a108`（引用标签修复）、`00d3c676`（DeepSeek 引用残渣修复）
- **二进制**：`./target/debug/lx-tui`（`cargo build` 通过）
- **验证方式**：功能项 V1–V12、边界项 V13–V17 均通过 CLI 实测；静态项 V0 通过 Cargo/Node CLI 实测；TUI 鼠标与视觉项见 §4，不计入 CLI 通过项。
- **任务空间**：首轮 E2E 使用临时 `lx-tui-ai-web-grok-verify`；引用修复复验使用临时 `inspect grok citations`；验证后均已关闭，代码常量已恢复为 `lx-tui-ai-web-grok`。

## 结论

- V0-1–V0-5、V1–V17：**全部通过**（CLI）。
- M1–M6：非 CLI 项；M5 的 Rust 逻辑由单元测试覆盖并通过，M1/M2 视觉待用户目测，M3/M4/M6 未执行。

---

## 1. 静态校验

### V0-1 `cargo fmt --check`

- **输入内容**：工作区全部 Rust 源码。
- **验证步骤**：
  1. `cd .worktrees/lx-ai-web-grok`
  2. `cargo fmt --check`
- **期望结果**：退出码 0，无格式差异。
- **CLI 验证结果**：通过，无 diff。

### V0-2 `cargo clippy`

- **输入内容**：全部 target（lib、bin、tests）。
- **验证步骤**：
  1. `cargo clippy --all-targets -- -D warnings`
- **期望结果**：退出码 0，无 warning。
- **CLI 验证结果**：通过。

### V0-3 `cargo test`

- **输入内容**：全量单元测试与集成测试。
- **验证步骤**：
  1. `cargo test`
- **期望结果**：0 failed；允许项目既有的 ignored 测试。
- **CLI 验证结果**：通过。最终全量 `4895 passed / 0 failed / 4 ignored`。早期一次 `event::chat::tests::user_shell::short_script_runs_persists_output_without_model` 失败为 shell 管道环境抖动，单独复跑通过，与本次改动无关。

### V0-4 JavaScript 语法检查

- **输入内容**：Grok / DeepSeek / Gemini 驱动脚本。
- **验证步骤**：
  1. `node --check src/chat/web/grok_helpers.js`
  2. `node --check src/chat/web/grok_script.js`
  3. `node --check src/chat/web/deepseek_helpers.js`
  4. `node --check src/chat/web/deepseek_script.js`
  5. `node --check src/chat/web/gemini_helpers.js`
  6. `node --check src/chat/web/gemini_script.js`
- **期望结果**：全部退出码 0。
- **CLI 验证结果**：通过。

### V0-5 `cargo build`

- **输入内容**：当前工作区源码。
- **验证步骤**：
  1. `cargo build`
- **期望结果**：生成 `./target/debug/lx-tui`，退出码 0。
- **CLI 验证结果**：通过。

---

## 2. 功能验证（CLI）

> 以下命令均在 `.worktrees/lx-ai-web-grok` 下执行；`<uuid>` 表示当次测试会话 ID，实际测试会话已在 V10 删除。

### V1 `web models` 模型与站点信息

- **输入内容**：`--site grok --json`，无用户对话输入。
- **验证步骤**：
  1. `./target/debug/lx-tui web models --site grok --json`
  2. 检查 `.sites[]` 中 `site == "grok"` 的 `label`、`defaultModel`、`models`、`thinkingVariants`、`sessionPrefix`、`taskSpace`。
- **期望结果**：
  - `site = "grok"`，`label = "Grok"`
  - `defaultModel = "Fast"`，`models = ["Fast", "Build"]`
  - `thinkingVariants = []`
  - `sessionPrefix = "ai-web-grok-"`
  - `taskSpace = "lx-tui-ai-web-grok"`
- **CLI 验证结果**：通过。实际输出：

  ```json
  {
    "site": "grok",
    "label": "Grok",
    "defaultModel": "Fast",
    "models": ["Fast", "Build"],
    "thinkingVariants": [],
    "sessionPrefix": "ai-web-grok-",
    "taskSpace": "lx-tui-ai-web-grok"
  }
  ```

### V2 `web chat` 真实发送

- **输入内容**：提示词 `Reply with the single word OK.`，站点 `grok`，默认模型 `Fast`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site grok --json "Reply with the single word OK."`
  2. 检查 stdout 的助手回答与 `sessionId`。
  3. 确认输出没有伪造的 `Thinking` 或 `工作了 Ns`。
- **期望结果**：回答只含 `OK`；`sessionId` 以 `ai-web-grok-` 开头；无 Thinking 块。
- **CLI 验证结果**：通过。实际回答 `OK`，`sessionId = ai-web-grok-414fe206...`，无 Thinking。

### V3 `web list` 会话列表

- **输入内容**：`--site grok --json`。
- **验证步骤**：
  1. `./target/debug/lx-tui web list --site grok --json`
  2. 在返回列表中查找 V2 的 `sessionId`，同时确认用户原有会话仍在。
- **期望结果**：列表包含 V2 测试会话，不丢失用户已有会话。
- **CLI 验证结果**：通过。列表同时包含测试会话与用户原有会话。

### V4 `web show` 会话详情

- **输入内容**：V2 的 `ai-web-grok-<uuid>`。
- **验证步骤**：
  1. `./target/debug/lx-tui web show ai-web-grok-<uuid> --json`
  2. 检查 `title` 与 `messages` 数量、用户/助手正文。
- **期望结果**：`title` 非 `null`（由会话列表 REST 补齐）；2 条消息：用户提示词与助手 `OK`。
- **CLI 验证结果**：通过。2 条消息，标题来自会话列表 REST，不再为 `null`。

### V5 `web rename` 重命名

- **输入内容**：V2 的 `ai-web-grok-<uuid>`，新标题 `LX Grok Test`。
- **验证步骤**：
  1. `./target/debug/lx-tui web rename ai-web-grok-<uuid> "LX Grok Test" --site grok --json`
  2. `./target/debug/lx-tui web list --site grok --json`
  3. 检查 `ok` 字段与列表标题。
- **期望结果**：`ok: true`；`web list` 中该会话标题变为 `LX Grok Test`。
- **CLI 验证结果**：通过，`ok: true`，列表标题已更新。

### V6 `--edit-index` 追加语义

- **输入内容**：V2 会话，`--edit-index 0`，新提示词 `Reply with the single word RETRY.`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site grok --session ai-web-grok-<uuid> --edit-index 0 "Reply with the single word RETRY."`
  2. 检查 stdout 的 `editIndex` 字段。
  3. 再执行 `web show ai-web-grok-<uuid> --json`，确认旧轮次仍在、新轮次追加。
- **期望结果**：回答 `RETRY`；`editIndex: null`；Grok 端保留旧轮次并追加新用户消息 + 新回答（Grok Edit 无法原地替换）。
- **CLI 验证结果**：通过。回答 `RETRY`，`editIndex: null`，旧轮次保留，新轮次追加。

### V7 `web new` 新建会话

- **输入内容**：`--site grok`，无提示词。
- **验证步骤**：
  1. `./target/debug/lx-tui web new --site grok --json`
  2. 检查命令返回值、浏览器 URL、页面消息数与 composer。
- **期望结果**：返回成功；页面回到 `https://grok.com/`；0 条消息；composer 可见。
- **CLI 验证结果**：通过。页面回到 `https://grok.com/`，0 条消息，composer 可见。

### V8 `web stop` 停止生成

- **输入内容**：长计数提示词（要求从 1 数到 200，每行一个数字）。
- **验证步骤**：
  1. 后台启动：
     ```sh
     ./target/debug/lx-tui web chat --site grok "Count from 1 to 200, one number per line." > /tmp/grok-stop.log 2>&1 &
     pid=$!
     ```
  2. 等待约 4 秒：`sleep 4`
  3. `./target/debug/lx-tui web stop --site grok`
  4. `wait $pid`，检查 `/tmp/grok-stop.log` 是否停止增长。
- **期望结果**：`stop` 返回成功；输出停在已生成位置，不继续增长。
- **CLI 验证结果**：通过。生成停止在 `one..eight` 区间，后续无新增内容。

### V9 `--file` 附件上传

- **输入内容**：临时文件内容 `BANANA`；提示词 `Reply with the secret word from the attached file.`。
- **验证步骤**：
  1. `printf 'BANANA\n' > /tmp/grok-secret.txt`
  2. `./target/debug/lx-tui web chat --site grok --file /tmp/grok-secret.txt "Reply with the secret word from the attached file."`
  3. 检查回答是否包含文件中的 `BANANA`。
- **期望结果**：附件 chip 上传成功；回答包含 `BANANA`。
- **CLI 验证结果**：通过。回答为 `BANANA`，附件上传成功。

### V10 `web delete` 清理测试会话

- **输入内容**：本轮创建的 4 个 `ai-web-grok-<uuid>` 测试会话。
- **验证步骤**：
  1. 对每个测试会话执行：
     `./target/debug/lx-tui web delete ai-web-grok-<uuid> --site grok`
  2. `./target/debug/lx-tui web list --site grok --json`
  3. 确认测试会话全部消失，用户原有会话仍在。
- **期望结果**：4 个测试会话全部删除；列表只剩用户原有 2 个会话。
- **CLI 验证结果**：通过。4 个测试会话全部删除，用户原有 2 个会话保留。

### V11 历史引用标签清洗

- **输入内容**：带联网引用的用户会话 `ai-web-grok-5ff186ff-ebd7-4cc4-9156-a620da9b0464`。
- **验证步骤**：
  1. `./target/debug/lx-tui web show ai-web-grok-5ff186ff-ebd7-4cc4-9156-a620da9b0464 --json > /tmp/grok-show.json`
  2. 检查是否含原始标签：
     `rg -n "grok:render|<argument|card_id" /tmp/grok-show.json`
  3. 检查是否映射为来源链接：
     `rg -n "\[[^]]+\]\(https?://" /tmp/grok-show.json`
- **期望结果**：第 2 步无命中；第 3 步出现 `[来源名](url)`，例如 `[Chinatimes](https://chinatimes.com.hk/...)`、`[Cctv](...)`、`[Bbc](...)`、`[Sina](...)`。
- **CLI 验证结果**：通过。无 `<grok:render>` / `<argument>` / `card_id`；来源链接正常显示。来源名为 URL 域名推导，不是完整文章标题。

### V12 实时引用标签清洗

- **输入内容**：提示词 `Search the web for today's top news headline and cite the source.`，站点 `grok`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site grok "Search the web for today's top news headline and cite the source." > /tmp/grok-live.log 2>&1`
  2. `rg -n "grok:render|<argument|card_id" /tmp/grok-live.log`
  3. 检查输出中的来源名。
- **期望结果**：实时回答显示来源名；不出现原始 `<grok:render>` 或未闭合标签。
- **CLI 验证结果**：通过。回答含来源名，无原始标签；该测试会话已删除。

---

## 3. CLI 边界验证

### V13 `--site gk` 别名

- **输入内容**：`--site gk --json`。
- **验证步骤**：
  1. `./target/debug/lx-tui web models --site gk --json`
  2. 与 V1 的 Grok 输出对比。
- **期望结果**：`gk` 等价于 `grok`，返回相同模型与任务空间信息。
- **CLI 验证结果**：通过，输出与 V1 一致。

### V14 Grok 拒绝 `--thinking`

- **输入内容**：`--site grok --thinking low`，提示词 `hi`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site grok --thinking low "hi"`
- **期望结果**：退出码 1，报错 `Grok has no thinking variants; omit '--thinking'`。
- **CLI 验证结果**：通过。实际输出：

  ```text
  lx-tui: Grok has no thinking variants; omit '--thinking'
  ```

### V15 Grok 非法模型

- **输入内容**：`--site grok --model Expert`，提示词 `hi`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site grok --model Expert "hi"`
- **期望结果**：退出码 1，报错 `invalid '--model' value 'Expert' for Grok; expected 'Fast' or 'Build'`。
- **CLI 验证结果**：通过。实际输出：

  ```text
  lx-tui: invalid '--model' value 'Expert' for Grok; expected 'Fast' or 'Build'
  ```

### V16 跨站点 session 冲突

- **输入内容**：`--site gemini --session ai-web-grok-00000000-0000-0000-0000-000000000000`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site gemini --session ai-web-grok-00000000-0000-0000-0000-000000000000 "hi"`
- **期望结果**：退出码 1，报错说明该 session 属于 `grok`，不能指定 `--site gemini`。
- **CLI 验证结果**：通过。实际输出：

  ```text
  lx-tui: session 'ai-web-grok-00000000-0000-0000-0000-000000000000' belongs to 'grok' but '--site gemini' was specified
  ```

### V17 CLI 帮助文案

- **输入内容**：`web chat --help`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --help | rg -n "site|thinking|model"`
- **期望结果**：帮助中出现 `<deepseek|gemini|grok>`，并说明 Grok 仅支持 `--model Fast|Build`、没有 thinking 档位。
- **CLI 验证结果**：通过。实际包含：

  ```text
  chat [<PROMPT>] [--site <deepseek|gemini|grok>] ...
  Grok supports --model Fast|Build and has no thinking variants
  ```

---

## 4. 人工验证项（非 CLI，不计入通过项）

以下项依赖真实鼠标/视觉或需要退出登录，无法用 CLI 自动断言；每一项仍按“输入 / 步骤 / 期望 / 结果”记录，结果由用户 `cargo run` 目测。

### M1 TUI 三站点下划线视觉

- **输入内容**：lx 首页 `DeepSeek · Gemini · Grok` 三个站点标签。
- **验证步骤**：
  1. `cargo run`
  2. 打开 lx 页，依次点击三个站点。
  3. 观察下划线位置、命中区域与窄窗格下的三等分布局。
- **期望结果**：下划线跟随当前站点；三个标签都可见、可点击；窄窗格不被裁切。
- **CLI 验证结果**：不适用，待用户目测。

### M2 lx 页 Toast 位置

- **输入内容**：在 lx 窗格触发一次 compact 失败提示（或同级的窗格内错误提示）。
- **验证步骤**：
  1. `cargo run`
  2. 在 lx 窗格触发提示。
  3. 观察 Toast 是否在 lx 窗格内容区内、是否按窗格宽度折行。
- **期望结果**：Toast 不横穿全屏、不遮挡其他窗格；全局槽仅用于非 lx 区域或弹窗未关闭场景。
- **CLI 验证结果**：不适用，待用户目测。

### M3 冷启动

- **输入内容**：关闭 Grok 所有标签页后，执行 `web chat --site grok "hi"`。
- **验证步骤**：
  1. 在 ego-browser 关闭 Grok 全部标签页。
  2. `./target/debug/lx-tui web chat --site grok "hi"`
  3. 观察是否等待页面渲染并成功拿到回答。
- **期望结果**：不报 `Grok composer not found` 或 `Grok model menu not found`；正常回答。
- **CLI 验证结果**：未执行；需关闭标签页后补测。

### M4 未登录场景

- **输入内容**：手动退出 Grok 登录态后，执行 `web chat --site grok "hi"`。
- **验证步骤**：
  1. 在 ego-browser 退出 Grok。
  2. `./target/debug/lx-tui web chat --site grok "hi"`
  3. 观察错误文案与任务空间控制权。
- **期望结果**：提示 `Please sign in to Grok in ego-browser`，并移交任务空间控制权。
- **CLI 验证结果**：未执行；本次验证时账号已登录，需手动退出后复测。

### M5 TUI `/rewind`

- **输入内容**：TUI 中含附件轮次与纯文本轮次分别执行 `/rewind`。
- **验证步骤**：
  1. `cargo run`
  2. 对带附件的用户消息执行 `/rewind`。
  3. 对纯文本用户消息执行 `/rewind`。
- **期望结果**：
  - 带附件：提示 `Grok /rewind cannot restore attachments; use /new or send without files`，消息与输入不变。
  - 纯文本：保留旧消息、回填输入，提示 `Grok /rewind appends a retry turn; use /new for a clean chat`；下一次发送追加新轮次。
- **CLI 验证结果**：Rust 逻辑由 `grok_rewind_with_attachments_is_rejected_and_keeps_messages`、`grok_text_rewind_restores_prompt_without_truncating_and_next_send_appends` 单元测试覆盖并通过；TUI 实际弹窗待目测。

### M6 `Build` 模式

- **输入内容**：`web chat --site grok --model Build "Reply with the single word OK."`。
- **验证步骤**：
  1. `./target/debug/lx-tui web chat --site grok --model Build "Reply with the single word OK."`
  2. 检查回答与正文提取。
- **期望结果**：回答 `OK`，正文不混入 `sr-only` 的 “Grok” 标题。
- **CLI 验证结果**：未执行；本次仅实测 `Fast`，`Build` 正文提取逻辑相同但未发消息。

---

## 5. 实测发现与修正

1. **正文提取**：助手消息头部有 `sr-only` 的 “Grok” 标题；正文未挂载时回退 `innerText` 会把 “Grok” 当回答。已改为只读 `.response-content-markdown`。
2. **登录误判**：页面出现普通 `sign-in` 链接不代表未登录；已移除“看到 sign-in 提前退出”，只信 `GET /rest/auth/get-user` 的 `email`。
3. **附件 chip 位置**：chip 容器在输入框 `<form>` 内，不在 `[data-testid="drop-ui"]` 内；选择器已改为 `form [role="list"][aria-label*="对话附件"]`。隐藏 input 轮询 10s，chip 等待 60s。
4. **Grok Edit 不是原地替换**：实测点击 Edit 提交后，Grok 清空旧用户消息正文并追加新用户消息 + 新回答；`/responses` 为线性追加。因此：
   - TUI `/rewind` 改为“重试追加”：保留旧轮次、回填输入、下一次发送追加；
   - 带附件轮次仍拒绝；
   - CLI `--edit-index` 对 Grok 降级为普通追加，输出 `editIndex: null`。
5. **会话标题**：`/responses` 不含标题；`load_session` 额外请求会话列表补标题。
6. **引用标签泄漏**：`/responses` 的 `message` 原样包含 `<grok:render card_id="…" ...><argument name="citation_id">N</argument></grok:render>`；直接在 `/resume` / `web show` 显示会看到 XML。已按同条 response 的 `cardAttachmentsJson` 把 `card_id` 映射为 URL，替换为 `[来源名](url)`；实时 DOM 采集也兜底剥离完整/未闭合标签。实测来源名为 `Chinatimes` / `Cctv` / `Bbc` / `Sina` 等域名标签。

---

## 6. 任务空间说明

正式任务空间 `lx-tui-ai-web-grok` 在第一次 `web chat` 时因旧版“sign-in 链接”误判触发 `handOffTaskSpace()`，控制权交给用户。按 ego-browser 规则未擅自夺回；修复登录判定后，用临时 `lx-tui-ai-web-grok-verify` 完成首轮端到端验证，引用标签修复使用临时 `inspect grok citations` 空间复验（验证后关闭），随后：

- 恢复 `grok_helpers.js` 的 `SPACE = 'lx-tui-ai-web-grok'`；
- 关闭临时任务空间；
- 正式任务空间仍由用户持有，可在 ego-browser 中释放或关闭；下次 lx 运行时会按现有 `reclaimSpaceTarget` 逻辑接管。

---

## 附录 A：DeepSeek / Gemini 同类检查

> 该附录为跨站点审计，不属于 Grok 功能项；DOM/历史原文通过 `ego-browser nodejs` CLI 读取，因此仍保留 CLI 验证记录。

### A1 DeepSeek 历史引用标记

- **输入内容**：DeepSeek 最新会话 `像素LX含义解析`；接口 `/api/v0/chat/history_messages`。
- **验证步骤**：
  1. 使用 `ego-browser nodejs` 打开 `https://chat.deepseek.com/`，读取 `localStorage.userToken`。
  2. 请求会话列表与目标会话历史。
  3. 扫描消息 `fragments[].content` 中的 `<...>` 标签与 `citation` / `reference` 标记。
- **期望结果**：无 XML 渲染标签；引用标记为 `[reference:N]` 形式，并已被 `deepseek_helpers.js` 的 `cleanText` 清除。
- **CLI 验证结果**：通过。原文含 `[reference:0]`，无 `<...>` 标签；`cleanText` 清洗后无引用标记。

### A2 DeepSeek 实时 DOM 引用残渣

- **输入内容**：同一会话页面；`.ds-markdown-cite` 引用角标。
- **验证步骤**：
  1. 使用 `ego-browser nodejs` 打开会话，读取最后一个 `.ds-markdown` 的 `innerText`。
  2. 临时隐藏 `.ds-markdown-cite` 后再次读取。
  3. 检查是否残留 `-\n13` 或 `[citation:N]`。
- **期望结果**：隐藏前有 `-\n13` 残渣；隐藏后正文干净。
- **CLI 验证结果**：通过。隐藏前确认 `-\n13`；隐藏后 `hasArtifact = false`、`hasCitation = false`。`deepseek_script.js` 的 `tickDom` 已加入 `readMarkdownText`，读正文前隐藏 `.ds-markdown-cite`；历史清洗正则同步放宽为 `[^\]]*`。

### A3 Gemini 历史/实时引用节点

- **输入内容**：Gemini 最新会话与带联网引用的 `近期 AI 行业动态速递`。
- **验证步骤**：
  1. 使用 `ego-browser nodejs` 打开 `https://gemini.google.com/app`，读取侧栏会话。
  2. 打开目标会话，隐藏 `source-footnote`、`sources-carousel-inline`、`sources-list`、`.cdk-visually-hidden`、按钮/图标。
  3. 读取 `message-content .markdown` 的 `innerText`，扫描 `<...>`、`citation`、`source` 标记。
- **期望结果**：来源名随隐藏节点一起消失；正文无标签、无引用占位符。
- **CLI 验证结果**：通过。`近期 AI 行业动态速递` 隐藏来源节点后只剩正文，`hasTag = false`、`hasCitation = false`；无需改动。
