//! `exec` 运行器测试夹具：脚本化模型流、临时工作目录与真实装配路径。
//!
//! 不触网：模型流由 [`ScriptedFactory`] 按请求用途（主轮 / 子 agent 轮）回放脚本；
//! 其余全部走生产路径——`assemble_exec_turn_with_snapshot` 装配、`run_exec_turn`
//! 事件循环、`AgentTools` 真实文件 IO 与 SQLite 落库。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use super::super::cli::ExecModelTarget;
use super::super::runner::{BoxStreamFuture, ExecStreamFactory};
use super::super::{ExecTurnConfig, assemble_exec_turn_with_snapshot};
use crate::chat::agent_loop::ModelContext;
use crate::chat::client::StreamOutcome;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, ModelItem, ModelStepStop, PartDelta, PartKind,
    RequestPurpose, StopReason,
};
use crate::db::Database;

/// 每个测试独立的临时目录（仓库未引入 `tempfile` 依赖）。
pub(super) struct TestDir {
    pub(super) path: PathBuf,
}

impl TestDir {
    pub(super) fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "lx-exec-runner-{}-{label}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub(super) fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub(super) fn model_target() -> ExecModelTarget {
    ExecModelTarget {
        provider: "test-provider".to_string(),
        model: "test-model".to_string(),
        variant: None,
        display_name: "Test Model".to_string(),
        transport: "openai".to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        api_key: "test-key".to_string(),
        context_limit: Some(200_000),
        output_limit: Some(8_192),
        variant_params: None,
        idle_timeout_ms: 1_000,
        supports_image_input: false,
    }
}

/// 接受图片输入的模型（图片水合回归用）。
pub(super) fn image_model_target() -> ExecModelTarget {
    ExecModelTarget {
        supports_image_input: true,
        ..model_target()
    }
}

/// 写一张合法 PNG（图片处理层会真实解码，不能用假字节）。
pub(super) fn write_test_png(path: &Path) {
    let image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_pixel(2, 2, image::Rgba([12, 34, 56, 255]));
    image.save(path).expect("write test png");
}

/// 一次脚本化模型响应。
pub(super) enum Step {
    Response {
        deltas: Vec<ChatDelta>,
        stop: StopReason,
    },
    /// 只等取消（`Ctrl+C` 中断测试）。
    Hang,
    /// 从上一条 spawn 结果里取出 agent id，生成 `send_input(interrupt = true)`。
    SendInputInterrupt { message: String },
    /// 先挂起 `millis` 毫秒再回放内层步骤（`wait_agent` 自动续等回归用）。
    Delay { millis: u64, then: Box<Step> },
}

/// 延迟 `millis` 毫秒后再回放内层步骤。
pub(super) fn delayed(millis: u64, then: Step) -> Step {
    Step::Delay {
        millis,
        then: Box::new(then),
    }
}

pub(super) fn text_step(text: &str) -> Step {
    Step::Response {
        deltas: vec![
            ChatDelta::Part(PartDelta {
                kind: PartKind::Text,
                text: text.to_string(),
                metadata: None,
            }),
            ChatDelta::ResponseFinished {
                stop: ModelStepStop::Final,
            },
            ChatDelta::Finished {
                reason: StopReason::Complete,
            },
        ],
        stop: StopReason::Complete,
    }
}

/// 一次要求执行工具的响应；`calls` 为 `(call_id, 工具名, 参数 JSON)`。
pub(super) fn tool_step(calls: &[(&str, &str, &str)]) -> Step {
    tool_step_with_text("", calls)
}

/// 工具调用前先输出一段正文（验证「工具前说明」不会被当作最终回复）。
pub(super) fn tool_step_with_text(text: &str, calls: &[(&str, &str, &str)]) -> Step {
    let mut deltas = Vec::new();
    if !text.is_empty() {
        deltas.push(ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: text.to_string(),
            metadata: None,
        }));
    }
    for (ordinal, (call_id, name, arguments)) in calls.iter().enumerate() {
        deltas.push(ChatDelta::ToolInputStarted {
            call_id: (*call_id).to_string(),
            name: (*name).to_string(),
            ordinal,
        });
        deltas.push(ChatDelta::ToolInputDelta {
            call_id: (*call_id).to_string(),
            chunk: (*arguments).to_string(),
        });
        deltas.push(ChatDelta::ToolInputEnded {
            call_id: (*call_id).to_string(),
            arguments_json: (*arguments).to_string(),
        });
    }
    deltas.push(ChatDelta::ResponseFinished {
        stop: ModelStepStop::ToolCalls,
    });
    deltas.push(ChatDelta::Finished {
        reason: StopReason::Complete,
    });
    Step::Response {
        deltas,
        stop: StopReason::Complete,
    }
}

/// 从请求历史里取最近一次 spawn 结果中的 agent id，生成打断输入。
pub(super) fn send_input_interrupt_step(request: &ChatRequest, message: &str) -> Step {
    let agent_id = request.items.iter().rev().find_map(|item| match item {
        ModelItem::ToolResult { content, .. } => {
            let (_, rest) = content.split_once("Spawned subagent `")?;
            Some(rest.split_once('`')?.0.to_string())
        }
        _ => None,
    });
    let Some(agent_id) = agent_id else {
        return Step::Response {
            deltas: Vec::new(),
            stop: StopReason::Failed {
                code: "missing spawn result".to_string(),
            },
        };
    };
    let arguments = serde_json::json!({
        "target": agent_id,
        "message": message,
        "interrupt": true,
    })
    .to_string();
    tool_step(&[("call-input", "send_input", arguments.as_str())])
}

/// 脚本化模型流：主轮与子 agent 轮各自一条队列，顺带记录全部请求快照。
#[derive(Default)]
pub(super) struct ScriptedFactory {
    main: Mutex<VecDeque<Step>>,
    subagent: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedFactory {
    pub(super) fn new(main: Vec<Step>, subagent: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            main: Mutex::new(main.into()),
            subagent: Mutex::new(subagent.into()),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }

    pub(super) fn main_requests(&self) -> Vec<ChatRequest> {
        self.requests()
            .into_iter()
            .filter(|request| !matches!(request.purpose, RequestPurpose::Subagent(_)))
            .collect()
    }

    fn take(&self, request: &ChatRequest) -> Step {
        let queue = match request.purpose {
            RequestPurpose::Subagent(_) => &self.subagent,
            _ => &self.main,
        };
        let step = queue.lock().ok().and_then(|mut queue| queue.pop_front());
        match step {
            Some(Step::SendInputInterrupt { message }) => {
                send_input_interrupt_step(request, &message)
            }
            Some(step) => step,
            None => Step::Response {
                deltas: Vec::new(),
                stop: StopReason::Failed {
                    code: "script exhausted".to_string(),
                },
            },
        }
    }
}

impl ExecStreamFactory for ScriptedFactory {
    fn stream(
        &self,
        _model: &ExecModelTarget,
        request: ChatRequest,
        cancel: watch::Receiver<bool>,
        mut on_delta: Box<dyn FnMut(&[ChatDelta]) + Send>,
    ) -> BoxStreamFuture {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request.clone());
        }
        let mut step = self.take(&request);
        Box::pin(async move {
            let step = loop {
                match step {
                    Step::Delay { millis, then } => {
                        tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
                        step = *then;
                    }
                    other => break other,
                }
            };
            match step {
                Step::Response { deltas, stop } => {
                    on_delta(&deltas);
                    StreamOutcome { deltas, stop }
                }
                Step::Hang => hang_until_cancelled(cancel).await,
                // `take` 已把动态步骤解析成具体响应；走到这里说明脚本状态异常。
                Step::SendInputInterrupt { .. } | Step::Delay { .. } => StreamOutcome {
                    deltas: Vec::new(),
                    stop: StopReason::Failed {
                        code: "unresolved dynamic step".to_string(),
                    },
                },
            }
        })
    }
}

async fn hang_until_cancelled(mut cancel: watch::Receiver<bool>) -> StreamOutcome {
    while !*cancel.borrow() {
        if cancel.changed().await.is_err() {
            // 发送端已释放：不会再有取消，按失败收尾而不是无限等待。
            return StreamOutcome {
                deltas: Vec::new(),
                stop: StopReason::Failed {
                    code: "script hang released".to_string(),
                },
            };
        }
    }
    StreamOutcome {
        deltas: Vec::new(),
        stop: StopReason::Cancelled,
    }
}

/// 走生产装配路径（MCP 快照注入空实现，测试不启动任何 server）。
pub(super) fn assemble(
    db: &Database,
    dir: &TestDir,
    prompt: &str,
    existing_session: Option<&str>,
) -> ExecTurnConfig {
    assemble_images(db, dir, prompt, existing_session, &[], &model_target())
}

/// 同 [`assemble`]，但可注入 `-i` 附件与模型（图片水合回归用）。
pub(super) fn assemble_images(
    db: &Database,
    dir: &TestDir,
    prompt: &str,
    existing_session: Option<&str>,
    images: &[PathBuf],
    model: &ExecModelTarget,
) -> ExecTurnConfig {
    assemble_exec_turn_with_snapshot(
        db,
        &dir.path,
        &dir.path,
        existing_session,
        prompt,
        images,
        model,
        Arc::new(crate::mcp::McpToolSnapshot::default()),
    )
    .expect("assemble exec turn")
}

/// 手工装配一轮；`purpose` 与上下文上限可覆盖（自动压缩回归用）。
pub(super) fn manual_turn_with(
    db: &Database,
    dir: &TestDir,
    prompt: &str,
    purpose: RequestPurpose,
    context_limit: Option<u64>,
) -> ExecTurnConfig {
    let model = model_target();
    let committed = db
        .begin_turn(&dir.path, None, prompt, &model.snapshot())
        .expect("begin turn");
    let request = ChatRequest {
        turns: vec![ChatTurn {
            role: "user".into(),
            text: prompt.to_string(),
            reasoning_metadata: None,
        }],
        items: vec![ModelItem::UserText(prompt.to_string())],
        model: model.snapshot(),
        variant_params: None,
        purpose,
        system_prompt: Some("test system prompt".to_string()),
        tools: Arc::new(crate::chat::tools::builtin_tools(
            crate::chat::agent::ToolSurface::Main,
        )),
    };
    ExecTurnConfig {
        workspace: dir.path.clone(),
        cwd: dir.path.clone(),
        session_id: committed.session_id,
        user_message_id: committed.user_message_id,
        assistant_id: committed.assistant_message_id,
        prompt_display: prompt.to_string(),
        model,
        context: ModelContext::new(request, dir.path.clone()).with_context_limit(context_limit),
        exa_api_key: String::new(),
        tavily_api_key: String::new(),
        mcp_snapshot: Arc::new(crate::mcp::McpToolSnapshot::default()),
    }
}

/// 手工装配一轮（工具面可覆盖）：用于验证 exec 宿主自己的 `ask` 兜底。
pub(super) fn manual_turn(db: &Database, dir: &TestDir, prompt: &str) -> ExecTurnConfig {
    manual_turn_with(db, dir, prompt, RequestPurpose::ExecTurn, None)
}

pub(super) fn session_messages(
    db: &Database,
    workspace: &Path,
    session_id: &str,
) -> Vec<crate::db::chat::ChatMessage> {
    db.load_chat_session(workspace, session_id)
        .expect("load session")
        .expect("session exists")
}

pub(super) fn tool_result_texts(request: &ChatRequest) -> Vec<String> {
    request
        .items
        .iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}
