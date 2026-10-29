//! 每轮冻结 MCP 快照：读配置、合并作用域、后台预热、取最近一次缓存。
//!
//! 首轮若 server 尚未就绪，本轮不带 MCP 工具；后台启动完成后下一轮即可用。
//! 快照本身在 turn 起点读取，同一 turn 的多次采样复用同一份。

use std::path::Path;
use std::sync::Arc;

use crate::config::mcp::{effective_servers, load_mcp, project_mcp_path, user_mcp_path};
use crate::db::service::Persistence;
use crate::mcp::{McpToolSnapshot, global_manager};

/// 取本轮冻结的 MCP 快照。
pub(crate) fn snapshot_for_turn(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
) -> Arc<McpToolSnapshot> {
    let Some(root) = workspace_root else {
        return Arc::new(McpToolSnapshot::default());
    };
    let user = user_mcp_path()
        .ok()
        .and_then(|path| load_mcp(&path).ok().flatten());
    let project = project_mcp_path(root)
        .ok()
        .and_then(|path| load_mcp(&path).ok().flatten());
    let trusted = persistence.is_trusted(root);
    let servers = effective_servers(user.as_ref(), project.as_ref(), trusted, Some(root));
    let manager = global_manager();
    manager.prewarm(root.to_path_buf(), servers);
    manager.cached_snapshot(root)
}
