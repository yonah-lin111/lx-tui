# lx Recap、统计行重排、系统事件撤销与 AI Web 生图 — Review 与修复清单 (`review.md`)

- **特性目录**：`docs/feature/lx-recap-and-turn-summary/`
- **Review 基线**：`feat/lx-recap-and-turn-summary` @ `b7d56d15`（相对 `dev`）
- **修复提交**：`8423e37e`
- **合并状态**：已合并至 `dev`，临时 worktree `.worktrees/lx-recap-and-turn-summary`、分支 `feat/lx-recap-and-turn-summary` 及隔离开发库已移除
- **关联文档**：
  - [design.md](design.md)
  - [task.md](task.md)
  - 项目规范：`docs/standards/{project-directory-structure,code-writing-standards,tui-design-requirements,html-assets,web-deepseek}.md`
- **Review 范围**：
  - Recap 数据库迁移（v25）、生成请求、自动失焦调度、手动 `/recap`、设置页开关与专用模型
  - 系统事件（`ModelSwitch` / `Compact` / `Recap`）`/rewind` 撤销、compact 检查点物理删除、底部 compact 禁 `/recap`
  - 消息尾部统计行单行降级、输入框状态行整会话累计缓存命中率、移除 `FileChangesCard` 与首页快捷键
  - AI Web（Gemini / Grok）实时生图预览落盘、预览树下划线定位资源管理器、原图下载（黄色 `[downloading...]`）与 `image_download_dir` 配置
- **结论**：未发现 P0；发现 **3 个 P1、12 个 P2、10 个 P3**，全部按优先级执行修复并通过回归测试。

---

## 0. 优先级汇总

| 级别 | 数量 | 判定标准 | 状态 |
| :--- | :---: | :--- | :---: |
| **P0** | 0 | 崩溃、数据损坏、安全漏洞、主流程完全不可用 | — |
| **P1** | 3 | 用户可见功能卡死、核心自动触发永久失效、历史会话阻塞 `/recap` 与 `/rewind` | 已修复 |
| **P2** | 12 | 边界条件、多窗格同步、缓存/状态行计算与布局覆盖、跨平台路径、生产文件超 1000 行上限、文档漂移 | 已修复 |
| **P3** | 10 | 状态细节、死字段/可见性残留、JS `try/finally` 防御、CLI 参数补全、注释与文案修正 | 已修复 |

---

## 1. P1（关键路径缺陷）

### [P1-1] `src/app/update/chat/web.rs:458-551` — 原图下载在会话绑定前发起、绑定后返回时，按钮永久卡在 `[downloading...]`

- **关键点**：
  - `trigger_ai_web_original_image_download` 将点击时刻的 `orig.session_id` 冻结进 `Effect::DownloadAiWebOriginalImage`。
  - `on_ai_web_original_image_downloaded` 与 `on_ai_web_original_image_download_failed` 均要求 `target.lx.chat.session_id.as_deref() == Some(session_id)` 才更新按钮状态。
  - `gemini_script.js` 与 `grok_script.js` 在首次对话完成时先发出 `image` 事件、后发出 `session` 事件（`SessionBound`）。
- **错误原因**：
  - 新会话第一轮生图时，`image` 事件到达时尚未绑定云端会话 ID，`orig.session_id` 为本地临时 ID（`ai-web-<pane>-<gen>`）。
  - 若用户在 `SessionBound` 到达前（Gemini 最多 1.8s、Grok 最多 5s）点击 `[Download original]`，下载效应携带的是临时 `session_id`；随后 `on_ai_web_session_bound` 将窗格 `chat.session_id` 和内存中的 `orig.session_id` 更新为真实云端会话 ID。
  - 待下载完成或失败事件返回时，事件携带的旧临时 `session_id` 与窗格当前真实 `session_id` 不匹配，成功回执直接返回 `false` 丢弃，失败回执也跳过 `orig.state = Idle` 恢复，导致按钮永久停留在 `[downloading...]`。
- **修复**：
  - 在 `on_ai_web_original_image_downloaded` 与 `on_ai_web_original_image_download_failed` 中按 `attachment_id` 在 `target.lx.chat.image_originals` 中定位目标项，只要该 `attachment_id` 存在于当前窗格且当前窗格处于同一 AI Web 会话（匹配窗格 `session_id` 或 `orig.session_id`），即正常更新状态并展示 Toast。
  - 新增“首轮 pending 会话点击下载 -> SessionBound 绑定真实会话 -> 下载完成/失败回执”回归测试。

### [P1-2] `src/app/update/chat/recap.rs:95-115, 222-246` — 自动 recap 计时器在“轮次运行中 + 轮次/compact 条件暂不满足”时被永久清空

- **关键点**：
  - `tick_recap` 到期时先执行 `state.recap_due_at = None`，随后调用 `auto_eligibility(state)`。
  - `auto_eligibility` 先检查 `!target.lx.recap.auto_eligible()`（`completed_turns >= 3` 且距上次 recap 至少 2 轮）与 `compact_at_bottom(&target.lx)`，最后才检查 `target_busy(target) || session_busy(state, pane)`。
  - 此外 `auto_eligibility` 要求 `state.focused == FocusArea::Panes`。
- **错误原因**：
  - 当会话仅有 2 个已完成轮次且第 3 轮正在长耗时运行，或底部刚执行 `/compact` 后正在运行新一轮时，用户切走终端满 30 分钟触发 `tick_recap`，会因 `!auto_eligible()` 或 `compact_at_bottom()` 判定为 `Ineligible`（而非 `Retry`），从而不再重设 `recap_due_at`。
  - 待该轮流式结束时，`on_stream_finished` 仅当 `state.recap_due_at.is_some()` 时才会顺延计时器，导致自动 recap 在本次失焦期间永久失效。
  - 同时，若用户切走终端前焦点在右侧 Prompt / Todo 面板或左侧 Sidebar，`state.focused != FocusArea::Panes` 也会直接返回 `Ineligible`。
- **修复**：
  - 在 `auto_eligibility` 中将 `target_busy(target) || session_busy(state, pane)` 移到 `auto_eligible()` 与 `compact_at_bottom()` 之前，在途运行期间统一返回 `Eligibility::Retry`（30 秒后复检）。
  - 移除 `state.focused == FocusArea::Panes` 限制，直接取当前活动工作区活动标签页的 `focused_pane` 若为 `PaneView::Lx` 即纳入自动 recap。

### [P1-3] `src/app/update/chat/rewind.rs:200-201, 341`、`src/app/update/chat/recap.rs:265-278`、`src/db/chat/compact.rs:106-124, 292-303` — 历史遗留 `state='reverted'` 的 compact 检查点仍被视为最新生效检查点

- **关键点**：
  - `compact_at_bottom`、`select_latest`、`select_compact` 均直接取 `lx.compact.checkpoints.last()`，未过滤 `c.active`。
  - `Database::revert_compact` 检查“是否存在更晚检查点”的 SQL 未限定 `AND state = 'active'`，且删除最新检查点后未将前一条检查点（若存在）恢复为 `active`。
  - 旧版数据库在撤销 compact 时将记录标为 `state='reverted'` 保留在表中，升级后仍会被加载进 `checkpoints` 末尾。
- **错误原因**：
  - 本分支将撤销 compact 改为物理删除行，但未清理旧库遗留的 `state='reverted'` 记录，也未在内存选择器中过滤 `active` 状态。
- **修复**：
  - `compact_at_bottom`、`select_latest`、`select_compact` 统一只检查 `c.active` 的检查点（`checkpoints.iter().rev().find(|c| c.active)`）。
  - `Database::revert_compact` 的更晚检查点查询增加 `AND state = 'active'`，并在删除目标检查点后将该会话剩余最新一条检查点（若有）置为 `state = 'active'`。
  - `migrate_v25_recap` 与 `compact_checkpoints` 查询过滤/清理 `state = 'reverted'` 的历史残留行。

---

## 2. P2（功能边界、多窗格一致性、布局与规范约束）

### [P2-1] `src/app/update/chat/recap.rs:389-448` — `recap_exchanges` 未排除子 agent 报告自动轮

- **关键点**：`recap_exchanges` 只检查 `kind == "chat"`、`status == "complete"` 与 `!is_user_shell()`，未过滤 `!message.reports.is_empty()` 的子 agent 报告合成用户消息；`message_text` 拼接多段文本时用 `join("")` 缺少换行。
- **错误原因**：子 agent 报告自动轮在 DB 中以 `role = "user"`、`kind = "chat"` 存储，其正文为 `<subagent_report>` 块，被当成真实用户输入送入 recap 提示词，违反设计 D1/D8“不读子 agent 报告”的约束。
- **修复**：在 `recap_exchanges` 中跳过 `!message.reports.is_empty()` 所在轮次；`message_text` 多段文本改用 `"\n"` 拼接。

### [P2-2] `src/app/update/chat/recap.rs:176-188, 280-293`、`src/app/update/chat/mod.rs:154-163`、`src/app/update/chat/compact.rs:39-44, 78, 210-257` — 多窗格共享会话时 recap 未互斥、取消与回填未跨窗格广播

- **关键点**：`session_busy` 未检查其他窗格的 `recap.pending`；发送新消息或执行 `/compact` 只取消当前窗格的 `recap.pending`；`apply_recap` 只更新发起窗格；`on_compact_reverted` 未清理被撤销的 `selected_compact` 与同会话其他窗格的失效 rewind 面板。
- **错误原因**：recap 与 compact 撤销的状态更新只处理了单个窗格，缺少按 `session_id` 的跨窗格同步。
- **修复**：`session_busy` 纳入 `other.lx.recap.pending.is_some()`；`cancel_pending_recap` 与 `apply_recap` 广播到同 `session_id` 的全部窗格；`on_compact_reverted` 同步清理同会话窗格的 `selected_compact` 与 rewind 选中态。

### [P2-3] `src/chat/recap.rs:74-79, 184-220` — `parse_generated_recap` 对带额外字段的 JSON 退化为返回 `"{"`，且超长文本直接报错丢弃

- **关键点**：`GeneratedRecap` 使用 `#[serde(deny_unknown_fields)]`，一旦模型多输出字段或外围带解释文字即解析失败，fallback `first_non_empty_line` 会将首行 `"{"` 当作摘要返回；此外 `summary` 超过 700 字符或 `next_action` 超过 200 字符时直接返回 `None`。
- **错误原因**：严格 JSON 反序列化叠加盲目取首行的 fallback，对真实模型输出容错不足。
- **修复**：移除 `deny_unknown_fields` 并支持提取首个 `{...}` JSON 对象；对超过 700 / 200 字符的 `summary` / `next_action` 按字符边界截断；纯文本 fallback 排除以 `{`、`[`、`"`、`}`、`]` 开头的 JSON 语法碎片行。

### [P2-4] `src/app/state/lx/chat/usage.rs:60-82` — 累计缓存命中率的分子与分母未按单条消息绑定

- **关键点**：`total_input` 与 `total_cache` 分别独立遍历求和；若某条消息缺失 `input_tokens` 但存在 `cache_read_tokens`，分子被计入而分母未计入。
- **错误原因**：未将 `cache_read_tokens` 约束在对应消息的 `input_tokens` 样本内。
- **修复**：仅在 `message.input_tokens.filter(|v| *v > 0)` 存在时配对累加 `input` 与 `cache_read.clamp(0, input)`。

### [P2-5] `src/ui/main_content/lx/status.rs:107-136, 298-360` — 状态行右侧 `omitted turns` 提示会覆盖 `context` 与 `cache` 字段

- **关键点**：`layout()` 计算候选字段时不知道 `omitted_turns` 的存在，`render()` 仅用 `notice_reserve` 缩短模型名，随后照常绘制 `variant`、`context`、`cache`，最后右对齐绘制 `omitted_turns` 造成文字重叠。
- **错误原因**：`notice` 预留宽度未在 `layout()` 阶段从可用宽度中扣除。
- **修复**：在 `render()` 中将扣除 `notice_reserve` 后的有效左半区矩形传入 `layout()`，使模型名与附加字段在剩余预算内按降级链自然收敛，彻底消除与右侧 `omitted_turns` 的重叠。

### [P2-6] `src/ui/main_content/lx/history/paint.rs:121-141, 615-635` — 助手图片无有效 `source_path` 时仍绘制下划线且未过滤非图片附件

- **关键点**：`AssistantRefLine::from_message` 未过滤非 `Image` 附件与空路径 `Some("")`；`draw_assistant_ref_lines` 无条件绘制下划线，当预览图未落盘时文件名带下划线但点击无响应。
- **错误原因**：绘制下划线未检查 `preview_path.is_some()`，且 `from_message` 未过滤空路径。
- **修复**：`from_message` 只保留 `MessageAttachmentKind::Image` 并将空串路径归一化为 `None`；仅在 `preview_path.is_some()` 时添加 `Modifier::UNDERLINED`。

### [P2-7] `src/chat/web/gemini_script.js:627`、`src/chat/web/grok_script.js:501`、`src/chat/web/gemini_helpers.js:138-163`、`src/chat/web/grok_helpers.js:180-205`、`src/app/update/chat/web.rs:371-391` — 实时生图复用同名旧缓存文件，且 `/rewind` 未清理被截断消息的 `image_originals`

- **关键点**：`saveGeneratedImages` 在发现目标路径已有同名 `gemini-r1-1.webp` 时直接复用返回；当用户 `/rewind` 编辑重发或临时会话落入 `temp` 目录时，新生成的图片不会覆盖旧文件，导致预览指向旧图而原图 URL 指向新图。同时 `rewind_ai_web` 截断消息时未移除 `image_originals` 中的旧条目。
- **错误原因**：实时生图与 `load_session` 历史加载共用了“存在即跳过写入”的缓存逻辑。
- **修复**：`saveGeneratedImages` 新增 `overwrite` 参数，实时流（`gemini_script.js` / `grok_script.js`）传 `overwrite = true` 强制覆盖写入最新图片，`load_session` 保持 `overwrite = false` 复用缓存；`rewind_ai_web` 截断消息后清理已不存在于 `messages` 中的 `image_originals` 条目。

### [P2-8] `src/chat/web/gemini_script.js:280-325` — Gemini 多图部分加载提前结束、坏图阻塞 300s、引用重排正文缩短导致 `caughtUp` 卡死

- **关键点**：
  1. `pendingImage` 仅判断 `loadedImgs.length === 0`，同一轮生成多张图时第 1 张完成即提前触发 `done`，而单张坏图会让 `pendingImage` 永久为真直至 300s 超时；
  2. 生成停止后若引用重排使最终 `bodyText.length < domText.length`，`caughtUp` 永远为 `false`。
- **错误原因**：缺少“全部图片加载完成 + 图片等待最大 tick 上限”以及“生成停止后稳定窗口放行 `caughtUp`”的兜底机制。
- **修复**：`pendingImage` 改为等待容器内全部图片完成且最多等待 250 ticks（约 15s）；当 `!isGenerating` 且文本已稳定达到阈值时放行 `caughtUp`。

### [P2-9] `src/chat/web/gemini_helpers.js:48-55` — `toGeminiOriginalImageUrl` 未先剥离 query 参数

- **关键点**：当传入的 `lh3.googleusercontent.com/gg/...` URL 已带 `?alr=yes` 时，`replace(/=[^=/]+$/, '')` 会错误截断 `?alr=yes` 中的 `=yes`，拼出非法 URL。
- **错误原因**：正则替换前未剥离 `?` 查询串。
- **修复**：先执行 `.replace(/\?.*$/, '')` 去掉查询参数，再剥离尺寸后缀并追加 `=s0-d-I?alr=yes`。

### [P2-10] `src/config/mod.rs:95-113`、`src/platform/clipboard_image.rs:109-124` — `resolved_image_download_dir` 与 `sessions_root_dir` 仅读取 `HOME`，Windows 下失效

- **关键点**：`resolved_image_download_dir` 与 `sessions_root_dir` 直接调用 `std::env::var_os("HOME")` 且只支持 `~/` 前缀，在 Windows（默认使用 `USERPROFILE` 与 `~\`）下会报错 `HomeMissing`。
- **错误原因**：未复用 `src/platform/paste_path.rs` 已有的跨平台 `home_dir()` 与 `expand_tilde_path()`。
- **修复**：在 `src/platform/mod.rs` 导出 `pub(crate) use paste_path::{expand_tilde_path, home_dir};`，并在 `resolved_image_download_dir` 与 `sessions_root_dir` 中统一使用。

### [P2-11] 8 个生产代码文件超过 1000 行上限

- **关键点**：
  - `src/chat/web/gemini_helpers.js`（1213 行）
  - `src/platform/mod.rs`（1046 行）
  - `src/db/chat/fork.rs`（1045 行）
  - `src/app/state/lx/chat.rs`（1041 行）
  - `src/event/mod.rs`（1009 行）
  - `src/app/update/chat/mod.rs`（1004 行）
  - `src/db/service.rs`（1002 行）
  - `src/db/chat/mod.rs`（1002 行）
- **错误原因**：新增功能代码直接合入既有大文件，突破 `AGENTS.md` 单文件 1000 行上限。
- **修复**：按职责拆分子模块/辅助文件，确保所有生产文件严格低于 1000 行。

### [P2-12] `docs/feature/lx-recap-and-turn-summary/{design,task}.md` 文档与实现漂移

- **关键点**：`design.md` 与 `task.md` 仍将已移除的 `FileChangesCard` 列为当前有效实现，且缺少系统事件 `/rewind`、底部 compact 禁 `/recap`、AI Web 生成图片预览与原图下载、`image_download_dir` 设置等章节。
- **错误原因**：后续需求变更未同步回写设计文档与任务清单。
- **修复**：同步更新 `design.md` 与 `task.md`。

---

## 3. P3（代码整洁度、防御性校验与文案细节）

| 编号 | 位置 | 关键点 / 错误原因 | 修复 |
| :--- | :--- | :--- | :--- |
| **P3-1** | `src/app/state/lx/recap.rs`、`src/app/update/chat/recap.rs` | `LxRecap::reset` 将 `next_generation` 归零，同会话重载时可能碰撞旧请求代号；`set_message_recaps` 清空非空集合时返回 `false`；`RecapPending.end_turn/end_sequence` 与 `LxRecap.last_recap_sequence` 只写不读 | `reset` 保留 `next_generation` 单调递增；修正 `set_message_recaps` 返回值；清理无用死字段 |
| **P3-2** | `src/app/update/chat/mod.rs`、`src/app/update/chat/slash.rs` | `try_send` 未在提交前立即取消 pending recap；`on_turn_committed` 在校验 generation 前就取消 recap；关闭 `recap_enabled` 时未清理自动计时器与在途自动请求 | `try_send` 立即取消 pending recap；`on_turn_committed` 先校验 generation；`apply_models_changed` 关闭开关时清理自动计时器与自动 pending |
| **P3-3** | `src/ui/main_content/lx/history/paint.rs`、`tool.rs`、`tool/file_diff.rs` | 删除 `FileChangesCard` 后残留 `UserRefLine.source_path` 死字段与 `file_diff` 不必要的 `pub(crate)` 暴露；`BlockPaint::bytes()` 未计入 `RefLines` 路径长度 | 删除 `UserRefLine.source_path`；收紧 `file_diff` 可见性为模块私有；补齐 `RefLines` 字节估算 |
| **P3-4** | `src/ui/main_content/lx/history/stats.rs`、`cache/signature.rs` | `0 < speed < 0.05` 会格式化为 `0.0 tok/s`；`list_signature` 未哈希 `message.turn_index` | 速度过滤门槛改为 `speed >= 0.05`；`list_signature` 补 `message.turn_index` |
| **P3-5** | `src/chat/web/grok_script.js`、`gemini_script.js` | 提取正文时临时隐藏 `.not-prose` / 按钮节点未使用 `try / finally` 保护 | 使用 `try { ... } finally { ... }` 确保节点样式与临时文本节点始终恢复 |
| **P3-6** | `src/chat/web/gemini_helpers.js`、`grok_helpers.js` | `download_image` 与 `saveGeneratedImages` 对 `DOWNLOAD_STEM` / `turnNum` 缺少防御性路径清洗 | 规范化 `turnNum` 为正整数，对 `DOWNLOAD_STEM` 执行 `path.basename` 与非法字符过滤 |
| **P3-7** | `src/cli/settings.rs`、`help.rs`、`args.rs`、`src/config/mod.rs` | `--image-download-dir` 未加入 `VALUE_OPTIONS`、`SETTINGS_HELP` 与 `settings show` 人类可读输出；`Config::from_json` 对 `"image_download_dir": null` 报错 | 补齐 `VALUE_OPTIONS`、help 与 `settings show` 输出；`from_json` 支持 `null` 视为空串 |
| **P3-8** | `assets/settings.html:193` | 设置页 Recap 开关文案 `"Generate a recap after each assistant reply"` 与“离开 30 分钟后自动生成”不符 | 改为 `"Automatically generate a recap after 30 minutes away"` |
| **P3-9** | `src/platform/mod.rs:467-480` | Linux 下 `reveal_in_file_manager` 对无父目录的裸文件名会取出空串 `""` 传给 `xdg-open` | 空父目录回退为 `Path::new(".")` |
| **P3-10** | `src/ui/main_content/lx/history.rs`、`status.rs`、`src/db/chat/compact.rs`、`src/app/state/lx/compact.rs` | 模块头注释仍描述“统计行含缓存命中”“compact 撤销保留 state=reverted”等旧行为 | 同步更新模块文档注释 |
