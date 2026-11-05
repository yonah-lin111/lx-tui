//! 本地设置 Web 服务：仅绑定 `127.0.0.1:0`，每次运行随机 capability token。
//!
//! 事件经 tokio channel 投递回 TUI 事件循环（配置保存、命令变更），
//! TUI 状态变更只发生在事件循环内；命令/设置读写复用 `settings::commands`
//! 与 `config` 的存储函数。浏览器启动由 `src/platform/` 负责。

mod handlers;
mod mcp;
mod models;
mod page;
mod skills;
mod subagents;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_helpers;
#[cfg(test)]
mod tests_web_search;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Config;

pub use handlers::{CommandView, Request, Response};

/// TUI 推送给设置页的已打开项目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectView {
    /// 作用域目录名（`user` 或 `<目录名>-<哈希>`）。
    pub scope: String,
    pub root: std::path::PathBuf,
    pub name: String,
    pub trusted: bool,
}

/// TUI 推送的 MCP 运行状态快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpStatusView {
    pub project: std::path::PathBuf,
    pub server: String,
    /// `starting` / `ready` / `failed` / `needs_login` / `disabled`。
    pub status: String,
    pub error: Option<String>,
}

/// Web 设置页产生的事件：由事件循环消费。
#[derive(Debug)]
pub enum SettingsEvent {
    /// 设置已保存（config 已落盘并通过校验）。
    ConfigChanged(Config),
    /// 命令文件发生增删改，TUI 应重载自定义命令。
    CommandsChanged,
    /// 模型配置已保存；携带合并后的新配置快照。
    ModelsChanged(Box<crate::config::ai::AiSettings>),
    /// MCP 配置发生增删改：事件层重新合并并预热。
    McpChanged,
    /// 设置页要求强制重启某个 server（所有打开的项目）。
    McpRestart { server: String },
    /// OAuth 登录/登出完成：凭据 revision 变化，需要重连。
    McpCredentialsChanged,
    /// skill 内容或启停状态变化：失效缓存并刷新 `/` 面板。
    SkillsChanged,
    /// 子 agent 角色或并发上限配置已保存。
    SubagentsChanged(Box<crate::config::subagents::SubagentsSection>),
}

/// 服务共享状态：token、事件发送端、命令快照与关闭标记。
pub struct ServiceState {
    token: String,
    sender: UnboundedSender<SettingsEvent>,
    /// 用户级 settings.json 路径；测试注入临时路径。
    config_path: PathBuf,
    /// 用户数据根覆盖（测试隔离；生产为 None 表示 `~/.lx-t[-dev]`）。
    user_root: Option<PathBuf>,
    /// 监听端口（同源校验用）。
    port: u16,
    /// 命令快照（由 TUI 推送）。
    commands: std::sync::Mutex<Vec<CommandView>>,
    /// 已打开项目（由 TUI 推送；scope → root 的唯一来源）。
    projects: std::sync::Mutex<Vec<ProjectView>>,
    /// MCP 运行状态快照（由 TUI 推送）。
    mcp_status: std::sync::Mutex<Vec<McpStatusView>>,
    /// 进行中的 OAuth 授权（state 一次性，10 分钟过期）。
    oauth: tokio::sync::Mutex<
        std::collections::HashMap<String, crate::mcp::oauth::PendingAuthorization>,
    >,
    /// 进行中的 Quick Connect OAuth 尝试（设置页轮询）。
    connect_oauth: models::connect::PendingConnectOAuthMap,
    shutdown: AtomicBool,
}

impl ServiceState {
    /// 本次运行的 capability token。
    pub fn token(&self) -> &str {
        &self.token
    }

    /// 事件发送端。
    pub fn sender(&self) -> &UnboundedSender<SettingsEvent> {
        &self.sender
    }

    /// 用户 settings.json 路径。
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// 监听端口。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 用户数据根：测试注入优先，否则按环境推导。
    pub fn user_root(&self) -> Option<PathBuf> {
        self.user_root.clone()
    }

    /// 用最新 User 命令替换快照中的 User 部分（Project 部分由 TUI 提供，保留）。
    pub fn replace_user_commands(&self, user: Vec<CommandView>) {
        if let Ok(mut slot) = self.commands.lock() {
            let mut merged = user;
            merged.extend(
                slot.iter()
                    .filter(|view| view.scope == "project")
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            *slot = merged;
        }
    }

    /// 命令快照副本。
    pub fn commands(&self) -> Vec<CommandView> {
        self.commands
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// 已打开项目副本。
    pub fn projects(&self) -> Vec<ProjectView> {
        self.projects
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// 替换项目列表；返回是否真的变化（调用方据此避免重复推送）。
    pub fn set_projects(&self, projects: Vec<ProjectView>) -> bool {
        let Ok(mut slot) = self.projects.lock() else {
            return false;
        };
        if *slot == projects {
            return false;
        }
        *slot = projects;
        true
    }

    /// MCP 状态快照副本。
    pub fn mcp_status(&self) -> Vec<McpStatusView> {
        self.mcp_status
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// 替换 MCP 状态快照；返回是否真的变化。
    pub fn set_mcp_status(&self, statuses: Vec<McpStatusView>) -> bool {
        let Ok(mut slot) = self.mcp_status.lock() else {
            return false;
        };
        if *slot == statuses {
            return false;
        }
        *slot = statuses;
        true
    }

    /// OAuth 授权表（设置服务内部使用）。
    pub(crate) fn oauth(
        &self,
    ) -> &tokio::sync::Mutex<
        std::collections::HashMap<String, crate::mcp::oauth::PendingAuthorization>,
    > {
        &self.oauth
    }

    /// Quick Connect OAuth 尝试表（后台任务与请求处理器共享同一份）。
    pub(crate) fn connect_oauth(&self) -> models::connect::PendingConnectOAuthMap {
        self.connect_oauth.clone()
    }

    /// 按 scope 找项目根；`user` 返回 None（调用方按用户作用域处理）。
    pub fn project_for_scope(&self, scope: &str) -> Option<ProjectView> {
        self.projects()
            .into_iter()
            .find(|project| project.scope == scope)
    }

    /// 按路径找项目（MCP 状态推送用）。
    pub fn project_for_root(&self, root: &std::path::Path) -> Option<ProjectView> {
        self.projects()
            .into_iter()
            .find(|project| project.root == root)
    }
}

/// 模型配置文件（`config/ai.json`）路径：测试注入的用户根优先。
///
/// `handlers`（General 页保存 Key）与 `models`（模型页读写）共用同一份推导。
pub(super) fn ai_path(state: &ServiceState) -> PathBuf {
    match state.user_root() {
        Some(root) => root.join("config").join("ai.json"),
        None => {
            crate::config::ai::ai_config_path().unwrap_or_else(|_| PathBuf::from("config/ai.json"))
        }
    }
}

/// 快捷连接配置（`config/connect.json`）路径：与 `ai.json` 同目录。
pub(super) fn connect_path(state: &ServiceState) -> PathBuf {
    crate::config::ai::connect::connect_path_for_ai_path(&ai_path(state))
}

/// 运行中的本地设置服务句柄。
pub struct SettingsService {
    state: Arc<ServiceState>,
    port: u16,
}

/// 请求行与 header 总字节上限。
const MAX_HEADER_BYTES: usize = 16 * 1024;
/// 请求体上限。
const MAX_BODY_BYTES: usize = 256 * 1024;
/// 单连接处理上限等待。
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
/// accept 轮询粒度：以该间隔醒来检查关闭标记。
const ACCEPT_TICK: Duration = Duration::from_millis(200);

impl SettingsService {
    /// 启动服务：绑定 `127.0.0.1:0`，生成本次运行的随机 token；
    /// 事件经 `sender` 投递（tokio unbounded mpsc）。
    pub async fn start(sender: UnboundedSender<SettingsEvent>) -> std::io::Result<Self> {
        let config_path =
            crate::config::settings_path().unwrap_or_else(|_| PathBuf::from("settings.json"));
        Self::start_inner(sender, config_path, None).await
    }

    /// 测试入口：注入 settings.json 路径与用户数据根，不触碰真实用户目录。
    #[cfg(test)]
    pub(crate) async fn start_in(
        sender: UnboundedSender<SettingsEvent>,
        config_path: PathBuf,
        user_root: PathBuf,
    ) -> std::io::Result<Self> {
        Self::start_inner(sender, config_path, Some(user_root)).await
    }

    async fn start_inner(
        sender: UnboundedSender<SettingsEvent>,
        config_path: PathBuf,
        user_root: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(ServiceState {
            token: random_token(),
            sender,
            config_path,
            user_root,
            port,
            commands: std::sync::Mutex::new(Vec::new()),
            projects: std::sync::Mutex::new(Vec::new()),
            mcp_status: std::sync::Mutex::new(Vec::new()),
            oauth: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            connect_oauth: models::connect::PendingConnectOAuthMap::default(),
            shutdown: AtomicBool::new(false),
        });
        let accept_state = state.clone();
        tokio::spawn(async move {
            accept_loop(listener, accept_state).await;
        });
        Ok(Self { state, port })
    }

    /// 实际监听端口。
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 本次运行的 capability token（访问设置页需要）。
    pub fn token(&self) -> &str {
        self.state.token()
    }

    /// 浏览器访问 URL：`http://127.0.0.1:<port>/?token=<token>`。
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?token={}", self.port, self.token())
    }

    /// 测试注入的用户数据根（生产为 None，走环境推导）。
    #[cfg(test)]
    pub(crate) fn user_root_for_tests(&self) -> PathBuf {
        self.state
            .user_root()
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 刷新服务端的命令列表快照（TUI 在命令变化后调用）。
    pub fn set_commands_snapshot(&self, commands: &[CommandView]) {
        if let Ok(mut slot) = self.state.commands.lock() {
            *slot = commands.to_vec();
        }
    }

    /// 推送已打开项目列表；返回是否变化。
    pub fn set_projects(&self, projects: Vec<ProjectView>) -> bool {
        self.state.set_projects(projects)
    }

    /// 推送 MCP 运行状态快照；返回是否变化。
    pub fn set_mcp_status(&self, statuses: Vec<McpStatusView>) -> bool {
        self.state.set_mcp_status(statuses)
    }

    /// 用最新 User 命令替换快照中的 User 部分（Project 部分保留）。
    pub fn replace_user_commands(&self, user: Vec<CommandView>) {
        self.state.replace_user_commands(user);
    }

    /// 请求关闭；监听任务随后退出。幂等。
    pub fn shutdown(&self) {
        self.state.shutdown.store(true, Ordering::SeqCst);
    }
}

/// 接受循环：超时轮询 accept，逐连接并发处理（读超时由 `serve` 内部兜底）。
async fn accept_loop(listener: TcpListener, state: Arc<ServiceState>) {
    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            return;
        }
        match tokio::time::timeout(ACCEPT_TICK, listener.accept()).await {
            Ok(Ok((stream, _))) => {
                let connection = state.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, connection).await;
                });
            }
            // accept 失败不致命：短暂退避后继续。
            Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(_) => {}
        }
    }
}

/// 单连接处理：读满请求（header + Content-Length body）、路由、写回响应。
///
/// 读请求受 `CONNECTION_TIMEOUT` 约束（防慢连接）；路由/写回不套该短超时，
/// 因为模型发现是显式用户动作且自带 10s 连接 / 30s 总超时。
async fn serve(mut stream: TcpStream, state: Arc<ServiceState>) -> std::io::Result<()> {
    let request = match tokio::time::timeout(CONNECTION_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(Some(request))) => request,
        Ok(Ok(None)) => {
            write_response(
                &mut stream,
                400,
                "application/json",
                "{\"error\":\"bad request\"}",
            )
            .await?;
            return Ok(());
        }
        Ok(Err(error)) => return Err(error),
        // 读超时（慢连接）：直接放弃，不写响应。
        Err(_) => return Ok(()),
    };
    let response = handlers::route(&request, &state).await;
    write_response(
        &mut stream,
        response.status,
        response.content_type,
        &response.body,
    )
    .await
}

/// 读满一个完整请求；数据不完整返回 None（EOF），超过上限返回 413 占位请求。
async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(request) = handlers::parse_request(&buffer)? {
            return Ok(Some(request));
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_HEADER_BYTES.saturating_add(MAX_BODY_BYTES) {
            return Ok(Some(handlers::oversized_request()));
        }
    }
}

/// 写回 HTTP/1.1 响应并关闭连接。
pub(crate) async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        status_text(status),
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// 状态码短语。
fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

/// 生成 16 字节十六进制随机 token（优先 /dev/urandom，失败退化为时间/进程熵）。
fn random_token() -> String {
    let mut bytes = [0_u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
        .is_err()
    {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or_default();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let fallback = now ^ u64::from(std::process::id()) ^ (seq << 32);
        bytes[..8].copy_from_slice(&fallback.to_le_bytes());
        bytes[8..].copy_from_slice(&seq.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
