//! 原子文件替换：同目录临时文件写完再 rename 覆盖目标；OS 差异在此按 cfg 门控。
//!
//! 目标已存在时尽量沿用其权限位（Unix），不存在时用系统默认创建权限。
//! 临时文件名含进程 ID 与进程内序号，并发写同一目标不会互相覆盖临时文件。

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内临时文件序号；只用于生成唯一临时名，不含业务含义。
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 原子写入整个文件：先写同目录临时文件（并 sync），再替换目标。
///
/// 失败时清理临时文件并返回原始错误；调用方负责按需创建父目录。
pub fn write_file_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temp = temp_path(path);
    write_synced(&temp, contents)?;
    // 权限沿用：尽力而为，失败不致命（新文件用系统默认权限即可）。
    if let Ok(metadata) = std::fs::metadata(path)
        && let Err(error) = std::fs::set_permissions(&temp, metadata.permissions())
    {
        tracing::debug!(error = %error, "atomic write: copy permissions failed");
    }
    if let Err(error) = rename_over(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

/// 用 `from` 替换 `to`；跨平台差异集中在 [`rename_over`]。
pub fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::rename(from, to)
    }
    #[cfg(not(unix))]
    {
        rename_over_fallback(from, to)
    }
}

/// Windows 下 `rename` 不能覆盖已存在目标：先删除目标再改名。
///
/// 这不是严格原子操作，属于平台写法的已知退化；调用方仍能获得正确的
/// 最终内容或明确的 IO 错误。
#[cfg(not(unix))]
fn rename_over_fallback(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) => {
            if !to.exists() {
                return Err(error);
            }
            std::fs::remove_file(to)?;
            std::fs::rename(from, to)
        }
    }
}

/// 写临时文件并落盘；创建失败或写失败时清理残留文件。
fn write_synced(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    if let Err(error) = written {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

/// 同目录临时文件名：`<name>.tmp.<pid>.<seq>`，不跨目录以便 rename 生效。
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.tmp.{}.{sequence}", std::process::id()))
}
