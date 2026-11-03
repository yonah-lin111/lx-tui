//! 单块绘制数据的指纹（不含宽度，宽度由 `BlockKey::width` 承担）。
//!
//! 这些指纹决定二级缓存里同一槽位的绘制数据何时重建；任何会改变行数或可见
//! 正文的状态都必须进指纹。

use super::super::source::HistorySource;
use super::signature::{Fnv, shell_signature};

/// 本地命令卡片的指纹（不含宽度）。
pub(crate) fn shell_block_signature(
    source: HistorySource<'_>,
    message: &crate::app::state::ChatMessageView,
    shell: &crate::app::state::UserShellView,
) -> u64 {
    let mut hash = Fnv::new();
    shell_signature(&mut hash, source, message, shell);
    hash.finish()
}

/// 汇报块指纹：身份、状态、计数、耗时与正文长度 + 展开状态。
pub(crate) fn report_block_signature(
    report: &crate::app::state::SubagentReportView,
    expanded: bool,
    more_steps: usize,
) -> u64 {
    let mut hash = Fnv::new();
    hash.write_str(&report.agent_id);
    hash.write_str(&report.title);
    hash.write_str(report.role.as_str());
    hash.write_str(&report.status);
    hash.write_usize(report.files_changed);
    hash.write_i64(report.duration_ms.unwrap_or(-1));
    hash.write_usize(report.body.len());
    hash.write_edges(&report.body);
    hash.write_u8(u8::from(expanded));
    hash.write_usize(more_steps);
    hash.finish()
}

/// 消息底部通知的指纹（命中消息通知文案）。
pub(crate) fn notice_block_signature(notice: &super::super::notice::Notice) -> u64 {
    let mut hash = Fnv::new();
    hash.write_u8(match notice.title {
        super::super::notice::NoticeTitle::Failed => 0,
        super::super::notice::NoticeTitle::Interrupted => 1,
    });
    hash.write_str(&notice.reason);
    hash.finish()
}

/// 统计行的指纹。
pub(crate) fn stats_block_signature(line: &str) -> u64 {
    let mut hash = Fnv::new();
    hash.write_str(line);
    hash.finish()
}

/// 撤销提示条目的指纹。
pub(crate) fn rewind_block_signature(
    notice: &crate::app::state::RewindNoticeView,
    count: usize,
) -> u64 {
    let mut hash = Fnv::new();
    hash.write_usize(count);
    hash.write_str(&notice.id);
    hash.write_str(&notice.kind);
    hash.write_usize(notice.restored_file_count);
    hash.write_u8(u8::from(notice.files_already_restored));
    hash.finish()
}

/// compact 检查点条目的指纹。
pub(crate) fn compact_block_signature(
    notice: &crate::app::state::CompactNoticeView,
    expanded: bool,
) -> u64 {
    let mut hash = Fnv::new();
    hash.write_str(&notice.id);
    hash.write_str(&notice.model_label);
    hash.write_i64(notice.created_at_ms);
    hash.write_i64(notice.start_sequence);
    hash.write_i64(notice.end_sequence);
    hash.write_str(&notice.summary);
    hash.write_u8(u8::from(notice.active));
    hash.write_u8(u8::from(notice.auto));
    hash.write_u8(u8::from(expanded));
    hash.finish()
}

/// 正在执行的 compact 压缩请求指纹。
pub(crate) fn running_compact_block_signature(compact: &crate::app::state::LxCompact) -> u64 {
    let mut hash = Fnv::new();
    hash.write_str(compact.running.as_deref().unwrap_or(""));
    hash.write_i64(compact.elapsed_ms.unwrap_or(0) / 1000);
    hash.finish()
}

/// 模型切换提示条目的指纹（覆盖整个合并区间，含 `is_variant_only` 的判定）。
pub(crate) fn model_switch_block_signature(
    slice: &[crate::app::state::ModelSwitchNoticeView],
    expanded: bool,
) -> u64 {
    let mut hash = Fnv::new();
    for notice in slice {
        hash.write_str(&notice.id);
        hash.write_str(notice.from_label.as_deref().unwrap_or(""));
        hash.write_str(&notice.to_label);
        hash.write_str(notice.to_variant.as_deref().unwrap_or(""));
        hash.write_u8(u8::from(notice.is_variant_only()));
        for src in &notice.updated_sources {
            hash.write_str(src);
        }
        hash.write_i64(notice.created_at_ms);
        // 折叠态也渲染提示词预览，正文必须一直参与指纹。
        hash.write_str(&notice.injected_prompt);
    }
    hash.write_usize(slice.len());
    hash.write_u8(u8::from(expanded));
    hash.finish()
}
