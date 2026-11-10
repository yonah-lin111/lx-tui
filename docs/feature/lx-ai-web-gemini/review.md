# `Web`（`DeepSeek · Gemini`）特性代码 Review 与修复清单 (`review.md`)

- **特性目录**：`docs/feature/lx-ai-web-gemini/`
- **Review 基线**：`feat/lx-ai-web-gemini` @ `87a0c2ae`（相对 `dev` `995b45dc`）
- **Review 范围**：双站点状态机、`/model` 两级面板、欢迎页品牌渐变、Gemini/DeepSeek 网页驱动、事件层流协议、`lx-tui web` CLI、相关测试与文档
- **验证方式**：代码逐行审查 + `cargo test`/`clippy` + `lx-tui web` 对双站点真实网页端回归（见 [`verification.md`](verification.md)）
- **结论**：未发现 P0；发现 **7 个 P1、9 个 P2、7 个 P3**。P0/P1/P2 可执行项已全部修复并回归；P3 中死代码、文案、文档与命名已清理，结构性遗留项在文末单独记录。

---

## 0. 优先级汇总

| 级别 | 数量 | 判定标准 |
| --- | --- | --- |
| **P0** | 0 | 崩溃、数据丢失、安全问题、主流程完全不可用 |
| **P1** | 7 | 明确的功能错误、云端/本地状态分叉、用户可见的错误结果 |
| **P2** | 9 | 边界条件、并发、安全加固、可观测性与测试缺口 |
| **P3** | 7 | 死代码、重复逻辑、文案/注释/文档与命名一致性 |

---

## 1. P1（已修复）

### [P1-1] `src/app/update/chat/web.rs:370` — `/resume` 迟到结果未校验请求归属，可覆盖已切走的窗格

- **关键点**：`on_ai_web_session_loaded` 只按 `pane` 写状态，入口没有检查 `chat.loading_session`，也没有校验发起请求时的站点；同时会无条件把 `target.lx.ai_web_site` 改成结果里的站点。
- **错误原因**：用户在云端历史返回前切换入口（`Web → Native`）或站点（`DeepSeek → Gemini`），`set_lx_entry` / `set_ai_web_site` 已把 `loading_session` 清空并重置会话视图；迟到的 `SessionLoaded` 仍会重建消息、写入 `ai-web-*` 会话并把站点改回去。随后 Native 轮次会因 `session_id` 是 `ai-web-*` 被当成 Web 流直接返回，整轮不落库。
- **修复**：入口先判断 `!target.lx.chat.loading_session` 并直接丢弃；新增回归测试 `stale_ai_web_session_loaded_is_ignored_without_loading_flag`。

### [P1-2] `src/chat/web/gemini_script.js:338` — Gemini `/rewind` 忽略附件增删，且 turn 0 按 DOM 元素数误判“附件已变化”

- **关键点**：Gemini 编辑分支只填入文本，从未调用 `syncContainerAttachments(true)`（DeepSeek 在 `deepseek_script.js` 中有对应调用）；turn 0 的 `existingFiles` 统计的是 `.file-preview-container button`、`a`、`gem-media-attachment` 等**元素个数**，与 `FILES.length + KEEP_FILES.length` 的**文件个数**直接比较。
- **错误原因**：一个附件通常同时包含容器与内部按钮，`existingFiles = 2` 而 `wantedFiles = 1`，任何带附件的 turn 0 `/rewind` 都会命中 `attachments_changed_turn0` 并调用 `ensureFreshChat()`，用户只想改文字却静默放弃原云端会话；turn > 0 则新增附件不会上传、被删除的附件也不会从编辑框移除，本地气泡与云端永久不一致。
- **修复**：Gemini 的原地编辑只改文本，附件集合视为不可变。编辑前按 `user-query-file-preview` 的 `aria-label` 与 `FILES`/`KEEP_FILES` 的 basename 做**全名集合比对**：
  - 集合一致：保留原附件并只改文本；
  - 集合变化且 `EDIT_INDEX == 0`：回退到 `ensureFreshChat()`，用新云端会话承载新附件；
  - 集合变化且 `EDIT_INDEX > 0`：显式报错 `Gemini /rewind cannot change attachments of an earlier turn`，绝不再静默丢弃。
  同时删除无实际作用的 `syncContainerAttachments(true)` 调用，避免误删编辑框内附件。

### [P1-3] `src/chat/web/gemini_script.js:223` — Gemini `/rewind` 重置检测把旧回答整段当新增量重发，并可能永远等不到完成

- **关键点**：检测到 `announcerBusy || 内容变化` 后仅把 `editResetSeen` 置真，没有清空 `domText`/`domThink`；紧接着对仍指向旧 `model-response` 的目标执行 `pushDom`，把旧回答全量入队。`done` 判定要求 `bodyText.length >= domText.length`。
- **错误原因**：Angular 先置 `aria-busy="true"`、后替换响应节点时，旧答案会被重复推送；若新答案比旧答案短，`domText` 停留在旧长度，`caughtUp` 永远为假，最终 5 分钟后报 “Gemini stream timed out”。
- **修复**：检测到重置后清空采集游标、`started`、稳定采样状态并跳过本帧；新回答从空游标重新累积。

### [P1-4] `src/chat/web/deepseek_script.js:520` — DeepSeek `/rewind` 索引越界时静默编辑最后一条消息或退化为新消息

- **关键点**：`const target = userItems[EDIT_INDEX] || userItems[userItems.length - 1]`，越界不报错而是编辑最后一条；编辑按钮缺失时 `openedEdit=false` 会落入普通发送分支。
- **错误原因**：页面历史未完整加载、云端分支与本地索引错位时，第 N 轮会改写最后一条提问（云端分支被改坏，本地却以为改的是第 N 轮）；按钮找不到时会重复发送一条新消息。
- **修复**：返回 `{ status: 'missing_turn' | 'no_button' | 'clicked' }`，越界与按钮缺失均 `emit error` 并终止，不再回退到最后一条或普通发送。

### [P1-5] `src/cli/web.rs:341` — CLI 允许无会话的 `/rewind`、无编辑索引的 `--keep-file`、以及 `--first-turn` 复用旧会话

- **关键点**：`parse_chat` 只解析参数，不校验组合关系。
- **错误原因**：
  - `web chat --edit-index 5 "x"` 无 `--session` 时会直接编辑当前标签页里任意会话的第 5 轮；
  - `--keep-file` 在没有 `--edit-index` 时不会生效（普通发送会清空全部 chip），属于静默无操作；
  - `--session ai-web-ds-* --first-turn` 会调用 `ensureFreshChat()` 清空网页端当前会话，再以新会话冒充旧会话返回。
- **修复**：三条约束在解析阶段直接报错并写入 `web --help`；新增 CLI 单测。

### [P1-6] `src/event/chat/web.rs` — 云端 rename/delete 失败仍报告成功，本地与云端分叉

- **关键点**：DeepSeek helper 的 `res.ok` 只返回不检查；Gemini helper 在 `menuOpened === false`、找不到菜单项/确认按钮时仍 `emit done`；Rust 侧 `rename_session`/`delete_session` 用 `let _ = run_query(...)` 丢弃结果，而 TUI 已先本地改名/删除。
- **错误原因**：token 过期、侧栏未展开、菜单按钮选择器漂移时，云端标题/会话没变，但 TUI 已改本地标题/删除本地视图并提示成功；下次 `/resume` 旧标题或“已删除”的会话又会出现。CLI 也会错误地返回 `ok: true`。
- **修复**：两个驱动的 rename/delete 在每一步失败时 `emit error`；Rust 侧检查事件并回投 `ActionFailed`，失败时弹 toast；无云端会话 ID 的本地会话 rename/delete 直接提示“未关联云端会话”。实测：`web rename ai-web-gm-deadbeef` 与 `web rename ai-web-ds-deadbeef` 均非零退出并给出具体原因。

### [P1-7] `src/chat/web/gemini_helpers.js:166` — Gemini 模型/思考等级同步失败被静默吞掉

- **关键点**：`if (!menuOpened) return;`，`{ found: false }` 从不被检查，思考等级点击后也不验证是否生效。
- **错误原因**：模式菜单按钮选择器漂移或账号没有目标模型时，脚本不切换也不报错；用户已在 `/model` 选择 `4 Argon + high`，状态栏也这样显示，但网页仍用旧模型/旧等级生成，用户无法察觉。
- **修复**：菜单缺失、模型不在菜单、思考等级菜单缺失、目标思考等级项不存在均抛错；`main` 捕获后 `emit error` 并停止发送。模型通过“重开菜单检查 `aria-checked`”做后置校验；思考等级选择后菜单立即关闭，无法再读 `aria-checked`，因此以“目标项存在且点击成功”为准（实测 `3.8 Flash + medium`、`3.5 Flash-Lite + high`、`3.8 Flash + low` 切换均通过）。

---

## 2. P2（已修复 / 部分修复）

### [P2-1] `src/chat/web/gemini_helpers.js:359` — 上传抽屉已展开时仍会被再次点击关闭

- **关键点**：条件 `if (!expanded || now - lastClickAt > 2500)` 中 `lastClickAt` 初值为 0，第一次循环 `now - 0 > 2500` 恒真。
- **错误原因**：只要按钮 `aria-expanded === 'true'`，脚本就会再点一次把它关掉，随后 2.5 秒后再打开，形成开关抖动；Angular 懒加载 `input[type=file]` 期间可能直接超时报 `Gemini file upload input not found`。
- **修复**：仅在 `aria-expanded !== 'true'` 时点击，并用 `window.__lxUploadClickAt` 做 1.2 秒冷却。

### [P2-2] `src/chat/web/gemini_helpers.js:421` — 媒体与文档落到同一个 file input 时，第二次注入覆盖第一次

- **关键点**：媒体分支选 `input[accept*="image"] || input[type=file]`，文档分支选 `images-files-uploader input || input:not([accept*="image"]) || input[type=file]`；页面只有一个 input 时两者是同一节点。
- **错误原因**：连续两次 `DOM.setFileInputFiles` 会替换 FileList，先注入的图片被后注入的文档覆盖；`expectedCount` 仍按全部文件等待，30 秒后超时。
- **修复**：先探测 input 数量，只有一个 input 时把 media + document 合并为一次注入；多个 input 时才分组注入。

### [P2-3] `src/chat/web/gemini_helpers.js:313` — `querySelector('..., button')` 的裸 `button` 兜底可能点到错误控件

- **关键点**：`querySelector` 按文档序返回首个匹配，不按选择器书写顺序优先；附件 chip 的移除按钮与侧栏会话行的“更多选项”都带有裸 `button` 兜底。
- **错误原因**：chip 内若先渲染预览按钮，点击只会打开预览；会话行若先有置顶/收藏按钮，点击会置顶会话而不是打开菜单。脚本仍返回成功并 `emit done`，云端实际未变。
- **修复**：删除裸 `button` 兜底，只接受明确的 `aria-label` 与 `.gem-attachment-close-button button`。

### [P2-4] `src/ui/main_content/lx/brand.rs:215` — 窄窗格下 Gemini 被整行裁掉，无法显示也无法点击

- **关键点**：`place_ai_web_sites` 在 `total > rect.width` 时仍按未截断宽度推进 `x`，DeepSeek 固定占 8 列后 Gemini 的可用宽度为 0，被 `if clamped_w > 0` 丢弃。
- **错误原因**：窗格内容宽度 8–16 时首页只剩 `DeepSeek`，`ai_web_site_hit` 永远命中不到 `Gemini`；站点切换只有鼠标路径，用户无法切到 Gemini。
- **修复**：`natural > rect.width` 时按 `(width - 3*(n-1)) / n` 等分宽度并分配余数；新增 8/10/12/16 列宽回归测试，保证两个站点均可见、不重叠、可命中。

### [P2-5] `src/event/chat/web.rs:834` — 本地流监听器无鉴权、无大小上限、无读超时

- **关键点**：`TcpListener::bind("127.0.0.1:0")` 后接受任意连接，不校验路径；`Content-Length` 直接参与循环读取，没有上限；每连接任务没有读超时，`handle.abort()` 只能停止 accept 循环。
- **错误原因**：同机任意进程可扫描临时端口并 POST `@@LX_AI_WEB@@{"event":"text",...}`，把伪造文本注入当前回复；`Content-Length: 18446744073709551615` 或半开连接可让单连接无限增长内存。
- **修复**：URL 携带一次性 token（时间 + pid + 序号经 SHA-256 生成），只接受 `/<token>`；header 上限 16 KiB、body 上限 1 MiB、每次读 5 秒超时；新增两条监听器回归测试（错误 token 返回 404 且不注入、超限 body 不返回 200）。

### [P2-6] `src/event/chat/web.rs` / `src/cli/web/exec.rs` — `stdin.write_all` 无超时，可能在现有超时生效前永久挂起

- **关键点**：脚本约 50–120 KB，超过 pipe buffer 后 `write_all` 必须等子进程消费；30 秒 / 300 秒超时只包住 `wait()`，位于写入之后。
- **错误原因**：`ego-browser` 在 daemon 握手、浏览器拉起或版本提示阶段不读 stdin 时，任务会永久阻塞：slash 命令一直 loading、聊天窗格一直 streaming，CLI 也无法退出。
- **修复**：`run_chat`、`spawn_action`、`run_query` 与 CLI `exec_chat` 的写入统一包 `tokio::time::timeout(30s, ...)`，超时按失败路径清理监听器并返回错误。

### [P2-7] `src/event/chat/web.rs:773` — `run_query` 的 30 秒预算短于 `load_session` 内部等待，且丢弃真实 stderr 诊断

- **关键点**：Gemini `load_session` 先 `openOrReuseTab(..., timeout: 25)`，再最多等 10 秒渲染；Rust 侧总预算只有 30 秒。`run_query` 只解析协议行，普通 stderr 文本全部丢弃。
- **错误原因**：慢网络或首次打开标签页时 Rust 先超时杀进程，用户看到 `ego-browser is not installed or unavailable`；脚本自身的 SyntaxError/CDP 失败也无法定位。
- **修复**：`load_session` 预算提升到 60 秒；`run_query` 复用 `should_record_diag_line` 保存首条真实异常，在无事件时作为错误返回。

### [P2-8] `src/chat/web/deepseek_script.js:646` — DeepSeek 流采集器是全局单例且无 `runId`，双窗格会互相消费队列（部分修复）

- **关键点**：collector 挂在 `window.__lxDeepSeekStream`，两个窗格同时使用同一站点时第二个 run 覆盖第一个；两个 Rust 循环同时 `splice` 同一队列。
- **错误原因**：同一段文本随机落到其中一个窗格，`done`/`session` 也可能被先启动的 run 消费，把第二个 run 的云端会话 ID 绑定到第一个窗格。
- **修复（部分）**：DeepSeek collector 增加 `runId`，循环发现 ID 不匹配时立即 `emit error` 并结束，不再串台。**仍受共享标签页限制**：同一站点的两个窗格无法真正并发，第二个流会显式失败而不是静默错乱；彻底解决需要“每窗格独立标签页”或 App 层站点互斥，记录为后续项。

### [P2-9] `src/chat/web/tests.rs` / `src/event/chat/web.rs` — 注入边界与监听器零测试

- **关键点**：原测试只断言 `scripts_for_site` 的字符串包含关系，没有 `script_source` 转义测试，也没有流监听器的分帧/鉴权测试。
- **错误原因**：`script_source` 是 Rust→JS 的唯一注入边界，任何一次把 `serde_json::to_string` 换成手写拼接都不会被发现；监听器新增的 token/大小限制也无回归保护。
- **修复**：新增 `script_source_escapes_user_controlled_values`（引号、反引号、`${}`、换行、`</script>`、`files`/`keepFiles` 数组）与 `src/event/chat/web/tests.rs` 两条监听器测试。

---

## 3. P3（已修复）

| 编号 | 位置 | 关键点 / 错误原因 | 修复 |
| --- | --- | --- | --- |
| **P3-1** | `src/app/update/chat/web.rs:41`、`:62` | 状态栏与助手气泡硬编码 `" · "`，违反“用户可见文案集中在 `src/ui/text.rs`”，改分隔符会漏改 | 新增 `text::ai_web_model_label(site, model)`，统一使用 `LX_CHAT_MODEL_SEP` |
| **P3-2** | `src/ui/style/lx.rs:212`、`:326` | 拆分后仍保留无调用方的 `lx_entry_border`、`lx_input_border`；`design.md` 明确要求删除前者 | 删除两个函数与不再使用的 `accent` 导入 |
| **P3-3** | `src/app/update/chat/web.rs:21` | `AI_WEB_REMOTE_SESSION_PREFIX` 仅定义与再导出、无任何调用方，且语义固定为 DeepSeek 前缀，容易误导 Gemini 路由 | 删除常量与再导出 |
| **P3-4** | `src/event/chat/web.rs` | `received` 只赋值不使用，`_keep_tx` 与任务自身持有的 sender 重复；JS 侧 `gemini_helpers.js` 的 `SIGN_IN`、`deepseek_helpers.js` 的 `MODEL` 未使用 | 全部删除 |
| **P3-5** | `src/app/update/chat/slash.rs:658`、`src/chat/web/*.js` 头部 | 注释仍写“Web 只暴露 `/new`”，JS 头部仍指向已重命名的 `src/event/chat/ai_web.rs` | 同步为当前白名单与 `web.rs` |
| **P3-6** | `src/app/update/chat/web.rs:220` | 手写 `SystemTime::now()` 重复实现 `crate::db::now_ms()` | 改用 `crate::db::now_ms()` |
| **P3-7** | `src/cli/web/exec.rs`、`src/ui/text.rs` | `exec_show` 用 `let _ = session_id` 吞参数；`LX_AI_WEB_ONLY_NEW_CMD` 名称与“5 个命令”不符 | `session_id` 作为返回 ID 为空时的兜底；常量重命名 `LX_WEB_ALLOWED_CMDS` |

---

## 4. 未修复项与原因（已记录）

| 编号 | 位置 | 问题 | 不修原因 / 后续建议 |
| --- | --- | --- | --- |
| **D-1** | `src/chat/web/gemini_helpers.js:506` | `list_sessions` 用 `now - idx * 60000` 伪造 `updated_at_ms`，所有 Gemini 会话显示为“just now / 1m ago / 2m ago” | Gemini 侧边栏 DOM 不提供真实时间；伪造值至少保留了“最近优先”的排序。后续可改为 `Option<i64>` + UI 空时间，或从侧栏隐藏节点/接口取真实时间 |
| **D-2** | `src/event/chat/web.rs` + `src/cli/web/exec.rs` | CLI `exec_chat` 与 TUI `run_chat` 各自维护一份 spawn/stdin/listener/stdout/Done 编排 | 抽公共 `chat::web::driver` 需要同时改造取消、流监听与事件回投，超出本轮 review 修复范围；当前只对两份实现同步修了超时/监听器 |
| **D-3** | `src/app/update/chat/web.rs:164` | `try_send_ai_web` 调用 `prepare_ai_web_message`，在 app 层执行 `fs::metadata` / `canonicalize` / `sniff_image_file` | 基线 `995b45dc` 已存在，属于既有架构债；下沉到 `event/` 需要把附件预检改为异步并新增失败回投通道 |
| **D-4** | `src/ui/main_content/lx/tests.rs` | 测试文件 2295 行，超过项目 1500 行上限 | 基线已 2251 行；本次新增的 AI Web 测试仅约 100 行，整体拆分应作为独立测试整理任务，避免与本特性混杂 |
| **D-5** | `src/ui/style/lx.rs` | 拆出子模块后相关测试仍留在 `src/ui/style/tests.rs`，未随生产模块下沉 | 测试覆盖的是样式函数行为，迁移不影响功能；按“测试随模块拆分”规范单独立项处理 |
| **D-6** | `src/app/state/lx/web.rs` / 共享标签页 | 同一站点双窗格仍共享一个 `ego-browser` 标签页，无法真正并发（P2-8 已让第二个流显式失败） | 需要每窗格独立任务空间/标签页或 App 层站点互斥 + UI 提示；属于交互设计变更，需单独确认 |

---

## 5. 修复后的验证

```text
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test
→ cargo test: 4901 passed, 4 ignored (16 suites)
```

真实网页端回归（完整记录见 `verification.md`）：

- `DeepSeek`：`deepthink/off` 对话、`/rewind` 有效索引成功、越界索引返回 `DeepSeek /rewind target turn not found`、rename/delete 成功。
- `Gemini`：`3.8 Flash + medium`、`3.5 Flash-Lite + high` 切换成功；文件上传返回 `GM_FILE_9900`；`/rewind` 保留附件返回 `EDITED_GM_9900`；第 1 轮 `/rewind` 无超时；更早轮次新增附件被显式拒绝；rename/delete 成功。
- 失败路径：不存在的 Gemini 会话 rename 返回 `Gemini session not found in sidebar`；不存在的 DeepSeek 会话 rename 返回 `DeepSeek rename failed`；CLI 参数组合错误在本地即被拦截。

---

## 6. Web Markdown 保真提取与流式渲染专项 Review（按 P0 → P3 排序）

> **审查范围**：Gemini / DeepSeek 共享 DOM → Markdown 序列化器（`src/chat/web/dom_markdown.js`）、`src/chat/web/gemini_script.js`、`src/chat/web/gemini_helpers.js`、`src/chat/web/deepseek_script.js`、`src/chat/web/deepseek_helpers.js`，以及 TUI 面向阅读的 Markdown 渲染器（`src/ui/markdown_render.rs`、`src/ui/markdown_render/inline.rs`、`src/ui/markdown_render/code_block.rs`）。
> **参考规范**：[`docs/web/web-gemini.md`](../../web/web-gemini.md) · [`docs/web/web-deepseek.md`](../../web/web-deepseek.md)

### 6.1 P0（已修复）

#### [P0-1] `src/chat/web/gemini_script.js:252-266`、`src/chat/web/deepseek_script.js:126-140, 661-664` — 正文流式输出期间思考块仍按 `streaming: true` 序列化，暂缓的闭合 `**` / ```` ``` ```` 晚于 `text` 吐出并分裂出第二个 `○ Thought` 块

- **关键点**：
  - `gemini_script.js` 的 `pollDom()` 原先对 `<model-thoughts>` 与 `<message-content>` 同时传入 `{ streaming: isGenerating }`；`deepseek_script.js` 的 `tickDom()` 对 `thinkEl` 同样传入 `isStreaming`。
  - 当思考块以粗体（`**总结**`）、行内代码（`` `code` ``）或围栏代码块（```` ``` ````）结尾时，`lxHtmlToMarkdown` 的 `streaming && isTail(el)` 会暂缓输出最末尾的右闭合标记，直到 `streaming` 变为 `false`。
- **错误原因**：
  - 模型进入正文输出阶段后，网页仍在生成（`isGenerating === true`），但思考块早已完成。由于思考块继续传 `streaming: true`，其末尾闭合 `**` 一直被扣留到整轮回答彻底停止（`isGenerating === false`）才作为新的 `thinking` delta 发出。
  - Rust 侧在已接收 `text` 增量后再收到迟到的 `thinking` 增量（`"**"`），会在正文下方追加第二个 `PartKind::Reasoning`，导致界面底部多出一个空白或只有标点的 `○ Thought · 0s`。
- **影响/复现**：
  - Gemini（如 `3.8 Flash + high`）或 DeepSeek（DOM 兜底）思考块末句含粗体或代码块时，回答完成后正文下方会多出一个孤立的 `○ Thought` 块。
- **修复（已修复）**：
  - 在 `gemini_script.js:258` 先提取 `bodyText`，将思考块的流式标志改为 `const thinkStreaming = isGenerating && !bodyText && !this.domText`（`deepseek_script.js:662` 为 `isStreaming && !bodyText && !this.domTextStarted`）：一旦正文非空，立即以 `streaming: false` 序列化思考块，在第一个 `text` delta 入队前刷出思考块末尾闭合标记；
  - 在 `gemini_script.js:103`（`if (kind === 'thinking' && this.domText) return;`）与 `deepseek_script.js:129, 139` 增加硬门禁，拒绝在正文启动后追加任何 `thinking` 事件。

#### [P0-3] `src/chat/web/deepseek_script.js:564-608, 648-650` — DeepThink 思考超过 1.2s 时 `tickDom()` 越权启动 DOM 通道 & `readFiberMarkdown` 无宿主边界爬树污染

- **关键点**：
  - `deepseek_script.js` 原先在 `tickDom()` 中仅当 `this.sseText` 为真时才跳过 DOM 采集，且 `graceOver` 从 `submittedAt` 起算 1.2s；`readFiberMarkdown(el)` 沿着 `fiber.return` 向上爬 20 层且不检查 `fiber.stateNode` 是否越出 `el`。
- **错误原因**：
  - DeepThink 思考通常持续数秒（`sseThinking = true, sseText = false`），1.2s 后 `tickDom` 误判 SSE 失效并置位 `domTextStarted = true`，反向锁死后续真正的 SSE 正文；Fiber 向上 20 层会爬到外层消息容器，拿到非当前块的 `content`。
- **影响/复现**：
  - DeepSeek 开启 `deepthink` 且思考超过 1.2s 时，DOM 通道抢跑；完成态读取 Fiber 时可能混入外层组件文本。
- **修复（已修复）**：
  - `tickDom()` 入口（`deepseek_script.js:598`）改为 `if (this.sseThinking || this.sseText)` 严格互斥，并记录 `sseSeenAt` 计算宽限期；
  - `readFiberMarkdown(el)`（`deepseek_script.js:564-591`）收紧为 `depth < 10`，校验 `el.contains(fiber.stateNode)` 边界与 `citeProps`/`cacheKey`/`mergeConsecutiveCites` 特征属性；新增回归测试 `deepseek_stream_collector_survives_non_sse_xhr_and_preserves_markdown_fences`。

---

### 6.2 P1（已修复）

#### [P1-1] `src/chat/web/dom_markdown.js:486-513, 563-570` — `<li>` 首块为代码块/块级结构时围栏倒置、列表内 `<h3>` 泄漏顶层标题、以及列表内代码正文被多加缩进

- **关键点**：
  - `serializeListItem` 原先将第 0 个块级分段的第一行直接拼在 `- ` 之后（产生 ```` - ```rust ````），并对围栏代码块的所有内部代码行都加上 `contIndent` 空格；`<li>` 内的 `<h3>` 直接输出 `### 标题`。
- **错误原因**：
  - 未识别必须独占一行的块级结构（围栏代码块、块级公式、表格、引用），未区分围栏开闭行与代码正文行的缩进语义，且在 `inListItem` 中未降级标题标签。
- **影响/复现**：
  - Gemini 回答常在 `<li>` 内嵌套 `<h3>` 或 `<code-block>`，导致列表断裂、代码块围栏倒置或代码每行多出 2 列缩进。
- **修复（已修复）**：
  - `serializeListItem`（`dom_markdown.js:488-509`）对 `requiresOwnLine` 首段先换行，对 `isFenceBlock` 仅在首尾围栏行加 `contIndent`；`serializeBlock`（`dom_markdown.js:563-566`）在 `ctx.inListItem` 下将 `h1`–`h6` 转为 `**...**` 粗体。

#### [P1-2] `src/ui/markdown_render/inline.rs:10-231` — 强调块内部未递归解析行内代码/链接/内层强调、不支持双反引号 `` ``code`` ``、且尾随 `***` 闭合错位

- **关键点**：
  - `inline_spans` 原先对 `**...**` / `*...*` / `***...***` / `~~...~~` 内部不再递归解析，不支持 `` ``code ` inner`` ``，且 `delimited_emphasis` 未跳过反引号代码段、在 `**bold *italic***` 末尾前两个 `*` 处提前闭合。
- **错误原因**：
  - 单层线性扫描缺少递归样式合成（`push_nested_spans`）、多反引号配对（`code_span`）与尾随同字符链对齐（`end = i + run - delim_len`）。
- **影响/复现**：
  - `**修改 \`config.toml\`**`、`**[链接](url)**`、`**bold *italic***` 在 TUI 中泄漏原始反引号、方括号或 `*`。
- **修复（已修复）**：
  - 新增 `push_nested_spans` 与 `code_span`（`inline.rs:99-161`），更新 `delimited_emphasis`（`inline.rs:167-231`）跳过代码片段并对齐尾随闭合链；新增测试 `nested_inline_code_links_and_emphasis_render_without_leaking_delimiters`。

#### [P1-3] `src/ui/markdown_render.rs:188-189, 338-345, 526-530, 605-629` & `src/ui/markdown_render/code_block.rs:124-131` — 有序/任务列表续段缩进错位、子列表回退清空整栈、以及列表内围栏代码复制残留缩进

- **关键点**：
  - 列表续段缩进原先写死为 `indent(depth) + 2`；`update_list_context` 在内层子列表结束后直接 `clear()` 整栈；`code_block.rs` 未剥离列表内围栏的前导空格。
- **错误原因**：
  - `ParserState` 缺少记录每层标记悬挂宽度的 `list_hangs` 栈；层级回退未逐层 `pop()`。
- **影响/复现**：
  - 有序列表 `1. `（3 列）与任务列表 `[x] `（4 列）下的多段续行无法与首行正文对齐；子列表后的外层续段掉回第 0 列。
- **修复（已修复）**：
  - 新增 `ListItem::hang_width` 与 `ParserState.list_hangs`，续段按 `indent(depth).saturating_add(hang)` 对齐；`update_list_context` 改为 `while indent < content` 逐层 `pop()`；`code_block.rs` 按 `fence_indent` 剥离前导空格；新增测试 `ordered_task_list_continuations_and_indented_fences_keep_exact_columns`。

#### [P1-4] `src/chat/web/deepseek_script.js:93-125, 422-436` & `src/chat/web/deepseek_helpers.js:390-405` — SSE 结束漏刷末尾暂缓的 `[` & 历史会话清洗破坏代码块与换行

- **关键点**：
  - 流式期间被 `holdIncomplete = true` 暂缓的末尾 `[` 在 SSE 结束时未补刷；`deepseek_helpers.js` 的 `load_session` 用 `\s*\[(?:citation|reference):[^\]]*\]` 清洗全文，误删代码块内字面量并吞掉前导 `\n`。
- **错误原因**：
  - 缺少 `finishSse()` 尾刷步骤；历史清洗未按 ```` ``` ```` 围栏分段且使用了包含 `\n` 的 `\s*`。
- **影响/复现**：
  - 回答末尾 `[` 丢失；历史代码块中的 `[citation:1]` 被删且段落换行被吞。
- **修复（已修复）**：
  - 新增 `finishSse()`（`deepseek_script.js:422-436`）以 `cleanText(raw, false)` 补刷尾部；`deepseek_helpers.js:390-405` 改为围栏分段清洗且仅匹配 `[ \t]*`。

---

### 6.3 P2 & P3（已修复）

#### [P2-1] `src/chat/web/dom_markdown.js:110-143, 233` & `src/chat/web/gemini_script.js:178-190, 259-264` — `hasMeaningfulContent` 图片/空公式判定、`markdown` 代码块四反引号围栏与 Gemini 子选择器顺序

- **关键点**：
  - `hasMeaningfulContent` 补齐 `<img src="https://...">`、非空 KaTeX 与非空 `<pre>` 判定；`serializeCodeBlock` 对 `markdown`/`md`/`mdx` 固定使用 4 反引号围栏；`gemini_script.js` 的 `extractCleanText` 按逗号拆分 `selectorList` 逐一查询，并将思考块选择器设为 `'model-thoughts .thoughts-content, thinking-overlay .thoughts-content, model-thoughts, thinking-overlay'`（优先命中子容器而非外层 `<model-thoughts>`）。
- **错误原因**：
  - 原生 `root.querySelector('a, b')` 按 DOM 前序返回外层祖先节点；`markdown` 代码块流中遇到内层 ```` ``` ```` 会中途跳变围栏长度。
- **影响/复现**：
  - Gemini 思考块可能选到外层 `<model-thoughts>` 包含头部冗余节点；生成 Markdown 代码示例时流式前缀断裂。
- **修复（已修复）**：
  - 已在 `dom_markdown.js:110-143, 233` 与 `gemini_script.js:178-190, 259-264` 修复，并由 `dom_markdown_serializer_converts_deepseek_gemini_grok_html_and_preserves_streaming_prefix` 覆盖验证。

#### [P2-2] `src/chat/web/deepseek_script.js:598-608` & `src/chat/web/gemini_script.js:207, 229` — DeepSeek SSE 断流无 `finish` 帧兜底完结 & Gemini `/rewind` 重置检测统一 `isGenerating`

- **关键点**：
  - DeepSeek `tickDom()` 在 `sseThinking || sseText` 分支中检测 `idleButton && !stopIcon` 满 15 ticks 自动调用 `finishSse()`；Gemini `pollDom()` 的 `/rewind` 重置检测将 `announcerBusy` 替换为 `isGenerating = Boolean(stopBtn) || announcerBusy`。
- **错误原因**：
  - SSE 尾帧丢失时缺少按钮空闲兜底；Angular 可能先渲染停止按钮再置 `aria-busy="true"`。
- **影响/复现**：
  - DeepSeek 偶发尾帧丢失时卡满 300s；Gemini `/rewind` 偶发晚一帧识别重置。
- **修复（已修复）**：
  - 已在 `deepseek_script.js:598-608` 与 `gemini_script.js:207, 229` 修复。

#### [P3-1] `src/chat/web/deepseek_script.js:161, 178` & `src/chat/web/gemini_script.js:128, 152` — `pushDom` 尾部锚点窗口（`240` → `1024`）与 LCP 窗口（`40` → `160`）扩充

- **关键点 & 错误原因**：
  - 单帧大表格挂载或长链接文本替换可能超过 240 / 40 字符窗口，导致单帧漏发增量。
- **影响/复现 & 修复（已修复）**：
  - 将 `pushDom` 尾部锚点窗口扩至 `1024`、LCP 闭合窗口扩至 `160`。
