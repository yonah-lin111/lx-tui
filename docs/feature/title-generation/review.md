# 功能代码评审报告（Review）：`lx` 自动对话标题、Prompt `/title` 命令与设置页模型/供应商拆分

## 1. 评审范围与概览

- **状态**：**全部 P0–P3 项已修复验证并合入 `dev` 分支**（2026-11-02）
- **原评审工作区**：`.worktrees/title-generation`（分支 `feat/title-generation`，已合并至 `dev` 并清理）
- **关联文档**：
  - `docs/feature/title-generation/design.md`
  - `docs/feature/title-generation/task.md`
  - 项目规范：`docs/standards/{project-directory-structure,code-writing-standards,tui-design-requirements,html-assets}.md`
- **评审覆盖模块**：
  - 配置与设置服务：`src/config/ai/`、`src/settings/service/models/`、`src/settings/service/handlers.rs`、`assets/settings*.{html,css,js}`
  - 标题生成核心与事件流：`src/chat/title.rs`、`src/event/chat/title.rs`、`src/app/update/chat/title.rs`、`src/event/mod.rs`
  - 状态与持久化：`src/app/state/lx/chat.rs`、`src/app/update/resume_edit.rs`、`src/db/chat/mod.rs`、`src/db/service/chat.rs`
  - TUI 渲染与编辑器：`src/ui/main_content/lx/input.rs`、`src/ui/main_content/lx/grid.rs`、`src/ui/style.rs`、`src/app/markdown/template.rs`

### 问题优先级汇总表（P0 -> P3）

| 编号 | 优先级 | 涉及文件与行号 | 关键点 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0-1** | **P0** | `src/config/ai/mod.rs:234-255, 399-401, 500-510`<br>`src/settings/service/handlers.rs:371-393` | 失效的 `titleSummary` / `titleLanguage` 会导致整份 `ai.json` 加载失败，且 General 页保存遇到校验错误时会用 `AiSettings::empty()` 覆盖清空所有模型与供应商 | 已修复 |
| **P0-2** | **P0** | `src/app/update/chat/title.rs:95-332`<br>`src/app/state.rs:148-152`<br>`src/event/mod.rs:862-880`<br>`src/app/markdown/template.rs:403-459` | `/title` 将瞬态占位符 `{title: Generating title...}` 立即落盘、丢弃 `request_id`、漏同步 `Pages` 模式/停泊工作区缓冲且不支持起始行内联 `{title: ...}`，中断后永久残留占位符或发生错写 | 已修复 |
| **P1-1** | **P1** | `src/ui/main_content/lx/input.rs:27-56`<br>`src/ui/style.rs`<br>`src/event/mod.rs` (1011 行)<br>`src/app/state/lx/chat.rs:503-596` | 违反样式收口与单文件行数规范：胶囊配色直接写死在 `input.rs`、生产文件 `src/event/mod.rs` 突破 1000 行上限、`app` 层直接调用 `ui::text::chat::single_line_label`（空标题会回退为 `"Working..."`） | 已修复 |
| **P1-2** | **P1** | `src/chat/title.rs:27-110, 173-314` | `build_title_request` 传入空串 `system_prompt: Some("")` 在部分 Provider 上触发 400、英文提示词中夹杂中文示例、`Auto` 语言把 `・` 误判为日文、未剥离 `` ```json `` 围栏与 `Title:` 前缀且拒答检测在截断之后执行 | 已修复 |
| **P1-3** | **P1** | `src/ui/main_content/lx/input.rs:58-89, 281-300`<br>`src/ui/main_content/lx/grid.rs:65-75` | 输入框右上角标题胶囊的 `display_width` 与 `Grid::put` 对零宽字符（如 `❤️` `U+FE0F`、组合字符）宽度口径不一致，导致胶囊越界覆盖最右侧 `─` 边框 | 已修复 |
| **P2-1** | **P2** | `assets/settings_models.js:299-309, 575-608, 705-752`<br>`assets/settings.js:23-36, 236-257`<br>`src/settings/service/models/mod.rs:347` | Provider 页新增模型后切到 Model 页下拉框不刷新、保存时丢失自定义 `variant`、未过滤已禁用 Provider、双 Tab 同时 dirty 时重复发送两次 `PUT`、服务端 `settings.extra.clear()` 误删未知顶层字段 | 已修复 |
| **P2-2** | **P2** | `src/app/state/lx/chat.rs:86-102`<br>`src/app/update/chat/session.rs:490-508`<br>`src/app/update/resume_edit.rs:178-195` | `resolve_session_title_model` 在窗格当前模型无 API Key 或被禁用时未继续回退至 `default_model`；删除会话时未取消 `pending_session_titles` | 已修复 |
| **P2-3** | **P2** | `src/app/update/tests.rs:221-228` | 全量测试中既有 `prompt_click_dismisses_worktree_chooser_without_text_change` 因块顶边框视觉行偏移失败，阻碍整库 `cargo test` 全绿 | 已修复 |
| **P3-1** | **P3** | `src/config/ai/mod.rs:304, 340`<br>`src/app/state/lx/chat.rs:193`<br>`docs/feature/title-generation/{design,task}.md`<br>`docs/standards/tui-design-requirements.md` | 代码注释仍写默认 `SimplifiedChinese`（实际为 `Auto`），新建模块内联测试未拆分为 `tests.rs` 子文件，`design.md` / `task.md` 与最终实现存在漂移 | 已修复 |

---

## 2. 逐项详细分析（关键点、错误原因与修复方案）

### [P0-1] 失效的 `titleSummary` / `titleLanguage` 导致整份 `ai.json` 加载失败，且 General 页保存会用空配置覆盖清空所有模型与供应商

- **关键点**：
  - `src/config/ai/mod.rs`：`TitleLanguage` 反序列化、`AiSettings::validate`、`parse_ai`（第 `234–255`、`399–401`、`500–510` 行）
  - `src/settings/service/handlers.rs`：`put_config`（第 `371–393` 行）
- **错误原因**：
  1. `parse_ai` 在加载 `ai.json` 时直接对 `title_summary` 执行严格的 `validate_selection`，且 `TitleLanguage` 使用严格枚举反序列化：一旦 `titleSummary` 指向后来被删除或禁用的 provider / model / variant，或 `titleLanguage` 含有未知枚举值，整份 `ai.json` 即被判定为非法并返回 `Err(AiConfigError::Invalid)`；
  2. 这会导致启动时 `state.models` 退化为空快照（所有已配置的供应商和默认模型全部不可用），且 `GET /api/models/settings` 直接返回 HTTP 500，用户无法在设置页内打开 Model / Provider 页修复；
  3. 更严重的是，`src/settings/service/handlers.rs:377-382` 的 `put_config`（General 页保存入口）在遇到 `Err(AiConfigError::Invalid)` 时会回退构造 `AiSettings::empty()` 并写回 `ai.json`——用户只要在 General 页点击一次保存，就会将整份 `providers` 与 `defaultModel` 彻底清空。
- **修复方案**：
  1. **加载宽容、保存严格（Load-lenient / Save-strict）**：
     - 为 `TitleLanguage` 实现带容错的反序列化（未知字符串记录告警并回退为 `TitleLanguage::Auto`）；
     - 在 `AiSettings` 新增 `sanitize_optional_selections(&mut self)`，在 `parse_ai` 加载阶段若检测到 `title_summary` 或 `default_model` 指向不存在/未启用的 provider 或不存在的 model，记录 `tracing::warn!` 并将其降级为 `None`（若仅 `variant` 不存在则将 `variant` 置为 `None`），确保 `load_ai` 不会因悬空的选择项而丢弃整份 `providers`；
     - `save_ai` 与 `PUT /api/models/settings` 仍调用严格的 `validate()`，阻止前端提交非法引用。
  2. 在 `src/settings/service/handlers.rs::put_config` 中，当 `load_ai` 返回 `AiConfigError::Invalid` 时直接返回 `bad_request` 错误，严禁用 `AiSettings::empty()` 覆盖用户配置文件。

---

### [P0-2] Prompt `/title` 将瞬态占位符 `{title: Generating title...}` 立即持久化落盘，且丢弃 `request_id`、漏同步 `Pages` 模式/停泊工作区缓冲、不支持内联 `{title: ...}`

- **关键点**：
  - `src/app/update/chat/title.rs`：`trigger_prompt_block_title`、`on_prompt_block_title_generated`、`update_background_page_title`（第 `95–332` 行）
  - `src/app/state.rs`：`prompt_title_request_counter`（第 `148–152` 行）
  - `src/event/mod.rs`：`AppEvent::PromptBlockTitleGenerated` 分发（第 `862–880` 行）
  - `src/app/markdown/template.rs`：`block_title_value_at_line`、`set_block_title_edit`（第 `403–459` 行）
- **错误原因**：
  1. **瞬态占位符落库**：`trigger_prompt_block_title` 在把标题行改为 `{title: Generating title...}` 后立即调用 `commit_editing_prompt(state)` 写入 SQLite。若生成期间应用退出、任务中止或工作区关闭，`Generating title...` 将永久留在数据库里；
  2. **`request_id` 未消费**：`src/event/mod.rs:863` 将 `request_id: _` 直接丢弃。用户在同一个块连续执行两次 `/title` 时，先返回的旧请求会覆盖后发起的请求；且当用户在生成途中手动改了标题或删除了块，回填失败分支仍会弹出 `Failed to generate title` Toast；
  3. **`PromptMode::Pages` 与停泊工作区缓冲未同步**：`on_prompt_block_title_generated` 仅在 `mode == PromptMode::Editor` 时更新 `state.prompt`。若用户在生成期间切到 `Pages` 页列表视图，后台分支只改了 `tree` 中的 `page.content`，未同步 `state.prompt`，切回 `Editor` 后旧缓冲里的占位符会再次覆盖已生成的标题；此外后台分支在 `page_id.saved()` 为 `None` 时直接丢弃了未保存新页（`PromptPageId::Temp`）的生成结果；
  4. **内联标题双写冲突**：`block_title_value_at_line` 与 `set_block_title_edit` 只检查 `start_line + 1` 的独立 `{title: ...}` 行，忽略了起始行自带的内联标题（如 `&&& task --start {title: Old}`），导致生成后在下一行又插入一个 `{title: New}`，形成双标题源。
- **修复方案**：
  1. 在 `AppState` 中新增 `pending_prompt_titles: HashMap<u64, PendingPromptTitle>`（记录 `request_id`、目标工作区、目标页、块起始行与 `original_title`）；同一工作区同一页面同一块的后续 `/title` 会移除旧挂起项并继承最初的 `original_title`；
  2. 触发 `/title` 时只更新内存缓冲与页树中的占位符，**不立即触发 `SavePromptContent` 落库**；待 `on_prompt_block_title_generated` 校验 `request_id` 命中有效挂起项后，再写入最终标题（或回滚为 `original_title`）并持久化落库；若挂起项已失效则静默忽略，不弹过时 Toast；
  3. 回填时无论当前处于 `PromptMode::Editor` 还是 `PromptMode::Pages`，只要 `editing` 与 `active_page_id` 指向目标页，统一更新 `state.prompt`（或停泊工作区的 `ws.prompt`）及页树（含 `PromptPageId::Temp`）；
  4. 在 `src/app/markdown/template.rs` 中让 `block_title_value_at_line` 与 `set_block_title_edit` 优先识别并原地替换起始行内联 `{title: ...}`，无内联标题时再读写 `start_line + 1` 独立标题行，并在插入新行时继承起始行的前导空格缩进。

---

### [P1-1] 违反样式与单文件行数规范——胶囊配色直接定义在 `input.rs`、`src/event/mod.rs` 超出 1000 行上限、`app` 层调用 `ui` 格式化函数

- **关键点**：
  - `src/ui/main_content/lx/input.rs:27-56`
  - `src/ui/style.rs`
  - `src/event/mod.rs`（1011 行）
  - `src/app/state/lx/chat.rs:503-596`
  - `docs/standards/tui-design-requirements.md`
- **错误原因**：
  1. `docs/standards/tui-design-requirements.md` 规定所有颜色与修饰符必须收口在 `src/ui/style.rs` 的语义函数中，禁止在 UI 组件文件内直接定义颜色常量与 `Style`；而 `SESSION_TITLE_PALETTE` 与 `session_title_badge_style` 直接写在了 `src/ui/main_content/lx/input.rs` 中；
  2. `src/event/mod.rs` 在加入标题事件变体与处理分支后达到 `1011` 行，超过 `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 规定的生产代码单文件 `1000` 行上限；
  3. `src/app/state/lx/chat.rs` 的 `set_session_title` 调用了 `crate::ui::text::chat::single_line_label(raw, TITLE_MAX_CHARS)`：不仅造成 `app -> ui` 逆向函数依赖，而且 `single_line_label` 在输入全为空白或控制字符时会回退返回 `"Working..."`（`chat_loading_fallback()`），导致空标题被错误显示成 `"Working..."` 胶囊；此外 `next_random_title_palette` 依赖 `SystemTime` 与全局原子变量，导致同一会话在 `/resume` 重载或多窗格间颜色不一致。
- **修复方案**：
  1. 将 `SESSION_TITLE_PALETTE`、`SESSION_TITLE_PALETTE_COUNT` 与 `session_title_badge(palette_index: u8) -> Style` 迁入 `src/ui/style.rs`，并在 `docs/standards/tui-design-requirements.md` 中登记会话标题胶囊的柔和暗色语义色板；
  2. 在 `src/app/state/lx/chat.rs` 中使用纯状态层的单行空白折叠与截断逻辑（空白输入直接返回 `None`），并支持基于 `session_id` 确定性哈希分配配色槽位（无 `session_id` 时使用相邻去重步进），确保同一会话跨重载与多窗格颜色稳定一致；
  3. 将 `src/event/mod.rs` 中的标题事件处理下沉到 `src/event/chat/title.rs`（`handle_session_title_generated` / `handle_prompt_block_title_generated`），将 `src/event/mod.rs` 降至 1000 行以内；同时将新建模块的单元测试拆入各自的 `tests.rs` 子文件。

---

### [P1-2] `build_title_request` 发送空 `system_prompt`、英文提示词夹杂中文示例、`Auto` 语言误判 `・` 及 `parse_generated_title` 解析缺陷

- **关键点**：
  - `src/chat/title.rs:27-110, 173-314`
- **错误原因**：
  1. `build_title_request` 设置了 `system_prompt: Some(String::new())` 且将 `variant_params` 置为 `None`；在 Google Gemini（会构造 `systemInstruction.parts[0].text = ""`）及部分 OpenAI 兼容网关上，空 system 文本块会返回 HTTP 400 错误；
  2. `title_instructions_for` 的 `SimplifiedChinese` 分支包含了 `'你是谁'`、`'你好'`、`'身份询问'`、`'日常问候'`、`'无法生成标题'` 等中文字符，违反 `AGENTS.md`「lx 的模型提示词一律用英文编写」的硬性规定；
  3. `contains_japanese_kana` 直接判断 `'\u{3040}'..='\u{30ff}'`，而中文常用的间隔号 `・`（`U+30FB`）恰好落在该区间内，导致含间隔号的中文文本在 `Auto` 模式下被强制要求输出日文标题；
  4. `parse_generated_title` 未剥离 Markdown 代码围栏（`` ```json ... ``` ``）与 `Title:` / `标题：` 前缀，当模型返回带围栏的 JSON 时会取第一行 `` ```json `` 并清洗出错误标题 `"json"`；同时 `is_refusal_title` 在截断到 36 字符之后才检测，缺少繁中/日/韩拒答匹配，且对任意包含 `"无法生成"` 的正常开发任务标题（如 `"修复无法生成发票问题"`）存在误杀。
- **修复方案**：
  1. 在 `build_title_request` 中将英文系统指令放入 `system_prompt: Some(instructions.to_string())`，将待概括正文放入 `UserText`，彻底消除空字符串 `system_prompt`，并透传 `variant_params: model.variant_params.clone()`；
  2. 将 `title_instructions_for` 中所有示例改为纯英文表述；
  3. 在 `contains_japanese_kana` 中限定平假名（`3041..=3096`）与片假名字符（`30A1..=30FA`），排除 `U+30FB`（`・`）与 `U+30FC` 等共用符号；同时增加常见繁体中文特征字识别；
  4. 在 `parse_generated_title` 中先剥离 `` ``` `` 代码块围栏，再解析 JSON 或提取首行；剥离行首 `Title:` / `标题:` / `标题：` 前缀；在截断**之前**对完整候选文本执行拒答判定，并将拒答判定收敛为元解释短语组合（如同时涉及无法生成/非任务描述 + 标题/任务/输入，或覆盖繁中/日/韩拒答短语），避免误杀 `"修复无法生成 PDF 报表"` 等正常标题。

---

### [P1-3] 输入框右上角标题胶囊宽度计算与 `Grid::put` 对零宽字符口径不一致，可越界覆盖右侧 `─` 边框

- **关键点**：
  - `src/ui/main_content/lx/input.rs:58-89, 281-300`
  - `src/ui/main_content/lx/grid.rs:65-75`
- **错误原因**：
  - `draw_session_title_badge` 调用 `display_width` 计算胶囊宽度时，零宽字符（如 `❤️` 中的变体选择符 `U+FE0F`、零宽连字 `U+200D` 或组合音标）宽度为 `0`；但 `Grid::put` 内部使用 `UnicodeWidthChar::width(ch).unwrap_or(1).max(1)` 将宽度为 `0` 的字符按 `1` 列写入并右移列游标，导致胶囊实际绘制列数超出 `badge_w`，覆盖右上角保留的 1 列 `─` 边框线。
- **修复方案**：
  - 在 `Grid::put` 中对齐 `Grid::put_clipped`：遇到 `UnicodeWidthChar::width(ch).unwrap_or(0) == 0` 的零宽字符时跳过独立占格；在 `truncate_to_width` 中允许零宽尾随字符跟随前一个可见字符，并新增含 `❤️` 的标题胶囊渲染边界测试。

---

### [P2-1] Settings 前端跨 Tab 状态同步、自定义 `variant` 保留、已禁用 Provider 过滤、双 Tab 重复保存与 `settings.extra.clear()`

- **关键点**：
  - `assets/settings_models.js:299-309, 575-608, 705-752, 790-801`
  - `assets/settings.js:23-36, 236-257`
  - `src/settings/service/models/mod.rs:347`
- **错误原因**：
  1. 在 `Provider` 页调用 `addModel()` 或通过 Fetch 候选添加模型后，未调用 `renderDefaultModel()`，切换到 `Model` 页时下拉框不显示新模型；
  2. `buildPayload` 重建 `defaultModel` 与 `titleSummary` 时使用 `model.variant || null`，覆盖了原配置中合法的 `selection.variant`；且 `renderDefaultModel` 将未启用（`!provider.enabled`）的 provider 模型也列为可选项，选中后保存会触发服务端 422；
  3. `saveAllDirty` 遍历 `dirtyTabs` 时，`model` 与 `provider` 共享同一个 `saveModels` 函数，两者同时 dirty 时会重复发起两次 `PUT /api/models/settings`；且 409 分支直接 `return` 未抛错；
  4. `src/settings/service/models/mod.rs:347` 的 `settings.extra.clear()` 会在保存模型时清空 `ai.json` 顶层的其它未知扩展字段。
- **修复方案**：
  1. 在 `addModel()`、Fetch 添加模型回调及 `showTab('model')` 中同步调用 `renderDefaultModel()`，并在选择器中过滤未启用的 provider；
  2. 在 `buildPayload` 中通过 `resolveSelectionVariant(selection, model)` 优先保留仍存在于 `model.variantRows` 中的原 `selection.variant`；
  3. 在 `saveAllDirty` 中按 `bar.save` 函数去重，并在 `saveModels` 的 409 分支抛出冲突错误；在 `showTab` 中同步重置 `prov-api-key` 的明文显示状态，清理未使用的 `modelsDirty` 变量与 `.models-toolbar-select` 样式；
  4. 移除 `src/settings/service/models/mod.rs` 中的 `settings.extra.clear()` 并补充顶层未知字段往返测试。

---

### [P2-2] `resolve_session_title_model` 未对 fallback 模型做完整可用性校验与二次回退，且删除会话未取消在途标题请求

- **关键点**：
  - `src/app/state/lx/chat.rs:86-102`
  - `src/app/update/chat/session.rs:490-508`
  - `src/app/update/resume_edit.rs:178-195`
- **错误原因**：
  1. `resolve_session_title_model` 在 `title_selection()` 为 `None` 时直接对传入的 `fallback_selection` 调用 `FrozenModel::resolve`，既未校验 provider 是否启用，又在 `fallback_selection` 无 API Key 失败时直接返回 `None`，未继续回退到可用的 `default_selection()`；
  2. 删除会话时未清理 `pending_session_titles`。
- **修复方案**：
  1. 在 `resolve_session_title_model` 中先用 `validated_selection` 校验 `fallback_selection`，若无效则继续回退到 `self.default_selection()`；
  2. 在会话删除路径中调用 `cancel_session_title(state, &session_id)`。

---

### [P2-3] 修复既有 `prompt_click_dismisses_worktree_chooser_without_text_change` 测试中的视觉行偏移

- **关键点**：
  - `src/app/update/tests.rs:221-228`
- **错误原因**：
  - `&&& addTemplate --start` 块上方包含 1 行 `top_border` 视觉行，使 `/gitWorkTree ` 位于视觉第 3 行而非第 2 行；原测试 `place_prompt_cursor(&mut state, 2, 4)` 落在了 `body` 行，导致 `cargo test --lib` 出现 1 个失败用例。
- **修复方案**：
  - 将光标定位修正为视觉第 3 行（`place_prompt_cursor(&mut state, 3, 4)`），使全库测试 100% 通过。

---

### [P3-1] 统一注释、拆分内联测试并同步 `design.md` / `task.md` / `tui-design-requirements.md`

- **关键点**：
  - `src/config/ai/mod.rs`、`src/app/state/lx/chat.rs`
  - `src/chat/title/tests.rs`、`src/event/chat/title/tests.rs`、`src/app/update/chat/title/tests.rs`
  - `docs/feature/title-generation/{design,task}.md`、`docs/standards/tui-design-requirements.md`
- **修复方案**：
  1. 将 `TitleLanguage` 相关注释统一修正为默认 `Auto`；
  2. 将 `src/chat/title.rs`、`src/event/chat/title.rs`、`src/app/update/chat/title.rs` 的内联 `mod tests` 拆分为符合项目规范的同层 `tests.rs` 子文件；
  3. 同步更新 `design.md`、`task.md` 与 `docs/standards/tui-design-requirements.md`。
