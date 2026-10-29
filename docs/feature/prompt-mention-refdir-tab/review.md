# Prompt @ 命令面板 refDir Tab 与工具栏调整 — 代码评审报告

## 评审概述

针对分支 `feat/prompt-mention-refdir-tab` 的实现进行全面评审，涵盖 Prompt 工具栏按钮、通用 `CommandPanel` 顶栏 Tab 渲染、`MentionState` 领域模型、键盘左右切 Tab 路由、鼠标命中及引用文件夹过滤与插入逻辑。

---

## 问题清单（按优先级排序）

### P0：面板未处于可视状态时，左右方向键可能被 `route_mention_panel` 拦截，导致编辑器光标无法移动

- **关键点**：`EditorCommand::Left` 与 `EditorCommand::Right` 的事件分发守卫。
- **错误原因**：
  在 `src/app/update/editor.rs` 的 `route_mention_panel` 中，原实现在入口处未先判断 `state.prompt.mention_active()`（即面板是否处于实际绘制可视态），而是直接对 `Left`/`Right` 执行 `state.prompt.mention_switch_tab()`。若 `MentionState` 的 `trigger` 存在但面板未处于可视态（如尚未加载完成或被关闭），`switch_tab` 若返回 `true` 会导致用户的常规左/右键移动光标操作被吞掉，造成编辑器光标卡死。
- **修复方案**：
  1. 在 `route_mention_panel` 函数入口统一增加 `if !state.prompt.mention_active() { return false; }` 快速短路，确保面板未可见时所有命令（包括上下、左右、回车）安全回退给普通编辑器处理；
  2. `MentionState::switch_tab` 与 `set_tab` 必须严格依赖 `self.view_open()`，不可在面板不可见时允许状态切换。

---

### P1：`Files` Tab 下当查询词在工作区无匹配时面板瞬间关闭，导致用户无法按右键切换至 `refDir` Tab

- **关键点**：`Files` Tab 空匹配时的面板生命周期与 Tab 连续性。
- **错误原因**：
  在单 Tab 时代，`filter_mentions` 返回空列表时将 `self.panel = None`，面板直接隐藏是合理的。但在引入双 Tab 后，若用户输入 `@xxx` 是为了引用已添加的外部引用文件夹 `xxx`，而当前工作区恰好没有以 `xxx` 命名的文件，工作区过滤返回空列表导致面板直接关闭，用户甚至看不到 `[Files] [refDir]` 顶栏，从而无法按 `Right` 方向键或鼠标点击切换到 `refDir` Tab。
- **修复方案**：
  1. 在 `MentionState::view_open()` 中，当会话处于激活状态（`trigger.is_some() && !suppressed`）且文件扫描缓存已到达（`cache.is_some()`）时，即便当前匹配结果为空也保持 `view_open = true`；
  2. 在 `src/ui/prompt/panels.rs` 中，若处于 `Files` Tab 且 `panel` 过滤为空时，渲染单行置灰的 `(no matches)` 空状态提示（`text::LX_MENTION_EMPTY`），保留顶栏 `[Files] [refDir]` 与底栏提示，确保用户能随时按 `Right` 键切换到 `refDir` Tab。

---

### P2：`refDir` 模式下过滤词变化时未安全钳制 `ref_viewport` 视口起点

- **关键点**：`ref_viewport` 视口有效性防护。
- **错误原因**：
  在 `refDir` 模式下，用户若先通过鼠标滚轮将视口向下滚动至较大偏移量（如偏移量为 5），随后继续输入字符收窄过滤范围（如匹配结果从 10 项缩减至 1 项），若未在 `refresh` 中重置或钳制 `ref_viewport`，视口起点仍为 5，可能导致可见行超出结果总数，面板内容区呈现空白。
- **修复方案**：
  在 `MentionState::refresh` 中计算 `filtered_count` 时，若 `ref_viewport` 存在且大于等于 `filtered_count`，同步重置 `ref_viewport = None`。

---

### P3：测试用例未覆盖带前缀查询词时 `Files` 0 匹配无缝切换到 `refDir` 的端到端场景

- **关键点**：跨 Tab 查询保留与空匹配保护测试。
- **错误原因**：
  现有测试主要验证了直接输入 `@` 以及在各 Tab 下的独立行为，缺少“用户先输入 `@external`，在 `Files` 无匹配时保持面板并按右键切到 `refDir` 命中对应目录”的集成测试用例。
- **修复方案**：
  在 `src/app/prompt/mention/tests.rs` 及 `tests/prompt_editor.rs` 中补充针对 P1 场景的回归测试用例。

---

## 修复执行结果

| 优先级 | 问题简述 | 修复状态 | 验证结果 |
| --- | --- | --- | --- |
| **P0** | 面板未处于可视状态时，`Left`/`Right` 键被拦截导致编辑器光标无法移动 | **已修复** | `route_mention_panel` 入口增加 `mention_active()` 守卫；新增集成测试 `p0_cursor_left_right_works_when_mention_panel_is_not_active` 通过 |
| **P1** | `Files` Tab 0 匹配时面板关闭，无法切到 `refDir` | **已修复** | 保持会话期间 `view_open` 稳定并在空匹配时渲染 `(no matches)` 占位行；新增测试 `files_empty_matches_keeps_view_open_allowing_tab_switch` 通过 |
| **P2** | `refDir` 模式下过滤词收窄时 `ref_viewport` 视口未安全钳制 | **已修复** | 在 `MentionState::refresh` 中当 `ref_viewport >= filtered_count` 时安全复位；新增测试 `ref_dir_viewport_clamps_on_query_narrowing` 通过 |
| **P3** | 测试用例覆盖带查询词跨 Tab 切换与端到端引用插入场景 | **已修复** | 单元测试与集成测试（13 个测试项全部通过）覆盖完整交互路径 |

