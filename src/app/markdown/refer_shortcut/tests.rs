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
    let text = "outside @abc\n&&& one\n@one\n&&& one --end\n&&& two\n@two\n&&& two --end";
    let first = cursor_after(text, "@one");
    let scope = refer_scope(text, first).expect("block scope");
    let candidates = refer_candidates(text, &scope, &(first..first), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@one"], "块内只取本块正文");
    let other = cursor_after(text, "@two");
    let scope = refer_scope(text, other).expect("block scope");
    let candidates = refer_candidates(text, &scope, &(other..other), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@two"], "其他块的内容不可见");
    // 块外作用域只收块外候选：块内引用不泄漏。
    let outside = text.find("outside").expect("outside") + 3;
    let whole = refer_scope(text, outside).expect("plain scope");
    let candidates = refer_candidates(text, &whole, &(outside..outside), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@abc"], "块内 @one / @two 不泄漏到块外");
}

#[test]
fn scope_keeps_unclosed_blocks_separate() {
    // 未闭合的外层块不把后面的新块并入自己的正文。
    let text = "&&& one\n@one\n&&& two\n@two\nac";
    let cursor = text.len();
    let scope = refer_scope(text, cursor).expect("block scope");
    let candidates = refer_candidates(text, &scope, &(cursor..cursor), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@two"], "内层块只取自己的正文");
    let outer = cursor_after(text, "@one");
    let scope = refer_scope(text, outer).expect("block scope");
    let candidates = refer_candidates(text, &scope, &(outer..outer), &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@one"], "外层块看不到后面的块");
}

#[test]
fn candidates_dedupe_and_keep_first_order() {
    let text = "@abc x @def @abc @[refer-image](/tmp/a.png) @def";
    let scope = 0..text.len();
    let candidates = refer_candidates(
        text,
        std::slice::from_ref(&scope),
        &(text.len()..text.len()),
        &[],
    );
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@abc", "@def", "@[refer-image](/tmp/a.png)"]);
    assert_eq!(candidates[2].tag, ReferTag::Refer(ReferKind::Image));
    assert_eq!(candidates[2].name, "a.png");
}

#[test]
fn candidates_mark_folders_and_skip_structural_markers() {
    let text = "@dir/ @start @end @plain";
    let scope = 0..text.len();
    let candidates = refer_candidates(
        text,
        std::slice::from_ref(&scope),
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
    let scope = 0..text.len();
    let candidates = refer_candidates(
        text,
        std::slice::from_ref(&scope),
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
fn insertion_uses_short_ref_with_trailing_space() {
    let candidate = ReferCandidate {
        text: "@abc".to_string(),
        name: "abc".to_string(),
        path: "a/b/abc.md".to_string(),
        tag: ReferTag::File,
        order: 0,
    };
    assert_eq!(refer_insertion(&candidate), "@ref/b/abc.md ");

    let folder = ReferCandidate {
        text: "@[refer-folder](/x/y/z)".to_string(),
        name: "z".to_string(),
        path: "/x/y/z".to_string(),
        tag: ReferTag::Refer(ReferKind::Folder),
        order: 0,
    };
    assert_eq!(refer_insertion(&folder), "@ref/y/z/ ", "文件夹带尾斜杠");

    let single = ReferCandidate {
        text: "@main.rs".to_string(),
        name: "main.rs".to_string(),
        path: "main.rs".to_string(),
        tag: ReferTag::File,
        order: 0,
    };
    assert_eq!(
        refer_insertion(&single),
        "@ref/main.rs ",
        "单段路径也能短化"
    );
}

#[test]
fn candidates_skip_fenced_mentions() {
    let text = "```\n@abc\n```\n@def\nac";
    let cursor = text.len();
    let scope = refer_scope(text, cursor).expect("scope");
    let trigger = refer_trigger(text, cursor).expect("trigger");
    let candidates = refer_candidates(text, &scope, &trigger.range, &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, vec!["@def"], "围栏内 @abc 不是提及，不作为候选");
}

#[test]
fn candidates_order_follows_position_across_mention_and_reference() {
    let text = "@[refer-file](/tmp/abc) @abc ac";
    let cursor = text.len();
    let scope = refer_scope(text, cursor).expect("scope");
    let trigger = refer_trigger(text, cursor).expect("trigger");
    let candidates = refer_candidates(text, &scope, &trigger.range, &[]);
    let filtered = filter_refer_candidates(candidates, "abc");
    let texts: Vec<&str> = filtered.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["@[refer-file](/tmp/abc)", "@abc"],
        "同名同分时按正文首次出现排序（引用在前）"
    );
}

#[test]
fn trigger_and_scope_work_inside_supplement_block_within_template() {
    let text = "&&& task --start\n- Reference: @docs/feature/prompt-per-workspace/\n+++ supplement --start\n{title: Review}\npro\n+++ supplement --end\n&&& task --end";
    let cursor = text.rfind("pro").expect("pro") + 3;
    let trigger = refer_trigger(text, cursor).expect("trigger inside supplement");
    assert_eq!(trigger.query, "pro");

    let scope = refer_scope(text, cursor).expect("scope inside supplement");
    let candidates = refer_candidates(text, &scope, &trigger.range, &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["@docs/feature/prompt-per-workspace/"],
        "补充块内能够访问到外层模板块中的引用"
    );
}

#[test]
fn supplement_references_are_visible_bidirectionally() {
    let text = "&&& task --start\n+++ s1 --start\n{title: S1}\n@docs/inside-s1.md\n+++ s1 --end\n+++ s2 --start\n{title: S2}\nins\n+++ s2 --end\nouter ins\n&&& task --end";
    // s2 内部能看到 s1 的引用
    let cursor_in_s2 = text.find("ins\n").expect("ins in s2") + 3;
    let scope_s2 = refer_scope(text, cursor_in_s2).expect("scope in s2");
    let candidates_s2 = refer_candidates(text, &scope_s2, &(cursor_in_s2 - 3..cursor_in_s2), &[]);
    assert_eq!(
        candidates_s2
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["@docs/inside-s1.md"],
        "s2 内部能够访问 s1 中声明的引用"
    );

    // 外层正文也能看到 s1 的引用
    let cursor_outer = text.find("outer ins").expect("outer ins") + "outer ins".len();
    let scope_outer = refer_scope(text, cursor_outer).expect("scope outer");
    let candidates_outer =
        refer_candidates(text, &scope_outer, &(cursor_outer - 3..cursor_outer), &[]);
    assert_eq!(
        candidates_outer
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["@docs/inside-s1.md"],
        "外层主体能访问补充块内声明的引用"
    );
}

#[test]
fn trigger_blocked_on_supplement_structural_lines() {
    let text = "&&& task --start\n+++ supplement --start\n{title: Review}\nbody\n+++ supplement --end\n&&& task --end";
    // +++ 起始行不触发
    let in_start = text.find("+++ supplement --start").expect("start") + 5;
    assert_eq!(refer_trigger(text, in_start), None);
    assert_eq!(refer_scope(text, in_start), None);

    // 标题行不触发
    let in_title = text.find("Review").expect("title") + 2;
    assert_eq!(refer_trigger(text, in_title), None);
    assert_eq!(refer_scope(text, in_title), None);

    // +++ 结束行不触发
    let in_end = text.find("+++ supplement --end").expect("end") + 5;
    assert_eq!(refer_trigger(text, in_end), None);
    assert_eq!(refer_scope(text, in_end), None);
}

#[test]
fn candidates_dedupe_when_mention_and_short_ref_refer_to_same_folder() {
    // 对应用户反馈的 bug 场景：先写 @docs/...，再使用快捷引用插入 @ref/...，输入 pro 再次唤起
    let text = "- Reference: @docs/feature/prompt-per-workspace/\n- @ref/feature/prompt-per-workspace/\n- pro";
    let cursor = text.len();
    let trigger = refer_trigger(text, cursor).expect("trigger");
    assert_eq!(trigger.query, "pro");

    let scope = refer_scope(text, cursor).expect("scope");
    let candidates = refer_candidates(text, &scope, &trigger.range, &[]);
    let filtered = filter_refer_candidates(candidates, &trigger.query);

    // 只有首个出现的完整引用条目被保留，短引用不会形成重复条目
    let texts: Vec<&str> = filtered.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["@docs/feature/prompt-per-workspace/"],
        "同一目标的短引用被去重，不出现重复条目"
    );
    assert_eq!(
        refer_insertion(&filtered[0]),
        "@ref/feature/prompt-per-workspace/ ",
        "确认插入结果仍为规范短引用"
    );
}

#[test]
fn candidates_keep_distinct_when_folder_and_file_share_stem() {
    let text = "@docs/feature/name/ @docs/feature/name na";
    let cursor = text.len();
    let trigger = refer_trigger(text, cursor).expect("trigger");
    let scope = refer_scope(text, cursor).expect("scope");
    let candidates = refer_candidates(text, &scope, &trigger.range, &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["@docs/feature/name/", "@docs/feature/name"],
        "同名但类型不同（文件夹 vs 文件）各自保留"
    );
}

#[test]
fn candidates_keep_different_files_with_same_tail_segments() {
    // 两个不同包下的 index.ts 文件，末两段均为 src/index.ts，但它们是不同的独立文件，绝不能误去重
    let text = "@packages/a/src/index.ts @packages/b/src/index.ts ind";
    let cursor = text.len();
    let trigger = refer_trigger(text, cursor).expect("trigger");
    let scope = refer_scope(text, cursor).expect("scope");
    let candidates = refer_candidates(text, &scope, &trigger.range, &[]);
    let texts: Vec<&str> = candidates.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["@packages/a/src/index.ts", "@packages/b/src/index.ts"],
        "不同路径的独立文件即使末两段相同也必须全部保留"
    );
}
