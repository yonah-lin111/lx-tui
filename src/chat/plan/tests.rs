//! 计划契约测试：全量替换、单进行中、清空、超限与 JSON 往返。

use super::*;
use serde_json::json;

/// 三个合法步骤（一进行中）——多数用例的基准输入。
fn arguments() -> Value {
    json!({
        "explanation": "Implementation finished; checking behavior.",
        "plan": [
            { "step": "Implement command tools", "status": "completed" },
            { "step": "Verify process cleanup", "status": "in_progress" },
            { "step": "Review terminal display", "status": "pending" }
        ]
    })
}

#[test]
fn valid_plan_is_normalized_in_order() {
    let (explanation, steps) = parse_plan(&arguments()).expect("valid plan");
    assert_eq!(
        explanation.as_deref(),
        Some("Implementation finished; checking behavior.")
    );
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].step, "Implement command tools");
    assert_eq!(steps[0].status, PlanStepStatus::Completed);
    assert_eq!(steps[1].status, PlanStepStatus::InProgress);
    assert_eq!(steps[2].status, PlanStepStatus::Pending);
}

#[test]
fn step_text_is_trimmed_and_blank_steps_are_rejected() {
    let (_, steps) = parse_plan(&json!({
        "plan": [{ "step": "  padded step  ", "status": "pending" }]
    }))
    .expect("valid");
    assert_eq!(steps[0].step, "padded step");

    for blank in ["", "   ", "\n\t"] {
        let error = parse_plan(&json!({
            "plan": [{ "step": blank, "status": "pending" }]
        }))
        .expect_err("blank step must fail");
        assert!(error.message.contains("non-empty"), "{}", error.message);
    }
}

#[test]
fn only_one_step_may_be_in_progress() {
    let error = parse_plan(&json!({
        "plan": [
            { "step": "a", "status": "in_progress" },
            { "step": "b", "status": "in_progress" }
        ]
    }))
    .expect_err("two in_progress must fail");
    assert!(error.message.contains("in_progress"), "{}", error.message);
}

#[test]
fn unknown_status_and_unknown_fields_are_rejected() {
    let status_error = parse_plan(&json!({
        "plan": [{ "step": "a", "status": "doing" }]
    }))
    .expect_err("unknown status");
    assert!(status_error.message.contains("pending"));

    let top_level = parse_plan(&json!({ "plan": [], "extra": true })).expect_err("unknown field");
    assert!(top_level.message.contains("extra"), "{}", top_level.message);

    let nested = parse_plan(&json!({
        "plan": [{ "step": "a", "status": "pending", "note": "x" }]
    }))
    .expect_err("unknown nested field");
    assert!(nested.message.contains("note"), "{}", nested.message);
}

#[test]
fn empty_plan_clears_and_is_not_a_completion() {
    let (explanation, steps) = parse_plan(&json!({ "plan": [] })).expect("empty plan is valid");
    assert!(explanation.is_none());
    assert!(steps.is_empty());
    let snapshot = PlanSnapshot {
        explanation,
        steps,
        revision: 4,
    };
    assert!(snapshot.is_empty());
    assert_eq!(snapshot.completed(), 0, "清空不等于全部完成");
}

#[test]
fn limits_are_enforced() {
    let too_many: Vec<Value> = (0..=MAX_PLAN_STEPS)
        .map(|index| json!({ "step": format!("step {index}"), "status": "pending" }))
        .collect();
    let error = parse_plan(&json!({ "plan": too_many })).expect_err("too many steps");
    assert!(error.message.contains(&MAX_PLAN_STEPS.to_string()));

    let long_step = "x".repeat(MAX_STEP_CHARS + 1);
    let error = parse_plan(&json!({
        "plan": [{ "step": long_step, "status": "pending" }]
    }))
    .expect_err("long step");
    assert!(error.message.contains(&MAX_STEP_CHARS.to_string()));

    let long_explanation = "y".repeat(MAX_EXPLANATION_CHARS + 1);
    let error = parse_plan(&json!({
        "explanation": long_explanation,
        "plan": []
    }))
    .expect_err("long explanation");
    assert!(error.message.contains(&MAX_EXPLANATION_CHARS.to_string()));
}

#[test]
fn missing_or_wrong_typed_plan_is_rejected() {
    assert!(parse_plan(&json!({})).is_err(), "plan is required");
    assert!(parse_plan(&json!({ "plan": null })).is_err());
    assert!(parse_plan(&json!({ "plan": "steps" })).is_err());
    assert!(parse_plan(&json!({ "plan": [], "explanation": 3 })).is_err());
}

#[test]
fn json_round_trip_preserves_snapshot() {
    let snapshot = PlanSnapshot {
        explanation: Some("note".into()),
        steps: vec![
            PlanStep {
                step: "a".into(),
                status: PlanStepStatus::Completed,
            },
            PlanStep {
                step: "b".into(),
                status: PlanStepStatus::InProgress,
            },
        ],
        revision: 7,
    };
    let json = snapshot.to_json();
    assert_eq!(json["tool"], "update_plan");
    let restored = PlanSnapshot::from_json(&json).expect("round trip");
    assert_eq!(restored, snapshot);
    assert_eq!(restored.completed(), 1, "只认 completed");
}

#[test]
fn foreign_json_is_not_mistaken_for_a_plan() {
    assert!(PlanSnapshot::from_json(&json!({ "tool": "read", "plan": [] })).is_none());
    assert!(PlanSnapshot::from_json(&json!({ "tool": "update_plan" })).is_none());
}
