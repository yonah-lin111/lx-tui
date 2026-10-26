//! 流式聊天模块：协议适配、SSE 帧解析、thinking 标签拆分与上下文装配。
//!
//! 公共网络入口只接收 `ChatRequest + cancellation`，输出纯数据事件；
//! 网络任务在独立 Tokio 任务运行，结果经 channel 回流事件循环。
//! 模块不依赖 `app`/`ui`；DB 依赖仅限记录类型转换。

pub mod agent;
pub mod agent_loop;
pub mod client;
pub mod compact;
pub mod context;
pub mod events;
pub mod plan;
pub mod process;
pub mod prompt;
pub mod sse;
pub mod thinking;
pub mod tools;
pub mod transport;

pub use context::{
    Assembly, AssemblyInput, MAX_TOOL_RESULT_REPLAY_BYTES, assemble, shrink_once,
    truncate_result_for_replay,
};
pub use events::{
    ChatDelta, ChatRequest, ChatTurn, MAX_SUMMARY_CHARS, MAX_TOOL_ARGUMENTS_BYTES, ModelItem,
    ModelStepStop, PartDelta, PartKind, StopReason,
};
