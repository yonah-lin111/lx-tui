//! 四工具的模型可见规格：稳定英文名、英文描述与严格 JSON Schema。
//!
//! Schema 只声明工具真正接受的字段（`additionalProperties: false`），`read` 的
//! `offset` 明确写为 1-based；运行时仍会重新校验，不信任模型遵守 schema。

use serde_json::{Value, json};

use super::types::ToolSpec;
use super::{MAX_READ_LINES, MAX_SEARCH_RESULTS};

/// 固定工具表：顺序即给模型的呈现顺序。
static TOOLS: [ToolSpec; 4] = [
    ToolSpec::new(
        "read",
        "Read a UTF-8 text file, or list a directory. \
         Returns numbered lines as `{line}: {text}` with 1-based line numbers. \
         `offset` is a 1-based first line (omitted = start at line 1) and `limit` caps \
         the number of returned lines. Long output is truncated explicitly and reports \
         `next_offset` so you can continue reading.",
        read_parameters,
    ),
    ToolSpec::new(
        "search",
        "Search file contents under a directory (default: the workspace root) using a \
         regular expression, or a literal string when `literal` is true. Respects \
         .gitignore/.ignore rules. Returns `path:line: text` matches sorted by path and \
         line. Truncation (limit, file cap or timeout) is reported explicitly.",
        search_parameters,
    ),
    ToolSpec::new(
        "edit",
        "Apply a Codex-style patch to one or more files. The patch text must start with \
         `*** Begin Patch` and end with `*** End Patch` and may contain `*** Add File:`, \
         `*** Update File:` (optionally followed by `*** Move to:`), `@@` context blocks \
         with `-`/`+`/space prefixed lines, and `*** Delete File:` hunks. Context lines \
         must match the current file uniquely; otherwise the whole tool call fails. \
         Multiple files may be partially applied; the result lists every file that was \
         actually changed.",
        edit_parameters,
    ),
    ToolSpec::new(
        "write",
        "Create a file or replace its entire UTF-8 text content. `content` is the \
         complete new file body (an empty string clears the file); this never appends. \
         Missing parent directories are created. Returns whether the file was created \
         and how many lines were added or removed.",
        write_parameters,
    ),
];

/// 全部可用工具规格（固定四个）。
pub fn available_tools() -> &'static [ToolSpec] {
    &TOOLS
}

/// `read` 参数 schema。
fn read_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "File or directory path; relative paths resolve against the workspace root."
            },
            "offset": {
                "type": "integer",
                "minimum": 1,
                "description": "1-based first line to read (default 1)."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "description": format!("Maximum number of lines to return (default and cap {MAX_READ_LINES}).")
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

/// `search` 参数 schema。
fn search_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Regular expression, or a literal string when `literal` is true."
            },
            "path": {
                "type": "string",
                "description": "Directory or file to search (default '.' = workspace root); absolute paths may point anywhere."
            },
            "include": {
                "type": "string",
                "description": "Glob filter applied to candidate paths, e.g. '*.rs' or 'src/**/*.ts'."
            },
            "literal": {
                "type": "boolean",
                "description": "Treat `pattern` as a literal string instead of a regular expression."
            },
            "case_sensitive": {
                "type": "boolean",
                "description": "Match case sensitively (default false = case insensitive)."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "description": format!("Maximum number of matches to return (default and cap {MAX_SEARCH_RESULTS}).")
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    })
}

/// `edit` 参数 schema：只有一个字符串字段 `patch`。
fn edit_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "patch": {
                "type": "string",
                "description": "Complete patch text from '*** Begin Patch' to '*** End Patch'."
            }
        },
        "required": ["patch"],
        "additionalProperties": false
    })
}

/// `write` 参数 schema。
fn write_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Target file path; relative paths resolve against the workspace root."
            },
            "content": {
                "type": "string",
                "description": "Complete new file content (empty string clears the file)."
            }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests;
