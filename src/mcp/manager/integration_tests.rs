//! 真实 stdio MCP server 的集成测试：懒启动、工具发现、调用、取消、超时与退出清理。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::config::mcp::McpScope;

fn python_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mcp_echo.py")
}

fn server(name: &str, extra_env: BTreeMap<String, String>, startup_sec: u64) -> EffectiveServer {
    EffectiveServer {
        name: name.into(),
        scope: McpScope::User,
        config: McpServerConfig {
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: "python3".into(),
                args: vec![fixture().display().to_string()],
                cwd: None,
                env: extra_env,
                env_vars: Vec::new(),
            },
            startup_timeout_sec: Some(startup_sec),
            tool_timeout_sec: Some(30),
            enabled_tools: None,
            disabled_tools: Vec::new(),
        },
        shadowed_user: false,
    }
}

fn temp_pid_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "lx-mcp-test-{}-{}-{}.pid",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ))
}

#[tokio::test]
async fn stdio_server_lists_and_calls_tools_then_shuts_down() {
    if !python_available() {
        return;
    }
    let pid_file = temp_pid_file("echo");
    let manager = Arc::new(McpManager::new());
    let project = std::env::temp_dir();
    let (tx, rx) = watch::channel(false);
    let snapshot = manager
        .snapshot_for(&project, &[server("echo", BTreeMap::new(), 10)], rx)
        .await;
    assert!(
        snapshot.tool("mcp__echo__echo").is_some(),
        "tools: {:?}",
        snapshot.tools()
    );
    let outcome = manager
        .call_tool(
            &snapshot,
            "sess-test",
            "mcp__echo__echo",
            json!({"text": "hello"}),
            watch::channel(false).1,
        )
        .await;
    match outcome {
        McpCallOutcome::Success { text, .. } => assert_eq!(text, "echo:hello"),
        other => panic!("unexpected {other:?}"),
    }
    let _ = pid_file;
    manager.shutdown().await;
    assert!(manager.statuses(&project).await.is_empty());
    drop(tx);
}

#[tokio::test]
async fn startup_timeout_marks_failed_and_does_not_retry() {
    let manager = McpManager::new();
    let project = std::env::temp_dir();
    let mut broken = server("broken", BTreeMap::new(), 1);
    broken.config.transport = McpTransportConfig::Stdio {
        command: "sleep".into(),
        args: vec!["10".into()],
        cwd: None,
        env: BTreeMap::new(),
        env_vars: Vec::new(),
    };
    let started = std::time::Instant::now();
    let _ = manager
        .snapshot_for(&project, &[broken.clone()], watch::channel(false).1)
        .await;
    assert!(started.elapsed() < Duration::from_secs(3));
    let statuses = manager.statuses(&project).await;
    assert!(
        matches!(statuses[0].1, ServerStatus::Failed { .. }),
        "{statuses:?}"
    );
    // 第二次 snapshot 不重新启动（Failed 不自动重试）：必须立即返回。
    let again = std::time::Instant::now();
    let _ = manager
        .snapshot_for(&project, &[broken], watch::channel(false).1)
        .await;
    assert!(again.elapsed() < Duration::from_millis(500));
    manager.shutdown().await;
}

#[tokio::test]
async fn cancelled_call_returns_without_waiting_for_server() {
    if !python_available() {
        return;
    }
    let manager = Arc::new(McpManager::new());
    let project = std::env::temp_dir();
    let snapshot = Arc::new(
        manager
            .snapshot_for(
                &project,
                &[server("echo", BTreeMap::new(), 10)],
                watch::channel(false).1,
            )
            .await,
    );
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let call_manager = manager.clone();
    let call_snapshot = snapshot.clone();
    let task = tokio::spawn(async move {
        call_manager
            .call_tool(
                &call_snapshot,
                "sess-test",
                "mcp__echo__sleep",
                json!({"seconds": 10}),
                cancel_rx,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel_tx.send(true).expect("cancel");
    let outcome = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("cancel must not wait for the server")
        .expect("join");
    assert_eq!(outcome, McpCallOutcome::Cancelled);
    manager.shutdown().await;
}

#[tokio::test]
async fn shutdown_terminates_the_stdio_process_group() {
    if !python_available() {
        return;
    }
    let pid_file = temp_pid_file("shutdown");
    let mut env = BTreeMap::new();
    env.insert("MCP_PID_FILE".to_string(), pid_file.display().to_string());
    let manager = McpManager::new();
    let project = std::env::temp_dir();
    let _ = manager
        .snapshot_for(
            &project,
            &[server("echo", env.clone(), 10)],
            watch::channel(false).1,
        )
        .await;
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("pid file")
        .trim()
        .parse()
        .expect("pid");
    assert!(crate::platform::process_group_has_members(pid));
    manager.shutdown().await;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while crate::platform::process_group_has_members(pid) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !crate::platform::process_group_has_members(pid),
        "child process group must be gone"
    );
    let _ = std::fs::remove_file(pid_file);
}

#[tokio::test]
async fn fingerprint_change_restarts_only_the_changed_server() {
    if !python_available() {
        return;
    }
    let project = std::env::temp_dir();
    let pid_a = temp_pid_file("a");
    let pid_b = temp_pid_file("b");
    let mut env_a = BTreeMap::new();
    env_a.insert("MCP_PID_FILE".into(), pid_a.display().to_string());
    let mut env_b = BTreeMap::new();
    env_b.insert("MCP_PID_FILE".into(), pid_b.display().to_string());
    let server_a = server("a", env_a, 10);
    let mut server_b = server("b", env_b, 10);
    let manager = McpManager::new();
    let _ = manager
        .snapshot_for(
            &project,
            &[server_a.clone(), server_b.clone()],
            watch::channel(false).1,
        )
        .await;
    let first_a: u32 = std::fs::read_to_string(&pid_a)
        .expect("a pid")
        .trim()
        .parse()
        .expect("a");
    let first_b: u32 = std::fs::read_to_string(&pid_b)
        .expect("b pid")
        .trim()
        .parse()
        .expect("b");
    server_b.config.disabled_tools = vec!["sleep".into()];
    let _ = manager
        .snapshot_for(&project, &[server_a, server_b], watch::channel(false).1)
        .await;
    let second_a: u32 = std::fs::read_to_string(&pid_a)
        .expect("a pid")
        .trim()
        .parse()
        .expect("a");
    let second_b: u32 = std::fs::read_to_string(&pid_b)
        .expect("b pid")
        .trim()
        .parse()
        .expect("b");
    assert_eq!(first_a, second_a, "unchanged server keeps its connection");
    assert_ne!(first_b, second_b, "changed server is restarted");
    manager.shutdown().await;
    let _ = std::fs::remove_file(pid_a);
    let _ = std::fs::remove_file(pid_b);
}

#[tokio::test]
async fn resources_are_listed_and_read_from_the_fixture_server() {
    if !python_available() {
        return;
    }
    let manager = McpManager::new();
    let project = std::env::temp_dir();
    let snapshot = manager
        .snapshot_for(
            &project,
            &[server("echo", BTreeMap::new(), 10)],
            watch::channel(false).1,
        )
        .await;
    assert!(snapshot.has_resources());
    let listed = manager.list_resources(&snapshot, None, None).await;
    match listed {
        McpCallOutcome::Success { text, .. } => {
            assert!(text.contains("echo://doc"), "{text}");
        }
        other => panic!("unexpected {other:?}"),
    }
    let read = manager
        .read_resource(&snapshot, "sess-test", "echo", "echo://doc")
        .await;
    match read {
        McpCallOutcome::Success { text, .. } => {
            assert!(text.contains("resource-body"), "{text}");
        }
        other => panic!("unexpected {other:?}"),
    }
    manager.shutdown().await;
}

#[tokio::test]
async fn tool_list_changed_replaces_the_connection_tools_for_later_snapshots() {
    if !python_available() {
        return;
    }
    let project = std::env::temp_dir();
    let tools_file = temp_pid_file("tools").with_extension("json");
    std::fs::write(
        &tools_file,
        r#"[{"name":"ping","description":"Ping","inputSchema":{"type":"object","properties":{}}}]"#,
    )
    .expect("tools file");
    let mut env = BTreeMap::new();
    env.insert(
        "MCP_TOOLS_FILE".to_string(),
        tools_file.display().to_string(),
    );
    let manager = McpManager::new();
    let first = manager
        .snapshot_for(
            &project,
            &[server("echo", env.clone(), 10)],
            watch::channel(false).1,
        )
        .await;
    assert!(first.tool("mcp__echo__ping").is_some());
    assert!(first.tool("mcp__echo__echo").is_none());
    // 模拟 server 发送 notifications/tools/list_changed 后的回调。
    let key = (project.clone(), "echo".to_string());
    let connection = {
        let slots = manager.slots.lock().await;
        slots
            .get(&key)
            .and_then(|slot| slot.connection.clone())
            .expect("connection")
    };
    connection.tools_changed.store(true, Ordering::SeqCst);
    let second = manager
        .snapshot_for(
            &project,
            &[server("echo", env.clone(), 10)],
            watch::channel(false).1,
        )
        .await;
    assert!(second.tool("mcp__echo__ping").is_some());
    // 再次构建快照仍读新列表（不会被旧 connection.tools 覆盖）。
    let third = manager
        .snapshot_for(
            &project,
            &[server("echo", env.clone(), 10)],
            watch::channel(false).1,
        )
        .await;
    assert!(third.tool("mcp__echo__ping").is_some());
    manager.shutdown().await;
    let _ = std::fs::remove_file(tools_file);
}
