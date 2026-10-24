//! 鼠标事件分派：按下、拖拽、释放、滚轮与移动悬停。

pub(crate) mod drag;
pub(crate) mod hit;
pub(crate) mod hover;
pub(crate) mod press;
pub(crate) mod wheel;

use std::collections::HashMap;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::app::state::AppState;
use crate::config::Config;
use crate::layout::PaneId;
use crate::pty::PtySession;

use super::session::Geometry;

/// 鼠标事件总入口：按事件种类分派至对应处理器。
pub(crate) fn handle_mouse_event(
    mouse: MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    config: &Config,
    cache: &mut crate::ui::RenderCache,
    dirty: &mut bool,
) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            press::handle_mouse_down_left(&mouse, state, sessions, geometry, config, cache, dirty);
        }
        MouseEventKind::Down(MouseButton::Right) => {
            press::handle_mouse_down_right(&mouse, state, geometry, dirty);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            drag::handle_mouse_drag(&mouse, state, sessions, geometry, config, cache, dirty);
        }
        MouseEventKind::Up(MouseButton::Left) => {
            drag::handle_mouse_up_left(&mouse, state, sessions, geometry, dirty);
        }
        MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => {
            wheel::handle_mouse_wheel(&mouse, state, sessions, geometry, cache, dirty);
        }
        MouseEventKind::Moved => {
            hover::handle_mouse_moved(&mouse, state, sessions, geometry, config, cache, dirty);
        }
        _ => {}
    }
}
