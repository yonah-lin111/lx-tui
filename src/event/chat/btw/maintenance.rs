//! BTW 线程维护：`/new` 删除当前 QA 的旁路线程。

use tokio::sync::mpsc;

use crate::app::state::AppState;
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::tiling::PaneId;

use super::{BtwEvent, workspace_for_pane};

/// 删除线程与 child session；成功/失败都回投，视图只在删除成功后重置。
pub(super) fn delete_thread(
    state: &mut AppState,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    parent_session_id: &str,
    qa_id: &str,
    thread_id: &str,
) -> bool {
    let Some(database) = persistence.database() else {
        return false;
    };
    let Some(workspace) = workspace_for_pane(state, pane) else {
        return false;
    };
    match database.delete_btw_thread(&workspace, thread_id) {
        Ok(()) => {
            let _ = sender.send(AppEvent::Btw(BtwEvent::ThreadDeleted {
                pane,
                parent_session_id: parent_session_id.to_string(),
                qa_id: qa_id.to_string(),
            }));
            true
        }
        Err(error) => {
            tracing::warn!(%error, "btw thread delete failed");
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane,
                qa_id: Some(qa_id.to_string()),
                message: crate::ui::text::detail::LX_BTW_RESET_FAILED.to_string(),
            }));
            false
        }
    }
}
