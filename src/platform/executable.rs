//! PATH 可执行文件查找：Agent 安装探测的唯一 OS 分支落点。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 在 `PATH` 中按候选顺序查找第一个存在的可执行文件。
///
/// 返回命中的候选名（写入 shell 的命令）与绝对路径（调用方通常只用命令名）。
pub fn find_executable(names: &[&str]) -> Option<(String, PathBuf)> {
    let path = std::env::var_os("PATH")?;
    find_executable_in(names, &path)
}

/// 在给定的 `PATH` 值中查找；不读取进程环境，便于测试。
pub(crate) fn find_executable_in(names: &[&str], path: &OsStr) -> Option<(String, PathBuf)> {
    #[cfg(windows)]
    let extensions = windows_extensions();

    for dir in std::env::split_paths(path) {
        // 跳过空 PATH 段（如 `:/usr/bin`），避免退化成按 TUI 进程 CWD 查相对路径。
        if dir.as_os_str().is_empty() {
            continue;
        }
        for name in names {
            #[cfg(not(windows))]
            let hit = executable_candidate(&dir, name);
            #[cfg(windows)]
            let hit = executable_candidate(&dir, name, &extensions);
            if let Some(candidate) = hit {
                return Some(((*name).to_string(), candidate));
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn executable_candidate(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidate = dir.join(name);
    is_executable_file(&candidate).then_some(candidate)
}

#[cfg(not(windows))]
fn is_executable_file(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    if !path.metadata().is_ok_and(|meta| meta.is_file()) {
        return false;
    }
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c_path` 是以 NUL 结尾的合法 C 字符串，`libc::X_OK` 只读检验有效用户的执行权限。
    unsafe { libc::access(c_path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(windows)]
fn executable_candidate(dir: &Path, name: &str, extensions: &[String]) -> Option<PathBuf> {
    // 显式带扩展名的候选名按原样匹配；其余补 `PATHEXT`（缺省 .exe/.cmd/.bat/.com）。
    if Path::new(name).extension().is_some() {
        let candidate = dir.join(name);
        return candidate.is_file().then_some(candidate);
    }
    for suffix in extensions {
        let mut file = std::ffi::OsString::from(name);
        file.push(suffix);
        let candidate = dir.join(file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(windows)]
fn windows_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|ext| !ext.is_empty())
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        })
        .filter(|exts| !exts.is_empty())
        .unwrap_or_else(|| {
            [".exe", ".cmd", ".bat", ".com"]
                .iter()
                .map(|ext| (*ext).to_string())
                .collect()
        })
}

#[cfg(test)]
mod tests;
