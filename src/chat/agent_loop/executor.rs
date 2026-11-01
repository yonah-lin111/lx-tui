//! 生产工具执行器：文件工具走 `spawn_blocking`，进程/计划工具走会话持有的运行实例。
//!
//! 分派规则（对应 design §5 / §8.1）：
//!
//! ```text
//! exec_command -> ProcessManager::exec（跨轮复用同一实例）
//! write_stdin  -> ProcessManager::interact / stop
//! update_plan  -> 纯校验 + 修订号分配（提交由 FinishTool 后的落库完成）
//! 其它         -> 既有两个文件工具执行器（保持 `spawn_blocking`）
//! ```
//!
//! BeginTool 门槛由 agent loop 原调用链先取得：执行器入口不会被绕过来 spawn 进程。
//! 执行器不修改 `AppState`、不写数据库、不碰 UI：结果只经 `ToolExecutionResult` 返回。

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::chat::plan::PlanSnapshot;
use crate::chat::process::{ProcessManager, ProcessResultKind, Rtk};
use crate::chat::tools::{
    MutationRecorder, MutationRecorderFactory, ToolExecutionResult, build_plan_result,
    parse_exec_args, parse_stdin_args, process_failure,
};
use crate::db::chat::ToolCallTicket;

use super::{ReadyToolCall, ToolExecutor};

/// 一轮执行上下文：本轮新建的进程集合与当前正在等待/交互的进程。
///
/// 供 D9 的停止语义使用：收到停止请求时清理「本轮新建集合 ∪ 当前活动交互」，
/// 本轮曾经读过、但此刻不再等待的旧服务不因这次停止被清理。
#[derive(Debug, Default)]
pub struct TurnExecution {
    /// 本轮新建的进程会话 ID。
    pub created: std::collections::BTreeSet<u64>,
    /// 当前正在等待/交互的进程（工具调用存续期间）。
    pub active_interaction: Option<u64>,
}

impl TurnExecution {
    /// 停止目标 = 本轮新建 ∪ 当前活动交互。
    pub fn stop_targets(&self) -> std::collections::BTreeSet<u64> {
        let mut targets = self.created.clone();
        if let Some(active) = self.active_interaction {
            targets.insert(active);
        }
        targets
    }

    /// 在 `exec_command` **spawn 成功后立刻**登记该进程。
    ///
    /// 与 [`Self::observe`] 的区别是时机：`observe` 只在工具返回后才被调用，而
    /// 命令的等待窗口默认 10s（上限 30s）。不提前登记的话，用户在等待窗口里按
    /// Esc 时 `stop_targets()` 是空集，命令会一直跑在后台。
    fn observe_spawned(&mut self, session_id: u64) {
        self.created.insert(session_id);
        self.active_interaction = Some(session_id);
    }

    /// 记录本次调用涉及的进程（新建或交互）。
    fn observe(&mut self, name: &str, result: &ToolExecutionResult) {
        let ToolExecutionResult::Process(result) = result else {
            return;
        };
        if result.kind == ProcessResultKind::ExecCommand {
            self.created.insert(result.session_id);
        }
        // 仍在运行的进程是本轮“当前活动交互”；已退出则清除，避免误杀历史服务。
        if result.state.is_terminal() {
            if self.active_interaction == Some(result.session_id) {
                self.active_interaction = None;
            }
        } else {
            self.active_interaction = Some(result.session_id);
        }
        let _ = name;
    }
}

/// 生产执行器：持有会话进程管理器、计划修订计数与本轮执行上下文。
///
/// `Clone` 只复制共享句柄（进程管理器、修订计数、本轮上下文共用同一份），
/// 因此「同一轮的多个执行器」不会各自持有互相矛盾的进程状态。
#[derive(Clone)]
pub struct AgentTools {
    processes: Arc<ProcessManager>,
    turn: Arc<Mutex<TurnExecution>>,
    /// 计划修订计数（每次成功 update_plan 单调递增）。
    plan_revision: Arc<std::sync::atomic::AtomicU64>,
    /// 文件 mutation recorder 工厂；生产入口必注入，测试可留空。
    recorder: Option<Arc<dyn MutationRecorderFactory>>,
    /// 会话级读版本登记表（读后版本保护）；生产入口必注入。
    reads: Option<Arc<crate::chat::tools::ReadRegistry>>,
    /// 当前模型是否支持图像输入模态。
    supports_image_input: bool,
    /// Exa MCP API Key（联网搜索优先引擎）。
    exa_api_key: String,
    /// Tavily REST API Key（联网搜索回退引擎）。
    tavily_api_key: String,
    /// rtk 后端（外部优先，缺失时内置过滤）；随执行器复用检测结果。
    rtk: Rtk,
    /// 本轮的 MCP 管理器与冻结快照；None = 本轮没有 MCP。
    mcp: Option<McpRuntime>,
    /// 会话 id：工具结果图片归档目录。
    session_id: String,
}

/// 本轮 MCP 依赖。
#[derive(Clone)]
struct McpRuntime {
    manager: Arc<crate::mcp::McpManager>,
    snapshot: Arc<crate::mcp::McpToolSnapshot>,
}

impl std::fmt::Debug for AgentTools {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentTools")
            .field("owner", self.processes.owner())
            .finish_non_exhaustive()
    }
}

impl AgentTools {
    /// 以会话进程管理器构造执行器（`turn` 由事件层在创建轮次时传入同一实例）。
    pub fn new(processes: Arc<ProcessManager>, turn: Arc<Mutex<TurnExecution>>) -> Self {
        Self {
            processes,
            turn,
            plan_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            recorder: None,
            reads: None,
            supports_image_input: true,
            exa_api_key: crate::config::ai::DEFAULT_EXA_API_KEY.to_string(),
            tavily_api_key: crate::config::ai::DEFAULT_TAVILY_API_KEY.to_string(),
            rtk: Rtk::detect(),
            mcp: None,
            session_id: String::new(),
        }
    }

    /// 注入每轮 mutation recorder 工厂（生产路径必须调用）。
    pub fn with_recorder(mut self, recorder: Arc<dyn MutationRecorderFactory>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// 注入会话级读版本登记表（生产路径必须调用；`read` 据此登记、写工具据此核对）。
    pub fn with_reads(mut self, reads: Arc<crate::chat::tools::ReadRegistry>) -> Self {
        self.reads = Some(reads);
        self
    }

    /// 注入当前模型是否支持图像输入（生产路径按 `FrozenModel` 设置）。
    pub fn with_image_support(mut self, supports: bool) -> Self {
        self.supports_image_input = supports;
        self
    }

    /// 注入联网搜索的两把 API Key（生产路径取当前 `AiSettings.web_search`）。
    pub fn with_web_search_keys(
        mut self,
        exa_api_key: impl Into<String>,
        tavily_api_key: impl Into<String>,
    ) -> Self {
        self.exa_api_key = exa_api_key.into();
        self.tavily_api_key = tavily_api_key.into();
        self
    }

    /// 当前模型是否支持图片输入。
    pub fn supports_image_input(&self) -> bool {
        self.supports_image_input
    }

    /// 注入本轮 MCP 管理器、冻结快照与会话 id（图片归档目录）。
    pub fn with_mcp(
        mut self,
        manager: Arc<crate::mcp::McpManager>,
        snapshot: Arc<crate::mcp::McpToolSnapshot>,
        session_id: String,
    ) -> Self {
        self.mcp = Some(McpRuntime { manager, snapshot });
        self.session_id = session_id;
        self
    }

    /// MCP 工具：按冻结快照反查 server / 原始工具名并调用。
    async fn execute_mcp(
        &self,
        call: &ReadyToolCall,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> ToolExecutionResult {
        let Some(runtime) = &self.mcp else {
            return crate::chat::tools::ToolFailure::new(
                crate::chat::tools::ToolErrorCode::UnknownTool,
                "mcp tools are not available in this turn",
            )
            .into_result(Vec::new());
        };
        let Some(entry) = runtime.snapshot.tool(&call.name).cloned() else {
            return crate::chat::tools::ToolFailure::new(
                crate::chat::tools::ToolErrorCode::UnknownTool,
                format!("mcp tool no longer available: {}", call.name),
            )
            .into_result(Vec::new());
        };
        let arguments = match serde_json::from_str::<serde_json::Value>(&call.arguments_json) {
            Ok(value) => value,
            Err(error) => {
                return crate::chat::tools::ToolFailure::invalid(format!(
                    "arguments are not valid JSON: {error}"
                ))
                .into_result(Vec::new());
            }
        };
        let started = std::time::Instant::now();
        let outcome = runtime
            .manager
            .call_tool(
                &runtime.snapshot,
                &self.session_id,
                &call.name,
                arguments,
                cancel,
            )
            .await;
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        mcp_outcome_result(entry.server, entry.raw_name, outcome, Some(duration_ms))
    }

    /// resources 三工具：参数解析后交给共享连接管理器（支持 Esc 取消与超时）。
    async fn execute_mcp_resource(
        &self,
        call: &ReadyToolCall,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> ToolExecutionResult {
        let Some(runtime) = &self.mcp else {
            return crate::chat::tools::ToolFailure::new(
                crate::chat::tools::ToolErrorCode::UnknownTool,
                "mcp resources are not available in this turn",
            )
            .into_result(Vec::new());
        };
        let arguments = match serde_json::from_str::<serde_json::Value>(&call.arguments_json) {
            Ok(value) => value,
            Err(error) => {
                return crate::chat::tools::ToolFailure::invalid(format!(
                    "arguments are not valid JSON: {error}"
                ))
                .into_result(Vec::new());
            }
        };
        let server = arguments.get("server").and_then(serde_json::Value::as_str);
        let cursor = arguments.get("cursor").and_then(serde_json::Value::as_str);
        let started = std::time::Instant::now();
        let outcome = match call.name.as_str() {
            "list_resources" => {
                runtime
                    .manager
                    .list_resources(&runtime.snapshot, server, cursor, cancel)
                    .await
            }
            "list_resource_templates" => {
                runtime
                    .manager
                    .list_resource_templates(&runtime.snapshot, server, cursor, cancel)
                    .await
            }
            _ => {
                let (Some(server), Some(uri)) = (
                    server,
                    arguments.get("uri").and_then(serde_json::Value::as_str),
                ) else {
                    return crate::chat::tools::ToolFailure::invalid(
                        "read_resource needs string `server` and `uri`",
                    )
                    .into_result(Vec::new());
                };
                runtime
                    .manager
                    .read_resource(&runtime.snapshot, &self.session_id, server, uri, cancel)
                    .await
            }
        };
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let label = server.unwrap_or("all servers").to_string();
        mcp_outcome_result(label, call.name.clone(), outcome, Some(duration_ms))
    }

    /// 文件工具：在阻塞线程池执行，并注入本次调用的 recorder。
    async fn execute_file(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
        ticket: &ToolCallTicket,
    ) -> ToolExecutionResult {
        let mut context = crate::chat::tools::ToolContext::new(cwd.to_path_buf())
            .with_image_support(self.supports_image_input)
            .with_session_id(self.session_id.clone());
        if let Some(reads) = self.reads.as_ref() {
            context = context.with_reads(Arc::clone(reads));
        }
        let name = call.name.clone();
        let arguments_json = call.arguments_json.clone();
        let call_id = call.call_id.clone();
        let recorder: Option<Arc<dyn MutationRecorder>> = self
            .recorder
            .as_ref()
            .map(|factory| factory.for_call(&call_id));
        // ticket 只用于归属校验（recorder 已按 call_id 绑定）；这里保留参数，
        // 防止将来误用模型参数代替已提交票据。
        let _ = ticket;
        let worker_name = name.clone();
        match tokio::task::spawn_blocking(move || {
            crate::chat::tools::execute_with(
                &context,
                &worker_name,
                &arguments_json,
                recorder.as_deref(),
            )
        })
        .await
        {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    target: "lx::agent",
                    call_id = %call_id,
                    name = %name,
                    %error,
                    "tool execution task failed"
                );
                ToolExecutionResult::Failed {
                    code: crate::chat::tools::ToolErrorCode::Io,
                    message: crate::chat::agent_loop::TOOL_EXECUTION_ABORTED_CODE.to_string(),
                    changes: Vec::new(),
                }
            }
        }
    }

    /// 会话进程管理器（事件层清理与查询用）。
    pub fn processes(&self) -> &Arc<ProcessManager> {
        &self.processes
    }

    /// 本轮执行上下文（Esc 停止时由事件层取停止目标）。
    pub fn turn(&self) -> &Arc<Mutex<TurnExecution>> {
        &self.turn
    }

    /// 执行进程工具：`exec_command` / `write_stdin`。
    async fn execute_process(
        &self,
        cwd: &Path,
        name: &str,
        arguments_json: &str,
        call_id: &str,
    ) -> ToolExecutionResult {
        if name == "exec_command" {
            let mut args = match parse_exec_args(arguments_json) {
                Ok(args) => args,
                Err(failure) => return failure.into_result(Vec::new()),
            };
            // rtk：外部安装优先；否则用内置过滤。包装只改实际执行的脚本，
            // 结果里的 command 仍还原为用户原始命令（卡片/上下文事实不变）。
            let prepared = self.rtk.prepare(&args.cmd);
            args.cmd = prepared.command.clone();
            // 用 `exec_observed` 提前登记：等待窗口期间也能被本轮停止杀掉。
            let turn = Arc::clone(&self.turn);
            let on_spawn = move |session_id: u64| {
                // 这里是同步回调（在 `exec` 内部、非 await 点），用 `try_lock`
                // 避免阻塞；拿不到锁说明别的路径正持有，退化为「结果返回后登记」。
                if let Ok(mut turn) = turn.try_lock() {
                    turn.observe_spawned(session_id);
                }
            };
            return match self
                .processes
                .exec_observed(args, cwd, call_id, &on_spawn)
                .await
            {
                Ok(mut result) => {
                    result.command = prepared.original.clone();
                    // 只有真实终态才过滤：等待窗口到期仍 Running 时返回的是半截输出，
                    // 不能把“还在编译”压缩成“ok”；后续 write_stdin 继续读原始增量。
                    if result.state.is_terminal() && self.rtk.should_filter_builtin(&prepared) {
                        result.output = self.rtk.filter_output(&prepared.original, &result.output);
                    }
                    ToolExecutionResult::Process(Box::new(result))
                }
                Err(error) => process_failure(error),
            };
        }
        let args = match parse_stdin_args(arguments_json) {
            Ok(args) => args,
            Err(failure) => return failure.into_result(Vec::new()),
        };
        let session_id = args.session_id;
        let result = match self.processes.interact(args).await {
            Ok(result) => ToolExecutionResult::Process(Box::new(result)),
            Err(error) => process_failure(error),
        };
        // 交互失败（进程不存在/已过期）不改变本轮活动集合，但也清除指向它的活动项。
        if result.is_failure() && self.processes.snapshot(session_id).is_none() {
            let mut turn = self.turn.lock().await;
            if turn.active_interaction == Some(session_id) {
                turn.active_interaction = None;
            }
        }
        result
    }
}

impl ToolExecutor for AgentTools {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        self.run(cwd, call, None).await
    }

    async fn execute_tracked(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
        ticket: &ToolCallTicket,
    ) -> ToolExecutionResult {
        self.run(cwd, call, Some(ticket)).await
    }

    async fn execute_mcp_cancellable(
        &self,
        _cwd: &Path,
        call: &ReadyToolCall,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Option<ToolExecutionResult> {
        let result = if crate::mcp::is_mcp_tool(&call.name) {
            self.execute_mcp(call, cancel).await
        } else if matches!(
            call.name.as_str(),
            "list_resources" | "list_resource_templates" | "read_resource"
        ) {
            self.execute_mcp_resource(call, cancel).await
        } else if call.name == crate::chat::tools::WEB_SEARCH_TOOL {
            crate::chat::tools::web_search::execute(
                &call.arguments_json,
                &self.exa_api_key,
                &self.tavily_api_key,
                cancel,
            )
            .await
        } else {
            return None;
        };
        if !call.name.is_empty() {
            let mut turn = self.turn.lock().await;
            turn.observe(&call.name, &result);
        }
        Some(result)
    }

    fn supports_image_input(&self) -> bool {
        self.supports_image_input
    }
}

impl AgentTools {
    async fn run(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
        ticket: Option<&ToolCallTicket>,
    ) -> ToolExecutionResult {
        let result = match call.name.as_str() {
            "exec_command" | "write_stdin" => {
                self.execute_process(cwd, &call.name, &call.arguments_json, &call.call_id)
                    .await
            }
            name if crate::mcp::is_mcp_tool(name) => {
                // 无取消信号的测试入口：通道永不取消，由 manager 超时兜底。
                let (_tx, cancel) = tokio::sync::watch::channel(false);
                self.execute_mcp(call, cancel).await
            }
            "list_resources" | "list_resource_templates" | "read_resource" => {
                let (_tx, cancel) = tokio::sync::watch::channel(false);
                self.execute_mcp_resource(call, cancel).await
            }
            name if name == crate::chat::tools::WEB_SEARCH_TOOL => {
                // 无取消信号的测试入口：通道永不取消，由单次引擎超时兜底。
                let (_tx, cancel) = tokio::sync::watch::channel(false);
                crate::chat::tools::web_search::execute(
                    &call.arguments_json,
                    &self.exa_api_key,
                    &self.tavily_api_key,
                    cancel,
                )
                .await
            }
            "update_plan" => {
                // 修订号先取号后校验：校验失败不消耗可观察的提交（顶层不落库）。
                let next = self
                    .plan_revision
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    .saturating_add(1);
                match build_plan_result(&call.arguments_json, next) {
                    Ok(snapshot) => ToolExecutionResult::Plan(snapshot),
                    Err(failure) => failure.into_result(Vec::new()),
                }
            }
            _ => match ticket {
                Some(ticket) => self.execute_file(cwd, call, ticket).await,
                None => crate::chat::agent_loop::FileTools.execute(cwd, call).await,
            },
        };
        if !call.name.is_empty() {
            let mut turn = self.turn.lock().await;
            turn.observe(&call.name, &result);
        }
        result
    }
}

/// MCP 结果 → 执行层结果。
fn mcp_outcome_result(
    server: String,
    tool: String,
    outcome: crate::mcp::McpCallOutcome,
    duration_ms: Option<u64>,
) -> ToolExecutionResult {
    use crate::chat::tools::{ArchivedImage, McpToolResult, ToolErrorCode, ToolFailure};
    use crate::mcp::McpFailureKind;
    match outcome {
        crate::mcp::McpCallOutcome::Success {
            text,
            images,
            images_omitted,
            truncated,
        } => ToolExecutionResult::Mcp(Box::new(McpToolResult {
            server,
            tool,
            text,
            images: images
                .into_iter()
                .map(|image| ArchivedImage {
                    path: image.path,
                    mime: image.mime,
                    width: image.width,
                    height: image.height,
                })
                .collect(),
            images_omitted,
            duration_ms,
            truncated,
        })),
        crate::mcp::McpCallOutcome::Failure { kind, text } => {
            let code = match kind {
                McpFailureKind::Timeout => ToolErrorCode::Timeout,
                McpFailureKind::InvalidArguments => ToolErrorCode::InvalidArguments,
                McpFailureKind::ToolMissing => ToolErrorCode::UnknownTool,
                McpFailureKind::AuthRequired
                | McpFailureKind::ToolError
                | McpFailureKind::Server => ToolErrorCode::McpError,
            };
            ToolFailure::new(code, text).into_result(Vec::new())
        }
        crate::mcp::McpCallOutcome::Cancelled => {
            ToolFailure::new(ToolErrorCode::Cancelled, "mcp call cancelled").into_result(Vec::new())
        }
    }
}

/// 计划快照的修订号（测试与事件层断言用）。
pub fn plan_snapshot_of(result: &ToolExecutionResult) -> Option<&PlanSnapshot> {
    match result {
        ToolExecutionResult::Plan(snapshot) => Some(snapshot),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
