//! 集成测试：真实 PTY 中 prompt 选区的复制快捷键（Ctrl+C / Cmd+C）。
//!
//! 覆盖 legacy `\x03`、kitty 协议 `CSI 99;5u`（ctrl+c）与 `CSI 99;9u`（super+c）：
//! 拖选 prompt 文本后按键应触发 `Copied to clipboard` toast。
//! 原生剪贴板写入经临时 PATH 中的 pbcopy 桩拦截，避免污染真实剪贴板。

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Line};
use lx_tui::terminal::Terminal;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 100;
const ROWS: u16 = 30;
/// prompt 面板内容区坐标（0 基），右栏中部空白处。
const PROMPT_X: u16 = 80;
const PROMPT_Y: u16 = 20;

/// 临时 `pbcopy` 桩：把 stdin 落盘到 `copied.txt`，避免测试写真实剪贴板。
struct PbcopyStub {
    dir: PathBuf,
}

impl PbcopyStub {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lx-tui-copy-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create stub dir");
        let script = dir.join("pbcopy");
        fs::write(
            &script,
            "#!/bin/sh\ncat > \"$(dirname \"$0\")/copied.txt\"\n",
        )
        .expect("write stub");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        Self { dir }
    }

    fn path(&self) -> &str {
        self.dir.to_str().expect("stub path is utf8")
    }

    /// 等待并读取最近一次复制内容。
    fn copied(&self, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(text) = fs::read_to_string(self.dir.join("copied.txt")) {
                return Some(text);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for PbcopyStub {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
    fn spawn(stub: &PbcopyStub) -> Self {
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
        command.env(
            "PATH",
            format!(
                "{}:{}",
                stub.path(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
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
        let app = Self {
            child,
            writer,
            emulator,
        };
        assert!(app.wait_for("placeholder", Duration::from_secs(10)));
        app
    }

    fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes).expect("write to pty");
        writer.flush().expect("flush pty");
    }

    fn screen(&self) -> String {
        let terminal = self.emulator.lock().unwrap();
        let grid = terminal.grid();
        let cols = usize::from(terminal.size().cols);
        let rows = usize::from(terminal.size().rows);
        let mut text = String::new();
        for row in 0..rows {
            for col in 0..cols {
                text.push(grid[Line(row as i32)][Column(col)].c);
            }
            text.push('\n');
        }
        text
    }

    /// 屏幕文本中的 (列, 行) 坐标（0 基）。
    fn find(&self, needle: &str) -> Option<(u16, u16)> {
        let terminal = self.emulator.lock().unwrap();
        let grid = terminal.grid();
        let cols = usize::from(terminal.size().cols);
        let rows = usize::from(terminal.size().rows);
        for row in 0..rows {
            let line: String = (0..cols)
                .map(|c| grid[Line(row as i32)][Column(c)].c)
                .collect();
            if let Some(byte) = line.find(needle) {
                return Some((line[..byte].chars().count() as u16, row as u16));
            }
        }
        None
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

    /// 点击 prompt 聚焦，输入文本，并拖选整段。
    fn select_prompt_text(&self, text: &str) {
        self.send(&press(PROMPT_X, PROMPT_Y));
        self.send(&release(PROMPT_X, PROMPT_Y));
        std::thread::sleep(Duration::from_millis(100));
        self.send(text.as_bytes());
        assert!(
            self.wait_for(text, Duration::from_secs(3)),
            "typed text visible: {}",
            self.screen()
        );
        let (x, y) = self.find(text).expect("text position");
        let end = x + text.chars().count() as u16;
        self.send(&press(x, y));
        std::thread::sleep(Duration::from_millis(50));
        self.send(&drag(end, y));
        std::thread::sleep(Duration::from_millis(50));
        self.send(&release(end, y));
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// SGR 鼠标按下（1 基坐标）。
fn press(x: u16, y: u16) -> Vec<u8> {
    format!("\x1b[<0;{};{}M", x + 1, y + 1).into_bytes()
}

/// SGR 鼠标拖动（按住左键移动）。
fn drag(x: u16, y: u16) -> Vec<u8> {
    format!("\x1b[<32;{};{}M", x + 1, y + 1).into_bytes()
}

/// SGR 鼠标释放。
fn release(x: u16, y: u16) -> Vec<u8> {
    format!("\x1b[<0;{};{}m", x + 1, y + 1).into_bytes()
}

#[test]
fn ctrl_c_copies_prompt_selection() {
    let stub = PbcopyStub::new();
    let app = App::spawn(&stub);
    app.select_prompt_text("hello world");
    app.send(b"\x03");
    assert!(
        app.wait_for("Copied to clipboard", Duration::from_secs(3)),
        "ctrl+c 应触发复制 toast：\n{}",
        app.screen()
    );
}

#[test]
fn kitty_ctrl_c_copies_prompt_selection() {
    let stub = PbcopyStub::new();
    let app = App::spawn(&stub);
    app.select_prompt_text("hello world");
    // kitty 键盘协议：CSI 99;5u = 'c' + ctrl。
    app.send(b"\x1b[99;5u");
    assert!(
        app.wait_for("Copied to clipboard", Duration::from_secs(3)),
        "kitty ctrl+c 应触发复制 toast：\n{}",
        app.screen()
    );
}

#[test]
fn kitty_super_c_copies_prompt_selection() {
    let stub = PbcopyStub::new();
    let app = App::spawn(&stub);
    app.select_prompt_text("hello world");
    // kitty 键盘协议：CSI 99;9u = 'c' + super（Ghostty 对 Cmd+C 的编码）。
    app.send(b"\x1b[99;9u");
    assert!(
        app.wait_for("Copied to clipboard", Duration::from_secs(3)),
        "cmd+c 应触发复制 toast：\n{}",
        app.screen()
    );
}

#[test]
fn cmd_shift_c_copies_template_block_without_title() {
    let stub = PbcopyStub::new();
    let app = App::spawn(&stub);
    app.send(&press(PROMPT_X, PROMPT_Y));
    app.send(&release(PROMPT_X, PROMPT_Y));
    std::thread::sleep(Duration::from_millis(100));
    // `/add` 回车插入模板块，光标落在标题占位行（块内）。
    app.send(b"/add");
    std::thread::sleep(Duration::from_millis(100));
    app.send(b"\r");
    assert!(
        app.wait_for("# Add Requirement", Duration::from_secs(3)),
        "template inserted:\n{}",
        app.screen()
    );
    // kitty 键盘协议：CSI 99;10u = 'c' + shift + super（Cmd+Shift+C）。
    app.send(b"\x1b[99;10u");
    assert!(
        app.wait_for("Copied to clipboard", Duration::from_secs(3)),
        "cmd+shift+c 应触发复制 toast：\n{}",
        app.screen()
    );
    let copied = stub
        .copied(Duration::from_secs(3))
        .expect("pbcopy 收到内容");
    assert_eq!(copied, "# Add Requirement", "复制内容不含标题占位行与空项");
}
