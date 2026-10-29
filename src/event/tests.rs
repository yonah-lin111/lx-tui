use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::overlay::{ConfirmSwitchCwd, Overlay};
use crate::app::prompt::{PasteCandidateKind, PromptMode, PromptRowKind};
use crate::app::state::{PaneView, RightTab};
use crate::app::toast::{Toast, ToastKind};
use crate::config::Config;
use crate::layout;

use super::*;

pub(crate) mod test_support;

mod focus_and_paste;
mod lx_toast;
mod lx_view;
mod mouse_click;
mod mouse_drag;
mod overlay;
mod prompt_editor;
mod prompt_menu;
mod prompt_tree;
mod terminal;
mod todo;
