# 会话与模板块标题生成（含 Settings Model/Provider 拆分）：执行任务清单

- 状态：**已完成并合并至主工作区 `dev` 分支（原工作区 `.worktrees/title-generation` 与分支 `feat/title-generation` 已清理）**
- 关联文档：
  - 设计方案：`docs/feature/title-generation/design.md`
  - 代码审查：`docs/feature/title-generation/review.md`

---

## Task 1：扩展 `AiSettings.{title_summary, title_language}` 与设置页 `Model` / `Provider` Tab 拆分

- [x] **`src/config/ai/mod.rs` & `src/config/ai/tests.rs`**：
  - 新增 `TitleLanguage` 枚举（默认 `Auto`，支持 `zh-CN`、`en`、`zh-TW`、`ja`、`ko`）与容错反序列化（未知值降级回退 `Auto`）。
  - 在 `AiSettings` 中新增 `title_summary: Option<ModelSelection>` 与 `title_language: Option<TitleLanguage>`。
  - 实现 `sanitize_optional_selections`（磁盘加载阶段自动清理悬空的 `titleSummary` 或失效 `variant`，不阻塞整份 `ai.json` 加载）与 `validate_selection`（保存阶段严格校验）。
- [x] **`src/app/state/lx/chat.rs`**：
  - 在 `ModelsSnapshot` 新增 `title_selection()`、`default_selection()`、`title_language()`、`resolve_prompt_title_model()`（`titleSummary` $\to$ `defaultModel`）与 `resolve_session_title_model()`（`titleSummary` $\to$ 窗格当前有效模型 $\to$ `defaultModel`）。
- [x] **`src/settings/service/models/mod.rs` & `tests.rs`**：
  - `GET`/`PUT` `/api/models/settings` 支持 `titleSummary` 与 `titleLanguage` 读写及 `revision` 计算，移除 `settings.extra.clear()` 以保留顶层未知扩展字段。
- [x] **`assets/settings.html`、`assets/settings.js`、`assets/settings_models.js`、`assets/settings.css`**：
  - 左侧导航栏拆分为 `Model`（`#tab-model` $\to$ `#view-model`）与 `Provider`（`#tab-provider` $\to$ `#view-provider`）。
  - `#view-model` 将 `Default Model` 与 `Title Generation`（`Title model` + `Title language`）拆为两个独立 `<fieldset>`。
  - `Provider` 页新增/抓取/重命名模型后自动刷新 `Model` 页下拉框；仅列出已启用 provider 的模型；保存时保留合法的自定义 `variant` 并对共享的 `saveModels` 去重。

---

## Task 2：实现标题生成核心模块 `src/chat/title.rs` 与 `src/chat/title/tests.rs`

- [x] **`src/ui/text.rs`**：
  - 新增英文文案常量 `PROMPT_TITLE_GENERATING`、`PROMPT_TITLE_EMPTY_BLOCK`、`PROMPT_TITLE_NO_MODEL`、`PROMPT_TITLE_FAILED`。
- [x] **`src/chat/title.rs` & `src/chat/title/tests.rs`**：
  - 实现纯英文系统提示词 `TITLE_SYSTEM_PROMPT` 与 `title_instructions_for(language, sample_text)`（支持 `Auto` 自动识别简中、繁中、日文假名、韩文谚文与英文，排除中文间隔号 `・` `U+30FB` 误判，且明确要求对问候/身份询问同样提炼简短标题、严禁拒答）。
  - 实现 `session_title_prompt_for` 与 `template_block_title_prompt_for`（按 `TITLE_PROMPT_MAX_BYTES = 960` 在 UTF-8 字符边界安全截断）。
  - 实现 `build_title_request(prompt, model)`（携带非空 `TITLE_SYSTEM_PROMPT`，避免空 system 块在部分 Provider 上触发 HTTP 400）。
  - 实现 `parse_generated_title(raw)`（剥离 `<think>`、```` ```json ```` 代码块围栏、`Title:` / `标题：` 前缀，在 36 字符截断前执行多语言拒答元解释拦截，避免误杀合法技术标题）。

---

## Task 3：实现 Prompt 模板块（`&&&` 与 `+++`）的 `/title` 命令与空行保留

- [x] **`src/app/markdown/{slash,supple,template}.rs`**：
  - 注册 `/title`（`SlashInsertion::GenerateBlockTitle`），仅在 `BlockContext::Template | BlockContext::Supple` 内可见；在 `src/settings/commands.rs` 将 `"title"` 列入保留字。
  - 新增 `find_enclosing_supple_block` 支持定位独立或嵌套在 `&&&` 内的 `+++` 补充块。
  - 实现 `block_title_value_at_line` 与 `set_block_title_edit`，同时兼容起始行内联 `{title: ...}` 与下一行独立 `{title: ...}` 行，并继承起始行缩进。
- [x] **`src/app/prompt/{mod,panel,template}.rs`**：
  - 确认 `/title` 时仅替换 `trigger.line_start..trigger.line_end` 为 `""`，严格保留行尾 `\n` 空行并将光标停在该空行行首。
  - 清洗块正文并剔除默认空模板标题行，正文为空时触发 `"Template block is empty"` Toast，非空时在内存写入 `{title: Generating title...}` 并平移光标。

---

## Task 4：实现后台异步标题生成、防覆盖回填与输入框右上角柔和标题胶囊

- [x] **`src/db/chat/mod.rs` & `src/db/service/chat.rs`**：
  - 新增 `Database::chat_session_title`（`/resume` 加载会话时回显标题）与 `Database::rename_chat_session_if_title_matches`（仅当 DB 标题仍等于首轮初始标题时才更新）。
- [x] **`src/app/state.rs`、`src/app/update/chat/title.rs` & `src/event/chat/title.rs`**：
  - 首轮 `on_turn_committed`（`turn_index == 0`）调度 `Effect::GenerateSessionTitle`；手动 `/rename`、`/resume` 改名或删除会话立即调用 `cancel_session_title`。
  - 在 `AppState` 维护 `pending_prompt_titles: HashMap<u64, PendingPromptTitle>`：触发 `/title` 时不提前将 `Generating title...` 持久化入库，回填时校验 `request_id` 并同步 `Editor` / `Pages` 模式与停泊工作区缓冲。
  - 将标题完成事件收口为 `AppEvent::TitleGenerated(TitleGeneratedEvent)`，保持 `src/event/mod.rs` 在 1000 行上限以内。
- [x] **`src/ui/style.rs`、`src/ui/main_content/lx/{input,grid}.rs` & `src/ui/main_content/lx/input/tests.rs`**：
  - 在 `src/ui/style.rs` 定义 8 套柔和暗底粉彩字配色 `SESSION_TITLE_PALETTE` 与 `session_title_badge(palette_index)`。
  - 在 `src/ui/main_content/lx/input.rs` 的 `draw_session_title_badge` 中将当前会话标题渲染在输入框顶边框右上角内侧（最右留 1 列 `─`），对齐 `Grid::put` 零宽字符宽度计算并在 `lx::render` 中保留宽字符 `CONTINUATION` 延续格样式。

---

## Task 5：Code Review（`review.md`）与全量验证

- [x] 编写 `docs/feature/title-generation/review.md`，按 `P0` $\to$ `P3` 梳理关键点、错误原因与修复方案，并完成全部问题修复。
- [x] 将 `src/chat/title`、`src/app/update/chat/title`、`src/event/chat/title`、`src/ui/main_content/lx/input` 的单元测试拆分为符合规范的独立 `tests.rs` 子文件。
- [x] 执行 `rtk cargo fmt --check`、`rtk cargo clippy --all-targets -- -D warnings` 与 `rtk cargo test` 全量通过。
