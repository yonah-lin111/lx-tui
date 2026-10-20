# 后台任务卡片橙色态与输出迁移到任务面板 — 执行任务

- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案集中在 `ui/text.rs` / `ui/text/agent.rs`。
- 先补复现测试，再改实现。

## 执行清单

### 0. 需求盘问与规格确认（已完成）
- [x] 盘问 1：橙色仅覆盖进程类卡片，判据为进程存活
- [x] 盘问 2：保留闪烁，动画停止时恒实心
- [x] 盘问 3：兜底放渲染层字形函数，状态层不归零
- [x] 盘问 4：面板接管历史区矩形
- [x] 盘问 5：卡片不再输出任务内容
- [x] 盘问 6：`[open]` 独占末行、灰色
- [x] 盘问 7：只有 `[open]` 可点
- [x] 盘问 8：面板 = 任务列表 + 高亮任务输出区
- [x] 盘问 9：`[open]` 携带 session_id 定位高亮，失效回落首项
- [x] 盘问 10：面板期间冻结历史滚动
- [x] 盘问 11：键位沿用，PageUp/PageDown 改翻输出区

### 1. 复现测试（均已完成）
- [x] 卡片存活期为橙色 / 停止后为绿色
- [x] 动画停止后方块恒实心（不再出现定格镂空）
- [x] 进程卡片恒 3 行、无输出行、`[open]` 灰色
- [x] 只有 `[open]` 行可点，标题行不可点
- [x] 面板占历史区、输入框原位不动
- [x] `[open]` 定位到指定任务；进程已结束回落首项
- [x] 面板期间历史滚动冻结，关闭后恢复

### 2. 样式层（ui/style.rs）
- [x] 新增橙色语义 token（mark / label），与既有色域可辨
- [x] 更新色域不重叠测试

### 3. 卡片渲染（ui/main_content/lx/history）
- [x] `StepMark` 增加 `Alive`，`glyph` 增加 animating 入参
- [x] `command_body` 收敛为命令 + `[open]`
- [x] 命中只登记 `[open]` 行并携带 session_id

### 4. 任务面板（ui/main_content/lx/task.rs、lx.rs、app/state/lx/task.rs）
- [x] `TaskPage` 增加 focus / output / output_scroll
- [x] 面板接管历史区矩形，输入框原位
- [x] 列表 + 输出区版式，高亮切换贴尾部
- [x] PageUp/PageDown 翻输出区

### 5. 事件层
- [x] `open_task_page` 接受 session_id
- [x] 进程快照写入高亮任务输出
- [x] 面板期间冻结历史滚动

### 6. 验证
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`（全量）
- [ ] 用户 `cargo run` 人工确认观感
