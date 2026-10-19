//! `update_plan` 的参数校验与结果构造（纯函数，无副作用）。
//!
//! 校验失败时**不提交任何计划**：调用方拿到 `Failed` 结果，当前计划保持不变。

use crate::chat::plan::{PlanSnapshot, parse_plan};

use super::types::{ToolErrorCode, ToolFailure, ToolResultPublic};

/// 校验 `update_plan` 参数并生成带修订号的快照。
///
/// `next_revision` 由运行实例单调分配；只有 `FinishTool` 落库成功后该快照才成为当前计划。
pub fn build_plan_result(
    arguments_json: &str,
    next_revision: u64,
) -> ToolResultPublic<PlanSnapshot> {
    let value: serde_json::Value = serde_json::from_str(arguments_json.trim())
        .map_err(|error| ToolFailure::invalid(format!("arguments are not valid JSON: {error}")))?;
    let (explanation, steps) = parse_plan(&value)
        .map_err(|error| ToolFailure::new(ToolErrorCode::PlanInvalid, error.message))?;
    Ok(PlanSnapshot {
        explanation,
        steps,
        revision: next_revision,
    })
}

#[cfg(test)]
mod tests;
