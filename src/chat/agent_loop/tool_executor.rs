//! 工具执行器抽象：只读工具面的执行入口与生产文件工具实现。
//!
//! `AgentTools` 的进程/计划/沙箱入口在 `executor.rs`；这里只保留 trait 与
//! 纯文件工具实现，避免把 `agent_loop/mod.rs` 撑到千行以上。

use std::path::Path;

use tokio::sync::watch;

use super::{ReadyToolCall, TOOL_EXECUTION_ABORTED_CODE};
use crate::chat::tools::{ToolContext, ToolErrorCode, ToolExecutionResult};
use crate::db::chat::ToolCallTicket;

pub trait ToolExecutor: Send + Sync {
    /// 执行一次调用（同步实现必须自行移出事件循环线程）。
    fn execute(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
    ) -> impl Future<Output = ToolExecutionResult> + Send;

    /// 带 BeginTool 票据的执行入口：生产执行器据此把 mutation 绑定到真实
    /// session / message / call；默认实现忽略票据（测试执行器无需关心快照）。
    fn execute_tracked(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
        _ticket: &ToolCallTicket,
    ) -> impl Future<Output = ToolExecutionResult> + Send
    where
        Self: Sized,
    {
        self.execute(cwd, call)
    }

    /// 带沙箱覆盖的执行入口（显式提权/on-failure 重试）。
    fn execute_tracked_sandboxed(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
        ticket: &ToolCallTicket,
        sandbox: crate::chat::sandbox::SandboxOverride,
    ) -> impl Future<Output = ToolExecutionResult> + Send
    where
        Self: Sized,
    {
        async move {
            let _ = sandbox;
            self.execute_tracked(cwd, call, ticket).await
        }
    }

    /// 本执行器冻结的沙箱计划（测试执行器返回 None）。
    fn sandbox_plan(&self) -> Option<&crate::chat::sandbox::SandboxPlan> {
        None
    }

    /// 可取消的 MCP 调用；非 MCP 工具返回 None，由调用方走通用执行入口。
    ///
    /// Esc 取消必须直达 `notifications/cancelled`，不能只丢弃 future 让 server
    /// 继续执行副作用。
    fn execute_mcp_cancellable(
        &self,
        _cwd: &Path,
        _call: &ReadyToolCall,
        _cancel: watch::Receiver<bool>,
    ) -> impl Future<Output = Option<ToolExecutionResult>> + Send
    where
        Self: Sized,
    {
        async { None }
    }

    /// 当前模型是否支持图片输入；测试执行器默认支持。
    fn supports_image_input(&self) -> bool {
        true
    }
}

/// 生产执行器：文件工具是阻塞 IO，必须放入阻塞线程池。
#[derive(Debug, Clone, Copy, Default)]
pub struct FileTools;

impl ToolExecutor for FileTools {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let context = ToolContext::new(cwd.to_path_buf());
        let name = call.name.clone();
        let arguments_json = call.arguments_json.clone();
        let call_id = call.call_id.clone();
        let worker_name = name.clone();
        match tokio::task::spawn_blocking(move || {
            crate::chat::tools::execute(&context, &worker_name, &arguments_json)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => {
                // 只记 call_id/工具名/错误类别：不打印参数全文与文件内容。
                tracing::warn!(
                    target: "lx::agent",
                    call_id = %call_id,
                    name = %name,
                    %error,
                    "tool execution task failed"
                );
                ToolExecutionResult::Failed {
                    code: ToolErrorCode::Io,
                    message: TOOL_EXECUTION_ABORTED_CODE.to_string(),
                    changes: Vec::new(),
                }
            }
        }
    }
}
