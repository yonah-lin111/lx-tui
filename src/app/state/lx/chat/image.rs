//! AI Web 生成图片的原图下载元数据与状态（按附件 ID 索引）。

/// AI Web 生成图片的原图下载状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginalImageDownloadState {
    Idle,
    Downloading,
    Downloaded { path: String },
}

/// AI Web 生成图片的原图下载信息（按附件 ID 索引）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImageOriginal {
    pub site: super::super::web::AiWebSite,
    pub session_id: String,
    pub original_url: String,
    pub fallback_url: String,
    pub file_stem: String,
    pub state: OriginalImageDownloadState,
}
