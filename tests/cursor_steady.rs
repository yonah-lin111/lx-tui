//! 集成测试：真实 PTY 中验证不闪烁光标与 lx 页空闲期的光标指令。
//!
//! ratatui 每帧经 `apply_buffer_with_cursor` 重发 `Show` + `MoveTo`；lx 页动画
//! 重绘会不断重置部分终端（Ghostty 等）的光标闪烁相位。这里断言：
//! - 启动即关闭光标闪烁（DEC 模式 12：`\x1b[?12l`），不改变终端光标形状；
//! - lx 页空闲期（prompt 已聚焦）不再重复下发 `Show`/`Hide`。

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Line};
use lx_tui::terminal::Terminal;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 100;
const ROWS: u16 = 30;
const WAIT: Duration = Duration::from_secs(10);
/// 空闲观察窗口（覆盖多个 200ms 动画帧）。
const IDLE: Duration = Duration::from_millis(1200);

/// 已启动的测试应用：子进程、PTY 写入端与原始输出。
struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    raw: Arc<Mutex<Vec<u8>>>,
}

impl App {
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

        assert!(
            wait_for(&emulator, "placeholder", WAIT),
            "app must render the lx page"
        );
        Self { child, writer, raw }
    }

    fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes).expect("write to pty");
        writer.flush().expect("flush pty");
    }

    fn clear_raw(&self) {
        self.raw.lock().unwrap().clear();
    }

    fn raw(&self) -> Vec<u8> {
        self.raw.lock().unwrap().clone()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

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

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// 统计 MoveTo（`\x1b[{r};{c}H`）个数。
fn count_moves(bytes: &[u8]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            let mut cursor = index + 2;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            if cursor > index + 2 && bytes.get(cursor) == Some(&b';') {
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                if cursor > start && bytes.get(cursor) == Some(&b'H') {
                    count += 1;
                    index = cursor + 1;
                    continue;
                }
            }
        }
        index += 1;
    }
    count
}

#[test]
fn init_disables_blink_and_idle_lx_is_quiet() {
    let app = App::spawn();

    assert!(
        count(&app.raw(), b"\x1b[?12l") >= 1,
        "启动即应关闭光标闪烁（DEC 模式 12）"
    );

    // 聚焦 prompt 编辑器（右侧栏内容区），使硬件光标处于可见状态。
    app.send(b"\x1b[<0;81;11M\x1b[<0;81;11m");
    std::thread::sleep(Duration::from_millis(300));

    app.clear_raw();
    std::thread::sleep(IDLE);
    let idle = app.raw();
    assert_eq!(count(&idle, b"\x1b[?25h"), 0, "空闲动画帧不得重复下发 Show");
    assert_eq!(
        count(&idle, b"\x1b[?25l"),
        0,
        "空闲动画帧不得下发 Hide（光标保持可见）"
    );
    assert!(
        count_moves(&idle) < 60,
        "空闲期光标移动序列应有界，实际 {}",
        count_moves(&idle)
    );
}
