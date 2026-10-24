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

/// 文件版本 token 的稳定前缀（内容与身份摘要；CAS 校验的唯一口径）。
pub const VERSION_PREFIX: &str = "lxv1-";

/// 一份镜像的内容/身份 token：类型、字节、权限与链接目标。
///
/// 用途是 CAS 前的期望值比较，**不是**加密摘要：用双 64 位 FNV-1a 降低碰撞面，
/// 不引入新依赖。同一字节序列必须永远得到同一 token（读写两侧共用本函数），
/// 因此 token 可以跨进程持久化（撤销/读取登记都依赖这一点）。
pub fn version_token(
    kind: RawKind,
    bytes: &[u8],
    mode: Option<u32>,
    link_target: Option<&Path>,
) -> String {
    let mut low: u64 = 0xcbf2_9ce4_8422_2325;
    let mut high: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut feed = |byte: u8| {
        low ^= u64::from(byte);
        low = low.wrapping_mul(0x0000_0100_0000_01b3);
        high ^= u64::from(byte).rotate_left(1);
        high = high.wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
    };
    let kind_byte = match kind {
        RawKind::Missing => 0,
        RawKind::Regular => 1,
        RawKind::Symlink => 2,
    };
    feed(kind_byte);
    for byte in mode.unwrap_or(u32::MAX).to_le_bytes() {
        feed(byte);
    }
    for byte in (bytes.len() as u64).to_le_bytes() {
        feed(byte);
    }
    for byte in bytes {
        feed(*byte);
    }
    if let Some(target) = link_target {
        for byte in crate::platform::encode_path(target).into_bytes() {
            feed(byte);
        }
    }
    format!("{VERSION_PREFIX}{low:016x}{high:016x}")
}

/// 镜像 → token。
pub fn version_token_of(image: &RawImage) -> String {
    version_token(
        image.kind,
        &image.bytes,
        image.mode,
        image.link_target.as_deref(),
    )
}

/// 由持久化字段（DB 稳定字符串 + 原始字节）重算 token。
///
/// `kind` 不是三个稳定值之一时返回 None（损坏记录不参与 CAS，直接判为不一致）。
pub fn version_token_from_parts(
    kind: &str,
    bytes: Option<&[u8]>,
    mode: Option<i64>,
    link_target: Option<&str>,
) -> Option<String> {
    let kind = match kind {
        "missing" => RawKind::Missing,
        "regular" => RawKind::Regular,
        "symlink" => RawKind::Symlink,
        _ => return None,
    };
    let mode = mode.and_then(|mode| u32::try_from(mode).ok());
    let link = link_target.map(PathBuf::from);
    Some(version_token(
        kind,
        bytes.unwrap_or(&[]),
        mode,
        link.as_deref(),
    ))
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
