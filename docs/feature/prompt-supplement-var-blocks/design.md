# Prompt 补充块与变量块设计

- 状态：待用户审核；审核确认前不实施生产代码。
- 位置：Prompt 编辑器与 Settings 的自定义命令。
- 参考：`/Users/yonah/projects/agent/lx-agent` 的 lxmdeditor feature。以参考项目当前行为为基线；本次对命令名的明确要求优先。

## 1. 背景和目标

当前 Prompt 已支持 `&&&` 任务模板块、斜杠命令、User/Project 自定义命令、文本选择和单步撤销。解析器及复制逻辑已经局部识别 `+++` 子块，但没有独立插入、编辑、设置或渲染；也没有 `$$$ varTemplate` 变量块。若把两种新块直接当作 `&&&` 普通正文渲染，父块的状态底色和边框会污染子块。

本次交付以下完整能力：

1. 内置 `/supplement`，在 `&&&` 内插入 `+++ supplement` 子块；Settings 可自定义其他 `+++` 块的内容。
2. 内置 `/var`，在普通正文插入 `$$$ varTemplate`；var 块的编辑、清理、删除、合并、置顶与引用行为对齐 lx-agent。
3. `+++` 与 `$$$` 各有固定边框和底色，不受 `&&&` 的 todo、run、done 状态色控制。
4. 现有 `&&&` 的状态、复制、清理、删除、撤销和设置行为继续成立。

## 2. 用户已确认的边界

| 项目 | 决策 |
| --- | --- |
| var 文本协议 | 使用 lx-agent 的 `$$$ varTemplate --start` / `$$$ varTemplate --end`。 |
| var 命令名 | 插入命令是 `/var`，而非 `/varTemplate`。 |
| var 插入位置 | 普通正文可用，不在 `&&&` 或 `+++` 内插入。 |
| 变量作用域 | 当前 Prompt 文档全文可引用；不同 Prompt、不同工作区之间不共享变量。 |
| 变量引用 | 选择候选后插入变量的当前值，生成普通文本；后续改定义不会自动改已插入内容。 |
| 同名变量 | 文档中后定义覆盖先定义，候选只出现一次。 |
| var 交互 | 清理、删除、合并到首个 var 块、移到顶部、块内 `/singleLine` 和 `/multiLine` 均纳入。 |
| `+++` 插入位置 | 只在 `&&&` 任务块内部提供；块外已有 `+++` 文本按普通文本保留。 |
| 内置补充命令 | `/supplement` 插入 `+++ supplement --start` / `+++ supplement --end`。 |
| 自定义补充命令 | Settings 复用 `agentBlock`，新增 `block: supple`；User/Project 两级，Project 同名覆盖 User。 |
| 独立样式 | `+++`、`$$$` 分别使用固定边框和背景，不继承父 `&&&` 状态色。 |

“全局引用”在本文只表示**同一份 Prompt 文档内全局**，不增加跨文档持久变量。上述决策已在逐项 Grill Me 中确认；下文未单独讨论的低风险实现细节由现有项目模式决定。

## 3. 当前实现与参考实现

### 3.1 lx-tui 现状

- `src/app/markdown/template.rs` 是 `&&&` 解析和复制正文的纯逻辑；复制已经剔除 `+++` 补充内容，并保留 `%%%` 记录块正文。
- `src/app/markdown/slash.rs` 的斜杠候选按模板块内外过滤；内置只有 `&&&` 任务模板。`src/app/prompt/panel.rs` 执行候选确认与单步编辑。
- `src/settings/commands.rs` 的 `agentBlock` 固定存 `block: template`；User 路径为用户数据根 `command/agentBlock`，Project 路径为项目根 `.lx-t/command/agentBlock`。设置服务及页面需要同步识别类型。
- `src/ui/prompt.rs` 的 `template_line_infos` 按 `&&&` 状态铺满整行背景并绘制边框；新增子块必须以更具体的行角色覆盖父样式。`ui/style.rs` 放样式 Token，`ui/text.rs` 放全部用户可见文案。

### 3.2 lx-agent 参考行为

- `markdownSlashCommands/builtinCommands.ts` 把补充命令限定在 template 范围，var 插入限定在 normal 范围，单行/多行命令限定在 varTemplate 范围。
- `templateContents.ts` 的 var 块有 `@content:` 和 `key: "var"` 初始占位；补充块有标题、参考、需求和备注字段。
- `markdownBlockCommands/markers.ts` 分别识别 `+++` 和 `$$$` 起止行；旧 `+++ log/logTemplate` 属于记录块兼容，不属于新的 supplement。
- `markdownVariableCommands/variableParsing.ts` 解析多个变量块，支持单行值、引号、点号层级和多行值；`varBlockMoves.ts` 实现合并与置顶。
- `useMarkdownFileMentionPanel.ts` 确认变量候选时以 `variable.value` 替换当前查询文本。

## 4. 文本协议与解析

### 4.1 `+++` 补充块

`/supplement` 替换当前斜杠命令行，插入下列结构。下面代码围栏内的标记实际是三个加号：

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

插入后光标定位在 `{title: }` 的 `}` 前。保存的自定义 `block: supple` 使用同样的起止外壳，即 `+++ <name> --start`、`+++ <name> --end`，正文和可选标题来自 Settings。编辑已经插入的块不受设置日后改动影响。

合法块只能位于 `&&&` 内，开始与结束命令名必须匹配。可读取参考格式的同行 `「title: ...」` 与结束行可选 `{id:...}`、`{wt:...}`；新插入沿用 lx-tui 现有的独立 `{title: }` 行。未闭合子块最多延续到父 `&&&` 结束前，不能吞掉下一个任务块。代码围栏内、var 块内、块外的 `+++` 不取得子块身份。

### 4.2 `$$$` 变量块

`/var` 在普通正文插入：

```text
$$$ varTemplate --start
{title: }
@content:
key: "var"
$$$ varTemplate --end
```

插入后选中初始 `key`，便于立即改名。若当前 TUI 选区状态无法稳定承载初始选中，允许光标落在 `key` 末尾，但实现时须记录差异并覆盖测试。读取兼容 lx-agent 的同行 `「title: ...」`、裸 `$$$`、`$$$ --end` 与结束行可选 `{id:...}`；新插入只使用规范形式。

只有位于普通正文且已闭合的 var 块参与变量索引。解析按文档顺序进行，后定义的同名键覆盖先定义，候选只显示一个名称。首部旧式 `---` frontmatter 仅在没有有效 var 块时作为兼容来源，不与新块混合。`@content:` 是保留内容块键，不加入变量候选，也不被清理。空值与默认 `"var"` 占位不进入候选。非法条目跳过，不使同块其他合法条目失效。

变量解析以参考项目支持的常用格式为验收范围：`key: "value"`、普通标量、`vars:` 层级/点号键、三引号多行以及 `|`、`>` 多行。解析结果只属于当前 Prompt 正文；文本编辑、撤销、删除或切换 Prompt 后重新计算，不新增数据库表或跨文档缓存。

### 4.3 位置优先级

上下文扫描先识别代码围栏和顶层 `&&&`、`$$$`，再在 `&&&` 内识别 `+++`。需要明确区分普通正文、任务块正文、补充块正文、变量块正文及结构标记行；只使用 `inside_block: bool` 无法表达这些范围。结束标记必须与当前开启的块匹配，错名结束行不能关闭块。历史 `%%%` 记录块及 `+++ log/logTemplate` 兼容读取，但不被当作 supplement 插入入口。

## 5. 命令和引用交互

- `/supplement` 只在 `&&&` 的可编辑正文显示；Settings 自定义 `block: supple` 也遵守相同范围。`/var` 只在普通正文显示。`/singleLine`、`/multiLine` 只在 var 块的键值正文显示。
- `/singleLine` 插入 `key: "value"`，`/multiLine` 插入 `key:` 加三引号正文；选中或定位初始键名。var 块普通键行输入全角冒号 `：` 时转为 `:`，三引号值正文不转换。Tab 依参考项目的键/值行跳转；其他上下文沿用现有输入行为。
- 变量候选复用 Prompt 的现有字母快捷/候选面板，与已有文件及引用候选同屏展示并标明来源。普通正文及 `&&&` 正文可引用；var 定义、`+++` 子块、代码围栏和结构标记行不触发引用候选。
- 选中变量候选后，将触发范围替换为变量值，并把光标放在插入文本末尾；多行值保留换行。插入动作单步撤销，不保留动态绑定，也不对值中的 `/`、`@` 或模板标记做二次命令执行。
- 复用现有面板互斥顺序：粘贴选择、文件提及、字母候选、斜杠命令、Markdown 块命令。Esc 只关闭当前面板，不擦除输入行；鼠标与 Enter 确认走相同的状态转换。

## 6. 块级操作

### 6.1 Supplement

起始行提供清理与删除按钮，不展示 `&&&` 的状态按钮。清理只移除补充正文中未填写的空列表项，保留标题、已填项和结构行。删除包含两端标记并至多吸收一侧多余空行。父 `&&&` 的复制结果继续剔除整个补充子块。每次插入、清理和删除都形成一个撤销记录。

### 6.2 Var

起始行提供清理、删除、合并到首块、置顶的按钮或等价 TUI 命令入口。清理移除空引号、`"var"` 默认占位、空三引号和没有有效值的项；保留 `@content:` 与已填写内容。删除后立即从候选索引移除其定义。

合并将当前块内部键值内容追加至文档首个 var 块内部，再删除当前块外壳；当前已经是首块时不动作。置顶保持完整块，将其移动到文首连续 var 块组之后；已经在该组内时不动作。多余空行只做局部归一，避免改写不相关正文。上述文本操作均单步可撤销，并保持光标在搬移后的内容附近。

折叠/展开对齐参考项目，但仅改变 Prompt 的视图状态，不改正文。折叠状态不得因编辑、Prompt 切换或尺寸变化而指向错误的行；若现有状态模型没有折叠基础设施，采用最小可维护的块身份/锚点，不在渲染时修改状态。

## 7. 设置与存储

`agentBlock` 增加稳定的块类型 `template|supple`，用 enum 或等价类型在解析、序列化、候选插入和 JSON 之间传递。旧文件缺少 `block` 或写作 `block: template` 时继续是 `&&&`；`block: supple` 生成 `+++`；未知值返回明确解析错误并在设置页展示，不静默退化。

User 保持用户数据根 `command/agentBlock`，Project 保持项目根 `.lx-t/command/agentBlock`，不新建类型专属目录。同名 Project 覆盖 User，已有覆盖确认和保存冲突语义保持。Settings 表单增加块类型选择，可编辑内容、描述和可选标题；保存的 frontmatter 用 `block: supple`。HTTP 列表 JSON 与 PUT 同步加入 `block_type`；旧客户端不传时默认 `template`。设置服务保存成功后按现有事件路径通知 TUI 重载命令，保存失败保留编辑草稿。

`supplement`、`var`、`singleLine`、`multiLine` 纳入保留名，防止自定义命令覆盖内置命令。旧 `+++ suppleTemplate` 文本可读取，但不作为新的内置斜杠候选。当前项目的 `.lx-t` 路径与参考项目 `.lx` 路径不相同，本次只对齐 User/Project 作用域和覆盖行为，不迁移已有目录。

## 8. 样式与渲染规则

`&&&` 保留 todo、in_progress、done 三态边框与背景。`+++` 使用一套固定的边框和暗色背景；`$$$` 使用另一套固定的边框和暗色背景。颜色定义集中在 `src/ui/style.rs`，不得直接硬编码在渲染分支。子块每个视觉行先取得父 `&&&` 角色，再以 `+++` 角色覆盖背景和边框；选区反显格跳过块背景，仍保持可辨识的选区效果。

`+++` 与 `$$$` 起始/结束标记、命令名、标题和变量键值分别走 Markdown token 语义。软换行续行绘制中段边框，起止角只在实际起止物理行出现。按钮占固定宽度，文本超宽时截断；窄窗口中按钮、右边框与文本不互相覆盖。渲染仅接收 `&Prompt`/`&AppState`，不执行 IO 或状态更新。所有新文案进入 `src/ui/text.rs`。

## 9. 模块职责与数据流

1. `src/app/markdown/`：纯解析、块范围、上下文、变量条目和文本变换。对现有 `&&&` 解析复用公共扫描数据，避免多套互相矛盾的正则和状态布尔量。
2. `src/app/prompt/`：候选面板、编辑快照、块操作、折叠和当前文档变量索引。所有修改经现有编辑路径一次提交。
3. `src/settings/commands.rs` 与 `src/settings/service/`：块类型存储、HTTP 契约与原子写入；阻塞 IO 留在服务侧。`src/event/` 接收重载消息及鼠标命中结果。
4. `src/input/` 与 `src/app/update.rs`：键盘命令到 Action、Action 到纯状态转换；不把输入判定放进 `ui/`。
5. `src/ui/markdown/`、`src/ui/prompt/`、`src/ui/style.rs`、`src/ui/text.rs`：分词、只读渲染、样式、文案。复用现有命令面板、选区、toast 和剪贴板设施。

不新增依赖、数据库表、线程或进程。新增文件只在现有职责达到行数或复杂度边界时创建；单文件不得超过 1000 行，拆分后目标约 500 行。

## 10. 测试与风险控制

- 协议：合法/未闭合/错名结束、相邻/交叉块、围栏、块外 `+++`、旧 `+++ log` 和 `%%%` 不误判。
- 变量：同名后者覆盖、空值过滤、多个 var 块、三引号/多行、删除/撤销刷新、跨 Prompt 隔离、值直接插入。
- 设置：旧 `agentBlock` round trip、`block: supple` 保存读取、非法值、同名覆盖、HTTP CRUD、保存后 TUI 重载。
- 渲染：父状态切换后子块固定色、`$$$` 独立色、选区背景、按钮命中、宽字符、软换行和窄屏。
- 回归：现有 `&&&` 状态与复制语义、`%%%` 记录块、斜杠及提及面板优先级不回退。

按项目规范在改代码后执行受影响模块的精确测试、`cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与 `cargo test`。Agent 以缓冲区测试核实颜色和几何；实际 TUI 交互与外观由用户运行 `cargo run` 人工确认。若实现必须改变本文已确认边界，应先更新文档并交用户审核。
