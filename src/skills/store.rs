//! 设置页 skill 编辑：只允许 lx 自有根；服务端生成 frontmatter。

use std::path::{Path, PathBuf};

use super::MAX_SKILL_FILE_BYTES;
use super::parse::validate_name;

/// 读取一个 skill 的 name/description/body（供设置页详情）。
pub fn read_skill(root: &Path, name: &str) -> Result<(String, String, String), String> {
    validate_name(name)?;
    let path = root.join(name).join("SKILL.md");
    let text = super::parse::load_skill_body(&path)?;
    let parsed = super::parse::parse_skill(&text, name)?;
    let body = strip_frontmatter(&text).unwrap_or_default();
    Ok((parsed.name, parsed.description, body))
}

/// 生成 `SKILL.md` 文本（frontmatter + 正文）。
pub fn serialize_skill(name: &str, description: &str, body: &str) -> Result<String, String> {
    validate_name(name)?;
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return Err("description must not be empty".into());
    }
    let body = body.trim_start_matches('\n');
    let text = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}");
    if text.len() > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "SKILL.md exceeds {} KiB",
            MAX_SKILL_FILE_BYTES / 1024
        ));
    }
    Ok(text)
}

/// 新建/更新 lx 自有根下的 skill。
pub fn write_skill(
    root: &Path,
    name: &str,
    description: &str,
    body: &str,
) -> Result<PathBuf, String> {
    let text = serialize_skill(name, description, body)?;
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).map_err(|error| format!("create failed: {error}"))?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, text.as_bytes()).map_err(|error| format!("write failed: {error}"))?;
    Ok(path)
}

/// 删除 lx 自有根下的 skill：只允许 `SKILL.md` 与空目录。
pub fn delete_skill(root: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let dir = root.join(name);
    let entries = std::fs::read_dir(&dir).map_err(|error| format!("read failed: {error}"))?;
    let mut only_skill = true;
    for entry in entries.flatten() {
        if entry.file_name() != "SKILL.md" {
            only_skill = false;
            break;
        }
    }
    if !only_skill {
        return Err("skill directory contains other files; delete them manually".into());
    }
    let skill = dir.join("SKILL.md");
    if skill.is_file() {
        std::fs::remove_file(&skill).map_err(|error| format!("delete failed: {error}"))?;
    }
    std::fs::remove_dir(&dir).map_err(|error| format!("delete failed: {error}"))?;
    Ok(())
}

fn strip_frontmatter(text: &str) -> Option<String> {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines {
        if !closed {
            if line.trim_end() == "---" {
                closed = true;
            }
            continue;
        }
        body.push(line);
    }
    closed.then(|| body.join("\n").trim_start_matches('\n').to_string())
}

#[cfg(test)]
mod tests;
