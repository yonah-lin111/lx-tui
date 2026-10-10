//! PTY 集成测试：prompt 面板真实鼠标/键盘流转。
//!
//! 覆盖用户反馈路径：[edit] 在无选中条目（含空树）时回退打开草稿、
//! 工具栏 [temp prompt] 进入草稿编辑、保存点 → 文件夹选择器落库、
//! 树视图搜索框（模态输入对齐）输入即过滤与 Esc 清空、条目右键 CRUD（重命名）。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alacritty_terminal::index::{Column, Line};
use lx_tui::terminal::Terminal;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

static DB_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

const COLS: u16 = 100;
const ROWS: u16 = 30;
const WAIT: Duration = Duration::from_secs(10);

struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
    fn spawn(cwd: &Path) -> Self {
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
        command.env("LX_TUI_DB_PATH", DB_PATH.get().expect("db path"));
        command.cwd(cwd);
        let child = pair.slave.spawn_command(command).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("writer")));
        let emulator = Arc::new(Mutex::new(Terminal::new(COLS, ROWS)));
        let sink = Arc::clone(&emulator);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        sink.lock().unwrap().feed(&buffer[..n]);
                    }
                }
            }
        });
        Self {
            child,
            writer,
            emulator,
        }
    }

    fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes).expect("write");
        writer.flush().expect("flush");
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

    fn find(&self, needle: &str) -> Option<(u16, u16)> {
        let terminal = self.emulator.lock().unwrap();
        let grid = terminal.grid();
        let cols = usize::from(terminal.size().cols);
        let rows = usize::from(terminal.size().rows);
        let width = needle.chars().count();
        for row in 0..rows {
            let mut line = String::new();
            for col in 0..cols {
                line.push(grid[Line(row as i32)][Column(col)].c);
            }
            if let Some(byte_index) = line.find(needle) {
                let col = line[..byte_index].chars().count() + width / 2;
                return Some((col as u16, row as u16));
            }
        }
        None
    }

    fn click_at(&self, col: u16, row: u16) {
        let x = col + 1;
        let y = row + 1;
        self.send(format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m").as_bytes());
    }

    fn click(&self, needle: &str) {
        let (col, row) = self
            .find(needle)
            .unwrap_or_else(|| panic!("screen missing {needle:?}\n{}", self.screen()));
        self.click_at(col, row);
    }

    fn right_click(&self, col: u16, row: u16) {
        let x = col + 1;
        let y = row + 1;
        self.send(format!("\x1b[<2;{x};{y}M\x1b[<2;{x};{y}m").as_bytes());
    }

    fn wait_for(&self, needle: &str) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.screen().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn wait_gone(&self, needle: &str) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if !self.screen().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn settle(&self) {
        std::thread::sleep(Duration::from_millis(250));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn temp_root(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lx-tui-ptm-{label}-{nanos:x}-{sequence:x}"))
}

/// 启动已信任工作区中的被测二进制（隔离数据库，预信任 cwd）。
fn spawn_trusted(label: &str) -> App {
    let root = temp_root(label);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let workspace = std::fs::canonicalize(&workspace).expect("canonical");
    let db_path = root.join("db").join("lx.db");
    let database = lx_tui::db::Database::open(&db_path).expect("db opens");
    database
        .trust_workspace(&workspace, "workspace")
        .expect("trusted");
    drop(database);
    let _ = DB_PATH.set(db_path);
    let app = App::spawn(&workspace);
    assert!(
        app.wait_for("Write your prompt"),
        "app started: {}",
        app.screen()
    );
    app
}

#[test]
fn draft_save_search_and_rename_via_real_mouse_and_keys() {
    let app = spawn_trusted("flow");

    // [list] → 树视图：搜索框、[temp prompt] 按钮与空状态。
    app.click("[list]");
    assert!(app.wait_for("no prompts yet"), "tree: \n{}", app.screen());
    assert!(app.wait_for("[edit]"), "mode button flips");
    assert!(app.wait_for("[temp prompt]"), "temp prompt button listed");

    // 空树点 [edit]：回退打开草稿编辑视图，不再无响应。
    app.click("[edit]");
    assert!(
        app.wait_for("[undo]"),
        "editor reentered: \n{}",
        app.screen()
    );

    // 输入草稿内容后点保存点 → 文件夹选择器。
    app.send(b"hello draft");
    assert!(app.wait_for("hello draft"), "draft typed");
    app.click("●");
    assert!(
        app.wait_for("save prompt to folder"),
        "picker: \n{}",
        app.screen()
    );
    assert!(app.wait_for("name"), "name input listed");
    assert!(app.wait_for("+ New folder"), "new folder option listed");
    // 点击文件夹仅选中，不执行：选择器保持打开。
    app.click("/");
    app.settle();
    assert!(
        app.screen().contains("save prompt to folder"),
        "click selects only: \n{}",
        app.screen()
    );
    // 点击 [save enter] 才执行保存。
    app.click("[save enter]");
    assert!(
        app.wait_gone("save prompt to folder"),
        "picker closed after confirm: \n{}",
        app.screen()
    );

    // 回到树：出现正式条目行与状态标签。
    app.click("[list]");
    assert!(app.wait_for("• New prompt"), "item row: \n{}", app.screen());
    assert!(app.wait_for("[todo]"), "status label");

    // 输入即过滤：命中保持可见，无命中显示占位；Esc 清空恢复。
    app.send(b"new");
    app.settle();
    assert!(app.screen().contains("• New prompt"), "match visible");
    app.send(b"zz");
    assert!(
        app.wait_for("no matching prompts"),
        "no matches: \n{}",
        app.screen()
    );
    app.send(b"\x1b");
    assert!(app.wait_for("• New prompt"), "esc clears filter");

    // 条目右键菜单 → 重命名。
    let (col, row) = app.find("• New prompt").expect("item row");
    app.right_click(col, row);
    assert!(app.wait_for("Rename"), "item menu: \n{}", app.screen());
    app.click("Rename");
    assert!(app.wait_for("rename prompt"), "rename overlay");
    app.send(b"X\r");
    assert!(
        app.wait_for("New promptX"),
        "renamed item: \n{}",
        app.screen()
    );

    // 点击工具栏 [temp prompt] → 草稿编辑视图；内联新建文件夹保存全流程。
    app.click("[temp prompt]");
    assert!(
        app.wait_for("Write your prompt"),
        "draft editor reopened: \n{}",
        app.screen()
    );
    app.send(b"second draft");
    assert!(app.wait_for("second draft"), "second draft typed");
    app.click("●");
    assert!(app.wait_for("+ New folder"), "picker reopened");
    // 点击动作行 → 内联新建行（名称直接在该行输入）。
    app.click("+ New folder");
    app.settle();
    app.send(b"\x03");
    app.send(b"notes");
    app.settle();
    app.click("[save enter]");
    assert!(
        app.wait_gone("save prompt to folder"),
        "inline folder saved: \n{}",
        app.screen()
    );
    app.click("[list]");
    assert!(
        app.wait_for("notes"),
        "new folder visible in tree: \n{}",
        app.screen()
    );
}
