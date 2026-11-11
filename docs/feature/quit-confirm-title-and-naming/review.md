# Review：退出二次确认、模板块标题生成与 UI 命名整理

> 审查对象：`feat/quit-confirm-title-and-naming` / `d6d1a159`（修复提交 `2bb2622b`，已合并至 `dev`）
> 审查方式：codegraph + codebase-memory 结构检索、源码逐行核对、针对性测试补跑、`cargo fmt --check` / `clippy --all-targets -- -D warnings` / 全量 `cargo test`。
> 基线结论：自动化验证全绿，但正常路径之外存在 2 个 P0、6 个 P1 与若干 P2；其中“退出确认被按键自动重复绕过”和“子块内嵌套/围栏导致标题源泄漏”必须先修。
>
> **2026-11-11 后续变更**：按要求移除退出确认模态，改为全局 toast + 2s 确认窗口。下方 P0-1、P1-2、P2-2 中关于独立 `quit_confirm` 层与取消路径的描述是当时的修复记录；当前实现以 `design.md` 第 10.1 节为准。

## 优先级定义

- **P0**：会破坏核心安全语义或核心数据正确性，合并前必须修复。
- **P1**：主要用户路径错误、异步生命周期丢失、明显架构回归，合并前应修复。
- **P2**：边界、提示质量、测试组织、命名与文档问题，按成本修复或记录。

---

## P0

### P0-1 长按 `Ctrl+Q` 可绕过二次确认直接退出；长按 `Esc` 可连续关闭底层浮层

- **位置**：`src/input/mod.rs:148-157`、`src/event/input.rs:31-44`、`src/app/update/quit.rs:7-16`
- **现象**：第一次 `Ctrl+Q` 打开 `Exit lx?`；终端自动重复产生的第二个 `Ctrl+Q` 在确认框刚出现时直接命中 `ConfirmQuit`，提交编辑器并退出。`Esc` 同理：第一次取消退出后，后续重复 `Esc` 落到刚恢复的重命名/`/connect`/引用文件夹弹窗上，可能把底层弹窗也关掉。
- **错误原因**：`input::route` 只过滤 `KeyEventKind::Release`，`Ctrl+Q` 在浮层判断之前返回，没有 Repeat/时间去抖；macOS 传统终端或不开启 kitty `REPORT_EVENT_TYPES` 时，自动重复是连续 `Press`，仅过滤 `Repeat` 不能修复。退出确认也没有“取消后的短窗口 latch”。
- **修复方向**：确认层独立于 `state.overlay`；`Ctrl+Q` 二次确认使用“连续 Press 静默窗口”（每次自动重复刷新窗口，按住期间永不满足）+ `Repeat` 忽略；`Esc` 取消后加会随重复刷新短窗口 latch，避免自动重复穿透到底层浮层。
- **回归测试**：`ctrl_q_auto_repeat_does_not_exit`、`ctrl_q_repeat_kind_is_ignored`、`esc_auto_repeat_keeps_underlying_overlay`。

### P0-2 `&&&` 标题源在嵌套子块/子块内围栏下泄漏内容

- **位置**：`src/app/markdown/template.rs:405-438`
- **现象**：`&&&` 中构造 `+++ a --start` → `%%% log --start` → `%%% log --end` → `+++ a --end` 时，`%%% --end` 把唯一的 `subblock_depth` 减到 0，之后的正文与 `+++ a --end` 被当成 `&&&` 直接正文；只有嵌套子块的模板因此返回 `+++ a --end` 而不是空串，`/title` 会拿块标记生成标题。子块内代码围栏中的 `+++ a --end` 也会提前闭合外层。
- **错误原因**：`subblock_depth` 进入子块后只减不增，不区分 `+++`/`%%%`；`subblock_depth > 0` 分支完全绕过 fence 状态；结束行没有匹配标记。
- **修复方向**：改为 fence-aware 的子块栈（按 `+++`/`%%%` 标记匹配，围栏内不改变深度）；围栏优先于子块识别。
- **回归测试**：`template_title_source_only_nested_children_is_empty`、`template_title_source_excludes_log_nested_inside_supple`、`template_title_source_ignores_fenced_end_inside_supple`。

---

## P1

### P1-1 正常 `/supplement` 生成的 `+++` 是 `&&&` 的兄弟块，标题源拿不到外层任务

- **位置**：`src/app/prompt/block.rs:93-103`、`src/app/prompt/template.rs:58-69`、`src/app/prompt/block/tests.rs:62-105`
- **现象**：`/supplement` 把 `+++` 追加到 `&&&` 结束行之后，`+++` 与 `&&&` 是兄弟块；`enclosing_template_body` 只用 `block_scan` 找“包含当前行”的 `Template` span，因此正常流程永远返回 `None`，`+++` 的 `/title` 只包含自身正文，外层任务静默丢失。设计中的“外层 `&&&` + 当前 `+++`”在正常 UI 流程不可复现。
- **错误原因**：把 `+++` 当成 `&&&` 的词法子块；实际协议落点是“关联的前置 `&&&`”。
- **修复方向**：标题关联扫描同时支持“包含当前 `+++` 的 `&&&`”和“最近的前置闭合 `&&&`”；多个 `&&&` 时取最近关联块。更新设计/验收说明，禁止把正常 `/supplement` 流程说成必须手工嵌套。
- **回归测试**：`enclosing_template_body_finds_preceding_template_for_sibling_supple`、`slash_title_in_supplement_after_template_includes_outer_task`。

### P1-2 围栏后的清理不识别围栏，围栏内容被删/折叠

- **位置**：`src/app/markdown/template.rs:440-446`、`:264-333`、`:824-842`
- **现象**：围栏里的 `- name:`、空列表项被 `clean_template_content` 删除；`# Execute Task` 被默认标题过滤；连续空行被折叠；围栏里的 `+++ --start` 还会被当成真子块。现有测试只覆盖围栏标记行，没有覆盖占位项/默认标题/空行。
- **错误原因**：`extract` 是 fence-aware 的，但 post-clean 和默认标题过滤不是；`clean_template_content` 没有 fence 状态。
- **修复方向**：新增标题专用的 fence-aware 清理：只对围栏外片段调用 `clean_template_content` 和默认标题过滤，围栏片段逐字保留；不改变 `copy_template_content` 的复制语义。
- **回归测试**：`template_title_source_keeps_fenced_placeholder_items_and_headings`。

### P1-3 `suspended_overlay` 让被挂起浮层失去异步回执，OAuth/引用文件夹/Worktree 可能永久卡住

- **位置**：`src/app/update/quit.rs:11-15`；消费点 `src/app/update/chat/connect/events.rs:411,431,466-469,509`、`src/app/update/referenced.rs:128-147,163-178`、`src/app/update/referenced/search.rs:117-149`、`src/app/update/worktree.rs:154-185`
- **现象**：`/connect` 刷新模型/OAuth 等待中打开退出确认，回执到达时 `state.overlay` 已是 `ConfirmQuit`，handler 只匹配 live overlay，结果被丢弃；`Esc` 取消后恢复的面板仍停在 `Refreshing…` / `validating=true` / 搜索 loading。`on_preset_oauth_prompt` 在未命中 live Connect 时还会主动 push `CancelPresetOAuth`，等于打开退出确认就取消正在进行的 OAuth。`discard_stale_resume_edit` / `invalidate_referenced_overlays` 也看不到 suspended 浮层。
- **错误原因**：用“移走 `state.overlay`”实现确认层，破坏所有异步 handler 对当前浮层的可见性。
- **修复方向**：改为独立 `quit_confirm` 覆盖层，`state.overlay` 保持不变；渲染和输入优先处理确认层。这样 Connect/Referenced/Worktree 的异步生命周期继续看到原浮层。
- **回归测试**：`suspended_connect_receives_model_refresh_on_cancel`、`suspended_oauth_prompt_does_not_cancel_attempt`、`suspended_referenced_input_restores_not_validating`、`suspended_referenced_search_restores_entries`、`suspended_worktree_loading_retries`。

### P1-4 异步信任弹窗可覆盖退出确认，并可能复活陈旧 suspended 浮层

- **位置**：`src/app/update/trust.rs:17-19`、`src/app/update/quit.rs:11-13`、`src/app/update/menu.rs:222-228`
- **现象**：待解析的 `TrustRootResolved` 在退出确认打开后调用 `open_trust_dialog`，无条件把 `state.overlay` 改成 `TrustWorkspace`，确认框消失，`Enter` 可能变成“授予信任”；如果确认是在 Rename/Connect 上打开的，`suspended_overlay` 仍保存旧浮层，关闭信任后再取消退出可能复活早已关闭的浮层。
- **错误原因**：单一 `Option<Overlay>` 加无条件异步挂载；没有 `suspended_overlay.is_some() <=> ConfirmQuit` 的不变量。
- **修复方向**：独立确认层后，信任弹窗只能在底层更新，确认层始终优先；移除 `suspended_overlay` 与 `request_quit` 的无条件暂存。
- **回归测试**：`quit_confirm_survives_trust_resolution`、`cancel_trust_after_replaced_confirm_does_not_restore_zombie`。

### P1-5 `app` 新增对 `ui::right_panel::prompt::geometry` 的依赖

- **位置**：`src/app/update/pane.rs:11,846`、`src/app/prompt/viewport.rs:16,466`、`src/app/update/prompt/recent.rs:189`、`src/app/update/prompt/tree_view.rs:18`
- **现象**：把根级中性 `layout` 几何搬进 `ui` 后，`app` 直接引用 UI 内部 prompt 几何（`prompt_body_split`、`PROMPT_BLOCK_GUTTER_LEFT`、`PROMPT_TREE_HEADER_HEIGHT`）。这比旧的 `app -> layout` 更差，违反 `AGENTS.md` 的 `app/ui` 边界与 task 3.2 的“确保 app 不依赖 ui”。
- **错误原因**：按目录映射机械替换 import，没有区分“app 实际使用的纯几何”和“UI 专属渲染几何”。
- **修复方向**：把 app 实际使用的纯 prompt/列表几何下沉到 `app::prompt::geometry`（或中性模块）；UI 反向从 app 取。至少让 `rg "crate::ui::right_panel::prompt::geometry" src/app` 归零。
- **回归测试**：`cargo test app::update::prompt app::prompt` + 全量测试；`rg` 验证。

### P1-6 嵌套 `+++` 与多 `&&&` 的标题目标定位仍会截断/丢失

- **位置**：`src/app/markdown/supple.rs:131-167`、`src/app/prompt/template.rs:19-69`
- **现象**：当前 `+++` 内再出现 `+++ inner --start/--end` 时，`supple_body_at_start` 在外层 `+++` 的 inner `--end` 截断，inner 之后的正文丢失；`find_enclosing_supple_block` 也把内层 `+++` 当成外层结束。页面有两个 `&&&` 时，第二个 `&&&` 里的 `+++` 被 `block_scan` 降级成顶层 `Supple`，外层任务静默丢失。
- **错误原因**：`supple_body_at_start` / `find_enclosing_supple_block` 只维护单个 `current_start`，没有嵌套栈；`block_scan` 的 `template_seen` 只认首个 `&&&`。
- **修复方向**：`+++` 扫描改为栈式（fence-aware）；`enclosing_template_body` 用“包含或最近前置 `&&&`”的关联扫描，支持多模板。若产品明确只允许一个 `&&&`，则在 `/title` 入口拒绝孤儿 `+++` 并提示，不能静默 own-only。
- **回归测试**：`nested_supple_block_title_keeps_content_after_inner_end`、`slash_title_in_supple_of_second_template_includes_second_task`。

---

## P2

### P2-1 退出时 `commit_editing_prompt` 实际执行两次

- **位置**：`src/app/update/quit.rs:25`、`src/event/mod.rs:662`、`src/db/service.rs:188-192`
- **现象**：`confirm_quit` 提交一次并 push `SavePromptContent`；主循环退出后 `persistence.shutdown` 又无条件提交一次，`page.touch` 与保存效应重复。
- **错误原因**：shutdown 不知道确认路径已经提交。
- **修复方向**：让 `commit_prompt_buffer` 在内容一致且已 pending 时直接返回，或让 `shutdown` 在确认已提交后跳过 prompt 提交。
- **测试**：`confirm_quit_commits_prompt_once`。

### P2-2 取消退出会丢失选区，已开始的鼠标拖拽仍继续生效

- **位置**：`src/app/update/quit.rs:14`、`src/event/mouse/drag.rs:332-360,378-400,586-600`、`src/event/mouse/press.rs:146-155`
- **现象**：`request_quit` 调用 `clear_selection`，取消后终端/lx/prompt/搜索选区全部丢失；拖动 prompt 树条目、窗格分割线、侧栏/prompt 尺寸、终端滚动条后按 `Ctrl+Q`，继续移动/松开仍会改状态。
- **错误原因**：用清选区绕开 Esc 清选区逻辑（实际已在 ConfirmQuit 分支绕过）；拖拽处理在浮层判断之前执行。
- **修复方向**：删除 `request_quit` 的 `clear_selection`；确认层打开时提前拦截鼠标拖拽/松开，清理活动手势状态但不破坏选区。
- **测试**：`cancel_quit_preserves_terminal_and_lx_selection`、`prompt_drag_does_not_reorder_under_quit_confirm`。

### P2-3 极小终端下任意点击会取消确认，紧凑布局标题与问题行重叠

- **位置**：`src/event/overlay.rs:408-411`、`src/ui/overlay/dialog.rs:635-668`、`src/ui/widgets/modal.rs:40-60`
- **现象**：`quit_confirm_shell` 返回 `None`（宽 <4 或高 <3）时，任意点击执行 `cancel_quit`，违反“点击弹窗外不关闭”；高度 <6 时 `modal::render` 的标题与问题行重叠，问题行可能压到底边框。
- **错误原因**：把“无法显示”误当成“取消”；缺少紧凑高度布局。
- **修复方向**：shell 不可用时吞掉点击，不取消；短屏使用紧凑布局或跳过标题，问题行按 `inner.height` 裁剪。
- **测试**：`tiny_screen_outside_click_keeps_confirm_quit`、`short_screen_does_not_overlap_title_and_question`。

### P2-4 模板块 few-shot 未覆盖 6–20 下界、Auto 英文 5 词限制，示例与约束矛盾

- **位置**：`src/chat/title.rs:47-133`
- **现象**：中文 base 只有“at most 20”，没有 6 字下界；`Auto` 英文只有 36 字符，没有“<=5 words”；`Add scrollbar to task panel` 是 5 词，与英文 base 的“under five words”矛盾；few-shot 是 `-` 列表，模型容易回显列表。
- **错误原因**：模板指令直接复用会话 base，只在末尾追加简中/英文示例。
- **修复方向**：模板专用指令显式写“6–20 Chinese characters / at most 36 English characters and 5 words”；示例改成不超过 5 词；在示例后强调“Return exactly one title, not a list”。
- **测试**：`template_prompt_specifies_six_to_twenty_chinese_chars`、`template_prompt_auto_en_specifies_five_words`。

### P2-5 `sanitize_title` 单次清理顺序会留下前缀/引号/强调/标点；拒答子串误杀合法标题

- **位置**：`src/chat/title.rs:324-382`、`:403-464`
- **现象**：`Title: - Fix bug` 留下 `-`；`- # Fix bug` 留下 `#`；`**Fix bug**` / `_Fix bug_` / `「Fix bug」` 原样保留；`"Fix bug".` 留下引号；`{title: Fix bug}` 留下 `title:`；`Fix bug…` / `，` / `）` 不裁。`is_refusal_title` 用 `contains` 匹配 `无法生成标题`，会把合法标题“修复无法生成标题的 bug”误杀。
- **错误原因**：清理只跑一遍且顺序固定；没有 emphasis/括号处理；拒答匹配过宽。
- **修复方向**：循环清理直到稳定，先去 braces/quotes，再 `#`/list/label/emphasis/punct；拒答短语改为整行或开头匹配，保留“修复无法生成标题”这类合法标题。
- **测试**：`sanitize_title_strips_mixed_prefixes_and_emphasis`、`parse_generated_title_keeps_legitimate_title_generation_task`。

### P2-6 测试组织与 1500 行上限

- **位置**：`src/app/update/prompt/tests.rs:1`（2516 行）、`src/app/update/tests.rs:28`（5721 行）、`src/input/tests.rs:480`（1537 行）、`src/ui/tests.rs:1482`（3321 行）
- **现象**：生产模块拆出 `items/menu/save`、`quit`、`frame`，测试仍堆在父模块根 `tests.rs`；新增 quit 测试继续塞进大文件；`input/tests.rs` 已越过 1500 行。
- **错误原因**：设计文档的“tests.rs 保持一个测试模块”与项目标准冲突，实现按设计执行。
- **修复方向**：本次新增的 quit/input 测试先落到 `quit/tests.rs`、`input/tests/quit.rs`；既有巨型测试文件按场景拆分（可分批，不要求一次全拆完）。更新 task.md 删除冲突表述。
- **验证**：`wc -l` 检查本次触及的新测试文件；全量测试。

### P2-7 文档、注释与实现偏差

- **位置**：`src/ui/controls/exit.rs:23`、`src/app/state.rs:319`、`docs/feature/quit-confirm-title-and-naming/design.md:3,796-799`、`task.md`、`docs/standards/tui-design-requirements.md:49`
- **现象**：`exit.rs` 注释仍写“点击退出应用”；`state.rs` overlay 注释未列 ConfirmQuit；task.md 全部 `[ ]` 且状态为“待用户审核”；design.md manager 拆分表遗漏 `lifecycle.rs`、controls 常量位置写成 `mod.rs`；`tui-design-requirements.md` 仍写已不存在的 `ui::prompt::token_style`。
- **错误原因**：实现中临时调整（lifecycle.rs、独立确认层）没有回写设计；文档未同步。
- **修复方向**：本次修复后统一更新 design/task/standards/注释，记录实际架构与验证结果。

### P2-8 `prompt::geometry` 实际是右栏通用几何，Todo/左栏反向依赖 prompt 内部

- **位置**：`src/ui/right_panel/todo.rs:61,177,583`、`src/ui/right_panel/todo/schedule.rs:161,164`、`src/ui/right_panel/mod.rs:39,70`、`src/ui/left_panel/history.rs:10`
- **现象**：Todo 和左侧栏历史使用 `prompt::geometry` 的 `prompt_body_rect` / `prompt_toolbar_rect` / `PROMPT_TREE_HEADER_HEIGHT`，命名和归属误导；改 prompt 布局可能影响 Todo/左栏。
- **错误原因**：`src/layout/prompt.rs` 整体搬进 prompt 子模块，没有区分“右栏通用”和“prompt 专属”。
- **修复方向**：把右栏通用几何上移到 `ui/right_panel/geometry.rs`，`PROMPT_TREE_HEADER_HEIGHT` 改为共享列表头高度或左栏本地常量；prompt 专属几何留在 `prompt/geometry.rs`。若本次不做，至少在文档中标注共享语义。

### P2-9 重复 helper 与文件体量

- **位置**：`src/app/update/prompt/mod.rs:317`、`src/app/update/prompt/pages.rs:744`、`src/ui/text.rs:1`（1002 行）
- **现象**：两个 `workspace_cwd` 函数体完全相同；`pages.rs` 内部继续用自己的副本。`text.rs` 已到 1002 行，新增 `EXIT_CONFIRM_*` 后无余量。
- **错误原因**：拆分时保留根 helper 与原副本；退出文案直接加在已临界的大文件。
- **修复方向**：删除 `pages.rs` 副本，统一 `super::workspace_cwd`；把退出文案移到 `ui/text/workspace.rs` 或新建 `ui/text/quit.rs` 并 `pub use`，保持 `text.rs` ≤1000 行。

### P2-10 公共 API 破坏性变更未说明

- **位置**：`src/lib.rs:15,22`、`src/ui/mod.rs:3-11`、`src/app/prompt/tree/panel.rs:15`
- **现象**：`lx_tui::layout`、`ui::sidebar`、`ui::prompt`、`ui::prompt_frame`、`layout::PromptMode` 等公共路径消失，没有 deprecated alias 或迁移说明。
- **错误原因**：按内部结构重构处理，未评估 `pub` 模块的外部使用者。
- **修复方向**：若仓库无外部使用者，在 design/README 明确 breaking change 与新路径；否则加临时 deprecated alias。本次优先记录，不引入兼容层除非确认有外部调用。

---

## 修复顺序与验收

1. P0-1、P0-2：先补失败测试，再修。
2. P1-1 至 P1-6：退出确认改独立层；标题关联/嵌套/fence 清理；app 依赖下沉。
3. P2-1 至 P2-5、P2-7、P2-9：低风险修正。
4. P2-6、P2-8、P2-10：按成本处理，至少更新文档与新增测试落点，避免继续扩大旧债。
5. 最终：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`；同步 `design.md` / `task.md` 并记录“未修/延后”项。

---

## 修复结果（2026-11-11）

| 编号 | 状态 | 修复摘要 |
| --- | --- | --- |
| P0-1 | 已修复（后续改为 toast） | 保留 300ms 去抖与 `Repeat` 忽略；最终移除独立 `quit_confirm` 层，改为全局 toast + 2s 确认窗口，无取消/latch 路径 |
| P0-2 | 已修复 | `extract_block_title_source` 改为围栏优先的 `+++`/`%%%` 标记栈；嵌套结束/围栏内结束不再提前闭合 |
| P1-1 | 已修复 | `enclosing_template_body` 支持“包含当前 `+++` 的 `&&&`”与“最近前置闭合 `&&&`”，正常 `/supplement` 可带出外层任务 |
| P1-2 | 已修复 | 标题源专用 fence-aware 清理；围栏内占位项、默认大标题、空行逐字保留 |
| P1-3 | 已修复 | 不再移走 `state.overlay`；Connect/Referenced/Worktree 异步回执继续命中 live 浮层 |
| P1-4 | 已修复（后续改为 toast） | 最终无模态层；信任弹窗与退出 toast 互不阻塞，异步回执正常更新底层浮层 |
| P1-5 | 已修复 | app 使用的纯几何下沉到 `src/app/prompt/geometry.rs`；`rg "crate::ui::right_panel::prompt::geometry" src/app` 零命中 |
| P1-6 | 已修复 | `+++` 扫描改栈式；支持嵌套 `+++`、多 `&&&` 与第二个模板内的 `+++` 关联 |
| P2-1 | 已修复 | `prompt_committed_for_quit` 标记使确认退出只提交一次；普通退出仍由 shutdown 提交 |
| P2-2 | 已修复（后续改为 toast） | 请求退出不再清文本选区；最终不拦截拖拽/滚轮（无模态层），只在请求时清理活动手势 |
| P2-3 | 已修复（后续移除模态） | 退出模态与短屏几何问题随需求移除；全局 toast 由现有 toast 层负责窄屏折行 |
| P2-4 | 已修复 | 模板指令补充 6–20 中文字符、≤36 字符且 ≤5 词、示例改 4 词、示例后要求单条标题 |
| P2-5 | 已修复 | `sanitize_title` 循环清理；拒答改为整句/句首匹配，保留“修复无法生成标题的 bug” |
| P2-6 | 部分修复 | 新增 quit 测试落在 `app/update/quit/tests.rs`；`input/tests.rs` 回到 1500 行；既有超限测试文件（`app/update/prompt/tests.rs`、`app/update/tests.rs`、`ui/tests.rs`）延后拆分 |
| P2-7 | 已修复 | 本次同步更新 design/task/standards/review 与源码注释 |
| P2-8 | 已修复 | 右栏通用几何下沉到 `ui/right_panel/geometry.rs`；左栏历史改用本地 `LIST_HEADER_HEIGHT` |
| P2-9 | 已修复 | 删除 `pages.rs` 重复 `workspace_cwd`；退出 toast 文案移入 `ui/text/workspace.rs`，`text.rs` 回到 1000 行内 |
| P2-10 | 记录，未加兼容层 | 仓库按内部重构处理公共路径变更；迁移说明写入 design 文档；未确认外部使用者前不加 deprecated alias |

### 最终验证

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：通过
- `cargo test`：移除退出模态后 5222 个 lib 测试通过，集成测试通过；`tests/prompt_copy.rs` 的 pbcopy 桩改为原子写入，消除全量并发下的空文件竞态
- 旧架构检索：`suspended_overlay` / `Overlay::ConfirmQuit` / `OverlayKind::ConfirmQuit` 零命中
- 退出模态检索：`quit_confirm` / `QuitConfirmState` / `quit_cancel_latch` / `cancel_quit` / `render_quit_confirm` 零命中
- 退出提示为全局 toast（`anchor = None`）；`Ctrl+Q`/`[EXIT]` 首次请求只提示，2s 窗口内再次请求才退出
- 依赖方向：`rg "crate::ui::right_panel::prompt::geometry" src/app` 零命中
- 文件体量：`src/ui/text.rs` 已低于 1000 行；`src/input/tests.rs` 为 1500 行（上限内）
- 合并 dev 后：`chat/process/manager` 保留 50 KiB 输出边界；`history/tool.rs` 抽出 `header.rs`，两者均低于 1000 行

### 合并与工作区清理

- 2026-11-11 将 `feat/quit-confirm-title-and-naming` 合并入 `dev`。
- 冲突处理：
  - `src/chat/process/manager/{mod,handle}.rs`：合并本分支的 `types/handle/lifecycle` 拆分与 dev 的 `feat/lx-read-output-truncation` 输出边界逻辑；`handle.rs` 只保留 `ProcessHandle`/`StdinRequest`，`stop_handle`/`resolve_cwd`/`StopWaiter` 在 `lifecycle.rs`。
  - `docs/standards/project-directory-structure.md`：合并 `history/tool/group.rs`、`history/tool/header.rs`、`right_panel/geometry.rs` 等 dev/本分支描述。
- 合并后 `ui/main_content/lx/history/tool.rs` 超过 1000 行，标题/分段/网格写入/折行原语抽到 `history/tool/header.rs`；测试全绿。
- 原 worktree `.worktrees/quit-confirm-title-and-naming`、分支 `feat/quit-confirm-title-and-naming` 与隔离数据库已删除；主工作区为 `/Users/yonah/projects/agent/lx-tui`。

### 未完成/需人工验收

- `cargo run` 真实 TUI：长按 `Ctrl+Q`（只显示一次 toast，不退出）、2s 窗口内第二次 `Ctrl+Q`/`[EXIT]` 退出、窗口过期后重新提示；`/connect` OAuth/引用文件夹弹窗打开期间 toast 不阻塞底层交互；`/supplement` 后 `+++` `/title`、多模板/嵌套 `+++`、短屏布局。
- 既有超限测试文件拆分（P2-6 的一部分）作为后续独立重构，不与本次行为修复混在一起。
