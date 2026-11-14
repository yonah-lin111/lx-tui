# MCP 身份卡、`read` 行数与合并组间隔 — Review

- 审查范围：`dev...feat/mcp-identity-card` 全部代码与文档改动
- 审查基线：`7ac4b12b`
- 审查方式：对照 `design.md` / 标准文档逐项阅读实现，CLI 会话复核，补充对抗性单测
- 结论：P0 无；P1 1 项、P2 10 项、P3 3 项，均已修复；`cargo test` 5408 passed / 0 failed / 6 ignored，`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 通过。

| 级别 | 数量 | 状态 |
| --- | --- | --- |
| P0 | 0 | — |
| P1 | 1 | 已修复 |
| P2 | 10 | 已修复 |
| P3 | 3 | 已修复 |

## P1

### P1-1 MCP 流式早期退回旧通用卡

- **关键点**：`ToolCallView::new` 创建调用时 `detail = None`，渲染分派只按 `detail` 判断，MCP 调用在参数到达前会走 `file_summary` 通用卡；标题仍由 `verb_label` 拼成 `server · tool`，参数完整时还可能把参数当路径渲染并登记点击面。
- **错误原因**：身份信息（server/tool）只由工具名决定，却被错误地绑定到参数接收完成后的 `accept_arguments`；渲染层没有按工具名兜底。
- **证据**：`src/app/state/lx/chat/steps.rs:541`、`src/ui/main_content/lx/history/tool.rs:146`、`src/ui/main_content/lx/history/tool/header.rs:93`。
- **修复**：`ToolCallView::new` 在工具名为 MCP/resources 时立即用 `McpDetailView::from_arguments(name, "")` 建身份详情；`verb_label` 对 MCP/resources 统一返回 `VERB_MCP`。
- **验证**：`mcp_call_has_identity_detail_before_arguments_arrive`、`resource_call_has_identity_detail_before_arguments_arrive`、`streaming_new_call_renders_identity_card_not_generic_card`。

## P2

### P2-1 中断 MCP 卡显示 `interrupted: interrupted`

- **关键点**：中断调用落库的稳定错误码就是 `interrupted`，原因行又无条件拼 `label: reason`，用户看到重复状态词。
- **错误原因**：原因行没有处理“原因与状态词相同”的情况；测试只用了自定义原因。
- **证据**：`src/db/chat/tools.rs:21`、`src/app/state/lx/chat/stream.rs:394`、`src/ui/main_content/lx/history/tool/mcp.rs:37`。
- **修复**：原因与 label 忽略大小写相同时只输出一次 label。
- **验证**：`interrupted_reason_equal_to_status_word_is_not_duplicated`。

### P2-2 `Cancelled` 的 MCP 卡没有任何状态语义

- **关键点**：用户 Esc 取消一轮时 MCP 调用会被标为 `Cancelled`，但 MCP 卡仍显示耗时/图片或空白身份行，看起来像成功。
- **错误原因**：`status_of` 只排除 `Failed | Interrupted`，原因行也只处理这两种状态；组内兜底同样漏掉 `Cancelled`。
- **证据**：`src/ui/main_content/lx/history/tool/mcp.rs:110`、`src/app/state/lx/chat/stream.rs:408`、`src/ui/main_content/lx/history/tool/tree.rs:448`。
- **修复**：`Cancelled` 不再显示耗时/图片；单卡追加 `cancelled` 行，合并态作为 tool 的兄弟节点显示；组头不新增计数。
- **验证**：`cancelled_card_shows_status_word_without_duration`。

### P2-3 MCP 身份字段无长度/控制字符保护

- **关键点**：`server` / `tool` 直接进入折行节点。超长 tool 名或模型传入的超长 `read_resource.server` 会生成上万物理行；换行/制表符会打乱折行与网格列宽。
- **错误原因**：其它摘要字段都会先走 `single_line(..., TEXT_SAMPLE_CHARS)`，唯独身份字段没有复用同一保护。
- **证据**：`src/ui/main_content/lx/history/tool/mcp.rs:22`、`src/mcp/manager/connect.rs:203`、`src/ui/main_content/lx/history/tool/header.rs:352`。
- **修复**：单体与合并态的 server/tool 都先 `single_line` 净化并按 512 字符截断，再交给树体折行。
- **验证**：`identity_fields_are_single_line_and_bounded`。

### P2-4 `images_omitted` 被静默丢弃

- **关键点**：结果超过图片上限时只显示 `images.len()`，6 张图片会显示成 `4 image(s)`。
- **错误原因**：状态只统计已归档图片，没有把 `images_omitted` 计入总数。
- **证据**：`src/ui/main_content/lx/history/tool/mcp.rs:118`、`src/mcp/result.rs:73`。
- **修复**：标题图片数改为 `images.len() + images_omitted`。
- **验证**：`image_count_includes_omitted_images`。

### P2-5 `read` 后缀恰好放得下时被丢弃

- **关键点**：内容宽 18、路径 1 列、后缀 17 列时，`available > suffix_width + 1` 仍为 false，白丢后缀并多截断路径。
- **错误原因**：无条件为路径预留“1 列内容 + 1 列省略符”；路径本身已放得下时不需要省略符。
- **证据**：`src/ui/main_content/lx/history/tool/file_summary.rs:102`。
- **修复**：先算路径宽度；路径放得下时只需要 1 列，放不下时才要求 2 列。
- **验证**：`read_suffix_is_kept_when_it_exactly_fits`。

### P2-6 组合字符/零宽字符让 `read` 路径折成多行

- **关键点**：macOS NFD 文件名含组合字符时，`truncate_with_ellipsis` 按 0 列计算，`wrap_spans` 却按每字符至少 1 列计算；截断认为放得下，布局仍会折行，违反 `read` 恒单行。
- **错误原因**：截断与折行使用了两套宽度口径；`Grid::put_clipped` 实际跳过 0 宽字符。
- **证据**：`src/ui/main_content/lx/history/tool/header.rs:319`、`src/ui/main_content/lx/history/sticky.rs:219`、`src/ui/main_content/lx/grid.rs:195`。
- **修复**：`wrap_spans` 改为 `ch.width().unwrap_or(0)`，与截断和网格写入统一口径。
- **验证**：`combining_marks_do_not_wrap_read_path`。

### P2-7 图片 `read` 带 offset/limit 时实时与重载摘要不一致

- **关键点**：图片结果没有 `total_lines`，`absorb_summary` 用 `total_lines.is_some()` 作为“结果体已下结论”的闸门，导致实时路径保留参数区间、重载路径清空。
- **错误原因**：闸门假设所有 `read` 结果都带 `total_lines`，与 `to_summary` 对图片返回 `None` 的事实冲突。
- **证据**：`src/app/state/lx/chat/steps.rs:424`、`src/app/state/lx/chat/steps.rs:335`、`src/chat/tools/output.rs:630`。
- **修复**：闸门改为 `read_kind.is_some() || total_lines.is_some()`；图片结果同样清除参数推测区间。
- **验证**：`image_result_clears_argument_derived_range`。

### P2-8 文档与实现不一致

- **关键点**：`design.md` 与标准文档写身份分隔 ` · ` 用正常文字色，实现和测试是 muted；`tui-design-requirements.md` 把合并态 MCP 写成单行 `server · tool`；失败 MCP 合并示例漏掉组头 `· 1 failed`；极窄宽度分隔行规则未写明。
- **错误原因**：实现收敛后只更新了部分文案，旧口径残留。
- **证据**：`docs/feature/mcp-identity-card-and-group-spacing/design.md:83`、`docs/standards/lx-message-block-display.md:482`、`docs/standards/tui-design-requirements.md:129`。
- **修复**：统一为“server 浅黄、` · ` muted、tool 正常色”；合并态改为 server 一级、tool 二级；失败示例补 `· 1 failed`；明确 `width > TREE_NESTED_COLS` 才画分隔竖线。
- **验证**：文档关键词复查 + `verification.md` V2/V4/V12。

### P2-9 验证报告中的断言未被测试覆盖

- **关键点**：V3 声称中断不写耗时、连接符黄色，测试只检查原因文本；V10 声称窄宽仍占行数，测试只比较两个同源行数；V11 声称点击面不覆盖分隔行，测试未证明分隔行真实存在。
- **错误原因**：测试只覆盖“文本包含”，没有覆盖语义颜色、绝对行数和分隔行本身。
- **证据**：`docs/feature/mcp-identity-card-and-group-spacing/verification.md:33`、`:76`、`:82`。
- **修复**：补中断颜色/无耗时断言、单根组 6–8 列绝对行数断言、分隔行文本断言。
- **验证**：`interrupted_single_card_uses_warn_connector_color`、`narrow_single_root_group_keeps_tree_but_blank_separator`、`group_separator_row_is_not_inside_any_tool_card_region`。

### P2-10 MCP 仍参与键盘“可展开卡片”导航

- **关键点**：Ctrl+O 仍会切换 MCP 隐藏的 `Output` 折叠位，Alt+↑/↓ 仍会把 MCP 身份卡当作可展开卡片选中。
- **错误原因**：`is_expandable` 只看 `detail.is_some()`，没有跟随“MCP 无点击面/无展开内容”的新规则。
- **证据**：`src/app/update/chat/process.rs:641`、`src/app/update/chat/process.rs:754`。
- **修复**：`is_expandable` 排除 MCP；Ctrl+O 命中 MCP 时直接 no-op；删除隐藏折叠位切换分支。
- **验证**：`mcp_identity_card_is_not_expandable_for_keyboard_navigation`、`ctrl_o_on_mcp_card_is_a_noop`。

## P3

### P3-1 `task.md` 阶段 5 引用不存在的测试名

- **关键点**：`single_card_shows_only_identity_and_duration`、`mcp_card_registers_no_click_region`、`narrow_width_does_not_panic` 等名称与实际测试不符。
- **错误原因**：测试重写后未回填任务清单。
- **修复**：按实际测试名回填，并补充新增回归测试项。
- **验证**：`rg` 核对全部名称存在。

### P3-2 `McpDetailView` 模块文档仍描述“参数/结果窗口”

- **关键点**：结构保留参数/结果字段用于兼容，但文档仍说卡片展示这些窗口，误导后续维护。
- **错误原因**：渲染层重写后未同步数据层注释。
- **修复**：改为“身份数据；参数/结果仅供旧记录兼容与指纹使用”。
- **验证**：文档复查。

### P3-3 `verification.md` 写死测试总数

- **关键点**：`5395 passed` 会随新增测试变化，报告容易过期。
- **错误原因**：把一次性运行结果当稳定事实。
- **修复**：改为“以本次运行为准”，保留验证命令。
- **验证**：全量测试重跑。

## 未修复 / 已知取舍

- MCP 参数与结果仍保留在 `McpDetailView` 数据结构中，供旧记录解码与缓存指纹使用；渲染与点击面完全不再消费它们。
- 宽度 6–8 时，单根节点调用体仍保留自己的 `├─`/`└─`，但分隔行只占空白不画 `│`；这是为避免孤立竖线的有意取舍，已在标准与测试中固化。
- 合并组中 `Cancelled` 只在该调用体显示 `cancelled`，组头不新增 cancelled 计数；组头仍只聚合 failed/interrupted/running。
- 范围外既有问题：内容宽 6–8 时，展开态 `exec_command` / `web_search` 的分节行数与组合布局阈值不一致，可能造成点击面偏移；本分支未改动这两条布局路径，未纳入本次修复。
- 范围外既有问题：结果体缺少 `kind` 的旧 `read` 记录无法区分文件与目录，因此不显示行数；这是为避免把目录条目序号误报为文件行数的安全退化。
