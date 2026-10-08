# Prompt 模板斜杠命令面板、Markdown 高亮对齐与模板块操作 — 执行任务

- 分支：`feature/prompt-template-commands`
- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案收敛至 `ui/text.rs`；样式只从 `ui/style.rs` 取；变更前后运行测试。

## 执行清单

### 1. 领域层：斜杠命令与模板（`src/app/markdown.rs`）
- [x] 定义 `SlashCommandId`（`Add`, `Bug`, `Common`, `Refactor`, `Style`）与别名映射
- [x] 实现 `slash_trigger`（行首 `/` 识别、代码块与模板块内部抑制）
- [x] 实现 `SlashPanel` 状态结构体与候选过滤（支持短名与长名模糊匹配）
- [x] 定义标准模板文本生成函数（对齐 `lx-agent` 协议与标题占位符光标偏移）
- [x] 实现模板块解析、字段清理（`clean_template_content`）与状态轮转逻辑
- [x] 单元测试：`src/app/markdown/tests.rs` 覆盖触发识别、过滤打分、模板生成与清理

### 2. 编辑器状态层：面板生命周期与模板操作（`src/app/prompt.rs`）
- [x] `Prompt` 状态机集成 `SlashPanel`（`slash_move`, `slash_confirm`, `slash_escape`, `slash_set_active`, `slash_scroll`）
- [x] `settle()` 收敛优先级（`mention` > `slash_panel` > `block_panel`）
- [x] 实现模板块操作动作：复制、清理空字段、删除整块、切换状态，均接入撤销栈
- [x] 单元测试：`src/app/prompt/tests.rs` 覆盖斜杠面板导航、插入回退、模板块就地删除与清理

### 3. 路由与事件处理（`src/app/update.rs` 与 `src/event/mod.rs`）
- [x] `src/app/update.rs` 键盘路由：`route_slash_panel` 接管上下选择、回车插入与 Esc
- [x] `src/app/update.rs` 鼠标路由：斜杠面板条目点选、悬停与滚动
- [x] `src/event/mod.rs` 鼠标事件：命中模板首行功能按钮（`[copy]` 触发剪贴板与 Toast，`[clean]` / `[del]` / 状态切换更新状态）
- [x] 单元测试：`src/app/update/tests.rs` 与 `src/event/tests.rs`

### 4. 词法高亮层（`src/ui/markdown.rs` 与 `src/ui/style.rs`）
- [x] `ui/markdown.rs`：`TokenKind` 扩充 `TemplateMarker`, `TemplateCommand`, `TemplateTitle`, `FileMention`
- [x] `ui/markdown.rs`：单行与跨行扫描支持合法 `&&&` 起止行解析
- [x] `ui/markdown.rs`：行内词法扫描支持 `@文件`（`@path/to/file`）
- [x] `ui/style.rs`：语义样式映射（起止标记 `dim`，命令名按业务加粗分色，标题 `Cyan` 下划线、文件 `Yellow` 下划线）
- [x] 单元测试：`src/ui/markdown/tests.rs` 覆盖各种起止行、不完整标记、文件提及分词

### 5. 渲染与文案层（`src/ui/prompt.rs` 与 `src/ui/text.rs`）
- [x] `src/ui/text.rs`：定义斜杠命令展示文案、描述、按钮文本（`[copy]`, `[clean]`, `[del]`）
- [x] `src/ui/prompt.rs`：接入 `command_panel::render` 渲染斜杠命令浮层（标题 `Templates`）
- [x] `src/ui/prompt.rs`：在模板起始行右缘渲染操作按钮组，实现窄屏避让逻辑
- [x] 单元测试：`src/ui/prompt/tests.rs` 与 `src/ui/text/tests.rs`

### 6. 集成测试与全量校验
- [x] 集成测试：`tests/prompt_template_commands.rs`（全链路：输入 `/add` → 弹面板 → 回车插入 → 高亮呈现 → 点击按钮操作）
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
