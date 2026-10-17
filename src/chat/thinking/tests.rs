//! thinking 标签状态机测试：跨 chunk 标签、正文比较符号、未闭合标签。

use super::*;

fn run(chunks: &[&str]) -> Vec<Segment> {
    let mut splitter = TagSplitter::new();
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(splitter.feed(chunk));
    }
    out.extend(splitter.finish());
    out
}

/// 按种类拼接文本（段边界由 chunk 决定，顺序与内容不丢不重）。
fn joined(segments: &[Segment]) -> Vec<(PartKind, String)> {
    let mut out: Vec<(PartKind, String)> = Vec::new();
    for segment in segments {
        if let Some(last) = out.last_mut()
            && last.0 == segment.kind
        {
            last.1.push_str(&segment.text);
        } else {
            out.push((segment.kind, segment.text.clone()));
        }
    }
    out
}

#[test]
fn plain_text_passes_through() {
    let segments = run(&["hello world", " more"]);
    assert_eq!(
        joined(&segments),
        vec![(PartKind::Text, "hello world more".into())]
    );
}

#[test]
fn comparison_symbols_pass_through() {
    let segments = run(&["a < b and c > d, x <y"]);
    assert_eq!(
        joined(&segments),
        vec![(PartKind::Text, "a < b and c > d, x <y".into())]
    );
}

#[test]
fn think_tag_splits_reasoning() {
    let segments = run(&["before <think>secret</think> after"]);
    assert_eq!(
        joined(&segments),
        vec![
            (PartKind::Text, "before ".into()),
            (PartKind::Reasoning, "secret".into()),
            (PartKind::Text, " after".into()),
        ]
    );
}

#[test]
fn thinking_tag_variant() {
    let segments = run(&["<thinking>deep</thinking>end"]);
    assert_eq!(
        joined(&segments),
        vec![
            (PartKind::Reasoning, "deep".into()),
            (PartKind::Text, "end".into()),
        ]
    );
}

#[test]
fn think_tag_split_across_chunks() {
    // 设计 fixture：`<thi` + `nk>` 跨 chunk，半截标签不泄漏进正文。
    let segments = run(&["a <thi", "nk>hidden</thi", "nk>b"]);
    assert_eq!(
        joined(&segments),
        vec![
            (PartKind::Text, "a ".into()),
            (PartKind::Reasoning, "hidden".into()),
            (PartKind::Text, "b".into()),
        ],
        "正文不残留标签"
    );
}

#[test]
fn thinking_tag_split_across_chunks() {
    // `<th` + `inking>`：更长的标签族跨 chunk 同样成立。
    let segments = run(&["x <th", "inking>深思</thi", "nking> y"]);
    assert_eq!(
        joined(&segments),
        vec![
            (PartKind::Text, "x ".into()),
            (PartKind::Reasoning, "深思".into()),
            (PartKind::Text, " y".into()),
        ]
    );
}

#[test]
fn unclosed_tag_keeps_reasoning_and_finishes_in_reasoning() {
    let mut splitter = TagSplitter::new();
    let out = splitter.feed("text <think>partial reasoning");
    // 单 chunk：开标签前的正文与标签后的 reasoning 一次吐出。
    assert_eq!(
        joined(&out),
        vec![
            (PartKind::Text, "text ".into()),
            (PartKind::Reasoning, "partial reasoning".into()),
        ]
    );
    assert!(splitter.is_in_reasoning());
    // 流结束无残余。
    assert_eq!(splitter.finish(), Vec::new());
    // 跨 chunk 的未闭合标签：reasoning 段在 feed 时即吐出（无 `<` 无缓冲），
    // 流中断后 splitter 保持在 reasoning 模式（调用方保留 partial 判定）。
    let mut splitter = TagSplitter::new();
    let out = splitter.feed("text <think>partial");
    assert_eq!(
        joined(&out),
        vec![
            (PartKind::Text, "text ".into()),
            (PartKind::Reasoning, "partial".into()),
        ]
    );
    assert!(splitter.is_in_reasoning());
    // 内容已在 feed 吐出，finish 无残余；模式保持 reasoning。
    assert_eq!(splitter.finish(), Vec::new());
    assert!(splitter.is_in_reasoning());
}

#[test]
fn interleaved_tags_preserve_order() {
    let segments = run(&["t1 <think>r1</think> t2 <think>r2</think> t3"]);
    assert_eq!(
        joined(&segments),
        vec![
            (PartKind::Text, "t1 ".into()),
            (PartKind::Reasoning, "r1".into()),
            (PartKind::Text, " t2 ".into()),
            (PartKind::Reasoning, "r2".into()),
            (PartKind::Text, " t3".into()),
        ]
    );
}

#[test]
fn chinese_text_across_chunks() {
    let segments = run(&["你好", "世界"]);
    assert_eq!(joined(&segments), vec![(PartKind::Text, "你好世界".into())]);
}

#[test]
fn partial_open_tag_at_stream_end_is_not_leaked_into_text() {
    // 流以半截标签结束：未进入 reasoning，残余缓冲按当前模式（正文）归还。
    let mut splitter = TagSplitter::new();
    let out = splitter.feed("done <thi");
    assert_eq!(
        joined(&out),
        vec![(PartKind::Text, "done ".into())],
        "半截标签不泄漏进正文"
    );
    let tail = splitter.finish();
    assert_eq!(joined(&tail), vec![(PartKind::Text, "<thi".into())]);
}

#[test]
fn empty_input_produces_nothing() {
    assert_eq!(run(&[""]), Vec::new());
    assert_eq!(run(&[]), Vec::new());
}
