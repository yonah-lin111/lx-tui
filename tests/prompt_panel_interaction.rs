//! PTY 集成测试：prompt 面板真实鼠标/键盘流转。
//!
//! 覆盖用户反馈路径：[/] 在无选中条目（含空树）时回退打开草稿、
//! 工具栏 [+] 进入草稿编辑、保存点 → 文件夹选择器落库、
//! 树视图搜索框（模态输入对齐）输入即过滤与 Esc 清空、条目右键 CRUD（重命名）。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alacritty_terminal::index::{Column, Line};
use lx_tui::terminal::Terminal;
use lx_tui::ui::text;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 100;
const ROWS: u16 = 30;
const WAIT: Duration = Duration::from_secs(10);

struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
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

    /// 在包含 `anchor` 的行内查找 `needle`（避开其他行的同名按钮，如侧栏/标签栏的 `[+]`）。
    fn find_in_row_of(&self, anchor: &str, needle: &str) -> Option<(u16, u16)> {
        let screen = self.screen();
        let width = needle.chars().count();
        for (row, line) in screen.lines().enumerate() {
            if !line.contains(anchor) {
                continue;
            }
            if let Some(byte_index) = line.find(needle) {
                let col = line[..byte_index].chars().count() + width / 2;
                return Some((col as u16, row as u16));
            }
        }
        None
    }

    /// 点击包含 `anchor` 的行内的 `needle`。
    fn click_in_row_of(&self, anchor: &str, needle: &str) {
        let (col, row) = self
            .find_in_row_of(anchor, needle)
            .unwrap_or_else(|| panic!("row with {anchor:?} missing {needle:?}\n{}", self.screen()));
        self.click_at(col, row);
    }

    /// 在包含 `anchor` 的行内、`anchor` 之后查找 `needle`
    /// （同行同名符号消歧，如钉住按钮 `[○]` 与草稿保存点 `○`）。
    fn find_after(&self, anchor: &str, needle: &str) -> Option<(u16, u16)> {
        let screen = self.screen();
        let width = needle.chars().count();
        for (row, line) in screen.lines().enumerate() {
            let Some(anchor_byte) = line.find(anchor) else {
                continue;
            };
            let start = anchor_byte + anchor.len();
            let Some(byte_index) = line[start..].find(needle) else {
                continue;
            };
            let col = line[..start + byte_index].chars().count() + width / 2;
            return Some((col as u16, row as u16));
        }
        None
    }

    /// 点击草稿选择器内的根条目行 `/`（避开标题行页控件的 `1/1`）。
    fn click_save_draft_root(&self) {
        let screen = self.screen();
        let rows: Vec<&str> = screen.lines().collect();
        let start = rows
            .iter()
            .position(|line| line.contains("Save prompt to folder"))
            .unwrap_or(0);
        for (row, line) in rows.iter().enumerate().skip(start) {
            if line.contains("1/1") || !line.contains('/') {
                continue;
            }
            let byte = line.find('/').expect("slash");
            let col = line[..byte].chars().count();
            self.click_at(col as u16, row as u16);
            return;
        }
        panic!("root entry row missing\n{}", self.screen());
    }

    /// 点击包含 `anchor` 的行内、`anchor` 之后的 `needle`。
    fn click_after(&self, anchor: &str, needle: &str) {
        let (col, row) = self
            .find_after(anchor, needle)
            .unwrap_or_else(|| panic!("{anchor:?} 之后缺少 {needle:?}\n{}", self.screen()));
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
    let app = App::spawn(&workspace, &db_path);
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

    // [≡] → 树视图：搜索框、[+] 按钮与 default temp 行（列表恒有 default temp）。
    app.click(text::PROMPT_MODE_LIST_LABEL);
    assert!(
        app.wait_for(text::PROMPT_TEMP_DEFAULT_NAME),
        "tree: \n{}",
        app.screen()
    );
    assert!(
        app.find_in_row_of(text::PROMPT_TEMP_DEFAULT_NAME, text::AGENT_SELECTED_MARKER)
            .is_some(),
        "default temp row with marker: \n{}",
        app.screen()
    );
    assert!(
        app.find_in_row_of(text::PROMPT_TEMP_DEFAULT_NAME, "[tmp]")
            .is_none(),
        "temp rows no longer carry the [tmp] badge: \n{}",
        app.screen()
    );
    assert!(
        app.wait_for(text::PROMPT_MODE_EDIT_LABEL),
        "mode button flips"
    );
    assert!(
        app.find_in_row_of(text::PROMPT_MODE_EDIT_LABEL, text::PROMPT_TEMP_PROMPT_LABEL)
            .is_some(),
        "temp prompt button listed: \n{}",
        app.screen()
    );

    // 空树点 [/]：回退打开草稿编辑视图，不再无响应。
    app.click(text::PROMPT_MODE_EDIT_LABEL);
    assert!(
        app.wait_for(text::PROMPT_UNDO_LABEL),
        "editor reentered: \n{}",
        app.screen()
    );

    // 输入草稿内容后点保存点 → 文件夹选择器。
    app.send(b"hello draft");
    assert!(app.wait_for("hello draft"), "draft typed");
    app.click_after(text::PROMPT_BLOCK_FOCUS_ALL_LABEL, text::PROMPT_DRAFT_DOT);
    assert!(
        app.wait_for("Save prompt to folder"),
        "picker: \n{}",
        app.screen()
    );
    assert!(
        app.wait_for(&format!("{} New prompt", text::INPUT_PREFIX_ICON)),
        "name input listed"
    );
    assert!(app.wait_for("+ New folder"), "new folder option listed");
    // 点击文件夹仅选中，不执行：选择器保持打开。
    app.click_save_draft_root();
    app.settle();
    assert!(
        app.screen().contains("Save prompt to folder"),
        "click selects only: \n{}",
        app.screen()
    );
    // 点击保存按钮（文案里的 Enter 用 `↵` 符号）才执行保存。
    app.click("[save ↵]");
    assert!(
        app.wait_gone("Save prompt to folder"),
        "picker closed after confirm: \n{}",
        app.screen()
    );

    // 回到树：出现正式条目行，不再有状态标签。
    app.click(text::PROMPT_MODE_LIST_LABEL);
    assert!(app.wait_for("• New prompt"), "item row: \n{}", app.screen());
    assert!(
        app.find_in_row_of("• New prompt", "[todo]").is_none(),
        "item rows carry no status label: \n{}",
        app.screen()
    );

    // 输入即过滤：命中保持可见，无命中显示占位；Ctrl+C 清空恢复，Esc 返回编辑视图。
    app.send(b"new");
    app.settle();
    assert!(app.screen().contains("• New prompt"), "match visible");
    app.send(b"zz");
    assert!(
        app.wait_for("no matching prompts"),
        "no matches: \n{}",
        app.screen()
    );
    app.send(b"\x03");
    assert!(app.wait_for("• New prompt"), "ctrl+c clears filter");
    app.send(b"\x1b");
    assert!(
        app.wait_for(text::PROMPT_UNDO_LABEL),
        "esc returns to editor: \n{}",
        app.screen()
    );
    // 回到列表继续后续断言。
    app.click(text::PROMPT_MODE_LIST_LABEL);
    assert!(
        app.wait_for("• New prompt"),
        "list reentered: \n{}",
        app.screen()
    );

    // 条目右键菜单 → 重命名。
    let (col, row) = app.find("• New prompt").expect("item row");
    app.right_click(col, row);
    assert!(app.wait_for("Rename"), "item menu: \n{}", app.screen());
    app.click("Rename");
    assert!(app.wait_for("Rename prompt"), "rename overlay");
    app.send(b"X\r");
    assert!(
        app.wait_for("New promptX"),
        "renamed item: \n{}",
        app.screen()
    );

    // 点击工具栏 [+] → 草稿编辑视图；命名浮层新建文件夹保存全流程。
    app.click_in_row_of(text::PROMPT_MODE_EDIT_LABEL, text::PROMPT_TEMP_PROMPT_LABEL);
    assert!(
        app.wait_for("Write your prompt"),
        "draft editor reopened: \n{}",
        app.screen()
    );
    app.send(b"second draft");
    assert!(app.wait_for("second draft"), "second draft typed");
    app.click_after(text::PROMPT_BLOCK_FOCUS_ALL_LABEL, text::PROMPT_DRAFT_DOT);
    assert!(app.wait_for("+ New folder"), "picker reopened");
    // 点击动作行 → 命名浮层；清空预填名后输入 notes，回车建库并恢复选择器（选中新文件夹）。
    app.click("+ New folder");
    assert!(
        app.wait_for("New folder"),
        "naming dialog: \n{}",
        app.screen()
    );
    app.send(b"\x03");
    app.send(b"notes");
    assert!(app.wait_for("notes"), "folder name typed");
    app.send(b"\r");
    assert!(
        app.wait_for("Save prompt to folder"),
        "picker restored: \n{}",
        app.screen()
    );
    app.click("[save ↵]");
    assert!(
        app.wait_gone("Save prompt to folder"),
        "draft saved into new folder: \n{}",
        app.screen()
    );
    app.click(text::PROMPT_MODE_LIST_LABEL);
    assert!(
        app.wait_for("notes"),
        "new folder visible in tree: \n{}",
        app.screen()
    );
}

#[test]
fn page_controls_create_switch_and_rename_via_real_mouse_and_keys() {
    let app = spawn_trusted("pages");

    // 默认绑定空 temp：标题行显示 `1/1` 页控件。
    assert!(
        app.wait_for("1/1"),
        "title page counter: \n{}",
        app.screen()
    );
    app.click("Write your");
    app.settle();
    app.send(b"page one body");
    assert!(app.wait_for("page one body"), "first page typed");

    // 点击计数打开页码菜单：New page / Delete page 与页行。
    app.click("1/1");
    assert!(
        app.wait_for(text::MENU_PAGE_NEW),
        "page menu: \n{}",
        app.screen()
    );
    assert!(app.wait_for(text::MENU_PAGE_DELETE));

    // 点击 New page：追加末尾、立即切换并进入默认名编辑
    // （正文为空；输入落入页名而不是编辑器）。
    app.click(text::MENU_PAGE_NEW);
    assert!(
        app.wait_for("2/2"),
        "switched to new page: \n{}",
        app.screen()
    );
    app.send(b"XYZ");
    app.send(b"\x1b");
    app.settle();
    assert!(
        app.screen().contains("Write your prompt"),
        "重命名输入不写入正文，新页正文为空: \n{}",
        app.screen()
    );
    assert!(!app.screen().contains("page one body"), "新页正文为空");

    // 左箭头回第一页：旧正文保留。
    app.click(text::PROMPT_PAGE_PREV);
    assert!(app.wait_for("1/2"), "back to page 1: \n{}", app.screen());
    assert!(
        app.wait_for("page one body"),
        "page one content kept: \n{}",
        app.screen()
    );
}

#[test]
fn block_focus_survives_prompt_list_roundtrip_via_real_mouse() {
    let app = spawn_trusted("block-focus-roundtrip");
    assert!(
        app.wait_for("Write your prompt"),
        "app started: \n{}",
        app.screen()
    );
    // 启动时焦点在主区；先点 prompt 文本区，确保粘贴进入编辑器而不是 lx 输入。
    app.click("Write your prompt");
    let body = "intro\n&&& a --start\n{title: A}\ninside-a\n&&& a --end\n\
                &&& b --start\ninside-b\n&&& b --end\noutro";
    app.send(format!("\x1b[200~{body}\x1b[201~").as_bytes());
    assert!(app.wait_for("outro"), "text typed: \n{}", app.screen());

    app.click(text::PROMPT_BLOCK_FOCUS_ALL_LABEL);
    assert!(
        app.wait_for(text::PROMPT_BLOCK_FOCUS_ONE_LABEL),
        "进入块聚焦: \n{}",
        app.screen()
    );
    assert!(app.wait_for("inside-a"), "当前块可见");
    assert!(!app.screen().contains("inside-b"), "块外文本不渲染");

    app.click(text::PROMPT_MODE_LIST_LABEL);
    assert!(
        app.wait_for(text::PROMPT_MODE_EDIT_LABEL),
        "进入列表: \n{}",
        app.screen()
    );
    app.click(text::PROMPT_MODE_EDIT_LABEL);
    assert!(
        app.wait_for(text::PROMPT_BLOCK_FOCUS_ONE_LABEL),
        "返回编辑仍处于块聚焦: \n{}",
        app.screen()
    );
    assert!(app.wait_for("inside-a"), "当前块仍可见");
    assert!(
        !app.screen().contains("inside-b"),
        "列表往返后不退回全文: \n{}",
        app.screen()
    );
}
