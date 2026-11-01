//! 块扫描单元测试：顶层 `&&&`（仅首个）与顶层 `+++` 平级语义。

use super::*;

#[test]
fn template_followed_by_multiple_supplements_scans_in_document_order() {
    let text = concat!(
        "&&& addTemplate --start\n",
        "{title: Task}\n",
        "- body\n",
        "&&& addTemplate --end in_progress\n",
        "\n",
        "+++ supplement --start\n",
        "{title: S1}\n",
        "- s1\n",
        "+++ supplement --end\n",
        "\n",
        "+++ extra --start\n",
        "- s2\n",
        "+++ extra --end",
    );
    let scan = block_scan(text);
    assert_eq!(
        scan.spans,
        vec![
            BlockSpan {
                kind: BlockKind::Template,
                start: 0,
                end: 3,
                status: TemplateStatus::InProgress,
                closed: true,
            },
            BlockSpan {
                kind: BlockKind::Supple,
                start: 5,
                end: 8,
                status: TemplateStatus::Todo,
                closed: true,
            },
            BlockSpan {
                kind: BlockKind::Supple,
                start: 10,
                end: 12,
                status: TemplateStatus::Todo,
                closed: true,
            },
        ]
    );
    assert_eq!(scan.infos[0].unwrap().kind, BlockKind::Template);
    assert_eq!(scan.infos[0].unwrap().role, BlockRole::Start);
    assert_eq!(scan.infos[2].unwrap().role, BlockRole::Middle);
    assert_eq!(scan.infos[3].unwrap().role, BlockRole::End);
    assert!(scan.infos[4].is_none());
    assert_eq!(scan.infos[5].unwrap().kind, BlockKind::Supple);
    assert_eq!(scan.infos[5].unwrap().role, BlockRole::Start);
    assert_eq!(scan.infos[8].unwrap().role, BlockRole::End);
    assert!(scan.infos[9].is_none());
    assert_eq!(scan.infos[10].unwrap().kind, BlockKind::Supple);
    assert_eq!(scan.infos[12].unwrap().role, BlockRole::End);
}

#[test]
fn second_template_block_is_ignored_as_plain_text() {
    let text = concat!(
        "&&& addTemplate --start\n",
        "first\n",
        "&&& addTemplate --end done\n",
        "&&& bugTemplate --start\n",
        "second\n",
        "&&& bugTemplate --end\n",
    );
    let scan = block_scan(text);
    assert_eq!(scan.spans.len(), 1);
    assert_eq!(scan.spans[0].start, 0);
    assert_eq!(scan.spans[0].end, 2);
    assert_eq!(scan.spans[0].status, TemplateStatus::Done);
    assert!(scan.infos[3].is_none(), "第二个 &&& 起始行不是块");
    assert!(scan.infos[4].is_none());
    assert!(scan.infos[5].is_none(), "第二个 &&& 结束行不是块");
}

#[test]
fn unclosed_first_template_extends_to_end_and_swallows_later_supple() {
    let text = concat!(
        "&&& addTemplate --start\n",
        "body\n",
        "+++ supplement --start\n",
        "sub\n",
        "+++ supplement --end",
    );
    let scan = block_scan(text);
    assert_eq!(
        scan.spans,
        vec![BlockSpan {
            kind: BlockKind::Template,
            start: 0,
            end: 4,
            status: TemplateStatus::Todo,
            closed: false,
        }]
    );
    assert_eq!(scan.infos[0].unwrap().role, BlockRole::Start);
    for line in 1..=4 {
        let info = scan.infos[line].expect("inside unclosed template");
        assert_eq!(info.kind, BlockKind::Template);
        assert_eq!(info.role, BlockRole::Middle);
    }
}

#[test]
fn supple_markers_inside_template_are_plain_middle_lines() {
    let text = concat!(
        "&&& addTemplate --start\n",
        "+++ supplement --start\n",
        "- inside\n",
        "+++ supplement --end\n",
        "&&& addTemplate --end",
    );
    let scan = block_scan(text);
    assert_eq!(scan.spans.len(), 1);
    assert_eq!(scan.spans[0].kind, BlockKind::Template);
    for line in 1..=3 {
        let info = scan.infos[line].expect("template middle");
        assert_eq!(info.kind, BlockKind::Template);
        assert_eq!(info.role, BlockRole::Middle);
    }
}

#[test]
fn template_markers_inside_supple_are_plain_middle_lines() {
    let text = concat!(
        "+++ supplement --start\n",
        "&&& addTemplate --start\n",
        "&&& addTemplate --end\n",
        "+++ supplement --end",
    );
    let scan = block_scan(text);
    assert_eq!(scan.spans.len(), 1);
    assert_eq!(scan.spans[0].kind, BlockKind::Supple);
    for line in 1..=2 {
        let info = scan.infos[line].expect("supple middle");
        assert_eq!(info.kind, BlockKind::Supple);
        assert_eq!(info.role, BlockRole::Middle);
    }
}

#[test]
fn supple_before_template_is_recognized_and_first_template_still_works() {
    let text = concat!(
        "+++ supplement --start\n",
        "- pre\n",
        "+++ supplement --end\n",
        "&&& addTemplate --start\n",
        "- main\n",
        "&&& addTemplate --end",
    );
    let scan = block_scan(text);
    assert_eq!(scan.spans.len(), 2);
    assert_eq!(scan.spans[0].kind, BlockKind::Supple);
    assert_eq!((scan.spans[0].start, scan.spans[0].end), (0, 2));
    assert_eq!(scan.spans[1].kind, BlockKind::Template);
    assert_eq!((scan.spans[1].start, scan.spans[1].end), (3, 5));
}

#[test]
fn fenced_markers_are_ignored() {
    let text = concat!(
        "```\n",
        "&&& addTemplate --start\n",
        "+++ supplement --start\n",
        "```\n",
        "&&& bugTemplate --start\n",
        "&&& bugTemplate --end\n",
        "~~~\n",
        "+++ supplement --start\n",
        "~~~\n",
        "+++ supplement --start\n",
        "+++ supplement --end",
    );
    let scan = block_scan(text);
    assert_eq!(scan.spans.len(), 2);
    assert_eq!(scan.spans[0].kind, BlockKind::Template);
    assert_eq!((scan.spans[0].start, scan.spans[0].end), (4, 5));
    assert_eq!(scan.spans[1].kind, BlockKind::Supple);
    assert_eq!((scan.spans[1].start, scan.spans[1].end), (9, 10));
    assert!(scan.infos[1].is_none());
    assert!(scan.infos[2].is_none());
    assert!(scan.infos[7].is_none());
}

#[test]
fn top_level_supple_marks_title_and_binding_separators() {
    let text = concat!(
        "+++ supplement --start\n",
        "{title: 补充}\n",
        "\n",
        "## Body\n",
        "\n",
        "{wt:main}\n",
        "+++ supplement --end",
    );
    let infos = block_line_infos(text);
    assert!(infos[2].unwrap().separator, "标题下空行应标记为分割行");
    assert!(infos[4].unwrap().separator, "绑定行上空行应标记为分割行");
    assert!(!infos[1].unwrap().separator);
    assert!(!infos[3].unwrap().separator);
}

#[test]
fn reserved_log_command_is_not_a_supple_block() {
    let text =
        "+++ log --start\n- record\n+++ log --end\n+++ logTemplate --start\n+++ logTemplate --end";
    let scan = block_scan(text);
    assert!(scan.spans.is_empty());
    assert!(scan.infos.iter().all(Option::is_none));
}

#[test]
fn block_context_reports_top_level_template_and_supple() {
    let text = concat!(
        "plain\n",
        "&&& x --start\n",
        "in template\n",
        "&&& x --end\n",
        "+++ supplement --start\n",
        "in supple\n",
        "+++ supplement --end\n",
    );
    assert_eq!(block_context(text, 0), BlockContext::Normal);
    assert_eq!(
        block_context(text, text.find("in template").unwrap()),
        BlockContext::Template
    );
    assert_eq!(
        block_context(text, text.find("in supple").unwrap()),
        BlockContext::Supple
    );
}
