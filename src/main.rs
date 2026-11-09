//! 单二进制入口：日志、终端初始化与事件循环编排。

use std::path::{Path, PathBuf};

use lx_tui::app::state::AppState;
use lx_tui::cli::{CliAction, EXIT_ERR, EXIT_OK, parse_cli, run_cli};
use lx_tui::config::Config;
use lx_tui::db::is_dev_environment;
use lx_tui::db::service::Persistence;
use lx_tui::event;
use lx_tui::git;
use lx_tui::tui::Tui;

fn main() {
    // guard 必须活到顶层错误分支之后，否则最后一条 error 日志会在 worker 关闭后丢失。
    let (result, log_guard) = run();
    match result {
        Ok(EXIT_OK) => drop(log_guard),
        Ok(code) => {
            drop(log_guard);
            std::process::exit(code);
        }
        Err(error) => {
            tracing::error!(error = %error, "lx-tui exited with error");
            // 顶层错误需要让用户看到；终端已恢复，这不是调试输出。
            eprintln!("lx-tui: {error}");
            drop(log_guard);
            std::process::exit(EXIT_ERR);
        }
    }
}

/// 入口分发：帮助/版本/无头 exec 直接返回退出码，无参数进入 TUI。
fn run() -> (
    Result<i32, Box<dyn std::error::Error>>,
    Option<lx_tui::logging::LogGuard>,
) {
    let action = match parse_cli(std::env::args_os()) {
        Ok(action) => action,
        Err(usage_error) => {
            eprintln!("lx-tui: {usage_error}");
            return (Ok(EXIT_ERR), None);
        }
    };
    let (config, config_error) = match lx_tui::config::load_settings() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error)),
    };
    let log_guard = lx_tui::logging::init(&config, role_of(&action));
    if let Some(error) = config_error {
        tracing::warn!(target: "lx::config", %error, "settings.json load failed; using defaults");
    }
    tracing::info!(target: "lx::logging", "lx-tui starting");

    let result = match action {
        CliAction::Tui => run_tui(config).map(|()| EXIT_OK),
        other => run_cli(other, database_worktree().as_deref()),
    };
    (result, Some(log_guard))
}

/// 日志角色：TUI 与 CLI 子命令共用一份日志，用字段区分。
fn role_of(action: &CliAction) -> &'static str {
    if matches!(action, CliAction::Tui) {
        "tui"
    } else {
        "cli"
    }
}

/// TUI 启动编排：配置 -> 信任页（行内）-> 终端 -> 事件循环。
fn run_tui(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(root) = lx_tui::config::user_data_root() {
        let _ = lx_tui::chat::agent::ensure_global_agents_md(&root);
    }
    let mut state = AppState::demo();
    let persistence = Persistence::start(database_worktree().as_deref());
    // 全局 prompt 历史：库不可用时内部降级为空历史，不阻断启动。
    persistence.load_prompt_history(&mut state);
    // 全局 todo 列表：库不可用时内部降级为空列表，不阻断启动。
    persistence.load_todos(&mut state);
    // 启动信任门：未信任时先在当前终端屏幕（行内）显示信任页；拒绝则不进入应用。
    if !event::startup_trust(&persistence, &mut state)? {
        eprintln!("lx-tui: {}", lx_tui::ui::text::TRUST_REFUSED_MESSAGE);
        return Ok(());
    }
    persistence.load_workspace_history(&mut state);
    let mut tui = Tui::init()?;
    let result = event::run(&mut tui, &mut state, &config, persistence);
    tui.restore()?;
    result?;

    tracing::info!("lx-tui stopped");
    Ok(())
}

/// 开发构建位于 linked worktree 时返回其根：持久层据此使用独立库文件，
/// 避免不同分支各自推进的 Schema 互相判废。主 checkout 与非仓库不隔离。
fn database_worktree() -> Option<PathBuf> {
    if !is_dev_environment() {
        return None;
    }
    git::linked_worktree_root(Path::new("."))
}
