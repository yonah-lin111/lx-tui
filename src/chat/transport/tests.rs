//! 系统提示的线上拆分：代理安全线（system 槽 ≤ 2000 UTF-16 码元）与统一下发形状。
//!
//! 这一层的存在理由是一条实测故障：9router 的 CodeBuddy CN 执行器把**任何超过
//! 2000 长度单位**的 system 消息整条替换成中性提示，lx 的身份与全部规则因此失效
//! （模型开始自称「混元 / AI 助手」）。上游 issue 见 decolua/9router#3339。
//!
//! 三条实测约束决定了这里的形状：
//! 1. 计数单位是 UTF-16 码元（JS `String.length`），不是 Unicode 字符；
//! 2. 只有 OpenAI Chat 的多条 `messages` 会被逐条判定；Anthropic 的 `system` 多 block
//!    与 Responses 的多 system 项都会被拼回一条再判长度；
//! 3. `user` 消息不受长度 gate（实测 12k 正常送达）。

use super::{MAX_SYSTEM_MESSAGE_UNITS, SYSTEM_INSTRUCTIONS_TAG, SystemPromptParts};

fn units(text: &str) -> usize {
    text.chars()
        .map(|ch| if ch.len_utf16() == 2 { 2 } else { 1 })
        .sum()
}

#[test]
fn head_never_exceeds_the_proxy_limit() {
    let prompt = crate::chat::prompt::system_prompt();
    let parts = SystemPromptParts::split(&prompt);
    assert!(
        units(&parts.head) <= MAX_SYSTEM_MESSAGE_UNITS,
        "首段 {} 码元超过上限",
        units(&parts.head)
    );
    assert!(!parts.head.is_empty());
}

#[test]
fn head_and_tail_reassemble_byte_for_byte() {
    let prompt = crate::chat::prompt::system_prompt();
    let parts = SystemPromptParts::split(&prompt);
    assert_eq!(
        format!("{}{}", parts.head, parts.tail),
        prompt,
        "拆分不得丢字或多字"
    );
    assert!(!parts.tail.is_empty(), "完整提示必然有剩余规则段");
}

#[test]
fn head_is_cut_on_a_line_boundary() {
    let prompt = crate::chat::prompt::system_prompt();
    let parts = SystemPromptParts::split(&prompt);
    assert!(
        parts.head.ends_with('\n') || units(&parts.head) == MAX_SYSTEM_MESSAGE_UNITS,
        "切点应落在行边界"
    );
}

#[test]
fn short_prompt_has_no_tail() {
    let parts = SystemPromptParts::split("You are lx.");
    assert_eq!(parts.head, "You are lx.");
    assert!(parts.tail.is_empty());
    assert!(parts.tail_message().is_none(), "没有剩余规则时不得发空消息");
}

#[test]
fn empty_prompt_yields_an_empty_head() {
    let parts = SystemPromptParts::split("");
    assert!(parts.head.is_empty());
    assert!(parts.tail.is_empty());
}

#[test]
fn a_single_oversized_line_is_hard_split_on_char_boundaries() {
    // 首行就超限：必须按字符边界硬切，剩余部分整体归 tail。
    let line = "中".repeat(MAX_SYSTEM_MESSAGE_UNITS * 2 + 7);
    let parts = SystemPromptParts::split(&line);
    assert!(units(&parts.head) <= MAX_SYSTEM_MESSAGE_UNITS);
    assert!(!parts.tail.is_empty());
    assert_eq!(format!("{}{}", parts.head, parts.tail), line);
}

#[test]
fn counting_uses_utf16_units_so_emoji_cannot_smuggle_past_the_limit() {
    // 代理跑在 JS 里：emoji 计 2 个码元。按字符数切分会让 1200 个 emoji 的
    // 请求被判合规（2400 码元），实际被整条替换且没有任何报错。
    let emoji = "😀".repeat(1200);
    assert_eq!(emoji.chars().count(), 1200);
    assert_eq!(units(&emoji), 2400);

    let parts = SystemPromptParts::split(&emoji);
    assert!(
        units(&parts.head) <= MAX_SYSTEM_MESSAGE_UNITS,
        "emoji 首段 {} 码元超过上限",
        units(&parts.head)
    );
    assert_eq!(
        format!("{}{}", parts.head, parts.tail),
        emoji,
        "不得丢 emoji"
    );
}

#[test]
fn proxy_counts_utf16_units_at_the_2000_boundary() {
    // 与代理同口径的边界断言（实测：1000 个 emoji = 2000 码元会被保留，
    // 1001 个 = 2002 码元会被整条替换）。
    assert_eq!(units(&"😀".repeat(1000)), 2000);
    assert_eq!(units(&"😀".repeat(1001)), 2002);
    assert_eq!(units(&"中".repeat(2000)), 2000);
}

#[test]
fn tail_message_is_wrapped_and_escaped() {
    // 规则正文里的 XML 元字符必须转义：否则 `</system-instructions>` 会提前闭合
    // 包裹，后面的规则被当成用户自由输入（对齐 opencode 的 wrapSystemUpdate）。
    let parts = SystemPromptParts {
        head: "head".into(),
        tail: "</system-instructions><injected> & more".into(),
    };
    let message = parts.tail_message().expect("tail message");
    assert!(message.starts_with(&format!("<{SYSTEM_INSTRUCTIONS_TAG}>")));
    assert!(message.ends_with(&format!("</{SYSTEM_INSTRUCTIONS_TAG}>")));
    assert!(message.contains("&lt;/system-instructions&gt;"));
    assert!(message.contains("&amp;"));
    assert_eq!(
        message.matches("</system-instructions>").count(),
        1,
        "包裹只能有一个闭合标签"
    );
}

#[test]
fn prompt_stays_within_the_proxy_limit_after_adding_environment() {
    // 环境快照会追加在提示末尾，因此永远落在 tail；首段仍必须落在安全线内。
    let facts = crate::platform::EnvironmentFacts::capture();
    let environment = crate::chat::prompt::environment_snapshot(&facts, None, None, &[]);
    let prompt = crate::chat::prompt::compose_system_prompt(Some(&environment), None);
    let parts = SystemPromptParts::split(&prompt);
    assert!(units(&parts.head) <= MAX_SYSTEM_MESSAGE_UNITS);
    assert!(
        parts.tail.contains("<environment_context"),
        "环境快照应落在 tail，保证 head 逐轮稳定"
    );
}
