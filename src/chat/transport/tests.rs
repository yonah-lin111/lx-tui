//! 系统提示分段：代理安全线（单条 system 不超过 2000 UTF-16 码元）与统一下发形状。
//!
//! 这一层的存在理由是一条实测故障：9router 的 CodeBuddy CN 执行器把**任何超过
//! 2000 长度单位**的 system 消息整条替换成中性提示，lx 的身份与全部规则因此失效
//! （模型开始自称「混元 / AI 助手」）。
//!
//! 两条实测约束决定了这里的形状：
//! 1. 计数单位是 UTF-16 码元（JS `String.length`），不是 Unicode 字符；
//! 2. 只有 OpenAI Chat 的多条 `messages` 会被逐条判定；Anthropic 的 `system` 多 block
//!    与 Responses 的多 system 项都会被拼回一条再判长度。

use super::{
    MAX_SYSTEM_MESSAGE_UNITS, SYSTEM_INSTRUCTIONS_TAG, split_system_prompt, system_messages,
    wrap_system_instructions,
};

fn units(text: &str) -> usize {
    text.chars()
        .map(|ch| if ch.len_utf16() == 2 { 2 } else { 1 })
        .sum()
}

#[test]
fn chunks_never_exceed_the_proxy_limit() {
    let prompt = crate::chat::prompt::system_prompt();
    let chunks = split_system_prompt(&prompt);
    assert!(chunks.len() > 1, "完整提示必须被切成多段");
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(
            units(chunk) <= MAX_SYSTEM_MESSAGE_UNITS,
            "第 {index} 段 {} 码元超过上限",
            units(chunk)
        );
    }
}

#[test]
fn chunks_reassemble_byte_for_byte() {
    let prompt = crate::chat::prompt::system_prompt();
    let joined: String = split_system_prompt(&prompt).into_iter().collect();
    assert_eq!(joined, prompt, "分段拼接必须逐字还原，不得丢字或多字");
}

#[test]
fn short_prompt_stays_a_single_chunk() {
    assert_eq!(
        split_system_prompt("You are lx."),
        vec!["You are lx.".to_string()]
    );
}

#[test]
fn empty_prompt_yields_one_empty_chunk() {
    // 调用方总要发至少一条 system 消息；返回空 vec 会让 body 里少一个字段。
    assert_eq!(split_system_prompt(""), vec![String::new()]);
}

#[test]
fn splitting_prefers_line_boundaries() {
    let line = "rule line that is short\n";
    let chunks = split_system_prompt(&line.repeat(200));
    assert!(chunks.len() > 1);
    for chunk in &chunks[..chunks.len() - 1] {
        assert!(chunk.ends_with('\n'), "切点必须落在行边界");
    }
}

#[test]
fn a_single_oversized_line_is_hard_split_on_char_boundaries() {
    // 单行超限只能硬切；必须仍然是合法 UTF-8，且拼回去逐字相等。
    let line = "中".repeat(MAX_SYSTEM_MESSAGE_UNITS * 2 + 7);
    let chunks = split_system_prompt(&line);
    assert!(chunks.len() >= 3);
    for chunk in &chunks {
        assert!(units(chunk) <= MAX_SYSTEM_MESSAGE_UNITS);
    }
    let joined: String = chunks.into_iter().collect();
    assert_eq!(joined, line);
}

#[test]
fn counting_uses_utf16_units_so_emoji_cannot_smuggle_past_the_limit() {
    // 代理跑在 JS 里：emoji 计 2 个码元。按字符数切分会把 1200 个 emoji 当成合规，
    // 实际是 2400 码元 —— 整条被替换且没有任何报错。
    let emoji = "😀".repeat(1200);
    assert_eq!(emoji.chars().count(), 1200);
    assert_eq!(units(&emoji), 2400);

    let chunks = split_system_prompt(&emoji);
    assert!(chunks.len() > 1, "必须按码元切成多段");
    for chunk in &chunks {
        assert!(
            units(chunk) <= MAX_SYSTEM_MESSAGE_UNITS,
            "emoji 段 {} 码元超过上限",
            units(chunk)
        );
    }
    let joined: String = chunks.into_iter().collect();
    assert_eq!(joined, emoji, "切分不得丢 emoji");
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
fn system_messages_puts_the_head_in_the_system_slot_and_the_rest_in_user_items() {
    let prompt = crate::chat::prompt::system_prompt();
    let (head, rest) = system_messages(&prompt);
    assert!(!rest.is_empty(), "完整提示必然需要前置规则段");
    assert_eq!(head, split_system_prompt(&prompt)[0]);

    // 首段 + 包裹后的其余段拼接 = 完整提示（包裹标签本身不丢内容）。
    let mut joined = head;
    for chunk in &rest {
        let wrapped = wrap_system_instructions(chunk);
        assert!(wrapped.starts_with(&format!("<{SYSTEM_INSTRUCTIONS_TAG}>")));
        assert!(wrapped.ends_with(&format!("</{SYSTEM_INSTRUCTIONS_TAG}>")));
        assert!(wrapped.contains(chunk.as_str()));
        joined.push_str(chunk);
    }
    assert_eq!(joined, prompt, "规则段不得被包裹改写");

    for chunk in &rest {
        assert!(units(chunk) <= MAX_SYSTEM_MESSAGE_UNITS);
    }
}

#[test]
fn prompt_stays_within_the_proxy_limit_after_adding_environment() {
    // 环境快照会追加在同一段末尾，整段仍必须落在安全线内。
    let facts = crate::platform::EnvironmentFacts::capture();
    let environment = crate::chat::prompt::environment_snapshot(&facts, None, None, &[]);
    let prompt = crate::chat::prompt::compose_system_prompt(Some(&environment), None);
    for (index, chunk) in split_system_prompt(&prompt).iter().enumerate() {
        assert!(
            units(chunk) <= MAX_SYSTEM_MESSAGE_UNITS,
            "含环境的第 {index} 段超过安全线"
        );
    }
}
