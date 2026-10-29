use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-mcp-config-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn stdio_server() -> McpServerConfig {
    McpServerConfig {
        enabled: true,
        transport: McpTransportConfig::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "server".into()],
            cwd: None,
            env: BTreeMap::new(),
            env_vars: Vec::new(),
        },
        startup_timeout_sec: None,
        tool_timeout_sec: None,
        enabled_tools: None,
        disabled_tools: Vec::new(),
    }
}

#[test]
fn parse_and_serialize_round_trip() {
    let text = r#"{
      "mcp": {
        "servers": {
          "github": {
            "enabled": false,
            "transport": "stdio",
            "command": "npx",
            "args": ["-y", "server-github"],
            "env": {"LOG_LEVEL": "warn"},
            "env_vars": ["GITHUB_TOKEN"],
            "tool_timeout_sec": 60,
            "enabled_tools": ["search"],
            "disabled_tools": ["delete"]
          },
          "linear": {
            "transport": "http",
            "url": "https://mcp.linear.app/mcp",
            "auth": "oauth",
            "scopes": ["read"]
          }
        }
      }
    }"#;
    let file = parse_mcp(text).expect("parse");
    assert_eq!(file.mcp.servers.len(), 2);
    let github = &file.mcp.servers["github"];
    assert!(!github.enabled);
    assert_eq!(github.tool_timeout_sec, Some(60));
    let temp = temp_dir("roundtrip").join("mcp.json");
    save_mcp(&temp, &file).expect("save");
    let loaded = load_mcp(&temp).expect("load").expect("present");
    assert!(matches!(
        loaded.mcp.servers["linear"].transport,
        McpTransportConfig::Http {
            auth: McpHttpAuth::Oauth,
            ..
        }
    ));
    let _ = std::fs::remove_dir_all(temp.parent().expect("parent"));
}

#[test]
fn server_name_rules() {
    assert!(validate_server_name("github").is_ok());
    assert!(validate_server_name("my-kb_1").is_ok());
    assert!(validate_server_name("").is_err());
    assert!(validate_server_name("a__b").is_err());
    assert!(validate_server_name(&"x".repeat(33)).is_err());
    assert!(validate_server_name("bad name").is_err());
}

#[test]
fn stdio_command_must_not_be_empty() {
    let mut server = stdio_server();
    if let McpTransportConfig::Stdio { command, .. } = &mut server.transport {
        command.clear();
    }
    assert_eq!(
        server.validate("s").unwrap_err().to_string(),
        "invalid mcp config: command must not be empty"
    );
}

#[test]
fn http_url_requires_tls_except_localhost() {
    let mut server = stdio_server();
    server.transport = McpTransportConfig::Http {
        url: "http://example.com/mcp".into(),
        auth: McpHttpAuth::None,
        bearer_token: None,
        bearer_token_env_var: None,
        http_headers: BTreeMap::new(),
        env_http_headers: BTreeMap::new(),
        scopes: Vec::new(),
        client_id: None,
        client_secret: None,
    };
    assert!(server.validate("s").is_err());
    server.transport = McpTransportConfig::Http {
        url: "http://localhost:3000/mcp".into(),
        auth: McpHttpAuth::None,
        bearer_token: None,
        bearer_token_env_var: None,
        http_headers: BTreeMap::new(),
        env_http_headers: BTreeMap::new(),
        scopes: Vec::new(),
        client_id: None,
        client_secret: None,
    };
    assert!(server.validate("s").is_ok());
}

#[test]
fn bearer_needs_exactly_one_secret_source() {
    let mut server = stdio_server();
    server.transport = McpTransportConfig::Http {
        url: "https://example.com/mcp".into(),
        auth: McpHttpAuth::Bearer,
        bearer_token: None,
        bearer_token_env_var: None,
        http_headers: BTreeMap::new(),
        env_http_headers: BTreeMap::new(),
        scopes: Vec::new(),
        client_id: None,
        client_secret: None,
    };
    assert!(server.validate("s").is_err());
    if let McpTransportConfig::Http {
        bearer_token,
        bearer_token_env_var,
        ..
    } = &mut server.transport
    {
        *bearer_token = Some("token".into());
        *bearer_token_env_var = Some("TOKEN".into());
    }
    assert!(server.validate("s").is_err());
    if let McpTransportConfig::Http {
        bearer_token_env_var,
        ..
    } = &mut server.transport
    {
        *bearer_token_env_var = None;
    }
    assert!(server.validate("s").is_ok());
}

#[test]
fn headers_reject_manual_authorization_and_bad_env_names() {
    let mut server = stdio_server();
    server.transport = McpTransportConfig::Http {
        url: "https://example.com/mcp".into(),
        auth: McpHttpAuth::None,
        bearer_token: None,
        bearer_token_env_var: None,
        http_headers: BTreeMap::from([("Authorization".into(), "x".into())]),
        env_http_headers: BTreeMap::new(),
        scopes: Vec::new(),
        client_id: None,
        client_secret: None,
    };
    assert!(server.validate("s").is_err());
    if let McpTransportConfig::Http {
        http_headers,
        env_http_headers,
        ..
    } = &mut server.transport
    {
        http_headers.clear();
        env_http_headers.insert("X-Trace".into(), "1BAD".into());
    }
    assert!(server.validate("s").is_err());
}

#[test]
fn timeouts_are_bounded() {
    let mut server = stdio_server();
    server.startup_timeout_sec = Some(0);
    assert!(server.validate("s").is_err());
    server.startup_timeout_sec = Some(MAX_TIMEOUT_SEC + 1);
    assert!(server.validate("s").is_err());
    server.startup_timeout_sec = Some(MAX_TIMEOUT_SEC);
    assert!(server.validate("s").is_ok());
}

#[test]
fn enabled_and_disabled_tools_must_not_overlap() {
    let mut server = stdio_server();
    server.enabled_tools = Some(vec!["a".into(), "b".into()]);
    server.disabled_tools = vec!["b".into()];
    assert!(server.validate("s").is_err());
}

#[test]
fn effective_servers_project_overrides_user_and_respects_trust() {
    let user = McpConfigFile {
        project_root: None,
        mcp: McpSection {
            servers: BTreeMap::from([
                ("github".into(), stdio_server()),
                ("linear".into(), stdio_server()),
            ]),
        },
    };
    let project = McpConfigFile {
        project_root: Some(PathBuf::from("/repo")),
        mcp: McpSection {
            servers: BTreeMap::from([("github".into(), stdio_server())]),
        },
    };
    let merged = effective_servers(Some(&user), Some(&project), true, Some(Path::new("/repo")));
    assert_eq!(merged.len(), 2);
    let github = merged.iter().find(|s| s.name == "github").expect("github");
    assert_eq!(github.scope, McpScope::Project);
    assert!(github.shadowed_user);

    let untrusted = effective_servers(Some(&user), Some(&project), false, Some(Path::new("/repo")));
    assert!(untrusted.iter().all(|s| s.scope == McpScope::User));
    let wrong_root =
        effective_servers(Some(&user), Some(&project), true, Some(Path::new("/other")));
    assert!(wrong_root.iter().all(|s| s.scope == McpScope::User));
}

#[test]
fn project_scope_path_is_stable() {
    let path = project_mcp_path(Path::new("/Users/yonah/projects/agent/lx-tui")).expect("path");
    let text = path.to_string_lossy();
    assert!(text.contains("config/mcp/projects/lx-tui-"));
    assert!(text.ends_with(".json"));
}
