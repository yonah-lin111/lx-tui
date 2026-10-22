//! 文件工具的**类型化差异视图**（纯数据，渲染层只读）。
//!
//! 由落库的结果 JSON 解码而来（实时结算与历史重载共用同一入口），不读磁盘、不解析
//! 模型参数。坏文件详情局部降级为 [`FileDiffView::Unavailable`]，不拖垮其它文件或会话。

use serde_json::Value;

use crate::chat::tools::{DiffLineKind, FileChangeKind, FileDiff};

/// 差异行（已校验）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLineView {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    pub no_final_newline: bool,
}

/// 差异块。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffHunkView {
    pub lines: Vec<DiffLineView>,
}

/// 单个文件的差异展示状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiffView {
    /// 完整差异（空 hunks = 没有内容变化）。
    Complete {
        hunks: Vec<DiffHunkView>,
        simplified: bool,
    },
    /// 正文被预算截断（计数仍是完整值）。
    Truncated {
        hunks: Vec<DiffHunkView>,
        simplified: bool,
    },
    /// 无法生成文本差异。
    Unavailable { reason: &'static str },
}

impl FileDiffView {
    /// 正文是否完整可展开。
    pub fn is_truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }

    /// 是否来自线性简化脚本。
    pub fn simplified(&self) -> bool {
        matches!(
            self,
            Self::Complete {
                simplified: true,
                ..
            } | Self::Truncated {
                simplified: true,
                ..
            }
        )
    }

    /// 是否没有内容差异。
    pub fn is_empty(&self) -> bool {
        self.hunks().is_empty()
    }

    /// 已保存的 hunks。
    pub fn hunks(&self) -> &[DiffHunkView] {
        match self {
            Self::Complete { hunks, .. } | Self::Truncated { hunks, .. } => hunks,
            Self::Unavailable { .. } => &[],
        }
    }

    /// 由工具层稳定的展示原因码（UI 文案在 `ui::text` 映射）。
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        match self {
            Self::Unavailable { reason } => Some(reason),
            _ => None,
        }
    }
}

/// 单个文件的展示数据（路径 / 动作 / 计数 / 差异）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChangeView {
    pub path: String,
    /// 移动目标。
    pub to: Option<String>,
    pub kind: FileChangeKind,
    pub added: Option<usize>,
    pub removed: Option<usize>,
    pub counts_known: bool,
    pub simplified: bool,
    pub diff: Option<FileDiffView>,
}

/// 一次工具调用的文件详情。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiffDetailView {
    /// 按提交顺序的文件列表。
    pub files: Vec<FileChangeView>,
    /// 调用级完整总计（裁切文件列表前计算）。
    pub files_total: usize,
    pub added: Option<usize>,
    pub removed: Option<usize>,
    pub simplified: bool,
    /// 因元数据超限未保存摘要的文件数。
    pub omitted_files: usize,
    /// 单个文件（`write`）时为 true：正文放在顶层 `diff`，不是 `changes[]`。
    pub single: bool,
}

impl FileDiffDetailView {
    /// 是否没有任何可展示文件（此时不应产生文件详情）。
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.files_total == 0
    }

    /// 由结果 JSON 解码；非文件工具或结构不符返回 None。
    pub fn from_result_json(result_json: Option<&str>) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        let tool = value.get("tool").and_then(Value::as_str)?;
        match tool {
            "edit" | "error" => Self::from_changes(&value),
            "write" => Self::from_write(&value),
            _ => None,
        }
    }

    /// `edit` / `error`：`changes[]` 是唯一来源。
    fn from_changes(value: &Value) -> Option<Self> {
        let changes = value.get("changes")?.as_array()?;
        let files: Vec<FileChangeView> = changes.iter().map(file_change_of).collect();
        let totals = totals_of(value);
        let files_total = totals
            .and_then(|totals| usize_of(totals.get("files")))
            .unwrap_or(files.len());
        let simplified = totals
            .and_then(|totals| totals.get("simplified"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || files.iter().any(|file| file.simplified);
        Some(Self {
            files,
            files_total,
            added: totals.and_then(|totals| usize_of(totals.get("added"))),
            removed: totals.and_then(|totals| usize_of(totals.get("removed"))),
            simplified,
            omitted_files: totals
                .and_then(|totals| usize_of(totals.get("omitted_files")))
                .unwrap_or(0),
            single: false,
        })
    }

    /// `write`：顶层就是单个文件（`diff` 与 `path` 并列）。
    fn from_write(value: &Value) -> Option<Self> {
        let path = value.get("path")?.as_str()?.to_string();
        let file = FileChangeView {
            path,
            to: None,
            kind: FileChangeKind::Updated,
            added: usize_of(value.get("added")),
            removed: usize_of(value.get("removed")),
            counts_known: value
                .get("counts_known")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            simplified: value
                .get("simplified")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            diff: value.get("diff").and_then(diff_of),
        };
        let totals = totals_of(value);
        Some(Self {
            files: vec![file],
            files_total: 1,
            added: totals.and_then(|totals| usize_of(totals.get("added"))),
            removed: totals.and_then(|totals| usize_of(totals.get("removed"))),
            simplified: value
                .get("simplified")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            omitted_files: 0,
            single: true,
        })
    }
}

/// `changes[]` 单项解码；坏字段局部降级。
fn file_change_of(change: &Value) -> FileChangeView {
    let to = change.get("to").and_then(Value::as_str).map(str::to_string);
    let kind = match change.get("kind").and_then(Value::as_str) {
        Some("added") => FileChangeKind::Added,
        Some("deleted") => FileChangeKind::Deleted,
        Some("moved") => FileChangeKind::Moved {
            to: to.clone().map(std::path::PathBuf::from).unwrap_or_default(),
        },
        _ => FileChangeKind::Updated,
    };
    FileChangeView {
        path: change
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        to,
        kind,
        added: usize_of(change.get("added")),
        removed: usize_of(change.get("removed")),
        counts_known: change
            .get("counts_known")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        simplified: change
            .get("simplified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        diff: change.get("diff").and_then(diff_of),
    }
}

/// `diff` 字段解码；版本/状态/行结构不合法时降级为不可用。
fn diff_of(diff: &Value) -> Option<FileDiffView> {
    if diff.get("version").and_then(Value::as_u64) != Some(1) {
        return Some(FileDiffView::Unavailable {
            reason: "unsupported_version",
        });
    }
    let simplified = diff
        .get("simplified")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match diff.get("status").and_then(Value::as_str) {
        Some("unavailable") => Some(FileDiffView::Unavailable {
            reason: match diff.get("reason").and_then(Value::as_str) {
                Some("binary") => "binary",
                Some("not_utf8") => "not_utf8",
                Some("read_failed") => "read_failed",
                _ => "unknown",
            },
        }),
        Some(status @ ("complete" | "truncated")) => {
            // 坏行/坏行号只让这一个文件降级，不能整条会话失败，也不能静默丢掉差异。
            let Some(hunks) = hunks_of(diff) else {
                return Some(FileDiffView::Unavailable { reason: "unknown" });
            };
            Some(if status == "truncated" {
                FileDiffView::Truncated { hunks, simplified }
            } else {
                FileDiffView::Complete { hunks, simplified }
            })
        }
        _ => Some(FileDiffView::Unavailable { reason: "unknown" }),
    }
}

/// hunks 解码；任何一行结构不合法即整份差异降级（不是整条会话失败）。
fn hunks_of(diff: &Value) -> Option<Vec<DiffHunkView>> {
    let hunks = diff.get("hunks")?.as_array()?;
    let mut out = Vec::with_capacity(hunks.len());
    for hunk in hunks {
        let lines = hunk.get("lines")?.as_array()?;
        let mut parsed = Vec::with_capacity(lines.len());
        for line in lines {
            let kind = DiffLineKind::from_stored(line.get("kind")?.as_str()?)?;
            // 行号与种类必须自洽：Added 只有新行号，Removed 只有旧行号。
            let old_line = usize_of(line.get("old_line"));
            let new_line = usize_of(line.get("new_line"));
            let valid = match kind {
                DiffLineKind::Added => old_line.is_none() && new_line.is_some(),
                DiffLineKind::Removed => old_line.is_some() && new_line.is_none(),
                DiffLineKind::Context => old_line.is_some() && new_line.is_some(),
            };
            if !valid {
                return None;
            }
            parsed.push(DiffLineView {
                kind,
                old_line,
                new_line,
                text: line.get("text")?.as_str()?.to_string(),
                no_final_newline: line
                    .get("no_final_newline")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
        out.push(DiffHunkView { lines: parsed });
    }
    Some(out)
}

/// `file_totals` 对象。
fn totals_of(value: &Value) -> Option<&Value> {
    value.get("file_totals").filter(|totals| totals.is_object())
}

/// 可选无符号整数（`null`/缺失 → None，负数/非数字 → None）。
fn usize_of(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(Value::as_u64)
        .map(|number| usize::try_from(number).unwrap_or(usize::MAX))
}

/// 文件差异的**空/缺失**判定：供 `ToolDetailView::from_result_json` 决定是否建分支。
pub fn has_file_detail(value: &Value) -> bool {
    match value.get("tool").and_then(Value::as_str) {
        Some("edit") => value
            .get("changes")
            .and_then(Value::as_array)
            .is_some_and(|changes| !changes.is_empty()),
        Some("error") => value
            .get("changes")
            .and_then(Value::as_array)
            .is_some_and(|changes| !changes.is_empty()),
        Some("write") => value.get("path").is_some(),
        _ => false,
    }
}

impl From<&FileDiff> for FileDiffView {
    /// 工具层差异 → 视图（测试与纯转换用；生产路径统一走 JSON 解码）。
    fn from(diff: &FileDiff) -> Self {
        let hunks = diff
            .hunks()
            .iter()
            .map(|hunk| DiffHunkView {
                lines: hunk
                    .lines
                    .iter()
                    .map(|line| DiffLineView {
                        kind: line.kind,
                        old_line: line.old_line,
                        new_line: line.new_line,
                        text: line.text.clone(),
                        no_final_newline: line.no_final_newline,
                    })
                    .collect(),
            })
            .collect();
        match diff {
            FileDiff::Unavailable { reason } => Self::Unavailable {
                reason: reason.as_str(),
            },
            FileDiff::Truncated { simplified, .. } => Self::Truncated {
                hunks,
                simplified: *simplified,
            },
            FileDiff::Complete { simplified, .. } => Self::Complete {
                hunks,
                simplified: *simplified,
            },
        }
    }
}

#[cfg(test)]
mod tests;
