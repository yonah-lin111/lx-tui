# 会话与模板块标题生成（含 Settings Model/Provider 拆分）：执行任务清单

- 状态：**已在工作树 `.worktrees/title-generation`（分支 `feat/title-generation`）完成全部任务与测试，待用户确认合并**
- 关联设计：`docs/feature/title-generation/design.md`

---

## Task 1：扩展 `AiSettings.title_summary` 与设置页 `Model` / `Provider` Tab 拆分

- **目标文件**：
  - `src/config/ai/mod.rs`
  - `src/app/state/lx/chat.rs`
  - `src/settings/service/models/mod.rs`
  - `assets/settings.html`
  - `assets/settings.js`
  - `assets/settings_models.js`
  - `src/settings/service/tests.rs`
  - `src/settings/service/models/tests.rs`
- **关键改动点**：
  1. **`src/config/ai/mod.rs`**：
     - 在 `AiSettings` 中新增 `#[serde(default, rename = "titleSummary", skip_serializing_if = "Option::is_none")] pub title_summary: Option<ModelSelection>`。
     - 提取 `validate_selection(role: &str, selection: &ModelSelection, providers: &BTreeMap<String, ProviderConfig>, enabled: &HashSet<String>) -> Result<(), AiConfigError>`，同时校验 `default_model` 与 `title_summary`。
  2. **`src/app/state/lx/chat.rs`**：
     - 在 `ModelsSnapshot` 新增 `title_selection()`、`default_selection()`、`resolve_prompt_title_model()` 与 `resolve_session_title_model(fallback: &ModelSnapshot)`。
  3. **`src/settings/service/models/mod.rs`**：
     - `revision_of` 纳入 `"titleSummary": &settings.title_summary`。
     - `get_settings` 响应 JSON 新增 `"titleSummary"`。
     - `merge_settings` 解析并合并 `"titleSummary"`。
  4. **`assets/settings.html`、`assets/settings.js`、`assets/settings_models.js`**：
     - 左侧导航栏拆分为 `Model`（`#tab-model` $\to$ `#view-model`）与 `Provider`（`#tab-provider` $\to$ `#view-provider`）。
     - `#view-model` 内含 `Default model`（`#default-model-host`）、`Title model`（`#title-model-host`）与 `Title language`（`#title-language-host`，默认 `zh-CN` 简体中文，支持 `auto` / `en` / `zh-TW` / `ja` / `ko`）下拉组件及 `#actions-model` 操作条。
     - `#view-provider` 承接原供应商与模型 CRUD 界面及 `#actions-provider` 操作条。
     - 更新 `src/settings/service/tests.rs` 与 `src/settings/service/models/tests.rs` 断言。

---

## Task 2：实现标题生成核心模块 `src/chat/title.rs`（对齐 `codex-main`）

- **目标文件**：
  - `src/chat/title.rs`（新建）
  - `src/chat/mod.rs`
  - `src/ui/text.rs`
- **关键改动点**：
  1. **在 `src/ui/text.rs` 新增英文文案常量**：
     - `PROMPT_TITLE_GENERATING: &str = "Generating title..."`
     - `PROMPT_TITLE_EMPTY_BLOCK: &str = "Template block is empty"`
     - `PROMPT_TITLE_NO_MODEL: &str = "No title model or default model configured"`
     - `PROMPT_TITLE_FAILED: &str = "Failed to generate title"`
  2. **新建 `src/chat/title.rs`**：
     - 实现 `title_instructions()`、`session_title_prompt(user_message)`、`template_block_title_prompt(block_content)`（按 `TITLE_PROMPT_MAX_BYTES = 960` 在 UTF-8 字符边界安全截断）。
     - 实现 `build_title_request(prompt, model)`（无工具、无系统提示、`max_output_tokens: Some(128)`、`variant_params: None`）。
     - 实现 `parse_generated_title(raw)`（剥离 `<think>` / `<thinking>` 标签、支持 JSON `{"title": "..."}` 与纯文本首行提取、剥离引号与末尾标点、过滤 `{` / `}`、按 `TITLE_MAX_CHARS = 36` 截断）。
     - 编写完整单元测试（覆盖中英文、超长截断、`<think>` 标签、JSON/纯文本、引号与标点剥离、空输入等场景）。

---

## Task 3：实现 Prompt 模板块（`&&&` 与 `+++`）的 `/title` 命令与空行保留

- **目标文件**：
  - `src/app/markdown/slash.rs`
  - `src/app/markdown/supple.rs`
  - `src/app/markdown/template.rs`
  - `src/app/markdown/mod.rs`
  - `src/app/prompt.rs`
  - `src/app/prompt/panel.rs`
  - `src/app/prompt/template.rs`
  - `src/settings/commands.rs`
- **关键改动点**：
  1. **斜杠候选注册（`src/app/markdown/slash.rs` & `src/settings/commands.rs`）**：
     - `SlashInsertion` 新增 `GenerateBlockTitle`。
     - `SlashItem::builtin_title()` 创建 `/title` 候选。
     - `slash_filter` 在 `BlockContext::Template | BlockContext::Supple` 分支加入 `SlashItem::builtin_title()`（`BlockContext::Normal` 不展示）。
     - `src/settings/commands.rs` 确认 `"title"` 在保留字列表中，防止自定义命令重名冲突。
  2. **块定位与标题行读写（`src/app/markdown/supple.rs` & `src/app/markdown/template.rs`）**：
     - 在 `supple.rs` 新增 `find_enclosing_supple_block(text: &str, cursor: usize) -> Option<(usize, usize, bool)>`，支持识别独立或嵌套在 `&&&` 内的 `+++` 补充块。
     - 在 `template.rs` 扩展 `title_markers` 同时保护 `+++` 起始行下一行的 `{title: ...}`。
     - 在 `template.rs` 新增 `block_title_value_at_line(text: &str, start_line: usize) -> Option<String>` 与 `set_block_title_edit(text: &str, start_line: usize, new_title: Option<&str>) -> Option<(Range<usize>, String)>`。
  3. **确认 `/title` 保留空行与写入加载态（`src/app/prompt/panel.rs` & `src/app/prompt/template.rs`）**：
     - 在 `Prompt` 新增 `pending_title_action: Option<PromptTitleAction>` 及 `take_title_action(&mut self) -> Option<PromptTitleAction>`。
     - 在 `slash_confirm` 处理 `SlashInsertion::GenerateBlockTitle`：
       - 先用 `self.splice_text(trigger.line_start, trigger.line_end, "")` **仅清空命令行字符、保留行末 `\n` 空行**，光标置于 `trigger.line_start`；
       - 提取当前块正文并调用 `copy_template_content` 清洗（同时剔除残留空模板壳）；
       - 若正文为空，记录 `PromptTitleAction::EmptyBlock`；
       - 若正文非空，记录原 `{title: ...}` 值，将块的标题行更新为 `{title: Generating title...}`（同步平移 `self.cursor` 保持在原空行行首），并记录 `PromptTitleAction::Generate { block_start_line, original_title, content }`。

---

## Task 4：实现后台异步标题生成与状态回填（`lx` 新对话 + Prompt 模板块）

- **目标文件**：
  - `src/app/effects.rs`
  - `src/app/state/mod.rs`
  - `src/app/update/chat/title.rs`（新建）
  - `src/app/update/chat/mod.rs`
  - `src/app/update/ chat/session.rs`
  - `src/app/update/resume_edit.rs`
  - `src/app/update/editor.rs`
  - `src/event/chat/title.rs`（新建）
  - `src/event/chat/mod.rs`
  - `src/event/mod.rs`
  - `src/db/chat/mod.rs`
- **关键改动点**：
  1. **数据库层防覆盖更新（`src/db/chat/mod.rs`）**：
     - 新增 `Database::rename_chat_session_if_title_matches(&self, workspace: &Path, session_id: &str, expected_title: &str, new_title: &str) -> Result<bool>`。
     - 在 `TurnCommitted` 中携带首轮创建的 `initial_title: Option<String>`（或由 `single_line_preview` 同口径得出）。
  2. **`lx` 新对话自动标题调度与回填（`src/app/update/chat/title.rs`）**：
     - 在 `on_turn_committed` 中当 `committed.turn_index == 1` 时调用 `title::maybe_schedule_session_title`，登记 `state.pending_session_titles` 并入队 `Effect::GenerateSessionTitle`。
     - 在 `rename_session` 与 `submit_resume_session_edit` 中调用 `title::cancel_session_title(&mut state, &session_id)`，确保手动改名立即作废在途自动标题。
     - 实现 `on_session_title_generated`：校验仍在 `pending_session_titles` 后调用 `db.rename_chat_session_if_title_matches` 并刷新打开中的 `/resume` 列表。
  3. **Prompt `/title` 调度与回填（`src/app/update/chat/title.rs` & `src/app/update/editor.rs`）**：
     - 在键盘 `apply_editor` 与鼠标 `select_slash` 后调用 `title::flush_prompt_title_action(state)`：
       - `EmptyBlock` $\to$ 弹全局 Warning Toast `"Template block is empty"`；
       - `Generate` $\to$ 调用 `state.models.resolve_prompt_title_model()`：
         - 若为 `None`，立即回滚 `{title: Generating title...}` 为 `original_title`，弹全局 Warning Toast `"No title model or default model configured"`；
         - 若为 `Some(frozen)`，调用 `commit_editing_prompt(state)` 并入队 `Effect::GeneratePromptBlockTitle`。
     - 实现 `on_prompt_block_title_generated`：
       - 成功返回 `Some(title)` 时，将目标页对应块的 `{title: Generating title...}` 替换为 `{title: <title>}`，并提交持久化；
       - 失败返回 `None` 时，将 `{title: Generating title...}` 恢复为 `original_title`，提交持久化并弹出 Warning Toast `"Failed to generate title"`。
  4. **事件层后台流执行（`src/event/chat/title.rs` & `src/event/mod.rs`）**：
     - `apply_title_effects(state, &sender)` 拦截 `Effect::GenerateSessionTitle` 与 `Effect::GeneratePromptBlockTitle`；
     - 在 `tokio::spawn` 中使用 `tokio::time::timeout(Duration::from_secs(30), ...)` 调用 `crate::chat::client::run_stream` 收集文本增量，解析标题后发送 `AppEvent::SessionTitleGenerated` / `AppEvent::PromptBlockTitleGenerated`。

---

## Task 5：单元测试与受影响模块精确验证

- **目标文件**：
  - `src/chat/title.rs`（内置单测）
  - `src/app/prompt/tests.rs`（或子模块测试）
  - `src/app/update/chat/tests/`
  - `src/settings/service/tests.rs`
  - `src/settings/service/models/tests.rs`
- **具体测试项**：
  1. **设置页与配置测试**：
     - `AiSettings` 序列化/反序列化 `titleSummary` 往返保留，非法 provider/model/variant 校验报错；
     - `GET / PUT /api/models/settings` 支持 `titleSummary` 读写与 `revision` 乐观锁；
     - `settings.html` 包含 `tab-model`、`view-model`、`tab-provider`、`view-provider`、`default-model-host`、`title-model-host`。
  2. **Prompt `/title` 命令测试**：
     - 光标在 `&&&` 块外时 `/` 面板不含 `/title`；光标在 `&&&` 或 `+++` 块内时 `/` 面板包含 `/title`；
     - 在块内空行输入 `/title` 按回车确认后，`/title` 文字被清空，但**该空行本身保留（行数不减少）**，光标停在该空行行首；
     - 块内容为空（或仅有未填写的模板项）时确认 `/title`，保留空行并触发 `"Template block is empty"` Toast；
     - 块内容非空且配置了模型时，标题行先变为 `{title: Generating title...}` 并入队 `Effect::GeneratePromptBlockTitle`；回投成功时更新为 `{title: <新标题>}`，回投失败时恢复原 `{title: ...}`。
  3. **`lx` 新对话自动标题测试**：
     - 首轮 `on_turn_committed`（`turn_index == 1`）自动入队 `Effect::GenerateSessionTitle`，续轮（`turn_index > 1`）不入队；
     - 自动标题返回后更新 DB `agent_session.title` 及已打开的 `/resume` 列表；
     - 若返回前用户已执行 `/rename`，自动标题被丢弃，手动标题保留。
  4. **精确验证命令**：
     - `rtk cargo fmt --check`
     - `rtk cargo clippy --all-targets -- -D warnings`
     - `rtk cargo test`（针对 `chat::title`、`settings`、`prompt`、`update::chat` 等受影响模块执行测试）
