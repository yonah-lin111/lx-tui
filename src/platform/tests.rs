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
