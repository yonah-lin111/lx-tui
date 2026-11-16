//! 平台沙箱后端与共享根解析。OS 专属实现按 `#[cfg]` 门控在本模块内。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub mod roots;

#[cfg(target_os = "linux")]
pub mod bwrap;
#[cfg(target_os = "macos")]
pub mod seatbelt;

/// 包装一次命令所需的沙箱规格。
#[derive(Debug, Clone, Copy)]
pub struct SandboxSpec<'a> {
    /// 命令工作目录（已解析为绝对路径）。
    pub cwd: &'a Path,
    /// 可写根（已规范化）。
    pub writable_roots: &'a [PathBuf],
    /// 保护路径（只读）。
    pub protected: &'a [PathBuf],
    /// 是否允许网络。
    pub network_access: bool,
}

/// 包装后的程序与参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    /// 实际启动的程序（`sandbox-exec` / `bwrap`）。
    pub program: PathBuf,
    /// 完整参数（含原程序与参数）。
    pub args: Vec<OsString>,
}

/// 沙箱包装失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// 平台后端不可用（缺失二进制或无法创建 namespace）。
    Unavailable(String),
    /// 规格非法（相对路径、控制字符等）。
    Invalid(String),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "sandbox unavailable: {message}"),
            Self::Invalid(message) => write!(formatter, "invalid sandbox spec: {message}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// 按平台包装命令；不支持的平台返回 `Unavailable`（调用方 fail-closed）。
pub fn wrap(
    spec: &SandboxSpec<'_>,
    program: &OsStr,
    args: &[OsString],
) -> Result<Wrapped, SandboxError> {
    #[cfg(target_os = "macos")]
    {
        seatbelt::wrap(spec, program, args)
    }
    #[cfg(target_os = "linux")]
    {
        bwrap::wrap(spec, program, args)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (spec, program, args);
        Err(SandboxError::Unavailable(
            "process sandbox is not implemented on this platform".into(),
        ))
    }
}

/// 当前平台的沙箱能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxSupport {
    /// macOS seatbelt。
    Seatbelt,
    /// Linux bubblewrap。
    Bubblewrap,
    /// 仅内置文件工具受模式约束，shell 直通（Windows）。
    FileToolsOnly,
    /// 受限档不可用（缺失后端），执行期 fail-closed。
    Unavailable,
}

impl SandboxSupport {
    /// 稳定展示值（设置页、环境快照与提示词共用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seatbelt => "seatbelt",
            Self::Bubblewrap => "bubblewrap",
            Self::FileToolsOnly => "file-tools-only",
            Self::Unavailable => "unavailable",
        }
    }

    /// 该平台是否支持进程级沙箱。
    pub const fn supports_process_sandbox(self) -> bool {
        matches!(self, Self::Seatbelt | Self::Bubblewrap)
    }
}

/// 探测当前平台的沙箱能力（不启动子进程）。
pub fn support() -> SandboxSupport {
    #[cfg(target_os = "macos")]
    {
        if Path::new("/usr/bin/sandbox-exec").exists() {
            SandboxSupport::Seatbelt
        } else {
            SandboxSupport::Unavailable
        }
    }
    #[cfg(target_os = "linux")]
    {
        if command_on_path("bwrap") {
            SandboxSupport::Bubblewrap
        } else {
            SandboxSupport::Unavailable
        }
    }
    #[cfg(target_os = "windows")]
    {
        SandboxSupport::FileToolsOnly
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        SandboxSupport::Unavailable
    }
}

#[cfg(target_os = "linux")]
fn command_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .any(|candidate| candidate.is_file())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_matches_platform() {
        let support = support();
        #[cfg(target_os = "macos")]
        assert!(matches!(
            support,
            SandboxSupport::Seatbelt | SandboxSupport::Unavailable
        ));
        #[cfg(target_os = "linux")]
        assert!(matches!(
            support,
            SandboxSupport::Bubblewrap | SandboxSupport::Unavailable
        ));
        #[cfg(target_os = "windows")]
        assert_eq!(support, SandboxSupport::FileToolsOnly);
        assert_eq!(
            support.supports_process_sandbox(),
            matches!(
                support,
                SandboxSupport::Seatbelt | SandboxSupport::Bubblewrap
            )
        );
    }
}
