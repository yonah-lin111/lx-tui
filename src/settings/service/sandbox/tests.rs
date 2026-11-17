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

#[tokio::test]
async fn corrupted_sandbox_json_falls_back_with_warning_and_put_recovers() {
    let (service, _events, dir) = start("sandbox-corrupt").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/sandbox?token={token}");
    let origin = format!("http://127.0.0.1:{port}");
    let path = dir.join("root").join("config").join("sandbox.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, "{not valid json").expect("write corrupt");

    let initial = request(port, &get(&url)).await;
    assert_eq!(status(&initial), 200, "{initial}");
    let initial_body = body(&initial);
    assert!(
        initial_body.contains("\"workspace-write\""),
        "{initial_body}"
    );
    assert!(
        initial_body.contains("\"warning\":"),
        "损坏配置回落默认值并附带 warning: {initial_body}"
    );

    // 校验错误（如相对路径）同样返回 200 默认值 + warning，不阻塞页面。
    std::fs::write(
        &path,
        r#"{"mode":"read-only","network_access":false,"escalation":"ask","writable_roots":["rel"]}"#,
    )
    .expect("write invalid");
    let invalid = request(port, &get(&url)).await;
    assert_eq!(status(&invalid), 200, "{invalid}");
    assert!(body(&invalid).contains("\"warning\":"), "{invalid}");
    assert!(body(&invalid).contains("\"workspace-write\""), "{invalid}");

    // 用户仍可通过 PUT 覆写修复损坏配置。
    let repaired = request(
        port,
        &write(
            "PUT",
            &url,
            r#"{"mode":"read-only","network_access":false,"escalation":"ask","writable_roots":[]}"#,
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&repaired), 200, "{repaired}");
    assert!(body(&repaired).contains("\"read-only\""));
    assert!(!body(&repaired).contains("\"warning\":"));
    service.shutdown();
}

#[tokio::test]
async fn sandbox_api_uses_isolated_user_root_and_returns_500_on_io_errors() {
    let (service, _events, dir) = start("sandbox-user-root-io").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/sandbox?token={token}");
    let origin = format!("http://127.0.0.1:{port}");

    let rules_dir = dir.join("root").join("rules");
    std::fs::create_dir_all(&rules_dir).expect("rules dir");
    std::fs::write(
        rules_dir.join("demo.rules"),
        "prefix_rule(pattern = [\"echo\"], decision = \"allow\")\n",
    )
    .expect("write rule");

    let response = request(port, &get(&url)).await;
    assert_eq!(status(&response), 200, "{response}");
    let parsed: serde_json::Value = serde_json::from_str(body(&response)).expect("json");
    let expected_config = dir.join("root").join("config").join("sandbox.json");
    assert_eq!(
        parsed["path"].as_str(),
        Some(expected_config.to_string_lossy().as_ref())
    );
    assert_eq!(
        parsed["rules"]["directory"].as_str(),
        Some(rules_dir.to_string_lossy().as_ref())
    );
    assert_eq!(parsed["rules"]["total_files"].as_u64(), Some(1));
    assert_eq!(parsed["rules"]["loaded_files"].as_u64(), Some(1));

    // 将 sandbox.json 路径占为非空目录，同时触发 GET 读取与 PUT 重命名的 IO 错误（必须返回 500 而非 422）。
    std::fs::create_dir_all(expected_config.join("blocker")).expect("dir at config path");
    let get_io_err = request(port, &get(&url)).await;
    assert_eq!(status(&get_io_err), 500, "{get_io_err}");

    let put_io_err = request(
        port,
        &write(
            "PUT",
            &url,
            r#"{"mode":"read-only","network_access":false,"escalation":"ask","writable_roots":[]}"#,
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&put_io_err), 500, "{put_io_err}");
    service.shutdown();
}
