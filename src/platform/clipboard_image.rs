//! 本地图像处理与会话图片归档：尺寸校验、等比缩放、重编码与
//! `~/.lx-t[-dev]/session/<session_id>/image/` 持久化。
//!
//! 仅支持本地图片文件（不读取系统剪贴板位图）；发送与工具结果归档到会话目录，
//! 防止桌面等外部原图被清理后无法 `rewind` 或历史回放。自动清理周期对齐 Claude Code（默认 30 天）。

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use image::imageops::FilterType;
use image::{ImageDecoder, ImageFormat, ImageReader};

/// 单张原图大小上限（20 MiB）。
pub const MAX_IMAGE_RAW_BYTES: usize = 20 * 1024 * 1024;
/// 单条消息图片数量上限（4 张）。
pub const MAX_ATTACHED_IMAGES: usize = 4;
/// 单条消息原图总量上限（40 MiB）。
pub const MAX_TOTAL_IMAGE_RAW_BYTES: usize = 40 * 1024 * 1024;
/// 发送前最大最长边（像素）。
pub const MAX_IMAGE_DIMENSION: u32 = 2048;
/// 解码像素上限（防止高压缩小文件解压耗尽内存）。
pub const MAX_DECODE_PIXELS: u64 = 32_000_000;
/// 未被任何会话引用的归档图片默认保留时长（30 天，对齐 Claude Code `cleanupPeriodDays`）。
pub const IMAGE_RETENTION_SECS: u64 = 30 * 24 * 3600;

/// 图像准备过程中的错误枚举（稳定错误码，不含敏感路径与私密数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImagePrepareError {
    FileNotFound,
    NotAnImage,
    SingleTooLarge,
    TooManyImages,
    TotalTooLarge,
    DecodeFailed,
    EncodeFailed,
    DimensionTooLarge,
    ClipboardUnavailable,
    ImageUnsupportedByModel,
    IoError,
}

impl ImagePrepareError {
    /// 稳定错误代码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::FileNotFound => "file_not_found",
            Self::NotAnImage => "not_an_image",
            Self::SingleTooLarge => "single_too_large",
            Self::TooManyImages => "too_many_images",
            Self::TotalTooLarge => "total_too_large",
            Self::DecodeFailed => "decode_failed",
            Self::EncodeFailed => "encode_failed",
            Self::DimensionTooLarge => "dimension_too_large",
            Self::ClipboardUnavailable => "clipboard_unavailable",
            Self::ImageUnsupportedByModel => "image_unsupported_by_model",
            Self::IoError => "io_error",
        }
    }
}

impl std::fmt::Display for ImagePrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())
    }
}

impl std::error::Error for ImagePrepareError {}

/// 已处理好、可供存储与模型传输的图片对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedImage {
    /// 原始展示文件名或标识。
    pub source_name: String,
    /// 原始文件路径。
    pub source_path: Option<PathBuf>,
    /// 最终编码 MIME（`image/png` 或 `image/jpeg`）。
    pub mime: String,
    /// 最终宽度。
    pub width: u32,
    /// 最终高度。
    pub height: u32,
    /// 最终编码字节（PNG 或 JPEG）。
    pub bytes: Vec<u8>,
    /// 原始字节数（计入总预算）。
    pub original_byte_count: usize,
}

/// 严格归档后的会话图片元数据（已写入受管目录并通过回读摘要校验）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedSessionImage {
    /// 展示标签（不含 base64 正文）。
    pub label: String,
    /// 受管归档文件的绝对路径（位于 `<images_root>/<session_id>/` 内）。
    pub archive_path: PathBuf,
    /// 最终编码 MIME（`image/png` 或 `image/jpeg`）。
    pub mime: String,
    /// 最终编码字节的 16 位十六进制 FNV-1a 摘要。
    pub content_hash: String,
    /// 宽度（像素）。
    pub width: u32,
    /// 高度（像素）。
    pub height: u32,
    /// 最终编码字节（供立即启动的子轮直接使用）。
    pub bytes: Vec<u8>,
}

/// 会话数据根目录：生产 `~/.lx-t/session`，开发 `~/.lx-t-dev/session`。
pub fn sessions_root_dir() -> Result<PathBuf, ImagePrepareError> {
    #[cfg(test)]
    if let Some(home) = super::test_home_override() {
        return Ok(home.join(".lx-t-dev").join("session"));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or(ImagePrepareError::IoError)?;
    let root = if super::dev_environment() {
        ".lx-t-dev"
    } else {
        ".lx-t"
    };
    Ok(home.join(root).join("session"))
}

/// 单个会话的图片目录：`<sessions_root>/<session_id>/image/`。
pub fn session_image_dir(sessions_root: &Path, session_id: &str) -> PathBuf {
    sessions_root.join(session_id).join("image")
}

/// FNV-1a 64 位哈希（用于同会话内图片内容去重命名）。
fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 清洗文件名中的非法路径字符。
fn sanitize_filename(name: &str, mime: &str) -> String {
    let ext = if mime == "image/jpeg" { "jpg" } else { "png" };
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let clean_stem: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let final_stem = if clean_stem.is_empty() {
        "image"
    } else {
        clean_stem.as_str()
    };
    format!("{final_stem}.{ext}")
}

/// 将本地图片处理后归档到 `<sessions_root>/<session_id>/image/<hash>-<filename>`；
/// 若已存在则复用并刷新 mtime。
pub fn archive_session_image_in(
    sessions_root: &Path,
    session_id: &str,
    source_path: &Path,
) -> Result<PathBuf, ImagePrepareError> {
    let session_dir = session_image_dir(sessions_root, session_id);
    if source_path.starts_with(&session_dir) && source_path.is_file() {
        touch_file_mtime(source_path);
        return Ok(source_path.to_path_buf());
    }

    let processed = load_and_process_image_file(source_path)?;
    std::fs::create_dir_all(&session_dir).map_err(|_| ImagePrepareError::IoError)?;
    let _ = super::restrict_dir_permissions(sessions_root);
    let _ = super::restrict_dir_permissions(&session_dir);

    let hash_hex = format!("{:016x}", fnv1a_hash(&processed.bytes));
    let safe_name = sanitize_filename(&processed.source_name, &processed.mime);
    let target = session_dir.join(format!("{hash_hex}-{safe_name}"));
    if target.is_file() {
        touch_file_mtime(&target);
        return Ok(target);
    }

    super::write_file_atomic(&target, &processed.bytes).map_err(|_| ImagePrepareError::IoError)?;
    let _ = super::restrict_file_permissions(&target);
    Ok(target)
}

/// 将本地图片归档到默认会话图片目录。
pub fn archive_session_image(
    session_id: &str,
    source_path: &Path,
) -> Result<PathBuf, ImagePrepareError> {
    let root = sessions_root_dir()?;
    archive_session_image_in(&root, session_id, source_path)
}

/// 把源会话的图片目录复制到目标会话（fork 用）。
///
/// 源目录不存在或没有可复制文件时返回 `Ok(false)`；目标目录按需创建。
pub fn fork_session_images(
    source_session: &str,
    target_session: &str,
) -> Result<bool, ImagePrepareError> {
    let root = sessions_root_dir()?;
    fork_session_images_in(&root, source_session, target_session)
}

/// 显式根版本的会话图片复制；供测试使用。
pub fn fork_session_images_in(
    sessions_root: &Path,
    source_session: &str,
    target_session: &str,
) -> Result<bool, ImagePrepareError> {
    if !valid_session_id(source_session)
        || !valid_session_id(target_session)
        || source_session == target_session
    {
        return Ok(false);
    }
    let source_dir = session_image_dir(sessions_root, source_session);
    if !source_dir.is_dir() {
        return Ok(false);
    }
    let target_dir = session_image_dir(sessions_root, target_session);
    let mut copied = false;
    let entries = std::fs::read_dir(&source_dir).map_err(|_| ImagePrepareError::IoError)?;
    for entry in entries {
        let entry = entry.map_err(|_| ImagePrepareError::IoError)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if !copied {
            std::fs::create_dir_all(&target_dir).map_err(|_| ImagePrepareError::IoError)?;
            let _ = super::restrict_dir_permissions(sessions_root);
            let _ = super::restrict_dir_permissions(&target_dir);
        }
        let target = target_dir.join(entry.file_name());
        std::fs::copy(&path, &target).map_err(|_| ImagePrepareError::IoError)?;
        let _ = super::restrict_file_permissions(&target);
        copied = true;
    }
    Ok(copied)
}

/// session id 只能是单段名字；拒绝空串、路径分隔符与 `.`。
fn valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty() && !session_id.contains(['/', '\\', '.'])
}

/// 删除某会话已归档的图片目录 `.../session/<session_id>/image/`（用户主动删除会话时调用）。
///
/// 只删除图片目录，再尝试删除空的会话目录：会话目录以后可能放别的数据。
pub fn remove_session_images(session_id: &str) {
    if session_id.is_empty() || session_id.contains(['/', '\\', '.']) {
        return;
    }
    if let Ok(root) = sessions_root_dir() {
        let session_root = root.join(session_id);
        let dir = session_image_dir(&root, session_id);
        if dir.is_dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = std::fs::remove_dir(&session_root);
    }
}

/// 刷新文件的最后修改时间为当前时刻。
fn touch_file_mtime(path: &Path) {
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// 若路径位于受管的会话图片根内，刷新其修改时间（供 Rewind 时延长保留期）。
pub fn touch_session_image_if_managed(path: &Path) {
    let is_managed = sessions_root_dir().is_ok_and(|dir| path.starts_with(dir));
    if is_managed && path.is_file() {
        touch_file_mtime(path);
    }
}

/// 清理指定会话根下各 `<session_id>/image/` 中未被引用且超过保留期的图片。
pub fn prune_session_images_in(
    sessions_root: &Path,
    referenced_paths: &std::collections::HashSet<String>,
    max_age: std::time::Duration,
) -> usize {
    let Ok(session_dirs) = std::fs::read_dir(sessions_root) else {
        return 0;
    };
    let referenced_keys: std::collections::HashSet<String> = referenced_paths
        .iter()
        .flat_map(|p| {
            let path = Path::new(p);
            let mut keys = vec![p.clone(), super::path_key(path)];
            if let Ok(canon) = std::fs::canonicalize(path) {
                keys.push(canon.to_string_lossy().into_owned());
            }
            keys
        })
        .collect();

    let now = std::time::SystemTime::now();
    let mut removed = 0;

    for session_entry in session_dirs.flatten() {
        let session_path = session_entry.path();
        if !session_path.is_dir() {
            continue;
        }
        let image_path = session_path.join("image");
        let Ok(files) = std::fs::read_dir(&image_path) else {
            continue;
        };
        for entry in files.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let raw = path.to_string_lossy().into_owned();
            let key = super::path_key(&path);
            let canon = std::fs::canonicalize(&path)
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
            if referenced_keys.contains(&raw)
                || referenced_keys.contains(&key)
                || canon.as_ref().is_some_and(|c| referenced_keys.contains(c))
            {
                continue;
            }
            let expired = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= max_age);
            if expired && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        // 空 image/ 与空会话目录一并清理。
        if std::fs::read_dir(&image_path)
            .ok()
            .is_some_and(|mut it| it.next().is_none())
        {
            let _ = std::fs::remove_dir(&image_path);
        }
        if std::fs::read_dir(&session_path)
            .ok()
            .is_some_and(|mut it| it.next().is_none())
        {
            let _ = std::fs::remove_dir(&session_path);
        }
    }
    removed
}

/// 启动时清理默认会话根下的过期（30 天）未引用图片。
pub fn prune_default_session_images(referenced_paths: &std::collections::HashSet<String>) -> usize {
    let Ok(dir) = sessions_root_dir() else {
        return 0;
    };
    prune_session_images_in(
        &dir,
        referenced_paths,
        std::time::Duration::from_secs(IMAGE_RETENTION_SECS),
    )
}

/// 进程内图片处理缓存上限（32 张，对齐 Codex `IMAGE_CACHE`），消除准备、归档与流启动的重复解码。
const MAX_IMAGE_CACHE_ENTRIES: usize = 32;

static IMAGE_CACHE: LazyLock<Mutex<Vec<(u64, ProcessedImage)>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// 处理原始图片字节：
/// 1. 命中内存缓存直接返回；
/// 2. 仅读取头部校验尺寸，若原图已是 PNG/JPEG 且宽高 `<= 2048`，直接复用原始字节（零像素解码/重编码，对齐 Codex `can_preserve_source_bytes`）；
/// 3. 超出 2048 或非常规格式才解码、等比缩放并重编码为 PNG/JPEG。
pub fn process_image_bytes(
    raw_bytes: &[u8],
    source_name: &str,
    source_path: Option<PathBuf>,
) -> Result<ProcessedImage, ImagePrepareError> {
    if raw_bytes.len() > MAX_IMAGE_RAW_BYTES {
        return Err(ImagePrepareError::SingleTooLarge);
    }
    let digest = fnv1a_hash(raw_bytes);
    if let Ok(cache) = IMAGE_CACHE.lock()
        && let Some((_, cached)) = cache.iter().find(|(k, _)| *k == digest)
    {
        return Ok(ProcessedImage {
            source_name: source_name.to_string(),
            source_path,
            mime: cached.mime.clone(),
            width: cached.width,
            height: cached.height,
            bytes: cached.bytes.clone(),
            original_byte_count: cached.original_byte_count,
        });
    }

    let original_byte_count = raw_bytes.len();
    let format = image::guess_format(raw_bytes).map_err(|_| ImagePrepareError::NotAnImage)?;
    match format {
        ImageFormat::Png
        | ImageFormat::Jpeg
        | ImageFormat::Gif
        | ImageFormat::WebP
        | ImageFormat::Bmp
        | ImageFormat::Tiff => {}
        _ => return Err(ImagePrepareError::NotAnImage),
    }

    let decoder = ImageReader::with_format(Cursor::new(raw_bytes), format)
        .into_decoder()
        .map_err(|_| ImagePrepareError::DecodeFailed)?;
    let (w, h) = decoder.dimensions();
    if (w as u64) * (h as u64) > MAX_DECODE_PIXELS {
        return Err(ImagePrepareError::DimensionTooLarge);
    }

    // 对齐 Codex：已是 PNG/JPEG 且宽高不超 2048 时直接复用原字节，跳过全量像素解码与重压缩。
    let (final_w, final_h, mime, encoded_bytes) =
        if matches!(format, ImageFormat::Png | ImageFormat::Jpeg)
            && w <= MAX_IMAGE_DIMENSION
            && h <= MAX_IMAGE_DIMENSION
        {
            if format == ImageFormat::Jpeg {
                let mut reader = ImageReader::with_format(Cursor::new(raw_bytes), format);
                reader.no_limits();
                let _ = reader
                    .decode()
                    .map_err(|_| ImagePrepareError::DecodeFailed)?;
            } else {
                let _ = ImageReader::with_format(Cursor::new(raw_bytes), format)
                    .into_decoder()
                    .and_then(image::DynamicImage::from_decoder)
                    .map_err(|_| ImagePrepareError::DecodeFailed)?;
            }
            let mime = if format == ImageFormat::Jpeg {
                "image/jpeg".to_string()
            } else {
                "image/png".to_string()
            };
            (w, h, mime, raw_bytes.to_vec())
        } else {
            let dyn_img = image::DynamicImage::from_decoder(decoder)
                .map_err(|_| ImagePrepareError::DecodeFailed)?;
            let dyn_img = if w > MAX_IMAGE_DIMENSION || h > MAX_IMAGE_DIMENSION {
                dyn_img.resize(
                    MAX_IMAGE_DIMENSION,
                    MAX_IMAGE_DIMENSION,
                    FilterType::Triangle,
                )
            } else {
                dyn_img
            };
            let (final_w, final_h) = (dyn_img.width(), dyn_img.height());
            if format == ImageFormat::Jpeg && !dyn_img.color().has_alpha() {
                let mut buf = Vec::new();
                dyn_img
                    .write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)
                    .map_err(|_| ImagePrepareError::EncodeFailed)?;
                (final_w, final_h, "image/jpeg".to_string(), buf)
            } else {
                let mut buf = Vec::new();
                dyn_img
                    .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                    .map_err(|_| ImagePrepareError::EncodeFailed)?;
                (final_w, final_h, "image/png".to_string(), buf)
            }
        };

    let result = ProcessedImage {
        source_name: source_name.to_string(),
        source_path,
        mime,
        width: final_w,
        height: final_h,
        bytes: encoded_bytes,
        original_byte_count,
    };

    if let Ok(mut cache) = IMAGE_CACHE.lock() {
        let encoded_digest = fnv1a_hash(&result.bytes);
        for key in [digest, encoded_digest] {
            if !cache.iter().any(|(k, _)| *k == key) {
                if cache.len() >= MAX_IMAGE_CACHE_ENTRIES {
                    cache.remove(0);
                }
                cache.push((key, result.clone()));
            }
        }
    }

    Ok(result)
}

/// 将已处理的图片严格归档到 `<sessions_root>/<session_id>/image/<hash>-<filename>`，
/// 并在落盘后回读校验存在性、可读性、尺寸、MIME 与内容摘要。
pub fn archive_processed_image_in(
    sessions_root: &Path,
    session_id: &str,
    processed: &ProcessedImage,
) -> Result<ArchivedSessionImage, ImagePrepareError> {
    if session_id.is_empty() || session_id.contains(['/', '\\', '.']) {
        return Err(ImagePrepareError::IoError);
    }
    let session_dir = session_image_dir(sessions_root, session_id);
    std::fs::create_dir_all(&session_dir).map_err(|_| ImagePrepareError::IoError)?;
    let _ = super::restrict_dir_permissions(sessions_root);
    let _ = super::restrict_dir_permissions(&session_dir);

    let hash_hex = format!("{:016x}", fnv1a_hash(&processed.bytes));
    let safe_name = sanitize_filename(&processed.source_name, &processed.mime);
    let target = session_dir.join(format!("{hash_hex}-{safe_name}"));
    let need_write = match std::fs::read(&target) {
        Ok(existing) => format!("{:016x}", fnv1a_hash(&existing)) != hash_hex,
        Err(_) => true,
    };
    if need_write {
        super::write_file_atomic(&target, &processed.bytes)
            .map_err(|_| ImagePrepareError::IoError)?;
        let _ = super::restrict_file_permissions(&target);
    } else {
        touch_file_mtime(&target);
    }
    let verified = verify_archived_image_in(
        sessions_root,
        session_id,
        &target,
        &processed.mime,
        &hash_hex,
        processed.width,
        processed.height,
    )?;
    Ok(ArchivedSessionImage {
        label: safe_name,
        archive_path: target,
        mime: processed.mime.clone(),
        content_hash: hash_hex,
        width: processed.width,
        height: processed.height,
        bytes: verified,
    })
}

/// 将已处理图片严格归档到默认会话图片目录。
pub fn archive_processed_image(
    session_id: &str,
    processed: &ProcessedImage,
) -> Result<ArchivedSessionImage, ImagePrepareError> {
    let root = sessions_root_dir()?;
    archive_processed_image_in(&root, session_id, processed)
}

/// 校验归档图片位于 `<sessions_root>/<session_id>/image/` 内，且可读、格式/尺寸/摘要与记录一致。
pub fn verify_archived_image_in(
    sessions_root: &Path,
    session_id: &str,
    archive_path: &Path,
    expected_mime: &str,
    expected_hash: &str,
    expected_width: u32,
    expected_height: u32,
) -> Result<Vec<u8>, ImagePrepareError> {
    if session_id.is_empty()
        || session_id.contains(['/', '\\', '.'])
        || expected_hash.is_empty()
        || !matches!(expected_mime, "image/png" | "image/jpeg")
        || expected_width == 0
        || expected_height == 0
    {
        return Err(ImagePrepareError::NotAnImage);
    }
    if archive_path
        .components()
        .any(|comp| matches!(comp, std::path::Component::ParentDir))
    {
        return Err(ImagePrepareError::FileNotFound);
    }
    let session_dir = session_image_dir(sessions_root, session_id);
    let in_session_dir = archive_path.starts_with(&session_dir) || {
        match (
            std::fs::canonicalize(&session_dir),
            std::fs::canonicalize(archive_path),
        ) {
            (Ok(canon_dir), Ok(canon_file)) => canon_file.starts_with(&canon_dir),
            _ => false,
        }
    };
    if !in_session_dir {
        return Err(ImagePrepareError::FileNotFound);
    }
    let meta = std::fs::metadata(archive_path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => ImagePrepareError::FileNotFound,
        _ => ImagePrepareError::IoError,
    })?;
    if !meta.is_file() {
        return Err(ImagePrepareError::FileNotFound);
    }
    if meta.len() > MAX_IMAGE_RAW_BYTES as u64 {
        return Err(ImagePrepareError::SingleTooLarge);
    }
    let bytes = std::fs::read(archive_path).map_err(|_| ImagePrepareError::IoError)?;
    let actual_hash = format!("{:016x}", fnv1a_hash(&bytes));
    if actual_hash != expected_hash {
        return Err(ImagePrepareError::DecodeFailed);
    }
    let format = image::guess_format(&bytes).map_err(|_| ImagePrepareError::NotAnImage)?;
    let actual_mime = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        _ => return Err(ImagePrepareError::NotAnImage),
    };
    if actual_mime != expected_mime {
        return Err(ImagePrepareError::DecodeFailed);
    }
    let (w, h) = {
        let decoder = ImageReader::with_format(Cursor::new(&bytes), format)
            .into_decoder()
            .map_err(|_| ImagePrepareError::DecodeFailed)?;
        decoder.dimensions()
    };
    if w != expected_width || h != expected_height {
        return Err(ImagePrepareError::DecodeFailed);
    }
    Ok(bytes)
}

/// 在默认会话图片归档根目录下校验归档图片。
pub fn verify_archived_image(
    session_id: &str,
    archive_path: &Path,
    expected_mime: &str,
    expected_hash: &str,
    expected_width: u32,
    expected_height: u32,
) -> Result<Vec<u8>, ImagePrepareError> {
    let root = sessions_root_dir()?;
    verify_archived_image_in(
        &root,
        session_id,
        archive_path,
        expected_mime,
        expected_hash,
        expected_width,
        expected_height,
    )
}

/// 解析并处理内联 `data:image/<format>;base64,<payload>` URL。
pub fn process_data_url_image(
    data_url: &str,
    index: usize,
) -> Result<ProcessedImage, ImagePrepareError> {
    use base64::Engine;
    let trimmed = data_url.trim();
    if trimmed.len() > MAX_IMAGE_RAW_BYTES.saturating_mul(2) {
        return Err(ImagePrepareError::SingleTooLarge);
    }
    let Some(rest) = trimmed
        .strip_prefix("data:")
        .or_else(|| trimmed.strip_prefix("DATA:"))
    else {
        return Err(ImagePrepareError::NotAnImage);
    };
    let Some((header, payload)) = rest.split_once(',') else {
        return Err(ImagePrepareError::NotAnImage);
    };
    let header_lower = header.trim().to_ascii_lowercase();
    let Some(media_type) = header_lower.strip_suffix(";base64") else {
        return Err(ImagePrepareError::NotAnImage);
    };
    let media_type = media_type.split(';').next().unwrap_or_default().trim();
    if !media_type.starts_with("image/") || media_type.len() <= "image/".len() {
        return Err(ImagePrepareError::NotAnImage);
    }
    let clean_payload: String = payload
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if clean_payload.is_empty() {
        return Err(ImagePrepareError::NotAnImage);
    }
    // 解码前字节上限检查（每 4 个 base64 字符最多 3 字节）。
    if (clean_payload.len() / 4).saturating_mul(3) > MAX_IMAGE_RAW_BYTES {
        return Err(ImagePrepareError::SingleTooLarge);
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(clean_payload.as_bytes())
        .map_err(|_| ImagePrepareError::DecodeFailed)?;
    if decoded.is_empty() {
        return Err(ImagePrepareError::NotAnImage);
    }
    let name = format!("inline-image-{}", index.saturating_add(1));
    process_image_bytes(&decoded, &name, None)
}

///读取本地文件并处理成图片对象。
pub fn load_and_process_image_file(path: &Path) -> Result<ProcessedImage, ImagePrepareError> {
    let metadata = std::fs::metadata(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => ImagePrepareError::FileNotFound,
        _ => ImagePrepareError::IoError,
    })?;
    if !metadata.is_file() {
        return Err(ImagePrepareError::FileNotFound);
    }
    if metadata.len() > MAX_IMAGE_RAW_BYTES as u64 {
        return Err(ImagePrepareError::SingleTooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| ImagePrepareError::IoError)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    process_image_bytes(&bytes, name, Some(path.to_path_buf()))
}

#[cfg(test)]
mod tests;
