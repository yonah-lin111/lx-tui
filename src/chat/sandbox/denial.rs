//! 沙箱拒绝的启发式识别（对齐 Codex `orchestrator` 的 on-failure 路径）。
//!
//! 只做一次判定：命中后由调用方最多重试一次，绝不循环。停止/超时/启动失败不参与，
//! 避免把用户取消误判成沙箱拒绝。

use crate::chat::process::{ProcessResult, ProcessState};

/// 输出中出现任一关键词才可能是沙箱拒绝（小写比较）。
const DENIAL_KEYWORDS: [&str; 7] = [
    "operation not permitted",
    "permission denied",
    "read-only file system",
    "sandbox",
    "seccomp",
    "landlock",
    "failed to write file",
];

/// 本次命令是否很可能被沙箱拒绝。
pub(crate) fn is_likely_sandbox_denied(result: &ProcessResult, process_sandboxed: bool) -> bool {
    if !process_sandboxed {
        return false;
    }
    let ProcessState::Exited { code } = result.state else {
        return false;
    };
    let Some(code) = code else {
        return false;
    };
    if code == 0 {
        return false;
    }
    // 用法错误与“命令不存在”不是沙箱边界。
    if matches!(code, 2 | 126 | 127) {
        return false;
    }
    // Linux seccomp SIGSYS 直接判定。
    #[cfg(target_os = "linux")]
    if code == 128 + 31 {
        return true;
    }
    let output = result.output.to_lowercase();
    DENIAL_KEYWORDS
        .iter()
        .any(|keyword| output.contains(keyword))
}

#[cfg(test)]
mod tests;
