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
