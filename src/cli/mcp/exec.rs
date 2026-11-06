//! `lx-tui mcp` 子命令执行、探测与 OAuth 授权。

use super::*;

mod add;
use add::{run_add_http, run_add_stdio};

pub fn run(cli: McpCli, linked_worktree: Option<&Path>) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(cli.global.json, cli.global.color);
    match cli.command {
        McpCommand::List { scope, disabled } => run_list(&ctx, &out, scope, &disabled)?,
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
    let db = ctx.open_db()?;
    Ok(ctx.is_trusted(&db))
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
    // 未信任时不读取项目文件：损坏的 `mcp.json` 不应阻断 `mcp list`。
    let project = if trusted {
        load_mcp(&project_path(&ctx.workspace)?).map_err(|error| error.to_string())?
    } else {
        Some(McpConfigFile::default())
    };
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
            ScopeArg::Project => {
                require_trusted(ctx)?;
                project_path(&ctx.workspace)?
            }
        };
        let file = load_file(&path)?;
        if scope == ScopeArg::Project
            && file
                .project_root
                .as_deref()
                .is_some_and(|root| root != ctx.workspace.as_path())
        {
            return Err(format!(
                "project MCP config '{}' does not belong to workspace '{}'",
                path.display(),
                ctx.workspace.display()
            ));
        }
        let config = file.mcp.servers.get(name).cloned().ok_or_else(|| {
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
fn run_list(
    ctx: &WorkspaceContext,
    out: &Output,
    scope: ListScopeArg,
    disabled: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let servers = if scope == ListScopeArg::User {
        // 显式查询用户作用域时直接读用户文件，避免被项目同名服务覆盖后消失。
        load_file(&user_path()?)?
            .mcp
            .servers
            .into_iter()
            .map(|(name, config)| EffectiveServer {
                name,
                scope: McpScope::User,
                config,
                shadowed_user: false,
            })
            .collect()
    } else {
        effective_servers_for(ctx)?
    };
    let (mut rows, mut values) = (Vec::new(), Vec::new());
    for server in &servers {
        if scope == ListScopeArg::Project && server.scope != McpScope::Project {
            continue;
        }
        let pane_disabled = disabled.contains(&server.name);
        let effective_enabled = server.config.enabled && !pane_disabled;
        let enabled_label = if !server.config.enabled {
            "no"
        } else if pane_disabled {
            "no (pane)"
        } else {
            "yes"
        };
        rows.push(vec![
            server.name.clone(),
            server.scope.as_str().into(),
            transport_label(&server.config).into(),
            enabled_label.into(),
            target_label(&server.config),
            if server.shadowed_user { "yes" } else { "-" }.into(),
        ]);
        values.push(json!({
            "name": server.name,
            "scope": server.scope.as_str(),
            "transport": transport_label(&server.config),
            "enabled": server.config.enabled,
            "paneDisabled": pane_disabled,
            "effectiveEnabled": effective_enabled,
            "target": target_label(&server.config),
            "overridesUser": server.shadowed_user
        }));
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
                } else if matches!(key.as_str(), "http_headers" | "env") {
                    if let Value::Object(entries) = item {
                        for entry in entries.values_mut() {
                            if let Some(raw) = entry.as_str() {
                                *entry = Value::String(mask_secret(raw));
                            }
                        }
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
    let (redirect_uri, authorize_url) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("failed to bind OAuth callback listener: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("failed to read OAuth callback port: {error}"))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/mcp/oauth/callback");
        let pending =
            crate::mcp::oauth::begin_authorization(&server, &ctx.workspace, &redirect_uri).await?;
        if !out.is_json() {
            eprintln!("Authorize URL: {}", pending.authorize_url);
            eprintln!("Waiting for the OAuth callback on {redirect_uri} ...");
        }
        if !args.no_open {
            let _ = crate::platform::open_browser(&pending.authorize_url);
        }
        let code = wait_for_callback(&listener, &pending.state).await?;
        crate::mcp::oauth::complete_authorization(&pending, &code, store.as_ref()).await?;
        Ok::<(String, String), String>((redirect_uri, pending.authorize_url))
    })?;
    out.emit(
        &json!({"name": args.name, "status": "authorized", "authorizeUrl": authorize_url, "redirectUri": redirect_uri}),
        |_, w| {
            writeln!(
                w,
                "OAuth credentials saved for MCP server '{}'.",
                args.name
            )
        },
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
