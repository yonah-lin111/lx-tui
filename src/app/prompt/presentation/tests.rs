//! 附件索引与显示投影测试：归属、降级、空白收敛与可见行映射。

use super::*;

/// 三父任务样例：无附件、20 个附件、仅正文。
fn three_tasks(supplements: usize) -> String {
    let mut text = String::from("&&& a --start\nonly body\n&&& a --end\n\n");
    text.push_str("&&& b --start\n{title: }\nbody b\n\n");
    for index in 0..supplements {
        text.push_str(&format!(
            "+++ supplement --start\n{{title: S{index}}}\nline {index}\n+++ supplement --end\n\n"
        ));
    }
    text.push_str("&&& b --end\n\n&&& c --start\ntail\n&&& c --end");
    text
}

#[test]
fn index_collects_only_closed_children_inside_closed_parents() {
    let text = three_tasks(3);
    let index = supplement_index(&text);
    assert_eq!(index.entries.len(), 3);
    assert!(
        index
            .parents()
            .iter()
            .all(|parent| index.count_for(*parent) == 3)
    );
    for entry in &index.entries {
        let snapshot = &text[entry.range.clone()];
        assert!(snapshot.starts_with("+++ supplement --start"), "{snapshot}");
        assert!(snapshot.ends_with("+++ supplement --end"), "{snapshot}");
    }
    assert_eq!(
        index
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect::<Vec<_>>(),
        vec!["S0", "S1", "S2"]
    );
}

#[test]
fn unclosed_child_or_parent_is_not_attached() {
    let unclosed_child = "&&& a --start\n+++ supplement --start\nbody\n&&& a --end";
    assert!(supplement_index(unclosed_child).entries.is_empty());
    let unclosed_parent = "&&& a --start\n+++ supplement --start\nbody\n+++ supplement --end";
    assert!(supplement_index(unclosed_parent).entries.is_empty());
}

#[test]
fn fenced_and_orphan_plus_markers_stay_visible() {
    let text = concat!(
        "&&& a --start\n",
        "```\n",
        "+++ supplement --start\nbody\n+++ supplement --end\n",
        "```\n",
        "&&& a --end\n",
        "+++ orphan --start\noutside\n+++ orphan --end",
    );
    assert!(supplement_index(text).entries.is_empty());
}

#[test]
fn empty_title_falls_back_to_ordinal_and_duplicates_keep_identity() {
    let text = concat!(
        "&&& a --start\n",
        "+++ supplement --start\n\none\n+++ supplement --end\n",
        "+++ supplement --start\n{title: same}\ntwo\n+++ supplement --end\n",
        "+++ supplement --start\n{title: same}\nthree\n+++ supplement --end\n",
        "&&& a --end",
    );
    let index = supplement_index(text);
    assert_eq!(index.entries[0].title, "Supplement 1");
    assert_eq!(index.entries[1].title, "same");
    assert_eq!(index.entries[2].title, "same");
    assert_ne!(index.entries[1].range, index.entries[2].range);
}

#[test]
fn compact_projection_has_one_entry_per_parent_and_hides_children() {
    let text = three_tasks(20);
    let projection = project_compact(&text, 60);
    let entries: Vec<usize> = projection
        .rows
        .iter()
        .filter_map(|row| match row {
            PromptDisplayRow::Supplements { count, .. } => Some(*count),
            PromptDisplayRow::Text { .. } => None,
        })
        .collect();
    assert_eq!(entries, vec![20], "只有一个父任务有附件");
    assert!(
        projection.rows.len() < text.split('\n').count(),
        "20 个附件压缩为一行入口"
    );
    // 后续正文仍可定位：第三个任务的文本行仍在投影中。
    assert!(projection.rows.iter().any(|row| match row {
        PromptDisplayRow::Text { start, .. } => text[*start..].starts_with("tail"),
        _ => false,
    }));
}

#[test]
fn entry_is_placed_before_parent_end_line() {
    let text =
        "&&& a --start\nbody\n+++ supplement --start\ninner\n+++ supplement --end\n&&& a --end";
    let projection = project_compact(text, 60);
    let entry = projection
        .rows
        .iter()
        .position(|row| matches!(row, PromptDisplayRow::Supplements { .. }))
        .expect("entry row");
    let rows: Vec<String> = projection
        .rows
        .iter()
        .map(|row| match row {
            PromptDisplayRow::Text { start, end, .. } => text[*start..*end].to_string(),
            PromptDisplayRow::Supplements { .. } => "<entry>".into(),
        })
        .collect();
    assert_eq!(rows[entry], "<entry>");
    assert_eq!(rows[entry + 1], "&&& a --end");
}

#[test]
fn blank_lines_adjacent_to_hidden_blocks_collapse() {
    let text = concat!(
        "&&& a --start\n",
        "A\n",
        "\n",
        "+++ supplement --start\nhidden\n+++ supplement --end\n",
        "\n",
        "B\n",
        "&&& a --end",
    );
    let projection = project_compact(text, 60);
    let visible: Vec<&str> = projection
        .rows
        .iter()
        .filter_map(|row| match row {
            PromptDisplayRow::Text { start, end, .. } => Some(&text[*start..*end]),
            _ => None,
        })
        .collect();
    assert_eq!(visible, vec!["&&& a --start", "A", "B", "&&& a --end"]);
}

#[test]
fn source_projection_keeps_everything_and_never_hides() {
    let text = three_tasks(3);
    let projection = project_source(&text, 60);
    assert!(
        projection
            .rows
            .iter()
            .all(|row| matches!(row, PromptDisplayRow::Text { .. }))
    );
    assert_eq!(projection.rows.len(), text.split('\n').count());
}

#[test]
fn wide_chars_wrap_by_display_width() {
    // 普通行按 6 列换行：4 个全角字 → 3 字 + 1 字，不按字节切断。
    let text = "中文字符";
    let projection = project_source(text, 6);
    let rows: Vec<&str> = projection
        .rows
        .iter()
        .filter_map(|row| match row {
            PromptDisplayRow::Text { start, end, .. } => Some(&text[*start..*end]),
            _ => None,
        })
        .collect();
    assert_eq!(rows, vec!["中文字", "符"]);
}

#[test]
fn hidden_range_hit_detection_covers_edits_and_insertions() {
    let text =
        "&&& a --start\nA\n+++ supplement --start\nhidden\n+++ supplement --end\nB\n&&& a --end";
    let index = supplement_index(text);
    let entry = &index.entries[0];
    assert!(range_hits_hidden(
        &index,
        entry.range.start + 1,
        entry.range.end - 1
    ));
    assert!(insertion_in_hidden(&index, entry.range.start + 1));
    assert!(!insertion_in_hidden(&index, entry.range.start));
    assert!(!range_hits_hidden(&index, 0, 2));
}

#[test]
fn hundred_supplements_keep_single_entry() {
    let text = three_tasks(100);
    let projection = project_compact(&text, 80);
    let count = projection
        .rows
        .iter()
        .filter(|row| matches!(row, PromptDisplayRow::Supplements { .. }))
        .count();
    assert_eq!(count, 1);
}
