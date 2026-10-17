//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn osc52_sequence_uses_bel_and_standard_base64() {
    assert_eq!(
        osc52_output("hello", false, false),
        "\x1b]52;c;aGVsbG8=\x07"
    );
    assert_eq!(osc52_output("", false, false), "\x1b]52;c;\x07");
}

#[test]
fn osc52_sequence_encodes_multibyte_text() {
    assert_eq!(osc52_output("你好", false, false), "\x1b]52;c;5L2g5aW9\x07");
}

#[test]
fn osc52_wraps_for_tmux_and_screen() {
    let plain = "\x1b]52;c;aGVsbG8=\x07";
    let passthrough = format!("\x1bPtmux;\x1b{plain}\x1b\\");
    assert_eq!(
        osc52_output("hello", true, false),
        format!("{plain}{passthrough}")
    );
    assert_eq!(osc52_output("hello", false, true), passthrough);
}

#[cfg(not(windows))]
#[test]
fn disable_sequence_covers_all_known_mouse_modes() {
    let Ok(text) = std::str::from_utf8(DISABLE_MOUSE_REPORTING) else {
        panic!("sequence is ascii");
    };
    for mode in ["1000", "1002", "1003", "1005", "1006", "1015", "1016"] {
        assert!(text.contains(&format!("\x1b[?{mode}l")), "missing {mode}");
    }
}

#[cfg(not(windows))]
#[test]
fn enable_sequence_requests_button_drag_and_move_events() {
    let Ok(text) = std::str::from_utf8(ENABLE_MOUSE_BUTTON_REPORTING) else {
        panic!("sequence is ascii");
    };
    for mode in ["1000", "1002", "1003", "1006"] {
        assert!(text.contains(&format!("\x1b[?{mode}h")), "missing {mode}");
    }
    for mode in ["1005", "1015", "1016"] {
        assert!(
            !text.contains(&format!("\x1b[?{mode}h")),
            "unexpected {mode}"
        );
    }
}

#[test]
fn pointer_shape_sequence_sets_and_resets() {
    assert_eq!(
        pointer_shape_sequence(PointerShape::EwResize, false, false),
        "\x1b]22;ew-resize\x1b\\"
    );
    assert_eq!(
        pointer_shape_sequence(PointerShape::Default, false, false),
        "\x1b]22;default\x1b\\"
    );
}

#[test]
fn pointer_shape_sequence_wraps_for_tmux_and_screen() {
    let plain = "\x1b]22;ew-resize\x1b\\";
    let passthrough = format!("\x1bPtmux;\x1b{plain}\x1b\\");
    assert_eq!(
        pointer_shape_sequence(PointerShape::EwResize, true, false),
        format!("{plain}{passthrough}")
    );
    assert_eq!(
        pointer_shape_sequence(PointerShape::EwResize, false, true),
        passthrough
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn process_cwd_reads_own_process_directory() {
    let cwd = process_cwd(std::process::id()).expect("own cwd is readable");
    let expected = std::env::current_dir().expect("cwd is available");
    // macOS 返回的路径可能带 /private 前缀，比较末段与存在性。
    assert!(cwd.is_dir(), "cwd should be a directory: {cwd:?}");
    assert_eq!(
        cwd.canonicalize().ok(),
        expected.canonicalize().ok(),
        "cwd should match the test process directory"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn process_cwd_returns_none_for_invalid_pid() {
    assert_eq!(process_cwd(u32::MAX), None);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn foreground_process_name_returns_none_for_invalid_pid() {
    assert_eq!(foreground_process_name(u32::MAX), None);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn resolve_process_name_keeps_native_agent_binaries() {
    let argv = vec!["/Users/user/.local/bin/claude".to_string()];
    assert_eq!(
        resolve_process_name("claude", &argv).as_deref(),
        Some("claude")
    );
    let argv = vec!["/opt/homebrew/bin/agy".to_string()];
    assert_eq!(resolve_process_name("agy", &argv).as_deref(), Some("agy"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn resolve_process_name_unwraps_node_script_agents() {
    let argv = vec![
        "node".to_string(),
        "/Users/user/.nvm/versions/node/v24/bin/codex".to_string(),
        "--model".to_string(),
        "gpt-5".to_string(),
    ];
    assert_eq!(
        resolve_process_name("node", &argv).as_deref(),
        Some("codex")
    );

    let argv = vec![
        "node".to_string(),
        "/Users/user/.nvm/versions/node/v24/lib/node_modules/@google/gemini-cli/bundle/gemini.js"
            .to_string(),
    ];
    assert_eq!(
        resolve_process_name("node", &argv).as_deref(),
        Some("gemini")
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn resolve_process_name_prefers_effective_argv0_name() {
    // exec -a codex sleep / nix 包装脚本：comm 与展示名不一致时跟随 argv0。
    let argv = vec!["codex".to_string(), "30".to_string()];
    assert_eq!(
        resolve_process_name("sleep", &argv).as_deref(),
        Some("codex")
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn resolve_process_name_falls_back_for_shells_and_eval_flags() {
    let argv = vec!["-zsh".to_string()];
    assert_eq!(resolve_process_name("zsh", &argv).as_deref(), Some("zsh"));

    let argv = vec!["node".to_string(), "-e".to_string(), "codex".to_string()];
    assert_eq!(resolve_process_name("node", &argv).as_deref(), Some("node"));

    assert_eq!(
        resolve_process_name("vim", &["vim".to_string()]).as_deref(),
        Some("vim")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn parse_procargs2_extracts_argv() {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&2i32.to_ne_bytes());
    buffer.extend_from_slice(b"/usr/bin/env\0\0\0");
    buffer.extend_from_slice(b"node\0");
    buffer.extend_from_slice(b"/Users/user/bin/codex\0");
    assert_eq!(
        macos::parse_procargs2(&buffer),
        vec!["node".to_string(), "/Users/user/bin/codex".to_string()]
    );
    assert!(macos::parse_procargs2(&[]).is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn parse_tpgid_handles_spaces_in_comm() {
    let stat = "123 (weird name) S 1 123 123 0 -1 4194560 0 0 0 0";
    assert_eq!(linux::parse_tpgid(stat), Some(123));
    let stat = "123 (zsh) S 1 123 123 0 -1 4194560 0 0 0 0";
    assert_eq!(linux::parse_tpgid(stat), None);
}

/// 原子写入：覆盖既有文件、内容正确、不留临时文件。
#[test]
fn atomic_write_replaces_content_without_temp_leftovers() {
    let dir = std::env::temp_dir().join(format!("lx-platform-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("a.txt");
    std::fs::write(&path, b"old").expect("seed file");

    write_file_atomic(&path, b"new content").expect("atomic write");
    assert_eq!(std::fs::read(&path).expect("read"), b"new content".to_vec());

    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["a.txt"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 原子写入会沿用既有文件的权限位（Unix）。
#[cfg(unix)]
#[test]
fn atomic_write_preserves_existing_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("lx-platform-perm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("mode.sh");
    std::fs::write(&path, b"#!/bin/sh\n").expect("seed file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    write_file_atomic(&path, b"#!/bin/sh\necho hi\n").expect("atomic write");
    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755, "权限位应保留");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `rename_over` 覆盖目标；目标不存在时直接改名。
#[test]
fn rename_over_replaces_target() {
    let dir = std::env::temp_dir().join(format!("lx-platform-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let from = dir.join("from.txt");
    let to = dir.join("to.txt");
    std::fs::write(&from, b"from").expect("seed from");
    std::fs::write(&to, b"to").expect("seed to");
    rename_over(&from, &to).expect("rename over");
    assert_eq!(std::fs::read(&to).expect("read"), b"from".to_vec());
    assert!(!from.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
