//! `diff` 模块测试：行号、hunk 分组、EOF、CRLF/BOM 无关性、计数与截断互不影响。

use super::*;

/// 行集合构造：便于在断言里写 `lines(&["a", "b"])`。
fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// 简化用：替换一次的计数。
fn counts_of(diff: &FileDiff, counts: DiffCounts) -> (usize, usize) {
    assert!(diff.counts_known() || !counts.truncated);
    (counts.added, counts.removed)
}

/// 单行替换：相邻删除 + 新增，行号各自正确。
#[test]
fn single_line_replacement_has_real_line_numbers() {
    let old = lines(&["a", "b", "c"]);
    let new = lines(&["a", "B", "c"]);
    let (diff, counts) = diff_lines(&old, &new, true, true, 5, 5);
    assert_eq!((counts.added, counts.removed), (1, 1));
    let hunks = diff.hunks();
    assert_eq!(hunks.len(), 1);
    let body = &hunks[0].lines;
    let removed = body
        .iter()
        .find(|line| line.kind == DiffLineKind::Removed)
        .expect("removed line");
    assert_eq!((removed.old_line, removed.new_line), (Some(2), None));
    assert_eq!(removed.text, "b");
    let added = body
        .iter()
        .find(|line| line.kind == DiffLineKind::Added)
        .expect("added line");
    assert_eq!((added.old_line, added.new_line), (None, Some(2)));
    // 上下文半径 1：前后各一行。
    assert_eq!(body.len(), 4);
}

/// 20 行里改两处远距行：两个独立 hunk，中间未改行不被吞进差异。
#[test]
fn distant_changes_form_independent_hunks() {
    let old: Vec<String> = (1..=20).map(|index| format!("line {index}")).collect();
    let mut new = old.clone();
    new[1] = "CHANGED 2".into();
    new[16] = "CHANGED 17".into();
    let (diff, counts) = diff_lines(&old, &new, true, true, 100, 100);
    assert_eq!((counts.added, counts.removed), (2, 2));
    let hunks = diff.hunks();
    assert_eq!(hunks.len(), 2, "远距修改必须拆成两个 hunk");
    // 每个 hunk = 1 行上下文 + 删除 + 新增 + 1 行上下文。
    assert_eq!(hunks[0].lines.len(), 4);
    assert_eq!(hunks[1].lines.len(), 4);
    let numbers: Vec<usize> = hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter_map(|line| line.old_line)
        .collect();
    assert!(numbers.contains(&2) && numbers.contains(&17));
    assert!(!numbers.contains(&10), "中间未改行不得进入差异");
}

/// 相邻修改的范围合并成一个 hunk。
#[test]
fn adjacent_changes_merge_into_one_hunk() {
    let old = lines(&["a", "b", "c", "d", "e"]);
    let new = lines(&["a", "B", "C", "d", "e"]);
    let (diff, _) = diff_lines(&old, &new, true, true, 5, 5);
    assert_eq!(diff.hunks().len(), 1);
}

/// 重复行不会让计数或行号错位。
#[test]
fn duplicate_lines_keep_counts_consistent() {
    let old = lines(&["x", "x", "x", "y"]);
    let new = lines(&["x", "y", "x", "y"]);
    let (_, counts) = diff_lines(&old, &new, true, true, 8, 8);
    // 只断言可应用的编辑规模与总数自洽，不要求唯一最短脚本。
    assert_eq!(counts.added, counts.removed);
    assert!(counts.added <= 2);
}

/// 空文件 → 新增：空内容不产生虚假的一行。
#[test]
fn empty_old_produces_only_added_lines() {
    let old: Vec<String> = Vec::new();
    let new = lines(&["one", "two"]);
    let (diff, counts) = diff_lines(&old, &new, true, true, 0, 8);
    assert_eq!((counts.added, counts.removed), (2, 0));
    let added: Vec<Option<usize>> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind == DiffLineKind::Added)
        .map(|line| line.new_line)
        .collect();
    assert_eq!(added, vec![Some(1), Some(2)]);
}

/// 删除全部内容：只有删除行，行号取旧文件。
#[test]
fn empty_new_produces_only_removed_lines() {
    let old = lines(&["one", "two", "three"]);
    let new: Vec<String> = Vec::new();
    let (diff, counts) = diff_lines(&old, &new, true, true, 14, 0);
    assert_eq!((counts.added, counts.removed), (0, 3));
    let numbers: Vec<usize> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter_map(|line| line.old_line)
        .collect();
    assert_eq!(numbers, vec![1, 2, 3]);
}

/// 只含空行的文件：空行是真实内容。
#[test]
fn blank_lines_are_content() {
    let old = lines(&["", ""]);
    let new = lines(&[""]);
    let (_, counts) = diff_lines(&old, &new, true, true, 2, 1);
    assert_eq!((counts.added, counts.removed), (0, 1));
}

/// 中文行按字符完整保留，不按字节切。
#[test]
fn chinese_lines_are_preserved() {
    let old = lines(&["标题", "正文"]);
    let new = lines(&["标题", "正文二"]);
    let (diff, counts) = diff_lines(&old, &new, true, true, 10, 12);
    assert_eq!((counts.added, counts.removed), (1, 1));
    let added = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .find(|line| line.kind == DiffLineKind::Added)
        .expect("added");
    assert_eq!(added.text, "正文二");
}

/// 末行换行状态变化必须显示为变化，并带上 EOF 标记。
#[test]
fn eof_only_change_is_visible() {
    let old = lines(&["hello"]);
    let new = lines(&["hello"]);
    let (diff, counts) = diff_lines(&old, &new, false, true, 5, 6);
    assert_eq!((counts.added, counts.removed), (1, 1));
    let body: Vec<&DiffLine> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .collect();
    let removed = body
        .iter()
        .find(|line| line.kind == DiffLineKind::Removed)
        .expect("removed");
    assert!(removed.no_final_newline, "旧文件末行无换行");
    let added = body
        .iter()
        .find(|line| line.kind == DiffLineKind::Added)
        .expect("added");
    assert!(!added.no_final_newline, "新文件末行有换行");
}

/// 内容相同且换行状态相同：无差异。
#[test]
fn identical_content_has_no_hunks() {
    let old = lines(&["a", "b"]);
    let new = lines(&["a", "b"]);
    let (diff, counts) = diff_lines(&old, &new, true, true, 4, 4);
    assert_eq!((counts.added, counts.removed), (0, 0));
    assert!(diff.hunks().is_empty());
    assert!(!diff.truncated());
    assert!(diff.counts_known());
}

/// 超限输入走线性简化脚本：计数精确、正文可应用、标记 simplified。
#[test]
fn oversized_input_uses_linear_simplified_script() {
    let old = lines(&["a", "b", "c", "d"]);
    let new = lines(&["a", "X", "c", "d"]);
    let (diff, counts) = diff_lines(
        &old,
        &new,
        true,
        true,
        MAX_DIFF_INPUT_BYTES,
        MAX_DIFF_INPUT_BYTES,
    );
    assert!(diff.simplified());
    assert_eq!((counts.added, counts.removed), (1, 1));
    assert!(counts.simplified);
    // 公共前后缀之外的整段替换：b → X。
    let body: Vec<&DiffLine> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .collect();
    assert!(
        body.iter()
            .any(|line| line.kind == DiffLineKind::Removed && line.text == "b")
    );
    assert!(
        body.iter()
            .any(|line| line.kind == DiffLineKind::Added && line.text == "X")
    );
}

/// 大文件超限：计数仍精确，正文被行数预算截断，二者互不影响。
#[test]
fn oversized_line_count_truncates_body_but_keeps_counts() {
    let old: Vec<String> = (0..MAX_DIFF_INPUT_LINES + 10)
        .map(|index| format!("old {index}"))
        .collect();
    let new: Vec<String> = (0..MAX_DIFF_INPUT_LINES + 10)
        .map(|index| format!("new {index}"))
        .collect();
    let (diff, counts) = diff_lines(&old, &new, true, true, 1, 1);
    assert_eq!(counts.added, new.len());
    assert_eq!(counts.removed, old.len());
    assert!(diff.truncated(), "正文必须被行数预算截断");
    assert!(diff.simplified());
    let saved: usize = diff.hunks().iter().map(|hunk| hunk.lines.len()).sum();
    assert_eq!(saved, MAX_DIFF_LINES);
}

/// 行数超限且内容相同：不产生虚假差异。
#[test]
fn oversized_identical_input_has_no_hunks() {
    let old: Vec<String> = (0..MAX_DIFF_INPUT_LINES + 1)
        .map(|index| format!("line {index}"))
        .collect();
    let (diff, counts) = diff_lines(&old, &old, true, true, 1, 1);
    assert_eq!((counts.added, counts.removed), (0, 0));
    assert!(diff.hunks().is_empty());
    assert!(!diff.truncated(), "没有正文可截断");
}

/// `Unavailable` 的计数已知性为 false。
#[test]
fn unavailable_diff_reports_unknown_counts() {
    let diff = FileDiff::Unavailable {
        reason: DiffUnavailableReason::Binary,
    };
    assert!(!diff.counts_known());
    assert!(diff.hunks().is_empty());
    assert!(!diff.simplified());
}

/// 稳定原因码与行种类可往返。
#[test]
fn stable_strings_round_trip() {
    for kind in [
        DiffLineKind::Context,
        DiffLineKind::Added,
        DiffLineKind::Removed,
    ] {
        assert_eq!(DiffLineKind::from_stored(kind.as_str()), Some(kind));
    }
    for reason in [
        DiffUnavailableReason::Binary,
        DiffUnavailableReason::NotUtf8,
        DiffUnavailableReason::ReadFailed,
    ] {
        assert_eq!(
            DiffUnavailableReason::from_stored(reason.as_str()),
            Some(reason)
        );
    }
}

/// 计数辅助函数覆盖：正文截断不影响计数。
#[test]
fn counts_and_body_are_independent() {
    let old = lines(&["a", "b"]);
    let new = lines(&["a", "b", "c"]);
    let (diff, counts) = diff_lines(&old, &new, true, true, 3, 5);
    assert_eq!(counts_of(&diff, counts), (1, 0));
    assert_eq!(counts.truncated, diff.truncated());
}

/// CRLF 文件按逻辑行比较：换行风格差异不当作全文件改动（调用方传入已切分的行）。
#[test]
fn newline_style_is_not_part_of_lines() {
    let old = lines(&["one", "two"]);
    let new = lines(&["one", "two"]);
    let (diff, counts) = diff_lines(&old, &new, true, true, 9, 9);
    assert_eq!((counts.added, counts.removed), (0, 0));
    assert!(diff.hunks().is_empty());
}
