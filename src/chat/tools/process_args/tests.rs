//! 进程工具参数校验测试：白名单、类型、范围、互斥与“校验先于副作用”。

use super::*;
use crate::chat::process::{
    DEFAULT_STDIN_YIELD_MS, DEFAULT_YIELD_MS, MAX_OUTPUT_TOKENS, MAX_STDIN_BYTES, MAX_TIMEOUT_MS,
    MAX_YIELD_MS,
};

/// 取失败结果里的稳定错误码（便于断言“是校验失败而不是执行失败”）。
fn failure_code(result: ToolExecutionResult) -> crate::chat::tools::ToolErrorCode {
    match result {
        ToolExecutionResult::Failed { code, .. } => code,
        other => panic!("expected failure, got {other:?}"),
    }
}

#[test]
fn exec_minimal_command_gets_defaults() {
    let args = parse_exec_args(r#"{"cmd":"echo hi"}"#).expect("valid");
    assert_eq!(args.cmd, "echo hi");
    assert_eq!(args.workdir, None);
    assert_eq!(args.shell, None);
    assert!(args.login, "默认登录 shell（对齐 codex）");
    assert!(!args.tty);
    assert_eq!(args.yield_time_ms, DEFAULT_YIELD_MS);
    assert_eq!(args.timeout_ms, None, "默认无总寿命上限");
    assert_eq!(args.max_output_tokens, MAX_OUTPUT_TOKENS);
}

#[test]
fn exec_accepts_all_documented_fields() {
    let args = parse_exec_args(
        r#"{"cmd":"ls","workdir":"sub","shell":"/bin/bash","login":true,"tty":true,
            "yield_time_ms":0,"timeout_ms":1000,"max_output_tokens":16}"#,
    )
    .expect("valid");
    assert_eq!(args.workdir.as_deref(), Some("sub"));
    assert_eq!(args.shell.as_deref(), Some("/bin/bash"));
    assert!(args.login);
    assert!(args.tty);
    assert_eq!(args.yield_time_ms, 0, "0 是合法的等待窗口");
    assert_eq!(args.timeout_ms, Some(1000));
    assert_eq!(args.max_output_tokens, 16);
}

#[test]
fn exec_rejects_blank_and_missing_command() {
    assert!(parse_exec_args(r#"{"cmd":"   "}"#).is_err(), "空白命令拒绝");
    assert!(parse_exec_args(r#"{}"#).is_err(), "cmd 必填");
    assert!(parse_exec_args(r#"{"cmd":42}"#).is_err(), "类型错误拒绝");
    assert!(parse_exec_args("not json").is_err());
    assert!(parse_exec_args("[1,2]").is_err(), "必须是对象");
}

#[test]
fn exec_rejects_unknown_fields() {
    let error = parse_exec_args(r#"{"cmd":"ls","env":{"A":"1"}}"#).expect_err("unknown field");
    assert!(error.message.contains("env"), "{}", error.message);
}

#[test]
fn exec_rejects_out_of_range_values_instead_of_clamping() {
    // 越界一律拒绝：不静默修正成上限值。
    for key in ["yield_time_ms", "timeout_ms", "max_output_tokens"] {
        let too_big = format!(r#"{{"cmd":"ls","{key}":99999999}}"#);
        assert!(parse_exec_args(&too_big).is_err(), "{key} 超上限必须被拒绝");
    }
    assert!(parse_exec_args(r#"{"cmd":"ls","yield_time_ms":-1}"#).is_err());
    assert!(
        parse_exec_args(r#"{"cmd":"ls","timeout_ms":0}"#).is_err(),
        "时长为 0 无意义"
    );
    assert!(parse_exec_args(r#"{"cmd":"ls","max_output_tokens":0}"#).is_err());
    assert!(parse_exec_args(r#"{"cmd":"ls","yield_time_ms":1.5}"#).is_err());
}

#[test]
fn exec_boundaries_are_accepted() {
    let max = format!(
        r#"{{"cmd":"ls","yield_time_ms":{MAX_YIELD_MS},"timeout_ms":{MAX_TIMEOUT_MS},"max_output_tokens":{MAX_OUTPUT_TOKENS}}}"#
    );
    let args = parse_exec_args(&max).expect("boundaries valid");
    assert_eq!(args.yield_time_ms, MAX_YIELD_MS);
    assert_eq!(args.timeout_ms, Some(MAX_TIMEOUT_MS));
    assert_eq!(args.max_output_tokens, MAX_OUTPUT_TOKENS);
}

#[test]
fn stdin_defaults_and_requires_session_id() {
    let args = parse_stdin_args(r#"{"session_id":7}"#).expect("valid");
    assert_eq!(args.session_id, 7);
    assert_eq!(args.chars, "", "空写 = 只读输出");
    assert!(!args.terminate);
    assert_eq!(
        args.yield_time_ms, DEFAULT_STDIN_YIELD_MS,
        "交互写默认快速返回（对齐 codex 的 250ms）"
    );
    assert_ne!(
        DEFAULT_STDIN_YIELD_MS, DEFAULT_YIELD_MS,
        "首轮命令与交互的默认等待窗口刻意不同"
    );

    assert!(parse_stdin_args(r#"{}"#).is_err(), "session_id 必填");
    assert!(
        parse_stdin_args(r#"{"session_id":0}"#).is_err(),
        "0 不是合法 ID"
    );
    assert!(parse_stdin_args(r#"{"session_id":"7"}"#).is_err());
}

#[test]
fn stdin_terminate_cannot_be_combined_with_chars() {
    let error = parse_stdin_args(r#"{"session_id":1,"terminate":true,"chars":"x"}"#)
        .expect_err("mutually exclusive");
    assert!(error.message.contains("terminate"), "{}", error.message);
    // 空 chars 与 terminate 可以共存（显式停止）。
    let args = parse_stdin_args(r#"{"session_id":1,"terminate":true,"chars":""}"#).expect("valid");
    assert!(args.terminate);
}

#[test]
fn stdin_enforces_the_byte_limit() {
    let too_long = "x".repeat(MAX_STDIN_BYTES + 1);
    let json = serde_json::json!({ "session_id": 1, "chars": too_long }).to_string();
    let error = parse_stdin_args(&json).expect_err("too large");
    assert_eq!(error.code, crate::chat::tools::ToolErrorCode::TooLarge);
}

#[test]
fn stdin_rejects_unknown_fields() {
    let error = parse_stdin_args(r#"{"session_id":1,"input":"x"}"#).expect_err("unknown");
    assert!(error.message.contains("input"), "{}", error.message);
}

#[test]
fn process_failures_map_to_stable_tool_codes() {
    let cases = [
        (
            "resource_limit",
            crate::chat::tools::ToolErrorCode::ResourceLimit,
        ),
        (
            "process_not_found",
            crate::chat::tools::ToolErrorCode::ProcessNotFound,
        ),
        (
            "process_expired",
            crate::chat::tools::ToolErrorCode::ProcessExpired,
        ),
        (
            "stdin_failed",
            crate::chat::tools::ToolErrorCode::StdinFailed,
        ),
        (
            "spawn_failed",
            crate::chat::tools::ToolErrorCode::ProcessFailed,
        ),
    ];
    for (code, expected) in cases {
        let result = process_failure(crate::chat::process::ProcessError {
            code,
            message: "boom".into(),
        });
        assert_eq!(failure_code(result), expected, "code {code}");
    }
}
