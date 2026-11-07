//! `/connect` 居中模态：几何、命中与渲染。
//!
//! 顶栏标题 + 右侧连接摘要；内部自带 `>` 单行输入框与状态/错误行，列表复用
//! 引用文件夹弹窗的 `❯` 标记、悬停底色与右缘滚动条；底栏为快捷键提示。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use unicode_width::UnicodeWidthStr;

use crate::app::referenced::menu_window_start;
use crate::app::state::{ConnectPanel, ConnectPromptStep, ConnectStage};
use crate::ui::widgets;
use crate::ui::{style, text};

use super::referenced::{fill_row, paint_text};

/// 连接模态宽度（列）。
pub const CONNECT_WIDTH: u16 = 72;
/// 连接模态高度（行）；固定尺寸，不随条目数量变化。
pub const CONNECT_HEIGHT: u16 = 20;
/// 行首标记列宽（`❯ ` 与空标记占位）。
const MARKER_WIDTH: u16 = 2;

/// 连接模态几何：输入行、状态行、列表区与滚动条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectLayout {
    pub area: Rect,
    /// 模态内置单行输入框。
    pub input_rect: Rect,
    /// 输入框下方的校验/错误/说明行。
    pub status_rect: Rect,
    /// 列表区（不含右缘滚动条列）。
    pub list_rect: Rect,
    /// 可见行矩形（顺序对应窗口内行，1 行高）。
    pub rows: Vec<Rect>,
    /// 窗口内首个可见行索引。
    pub start: usize,
    /// 列表可视行数。
    pub visible: usize,
    /// 列表滚动条；不溢出时为 None。
    pub scrollbar: Option<widgets::scrollbar::ScrollbarLayout>,
}

/// 连接模态几何；屏幕放不下时返回 None。
pub fn connect_layout(screen: Rect, panel: &ConnectPanel) -> Option<ConnectLayout> {
    let width = CONNECT_WIDTH.min(screen.width);
    let height = CONNECT_HEIGHT.min(screen.height);
    if width < 8 || height < 10 {
        return None;
    }
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    // 顶栏 3 行（顶边 + 标题行 + 呼吸行）、底栏 3 行（呼吸行 + 提示行 + 底边）。
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(3),
        area.width.saturating_sub(2),
        area.height.saturating_sub(6),
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let input_rect = Rect::new(inner.x, inner.y, inner.width, 1);
    let status_rect = Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 1);
    let list_rect = Rect::new(
        inner.x,
        inner.y.saturating_add(2),
        inner.width.saturating_sub(1),
        inner.height.saturating_sub(2),
    );
    let total = panel.item_count();
    let visible = usize::from(list_rect.height);
    let max_start = total.saturating_sub(visible);
    let window = panel.window();
    let start = window
        .viewport
        .map(|offset| offset.min(max_start))
        .unwrap_or_else(|| menu_window_start(window.active, visible, total));
    let shown = visible.min(total.saturating_sub(start));
    let rows = (0..shown)
        .map(|offset| Rect::new(list_rect.x, list_rect.y + offset as u16, list_rect.width, 1))
        .collect();
    let scrollbar = widgets::scrollbar::layout(
        Rect {
            width: inner.width,
            ..list_rect
        },
        total,
        visible.max(1),
        start,
    );
    Some(ConnectLayout {
        area,
        input_rect,
        status_rect,
        list_rect,
        rows,
        start,
        visible,
        scrollbar,
    })
}

/// 命中共模态列表行；返回可见行索引（与窗口高亮同一口径）。
pub fn connect_row_at(screen: Rect, panel: &ConnectPanel, column: u16, row: u16) -> Option<usize> {
    let layout = connect_layout(screen, panel)?;
    layout
        .rows
        .iter()
        .position(|rect| rect.contains((column, row).into()))
        .map(|offset| layout.start + offset)
}

/// 渲染连接模态；返回输入框硬件光标位置（校验中或不需要输入时为 None）。
pub fn render_connect(
    frame: &mut Frame<'_>,
    screen: Rect,
    panel: &ConnectPanel,
) -> Option<(u16, u16)> {
    let layout = connect_layout(screen, panel)?;
    widgets::modal::dim_background(frame, screen);
    widgets::clear::render(frame, layout.area);
    let block = Block::bordered().border_style(style::accent());
    frame.render_widget(block, layout.area);
    render_header(frame, &layout, panel);
    let cursor = if panel.has_text_input() {
        super::dialog::render_input(
            frame,
            layout.input_rect,
            panel.input.text(),
            panel.input.cursor(),
            panel.input.selection(),
            Some(input_placeholder(panel)),
        )
    } else {
        None
    };
    render_status(frame, &layout, panel);
    render_rows(frame, &layout, panel);
    if let Some(scrollbar) = layout.scrollbar {
        widgets::scrollbar::render(frame, &scrollbar);
    }
    render_footer(frame, &layout, panel);
    if panel.validating { None } else { cursor }
}

/// 顶栏：左侧标题（strong），右侧连接摘要/供应商名（muted）。
fn render_header(frame: &mut Frame<'_>, layout: &ConnectLayout, panel: &ConnectPanel) {
    if layout.area.width < 4 {
        return;
    }
    let row = Rect::new(
        layout.area.x + 2,
        layout.area.y + 1,
        layout.area.width.saturating_sub(4),
        1,
    );
    let title = header_title(panel);
    let right = header_summary(panel);
    let title_width = u16::try_from(title.width())
        .unwrap_or(u16::MAX)
        .min(row.width);
    paint_text(
        frame,
        Rect {
            width: title_width,
            ..row
        },
        &text::truncate_to_width(&title, usize::from(title_width)),
        style::strong(),
    );
    if !right.is_empty() {
        let available = row.width.saturating_sub(title_width + 2);
        if available > 0 {
            let clipped = text::truncate_to_width(&right, usize::from(available));
            let right_width = u16::try_from(clipped.width()).unwrap_or(0);
            paint_text(
                frame,
                Rect::new(
                    row.right().saturating_sub(right_width),
                    row.y,
                    right_width,
                    1,
                ),
                &clipped,
                style::muted(),
            );
        }
    }
}

/// 当前阶段标题。
fn header_title(panel: &ConnectPanel) -> String {
    match &panel.stage {
        ConnectStage::Providers => text::LX_CONNECT_TITLE_PROVIDERS.to_string(),
        ConnectStage::Manage { provider_name, .. } => text::lx_connect_title_manage(provider_name),
        ConnectStage::Methods { provider_name, .. } => {
            text::lx_connect_title_connect(provider_name)
        }
        ConnectStage::InputPrompt {
            provider_name,
            step,
            ..
        } => match step {
            ConnectPromptStep::ApiKey => text::lx_connect_title_verify(provider_name),
            ConnectPromptStep::FormField { .. } => text::lx_connect_title_connect(provider_name),
        },
        ConnectStage::OAuthWaiting { provider_name, .. } => {
            text::lx_connect_title_authorize(provider_name)
        }
    }
}

/// 顶栏右侧摘要：供应商阶段显示已连接数量，其余显示当前供应商名。
fn header_summary(panel: &ConnectPanel) -> String {
    match &panel.stage {
        ConnectStage::Providers => text::lx_connect_connected_count(panel.connected_count()),
        _ => panel.provider_name().unwrap_or_default().to_string(),
    }
}

/// 输入框占位文案。
fn input_placeholder(panel: &ConnectPanel) -> &'static str {
    match &panel.stage {
        ConnectStage::Providers => text::LX_CONNECT_SEARCH_PLACEHOLDER,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::ApiKey,
            ..
        } => text::LX_CONNECT_API_KEY_PLACEHOLDER,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::FormField { field, .. },
            ..
        } => field.placeholder,
        ConnectStage::OAuthWaiting { .. } => text::LX_CONNECT_OAUTH_CODE_PLACEHOLDER,
        _ => "",
    }
}

/// 输入框下方状态行：校验中 / 错误 / 输入提示。
fn render_status(frame: &mut Frame<'_>, layout: &ConnectLayout, panel: &ConnectPanel) {
    let Some((message, message_style)) = status_line(panel) else {
        return;
    };
    paint_text(
        frame,
        layout.status_rect,
        &text::truncate_to_width(&message, usize::from(layout.status_rect.width)),
        message_style,
    );
}

fn status_line(panel: &ConnectPanel) -> Option<(String, Style)> {
    if panel.validating {
        let message = panel
            .status_message
            .clone()
            .unwrap_or_else(|| text::LX_CONNECT_VERIFYING.to_string());
        return Some((message, style::muted()));
    }
    if let Some(error) = panel.current_error() {
        return Some((error.to_string(), style::error()));
    }
    match &panel.stage {
        ConnectStage::InputPrompt {
            provider_name,
            step,
            ..
        } => Some(match step {
            ConnectPromptStep::ApiKey => (
                text::lx_connect_api_key_instruction(provider_name),
                style::muted(),
            ),
            ConnectPromptStep::FormField { field, .. } => (field.label.to_string(), style::muted()),
        }),
        ConnectStage::OAuthWaiting { instructions, .. } if !instructions.is_empty() => {
            Some((instructions.clone(), style::muted()))
        }
        _ => None,
    }
}

/// 列表行：空态占位或按阶段绘制可见行。
fn render_rows(frame: &mut Frame<'_>, layout: &ConnectLayout, panel: &ConnectPanel) {
    if layout.rows.is_empty() {
        if let Some(placeholder) = empty_placeholder(panel) {
            let clipped = text::truncate_to_width(placeholder, usize::from(layout.list_rect.width));
            let text_w = u16::try_from(clipped.width()).unwrap_or(0);
            let x = layout
                .list_rect
                .x
                .saturating_add(layout.list_rect.width.saturating_sub(text_w) / 2);
            let y = layout
                .list_rect
                .y
                .saturating_add(layout.list_rect.height.saturating_sub(1) / 2);
            frame.buffer_mut().set_stringn(
                x,
                y,
                &clipped,
                usize::from(layout.list_rect.width),
                style::muted(),
            );
        }
        return;
    }
    let active = panel.window().active;
    for (offset, rect) in layout.rows.iter().enumerate() {
        let index = layout.start + offset;
        let base = if panel.hover == Some(index) {
            style::overlay_selection()
        } else {
            style::text()
        };
        fill_row(frame, *rect, base);
        render_row(frame, *rect, panel, index, index == active, base);
    }
}

fn empty_placeholder(panel: &ConnectPanel) -> Option<&'static str> {
    match &panel.stage {
        ConnectStage::Providers => Some(if panel.is_filter_empty() {
            text::LX_SLASH_NO_MATCHES
        } else {
            text::LX_CONNECT_EMPTY
        }),
        _ => None,
    }
}

/// 单行：标记 + 名称/动作 + 预览 + 右侧标签。
fn render_row(
    frame: &mut Frame<'_>,
    rect: Rect,
    panel: &ConnectPanel,
    index: usize,
    selected: bool,
    base: Style,
) {
    let x = paint_marker(frame, rect, selected, base);
    match &panel.stage {
        ConnectStage::Providers => {
            let Some(entry) = panel.entry_at_view(index) else {
                return;
            };
            let label_style = if entry.source.is_some() {
                base.patch(style::strong())
            } else {
                base
            };
            let right = paint_tag(frame, rect, base, &entry.tag, style::accent());
            let available = right.saturating_sub(x);
            let name_width = u16::try_from(entry.name.width())
                .unwrap_or(u16::MAX)
                .min(available / 2)
                .max(1);
            paint_text(
                frame,
                Rect::new(x, rect.y, name_width, 1),
                &text::truncate_to_width(&entry.name, usize::from(name_width)),
                label_style,
            );
            let note_x = x.saturating_add(name_width).saturating_add(2);
            paint_preview(frame, rect, note_x, right, &entry.note, base);
        }
        ConnectStage::Manage { actions, .. } => {
            let Some(action) = actions.get(index) else {
                return;
            };
            paint_label(frame, rect, x, action.label(), base);
        }
        ConnectStage::Methods { methods, .. } => {
            let Some(method) = methods.get(index) else {
                return;
            };
            let tag = if method.oauth_flow().is_some() {
                text::LX_CONNECT_METHOD_OAUTH_TAG
            } else {
                text::LX_CONNECT_METHOD_API_KEY_TAG
            };
            let right = paint_tag(frame, rect, base, tag, style::muted());
            let width = right.saturating_sub(x);
            if width > 0 {
                let label_area = Rect::new(x, rect.y, width, 1);
                paint_label(frame, label_area, x, method.label(), base);
            }
        }
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::FormField { field, .. },
            ..
        } => {
            let Some(option) = field.options.get(index) else {
                return;
            };
            paint_label(frame, rect, x, option.label, base);
            let label_width = u16::try_from(option.label.width()).unwrap_or(u16::MAX);
            let detail_x = x.saturating_add(label_width).saturating_add(2);
            paint_preview(
                frame,
                rect,
                detail_x,
                rect.right(),
                option.description,
                base,
            );
        }
        ConnectStage::InputPrompt { .. } => {}
        ConnectStage::OAuthWaiting { .. } => {
            render_oauth_row(frame, rect, index, &panel.stage, x, base);
        }
    }
}

fn render_oauth_row(
    frame: &mut Frame<'_>,
    rect: Rect,
    index: usize,
    stage: &ConnectStage,
    x: u16,
    base: Style,
) {
    let ConnectStage::OAuthWaiting {
        url,
        user_code,
        instructions,
        error,
        ..
    } = stage
    else {
        return;
    };
    if url.is_empty() {
        if index == 0 {
            let has_error = error.as_deref().is_some_and(|error| !error.is_empty());
            let status = if has_error {
                text::LX_CONNECT_OAUTH_FAILED_HINT
            } else if instructions.is_empty() {
                text::LX_CONNECT_OAUTH_WAITING
            } else {
                instructions
            };
            paint_label(frame, rect, x, status, base);
        }
        return;
    }
    match index {
        0 => {
            paint_label(frame, rect, x, text::LX_CONNECT_OPEN_BROWSER, base);
            let label_width = u16::try_from(text::LX_CONNECT_OPEN_BROWSER.width()).unwrap_or(0);
            paint_preview(frame, rect, x + label_width + 2, rect.right(), url, base);
        }
        1 => {
            paint_label(frame, rect, x, text::LX_CONNECT_COPY_CODE, base);
            let label_width = u16::try_from(text::LX_CONNECT_COPY_CODE.width()).unwrap_or(0);
            let value = user_code.as_deref().unwrap_or(url);
            paint_preview(frame, rect, x + label_width + 2, rect.right(), value, base);
        }
        _ => {
            let status = if instructions.is_empty() {
                text::LX_CONNECT_OAUTH_WAITING
            } else {
                instructions
            };
            paint_label(frame, rect, x, status, base);
            if let Some(error) = error.as_deref().filter(|error| !error.is_empty()) {
                let label_width = u16::try_from(status.width()).unwrap_or(0);
                paint_text(
                    frame,
                    Rect::new(
                        x.saturating_add(label_width).saturating_add(2),
                        rect.y,
                        rect.right().saturating_sub(x + label_width + 2),
                        1,
                    ),
                    &text::truncate_to_width(
                        error,
                        usize::from(rect.right().saturating_sub(x + label_width + 2)),
                    ),
                    base.patch(style::error()),
                );
            }
        }
    }
}

/// 行首标记列：选中行 `❯ `，否则等宽空格占位；返回其后起始列。
fn paint_marker(frame: &mut Frame<'_>, rect: Rect, selected: bool, base: Style) -> u16 {
    let marker = if selected {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        " ".repeat(usize::from(MARKER_WIDTH))
    };
    paint_text(frame, rect, &marker, base);
    rect.x.saturating_add(MARKER_WIDTH)
}

/// 绘制左侧标签；返回结束列。
fn paint_label(frame: &mut Frame<'_>, rect: Rect, x: u16, label: &str, base: Style) -> u16 {
    if x >= rect.right() {
        return x;
    }
    let available = usize::from(rect.right().saturating_sub(x));
    let clipped = text::truncate_to_width(label, available);
    let width = u16::try_from(clipped.width()).unwrap_or(0);
    paint_text(frame, Rect::new(x, rect.y, width, 1), &clipped, base);
    x.saturating_add(width)
}

/// 绘制 muted 预览（在 `start..right` 内截断）。
fn paint_preview(
    frame: &mut Frame<'_>,
    rect: Rect,
    start: u16,
    right: u16,
    preview: &str,
    base: Style,
) {
    if preview.is_empty() || start >= right {
        return;
    }
    let width = right.saturating_sub(start);
    paint_text(
        frame,
        Rect::new(start, rect.y, width, 1),
        &text::truncate_to_width(preview, usize::from(width)),
        base.patch(style::muted()),
    );
}

/// 右对齐标签；返回标签左侧可用的右界。
fn paint_tag(frame: &mut Frame<'_>, rect: Rect, base: Style, tag: &str, tag_style: Style) -> u16 {
    if tag.is_empty() {
        return rect.right();
    }
    let available = rect.width.saturating_sub(MARKER_WIDTH + 1);
    let clipped = text::truncate_to_width(tag, usize::from(available));
    let width = u16::try_from(clipped.width()).unwrap_or(0);
    if width == 0 {
        return rect.right();
    }
    let x = rect.right().saturating_sub(width);
    paint_text(
        frame,
        Rect::new(x, rect.y, width, 1),
        &clipped,
        base.patch(tag_style),
    );
    x.saturating_sub(1)
}

/// 底栏快捷键提示。
fn render_footer(frame: &mut Frame<'_>, layout: &ConnectLayout, panel: &ConnectPanel) {
    let row = Rect::new(
        layout.area.x + 2,
        layout.area.bottom().saturating_sub(2),
        layout.area.width.saturating_sub(4),
        1,
    );
    paint_text(frame, row, footer_text(panel), style::muted_on_border());
}

fn footer_text(panel: &ConnectPanel) -> &'static str {
    match &panel.stage {
        ConnectStage::Providers => text::LX_CONNECT_PROVIDERS_FOOTER,
        ConnectStage::Manage { .. } => text::LX_CONNECT_MANAGE_FOOTER,
        ConnectStage::Methods { .. } => text::LX_CONNECT_SELECT_FOOTER,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::FormField { field, .. },
            ..
        } if !field.options.is_empty() => text::LX_CONNECT_SELECT_FOOTER,
        ConnectStage::InputPrompt { .. } => text::LX_CONNECT_INPUT_FOOTER,
        ConnectStage::OAuthWaiting { mode, .. } => {
            if *mode == crate::config::ai::oauth::OAuthPromptMode::Code {
                text::LX_CONNECT_OAUTH_CODE_FOOTER
            } else {
                text::LX_CONNECT_OAUTH_FOOTER
            }
        }
    }
}
