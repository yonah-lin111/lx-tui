//! CLI 单元测试夹具与测试子模块入口。

mod args_and_help_tests;
mod config_domains_tests;
mod prompt_and_todo_tests;
mod session_tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cli::{EXIT_OK, parse_cli, run_cli};
use crate::db::Database;

/// 隔离的 CLI 测试环境：独占临时 `HOME`、临时 SQLite `lx.db` 与临时工作区目录。
pub(super) struct CliTestEnv {
    root: PathBuf,
    pub db_path: PathBuf,
    pub workspace: PathBuf,
}

impl CliTestEnv {
    pub fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let raw_root = std::env::temp_dir().join(format!(
            "lx-tui-cli-test-{tag}-{}-{nanos}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&raw_root).expect("create temp root");
        let root = std::fs::canonicalize(&raw_root).unwrap_or(raw_root);
        let home = root.join("home");
        let workspace = root.join("workspace");
        let db_dir = home.join(".lx-t-dev").join("db");
        std::fs::create_dir_all(&db_dir).expect("create db dir");
        std::fs::create_dir_all(&workspace).expect("create workspace dir");
        let db_path = db_dir.join("lx.db");
        crate::platform::set_test_env(Some(home), Some(db_path.clone()));
        Self {
            root,
            db_path,
            workspace,
        }
    }

    pub fn open_db(&self) -> Database {
        Database::open(&self.db_path).expect("open test db")
    }

    pub fn trust_workspace(&self) {
        let db = self.open_db();
        db.trust_workspace(&self.workspace, "workspace")
            .expect("trust workspace");
    }

    /// 构造带 `argv[0] = "lx-tui"` 与 `-C <workspace>` 的参数并执行 `parse_cli` + `run_cli`。
    pub fn exec(&self, args: &[&str]) -> Result<i32, String> {
        let mut full = vec![
            "lx-tui".to_string(),
            args[0].to_string(),
            "-C".to_string(),
            self.workspace.to_string_lossy().into_owned(),
        ];
        full.extend(args[1..].iter().map(|s| (*s).to_string()));
        let action = parse_cli(full)?;
        run_cli(action, None).map_err(|e| e.to_string())
    }

    /// 不自动注入 `-C` 的原始执行（用于 `model`、`todo` 等）。
    pub fn exec_raw(&self, args: &[&str]) -> Result<i32, String> {
        let mut full = vec!["lx-tui".to_string()];
        full.extend(args.iter().map(|s| (*s).to_string()));
        let action = parse_cli(full)?;
        run_cli(action, None).map_err(|e| e.to_string())
    }

    pub fn workspace_str(&self) -> &str {
        self.workspace.to_str().expect("utf8 workspace path")
    }

    pub fn create_subdir(&self, rel: &str) -> PathBuf {
        let dir = self.root.join(rel);
        std::fs::create_dir_all(&dir).expect("create subdir");
        std::fs::canonicalize(&dir).unwrap_or(dir)
    }

    pub fn write_file(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, content).expect("write file");
        path
    }
}

impl Drop for CliTestEnv {
    fn drop(&mut self) {
        crate::platform::set_test_env(None, None);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) fn assert_exit_ok(res: Result<i32, String>) {
    match res {
        Ok(code) => assert_eq!(code, EXIT_OK),
        Err(err) => panic!("expected EXIT_OK, got error: {err}"),
    }
}

pub(super) fn path_str(path: &Path) -> &str {
    path.to_str().expect("utf8 path")
}
