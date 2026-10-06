//! 集成测试：在真实 PTY 中运行 lx-tui 二进制，验证终端窗格滚轮与边缘自动滚动。

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Line};
use lx_tui::terminal::Terminal;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 100;
const ROWS: u16 = 30;
/// 等应用启动、shell 输出与滚轮重绘的上限；慢机器也够用。
const WAIT: Duration = Duration::from_secs(10);
/// 终端窗格内容区的 SGR 坐标（1 基）：主区首列与其内部首行。
const PANE_X: u16 = 30;
const PANE_Y: u16 = 10;
/// 内容区顶行（SGR 1 基），用于拖拽到边缘。
const PANE_TOP_Y: u16 = 3;

/// 已启动的测试应用：子进程、PTY 写入端与屏幕仿真器。
struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
    /// 在 PTY 中启动真实二进制并等待首帧。
    fn spawn() -> Self {
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
        command.cwd(env!("CARGO_MANIFEST_DIR"));
        let child = pair.slave.spawn_command(command).expect("spawn lx-tui");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("pty reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("pty writer")));
        let emulator = Arc::new(Mutex::new(Terminal::new(COLS, ROWS)));
        let sink = Arc::clone(&emulator);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        sink.lock().unwrap().feed(&buffer[..read]);
                    }
                }
            }
        });

        assert!(
            wait_for(&emulator, "[exit]", WAIT),
            "app must render the exit button"
        );
        // 焦点默认在终端窗格：键入 200 行编号输出。
        send(
            &writer,
            b"awk 'BEGIN{for(i=1;i<=200;i++)printf \"WHEELTEST-%03d\\n\", i}'\r",
        );
        assert!(
            wait_for(&emulator, "WHEELTEST-200", WAIT),
            "shell output must reach the pane"
        );

        Self {
            child,
            writer,
            emulator,
        }
    }

    fn send(&self, bytes: &[u8]) {
        send(&self.writer, bytes);
    }

    fn wait_for(&self, needle: &str) -> bool {
        wait_for(&self.emulator, needle, WAIT)
    }

    /// 向上滚 `notches` 格（SGR 滚轮上报，每格由应用滚动 3 行）。
    fn wheel_up(&self, notches: usize) {
        for _ in 0..notches {
            self.send(format!("\x1b[<64;{PANE_X};{PANE_Y}M").as_bytes());
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// 读取仿真器当前视口的纯文本。
fn screen_text(terminal: &Terminal) -> String {
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

/// 轮询等待屏幕上出现指定文本。
fn wait_for(emulator: &Arc<Mutex<Terminal>>, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if screen_text(&emulator.lock().unwrap()).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn send(writer: &Arc<Mutex<Box<dyn Write + Send>>>, bytes: &[u8]) {
    let mut writer = writer.lock().unwrap();
    writer.write_all(bytes).expect("write to pty");
    writer.flush().expect("flush pty");
}

#[test]
fn wheel_scrolls_terminal_backlog_in_real_binary() {
    let app = App::spawn();
    app.wheel_up(15);
    assert!(
        app.wait_for("WHEELTEST-150"),
        "wheel up must scroll into the pane backlog"
    );
}

#[test]
fn wheel_during_selection_scrolls_terminal_backlog() {
    let app = App::spawn();
    // 在终端窗格按下并保持（开始选区），再滚轮：选区进行中仍应滚动。
    app.send(format!("\x1b[<0;{PANE_X};{PANE_Y}M").as_bytes());
    app.wheel_up(15);
    assert!(
        app.wait_for("WHEELTEST-150"),
        "wheel during selection must scroll the pane"
    );
}

#[test]
fn drag_to_top_edge_autoscrolls_terminal_backlog() {
    let app = App::spawn();
    // 按下后拖动到内容区顶行：登记自动滚动，由 30ms 定时器持续回滚。
    app.send(format!("\x1b[<0;{PANE_X};{PANE_Y}M").as_bytes());
    app.send(format!("\x1b[<32;{PANE_X};{PANE_TOP_Y}M").as_bytes());
    assert!(
        app.wait_for("WHEELTEST-150"),
        "edge drag must autoscroll the pane backlog"
    );
}
