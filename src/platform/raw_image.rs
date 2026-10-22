//! 原始文件镜像：按字节读取/写入普通文件与符号链接，保留权限；不做文本解码。
//!
//! 撤销快照必须对二进制、BOM、CRLF、无末尾换行的文件逐字节一致，因此这一层只认
//! `symlink_metadata` 与原始字节。目录、设备、socket 不是可快照对象，读取直接报错。

use std::io;
use std::path::{Path, PathBuf};

/// 文件镜像类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    /// 路径不存在。
    Missing,
    /// 普通文件。
    Regular,
    /// 符号链接（保存链接目标本身，不跟随）。
    Symlink,
}

/// 一份原始文件镜像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImage {
    pub kind: RawKind,
    /// 普通文件内容；其它类型为空。
    pub bytes: Vec<u8>,
    /// Unix 权限位（`0o777` 掩码）；非 Unix 或缺失为 None。
    pub mode: Option<u32>,
    /// 符号链接目标；其它类型为 None。
    pub link_target: Option<PathBuf>,
}

impl RawImage {
    /// 不存在的目标。
    pub fn missing() -> Self {
        Self {
            kind: RawKind::Missing,
            bytes: Vec::new(),
            mode: None,
            link_target: None,
        }
    }

    /// 普通文件镜像。
    pub fn regular(bytes: Vec<u8>, mode: Option<u32>) -> Self {
        Self {
            kind: RawKind::Regular,
            bytes,
            mode,
            link_target: None,
        }
    }

    /// 符号链接镜像。
    pub fn symlink(target: PathBuf) -> Self {
        Self {
            kind: RawKind::Symlink,
            bytes: Vec::new(),
            mode: None,
            link_target: Some(target),
        }
    }

    /// 快照占用字节数。
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// 内容同一性：类型、字节、链接目标、权限。
    pub fn same_content(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.bytes == other.bytes
            && self.link_target == other.link_target
            && self.mode == other.mode
    }
}

/// 读取镜像。
///
/// `follow` 为真时符号链接读取其物理目标（`write`/`update` 的真实写入对象）；
/// 为假时保存链接本身（`delete`/`move` 的目录项语义）。
pub fn read(path: &Path, follow: bool) -> io::Result<RawImage> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RawImage::missing());
        }
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        if follow {
            let metadata = std::fs::metadata(path)?;
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "symlink target is not a regular file",
                ));
            }
            let bytes = std::fs::read(path)?;
            return Ok(RawImage::regular(bytes, mode_of(&metadata)));
        }
        return Ok(RawImage::symlink(std::fs::read_link(path)?));
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target is not a regular file or symlink",
        ));
    }
    Ok(RawImage::regular(std::fs::read(path)?, mode_of(&metadata)))
}

/// 原子写入普通文件（同目录临时文件 + rename），并按镜像恢复权限。
pub fn write_regular(path: &Path, image: &RawImage) -> io::Result<()> {
    if image.kind != RawKind::Regular {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "write_regular requires a regular image",
        ));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    super::write_file_atomic(path, &image.bytes)?;
    if let Some(mode) = image.mode {
        set_mode(path, mode)?;
    }
    Ok(())
}

/// 重建符号链接：先移除同路径目录项，再创建链接。
pub fn write_symlink(path: &Path, target: &Path) -> io::Result<()> {
    remove_entry(path)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    symlink(target, path)
}

/// 删除单个文件或链接目录项；不存在视为成功，目录明确拒绝（不做递归删除）。
pub fn remove_entry(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to remove a directory",
        )),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 是否存在（含符号链接本身）。
pub fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// 普通文件的硬链接数；不存在、链接或不支持的系统返回 None。
///
/// 原地写入多硬链接对象会连带修改其他路径，撤销快照无法只恢复其中一个目录项；
/// 调用方据此保守拒绝。
pub fn hard_link_count(path: &Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    link_count(&metadata)
}

#[cfg(unix)]
fn link_count(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.nlink())
}

#[cfg(not(unix))]
fn link_count(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn mode_of(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(windows)]
fn symlink(target: &Path, path: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, path)
}

#[cfg(test)]
mod tests;
