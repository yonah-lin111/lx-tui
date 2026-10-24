//! `/compact` 执行中的聊天栏（A22–A25）：压缩中显示 loading、统计换成本次压缩耗时、
//! 不出现 Esc 停止提示、与「回到底部」共存。
//!
//! 压缩期间聊天请求是空闲的（没有 assistant 流），这一行必须自带可见反馈：
//! 否则用户按下 `/compact` 后界面毫无变化，只能盯着静止画面等摘要。

use super::*;
use crate::app::state::LxCompact;

/// 进入「压缩中」的 lx：有历史（否则聊天栏整行不占位）、空闲请求。
fn compacting_lx() -> Lx {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    // 压缩只在空闲时允许：把请求回到 Idle，并关掉流中标记。
    lx.chat.request = RequestState::Idle;
    lx.chat.status.streaming = false;
    lx.chat.turn_elapsed_ms = None;
    lx.compact = LxCompact::default();
    lx.compact.start("op-compact".into(), 0);
    lx
}

/// A22：压缩中聊天栏可见，文案是 `Compacting context...`，且带扫描高亮。
#[test]
fn compacting_shows_the_loading_row() {
    let lx = compacting_lx();
    assert!(visible(&lx), "压缩中聊天栏必须可见");
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(
        row.contains(text::chat::COMPACTING_LABEL),
        "压缩中文案：{row:?}"
    );
    // 扫描窗口逐帧推进：frame 0 时窗口还在文案左侧之外，跨几帧必然命中。
    let mut highlighted = false;
    for frame in 0..8 {
        let mut lx = compacting_lx();
        lx.chat.loading_frame = frame;
        let (grid, bar) = draw_bar(&lx, 60);
        if row_styles(&grid, bar)
            .iter()
            .any(|style| style.add_modifier.contains(ratatui::style::Modifier::BOLD))
        {
            highlighted = true;
            break;
        }
    }
    assert!(highlighted, "压缩文案同样带扫描高亮");
}

/// A23：统计显示**本次压缩耗时**，不残留上一轮聊天的耗时或输出 token。
#[test]
fn compacting_shows_its_own_elapsed_time() {
    let mut lx = compacting_lx();
    // 上一轮聊天的陈旧统计：压缩期间一个都不许出现。
    lx.chat.turn_elapsed_ms = Some(12_000);
    if let Some(message) = lx.chat.messages.last_mut()
        && let Some(part) = message.steps.first_mut().and_then(|step| step.part_mut())
    {
        part.content = "a".repeat(1_280);
    }
    lx.compact.elapsed_ms = Some(3_200);
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains("(3s)"), "显示本次压缩耗时：{row:?}");
    assert!(!row.contains("12s"), "不得显示上一轮聊天耗时：{row:?}");
    assert!(!row.contains("tok"), "压缩没有输出 token：{row:?}");
}

/// A23 续：耗时未知（刚开始那一帧）时整段统计不画，只留文案。
#[test]
fn compacting_without_elapsed_shows_no_stats() {
    let lx = compacting_lx();
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::COMPACTING_LABEL), "{row:?}");
    assert!(!row.contains('('), "没有耗时时不画空括号：{row:?}");
}

/// A24：压缩中按 Esc 不出现停止提示（压缩走 `/compact` 自己的忙碌语义，不可中断）。
#[test]
fn compacting_never_shows_the_stop_hint() {
    let mut lx = compacting_lx();
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::COMPACTING_LABEL), "{row:?}");
    for phrase in text::LX_CHAT_STOP_PHRASES {
        assert!(!row.contains(phrase), "压缩中不得出现停止提示：{row:?}");
    }
}

/// A25：上滚未跟随时「回到底部」与压缩文案共存（两者各占一端）。
#[test]
fn compacting_coexists_with_jump_bottom() {
    let mut lx = compacting_lx();
    lx.chat.follow_bottom = false;
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::COMPACTING_LABEL), "{row:?}");
    assert!(row.contains(text::LX_JUMP_BOTTOM), "{row:?}");
    assert!(row.contains(text::LX_JUMP_BOTTOM_HINT), "{row:?}");
}

/// A25 续：压缩结束后整行立刻消失（空闲 + 跟随底部），不留残影。
#[test]
fn finishing_compact_hides_the_loading_row() {
    let mut lx = compacting_lx();
    assert!(visible(&lx));
    lx.compact.finish();
    assert!(!visible(&lx), "压缩结束且空闲：整行消失");
}
