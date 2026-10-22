//! rtk 路由与内置过滤的纯函数测试。

use std::path::PathBuf;

use super::builtin::filter_output;
use super::{Mode, Rtk};

fn external() -> Rtk {
    Rtk::with(Mode::Auto, Some(PathBuf::from("rtk")))
}

#[test]
fn wraps_supported_short_commands() {
    let rtk = external();
    for command in [
        "cargo test",
        "cargo test --locked",
        "cargo clippy --all-targets -- -D warnings",
        "git status",
        "git diff HEAD",
        "git log --oneline",
        "rg foo src",
        "ls -la",
        "find . -name '*.rs'",
        "pnpm test",
        "curl https://example.com",
    ] {
        let prepared = rtk.prepare(command);
        assert_eq!(prepared.command, format!("rtk {command}"), "{command}");
        assert!(prepared.external, "{command}");
        assert_eq!(prepared.original, command);
    }
}

#[test]
fn leaves_unsupported_shapes_untouched() {
    let rtk = external();
    for command in [
        "cargo run",
        "git rebase main",
        "cd src && cargo test",
        "RUST_LOG=debug cargo test",
        "cargo test\necho done",
        "rtk cargo test",
        "time cargo test",
        "sudo ls",
        "echo hi",
        "docker compose up",
        "kubectl logs -f pod",
        "npm run dev",
        "pnpm dev",
        "npx cowsay hi",
        "",
    ] {
        let prepared = rtk.prepare(command);
        assert_eq!(prepared.command, command, "{command:?} 不应被包装");
        assert!(!prepared.external, "{command:?}");
    }
}

#[test]
fn pipes_and_redirections_after_the_first_command_stay_wrapped() {
    let prepared = external().prepare("cargo test 2>&1 | tail -20");
    assert_eq!(prepared.command, "rtk cargo test 2>&1 | tail -20");
    assert!(prepared.external);
}

#[test]
fn builtin_mode_never_uses_external_binary() {
    let rtk = Rtk::with(Mode::Builtin, Some(PathBuf::from("rtk")));
    let prepared = rtk.prepare("cargo test");
    assert_eq!(prepared.command, "cargo test");
    assert!(!prepared.external);
    assert!(rtk.should_filter_builtin(&prepared));
}

#[test]
fn off_mode_is_a_no_op() {
    let rtk = Rtk::with(Mode::Off, Some(PathBuf::from("rtk")));
    let prepared = rtk.prepare("cargo test");
    assert_eq!(prepared.command, "cargo test");
    assert!(!rtk.should_filter_builtin(&prepared));
    assert_eq!(rtk.filter_output("cargo test", "hello\n"), "hello\n");
}

#[test]
fn external_path_with_spaces_is_shell_quoted() {
    let rtk = Rtk::with(Mode::Auto, Some(PathBuf::from("/tmp/my tools/rtk")));
    assert_eq!(rtk.prepare("ls").command, "'/tmp/my tools/rtk' ls");
}

#[test]
fn test_filter_keeps_failures_and_summary_only() {
    let output = "\
   Compiling foo v0.1.0
    Finished `test` profile in 1.2s
     Running unittests src/lib.rs

running 3 tests
test a ... ok
test b ... FAILED
test c ... ok

failures:

---- b stdout ----
thread 'b' panicked at src/lib.rs:1:1:
assertion `left == right` failed
  left: 1
 right: 2

failures:
    b

test result: FAILED. 2 passed; 1 failed; 0 ignored
";
    let filtered = filter_output("cargo test", output);
    assert!(filtered.contains("test b ... FAILED"));
    assert!(filtered.contains("---- b stdout ----"));
    assert!(filtered.contains("left: 1"));
    assert!(filtered.contains("test result: FAILED"));
    assert!(!filtered.contains("Compiling foo"));
    assert!(!filtered.contains("test a ... ok"), "通过的测试行不保留");
}

#[test]
fn successful_test_filter_keeps_only_summary() {
    let output = "\
   Compiling foo v0.1.0
running 2 tests
test a ... ok
test b ... ok
test result: ok. 2 passed; 0 failed
";
    let filtered = filter_output("cargo test", output);
    assert_eq!(filtered.trim(), "test result: ok. 2 passed; 0 failed");
}

#[test]
fn build_filter_drops_progress_but_keeps_diagnostics() {
    let output = "\
   Compiling lx-tui v0.1.0
warning: unused variable: `x`
  --> src/lib.rs:1:5
   |
 1 | let x = 1;
   |     ^ help: use `_x`
   |
    Finished `dev` profile in 1.0s
";
    let filtered = filter_output("cargo check", output);
    assert!(!filtered.contains("Compiling"));
    assert!(filtered.contains("warning: unused variable"));
    assert!(filtered.contains("--> src/lib.rs:1:5"));
    assert!(filtered.contains("Finished"));
}

#[test]
fn status_filter_caps_files_and_reports_omissions() {
    let mut output = String::from("On branch main\nChanges not staged for commit:\n");
    for index in 0..20 {
        output.push_str(&format!(" M src/file{index}.rs\n"));
    }
    output.push_str("Untracked files:\n  (use \"git add <file>...\" to include)\n");
    for index in 0..15 {
        output.push_str(&format!("\tnew{index}.txt\n"));
    }
    let filtered = filter_output("git status", &output);
    assert_eq!(filtered.matches(" M src/file").count(), 15);
    assert_eq!(filtered.matches("\tnew").count(), 10);
    assert!(!filtered.contains("use \"git add"), "提示行被裁掉");
    assert!(filtered.contains("5 more changed files"));
    assert!(filtered.contains("5 more untracked files"));
}

#[test]
fn grep_filter_limits_per_file_and_total() {
    let mut output = String::new();
    for index in 0..40 {
        output.push_str(&format!("src/a.rs:{index}:hit {index}\n"));
    }
    let filtered = filter_output("rg hit src", &output);
    assert_eq!(filtered.matches("src/a.rs:").count(), 25);
    assert!(filtered.contains("15 matches omitted"));
}

#[test]
fn diff_filter_drops_context_lines() {
    let output = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,3 @@
 context
-old
+new
";
    let filtered = filter_output("git diff", output);
    assert!(filtered.contains("diff --git"));
    assert!(filtered.contains("+new"));
    assert!(filtered.contains("-old"));
    assert!(!filtered.contains(" context"));
}

#[test]
fn generic_filter_passes_short_and_trims_long() {
    assert_eq!(filter_output("echo", "hello\n"), "hello\n");
    let long: String = (0..400).map(|n| format!("line {n}\n")).collect();
    let filtered = filter_output("custom-script", &long);
    assert!(filtered.contains("line 0"));
    assert!(filtered.contains("line 399"));
    assert!(filtered.contains("lines omitted"));
}

#[test]
fn ansi_and_control_bytes_are_stripped() {
    let filtered = filter_output("echo", "\u{1b}[31mred\u{1b}[0m\u{7}\n");
    assert_eq!(filtered, "red\n");
}

#[test]
fn empty_output_stays_empty() {
    assert_eq!(filter_output("cargo test", ""), "");
    assert_eq!(filter_output("cargo test", "  \n"), "");
}
