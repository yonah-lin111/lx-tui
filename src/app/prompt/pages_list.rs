//! 页列表行派生模型：从页正文提取显示标题与 `&&&` 状态，并按显示标题过滤。

use crate::app::markdown::{self, BlockKind, TemplateStatus};
use crate::ui::text;

use super::tree::{PromptPage, PromptPageId, name_matches};

/// 页列表一行的派生数据：页 id、显示标题、`&&&` 状态（无 `&&&` 为 None）、是否当前页。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRow {
    pub page: PromptPageId,
    pub title: String,
    pub status: Option<TemplateStatus>,
    pub current: bool,
}

/// 页正文的显示标题与状态：独立 `{title:}` → 行内 `{title:}` → 模板名 → None（调用方回退页名）。
pub fn page_title_status(content: &str) -> (Option<String>, Option<TemplateStatus>) {
    let scan = markdown::block_scan(content);
    let Some(span) = scan
        .spans
        .iter()
        .find(|s| s.kind == BlockKind::Template && s.closed)
        .or_else(|| scan.spans.iter().find(|s| s.kind == BlockKind::Template))
    else {
        return (None, None);
    };
    let lines: Vec<&str> = content.split('\n').collect();
    let start = lines
        .get(span.start)
        .and_then(|line| markdown::parse_template_start_line(line));
    let standalone = lines
        .get(span.start + 1)
        .and_then(|line| markdown::parse_title_line(line))
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let inline = start
        .and_then(|start| start.title)
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let command = start.map(|start| start.command);
    let title = standalone.or(inline).or(command).map(str::to_string);
    (title, Some(span.status))
}

/// 页显示标题：优先块标题，其次页名，空页名回退 `Page N`。
pub fn page_display_title(page: &PromptPage, index: usize, content: &str) -> String {
    let (block_title, _) = page_title_status(content);
    block_title.unwrap_or_else(|| {
        let trimmed = page.name.trim();
        if trimmed.is_empty() {
            text::default_page_name(index + 1)
        } else {
            trimmed.to_string()
        }
    })
}

/// 页列表全部行（按页顺序），按显示标题过滤（大小写不敏感子串，复用 `name_matches`）。
pub fn page_rows(pages: &[PromptPage], active: PromptPageId, query: &str) -> Vec<PageRow> {
    let query = query.trim().to_lowercase();
    pages
        .iter()
        .enumerate()
        .filter_map(|(index, page)| {
            let title = page_display_title(page, index, &page.content);
            let (_, status) = page_title_status(&page.content);
            name_matches(&title, &query).then_some(PageRow {
                page: page.id,
                title,
                status,
                current: page.id == active,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
