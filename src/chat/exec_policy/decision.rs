//! 规则决策及其严格度顺序（`Allow < Prompt < PromptUser < Forbidden`）。
//!
//! 移植自 OpenAI Codex `execpolicy/src/decision.rs`（Apache-2.0）。

/// 一条规则对命令的裁决。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Decision {
    /// 可直接运行，不套沙箱、不弹审批。
    Allow,
    /// 请求用户审批（`escalation = never` 时按拒绝处理）。
    Prompt,
    /// 必须由用户亲自决定（lx 与 `Prompt` 共用审批 strip）。
    PromptUser,
    /// 直接拒绝。
    Forbidden,
}

impl Decision {
    /// 解析 Starlark 字符串值。
    pub fn parse(raw: &str) -> Result<Self, super::error::Error> {
        match raw {
            "allow" => Ok(Self::Allow),
            "prompt" => Ok(Self::Prompt),
            "prompt-user" | "prompt_user" => Ok(Self::PromptUser),
            "forbidden" | "deny" => Ok(Self::Forbidden),
            other => Err(super::error::Error::InvalidDecision(other.to_string())),
        }
    }

    /// 稳定展示值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Prompt => "prompt",
            Self::PromptUser => "prompt-user",
            Self::Forbidden => "forbidden",
        }
    }

    /// 是否需要在执行前询问用户。
    pub const fn needs_approval(self) -> bool {
        matches!(self, Self::Prompt | Self::PromptUser)
    }
}
