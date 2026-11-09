//! 流中 steer 的内存态：待注入项与按上限合并规则。
//!
//! 这里只放纯数据与纯函数，不触碰 DB / UI / IO；提交、撤回与终态处理在
//! `app::update::chat::steer`，生效落库由事件层完成。

use crate::chat::events::ModelImage;
use crate::db::chat::{MAX_MESSAGE_BYTES, MessageAttachmentInput, ModelSnapshot};

/// 单个窗格最多同时挂起的 steer 条数。
pub(crate) const MAX_PENDING_STEERS: usize = 8;
/// pending steer 图片字节总预算（按准备给模型的图片字节计）。
pub(crate) const MAX_PENDING_IMAGE_BYTES: u64 = 40 * 1024 * 1024;

/// 一条已提交、尚未注入当前轮的 steer。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSteer {
    /// 原始输入文本（保留 `@` 引用、skill 标记与图片占位符）。
    pub text: String,
    /// 准备写入数据库的附件（图片/文件/文件夹/skill）。
    pub attachments: Vec<MessageAttachmentInput>,
    /// 准备送给当前模型的图片字节（来自 `PreparedUserMessage::model_images`）。
    pub model_images: Vec<ModelImage>,
    /// 提交时冻结的模型快照；本轮结束提升为下一轮时的回退值。
    pub model: ModelSnapshot,
}

impl PendingSteer {
    /// 准备给模型的去重图片数量。
    pub fn image_count(&self) -> usize {
        self.model_images.len()
    }

    /// 已准备图片字节总量。
    pub fn image_bytes(&self) -> u64 {
        self.model_images
            .iter()
            .map(|image| u64::try_from(image.bytes.len()).unwrap_or(u64::MAX))
            .sum()
    }
}

/// 一个边界上可注入的 steer 批次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteerBatch {
    pub text: String,
    pub attachments: Vec<MessageAttachmentInput>,
    pub model_images: Vec<ModelImage>,
    pub model: ModelSnapshot,
    /// 本批消费了 pending 头部多少条。
    pub consumed: usize,
}

impl SteerBatch {
    /// 本批图片字节总量。
    pub fn image_bytes(&self) -> u64 {
        self.model_images
            .iter()
            .map(|image| u64::try_from(image.bytes.len()).unwrap_or(u64::MAX))
            .sum()
    }
}

/// 从 pending 头部尽量合并，返回本批内容；不修改输入。
///
/// 至少取第 1 条；后续条目在文本字节、图片数量或图片字节超限时停止合并，
/// 未并入的条目留在 pending，等下一个安全边界。
pub fn build_batch(pending: &[PendingSteer]) -> Option<SteerBatch> {
    let first = pending.first()?;
    let mut batch = SteerBatch {
        text: first.text.clone(),
        attachments: first.attachments.clone(),
        model_images: first.model_images.clone(),
        model: first.model.clone(),
        consumed: 1,
    };
    for next in &pending[1..] {
        if batch.text.len().saturating_add(2 + next.text.len()) > MAX_MESSAGE_BYTES {
            break;
        }
        if batch
            .model_images
            .len()
            .saturating_add(next.model_images.len())
            > crate::platform::MAX_ATTACHED_IMAGES
        {
            break;
        }
        if batch.image_bytes().saturating_add(next.image_bytes()) > MAX_PENDING_IMAGE_BYTES {
            break;
        }
        let offset = batch.text.len() + 2;
        batch.text.push_str("\n\n");
        batch.text.push_str(&next.text);
        for attachment in &next.attachments {
            let mut attachment = attachment.clone();
            attachment.marker_start = attachment
                .marker_start
                .map(|position| position.saturating_add(offset));
            attachment.marker_end = attachment
                .marker_end
                .map(|position| position.saturating_add(offset));
            batch.attachments.push(attachment);
        }
        for image in &next.model_images {
            if !batch
                .model_images
                .iter()
                .any(|existing| existing.bytes == image.bytes)
            {
                batch.model_images.push(image.clone());
            }
        }
        batch.consumed += 1;
    }
    Some(batch)
}

#[cfg(test)]
mod tests;
