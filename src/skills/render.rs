//! skill 提示渲染：系统提示目录段、当轮注入块与历史回放摘要行。

use std::path::Path;

use super::{MAX_CATALOG_CHARS, MAX_CATALOG_DESCRIPTION_CHARS, SkillEntry};

const HEADER: &str = "## Skills";
const INTRO: &str = "A skill is a set of instructions stored in a `SKILL.md` file. The skills below are available in this session; each entry lists a name, a description and the absolute path of its `SKILL.md`.";
const AVAILABLE: &str = "### Available skills";

/// `### How to use skills` 段的固定文本；子 agent 替换委派一条。
fn render_policy(subagent: bool) -> String {
    let subagents = if subagent {
        "- If your task names a skill, read its SKILL.md completely before starting the work."
    } else {
        "- Subagents: when you delegate part of a skill's work, name the skill and its `SKILL.md` path in the `spawn_agent` task so the subagent reads the original instructions itself."
    };
    format!(
        "### How to use skills\n\
- Trigger: if the user names a skill, or the task clearly matches a skill's description, use that skill for this turn. Several matches mean use all of them, in an order you state. Do not carry a skill into later turns unless it is named again.\n\
- Load: before acting, read the listed `SKILL.md` completely with `read` (continue with `offset` until the end if it is truncated).\n\
- Paths: resolve relative paths mentioned in `SKILL.md` (for example `scripts/build.sh`) against the directory that contains that `SKILL.md`.\n\
- Reuse: prefer running or adapting a skill's `scripts/` and `assets/` over rewriting them. Only open the reference files the task needs.\n\
{subagents}\n\
- Missing: if a skill file cannot be read, say so in one line and continue with the best fallback.\n\
- Announce which skill you are using in one short line."
    )
}

/// 渲染系统提示的 skill 目录段；没有可用 skill 时返回 None。
pub fn render_catalog(entries: &[SkillEntry], subagent: bool) -> Option<String> {
    let visible: Vec<&SkillEntry> = entries
        .iter()
        .filter(|entry| entry.enabled && entry.shadowed_by.is_none())
        .collect();
    if visible.is_empty() {
        return None;
    }
    let policy = render_policy(subagent);
    let fixed = format!("{HEADER}\n{INTRO}\n{AVAILABLE}\n") + &policy;
    let mut lines: Vec<String> = visible
        .iter()
        .map(|entry| {
            let description = truncate_chars(&entry.description, MAX_CATALOG_DESCRIPTION_CHARS);
            format!(
                "- {}: {} (file: {})",
                entry.name,
                description,
                entry.path.display()
            )
        })
        .collect();
    let mut body = format!("{HEADER}\n{INTRO}\n{AVAILABLE}\n");
    let mut included = 0usize;
    for line in lines.drain(..) {
        let candidate = format!("{body}{line}\n");
        if candidate.chars().count() + fixed.chars().count() > MAX_CATALOG_CHARS {
            break;
        }
        body = candidate;
        included += 1;
    }
    let omitted = visible.len().saturating_sub(included);
    if omitted > 0 {
        body.push_str(&format!("- {omitted} additional skills omitted.\n"));
        tracing::warn!(
            target: "lx::skills",
            omitted,
            "skill catalog exceeded the prompt budget"
        );
    }
    body.push_str(&policy);
    Some(body.trim_end().to_string())
}

/// `<skill>` 注入块；返回 `(block, truncated)`。
pub fn render_injection(name: &str, path: &Path, body: &str, max_bytes: usize) -> (String, bool) {
    let (body, truncated) = truncate_bytes(body, max_bytes);
    let mut block = String::new();
    block.push_str("<skill>\n<name>");
    block.push_str(name);
    block.push_str("</name>\n<path>");
    block.push_str(&path.display().to_string());
    block.push_str("</path>\n");
    block.push_str(&body);
    if truncated {
        block.push_str(&format!(
            "\n[skill truncated at {max_bytes} bytes; read the file for the rest]"
        ));
    }
    block.push_str("\n</skill>");
    (block, truncated)
}

/// 历史用户消息里的 skill 附件回放行（不重新读文件）。
pub fn render_replay_line(name: &str, path: &Path) -> String {
    format!(
        "[skill {name} was loaded from {} in this turn]",
        path.display()
    )
}

/// 按字符截断（不破坏 UTF-8）。
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 按字节上限截断到字符边界。
fn truncate_bytes(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

#[cfg(test)]
mod tests;
