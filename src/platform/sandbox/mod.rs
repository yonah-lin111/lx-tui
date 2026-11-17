//! 平台沙箱后端与共享根解析。OS 专属实现按 `#[cfg]` 门控在本模块内。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub mod glob;
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
    /// 写根内的只读保护路径（`.git` 等 + `read` carveout）。
    pub read_only_paths: &'a [PathBuf],
    /// 显式拒绝的字面路径（读写都禁止）。
    pub deny_paths: &'a [PathBuf],
    /// 显式拒绝的 glob（读写都禁止）。
    pub deny_globs: &'a [String],
    /// glob 扫描深度上限。
    pub glob_scan_max_depth: usize,
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
    /// `bwrap` 为不存在挂载目标占位的空文件路径；子进程退出后按需清理。
    pub synthetic_targets: Vec<PathBuf>,
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

/// 探测当前平台的沙箱能力（Linux 首次调用经 `bwrap` `OnceLock` 缓存真实能力探测）。
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
        if bwrap::is_available() {
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

/// 退出码是否表示平台沙箱系统调用拦截（如 Linux seccomp `SIGSYS`）。
pub const fn is_seccomp_exit_code(code: i32) -> bool {
    #[cfg(target_os = "linux")]
    {
        code == 128 + 31
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = code;
        false
    }
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
