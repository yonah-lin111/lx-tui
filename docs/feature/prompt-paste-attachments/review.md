# Prompt 粘贴路径与引用快捷输入 Review

审查基线：`dev...85b3db8`；以本 worktree 的 `design.md` §14.3/§14.4 与现有项目规范为准。只审查，未修改功能代码。

## Findings

### P1：未确认的路径占位会被保存，取消后数据库与界面分叉

`src/app/prompt/paste.rs:296-320` 在打开选择面板前调用 `replace_at` 写入路径占位并置脏。`src/app/update.rs:1141-1148` 的 800 ms 自动保存没有检查粘贴面板；`src/event/mod.rs:609-612` 的手动保存也直接进入 `save_prompt_shortcut`，不经过面板按键拦截。用户粘贴有效路径，保持面板打开等待 800 ms（或按 Ctrl/Cmd+S），占位会经 `commit_prompt_buffer` 写入数据库。随后 Esc 调用 `paste_cancel`，仅在内存中弹出撤销快照并恢复 `is_saved`/修订号（`src/app/prompt/paste.rs:364-382`），没有发出逆向保存效应。重开条目后已取消的路径重新出现。

建议：提案未确认时不要让占位参与保存，或取消后确保数据库与内存同步；手动和自动保存走同一门禁。补“面板打开超过自动保存延迟”和“面板打开时手动保存后 Esc、重开条目”的回归测试。

### P2：取消粘贴会清空此前的 Redo 历史

`src/app/prompt/paste.rs:399-408` 的占位写入调用 `record(EditKind::Other)`；`src/app/prompt.rs:801-810` 的 `record` 无条件清空 `redo`。取消时 `restore_staged` 只弹出并恢复 `undo` 顶部快照（`src/app/prompt/paste.rs:375-382`），没有恢复被清空的 `redo`。复现：输入 `abc` → Undo → 粘贴有效路径 → Esc；正文回到空白，但原本可 Redo 的 `abc` 永久消失。文档承诺取消不留编辑痕迹，此处不满足。

建议：面板确认前不要提交编辑历史，或在提案状态中完整保存并恢复 redo/编辑分组与自动保存观察状态。补取消前已有 Redo 分支的测试。

### P2：代码围栏里的 `@` 提及进入快捷候选

`src/app/markdown.rs:514-540` 的 `mention_token_ranges` 扫描全文时只检查 `@` 前的空白，不跳过 fenced code。`src/app/markdown/refer_shortcut.rs:225-247` 把这些区间直接作为候选；同文件后续的 `parse_references_in` 则会跳过围栏，两个来源规则不一致。普通文本区输入两个匹配字符时，围栏中的示例 `@abc` 会作为 `[file]` 候选出现并可插入，违反 design §10.3 的来源排除规则。

建议：候选扫描复用现有围栏判断，并测试“围栏内 `@abc` + 围栏外输入 `ac`”不出现候选。

### P2：不同类型引用的同分候选没有按正文首次出现排序

`src/app/markdown/refer_shortcut.rs:225-269` 先遍历全部 `@` 提及，再遍历全部 `@[refer-*]` 标记；`push_unique` 用加入顺序生成 `order`（同文件 `279-297`），排序又以它作为同分次序（`303-326`）。因此正文 `@[refer-file](/tmp/abc) @abc ac` 中，两项同为名称 `abc` 的前缀/子序列命中时，后出现的 `@abc` 总会排在先出现的引用前。当前测试只覆盖同一类型内的去重顺序。

建议：将两类 token 按原文字节位置合并排序后再去重、评分，补跨类型同分排序测试。

## 验证与范围

- `rtk cargo test paste_stage_inserts_placeholder_then_reference_rewrites`：1 passed。
- `rtk cargo test candidates_dedupe_and_keep_first_order`：1 passed。两项测试未覆盖上述反例。
- 未运行全量测试或真实 TUI 交互；本文仅记录静态数据流审查与两项聚焦测试的结论。
- `src/event/paste.rs:44-65` 在事件循环中同步执行路径元数据查询与文件头读取。普通本地文件通常很快，但慢速或挂载文件系统可能阻塞输入与绘制；建议后续按项目“阻塞文件 IO 放后台任务”的规范处理，并补过期结果路由测试。
