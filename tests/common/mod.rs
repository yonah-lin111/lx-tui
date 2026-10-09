//! 集成测试公共辅助：隔离被测二进制的数据库并预信任其工作目录。
//!
//! 启动信任检查在未信任目录会弹出模态浮层，会干扰 PTY 交互断言；
//! 测试统一通过 `LX_TUI_DB_PATH` 指向独立临时库并预先信任 manifest 目录。

use portable_pty::CommandBuilder;

/// 为被测二进制指定隔离数据库，并预信任其工作目录（manifest 目录）。
pub fn isolate_database(command: &mut CommandBuilder) {
    let db_path = std::env::temp_dir()
        .join(format!("lx-tui-it-db-{}", std::process::id()))
        .join("lx.db");
    let database = lx_tui::db::Database::open(&db_path).expect("isolated database opens");
    let workspace = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).expect("manifest dir");
    database
        .trust_workspace(&workspace, "lx-tui")
        .expect("workspace trusted");
    command.env("LX_TUI_DB_PATH", db_path);
}
