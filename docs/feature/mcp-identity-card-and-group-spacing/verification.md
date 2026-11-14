# MCP 身份卡、`read` 行数与合并组间隔 — 验证报告

- 工作区：`/Users/yonah/projects/agent/lx-tui`（`dev`，由 `feat/mcp-identity-card` 快进合入）
- 代码基线：`41fc6751 fix(ui): address MCP identity card review findings`
- CLI 验证会话：`sess-18e7c5ca8ad0dae8-0`（`lx-tui exec`，默认模型 `provider-alq374/cbcn/deepseek-v4.1-flash#medium`）
- 结论：CLI、自动化测试、文档检查与合入验证全部通过；真实终端观感待用户 `cargo run` 确认。

| 验证范围 | 结果 |
| --- | --- |
| V1–V13：CLI 会话、渲染单测、几何、文档、构建 | 通过 |
| V14：真实 TUI 视觉与交互 | 待用户人工确认 |
| V15：合入 `dev` 与主工作区验证 | 通过 |

## V1 CLI 会话端到端 — 通过

- 输入内容：在目标工作区新建 `lx-tui exec` 会话，执行 7 个只读步骤：`codegraph_explore` → `codebase-memory-mcp/list_projects`（与上一步相邻）→ `read src/ui/text/agent.rs`（offset 58，limit 50）→ `spawn_agent` ×2 → `wait_agent` ×2（相邻、分别等待一个目标）。
- 验证步骤：
  ```bash
  ./target/debug/lx-tui exec -C . --color never - < prompt.txt
  ./target/debug/lx-tui session show sess-18e7c5ca8ad0dae8-0 --tools --json
  ```
- 期望结果：生成 session；工具顺序严格为 `mcp__codegraph__codegraph_explore`、`mcp__codebase-memory-mcp__list_projects`、`read`、`spawn_agent` ×2、`wait_agent` ×2；`read` 返回 `start_line=58`、`end_line=107`（50 行）；工作区无文件改动。
- 实测：全部命中；会话中 MCP 时长为 `162ms` / `1.1s`。

## V2 MCP 单体身份卡 — 通过

- 输入内容：`McpDetailView { server: "github", tool: "create_issue", duration_ms: 1200, images: 1, arguments/result 非空 }`。
- 验证步骤：`cargo test single_card_shows_only_identity_duration_and_image_count`、`cargo test identity_line_uses_param_normal_and_muted_styles`、`cargo test long_server_and_tool_wrap_without_losing_content`、`cargo test narrow_widths_keep_rows_and_draw_in_step`、`cargo test image_count_includes_omitted_images`。
- 期望结果：2 行——`■ MCP · 1.2s · 1 image(s)` 与 `└─ github · create_issue`；server 浅黄、` · ` muted、tool 正常色；不出现参数、结果、`[image`；长身份名折行不丢字符；宽度 1/4/8/12/20 下 `rows == draw`；`images_omitted` 计入图片总数。

## V3 MCP 失败/中断 — 通过

- 输入内容：`Failed` / `Interrupted` / `Cancelled` 且带原因；原因为空；多行超长原因；稳定中断错误码 `interrupted`。
- 验证步骤：`cargo test failed_card_appends_one_reason_line_without_duration`、`cargo test interrupted_card_appends_one_reason_line`、`cargo test interrupted_reason_equal_to_status_word_is_not_duplicated`、`cargo test cancelled_card_shows_status_word_without_duration`、`cargo test failed_card_without_reason_has_no_reason_line`、`cargo test multiline_reason_is_collapsed_to_one_truncated_line`、`cargo test failed_single_card_uses_semantic_connector_color`、`cargo test interrupted_single_card_uses_warn_connector_color`。
- 期望结果：标题不写耗时；原因行恒为一条 `failed: …` / `interrupted: …`；稳定错误码与状态词重复时只显示一次；取消显示 `cancelled`；原因为空时失败卡不追加行（总行数 2）；失败首节点连接符红色、中断黄色。

## V4 MCP 合并组 — 通过

- 输入内容：两个不同 server/tool 的相邻 MCP 调用；`read_resource` 桥接工具；流式早期尚未收到参数的 MCP 调用；失败 MCP 合并调用。
- 验证步骤：`cargo test adjacent_mcp_calls_merge_into_one_identity_group`、`cargo test mcp_and_resource_bridge_tools_share_one_group_key`、`cargo test group_body_puts_server_at_root_and_tool_below_it`、`cargo test group_body_puts_error_reason_below_tool`、`cargo test streaming_new_call_renders_identity_card_not_generic_card`、`cargo test mcp_call_has_identity_detail_before_arguments_arrive`；对照 V1 会话前两个调用。
- 期望结果：组头 `■ MCP · N calls`（含失败调用时追加 `· N failed`）；server 为一级节点并承载耗时/图片数，tool 为二级；不同 server/tool 与 resources 桥接工具同组；失败原因作为 tool 的兄弟二级节点；流式早期也直接显示身份卡，不退回 `server · tool` 通用卡。
- 实测：V1 会话前两个调用相邻，TUI 应显示 `■ MCP · 2 calls`，`codegraph · 162ms` 与 `codebase-memory-mcp · 1.1s` 各带二级 tool。

## V5 MCP 点击面与可展开性 — 通过

- 输入内容：带长参数、长结果、图片元数据的 MCP 调用。
- 验证步骤：`cargo test mcp_card_has_no_expandable_content_or_click_region`、`cargo test mcp_identity_card_is_not_expandable_for_keyboard_navigation`、`cargo test ctrl_o_on_mcp_card_is_a_noop`。
- 期望结果：`has_expandable_content == false`；`card_regions` 为空；参数/结果不因点击出现；Alt+↑/↓ 不选中 MCP；Ctrl+O 不切换隐藏折叠位、不打开任务面板。

## V6 `read` 完成态区间与行数 — 通过

- 输入内容：`path = src/ui/style.rs`、`range = (12, 40)`、`read_lines = 29`；单行 `range = (12, 12)`、`read_lines = 1`；V1 会话的真实结果 `58–107`。
- 验证步骤：`cargo test read_file_shows_kind_in_title_and_range_in_body`、`cargo test single_line_read_uses_one_line_number_and_singular_label`、`cargo test read_suffix_is_kept_when_it_exactly_fits`；对照 V1 会话 `read` 结果。
- 期望结果：`src/ui/style.rs:12-40 · 29 lines`；单行 `src/a.rs:12 · 1 line`；后缀 muted；路径与后缀恰好占满内容宽时仍保留后缀；V1 会话显示 `src/ui/text/agent.rs:58-107 · 50 lines`。

## V7 `read` 运行中/旧记录 — 通过

- 输入内容：运行中且只有参数区间；完成态但只有参数派生摘要（无 `read_lines`）。
- 验证步骤：`cargo test running_read_shows_path_only`、`cargo test read_without_result_line_count_shows_path_only`。
- 期望结果：两种情况都只显示路径，不出现 `:12-40` 或 `line`；不拿参数 `offset` / `limit` 预估。

## V8 `read` 长路径、窄宽、目录与图片 — 通过

- 输入内容：120 字符路径；宽 12；含组合字符的 NFD 路径；宽 18；目录读取；图片读取。
- 验证步骤：`cargo test long_read_path_keeps_suffix_and_truncates_path`、`cargo test narrow_read_drops_suffix_before_path`、`cargo test combining_marks_do_not_wrap_read_path`、`cargo test read_directory_shows_count_in_title_and_path_in_body`、`cargo test read_image_shows_image_kind_in_title_and_path_in_body`。
- 期望结果：长路径优先保留 `:12-40 · 29 lines`，路径自身省略；后缀放不下时整段丢弃；目录/图片不显示行数；组合字符按 0 列计宽，路径始终单行。

## V9 `wait_agent` 不合并 — 通过

- 输入内容：连续两个 `wait_agent`（首个 2 targets，第二个 1 target）；V1 会话中的两个相邻 `wait_agent` 调用。
- 验证步骤：`cargo test consecutive_wait_agent_calls_stay_as_separate_cards`、`cargo test wait_agent_is_never_groupable`；对照 V1 会话工具序列。
- 期望结果：两张独立 `■ Wait agents` 卡片，无 `N calls` 组头；卡间为普通块间空行；每张卡内多目标仍是一级平级分支。

## V10 列表合并组分隔行 — 通过

- 输入内容：连续 3 个 `read`、2 个 `web_search`、2 个 `edit`；宽度 6 的窄窗格；宽度 7 的两个 `read` 单根组。
- 验证步骤：`cargo test consecutive_tool_steps_share_one_group_header`、`cargo test web_search_group_nests_results_under_each_query`、`cargo test consecutive_file_tools_share_a_group_header`、`cargo test narrow_group_counts_separator_rows_without_drawing_tree_prefix`、`cargo test narrow_single_root_group_keeps_tree_but_blank_separator`。
- 期望结果：每个非末尾调用体尾部恰有 1 行 `  │`，末尾调用与快照组不追加；分隔行计入块行数与滚动几何，但不登记点击面；宽度大于 `TREE_NESTED_COLS` 才画竖线，更窄时仅保留一行空白（单根组在 6–8 列仍保留自己的 `├─`）。

## V11 分隔行点击面与快照组 — 通过

- 输入内容：两个合并 `edit` 调用；`update_plan` 连续两次更新。
- 验证步骤：`cargo test group_separator_row_is_not_inside_any_tool_card_region`、`cargo test plan_snapshot_group_shows_only_the_latest_update_and_header_progress`。
- 期望结果：先断言分隔行文本确为 `  │`，再确认所有调用体点击面止于分隔行之前；快照组合并只有一个调用体，不出现分隔行。

## V12 规范文档同步 — 通过

- 输入内容：旧口径关键词：`不显示请求区间`、`MCP 卡片的参数块`、`MCP 结果`、`组内不空行`、`server · tool · 840ms`、`MCP_NO_ARGUMENTS`。
- 验证步骤：
  ```bash
  rg -n "不显示请求区间|MCP 卡片的参数块|MCP 参数块|MCP 结果|组内不空行|合并组内调用之间不空行|server · tool · 840ms|MCP_NO_ARGUMENTS" docs/standards src/ui
  ```
- 期望结果：无匹配；`lx-message-block-display.md` 已包含 `MCP · 840ms`、常量键 `mcp`、`wait_agent` 不合并、`:12-40 · 29 lines` 与分隔行规则。

## V13 构建与全量测试 — 通过

- 输入内容：当前工作区全部代码与测试。
- 验证步骤：
  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```
- 期望结果：格式无差异；clippy 零警告；全量测试通过。
- 实测：lib 全量测试与全部集成测试通过（具体数量以本次运行为准）。

## V14 真实 TUI 视觉验收 — 待用户确认

- 输入内容：V1 会话 `sess-18e7c5ca8ad0dae8-0`。
- 验证步骤：在目标工作区运行 `cargo run`，打开上述会话，依次检查 MCP 合并卡、`read` 卡、两张 `wait_agent` 卡与滚动/点击。
- 期望结果：MCP 卡为 `■ MCP · 2 calls`（server 一级、tool 二级、中间 `│`）；`read` 卡显示 `src/ui/text/agent.rs:58-107 · 50 lines`；两张 `wait_agent` 卡之间空一行；MCP/`read` 卡点击无展开；窄窗格无裁切、滚动位置与点击命中不偏移。

## V15 合入 `dev` — 通过

- 输入内容：`feat/mcp-identity-card` 的 3 个提交。
- 验证步骤：`git merge --ff-only feat/mcp-identity-card`；随后在主工作区运行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- 期望结果：无合并冲突；格式与 clippy 零告警；全量测试通过。
- 实测：`dev` 从 `8c995980` 快进至 `41fc6751`；`cargo test` 5408 passed / 0 failed / 6 ignored。
- 清理：合入后移除 `mcp-identity-card` worktree、`feat/mcp-identity-card` 分支与隔离开发库。
