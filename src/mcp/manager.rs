//! 全局 MCP 连接管理器：按项目根 + server 名缓存，懒启动，失败不自动重试。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientRequest, PaginatedRequestParams,
    ReadResourceRequestParams, ResourceContents, ServerResult,
};
use rmcp::service::{
    NotificationContext, PeerRequestOptions, RunningService, RunningServiceCancellationToken,
};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use tokio::sync::{Mutex, watch};

use crate::config::mcp::{EffectiveServer, McpHttpAuth, McpServerConfig, McpTransportConfig};
use crate::platform::path_text::bytes_hash;

use super::naming::{McpToolEntry, qualified_name};

mod connect;
use super::result::{McpCallOutcome, McpFailureKind, convert_result};
use super::schema::{SCHEMA_OMITTED_NOTE, sanitize_json_schema};
use connect::{StartError, build_entries, start_connection};

mod snapshot;
pub use snapshot::{McpToolSnapshot, ServerSnapshot};

mod status;
pub use status::{McpEvent, ServerStatus};

struct ServerSlot {
    fingerprint: u64,
    status: ServerStatus,
    connection: Option<Arc<Connection>>,
    /// 已有一个启动任务在途：防止同一 server 被每轮预热重复 spawn。
    starting: bool,
}

struct Connection {
    client: RunningService<RoleClient, LxClientHandler>,
    cancel: StdMutex<Option<RunningServiceCancellationToken>>,
    /// `tools/list_changed` 后原地替换；快照读取时克隆，已冻结的旧快照不受影响。
    tools: StdMutex<Vec<McpToolEntry>>,
    instructions: Option<String>,
    has_resources: bool,
    tools_changed: Arc<AtomicBool>,
    config: McpServerConfig,
    /// stdio 子进程 pid（进程组组长）；HTTP 为 None。
    pid: Option<u32>,
    /// OAuth 凭据键；非 OAuth 为 None。
    credential_key: Option<String>,
}

impl Connection {
    fn stop(&self) {
        if let Ok(mut cancel) = self.cancel.lock()
            && let Some(cancel) = cancel.take()
        {
            cancel.cancel();
        }
    }

    /// 强制终止 stdio 进程组（优雅关闭超时后的兜底）。
    fn kill_group(&self) {
        if let Some(pid) = self.pid {
            crate::platform::terminate_process_group(pid, true);
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct LxClientHandler {
    tools_changed: Arc<AtomicBool>,
}

impl ClientHandler for LxClientHandler {
    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.tools_changed.store(true, Ordering::SeqCst);
    }
}

/// 全局 MCP 连接管理器。
///
/// 字段全部是共享句柄，`Clone` 只复制指针：后台任务（取消后仍在启动的 server）
/// 需要持有管理器继续结算，而不是丢弃任务。
#[derive(Clone)]
pub struct McpManager {
    slots: Arc<Mutex<HashMap<(PathBuf, String), ServerSlot>>>,
    snapshot_cache: Arc<StdMutex<HashMap<PathBuf, Arc<McpToolSnapshot>>>>,
    events: Arc<StdMutex<Option<tokio::sync::mpsc::UnboundedSender<McpEvent>>>>,
    credentials_revision: Arc<AtomicU64>,
    /// 当前启动批次的总数（进度事件按项目记账）。
    startup_totals: Arc<StdMutex<HashMap<PathBuf, usize>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            slots: Arc::new(Mutex::new(HashMap::new())),
            snapshot_cache: Arc::new(StdMutex::new(HashMap::new())),
            events: Arc::new(StdMutex::new(None)),
            credentials_revision: Arc::new(AtomicU64::new(0)),
            startup_totals: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// 设置状态事件通道（事件层启动时注入）。
    pub fn set_events(&self, sender: tokio::sync::mpsc::UnboundedSender<McpEvent>) {
        if let Ok(mut slot) = self.events.lock() {
            *slot = Some(sender);
        }
    }

    /// OAuth 凭据变化：递增 revision，下一次指纹比较触发重连。
    pub fn bump_credentials(&self) {
        self.credentials_revision.fetch_add(1, Ordering::SeqCst);
    }

    /// 最近一次成功构建的冻结快照（同步读取，供 turn 起点使用）。
    pub fn cached_snapshot(&self, project_root: &Path) -> Arc<McpToolSnapshot> {
        if let Ok(cache) = self.snapshot_cache.lock()
            && let Some(snapshot) = cache.get(project_root)
        {
            return snapshot.clone();
        }
        Arc::new(McpToolSnapshot {
            project: Some(project_root.to_path_buf()),
            ..McpToolSnapshot::default()
        })
    }

    /// 后台预热：启动缺失的 server 并更新缓存。每个 server 受启动超时约束。
    pub fn prewarm(self: &Arc<Self>, project_root: PathBuf, servers: Vec<EffectiveServer>) {
        // 同步测试/非运行时调用：不启动后台任务，本轮按空快照继续。
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let manager = self.clone();
        tokio::spawn(async move {
            let (_tx, cancel) = watch::channel(false);
            let snapshot = manager.snapshot_for(&project_root, &servers, cancel).await;
            manager.store_snapshot(snapshot);
        });
    }

    /// 配置变化：比较指纹，只重启变化 / 新增 / 删除的 server，并更新缓存。
    pub async fn reconcile(&self, project_root: &Path, servers: &[EffectiveServer]) {
        let (_tx, cancel) = watch::channel(false);
        let snapshot = self.snapshot_for(project_root, servers, cancel).await;
        self.store_snapshot(snapshot);
    }

    fn store_snapshot(&self, snapshot: McpToolSnapshot) {
        let Some(project) = snapshot.project.clone() else {
            return;
        };
        if let Ok(mut cache) = self.snapshot_cache.lock() {
            cache.insert(project, Arc::new(snapshot));
        }
    }

    /// 确保所有生效 server 已启动，返回冻结快照。
    pub async fn snapshot_for(
        &self,
        project_root: &Path,
        servers: &[EffectiveServer],
        mut cancel: watch::Receiver<bool>,
    ) -> McpToolSnapshot {
        let mut starts: Vec<(EffectiveServer, u64)> = Vec::new();
        let mut starting: Vec<String> = Vec::new();
        let mut total = 0usize;
        {
            let mut slots = self.slots.lock().await;
            for server in servers {
                if !server.config.enabled {
                    continue;
                }
                total = total.saturating_add(1);
                let fingerprint = self.fingerprint(server);
                let key = (project_root.to_path_buf(), server.name.clone());
                let existed = slots.contains_key(&key);
                let entry = slots.entry(key).or_insert_with(|| ServerSlot {
                    fingerprint,
                    status: ServerStatus::Starting,
                    connection: None,
                    starting: false,
                });
                if !existed {
                    starting.push(server.name.clone());
                }
                if entry.fingerprint != fingerprint {
                    if let Some(connection) = entry.connection.take() {
                        connection.stop();
                    }
                    entry.fingerprint = fingerprint;
                    entry.status = ServerStatus::Starting;
                    entry.starting = false;
                    starting.push(server.name.clone());
                }
                if matches!(entry.status, ServerStatus::Starting)
                    && entry.connection.is_none()
                    && !entry.starting
                {
                    entry.starting = true;
                    starts.push((server.clone(), fingerprint));
                }
            }
            // 已移除的 server：停止并释放连接。
            let active: HashSet<(PathBuf, String)> = servers
                .iter()
                .filter(|server| server.config.enabled)
                .map(|server| (project_root.to_path_buf(), server.name.clone()))
                .collect();
            let removed: Vec<(PathBuf, String)> = slots
                .keys()
                .filter(|key| key.0 == project_root && !active.contains(*key))
                .cloned()
                .collect();
            for key in removed {
                if let Some(slot) = slots.remove(&key)
                    && let Some(connection) = slot.connection
                {
                    connection.stop();
                }
            }
        }
        for name in starting {
            self.emit(McpEvent::Status {
                project: project_root.to_path_buf(),
                server: name,
                status: ServerStatus::Starting,
            });
        }

        if !starts.is_empty() {
            if let Ok(mut totals) = self.startup_totals.lock() {
                totals.insert(project_root.to_path_buf(), total);
            }
            self.emit_progress(project_root).await;
            let mut tasks = tokio::task::JoinSet::new();
            for (server, fingerprint) in starts {
                let project = project_root.to_path_buf();
                tasks.spawn(async move {
                    let name = server.name.clone();
                    let timeout = Duration::from_secs(server.config.startup_timeout_sec());
                    let result =
                        tokio::time::timeout(timeout, start_connection(&server, &project)).await;
                    let outcome = match result {
                        Ok(Ok(connection)) => Ok(connection),
                        Ok(Err(error)) => Err(error),
                        Err(_) => Err(StartError::Failed(format!(
                            "MCP server \"{name}\" startup timed out after {}s",
                            server.config.startup_timeout_sec()
                        ))),
                    };
                    (name, fingerprint, outcome)
                });
            }
            loop {
                tokio::select! {
                    joined = tasks.join_next() => {
                        let Some(joined) = joined else { break };
                        if let Ok((name, fingerprint, outcome)) = joined {
                            self.finish_start(project_root, &name, fingerprint, outcome).await;
                        }
                    }
                    _ = cancel.changed() => {
                        // 用户取消：已就绪的立即返回，未完成的继续在后台启动。
                        break;
                    }
                }
            }
            // 剩余任务转入后台，避免取消时丢弃已经发出的 initialize。
            let manager = self.clone();
            let project = project_root.to_path_buf();
            tokio::spawn(async move {
                while let Some(joined) = tasks.join_next().await {
                    if let Ok((name, fingerprint, outcome)) = joined {
                        manager
                            .finish_start(&project, &name, fingerprint, outcome)
                            .await;
                    }
                }
            });
        }

        let mut snapshot = McpToolSnapshot {
            project: Some(project_root.to_path_buf()),
            ..McpToolSnapshot::default()
        };
        let mut slots = self.slots.lock().await;
        let mut ready: Vec<(String, Arc<Connection>)> = Vec::new();
        for ((root, name), slot) in slots.iter_mut() {
            if root != project_root {
                continue;
            }
            if let Some(connection) = &slot.connection {
                ready.push((name.clone(), connection.clone()));
            }
        }
        drop(slots);
        ready.sort_by(|left, right| left.0.cmp(&right.0));
        let mut taken = HashSet::new();
        for (name, connection) in ready {
            if connection.tools_changed.swap(false, Ordering::SeqCst) {
                match connection.client.list_all_tools().await {
                    Ok(tools) => {
                        let rebuilt = build_entries(&name, &tools, &connection.config, &mut taken);
                        // 原地替换连接的工具列表；下一轮快照读到的也是新列表。
                        if let Ok(mut current) = connection.tools.lock() {
                            *current = rebuilt.clone();
                        }
                        snapshot.servers.push(ServerSnapshot {
                            name: name.clone(),
                            instructions: connection.instructions.clone(),
                            has_resources: connection.has_resources,
                        });
                        snapshot.tools.extend(rebuilt);
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(target: "lx::mcp", %error, server = %name, "tool list refresh failed");
                    }
                }
            }
            if let Ok(tools) = connection.tools.lock() {
                snapshot.tools.extend(tools.clone());
            }
            snapshot.servers.push(ServerSnapshot {
                name,
                instructions: connection.instructions.clone(),
                has_resources: connection.has_resources,
            });
        }
        snapshot.tools.sort_by(|left, right| {
            left.server
                .cmp(&right.server)
                .then(left.raw_name.cmp(&right.raw_name))
        });
        self.store_snapshot(snapshot.clone());
        snapshot
    }

    async fn finish_start(
        &self,
        project_root: &Path,
        name: &str,
        fingerprint: u64,
        outcome: Result<Connection, StartError>,
    ) {
        let key = (project_root.to_path_buf(), name.to_string());
        let mut slots = self.slots.lock().await;
        let Some(slot) = slots.get_mut(&key) else {
            if let Ok(connection) = outcome {
                connection.stop();
            }
            return;
        };
        if slot.fingerprint != fingerprint {
            // 启动期间配置又变了：这次结果属于旧配置，丢弃。
            if let Ok(connection) = outcome {
                connection.stop();
            }
            return;
        }
        slot.starting = false;
        match outcome {
            Ok(connection) => {
                let tool_count = connection
                    .tools
                    .lock()
                    .map(|tools| tools.len())
                    .unwrap_or(0);
                slot.status = ServerStatus::Ready { tools: tool_count };
                slot.connection = Some(Arc::new(connection));
            }
            Err(StartError::NeedsLogin) => {
                slot.status = ServerStatus::NeedsLogin;
                tracing::warn!(target: "lx::mcp", server = %name, "MCP server needs login");
            }
            Err(error) => {
                let error = error.message(name);
                slot.status = ServerStatus::Failed {
                    error: error.clone(),
                };
                tracing::warn!(target: "lx::mcp", server = %name, %error, "MCP server failed to start");
            }
        }
        let status = slot.status.clone();
        drop(slots);
        self.emit(McpEvent::Status {
            project: project_root.to_path_buf(),
            server: name.to_string(),
            status,
        });
        self.emit_progress(project_root).await;
    }

    /// 结算当前项目的启动进度（Ready / Failed / NeedsLogin 都算完成）。
    async fn emit_progress(&self, project_root: &Path) {
        let total = self
            .startup_totals
            .lock()
            .ok()
            .and_then(|totals| totals.get(project_root).copied())
            .unwrap_or(0);
        if total == 0 {
            return;
        }
        let done = {
            let slots = self.slots.lock().await;
            slots
                .iter()
                .filter(|((root, _), _)| root == project_root)
                .filter(|(_, slot)| {
                    matches!(
                        slot.status,
                        ServerStatus::Ready { .. }
                            | ServerStatus::Failed { .. }
                            | ServerStatus::NeedsLogin
                    )
                })
                .count()
        };
        self.emit(McpEvent::StartupProgress {
            project: project_root.to_path_buf(),
            done: done.min(total),
            total,
        });
    }

    fn emit(&self, event: McpEvent) {
        if let Ok(events) = self.events.lock()
            && let Some(sender) = events.as_ref()
        {
            let _ = sender.send(event);
        }
    }

    fn fingerprint(&self, server: &EffectiveServer) -> u64 {
        let config = serde_json::to_vec(&server.config).unwrap_or_default();
        let mut seed = config;
        // 凭据写入（登录/登出/刷新）会推进 credentials.json 的 revision；
        // 内存计数让测试与「尚未落盘」的变更同样能触发重连。
        let revision = self
            .credentials_revision
            .load(Ordering::SeqCst)
            .wrapping_add(super::oauth::revision());
        seed.extend_from_slice(&revision.to_le_bytes());
        bytes_hash(&seed)
    }

    /// 调用一个 MCP 工具；快照里没有该工具时返回明确失败。
    pub async fn call_tool(
        &self,
        snapshot: &McpToolSnapshot,
        session_id: &str,
        qualified: &str,
        arguments: serde_json::Value,
        mut cancel: watch::Receiver<bool>,
    ) -> McpCallOutcome {
        let Some(entry) = snapshot.tool(qualified) else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::ToolMissing,
                text: format!("mcp tool no longer available: {qualified}"),
            };
        };
        let Some(project) = snapshot.project() else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::ToolMissing,
                text: format!("mcp tool no longer available: {qualified}"),
            };
        };
        let connection = {
            let slots = self.slots.lock().await;
            slots
                .get(&(project.to_path_buf(), entry.server.clone()))
                .and_then(|slot| slot.connection.clone())
        };
        let Some(connection) = connection else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::ToolMissing,
                text: format!("mcp tool no longer available: {qualified}"),
            };
        };
        let Some(arguments) = arguments.as_object().cloned() else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::InvalidArguments,
                text: format!("arguments of {qualified} are not a JSON object"),
            };
        };
        let params = CallToolRequestParams::new(entry.raw_name.clone()).with_arguments(arguments);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let mut handle = match connection
            .client
            .peer()
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
        {
            Ok(handle) => handle,
            Err(error) => {
                let text = format!("mcp tool call failed: {error}");
                let needs_login = self
                    .mark_failed(project, &entry.server, &connection, &text)
                    .await;
                return if needs_login {
                    McpCallOutcome::Failure {
                        kind: McpFailureKind::AuthRequired,
                        text: format!("mcp auth required: {}", entry.server),
                    }
                } else {
                    McpCallOutcome::Failure {
                        kind: McpFailureKind::Server,
                        text,
                    }
                };
            }
        };
        let timeout_sec = connection.config.tool_timeout_sec();
        let timeout = tokio::time::sleep(Duration::from_secs(timeout_sec));
        tokio::pin!(timeout);
        tokio::select! {
            response = &mut handle.rx => match response {
                Ok(Ok(ServerResult::CallToolResult(result))) => {
                    convert_result(&result, session_id)
                }
                Ok(Ok(other)) => McpCallOutcome::Failure {
                    kind: McpFailureKind::Server,
                    text: format!("mcp tool call returned unexpected response: {other:?}"),
                },
                Ok(Err(error)) => {
                    let text = format!("mcp tool call failed: {error}");
                    let needs_login = self
                        .mark_failed(project, &entry.server, &connection, &text)
                        .await;
                    if needs_login {
                        McpCallOutcome::Failure {
                            kind: McpFailureKind::AuthRequired,
                            text: format!("mcp auth required: {}", entry.server),
                        }
                    } else {
                        McpCallOutcome::Failure {
                            kind: McpFailureKind::Server,
                            text,
                        }
                    }
                }
                Err(_) => {
                    let text = "mcp tool call failed: connection closed".to_string();
                    let needs_login = self
                        .mark_failed(project, &entry.server, &connection, &text)
                        .await;
                    if needs_login {
                        McpCallOutcome::Failure {
                            kind: McpFailureKind::AuthRequired,
                            text: format!("mcp auth required: {}", entry.server),
                        }
                    } else {
                        McpCallOutcome::Failure {
                            kind: McpFailureKind::Server,
                            text,
                        }
                    }
                }
            },
            _ = cancel.changed() => {
                let _ = handle.cancel(Some("user cancelled".into())).await;
                McpCallOutcome::Cancelled
            }
            _ = &mut timeout => {
                let _ = handle.cancel(Some("tool timeout".into())).await;
                McpCallOutcome::Failure {
                    kind: McpFailureKind::Timeout,
                    text: format!("mcp tool timed out after {timeout_sec}s"),
                }
            }
        }
    }

    /// 传输层失败：OAuth server 先尝试刷新 token；刷新成功则下一轮重连，
    /// 否则标记 NeedsLogin / Failed（不自动重试）。
    async fn mark_failed(
        &self,
        project: &Path,
        server: &str,
        connection: &Connection,
        error: &str,
    ) -> bool {
        let mut status = ServerStatus::Failed {
            error: error.to_string(),
        };
        if let (Some(key), McpTransportConfig::Http { url, .. }) =
            (&connection.credential_key, &connection.config.transport)
        {
            if super::oauth::refresh(key, url).await {
                status = ServerStatus::Starting;
            } else {
                status = ServerStatus::NeedsLogin;
            }
        }
        let key = (project.to_path_buf(), server.to_string());
        let needs_login = matches!(status, ServerStatus::NeedsLogin);
        let mut slots = self.slots.lock().await;
        let Some(slot) = slots.get_mut(&key) else {
            return needs_login;
        };
        if let Some(connection) = slot.connection.take() {
            connection.stop();
        }
        slot.starting = false;
        slot.status = status.clone();
        drop(slots);
        self.emit(McpEvent::Status {
            project: project.to_path_buf(),
            server: server.to_string(),
            status,
        });
        needs_login
    }

    /// 列资源；`server` 省略时遍历所有声明 resources 的 server。
    pub async fn list_resources(
        &self,
        snapshot: &McpToolSnapshot,
        server: Option<&str>,
        cursor: Option<&str>,
    ) -> McpCallOutcome {
        self.list_resources_inner(snapshot, server, cursor, false)
            .await
    }

    /// 列资源模板。
    pub async fn list_resource_templates(
        &self,
        snapshot: &McpToolSnapshot,
        server: Option<&str>,
        cursor: Option<&str>,
    ) -> McpCallOutcome {
        self.list_resources_inner(snapshot, server, cursor, true)
            .await
    }

    async fn list_resources_inner(
        &self,
        snapshot: &McpToolSnapshot,
        server: Option<&str>,
        cursor: Option<&str>,
        templates: bool,
    ) -> McpCallOutcome {
        let servers: Vec<ServerSnapshot> = match server {
            Some(name) => match snapshot.server(name) {
                Some(server) if server.has_resources => vec![server.clone()],
                Some(_) => {
                    return McpCallOutcome::Failure {
                        kind: McpFailureKind::InvalidArguments,
                        text: format!("MCP server '{name}' does not expose resources"),
                    };
                }
                None => {
                    return McpCallOutcome::Failure {
                        kind: McpFailureKind::InvalidArguments,
                        text: format!("unknown MCP server '{name}'"),
                    };
                }
            },
            None => snapshot
                .servers
                .iter()
                .filter(|server| server.has_resources)
                .cloned()
                .collect(),
        };
        let mut items = Vec::new();
        let mut next_cursor = None;
        for server_snapshot in &servers {
            let Some(connection) = self.connection_for(snapshot, &server_snapshot.name).await
            else {
                continue;
            };
            let params = cursor.map(|cursor| {
                PaginatedRequestParams::default().with_cursor(Some(cursor.to_string()))
            });
            if templates {
                match connection.client.list_resource_templates(params).await {
                    Ok(result) => {
                        for template in result.resource_templates {
                            items.push(serde_json::json!({
                                "server": server_snapshot.name,
                                "uriTemplate": template.uri_template,
                                "name": template.name,
                                "description": template.description,
                                "mimeType": template.mime_type,
                            }));
                        }
                        next_cursor = result.next_cursor;
                    }
                    Err(error) => {
                        return resource_error(&server_snapshot.name, error);
                    }
                }
            } else {
                match connection.client.list_resources(params).await {
                    Ok(result) => {
                        for resource in result.resources {
                            items.push(serde_json::json!({
                                "server": server_snapshot.name,
                                "uri": resource.uri,
                                "name": resource.name,
                                "mimeType": resource.mime_type,
                                "description": resource.description,
                            }));
                        }
                        next_cursor = result.next_cursor;
                    }
                    Err(error) => {
                        return resource_error(&server_snapshot.name, error);
                    }
                }
            }
            if server.is_some() {
                break;
            }
        }
        let mut payload = serde_json::json!({ "resources": items });
        if templates {
            payload["kind"] = serde_json::json!("resource_templates");
        }
        if let Some(cursor) = next_cursor {
            payload["nextCursor"] = serde_json::json!(cursor);
        }
        McpCallOutcome::Success {
            text: payload.to_string(),
            images: Vec::new(),
            images_omitted: 0,
            truncated: false,
        }
    }

    async fn connection_for(
        &self,
        snapshot: &McpToolSnapshot,
        server: &str,
    ) -> Option<Arc<Connection>> {
        let project = snapshot.project()?;
        let slots = self.slots.lock().await;
        slots
            .get(&(project.to_path_buf(), server.to_string()))
            .and_then(|slot| slot.connection.clone())
    }

    /// 读取一个资源；内容按工具结果规则转换（图片归档）。
    pub async fn read_resource(
        &self,
        snapshot: &McpToolSnapshot,
        session_id: &str,
        server: &str,
        uri: &str,
    ) -> McpCallOutcome {
        let Some(connection) = self.connection_for(snapshot, server).await else {
            return McpCallOutcome::Failure {
                kind: McpFailureKind::InvalidArguments,
                text: format!("unknown MCP server '{server}'"),
            };
        };
        match connection
            .client
            .read_resource(ReadResourceRequestParams::new(uri.to_string()))
            .await
        {
            Ok(result) => {
                let mut text = String::new();
                let mut images = Vec::new();
                let mut omitted = 0usize;
                for content in result.contents {
                    match content {
                        ResourceContents::TextResourceContents {
                            uri, text: body, ..
                        } => {
                            text.push_str(&format!("{uri}:\n{body}\n"));
                        }
                        ResourceContents::BlobResourceContents {
                            uri,
                            mime_type,
                            blob,
                            ..
                        } => {
                            let mime =
                                mime_type.unwrap_or_else(|| "application/octet-stream".into());
                            if mime.starts_with("image/") {
                                if images.len() >= super::result::MAX_RESULT_IMAGES {
                                    omitted = omitted.saturating_add(1);
                                } else {
                                    match super::result::archive_image(&blob, &mime, session_id) {
                                        Ok(image) => images.push(image),
                                        Err(error) => {
                                            omitted = omitted.saturating_add(1);
                                            tracing::warn!(
                                                target: "lx::mcp",
                                                %error,
                                                "resource image dropped"
                                            );
                                        }
                                    }
                                }
                            } else {
                                text.push_str(&format!(
                                    "[resource {uri} {mime}, {} bytes omitted]\n",
                                    super::result::decoded_len(&blob)
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                if omitted > 0 {
                    text.push_str(&format!("[{omitted} more image(s) omitted]\n"));
                }
                let (text, truncated) = super::result::truncate_text(
                    text.trim_end(),
                    super::result::MAX_TOOL_RESULT_BYTES,
                );
                McpCallOutcome::Success {
                    text,
                    images,
                    images_omitted: omitted,
                    truncated,
                }
            }
            Err(error) => resource_error(server, error),
        }
    }

    /// 设置页「测试连接」：一次性启动、列举工具、立即关闭。
    pub async fn probe(
        &self,
        server: &EffectiveServer,
        project_root: &Path,
    ) -> Result<Vec<String>, String> {
        let name = &server.name;
        let timeout = Duration::from_secs(server.config.startup_timeout_sec());
        let connection = tokio::time::timeout(timeout, start_connection(server, project_root))
            .await
            .map_err(|_| format!("MCP server \"{name}\" startup timed out"))?
            .map_err(|error| error.message(name))?;
        let tools: Vec<String> = connection
            .tools
            .lock()
            .map(|tools| tools.iter().map(|tool| tool.raw_name.clone()).collect())
            .unwrap_or_default();
        connection.stop();
        Ok(tools)
    }

    /// 关闭全部连接：先取消，2 秒内优雅退出；随后强制终止 stdio 进程组。
    pub async fn shutdown(&self) {
        let connections: Vec<Arc<Connection>> = {
            let mut slots = self.slots.lock().await;
            slots
                .drain()
                .filter_map(|(_, slot)| slot.connection)
                .collect()
        };
        for connection in &connections {
            connection.stop();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        for connection in &connections {
            while !connection.client.is_closed() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            // 直接子进程退出不等于它拉起的 npx/node 子树退出：整组兜底。
            connection.kill_group();
        }
    }

    /// 强制重启一个（或全部）server：释放连接后下一次预热重新启动。
    pub async fn restart(&self, project_root: &Path, server: Option<&str>) {
        let mut stopped = Vec::new();
        {
            let mut slots = self.slots.lock().await;
            let keys: Vec<(PathBuf, String)> = slots
                .keys()
                .filter(|(root, name)| {
                    root == project_root && server.is_none_or(|server| name == server)
                })
                .cloned()
                .collect();
            for key in keys {
                if let Some(slot) = slots.remove(&key)
                    && let Some(connection) = slot.connection
                {
                    stopped.push(connection);
                }
            }
        }
        for connection in stopped {
            connection.stop();
        }
    }

    /// 当前状态（设置页 / 启动进度）。
    pub async fn statuses(&self, project_root: &Path) -> Vec<(String, ServerStatus)> {
        let slots = self.slots.lock().await;
        let mut out: Vec<(String, ServerStatus)> = slots
            .iter()
            .filter(|((root, _), _)| root == project_root)
            .map(|((_, name), slot)| (name.clone(), slot.status.clone()))
            .collect();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        out
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级单例。
pub fn manager() -> &'static Arc<McpManager> {
    static MANAGER: OnceLock<Arc<McpManager>> = OnceLock::new();
    MANAGER.get_or_init(|| Arc::new(McpManager::new()))
}

fn resource_error(server: &str, error: impl std::fmt::Display) -> McpCallOutcome {
    McpCallOutcome::Failure {
        kind: McpFailureKind::Server,
        text: format!("MCP server '{server}': {error}"),
    }
}

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::mcp::McpSection;
    use std::collections::BTreeMap;

    #[test]
    fn snapshot_prompt_lists_ready_servers_only() {
        let snapshot = McpToolSnapshot {
            project: None,
            tools: Vec::new(),
            servers: vec![ServerSnapshot {
                name: "github".into(),
                instructions: Some("use issues".into()),
                has_resources: false,
            }],
        };
        let prompt = snapshot.instructions_prompt().expect("prompt");
        assert!(prompt.contains("## MCP servers"));
        assert!(prompt.contains("### github"));
        assert!(prompt.contains("use issues"));
        assert!(McpToolSnapshot::default().instructions_prompt().is_none());
    }

    #[test]
    fn empty_config_fingerprint_is_stable() {
        let manager = McpManager::new();
        let server = EffectiveServer {
            name: "x".into(),
            scope: crate::config::mcp::McpScope::User,
            config: McpServerConfig {
                enabled: true,
                transport: McpTransportConfig::Stdio {
                    command: "true".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    env_vars: Vec::new(),
                },
                startup_timeout_sec: None,
                tool_timeout_sec: None,
                enabled_tools: None,
                disabled_tools: Vec::new(),
            },
            shadowed_user: false,
        };
        assert_eq!(manager.fingerprint(&server), manager.fingerprint(&server));
        let _ = McpSection::default();
    }
}
