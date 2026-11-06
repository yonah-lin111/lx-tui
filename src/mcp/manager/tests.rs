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

#[test]
fn snapshot_without_servers_filters_tools_prompts_and_resources() {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    let snapshot = Arc::new(McpToolSnapshot {
        project: Some(PathBuf::from("/repo")),
        tools: vec![
            McpToolEntry {
                qualified: "mcp__github__list_issues".into(),
                server: "github".into(),
                raw_name: "list_issues".into(),
                description: "List issues".into(),
                parameters: serde_json::json!({"type": "object"}),
                read_only_hint: true,
            },
            McpToolEntry {
                qualified: "mcp__codegraph__explore".into(),
                server: "codegraph".into(),
                raw_name: "explore".into(),
                description: "Explore graph".into(),
                parameters: serde_json::json!({"type": "object"}),
                read_only_hint: true,
            },
        ],
        servers: vec![
            ServerSnapshot {
                name: "github".into(),
                instructions: Some("use github issues".into()),
                has_resources: true,
            },
            ServerSnapshot {
                name: "codegraph".into(),
                instructions: Some("use codegraph explore".into()),
                has_resources: false,
            },
        ],
    });

    // Empty disabled set returns the exact same Arc without cloning inner vectors.
    let unchanged = snapshot.without_servers(&BTreeSet::new());
    assert!(Arc::ptr_eq(&snapshot, &unchanged));
    assert!(unchanged.has_resources());

    // Disabling "github" strips its tools, instructions, and resources capability.
    let mut disabled = BTreeSet::new();
    disabled.insert("github".to_string());
    let filtered = snapshot.without_servers(&disabled);
    assert_eq!(filtered.tools().len(), 1);
    assert_eq!(filtered.tools()[0].server, "codegraph");
    assert!(filtered.tool("mcp__github__list_issues").is_none());
    assert!(filtered.tool("mcp__codegraph__explore").is_some());
    assert!(filtered.server("github").is_none());
    assert!(filtered.server("codegraph").is_some());
    assert!(!filtered.has_resources());

    let prompt = filtered.instructions_prompt().expect("prompt");
    assert!(!prompt.contains("### github"));
    assert!(prompt.contains("### codegraph"));
}
