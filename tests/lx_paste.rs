//! 集成测试：在真实 PTY 中运行 lx-tui 二进制，验证 lx 输入框粘贴（文本与路径）。

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lx_tui::terminal::Terminal;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

mod common;

const COLS: u16 = 100;
const ROWS: u16 = 30;
/// 等应用启动与重绘的上限；慢机器也够用。
const WAIT: Duration = Duration::from_secs(10);

/// 已启动的测试应用：子进程、PTY 写入端与屏幕仿真器。
struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
    /// 在 PTY 中启动真实二进制并等待 lx 页首帧（输入框占位可见）。
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
        common::isolate_database(&mut command);
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
            wait_for(&emulator, "Ask anything", WAIT),
            "app must render the lx page input placeholder"
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
            text.push(
                grid[alacritty_terminal::index::Line(row as i32)]
                    [alacritty_terminal::index::Column(col)]
                .c,
            );
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

/// 以括号粘贴上报发送文本，触发应用内 `Event::Paste`。
fn bracketed_paste(text: &str) -> Vec<u8> {
    format!("\x1b[200~{text}\x1b[201~").into_bytes()
}

#[test]
fn bracketed_paste_inserts_text_into_lx_input() {
    let app = App::spawn();
    app.send(&bracketed_paste("LXMARKERPASTE"));
    assert!(
        app.wait_for("LXMARKERPASTE"),
        "粘贴文本必须进入 lx 输入框:\n{}",
        screen_text(&app.emulator.lock().unwrap())
    );
}

#[test]
fn bracketed_paste_of_path_opens_reference_panel() {
    let app = App::spawn();
    let dir = env!("CARGO_MANIFEST_DIR");
    app.send(&bracketed_paste(dir));
    assert!(
        app.wait_for("Insert reference"),
        "路径粘贴必须弹出参照面板:\n{}",
        screen_text(&app.emulator.lock().unwrap())
    );
}

#[test]
fn long_paste_wraps_and_shows_input_scrollbar() {
    let app = App::spawn();
    let long = "L".repeat(20000);
    app.send(&bracketed_paste(&long));
    assert!(
        app.wait_for("▕"),
        "输入框内容溢出时必须显示滚动条:\n{}",
        screen_text(&app.emulator.lock().unwrap())
    );
}

#[test]
fn wheel_scrolls_lx_input_viewport() {
    let app = App::spawn();
    let body = format!("HEADMARK{}", "L".repeat(20000));
    app.send(&bracketed_paste(&body));
    assert!(app.wait_for("▕"), "溢出时显示滚动条");
    assert!(
        !screen_text(&app.emulator.lock().unwrap()).contains("HEADMARK"),
        "初始视口贴底，看不到输入开头"
    );
    // 指针停在窗格内（SGR 滚轮上滚）；滚轮路径按窗格滚动 lx 输入视口。
    for _ in 0..300 {
        app.send(b"\x1b[<64;60;20M");
    }
    assert!(
        app.wait_for("HEADMARK"),
        "滚轮上滚后应看到输入开头:\n{}",
        screen_text(&app.emulator.lock().unwrap())
    );
}
