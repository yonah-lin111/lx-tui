//! `SKILL.md` 有界解析：手写 frontmatter（标量 + `>`/`|` 块标量），不引入 YAML crate。

use std::path::Path;

use super::{MAX_SKILL_FILE_BYTES, SkillCatalog};

/// 解析后的 frontmatter。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSkill {
    pub name: String,
    pub description: String,
}

/// 读取 `SKILL.md` 正文；超过 256 KiB 拒绝。
pub fn load_skill_body(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("read failed: {error}"))?;
    if metadata.len() > MAX_SKILL_FILE_BYTES as u64 {
        return Err(format!(
            "SKILL.md exceeds {} KiB",
            MAX_SKILL_FILE_BYTES / 1024
        ));
    }
    std::fs::read_to_string(path).map_err(|error| format!("read failed: {error}"))
}

/// 解析 `SKILL.md` 文本；`dir_name` 是包含它的目录名（name 缺省值）。
pub fn parse_skill(text: &str, dir_name: &str) -> Result<ParsedSkill, String> {
    if text.len() > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "SKILL.md exceeds {} KiB",
            MAX_SKILL_FILE_BYTES / 1024
        ));
    }
    let text = text.replace("\r\n", "\n");
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("missing YAML frontmatter delimited by ---".into());
    }
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut closed = false;
    while let Some(line) = lines.next() {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err("unexpected indented frontmatter line".into());
        }
        let (key, raw) = match line.split_once(':') {
            Some((key, raw)) => (key.trim(), raw.trim()),
            None => return Err(format!("invalid frontmatter line '{line}'")),
        };
        let mut value = raw.to_string();
        if matches!(raw, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
            let folded = raw.starts_with('>');
            let mut parts: Vec<String> = Vec::new();
            while let Some(next) = lines.clone().next() {
                if next.trim().is_empty() {
                    lines.next();
                    if folded {
                        parts.push(String::new());
                    }
                    continue;
                }
                if !next.starts_with(' ') && !next.starts_with('\t') {
                    break;
                }
                lines.next();
                parts.push(next.trim().to_string());
            }
            value = if folded {
                parts.join(" ").trim().to_string()
            } else {
                parts.join("\n")
            };
        } else {
            value = unquote(&value);
            // 标量续行：后续缩进行并入同一值。
            while let Some(next) = lines.clone().next() {
                if next.trim().is_empty() || (!next.starts_with(' ') && !next.starts_with('\t')) {
                    break;
                }
                lines.next();
                let continuation = next.trim();
                if !continuation.is_empty() {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(continuation);
                }
            }
        }
        fields.push((key.to_lowercase(), value));
    }
    if !closed {
        return Err("missing YAML frontmatter delimited by ---".into());
    }

    let mut name = None;
    let mut description = None;
    for (key, value) in fields {
        match key.as_str() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }
    let name = match name {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => dir_name.to_string(),
    };
    validate_name(&name)?;
    let description = description
        .map(|description| description.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|description| !description.is_empty())
        .ok_or_else(|| "missing field `description`".to_string())?;
    Ok(ParsedSkill { name, description })
}

/// skill 名：1-64 个 `[A-Za-z0-9_.-]`，首字符为字母或数字。
pub fn validate_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'));
    if valid {
        Ok(())
    } else {
        Err("invalid name: use 1-64 of [A-Za-z0-9_.-]".into())
    }
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// 解析目录里的 `SKILL.md`；错误进入目录错误列表。
pub(crate) fn parse_dir(
    path: &Path,
    dir_name: &str,
    catalog: &mut SkillCatalog,
) -> Option<ParsedSkill> {
    let text = match load_skill_body(path) {
        Ok(text) => text,
        Err(error) => {
            catalog.errors.push((path.to_path_buf(), error));
            return None;
        }
    };
    match parse_skill(&text, dir_name) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            catalog.errors.push((path.to_path_buf(), error));
            None
        }
    }
}

#[cfg(test)]
mod tests;
