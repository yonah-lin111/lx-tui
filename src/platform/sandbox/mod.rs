//! 平台沙箱后端与共享根解析。OS 专属实现按 `#[cfg]` 门控在本模块内。

use std::path::Path;

pub mod roots;

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
