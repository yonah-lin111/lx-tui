//! 集成测试公共辅助：隔离被测二进制的数据库并预信任其工作目录。
//!
//! 启动信任门在未信任目录会先输出控制台信任页，干扰 PTY 交互断言；
//! 测试统一通过 `LX_TUI_DB_PATH` 指向独立临时库（每次调用独立，避免并发锁）
//! 并预先信任 manifest 目录。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::CommandBuilder;

/// 为被测二进制指定隔离数据库，并预信任其工作目录（manifest 目录）。
pub fn isolate_database(command: &mut CommandBuilder) {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir()
        .join(format!(
            "lx-tui-it-db-{}-{nanos:x}-{sequence:x}",
            std::process::id()
        ))
        .join("lx.db");
    let database = lx_tui::db::Database::open(&db_path).expect("isolated database opens");
    let workspace = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).expect("manifest dir");
    database
        .trust_workspace(&workspace, "lx-tui")
        .expect("workspace trusted");
    command.env("LX_TUI_DB_PATH", db_path);
}
