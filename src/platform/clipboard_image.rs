//! 剪贴板图片读取与图像处理：文件列表优先、像素数据回退、等比缩放与限额保护。
//!
//! OS 剪贴板与底层图像解码收敛于此，核心状态层不出现平台 `cfg` 与第三方 crate 泄漏。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::ImageFormat;
use image::imageops::FilterType;

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
    /// 原始文件路径（剪贴板纯像素为 None）。
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

/// 剪贴板读取到的图像数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImagePayload {
    pub image: ProcessedImage,
}

/// 处理原始图片字节：验证大小、解码、按需等比缩放、重新编码为 PNG 或 JPEG。
pub fn process_image_bytes(
    raw_bytes: &[u8],
    source_name: &str,
    source_path: Option<PathBuf>,
) -> Result<ProcessedImage, ImagePrepareError> {
    if raw_bytes.len() > MAX_IMAGE_RAW_BYTES {
        return Err(ImagePrepareError::SingleTooLarge);
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

    let dyn_img =
        image::load_from_memory(raw_bytes).map_err(|_| ImagePrepareError::DecodeFailed)?;
    let (w, h) = (dyn_img.width(), dyn_img.height());
    if (w as u64) * (h as u64) > MAX_DECODE_PIXELS {
        return Err(ImagePrepareError::DimensionTooLarge);
    }

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
    // 如果原图是 JPEG 且无透明通道，保留 JPEG 编码；其余一律编码为 PNG
    let (mime, encoded_bytes) = if format == ImageFormat::Jpeg && !dyn_img.color().has_alpha() {
        let mut buf = Vec::new();
        dyn_img
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)
            .map_err(|_| ImagePrepareError::EncodeFailed)?;
        ("image/jpeg".to_string(), buf)
    } else {
        let mut buf = Vec::new();
        dyn_img
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .map_err(|_| ImagePrepareError::EncodeFailed)?;
        ("image/png".to_string(), buf)
    };

    Ok(ProcessedImage {
        source_name: source_name.to_string(),
        source_path,
        mime,
        width: final_w,
        height: final_h,
        bytes: encoded_bytes,
        original_byte_count,
    })
}

/// 读取本地文件并处理成图片对象。
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

/// 从系统剪贴板读取图片：文件列表优先，像素数据回退；无图返回 Ok(None)。
pub fn read_clipboard_image() -> Result<Option<ClipboardImagePayload>, ImagePrepareError> {
    let mut cb = arboard::Clipboard::new().map_err(|_| ImagePrepareError::ClipboardUnavailable)?;

    // 1. 尝试剪贴板中的文件列表
    if let Ok(files) = cb.get().file_list() {
        for file in files {
            if file.is_file() && super::sniff_image_file(&file) {
                match load_and_process_image_file(&file) {
                    Ok(processed) => {
                        return Ok(Some(ClipboardImagePayload { image: processed }));
                    }
                    Err(ImagePrepareError::FileNotFound) => continue,
                    Err(e) => return Err(e),
                }
            }
        }
    }

    // 2. 尝试剪贴板中的像素数据
    match cb.get_image() {
        Ok(img) => {
            let (w, h) = (img.width as u32, img.height as u32);
            let raw_len = img.bytes.len();
            if raw_len > MAX_IMAGE_RAW_BYTES {
                return Err(ImagePrepareError::SingleTooLarge);
            }
            if (w as u64) * (h as u64) > MAX_DECODE_PIXELS {
                return Err(ImagePrepareError::DimensionTooLarge);
            }
            let Some(rgba) = image::RgbaImage::from_raw(w, h, img.bytes.into_owned()) else {
                return Err(ImagePrepareError::DecodeFailed);
            };
            let dyn_img = image::DynamicImage::ImageRgba8(rgba);
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
            let mut png_bytes = Vec::new();
            dyn_img
                .write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
                .map_err(|_| ImagePrepareError::EncodeFailed)?;

            Ok(Some(ClipboardImagePayload {
                image: ProcessedImage {
                    source_name: "clipboard.png".to_string(),
                    source_path: None,
                    mime: "image/png".to_string(),
                    width: final_w,
                    height: final_h,
                    bytes: png_bytes,
                    original_byte_count: raw_len,
                },
            }))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests;
