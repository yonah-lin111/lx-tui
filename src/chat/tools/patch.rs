//! Codex 风格补丁的纯解析模型：只把文本变成 hunk 结构，不做任何文件 IO。
//!
//! 支持的语法子集（刻意不扩展到别的 diff 格式）：
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path            （后续行必须全部以 `+` 开头）
//! *** Update File: path
//! *** Move to: path             （必须在 Update File 之后、第一个 `@@` 之前）
//! @@                            （可选的定位上下文：`@@ <context>`）
//! -old line
//! +new line
//!  context line
//! *** End of File               （可选：本块必须贴住文件末尾）
//! *** Delete File: path
//! *** End Patch
//! ```
//!
//! 语义要点：
//!
//! - 标记行（已知 `***` 标记）允许前后空白，判定前两侧 trim；**正文行绝不 trim**，
//!   行首的 `+` / `-` / ` ` 前缀属于补丁语义。
//! - `*** End of File` 属于**当前块**：该块只在「贴住文件末尾」的位置匹配，
//!   且只能标记本 hunk 的最后一个块（后面不能再出现 `@@` 块）。
//! - `@@` 后只有 `+` 行的**纯插入块**：带 `@@ <context>` 时插在该定位行之后；
//!   只有裸 `@@`（或没有 `@@`）时**追加到文件末尾**（与 codex `file_update.rs`
//!   的 `replacements.push((original_lines.len(), 0, …))` 一致）。
//!
//! 匹配档位（rstrip 回退与唯一性）由执行阶段 [`super::edit`] 负责；本模块只负责
//! 把文本变成结构，不做任何文件 IO，也不判断上下文能否匹配。
//!
//! 不支持的项：统一 diff 头（`---`/`+++`/`@@ -1,2 +1,3 @@` 行号形式）、
//! `*** Environment ID`、heredoc 包裹（`<<'EOF'`）、二进制补丁、Git 元数据行。
//! 出现未知标记一律返回 [`ToolErrorCode::PatchInvalid`]，绝不猜测语义。

use std::path::PathBuf;

use super::types::{ToolErrorCode, ToolFailure, ToolResult};

/// 补丁起始标记。
const BEGIN_MARKER: &str = "*** Begin Patch";
/// 补丁结束标记。
const END_MARKER: &str = "*** End Patch";
/// 新增文件标记前缀。
const ADD_PREFIX: &str = "*** Add File: ";
/// 删除文件标记前缀。
const DELETE_PREFIX: &str = "*** Delete File: ";
/// 更新文件标记前缀。
const UPDATE_PREFIX: &str = "*** Update File: ";
/// 移动目标标记前缀。
const MOVE_PREFIX: &str = "*** Move to: ";
/// 块锚定文件末尾标记。
const EOF_MARKER: &str = "*** End of File";
/// 块定位标记。
const CONTEXT_MARKER: &str = "@@";

/// 一个补丁 hunk。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hunk {
    /// 新建文件；`contents` 已按补丁原样保留行结构。
    Add {
        path: PathBuf,
        /// 文件行（不含换行）。
        lines: Vec<String>,
    },
    /// 删除文件。
    Delete { path: PathBuf },
    /// 更新文件（可同时移动）。
    Update {
        path: PathBuf,
        move_to: Option<PathBuf>,
        blocks: Vec<Block>,
    },
}

impl Hunk {
    /// 受影响的源路径。
    pub(crate) fn path(&self) -> &PathBuf {
        match self {
            Self::Add { path, .. } | Self::Delete { path } | Self::Update { path, .. } => path,
        }
    }

    /// 移动目标（无移动为 None）。
    pub(crate) fn move_to(&self) -> Option<&PathBuf> {
        match self {
            Self::Update { move_to, .. } => move_to.as_ref(),
            _ => None,
        }
    }
}

/// `@@` 上下文块：`context` 是定位行（必须唯一匹配），`body` 是逐行变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    /// `@@ <context>` 的上下文（`@@` 单独出现时为 None）。
    pub(crate) context: Option<String>,
    /// 补丁中的行；每项前缀已剥离，`kind` 说明其语义。
    pub(crate) lines: Vec<BodyLine>,
    /// 是否带 `*** End of File` 标记：为真时该块只能贴住文件末尾匹配。
    pub(crate) at_eof: bool,
}

/// 块内一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BodyLine {
    pub(crate) kind: BodyLineKind,
    /// 已剥离前缀的行文本。
    pub(crate) text: String,
}

/// 块内行的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyLineKind {
    /// ` ` 上下文行（必须存在于原文）。
    Context,
    /// `-` 删除行（必须存在于原文）。
    Remove,
    /// `+` 新增行。
    Add,
}

/// 解析补丁文本为 hunk 列表。
///
/// 只校验语法结构（标记顺序、缩进前缀、非空路径、重复路径）；文件是否存在、
/// 上下文是否匹配由执行阶段判断，以便先完成全量格式校验再动文件。
pub(crate) fn parse(patch: &str) -> ToolResult<Vec<Hunk>> {
    let trimmed = patch.trim();
    if trimmed.is_empty() {
        return Err(invalid("patch is empty"));
    }
    let mut lines = trimmed.lines().peekable();
    let first = lines.next().unwrap_or_default().trim();
    if first != BEGIN_MARKER {
        return Err(invalid("patch must start with '*** Begin Patch'"));
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut closed = false;
    while let Some(line) = lines.next() {
        // 顶层只出现标记行；`***` 开头的行允许两侧空白（codex strict 同样 trim）。
        let line = line.trim();
        if line == END_MARKER {
            closed = true;
            break;
        }
        if let Some(rest) = line.strip_prefix(ADD_PREFIX) {
            let path = patch_path(rest)?;
            hunks.push(parse_add(path, &mut lines)?);
            continue;
        }
        if let Some(rest) = line.strip_prefix(DELETE_PREFIX) {
            let path = patch_path(rest)?;
            hunks.push(Hunk::Delete { path });
            continue;
        }
        if let Some(rest) = line.strip_prefix(UPDATE_PREFIX) {
            let path = patch_path(rest)?;
            hunks.push(parse_update(path, &mut lines)?);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        return Err(invalid(format!(
            "unexpected line in patch: '{line}' (expected a file marker or '*** End Patch')"
        )));
    }
    if !closed {
        return Err(invalid("patch must end with '*** End Patch'"));
    }
    if hunks.is_empty() {
        return Err(invalid("patch contains no file hunks"));
    }
    reject_duplicate_targets(&hunks)?;
    Ok(hunks)
}

/// 解析 `*** Add File` 之后的行：必须全部以 `+` 开头，直到下一个标记。
fn parse_add<'a, I>(path: PathBuf, lines: &mut std::iter::Peekable<I>) -> ToolResult<Hunk>
where
    I: Iterator<Item = &'a str>,
{
    let mut body = Vec::new();
    while let Some(line) = lines.peek() {
        if marker(line).is_some() {
            break;
        }
        // 正文行只看 `+` 前缀与行尾；前导空格（`+` 之后的内容）属于正文。
        let line = line.trim_end();
        let Some(rest) = line.strip_prefix('+') else {
            return Err(invalid(format!(
                "'*** Add File: {}' body lines must start with '+'",
                path.display()
            )));
        };
        body.push(rest.to_string());
        lines.next();
    }
    if body.is_empty() {
        return Err(invalid(format!(
            "'*** Add File: {}' has no content lines",
            path.display()
        )));
    }
    Ok(Hunk::Add { path, lines: body })
}

/// 解析 `*** Update File` 之后的 `Move to`、`@@` 块与变更行。
fn parse_update<'a, I>(path: PathBuf, lines: &mut std::iter::Peekable<I>) -> ToolResult<Hunk>
where
    I: Iterator<Item = &'a str>,
{
    let mut move_to = None;
    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;
    let mut seen_context = false;
    let mut seen_eof = false;
    while let Some(line) = lines.peek() {
        // 标记行（`***` 开头）才允许前后空白；正文行只剥行尾，前导空格是前缀语义。
        if let Some(marker) = marker(line) {
            if let Some(rest) = marker.strip_prefix(MOVE_PREFIX) {
                if seen_context {
                    return Err(invalid(format!(
                        "'*** Move to:' must appear before the first '@@' block (file {})",
                        path.display()
                    )));
                }
                if move_to.is_some() {
                    return Err(invalid(format!(
                        "'*** Move to:' appears more than once for {}",
                        path.display()
                    )));
                }
                move_to = Some(patch_path(rest)?);
                lines.next();
                continue;
            }
            if marker == EOF_MARKER {
                let Some(block) = current.as_mut() else {
                    return Err(invalid(format!(
                        "'*** End of File' must follow the changed lines of a block (file {})",
                        path.display()
                    )));
                };
                if block.lines.is_empty() {
                    return Err(invalid(format!(
                        "'*** End of File' must follow the changed lines of a block (file {})",
                        path.display()
                    )));
                }
                if block.at_eof {
                    return Err(invalid(format!(
                        "'*** End of File' appears more than once for {}",
                        path.display()
                    )));
                }
                block.at_eof = true;
                seen_eof = true;
                lines.next();
                continue;
            }
            break;
        }
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix(CONTEXT_MARKER) {
            if seen_eof {
                return Err(invalid(format!(
                    "'*** End of File' must mark the last block of '*** Update File: {}'; \
                     no '@@' block may follow it",
                    path.display()
                )));
            }
            if let Some(block) = current.take() {
                blocks.push(block);
            }
            seen_context = true;
            let context = rest.trim();
            current = Some(Block {
                context: (!context.is_empty()).then(|| context.to_string()),
                lines: Vec::new(),
                at_eof: false,
            });
            lines.next();
            continue;
        }
        if seen_eof {
            // `*** End of File` 之后只允许空行（codex 的样例里也有这一行）。
            if line.is_empty() {
                lines.next();
                continue;
            }
            return Err(invalid(format!(
                "no changed lines may follow '*** End of File' (file {})",
                path.display()
            )));
        }
        let Some(prefix) = line.chars().next() else {
            // 空行：作为上下文空行处理（模型常把空行写成空字符串）。
            current
                .get_or_insert_with(empty_block)
                .lines
                .push(BodyLine {
                    kind: BodyLineKind::Context,
                    text: String::new(),
                });
            lines.next();
            continue;
        };
        let kind = match prefix {
            '+' => BodyLineKind::Add,
            '-' => BodyLineKind::Remove,
            ' ' => BodyLineKind::Context,
            _ => {
                return Err(invalid(format!(
                    "line in '*** Update File: {}' must start with '+', '-' or ' '",
                    path.display()
                )));
            }
        };
        let text = line
            .get(prefix.len_utf8()..)
            .unwrap_or_default()
            .to_string();
        current
            .get_or_insert_with(empty_block)
            .lines
            .push(BodyLine { kind, text });
        lines.next();
    }
    if let Some(block) = current.take() {
        blocks.push(block);
    }
    if blocks.is_empty() {
        return Err(invalid(format!(
            "'*** Update File: {}' has no changes",
            path.display()
        )));
    }
    if blocks.iter().all(|block| block.lines.is_empty()) {
        return Err(invalid(format!(
            "'*** Update File: {}' has no changes",
            path.display()
        )));
    }
    Ok(Hunk::Update {
        path,
        move_to,
        blocks,
    })
}

/// 未显式 `@@` 的块：第一段变更行隐式属于它。
fn empty_block() -> Block {
    Block {
        context: None,
        lines: Vec::new(),
        at_eof: false,
    }
}

/// 识别补丁结构标记（已知的 `***` 标记），返回两侧 trim 后的形式；正文行返回 None。
///
/// 只对标记行 trim：正文行首的 ` ` / `-` / `+` 是前缀语义，绝不能 trim。
/// 取舍：为了接受 `  *** End Patch  ` 这类缩进标记，标记判定前会去掉前导空白；
/// 只有当正文行（剥掉前缀后）**恰好等于**某个已知标记时才会歧义（如上下文行
/// 内容就是 `*** End of File`），这在实际代码里不可见，而缩进标记是模型常见产出。
fn marker(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let known = trimmed == END_MARKER
        || trimmed == EOF_MARKER
        || trimmed.starts_with(ADD_PREFIX)
        || trimmed.starts_with(DELETE_PREFIX)
        || trimmed.starts_with(UPDATE_PREFIX)
        || trimmed.starts_with(MOVE_PREFIX);
    known.then_some(trimmed)
}

/// 解析并校验补丁中的文件路径文本。
///
/// 只接受非空、不含 NUL 与换行的路径；绝对路径允许（由调用方按工作区规则解析）。
fn patch_path(raw: &str) -> ToolResult<PathBuf> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(invalid("file path in patch is empty"));
    }
    if text.contains('\0') || text.contains('\n') {
        return Err(invalid("file path in patch contains invalid characters"));
    }
    Ok(PathBuf::from(text))
}

/// 拒绝同一补丁内重复的目标：先全量校验再执行，避免半途才发现冲突。
fn reject_duplicate_targets(hunks: &[Hunk]) -> ToolResult<()> {
    let mut seen: Vec<String> = Vec::new();
    for hunk in hunks {
        for path in std::iter::once(hunk.path()).chain(hunk.move_to()) {
            let key = crate::platform::path_key(path);
            if seen.contains(&key) {
                return Err(invalid(format!(
                    "patch targets the same path more than once: {}",
                    path.display()
                )));
            }
            seen.push(key);
        }
    }
    Ok(())
}

/// 构造补丁格式错误。
fn invalid(message: impl Into<String>) -> ToolFailure {
    ToolFailure::new(ToolErrorCode::PatchInvalid, message)
}

#[cfg(test)]
mod tests;
