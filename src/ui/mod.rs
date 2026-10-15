//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod controls;
pub mod layout;
pub mod main_content;
pub mod markdown;
pub mod mention;
pub mod overlay;
mod panes;
pub mod prompt;
pub mod prompt_frame;
pub mod sidebar;
pub mod style;
pub mod tab_bar;
pub mod text;
pub mod toast;
pub mod tooltip;
pub mod trust;
pub mod widgets;

pub use controls::*;
pub use prompt_frame::*;

#[cfg(test)]
pub(crate) use controls::collapsed_content_x;

pub use sidebar::{
    add_workspace_button, agent_item_at, agent_item_index_at, agent_list_rows, agent_scrollbar,
    agents_section_at, sidebar_boundary_at, workspace_drop_index, workspace_group_toggle_at,
    workspace_item_at, workspace_list_blank_at, workspace_list_rows, workspace_scrollbar,
    workspace_section_at,
};

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::state::AppState;
use crate::config::Config;

/// 渲染整个界面。
pub fn render(frame: &mut Frame<'_>, state: &AppState, config: &Config) {
    let area = frame.area();
    if area.width < config.min_width || area.height < config.min_height {
        render_min_size_notice(frame, area, config);
        return;
    }

    let view = layout::compute(
        area,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let pane_rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );

    if view.sidebar.width > 0 {
        if state.sidebar_collapsed {
            controls::render_collapsed_strip(frame, view.sidebar, true);
        } else {
            sidebar::render(frame, view.sidebar, state);
            controls::render_settings_button(frame, &view, state);
            controls::render_ui_gallery_button(frame, &view, state);
        }
    }
    tab_bar::render(frame, &view, state);
    controls::render_exit_button(frame, &view);
    // 主区折叠：只画右缘窄条，不渲染窗格内容（PTY 尺寸保持折叠前）。
    let pane_cursor = if state.prompt_left && state.panes_collapsed {
        controls::render_collapsed_strip(frame, view.panes, false);
        None
    } else {
        panes::render_panes(frame, &pane_rects, state)
    };
    let prompt_cursor = prompt_frame::render_prompt(frame, &view, state).or(pane_cursor);
    controls::render_swap_button(frame, &view);
    controls::render_panel_buttons(frame, &view, state);
    controls::render_resize_hint(frame, &view, state);
    // 页码菜单/引用文件夹浮层的边框按钮 tooltip 需绘制在浮层之上（常规 tooltip 早于浮层绘制会被盖住）。
    let border_tooltip_open = matches!(
        state.overlay,
        Some(
            crate::app::overlay::Overlay::PageMenu(_)
                | crate::app::overlay::Overlay::ReferencedFolderMenu(_)
                | crate::app::overlay::Overlay::ReferencedFolderSearch(_)
        )
    );
    let tooltip_area = (!border_tooltip_open)
        .then(|| tooltip::render(frame, area, state, &view, &pane_rects))
        .flatten();
    let toast_area = toast::render(frame, area, state, &view, &pane_rects, config);
    let overlay_cursor = overlay::render(frame, area, state);
    let menu_tooltip_area = border_tooltip_open
        .then(|| tooltip::render(frame, area, state, &view, &pane_rects))
        .flatten();

    // 浮层是模态：重命名浮层接管硬件光标，其余浮层不显示光标。
    // 非模态时若光标位置被 tooltip/toast 盒覆盖（后绘制），一并隐藏，避免覆盖浮层。
    let cursor = match state.overlay {
        Some(_) => overlay_cursor,
        None => prompt_cursor.filter(|position| {
            !tooltip_area
                .into_iter()
                .chain(menu_tooltip_area)
                .chain(toast_area)
                .any(|box_area| box_area.contains((*position).into()))
        }),
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}

/// 终端尺寸不足时的提示。
fn render_min_size_notice(frame: &mut Frame<'_>, area: Rect, config: &Config) {
    let message = format!(
        "{} ({}x{})",
        text::MIN_SIZE_HINT,
        config.min_width,
        config.min_height
    );
    let line = Rect {
        y: area.y + area.height / 2,
        height: 1.min(area.height),
        ..area
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        line,
    );
}

#[cfg(test)]
mod tests;
