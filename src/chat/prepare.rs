//! 用户消息发送前准备：解析引用、读取/解码图片、去重、限额检查与模型图片装配。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::markdown::{ReferKind, parse_references};
use crate::chat::events::ModelImage;
use crate::db::chat::{MessageAttachmentInput, MessageAttachmentKind};
use crate::platform::{
    MAX_ATTACHED_IMAGES, MAX_TOTAL_IMAGE_RAW_BYTES, ProcessedImage, load_and_process_image_file,
    sniff_image_file,
};

pub use crate::platform::ImagePrepareError;

/// 草稿中的图片附件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftImageAttachment {
    pub id: String,
    pub image: ProcessedImage,
}

/// 准备完成的用户消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedUserMessage {
    /// 原始用户输入文本（保留所有原始标记）。
    pub raw_text: String,
    /// 准备写入数据库的附件列表（含元数据与图片数据）。
    pub attachments: Vec<MessageAttachmentInput>,
    /// 准备提供给模型的去重图片列表。
    pub model_images: Vec<ModelImage>,
}

/// 准备一条用户消息。
///
/// 1. 提取所有直接草稿附件；
/// 2. 解析文本中的 `@[refer-*](path)` 显式引用；
/// 3. 解析文本中的 `@相对路径` 有效引用（跳过代码围栏）；
/// 4. 尝试解析 `@ref/` 唯一短引用；
/// 5. 校验图片存在性、格式与 20 MiB/张、4 张/条、40 MiB/条总量预算；
/// 6. 对规范化路径或二进制相同的图片去重（模型只发一份，显示行仍保留位置）。
pub fn prepare_user_message(
    workspace_root: &Path,
    raw_text: &str,
    draft_images: &[DraftImageAttachment],
) -> Result<PreparedUserMessage, ImagePrepareError> {
    if raw_text.trim().is_empty() && draft_images.is_empty() {
        return Err(ImagePrepareError::NotAnImage);
    }

    let mut attachments: Vec<MessageAttachmentInput> = Vec::new();
    let mut unique_images: Vec<ProcessedImage> = Vec::new();
    let mut total_original_bytes: usize = 0;

    // 已处理的文本区间，避免重复命中
    let mut claimed_ranges: Vec<(usize, usize)> = Vec::new();

    // 辅助闭包：向 unique_images 追加图片并检查去重与限额
    let mut add_unique_image = |img: ProcessedImage| -> Result<(), ImagePrepareError> {
        let is_dup = unique_images.iter().any(|existing| {
            if let (Some(p1), Some(p2)) = (&existing.source_path, &img.source_path)
                && let (Ok(c1), Ok(c2)) = (std::fs::canonicalize(p1), std::fs::canonicalize(p2))
                && c1 == c2
            {
                return true;
            }
            existing.bytes == img.bytes
        });

        if !is_dup {
            if unique_images.len() + 1 > MAX_ATTACHED_IMAGES {
                return Err(ImagePrepareError::TooManyImages);
            }
            if total_original_bytes.saturating_add(img.original_byte_count)
                > MAX_TOTAL_IMAGE_RAW_BYTES
            {
                return Err(ImagePrepareError::TotalTooLarge);
            }
            total_original_bytes = total_original_bytes.saturating_add(img.original_byte_count);
            unique_images.push(img);
        }
        Ok(())
    };

    // 1. 处理直接草稿附件（测试或预准备附件）
    for draft in draft_images {
        add_unique_image(draft.image.clone())?;
        let source_path = draft
            .image
            .source_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| draft.image.source_name.clone());
        attachments.push(MessageAttachmentInput {
            kind: MessageAttachmentKind::Image,
            label: draft.image.source_name.clone(),
            marker_start: None,
            marker_end: None,
            source_path: Some(source_path),
            mime: Some(draft.image.mime.clone()),
            width: Some(draft.image.width),
            height: Some(draft.image.height),
        });
    }

    // 2. 解析显式 `@[refer-*](path)` 引用
    let explicit_refs = parse_references(raw_text);
    for reference in &explicit_refs {
        let range_start = reference.range.start;
        let mut range_end = reference.range.end;
        if raw_text[range_end..].starts_with(' ') {
            range_end += 1;
        }
        claimed_ranges.push((range_start, range_end));

        match reference.kind {
            ReferKind::Folder => {
                attachments.push(MessageAttachmentInput {
                    kind: MessageAttachmentKind::Folder,
                    label: reference.path.clone(),
                    marker_start: Some(range_start),
                    marker_end: Some(range_end),
                    source_path: Some(reference.path.clone()),
                    mime: None,
                    width: None,
                    height: None,
                });
            }
            ReferKind::File => {
                attachments.push(MessageAttachmentInput {
                    kind: MessageAttachmentKind::File,
                    label: reference.path.clone(),
                    marker_start: Some(range_start),
                    marker_end: Some(range_end),
                    source_path: Some(reference.path.clone()),
                    mime: None,
                    width: None,
                    height: None,
                });
            }
            ReferKind::Image => {
                let resolved = resolve_reference_path(workspace_root, &reference.path);
                let processed = load_and_process_image_file(&resolved)?;
                add_unique_image(processed.clone())?;
                attachments.push(MessageAttachmentInput {
                    kind: MessageAttachmentKind::Image,
                    label: reference.path.clone(),
                    marker_start: Some(range_start),
                    marker_end: Some(range_end),
                    source_path: Some(resolved.to_string_lossy().into_owned()),
                    mime: Some(processed.mime),
                    width: Some(processed.width),
                    height: Some(processed.height),
                });
            }
        }
    }

    // 3. 解析 `@相对路径` 引用
    for range in markdown_mention_token_ranges(raw_text) {
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
            let mut end = range.end;
            if raw_text[end..].starts_with(' ') {
                end += 1;
            }
            claimed_ranges.push((range.start, end));

            if metadata.is_dir() {
                attachments.push(MessageAttachmentInput {
                    kind: MessageAttachmentKind::Folder,
                    label: token_text.to_string(),
                    marker_start: Some(range.start),
                    marker_end: Some(end),
                    source_path: Some(token_text.to_string()),
                    mime: None,
                    width: None,
                    height: None,
                });
            } else if sniff_image_file(&resolved) {
                let processed = load_and_process_image_file(&resolved)?;
                add_unique_image(processed.clone())?;
                attachments.push(MessageAttachmentInput {
                    kind: MessageAttachmentKind::Image,
                    label: token_text.to_string(),
                    marker_start: Some(range.start),
                    marker_end: Some(end),
                    source_path: Some(resolved.to_string_lossy().into_owned()),
                    mime: Some(processed.mime),
                    width: Some(processed.width),
                    height: Some(processed.height),
                });
            } else {
                attachments.push(MessageAttachmentInput {
                    kind: MessageAttachmentKind::File,
                    label: token_text.to_string(),
                    marker_start: Some(range.start),
                    marker_end: Some(end),
                    source_path: Some(token_text.to_string()),
                    mime: None,
                    width: None,
                    height: None,
                });
            }
        }
    }

    // 4. 解析 `@ref/` 短引用
    for (start, end, short_name) in parse_short_ref_tokens(raw_text) {
        if claimed_ranges.iter().any(|(s, e)| start < *e && end > *s) {
            continue;
        }
        // 查找是否唯一匹配已有完整引用
        let matches: Vec<&MessageAttachmentInput> = attachments
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
            attachments.push(MessageAttachmentInput {
                kind: matched.kind,
                label: short_name.to_string(),
                marker_start: Some(start),
                marker_end: Some(marker_end),
                source_path: matched.source_path.clone(),
                mime: matched.mime.clone(),
                width: matched.width,
                height: matched.height,
            });
        }
    }

    // 按在输入文本中的起始位置排序，使引用树与文本出现顺序严格一致
    attachments.sort_by_key(|att| att.marker_start.unwrap_or(0));

    let model_images = unique_images
        .into_iter()
        .map(|img| ModelImage {
            mime: img.mime,
            bytes: Arc::new(img.bytes),
        })
        .collect();

    Ok(PreparedUserMessage {
        raw_text: raw_text.to_string(),
        attachments,
        model_images,
    })
}

/// 解析文本中全文的 @ 提及 token 区间（跳过代码围栏）。
fn markdown_mention_token_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find('@') {
        let at = index + offset;
        // 排除 `@[refer-`
        if text[at..].starts_with("@[refer-") || text[at..].starts_with("@ref/") {
            index = at + 1;
            continue;
        }
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|previous| previous.is_whitespace());
        if boundary && !inside_code_fence(text, at) {
            let mut end = at + 1;
            for ch in text[end..].chars() {
                if ch.is_whitespace() || ch == '@' {
                    break;
                }
                end += ch.len_utf8();
            }
            if end > at + 1 {
                ranges.push(at..end);
            }
            index = end;
        } else {
            index = at + 1;
        }
    }
    ranges
}

/// 查找文本中的 `@ref/` token 区间与短名称。
fn parse_short_ref_tokens(text: &str) -> Vec<(usize, usize, String)> {
    let mut result = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find("@ref/") {
        let at = index + offset;
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|p| p.is_whitespace());
        if boundary && !inside_code_fence(text, at) {
            let start = at;
            let mut end = at + "@ref/".len();
            for ch in text[end..].chars() {
                if ch.is_whitespace() || ch == '@' {
                    break;
                }
                end += ch.len_utf8();
            }
            let name = text[at + "@ref/".len()..end].trim_end_matches('/');
            if !name.is_empty() {
                result.push((start, end, name.to_string()));
            }
            index = end;
        } else {
            index = at + 1;
        }
    }
    result
}

/// 判定位置是否在代码围栏或行内代码反引号保护区内。
fn inside_code_fence(text: &str, pos: usize) -> bool {
    let pos = pos.min(text.len());
    let prefix = &text[..pos];
    if crate::app::markdown::open_fence(prefix).is_some() {
        return true;
    }
    let line_start = prefix.rfind('\n').map_or(0, |idx| idx + 1);
    crate::app::markdown::inline_code_open(&text[line_start..pos])
}

/// 路径解析：若为相对路径，按工作区根解析。
fn resolve_reference_path(workspace_root: &Path, path_str: &str) -> PathBuf {
    let path = Path::new(path_str);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

/// 移除展示用正文中的有效引用标记，并清理多余空白。
pub fn strip_references_for_display(
    raw_text: &str,
    attachments: &[crate::db::chat::MessageAttachmentMeta],
) -> String {
    strip_marker_ranges(
        raw_text,
        attachments
            .iter()
            .filter_map(|att| att.marker_start.zip(att.marker_end)),
    )
}

/// 移除尚未落库的附件输入对应的引用标记，供生成会话标题/预览与空正文回退使用。
pub fn strip_references_from_inputs(
    raw_text: &str,
    attachments: &[crate::db::chat::MessageAttachmentInput],
) -> String {
    strip_marker_ranges(
        raw_text,
        attachments
            .iter()
            .filter_map(|att| att.marker_start.zip(att.marker_end)),
    )
}

fn strip_marker_ranges(
    raw_text: &str,
    marker_ranges: impl IntoIterator<Item = (usize, usize)>,
) -> String {
    let mut ranges: Vec<(usize, usize)> = marker_ranges
        .into_iter()
        .filter(|(s, e)| {
            *s < *e
                && *e <= raw_text.len()
                && raw_text.is_char_boundary(*s)
                && raw_text.is_char_boundary(*e)
        })
        .collect();

    if ranges.is_empty() {
        return raw_text.trim().to_string();
    }

    // 从后往前切除
    ranges.sort_by_key(|b| std::cmp::Reverse(b.0));
    let mut text = raw_text.to_string();
    for (start, end) in ranges {
        text.replace_range(start..end, "");
    }

    // 清理连续空格但保留换行
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests;
