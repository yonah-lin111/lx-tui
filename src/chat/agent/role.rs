//! 子 agent 角色体系：底层安全权限基线（`BaseRole`）与动态角色标识（`AgentRole`）。
//!
//! 内置角色（`explorer` / `executor`）与用户自定义角色共用同一 [`AgentRole`] 类型；
//! 自定义角色名经 [`is_valid_role_name`] 校验后驻留于进程级去重静态池，从而保持
//! [`AgentRole`] 的 `Copy` 语义与 `&'static str` 零拷贝访问。

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use crate::config::subagents::{BaseRole, MAX_ROLE_NAME_LEN, is_valid_role_name};

use super::{EXECUTOR_PROMPT, EXPLORER_PROMPT, ToolSurface};

/// 子 agent 角色标识（内置 `Explorer` / `Executor` 或自定义角色）。
#[derive(Debug, Clone, Copy)]
pub enum AgentRole {
    /// 内置只读调查角色（`"explorer"`）。
    Explorer,
    /// 内置有边界实现角色（`"executor"`）。
    Executor,
    /// 用户自定义角色（名称满足 `[a-z][a-z0-9_]{0,31}`，绑定冻结的基础权限档位）。
    Custom { name: &'static str, base: BaseRole },
}

impl PartialEq for AgentRole {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for AgentRole {}

impl Hash for AgentRole {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl Serialize for AgentRole {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentRole {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid agent role '{text}'")))
    }
}

impl AgentRole {
    /// 两个内置角色（固定顺序）。
    pub const ALL: [AgentRole; 2] = [Self::Explorer, Self::Executor];

    /// 稳定角色名（如 `"explorer"`、`"executor"` 或自定义 `"reviewer"`）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Executor => "executor",
            Self::Custom { name, .. } => name,
        }
    }

    /// 底层安全权限档位。
    pub const fn base(self) -> BaseRole {
        match self {
            Self::Explorer => BaseRole::Explorer,
            Self::Executor => BaseRole::Executor,
            Self::Custom { base, .. } => base,
        }
    }

    /// 是否为内置角色（`explorer` / `executor`）。
    pub const fn is_builtin(self) -> bool {
        matches!(self, Self::Explorer | Self::Executor)
    }

    /// 用指定基础权限档位更新自定义角色（内置角色保持不变）。
    pub const fn with_base(self, base: BaseRole) -> Self {
        match self {
            Self::Explorer => Self::Explorer,
            Self::Executor => Self::Executor,
            Self::Custom { name, .. } => Self::Custom { name, base },
        }
    }

    /// 由角色名与可选基础权限档位构造角色；角色名语法非法返回 `None`。
    pub fn from_name_and_base(name: &str, base: Option<BaseRole>) -> Option<Self> {
        match name {
            "explorer" => Some(Self::Explorer),
            "executor" => Some(Self::Executor),
            other if is_valid_role_name(other) => Some(Self::Custom {
                name: intern_role_name(other),
                base: base.unwrap_or(BaseRole::Explorer),
            }),
            _ => None,
        }
    }

    /// 解析稳定角色名（内置或满足 `[a-z][a-z0-9_]{0,31}` 的自定义角色名）。
    pub fn parse(text: &str) -> Option<Self> {
        Self::from_name_and_base(text, None)
    }

    /// 解析仅限内置角色名（`"explorer"` / `"executor"`）。
    pub fn parse_builtin(text: &str) -> Option<Self> {
        match text {
            "explorer" => Some(Self::Explorer),
            "executor" => Some(Self::Executor),
            _ => None,
        }
    }

    /// 角色默认系统提示（英文；自定义角色未指定提示词时回退对应 `BaseRole` 提示词）。
    pub const fn system_prompt(self) -> &'static str {
        match self.base() {
            BaseRole::Explorer => EXPLORER_PROMPT,
            BaseRole::Executor => EXECUTOR_PROMPT,
        }
    }

    /// 该角色可用工具名白名单。
    pub const fn tools(self) -> &'static [&'static str] {
        ToolSurface::from_role(self).names()
    }
}

/// 将已通过 [`is_valid_role_name`] 校验的自定义角色名驻留到去重静态池。
fn intern_role_name(name: &str) -> &'static str {
    static POOL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut guard) = pool.lock() {
        if let Some(&existing) = guard.get(name) {
            return existing;
        }
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        guard.insert(leaked);
        leaked
    } else {
        Box::leak(name.to_string().into_boxed_str())
    }
}
