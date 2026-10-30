//! 设置页 skill 编辑：只允许 lx 自有根；服务端生成 frontmatter。

use std::path::{Path, PathBuf};

use super::MAX_SKILL_FILE_BYTES;
use super::parse::validate_name;

/// 读取一个 skill 的 name/description/body（供设置页详情）。
pub fn read_skill(root: &Path, name: &str) -> Result<(String, String, String), String> {
    validate_name(name)?;
    let dir = root.join(name);
    let path =
        super::find_skill_file(&dir).ok_or_else(|| format!("skill '{name}' has no SKILL.md"))?;
    let text = super::parse::load_skill_body(&path)?;
    let parsed = super::parse::parse_skill(&text, name)?;
    let body = strip_frontmatter(&text).unwrap_or_default();
    Ok((parsed.name, parsed.description, body))
}

/// 读取任意已发现 `SKILL.md` 的 name/description/正文（设置页只读预览）。
pub fn read_skill_at(path: &Path) -> Result<(String, String, String), String> {
    let dir_name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let text = super::parse::load_skill_body(path)?;
    let parsed = super::parse::parse_skill(&text, dir_name)?;
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
    // 已存在的 `skill.md` 就地更新，不额外生成一个 `SKILL.md`（大小写不敏感文件系统上
    // 两个名字会指向同一个文件；显式保留原 casing 更可预期）。
    let path = super::find_skill_file(&dir).unwrap_or_else(|| dir.join(super::SKILL_FILE_NAME));
    std::fs::write(&path, text.as_bytes()).map_err(|error| format!("write failed: {error}"))?;
    Ok(path)
}

/// 删除 lx 自有根下的 skill：只允许 `SKILL.md` 与空目录。
pub fn delete_skill(root: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let dir = root.join(name);
    let entries = std::fs::read_dir(&dir).map_err(|error| format!("read failed: {error}"))?;
    let mut only_skill = true;
    let mut skill_file = None;
    for entry in entries.flatten() {
        if super::is_skill_file_name(&entry.file_name()) {
            skill_file = Some(entry.path());
            continue;
        }
        only_skill = false;
        break;
    }
    if !only_skill {
        return Err("skill directory contains other files; delete them manually".into());
    }
    if let Some(skill) = skill_file.filter(|path| path.is_file()) {
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
