//! 界面控制按钮与装饰：折叠、退出、交换、设置按钮以及分割线高亮提示。

mod exit;
mod footer;
mod panels;
mod resize;
mod swap;

#[cfg(test)]
mod tests;

pub(crate) use exit::render_exit_button;
pub use exit::{exit_button, exit_button_at};
pub(crate) use footer::{render_settings_button, render_usage_button};
pub use footer::{settings_button, settings_button_at, usage_button, usage_button_at};
#[cfg(test)]
pub(crate) use panels::collapsed_content_x;
pub use panels::{
    CollapseButton, CollapseTarget, collapse_button_at, collapse_buttons, prompt_pin_at,
    prompt_pin_button,
};
pub(crate) use panels::{
    PANEL_BUTTON_MARGIN, PANEL_BUTTON_WIDTH, render_collapsed_strip, render_panel_buttons,
};
pub(crate) use resize::render_resize_hint;
pub(crate) use swap::render_swap_button;
pub use swap::{swap_button, swap_button_at};
