//! 会话中途模型切换与沙箱权限切换的 `<system-update>` 归并与裁剪保留。

use crate::db::chat::{ModelSwitchRecord, PermissionSwitchRecord};

/// 按 `sequence` 排序的系统更新条目（模型切换 / 权限切换共用）。
#[derive(Debug, Clone, Copy)]
pub(super) struct SystemUpdateRef<'a> {
    pub sequence: i64,
    pub injected_prompt: &'a str,
    order: u8,
}

/// 估算保留的系统更新 `<system-update>` 包裹文本占用的 token 数。
pub(super) fn retained_update_tokens(updates: &[SystemUpdateRef<'_>]) -> u64 {
    updates
        .iter()
        .filter(|sw| !sw.injected_prompt.trim().is_empty())
        .map(|sw| {
            let wrapped = crate::chat::agent::wrap_system_update(sw.injected_prompt);
            super::estimate_tokens(&wrapped) as u64
        })
        .sum()
}

/// 合并模型切换与权限切换记录：过滤 `min_sequence` 之后记录，裁剪过历史时各自保留最后一条前置切换。
pub(super) fn collect_retained_updates<'a>(
    model_switches: &'a [ModelSwitchRecord],
    permission_switches: &'a [PermissionSwitchRecord],
    omitted_turns: usize,
    start_after: i64,
    min_sequence: i64,
) -> Vec<SystemUpdateRef<'a>> {
    let mut updates = Vec::new();
    if omitted_turns > 0 {
        if let Some(prior) = model_switches
            .iter()
            .rfind(|sw| sw.sequence > start_after && sw.sequence < min_sequence)
        {
            updates.push(SystemUpdateRef {
                sequence: prior.sequence,
                injected_prompt: &prior.injected_prompt,
                order: 0,
            });
        }
        if let Some(prior) = permission_switches
            .iter()
            .rfind(|sw| sw.sequence > start_after && sw.sequence < min_sequence)
        {
            updates.push(SystemUpdateRef {
                sequence: prior.sequence,
                injected_prompt: &prior.injected_prompt,
                order: 1,
            });
        }
    }
    for sw in model_switches
        .iter()
        .filter(|sw| sw.sequence > min_sequence)
    {
        updates.push(SystemUpdateRef {
            sequence: sw.sequence,
            injected_prompt: &sw.injected_prompt,
            order: 0,
        });
    }
    for sw in permission_switches
        .iter()
        .filter(|sw| sw.sequence > min_sequence)
    {
        updates.push(SystemUpdateRef {
            sequence: sw.sequence,
            injected_prompt: &sw.injected_prompt,
            order: 1,
        });
    }
    updates.sort_by_key(|item| (item.sequence, item.order));
    updates
}
