//! 单二进制入口：日志、终端初始化与事件循环编排。

use lx_tui::app::state::AppState;
use lx_tui::config::Config;
use lx_tui::db::service::Persistence;
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

/// 启动编排：日志 -> 配置 -> 信任页（行内）-> 终端 -> 事件循环。
fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing()?;
    tracing::info!("lx-tui starting");

    // 首次运行导入 lx-agent 模型配置（一次性；失败只记录，不阻断启动）。
    import_model_config();

    let config = Config::default();
    let mut state = AppState::demo();
    let persistence = Persistence::start();
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

/// 首次运行把 lx-agent 的 `~/.lx/config/ai.json` 导入本项目 `config/ai.json`。
///
/// 目标已存在即跳过（文件存在为幂等标记）；源缺失写入空配置；
/// 源损坏记录错误并留待修复后重试，绝不覆盖已有文件。
fn import_model_config() {
    use lx_tui::config::ai;
    let path = match ai::ai_config_path() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "ai config path unavailable");
            return;
        }
    };
    match ai::import_from_lx_agent(&path) {
        Ok(true) => tracing::info!(path = %path.display(), "imported model config"),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "model config import failed")
        }
    }
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
