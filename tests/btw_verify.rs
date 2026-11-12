//! Manual E2E verification harness for `/btw` against a CLI-created session.
//!
//! Seed (one-time, real model):
//!   WS=$(mktemp -d); DB=$(mktemp -d)/lx.db
//!   printf 'BTW_VERIFY_TOKEN=42\n' > "$WS/note.txt"
//!   LX_TUI_DB_PATH="$DB" lx-tui workspace add --trust -C "$WS"
//!   LX_TUI_DB_PATH="$DB" lx-tui exec -C "$WS" \
//!     "Call spawn_agent with role explorer and task: 'Read note.txt in the workspace root and report the exact value after BTW_VERIFY_TOKEN=.' Then call wait_agent ... reply with exactly SUBAGENT_VERIFIED."
//!   LX_TUI_DB_PATH="$DB" lx-tui session rename -C "$WS" --last "BTW verify"
//!
//! Run with:
//!   BTW_VERIFY_WS="$WS" BTW_VERIFY_DB="$DB" cargo test --test btw_verify -- --ignored --nocapture
//!
//! The test is intentionally ignored: it uses the real configured model and a
//! session seeded by `lx-tui exec`, so it is not part of the normal test suite.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lx_tui::terminal::Terminal;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 120;
const ROWS: u16 = 40;
const WAIT: Duration = Duration::from_secs(120);

struct App {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emulator: Arc<Mutex<Terminal>>,
}

impl App {
    fn spawn() -> Self {
        let workspace = std::env::var("BTW_VERIFY_WS").expect("BTW_VERIFY_WS");
        let db = std::env::var("BTW_VERIFY_DB").expect("BTW_VERIFY_DB");
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
        command.env("LX_TUI_DB_PATH", db);
        command.cwd(workspace);
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
            "app did not render the lx input placeholder:\n{}",
            screen_text(&emulator.lock().unwrap())
        );
        Self {
            child,
            writer,
            emulator,
        }
    }

    fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes).expect("write to pty");
        writer.flush().expect("flush pty");
    }

    fn type_text(&self, text: &str) {
        self.send(text.as_bytes());
    }

    fn wait_for(&self, needle: &str) -> bool {
        wait_for(&self.emulator, needle, WAIT)
    }

    fn assert_wait(&self, needle: &str) {
        assert!(
            self.wait_for(needle),
            "screen did not contain {needle:?}:\n{}",
            screen_text(&self.emulator.lock().unwrap())
        );
    }

    fn screen(&self) -> String {
        screen_text(&self.emulator.lock().unwrap())
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

fn wait_for(emulator: &Arc<Mutex<Terminal>>, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if screen_text(&emulator.lock().unwrap()).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// 读取验证库里的单值；仅用于 ignored E2E，依赖系统 `sqlite3`。
fn sqlite(db_path: &str, sql: &str) -> String {
    let output = std::process::Command::new("sqlite3")
        .arg(db_path)
        .arg(sql)
        .output()
        .expect("run sqlite3");
    assert!(
        output.status.success(),
        "sqlite3 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
#[ignore = "real model E2E; requires BTW_VERIFY_WS and BTW_VERIFY_DB"]
fn btw_chat_and_agent_mention_e2e() {
    let app = App::spawn();

    // /resume opens the history panel; the seeded session is the newest entry.
    app.type_text("/resume");
    app.send(b"\r");
    app.assert_wait("BTW verify");
    app.send(b"\r");
    app.assert_wait("SUBAGENT_VERIFIED");

    // Open BTW. Tab accepts `/btw` and leaves `[prompt]` in the main composer;
    // wait for that state, then type the argument and submit.
    app.type_text("/btw");
    app.send(b"\t");
    app.assert_wait("[prompt]");
    app.type_text("Reply with exactly SIDE_OK.");
    app.send(b"\r");
    app.assert_wait("QA 1");
    app.assert_wait("SIDE_OK");

    // @Turing: type the question first, then append the target at the end.
    // Typing after confirming a mention can re-trigger the refer shortcut panel.
    app.type_text("What token did you find? Reply with the token only. ");
    app.type_text("@");
    app.assert_wait("Files");
    app.send(b"\x1b[C\x1b[C\x1b[C\x1b[C");
    app.assert_wait("Subagents");
    app.type_text("Turing");
    app.assert_wait("Turing");
    app.send(b"\r");
    std::thread::sleep(Duration::from_secs(1));
    app.send(b"\r");
    app.assert_wait("42");

    // Read-only smoke: ask BTW to mutate a file; the file must remain untouched.
    app.type_text("Write 99 into note.txt. Use the write tool.");
    app.send(b"\r");
    // The previous @ target (Turing) must persist for this follow-up. Compare
    // the *last* assistant label: if the follow-up fell back to lx, the final
    // label before the composer would be `lx [lx]` instead of Turing.
    let db_path = std::env::var("BTW_VERIFY_DB").expect("BTW_VERIFY_DB");
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline
        && sqlite(
            &db_path,
            "select status from agent_btw_batch order by created_at_ms desc limit 1;",
        ) != "completed"
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        sqlite(
            &db_path,
            "select status from agent_btw_batch order by created_at_ms desc limit 1;"
        ),
        "completed",
        "read-only follow-up did not finish"
    );
    assert_eq!(
        sqlite(
            &db_path,
            "select target_kind from agent_btw_recipient order by rowid desc limit 1;"
        ),
        "subagent",
        "follow-up lost the previous @ target"
    );
    let workspace = std::env::var("BTW_VERIFY_WS").expect("BTW_VERIFY_WS");
    let note = std::fs::read_to_string(std::path::Path::new(&workspace).join("note.txt"))
        .expect("note.txt");
    assert!(
        note.contains("42") && !note.contains("99"),
        "BTW must be read-only; note.txt changed to {note:?}"
    );

    // Scroll the BTW conversation up and verify the shared jump-to-bottom
    // control appears; click it and verify it disappears at the bottom.
    for _ in 0..12 {
        app.send(b"\x1b[<64;40;15M");
    }
    app.assert_wait("jump to bottom");
    let screen = app.screen();
    let (row, line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("jump to bottom"))
        .expect("jump button line");
    let col = line.chars().take_while(|ch| *ch != '↓').count();
    app.send(format!("\x1b[<0;{};{}M", col + 1, row + 1).as_bytes());
    app.send(format!("\x1b[<0;{};{}m", col + 1, row + 1).as_bytes());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && app.screen().contains("jump to bottom") {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !app.screen().contains("jump to bottom"),
        "jump-to-bottom click did not return to the bottom:\n{}",
        app.screen()
    );

    // `/btwList` in the BTW composer opens the second-level QA panel.
    app.type_text("/btwList");
    app.send(b"\t");
    app.assert_wait("Side questions");
    app.assert_wait("QA 1");
    app.send(b"\x1b");
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && app.screen().contains("Side questions") {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !app.screen().contains("Side questions"),
        "esc did not close the /btwList panel:\n{}",
        app.screen()
    );

    // Keep the screen in the panic message for manual inspection.
    println!("FINAL SCREEN:\n{}", app.screen());
}
