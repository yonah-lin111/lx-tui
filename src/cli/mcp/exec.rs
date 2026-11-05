//! `lx-tui mcp` 子命令执行、探测与 OAuth 授权。

use super::*;

pub fn run(cli: McpCli, linked_worktree: Option<&Path>) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(cli.global.json, cli.global.color);
    match cli.command {
        McpCommand::List { scope } => run_list(&ctx, &out, scope)?,
        McpCommand::Show { name, scope } => run_show(&ctx, &out, &name, scope)?,
        McpCommand::AddStdio(args) => run_add_stdio(&ctx, &out, &args)?,
        McpCommand::AddHttp(args) => run_add_http(&ctx, &out, &args)?,
        McpCommand::Remove { name, scope } => run_remove(&ctx, &out, &name, scope)?,
        McpCommand::Enable { name, scope } => run_toggle(&ctx, &out, &name, scope, true)?,
        McpCommand::Disable { name, scope } => run_toggle(&ctx, &out, &name, scope, false)?,
        McpCommand::Probe { name, scope } => run_probe(&ctx, &out, &name, scope)?,
        McpCommand::Login(args) => run_login(&ctx, &out, &args)?,
        McpCommand::Logout { name, scope } => run_logout(&ctx, &out, &name, scope)?,
    }
    Ok(EXIT_OK)
}
fn user_path() -> Result<PathBuf, String> {
    user_mcp_path().map_err(|error| error.to_string())
}
fn project_path(workspace: &Path) -> Result<PathBuf, String> {
    project_mcp_path(workspace).map_err(|error| error.to_string())
}
fn load_file(path: &Path) -> Result<McpConfigFile, String> {
    load_mcp(path)
        .map_err(|error| error.to_string())
        .map(|file| file.unwrap_or_default())
}
fn is_trusted(ctx: &WorkspaceContext) -> Result<bool, String> {
    ctx.open_db()?
        .is_trusted(&ctx.workspace)
        .map_err(|error| error.code().to_string())
}
fn require_trusted(ctx: &WorkspaceContext) -> Result<(), String> {
    if is_trusted(ctx)? {
        return Ok(());
    }
    Err(format!(
        "workspace '{}' is not trusted; run 'lx-tui workspace trust' first",
        ctx.workspace.display()
    ))
}
fn write_path(ctx: &WorkspaceContext, scope: ScopeArg) -> Result<PathBuf, String> {
    match scope {
        ScopeArg::User => user_path(),
        ScopeArg::Project => {
            require_trusted(ctx)?;
            project_path(&ctx.workspace)
        }
    }
}
fn effective_servers_for(ctx: &WorkspaceContext) -> Result<Vec<EffectiveServer>, String> {
    let trusted = is_trusted(ctx)?;
    let user = load_mcp(&user_path()?).map_err(|error| error.to_string())?;
    let project = load_mcp(&project_path(&ctx.workspace)?).map_err(|error| error.to_string())?;
    Ok(effective_servers(
        user.as_ref(),
        project.as_ref(),
        trusted,
        Some(&ctx.workspace),
    ))
}
fn fetch_server(
    ctx: &WorkspaceContext,
    name: &str,
    scope: Option<ScopeArg>,
) -> Result<EffectiveServer, String> {
    if let Some(scope) = scope {
        let path = match scope {
            ScopeArg::User => user_path()?,
            ScopeArg::Project => project_path(&ctx.workspace)?,
        };
        let config = load_file(&path)?
            .mcp
            .servers
            .get(name)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "MCP server '{name}' is not configured in the {} scope",
                    scope.as_str()
                )
            })?;
        return Ok(EffectiveServer {
            name: name.into(),
            scope: match scope {
                ScopeArg::User => McpScope::User,
                ScopeArg::Project => McpScope::Project,
            },
            config,
            shadowed_user: false,
        });
    }
    effective_servers_for(ctx)?
        .into_iter()
        .find(|server| server.name == name)
        .ok_or_else(|| {
            format!("MCP server '{name}' is not available; check '--scope' or trust the workspace")
        })
}
fn transport_label(config: &McpServerConfig) -> &'static str {
    match &config.transport {
        McpTransportConfig::Stdio { .. } => "stdio",
        McpTransportConfig::Http { .. } => "http",
    }
}
fn target_label(config: &McpServerConfig) -> String {
    match &config.transport {
        McpTransportConfig::Stdio { command, args, .. } => {
            if args.is_empty() {
                command.clone()
            } else {
                format!("{command} {}", args.join(" "))
            }
        }
        McpTransportConfig::Http { url, .. } => url.clone(),
    }
}
fn mask_opt(value: Option<&str>) -> String {
    match value {
        Some(raw) if !raw.trim().is_empty() => mask_secret(raw),
        _ => "-".to_string(),
    }
}
fn run_list(ctx: &WorkspaceContext, out: &Output, scope: ListScopeArg) -> Result<(), String> {
    let servers = effective_servers_for(ctx)?;
    let (mut rows, mut values) = (Vec::new(), Vec::new());
    for server in &servers {
        if scope == ListScopeArg::User && server.scope != McpScope::User {
            continue;
        }
        if scope == ListScopeArg::Project && server.scope != McpScope::Project {
            continue;
        }
        rows.push(vec![
            server.name.clone(),
            server.scope.as_str().into(),
            transport_label(&server.config).into(),
            if server.config.enabled { "yes" } else { "no" }.into(),
            target_label(&server.config),
            if server.shadowed_user { "yes" } else { "-" }.into(),
        ]);
        values.push(json!({"name": server.name, "scope": server.scope.as_str(), "transport": transport_label(&server.config), "enabled": server.config.enabled, "target": target_label(&server.config), "overridesUser": server.shadowed_user}));
    }
    out.emit(&json!({"servers": values}), |out, w| {
        if rows.is_empty() {
            return writeln!(w, "No MCP servers configured.");
        }
        out.print_table(
            w,
            &[
                "NAME",
                "SCOPE",
                "TRANSPORT",
                "ENABLED",
                "TARGET",
                "OVERRIDES",
            ],
            &rows,
        )
    })
}
fn run_show(
    ctx: &WorkspaceContext,
    out: &Output,
    name: &str,
    scope: Option<ScopeArg>,
) -> Result<(), String> {
    let server = fetch_server(ctx, name, scope)?;
    let mut config = serde_json::to_value(&server.config)
        .map_err(|error| format!("failed to serialize MCP server config: {error}"))?;
    mask_secrets(&mut config);
    let value = json!({"name": server.name, "scope": server.scope.as_str(), "config": config});
    out.emit(&value, |_, w| {
        let pretty = serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?;
        writeln!(w, "{pretty}")
    })
}
fn mask_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if matches!(key.as_str(), "bearer_token" | "client_secret") {
                    if let Some(raw) = item.as_str() {
                        *item = Value::String(mask_opt(Some(raw)));
                    }
                } else {
                    mask_secrets(item);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask_secrets),
        _ => {}
    }
}
fn save_server(
    ctx: &WorkspaceContext,
    out: &Output,
    scope: ScopeArg,
    name: &str,
    transport: McpTransportConfig,
    startup: Option<u64>,
    tool: Option<u64>,
) -> Result<(), String> {
    validate_server_name(name).map_err(|error| error.to_string())?;
    let path = write_path(ctx, scope)?;
    let mut file = load_file(&path)?;
    if scope == ScopeArg::Project {
        file.project_root = Some(ctx.workspace.clone());
    }
    let existing = file.mcp.servers.get(name).cloned();
    file.mcp.servers.insert(
        name.into(),
        McpServerConfig {
            enabled: existing
                .as_ref()
                .map(|server| server.enabled)
                .unwrap_or(true),
            transport,
            startup_timeout_sec: startup.or_else(|| {
                existing
                    .as_ref()
                    .and_then(|server| server.startup_timeout_sec)
            }),
            tool_timeout_sec: tool
                .or_else(|| existing.as_ref().and_then(|server| server.tool_timeout_sec)),
            enabled_tools: existing
                .as_ref()
                .and_then(|server| server.enabled_tools.clone()),
            disabled_tools: existing
                .as_ref()
                .map(|server| server.disabled_tools.clone())
                .unwrap_or_default(),
        },
    );
    save_mcp(&path, &file).map_err(|error| error.to_string())?;
    out.emit(
        &json!({"name": name, "scope": scope.as_str(), "path": path}),
        |_, w| {
            writeln!(
                w,
                "MCP server '{name}' saved in the {} scope.",
                scope.as_str()
            )
        },
    )
}
fn run_add_stdio(ctx: &WorkspaceContext, out: &Output, args: &AddStdioArgs) -> Result<(), String> {
    save_server(
        ctx,
        out,
        args.scope,
        &args.name,
        McpTransportConfig::Stdio {
            command: args.command.clone(),
            args: args.args.clone(),
            cwd: args.cwd.clone(),
            env: args.env.clone(),
            env_vars: args.env_vars.clone(),
        },
        args.startup_timeout,
        args.tool_timeout,
    )
}
fn run_add_http(ctx: &WorkspaceContext, out: &Output, args: &AddHttpArgs) -> Result<(), String> {
    save_server(
        ctx,
        out,
        args.scope,
        &args.name,
        McpTransportConfig::Http {
            url: args.url.trim().into(),
            auth: args.auth,
            bearer_token: args.bearer_token.clone(),
            bearer_token_env_var: args.bearer_env.clone(),
            http_headers: args.headers.clone(),
            env_http_headers: BTreeMap::new(),
            scopes: args.scopes.clone(),
            client_id: args.client_id.clone(),
            client_secret: args.client_secret.clone(),
        },
        args.startup_timeout,
        args.tool_timeout,
    )
}
fn run_remove(
    ctx: &WorkspaceContext,
    out: &Output,
    name: &str,
    scope: ScopeArg,
) -> Result<(), String> {
    let path = write_path(ctx, scope)?;
    let mut file = load_file(&path)?;
    if file.mcp.servers.remove(name).is_none() {
        return Err(format!(
            "MCP server '{name}' is not configured in the {} scope",
            scope.as_str()
        ));
    }
    save_mcp(&path, &file).map_err(|error| error.to_string())?;
    out.emit(
        &json!({"removed": name, "scope": scope.as_str()}),
        |_, w| {
            writeln!(
                w,
                "MCP server '{name}' removed from the {} scope.",
                scope.as_str()
            )
        },
    )
}
fn run_toggle(
    ctx: &WorkspaceContext,
    out: &Output,
    name: &str,
    scope: ScopeArg,
    enabled: bool,
) -> Result<(), String> {
    let path = write_path(ctx, scope)?;
    let mut file = load_file(&path)?;
    let server = file.mcp.servers.get_mut(name).ok_or_else(|| {
        format!(
            "MCP server '{name}' is not configured in the {} scope",
            scope.as_str()
        )
    })?;
    server.enabled = enabled;
    save_mcp(&path, &file).map_err(|error| error.to_string())?;
    out.emit(
        &json!({"name": name, "scope": scope.as_str(), "enabled": enabled}),
        |_, w| {
            writeln!(
                w,
                "MCP server '{name}' {}.",
                if enabled { "enabled" } else { "disabled" }
            )
        },
    )
}
fn run_probe(
    ctx: &WorkspaceContext,
    out: &Output,
    name: &str,
    scope: Option<ScopeArg>,
) -> Result<(), String> {
    if scope == Some(ScopeArg::Project) {
        require_trusted(ctx)?;
    }
    let server = fetch_server(ctx, name, scope)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize async runtime: {error}"))?;
    let tools = runtime.block_on(async {
        let result = crate::mcp::manager::manager()
            .probe(&server, &ctx.workspace)
            .await;
        crate::mcp::manager::manager().shutdown().await;
        result
    })?;
    out.emit(
        &json!({"name": name, "scope": server.scope.as_str(), "tools": tools}),
        |_, w| {
            writeln!(
                w,
                "MCP server '{name}' is reachable ({} tool(s)).",
                tools.len()
            )?;
            for tool in &tools {
                writeln!(w, "  {tool}")?;
            }
            Ok(())
        },
    )
}
fn run_login(ctx: &WorkspaceContext, out: &Output, args: &LoginArgs) -> Result<(), String> {
    if args.scope == Some(ScopeArg::Project) {
        require_trusted(ctx)?;
    }
    let server = fetch_server(ctx, &args.name, args.scope)?;
    let McpTransportConfig::Http { auth, .. } = &server.config.transport else {
        return Err(format!("MCP server '{}' is not an HTTP server", args.name));
    };
    if !matches!(auth, McpHttpAuth::Oauth) {
        return Err(format!(
            "MCP server '{}' does not use OAuth authentication",
            args.name
        ));
    }
    let store = crate::mcp::oauth::global_store()
        .ok_or_else(|| "cannot resolve the MCP credentials path".to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize async runtime: {error}"))?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|error| format!("failed to bind OAuth callback listener: {error}"))?;
        let port = listener.local_addr().map_err(|error| format!("failed to read OAuth callback port: {error}"))?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/mcp/oauth/callback");
        let pending = crate::mcp::oauth::begin_authorization(&server, &ctx.workspace, &redirect_uri).await?;
        out.emit(&json!({"name": args.name, "authorizeUrl": pending.authorize_url, "redirectUri": redirect_uri}), |_, w| {
            writeln!(w, "Authorize URL: {}", pending.authorize_url)?;
            writeln!(w, "Waiting for the OAuth callback on {redirect_uri} ...")
        })?;
        if !args.no_open { let _ = crate::platform::open_browser(&pending.authorize_url); }
        let code = wait_for_callback(&listener, &pending.state).await?;
        crate::mcp::oauth::complete_authorization(&pending, &code, store.as_ref()).await?;
        Ok::<(), String>(())
    })?;
    out.emit(
        &json!({"name": args.name, "status": "authorized"}),
        |_, w| writeln!(w, "OAuth credentials saved for MCP server '{}'.", args.name),
    )
}
fn run_logout(
    ctx: &WorkspaceContext,
    out: &Output,
    name: &str,
    scope: Option<ScopeArg>,
) -> Result<(), String> {
    let server = fetch_server(ctx, name, scope)?;
    let store = crate::mcp::oauth::global_store()
        .ok_or_else(|| "cannot resolve the MCP credentials path".to_string())?;
    store.remove(&crate::mcp::oauth::credential_key(&server, &ctx.workspace))?;
    out.emit(
        &json!({"name": name, "scope": server.scope.as_str(), "loggedOut": true}),
        |_, w| writeln!(w, "OAuth credentials removed for MCP server '{name}'."),
    )
}
async fn wait_for_callback(
    listener: &tokio::net::TcpListener,
    expected_state: &str,
) -> Result<String, String> {
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .map_err(|_| "OAuth callback timed out after 300 seconds".to_string())?
        .map_err(|error| format!("failed to accept OAuth callback: {error}"))?;
    let mut buffer = vec![0u8; 8192];
    let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer))
        .await
        .map_err(|_| "timed out reading the OAuth callback".to_string())?
        .map_err(|error| format!("failed to read the OAuth callback: {error}"))?;
    let result = parse_callback(&String::from_utf8_lossy(&buffer[..read]), expected_state);
    let (status, body) = if result.is_ok() {
        (
            "200 OK",
            "<!doctype html><html><body><h1>Authorization complete</h1><p>You can close this window and return to the terminal.</p></body></html>",
        )
    } else {
        (
            "400 Bad Request",
            "<!doctype html><html><body><h1>Authorization failed</h1><p>Return to the terminal for details.</p></body></html>",
        )
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    result
}
fn parse_callback(request: &str, expected_state: &str) -> Result<String, String> {
    let first = request
        .lines()
        .next()
        .ok_or_else(|| "empty OAuth callback request".to_string())?;
    let mut parts = first.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "malformed OAuth callback request line".to_string())?;
    let target = parts
        .next()
        .ok_or_else(|| "malformed OAuth callback request line".to_string())?;
    if method != "GET" {
        return Err("OAuth callback must use GET".to_string());
    }
    let (path, query) = target
        .split_once('?')
        .ok_or_else(|| "OAuth callback is missing query parameters".to_string())?;
    if path != "/mcp/oauth/callback" {
        return Err("unexpected OAuth callback path".to_string());
    }
    let (mut code, mut state, mut error, mut description) = (None, None, None, None);
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = percent_decode(value);
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            "error_description" => description = Some(value),
            _ => {}
        }
    }
    if let Some(error) = error {
        let detail = description.unwrap_or_default();
        return Err(if detail.is_empty() {
            format!("authorization was denied: {error}")
        } else {
            format!("authorization was denied: {error} ({detail})")
        });
    }
    if state.as_deref() != Some(expected_state) {
        return Err("OAuth callback state does not match the pending authorization".to_string());
    }
    code.filter(|value| !value.is_empty())
        .ok_or_else(|| "OAuth callback did not include an authorization code".to_string())
}
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let (high, low) = (
                    (bytes[i + 1] as char).to_digit(16),
                    (bytes[i + 2] as char).to_digit(16),
                );
                if let (Some(high), Some(low)) = (high, low) {
                    out.push((high * 16 + low) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
