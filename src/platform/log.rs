//! 日志目录推导：生产 `~/.lx-t/logs`，开发 `~/.lx-t-dev/logs`。
//!
//! `LX_TUI_LOG_DIR` 可显式覆盖；测试通过 `test_home_override` 隔离，
//! 避免并行测试写入真实用户目录。

use std::io;
use std::path::{Path, PathBuf};

/// 日志根目录。
pub fn log_dir() -> io::Result<PathBuf> {
    let override_dir = std::env::var_os("LX_TUI_LOG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    #[cfg(test)]
    if let Some(home) = super::test_home_override() {
        return Ok(resolve_log_dir(&home, true, override_dir.as_deref()));
    }
    let home = super::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory unavailable"))?;
    Ok(resolve_log_dir(
        &home,
        super::dev_environment(),
        override_dir.as_deref(),
    ))
}

/// 纯路径推导：显式覆盖优先，否则按环境根拼 `logs`。
pub(crate) fn resolve_log_dir(home: &Path, is_dev: bool, override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    let root = if is_dev { ".lx-t-dev" } else { ".lx-t" };
    home.join(root).join("logs")
}

#[cfg(test)]
mod tests;
