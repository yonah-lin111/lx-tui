//! AI Web 模式的用户消息准备：解析本地文件与图片引用、比对保留的云端历史附件、
//! 拦截目录/Skill/MCP，并生成清洗后的网页输入文本与本地上传路径列表。

use std::path::Path;

use crate::app::markdown::{ReferKind, parse_references};
use crate::db::chat::{MessageAttachmentInput, MessageAttachmentKind, MessageAttachmentMeta};
use crate::platform::sniff_image_file;

use super::{
    markdown_mention_token_ranges, parse_short_ref_tokens, resolve_ref_content_target,
    resolve_reference_path, strip_marker_ranges,
};

/// AI Web 消息准备错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiWebPrepareError {
    /// 引用了目录（AI Web 模式下禁止发送文件夹）。
    FolderUnsupported,
    /// 引用了不存在的本地文件。
    FileNotFound,
    /// 引用了本地 Skill 或 MCP（在 AI Web 模式下不可用）。
    LocalReferenceUnsupported,
}

/// AI Web 模式准备完成的用户消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAiWebMessage {
    /// 发往网页输入框的清洗后文本（已剥离引用标记并展开折叠粘贴块）。
    pub web_prompt: String,
    /// 供 TUI 消息气泡展示的附件列表。
    pub attachments: Vec<MessageAttachmentMeta>,
    /// 待通过 CDP `DOM.setFileInputFiles` 注入网页的去重本地绝对路径列表。
    pub files: Vec<String>,
    /// 原地编辑（`/rewind`）时需在网页编辑框内保留的已有云端附件文件名列表。
    pub keep_files: Vec<String>,
}

/// 根据文件名后缀判断是否为网页端支持展示为图片的类型。
pub fn is_image_filename(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    matches!(
        ext.as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "webp"
                | "gif"
                | "bmp"
                | "svg"
                | "svgz"
                | "ico"
                | "tif"
                | "tiff"
                | "avif"
                | "apng"
                | "jfif"
                | "pjpeg"
                | "pjp"
        )
    )
}

/// 为 AI Web 模式准备用户消息：解析本地文件与图片引用、比对保留的云端附件 `cloud_files`、
/// 拦截目录/Skill/MCP，生成本地路径列表 `files`、云端保留文件名 `keep_files` 与清洗后的 `web_prompt`。
pub fn prepare_ai_web_message(
    workspace_root: &Path,
    user_id: &str,
    raw_text: &str,
    cloud_files: &[String],
) -> Result<PreparedAiWebMessage, AiWebPrepareError> {
    let mut inputs: Vec<MessageAttachmentInput> = Vec::new();
    let mut claimed_ranges: Vec<(usize, usize)> = Vec::new();
    let mut remaining_cloud: Vec<String> = cloud_files.to_vec();
    let mut keep_files: Vec<String> = Vec::new();
    let masked = crate::app::paste::mask_pasted_text(raw_text);

    // 1. 解析显式 `@[refer-*](path)` 引用
    let explicit_refs = parse_references(&masked);
    for reference in &explicit_refs {
        let range_start = reference.range.start;
        let mut range_end = reference.range.end;
        if raw_text[range_end..].starts_with(' ') {
            range_end += 1;
        }
        claimed_ranges.push((range_start, range_end));

        match reference.kind {
            ReferKind::Folder => return Err(AiWebPrepareError::FolderUnsupported),
            ReferKind::Skill | ReferKind::Mcp => {
                return Err(AiWebPrepareError::LocalReferenceUnsupported);
            }
            ReferKind::File | ReferKind::Image => {
                let resolved = resolve_reference_path(workspace_root, &reference.path);
                match std::fs::metadata(&resolved) {
                    Ok(metadata) => {
                        if metadata.is_dir() {
                            return Err(AiWebPrepareError::FolderUnsupported);
                        }
                        if !metadata.is_file() {
                            return Err(AiWebPrepareError::FileNotFound);
                        }
                        let abs_path = std::fs::canonicalize(&resolved)
                            .unwrap_or(resolved.clone())
                            .to_string_lossy()
                            .into_owned();
                        let kind = if reference.kind == ReferKind::Image
                            || sniff_image_file(&resolved)
                            || is_image_filename(&reference.path)
                        {
                            MessageAttachmentKind::Image
                        } else {
                            MessageAttachmentKind::File
                        };
                        inputs.push(MessageAttachmentInput {
                            kind,
                            label: reference.path.clone(),
                            marker_start: Some(range_start),
                            marker_end: Some(range_end),
                            source_path: Some(abs_path),
                            mime: None,
                            width: None,
                            height: None,
                        });
                    }
                    Err(_) => {
                        if let Some(pos) = remaining_cloud
                            .iter()
                            .position(|name| name == &reference.path)
                        {
                            let kept = remaining_cloud.remove(pos);
                            let kind =
                                if reference.kind == ReferKind::Image || is_image_filename(&kept) {
                                    MessageAttachmentKind::Image
                                } else {
                                    MessageAttachmentKind::File
                                };
                            keep_files.push(kept.clone());
                            inputs.push(MessageAttachmentInput {
                                kind,
                                label: kept,
                                marker_start: Some(range_start),
                                marker_end: Some(range_end),
                                source_path: None,
                                mime: None,
                                width: None,
                                height: None,
                            });
                        } else {
                            return Err(AiWebPrepareError::FileNotFound);
                        }
                    }
                }
            }
        }
    }

    // 2. 解析 `@相对路径` 引用
    for range in markdown_mention_token_ranges(&masked) {
        if claimed_ranges
            .iter()
            .any(|(s, e)| range.start < *e && range.end > *s)
        {
            continue;
        }
        let token_text = &raw_text[range.start + 1..range.end];
        let trimmed_token = token_text.trim_end_matches('/');
        let resolved = resolve_reference_path(workspace_root, trimmed_token);
        if let Ok(metadata) = std::fs::metadata(&resolved) {
            if metadata.is_dir() {
                return Err(AiWebPrepareError::FolderUnsupported);
            }
            if !metadata.is_file() {
                continue;
            }
            let mut end = range.end;
            if raw_text[end..].starts_with(' ') {
                end += 1;
            }
            claimed_ranges.push((range.start, end));
            let abs_path = std::fs::canonicalize(&resolved)
                .unwrap_or(resolved.clone())
                .to_string_lossy()
                .into_owned();
            let kind = if sniff_image_file(&resolved) || is_image_filename(trimmed_token) {
                MessageAttachmentKind::Image
            } else {
                MessageAttachmentKind::File
            };
            inputs.push(MessageAttachmentInput {
                kind,
                label: token_text.to_string(),
                marker_start: Some(range.start),
                marker_end: Some(end),
                source_path: Some(abs_path),
                mime: None,
                width: None,
                height: None,
            });
        }
    }

    // 3. 解析 `@[ref-content]/` 快捷引用
    for range in crate::app::markdown::ref_content_ranges(&masked) {
        if claimed_ranges
            .iter()
            .any(|(s, e)| range.start < *e && range.end > *s)
        {
            continue;
        }
        let token_text =
            &raw_text[range.start + crate::app::markdown::REF_CONTENT_PREFIX.len()..range.end];
        let trimmed_token = token_text.trim_end_matches('/');
        if let Some((resolved, relative_path)) =
            resolve_ref_content_target(workspace_root, trimmed_token, &inputs)
            && let Ok(metadata) = std::fs::metadata(&resolved)
        {
            if metadata.is_dir() {
                return Err(AiWebPrepareError::FolderUnsupported);
            }
            if !metadata.is_file() {
                continue;
            }
            let mut end = range.end;
            if raw_text[end..].starts_with(' ') {
                end += 1;
            }
            claimed_ranges.push((range.start, end));
            let abs_path = std::fs::canonicalize(&resolved)
                .unwrap_or(resolved.clone())
                .to_string_lossy()
                .into_owned();
            let kind = if sniff_image_file(&resolved) || is_image_filename(&relative_path) {
                MessageAttachmentKind::Image
            } else {
                MessageAttachmentKind::File
            };
            inputs.push(MessageAttachmentInput {
                kind,
                label: relative_path,
                marker_start: Some(range.start),
                marker_end: Some(end),
                source_path: Some(abs_path),
                mime: None,
                width: None,
                height: None,
            });
        }
    }

    // 4. 解析 `@ref/` 短引用
    for (start, end, short_name) in parse_short_ref_tokens(&masked) {
        if claimed_ranges.iter().any(|(s, e)| start < *e && end > *s) {
            continue;
        }
        let matches: Vec<&MessageAttachmentInput> = inputs
            .iter()
            .filter(|att| {
                if let Some(name) = crate::app::markdown::short_ref_name(&att.label)
                    && name == short_name
                {
                    return true;
                }
                if let Some(src) = &att.source_path
                    && let Some(name) = crate::app::markdown::short_ref_name(src)
                {
                    return name == short_name;
                }
                false
            })
            .collect();
        if matches.len() == 1 {
            let matched = matches[0];
            let mut marker_end = end;
            if raw_text[marker_end..].starts_with(' ') {
                marker_end += 1;
            }
            claimed_ranges.push((start, marker_end));
            inputs.push(MessageAttachmentInput {
                kind: matched.kind,
                label: short_name.to_string(),
                marker_start: Some(start),
                marker_end: Some(marker_end),
                source_path: matched.source_path.clone(),
                mime: None,
                width: None,
                height: None,
            });
        }
    }

    inputs.sort_by_key(|att| att.marker_start.unwrap_or(0));

    let mut files: Vec<String> = Vec::new();
    for att in &inputs {
        if let Some(src) = &att.source_path
            && !files.contains(src)
        {
            files.push(src.clone());
        }
    }

    let stripped = strip_marker_ranges(
        raw_text,
        inputs
            .iter()
            .filter_map(|att| att.marker_start.zip(att.marker_end)),
    );
    let web_prompt = crate::app::paste::folded::expand_all(&stripped)
        .trim()
        .to_string();

    let attachments = inputs
        .into_iter()
        .enumerate()
        .map(|(idx, att)| MessageAttachmentMeta {
            id: format!("{user_id}-att-{idx}"),
            position: i64::try_from(idx).unwrap_or(0),
            kind: att.kind,
            label: att.label,
            marker_start: att.marker_start,
            marker_end: att.marker_end,
            source_path: att.source_path,
            mime: att.mime,
            width: att.width,
            height: att.height,
        })
        .collect();

    Ok(PreparedAiWebMessage {
        web_prompt,
        attachments,
        files,
        keep_files,
    })
}
