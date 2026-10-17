//! 重试闸门单测：只有“零增量 + 零工具输入 + 临时故障”才允许重试。

use super::*;

// ------------------------------------------------------------------ 重试规则单测

/// 零增量 + 临时故障：可重试。
#[test]
fn can_retry_requires_no_delta_and_transient_error() {
    let reason = StopReason::Failed {
        code: "http 503".into(),
    };
    assert!(can_retry(&[], &reason));
}

/// 任何工具输入事件（Started/Delta/Ended）都会永久关闭重试。
#[test]
fn can_retry_is_closed_by_any_tool_input_event() {
    let reason = StopReason::Failed {
        code: "http 503".into(),
    };
    for delta in [
        ChatDelta::ToolInputStarted {
            call_id: "c".into(),
            name: "read".into(),
            ordinal: 0,
        },
        ChatDelta::ToolInputDelta {
            call_id: "c".into(),
            chunk: "{".into(),
        },
        ChatDelta::ToolInputEnded {
            call_id: "c".into(),
            arguments_json: "{}".into(),
        },
    ] {
        assert!(
            !can_retry(std::slice::from_ref(&delta), &reason),
            "工具输入后不得重试：{delta:?}"
        );
    }
}

/// 已产出正文或响应终态同样不重试（避免重复内容）。
#[test]
fn can_retry_is_closed_by_content_or_response_finished() {
    let reason = StopReason::Failed {
        code: "timeout".into(),
    };
    assert!(!can_retry(
        &[ChatDelta::Part(crate::chat::events::PartDelta {
            kind: PartKind::Text,
            text: "x".into(),
            metadata: None,
        })],
        &reason
    ));
    assert!(!can_retry(
        &[ChatDelta::ResponseFinished {
            stop: crate::chat::events::ModelStepStop::Final
        }],
        &reason
    ));
}

/// 非临时故障（4xx 能力错误）不重试。
#[test]
fn non_transient_error_is_not_retried() {
    assert!(!can_retry(
        &[],
        &StopReason::Failed {
            code: format!(
                "{} · http 400",
                crate::chat::transport::tools::TOOLS_UNSUPPORTED_CODE
            ),
        }
    ));
    assert!(!can_retry(&[], &StopReason::Cancelled));
}
