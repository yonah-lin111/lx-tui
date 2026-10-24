//! 状态栏动态文案的净化与回退测试：控制字符、空白折叠、空文案回退。
//!
//! 只测纯函数，不依赖终端或布局。

use super::*;

/// 中文原文不被改动（不需要净化的常见路径）。
#[test]
fn plain_todo_text_is_borrowed_unchanged() {
    let label = single_line_label("修复输入框布局");
    assert_eq!(label, "修复输入框布局");
    assert!(matches!(label, Cow::Borrowed(_)), "常见路径不应每帧分配");
}

/// 换行 / Tab / CR 一律折叠成一个空格并去首尾空白。
#[test]
fn control_characters_collapse_to_single_spaces() {
    assert_eq!(
        single_line_label("修复\n输入框\t布局\r\n"),
        "修复 输入框 布局"
    );
    assert_eq!(single_line_label("  a \t b  "), "a b");
    assert_eq!(single_line_label("a\rb"), "a b");
    assert_eq!(single_line_label("a\u{1b}[31mb"), "a [31mb");
}

/// 净化只作用于展示副本：空文案回退为兜底 loading 文案，不留空白动画。
#[test]
fn blank_todo_falls_back_to_working_label() {
    assert_eq!(single_line_label(""), LOADING_LABEL);
    assert_eq!(single_line_label("   \n\t\r\n "), LOADING_LABEL);
}

/// 零宽字符本身不占列：整串没有可见列时同样回退。
#[test]
fn zero_width_only_text_falls_back_to_working_label() {
    assert_eq!(single_line_label("\u{200b}\u{200d}"), LOADING_LABEL);
    // 与可见字符混排时保留原文（渲染层按列宽决定是否绘制）。
    assert_eq!(single_line_label("修复\u{200b}布局"), "修复\u{200b}布局");
}

/// 阶段文案是固定英文短语，测试常量本身防止后续改动跑偏。
#[test]
fn stage_labels_are_short_fixed_phrases() {
    assert_eq!(LOADING_LABEL, "Working...");
    assert_eq!(THINKING_LABEL, "Thinking...");
    assert_eq!(RUNNING_TOOLS_LABEL, "Running tools...");
}
