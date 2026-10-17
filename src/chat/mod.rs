//! 流式聊天模块：协议适配、SSE 帧解析、thinking 标签拆分与上下文装配。
//!
//! 公共网络入口只接收 `ChatRequest + cancellation`，输出纯数据事件；
//! 网络任务在独立 Tokio 任务运行，结果经 channel 回流事件循环。
//! 模块不依赖 `app`/`ui`；DB 依赖仅限记录类型转换。

pub mod client;
pub mod context;
pub mod events;
pub mod sse;
pub mod thinking;
pub mod transport;

pub use context::{Assembly, AssemblyInput, assemble, shrink_once};
pub use events::{ChatDelta, ChatRequest, ChatTurn, PartDelta, PartKind, StopReason};
