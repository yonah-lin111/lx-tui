//! 同一响应内相邻只读工具调用的并行批次执行与统一结算。
//!
//! 边界：
//! - 只负责“已按 ordinal 排好的一段只读调用”；分类与分批由 [`super::run_agent_turn`] 决定；
//! - Begin 必须逐个串行完成（事件层按 `MAX(position)+1` 分配工具位置）；
//! - 执行可并发，但 Finish、DB 终态与模型上下文严格按批次原顺序提交；
//! - 取消语义与串行路径一致：已开始执行的调用跑完并记录真实结果；未执行的票据记
//!   `cancelled`，绝不谎报成功。

use std::path::Path;

use futures::future::{BoxFuture, join_all};
use tokio::sync::watch;

use super::context::ModelContext;
use super::{AgentHost, HostError, ReadyToolCall, ToolExecutor, ToolOutcome};
use crate::chat::agent::UNAVAILABLE_CODE;
use crate::chat::tools::{ToolErrorCode, ToolExecutionResult, ToolFailure, format_for_model};

/// 单批最多并行的只读调用数。
pub(super) const MAX_PARALLEL_TOOL_CALLS: usize = 4;

/// 批次执行结局：由调用方映射成 `TurnSummary`。
pub(super) enum BatchOutcome {
    /// 全部调用已执行并按 ordinal 结算。
    Completed,
    /// 用户取消：已开始的调用已如实落库，未开始的票据已按 `cancelled` 收尾。
    Interrupted,
    /// 持久化/状态机失败：本轮必须停止且不重试。
    Failed(String),
}

/// 执行一批相邻只读调用；`calls` 里的 `allowed` 已由调用方按冻结工具面判定。
pub(super) async fn execute_batch<H, E>(
    host: &mut H,
    context: &mut ModelContext,
    executor: &E,
    calls: Vec<(ReadyToolCall, bool)>,
    cancel: &watch::Receiver<bool>,
) -> BatchOutcome
where
    H: AgentHost,
    E: ToolExecutor,
{
    let cwd = context.cwd().to_path_buf();
    let supports_image_input = executor.supports_image_input();

    // 1) 全部先 begin：任何一个失败都要把已 begin 的票据结算掉，不能留下 pending 行。
    //    Begin 不能并发：事件层按消息内 `MAX(position)+1` 分配工具位置。
    let mut begun = Vec::with_capacity(calls.len());
    for (call, allowed) in calls {
        match host.begin_tool(call.clone()).await {
            Ok(ticket) => begun.push((call, ticket, allowed)),
            Err(error) => {
                cancel_tickets(host, begun.into_iter().map(|(_, ticket, _)| ticket)).await;
                return outcome_of(error);
            }
        }
    }
    let batch_len = begun.len();
    if *cancel.borrow() {
        // 已入库 pending 但尚未执行：全部按 cancelled 结算，零 IO。
        cancel_tickets(host, begun.into_iter().map(|(_, ticket, _)| ticket)).await;
        return BatchOutcome::Interrupted;
    }

    // 2) 并发执行；不允许的调用用 ready future 直接产出失败结果，绝不进 executor。
    //    仍先走可取消入口：`web_search` 的 Esc 取消必须直达 HTTP 请求。
    let futures: Vec<BoxFuture<'_, ToolExecutionResult>> = begun
        .iter()
        .map(|(call, ticket, allowed)| {
            let future: BoxFuture<'_, ToolExecutionResult> = if *allowed {
                Box::pin(run_one(executor, &cwd, call, ticket, cancel.clone()))
            } else {
                Box::pin(std::future::ready(unavailable_result()))
            };
            future
        })
        .collect();
    let results = join_all(futures).await;

    // 3) 按 ordinal 顺序结算。
    //
    // `join_all` 返回时所有 future 都已结束：某个调用报告 cancelled（目前只有
    // `web_search` 会在 Esc 时这样做）不代表同批其它调用没执行。这里只把该票据记为
    // cancelled，其余结果仍按真实结果落库；全部结算后返回 Interrupted，让调用方停止
    // 本轮采样并退掉批次之外的调用。绝不能把已经完成的读盘结果谎报成 cancelled。
    let mut interrupted = false;
    let mut iter = begun.into_iter().zip(results);
    while let Some(((call, ticket, _), result)) = iter.next() {
        if result.error_code() == Some(ToolErrorCode::Cancelled) {
            let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
            interrupted = true;
            continue;
        }
        if let Err(error) =
            commit_result(host, context, &call, ticket, result, supports_image_input).await
        {
            // 失败票据按契约不重试；其余结果已经产生，尽力按真实结果落库，
            // 避免把同批“已执行”的调用误记成 cancelled。
            settle_remaining(host, context, iter, supports_image_input).await;
            return outcome_of(error);
        }
    }
    if interrupted {
        return BatchOutcome::Interrupted;
    }
    tracing::debug!(
        target: "lx::agent",
        batch = batch_len,
        "parallel read-only tool batch settled"
    );
    BatchOutcome::Completed
}

/// 提交失败后尽力结算剩余结果：失败的票据不重试，但其余真实结果必须落库。
///
/// 某个结果落库失败通常是该行自身的问题（例如终态冲突）；把它之后的已执行结果
/// 全部记成 cancelled 会制造假状态。逐条提交，失败只记 warn，最终仍由调用方按
/// 原始错误结束本轮。
async fn settle_remaining<H: AgentHost>(
    host: &mut H,
    context: &mut ModelContext,
    iter: impl Iterator<
        Item = (
            (ReadyToolCall, crate::db::chat::ToolCallTicket, bool),
            ToolExecutionResult,
        ),
    >,
    supports_image_input: bool,
) {
    for ((call, ticket, _), result) in iter {
        if result.error_code() == Some(ToolErrorCode::Cancelled) {
            let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
            continue;
        }
        if let Err(error) =
            commit_result(host, context, &call, ticket, result, supports_image_input).await
        {
            tracing::warn!(
                target: "lx::agent",
                call_id = %call.call_id,
                error = ?error,
                "failed to settle remaining tool result"
            );
        }
    }
}

/// 可取消入口优先（`web_search` / MCP），其余工具回落到带票据的通用执行入口。
async fn run_one<E: ToolExecutor>(
    executor: &E,
    cwd: &Path,
    call: &ReadyToolCall,
    ticket: &crate::db::chat::ToolCallTicket,
    cancel: watch::Receiver<bool>,
) -> ToolExecutionResult {
    match executor.execute_mcp_cancellable(cwd, call, cancel).await {
        Some(result) => result,
        None => executor.execute_tracked(cwd, call, ticket).await,
    }
}

/// 把一次执行结果按固定口径提交：格式化 → 落库终态 → 进入模型上下文。
///
/// 提交失败直接返回 [`HostError`]，调用方必须停止本轮且不重试。
pub(super) async fn commit_result<H: AgentHost>(
    host: &mut H,
    context: &mut ModelContext,
    call: &ReadyToolCall,
    ticket: crate::db::chat::ToolCallTicket,
    result: ToolExecutionResult,
    supports_image_input: bool,
) -> Result<(), HostError> {
    let mut content = format_for_model(&result);
    let is_error = result.is_failure();
    let mut images = tool_result_images(&result);
    if !supports_image_input && !images.is_empty() {
        images.clear();
        content.push_str("\n[image omitted: this model does not support image input]");
    }
    // 结果必须先落库再进入上下文；落库失败即停止本轮，**不重试写入**。
    host.finish_tool(ticket, ToolOutcome::Executed(result))
        .await?;
    context.push_tool_result(&call.call_id, content, is_error, images);
    Ok(())
}

/// 把 host 错误映射成批次结局：只有 `Cancelled` 代表用户中断。
fn outcome_of(error: HostError) -> BatchOutcome {
    match error {
        HostError::Cancelled => BatchOutcome::Interrupted,
        HostError::Failed { code } => BatchOutcome::Failed(code),
    }
}

/// 把未执行的票据按 cancelled 结算；错误只记 debug，不重试。
async fn cancel_tickets<H: AgentHost>(
    host: &mut H,
    tickets: impl Iterator<Item = crate::db::chat::ToolCallTicket>,
) {
    for ticket in tickets {
        if host
            .finish_tool(ticket, ToolOutcome::Cancelled)
            .await
            .is_err()
        {
            tracing::debug!(target: "lx::agent", "failed to settle cancelled tool ticket");
        }
    }
}

/// 工具面不允许的调用：与串行路径同一条失败结果。
fn unavailable_result() -> ToolExecutionResult {
    ToolFailure::new(ToolErrorCode::Io, UNAVAILABLE_CODE).into_result(Vec::new())
}

/// 取工具结果的图片字节（归档路径缺失或读失败时跳过，不阻断工具结果）。
fn tool_result_images(result: &ToolExecutionResult) -> Vec<crate::chat::events::ModelImage> {
    let archived: Vec<&crate::chat::tools::ArchivedImage> = match result {
        ToolExecutionResult::Mcp(mcp) => mcp.images.iter().collect(),
        ToolExecutionResult::Read {
            image: Some(image), ..
        } => vec![image],
        _ => Vec::new(),
    };
    archived
        .into_iter()
        .filter_map(|image| {
            let bytes = std::fs::read(&image.path).ok()?;
            Some(crate::chat::events::ModelImage {
                mime: image.mime.clone(),
                bytes: std::sync::Arc::new(bytes),
            })
        })
        .collect()
}
