use super::*;
use crate::db::Database;
use crate::db::chat::ModelSnapshot;

#[test]
fn handle_session_title_generated_updates_db_and_respects_manual_rename() {
    let db = Database::open_in_memory().expect("in memory db");
    let workspace = std::path::PathBuf::from("/tmp/lx-title-test-ws");
    let model = ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let committed = db
        .begin_turn(&workspace, None, "First long message from user", &model)
        .expect("begin turn");
    let persistence = Persistence::with_database(db);
    let mut state = AppState::demo();
    state
        .pending_session_titles
        .insert(committed.session_id.clone());

    // 自动标题正常返回：更新数据库标题
    handle_session_title_generated(
        &mut state,
        &persistence,
        &workspace,
        &committed.session_id,
        "First long message from user",
        Some("Concise Generated Title"),
    );
    let sessions = persistence
        .database()
        .expect("db")
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions[0].title, "Concise Generated Title");

    // 若已被消费或被手动重命名，再次回投不得覆盖
    persistence
        .database()
        .expect("db")
        .rename_chat_session(&workspace, &committed.session_id, "User Manual Title")
        .expect("manual rename");
    state
        .pending_session_titles
        .insert(committed.session_id.clone());
    handle_session_title_generated(
        &mut state,
        &persistence,
        &workspace,
        &committed.session_id,
        "First long message from user",
        Some("Stale Auto Title"),
    );
    let sessions = persistence
        .database()
        .expect("db")
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions[0].title, "User Manual Title");
}
