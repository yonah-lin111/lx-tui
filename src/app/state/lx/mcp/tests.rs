use std::collections::BTreeSet;

use super::*;
use crate::ui::text;

fn sample_catalog() -> Vec<McpCatalogEntry> {
    vec![
        McpCatalogEntry {
            name: "alpha".to_string(),
            summary: "stdio: npx alpha".to_string(),
            config_enabled: true,
            runtime: McpRuntimeState::Ready { tools: 1 },
        },
        McpCatalogEntry {
            name: "beta".to_string(),
            summary: "https://example.com/mcp".to_string(),
            config_enabled: true,
            runtime: McpRuntimeState::Ready { tools: 4 },
        },
        McpCatalogEntry {
            name: "gamma".to_string(),
            summary: "stdio: gamma-bin".to_string(),
            config_enabled: true,
            runtime: McpRuntimeState::Connecting,
        },
        McpCatalogEntry {
            name: "delta".to_string(),
            summary: "stdio: delta-bin".to_string(),
            config_enabled: true,
            runtime: McpRuntimeState::Failed {
                error: "\n  \x1b[31mconnection refused\x1b[0m\nextra trace line".to_string(),
            },
        },
        McpCatalogEntry {
            name: "epsilon".to_string(),
            summary: "https://auth.example.com/mcp".to_string(),
            config_enabled: true,
            runtime: McpRuntimeState::NeedsLogin,
        },
        McpCatalogEntry {
            name: "zeta".to_string(),
            summary: "stdio: zeta-bin".to_string(),
            config_enabled: false,
            runtime: McpRuntimeState::Connecting,
        },
    ]
}

#[test]
fn catalog_to_panel_entry_maps_all_six_states_and_tool_pluralization() {
    let catalog = sample_catalog();
    let mut disabled = BTreeSet::new();
    disabled.insert("beta".to_string());

    let entries: Vec<McpPanelEntry> = catalog
        .iter()
        .map(|entry| McpPanelEntry::from_catalog(entry, &disabled))
        .collect();

    assert_eq!(entries[0].state, McpServerState::Connected { tools: 1 });
    assert_eq!(entries[0].preview, "1 tool · stdio: npx alpha");
    assert_eq!(entries[0].status_tag(), text::LX_MCP_STATUS_CONNECTED);
    assert!(!entries[0].is_settings_disabled());

    assert_eq!(entries[1].state, McpServerState::DisabledInPane);
    assert_eq!(entries[1].preview, "https://example.com/mcp");
    assert_eq!(entries[1].status_tag(), text::LX_MCP_STATUS_DISABLED);
    assert!(!entries[1].is_settings_disabled());

    // When beta is not disabled in pane, plural tool count is shown.
    let beta_enabled = McpPanelEntry::from_catalog(&catalog[1], &BTreeSet::new());
    assert_eq!(beta_enabled.state, McpServerState::Connected { tools: 4 });
    assert_eq!(beta_enabled.preview, "4 tools · https://example.com/mcp");

    assert_eq!(entries[2].state, McpServerState::Connecting);
    assert_eq!(entries[2].preview, "stdio: gamma-bin");
    assert_eq!(entries[2].status_tag(), text::LX_MCP_STATUS_CONNECTING);

    assert_eq!(
        entries[3].state,
        McpServerState::Failed {
            error: "connection refused".to_string()
        }
    );
    assert_eq!(entries[3].preview, "connection refused");
    assert_eq!(entries[3].status_tag(), text::LX_MCP_STATUS_FAILED);

    assert_eq!(entries[4].state, McpServerState::NeedsLogin);
    assert_eq!(entries[4].preview, "https://auth.example.com/mcp");
    assert_eq!(entries[4].status_tag(), text::LX_MCP_STATUS_NEEDS_LOGIN);

    assert_eq!(entries[5].state, McpServerState::DisabledInSettings);
    assert_eq!(entries[5].preview, "stdio: zeta-bin");
    assert_eq!(
        entries[5].status_tag(),
        text::LX_MCP_STATUS_DISABLED_SETTINGS
    );
    assert!(entries[5].is_settings_disabled());
}

#[test]
fn mcp_panel_refilter_and_sync_entries_preserve_highlighted_server() {
    let catalog = sample_catalog();
    let entries: Vec<McpPanelEntry> = catalog
        .iter()
        .map(|entry| McpPanelEntry::from_catalog(entry, &BTreeSet::new()))
        .collect();
    let mut panel = McpPanel::new(entries);
    panel.window.set_active(3);
    assert_eq!(panel.active_entry().map(|e| e.name.as_str()), Some("delta"));

    // Syncing updated entries preserves the active server ("delta").
    let mut disabled = BTreeSet::new();
    disabled.insert("delta".to_string());
    let updated: Vec<McpPanelEntry> = catalog
        .iter()
        .map(|entry| McpPanelEntry::from_catalog(entry, &disabled))
        .collect();
    panel.sync_entries(updated);
    let active = panel.active_entry().expect("active entry");
    assert_eq!(active.name, "delta");
    assert_eq!(active.state, McpServerState::DisabledInPane);

    // Filtering narrows visible entries and detects empty matches.
    panel.refilter("eps");
    assert_eq!(panel.visible().len(), 1);
    assert_eq!(
        panel.active_entry().map(|e| e.name.as_str()),
        Some("epsilon")
    );
    assert!(!panel.is_filter_empty());

    panel.refilter("no-such-mcp");
    assert!(panel.visible().is_empty());
    assert!(panel.is_filter_empty());

    // When active item disappears under current filter during sync_entries, window resets cleanly to 0.
    panel.refilter("stdio");
    panel.window.set_active(2);
    panel.sync_entries(vec![entries_for_sync_test()]);
    assert_eq!(panel.window.active, 0);
}

fn entries_for_sync_test() -> McpPanelEntry {
    McpPanelEntry {
        name: "alpha".to_string(),
        summary: "stdio: npx alpha".to_string(),
        preview: "1 tool · stdio: npx alpha".to_string(),
        state: McpServerState::Connected { tools: 1 },
    }
}
