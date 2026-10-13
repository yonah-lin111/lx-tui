//! 自定义斜杠命令的 Markdown 文件存储：`agentMD`（Markdown 插入）与
//! `agentBlock`（模板块插入）两类，User 与 Project 两个作用域。
//!
//! 目录布局（与 lx-agent 对齐）：
//! - User：`<data-root>/command/{agentMD,agentBlock}/<name>.md`
//!   （生产 `~/.lx-t`，开发 `~/.lx-t-dev`）
//! - Project（git）：`<主 checkout>/.lx-t/command/...`
//! - Project（非 git）：`<工作区根>/.lx-t/command/...`
//!
//! 文件为 UTF-8 Markdown + 小型 frontmatter 子集；文件名即命令名（ASCII
//! 字母/数字/`_`/`-`）。写入一律走同目录临时文件加重命名，避免半截文件。

use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{ConfigError, user_data_root};

/// 命令类型目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    /// Markdown 插入；frontmatter 使用 `scope: global|template`。
    AgentMd,
    /// 完整模板块插入；frontmatter 使用 `block: template`。
    AgentBlock,
}

impl CommandKind {
    /// 全部类型（编辑/list 遍历顺序）。
    pub const ALL: [CommandKind; 2] = [CommandKind::AgentMd, CommandKind::AgentBlock];

    /// 目录名。
    pub fn dir_name(self) -> &'static str {
        match self {
            CommandKind::AgentMd => "agentMD",
            CommandKind::AgentBlock => "agentBlock",
        }
    }

    /// 稳定标识（设置页与 JSON 使用）。
    pub fn as_str(self) -> &'static str {
        match self {
            CommandKind::AgentMd => "agentMD",
            CommandKind::AgentBlock => "agentBlock",
        }
    }

    /// 由稳定标识解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "agentMD" => Some(CommandKind::AgentMd),
            "agentBlock" => Some(CommandKind::AgentBlock),
            _ => None,
        }
    }
}

/// 命令作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandScope {
    User,
    Project,
}

impl CommandScope {
    /// 稳定标识（设置页与 JSON 使用）。
    pub fn as_str(self) -> &'static str {
        match self {
            CommandScope::User => "user",
            CommandScope::Project => "project",
        }
    }

    /// 由稳定标识解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(CommandScope::User),
            "project" => Some(CommandScope::Project),
            _ => None,
        }
    }
}

/// `agentMD` 的插入范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdScope {
    /// 任意位置可用的 Markdown 插入。
    Global,
    /// 仅模板块内可用的 Markdown 插入。
    Template,
}

impl MdScope {
    /// 稳定标识（frontmatter 与设置页使用）。
    pub fn as_str(self) -> &'static str {
        match self {
            MdScope::Global => "global",
            MdScope::Template => "template",
        }
    }

    /// 由稳定标识解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "global" => Some(MdScope::Global),
            "template" => Some(MdScope::Template),
            _ => None,
        }
    }
}

/// `agentBlock` 的块类型：`template` 生成 `&&&`，`supple` 生成 `+++`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Template,
    Supple,
}

impl BlockType {
    /// 稳定标识（frontmatter 与 JSON 使用）。
    pub fn as_str(self) -> &'static str {
        match self {
            BlockType::Template => "template",
            BlockType::Supple => "supple",
        }
    }

    /// 由稳定标识解析；未知值返回 None（调用方报解析错误）。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "template" => Some(BlockType::Template),
            "supple" => Some(BlockType::Supple),
            _ => None,
        }
    }
}

/// 一条已解析的自定义命令；`name` 由文件名派生。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRecord {
    pub name: String,
    /// `agentMD` 的插入范围；`agentBlock` 恒为 `Template`。
    pub scope: MdScope,
    pub description: String,
    pub argument_hint: Option<String>,
    /// `agentBlock` 的可选标题；写入默认模板块的 `{title: …}`。
    pub title: Option<String>,
    /// 插入正文。
    pub body: String,
    pub kind: CommandKind,
    /// `agentBlock` 的块类型；`agentMD` 恒为 `Template`。
    pub block: BlockType,
    /// 来源文件路径（保存/删除用）。
    pub path: PathBuf,
}

/// 解析错误：文件不可用时跳过并在设置页展示，不影响其它命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandParseError {
    pub path: PathBuf,
    pub message: String,
}

/// 存储操作结果。
#[derive(Debug)]
pub enum CommandsError {
    /// HOME 缺失。
    HomeMissing,
    /// 命令名非法。
    InvalidName(String),
    /// frontmatter 非法。
    InvalidMetadata(String),
    /// 目标已存在（保存/重命名冲突）。
    AlreadyExists(String),
    /// 同名命令属于另一作用域（Project 覆盖 User），删除/重命名被拒绝。
    OtherScope(String),
    /// 目标不存在。
    NotFound(String),
    /// IO 错误。
    Io(std::io::Error),
}

impl fmt::Display for CommandsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::InvalidName(name) => write!(formatter, "invalid command name: {name}"),
            Self::InvalidMetadata(message) => {
                write!(formatter, "invalid command metadata: {message}")
            }
            Self::AlreadyExists(name) => write!(formatter, "command already exists: {name}"),
            Self::OtherScope(name) => {
                write!(formatter, "command belongs to another scope: {name}")
            }
            Self::NotFound(name) => write!(formatter, "command not found: {name}"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
        }
    }
}

impl std::error::Error for CommandsError {}

impl From<std::io::Error> for CommandsError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ConfigError> for CommandsError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::HomeMissing => Self::HomeMissing,
            ConfigError::Io(error) => Self::Io(error),
            ConfigError::Invalid(message) => Self::InvalidMetadata(message),
        }
    }
}

/// 存储操作结果别名。
pub type Result<T> = std::result::Result<T, CommandsError>;

/// 内置命令名（短名与长别名）与模板块保留词；自定义命令不得占用。
pub const RESERVED_NAMES: [&str; 22] = [
    "add",
    "addTemplate",
    "bug",
    "bugTemplate",
    "common",
    "commonTemplate",
    "refactor",
    "refactorTemplate",
    "style",
    "styleTemplate",
    "gitWorkTree",
    "done",
    "in_progress",
    "supple",
    "supplement",
    "log",
    "tmp",
    "var",
    "varTemplate",
    "singleLine",
    "multiLine",
    "addVarContent",
];

/// 校验命令名：ASCII 字母开头，后随字母/数字/`_`/`-`，且不占用保留名。
pub fn validate_name(name: &str) -> Result<()> {
    let valid = name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if !valid || name.len() > 64 {
        return Err(CommandsError::InvalidName(name.to_string()));
    }
    if RESERVED_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(name))
    {
        return Err(CommandsError::InvalidName(format!("{name} is reserved")));
    }
    Ok(())
}

/// 作用域根目录（不含 `command/<kind>` 段）。
///
/// User 作用域取数据根；Project 作用域由调用方解析：git 主 checkout 优先，
/// 否则工作区根。`None` 表示工作区无可用根（Project 命令不可用）。
pub fn scope_root(scope: CommandScope, project_root: Option<&Path>) -> Result<PathBuf> {
    match scope {
        CommandScope::User => Ok(user_data_root()?),
        CommandScope::Project => project_root
            .map(Path::to_path_buf)
            .ok_or_else(|| CommandsError::InvalidName("project root unavailable".into())),
    }
}

/// 某作用域 + 类型的命令目录。
pub fn commands_dir(scope: CommandScope, root: &Path, kind: CommandKind) -> PathBuf {
    let base = match scope {
        CommandScope::User => root.to_path_buf(),
        CommandScope::Project => root.join(".lx-t"),
    };
    base.join("command").join(kind.dir_name())
}

/// 列出单个目录下的全部有效命令与解析失败项；目录缺失视为空。
pub fn list_dir(dir: &Path, kind: CommandKind) -> (Vec<CommandRecord>, Vec<CommandParseError>) {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (records, errors);
        }
        Err(error) => {
            errors.push(CommandParseError {
                path: dir.to_path_buf(),
                message: error.to_string(),
            });
            return (records, errors);
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    for path in paths {
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            errors.push(CommandParseError {
                path: path.clone(),
                message: "file name is not valid utf-8".into(),
            });
            continue;
        };
        if validate_name(name).is_err() {
            errors.push(CommandParseError {
                path: path.clone(),
                message: format!("invalid command name: {name}"),
            });
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse_file(name, &text, kind, &path) {
                Ok(record) => records.push(record),
                Err(message) => errors.push(CommandParseError {
                    path: path.clone(),
                    message,
                }),
            },
            Err(error) => errors.push(CommandParseError {
                path: path.clone(),
                message: error.to_string(),
            }),
        }
    }
    (records, errors)
}

/// 列出某作用域的全部有效命令与解析失败项。
pub fn list(scope: CommandScope, root: &Path) -> (Vec<CommandRecord>, Vec<CommandParseError>) {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for kind in CommandKind::ALL {
        let (mut found, mut failed) = list_dir(&commands_dir(scope, root, kind), kind);
        records.append(&mut found);
        errors.append(&mut failed);
    }
    records.sort_by(|left, right| left.name.cmp(&right.name));
    (records, errors)
}

/// 合并 User 与 Project 命令：Project 同名覆盖 User；冲突时记录被覆盖的 User 项。
pub fn merge(
    user: Vec<CommandRecord>,
    project: Vec<CommandRecord>,
) -> (Vec<CommandRecord>, Vec<CommandRecord>) {
    let mut overridden = Vec::new();
    let mut merged = Vec::new();
    for record in project {
        if let Some(position) = user.iter().position(|item| item.name == record.name) {
            overridden.push(user[position].clone());
        }
        merged.push(record);
    }
    for record in user {
        if !merged.iter().any(|item| item.name == record.name) {
            merged.push(record);
        }
    }
    merged.sort_by(|left, right| left.name.cmp(&right.name));
    (merged, overridden)
}

/// 解析命令文件：frontmatter 与正文分离；未知键忽略，已知键取值非法报错。
pub fn parse_file(
    name: &str,
    text: &str,
    kind: CommandKind,
    path: &Path,
) -> std::result::Result<CommandRecord, String> {
    validate_name(name).map_err(|error| error.to_string())?;
    let (front, body) = split_frontmatter(text);
    let mut description = String::new();
    let mut argument_hint = None;
    let mut title = None;
    let mut scope = None;
    let mut block = BlockType::Template;
    for line in front.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("frontmatter line is not `key: value`: {line}"));
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').to_string();
        match key {
            "description" => description = value,
            "argument-hint" => argument_hint = Some(value),
            "title" => title = Some(value),
            "scope" => {
                let Some(parsed) = MdScope::parse(&value) else {
                    return Err(format!("invalid scope: {value}"));
                };
                scope = Some(parsed);
            }
            "block" => {
                let Some(parsed) = BlockType::parse(&value) else {
                    return Err(format!("invalid block value: {value}"));
                };
                block = parsed;
            }
            // 未知键忽略，保持与参考实现的前向兼容。
            _ => {}
        }
    }
    let scope = match kind {
        CommandKind::AgentMd => scope.unwrap_or(MdScope::Global),
        CommandKind::AgentBlock => {
            if scope == Some(MdScope::Global) {
                return Err("agentBlock commands cannot be global".into());
            }
            MdScope::Template
        }
    };
    let body = body.trim_end().to_string();
    if body.is_empty() {
        return Err("command body is empty".into());
    }
    Ok(CommandRecord {
        name: name.to_string(),
        scope,
        description,
        argument_hint,
        title,
        body,
        kind,
        block,
        path: path.to_path_buf(),
    })
}

/// 拆分 frontmatter 与正文：仅当文件以 `---` 行开头时按 frontmatter 解析。
fn split_frontmatter(text: &str) -> (&str, &str) {
    let rest = text.strip_prefix("---").and_then(|rest| {
        rest.strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
    });
    let Some(rest) = rest else {
        return ("", text);
    };
    // 结束定界符必须独占一行。
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed == "---" {
            let front = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (front, body);
        }
        offset += line.len();
    }
    ("", text)
}

/// 序列化命令文件：frontmatter（仅非空字段）+ 正文。
pub fn serialize(record: &CommandRecord) -> String {
    let mut out = String::from("---\n");
    if !record.description.is_empty() {
        out.push_str("description: ");
        out.push_str(&record.description);
        out.push('\n');
    }
    if let Some(hint) = record.argument_hint.as_deref()
        && !hint.is_empty()
    {
        out.push_str("argument-hint: ");
        out.push_str(hint);
        out.push('\n');
    }
    match record.kind {
        CommandKind::AgentMd => {
            out.push_str("scope: ");
            out.push_str(record.scope.as_str());
            out.push('\n');
        }
        CommandKind::AgentBlock => {
            out.push_str("block: ");
            out.push_str(record.block.as_str());
            out.push('\n');
            if let Some(title) = record.title.as_deref()
                && !title.is_empty()
            {
                out.push_str("title: ");
                out.push_str(title);
                out.push('\n');
            }
        }
    }
    out.push_str("---\n");
    out.push_str(record.body.trim_end());
    out.push('\n');
    out
}

/// 保存命令到目标作用域：目标已存在时默认拒绝（调用方确认覆盖后可传 `overwrite`）。
///
/// 冲突判定覆盖两个作用域：同名 User 命令存在时 Project 保存需显式覆盖，
/// 防止静默改写另一份文件。
pub fn save(
    scope: CommandScope,
    root: &Path,
    record: &CommandRecord,
    overwrite: bool,
) -> Result<()> {
    validate_name(&record.name)?;
    let dir = commands_dir(scope, root, record.kind);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.md", record.name));
    if path.exists() && !overwrite {
        return Err(CommandsError::AlreadyExists(record.name.clone()));
    }
    write_atomic(&path, &serialize(record))?;
    Ok(())
}

/// 删除命令文件；文件不存在返回 `NotFound`。
pub fn delete(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(CommandsError::NotFound(
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        )),
        Err(error) => Err(CommandsError::Io(error)),
    }
}

/// 重命名命令文件：新名合法、目标不存在时改名；内容不变。
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_name(new_name)?;
    let dir = path
        .parent()
        .ok_or_else(|| CommandsError::InvalidName(new_name.to_string()))?;
    let target = dir.join(format!("{new_name}.md"));
    if target.exists() {
        return Err(CommandsError::AlreadyExists(new_name.to_string()));
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// 同目录临时文件原子写入。
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "command.md".into());
    let temp = path.with_file_name(format!("{name}.tmp.{}", std::process::id()));
    std::fs::write(&temp, content)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(CommandsError::Io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
