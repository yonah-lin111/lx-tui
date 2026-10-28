# Prompt 补充块设计

- 状态：已实施。
- 位置：Prompt 编辑器与 Settings 的自定义命令。
- 参考：`/Users/yonah/projects/agent/lx-agent` 的 lxmdeditor feature（`markdownBlockCommands`）。

## 1. 背景和目标

Prompt 已支持 `&&&` 任务模板块、斜杠命令、User/Project 自定义命令、文本选择与单步撤销。本次为 `&&&` 引入可插入、可编辑、可设置、可渲染的 `+++` 补充子块，使其拥有独立于父 `&&&` 状态色（todo/in_progress/done）的固定边框与底色。

> 变更记录：原设计同时包含 `$$$` 变量块；按用户决定，变量块相关功能（`/var`、`/singleLine`、`/multiLine`、`/addVarContent`、变量引用候选、解析/块操作/样式/保留名）已全部移除。变量块折叠按钮也已移除。本文只描述最终保留的 `+++` 补充块。

## 2. 文本协议

`/supplement` 替换当前斜杠命令行，插入下列结构（围栏内标记实际是三个加号）：

```text
+++ supplement --start
{title: }

## Supplementary Requirements

- Reference:
- Requirements:
  -
- Notes:
  -

+++ supplement --end
```

- 起始行 `+++ <command> --start [{title: …}|「title: …」]`，结束行 `+++ <command> --end [{id: …}] [{wt: …}]`；两端命令名必须匹配。
- 标题行固定为起始行下一行的 `{title: }`（兼容同行 `「title: …」`）；标题行与正文之间固定一空行，该空行渲染为块分割线 `├─…─┤`（与 `&&&` 一致）；正文与结束行之间同样固定一空行（块底部留白，按普通空行渲染）。
- 合法 `+++` 只能位于 `&&&` 内；未闭合子块最多延续到父 `&&&` 结束前，不能吞掉下一个任务块。
- 代码围栏内、块外的 `+++` 不取得子块身份；旧 `+++ log/logTemplate` 保留记录块兼容语义。

插入后光标定位在 `{title: }` 的 `}` 前。自定义 `block: supple` 使用同样的外壳，正文与可选标题来自 Settings。

## 3. 命令与范围

- `/supplement` 只在 `&&&` 的可编辑正文显示；Settings 自定义 `block: supple` 遵守相同范围。
- `/gitWorkTree` 在 `&&&`（含 `+++`）且有 git 元数据时出现；选中后把分支写到所在块结束行上一行的 `{wt:…}` 绑定行（`+++` 子块绑定自身结束行）。
- 斜杠候选按块上下文过滤：普通正文只列内置模板；`&&&` 正文列 `/supplement`（+ `/gitWorkTree`）；`+++` 正文列 `/gitWorkTree`。globally 作用域自定义 Markdown 任意位置出现，template 作用域仅在 `&&&`/`+++` 内。
- 复用现有面板互斥顺序（粘贴 > 文件提及 > 字母候选 > 斜杠 > 块命令）；Esc 只关闭当前面板。

## 4. 块操作

- 起始行提供复制、清理与删除按钮（`[❐]`/`[⌫]`/`[✕]`），不展示 `&&&` 的状态按钮。
- 清理只移除补充正文中未填写的空列表项，保留标题、已填项与结构行。
- 删除包含两端标记并至多吸收一侧多余空行。
- 父 `&&&` 的复制结果继续剔除整个补充子块。
- 复制（`[❐]`）与 `Ctrl/Cmd+Shift+C` 提取补充正文（剔除两端标记与标题占位行）；光标位于补充子块内时优先复制该子块，否则回退复制所在 `&&&` 模板块。
- 每次清理、删除都形成一条撤销记录。

## 5. 设置与存储

- `agentBlock` 记录增加稳定块类型 `block: template|supple`；旧文件缺少该键或写作 `block: template` 时仍是 `&&&`，`block: supple` 生成 `+++`，未知值返回明确解析错误并在设置页展示。
- User 保持用户数据根 `command/agentBlock`，Project 保持项目根 `.lx-t/command/agentBlock`，不新增类型专属目录；同名 Project 覆盖 User。
- Settings 表单增加块类型选择，可编辑内容、描述与可选标题；HTTP 列表 JSON 与 PUT 同步加入 `block_type`（旧客户端缺省 `template`）；保存成功后沿用现有事件路径通知 TUI 重载命令，失败保留草稿。
- 保留名包含 `supple`、`supplement`（防止自定义命令覆盖内置命令）。

## 6. 样式与渲染

- `&&&` 保留 todo、in_progress、done 三态边框与背景；`+++` 使用固定边框（LightMagenta）与暗紫色底，不继承父 `&&&` 状态色。颜色集中在 `src/ui/style.rs`。
- `+++` 起始/结束标记、命令名、标题与正文分别走 Markdown token 语义；子块每个视觉行先取得父 `&&&` 角色，再以 `+++` 角色覆盖背景与边框；选区反显格跳过块背景。
- 标题行下的空行渲染为块分割线 `├─…─┤`，与块上下边同色；软换行续行绘制中段边框，起止角只在实际起止物理行出现。
- 起始行顶边右端常显该块操作按钮（复制、清理、删除）；按钮占固定宽度，文本超宽以省略号截断，窄窗口中按钮、右边框与文本不互相覆盖。
- **字母快捷引用**：嵌套在 `&&&` 模板块内部的 `+++` 补充块共享当前父级模板块的作用域（补充块内部可快捷引用父级模板块中出现的文件与文件夹，补充块内新声明的引用也对父级模板块可见；但补充块起止行与标题行仍作为结构保护行不触发快捷输入）。
- 渲染仅接收 `&Prompt`/`&AppState`，不执行 IO 或状态更新；所有新文案进入 `src/ui/text.rs`。

## 7. 模块职责与数据流

1. `src/app/markdown/`：纯解析、块范围与上下文（`blocks.rs`、`supple.rs`、`template.rs`、`worktree.rs`、`slash.rs`、`refer_shortcut.rs`）。
2. `src/app/prompt/`：候选面板、编辑快照与块操作（`block.rs` 的补充块清理/删除）。所有修改经现有编辑路径一次提交。
3. `src/settings/commands.rs` 与 `src/settings/service/`：块类型存储、HTTP 契约与原子写入；阻塞 IO 留在服务侧。`src/event/` 接收重载消息及鼠标命中结果。
4. `src/input/` 与 `src/app/update.rs`：键盘命令到 Action、Action 到纯状态转换；不把输入判定放进 `ui/`。
5. `src/ui/markdown.rs`、`src/ui/prompt.rs`、`src/ui/style.rs`、`src/ui/text.rs`：分词、只读渲染、样式、文案。

不新增依赖、数据库表、线程或进程。单文件不超过 1000 行。

## 8. 测试与风险控制

- 协议：合法/未闭合/错名结束、相邻/交叉块、围栏、块外 `+++`、旧 `+++ log` 与 `%%%` 不误判。
- 设置：旧 `agentBlock` round trip、`block: supple` 保存读取、非法值、同名覆盖、HTTP CRUD、保存后 TUI 重载。
- 渲染：父状态切换后子块固定色、标题行分割线、选区背景、按钮命中、宽字符、软换行和窄屏。
- 回归：现有 `&&&` 状态与复制语义、`%%%` 记录块、斜杠及提及面板优先级不回退。

按项目规范在改代码后执行受影响模块的精确测试、`cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与 `cargo test`。Agent 以缓冲区测试核实颜色与几何；实际 TUI 交互与外观由用户 `cargo run` 人工确认。
