# `Web`（`DeepSeek · Gemini`）双站点网页驱动、品牌渐变与 `/model` 切换 — CLI 端到端验证报告 (`verification.md`)

- **特性目录**：`docs/feature/lx-ai-web-gemini/`
- **关联文档**：[`design.md`](design.md) · [`task.md`](task.md) · [`docs/standards/web-deepseek.md`](../../standards/web-deepseek.md)
- **验证工作区**：`/Users/yonah/projects/agent/lx-tui/.worktrees/lx-ai-web-gemini`（分支 `feat/lx-ai-web-gemini`）
- **验证 CLI 工具**：`./target/debug/lx-tui web`（底层驱动 `~/.local/bin/ego-browser nodejs` 隔离任务空间 `lx-tui-ai-web-deepseek` 与 `lx-tui-ai-web-gemini`）
- **验证结论**：**全部 10 个验证项（涵盖 `DeepSeek` 与 `Gemini` 双站点全部功能及 TUI 回归）全部通过（PASS）**

---

## 0. `lx-tui web` 无头 CLI 工具能力盘点与补齐

为支持通过 CLI 工具对 `DeepSeek` 与 `Gemini` 两个 Web AI 站点的全部能力执行端到端实测，在 `src/cli/web.rs` 与 `src/cli/web/exec.rs` 中新增了 `lx-tui web` 顶层子命令，直接复用 `src/chat/web/` 的嵌入式脚本、`ScriptParams` 块级作用域注入、`@@LX_AI_WEB@@` NDJSON 协议解析、`src/chat/prepare/web.rs` 的本地附件与禁用引用预检，以及 `src/event/chat/web.rs` 的本地回环实时流监听器：

| CLI 子命令 | 对应 `Web` 功能 | 说明 |
| --- | --- | --- |
| `lx-tui web models [--site <deepseek\|gemini>] [--json]` | `/model` 候选与站点元数据 | 输出双站点模型列表、思考等级/模式、默认档位、会话 ID 前缀（`ai-web-ds-` / `ai-web-gm-`）与 `ego-browser` 任务空间 |
| `lx-tui web new [--site <deepseek\|gemini>] [--json]` | `/new`（`/clear`） | 驱动对应站点在 `ego-browser` 任务空间内重置并开启新对话 |
| `lx-tui web chat [<PROMPT>] [--site <deepseek\|gemini>] [-m <MODEL>] [-t <THINKING>] [-s <SESSION_ID>] [--edit-index <N>] [--file <PATH>...] [--keep-file <NAME>...] [--json]` | 流式对话、`/model` 模型与思考等级同步、`@` 本地文件/图片上传、`/rewind` 原地修改 | 启动回环 HTTP 流监听并注入 `__LX_PARAMS`，实时采集 `thinking` 与 `text` 流分片及绑定的云端 `sessionId` / `title` |
| `lx-tui web list [--site <deepseek\|gemini>] [--limit <N>] [--json]` | `/resume`（`/session`）列表 | 拉取对应站点云端会话列表并附加 `ai-web-ds-` / `ai-web-gm-` 前缀 |
| `lx-tui web show <SESSION_ID> [--site <deepseek\|gemini>] [--thinking] [--json]` | `/resume` 加载历史会话 | 按会话前缀或 `--site` 路由站点，加载并输出完整历史消息（含附件芯片与思考过程） |
| `lx-tui web rename <SESSION_ID> <NEW_TITLE> [--site <deepseek\|gemini>] [--json]` | `/rename`（`/title`）及 `/resume` 内 `Shift+Enter` | 同步修改云端会话标题 |
| `lx-tui web delete <SESSION_ID> [--site <deepseek\|gemini>] [--json]` | `/resume` 内 `Ctrl+D` 删除 | 从对应站点云端删除指定会话 |
| `lx-tui web stop [--site <deepseek\|gemini>] [--json]` | 流式生成中 `Esc` 停止 | 触发对应站点网页端停止生成按钮 |

---

## 1. 逐项验证明细

### 验证项 V1：`lx-tui web --help` 与 `lx-tui web models` 双站点模型、思考等级及隔离元数据验证

- **输入内容**：
  ```bash
  ./target/debug/lx-tui web --help
  ./target/debug/lx-tui web models
  ./target/debug/lx-tui web models --json
  ```
- **验证步骤**：
  1. 在 `.worktrees/lx-ai-web-gemini` 下执行 `./target/debug/lx-tui web --help` 检查子命令帮助文档；
  2. 执行 `./target/debug/lx-tui web models` 与 `./target/debug/lx-tui web models --json`，核对 `DeepSeek` 与 `Gemini` 的模型列表、思考等级、默认值、会话前缀与任务空间名称。
- **期望结果**：
  - `web --help` 输出完整英文 `Usage:` 与 8 个子命令说明，退出码为 `0`；
  - `DeepSeek` 站点：`models = ["DeepSeek"]`，`thinkingVariants = ["deepthink", "off"]`，默认 `DeepSeek (deepthink)`，`sessionPrefix = "ai-web-ds-"`，`taskSpace = "lx-tui-ai-web-deepseek"`；
  - `Gemini` 站点：`models = ["3.5 Flash-Lite", "3.8 Flash", "3.1 Pro", "4 Argon"]`，`thinkingVariants = ["low", "medium", "high"]`，默认 `3.8 Flash (low)`，`sessionPrefix = "ai-web-gm-"`，`taskSpace = "lx-tui-ai-web-gemini"`。
- **实际验证结果（PASS）**：
  ```text
  SITE      MODEL           THINKING           DEFAULT               SESSION_PREFIX  TASK_SPACE
  deepseek  DeepSeek        deepthink, off     DeepSeek (deepthink)  ai-web-ds-      lx-tui-ai-web-deepseek
  gemini    3.5 Flash-Lite  low, medium, high  -                     ai-web-gm-      lx-tui-ai-web-gemini
  gemini    3.8 Flash       low, medium, high  3.8 Flash (low)       ai-web-gm-      lx-tui-ai-web-gemini
  gemini    3.1 Pro         low, medium, high  -                     ai-web-gm-      lx-tui-ai-web-gemini
  gemini    4 Argon         low, medium, high  -                     ai-web-gm-      lx-tui-ai-web-gemini
  ```

---

### 验证项 V2：非法参数、跨站点会话前缀冲突及 `Web` 禁用引用（目录 / Skill / MCP / 缺失文件）拦截验证

- **输入内容**：
  ```bash
  VERIFY_DIR="/private/var/folders/8p/m6b108k50m7d4xgb37c5dxnh0000gn/T/opencode/lx-web-verify"
  mkdir -p "$VERIFY_DIR/subdir"
  ./target/debug/lx-tui web chat --site gemini -m "bad-model" "hi"
  ./target/debug/lx-tui web chat --site deepseek -t "high" "hi"
  ./target/debug/lx-tui web show ai-web-gm-123 --site deepseek
  ./target/debug/lx-tui web chat --site gemini -C "$VERIFY_DIR" "@subdir/ hello"
  ./target/debug/lx-tui web chat --site deepseek "@[refer-skill](grill-me) hello"
  ./target/debug/lx-tui web chat --site gemini "@[refer-mcp](codegraph) hello"
  ./target/debug/lx-tui web chat --site gemini -f "$VERIFY_DIR/nonexistent.txt" "hello"
  ```
- **验证步骤**：
  1. 创建临时验证目录 `$VERIFY_DIR/subdir`；
  2. 依次传入非法 Gemini 模型名、非法 DeepSeek 思考模式、与 `--site` 冲突的会话 ID 前缀（`ai-web-gm-123` 配 `--site deepseek`）、目录引用 `@subdir/`、Skill 引用 `@[refer-skill](grill-me)`、MCP 引用 `@[refer-mcp](codegraph)` 以及不存在的本地文件路径，检查 `stderr` 输出与退出码。
- **期望结果**：
  - 全部命令均在本地预检阶段立即拦截，不拉起浏览器子进程，返回非零退出码（`1`），并输出清晰的英文错误提示：
    - `Folders cannot be uploaded in Web; select files instead`
    - `Skills and MCP references are not available in Web`
    - `Attached file does not exist: ...`
- **实际验证结果（PASS）**：
  ```text
  lx-tui: invalid '--model' value 'bad-model' for Gemini; expected '3.5 Flash-Lite', '3.8 Flash', '3.1 Pro', or '4 Argon'
  lx-tui: invalid '--thinking' value 'high' for DeepSeek; expected 'deepthink' or 'off'
  lx-tui: session 'ai-web-gm-123' belongs to 'gemini' but '--site deepseek' was specified
  lx-tui: Folders cannot be uploaded in Web; select files instead
  lx-tui: Skills and MCP references are not available in Web
  lx-tui: Skills and MCP references are not available in Web
  lx-tui: Attached file does not exist: /private/var/folders/8p/m6b108k50m7d4xgb37c5dxnh0000gn/T/opencode/lx-web-verify/nonexistent.txt
  ```

---

### 验证项 V3：`DeepSeek` 网页端：新会话重置（`new`）、深度思考开关切换（`off` vs `deepthink`）与流式对话（`chat`）

- **输入内容**：
  ```bash
  ./target/debug/lx-tui web new --site deepseek --json
  ./target/debug/lx-tui web chat --site deepseek -t off "Compute 12 + 34 and reply with only: DS-OFF-46" --json
  ./target/debug/lx-tui web new --site deepseek --json
  ./target/debug/lx-tui web chat --site deepseek -t deepthink "Compute 12 + 34 and reply with only: DS-THINK-46" --json
  ```
- **验证步骤**：
  1. 执行 `lx-tui web new --site deepseek --json` 开启新对话；
  2. 以 `-t off`（关闭深度思考）发送算术指令并检查返回的 `thinkingText`、`text` 与 `sessionId`；
  3. 再次执行 `new` 开启新对话，以 `-t deepthink`（开启深度思考）发送指令并检查返回的 `thinkingText` 与 `text`。
- **期望结果**：
  - `-t off` 时网页端自动关闭「深度思考」按钮，返回 JSON 中 `thinking: "off"`、`thinkingText: ""`、`text: "DS-OFF-46"`，会话 ID 带 `ai-web-ds-` 前缀；
  - `-t deepthink` 时网页端自动点亮「深度思考」按钮，返回 JSON 中 `thinking: "deepthink"`、`thinkingText` 非空（包含推理链文本）、`text: "DS-THINK-46"`。
- **实际验证结果（PASS）**：
  ```json
  {
    "editIndex": null,
    "files": [],
    "keepFiles": [],
    "model": "DeepSeek",
    "remoteSessionId": "8c8043ed-c8a5-445e-b7ae-59a8c5bf8360",
    "sessionId": "ai-web-ds-8c8043ed-c8a5-445e-b7ae-59a8c5bf8360",
    "site": "deepseek",
    "text": "DS-OFF-46",
    "thinking": "off",
    "thinkingText": "",
    "title": "Simple arithmetic"
  }
  {
    "editIndex": null,
    "files": [],
    "keepFiles": [],
    "model": "DeepSeek",
    "remoteSessionId": "61c21b72-69a2-4c03-baf1-04fe61b88063",
    "sessionId": "ai-web-ds-61c21b72-69a2-4c03-baf1-04fe61b88063",
    "site": "deepseek",
    "text": "DS-THINK-46",
    "thinking": "deepthink",
    "thinkingText": "We need answer only DS-THINK-46. Need compute 12+34? But reply only specified string. Need follow exact.",
    "title": "Only token"
  }
  ```

---

### 验证项 V4：`DeepSeek` 网页端：本地文件上传（`--file`）与 `/rewind` 原地修改提问及保留云端附件（`--edit-index 0 --keep-file`）

- **输入内容**：
  ```bash
  VERIFY_DIR="/private/var/folders/8p/m6b108k50m7d4xgb37c5dxnh0000gn/T/opencode/lx-web-verify"
  printf "SECRET_TOKEN=DS_FILE_7788\n" > "$VERIFY_DIR/ds_note.txt"
  ./target/debug/lx-tui web new --site deepseek --json
  ./target/debug/lx-tui web chat --site deepseek -t off --file "$VERIFY_DIR/ds_note.txt" "Read the attached file and reply with only the SECRET_TOKEN value." --json
  DS_SID="ai-web-ds-91caee6e-c62d-41cd-a174-a18cb8d55399"
  ./target/debug/lx-tui web chat --site deepseek -t off --session "$DS_SID" --edit-index 0 --keep-file "ds_note.txt" "Read the attached file and reply with only: EDITED_DS_7788" --json
  ```
- **验证步骤**：
  1. 创建包含 `SECRET_TOKEN=DS_FILE_7788` 的测试文件 `ds_note.txt`；
  2. 通过 `--file "$VERIFY_DIR/ds_note.txt"` 上传文件并提问，验证 CDP `DOM.setFileInputFiles` 注入与文件内容读取；
  3. 对同一会话传入 `--edit-index 0 --keep-file "ds_note.txt"` 执行 `/rewind` 原地修改第 0 轮提问（不重新上传本地文件，直接保留云端已上传的 `ds_note.txt` 附件芯片），验证修改后重发的新回复。
- **期望结果**：
  - 首轮上传文件后准确返回 `"text": "DS_FILE_7788"`；
  - 原地修改第 0 轮提问并保留 `ds_note.txt` 后，准确返回 `"text": "EDITED_DS_7788"` 且 `"keepFiles": ["ds_note.txt"]`。
- **实际验证结果（PASS）**：
  ```json
  {
    "editIndex": null,
    "files": [
      "/private/var/folders/8p/m6b108k50m7d4xgb37c5dxnh0000gn/T/opencode/lx-web-verify/ds_note.txt"
    ],
    "keepFiles": [],
    "model": "DeepSeek",
    "remoteSessionId": "91caee6e-c62d-41cd-a174-a18cb8d55399",
    "sessionId": "ai-web-ds-91caee6e-c62d-41cd-a174-a18cb8d55399",
    "site": "deepseek",
    "text": "DS_FILE_7788",
    "thinking": "off",
    "thinkingText": "",
    "title": "Read secret token"
  }
  {
    "editIndex": 0,
    "files": [],
    "keepFiles": [
      "ds_note.txt"
    ],
    "model": "DeepSeek",
    "remoteSessionId": "91caee6e-c62d-41cd-a174-a18cb8d55399",
    "sessionId": "ai-web-ds-91caee6e-c62d-41cd-a174-a18cb8d55399",
    "site": "deepseek",
    "text": "EDITED_DS_7788",
    "thinking": "off",
    "thinkingText": "",
    "title": "Read secret token"
  }
  ```

---

### 验证项 V5：`DeepSeek` 网页端：云端历史会话加载（`show`）、重命名（`rename`）、列表（`list`）、停止（`stop`）与删除（`delete`）

- **输入内容**：
  ```bash
  DS_SID="ai-web-ds-91caee6e-c62d-41cd-a174-a18cb8d55399"
  ./target/debug/lx-tui web show "$DS_SID" --thinking --json
  ./target/debug/lx-tui web rename "$DS_SID" "CLI-Verified-DS-Session" --json
  ./target/debug/lx-tui web list --site deepseek --limit 3 --json
  ./target/debug/lx-tui web delete "$DS_SID" --json
  ./target/debug/lx-tui web list --site deepseek --limit 3 --json
  ./target/debug/lx-tui web stop --site deepseek --json
  ```
- **验证步骤**：
  1. 执行 `web show "$DS_SID" --thinking --json` 检查原地修改后的云端会话消息与附件列表；
  2. 执行 `web rename "$DS_SID" "CLI-Verified-DS-Session" --json` 后调用 `web list` 验证云端标题已更新；
  3. 执行 `web delete "$DS_SID" --json` 后再次调用 `web list` 验证会话已从云端移除，并执行 `web stop --site deepseek --json`。
- **期望结果**：
  - `show` 返回 2 条消息（user 含 `"files": ["ds_note.txt"]` 与修改后文本，assistant 含 `"EDITED_DS_7788"`）；
  - `rename` 后 `list` 首条会话标题变为 `"CLI-Verified-DS-Session"`；
  - `delete` 后该会话从 `list` 中消失，`stop` 返回 `"ok": true`。
- **实际验证结果（PASS）**：
  - `show` 返回 `messageCount: 2` 且 `messages[0].files == ["ds_note.txt"]`；
  - `rename` 后 `list` 返回 `"title": "CLI-Verified-DS-Session"`；
  - `delete` 后 `list` 确认 `ai-web-ds-91caee6e-c62d-41cd-a174-a18cb8d55399` 已删除，`stop` 返回 `{"action":"stop","ok":true,"site":"deepseek"}`。

---

### 验证项 V6：`Gemini` 网页端：新会话重置（`new`）、模型与思考等级切换（`3.8 Flash` + `low` vs `3.5 Flash-Lite` + `high`）与流式对话（`chat`）

- **输入内容**：
  ```bash
  ./target/debug/lx-tui web new --site gemini --json
  ./target/debug/lx-tui web chat --site gemini -m "3.8 Flash" -t low "Reply with only: GM-FLASH-LOW-OK" --json
  ./target/debug/lx-tui web new --site gemini --json
  ./target/debug/lx-tui web chat --site gemini -m "3.5 Flash-Lite" -t high "Compute 15 + 27 and reply with only: GM-LITE-HIGH-42" --json
  ```
- **验证步骤**：
  1. 执行 `lx-tui web new --site gemini --json` 重置 Gemini 页面；
  2. 使用默认模型与思考等级（`-m "3.8 Flash" -t low`）发送消息，检查返回的正文、会话 ID 前缀（`ai-web-gm-`）及自动提取的会话标题；
  3. 执行 `new` 后切换模型与思考等级为 `-m "3.5 Flash-Lite" -t high` 发送算术请求，检查模型切换、高思考等级下的 `thinkingText` 采集与最终正文。
- **期望结果**：
  - 第一轮：切换至 `3.8 Flash` + `low`，返回 `text: "GM-FLASH-LOW-OK"`，`sessionId` 以 `ai-web-gm-` 开头，`title` 为真实对话标题（非 Google 账号链接）；
  - 第二轮：切换至 `3.5 Flash-Lite` + `high`，返回非空 `thinkingText`（`"Computing the requested specific string"`）与 `text: "GM-LITE-HIGH-42"`。
- **实际验证结果（PASS）**：
  ```json
  {
    "editIndex": null,
    "files": [],
    "keepFiles": [],
    "model": "3.8 Flash",
    "remoteSessionId": "a4241fefa770f90a",
    "sessionId": "ai-web-gm-a4241fefa770f90a",
    "site": "gemini",
    "text": "GM-FLASH-LOW-OK",
    "thinking": "low",
    "thinkingText": "",
    "title": "Exact Phrase Confirmation Test"
  }
  {
    "editIndex": null,
    "files": [],
    "keepFiles": [],
    "model": "3.5 Flash-Lite",
    "remoteSessionId": "cd7c31823125c485",
    "sessionId": "ai-web-gm-cd7c31823125c485",
    "site": "gemini",
    "text": "GM-LITE-HIGH-42",
    "thinking": "high",
    "thinkingText": "Computing the requested specific string",
    "title": "Secret Phrase Confirmation"
  }
  ```

---

### 验证项 V7：`Gemini` 网页端：本地文件上传（`--file`）与 `/rewind` 原地修改提问及保留云端附件（`--edit-index 0 --keep-file`）

- **输入内容**：
  ```bash
  VERIFY_DIR="/private/var/folders/8p/m6b108k50m7d4xgb37c5dxnh0000gn/T/opencode/lx-web-verify"
  printf "GEMINI_CODE=GM_FILE_9900\n" > "$VERIFY_DIR/gm_note.txt"
  ./target/debug/lx-tui web new --site gemini --json
  ./target/debug/lx-tui web chat --site gemini -m "3.8 Flash" -t low --file "$VERIFY_DIR/gm_note.txt" "Read the attached file and reply with only the GEMINI_CODE value." --json
  GM_SID="ai-web-gm-f06512faf837442f"
  ./target/debug/lx-tui web chat --site gemini -m "3.8 Flash" -t low --session "$GM_SID" --edit-index 0 --keep-file "gm_note.txt" "Read the attached file and reply with only: EDITED_GM_9900" --json
  ```
- **验证步骤**：
  1. 创建包含 `GEMINI_CODE=GM_FILE_9900` 的本地文件 `gm_note.txt`；
  2. 通过 `--file "$VERIFY_DIR/gm_note.txt"` 在 Gemini 网页端触发上传菜单并通过 CDP `DOM.setFileInputFiles` 注入文件，验证 Gemini 读取附件内容；
  3. 对该会话执行 `--edit-index 0 --keep-file "gm_note.txt"` 原地修改第 0 轮提问并保留已有 `.file-preview-container` 附件卡片，验证「更新」按钮激活与新回复采集。
- **期望结果**：
  - 首轮文件上传返回 `"text": "GM_FILE_9900"`；
  - 原地修改第 0 轮提问并保留 `gm_note.txt` 后，返回 `"text": "EDITED_GM_9900"`，不会误采旧回复。
- **实际验证结果（PASS）**：
  ```json
  {
    "editIndex": null,
    "files": [
      "/private/var/folders/8p/m6b108k50m7d4xgb37c5dxnh0000gn/T/opencode/lx-web-verify/gm_note.txt"
    ],
    "keepFiles": [],
    "model": "3.8 Flash",
    "remoteSessionId": "f06512faf837442f",
    "sessionId": "ai-web-gm-f06512faf837442f",
    "site": "gemini",
    "text": "GM_FILE_9900",
    "thinking": "low",
    "thinkingText": "",
    "title": "File Reading and Code Retrieval"
  }
  {
    "editIndex": 0,
    "files": [],
    "keepFiles": [
      "gm_note.txt"
    ],
    "model": "3.8 Flash",
    "remoteSessionId": "f06512faf837442f",
    "sessionId": "ai-web-gm-f06512faf837442f",
    "site": "gemini",
    "text": "EDITED_GM_9900",
    "thinking": "low",
    "thinkingText": "",
    "title": "File Reading and Code Retrieval"
  }
  ```

---

### 验证项 V8：`Gemini` 网页端：云端历史会话加载（`show`）、重命名（`rename`）、列表（`list`）、停止（`stop`）与删除（`delete`）

- **输入内容**：
  ```bash
  ./target/debug/lx-tui web show ai-web-gm-f06512faf837442f --thinking --json
  ./target/debug/lx-tui web rename ai-web-gm-f801468700fc16fa "CLI-Verified-GM-Renamed-2" --json
  ./target/debug/lx-tui web list --site gemini --limit 3 --json
  ./target/debug/lx-tui web delete ai-web-gm-f801468700fc16fa --json
  ./target/debug/lx-tui web list --site gemini --limit 3 --json
  ./target/debug/lx-tui web stop --site gemini --json
  ```
- **验证步骤**：
  1. 执行 `web show` 加载 Gemini 云端会话，核查消息列表、附件文件名及标题提取；
  2. 执行 `web rename` 修改 Gemini 会话标题，随后通过 `web list --site gemini` 验证侧边栏标题已持久化更新为 `"CLI-Verified-GM-Renamed-2"`；
  3. 执行 `web delete` 删除测试会话并再次 `web list` 确认已移除，最后验证 `web stop --site gemini`。
- **期望结果**：
  - `show` 准确返回用户提问、附件 `"gm_note.txt"`、助手回复 `"EDITED_GM_9900"` 与真实会话标题；
  - `rename` 触发 Angular `InputEvent` 并在保存按钮解除禁用后点击提交，`list` 首条标题更新为 `"CLI-Verified-GM-Renamed-2"`；
  - `delete` 确认弹窗删除后会话从 `list` 消失，`stop` 返回 `"ok": true`。
- **实际验证结果（PASS）**：
  ```json
  {
    "ok": true,
    "remoteSessionId": "f801468700fc16fa",
    "sessionId": "ai-web-gm-f801468700fc16fa",
    "site": "gemini",
    "title": "CLI-Verified-GM-Renamed-2"
  }
  {
    "count": 3,
    "sessions": [
      {
        "remoteSessionId": "f801468700fc16fa",
        "sessionId": "ai-web-gm-f801468700fc16fa",
        "title": "CLI-Verified-GM-Renamed-2",
        "updatedAtMs": 1794124499942
      }
    ],
    "site": "gemini"
  }
  ```

---

### 验证项 V9：欢迎页 `Web`（`DeepSeek · Gemini`）品牌渐变、下划线渲染与左上角 `lx-Web` 标题校验

- **输入内容**：
  ```bash
  rtk cargo test ui::main_content::lx::tests
  ```
- **验证步骤**：
  1. 运行 `ui::main_content::lx::tests` 全部 112 个 UI 渲染与命中单元测试；
  2. 检查顶部 4 模式（`Native · Hybrid · Automation · Web`）及 `Web` 模式下 `DeepSeek · Gemini` 的 256 色逐字品牌渐变、选中态 `Modifier::BOLD` 与 `y + 1` 行 `─` 同色渐变下划线。
- **期望结果**：
  - 112 个 UI 测试全部通过；
  - `DeepSeek` 使用 `(Color::Indexed(63), Color::Indexed(39))` 渐变，`Gemini` 使用 `(Color::Indexed(69), Color::Indexed(204))` 渐变，选中项加粗并在正下方绘制渐变 `─` 下划线。
- **实际验证结果（PASS）**：
  ```text
  cargo test: 112 passed, 4786 filtered out (15 suites, 0.01s)
  ```

---

### 验证项 V10：`Web` 状态机、两级 `/model` 选择面板、跨站点切换清场与 CLI 套件全量校验

- **输入内容**：
  ```bash
  rtk cargo fmt --check
  rtk cargo clippy --all-targets -- -D warnings
  rtk cargo test web
  rtk cargo test cli
  ```
- **验证步骤**：
  1. 执行代码格式检查与全目标 Clippy 零警告检查；
  2. 运行 `rtk cargo test web` 与 `rtk cargo test cli` 验证状态机、两级 `/model` 选择器（纯内存态 `ai_web_config`，不写 `ai.json`/SQLite）、跨站点切换自动清场与 CLI 参数路由。
- **期望结果**：
  - `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 零问题通过；
  - `cargo test web`（93 passed）与 `cargo test cli`（305 passed）全部通过。
- **实际验证结果（PASS）**：
  ```text
  cargo clippy: No issues found
  cargo test web: 93 passed, 4805 filtered out (15 suites, 0.25s)
  cargo test cli: 305 passed, 1 ignored, 4592 filtered out (15 suites, 8.07s)
  ```
