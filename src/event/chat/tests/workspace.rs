//! 回归：工具落库必须使用会话 DB 工作区键，而不是实际执行 cwd。
//!
//! linked worktree / 非工作区写入时，会话仍挂在共享的仓库主 checkout 键下；
//! 旧实现用执行 cwd 调 `begin_tool_call`，直接报 `workspace missing` 并中断本轮。

use super::*;
use tokio::sync::oneshot;

/// 迟到的 `StartChatStream` 不能跨会话启动：切换后旧 key 必须被丢弃。
#[tokio::test]
async fn stale_start_after_session_switch_is_dropped() {
    let fixture = Fixture::new("stale-start");
    let (mut state, key, _persistence, _session) = streaming_state(&fixture);
    let assistant_id = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .last()
        .expect("assistant placeholder")
        .id
        .clone();
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("another-session".into());

    let mut streams = ChatStreams::default();
    let (sender, _receiver) = mpsc::unbounded_channel();
    assert!(
        !start_stream(
            &mut state,
            &mut streams,
            &sender,
            &Persistence::without_database(),
            key,
            assistant_id,
        ),
        "session 已切走，旧 StartChatStream 必须丢弃"
    );
    assert!(streams.cancels.is_empty(), "不得为旧 key 注册取消句柄");
}

#[tokio::test]
async fn tool_pending_uses_session_workspace_not_execution_cwd() {
    let fixture = Fixture::new("workspace-key");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    // 实际执行目录在 linked worktree / 工作区外；会话键仍是仓库主 checkout。
    let outside = fixture.root.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    {
        let workspace = state.active_workspace_mut();
        workspace.cwd = Some(outside.clone());
        workspace.git = Some(crate::app::state::WorkspaceGit {
            repo_root: fixture.workspace().to_path_buf(),
            checkout_path: outside.clone(),
            is_linked: true,
            branch: Some("task".into()),
            main_branch: Some("dev".into()),
        });
    }
    let assistant_id = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .last()
        .expect("assistant placeholder")
        .id
        .clone();
    let mut streams = ChatStreams::default();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let (process_sender, _process_receiver) = mpsc::unbounded_channel();
    streams.set_process_sender(process_sender);
    assert!(start_stream(
        &mut state,
        &mut streams,
        &sender,
        &Persistence::without_database(),
        key.clone(),
        assistant_id
    ));
    assert_eq!(
        streams.db_workspaces.get(&key),
        Some(&fixture.workspace().to_path_buf()),
        "落库键必须是会话共享键，不能是执行 cwd"
    );

    let call = ReadyToolCall {
        call_id: "c1".into(),
        name: "write".into(),
        arguments_json: r#"{"path":"/tmp/lx-outside-test.txt","content":"x"}"#.into(),
        ordinal: 0,
    };
    let (reply, receiver) = oneshot::channel();
    begin_tool(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        call,
        reply,
    );
    let ticket = receiver
        .await
        .expect("pending ack")
        .expect("pending must not fail with workspace missing");
    assert_eq!(ticket.session_id, key.session);
}
