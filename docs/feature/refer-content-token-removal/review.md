# 移除 `@[ref-content]/` 快捷引用标记评审

审查对象：`083b5bab`（`refactor(mention): drop @[ref-content] shortcut token`）及当前工作区后续修正。
审查方式：对照设计 D1–D4 检查插入、解析、高亮、退格、prepare、历史展示与测试；对边界输入做定向测试。
状态：**已按本文件修复并验证**。

## 结论

| 级别 | 数量 | 说明 |
| --- | --- | --- |
| P0 | 0 | 未发现崩溃或数据损坏 |
| P1 | 0 | — |
| P2 | 5 | 短引用单独发送不再解析（按设计接受）、尾句点双候选、旧标签暴露、测试失效、扫描逻辑重复 |

---

## P2

### P2-1 确认 `@ref/` 候选后原样插入短引用，单独发送时不再挂载文件（按设计接受）

**现象**
快捷面板中确认一个 `@ref/x/design.md` 候选，插入的仍是 `@ref/x/design.md	`。如果同一消息里没有对应的完整引用 `@docs/feature/x/design.md`，发送后该短引用不产生附件。

**错误原因**
旧流程确认短引用候选时插入 `@[ref-content]/...`，由 `prepare` 的“工作区后缀扫描”隐式恢复完整路径。本次按 D1 删除了该 token 与扫描，按 D2 改为插入候选原文；`@ref/` 的既有解析规则（同一消息内必须存在唯一完整引用）没有改变。

**处置：不修，按 D2/D3 接受**
- D2 明确要求插入候选原文；D3 明确保留 `@ref/` 的既有解析语义，不恢复 ref-content 的后缀扫描。
- 合法的 `@ref/末两段` 仍可解析：同一消息中存在完整引用时，`prepare` 第 4 步按短名匹配。
- 单独、悬空的 `@ref/...` 在旧流程中能工作，是 `@[ref-content]` 隐式扫描带来的副作用，不是 `@ref/` 的契约。

**补充验证**
- `prepare_resolves_short_ref_when_full_reference_exists`：完整引用 + 短引用 → 2 条附件，目标路径一致。
- 设计文档已把“语义与分辨率完全不变”修正为“候选原文语义不变；短引用仍需同消息完整引用”。

### P2-2 `@ref/...` 尾随句点同时产生 mention 与短引用两个候选

**现象**
正文 `@ref/foo/bar.md.` 后输入 `bar`，面板出现两个候选：`@ref/foo/bar.md.`（file）与 `@ref/foo/bar.md`（ref-file）。默认选中前者，确认后插入带句点的死引用。

**错误原因**
`short_ref_ranges` 会剔除句尾 `.`，`mention_token_ranges` 不会；原过滤条件只判断“mention 区间完全包含于短引用区间”，带句点时 mention 多一个字符，过滤失败。

**修复**
`refer_candidates` 直接跳过以 `SHORT_REF_PREFIX` 开头的 mention 区间，统一由短引用来源处理。

**验证**
`candidates_do_not_double_count_short_ref_with_trailing_period`。

### P2-3 历史消息中的旧 `@[ref-content]` 附件行会暴露内部 token

**现象**
旧消息附件树的标签是 `@[ref-content]/lx/refer.rs`，而不是可读路径。

**错误原因**
`UserRefLine::from_attachments` 原样渲染 `label`；旧版 prepare 把 label 写成内部标记。删除附件树过滤后没有替代的可读化处理。

**修复**
展示层 `display_label`：label 以 `@[` 开头且存在 `source_path` 时显示 `source_path`。同时去重键使用规范化后的展示目标。

**验证**
`user_ref_line_renders_legacy_token_label_as_path`。

### P2-4 测试命名/断言与“插入原文”语义脱节

**现象**
- `tests/prompt_short_ref.rs` 注释仍写“短名带尾斜杠”。
- `assert!(!text.contains("@/agent"))` 在新语义下恒真，无法验证任何行为。
- `src/app/prompt/tests.rs` 的失败消息仍写“带尾斜杠且保留末两段”。
- 测试名 `refer_shortcut_reference_candidates_insert_short_ref` 实际断言原标记。

**修复**
- 改名并更新消息。
- 无效否定断言替换为整段插入文本的精确 `assert_eq!`。
- 短引用重选用例补上同目标完整引用，验证“短引用仍可解析”的有效场景。

### P2-5 关键回归覆盖缺失，`mention_paths` 复制了第三套 `@` 扫描

**现象**
删除 ref-content 用例后，没有测试锁定：
1. 旧 `@[ref-content]/...` 文本发送后 0 附件且正文原样保留；
2. 单独 `@ref/` 的解析行为；
3. 尾句点双候选；
4. 旧附件标签展示。

`mention_paths::extract_mention_paths` 与 `markdown::mention_token_ranges` 几乎逐行重复，边界规则已出现分歧（P2-2）。

**修复**
- 补 `prepare_treats_legacy_ref_content_token_as_plain_text`、`prepare_resolves_short_ref_when_full_reference_exists`、尾句点候选测试、旧标签展示测试。
- `mention_token_ranges` 提升为 `pub(crate)`；`extract_mention_paths` 改为复用它，只过滤 `@[refer-` 与 `@ref/`。

---

## 已核查未发现问题

- `refer_insertion` 对文件、目录、`@[refer-*]`、`@ref/` 均插入 `candidate.text` 原文 + 一个尾随空格；目录尾 `/` 保留。
- 旧 `@[ref-content]/...` 文本按普通正文渲染，不误高亮、不 panic、不挂载附件。
- 普通 `@`、`@[refer-*]`、`@ref/` 的 prepare 行为未因删除 ref-content 改变。
- `rg "ref_content|RefContent|REF_CONTENT" src tests` 无残留（历史 docs 除外）；展示层仅用通用的 `@[` 前缀判断旧标签。

## 验证

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- 定向：`refer_shortcut`、`mention_paths`、`prompt_short_ref`、`prepare_treats_legacy`、`prepare_resolves_short_ref`。

未覆盖：TUI 视觉与交互，需用户 `cargo run` 确认。
