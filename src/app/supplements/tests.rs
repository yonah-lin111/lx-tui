//! 附件会话与局部提交测试：目标快照、标题/正文重建、目标失效与撤销边界。

use super::*;
use crate::app::prompt::presentation::{prompt_display_row_count, supplement_index};
use crate::app::prompt::{Prompt, PromptPresentation};

fn editor(text: &str) -> Prompt {
    let mut prompt = Prompt::new(crate::layout::PaneId::from_raw_for_test(7));
    prompt.resize(60, 20);
    prompt.set_text(text);
    prompt
}

fn first_target(prompt: &Prompt) -> SupplementTarget {
    let index = supplement_index(prompt.text());
    let entry = index.entries.first().expect("entry");
    SupplementTarget::from_entry(prompt.text(), entry).expect("target")
}

#[test]
fn target_splits_title_and_body_without_touching_markers() {
    let text = concat!(
        "&&& a --start\n",
        "+++ supplement --start\n",
        "{title: Review}\n",
        "\n",
        "- keep me\n",
        "\n",
        "+++ supplement --end\n",
        "&&& a --end",
    );
    let prompt = editor(text);
    let target = first_target(&prompt);
    assert_eq!(&prompt.text()[target.title.clone().unwrap()], "Review");
    assert_eq!(&prompt.text()[target.body.clone()], "- keep me");
    assert_eq!(target.label, "Review");
    assert!(target.snapshot.starts_with("+++ supplement --start"));
}

#[test]
fn draft_save_changes_only_target_block_and_keeps_markers() {
    let text = concat!(
        "&&& a --start\n",
        "+++ supplement --start\n",
        "{title: Old}\n",
        "\n",
        "body\n",
        "{wt:main}\n",
        "+++ supplement --end\n",
        "&&& a --end\n",
        "tail",
    );
    let mut prompt = editor(text);
    let target = first_target(&prompt);
    let mut draft = SupplementDraft::existing(prompt.text(), target.clone());
    draft.title = crate::app::overlay::TextInput::new("New");
    draft.body.set_text("changed\nmore");

    assert_eq!(apply_draft(&mut prompt, &draft), SaveOutcome::Changed);
    assert!(prompt.text().contains("{title: New}"));
    assert!(prompt.text().contains("changed\nmore"));
    assert!(prompt.text().contains("{wt:main}"), "绑定元数据保留");
    assert!(prompt.text().contains("&&& a --end\ntail"), "块外文本不变");
    assert!(prompt.can_undo());
    prompt.undo();
    assert_eq!(prompt.text(), text, "一次撤销恢复整份附件修改");
}

#[test]
fn unchanged_draft_does_not_create_history_or_dirty() {
    let text = "&&& a --start\n+++ supplement --start\n{title: T}\n\nbody\n+++ supplement --end\n&&& a --end";
    let mut prompt = editor(text);
    let target = first_target(&prompt);
    let draft = SupplementDraft::existing(prompt.text(), target);
    assert!(!draft.dirty());
    assert_eq!(apply_draft(&mut prompt, &draft), SaveOutcome::Unchanged);
    assert!(!prompt.can_undo());
    assert!(prompt.is_saved());
}

#[test]
fn stale_snapshot_is_rejected_without_writing() {
    let text = "&&& a --start\n+++ supplement --start\n{title: T}\n\nbody\n+++ supplement --end\n&&& a --end";
    let mut prompt = editor(text);
    let target = first_target(&prompt);
    let mut draft = SupplementDraft::existing(prompt.text(), target);
    draft.body.set_text("edits");

    // 模拟打开后外部改写了目标块（加载新文本，revision 不增）。
    prompt.set_text(
        "&&& a --start\n+++ supplement --start\n{title: T}\n\nother\n+++ supplement --end\n&&& a --end",
    );
    assert_eq!(apply_draft(&mut prompt, &draft), SaveOutcome::TargetChanged);
    assert!(prompt.text().contains("other"));
}

#[test]
fn empty_body_rebuild_keeps_block_structure() {
    let text = "&&& a --start\n+++ supplement --start\n{title: T}\n\nbody\n+++ supplement --end\n&&& a --end";
    let mut prompt = editor(text);
    let target = first_target(&prompt);
    let mut draft = SupplementDraft::existing(prompt.text(), target);
    draft.body.set_text("");
    assert_eq!(apply_draft(&mut prompt, &draft), SaveOutcome::Changed);
    assert!(prompt.text().contains("+++ supplement --end"));
    assert_eq!(supplement_index(prompt.text()).entries.len(), 1);
}

#[test]
fn insert_new_supplement_adds_block_before_parent_end() {
    let text = "&&& a --start\nbody\n&&& a --end";
    let mut prompt = editor(text);
    let draft = SupplementDraft::blank("New", "content", 0);
    let block = new_block_text(&draft, "supplement");
    assert!(prompt.insert_supplement_before(2, &block));
    assert_eq!(supplement_index(prompt.text()).entries.len(), 1);
    assert!(prompt.text().contains("{title: New}"));
    prompt.undo();
    assert_eq!(prompt.text(), text, "新建保存是一步撤销");
}

#[test]
fn compact_editor_reports_entry_focus_after_hidden_cursor() {
    let text =
        "&&& a --start\nbody\n+++ supplement --start\nhidden\n+++ supplement --end\n&&& a --end";
    let mut prompt = editor(text);
    let hidden = prompt.text().find("hidden").expect("hidden") + 2;
    prompt.set_cursor(hidden);
    prompt.set_presentation(PromptPresentation::Compact);
    assert!(prompt.focus().is_supplements(), "隐藏区光标转为入口焦点");
    let rows = prompt.display_rows();
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(
                row,
                crate::app::prompt::PromptDisplayRow::Supplements { .. }
            ))
            .count(),
        1,
        "只生成一个合成入口行"
    );
    assert!(
        rows.len() < prompt_display_row_count(prompt.text()),
        "隐藏块不占行"
    );
}
