//! 连接建立：stdio / Streamable HTTP、OAuth token 注入、initialize 与工具发现。

use super::*;

/// 启动失败的两种语义：普通失败与需要 OAuth 登录。
#[derive(Debug, Clone)]
pub(super) enum StartError {
    Failed(String),
    NeedsLogin,
}

impl StartError {
    pub(super) fn message(&self, name: &str) -> String {
        match self {
            Self::Failed(message) => message.clone(),
            Self::NeedsLogin => format!("mcp auth required: {name}"),
        }
    }
}
pub(super) async fn start_connection(
    server: &EffectiveServer,
    project_root: &Path,
) -> Result<Connection, StartError> {
    let name = &server.name;
    let config = &server.config;
    match &config.transport {
        McpTransportConfig::Stdio {
            command,
            args,
            cwd,
            env,
            env_vars,
        } => {
            let command = crate::platform::mcp_command(
                command,
                args,
                cwd.as_deref(),
                project_root,
                env_vars,
                env,
            );
            let (transport, stderr) = rmcp::transport::TokioChildProcess::builder(command)
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|error| {
                    StartError::Failed(format!("failed to spawn '{name}': {error}"))
                })?;
            let pid = transport.id();
            if let Some(stderr) = stderr {
                spawn_stderr_logger(name.to_string(), stderr);
            }
            let handler = LxClientHandler::default();
            let tools_changed = handler.tools_changed.clone();
            let client = handler
                .serve(transport)
                .await
                .map_err(|error| StartError::Failed(format!("initialize failed: {error}")))?;
            connect_client(name, config, client, tools_changed, pid, None).await
        }
        McpTransportConfig::Http {
            url,
            auth,
            bearer_token,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            scopes,
            client_id,
            client_secret,
        } => {
            let mut transport_config =
                rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                    url.clone(),
                );
            let mut credential_key = None;
            match auth {
                McpHttpAuth::Bearer => {
                    let token = bearer_token
                        .clone()
                        .or_else(|| {
                            bearer_token_env_var
                                .as_ref()
                                .and_then(|name| std::env::var(name).ok())
                        })
                        .ok_or(StartError::NeedsLogin)?;
                    transport_config = transport_config.auth_header(token);
                }
                McpHttpAuth::Oauth => {
                    let key = crate::mcp::oauth::credential_key(server, project_root);
                    let token = crate::mcp::oauth::access_token_for(
                        &key,
                        url,
                        scopes,
                        client_id.as_deref(),
                        client_secret.as_deref(),
                    )
                    .await
                    .map_err(|error| match error {
                        crate::mcp::oauth::TokenError::NeedsLogin => StartError::NeedsLogin,
                        crate::mcp::oauth::TokenError::Failed(message) => {
                            StartError::Failed(message)
                        }
                    })?;
                    transport_config = transport_config.auth_header(token);
                    credential_key = Some(key);
                }
                McpHttpAuth::None => {}
            }
            let mut headers = std::collections::HashMap::new();
            for (header, value) in http_headers {
                headers.insert(
                    header.parse().map_err(|_| {
                        StartError::Failed(format!("invalid header name '{header}'"))
                    })?,
                    value.parse().map_err(|_| {
                        StartError::Failed(format!("invalid header value for '{header}'"))
                    })?,
                );
            }
            for (header, variable) in env_http_headers {
                if let Ok(value) = std::env::var(variable) {
                    headers.insert(
                        header.parse().map_err(|_| {
                            StartError::Failed(format!("invalid header name '{header}'"))
                        })?,
                        value.parse().map_err(|_| {
                            StartError::Failed(format!("invalid header value for '{header}'"))
                        })?,
                    );
                }
            }
            transport_config.custom_headers = headers;
            let transport =
                rmcp::transport::StreamableHttpClientTransport::from_config(transport_config);
            let handler = LxClientHandler::default();
            let tools_changed = handler.tools_changed.clone();
            let client = handler
                .serve(transport)
                .await
                .map_err(|error| StartError::Failed(format!("initialize failed: {error}")))?;
            connect_client(name, config, client, tools_changed, None, credential_key).await
        }
    }
}

async fn connect_client(
    name: &str,
    config: &McpServerConfig,
    client: RunningService<RoleClient, LxClientHandler>,
    tools_changed: Arc<AtomicBool>,
    pid: Option<u32>,
    credential_key: Option<String>,
) -> Result<Connection, StartError> {
    let info = client.peer().peer_info();
    let instructions = info.as_ref().and_then(|info| info.instructions.clone());
    let has_resources = info
        .as_ref()
        .is_some_and(|info| info.capabilities.resources.is_some());
    let tools = client
        .list_all_tools()
        .await
        .map_err(|error| StartError::Failed(format!("tools/list failed: {error}")))?;
    let mut taken = HashSet::new();
    let entries = build_entries(name, &tools, config, &mut taken);
    let cancel = StdMutex::new(Some(client.cancellation_token()));
    Ok(Connection {
        client,
        cancel,
        tools: StdMutex::new(entries),
        instructions,
        has_resources,
        tools_changed,
        config: config.clone(),
        pid,
        credential_key,
    })
}

pub(super) fn build_entries(
    server: &str,
    tools: &[rmcp::model::Tool],
    config: &McpServerConfig,
    taken: &mut HashSet<String>,
) -> Vec<McpToolEntry> {
    let enabled: Option<HashSet<String>> = config
        .enabled_tools
        .as_ref()
        .map(|tools| tools.iter().cloned().collect());
    let disabled: HashSet<String> = config.disabled_tools.iter().cloned().collect();
    let mut entries = Vec::new();
    for tool in tools {
        let raw_name = tool.name.to_string();
        if let Some(enabled) = &enabled
            && !enabled.contains(&raw_name)
        {
            continue;
        }
        if disabled.contains(&raw_name) {
            continue;
        }
        let schema = serde_json::Value::Object(tool.input_schema.as_ref().clone());
        let (parameters, omitted) = sanitize_json_schema(&schema);
        let mut description = tool.description.as_deref().unwrap_or_default().to_string();
        if description.chars().count() > 1024 {
            description = description.chars().take(1024).collect();
        }
        if omitted {
            description.push_str(SCHEMA_OMITTED_NOTE);
        }
        entries.push(McpToolEntry {
            qualified: qualified_name(server, &raw_name, taken),
            server: server.to_string(),
            raw_name,
            description,
            parameters,
            read_only_hint: tool
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.read_only_hint)
                .unwrap_or(false),
        });
    }
    for entry in &entries {
        taken.insert(entry.qualified.clone());
    }
    entries.sort_by(|left, right| left.raw_name.cmp(&right.raw_name));
    entries
}

fn spawn_stderr_logger(server: String, stderr: tokio::process::ChildStderr) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line: String = line.chars().take(1024).collect();
            tracing::info!(target: "lx::mcp", server = %server, "{line}");
        }
    });
}
