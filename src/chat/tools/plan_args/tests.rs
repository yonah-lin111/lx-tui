//! `update_plan` 参数入口测试：失败不产生任何结果、修订号递增、快照形状。

use super::*;
use crate::chat::plan::{MAX_PLAN_STEPS, PlanStepStatus};

#[test]
fn valid_arguments_produce_a_snapshot_with_revision() {
    let snapshot = build_plan_result(
        r#"{"explanation":"doing it","plan":[
             {"step":"a","status":"completed"},
             {"step":"b","status":"in_progress"}
           ]}"#,
        9,
    )
    .expect("valid");
    assert_eq!(snapshot.revision, 9);
    assert_eq!(snapshot.explanation.as_deref(), Some("doing it"));
    assert_eq!(snapshot.steps.len(), 2);
    assert_eq!(snapshot.steps[1].status, PlanStepStatus::InProgress);
    assert_eq!(snapshot.completed(), 1);
}

#[test]
fn invalid_arguments_fail_without_a_snapshot() {
    let error = build_plan_result(r#"{"plan":[{"step":"a","status":"nope"}]}"#, 1)
        .expect_err("invalid status");
    assert_eq!(error.code, ToolErrorCode::PlanInvalid);

    let too_many: Vec<String> = (0..=MAX_PLAN_STEPS)
        .map(|index| format!(r#"{{"step":"s{index}","status":"pending"}}"#))
        .collect();
    let json = format!(r#"{{"plan":[{}]}}"#, too_many.join(","));
    assert!(build_plan_result(&json, 1).is_err(), "超过步骤上限");
}

#[test]
fn non_json_arguments_are_rejected() {
    assert!(build_plan_result("not json", 1).is_err());
    assert!(build_plan_result(r#"[{"step":"a","status":"pending"}]"#, 1).is_err());
}

#[test]
fn empty_plan_is_a_valid_clear() {
    let snapshot = build_plan_result(r#"{"plan":[]}"#, 3).expect("clear");
    assert!(snapshot.is_empty());
    assert_eq!(snapshot.completed(), 0, "清空不等于完成");
}
