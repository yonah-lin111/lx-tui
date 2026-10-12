//! 本地设置 Web 服务：仅绑定 `127.0.0.1:0`，每次运行随机 capability token。
//!
//! 事件经 tokio channel 投递回 TUI 事件循环（配置保存、命令变更），
//! TUI 状态变更只发生在事件循环内；命令/设置读写复用 `settings::commands`
//! 与 `config` 的存储函数。浏览器启动由 `src/platform/` 负责。

mod handlers;
mod page;

#[cfg(test)]
mod tests;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Config;

pub use handlers::{CommandView, Response};

/// Web 设置页产生的事件：由事件循环消费。
#[derive(Debug)]
pub enum SettingsEvent {
    /// 设置已保存（config 已落盘并通过校验）。
    ConfigChanged(Config),
    /// 命令文件发生增删改，TUI 应重载自定义命令。
    CommandsChanged,
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

    /// 命令快照副本。
    pub fn commands(&self) -> Vec<CommandView> {
        self.commands
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }
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

    /// 刷新服务端的命令列表快照（TUI 在命令变化后调用）。
    pub fn set_commands_snapshot(&self, commands: &[CommandView]) {
        if let Ok(mut slot) = self.state.commands.lock() {
            *slot = commands.to_vec();
        }
    }

    /// 请求关闭；监听任务随后退出。幂等。
    pub fn shutdown(&self) {
        self.state.shutdown.store(true, Ordering::SeqCst);
    }
}

/// 接受循环：超时轮询 accept，逐连接并发处理（上限由连接超时兜底）。
async fn accept_loop(listener: TcpListener, state: Arc<ServiceState>) {
    loop {
        if state.shutdown.load(Ordering::SeqCst) {
            return;
        }
        match tokio::time::timeout(ACCEPT_TICK, listener.accept()).await {
            Ok(Ok((stream, _))) => {
                let connection = state.clone();
                tokio::spawn(async move {
                    let _ =
                        tokio::time::timeout(CONNECTION_TIMEOUT, serve(stream, connection)).await;
                });
            }
            // accept 失败不致命：短暂退避后继续。
            Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(_) => {}
        }
    }
}

/// 单连接处理：读满请求（header + Content-Length body）、路由、写回响应。
async fn serve(mut stream: TcpStream, state: Arc<ServiceState>) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let request = loop {
        if let Some(request) = handlers::parse_request(&buffer)? {
            break Some(request);
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_HEADER_BYTES.saturating_add(MAX_BODY_BYTES) {
            write_response(
                &mut stream,
                413,
                "application/json",
                "{\"error\":\"request too large\"}",
            )
            .await?;
            return Ok(());
        }
    };
    let Some(request) = request else {
        write_response(
            &mut stream,
            400,
            "application/json",
            "{\"error\":\"bad request\"}",
        )
        .await?;
        return Ok(());
    };
    let response = handlers::route(&request, &state);
    write_response(
        &mut stream,
        response.status,
        response.content_type,
        &response.body,
    )
    .await
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
