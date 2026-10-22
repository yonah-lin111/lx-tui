//! 单二进制入口：日志、终端初始化与事件循环编排。

use std::path::{Path, PathBuf};

use lx_tui::app::state::AppState;
use lx_tui::config::Config;
use lx_tui::db::is_dev_environment;
use lx_tui::db::service::Persistence;
use lx_tui::event;
use lx_tui::git;
use lx_tui::tui::Tui;
use tracing_subscriber::EnvFilter;

fn main() {
    if let Err(error) = run() {
        tracing::error!(error = %error, "lx-tui exited with error");
        // 顶层错误需要让用户看到；终端已恢复，这不是调试输出。
        eprintln!("lx-tui: {error}");
        std::process::exit(1);
    }
}

/// 启动编排：日志 -> 配置 -> 信任页（行内）-> 终端 -> 事件循环。
fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing()?;
    tracing::info!("lx-tui starting");

    let config = Config::default();
    let mut state = AppState::demo();
    let persistence = Persistence::start(database_worktree().as_deref());
    // 全局 prompt 历史：库不可用时内部降级为空历史，不阻断启动。
    persistence.load_prompt_history(&mut state);
    // 启动信任门：未信任时先在当前终端屏幕（行内）显示信任页；拒绝则不进入应用。
    if !event::startup_trust(&persistence, &mut state)? {
        eprintln!("lx-tui: {}", lx_tui::ui::text::TRUST_REFUSED_MESSAGE);
        return Ok(());
    }
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

/// 日志写入临时目录文件；TUI 运行期间 stdout 不可用。
fn init_tracing() -> std::io::Result<()> {
    let log_path = std::env::temp_dir().join("lx-tui.log");
    let file = std::fs::File::create(log_path)?;
    let filter = EnvFilter::try_from_env("LX_TUI_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .try_init();
    Ok(())
}
