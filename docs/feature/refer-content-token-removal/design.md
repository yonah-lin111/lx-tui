# 移除 `@[ref-content]/` 快捷引用标记设计

状态：**已实施、通过评审修正并合并到 `dev`**（2026-11-14）。用户已确认：全链路删除；字母快捷面板改为插入候选原文。评审记录见 `review.md`。

关联任务清单：`docs/feature/refer-content-token-removal/task.md`

---

## 1. 背景与目标

字母快捷引用面板（在输入框里输入若干字母，复用正文中**已经出现过**的引用）当前确认时统一插入

```text
@[ref-content]/<末两段路径> 
```

该标记是截断形式，必须依赖 `chat/prepare` 的工作区后缀扫描才能还原完整路径，额外带来解析、高亮、整块退格等一整套专用逻辑。

目标：

1. **移除 `@[ref-content]/` token 标记的全链路**：不再产生、不再解析、不再高亮、不再整块删除。
2. 字母快捷面板确认时**插入候选原文**（`@docs/.../design.md`、`@[refer-folder](/abs)`、`@ref/...`），不再截断路径。普通完整引用的语义不变；`@ref/` 仍遵循既有规则（同一消息内需存在唯一完整引用），旧 ref-content 的隐式工作区后缀扫描随标记一起删除。

非目标：

- 不改 `@` 文件提及面板（它本来就插入 `@相对路径`）。
- 不改 `@[refer-file|folder|image|skill|mcp|subagent](...)` 语法。
- 不改 `@ref/` 遗留短引用（解析、高亮、整块删除、`prepare` 第 4 步、按短名去重全部保留）。
- 不改数据库 schema；不做数据迁移。

---

## 2. 已确认决策

| 决策 | 内容 |
| --- | --- |
| D1 | 全链路删除 `@[ref-content]/`：插入、解析、高亮、整块退格、`prepare` 后缀解析、文档与测试 |
| D2 | 快捷面板确认插入 `candidate.text` 原文 + 一个尾随空格 |
| D3 | `@ref/` 遗留短引用维持现状，不随本次删除 |
| D4 | 不迁移旧草稿：库中已有的 `@[ref-content]/x/y` 变成普通文本，不再挂载文件 |

---

## 3. 现状与影响面

| 位置 | 现状 | 处理 |
| --- | --- | --- |
| `src/app/markdown/ref_content.rs` | 前缀、格式化、区间识别、整块删除、路径提取 | 删除；路径提取迁出 |
| `src/app/markdown/refer_shortcut.rs` | 候选来源含 `RefContent`；确认统一 `format_ref_content` | 删除来源与格式；改插原文 |
| `src/app/prompt.rs`、`src/app/state/lx/edit.rs` | 退格整块删除 `ref_content_deletion_range` | 删除该分支 |
| `src/ui/markdown.rs`、`src/ui/style.rs`、`src/ui/right_panel/prompt/mod.rs` | `TokenKind::RefContent` 专用着色 | 删除 |
| `src/chat/prepare.rs`、`src/chat/prepare/web.rs` | `@[ref-content]/` 后缀扫描与附件解析 | 删除 |
| `src/app/state/lx/chat.rs` | 历史消息路径提取依赖 `extract_mention_paths`（含 ref-content） | 函数迁到 `mention_paths.rs`，只提取普通 `@` 提及 |
| `src/ui/main_content/lx/history/paint.rs` | 附件树过滤 `@[ref-content]/` 标签 | 删除过滤（新数据不再产生该标签） |
| `tests/prompt_short_ref.rs`、各模块测试 | 断言截断格式与专用高亮 | 改为原文插入并删除过时断言 |

---

## 4. 详细设计

### 4.1 快捷面板插入原文

```rust
/// 确认替换文本：插入候选原文并补一个尾随空格，便于继续输入。
pub fn refer_insertion(candidate: &ReferCandidate) -> String {
    format!("{} ", candidate.text.trim_end())
}
```

- `candidate.text` 就是正文中该引用首次出现时的原始文本，因此文件、目录、`@[refer-*]`、`@ref/...` 全部保持原语义。
- `trim_end` 只去掉原文多余尾随空白，避免双空格。
- 不再截断为末两段，长路径按原样插入。
- **边界**：单独确认一个悬空的 `@ref/末两段` 不会额外解析文件；它仍要求同一消息内存在唯一完整引用。旧流程能做到这一点，是因为 `@[ref-content]` 会做工作区后缀扫描；该能力已按 D1 删除。

### 4.2 候选收集去掉 `RefContent`

`refer_candidates` 删除 `Source::RefContent` 分支及对应区间收集。普通 mention 区间直接跳过以 `@ref/` 开头的 token：尾句点会让 mention 区间比短引用多一个字符，只靠区间包含判断会产生双候选。其余来源（普通 `@`、`@[refer-*]`、`@ref/`）与排序、去重规则不变。

### 4.3 历史消息路径提取

`extract_mention_paths` 原本在 `ref_content.rs`，同时承担 `@[ref-content]/` 与普通 `@` 的提取。删除标记后迁到 `src/app/markdown/mention_paths.rs`，复用 `markdown::mention_token_ranges`，只过滤 `@[refer-` 与 `@ref/`：

```text
跳过 @ref/ 与 @[refer- 前缀；按行首/空白边界解析 @路径；围栏内不解析。
```

调用方 `src/app/state/lx/chat.rs` 不变。

### 4.4 `prepare` 清理

- 删除 `chat/prepare.rs` 第 3.5 步（ref-content 后缀扫描）与 `resolve_ref_content_target`。
- 删除 `chat/prepare/web.rs` 第 3 步与 `resolve_ref_content_target` 导入。
- 普通 `@路径`（第 3 步 / 第 2 步）行为不变。

### 4.5 UI 清理

- 删除 `TokenKind::RefContent`、`ui/style.rs::markdown_ref_content` 与 prompt 渲染分支。
- `@[ref-content]/...` 文本此后按普通正文渲染；`@ref/...` 仍有独立高亮。
- 附件树不再过滤旧标签，但展示层把 label 以 `@[` 开头且带 `source_path` 的旧附件还原成可读路径，不暴露内部 token。

---

## 5. 不变量

1. 字母快捷面板仍只从当前分段已出现的引用生成候选，不扫描文件系统。
2. 确认后插入的文本必须与候选原文逐字一致（仅补一个尾随空格）。
3. `@` 文件提及面板、`@[refer-*]`、`@ref/` 的解析与附件语义不变。
4. 生产代码中不再存在 `@[ref-content]/` 前缀、格式化函数、区间解析、专用高亮或 prepare 分支；测试保留该字面量仅用于锁定旧数据的降级行为。
5. 不改数据库 schema，不写迁移。

---

## 6. 测试计划

### 6.1 快捷面板

文件：`src/app/markdown/refer_shortcut/tests.rs`、`src/app/prompt/tests.rs`、`src/app/update/tests.rs`、`src/app/state/lx/refer/tests.rs`

- 确认文件候选插入原文：`@docs/feature/x/design.md `。
- 确认 `@[refer-image]` / `@[refer-folder]` 候选插入原标记，且目录保留尾 `/`。
- 确认 `@ref/...` 候选重新插入 `@ref/...`，不再转换。
- 删除 `@[ref-content]/` 专用插入与整块退格断言；`@ref/` 整块删除仍保留。

### 6.2 解析与历史

- `chat/prepare/tests.rs`：删除 ref-content 两个用例；普通 `@` 与 `@[refer-*]` 用例保持。
- `src/app/state/lx/chat/steps/tests.rs`：把夹具里的 ref-content 文本换成普通 `@docs/readme.md`，断言不变。
- `mention_paths` 单测：只提取普通 `@`，跳过 `@ref/`、`@[refer-` 与围栏内容。

### 6.3 UI

- `src/ui/markdown/tests.rs`：删除 ref-content 高亮用例；`@ref/` 与普通 `@` 高亮用例保留。
- `tests/prompt_short_ref.rs`：更新插入断言为原文；删除 ref-content 高亮与整块删除用例。

### 6.4 回归

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

---

## 7. 风险与回滚

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| 旧草稿不再挂载 | 库中已有 `@[ref-content]/x/y` 变成普通文本 | 已确认不做迁移；重新选择候选即可生成正常 `@路径` |
| 历史消息附件树多一行 | 旧消息仍有 ref-content 标签附件 | 仅影响旧数据；新消息不再产生 |
| 长路径变长 | 不再截断为末两段 | 与用户要求一致；`@` 提及本来也是完整路径 |

回滚：改动集中在本地纯函数与 UI 分支，不涉及 schema；按文件回退即可。

---

## 8. 实施结果

### 8.1 代码改动

| 文件 | 改动 |
| --- | --- |
| `src/app/markdown/refer_shortcut.rs` | `refer_insertion` 插入候选原文；删除 `Source::RefContent` 与相关区间过滤 |
| `src/app/markdown/ref_content.rs`、`ref_content/` | 删除 |
| `src/app/markdown/mention_paths.rs`、`mention_paths/tests.rs` | 新增；迁入 `extract_mention_paths`，只提取普通 `@` 提及 |
| `src/app/prompt.rs`、`src/app/state/lx/edit.rs` | 退格整块删除去掉 ref-content 分支 |
| `src/chat/prepare.rs`、`src/chat/prepare/web.rs` | 删除 ref-content 后缀解析与 `resolve_ref_content_target` |
| `src/ui/markdown.rs`、`src/ui/style.rs`、`src/ui/right_panel/prompt/mod.rs` | 删除 `TokenKind::RefContent` 与专用样式 |
| `src/ui/main_content/lx/history/paint.rs` | 附件树不再过滤 ref-content 标签 |
| `src/app/state/lx/chat.rs` | 注释更新（历史提取只含普通 `@`） |

### 8.2 测试

- `refer_shortcut`：原文插入（文件 / `@[refer-folder]` / `@ref/`）、去重后仍插首个完整原文、尾句点短引用只产生一个候选。
- `mention_paths`：普通 `@` 提取、跳过 `@ref/` 与 `@[refer-`、围栏内忽略。
- prompt / lx / update / 集成测试：快捷面板确认后插入原文；`@ref/` 高亮与整块删除保留。
- prepare：旧 `@[ref-content]` 文本 0 附件且正文原样；完整引用 + 短引用两条附件指向同一目标。
- 历史展示：旧 `@[ref-content]` 附件标签显示为 `source_path`。
- 删除 ref-content 专用插入、高亮、退格、prepare 解析与附件树过滤用例。

验证结果（在本 worktree）：

- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：lib 5436 passed / 0 failed（6 ignored），全部集成测试通过。
- `rg "@\[ref-content\]|REF_CONTENT_PREFIX|ref_content|RefContent" src tests`：无残留。

### 8.3 未验证

- TUI 交互与渲染需用户 `cargo run` 确认（验收清单见 `task.md`）。
- 旧草稿/历史消息中的 `@[ref-content]/...` 已按约定变为普通文本。


---

## 9. 评审修正（review.md）

| 级别 | 问题 | 处置 |
| --- | --- | --- |
| P2 | `@ref/` 候选单独发送不再解析 | 按 D2/D3 接受：插入候选原文；合法短引用仍要求同消息完整引用，补测试锁定 |
| P2 | 尾句点产生 mention + 短引用双候选 | `refer_candidates` 跳过 `@ref/` 开头的 mention 区间 |
| P2 | 旧 ref-content 标签暴露内部 token | 展示层用 `source_path` 还原可读标签 |
| P2 | 测试名/断言与原文插入语义脱节 | 改名、改消息、替换恒真断言为精确插入断言 |
| P2 | 缺少旧 token / 短引用 / 旧标签覆盖；扫描逻辑三份重复 | 补测试；`extract_mention_paths` 复用 `mention_token_ranges` |
