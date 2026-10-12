//! 设置与用户级持久化：`settings.json` 与自定义命令文件。
//!
//! 本模块是唯一的设置/命令文件读写落点；`ui/` 不执行 IO，TUI 状态变更
//! 一律经事件消息进入，浏览器设置页与 TUI 共享同一套存储函数。

pub mod commands;
pub mod service;

pub use commands::{
    CommandKind, CommandParseError, CommandRecord, CommandScope, CommandsError, MdScope,
};
