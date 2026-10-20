//! 计划快照：`update_plan` 的全量替换语义与规范化。
//!
//! 边界：本模块只有**纯数据 + 校验**，不读 DB、不碰 UI。
//!
//! 不变量（对应 design §6.3 / §9.3）：
//!
//! 1. 每次调用全量替换计划，不做文本匹配、不做隐式增删；
//! 2. 最多一个 `in_progress`；空数组是合法的「清空」，不等于把旧步骤全部完成；
//! 3. 未知状态、空白步骤、超长文本一律拒绝，**失败不产生任何副作用**；
//! 4. 结果携带完整规范化快照与修订标识，只有落库成功后它才成为当前计划。

use serde_json::Value;

/// 单次计划最多步骤数。
pub const MAX_PLAN_STEPS: usize = 32;
/// 单个步骤文本字符上限。
pub const MAX_STEP_CHARS: usize = 512;
/// 计划解释文本字符上限。
pub const MAX_EXPLANATION_CHARS: usize = 2048;

/// 计划步骤状态（wire 值是稳定英文小写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

impl PlanStepStatus {
    /// 稳定 wire 值（同时用于结果 JSON 与 DB 快照）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    /// 严格解析：未知值返回 None（调用方按参数非法拒绝）。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    /// 是否为已完成（完成计数只认它，`in_progress` 不算完成）。
    pub fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// 计划中的一个步骤：文本 + 状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub step: String,
    pub status: PlanStepStatus,
}

/// 规范化后的完整计划快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSnapshot {
    /// 可选解释（已限长）。
    pub explanation: Option<String>,
    /// 步骤（顺序即模型给出的顺序）。
    pub steps: Vec<PlanStep>,
    /// 修订标识：由运行实例在每次成功 update_plan 时单调递增分配。
    pub revision: u64,
}

impl PlanSnapshot {
    /// 已完成步骤数（只认 `completed`）。
    pub fn completed(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| step.status.is_completed())
            .count()
    }

    /// 是否为空计划（清空语义）。
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// 转成结果/落库共用的 JSON（唯一形状，避免多处拼装）。
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "tool": "update_plan",
            "explanation": self.explanation,
            "revision": self.revision,
            "plan": self
                .steps
                .iter()
                .map(|step| serde_json::json!({
                    "step": step.step,
                    "status": step.status.as_str(),
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// 从结果 JSON 还原快照（历史重载用）；形状不符返回 None。
    pub fn from_json(value: &Value) -> Option<Self> {
        if value.get("tool").and_then(Value::as_str) != Some("update_plan") {
            return None;
        }
        let steps = value.get("plan")?.as_array()?;
        let explanation = value
            .get("explanation")
            .and_then(Value::as_str)
            .map(str::to_string);
        let revision = value
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let mut parsed = Vec::with_capacity(steps.len());
        for step in steps {
            let text = step.get("step")?.as_str()?.to_string();
            let status = PlanStepStatus::parse(step.get("status")?.as_str()?)?;
            parsed.push(PlanStep { step: text, status });
        }
        Some(Self {
            explanation,
            steps: parsed,
            revision,
        })
    }
}

/// 计划参数校验失败的原因（稳定、可回给模型；不含参数全文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError {
    pub message: String,
}

impl PlanError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// 校验并规范化 `update_plan` 的参数对象。
///
/// 只接受 `explanation` / `plan`（顶层）与 `step` / `status`（步骤）四个字段；
/// 任何未知字段、类型错误、超限或非法状态都返回错误，且不产生部分结果。
pub fn parse_plan(arguments: &Value) -> Result<(Option<String>, Vec<PlanStep>), PlanError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| PlanError::new("arguments must be a JSON object"))?;
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "explanation" | "plan"))
    {
        return Err(PlanError::new(format!(
            "unknown argument '{unknown}'; allowed: explanation, plan"
        )));
    }
    // `plan` 必填：缺失与 null 都拒绝（清空必须显式给出空数组）。
    let steps = object
        .get("plan")
        .ok_or_else(|| PlanError::new("missing required argument 'plan'"))?
        .as_array()
        .ok_or_else(|| PlanError::new("'plan' must be an array"))?;
    if steps.len() > MAX_PLAN_STEPS {
        return Err(PlanError::new(format!(
            "'plan' may contain at most {MAX_PLAN_STEPS} steps"
        )));
    }
    let explanation = match object.get("explanation") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                if trimmed.chars().count() > MAX_EXPLANATION_CHARS {
                    return Err(PlanError::new(format!(
                        "'explanation' must be at most {MAX_EXPLANATION_CHARS} characters"
                    )));
                }
                Some(trimmed.to_string())
            }
        }
        Some(_) => return Err(PlanError::new("'explanation' must be a string")),
    };
    let mut parsed: Vec<PlanStep> = Vec::with_capacity(steps.len());
    let mut in_progress = 0usize;
    for (index, step) in steps.iter().enumerate() {
        let entry = step
            .as_object()
            .ok_or_else(|| PlanError::new(format!("plan[{index}] must be an object")))?;
        if let Some(unknown) = entry
            .keys()
            .find(|key| !matches!(key.as_str(), "step" | "status"))
        {
            return Err(PlanError::new(format!(
                "unknown field '{unknown}' in plan[{index}]; allowed: step, status"
            )));
        }
        let text = entry
            .get("step")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                PlanError::new(format!("plan[{index}].step must be a non-empty string"))
            })?;
        if text.chars().count() > MAX_STEP_CHARS {
            return Err(PlanError::new(format!(
                "plan[{index}].step must be at most {MAX_STEP_CHARS} characters"
            )));
        }
        let status = entry
            .get("status")
            .and_then(Value::as_str)
            .and_then(PlanStepStatus::parse)
            .ok_or_else(|| {
                PlanError::new(format!(
                    "plan[{index}].status must be one of pending, in_progress, completed"
                ))
            })?;
        if status == PlanStepStatus::InProgress {
            in_progress += 1;
        }
        parsed.push(PlanStep {
            step: text.to_string(),
            status,
        });
    }
    if in_progress > 1 {
        return Err(PlanError::new(
            "at most one step may be in_progress".to_string(),
        ));
    }
    Ok((explanation, parsed))
}

#[cfg(test)]
mod tests;
