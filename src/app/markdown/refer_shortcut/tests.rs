use super::*;

/// 光标放在 `marker` 之后的便捷构造。
fn cursor_after(text: &str, marker: &str) -> usize {
    text.find(marker).expect("marker present") + marker.len()
}

fn mention(path: &str, directory: bool) -> MentionEntry {
    MentionEntry {
        path: path.to_string(),
        is_directory: directory,
    }
}

#[test]
fn trigger_requires_two_chars() {
    assert_eq!(refer_trigger("a", 1), None);
    let trigger = refer_trigger("ac", 2).expect("trigger");
    assert_eq!(trigger.query, "ac");
    assert_eq!(trigger.range, 0..2);
}

#[test]
fn trigger_takes_contiguous_path_token() {
    // 无分隔符时整段连续路径字符都是触发词；空白边界处只取最后的 token。
    let text = "hello/src/na";
    let trigger = refer_trigger(text, text.len()).expect("trigger");
    assert_eq!(trigger.query, "hello/src/na");
    assert_eq!(trigger.range, 0..text.len());

    let spaced = "hello src/na";
    let trigger = refer_trigger(spaced, spaced.len()).expect("trigger");
    assert_eq!(trigger.query, "src/na");
    assert_eq!(trigger.range, 6..spaced.len());
}

#[test]
fn trigger_blocked_after_at_and_inside_marker() {
    assert_eq!(refer_trigger("@ac", 3), None);
    let text = "x @[refer-file](/tmp/ab)";
    assert_eq!(refer_trigger(text, text.len()), None);
    // 标记内部也不触发。
    let inside = text.find("tmp").expect("path") + 2;
    assert_eq!(refer_trigger(text, inside), None);
}

#[test]
fn trigger_blocked_in_fence_and_structural_lines() {
    let fenced = "```\nabc\n```";
    assert_eq!(refer_trigger(fenced, 7), None);
    let block = "&&& cmd\n&&& cmd --end";
    assert_eq!(refer_trigger(block, block.find('\n').expect("line")), None);
    let separator = "&&& cmd\n\nac\n&&& cmd --end";
    let body = separator.find("ac").expect("body") + 2;
    assert!(refer_trigger(separator, body).is_some());
}

#[test]
fn trigger_blocked_on_title_and_wt_structural_lines() {
    let title = "&&& cmd\n{title: abc}\n&&& cmd --end";
    let in_title = title.find("abc").expect("title text") + 2;
    assert_eq!(refer_trigger(title, in_title), None);
    assert_eq!(refer_scope(title, in_title), None);
    let binding = "&&& cmd\n{wt:main}\n&&& cmd --end";
    let in_binding = binding.find("main").expect("branch") + 2;
    assert_eq!(refer_trigger(binding, in_binding), None);
    assert_eq!(refer_scope(binding, in_binding), None);
}

#[test]
fn scope_isolates_template_blocks() {
    let text = "outside @abc\n&&& one\n@one\n&&& one --end\n&&& two\nbody\n&&& two --end";
    let first = cursor_after(text, "@one");
    let scope = refer_scope(text, first).expect("block scope");
    let candidates = refer_candidates(text, &scope, &(first..first), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@one"]);
    // 块外作用域可读到全文候选。
    let outside = text.find("outside").expect("outside") + 3;
    let whole = refer_scope(text, outside).expect("whole scope");
    let candidates = refer_candidates(text, &whole, &(outside..outside), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    // 普通文本区收全文引用（含块内已有引用）。
    assert_eq!(texts, vec!["@abc", "@one"]);
}

#[test]
fn candidates_dedupe_and_keep_first_order() {
    let text = "@abc x @def @abc @[refer-image](/tmp/a.png) @def";
    let scope = 0..text.len();
    let candidates = refer_candidates(text, &scope, &(text.len()..text.len()), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@abc", "@def", "@[refer-image](/tmp/a.png)"]);
    assert_eq!(candidates[2].tag, ReferTag::Refer(ReferKind::Image));
    assert_eq!(candidates[2].name, "a.png");
}

#[test]
fn candidates_mark_folders_and_skip_structural_markers() {
    let text = "@dir/ @start @end @plain";
    let candidates = refer_candidates(
        text,
        &(0..text.len()),
        &(text.len()..text.len()),
        &[mention("dir", true)],
    );
    let rows: Vec<(&str, ReferTag)> = candidates
        .iter()
        .map(|item| (item.text.as_str(), item.tag))
        .collect();
    assert_eq!(
        rows,
        vec![("@dir/", ReferTag::Folder), ("@plain", ReferTag::File)]
    );
}

#[test]
fn candidates_detect_folder_from_scan_entries() {
    let text = "@src";
    let candidates = refer_candidates(
        text,
        &(0..text.len()),
        &(text.len()..text.len()),
        &[mention("src", true)],
    );
    assert_eq!(candidates[0].tag, ReferTag::Folder);
}

#[test]
fn filter_prioritizes_name_prefix_then_subsequence() {
    let mut candidates = vec![
        ReferCandidate {
            text: "@abc".to_string(),
            name: "abc".to_string(),
            path: "abc".to_string(),
            tag: ReferTag::File,
            order: 0,
        },
        ReferCandidate {
            text: "@a-b-c".to_string(),
            name: "a-b-c".to_string(),
            path: "a-b-c".to_string(),
            tag: ReferTag::File,
            order: 1,
        },
        ReferCandidate {
            text: "@zzz".to_string(),
            name: "zzz".to_string(),
            path: "zzz".to_string(),
            tag: ReferTag::File,
            order: 2,
        },
    ];
    let filtered = filter_refer_candidates(candidates.clone(), "ac");
    let texts: Vec<&str> = filtered.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@abc", "@a-b-c"]);

    // 名称完全匹配优先于前缀；同分保持首次出现顺序。
    candidates[1].name = "ac".to_string();
    candidates[1].path = "ac".to_string();
    let filtered = filter_refer_candidates(candidates, "ac");
    assert_eq!(filtered[0].text, "@a-b-c");
    assert_eq!(filtered[1].text, "@abc");
}

#[test]
fn filter_empty_query_returns_empty() {
    let candidates = vec![ReferCandidate {
        text: "@abc".to_string(),
        name: "abc".to_string(),
        path: "abc".to_string(),
        tag: ReferTag::File,
        order: 0,
    }];
    assert!(filter_refer_candidates(candidates, "").is_empty());
}

#[test]
fn panel_wraps_highlight() {
    let items = ["@a", "@b", "@c"]
        .iter()
        .enumerate()
        .map(|(order, text)| ReferCandidate {
            text: text.to_string(),
            name: text.to_string(),
            path: text.to_string(),
            tag: ReferTag::File,
            order,
        })
        .collect();
    let trigger = ReferTrigger {
        range: 0..2,
        query: "ac".to_string(),
    };
    let mut panel = ReferPanel::new(trigger, items);
    assert_eq!(panel.active(), 0);
    panel.move_active(-1);
    assert_eq!(panel.active(), 2);
    panel.move_active(1);
    assert_eq!(panel.active(), 0);
}

#[test]
fn insertion_appends_trailing_space() {
    let candidate = ReferCandidate {
        text: "@abc".to_string(),
        name: "abc".to_string(),
        path: "abc".to_string(),
        tag: ReferTag::File,
        order: 0,
    };
    assert_eq!(refer_insertion(&candidate), "@abc ");
}
