//! 在系统文件管理器中定位路径、用默认浏览器打开 URL；OS 命令按 cfg 门控。
//!
//! 调用方须在后台线程执行（进程启动会短暂阻塞）。

use std::io;

/// 在系统文件管理器中定位并选中指定路径（macOS Finder `open -R` / Windows Explorer `/select,` / Linux 打开父目录）。
///
/// 调用方须在后台线程执行。
pub fn reveal_in_file_manager(path: &std::path::Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let path_str = path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid utf-8 path"))?;
        native_open("open", &["-R", path_str])
    }
    #[cfg(target_os = "linux")]
    {
        let target = if path.is_dir() {
            path
        } else {
            // 裸相对文件名（如 `a.png`）的 parent 是空路径，必须回退到当前目录，
            // 否则 `xdg-open ""` 直接失败。
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
        };
        let target_str = target
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid utf-8 path"))?;
        match native_open("xdg-open", &[target_str]) {
            Ok(()) => Ok(()),
            Err(error) => native_open("gio", &["open", target_str]).map_err(|_| error),
        }
    }
    #[cfg(windows)]
    {
        let select_arg = format!("/select,{}", path.display());
        native_open("explorer", &[&select_arg])
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "revealing a file in file manager is not supported on this platform",
        ))
    }
}

/// 在系统默认浏览器中打开 URL；OS 命令按 cfg 门控，失败返回错误。
///
/// 调用方须在后台线程执行（进程启动会短暂阻塞）；`open`/`xdg-open` 等命令
/// 本身立即返回，不等待浏览器窗口。
pub fn open_browser(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        native_open("open", &[url])
    }
    #[cfg(target_os = "linux")]
    {
        match native_open("xdg-open", &[url]) {
            Ok(()) => Ok(()),
            Err(error) => native_open("gio", &["open", url]).map_err(|_| error),
        }
    }
    #[cfg(windows)]
    {
        native_open("rundll32", &["url.dll,FileProtocolHandler", url])
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = url;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "opening a browser is not supported on this platform",
        ))
    }
}

/// 启动平台命令并立即返回；非零退出码视为失败。
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn native_open(program: &str, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} exited with status {status}"
        )))
    }
}
