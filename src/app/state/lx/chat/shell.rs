//! 用户 shell 命令的纯状态：命令记录视图、待提交草稿与队列快照。
//!
//! 命令**不占用**模型 `RequestState`：模型流与本地命令可以同时进行。执行事实、
//! 进程句柄与数据库写入都在事件层；这里只保存渲染与状态机需要的纯数据。

use std::path::PathBuf;

use super::tool_view::ProcessStateView;

/// 一条用户 shell 命令的展示记录（与落库 JSON 同构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserShellView {
    /// 用户输入的原始脚本（已去模式前缀，保留换行与引号）。
    pub command: String,
    /// 实际执行目录。
    pub cwd: String,
    /// 实际使用的 shell。
    pub shell: String,
    pub state: ProcessStateView,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    /// 受控输出（已剥离控制序列；结束态为数据库保存的同一份快照）。
    pub output: String,
    pub output_start: u64,
    pub output_end: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    /// 运行实例代次（跨轮事件归属）。
    pub runtime_epoch: Option<String>,
    /// 进程会话 ID（on_spawn 后才有）。
    pub process_id: Option<u64>,
    /// 单调修订号：迟到的 Running 不能覆盖终态。
    pub revision: u64,
}

impl UserShellView {
    /// 命令是否仍在运行（尚无终态）。
    pub fn is_running(&self) -> bool {
        self.state.is_running()
    }

    /// 是否失败（进程级失败；非零退出是命令结果，不算失败）。
    pub fn is_failed(&self) -> bool {
        self.state.is_process_failure() || self.state.is_nonzero_exit()
    }

    /// 文案键：运行中 / 已运行。
    pub fn label_running(&self) -> bool {
        self.is_running()
    }

    /// 受控输出是否被截断（丢弃了更早的字节）。
    pub fn output_truncated(&self) -> bool {
        self.truncated || self.dropped_bytes > 0
    }

    /// 消息状态列：运行中 `streaming`；正常退出（含非零）为 `complete`。
    pub fn message_status(&self) -> &'static str {
        match self.state {
            ProcessStateView::Starting | ProcessStateView::Running => "streaming",
            ProcessStateView::Exited { .. } => "complete",
            ProcessStateView::Failed => "failed",
            ProcessStateView::Stopped
            | ProcessStateView::TimedOut
            | ProcessStateView::Interrupted => "interrupted",
        }
    }
}

/// 一次在途提交：冻结草稿修订与命令，等待 spawn 回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingShell {
    /// 窗格内单调代号（与 `BeginUserShell` 效应一一对应）。
    pub generation: u64,
    /// 冻结时的输入修订号：只有它仍匹配才允许清空草稿。
    pub revision: u64,
    /// 冻结的命令正文（trim 后）。
    pub command: String,
    /// 提交时绑定的会话（可为 None，表示命令会创建会话）。
    pub session: Option<String>,
}

/// 排队中的下一次提交（纯数据，按窗格/会话隔离）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedSubmission {
    /// shell 命令：保存完整序列化文本与冻结执行目录。
    Shell { full_text: String, cwd: PathBuf },
    /// 普通聊天草稿：等独立 shell 结束后再走正常发送校验。
    Chat { draft: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(state: ProcessStateView) -> UserShellView {
        UserShellView {
            command: "pwd".into(),
            cwd: "/tmp".into(),
            shell: "/bin/zsh".into(),
            state,
            exit_code: None,
            duration_ms: None,
            output: String::new(),
            output_start: 0,
            output_end: 0,
            dropped_bytes: 0,
            truncated: false,
            runtime_epoch: None,
            process_id: None,
            revision: 0,
        }
    }

    #[test]
    fn running_state_is_not_terminal() {
        assert!(view(ProcessStateView::Running).is_running());
        assert!(!view(ProcessStateView::Exited { code: Some(0) }).is_running());
    }
}
