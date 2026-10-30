use super::*;

#[test]
fn credential_file_round_trips_with_revision() {
    let dir = std::env::temp_dir().join(format!("lx-oauth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("credentials.json");
    let store = store_at(path.clone());
    let credentials = Credentials {
        client_id: "client".into(),
        client_secret: None,
        access_token: "token".into(),
        refresh_token: Some("refresh".into()),
        expires_at_ms: 10,
        scopes: vec!["read".into()],
        issuer: None,
        token_endpoint: Some("https://auth.example/token".into()),
    };
    store
        .set("user:linear:https://mcp", credentials.clone())
        .unwrap();
    assert_eq!(store.revision(), 1);
    let reloaded = read_file(&path).expect("file");
    assert_eq!(reloaded.servers["user:linear:https://mcp"], credentials);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    store.remove("user:linear:https://mcp").unwrap();
    assert_eq!(store.revision(), 2);
    assert!(store.get("user:linear:https://mcp").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn www_authenticate_parses_quoted_resource_metadata() {
    let base = reqwest::Url::parse("https://mcp.example/api").expect("url");
    let url = www_authenticate_resource_metadata(
        "Bearer resource_metadata=\"https://auth.example/meta\"",
        &base,
    )
    .expect("url");
    assert_eq!(url, "https://auth.example/meta");
}
