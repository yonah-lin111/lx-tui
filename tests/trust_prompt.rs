//! 集成测试：启动信任门（进入应用前的系统控制台信任页）在真实 PTY 中的行为。
//!
//! 未信任目录启动 `lx-tui`：先在控制台显示信任页（不进应用）；
//! 输入 `y` 放行并进入应用；输入 `n` 不进入应用、输出提示后退出。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alacritty_terminal::index::{Column, Line};
use lx_tui::terminal::Terminal;
use lx_tui::ui;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use ratatui::layout::Rect;

const COLS: u16 = 100;
const ROWS: u16 = 30;
const WAIT: Duration = Duration::from_secs(10);

/// 已启动的测试应用：子进程、PTY 写入端、原始输出与终端仿真画面。
struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    raw: Arc<Mutex<Vec<u8>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
    /// 在未信任目录启动被测二进制，使用独立数据库。
    fn spawn(cwd: &Path, db_path: &Path) -> Self {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_lx-tui"));
        command.env("TERM", "xterm-256color");
        command.env("LX_TUI_DB_PATH", db_path);
        command.cwd(cwd);
        let child = pair.slave.spawn_command(command).expect("spawn lx-tui");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("pty reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("pty writer")));
        let emulator = Arc::new(Mutex::new(Terminal::new(COLS, ROWS)));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&emulator);
        let raw_sink = Arc::clone(&raw);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        raw_sink.lock().unwrap().extend_from_slice(&buffer[..read]);
                        sink.lock().unwrap().feed(&buffer[..read]);
                    }
                }
            }
        });
        Self {
            child,
            writer,
            raw,
            emulator,
        }
    }

    fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes).expect("write to pty");
        writer.flush().expect("flush pty");
    }

    fn raw(&self) -> Vec<u8> {
        self.raw.lock().unwrap().clone()
    }

    fn screen(&self) -> String {
        let terminal = self.emulator.lock().unwrap();
        let grid = terminal.grid();
        let cols = usize::from(terminal.size().cols);
        let rows = usize::from(terminal.size().rows);
        let mut text = String::with_capacity(rows * (cols + 1));
        for row in 0..rows {
            for col in 0..cols {
                text.push(grid[Line(row as i32)][Column(col)].c);
            }
            text.push('\n');
        }
        text
    }

    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.screen().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// 等待子进程退出；返回是否已退出。
    fn wait_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => return false,
            }
        }
        false
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// 独立临时目录与数据库路径。
fn temp_paths(label: &str) -> (PathBuf, PathBuf) {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("lx-tui-trust-{label}-{nanos:x}-{sequence:x}"));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    (workspace, root.join("db").join("lx.db"))
}

#[test]
fn untrusted_startup_shows_console_trust_page_then_enters_on_yes() {
    let (workspace, db_path) = temp_paths("accept");
    let app = App::spawn(&workspace, &db_path);

    // 进入应用前先显示信任页：含路径与询问，且未出现应用界面。
    assert!(
        app.wait_for("lx-tui workspace trust", WAIT),
        "控制台信任页先于应用显示"
    );
    assert!(
        app.wait_for("Do you trust the files in this folder?", WAIT),
        "信任页包含询问"
    );
    assert!(!app.screen().contains("placeholder"), "未回答前不进入应用");

    // 输入 y：放行并进入应用。
    app.send(b"y\n");
    assert!(
        app.wait_for("placeholder", WAIT),
        "信任后进入应用渲染 lx 页"
    );
}

#[test]
fn untrusted_startup_declined_does_not_enter_app() {
    let (workspace, db_path) = temp_paths("decline");
    let mut app = App::spawn(&workspace, &db_path);

    assert!(
        app.wait_for("Do you trust the files in this folder?", WAIT),
        "信任页显示"
    );
    app.send(b"n\n");
    assert!(app.wait_exit(WAIT), "拒绝后进程退出");

    let output = String::from_utf8_lossy(&app.raw()).into_owned();
    assert!(
        output.contains("workspace not trusted; exiting"),
        "输出拒绝提示: {output}"
    );
    assert!(!app.screen().contains("placeholder"), "拒绝后不得进入应用");
}

#[test]
fn arrow_keys_move_selection_and_enter_confirms_no() {
    let (workspace, db_path) = temp_paths("arrows");
    let mut app = App::spawn(&workspace, &db_path);

    assert!(app.wait_for("No, exit", WAIT), "信任页显示两个选项");
    // 默认选中 Yes：下移到 No 并回车 → 拒绝退出。
    app.send(b"\x1b[B\r");
    assert!(app.wait_exit(WAIT), "选择 No 后进程退出");
    assert!(!app.screen().contains("placeholder"), "拒绝后不得进入应用");
}

#[test]
fn mouse_click_selects_option() {
    let (workspace, db_path) = temp_paths("mouse");
    let canonical = std::fs::canonicalize(&workspace).expect("canonical workspace");
    let mut app = App::spawn(&workspace, &db_path);

    assert!(app.wait_for("No, exit", WAIT), "信任页显示两个选项");
    let layout = ui::trust::layout(Rect::new(0, 0, COLS, ROWS), &canonical);
    let no = layout.options[1];
    // SGR 鼠标左键按下 + 释放（1 基坐标）点击 `No, exit`。
    let x = no.x + 1;
    let y = no.y + 1;
    app.send(format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m").as_bytes());
    assert!(app.wait_exit(WAIT), "点击 No 后进程退出");
    assert!(!app.screen().contains("placeholder"), "拒绝后不得进入应用");
}

#[test]
fn mouse_click_yes_enters_app() {
    let (workspace, db_path) = temp_paths("mouse-yes");
    let canonical = std::fs::canonicalize(&workspace).expect("canonical workspace");
    let app = App::spawn(&workspace, &db_path);

    assert!(app.wait_for("Yes, proceed", WAIT), "信任页显示两个选项");
    let layout = ui::trust::layout(Rect::new(0, 0, COLS, ROWS), &canonical);
    let yes = layout.options[0];
    let x = yes.x + 1;
    let y = yes.y + 1;
    app.send(format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m").as_bytes());
    assert!(app.wait_for("placeholder", WAIT), "点击 Yes 后进入应用");
}
