//! 七工具的模型可见规格：稳定英文名、英文描述与严格 JSON Schema。
//!
//! Schema 只声明工具真正接受的字段（`additionalProperties: false`），`read` 的
//! `offset` 明确写为 1-based；运行时仍会重新校验，不信任模型遵守 schema。
//!
//! 三个进程/计划工具的字段白名单、默认值与上限与 [`super::process`] /
//! [`super::plan`] 的常量**同一来源**：schema 里的数字由常量生成，不手写第二份。

use serde_json::{Value, json};

use super::types::ToolSpec;
use super::{MAX_READ_LINES, MAX_SEARCH_RESULTS};
use crate::chat::plan::{MAX_EXPLANATION_CHARS, MAX_PLAN_STEPS, MAX_STEP_CHARS};
use crate::chat::process::{
    DEFAULT_STDIN_YIELD_MS, DEFAULT_YIELD_MS, MAX_OUTPUT_TOKENS, MAX_TIMEOUT_MS, MAX_YIELD_MS,
};

/// 固定工具表：顺序即给模型的呈现顺序。
static TOOLS: [ToolSpec; 7] = [
    ToolSpec::new(
        "read",
        "Read a UTF-8 text file, or list a directory. \
         Returns numbered lines as `{line}: {text}` with 1-based line numbers. \
         `offset` is a 1-based first line (omitted = start at line 1) and `limit` caps \
         the number of returned lines. Long output is truncated explicitly and reports \
         `next_offset` so you can continue reading. A single file larger than 16 MiB is \
         rejected with a `too_large` error (the limit applies to the bytes read into \
         memory, not to the returned lines); use `search` to locate the content first.",
        read_parameters,
    ),
    ToolSpec::new(
        "search",
        "Search file contents under a directory (default: the workspace root) using a \
         regular expression, or a literal string when `literal` is true. Reaches the same \
         files ripgrep does by default: `.git/` and `.jj/` are always skipped, \
         `.gitignore`/`.ignore` rules are honored (only inside a real git repository), and \
         hidden files such as `.env` ARE searched. Returns `path:line: text` matches sorted \
         by path and line. Truncation (limit, file cap, depth cap or timeout) is reported \
         explicitly via `truncated`.",
        search_parameters,
    ),
    ToolSpec::new(
        "edit",
        "Apply a Codex-style patch to one or more files. The patch text must start with \
         `*** Begin Patch` and end with `*** End Patch` and may contain `*** Add File:`, \
         `*** Update File:` (optionally followed by `*** Move to:`), `@@` context blocks \
         with `-`/`+`/space prefixed lines, `*** Delete File:` hunks and a trailing \
         `*** End of File` marker that pins a block to the end of the file. Context lines \
         must match the current file uniquely; otherwise the whole tool call fails. \
         A block with only `+` lines inserts right after its `@@ <context>` line, or at \
         the end of the file when there is no context line. `*** Move to:` refuses an \
         existing destination. Multiple files may be partially applied; the result lists \
         every file that was actually changed.",
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
    ToolSpec::new(
        "exec_command",
        "Run a shell command in the workspace and return its output. The command runs \
         directly without an approval prompt. `workdir` defaults to the frozen workspace \
         directory and relative paths in it resolve against that directory. `tty: true` \
         runs the command in a dedicated pseudo terminal (fixed 120x40) so that it can be \
         driven with `write_stdin`; the default is a plain pipe. The call returns as soon \
         as the yield window expires: a still-running command comes back with \
         `process_state: \"running\"` and a `session_id` that can be used by `write_stdin` \
         in later turns. A non-zero exit code means the command ran and failed; it is \
         reported as an error result with the real output and exit code.",
        exec_command_parameters,
    ),
    ToolSpec::new(
        "write_stdin",
        "Write to the stdin of a process started by `exec_command`, or read its further \
         output. `chars` defaults to an empty string, which only reads new output; a \
         non-empty value is written verbatim and no newline is appended (include `\\n` to \
         submit a line). The explicit `terminate: true` extension stops the process by \
         `session_id` (it cannot be combined with non-empty `chars`) and returns its final \
         state and remaining output. Only output not yet returned to the model is \
         returned; `output_start`/`output_end` are byte offsets in the raw output and \
         `dropped_bytes` reports output that was discarded before it could be read.",
        write_stdin_parameters,
    ),
    ToolSpec::new(
        "update_plan",
        "Replace the whole task plan for this conversation. Every call replaces the \
         previous plan; there is no patching or matching by text. Each step has a short \
         `step` text and a `status` of `pending`, `in_progress`, or `completed`, and at \
         most one step may be `in_progress`. An empty `plan` array clears the plan (that \
         does not mean every previous step is complete). The result contains the \
         normalized plan snapshot; the plan is shown to the user as a checklist.",
        update_plan_parameters,
    ),
];

/// 全部可用工具规格（固定七个）。
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

/// `exec_command` 参数 schema。
fn exec_command_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cmd": {
                "type": "string",
                "description": "Shell command to run verbatim; blank commands are rejected."
            },
            "workdir": {
                "type": "string",
                "description": "Working directory (default: the frozen workspace root); relative paths resolve against the workspace root."
            },
            "shell": {
                "type": "string",
                "description": "Shell program to use (default: the platform shell)."
            },
            "login": {
                "type": "boolean",
                "description": "Start a login shell so the user's PATH and aliases apply (default true; pass false for a bare non-login shell)."
            },
            "tty": {
                "type": "boolean",
                "description": "Run in a dedicated pseudo terminal so the command can be driven with `write_stdin` (default false)."
            },
            "yield_time_ms": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_YIELD_MS,
                "description": format!("How long to wait for output before returning (default {DEFAULT_YIELD_MS}, max {MAX_YIELD_MS}); this never kills the process.")
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_TIMEOUT_MS,
                "description": "Total lifetime limit for the command; when it expires the process is stopped (default: no limit)."
            },
            "max_output_tokens": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_OUTPUT_TOKENS,
                "description": format!("Approximate token budget for the returned output (max {MAX_OUTPUT_TOKENS}); the rest stays readable with `write_stdin`.")
            }
        },
        "required": ["cmd"],
        "additionalProperties": false
    })
}

/// `write_stdin` 参数 schema。
fn write_stdin_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "session_id": {
                "type": "integer",
                "minimum": 1,
                "description": "Process session id returned by `exec_command`."
            },
            "chars": {
                "type": "string",
                "description": "Bytes to write to stdin; empty (default) only reads new output. No newline is appended."
            },
            "terminate": {
                "type": "boolean",
                "description": "Stop the process instead of writing (default false); cannot be combined with non-empty `chars`."
            },
            "yield_time_ms": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_YIELD_MS,
                "description": format!("How long to wait for new output (default {DEFAULT_STDIN_YIELD_MS}, max {MAX_YIELD_MS}).")
            },
            "max_output_tokens": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_OUTPUT_TOKENS,
                "description": format!("Approximate token budget for the returned output (max {MAX_OUTPUT_TOKENS}).")
            }
        },
        "required": ["session_id"],
        "additionalProperties": false
    })
}

/// `update_plan` 参数 schema。
fn update_plan_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "explanation": {
                "type": "string",
                "description": format!("Optional short explanation of the change (max {MAX_EXPLANATION_CHARS} characters).")
            },
            "plan": {
                "type": "array",
                "maxItems": MAX_PLAN_STEPS,
                "description": format!("Complete plan; replaces the previous one (max {MAX_PLAN_STEPS} steps, empty array clears it)."),
                "items": {
                    "type": "object",
                    "properties": {
                        "step": {
                            "type": "string",
                            "description": format!("Short step description (max {MAX_STEP_CHARS} characters).")
                        },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "completed"],
                            "description": "Step status; at most one step may be `in_progress`."
                        }
                    },
                    "required": ["step", "status"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["plan"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests;
