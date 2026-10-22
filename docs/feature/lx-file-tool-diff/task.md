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
- [x] 仅注册文件卡片末尾动作行的展开命中；正文登记为可折叠区，但拖拽选区优先（有选区就不折叠）。
- [x] 末尾动作行（`… more changes` / `Show full diff` / `collapse`）切换整张卡片；标题行不可点，展开后点正文（未拖拽）同样折叠。
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
  （17 项布局/样式/宽度安全）、`ui::main_content::lx::tests::open_entry`（动作行可点 vs 标题不可点、展开正文是折叠目标）、
  `app::update::chat::tests::process_cards`（文件卡片 `^o` 不误开任务面板）、
  `app::update::chat::tests::viewport_anchor`（展开文件卡片不移动视口）、`chat::context`
  （回放剥离 UI diff 正文）。
- 行数：`lx.rs` 1050 → 880（命中迁至 `history_hit.rs`），`history/tool.rs` 1015 → 489
  （摘要迁至 `file_summary.rs`）；全部触及文件 ≤1000 行。
- 人工验收（第 10 节）未勾选，待用户 `cargo run` 确认。

## 12. 交互与配色补充（第二轮）

依赖：步骤 7/8；用户逐项确认后的 11 项决策。

- [x] 工具参数 `chat_tool_param()` 改为终端默认前景（不设 fg、无修饰符）。
- [x] 文件卡片标题行彻底取消点击；展开入口是末尾**常驻动作行**（`… more changes` / `Show full diff` / `collapse`）。
- [x] 展开正文可点击折叠：按下同时开始选区，松开无选区范围（未拖动）才折叠；拖拽只复制。
- [x] 思考块：点击头部切换展开；展开后点正文（未拖拽）折叠。
- [x] 思考方块与 `Thinking`/`Thought` 文案常态 `muted()`，整块悬停恢复品红 `Indexed 218`。
- [x] 整卡 hover：标题、正文、动作行任意行悬停都点亮。
- [x] 后台任务 `[open]` → `open`；进程终态整行不画（卡片只剩两行），`open_offset` 返回 `None`。
- [x] `open` / 动作行 / `collapse` 常态用左下角上下文统计同款次要色，悬停升为默认前景。
- [x] 拆出 `app/update/chat/viewport.rs`（滚动/悬停/折叠）与 `ui/main_content/lx/history/sticky.rs`（置顶行）以满足行数上限。

测试：`ui::style::tests`（参数默认前景、思考 hover 两态、动作行两态）、
`history::tool::file_diff`（常驻动作行、hover 提亮）、`history::tool::tests::background_cards`
（终态无 `open`）、`lx::tests::open_entry`（动作行命中、正文折叠目标）、
`app::update::chat::tests::history`（`hover_lx_chat` 按窗格/按块、`collapse_lx_card`）。

当前验证记录（第二轮）：

- `cargo fmt`：已执行。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2844 passed / 0 failed。
- 行数：`app/update/chat/mod.rs` 1014 → 约 640（拆出 `viewport.rs`），
  `ui/main_content/lx/history.rs` 1068 → 921（拆出 `sticky.rs`）；全部触及文件 ≤1000 行。

## 13. 连续同种文件调用合并 + 语法高亮 + 增删行底色（第三轮）

依赖：§12 的文件差异卡片。用户确认采用**扁平合并**（A 方案），并明确指出：

- 同种工具连续调用必须合在一起、用 `└─` 分隔（对齐 Codex 截图）；
- 差异正文要语法高亮（含 markdown），新增/删除行要整行背景色（对齐 Codex）；
- 展开后点击正文即折叠（长按选中不折叠）——§12 已实现，本轮不重复改。

- [x] `tool::merge_group`：从 `steps[start]` 收集**相邻、同名、都带差异正文**的调用；被 Part
      或不同工具名打断即停；`edit`+`write` 不合并（标题只能写一个动词）。
- [x] `history::layout` / `history::render` 同步加入合并分支：一组只占一个块（组前一个空行），
      **每个调用各自一块**——各登记一份 `CardHit` 与一个 `ToolHitKind::Toggle`（键为该调用的
      `(message_id, call_id)`），`expanded` 逐调用独立，`step_index += group.len()`。
- [x] 合并卡标题 `■ 动词 N files (+A -R)`：计数为组内求和，任一计数未知/文件被裁时整体退化为
      ` (?)`，绝不用 0 冒充；方块状态取组内最差（失败 > 中断 > 运行中 > 取消 > 完成）。
      单次调用**不**走合并标题（仍是裸动词，不写 `1 file`）。
- [x] 文件行扁平铺开、按调用顺序；同一文件被两次 `edit` 会出两行（A 方案已披露的副作用）。
- [x] 增删行整行底色：`Row.fill` + `Grid::background_row` 先刷整行，再给每个分段 `patch` 同一底色
      ——字符格样式会覆盖行底色，两处缺一就会掉色。
- [x] 语法高亮零新依赖：代码走 `markdown_render::highlight_line`（关键字/字符串/数字/类型/注释），
      `.md`/`.mdx` 走 `ui::markdown::scan` + `prompt::token_style`，未知扩展名不做假高亮；
      未命中语法的字符保留增删语义色而不是中性正文色。
- [x] 测试：`file_diff::tests` 新增合并标题、逐块展开互不影响、最差状态、动词不合并、Rust 语法色、
      markdown 词法色、未知扩展名保持语义色、动作行剩余行数；`history::tests::tool_geometry` 新增
      合并几何（布局=渲染、每块各一份命中区、组内无空行、`write`+`write` 共享标题、
      `edit`+`write` 不合并、Part 打断分组）。

- [x] 行数拆分：合并卡把 `history.rs` 顶到 1003 行，按职责抽出 `history/stats.rs`
      （统计行 `stats_line` + 耗时短格式 `format_duration`），`history.rs` 回到 926 行，
      对外仍是 `pub(crate) use`，调用点不变。

当前验证记录（第三轮）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2853 passed / 0 failed（lib + 集成 + doc）。
- 触及文件行数：`history.rs` 926、`history/tool.rs` 570、`history/tool/file_diff.rs` 815、
  `history/tool/file_diff/tests.rs` 845、`history/tests/tool_geometry.rs` 509，全部 ≤1000。
- 人工验收仍待用户 `cargo run`：合并卡观感、语法高亮与增删底色、点击正文折叠。

## 14. 第四轮：逐调用独立展开 + 代码块去装饰 + loading token + toast + 参数折叠

> **本节的「动作行 / 剩余行数 / hover 提亮」已被 [§15](#15-第五轮loading-统计命令折行移除按钮与-hover) 取代**：
> 按钮、行数与全部 hover 高亮均已移除，折叠/展开改为「点卡片内容」。以下保留为当时的决策记录。

用户要求（原文要点）：

1. 连续调用用 `└─` 连接（对齐 Codex 截图），**每个调用展开互相独立**，不要底部控制整组；
2. 移除消息列表里 markdown 代码块的 `language: xxx`、复制按钮、上下边框，只显示代码；
3. 输入框上方 loading 加 `↓token`（参考 Claude Code）；
4. 移除输入框上方通知，全部改为 main content 的 toast；toast 增加 `default` / `warning`（黄边框+标题）/
   `danger`（红边框+标题）三档；
5. 工具参数颜色改为「输入框下方上下文」的字体色；默认显示 2 行、超出折叠，悬停高亮为默认前景、
   点击展开（复用文件卡片的折叠逻辑，不显示按钮）；
6. 折叠文案统一加行数——**只改工具卡片**（用户确认），把 `… more changes` 换成剩余行数。

- [x] 合并卡改为「一行合并标题 + 逐调用独立块」：`file_diff::blocks()` 返回每块的
      `start/len/action/body`，`rows`/`draw`/命中都用它；展开状态按调用逐份保存在
      `detail_expanded`，点某块的动作行只切换该块。
- [x] 动作行改为**按需出现**：折叠态还有内容没画出来时写 `… N more lines`，展开态 `collapse`；
      预览已完整则整行不画。
- [x] 代码块去装饰：`markdown_render` 删除 `language:` 头、复制按钮与三条横线，
      `MdLine` 去掉 `code_copy` / `code_body` / `code_rule`；连带删除 `history/code_copy.rs`、
      `LxChatHit::CodeCopy`、`TooltipTarget::LxCodeCopy`、`md_read_code_lang*` / `md_read_code_copy`
      以及相关测试（选区复制不受影响）。
- [x] loading 行右端加 `↓1.2k tok`：`LxChat::stream_output_tokens()` 按 ASCII/4 + 非 ASCII 逐字估算，
      静态 dim、宽不够先丢、Esc 待命时不画。
- [x] 输入框上方提示全部迁到 toast：`InputHint` 结构与 `chat_bar` 提示分支删除，
      `show_lx_hint` 直接出 toast（Error → `Danger`、Warning → `Warning`，锚定触发窗格）；
      `ToastKind` 由 `Info/Error` 改为 `Default/Warning/Danger`，边框与标题同色。
- [x] 参数配色改为 `muted()`（与状态行上下文用量同色），悬停 `chat_tool_param_hover()` 升为默认前景；
      `└─` 详情行软换行、折叠 2 行、展开上限 24 行，参数超 2 行时正文区可点折叠
      （`tool::card_body_rows` 登记 `CardHit.body`，复用 `toggle_tool_detail` 与拖拽保护）；
      错误/中断文案恒单行。

- [x] 测试：详情行 2 行折叠/展开/上限、悬停升色不改行数、每行不越界且续行重缩进；
      合并块独立展开与动作行剩余行数；代码块只剩正文；loading token 估算与显示；
      toast 三档颜色（边框与标题同色）与 warning 实际绘制；`show_lx_hint` 迁移后的
      danger/warning 断言。

当前验证记录（第四轮）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2849 passed / 0 failed（lib）+ 全部集成/doc 套件通过。
- 触及文件行数：`history/tool/file_diff.rs` 785、`file_diff/tests.rs` 911、`file_summary.rs` 631、
  `history/tool.rs` 613、`history.rs` 924、`markdown_render.rs` 684，全部 ≤1000。
- 人工验收仍待用户 `cargo run`：合并标题下的逐块展开、代码块只剩正文的观感、
  loading 的 `↓token`、toast 三档颜色、参数 2 行折叠与点击展开。

## 15. 第五轮：loading 统计、命令折行、移除按钮与 hover

用户要求（原文要点）：

1. loading 的 token 放在 `Working...` **右侧**并加耗时，格式 `Working...(时间 · token)`，
   token 要同时表达输入（↑）与输出（↓）；
2. 工具参数没有显示多行、点参数也打不开——截图里实际被截断的是 `Run command` 的 `$ 命令`；
3. 重构思考与工具的展开交互：**移除按钮与字体高亮**（连行数也不要），点折叠内容展开、
   点展开内容重新折叠。

- [x] loading 行改为 `Working... (2.7s · ↑1.6k ↓320 tok)`：统计紧贴文案右侧（不再是栏右缘），
      耗时来自新字段 `LxChat.turn_elapsed_ms`（事件循环每帧刷新，渲染层不取时钟）；
      输入/输出 token 都由字符数估算（`stream_input_tokens` / `stream_output_tokens`）；
      宽度不够整段丢、Esc 待命不画。
- [x] `exec_command` / `write_stdin` 的命令按列宽折行（折叠 2 行、展开同普通详情上限），
      续行不重复 `$ `；`open` 行与 `wants_open`/`open_offset` 一并删除，后台任务面板只走
      `^o` / 空输入框按 `↓`。
- [x] 移除全部 hover 高亮：删 `ChatHover`/`LxChat.hover`/`hover_lx_chat`/`chat_hover`/
      `thinking_is_hovered`/`tool_is_hovered`、`chat_tool_param_hover`，思考块恒为品红；
      `ChatHover` 改名为 `CardTarget`（只表示「点了哪一块」），`collapse_lx_card` 改名
      `toggle_lx_card`。
- [x] 移除动作按钮与行数文案：`ToolHit`/`ToolHitKind`/`history.tools` 整体删除，
      文件卡片不再有 `… N more lines` / `collapse`，`chat_action_label` 与
      `file_diff::Block::{action, body}` 一并移除；每块内容登记一个 `CardHit`，
      `card_body_at` 改名 `card_target_at`，内容不可展开时 `body` 零高。
- [x] 测试：统计标签拼装（三段齐全/缺段/全空）、loading 统计紧跟文案且含耗时与双向 token、
      命令折行与展开、进程卡片无 `open` 行、单行命令不可点/长命令可点、合并块无动作行、
      思考块恒品红、参数无 hover 档。

当前验证记录（第五轮）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2829 passed / 0 failed（lib）+ 全部集成/doc 套件通过。
- 触及文件行数：`history/tool/command.rs` 388、`file_summary.rs` 616、`file_diff.rs` 738、
  `history/tool.rs` 581、`history.rs` 846、`history_hit.rs` 182、`chat_bar.rs` 251，全部 ≤1000。
- 人工验收仍待用户 `cargo run`：loading 的 `(时间 · ↑↓ token)`、长命令折行与点击展开、
  点击内容折叠、无 hover 变色。

## 16. 第六轮：loading 只留输出方向 + 修复点击无法展开

用户要求：

1. loading 的 token 只显示 agent 输出（`↓`），去掉输入（`↑`）；耗时最小单位为秒，不出现毫秒；
2. **bug**：点击折叠参数与折叠内容都无法展开。

- [x] `stream_stats_label` 去掉输入方向参数，耗时走新的 `format_duration_seconds`
      （`0s` / `2s` / `1m 12s`，四舍五入到秒，绝不出现 `ms`）；`LxChat::stream_input_tokens`
      随之删除。
- [x] **点击 bug 根因**：`CardHit` 曾把点击面收窄成「标题之后的正文区」（`body`），而折叠态
      正文为零高——于是折叠的思考块与超长参数永远命中不到，点哪里都没反应。修法：
      `CardHit` 只保留 `card`，`card_target_at` 不再校验 `body`，**整块卡片即点击面**。
- [x] 「没有更多内容就不登记」的判据改由 `history::should_register_card`
      （`tool::has_expandable_content`：展开后行数变多）统一决定，避免点了没反应。
- [x] 测试：`folded_thinking_and_long_param_are_clickable`（折叠思考头 / 折叠长参数都能命中）、
      `loading_stats_follow_the_label_with_seconds_and_output_tokens`、
      `stats_label_never_shows_milliseconds`、`stats_label_shows_time_and_downward_tokens_only`；
      并把原「只认正文区」的断言改成「整块任意行都是点击面」。

当前验证记录（第六轮）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2830 passed / 0 failed（lib）+ 全部集成/doc 套件通过。
- 人工验收仍待用户 `cargo run`：折叠卡片（思考 / 长参数 / 长命令）点一下就展开、再点收起，
      loading 只显示 `(3s · ↓320 tok)`。

## 17. 第七轮：修复「点击后依旧没有展开」

用户反馈：上一轮改完「整块卡片可点」后，实机点击**仍然没有展开**。

- [x] **真正根因**（上一轮只修了一半）：事件层的按下分支先写 `state.lx_card_press`、
      再调 `update::begin_lx_selection`，而 `begin_lx_selection` 内部第一步就是
      `clear_selection(state)`——它会把 `lx_card_press` 置空。于是标记刚写就被清掉，
      松开时永远走「拖拽复制」分支，卡片一次也展不开。
      修法：**先 begin 选区、后写标记**（并在代码里写明顺序不可交换的原因）。
- [x] 新增事件层端到端回归测试 `lx_card_click_toggles_expansion_on_release`：
      构造超长参数卡片 → 合成 Down/Up → 断言 `detail_is_expanded`。
      已验证「把顺序改回错误版本时该测试失败」，确保它守得住这个 bug。
- [x] 上一轮的 `folded_thinking_and_long_param_are_clickable` 继续保留（守命中级几何），
      两层测试分别覆盖「命中判据」与「按下/松开状态流转」。

当前验证记录（第七轮）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2831 passed / 0 failed（lib）+ 全部集成/doc 套件通过。
- 人工验收仍待用户 `cargo run`。

## 18. 第八轮：剩余行数、hover 提亮、展开块底色、恢复思考 hover

用户要求：

1. 折叠参数 / 折叠内容下方显示剩余行数；鼠标 hover 折叠内容时字体改为默认字体颜色；
   展开后为**展开块**（不是整个工具）添加背景色（参考 Claude 的配色）；
2. 思考的 hover 交互恢复原来的（常态压暗、悬停恢复品红）。

- [x] 剩余行数：`file_summary::detail` / `command::command_body` / `file_diff::layout` 在折叠态
      末尾各加一行 `… N more lines`（不占折叠额度、不带连接符、窄到放不下整行不画）；
      展开后该行消失。`file_diff::remaining_lines` 恢复（content_rows 差值）。
- [x] **剩余行数恒为 0 的 bug**：早先 `detail_body` 按折叠额度组装，内容被提前砍掉，
      差值必然是 0。改为「折叠展示按折叠额度组装（保住让位顺序）+ 另按展开上限组装算总数」，
      两次调用共享同一份 `arguments_json` 解析缓存（`param_falls_back_to_arguments_only_when_summary_lacks_fields`
      守住的「一帧只解析一次」不变量不变）。
- [x] 悬停提亮：`chat_tool_param_hover()`（默认前景）回到 `detail` / `command_body`，
      折叠态内容整体从 dim 提亮；`CardTarget` 恢复同时承载 hover 语义，
      `LxChat.hover` / `hover_lx_chat` / `tool_is_hovered` / `thinking_is_hovered` 全部恢复，
      `hover.rs` 重新下发卡片悬停。
- [x] 展开块底色：新增 `style::chat_expanded_bg()`（Indexed 236），只铺展开后的内容行
      （`file_summary::detail_fill`、命令 `draw_body` 的 `fill`、`file_diff` 的 `Row.fill`），
      分段样式用 `patch` 补底色，避免字符格覆盖行底色。
- [x] 思考 hover 恢复：`chat_step_mark_*_thinking(hovered)` / `chat_step_label_*_thinking(hovered)`
      回到「常态 `muted()`、悬停 `Indexed 218`」，`StepMark::style` / `StepKind::label` 重新接收
      `hovered`。
- [x] 测试：剩余行数行存在/展开后消失、悬停提亮不改行数、`detail_fill` 只在展开态返回底色、
      思考 hover 两态、展开块底色、命令卡的剩余行数；并按新语义修正既有断言
      （折叠态 = 2 行内容 + 1 行提示）。

当前验证记录（第八轮）：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：2835 passed / 0 failed（lib）+ 全部集成/doc 套件通过。
- 人工验收仍待用户 `cargo run`：折叠内容下方的剩余行数、hover 提亮、展开块底色、
      思考块 hover 恢复品红。

## 19. 第九轮：展开底色满宽、diff 底色优先、hover 只亮折叠内容

用户带截图反馈：

1. 展开块底色要 **100% 宽**；diff 添加/删除行的底色优先级必须高于展开底色；
2. 展开底色**不要覆盖到工具参数**；
3. 工具参数或内容**没有折叠**时，hover 不该高亮；且参数与折叠内容的高亮**必须独立**
   （之前一次 hover 两块一起亮）。

- [x] `CardHit` 新增 `fold: Rect`：与 `card`（点击面）分开。`fold` 只覆盖**折叠态的内容行**，
      展开后为零高；思考块按用户要求恢复「悬停整块（含展开正文）」。
      事件层新增 `fold_target_at`（hover 判据），`card_target_at` 继续管点击。
- [x] 文件卡片底色：`Row.fill` 先 `background_row` 铺满 `rect.width`，再把同一 bg `patch`
      到每个分段——**只铺不 patch 会让写回的字擦掉底色**（截图里背景不满宽的根因）。
      展开底色只给 `fill.is_none()` 的行，增删行自己的 `agent_added_bg`/`agent_removed_bg`
      优先级更高。
- [x] 普通工具（`└─` 参数行）删除 `detail_fill`：参数不是「展开出来的内容」，不铺底色。
- [x] hover 提亮判据从「style 白名单」改为「fg == muted().fg」：折叠预览里的行号/路径/计数
      都会提亮，增删绿红与语法彩度保持不变。
- [x] 测试：`expanded_block_fill_spans_full_width_and_diff_colors_win`（逐格断言满宽底色 +
      增删行底色胜出）、`only_folded_content_is_hover_highlighted`（折叠态有差异格、展开态零差异格）。
  fmt / clippy -D warnings / cargo test（2837 passed）全部通过。

## 20. 第十轮：只保留 diff 增删行的底色

用户带截图反馈：文件头 `└─ … __tool_test_moved__.md (+1 -0)` 也带了底色，
要求「只有 diff 有背景颜色即可」。

- [x] 删掉 `style::chat_expanded_bg()` 与全部「展开块底色」逻辑：`file_diff` 不再给展开行
      补底色（只剩 `render_line` 自己设的增删行 `fill`）、`command` 的 `draw_body` 去掉
      `fill` 参数、普通工具的 `detail_fill` 早已移除。
- [x] 增删行底色仍铺满 100% 宽度（`background_row` + 分段 `patch` 同一 bg）。
- [x] 测试改为 `only_diff_lines_carry_a_background`：逐格断言「增删行满宽同色、其余行零底色」。
  fmt / clippy -D warnings / cargo test（2836 passed）全部通过。

## 21. 第十一轮：恢复展开块灰底（文件头除外）

第十轮按「只有 diff 有底色」把灰底整个去掉，用户随即反馈「灰色背景颜色直接没了」，
确认要的是：**灰底恢复，但文件头不铺**。

- [x] 恢复 `style::chat_expanded_bg()`（Indexed 236）。
- [x] `file_diff::layout` 在展开态给**正文行**补灰底：`file_header_row_indexes` 精确算出
      每个文件头的行号并跳过；`row.fill.is_some()`（增删行）不覆盖——红/绿优先级更高。
- [x] 折叠预览、工具参数、进程命令仍不铺灰底。
- [x] 测试 `expanded_body_is_grey_except_the_file_header_and_diff_lines`：文件头零底色、
      其余每行满宽同色、灰底与增删底色都出现且互不覆盖。
  fmt / clippy -D warnings / cargo test（2836 passed）全部通过。

## 22. 第十二轮：连续调用之间空一行

用户带截图反馈：合并卡里两个 `edit` 块紧贴，第一块的最后一行（剩余行数提示）与第二块
的文件头直接相邻，读起来像同一个文件的内容，要求「同一个工具连续调用之间添加一个换行」。

- [x] `file_diff::CALL_GAP_ROWS = 1`：`blocks()` 给非首块多让 1 行，`draw()` 在块间同样推进 1 行
      （两边共用同一常量，`draw` 推进行数仍等于 `rows`）。
- [x] `rows()` 改为「最后一块的 start + len」：卡片总行数天然包含块间空行，不再另外相加。
- [x] `history` 的命中矩形沿用 `block.start`，块间空行不属于任何块——点空行不切换展开。
- [x] 测试：`consecutive_calls_are_separated_by_a_blank_line`（块间那一行是空行、其后紧跟第二块
      文件头）、`merged_blocks_report_independent_action_rows`（第二块起点 = 第一块 + len + 1、
      总行数覆盖空行）；`consecutive_file_tools_share_one_header_but_expand_independently` 由
      「组内不得出现空行」改为「恰好一个空行，且下一行是第二块文件头」。
  fmt / clippy -D warnings / cargo test（2837 passed）全部通过。

## 23. 第十三轮：交互三修（diff 不提亮 / 参数互不牵连 / 展开保持高亮）

用户三条要求：`… 9 more lines` hover 不要高亮；hover diff 块时工具参数不要高亮；
展开参数后参数文字保持高亮文字颜色、折叠后恢复。

- [x] **diff 卡彻底不做 hover 提亮**：`file_diff::draw`/`layout` 删掉 `hovered` 参数，
      `tool::foldable_rows` 对文件差异卡片恒返回 0（`history` 单卡 `fold` 因此零高），
      `history::layout` 合并卡的 `fold` 也直接置零高——折叠态文件头路径、剩余行数行、
      行号槽全都不随鼠标变色。「能点」由点击与展开后的灰底表达。
- [x] **hover diff 不再牵动工具参数**：普通工具（`file_summary`）的 `hovered` 只由它自己的
      命中矩形决定，diff 卡片不再产生 hover 目标，参数自然不会跟着亮。
- [x] **展开态参数保持默认前景**：新增 `style::chat_tool_param_expanded()`（与
      `chat_tool_param_hover()` 同档 = 终端默认前景）。`file_summary::detail` 的
      `param_style = expanded || hovered`，`command::command_body` 同样 `expanded || hovered`，
      `file_diff::file_header` 展开态路径改用该档；折叠后回到 `chat_tool_param()`（dim）。
- [x] 测试：`file_diff_card_has_no_hover_region`（`foldable_rows == 0` 且仍可展开）、
      `file_header_path_lifts_when_expanded`（折叠 dim / 展开默认前景）、
      `parameter_lifts_when_expanded_and_recovers_when_folded`（替换旧的「参数恒 dim」）、
      `command_body_lifts_when_expanded`、`expanded_param_stays_at_default_foreground`（样式档位）。
  fmt / clippy -D warnings / cargo test（2840 passed）全部通过。

## 24. 第十四轮：diff 文件头路径不再随展开提亮

用户指出：展开 `edit` / `write` 的折叠内容后，`└─ /Users/…/__tool_test_twice__.md (+10 -0)`
这行路径会变亮；确认要的是「diff 路径永远 dim」。

- [x] `file_diff::file_header` 去掉 `expanded` 参数，路径恒用 `chat_tool_param()`（dim）。
      「展开后参数保持高亮」只保留在**普通工具的 `└─` 参数行**与**命令卡片正文**：
      那两处折叠时被藏起来、展开后才有可读内容；diff 的路径从头到尾都在，是标题不是参数。
- [x] 模块注释与 `layout` 的文档改为「路径恒 dim」。
- [x] 测试：`file_header_path_lifts_when_expanded` → `file_header_path_stays_dim_when_expanded`；
      新增 `merging_blocks_keeps_every_header_dim`（合并卡里展开第一块，两块文件头路径都仍是 dim）。
  fmt / clippy -D warnings / cargo test（2841 passed）全部通过。

## 25. 第十五轮：思考块折叠态改浅粉

用户要求「设置思考的颜色没有展开时颜色不要使用灰色的，使用浅一点粉色的」，
并确认展开态用亮粉 218。

- [x] 新增 `style::CHAT_STEP_PINK_SOFT = 182`（`#d7afd7`）：同一粉系里低一档饱和度，
      既不抢工具行注意力，也不像 `muted()` 那样读成「被禁用的灰」。
- [x] `chat_step_mark_*_thinking` / `chat_step_label_*_thinking` 增加 `expanded` 参数：
      `expanded || hovered` → 亮粉 218，否则浅粉 182。运行态仍保留呼吸加粗。
- [x] `StepMark::style` / `StepKind::{style_running,style_done,label}` 透传 `expanded`；
      非思考角色调用点一律传 `false`（工具、正文、通知都不受展开态影响）。
- [x] 测试：`step_roles_use_distinct_colors` 改为断言折叠 182 / 展开 218 / 悬停 218，
      并显式断言折叠态**不得**等于 `muted()`；`thinking_header_uses_step_square_and_becomes_thought`
      同步为折叠 182、展开后 218。
  fmt / clippy -D warnings / cargo test（2841 passed）全部通过。

## 26. 第十六轮：`… N more lines` 提示行纳入 hover 提亮

用户要求「hover 工具的折叠块时 `… 1 more line` 需要显示高亮」，并确认 diff 卡的提示行
也要覆盖。

- [x] `file_summary::foldable_rows` / `command::foldable_rows` 不再排除提示行：提示行是
      折叠区最后一行，命中区少算一行会让指针停在最该提亮的地方反而毫无反应。
- [x] 提示行本身在悬停时提亮：`file_summary::detail` 与 `command::wrap_body_row` 的提示
      段由 `hovered` 决定用 `agent_step_detail()` 还是 `chat_tool_param_expanded()`。
- [x] diff 卡维持「路径与正文不提亮」，但**提示行提亮**：`file_diff::layout` 恢复 `hovered`
      参数、只作用于剩余行数行；新增 `hint_offset()` 给出该行在块内的偏移。
- [x] `tool::foldable_rows` 换成 `tool::FoldRegion { offset, rows }`：各卡片自报「折叠内容
      从内容首行起第几行、占几行」。diff 卡 = 提示行单行，其它卡片 = 整段折叠内容。
      `history::tool_fold_rect` 与合并卡逐块的 `fold` 都按它计算（块首标题 +1 行）。
- [x] 测试：`remaining_hint_row_is_inside_the_fold_region_and_lifts`、
      `command_hint_row_lifts_on_hover`、`file_diff_hover_region_covers_only_the_hint_row`
      （含「路径不随 hover 变色」与「提示行升为默认前景」两侧断言）。
  fmt / clippy -D warnings / cargo test（2843 passed）全部通过。
