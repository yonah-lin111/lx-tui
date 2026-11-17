//! `/api/sandbox` 往返、校验与事件。

use super::super::SettingsEvent;
use super::super::tests::start;
use super::super::tests_helpers::{body, get, request, status, write};

#[tokio::test]
async fn sandbox_api_round_trips_and_validates_absolute_roots() {
    let (service, mut events, dir) = start("sandbox-api").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/sandbox?token={token}");
    let origin = format!("http://127.0.0.1:{port}");

    let initial = request(port, &get(&url)).await;
    assert_eq!(status(&initial), 200, "{initial}");
    assert!(body(&initial).contains("\"workspace-write\""));
    assert!(
        body(&initial).contains("\"rules\""),
        "rules status: {initial}"
    );
    assert!(body(&initial).contains("\"loaded_files\""));

    let invalid = request(
        port,
        &write(
            "PUT",
            &url,
            r#"{"mode":"read-only","network_access":false,"escalation":"ask","writable_roots":["relative"]}"#,
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&invalid), 422, "{invalid}");

    let saved = request(
        port,
        &write(
            "PUT",
            &url,
            r#"{"mode":"read-only","network_access":true,"escalation":"never","writable_roots":["/tmp/out"]}"#,
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    assert!(body(&saved).contains("\"read-only\""));
    match events.try_recv() {
        Ok(SettingsEvent::SandboxChanged(settings)) => {
            assert_eq!(settings.mode, crate::config::sandbox::SandboxMode::ReadOnly);
            assert!(settings.network_access);
            assert_eq!(
                settings.escalation,
                crate::config::sandbox::EscalationPolicy::Never
            );
        }
        other => panic!("expected SandboxChanged, got {other:?}"),
    }
    let path = dir.join("root").join("config").join("sandbox.json");
    assert!(path.exists(), "config written to {}", path.display());
}

#[test]
fn capabilities_name_the_platform_backend() {
    let support = crate::platform::sandbox::support();
    assert!(!support.as_str().is_empty());
}
