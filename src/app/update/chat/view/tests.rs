//! DB 记录 → 步骤时间线投影的纯测试：跨表按 position 合并、顺序唯一、中断工具可见。

use super::*;
use crate::app::state::{AppState, ProcessStateView};
use crate::app::update::chat::reconcile_process_records;
use crate::db::chat::{ChatMessage, ChatPart, ProcessRecord, ToolCallRecord};

fn part(position: i64, kind: &str, content: &str) -> ChatPart {
    ChatPart {
        position,
        kind: kind.into(),
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn tool(position: i64, call_id: &str, name: &str, status: &str) -> ToolCallRecord {
    ToolCallRecord {
        call_id: call_id.into(),
        message_id: "a1".into(),
        name: name.into(),
        arguments_json: r#"{"path":"src/a.rs"}"#.into(),
        position,
        status: status.into(),
        result_json: None,
        error_text: None,
        started_at_ms: 100,
        completed_at_ms: Some(200),
        duration_ms: Some(100),
    }
}

fn message(parts: Vec<ChatPart>, tools: Vec<ToolCallRecord>) -> ChatMessage {
    ChatMessage {
        external_id: "a1".into(),
        sequence: 1,
        turn_index: 0,
        role: "assistant".into(),
        status: "complete".into(),
        model_provider: None,
        model_id: None,
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 0,
        completed_at_ms: None,
        parts,
        tools,
    }
}

/// 交错 position 的 parts 与 tools 必须按 position 合并成唯一步骤序列。
#[test]
fn parts_and_tools_merge_by_position() {
    let models = ModelsSnapshot::default();
    let db = message(
        vec![
            part(0, "reasoning", "想"),
            part(2, "text", "答"),
            part(4, "text", "续答"),
        ],
        vec![
            // 故意乱序给出（DB 查询按 position 有序，这里验证投影不依赖入参顺序）。
            tool(3, "c2", "search", "completed"),
            tool(1, "c1", "read", "completed"),
        ],
    );
    let view = view_message_of(&db, &models);
    let positions: Vec<i64> = view.steps.iter().map(|step| step.position()).collect();
    assert_eq!(positions, vec![0, 1, 2, 3, 4], "唯一时间线顺序");
    assert_eq!(view.steps.len(), 5);
    assert_eq!(view.reasoning_parts().count(), 1);
    assert_eq!(view.text_parts().count(), 2);
    assert_eq!(view.tools().count(), 2);
    assert_eq!(view.tool("c1").map(|call| call.position), Some(1));
    assert_eq!(
        view.tool("c1").map(|call| call.status),
        Some(ToolStatus::Completed)
    );
}

/// `interrupted` 工具在历史中可见（不隐藏、不伪装完成）。
#[test]
fn interrupted_tool_is_visible_with_reason() {
    let models = ModelsSnapshot::default();
    let mut record = tool(1, "c1", "write", "interrupted");
    record.error_text = Some(crate::db::chat::INTERRUPTED_ERROR_CODE.into());
    let view = view_message_of(&message(vec![part(0, "text", "答")], vec![record]), &models);
    let call = view.tool("c1").expect("可见");
    assert_eq!(call.status, ToolStatus::Interrupted);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::db::chat::INTERRUPTED_ERROR_CODE)
    );
    assert_eq!(
        call.summary
            .as_ref()
            .and_then(|summary| summary.path.as_deref()),
        Some("src/a.rs"),
        "摘要只装路径，不装结果体"
    );
}

/// 未知状态按 interrupted 保守处理（不伪装成功）。
#[test]
fn unknown_status_falls_back_to_interrupted() {
    let models = ModelsSnapshot::default();
    let record = tool(0, "c1", "read", "some-new-status");
    let view = view_message_of(&message(Vec::new(), vec![record]), &models);
    assert_eq!(
        view.tool("c1").map(|call| call.status),
        Some(ToolStatus::Interrupted)
    );
}

/// 旧纯文本会话（无工具）投影等价于原来的 parts 顺序（回归）。
#[test]
fn plain_text_history_matches_legacy_order() {
    let models = ModelsSnapshot::default();
    let db = message(
        vec![part(0, "reasoning", "旧思考"), part(1, "text", "旧正文")],
        Vec::new(),
    );
    let view = view_message_of(&db, &models);
    assert_eq!(
        view.parts()
            .map(|part| (part.kind_str(), part.content.as_str()))
            .collect::<Vec<_>>(),
        vec![("reasoning", "旧思考"), ("text", "旧正文")]
    );
    assert!(view.tools().next().is_none());
    assert_eq!(view.text(), "旧正文");
}

// ===== 重进会话：工具详情必须从结果体补全（回归：写文件类操作信息消失）=====

/// 重载后的工具视图：参数派生摘要 + 结果体补全。
fn reloaded(name: &str, arguments: &str, result: &str) -> ToolCallView {
    let mut record = tool(1, "call-1", name, "completed");
    record.arguments_json = arguments.into();
    record.result_json = Some(result.into());
    let view = view_message_of(
        &message(Vec::new(), vec![record]),
        &ModelsSnapshot::default(),
    );
    let AssistantStepView::Tool { call, .. } = &view.steps[0] else {
        panic!("应为工具步骤");
    };
    call.clone()
}

/// 回归：`edit` 的增删计数只在结果体里，重载后必须还在
/// （旧 bug：重进 session 后 `Edit` 只剩方块与空 `└─`）。
#[test]
fn reload_keeps_edit_added_and_removed_counts() {
    let call = reloaded(
        "edit",
        r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** End Patch"}"#,
        r#"{"tool":"edit","changes":[{"path":"a.rs","added":12,"removed":4,"kind":"updated"}],"partial":false}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.added, Some(12), "新增计数必须保留");
    assert_eq!(summary.removed, Some(4), "删除计数必须保留");
}

/// 回归：`Edit.partial` 只在结果体里，重载后必须还在（UI 的 `· partial` 靠它）。
#[test]
fn reload_keeps_edit_partial_flag() {
    let call = reloaded(
        "edit",
        r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n-x\n*** End Patch"}"#,
        r#"{"tool":"edit","changes":[{"path":"a.rs","added":0,"removed":0,"kind":"updated"}],"partial":true}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert!(summary.partial, "partial 必须从结果体补回");
}

/// 回归：`write` 的增删计数同样来自结果体。
#[test]
fn reload_keeps_write_added_and_removed_counts() {
    let call = reloaded(
        "write",
        r#"{"path":"docs/notes.md","content":"a\nb"}"#,
        r#"{"tool":"write","path":"docs/notes.md","created":true,"added":18,"removed":0}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.added, Some(18));
    assert_eq!(summary.removed, Some(0), "0 删也要保留（表示整文件覆盖）");
}

/// 回归：`search` 的命中数来自结果体、pattern 来自参数，两者都要在。
#[test]
fn reload_keeps_search_pattern_and_match_count() {
    let call = reloaded(
        "search",
        r#"{"pattern":"测试","path":"tool_test.txt"}"#,
        r#"{"tool":"search","matches":[{"path":"tool_test.txt","line":1,"text":"x","line_truncated":false},{"path":"tool_test.txt","line":3,"text":"y","line_truncated":false},{"path":"tool_test.txt","line":4,"text":"z","line_truncated":false}],"truncated":false,"files_scanned":1}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.pattern.as_deref(), Some("测试"), "pattern 来自参数");
    assert_eq!(summary.matches, Some(3), "命中数来自结果体");
    assert_eq!(summary.path.as_deref(), Some("tool_test.txt"));
}

/// 回归：`read` 的实际行数来自结果体（参数里只有 offset/limit）。
#[test]
fn reload_keeps_read_total_lines() {
    let call = reloaded(
        "read",
        r#"{"path":"src/a.rs","offset":12,"limit":29}"#,
        r#"{"tool":"read","path":"src/a.rs","kind":"file","content":"x","start_line":12,"end_line":40,"next_offset":null,"truncated":false,"total_lines":1000}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.range, Some((12, 40)), "区间来自结果体");
    assert_eq!(summary.total_lines, Some(1000), "文件总行数来自结果体");
    assert_eq!(summary.read_lines, Some(29), "实际读到的行数来自结果体");
}

// ===== 实时 vs 重载：两条路径必须给出同一份摘要 =====

/// chat 层摘要 → app 层摘要的字段映射（与事件层 `view_summary` 同口径）。
///
/// 逐字段列出、**不写** `..Default::default()`：`ToolSummary` 新增字段时这里会编译
/// 失败，强制两份清单同步。app 层测试不直接依赖 `event`（单向依赖约束），事件层
/// 那份映射由 `event::chat::tests::view_summary_maps_every_field` 单独守住。
fn map_summary(summary: &crate::chat::tools::ToolSummary) -> ToolSummary {
    ToolSummary {
        path: summary.path.clone(),
        range: summary.range,
        added: summary.added,
        removed: summary.removed,
        matches: summary.matches,
        pattern: summary.pattern.clone(),
        read_lines: summary.read_lines,
        total_lines: summary.total_lines,
        truncated: summary.truncated,
        partial: summary.partial,
    }
}

/// 实时结算路径的摘要：参数派生 + 结果侧摘要合并（`settle_tool_result` 的口径）。
fn live_summary(arguments: &str, result: &crate::chat::tools::ToolExecutionResult) -> ToolSummary {
    let from_result = map_summary(&crate::chat::tools::to_summary(result));
    ToolSummary::from_arguments(arguments).absorb_summary(&from_result)
}

/// 重进会话路径的摘要：参数派生 + 落库结果体补全（`tool_view_of` 的口径）。
fn reload_summary(
    arguments: &str,
    result: &crate::chat::tools::ToolExecutionResult,
) -> ToolSummary {
    let json = crate::chat::tools::to_result_json(result);
    ToolSummary::from_arguments(arguments).absorb_result_json(json.as_deref())
}

/// 由结果体 JSON 直接断言重载摘要（不经过 `to_result_json`，用于锁定老数据形状）。
fn reload_from_json(args: &str, result_json: &str) -> ToolSummary {
    ToolSummary::from_arguments(args).absorb_result_json(Some(result_json))
}

/// P1-1：参数只有 `path` 时，区间与行数必须从结果体补全。
#[test]
fn reload_keeps_read_range_without_offset() {
    let summary = reload_from_json(
        r#"{"path":"src/a.rs"}"#,
        r#"{"tool":"read","path":"src/a.rs","kind":"file","content":"x","start_line":1,"end_line":40,"next_offset":null,"truncated":false,"total_lines":40}"#,
    );
    assert_eq!(summary.range, Some((1, 40)), "区间来自结果体");
    assert_eq!(summary.read_lines, Some(40), "实际读到的行数来自结果体");
}

/// P1-1：目录读取的条目数在结果体里叫 `total_lines`，必须映射成 `matches`。
#[test]
fn reload_keeps_directory_entry_count() {
    let summary = reload_from_json(
        r#"{"path":"src"}"#,
        r#"{"tool":"read","path":"/w/src","kind":"directory","content":"[file] a\n[file] b\n","start_line":1,"end_line":2,"next_offset":null,"truncated":false,"total_lines":2}"#,
    );
    assert_eq!(summary.matches, Some(2), "目录条目数映射成 matches");
    assert_eq!(summary.range, None, "目录读取没有行区间");
    assert_eq!(summary.read_lines, None, "目录读取没有「读到的行数」");
}

/// P2-6：只有 `offset` 时参数侧不推导区间，重载区间来自结果体、与实时一致。
#[test]
fn reload_with_offset_only_does_not_collapse_range() {
    let args = r#"{"path":"src/a.rs","offset":12}"#;
    let result = file_read_result("src/a.rs", 12, 1000, 1000);
    // 参数侧单独推导不出来（旧实现会塌成 `(12, 12)`）。
    assert_eq!(ToolSummary::from_arguments(args).range, None);
    let reloaded = reload_summary(args, &result);
    assert_eq!(reloaded.range, Some((12, 1000)), "区间来自结果体");
    assert_eq!(reloaded.read_lines, Some(989));
    assert_eq!(reloaded, live_summary(args, &result), "实时与重载必须一致");
}

/// P2-6：只有 `limit` 时（最常见的「读前 N 行」）同样由结果体给出区间。
#[test]
fn reload_with_limit_only_gets_range_from_result() {
    let args = r#"{"path":"src/a.rs","limit":29}"#;
    let result = file_read_result("src/a.rs", 1, 29, 40);
    assert_eq!(ToolSummary::from_arguments(args).range, None);
    let reloaded = reload_summary(args, &result);
    assert_eq!(reloaded.range, Some((1, 29)), "区间来自结果体");
    assert_eq!(reloaded.read_lines, Some(29));
    assert_eq!(reloaded, live_summary(args, &result), "实时与重载必须一致");
}

/// 构造一次文件读取结果（含 `content` 长度与行数无关的简化正文）。
fn file_read_result(
    path: &str,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
) -> crate::chat::tools::ToolExecutionResult {
    crate::chat::tools::ToolExecutionResult::Read {
        path: std::path::PathBuf::from(path),
        kind: crate::chat::tools::ReadKind::File,
        content: "1: a\n".into(),
        start_line,
        end_line,
        next_offset: (end_line < total_lines).then_some(end_line + 1),
        truncated: end_line < total_lines,
        total_lines,
    }
}

/// 构造一次目录读取结果。
fn directory_read_result(path: &str, entries: usize) -> crate::chat::tools::ToolExecutionResult {
    crate::chat::tools::ToolExecutionResult::Read {
        path: std::path::PathBuf::from(path),
        kind: crate::chat::tools::ReadKind::Directory,
        content: "[file] a\n".into(),
        start_line: 1,
        end_line: entries,
        next_offset: None,
        truncated: false,
        total_lines: entries,
    }
}

/// 表驱动锁死「实时摘要 == 重载摘要」：同一份参数 + 同一份结果，两条路径必须逐字段相等。
///
/// 覆盖 `read`（带/不带 offset+limit）、目录 `read`、`search`、单文件/多文件 `edit`、`write`。
/// 两条路径的差异只允许出现在**参数侧与结果侧都缺失**的字段上；本表里的 case 都已对齐。
#[test]
fn live_and_reloaded_summaries_are_identical() {
    use crate::chat::tools::{FileChange, FileChangeKind};

    let edit_patch = "*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** End Patch";
    let multi_patch = "*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** Update File: b.rs\n@@\n-z\n+w\n*** End Patch";
    // 参数用 serde_json 生成（patch 里有换行，手拼 JSON 会得到非法文本）。
    let patch_args = |patch: &str| serde_json::json!({ "patch": patch }).to_string();
    let cases: Vec<(&str, String, crate::chat::tools::ToolExecutionResult)> = vec![
        (
            "read 带 offset+limit",
            r#"{"path":"src/a.rs","offset":12,"limit":29}"#.into(),
            file_read_result("src/a.rs", 12, 40, 1000),
        ),
        (
            "read 不带 offset/limit",
            r#"{"path":"src/a.rs"}"#.into(),
            file_read_result("src/a.rs", 1, 40, 40),
        ),
        (
            "read 目录",
            r#"{"path":"src"}"#.into(),
            directory_read_result("/w/src", 2),
        ),
        (
            "search",
            r#"{"pattern":"测试","path":"tool_test.txt"}"#.into(),
            crate::chat::tools::ToolExecutionResult::Search {
                matches: vec![
                    crate::chat::tools::SearchMatch {
                        path: std::path::PathBuf::from("tool_test.txt"),
                        line: 1,
                        text: "x".into(),
                        line_truncated: false,
                    },
                    crate::chat::tools::SearchMatch {
                        path: std::path::PathBuf::from("tool_test.txt"),
                        line: 3,
                        text: "y".into(),
                        line_truncated: false,
                    },
                ],
                truncated: true,
                files_scanned: 1,
            },
        ),
        (
            "edit 单文件",
            patch_args(edit_patch),
            crate::chat::tools::ToolExecutionResult::Edit {
                changes: vec![FileChange {
                    path: std::path::PathBuf::from("a.rs"),
                    added: 12,
                    removed: 4,
                    kind: FileChangeKind::Updated,
                }],
                partial: false,
            },
        ),
        (
            "edit 多文件",
            patch_args(multi_patch),
            crate::chat::tools::ToolExecutionResult::Edit {
                changes: vec![
                    FileChange {
                        path: std::path::PathBuf::from("a.rs"),
                        added: 1,
                        removed: 1,
                        kind: FileChangeKind::Updated,
                    },
                    FileChange {
                        path: std::path::PathBuf::from("b.rs"),
                        added: 2,
                        removed: 2,
                        kind: FileChangeKind::Updated,
                    },
                ],
                partial: false,
            },
        ),
        (
            "edit 部分成功（partial）",
            patch_args(edit_patch),
            crate::chat::tools::ToolExecutionResult::Edit {
                changes: vec![FileChange {
                    path: std::path::PathBuf::from("a.rs"),
                    added: 0,
                    removed: 0,
                    kind: FileChangeKind::Updated,
                }],
                partial: true,
            },
        ),
        (
            "write",
            r#"{"path":"docs/notes.md","content":"a\nb"}"#.into(),
            crate::chat::tools::ToolExecutionResult::Write {
                path: std::path::PathBuf::from("docs/notes.md"),
                created: true,
                added: 18,
                removed: 0,
            },
        ),
    ];

    for (name, arguments, result) in cases {
        let live = live_summary(&arguments, &result);
        let reloaded = reload_summary(&arguments, &result);
        assert_eq!(live, reloaded, "{name}：实时摘要必须等于重载摘要");
    }
}

/// 结果体缺失/非法时退回纯参数摘要，不 panic 也不编造。
#[test]
fn reload_without_result_json_falls_back_to_arguments_only() {
    let record = {
        let mut record = tool(1, "call-1", "edit", "interrupted");
        record.arguments_json =
            r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n*** End Patch"}"#.into();
        record.result_json = None;
        record
    };
    let view = view_message_of(
        &message(Vec::new(), vec![record]),
        &ModelsSnapshot::default(),
    );
    let AssistantStepView::Tool { call, .. } = &view.steps[0] else {
        panic!("应为工具步骤");
    };
    let summary = call.summary.as_ref().expect("参数摘要");
    assert_eq!(summary.added, None, "没有结果体就没有增删计数");
    assert_eq!(summary.removed, None);

    // 非法结果体同样不 panic：仍然只按参数派生（patch 里有目标文件，所以有摘要）。
    let call = reloaded(
        "edit",
        r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n*** End Patch"}"#,
        "not json",
    );
    let summary = call.summary.expect("参数派生摘要");
    assert_eq!(
        summary.path.as_deref(),
        Some("a.rs"),
        "非法结果体时仍能从 patch 取到目标文件"
    );
    assert_eq!(summary.added, None, "非法结果体不编造计数");

    // 参数也没有可用字段时，摘要为空（不凭空造一个空摘要）。
    let call = reloaded("edit", "{}", "not json");
    assert!(call.summary.is_none(), "无任何可用字段时不造摘要");
}

// ===== 旧会话里的非零退出：按结果体重判，不照抄库里的 failed =====
// 详见 design §14.6：早期版本把"命令非零退出"判成工具失败，库里因此留下 failed 行。

/// 一条进程工具记录：`status` 是库里存的状态，`result_json` 是当时的真实结果体。
fn process_tool(status: &str, process_state: &str, exit_code: i32) -> ToolCallRecord {
    let mut record = tool(0, "call-1", "exec_command", status);
    record.result_json = Some(
        serde_json::json!({
            "tool": "exec_command",
            "session_id": 2,
            "command": "pgrep -af x; echo ---; ps -ef | grep -c y",
            "cwd": "/workspace",
            "tty": false,
            "process_state": process_state,
            "exit_code": exit_code,
            "output": "35293\n---\n0\n",
            "output_start": 0,
            "output_end": 12,
            "dropped_bytes": 0,
            "truncated": false,
            "wrote_input": false,
            "terminate": false,
        })
        .to_string(),
    );
    record.error_text = Some(format!("command exited with code {exit_code}"));
    record
}

/// 取唯一工具步骤。
fn only_tool(message: &ChatMessageView) -> &ToolCallView {
    let mut found = None;
    for step in &message.steps {
        if let AssistantStepView::Tool { call, .. } = step {
            found = Some(call);
        }
    }
    found.expect("tool step")
}

#[test]
fn replay_downgrades_legacy_nonzero_exit_failure_to_completed() {
    let models = ModelsSnapshot::default();
    // 库里存的是旧判定留下的 failed + "command exited with code 1"。
    let record = process_tool("failed", "exited", 1);
    let view = view_message_of(&message(Vec::new(), vec![record]), &models);
    let call = only_tool(&view);
    assert_eq!(
        call.status,
        ToolStatus::Completed,
        "非零退出按结果体重判为 completed，不能继续显示成红色失败"
    );
    assert_eq!(call.error, None, "旧记录的\"失败原因\"随之作废");
    // 退出码仍从结果体回放出来，标题照常显示 `exit 1`。
    let Some(ToolDetailView::Command(command)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert!(command.state.is_nonzero_exit());
}

#[test]
fn replay_keeps_genuine_process_failures_failed() {
    let models = ModelsSnapshot::default();
    // 超时是真正的失败：库里 failed 与结果体一致，必须保持红。
    let record = process_tool("failed", "timed_out", 124);
    let view = view_message_of(&message(Vec::new(), vec![record]), &models);
    let call = only_tool(&view);
    assert_eq!(call.status, ToolStatus::Failed, "超时仍为失败");
    assert!(call.error.is_some(), "失败原因保留");
}

#[test]
fn replay_keeps_completed_records_completed() {
    let models = ModelsSnapshot::default();
    let mut record = process_tool("completed", "exited", 0);
    record.error_text = None;
    let view = view_message_of(&message(Vec::new(), vec![record]), &models);
    assert_eq!(only_tool(&view).status, ToolStatus::Completed);
}

// ===== 历史回放：卡片状态按 `agent_process` 校正 =====
// 结果 JSON 记的是「结算那一刻」（通常是 running），进程之后可能早已退出；
// 只按结果回放会让历史卡片永远停在 `Running`（用户报的「关掉了还在计时」）。

/// 一条 `exec_command` 记录：结果体里写着 `running`（结算那一刻）。
fn running_command_tool(call_id: &str) -> ToolCallRecord {
    let mut record = tool(0, call_id, "exec_command", "completed");
    record.result_json = Some(
        serde_json::json!({
            "tool": "exec_command",
            "session_id": 1,
            "command": "while true; do read -r line; done",
            "cwd": "/workspace",
            "tty": true,
            "process_state": "running",
            "exit_code": null,
            "output": "",
            "output_start": 0,
            "output_end": 0,
            "dropped_bytes": 0,
            "truncated": false,
            "wrote_input": false,
            "terminate": false,
        })
        .to_string(),
    );
    record
}

/// 进程表里的一条终态记录。
fn process_record(origin_call_id: &str, state: &str, ended_at_ms: i64) -> ProcessRecord {
    ProcessRecord {
        external_id: "p1".into(),
        chat_session_id: "s1".into(),
        origin_call_id: origin_call_id.into(),
        runtime_epoch: "e1".into(),
        process_session_id: 1,
        state: state.into(),
        exit_code: None,
        stop_reason: Some("stopped".into()),
        command: "while true; do read -r line; done".into(),
        cwd: "/workspace".into(),
        tty: true,
        output_tail: String::new(),
        output_start: 0,
        output_end: 0,
        dropped_bytes: 0,
        started_at_ms: 1_000,
        ended_at_ms: Some(ended_at_ms),
        revision: 1,
    }
}

#[test]
fn replay_settles_a_running_card_from_the_process_record() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let models = ModelsSnapshot::default();
    let db = message(Vec::new(), vec![running_command_tool("call-1")]);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![view_message_of(&db, &models)];
    }
    let records = vec![process_record("call-1", "stopped", 9_999)];
    assert!(reconcile_process_records(&mut state, pane, &records));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(
        view.state,
        ProcessStateView::Stopped,
        "历史卡片必须收敛到进程表的终态，不能停在 Running"
    );
    // 时长取自进程记录：终态后不再随 now 增长。
    assert_eq!(view.started_at_ms, Some(1_000));
    assert_eq!(view.ended_at_ms, Some(9_999));
    // 幂等：再跑一次没有变化。
    assert!(!reconcile_process_records(&mut state, pane, &records));
}

#[test]
fn replay_keeps_running_cards_running() {
    // 进程表说还在跑：卡片就该继续显示 Running（不能反过来把活的判死）。
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let models = ModelsSnapshot::default();
    let db = message(Vec::new(), vec![running_command_tool("call-1")]);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![view_message_of(&db, &models)];
    }
    let mut record = process_record("call-1", "running", 9_999);
    record.ended_at_ms = None;
    // 第一次会补上起始时刻（历史卡片原本没有，补上后才显示「跑了多久」）。
    assert!(reconcile_process_records(
        &mut state,
        pane,
        &[record.clone()]
    ));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(
        view.state,
        ProcessStateView::Running,
        "活着的进程不许被判死"
    );
    assert_eq!(view.started_at_ms, Some(1_000), "补上起始时刻");
    assert_eq!(view.ended_at_ms, None, "没有结束时刻");
    // 幂等。
    assert!(!reconcile_process_records(&mut state, pane, &[record]));
}

#[test]
fn replay_without_process_records_leaves_the_card_alone() {
    // 库不可用/记录缺失时保持结果 JSON 原样，绝不凭空捏造终态。
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let models = ModelsSnapshot::default();
    let db = message(Vec::new(), vec![running_command_tool("call-1")]);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.messages = vec![view_message_of(&db, &models)];
    }
    assert!(!reconcile_process_records(&mut state, pane, &[]));
}
