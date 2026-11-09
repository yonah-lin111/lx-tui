# Grok 接入 Web 入口 — Review 与修复清单 (`review.md`)

- **特性目录**：`docs/feature/lx-ai-web-grok/`
- **Review 基线**：`feat/lx-ai-web-grok` @ `84745ee1`（相对 `dev` `d6e8bada`）
- **修复提交**：`faff8426`（code）、`aa039534` / `a7cbf620`（review/docs）
- **合并状态**：已合并至 `dev`，临时 worktree `.worktrees/lx-ai-web-grok` 与分支 `feat/lx-ai-web-grok` 已移除
- **Review 范围**：Grok 站点枚举/状态机、`/model`、`/rewind`、CLI、首页品牌渐变、Grok 浏览器驱动、历史引用清洗、DeepSeek 引用残渣修复、跨站点回归、测试与三份特性文档
- **验证方式**：三路只读代码审查（Rust / Grok JS / 跨站点与文档）+ `cargo test`/`clippy`/`fmt` + 六个 JS 文件 `node --check` + ego-browser DOM 抽查
- **结论**：未发现 P0；发现 **5 个 P1、7 个 P2、14 个 P3**。P1/P2 全部修复；P3 修复文档、布局、渐变、CLI 与清洗类问题，结构性遗留单列。

---

## 0. 优先级汇总

| 级别 | 数量 | 判定标准 |
| --- | --- | --- |
| **P0** | 0 | 崩溃、数据丢失、安全问题、主流程完全不可用 |
| **P1** | 5 | 用户可见的错误结果、数据/草稿泄漏、回答截断或旧回答重放 |
| **P2** | 7 | 边界条件、超时/状态竞争、选择器与文档不一致、测试缺口 |
| **P3** | 14 | 视觉/文案/CLI 输出、代码健壮性、测试组织与文档漂移 |

---

## 1. P1（已修复）

### [P1-1] `src/chat/web/grok_helpers.js:296-370` — 附件清理最多 5 个，残留 chip 会被当成本轮附件发送

- **关键点**：清理循环固定 `for (round = 0; round < 5; round += 1)`，每轮只移除一个 `[role="listitem"]`；上传等待只判断 `chips >= FILES.length`，统计的是输入区全部 chip，不是本轮新注入的文件。
- **错误原因**：用固定轮数代替“清空为止”，且没有校验最终 `chips === 0`；上传完成后也没有校验 `chips === FILES.length`。
- **影响/复现**：Grok 输入框先手动挂 6 个文件，再从 lx 发送纯文本提示词，第 6 个文件仍会随消息提交；清理失败时旧文件会和新文件一起发送，属于跨轮数据泄漏。
- **修复**：按 10s 截止时间循环删到 0 个 chip，超时或仍残留则 `emit error`；上传后要求 chip 数等于 `FILES.length`，并保留 `input.files` 计数用于超时诊断。

### [P1-2] `src/chat/web/grok_script.js:240-281` — 发送成功没有确认，空 prompt 不清空编辑器，可能发送用户旧草稿

- **关键点**：`execCommand('insertText', false, PROMPT)` 在 `PROMPT === ""` 时不会清空编辑器；`filled` 只看 editor 是否存在，`clickGrokSubmit()` 只看按钮是否可点，从不校验提交后是否出现新 user/assistant 消息或停止按钮。
- **错误原因**：把“点击已执行”当成“消息已提交”，没有发送前后 DOM 状态对比；空 prompt 没有走 `delete` 分支。
- **影响/复现**：用户只发附件不写正文时，共享 Grok 标签页里的旧草稿可能被一起提交；点击被弹层/焦点吞掉时会空等 300s。
- **修复**：空 prompt 用 `selectAll + delete` 清空；写入后校验 editor 文本；点击后 5s 内要求 user/assistant 数量增加或出现有效停止按钮，否则立即报错。

### [P1-3] `src/chat/web/grok_script.js:187-212` — 完成判定只看停止按钮，600ms 文本稳定就结束

- **关键点**：`generating` 仅按任意停止按钮存在判断，没有过滤 disabled；完成条件为“停止按钮消失 + 10 tick（约 600ms）文本稳定”，没有设计要求的 `chat-submit`/响应完成态校验。
- **错误原因**：Grok 的停止按钮可能在工具调用、Build 或慢网络阶段短暂消失；600ms 的静默窗口不足以覆盖 Markdown/引用重排。
- **影响/复现**：回答在搜索/Build 中途被截断并立即 `done`，本地完成而网页仍在生成。
- **修复**：停止按钮过滤 `disabled`/`aria-disabled=true`；稳定窗口提升到 30 tick（约 1.8s）；新增“助手消息已出现但长时间无正文”的显式错误（见 P2-1）。

### [P1-4] `src/chat/web/grok_script.js:129-130, 187-198` — 固定 `targetIndex`，历史晚挂载时重放旧回答

- **关键点**：采集器安装时把 `targetIndex` 固定为发送前 assistant 数量；之后始终读 `messages[targetIndex]`，不验证该节点是否本轮新增。
- **错误原因**：`waitGrokHistoryReady()` 只等 8s 稳定，首轮分支完全跳过；长会话/虚拟列表晚挂载会把旧回答放到该下标。
- **影响/复现**：冷启动加载长会话后发送新消息，`pollDom` 把上一轮回答整段当新增量推送，真正的新回答被忽略。
- **修复**：采集器记录发送前 assistant 基线数；发送前 `messages.length <= baseline` 不读，出现新 assistant 后固定取最后一个 assistant 节点（Grok 新回答始终追加在末尾）。

### [P1-5] `src/chat/web/grok_helpers.js:203-218` + `grok_script.js:63-74` — 登录检查把 5xx/网络异常当成未登录并移交任务空间

- **关键点**：`grokLoggedIn()` 只返回布尔值；403/5xx、Cloudflare、网络异常、JSON 解析失败全部变成 `{ok:false}`，20s 后无条件 `handOffTaskSpace()` + `login_required`。
- **错误原因**：登录判定丢失了 HTTP 状态，无法区分“匿名/401”和“接口暂时不可用”。
- **影响/复现**：已登录用户在接口瞬断时被提示重新登录，任务空间控制权被交给用户，本轮直接失败。
- **修复**：返回 `{ok, status, anonymous}`；仅 401 或 200 且无 email 时 `login_required + handOffTaskSpace`；其它状态/网络错误 `emit error` 且不移交控制权。

---

## 2. P2（已修复）

### [P2-1] `src/chat/web/grok_script.js:197-212, 359-360` — 无正文响应永远不 `done`，空等 300s

- **关键点**：`done` 只在 `text.length > 0` 时置位；助手消息存在但没有 `.response-content-markdown`（错误横幅、纯附件回答、Build 画布、限流提示）时，采集器不会报错。
- **错误原因**：完成判定绑定正文文本，没有“助手节点已出现但无正文”的失败路径。
- **影响/复现**：Build 或限流场景 TUI 卡 5 分钟后报 `Grok stream timed out`，而不是即时错误。
- **修复**：助手节点出现后 15s 仍无正文且无有效停止按钮时，采集器写 `error` 并 `done`；15s 是文件上传/慢网络下空占位节点的安全窗口（实测附件回答约 6s 挂载正文）。

### [P2-2] `src/ui/main_content/lx/brand.rs:219-253` — 8 列丢 Grok，10 列 Gemini/Grok 都显示单字母 `G`

- **关键点**：`SITE_GAP` 固定 3；8 列时 `available = 8 - 6 = 2`，三等分后 `[1,1,1]`，总宽 9，Grok 被裁掉；10 列时 `[2,1,1]`，渲染为 `De · G · G`。
- **错误原因**：窄窗格只做了“扣固定间隔再均分”，没有压缩站点间距；新增测试把宽度 8 从用例中删掉，掩盖了回归。
- **影响/复现**：内容宽 8 的分屏中 Grok 不可见也不可点击；宽 10 时两个 `G` 无法区分。
- **修复**：`natural > rect.width` 时间距降为 1；宽 8 得到 `[2,2,2]`，宽 10 得到 `[3,3,2]`；测试恢复 8/9/10/12/16 并断言渲染文本。

### [P2-3] `src/chat/web/grok_script.js:76-85` — 非 chat 动作被 composer 等待阻塞，可能超过 Rust 30s 预算

- **关键点**：`ensureGrokComposer()` 位于 `ACTION !== 'chat'` 分派之前，`new/list/load/rename/delete` 只用 REST/页面导航，却先等最多 20s 的 ProseMirror；叠加打开标签页 25s、登录 20s，`run_query` 的 30s 预算可能耗尽。
- **错误原因**：冷启动等待放在动作分流之前；`run_query` 只给 `load_session` 60s。
- **影响/复现**：关闭所有 Grok 标签页后执行 `web list`/`web rename`，用户可能看到误导性的 `ego-browser is not installed or unavailable`。
- **修复**：composer 等待移入 chat 分支；Grok 的 `run_query` 预算统一提升到 60s。

### [P2-4] `src/chat/web/grok_helpers.js:71-87` — 未闭合 `<grok:render>` 残留 `<argument>`，自闭合引用丢链接

- **关键点**：完整标签替换后只清 `<grok:...>`，不处理 `<argument>`；自闭合 `<grok:render card_id="..."/>` 无条件删除，不查 card 映射。
- **错误原因**：清理规则只覆盖完整标签；缺少 `<argument>` 与自闭合映射分支。
- **影响/复现**：截断会话在 `/resume`/`web show` 显示 `<argument name="citation_id">1</argument>`；自闭合引用明明有 URL 却不生成来源链接。
- **修复**：自闭合标签走同一 `card_id` 映射；追加 `<argument>` 清理；URL 仅允许 http(s) 并转义 `(`/`)`/空白。

### [P2-5] `src/chat/web/grok_helpers.js:233-284` — 模型菜单只等按钮，菜单项一次查询就硬失败

- **关键点**：15s 轮询只等模型按钮出现；点击后固定 sleep 350ms，再单次查询 `[role="menuitemradio"]`；按钮选择器是文档级 `aria-label^="型号"/"Model"`。
- **错误原因**：菜单项没有纳入同一个 deadline；没有先检查当前按钮是否已是目标模型。
- **影响/复现**：冷启动/慢机器上偶发 `Grok model "Fast" not found`；页面其它同名按钮可能被点中。
- **修复**：先读当前按钮标签，已是目标值则直接返回；菜单项在 15s deadline 内轮询，按钮优先限定 composer form。

### [P2-6] `src/chat/web/grok_script.js:288-301` — 采集器丢失/被覆盖被吞成空 batch，静默空等

- **关键点**：`js()` 失败统一 `.catch(() => ({items:[],done:false}))`；`batch.superseded` 直接 `return`，不 `emit error`。
- **错误原因**：没有连续失败计数，也没有把 superseded 转成显式错误。
- **影响/复现**：生成中页面刷新或同站点双窗格时，TUI 空转 5 分钟，最后显示与真实原因无关的通用错误。
- **修复**：连续 10 次 `js` 失败 `emit error`；`superseded` 立即报 `Grok stream was superseded by another pane`。

### [P2-7] `src/chat/web/grok_helpers.js:439-442, 533-536, 589-592, 622-625` — 非 chat 动作 401 不移交任务空间

- **关键点**：`list/load/rename/delete` 的 401 只 `emit login_required`，没有 `handOffTaskSpace()`。
- **错误原因**：登录预检不在这些 REST 动作中；会话中途过期时控制权仍在 agent。
- **影响/复现**：用户看到登录提示但无法直接接管浏览器完成登录。
- **修复**：抽出 `emitLoginRequired()`，统一先 `handOffTaskSpace()` 再 `emit login_required`。

---

## 3. P3（已修复 / 部分记录）

| 编号 | 位置 | 关键点 / 错误原因 | 修复 |
| --- | --- | --- | --- |
| **P3-1** | `src/ui/style/lx.rs:251, 276-320` | Grok 端点 `255/250` 在 232..=255 灰阶，`cube_rgb` 只支持 16..=231，插值退化为硬切换 | `lerp_indexed` 对两端都在 232..=255 时按灰阶序号插值；补中点断言 |
| **P3-2** | `src/app/update/chat/web.rs:319-333` | Grok 带附件 `/rewind` 拒绝分支只清 `slash`，未清 `rewind.armed_at` | 拒绝分支补 `clear_rewind()`；补双 Esc 回归 |
| **P3-3** | `src/cli/web/exec.rs:662-670` | Grok `thinking` 为空仍拼接，成功 footer 出现 `Fast · ]` | 仅在 `thinking` 非空时追加 `sep + thinking` |
| **P3-4** | `src/cli/web.rs:360-365`、`exec.rs:468-509` | Grok 附件检查只看 `--file/--keep-file`，内联 `@[refer-file]` 可绕过，且早于结构校验 | 删除 parse 阶段检查，移到 `prepare` 之后按 `prepared.files/keep_files` 判定 |
| **P3-5** | `src/cli/help.rs:416` | help 先说支持 inline-edit，再说 Grok 是 append retry，自相矛盾 | 改成“rewind/send”，Grok 单独说明追加 |
| **P3-6** | `src/chat/web/deepseek_script.js:445-459` | 引用节点 `display:none` 后无 `try/finally`；DOM 文本未再套 `cleanText` | `try/finally` 恢复；返回前 `this.cleanText(out, false)` |
| **P3-7** | `src/chat/web/grok_helpers.js:74-84` | 引用 URL 原样拼进 Markdown，`Wikipedia_(bar)` 会在第一个 `)` 截断 | http(s) 校验 + `(`/`)`/空白百分号编码 |
| **P3-8** | `src/chat/web/grok_helpers.js:548` | 历史路径对 user 消息也清洗 `<grok:render>`，用户提问会被删 | 只对 `role === "assistant"` 清洗 |
| **P3-9** | `src/chat/web/grok_script.js:55` | `startsWith('https://grok.com')` 接受 `grok.com.evil.com` | 解析 hostname，仅接受 `grok.com` |
| **P3-10** | `src/chat/web/grok_script.js:322-344` | 完成后 session meta 只等 1.8s，慢网络可能不发出 `session` | 等待窗口提升到 5s |
| **P3-11** | `src/chat/web/grok_script.js:221-259` | `EDIT_INDEX` 兼容分支已无调用方，且不清理附件/不校验越界 | 保留兼容但补附件清理与越界显式错误（不重新启用） |
| **P3-12** | `src/chat/web/tests.rs:105-140` | 只有 bundle 子串断言；`replaceGrokCitations`、Grok `script_source` 转义无行为测试 | 新增 static/behavior 断言与 citation 场景检查 |
| **P3-13** | `src/ui/main_content/lx/tests.rs` | 测试文件 2375 行，超过项目 1500 行上限；新增 AI Web 测试仍堆在根文件 | **未修**：基线已超限，整体拆分独立任务；本次至少补宽度回归 |
| **P3-14** | `docs/feature/lx-ai-web-grok/{design,task,verification}.md` | 文档与实现漂移：登录/composer 顺序、cookie 选择器、附件清理、完成判定、布局算法、`\n` 字面量、task 全未勾选、verification 声称完成但 M3/M4/M6 未执行 | 按实际实现更新 design/task/verification，`\n` 改真实换行，勾选已完成项并标注未执行项 |

---

## 4. 未修复项与原因

| 编号 | 位置 | 问题 | 不修原因 / 后续建议 |
| --- | --- | --- | --- |
| **D-1** | `src/chat/web/grok_helpers.js:418, 512` | 会话列表/标题补全固定 `pageSize=60`，旧会话不可见、title 可能 null | 需要翻页/cursor 或按 id 查询；本轮只修当前可见会话 |
| **D-2** | `src/chat/web/grok_helpers.js:470-480` | `/responses` 父链缺失 parent 时静默截断，默认取数组最后一项 | 需要分页/活跃分支字段；Grok API 文档不足，先记录 |
| **D-3** | `src/chat/web/grok_script.js:221-259` | `EDIT_INDEX` 分支已降级为死代码，附件/越界未完整覆盖 | 本轮补显式越界与附件清理，不恢复 TUI/CLI 调用 |
| **D-4** | `src/ui/main_content/lx/tests.rs` | 测试文件 2375 行，超过 1500 行上限 | 基线 2251 行已超限，整体迁移到 `tests/` 子模块需独立整理 |
| **D-5** | `src/chat/web/grok_script.js:121-126` | 实时兜底会删除未闭合 `<grok:render` 到文本末尾，代码块中的字面量也可能被删 | 需要 DOM 节点级识别而非正则；先保留兜底，限制为 assistant 文本 |
| **D-6** | `src/chat/web/deepseek_helpers.js:390` | 历史清洗 `[^\\]]*` 会删除任意 `[citation:任意内容]` / `[reference:任意内容]` | 与 `web-deepseek.md` 标准一致；收紧为数字需同步标准，单独立项 |

---

## 5. 修复后验证

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test`：`4896 passed / 0 failed / 4 ignored`，集成测试全通过
- `node --check`：六个 Web 驱动脚本通过
- `web models --site grok/gk`、`--thinking`/`--model`/跨站点 session：通过
- 真实 Grok E2E（临时空间 `lx-tui-ai-web-grok-review`，验证后关闭）：
  - `--model Fast`：回答 `FAST`，`sessionId=ai-web-grok-66039c53...`
  - `--model Build`：首次暴露 `Build\nBeta` 标签比较错误，修正后回答 `BUILD`
  - 附件：`--file` 上传后回答 `The secret word is BANANA.`
  - `web new` / `web list` / `web delete`：通过，测试会话已删除
  - 正式 `SPACE` 已恢复为 `lx-tui-ai-web-grok`，二进制待最终 `cargo build` 重建
- 未执行：M3 冷启动、M4 未登录；M1/M2/M5 TUI 视觉与弹窗仍需用户 `cargo run` 目测。M6 `Build` 已执行并通过。
