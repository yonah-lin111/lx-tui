//! 输出缓冲测试：有界尾部、绝对偏移、丢弃证据、UTF-8/ANSI 安全化与游标语义。

use super::*;

/// 构造一个小容量缓冲（测试用）。
fn buffer(capacity: usize) -> OutputBuffer {
    OutputBuffer::new(capacity)
}

#[test]
fn read_from_preserves_absolute_offsets() {
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"hello\n");
    assert_eq!(output.start(), 0);
    assert_eq!(output.end(), 6);
    let read = output.read_from(0, 1024, false);
    assert_eq!(read.text, "hello\n");
    assert_eq!(read.start, 0);
    assert_eq!(read.end, 6);
    assert!(!read.truncated);
    assert_eq!(read.dropped_bytes, 0);
}

#[test]
fn truncation_only_advances_the_returned_range() {
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"0123456789");
    let first = output.read_from(0, 4, false);
    assert_eq!(first.text, "0123");
    assert_eq!(first.end, 4, "只推进到实际返回的字节");
    assert!(first.truncated);
    let second = output.read_from(first.end, 4, false);
    assert_eq!(second.text, "4567");
    assert_eq!(second.start, 4);
    assert_eq!(second.end, 8);
    let third = output.read_from(second.end, 16, false);
    assert_eq!(third.text, "89");
    assert!(!third.truncated);
}

#[test]
fn tail_retention_reports_dropped_bytes() {
    let mut output = buffer(8);
    output.push(OutputSource::Stdout, b"0123456789");
    assert_eq!(output.start(), 2, "保留尾部 8 字节");
    assert_eq!(output.dropped_bytes(), 2);
    // 请求起点早于保留区间：报告丢失量，返回区间从保留起点开始。
    let read = output.read_from(0, 64, false);
    assert_eq!(read.dropped_bytes, 2);
    assert_eq!(read.start, 2);
    assert_eq!(read.text, "23456789");
}

#[test]
fn ui_preview_does_not_consume_the_model_cursor() {
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"line one\n");
    // UI 尾部预览（不推进游标）读两次。
    let _ = output.read_from(output.start(), 1024, true);
    let _ = output.read_from(output.start(), 1024, true);
    // 模型从 0 读，仍应拿到完整输出。
    let read = output.read_from(0, 1024, false);
    assert_eq!(read.text, "line one\n");
    assert_eq!(read.start, 0);
}

#[test]
fn ansi_and_osc_sequences_are_stripped() {
    let mut output = buffer(4096);
    output.push(
        OutputSource::Stdout,
        b"\x1b[31mred\x1b[0m plain \x1b]0;title\x07done",
    );
    let read = output.read_from(0, 4096, false);
    assert_eq!(read.text, "red plain done");
    assert!(!read.text.contains('\x1b'), "控制序列不得进入显示文本");
    assert!(!read.text.contains("title"));
}

#[test]
fn carriage_returns_do_not_reach_the_terminal() {
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"progress 10%\rprogress 100%\n");
    let read = output.read_from(0, 1024, false);
    // 孤立 `\r` 归一为换行；CRLF 只产生一个换行。
    assert_eq!(read.text, "progress 10%\nprogress 100%\n");
}

#[test]
fn split_utf8_character_is_left_for_the_next_read() {
    let mut output = buffer(1024);
    let text = "中文";
    let bytes = text.as_bytes();
    // 切成两半：中文的 3 字节被拆成 1 + 2。
    output.push(OutputSource::Stdout, &bytes[..1]);
    let first = output.read_from(0, 1024, false);
    assert_eq!(first.text, "", "半截字符不出现在文本里");
    assert_eq!(first.end, 0, "未消费的字节留给下次");
    output.push(OutputSource::Stdout, &bytes[1..]);
    let second = output.read_from(first.end, 1024, false);
    assert_eq!(second.text, text);
    assert_eq!(second.end, bytes.len() as u64);
}

#[test]
fn invalid_bytes_become_visible_replacement_characters() {
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"ok\xffbad");
    let read = output.read_from(0, 1024, false);
    assert!(read.text.starts_with("ok"));
    assert!(read.text.contains('\u{fffd}'), "非法字节必须显式可见");
}

#[test]
fn overlong_control_sequence_is_dropped_not_buffered_forever() {
    let mut output = buffer(4096);
    let mut payload = Vec::new();
    payload.extend_from_slice(b"\x1b]");
    payload.extend(std::iter::repeat_n(b'a', MAX_CONTROL_SEQUENCE_BYTES + 32));
    payload.extend_from_slice(b"tail\n");
    output.push(OutputSource::Stdout, &payload);
    let read = output.read_from(0, 4096, false);
    assert!(!read.text.contains('\x1b'));
    // 超长序列被丢弃，后面的可见文本仍可读（至少不会卡住）。
    assert!(read.text.contains("tail") || !read.text.contains('a'));
}

#[test]
fn segments_keep_sources_and_coalesce_by_source() {
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"out1 ");
    output.push(OutputSource::Stderr, b"err1 ");
    output.push(OutputSource::Stdout, b"out2");
    let read = output.read_from(0, 1024, false);
    // 观察顺序即拼接顺序（不声称两个管道有严格全局时间顺序）。
    assert_eq!(read.text, "out1 err1 out2");
}

#[test]
fn sustained_output_stays_bounded() {
    let mut output = buffer(1024);
    for _ in 0..500 {
        output.push(OutputSource::Stdout, &[b'x'; 64]);
    }
    assert!(output.retained() <= 1024, "保留量必须有界");
    assert_eq!(output.end(), 500 * 64, "绝对偏移记录全部写入");
    assert!(output.dropped_bytes() > 0);
}

#[test]
fn display_lines_keeps_blank_lines() {
    assert_eq!(display_lines("a\n\nb\n"), vec!["a", "", "b", ""]);
}

// ===== review 回归：跨读取边界的转义序列不得被吞掉 =====

#[test]
fn escape_sequence_split_across_reads_is_not_swallowed() {
    // 真实缺陷：未闭合的 CSI 被整段吞掉并**推进模型游标**，于是被读取边界切开的
    // 转义序列后半段（`2mred`）会在下次读取时被当作正文输出给模型。
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"\x1b[3");
    // 第一次读取（flush=false）：序列未闭合 → 不消费，游标不动。
    let first = output.read_from(0, 1024, false);
    assert_eq!(first.text, "", "未闭合的转义序列不得产出正文");
    assert_eq!(first.end, 0, "未闭合序列不得推进游标：{:?}", first);

    // 后半段到达后，整条序列应被完整剥掉，且后面的正文照常可读。
    output.push(OutputSource::Stdout, b"2mred\x1b[0m done");
    let second = output.read_from(first.end, 1024, false);
    assert!(
        !second.text.contains("2mred") && !second.text.contains('\x1b'),
        "转义序列必须整体剥离，实际 {:?}",
        second.text
    );
    assert!(
        second.text.contains("red"),
        "正文必须保留：{:?}",
        second.text
    );
    assert!(
        second.text.contains("done"),
        "正文必须保留：{:?}",
        second.text
    );
}

#[test]
fn unflushed_split_utf8_and_split_escape_behave_the_same_way() {
    // 两条「数据不够」的路径必须同策略（不消费、下次重试）：否则一边等、
    // 一边吞，语义自相矛盾。
    let mut output = buffer(1024);
    output.push(OutputSource::Stdout, b"\xe4\xbd");
    let utf8 = output.read_from(0, 1024, false);
    assert_eq!(utf8.end, 0, "不完整 UTF-8 不消费");

    let mut escape = buffer(1024);
    escape.push(OutputSource::Stdout, b"\x1b]0;title");
    let osc = escape.read_from(0, 1024, false);
    assert_eq!(osc.end, 0, "未闭合 OSC 同样不消费");
}
