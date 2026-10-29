//! 端到端撤销：真实文件 + 临时 SQLite，验证恢复内容、冲突零写入与提示持久化。

use super::*;
use crate::db::chat::{
    ImageWrite, ModelSnapshot, ObservedWrite, PathMutationWrite, TurnOutcome, Usage,
};

fn fixture(label: &str) -> (Database, std::path::PathBuf, std::path::PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "lx-rewind-e2e-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    let db_path = root.join("lx.db");
    let database = Database::open(&db_path).expect("db");
    (database, root, db_path)
}

/// 读后版本 token：与执行层同一口径。
fn version_of(bytes: &[u8]) -> String {
    crate::platform::version_token_from_parts("regular", Some(bytes), Some(0o644), None)
        .expect("token")
}

fn image(bytes: &[u8]) -> ImageWrite {
    ImageWrite {
        kind: "regular".into(),
        bytes: Some(bytes.to_vec()),
        mode: Some(0o644),
        link_target: None,
        git_oid: None,
        worktree_root: None,
        git_dir: None,
        head_ref: None,
        head_oid: None,
    }
}

/// 造一轮“Agent 将 old 写成 new”的完整记录。
fn turn_with_file(
    database: &Database,
    workspace: &Path,
    existing_session: Option<&str>,
    file: &Path,
    before: &[u8],
    after: &[u8],
) -> crate::db::chat::TurnCommitted {
    let turn = database
        .begin_turn(
            workspace,
            existing_session,
            "edit it",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[PathMutationWrite {
                operation: "update".into(),
                path: file.to_string_lossy().into_owned(),
                target_path: file.to_string_lossy().into_owned(),
                anchor_path: None,
                anchor_canonical: None,
                before: image(before),
                planned_after: image(after),
                expected_version: Some(version_of(before)),
            }],
        )
        .expect("prepare");
    database
        .commit_mutations(
            workspace,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: image(after),
                success: true,
            }],
        )
        .expect("commit mutation");
    database
        .finish(
            workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    turn
}

#[cfg(unix)]
fn record(bytes: &[u8]) -> crate::db::chat::ImageRecord {
    crate::db::chat::ImageRecord {
        kind: "regular".into(),
        bytes: Some(bytes.to_vec()),
        mode: Some(0o644),
        link_target: None,
        git_oid: None,
    }
}

#[cfg(unix)]
fn restore_path(target: &Path) -> RestorePath {
    RestorePath {
        path: target.to_path_buf(),
        target_path: target.to_path_buf(),
        anchor_path: None,
        anchor_canonical: None,
        before: record(b"before"),
        expected_after: record(b"after"),
        worktree_root: None,
        git_dir: None,
        head_ref: None,
        head_oid: None,
    }
}

/// 补偿必须以“本操作刚写入的内容”为判据，并真正写回执行前镜像。
#[cfg(unix)]
#[test]
fn compensation_writes_preimage_back_when_current_is_restored_content() {
    let (_database, workspace, _db_path) = fixture("compensate-ok");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"after").expect("restored content");
    let restored_to = RawImage::regular(b"after".to_vec(), Some(0o644));
    let preimage = RawImage::regular(b"before".to_vec(), Some(0o644));
    assert!(super::compensate(
        &[restore_path(&file)],
        &[restored_to],
        &[preimage]
    ));
    assert_eq!(std::fs::read(&file).expect("read"), b"before");
}

/// 恢复后又被外部程序改过：拒绝补偿，保持现状。
#[cfg(unix)]
#[test]
fn compensation_refuses_when_external_write_happened() {
    let (_database, workspace, _db_path) = fixture("compensate-conflict");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"someone else").expect("external content");
    let restored_to = RawImage::regular(b"after".to_vec(), Some(0o644));
    let preimage = RawImage::regular(b"before".to_vec(), Some(0o644));
    assert!(!super::compensate(
        &[restore_path(&file)],
        &[restored_to],
        &[preimage]
    ));
    assert_eq!(std::fs::read(&file).expect("read"), b"someone else");
}

fn request(
    turn: &crate::db::chat::TurnCommitted,
    workspace: &Path,
    action: RewindAction,
) -> RewindRequest {
    RewindRequest {
        pane: crate::layout::PaneId::alloc(),
        session_id: turn.session_id.clone(),
        target_user_message_id: turn.user_message_id.clone(),
        expected_revision: u64::try_from(turn.revision).expect("revision"),
        action,
        workspace: workspace.to_path_buf(),
        input_at_open: String::new(),
        user_text: "edit it".into(),
        operation_id: format!("op-{}", turn.user_message_id),
    }
}

#[test]
fn restores_bytes_and_removes_empty_session() {
    let (database, workspace, _db_path) = fixture("restore");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, None, &file, b"old", b"new");

    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertAndDelete),
    );
    assert!(
        matches!(
            outcome,
            RewindOutcome::Succeeded {
                restored_files: 1,
                session_deleted: true,
                ..
            }
        ),
        "unexpected: {outcome:?}"
    );
    assert_eq!(std::fs::read(&file).expect("file"), b"old");
    assert!(
        database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .is_none(),
        "只剩 Rewind 提示的空会话必须整体删除"
    );
    assert!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .is_empty(),
        "提示随会话级联清除"
    );
}

#[test]
fn restores_and_keeps_tip_when_earlier_turns_remain() {
    let (database, workspace, _db_path) = fixture("restore-tip");
    let first = database
        .begin_turn(
            &workspace,
            None,
            "first",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("first turn");
    database
        .finish(
            &workspace,
            &first.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish first");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(
        &database,
        &workspace,
        Some(&first.session_id),
        &file,
        b"old",
        b"new",
    );
    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertAndDelete),
    );
    assert!(
        matches!(
            outcome,
            RewindOutcome::Succeeded {
                restored_files: 1,
                session_deleted: false,
                ..
            }
        ),
        "unexpected: {outcome:?}"
    );
    let messages = database
        .load_chat_session(&workspace, &turn.session_id)
        .expect("load")
        .expect("session");
    assert_eq!(messages.len(), 2, "上一轮保留");
    let notices = database
        .load_notices(&workspace, &turn.session_id)
        .expect("notices");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, "revert_and_delete");
    assert_eq!(notices[0].restored_file_count, 1);
}

#[test]
fn later_external_write_is_a_conflict_and_nothing_is_rewritten() {
    let (database, workspace, _db_path) = fixture("conflict");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, None, &file, b"old", b"new");
    // 该轮之后，外部程序（或另一个 Agent）又改了同一文件。
    std::fs::write(&file, b"later").expect("external write");

    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertAndDelete),
    );
    assert!(matches!(outcome, RewindOutcome::Failed { .. }));
    assert_eq!(
        std::fs::read(&file).expect("file"),
        b"later",
        "冲突必须零写入"
    );
    assert!(
        !database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .expect("session")
            .is_empty(),
        "冲突时不能删除对话"
    );
    assert!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .is_empty(),
        "失败不插成功 tip"
    );
    assert!(
        database
            .unfinished_operation(&turn.session_id)
            .expect("operation")
            .is_none(),
        "预检冲突必须结算 operation，不能永久阻塞该会话"
    );
}

#[test]
fn revert_only_keeps_conversation_and_is_idempotent() {
    let (database, workspace, _db_path) = fixture("revert-only");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, None, &file, b"old", b"new");

    let first = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertOnly),
    );
    assert!(matches!(
        first,
        RewindOutcome::Succeeded {
            restored_files: 1,
            ..
        }
    ));
    assert_eq!(std::fs::read(&file).expect("file"), b"old");
    assert!(
        !database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .expect("session")
            .is_empty(),
        "仅恢复文件保留对话"
    );
    // 第二次仅恢复文件：幂等无操作，不插新 tip（revision 已随首次成功推进）。
    let revision = u64::try_from(
        database
            .session_revision(&turn.session_id)
            .expect("revision"),
    )
    .expect("u64");
    let second = run(
        &database,
        &workspace,
        &RewindRequest {
            operation_id: "op-second".into(),
            expected_revision: revision,
            ..request(&turn, &workspace, RewindAction::RevertOnly)
        },
    );
    assert!(
        matches!(
            second,
            RewindOutcome::Succeeded {
                already_restored: true,
                restored_files: 0,
                ..
            }
        ),
        "second = {second:?}"
    );
    // 首次恢复插入一条 tip；第二次幂等无操作不得再插。
    assert_eq!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .len(),
        1
    );
}

#[test]
fn unreachable_message_without_snapshot_can_only_delete_conversation() {
    let (database, workspace, _db_path) = fixture("no-snapshot");
    let turn = database
        .begin_turn(
            &workspace,
            None,
            "plain",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    // 模拟旧轮次：删除 scope，等价于升级前的历史。
    database
        .drop_turn_scope_for_tests(&turn.user_message_id)
        .expect("drop scope");
    database
        .finish(
            &workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertOnly),
    );
    assert!(matches!(outcome, RewindOutcome::Failed { .. }));
    let outcome = run(
        &database,
        &workspace,
        &RewindRequest {
            operation_id: "op-delete-only".into(),
            ..request(&turn, &workspace, RewindAction::DeleteOnly)
        },
    );
    assert!(matches!(outcome, RewindOutcome::Succeeded { .. }));
}

#[test]
fn rewind_rewrites_deleted_desktop_image_to_session_archived_copy() {
    let (database, workspace, _db_path) = fixture("rewind-img-fallback");
    let desktop_img = workspace.join("Desktop_PixPin.png");
    let img: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_pixel(4, 4, image::Rgba([9, 8, 7, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png");
    std::fs::write(&desktop_img, &png).expect("write desktop img");

    let desktop_str = desktop_img.to_string_lossy().into_owned();
    let user_text = format!("请分析 @[refer-image]({desktop_str}) 布局");
    let prepared = crate::chat::prepare_user_message(&workspace, &user_text, &[]).expect("prepare");

    let turn = database
        .begin_turn_with_attachments(
            &workspace,
            None,
            &user_text,
            &prepared.attachments,
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("begin turn");
    database
        .finish(
            &workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");

    let archived_path = turn.attachments[0]
        .source_path
        .clone()
        .expect("archived path");
    assert!(archived_path.contains(&format!("/images/{}/", turn.session_id)));

    // 模拟用户清理了桌面原图后执行 rewind
    std::fs::remove_file(&desktop_img).expect("remove desktop image");
    let outcome = run(
        &database,
        &workspace,
        &RewindRequest {
            operation_id: "op-img-rewind".into(),
            user_text: user_text.clone(),
            ..request(&turn, &workspace, RewindAction::DeleteOnly)
        },
    );
    match outcome {
        RewindOutcome::Succeeded {
            user_text: rewound_text,
            ..
        } => {
            assert_eq!(
                rewound_text,
                format!("请分析 @[refer-image]({archived_path}) 布局"),
                "桌面原图被删后，Rewind 回填文本应自动替换为受管归档路径"
            );
            // 回填后的草稿可再次通过 prepare_user_message 校验发送
            assert!(
                crate::chat::prepare_user_message(&workspace, &rewound_text, &[]).is_ok(),
                "回填后的文本必须可直接再次发送"
            );
        }
        other => panic!("expected Succeeded, got {other:?}"),
    }

    let _ = std::fs::remove_file(&archived_path);
}

/// 会话 cwd 在主 checkout，实际目标在 linked worktree：只恢复实际目标路径。
#[test]
fn restores_actual_linked_worktree_target_only() {
    let (database, workspace, _db_path) = fixture("worktree");
    let repo = workspace.join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let git = |dir: &Path, args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&repo, &["init", "-q", "-b", "dev"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), b"base").expect("base");
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let linked = workspace.join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "task",
            linked.to_str().expect("utf8"),
        ],
    );
    let target = linked.join("a.txt");
    std::fs::write(&target, b"new").expect("agent write");
    let identity = crate::git::identify(&target).expect("identity");
    let with_git = |bytes: &[u8]| ImageWrite {
        kind: "regular".into(),
        bytes: Some(bytes.to_vec()),
        mode: Some(0o644),
        link_target: None,
        git_oid: None,
        worktree_root: Some(identity.root.to_string_lossy().into_owned()),
        git_dir: Some(identity.git_dir.to_string_lossy().into_owned()),
        head_ref: identity.head_ref.clone(),
        head_oid: identity.head_oid.clone(),
    };
    let turn = database
        .begin_turn(
            &repo,
            None,
            "edit linked",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            &repo,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[PathMutationWrite {
                operation: "update".into(),
                path: target.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                anchor_path: None,
                anchor_canonical: None,
                before: with_git(b"base"),
                planned_after: with_git(b"new"),
                expected_version: Some(version_of(b"base")),
            }],
        )
        .expect("prepare");
    database
        .commit_mutations(
            &repo,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: with_git(b"new"),
                success: true,
            }],
        )
        .expect("commit");
    database
        .finish(
            &repo,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    // 主 checkout 同名文件也改过，但不在本轮记录里，必须保持不变。
    std::fs::write(repo.join("a.txt"), b"main-dirty").expect("main write");
    let outcome = run(
        &database,
        &repo,
        &crate::app::effects::RewindRequest {
            pane: crate::layout::PaneId::alloc(),
            session_id: turn.session_id.clone(),
            target_user_message_id: turn.user_message_id.clone(),
            expected_revision: u64::try_from(turn.revision).expect("rev"),
            action: RewindAction::RevertAndDelete,
            workspace: repo.clone(),
            input_at_open: String::new(),
            user_text: "edit linked".into(),
            operation_id: "op-worktree".into(),
        },
    );
    assert!(
        matches!(
            outcome,
            RewindOutcome::Succeeded {
                restored_files: 1,
                ..
            }
        ),
        "unexpected: {outcome:?}"
    );
    assert_eq!(std::fs::read(&target).expect("linked"), b"base");
    assert_eq!(
        std::fs::read(repo.join("a.txt")).expect("main"),
        b"main-dirty",
        "主 checkout 同名文件不得被恢复"
    );
}

/// session guard 由 `run` 全程持有：另一实例（或同进程已持锁）运行时零写入拒绝。
#[cfg(unix)]
#[test]
fn held_session_guard_blocks_rewind_without_writing() {
    let (database, workspace, _db_path) = fixture("session-guard");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, None, &file, b"old", b"new");
    let request = request(&turn, &workspace, RewindAction::RevertAndDelete);
    let guard = crate::platform::try_lock_session(&turn.session_id)
        .expect("lock")
        .expect("free");
    let outcome = run(&database, &workspace, &request);
    assert!(
        matches!(outcome, RewindOutcome::Failed { .. }),
        "持有 session guard 时必须拒绝执行"
    );
    assert_eq!(std::fs::read(&file).expect("read"), b"new", "零写入");
    drop(guard);
}
