//! PTY 会话单元测试：启动目录与回退。

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-pty-tests-{}-{}-{}",
        name,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// 启动会话、写入命令并收集输出，直到出现期望文本或超时。
fn run_command(
    session: &mut PtySession,
    receiver: &mpsc::Receiver<PtyEvent>,
    command: &str,
    needle: &str,
) -> String {
    session.write(command.as_bytes()).expect("pty write");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = String::new();
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(PtyEvent::Output(bytes)) => {
                output.push_str(&String::from_utf8_lossy(&bytes));
                if output.contains(needle) {
                    break;
                }
            }
            Ok(PtyEvent::Exited) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    output
}

#[test]
fn spawn_starts_shell_in_requested_cwd() {
    let dir = temp_dir("cwd");
    std::fs::write(dir.join("lx-tui-marker"), "lx-tui-marker-ok").expect("marker file");
    let (sender, receiver) = mpsc::channel();
    let mut session = PtySession::spawn(PaneId::alloc(), 80, 24, Some(dir.clone()), move |event| {
        let _ = sender.send(event);
    })
    .expect("pty spawns");
    let output = run_command(
        &mut session,
        &receiver,
        "cat lx-tui-marker\n",
        "lx-tui-marker-ok",
    );
    session.kill();
    std::fs::remove_dir_all(&dir).expect("cleanup");
    assert!(
        output.contains("lx-tui-marker-ok"),
        "shell did not start in requested cwd: {output:?}"
    );
}

#[test]
fn spawn_falls_back_to_process_cwd_when_none() {
    let (sender, receiver) = mpsc::channel();
    let mut session = PtySession::spawn(PaneId::alloc(), 80, 24, None, move |event| {
        let _ = sender.send(event);
    })
    .expect("pty spawns");
    let output = run_command(&mut session, &receiver, "cat Cargo.toml\n", "[package]");
    session.kill();
    assert!(
        output.contains("[package]"),
        "shell did not fall back to process cwd: {output:?}"
    );
}
