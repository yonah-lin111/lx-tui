//! 用户消息底色：主会话 / 子 agent steer / 私聊 / BTW 必须互不相同。

use super::super::paint::user_message_bg;
use super::*;
use crate::ui::main_content::lx::history::HistorySurface;

#[test]
fn backgrounds_distinguish_surface_and_message_origin() {
    let main = user_message_bg(HistorySurface::Main, false, false, false);
    let steer = user_message_bg(HistorySurface::Subagent, false, true, false);
    let private = user_message_bg(HistorySurface::Subagent, false, false, true);
    let btw = user_message_bg(HistorySurface::Btw, false, false, false);
    assert_eq!(main.bg, style::chat_user_bg().bg, "主会话保持原底色");
    assert_eq!(steer.bg, style::chat_user_steer_bg().bg);
    assert_eq!(private.bg, style::chat_user_private_bg().bg);
    assert_eq!(btw.bg, style::chat_user_btw_bg().bg);

    let colors = [main.bg, steer.bg, private.bg, btw.bg];
    for left in 0..colors.len() {
        for right in left + 1..colors.len() {
            assert_ne!(colors[left], colors[right], "四种底色不得重叠");
        }
    }
}

#[test]
fn rewind_selection_still_wins_over_surface() {
    for surface in [
        HistorySurface::Main,
        HistorySurface::Subagent,
        HistorySurface::Btw,
    ] {
        let selected = user_message_bg(surface, true, true, true);
        assert_eq!(selected.bg, style::chat_user_target_bg().bg);
    }
}

#[test]
fn main_chat_ignores_private_and_btw_never_uses_steer_colors() {
    assert_eq!(
        user_message_bg(HistorySurface::Main, false, false, true).bg,
        style::chat_user_bg().bg
    );
    assert_eq!(
        user_message_bg(HistorySurface::Btw, false, true, true).bg,
        style::chat_user_btw_bg().bg
    );
}
