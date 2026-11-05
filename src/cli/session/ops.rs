//! `session rewind` 与 `session compact`：复用事件层撤销执行器与压缩纯逻辑。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::app::effects::RewindRequest;
use crate::app::state::RewindAction;
use crate::chat::agent::exec::resolve_model_target;
use crate::chat::compact::{incremental, playable_messages, summary_request, validate_summary};
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::cli::context::{WorkspaceContext, acquire_session_lock};
use crate::cli::output::Output;
use crate::db::Database;
use crate::db::chat::{CompactCommit, CompactRevert, FileAvailability, RestorePath};
use crate::event::chat::rewind::{RewindOutcome, execute_rewind};
use crate::layout::PaneId;

/// `session rewind --mode` 的四种取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RewindModeArg {
    #[default]
    RevertAndDelete,
    DeleteOnly,
    RevertOnly,
    RestoreCompact,
}

impl RewindModeArg {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "revert-and-delete" => Ok(Self::RevertAndDelete),
            "delete-only" => Ok(Self::DeleteOnly),
            "revert-only" => Ok(Self::RevertOnly),
            "restore-compact" => Ok(Self::RestoreCompact),
            other => Err(format!(
                "invalid '--mode' value '{other}'; expected 'revert-and-delete', 'delete-only', 'revert-only', or 'restore-compact'"
            )),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::RevertAndDelete => "revert-and-delete",
            Self::DeleteOnly => "delete-only",
            Self::RevertOnly => "revert-only",
            Self::RestoreCompact => "restore-compact",
        }
    }

    fn action(self) -> Option<RewindAction> {
        match self {
            Self::RevertAndDelete => Some(RewindAction::RevertAndDelete),
            Self::DeleteOnly => Some(RewindAction::DeleteOnly),
            Self::RevertOnly => Some(RewindAction::RevertOnly),
            Self::RestoreCompact => None,
        }
    }
}

/// 进程内唯一操作 id（幂等重放键；不依赖 uuid 依赖）。
fn operation_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("cli-{prefix}-{nanos:x}")
}

fn availability_label(availability: &FileAvailability) -> String {
    match availability {
        FileAvailability::Ready { paths } => format!("ready ({paths} path(s))"),
        FileAvailability::KnownEmpty => "known-empty".to_string(),
        FileAvailability::AlreadyRestored => "already-restored".to_string(),
        FileAvailability::Unavailable => "unavailable".to_string(),
        FileAvailability::Incomplete => "incomplete".to_string(),
    }
}

fn path_json(path: &RestorePath) -> serde_json::Value {
    json!({
        "path": path.path,
        "target_path": path.target_path,
        "kind": path.before.kind,
        "expected_after": path.expected_after.kind,
    })
}

/// 执行 `session rewind`。
pub fn run_rewind(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    mode: RewindModeArg,
    dry_run: bool,
    out: &Output,
) -> Result<(), String> {
    if dry_run {
        return rewind_dry_run(ctx, db, session_id, mode, out);
    }
    if mode == RewindModeArg::RestoreCompact {
        return restore_compact(ctx, db, session_id, out);
    }
    // 先加锁再读取目标轮次，避免活跃会话被读取后立即改写；
    // execute_rewind 内部会再次取同一把锁，因此读完即释放。
    let guard = acquire_session_lock(session_id)?;
    let turn = db
        .latest_user_turn(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to read the latest user turn: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' has no user turn to rewind"))?;
    let expected_revision = u64::try_from(turn.revision)
        .map_err(|_| format!("session '{session_id}' has an invalid revision"))?;
    let action = mode
        .action()
        .ok_or_else(|| "rewind mode does not support file actions".to_string())?;
    let request = RewindRequest {
        pane: PaneId::alloc(),
        session_id: session_id.to_string(),
        target_user_message_id: turn.user_message_id.clone(),
        expected_revision,
        action,
        workspace: ctx.workspace.clone(),
        input_at_open: String::new(),
        user_text: turn.text.clone(),
        operation_id: operation_id("rewind"),
    };
    drop(guard);
    match execute_rewind(db, &ctx.workspace, &request) {
        RewindOutcome::Succeeded {
            action,
            restored_files,
            already_restored,
            user_text,
            deleted,
            session_deleted,
        } => {
            let value = json!({
                "rewound": true,
                "session_id": session_id,
                "mode": mode.label(),
                "action": action.as_str(),
                "target_user_message_id": turn.user_message_id,
                "restored_files": restored_files,
                "already_restored": already_restored,
                "deleted": deleted,
                "session_deleted": session_deleted,
                "user_text": user_text,
            });
            out.emit(&value, |out, writer| {
                writeln!(
                    writer,
                    "{} rewound session '{}' ({})",
                    out.green("OK"),
                    session_id,
                    mode.label()
                )?;
                writeln!(writer, "  files restored: {restored_files}")?;
                if deleted {
                    writeln!(
                        writer,
                        "  conversation: latest user turn deleted{}",
                        if session_deleted {
                            " (session is now empty)"
                        } else {
                            ""
                        }
                    )?;
                }
                Ok(())
            })
        }
        RewindOutcome::Failed { message } => Err(message),
    }
}

/// `--dry-run`：只读预检，输出目标轮次、可用性与受影响路径。
fn rewind_dry_run(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    mode: RewindModeArg,
    out: &Output,
) -> Result<(), String> {
    if mode == RewindModeArg::RestoreCompact {
        return restore_compact_dry_run(ctx, db, session_id, out);
    }
    let turn = db
        .latest_user_turn(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to read the latest user turn: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' has no user turn to rewind"))?;
    let plan = db
        .restore_plan(&ctx.workspace, session_id, &turn.user_message_id)
        .map_err(|e| format!("failed to derive the restore plan: {}", e.code()))?;
    let (preview, preview_revision, _) = db
        .rewind_availability(&ctx.workspace, session_id, &turn.user_message_id)
        .map_err(|e| format!("failed to check rewind availability: {}", e.code()))?;
    let value = json!({
        "dry_run": true,
        "session_id": session_id,
        "target_user_message_id": turn.user_message_id,
        "sequence": turn.sequence,
        "turn_index": turn.turn_index,
        "user_text": plan.user_text,
        "revision": plan.revision,
        "preview_revision": preview_revision,
        "availability": availability_label(&plan.availability),
        "preview_availability": availability_label(&preview),
        "paths": plan.paths.iter().map(path_json).collect::<Vec<_>>(),
    });
    out.emit(&value, |out, writer| {
        writeln!(writer, "{}", out.bold("Rewind dry run"))?;
        writeln!(writer, "  session: {session_id}")?;
        writeln!(writer, "  target:  {}", turn.user_message_id)?;
        writeln!(
            writer,
            "  turn:    {} (sequence {})",
            turn.turn_index, turn.sequence
        )?;
        writeln!(writer, "  revision: {}", plan.revision)?;
        writeln!(
            writer,
            "  availability: {}",
            availability_label(&plan.availability)
        )?;
        writeln!(writer, "  preview: {}", availability_label(&preview))?;
        if plan.paths.is_empty() {
            writeln!(writer, "  files: none")?;
        } else {
            writeln!(writer, "  files:")?;
            for path in &plan.paths {
                writeln!(writer, "    - {}", path.target_path.display())?;
            }
        }
        Ok(())
    })
}

/// `--dry-run --mode restore-compact`：只读预检当前生效的 compact 检查点。
fn restore_compact_dry_run(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    out: &Output,
) -> Result<(), String> {
    let checkpoint = db
        .active_checkpoint(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to query compact checkpoint: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' has no active compact checkpoint"))?;
    let revision = db
        .session_revision(session_id)
        .map_err(|e| format!("failed to read session revision: {}", e.code()))?;
    let value = json!({
        "dry_run": true,
        "session_id": session_id,
        "mode": "restore-compact",
        "revision": revision,
        "checkpoint_id": checkpoint.id,
        "sequence": checkpoint.sequence,
        "start_sequence": checkpoint.start_sequence,
        "end_sequence": checkpoint.end_sequence,
        "model": checkpoint.model_label,
        "summary": checkpoint.summary_text,
    });
    out.emit(&value, |out, writer| {
        writeln!(writer, "{}", out.bold("Rewind dry run"))?;
        writeln!(writer, "  session: {session_id}")?;
        writeln!(writer, "  mode:    restore-compact")?;
        writeln!(
            writer,
            "  checkpoint: {} ({})",
            checkpoint.id, checkpoint.state
        )?;
        writeln!(
            writer,
            "  range:   {}..{}",
            checkpoint.start_sequence, checkpoint.end_sequence
        )?;
        writeln!(writer, "  revision: {revision}")
    })
}

/// `--mode restore-compact`：撤销当前生效的 compact 检查点。
fn restore_compact(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    out: &Output,
) -> Result<(), String> {
    let _guard = acquire_session_lock(session_id)?;
    let revision = db
        .session_revision(session_id)
        .map_err(|e| format!("failed to read session revision: {}", e.code()))?;
    let checkpoint = db
        .active_checkpoint(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to query compact checkpoint: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' has no active compact checkpoint"))?;
    let reverted = db
        .revert_compact(
            &ctx.workspace,
            &CompactRevert {
                operation_id: operation_id("compact-revert"),
                session_id: session_id.to_string(),
                checkpoint_id: checkpoint.id.clone(),
                expected_revision: revision,
            },
        )
        .map_err(|e| format!("failed to restore compact checkpoint: {}", e.code()))?;
    let value = json!({
        "restored": true,
        "session_id": session_id,
        "checkpoint_id": reverted.id,
        "sequence": reverted.sequence,
        "state": reverted.state,
        "start_sequence": reverted.start_sequence,
        "end_sequence": reverted.end_sequence,
    });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} restored compact checkpoint '{}' for session '{}'",
            out.green("OK"),
            reverted.id,
            session_id
        )
    })
}

/// 执行 `session compact`（默认摘要提交或 `--revert` 撤销）。
pub fn run_compact(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    model_override: Option<&str>,
    revert: bool,
    out: &Output,
) -> Result<(), String> {
    let _guard = acquire_session_lock(session_id)?;
    let revision = db
        .session_revision(session_id)
        .map_err(|e| format!("failed to read session revision: {}", e.code()))?;
    let active = db
        .active_checkpoint(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to query compact checkpoint: {}", e.code()))?;

    if revert {
        let Some(checkpoint) = active else {
            return Err(format!(
                "session '{session_id}' has no active compact checkpoint to revert"
            ));
        };
        let reverted = db
            .revert_compact(
                &ctx.workspace,
                &CompactRevert {
                    operation_id: operation_id("compact-revert"),
                    session_id: session_id.to_string(),
                    checkpoint_id: checkpoint.id,
                    expected_revision: revision,
                },
            )
            .map_err(|e| format!("failed to revert compact checkpoint: {}", e.code()))?;
        let value = json!({
            "reverted": true,
            "session_id": session_id,
            "checkpoint_id": reverted.id,
            "state": reverted.state,
            "sequence": reverted.sequence,
        });
        return out.emit(&value, |out, writer| {
            writeln!(
                writer,
                "{} reverted compact checkpoint '{}' for session '{}'",
                out.green("OK"),
                reverted.id,
                session_id
            )
        });
    }

    let messages = db
        .load_chat_session(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to load session: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' not found"))?;
    let playable = playable_messages(&messages);
    let input = incremental(
        &playable,
        active.as_ref().map(|checkpoint| checkpoint.end_sequence),
    );
    if input.is_empty() {
        return Err("no new completed turns available to compact".to_string());
    }

    let ai_path = crate::config::ai::ai_config_path()
        .map_err(|e| format!("failed to locate ai config: {e}"))?;
    let settings = crate::config::ai::load_ai(&ai_path)
        .map_err(|e| format!("failed to load ai config: {e}"))?
        .unwrap_or_else(crate::config::ai::AiSettings::empty);
    let session_model = db
        .chat_session_model(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to read session model: {}", e.code()))?;
    let session_model_ref = session_model.as_ref().map(|snapshot| {
        (
            snapshot.provider.as_str(),
            snapshot.model.as_str(),
            snapshot.variant.as_deref(),
        )
    });
    let target = resolve_model_target(&settings, model_override, session_model_ref)?;
    let snapshot = target.snapshot();
    let request = summary_request(&input, &snapshot, target.variant_params.clone());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;
    let summary = runtime.block_on(async {
        let mut text = String::new();
        let mut saw_tool_call = false;
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let outcome = crate::chat::client::run_stream(
            &target.transport,
            &target.base_url,
            Some(target.api_key.as_str()),
            Duration::from_millis(target.idle_timeout_ms),
            request,
            cancel_rx,
            |deltas: &[ChatDelta]| {
                for delta in deltas {
                    match delta {
                        ChatDelta::ToolInputStarted { .. }
                        | ChatDelta::ToolInputDelta { .. }
                        | ChatDelta::ToolInputEnded { .. } => {
                            saw_tool_call = true;
                            let _ = cancel_tx.send(true);
                        }
                        ChatDelta::Part(part) if part.kind == PartKind::Text => {
                            text.push_str(&part.text);
                        }
                        _ => {}
                    }
                }
            },
        )
        .await;
        if saw_tool_call {
            return Err("compact summary response unexpectedly contained tool calls".to_string());
        }
        if outcome.stop != StopReason::Complete {
            return Err(format!(
                "compact summary request failed: {}",
                outcome.stop.as_str()
            ));
        }
        validate_summary(&text).map_err(str::to_string)
    })?;

    let committed = db
        .commit_compact(
            &ctx.workspace,
            &CompactCommit {
                session_id: session_id.to_string(),
                expected_revision: revision,
                base_checkpoint_id: active.as_ref().map(|checkpoint| checkpoint.id.clone()),
                start_sequence: input.start_sequence,
                end_sequence: input.end_sequence,
                summary_text: summary,
                model_label: target.display_name,
                operation_id: operation_id("compact"),
            },
        )
        .map_err(|e| format!("failed to commit compact checkpoint: {}", e.code()))?;
    let value = json!({
        "compacted": true,
        "created": committed.created,
        "session_id": session_id,
        "checkpoint_id": committed.checkpoint.id,
        "sequence": committed.checkpoint.sequence,
        "start_sequence": committed.checkpoint.start_sequence,
        "end_sequence": committed.checkpoint.end_sequence,
        "model": committed.checkpoint.model_label,
        "summary": committed.checkpoint.summary_text,
    });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} compacted session '{}' ({}..{}, checkpoint '{}')",
            out.green("OK"),
            session_id,
            committed.checkpoint.start_sequence,
            committed.checkpoint.end_sequence,
            committed.checkpoint.id
        )
    })
}
