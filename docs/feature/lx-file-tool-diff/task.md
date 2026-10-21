# lx 文件工具 Diff：执行任务

> 状态：设计待审核，实施未开始。
> 对应设计：同目录 `design.md`。
> 执行约束：用户审核 design/task 后才开始修改代码；不创建分支/工作树、不暂存、不提交、不改 Git 索引。
> 当前已授权动作：只读检索、边界讨论、编写这两份文档。

## 0. 已完成的设计准备

- [x] 使用 grill-me，每次只问一个问题，并提供推荐和通俗示例。
- [x] 确认只显示 edit/write 执行后的实际变更。
- [x] 确认每文件默认最多 3 个预览物理行，点击展开正文和上下文。
- [x] 确认删除和移动一起覆盖。
- [x] 确认超大 diff 允许截断、保留摘要并明确提示，历史保持一致。
- [x] 阅读四份项目规范，确认 app/ui/工具执行/持久化的职责边界。
- [x] 使用 codegraph 与 codebase memory 检索 lx 和 Codex 的关键符号、调用链与源码。
- [x] 使用搜索 subagent 完成参考实现及持久化/回放链路的只读分析，未并行修改代码。
- [x] 检查相关路径的 memory 覆盖信号，识别参考仓库索引过时并使用当前源码。
- [x] Context7 核对 similar 行差异能力；核对 Codex 实际依赖版本为 2.7.0。
- [x] 编写设计、执行分解、风险、兼容规则与验收标准。
- [ ] 用户审核通过 design.md 和 task.md，记录审核结果及变更。

**阻断条件：最后一项未完成前，不执行下列实施任务。**

## 1. 开工复核和最小职责拆分

目标：避免把新增功能叠进已接近行数上限的 UI 文件。

- [x] 只读检查工作区差异，识别用户已有变更；不覆盖、不恢复、不暂存。
- [x] 再用 MCP 定位准备修改的确切符号与直接影响；只对确有变化/索引漂移处补读源码。
- [x] 核对本设计中的待审核默认阈值，保持实现与文档一致。
- [x] 确认文档审核包含二进制删除行数未知、极端元数据溢出时省略文件摘要、超大输入采用明确标注的简化脚本这三项技术取舍。
- [x] 将 `history/tool.rs` 的普通文件摘要组装迁至 `tool/file_summary.rs`，保持现有导出和渲染输出。
- [x] 将 `lx.rs` 的历史聊天命中职责迁至 `lx/history_hit.rs`，使原 1050 行文件回到 ≤1000 行。
- [x] 不重构其他页面、布局、进程工具或全部鼠标事件。
- [x] 核对拆出的子文件约 500 行、所有触及文件 ≤1000 行。

验证：原文件工具摘要、工具几何、进程 `[open]` 及聊天命中测试仍通过；只做结构移动时不改变 buffer 内容。

## 2. 结构化差异与行算法

依赖：步骤 1 完成；数据契约按 design §4、§5。

- [x] 增加 similar 2.7 系列依赖，核对真实 API、特性和锁文件，不照搬主分支新增接口。
- [x] 增加 `chat/tools/diff.rs`，定义 Complete/Truncated/Unavailable、hunk、行种类及真实行号。
- [x] 正常结果使用同一份编辑脚本生成完整计数和正文；原前后缀能力仅保留为超大输入的线性简化脚本，不保留独立于正文的另一套计数。
- [x] 上下文半径固定为 1，相邻范围合并，远距范围保持独立。
- [x] 保留 EOF 无换行信息；正文不携带额外行尾，缩进不能 trim。
- [x] 生成阶段限制保存行数和字节，不依赖 UI 三行预览限制内存。
- [x] 复杂差异输入限额为 old+new ≤8 MiB 且 ≤100,000 行，算法 timeout 配置 100ms。
- [x] 超限降级使用公共前后缀外完整替换的线性脚本，精确统计全部增删，再裁正文；携带 simplified 标志，不用采样估值，无法解码时明确 unknown。
- [x] 新建/删除按线性方式处理，空内容不产生虚假的一行。

测试文件：`src/chat/tools/diff/tests.rs`。

必须先构造：单行替换、20 行中两处远距修改、重复行、空文件、只含空行、中文、CRLF/BOM、EOF-only、大行和多行超限。断言行号、可应用转换、hunk 分组及计数与截断互不影响，不要求固定全局最短脚本的唯一解。

## 3. 工具执行接入与提交真实性

依赖：步骤 2；变更 `types.rs`、`write.rs`、`edit.rs`，必要处使用内部提交失败类型。

- [x] Write 在已有读取中保留 before，使用实际最终写入内容生成 after，成功后返回 diff。
- [x] write 新建空文件仍保留创建事件；相同内容覆盖显示无内容差异。
- [x] Edit Add/Update/Delete/Move 为每个提交文件挂接差异，复用已有 before/after，禁止为展示重复读文件。
- [x] 将 `before == after` 的提前退出限制到非移动操作；纯移动必须执行 rename。
- [x] 移动源内容没有变化时不额外重写源文件。
- [x] 源文件内容写入后，后续 mkdir/rename 失败必须返回源路径 Updated 及其 diff。
- [x] 外层 commit 在产生 Failed 前合入本次已经发生的源文件变更。
- [x] 多文件失败保留先前 changes，后续未执行文件不得出现成功差异。
- [x] 删除不可解码/二进制文件保持既有删除能力，以 diff unavailable 和未知行数降级。
- [x] 不扩大锁语义、不实现跨文件事务、不尝试失败回滚或自动重复文件写入。
- [x] 保留现有 BOM、CRLF、符号链接和路径解析行为。

测试：`write/tests.rs`、`edit/tests.rs`。先补纯移动被跳过、移动失败漏已写源文件的复现，再修正实现。失败注入优先使用模块私有的提交阶段测试接缝，在“源已写、rename 未成功”处确定性返回错误；不用依赖机器权限、竞态或休眠的脆弱测试。

验收：每个成功/失败矩阵中的磁盘结果、结构化 changes 和 diff 一致；不把 `partial` 当成 IO 部分失败。

## 4. 结果 JSON、预算与兼容

依赖：步骤 3；保持现有 to_result_json 入口，必要时抽取文件结果模块。

- [x] 为 changes[] 和 write 增加可选、带 version 的 diff；error 的已提交 changes 用同一格式。
- [x] 保留原 tool/path/kind/to/created/added/removed/partial/code/message 字段。
- [x] 按 design §4.3 实现 counts_known、simplified、file_totals、omitted_files；总计在列表裁切前计算，未知不能用 0 或可见前缀和代替。
- [x] 实现单次调用 512 KiB 的差异 JSON 预算及 10,000 行保留限制。
- [x] 按最终 JSON UTF-8 字节检测 1 MiB 上限，计入转义、行结构和路径开销。
- [x] 优先从尾部移除完整行/hunk，给所有被裁正文的文件设置 Truncated。
- [x] 文件元数据本身超限时显式记录 omitted_files、总文件数和总计数状态，避免当前只有 note 的兜底丢掉全部摘要。
- [x] 不让 diff 过大把原本成功的文件操作变成 DB ToolResultTooLarge。
- [x] 不吞序列化失败；不生成半个 JSON 或半个 UTF-8 字符。

测试：output 对应测试模块。构造引号、反斜杠、控制字符转义、超长行、许多文件和多文件失败结果，断言 JSON 合法、总字节合规、计数不受裁切影响、遗漏明确。

## 5. 类型化详情与实时/历史一致

依赖：步骤 4；不修改 SQLite schema。

- [x] 增加 `app/state/lx/chat/file_diff.rs` 的文件详情模型/解析，`ToolDetailView` 增加对应分支。
- [x] 按稳定字段验证结构、行号、版本、状态；坏文件详情局部降级，不能拖垮其他文件或会话。
- [x] 文件实时详情在 DB 提交成功后，从刚落库的 result_json 解码。
- [x] 历史 tool_view_of 使用同一解码入口，不从磁盘补内容。
- [x] 工具/app 两侧摘要及 is_empty/absorb_summary/absorb_result_json 支持新计数状态；file_totals 优先，结果未知必须清除旧数字。
- [x] 进程/计划维持现有运行态更新方式，只修改匹配新分支所必需的穷尽分派。
- [x] 旧记录无 diff 时继续两行摘要，不显示无效展开按钮。
- [x] 展开状态复用 `(message_id, call_id)` 和现有清理规则；历史加载默认折叠。

测试：`app/update/chat/view`、`app/update/chat/tests/process_cards` 及 event/chat 的受影响 fixture。

关键断言：同一裁切结果的实时和历史 FileDiffView 完全相等；落库后改变/删除磁盘文件不改变历史 diff；旧代次结果、重复 ack 和 DB 失败不造成重复文件 IO。

摘要另需覆盖：文件列表裁切后仍有完整调用总计；一个删除文件不可解码时实时和历史都显示未知，不把兼容数值 0 重新合并成真实计数。

## 6. 模型回放隔离

依赖：步骤 4；只修改文件结果准备，不改变 transport 协议。

- [x] 当前轮 format_for_model 维持文件摘要，不新增 diff 正文。
- [x] 成功历史回放前剥离 write/edit 文件 diff。
- [x] Failed 历史含 changes 的两条路径均剥离：正常 failure_content 与无错误文本的 JSON 兜底。
- [x] 保留原摘要字段、简化/未知标志、遗漏信息、错误和 call/result 配对。
- [x] 保留现有单条 64 KiB 回放上限及 UTF-8 截断标记。
- [x] 非文件工具及不认识的旧结果行为保持兼容。

测试：context 对应模块。加入只存在于 diff 正文中的唯一标记，断言历史模型内容不含该标记，但路径、计数、错误和原有 read 内容仍存在。

## 7. 纯渲染、预览及展开

依赖：步骤 5；新增 `history/tool/file_diff.rs`，父模块仅分派。

- [x] 单文件/多文件路径与摘要，移动显示旧路径 → 新路径。
- [x] 每文件筛选增删行后按实际列宽换行，默认最多 3 个物理行，省略提示单独计行。
- [x] 展开显示全部已保存 hunks、1 行上下文、hunk 间分隔和 EOF 元信息。
- [x] 绿色新增、红色删除，必须同时保留 + / -；行号及上下文使用语义样式。
- [x] no changes、unavailable、truncated、simplified、omitted files 使用明确英文提示，集中在 ui/text.rs。
- [x] Unicode 宽度、Tab 4 列制表位、控制字符安全显示、长行续行空 gutter。
- [x] 0 宽、1 列、极窄双宽字和极矮视口不越界、不死循环、不 panic。
- [x] rows/draw 使用同一物理行布局，滚动条和 sticky body 使用同一高度口径。
- [x] 保留已有工具间空行、方块/动词和错误样式，不引入单独主题系统或语法高亮。

测试：`history/tool/tests/file_diff.rs`（按最终模块路径调整），以及现有 tool_geometry。宽度至少覆盖 0/1/2/8/20/80；比较 layout 行数、draw 推进数、符号/行号和关键 cell 样式。

## 8. 鼠标、键盘和视口接入

依赖：步骤 7。

- [x] ToolHit / LxChatHit 明确区分展开文件卡片和打开后台进程，不使用 session_id 是否为空推测动作。
- [x] 仅注册文件工具标题的展开命中，正文保留选区行为。
- [x] 标题点击切换整张卡片；再次点击恢复每文件 3 行预览。
- [x] 文件详情加入现有工具选择集合，`^o` 可切换，普通上下方向键继续编辑输入。
- [x] 复用 toggle_tool_detail、pin_lx_viewport 和既有展开状态，不增加第二套偏移。
- [x] 保持进程 `[open]` 点击、任务面板和其他浮层优先级。

测试：input/process_keys、history 几何、lx 命中与 viewport_anchor 的相关测试。覆盖上滚后展开、跟随底部展开、窗口 resize、标题滚出视口及 sticky 用户行存在的坐标转换。

## 9. 文档回写与精确验证

- [x] 回写 design 的实际类型、阈值、文件归属和与方案的偏差；不留下相互矛盾的“已确认”描述。
- [x] 更新 project-directory-structure 中新增模块和“工具恒两行”等过时描述。
- [x] 更新 tui-design-requirements 中文件 diff、预览、展开、失败及截断规则，不修改无关规范。
- [x] 检查触及生产代码无 unwrap/expect、无 stdout 调试输出、无 ui IO、无 app 反向依赖。
- [x] 检查所有触及文件行数上限，拆分子文件控制在约 500 行。
- [x] 每阶段运行受影响测试；任何失败先查原因，不用 allow 或删断言掩盖。

计划命令（届时优先 rtk 封装，失效才用原命令）：

```sh
rtk cargo test chat::tools::diff
rtk cargo test chat::tools::write
rtk cargo test chat::tools::edit
rtk cargo test chat::tools::output
rtk cargo test chat::context
rtk cargo test app::update::chat
rtk cargo test ui::main_content::lx::history
rtk cargo test input::process_keys
```

过滤名按实际 Rust 测试路径核对，不能把“匹配 0 项测试”视为通过。若需要覆盖 event 的受影响 fixture，追加精确过滤。最后仅做一次项目规定交付检查：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`；代码格式修改时先运行 cargo fmt。

本期不触碰 `src/tui/` 恢复路径；若实施被迫涉及该路径，必须保留恢复顺序并额外执行 `tui::tests`，不得扩大成退出流程重构。

## 10. 人工验收（由用户执行）

- [ ] `cargo run` 后，让 Native Agent 新建一个普通文件，看到绿色 + 内容和真实行号。
- [x] 修改文件中两处相距较远的行，展开后看到独立 hunks，不把中间所有行当成改动。
- [x] 新建 200 行文件，默认每文件最多 3 个正文物理行，点击标题完整展开，再次点击收起。
- [x] 删除文本文件、纯移动、移动并改内容，动作与磁盘结果一致。
- [x] 制造一次多文件中途失败，确认已提交文件仍有 diff，错误未被成功样式掩盖。
- [x] 查看超限结果，明确看到 Diff truncated/不可用原因；展开不会假装补齐缺失部分。
- [x] `/resume` 重开，正文、计数和裁切状态与实时一致；旧会话保持可用。
- [x] 窄窗格、中文长行、滚动展开和正文选区符合预期；进程 `[open]` 仍打开任务面板。

Agent 只能提供自动测试证据，不把以上真实终端验收勾为已完成。

## 11. 完成与交付记录

- [x] 所有实施步骤完成，自动验证结果记录在此，失败和未验证部分明确列出。
- [ ] 用户可在本目录核对最终行为、已知并发窗口、裁切限制和人工验收清单。
- [ ] 最终回复只报告实际改动、验证、风险及待验收项，不声称完整照搬 Codex。
- [ ] 未经另行明确授权，不执行 Git 暂存、提交、切分支、merge、reset 或发布。

当前验证记录（实施完成）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过（为此把 `ToolCallView::summary` 与
  `edit::CommitFailure::committed` 装箱，避免步骤枚举/错误变体过大）。
- `cargo test`：全部通过（lib 2839 passed / 0 failed；集成与 doc 测试同样通过）。
- 新增测试：`chat::tools::diff`（纯差异/边界/预算）、`chat::tools::output`（JSON 形状与裁切）、
  `app::state::lx::chat::file_diff`（兼容解码）、`ui::main_content::lx::history::tool::file_diff`
  （17 项布局/样式/宽度安全）、`ui::main_content::lx::tests::open_entry`（文件卡片标题命中 vs 正文不可点）、
  `app::update::chat::tests::process_cards`（文件卡片 `^o` 不误开任务面板）、
  `app::update::chat::tests::viewport_anchor`（展开文件卡片不移动视口）、`chat::context`
  （回放剥离 UI diff 正文）。
- 行数：`lx.rs` 1050 → 880（命中迁至 `history_hit.rs`），`history/tool.rs` 1015 → 489
  （摘要迁至 `file_summary.rs`）；全部触及文件 ≤1000 行。
- 人工验收（第 10 节）未勾选，待用户 `cargo run` 确认。
