//! `mcp add-stdio` / `add-http`：新增或更新 server，省略的可选项保留旧值。

use super::*;

pub(super) fn run_add_stdio(
    ctx: &WorkspaceContext,
    out: &Output,
    args: &AddStdioArgs,
) -> Result<(), String> {
    save_server(
        ctx,
        out,
        args.scope,
        &args.name,
        |existing| build_stdio(args, existing),
        args.startup_timeout,
        args.tool_timeout,
    )
}

pub(super) fn run_add_http(
    ctx: &WorkspaceContext,
    out: &Output,
    args: &AddHttpArgs,
) -> Result<(), String> {
    save_server(
        ctx,
        out,
        args.scope,
        &args.name,
        |existing| build_http(args, existing),
        args.startup_timeout,
        args.tool_timeout,
    )
}

fn save_server(
    ctx: &WorkspaceContext,
    out: &Output,
    scope: ScopeArg,
    name: &str,
    build: impl FnOnce(Option<&McpServerConfig>) -> McpTransportConfig,
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
    let transport = build(existing.as_ref());
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

fn build_stdio(args: &AddStdioArgs, existing: Option<&McpServerConfig>) -> McpTransportConfig {
    let previous = existing.and_then(|server| match &server.transport {
        McpTransportConfig::Stdio { args, cwd, env, .. } => Some((args, cwd, env)),
        McpTransportConfig::Http { .. } => None,
    });
    McpTransportConfig::Stdio {
        command: args.command.clone(),
        args: if args.args.is_empty() {
            previous
                .map(|previous| previous.0.clone())
                .unwrap_or_default()
        } else {
            args.args.clone()
        },
        cwd: args
            .cwd
            .clone()
            .or_else(|| previous.and_then(|previous| previous.1.clone())),
        env: if args.env.is_empty() {
            previous
                .map(|previous| previous.2.clone())
                .unwrap_or_default()
        } else {
            args.env.clone()
        },
        env_vars: args.env_vars.clone(),
    }
}

fn build_http(args: &AddHttpArgs, existing: Option<&McpServerConfig>) -> McpTransportConfig {
    let (
        mut auth,
        mut bearer_token,
        mut bearer_token_env_var,
        mut http_headers,
        mut env_http_headers,
        mut scopes,
        mut client_id,
        mut client_secret,
    ) = match existing.and_then(|server| match &server.transport {
        McpTransportConfig::Http {
            auth,
            bearer_token,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            scopes,
            client_id,
            client_secret,
            ..
        } => Some((
            *auth,
            bearer_token.clone(),
            bearer_token_env_var.clone(),
            http_headers.clone(),
            env_http_headers.clone(),
            scopes.clone(),
            client_id.clone(),
            client_secret.clone(),
        )),
        McpTransportConfig::Stdio { .. } => None,
    }) {
        Some(values) => values,
        None => (
            McpHttpAuth::None,
            None,
            None,
            BTreeMap::new(),
            BTreeMap::new(),
            Vec::new(),
            None,
            None,
        ),
    };
    if let Some(value) = args.auth {
        auth = value;
    }
    if !args.headers.is_empty() {
        http_headers = args.headers.clone();
    }
    if !args.env_http_headers.is_empty() {
        env_http_headers = args.env_http_headers.iter().cloned().collect();
    }
    if !args.scopes.is_empty() {
        scopes = args.scopes.clone();
    }
    bearer_token = match &args.bearer_token {
        Some(value) => Some(value.clone()),
        None if auth == McpHttpAuth::Bearer => bearer_token.clone(),
        None => None,
    };
    bearer_token_env_var = match &args.bearer_env {
        Some(value) => Some(value.clone()),
        None if auth == McpHttpAuth::Bearer => bearer_token_env_var.clone(),
        None => None,
    };
    client_id = match &args.client_id {
        Some(value) => Some(value.clone()),
        None if auth == McpHttpAuth::Oauth => client_id.clone(),
        None => None,
    };
    client_secret = match &args.client_secret {
        Some(value) => Some(value.clone()),
        None if auth == McpHttpAuth::Oauth => client_secret.clone(),
        None => None,
    };
    McpTransportConfig::Http {
        url: args.url.trim().into(),
        auth,
        bearer_token,
        bearer_token_env_var,
        http_headers,
        env_http_headers,
        scopes,
        client_id,
        client_secret,
    }
}
