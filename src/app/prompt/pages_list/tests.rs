//! 页列表行模型测试：标题提取回退链、状态与过滤。

use super::*;

#[test]
fn standalone_title_takes_highest_priority() {
    let content = "&&& refactorTemplate --start {title: inline}\n{title: 重构}\n&&& refactorTemplate --end in_progress";
    let (title, status) = page_title_status(content);
    assert_eq!(title.as_deref(), Some("重构"));
    assert_eq!(status, Some(TemplateStatus::InProgress));
}

#[test]
fn empty_standalone_falls_back_to_inline_title() {
    let content =
        "&&& refactorTemplate --start {title:x}\n{title:   }\n&&& refactorTemplate --end done";
    let (title, status) = page_title_status(content);
    assert_eq!(title.as_deref(), Some("x"));
    assert_eq!(status, Some(TemplateStatus::Done));
}

#[test]
fn empty_both_titles_falls_back_to_command_name() {
    let content = "&&& refactorTemplate --start {title: }\n{title: }\n&&& refactorTemplate --end";
    let (title, status) = page_title_status(content);
    assert_eq!(title.as_deref(), Some("refactorTemplate"));
    assert_eq!(status, Some(TemplateStatus::Todo));
}

#[test]
fn page_without_template_falls_back_to_page_name_and_has_no_status() {
    let pages = vec![PromptPage::saved(
        1,
        "Custom Page",
        "plain text\n+++ supplement --start\n{title: S}\n+++ supplement --end",
    )];
    let rows = page_rows(&pages, PromptPageId::Saved(1), "");
    assert_eq!(
        rows,
        vec![PageRow {
            page: PromptPageId::Saved(1),
            title: "Custom Page".to_string(),
            status: None,
            current: true,
        }]
    );
}

#[test]
fn empty_page_name_falls_back_to_default_placeholder() {
    let pages = vec![PromptPage::saved(1, "   ", "plain text")];
    let rows = page_rows(&pages, PromptPageId::Saved(1), "");
    assert_eq!(rows[0].title, text::default_page_name(1));
    assert_eq!(rows[0].status, None);
}

#[test]
fn unclosed_template_reports_todo_status() {
    let content = "&&& bugTemplate --start\n{title: Fix crash}\nstill editing";
    let (title, status) = page_title_status(content);
    assert_eq!(title.as_deref(), Some("Fix crash"));
    assert_eq!(status, Some(TemplateStatus::Todo));
}

#[test]
fn second_template_title_is_ignored() {
    let content = concat!(
        "&&& addTemplate --start\n",
        "{title: }\n",
        "&&& addTemplate --end\n",
        "&&& bugTemplate --start\n",
        "{title: Second Title}\n",
        "&&& bugTemplate --end done",
    );
    let (title, status) = page_title_status(content);
    assert_eq!(title.as_deref(), Some("addTemplate"));
    assert_eq!(status, Some(TemplateStatus::Todo));
}

#[test]
fn page_rows_filters_by_display_title_case_insensitively() {
    let pages = vec![
        PromptPage::saved(
            1,
            "Page 1",
            "&&& addTemplate --start\n{title: Alpha Feature}\n&&& addTemplate --end",
        ),
        PromptPage::saved(2, "Beta Notes", "plain"),
        PromptPage::saved(
            3,
            "Page 3",
            "&&& bugTemplate --start\n{title: Gamma Bug}\n&&& bugTemplate --end done",
        ),
    ];
    let filtered = page_rows(&pages, PromptPageId::Saved(2), "BETA");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].page, PromptPageId::Saved(2));
    assert!(filtered[0].current);

    let alpha = page_rows(&pages, PromptPageId::Saved(2), "alpha");
    assert_eq!(alpha.len(), 1);
    assert_eq!(alpha[0].page, PromptPageId::Saved(1));
    assert!(!alpha[0].current);
}
