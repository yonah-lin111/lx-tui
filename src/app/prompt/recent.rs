//! 最近修改页面的派生模型：跨条目汇总已入库页，按修改时间倒序供底部抽屉渲染。

use crate::app::markdown::TemplateStatus;
use crate::platform::format_relative_time;

use super::{PromptEditing, PromptPageId, PromptPanel, page_display_title, page_title_status};

/// 抽屉最多展示的条目数（每项占 2 行）。
pub const RECENT_VISIBLE_ENTRIES: usize = 3;
/// 抽屉展开时至少保留的条目高度；记录不足时也留出第二行位置。
pub const RECENT_MIN_VISIBLE_ENTRIES: usize = 2;
/// 参与排序的最近页面上限。
pub const MAX_RECENT_PAGES: usize = 20;

/// 抽屉一行（渲染占 2 行）：条目、页、显示标题、状态与相对时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentPageRow {
    pub item_id: i64,
    pub page_id: PromptPageId,
    pub prompt_name: String,
    pub page_title: String,
    pub status: Option<TemplateStatus>,
    pub updated_at: i64,
    pub time_label: String,
    /// 是否为当前正在编辑的条目 + 页。
    pub current: bool,
}

/// 实时编辑上下文：编辑目标、当前页与实时正文（用于抽屉里的标题实时刷新）。
pub type LiveEditor<'a> = (PromptEditing, PromptPageId, &'a str);

/// 汇总当前工作区全部已入库页，按 `updated_at` 倒序（同刻按页 id 倒序），截断到 20 条。
pub fn recent_page_rows(
    panel: &PromptPanel,
    live: Option<LiveEditor<'_>>,
    now_secs: i64,
) -> Vec<RecentPageRow> {
    let mut rows = Vec::new();
    for item in panel.tree.items() {
        for (index, page) in item.pages.iter().enumerate() {
            if page.id.saved().is_none() {
                continue;
            }
            let content = match live {
                Some((PromptEditing::Item(item_id), page_id, text))
                    if item_id == item.id && page_id == page.id =>
                {
                    text
                }
                _ => page.content.as_str(),
            };
            let (_, status) = page_title_status(content);
            let current = matches!(
                live,
                Some((PromptEditing::Item(item_id), page_id, _))
                    if item_id == item.id && page_id == page.id
            );
            rows.push(RecentPageRow {
                item_id: item.id,
                page_id: page.id,
                prompt_name: item.name.clone(),
                page_title: page_display_title(page, index, content),
                status,
                updated_at: page.updated_at,
                time_label: format_relative_time(page.updated_at, now_secs),
                current,
            });
        }
    }
    rows.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.page_id.saved().cmp(&left.page_id.saved()))
    });
    rows.truncate(MAX_RECENT_PAGES);
    rows
}

/// 已入库页总数（抽屉高度只关心前 3 条，超过 3 条按 3 条计）。
pub fn recent_page_count(panel: &PromptPanel) -> usize {
    panel
        .tree
        .items()
        .iter()
        .map(|item| {
            item.pages
                .iter()
                .filter(|page| page.id.saved().is_some())
                .count()
        })
        .sum()
}

#[cfg(test)]
mod tests;
