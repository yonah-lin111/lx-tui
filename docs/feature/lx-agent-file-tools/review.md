# lx Agent 思考生命周期与文件工具 Review

审查基线：`dev...d1c9e33`（分支 `feat/lx-agent-file-tools`，29 个提交 / 117 文件 / +29346 −2685）。
参考实现：`codex-main/codex-rs`（apply-patch / file-search / exec-server / core/src/tools / tui/src/exec_cell）。
审查对象：`design.md`（§1–§24）与 `task.md` 声明的全部交付内容，重点为**文件工具执行层**、**agent loop 握手**、**上下文回放**、**工具步骤渲染**。

只审查，未修改任何功能代码；所有结论都有本机可复现的证据（见 §9 探针清单）。

## 0. 结论摘要

整体实现质量高：分层干净（`chat/tools` 不依赖 `app/ui`）、截断语义显式、写路径有原子替换 + 路径锁、DB v9 迁移带行数核对与外键自检、参数白名单 + 无 shell 拼接、tracing 不打参数全文。
**在若干点上明确优于 codex**（写路径真原子、`Add File` 拒绝覆盖、无 `exact` 不确定性）。

但**最近一轮（G14）的「重进会话必须与实时一致」这条不变量并没有真正成立**：最常用的 `read`（不带 `offset`/`limit`）与目录读取重进会话后仍会丢详情。与 codex 逐项对照后，还暴露出三个**语义级**差异：`search` 的忽略规则与 ripgrep/codex 相反、纯插入语句的落点与 codex 相反、补丁兼容面比 codex 窄得多。

| 编号 | 级别 | 问题 | 影响面 |
| --- | --- | --- | --- |
| P1-1 | 阻断 | `read`/目录读取重载后丢区间与条目数（G14 声称已修的同一类 bug 的另一半） | 重进会话的 UI 与实时不一致 |
| P1-2 | 阻断 | `search` 不排除 `.git/`、且 `parents(false)` 让忽略规则随搜索根漂移 | 凭据进上下文；同一棵树结果不幂等 |
| P1-3 | 阻断 | 失败补丁的「部分成功」信息同轮给了、跨轮丢了 | 下一轮模型不知道哪些文件已被改 |
| P1-4 | 阻断 | `(N lines)` 用的是文件总行数，不是本次读到的行数 | 展示误导 + 两份文档失真 |
| P2-1 | 严重 | 多文件 `edit` 实时渲染出现孤立 `· `（`└─  · +12 -4`） | 每次多文件补丁都可见 |
| P2-2 | 严重 | `search` 深度上限 32 静默截断，不置 `truncated` | 违反「截断必须显式」契约 |
| P2-3 | 严重 | `read` 把整个文件读进内存；目录读取无条目上限 | 大文件 OOM（codex 有 512 MiB 硬上限） |
| P2-4 | 严重 | `edit` 的 `Move to` 静默覆盖目标文件已有内容且不上报 | 数据丢失且模型不知情 |
| P2-5 | 严重 | 多 hunk 的增删计数用前后缀剥离，20 行文件改 2 行显示 `+18 -18` | UI 与模型都拿到误导性规模 |
| P2-6 | 严重 | `offset`-only / `limit`-only 的 `read` 区间实时与重载不同 | 重载显示错误的行范围 |
| P2-7 | 严重 | 窄宽度下 `search` 详情把 `"pattern" in path` 当一整串中间省略 | 窄窗格可读性 |
| P2-8 | 严重 | 结算路径只合并 `pattern`，与重载路径的全量合并口径不一致 | 实时/重载行为漂移的温床 |
| P2-9 | 严重 | 不支持 `*** End of File`（codex grammar 明确支持） | GPT-5 系列产出的补丁整轮失败 |
| P2-10 | 严重 | 只有精确匹配 + 强制唯一，无 rstrip/trim 回退（codex 有四级） | 常见代码（`}`、`return;`）补丁失败 |
| P2-11 | 严重 | 纯插入语句（无 `@@` 上下文）落到**文件第 0 行**，codex 是追加到末尾 | 语义与参考相反且未文档化 |
| P3 | 中 | `has_summary` 手写六字段 or、JSON 形状两处手写且无往返测试 | 再加字段会静默回归 P1-1 |
| P3 | 中 | `needs_spacer` 恒 `true`，`Block` 枚举沦为占位参数 | 死抽象 |
| P3 | 中 | `tool.rs` 模块文档、`style::agent_param` 与实现漂移 | 后续维护误改 |
| P3 | 中 | `param()` 每帧重新解析最多 1 MiB 的 `arguments_json` | 流式期间的每帧 CPU |

## 1. 与 codex 逐项对照

这一节是本次复审的核心：把每条设计选择与 codex 的实际做法并排放，标出「一致 / 我们更严 / 我们更松 / 我们相反」。
参考文件均在 `codex-main/codex-rs/` 下。

### 1.1 补丁定位：匹配严格度

| | codex | lx | 判定 |
| --- | --- | --- | --- |
| 匹配档位 | **四级**：精确 → rstrip → trim 两侧 → Unicode 标点归一化（`apply-patch/src/seek_sequence.rs:12-110`） | **一级**：精确（`chat/tools/edit.rs:330-335`） | 我们更严 |
| 多重匹配 | 返回**第一个**（`seek_sequence` 找不到歧义就报错，找到即用） | 要求**恰好一次**，多处即 `PatchContextMismatch`（`edit.rs:336-346`） | 我们更严 |
| `@@` 上下文 | 用 `seek_sequence` 定位，**不要求唯一**（`file_update.rs:69-82`） | 定位行**必须唯一**（`edit.rs:299-322`） | 我们更严 |
| 缺失容错 | 末行是空串时回退去掉末尾空行重试（`file_update.rs:108-122`） | 无 | 我们更严 |
| `*** End of File` | grammar 明确支持（`parser.rs:16-23`、`is_end_of_file` 参与定位） | 显式列为**不做项**（`design.md:605`） | 我们更严 |

**判断**：`design.md:605` 把这四条都写成「有意的保守选择」，理由是"模型基于旧文件内容生成的补丁会失败并要求重读"。这个理由对 **Unicode 归一化**成立（那是给人类 diff 用的），但对 **rstrip** 与 **End of File** 不成立——它们不是"模糊猜测"，而是模型实际会产出的格式。

实测（§9 探针 12-15）：

```text
*** End of File        → patch_invalid :: line in '*** Update File: eof.txt' must start with '+', '-' or ' '
标记前后带空格          → patch_invalid :: patch must end with '*** End Patch'
上下文行尾有空格         → patch_context_mismatch
```

第一条是关键风险：codex 的 `apply_patch` 是 **FREEFORM（lark grammar）** 工具（`core/src/tools/handlers/apply_patch_spec.rs:9-30`），grammar 里就有 `eof_line: "*** End of File" LF`。在 codex 上训练/调过的模型会自然产出这个标记；我们把它当语法错误，整个补丁直接失败。

**建议**：
1. `rstrip` 一档（成本极低、语义明确、不引入误改风险）；
2. 解析时**接受并忽略** `*** End of File`（或实现其真正的"锚定文件末尾"语义）；
3. 唯一性要求保留（这是安全优势），但**失败消息里给出候选行号**，让模型能自己补上下文：

```text
the changed lines match 3 places in src/a.rs (lines 12, 48, 91); add surrounding context lines
```

### 1.2 纯插入语句的落点

codex（`apply-patch/src/file_update.rs:86-89`）：

```rust
if chunk.old_lines.is_empty() {
    replacements.push((original_lines.len(), 0, chunk.new_lines.clone()));
    continue;
}
```

→ **无条件追加到文件末尾**。

lx（`chat/tools/edit.rs:323-326`）：

```rust
let old_lines = block_old_lines(block);
if old_lines.is_empty() {
    return Ok(start.min(lines.len()));   // start = 上一个 block 之后 / @@ 上下文之后 / 0
}
```

实测（§9 探针 16）：

```text
原文件: line1 / line2 / line3
补丁:   *** Update File: a.txt
        @@
        +tail
lx 结果: "tail\nline1\nline2\nline3\n"      ← 插到第 0 行
codex:   "line1\nline2\nline3\ntail\n"      ← 追加到末尾
```

带 `@@ ctx` 时我们插在 `ctx` 之后（比 codex 更符合直觉），但**无上下文时插到文件开头**。更糟的是：四份文档（`design.md`、`task.md`、`schema.rs` 的工具描述、`patch.rs` 模块注释）**都没有定义这个语义**。

**建议**：显式规定一条（推荐：无上下文 → 追加末尾，与 codex 一致；有 `@@ ctx` → 紧跟 `ctx` 之后），写进工具描述与 `design.md`，并补两侧测试。

### 1.3 搜索：可达文件集合

| | codex / ripgrep | lx | 判定 |
| --- | --- | --- | --- |
| `.git/` | **排除**（ripgrep 的 always-ignore；codex 内容搜索走 `rg`） | **包含**（`hidden(false)`，无 always-ignore） | **我们相反** |
| `.gitignore` | 遵守，`require_git(true)` 限定在 git 上下文（`file-search/src/lib.rs:437-451`，注释明确说明这是为了避开父目录宽泛 ignore 吞掉子树） | 遵守 `git_ignore(true)`，但 `parents(false)` | 部分 |
| 隐藏文件（`.env` 等） | `rg` 默认**不含**；`rg --hidden` 含（但仍跳 `.git`） | 含 | 产品选择 |
| 父级忽略文件 | `require_git(true)` + 默认 parents | `parents(false)` | 我们更严但语义漂移 |

实测（§9 探针 6、7；并用本机 `ripgrep 15.1.0` 对照）：

```text
fixture: .env(SECRET), .git/config(url=…pw…), visible.txt, .gitignore(*.log), drop.log
rg SECRET                 →  visible.txt                    ← 隐藏文件也不搜
rg --hidden SECRET        →  .env, visible.txt              ← 仍跳过 .git
rg -uuu SECRET            →  全部（含 .git）
lx search SECRET          →  .env, visible.txt              ← 相当于 rg --hidden，但少了 .git 排除
lx search（fixture 含 .git/config）→ 命中 .git/config       ← 凭据进上下文
lx search path:"sub"      →  父级 .gitignore 失效（root 1 hit / sub 2 hits）
```

**精确结论**（比初版 review 更准确）：缺陷**不是**"搜了隐藏文件"，而是两件独立的事：

1. **缺 `.git/` always-ignore** —— 这是 ripgrep 的硬规则，与"是否搜隐藏文件"无关。`.git/config` 含 remote URL 凭据、`.git/logs` 含操作历史，都应该永远排除。
2. **`parents(false)` 让忽略规则随搜索根变化** —— 同一棵树只因 `path` 不同而结果不同，且工具描述说"Respects `.gitignore`"（`schema.rs:24-27`），模型无法预判。

**建议**：加 `.git/`（及 `.jj/`）到 always-ignore；`parents(false)` 改成 `parents(true)`（codex 用 `require_git(true)` 解决"父目录宽泛 ignore"的问题，我们也可以照抄这一条，而不是关掉 parents）。隐藏文件是否默认搜，写进 schema 并给显式开关。

### 1.4 失败后的变更可见性

| | codex | lx | 判定 |
| --- | --- | --- | --- |
| 失败是否带已提交变更 | 是（`AppliedPatchDelta` 随 `ApplyPatchFailure::into_parts()` 带出，`lib.rs:246-330`） | 是（`Failed{changes}`） | 一致 |
| 是否区分"确定提交"与"可能有副作用" | 是（`delta.exact` 标志，写失败时置 false，`lib.rs:455-470`） | **不需要**（原子写保证失败时目标不变） | **我们更干净** |
| 模型同轮是否看到 changes | 否（只回 `verification failed: {err}`，`handlers/apply_patch.rs:365-368`） | **是**（`format_for_model` 写"partially applied: N file(s)…"，`output.rs:154-173`） | 我们更详细 |
| 跨轮回放是否保留 changes | —（模型看历史 tool_result，codex 只留错误文本） | **否**（`replay_pair` 优先 `error_text`，丢掉 `result_json`，`context/mod.rs:340-358`） | **同轮跨轮不一致** |
| UI 是否显示 changes | 是（`FileChangeItem { changes, status }`，`tools/events.rs:275-300`） | 是（渲染层有摘要） | 一致 |

**修正初版结论**：这不是"不如 codex"，而是**我们自己的两条路径打架**——同一轮告诉模型"partially applied: 1 file(s) were already changed on disk"，下一轮却只剩一句 `hunk lines do not match…`。模型会误以为这条信息还在上下文里。

**建议**：选一个口径并统一。推荐**两处都带**（信息更完整）：`error_text` 落库时追加 partial 摘要，或让回放对失败调用优先用 `result_json`（它含结构化 `changes`）。

### 1.5 原子写与覆盖语义

| | codex | lx | 判定 |
| --- | --- | --- | --- |
| 写实现 | `fs.write_file` 直接写（`exec-server/src/local_file_system.rs:174`），靠 `delta.exact` 兜不确定性 | 同目录临时文件 + `sync_all` + rename（`platform/atomic_write.rs:16-30`） | **我们更好** |
| `Add File` 目标已存在 | **静默覆盖**，只记 `overwritten_content`（`lib.rs:487-516`） | **拒绝**并提示改用 `*** Update File`（`edit.rs:70-82`） | **我们更好** |
| `Move to` 目标已存在 | **静默覆盖**，但把 `overwritten_move_content` 记进 delta 并上报（`lib.rs:583-600`） | **静默覆盖，完全不上报**（`edit.rs:93-109`、`:231-259`） | **我们更差** |
| 符号链接 | `follow_symlinks` 可配 | 写真实目标而非替换链接（`path.rs:64-74`） | 一致（我们更安全） |

`Add File` 与 `Move to` 两处的**不对称**很扎眼：同一类"目标已存在"的风险，一个拦、一个放。实测（§9 探针 11）：

```text
from.txt = "new content\n";  to.txt = "PRECIOUS USER DATA\n"
patch: Update File: from.txt / Move to: to.txt
结果: to.txt = "new content v2\n"    ← 原内容被抹掉
      FileChange { kind: Moved { to } }  ← 只说"移动"
```

**建议**：`Move to` 目标已存在时按 `Add File` 的口径处理（拒绝），或至少把"覆盖了 X"写进 `format_for_model` 与 `FileChange`。

### 1.6 大小上限

| 项 | codex | lx | 判定 |
| --- | --- | --- | --- |
| 读文件输入 | **512 MiB 硬上限** + 分块 + 可取消（`exec-server/src/local_file_system_read.rs:13,31-36`） | **无上限**（`read.rs:54-56` 全量 `fs::read`） | 我们缺 |
| 目录条目 | **50 000 上限并报错**（`exec-server/src/server/file_system_handler.rs:55,285`） | **无上限**（`read.rs:135-163` 全收进 Vec 再排序） | 我们缺 |
| 结果输出 | `EXEC_OUTPUT_MAX_BYTES = 1 MiB`（`core/src/exec.rs:81` + `utils/pty/src/lib.rs:25`） | `MAX_TOOL_RESULT_BYTES = 1 MiB`（`chat/tools/mod.rs:66`） | **刻意一致** |
| 参数字节 | —（freeform，无 JSON 参数） | 1 MiB，内存与 DB 双层由测试锁定 | 我们更好 |

实测（§9 探针 9）：34.9 MB 文件 + `limit:10` → 仍整文件读入内存。

**建议**：读前 `metadata().len()` 判上限（可参考 codex 的 512 MiB，或更保守的 16 MiB + 明确错误）；目录读取加上限。

### 1.7 增删计数的来源

codex 用 **diffy 精确统计**（`tui/src/diff_render.rs:815-830`）：

```rust
diffy::Patch::from_str(diff) → 逐 hunk 逐行数 Insert/Delete
```

lx 用**前后缀剥离估算**（`chat/tools/text.rs:135-152`）。实测（§9 探针 10）：

```text
20 行文件，只改第 2 行和第 19 行  →  added=18 removed=18
```

**建议**：`edit` 分支直接用补丁里的 `+`/`-` 行数累加（`edit.rs` 的 `block_new_lines`/`block_old_lines` 已给出每块数量）；`write` 整体覆盖保持现状（那里语义上确实是整体替换）。

### 1.8 展示语法（确认我们的选择）

| 项 | codex | lx | 判定 |
| --- | --- | --- | --- |
| 搜索详情 | `query` + `" in ".dim()` + `path`（`tui/src/exec_cell/render.rs:377-383`） | `"pattern" in path` 合成**一段**金黄（`tool.rs:231-264`） | 文案一致，**着色粒度不同** |
| 是否显示命中数 | 否（只在非零退出时给 `(exit N)`，`exit 1` 特意不当失败） | 显示 `· N matches` | 我们跟 opencode |
| 增删计数格式 | `(+12 -4)`（`diff_render.rs:432-440`） | `· +12 -4` | 一致 |
| 连续 read | 合并成一行 `Read a, b, c`（`render.rs:363-374`） | 每个调用独立两行 | 我们不复用聚合 |
| 动词时态 | `Exploring`/`Explored`、`Ran` | `Read`/`Search`/`Edit`/`Write`（现在式） | 已定稿，不改 |

搜索详情的**着色粒度**差异是 P2-7 的根因：codex 是 `query` 与 `" in "` 与 `path` 三段独立 `Span`，窄宽度时可以只丢一段；我们是拼接后一个整体 `ellipsize_middle`，窄宽度时会切出 `"测…txt` 这种残缺。实测（§9 探针 12）：

```text
w=30  └─ "测试"…ル.txt · 3 matches      ← in 没了
w=24  └─ "测…txt · 3 matches            ← pattern 只剩 "测
```

**建议**：拆成三段（pattern 段 / `in` 段 / path 段），让位顺序「丢 path 及 `in` → 截断 pattern 尾部」。这既修 bug，也让渲染结构与 codex 对齐。

## 2. P1：必须先修的问题

### P1-1 `read` / 目录读取重进会话后丢详情（G14 只修了一半）

G14 的核心承诺写在 `docs/standards/tui-design-requirements.md`：

> **重进会话的工具行必须与实时一致**：摘要 = 参数派生（`path`/`pattern`/`range`）+ **结果体补全**（`added`/`removed`/`matches`/`total_lines`/`truncated`，见 `ToolSummary::absorb_result_json`）

但 `absorb_result_json`（`src/app/state/lx/chat/steps.rs:188-259`）**只补 `path` / `matches`（顶层数组长度）/ `added` / `removed` / `total_lines` / `truncated`，从不补 `range`**；目录读取的条目数走的是 `matches` 字段，而结果体里没有 `matches` 数组（`kind: "directory"` 只有 `total_lines`）。

| 调用 | 实时（`to_summary`） | 重载（`from_arguments().absorb_result_json()`） |
| --- | --- | --- |
| `read {"path":"src/a.rs"}`（最常用） | `└─ src/a.rs · 1-40 · (40 lines)` | `└─ src/a.rs` ← **区间与行数全丢** |
| `read {"path":"src"}`（目录） | `└─ /w/src · 2 matches` | `└─ /w/src` ← **条目数丢失** |

探针证据（§9 第 1、3 条）：

```text
LIVE    └─ src/a.rs · 1-40 · (40 lines)
RELOAD  └─ src/a.rs
DIR-LIVE   └─ /w/src · 2 matches
DIR-RELOAD └─ /w/src
```

根因链：`to_summary`（`output.rs:216-221`）的 `range` 来自结果体，而 `from_arguments`（`steps.rs:131-180`）只能从 `offset`/`limit` 推区间——**不带 `offset` 就没区间**；`absorb_result_json` 既不读 `start_line`/`end_line`，也不在 `kind=="directory"` 时把 `total_lines` 映射成 `matches`。

为什么现有测试没抓到：`src/app/update/chat/view/tests.rs:140-235` 的五条 reload 测试，`read` 那条刻意带了 `offset:12,limit:29` 并只断言 `total_lines`（不查 `range`），刚好绕过缺口。

**建议修复**（`absorb_result_json` 内，约 12 行）：补 `range`（`kind=="file"` 且 `start_line>0`）、补目录 `matches`；加 3 条测试。

### P1-2 `search` 不排除 `.git/`，且忽略规则随搜索根漂移

见 §1.3 的完整对照。两条独立缺陷：

1. **缺 `.git/` always-ignore**（`search.rs:150-158` 只有 `hidden(false)`）→ `.git/config` 的凭据进请求正文与 `result_json` 落库；
2. **`parents(false)`** → 从 `sub/` 搜时父级 `.gitignore` 失效（root 1 hit / sub 2 hits），工具描述却说"Respects `.gitignore`"。

附带后果：本仓实测 `files_scanned=1029`（含 `.git/`），直接吃掉 200 hits / 20k files / 10s 预算。

### P1-3 失败补丁的部分成功信息，同轮给了、跨轮丢了

见 §1.4。`format_for_model`（`output.rs:154-173`）同轮会写"partially applied: N file(s) were already changed on disk"，但 `error_text` 落库只有 512 字符的 message（`event/chat/mod.rs:727-732`），而 `replay_pair`（`context/mod.rs:340-358`）对失败调用**优先 `error_text`、丢弃 `result_json`**。

### P1-4 `(N lines)` 是文件总行数，不是本次读到的行数

三处口径互相矛盾：G14 决策与 `design.md:1426` 写"实际读到的行数"；`lines_label` 注释（`ui/text/agent.rs:148-152`）也这么写；实现却打印 `total_lines`（`output.rs:221` → `tool.rs:305-313`）。

```text
read {"offset":12,"limit":29}，文件 1000 行
└─ src/big.rs · 12-40 · (1000 lines) · …[truncated]
```

`12-40` 与 `(1000 lines)` 并列会被读成"读了 1000 行"，实际 29 行。整文件读取时两者还完全重复：`1-40 · (40 lines)`。

**建议**：摘要增加 `read_lines`（= `end_line - start_line + 1`），`(N lines)` 用它；`total_lines` 只留给目录条目数。若要显示"共 M 行"，另起一段（如 `· of 1000`）并同步两份文档。

## 3. P2：严重问题

### P2-1 多文件 `edit` 实时行出现孤立 `· `

`to_summary` 的 `Edit` 分支用 `single_path`（`output.rs:274-279`）——多文件时 `path = None`；而 `extras()`（`tool.rs:279-293`）给每个次要段无条件加 `{sep}`：

```text
EDIT-LIVE    └─  · +12 -4      ← 孤立分隔符，且丢失全部文件名
EDIT-RELOAD  └─ a.rs · +12 -4   ← 重载反而更好（从 patch 取到第一个文件）
```

实时比重载更差，而多文件补丁正是 `edit` 的主要用途。

**建议**：`extras` 拼装时按"当前是否已有可见前置段"决定是否加 `sep`；多文件时 `path` 改为 `2 files` 或首个文件名 + `…`。

### P2-2 `search` 深度上限静默截断

`MAX_DEPTH = 32`（`search.rs:38`、`:159`）。探针（§9 第 8 条）：40 层深的 `deep.txt` 未命中，`truncated` 仍 `false`。违反模块自己声明的契约（`mod.rs:9-10`、`output.rs:13-15`）。**建议**：触发时置 `truncated`。

### P2-3 `read` 无输入字节上限；目录读取无条目上限

见 §1.6。`read.rs:54-56` 全量 `fs::read`；`read.rs:135-163` 把整个目录收进 Vec 再排序。codex 分别有 512 MiB 与 50 000 的硬上限。

### P2-4 `edit` 的 `Move to` 静默覆盖目标文件

见 §1.5。与自家 `Add File` 的拒绝策略自相矛盾。

### P2-5 多 hunk 的增删计数失真

见 §1.7。

### P2-6 `offset`-only / `limit`-only 的区间错误

`steps.rs:161-171`：只有 `offset` 时区间塌成 `(start, start)`；只有 `limit` 时**没有区间**。

```text
read {"offset":12}   live 12-1000   reload 12-12
read {"limit":29}    live 1-29      reload 无区间
```

`limit`-only 是最常见的"读前 N 行"写法。**建议**：与 P1-1 合并修——把 `range` 的来源统一交给结果体。

### P2-7 窄宽度下 `search` 详情残缺

见 §1.8。`param()`（`tool.rs:231-264`）把 `"pattern" in path` 合成一个 `Segment`，`ellipsize_middle` 对整串做中间省略（`tool.rs:493`），最窄档把 pattern 切碎——而 pattern 是这一行唯一必须保住的信息。

### P2-8 结算路径与重载路径的合并口径不一致

`settle_tool_result`（`stream.rs:281-290`）注释说"合并一次参数派生值（pattern / path / range）"，实际**只合并了 `pattern`**；重载路径（`view.rs:72-73`）是 `from_arguments().absorb_result_json()` 全量合并。这直接造成 P2-1（实时丢 path、重载有 path），也让 `param()` 在实时多文件 edit 时退化成"每帧重新解析 1 MiB patch JSON"。

**建议**：抽一个共用合并函数，两条路径共用；加一条「同一调用：实时摘要 == 重载摘要」的表驱动测试。

### P2-9 不支持 `*** End of File`

见 §1.1。codex grammar 明确支持，GPT-5 系列会产出该标记。**建议**：解析时接受（实现"锚定末尾"或至少忽略）。`design.md:605` 的"不做项"需要升级为"兼容性风险 + 处理方式"。

### P2-10 只有精确匹配、强制唯一，缺少 rstrip 回退

见 §1.1。建议补 rstrip 一档，并在 `PatchContextMismatch` 消息里给出候选行号。

### P2-11 纯插入语句落到文件第 0 行

见 §1.2。语义未文档化且与 codex 相反。

## 4. P3：可维护性问题

1. **`has_summary` 手写六字段 or**（`view.rs:74-79`）：新增字段忘了加进 `||` 链就会静默回归 P1-1。建议 `impl ToolSummary { fn is_empty(&self) }`。
2. **JSON 形状知识写了两遍**：`to_result_json`（`output.rs:296-380`）与 `absorb_result_json`（`steps.rs:188-259`）各自手写字段名/嵌套形状（`changes[].added` 汇总、`kind` 分支）。这是唯一"写端与读端必须同步、编译器不管"的接缝，也是 G14 bug 的根源。建议加**往返测试**：四工具各构造真实 `ToolExecutionResult` → `to_result_json` → `from_arguments(参数).absorb_result_json()` → 断言与 `to_summary` 逐字段相等。
3. **`needs_spacer` 恒 `true`**（`history.rs:174-181`）：参数改成 `_previous`/`_next`，`Block` 枚举成了纯占位。另外恒 `true` 意味着连续两个 text part 之间也会空一行（DB 加载出多个相邻 text part 时可见）。
4. **`tool.rs` 模块文档与实现漂移**：`:12` 仍写"参数（路径金黄**下划线**）"，`:16` 仍写"第 2 行**永不为空**"（完成态无摘要时已可空）。
5. **`style::agent_param()` 仍带下划线**（`style.rs:162-166`），`chat_tool_param()` 没有；`style/tests.rs:125-129` 只比较 `.fg`，把差异掩盖了。
6. **`param()` 每帧重解析参数 JSON**：最坏情况（多文件 edit 的 1 MiB patch）每帧一次 JSON 解析。修完 P2-8 后消失，但记一笔：渲染层不该做解析。
7. **`search` 两段各自的 10s deadline**（`search.rs:147` 与 `:195`）：单次调用最坏 20s，而文档写的是一个 `SEARCH_TIMEOUT`。
8. **写锁表只增不减**（`lock.rs:12-15`）：按写入路径数单调增长（条目很小，优先级低）。
9. **`edit` 对同一文件读两次**（`edit.rs:211` 与 `:217`）：先 `read_text` 再 `std::fs::read` + `parse`，两次读之间有窗口。合并成一次即可。
10. **`Edit.partial` 在 UI 完全不可见**：`to_summary` 忽略 `partial`（`output.rs:232-247`）。

## 5. 文档 / 规范一致性

| 位置 | 现状 | 建议 |
| --- | --- | --- |
| `tui-design-requirements.md`（工具行条目） | 写"`read` 为 `path · 12-40 (31 lines)`（请求区间 + 实际读到的行数）" | 与 P1-4 一起改 |
| `design.md:1426` | "`(31 lines)` 用括号区分『哪一段』与『多大一块』" | 同上；当前实现给的是总行数 |
| `design.md:605` | 把 `*** End of File` 等列为"不做项" | 升级为兼容性风险（P2-9） |
| `design.md:1449` | 声称 G14「测试证据」含 5 条 reload 回归 | 漏了无 offset 的 read / 目录 read（P1-1） |
| `chat/tools/schema.rs:24-27` | "Respects `.gitignore`/.ignore rules" | 与 P1-2 一起改 |
| `chat/tools/schema.rs`（edit 描述） | 只写"Complete patch text from … to …" | 补纯插入落点、唯一匹配、不支持 End of File 等语义（P2-11、P2-10、P2-9） |
| `tool.rs:12,16` | 仍写下划线 / 第 2 行永不为空 | 见 P3-4 |
| `task.md:11,138,139` | 3 项未勾（用户审核意见、人工验收清单、最终报告） | 本次 review 可作为第 11 项的输入 |

另外注意：主检出 `/Users/yonah/projects/agent/lx-tui/docs/feature/lx-agent-file-tools/` 下有一份**未被 git 跟踪的旧版** `design.md`（584 行，止于 §19），而分支里是 1471 行（含 §20–§24）。合并前请先删除/覆盖，否则 checkout 会被挡下，或误留两份互相矛盾的设计文档。

## 6. 测试覆盖缺口

已覆盖得很细（`tool/tests.rs` 29 项、`view/tests.rs` 11 项、`tool_geometry.rs` 10 项），但下面这些**正是本轮暴露出来的洞**：

| 缺的测试 | 能抓到的 bug |
| --- | --- |
| `reload_keeps_read_range_without_offset`（断言 `range == Some((1,40))`） | P1-1 |
| `reload_keeps_directory_entry_count` | P1-1 |
| `reload_with_offset_only_does_not_collapse_range` / `limit_only` | P2-6 |
| `live_and_reloaded_summaries_are_identical`（表驱动，四工具 × 有/无 offset × 目录/文件 × 多文件 edit） | P1-1、P2-1、P2-8 一次性锁死 |
| `to_result_json_round_trips_into_summary` | P3-2：编译器不管的接缝 |
| `search_skips_git_dir`（fixture 含 `.git/config`，断言不命中） | P1-2 |
| `search_respects_parent_gitignore_from_subdirectory` | P1-2 |
| `search_depth_limit_sets_truncated` | P2-2 |
| `read_rejects_file_larger_than_input_cap` / `directory_read_caps_entries` | P2-3 |
| `move_to_existing_target_is_reported` | P2-4 |
| `failed_multi_file_patch_replay_keeps_changes`（重开 session 后回放仍含已改文件） | P1-3 |
| `multi_file_edit_detail_has_no_orphan_separator` | P2-1 |
| `search_detail_keeps_pattern_at_narrow_widths` | P2-7 |
| `multi_hunk_edit_reports_patch_line_counts`（20 行改 2 行 → `+2 -2`） | P2-5 |
| `patch_accepts_end_of_file_marker` | P2-9 |
| `patch_matches_after_trailing_whitespace_difference` | P2-10 |
| `pure_insert_without_context_appends_to_end` | P2-11 |

另建议加一条 **codex 兼容 fixture 集**：把 `codex-rs/apply-patch/src/parser.rs:290-520` 里的官方样例补丁（含 `@@ def f():`、`*** Move to:`、多 hunk）搬成 fixture，逐条断言"我们接受/我们明确报错"。这样"兼容子集"的边界从注释变成可执行断言。

## 7. 合并前注意

1. **先修 P1**：P1-1/P1-2/P1-3/P1-4 都在"已经声称做完"的范围内，直接合进 `dev` 会让上一条不变量变成假承诺。
2. **P2-9/P2-10/P2-11 是本轮新增的语义问题**，其中 P2-9 与 P2-11 会**静默产生错误结果或整轮失败**，建议与 P1 同批处理。
3. **untracked 旧文档**：见 §5 末尾。
4. **既有 flake**：`tests/prompt_copy.rs::cmd_shift_c_copies_template_block_without_title` 是 PTY 时序 flake，与本分支无关；本次全量 2374 项全绿未复现。
5. **人工验收仍未做**（`task.md:138`）：四类供应商各一次工具调用、思考先完成、loading 先出现、同轮多工具、`/resume`、异常中断后的历史、长路径与窄窗口。P2-7 正是只在真实终端宽度下才看得到的那类。

## 8. 建议的修复顺序与工作量

| 顺序 | 内容 | 估计 |
| --- | --- | --- |
| 1 | P1-1 + P2-6：`absorb_result_json` 补 `range`/目录 `matches`，参数侧不再猜区间；加 3 条 reload 测试 | 小（~40 行 + 测试） |
| 2 | P1-4：摘要加 `read_lines`，`(N lines)` 改用它；同步两份文档 | 小 |
| 3 | P2-11 + P2-9：显式定义纯插入落点、接受 `*** End of File`；写进工具描述 | 小~中 |
| 4 | P1-2：`.git/` always-ignore + `parents(true)`；加安全测试 | 小（但要定隐藏文件语义） |
| 5 | P2-8 + P2-1：统一 `settle`/reload 合并口径；`extras` 无前置段时不加 `sep` | 中（~60 行） |
| 6 | P1-3：`error_text` 带 partial 摘要（或回放改用 `result_json`） | 小 |
| 7 | P2-10：rstrip 一档 + 候选行号提示 | 小~中 |
| 8 | P2-2 / P2-3 / P2-4 / P2-5：显式标志与上限 | 中 |
| 9 | P2-7：`search` 详情分段让位（结构对齐 codex） | 中 |
| 10 | P3 全清（`is_empty`、往返测试、文档漂移、死抽象）+ codex 兼容 fixture 集 | 中 |

## 9. 探针复现清单

以下探针都在 worktree 内临时追加、跑完即删除（`git status` 干净），未改任何生产代码。

| # | 位置 | 探针内容 | 结论 |
| --- | --- | --- | --- |
| 1 | `app/update/chat/view/tests.rs` | `read {"path":"src/a.rs"}` + 结果体 `start_line:1,end_line:40` | reload `range=None`（P1-1） |
| 2 | 同上 | `offset:12,limit:29`，文件 1000 行 | 渲染 `12-40 · (1000 lines)`（P1-4） |
| 3 | 同上 | `kind:"directory"`，`total_lines:2` | reload `matches=None`（P1-1） |
| 4 | 同上 | `offset:12` only / `limit:29` only | `12-12` / 无区间（P2-6） |
| 5 | `ui/main_content/lx/history/tool/tests.rs` | 多文件 edit `to_summary` 现场构造 | `└─  · +12 -4`（P2-1） |
| 6 | `chat/tools/search/tests.rs` | fixture 含 `.env` / `.git/config` | 两者都命中（P1-2） |
| 7 | 同上 | 根 vs `path:"sub"` + 父级 `.gitignore` | 1 hit vs 2 hits（P1-2） |
| 8 | 同上 | 40 层目录嵌套 | `truncated=false` 且未命中（P2-2） |
| 9 | `chat/tools/read/tests.rs` | 34.9 MB 文件 + `limit:10` | 全量读入（P2-3） |
| 10 | `chat/tools/edit/tests.rs` | 20 行文件两个 `@@` 块 | `+18 -18`（P2-5） |
| 11 | 同上 | `Move to` 已存在目标 | 静默覆盖（P2-4） |
| 12 | `ui/main_content/lx/history/tool/tests.rs` | `search` 详情宽度 70→6 | 窄档 pattern 被切碎（P2-7） |
| 13 | 同上 | 长路径 + `range` + `total_lines` | `w=32` 起 `(N lines)` 先丢，`1200-1240` 保留（符合设计优先级） |
| 14 | `chat/tools/edit/tests.rs` | 补丁含 `*** End of File` | `patch_invalid`（P2-9） |
| 15 | 同上 | 标记前后带空格 / 上下文行尾空格 / Unicode 引号 | 三者全失败（P2-10） |
| 16 | 同上 | `@@` + 仅 `+tail`（无上下文） | 插到第 0 行（P2-11） |
| 17 | 同上 | 两个 hunk 指向同一文件 / `Add File` 已存在 | 两者都被拒绝（**优点**，与 codex 对照见 §1.5） |

本机 `ripgrep 15.1.0` 对照实验（`/tmp/rgt`，用于确认 P1-2 的目标行为）：

```text
rg SECRET              → sub/x.txt                     （隐藏文件不搜，.gitignore 生效）
rg --hidden SECRET     → .env, sub/x.txt               （仍跳过 .git/）
rg -uuu SECRET         → 全部（含 .git/）
rg SECRET sub          → sub/x.txt                     （父级 .gitignore 仍生效）
```

全量验证（本次审查期间执行）：

```text
cargo fmt --check                                  → 0
cargo clippy --all-targets -- -D warnings          → 无 warning/error
cargo test                                         → 2374 passed, 0 failed（lib 2328 + 14 个集成二进制）
```
