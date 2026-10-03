//! 单二进制入口：日志、终端初始化与事件循环编排。

use lx_tui::app::state::AppState;
use lx_tui::config::Config;
use lx_tui::event;
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

/// 启动编排：日志 -> 配置 -> 终端 -> 事件循环。
fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing()?;
    tracing::info!("lx-tui starting");

    let config = Config::default();
    let mut state = AppState::demo();
    let mut tui = Tui::init()?;
    let result = event::run(&mut tui, &mut state, &config);
    tui.restore()?;
    result?;

    tracing::info!("lx-tui stopped");
    Ok(())
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
