# Grok 接入 Web 入口 — 验证记录 (`verification.md`)

- **执行日期**：2026-11-08
- **工作区**：`.worktrees/lx-ai-web-grok`（分支 `feat/lx-ai-web-grok`，基线 `d6e8bada`）
- **二进制**：`./target/debug/lx-tui`（`cargo build` 通过）
- **任务空间**：验证使用临时 `lx-tui-ai-web-grok-verify`（原因见文末“任务空间说明”）；验证结束后已恢复代码常量 `lx-tui-ai-web-grok` 并关闭临时空间。

## 1. 静态校验

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| `cargo test` | 4894 passed / 1 failed / 4 ignored；唯一失败为 `event::chat::tests::user_shell::short_script_runs_persists_output_without_model`，单独复跑通过，判定为 shell 管道环境抖动，与本次改动无关 |
| `node --check src/chat/web/grok_helpers.js` | 通过 |
| `node --check src/chat/web/grok_script.js` | 通过 |

## 2. 命令验证

| # | 命令 | 实际结果 |
| --- | --- | --- |
| 1 | `web models --site grok --json` | 输出 `grok`、`Fast/Build`、空 `thinkingVariants`、`ai-web-grok-` |
| 2 | `web chat --site grok --json "Reply with the single word OK."` | 输出 `OK`；`sessionId=ai-web-grok-414fe206...` |
| 3 | `web list --site grok --json` | 列表包含测试会话与用户原有会话 |
| 4 | `web show ai-web-grok-<id> --json` | 2 条消息；标题从会话列表 REST 补齐（不再为 `null`） |
| 5 | `web rename ai-web-grok-<id> "LX Grok Test" --json` | `ok: true`，列表标题更新 |
| 6 | `web chat --site grok --session ai-web-grok-<id> --edit-index 0 "Reply with the single word RETRY."` | 输出 `RETRY`；`editIndex: null`；Grok 端追加新轮次，旧轮次保留 |
| 7 | `web new --site grok --json` | 页面回到 `https://grok.com/`，0 条消息，composer 可见 |
| 8 | 长回答 + 4s 后 `web stop --site grok` | 已停止在 `one..eight`，未继续增长 |
| 9 | `web chat --site grok --file <secret.txt> "Reply with the secret word..."` | 输出 `BANANA`，附件上传成功 |
| 10 | `web delete ai-web-grok-<id> --site grok` × 4 | 4 个测试会话全部删除；`list` 只剩用户原有 2 个会话 |
| 11 | `web show ai-web-grok-5ff186ff... --json`（带联网引用的用户会话） | 输出不含 `<grok:render>`；引用变为 `[Chinatimes](https://chinatimes.com.hk/...)` 等来源链接 |
| 12 | `web chat --site grok "Search the web for today's top news headline..."` | 实时回答输出来源名，不含原始标签；测试会话已删除 |

## 3. 实测发现与修正

1. **正文提取**：助手消息头部有 `sr-only` 的 “Grok” 标题；正文未挂载时回退 `innerText` 会把 “Grok” 当回答。已改为只读 `.response-content-markdown`。
2. **登录误判**：页面出现普通 `sign-in` 链接不代表未登录；已移除“看到 sign-in 提前退出”，只信 `GET /rest/auth/get-user` 的 `email`。
3. **附件 chip 位置**：chip 容器在输入框 `<form>` 内，不在 `[data-testid="drop-ui"]` 内；选择器已改为 `form [role="list"][aria-label*="对话附件"]`。隐藏 input 轮询 10s，chip 等待 60s。
4. **Grok Edit 不是原地替换**：实测点击 Edit 提交后，Grok 清空旧用户消息正文并追加新用户消息 + 新回答；`/responses` 为线性追加。因此：
   - TUI `/rewind` 改为“重试追加”：保留旧轮次、回填输入、下一次发送追加；
   - 带附件轮次仍拒绝；
   - CLI `--edit-index` 对 Grok 降级为普通追加，输出 `editIndex: null`。
5. **会话标题**：`/responses` 不含标题；`load_session` 额外请求会话列表补标题。
6. **引用标签泄漏**：`/responses` 的 `message` 原样包含 `<grok:render card_id="…" ...><argument name="citation_id">N</argument></grok:render>`；直接在 `/resume` / `web show` 显示会看到 XML。已按同条 response 的 `cardAttachmentsJson` 把 `card_id` 映射为 URL，替换为 `[来源名](url)`；实时 DOM 采集也兜底剥离完整/未闭合标签。实测来源名为 `Chinatimes` / `Cctv` / `Bbc` / `Sina` 等域名标签。

## 4. 未自动化验证

- TUI 鼠标点击、三站点下划线视觉、Toast 实际位置：由用户 `cargo run` 目测。
- 未登录真实场景：本次验证时账号已登录；代码路径已按 `get-user` 的 `email` 判定并 `handOffTaskSpace()`。
- `Build` 模式：本次仅验证 `Fast`；`Build` 的正文提取逻辑相同，但未实际发消息。

## 5. 任务空间说明

正式任务空间 `lx-tui-ai-web-grok` 在第一次 `web chat` 时因旧版“sign-in 链接”误判触发 `handOffTaskSpace()`，控制权交给用户。按 ego-browser 规则未擅自夺回；修复登录判定后，用临时 `lx-tui-ai-web-grok-verify` 完成首轮端到端验证，引用标签修复使用临时 `inspect grok citations` 空间复验（验证后关闭），随后：

- 恢复 `grok_helpers.js` 的 `SPACE = 'lx-tui-ai-web-grok'`；
- 关闭临时任务空间；
- 正式任务空间仍由用户持有，可在 ego-browser 中释放或关闭；下次 lx 运行时会按现有 `reclaimSpaceTarget` 逻辑接管。
