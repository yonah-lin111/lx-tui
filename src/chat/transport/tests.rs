//! 系统提示分段：代理安全线（单条 system 不超过 2000 字符）与切分不变量。
//!
//! 这一层的存在理由是一条实测故障：9router 的 CodeBuddy CN 执行器把**任何超过
//! 2000 字符**的 system 消息整条替换成中性提示，lx 的身份与全部规则因此失效
//! （模型开始自称「混元 / AI 助手」）。分段是唯一不改提示正文又能原样送达的办法。

use super::{MAX_SYSTEM_MESSAGE_CHARS, split_system_prompt};

fn chars(text: &str) -> usize {
    text.chars().count()
}

#[test]
fn chunks_never_exceed_the_proxy_limit() {
    let prompt = crate::chat::prompt::system_prompt();
    let chunks = split_system_prompt(&prompt);
    assert!(chunks.len() > 1, "完整提示必须被切成多段");
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(
            chars(chunk) <= MAX_SYSTEM_MESSAGE_CHARS,
            "第 {index} 段 {} 字符超过上限",
            chars(chunk)
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
    let chunks = split_system_prompt("You are lx.");
    assert_eq!(chunks, vec!["You are lx.".to_string()]);
}

#[test]
fn empty_prompt_yields_one_empty_chunk() {
    // 调用方总要发至少一条 system 消息；返回空 vec 会让 body 里少一个字段。
    assert_eq!(split_system_prompt(""), vec![String::new()]);
}

#[test]
fn splitting_prefers_line_boundaries() {
    let line = "rule line that is short\n";
    let prompt = line.repeat(200);
    let chunks = split_system_prompt(&prompt);
    assert!(chunks.len() > 1);
    // 每段都以换行结尾（除最后一段），说明切点落在行边界而不是句中。
    for chunk in &chunks[..chunks.len() - 1] {
        assert!(chunk.ends_with('\n'), "切点必须落在行边界");
    }
}

#[test]
fn a_single_oversized_line_is_hard_split_on_char_boundaries() {
    // 单行超限只能硬切；必须仍然是合法 UTF-8，且拼回去逐字相等。
    let line = "中".repeat(MAX_SYSTEM_MESSAGE_CHARS * 2 + 7);
    let chunks = split_system_prompt(&line);
    assert!(chunks.len() >= 3);
    for chunk in &chunks {
        assert!(chars(chunk) <= MAX_SYSTEM_MESSAGE_CHARS);
    }
    let joined: String = chunks.into_iter().collect();
    assert_eq!(joined, line);
}

#[test]
fn prompt_stays_within_the_proxy_limit_after_adding_environment() {
    // 环境快照会追加在同一段末尾，整段仍必须落在安全线内。
    let facts = crate::platform::EnvironmentFacts::capture();
    let environment = crate::chat::prompt::environment_snapshot(&facts, None, None, &[]);
    let prompt = crate::chat::prompt::compose_system_prompt(Some(&environment), None);
    for (index, chunk) in split_system_prompt(&prompt).iter().enumerate() {
        assert!(
            chars(chunk) <= MAX_SYSTEM_MESSAGE_CHARS,
            "含环境的第 {index} 段超过安全线"
        );
    }
}
