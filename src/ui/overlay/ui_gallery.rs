//! UI 组件查看弹窗：几何、左树行渲染、右侧静态预览与命中。
//!
//! 弹窗占屏幕宽高各减 4（四周留 2 列/2 行）居中；内部左区固定 24 列展示组件树，
//! 右区占剩余宽度按选中组件分派静态预览。只读状态，不驱动真实业务逻辑。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::app::ui_gallery::{UiGallery, UiGalleryComponent, UiGalleryComponentId, UiGalleryNode};
use crate::ui::widgets;
use crate::ui::{style, text};

use super::super::widgets::modal::ModalShell;

/// 弹窗四周留白（列/行）。
const GALLERY_MARGIN: u16 = 2;
/// 左侧组件树区宽度（列）。
const GALLERY_LEFT_WIDTH: u16 = 24;

/// 弹窗几何：整体区域、左侧树区与右侧预览区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGalleryLayout {
    pub shell: ModalShell,
    /// 左侧组件树区（含每行）。
    pub left: Rect,
    /// 右侧预览区。
    pub right: Rect,
}

/// 弹窗几何；屏幕放不下时返回 None。
pub fn ui_gallery_shell(screen: Rect) -> Option<ModalShell> {
    let width = screen.width.saturating_sub(GALLERY_MARGIN * 2);
    let height = screen.height.saturating_sub(GALLERY_MARGIN * 2);
    widgets::modal::layout(screen, width, height)
}

/// 弹窗分区几何；`left` 在内容区过窄时压缩，最少给右侧留 1 列。
pub fn ui_gallery_layout(screen: Rect) -> Option<UiGalleryLayout> {
    let shell = ui_gallery_shell(screen)?;
    let inner = shell.inner;
    if inner.width < 2 || inner.height == 0 {
        return None;
    }
    // 左侧 24 列、右侧至少 1 列，外加中间 1 列分隔线。
    let left_width = GALLERY_LEFT_WIDTH.min(inner.width.saturating_sub(2));
    let left = Rect::new(inner.x, inner.y, left_width, inner.height);
    let right = Rect::new(
        inner.x + left_width + 1,
        inner.y,
        inner.width.saturating_sub(left_width + 1),
        inner.height,
    );
    Some(UiGalleryLayout { shell, left, right })
}

/// 命中组件树的可见行下标；未命中（分隔线、预览区、按钮行或越界）返回 None。
pub fn ui_gallery_row_at(
    layout: &UiGalleryLayout,
    rows: usize,
    column: u16,
    row: u16,
) -> Option<usize> {
    if !layout.left.contains((column, row).into()) {
        return None;
    }
    // 内容区首行为树行起点；末行留给底部快捷键提示。
    let list_height = usize::from(layout.left.height.saturating_sub(1));
    if list_height == 0 {
        return None;
    }
    let index = usize::from(row - layout.left.y);
    (index < list_height && index < rows).then_some(index)
}

/// 渲染弹窗：外框、左侧组件树、中间分隔线与右侧预览。
pub(crate) fn render_ui_gallery(frame: &mut Frame<'_>, screen: Rect, gallery: &UiGallery) {
    let Some(layout) = ui_gallery_layout(screen) else {
        return;
    };
    widgets::modal::render(frame, &layout.shell, text::UI_GALLERY_TITLE);
    render_divider(frame, &layout);
    render_tree(frame, &layout, gallery);
    render_preview(frame, &layout, gallery);
    render_hint(frame, &layout);
}

/// 左侧树与右侧预览之间的竖直分隔线。
fn render_divider(frame: &mut Frame<'_>, layout: &UiGalleryLayout) {
    let x = layout.left.right();
    let buf = frame.buffer_mut();
    for y in layout.left.y..layout.left.bottom() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(text::STRIP_LINE);
            cell.set_style(style::border(false));
        }
    }
}

/// 组件树行：文件夹 `▾`/`▸` 三角 + 名称、组件 `•` 图标 + 名称；缩进 2 列、高亮行首 `❯`。
fn render_tree(frame: &mut Frame<'_>, layout: &UiGalleryLayout, gallery: &UiGallery) {
    let rows = gallery.rows();
    let list_height = layout.left.height.saturating_sub(1);
    let buf = frame.buffer_mut();
    for (index, row) in rows.iter().enumerate().take(usize::from(list_height)) {
        let y = layout.left.y + index as u16;
        let x = layout.left.x;
        if index == gallery.selected() {
            put_cell(buf, x, y, text::AGENT_SELECTED_MARKER, style::accent());
        }
        match row.node {
            UiGalleryNode::Folder(id) => {
                let marker = if gallery.is_collapsed(id) {
                    text::PROMPT_TREE_COLLAPSED
                } else {
                    text::PROMPT_TREE_EXPANDED
                };
                let mut cursor =
                    put_text(buf, x + 2, y, marker, style::accent(), layout.left.right());
                cursor = cursor.saturating_add(1);
                put_text(
                    buf,
                    cursor,
                    y,
                    id.label(),
                    style::text(),
                    layout.left.right(),
                );
            }
            UiGalleryNode::Component(id) => {
                let mut cursor = put_text(
                    buf,
                    x + 4,
                    y,
                    text::PROMPT_TREE_ITEM_ICON,
                    style::muted(),
                    layout.left.right(),
                );
                cursor = cursor.saturating_add(1);
                put_text(
                    buf,
                    cursor,
                    y,
                    id.label(),
                    style::text(),
                    layout.left.right(),
                );
            }
        }
    }
}

/// 右侧预览：标题 + 静态样例 + 用途说明；无选中组件时显示提示。
fn render_preview(frame: &mut Frame<'_>, layout: &UiGalleryLayout, gallery: &UiGallery) {
    let right = layout.right;
    if right.width == 0 || right.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    put_text(
        buf,
        right.x,
        right.y,
        text::UI_GALLERY_PREVIEW_TITLE,
        style::border_title(),
        right.right(),
    );
    let Some(component) = gallery.selected_component() else {
        put_text(
            buf,
            right.x,
            right.y + 2,
            text::UI_GALLERY_PREVIEW_HINT,
            style::muted(),
            right.right(),
        );
        return;
    };
    let body = Rect::new(
        right.x,
        right.y.saturating_add(1),
        right.width,
        right.height.saturating_sub(1),
    );
    render_component_preview(frame, body, component);
}

/// 按组件 id 分派静态预览样例。
fn render_component_preview(frame: &mut Frame<'_>, area: Rect, component: &UiGalleryComponent) {
    // 样例外框：圆角、muted 边框、面板底色，与真实组件同款。
    let sample = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(1),
        SAMPLE_HEIGHT.min(area.height),
    );
    if sample.width >= 4 && sample.height >= 3 {
        let title = component.id.label();
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(style::muted())
                .style(style::overlay_panel())
                .title(Span::styled(format!(" {title} "), style::border_title())),
            sample,
        );
        render_sample_body(frame, sample, component.id);
    }
    // 说明文字：样例下方逐行渲染。
    let desc_y = area.y.saturating_add(SAMPLE_HEIGHT).saturating_add(1);
    let buf = frame.buffer_mut();
    for (offset, line) in component.description.iter().enumerate() {
        let y = desc_y + offset as u16;
        if y >= area.bottom() {
            break;
        }
        put_text(buf, area.x, y, line, style::muted(), area.right());
    }
}

/// 各类组件的样例内容（只读、静态）。
fn render_sample_body(frame: &mut Frame<'_>, sample: Rect, id: UiGalleryComponentId) {
    let inner = Block::bordered().inner(sample);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let label = match id {
        UiGalleryComponentId::Toast => "Copied to clipboard          [✕]",
        UiGalleryComponentId::Modal => "Are you sure you want to close?",
        UiGalleryComponentId::Tooltip => "undo (ctrl+z)",
        UiGalleryComponentId::Button => "[SET]  [UI]  [EXIT]  [≡]  [◉]",
        UiGalleryComponentId::Menu => "Renaming highlight follows hover",
        UiGalleryComponentId::PromptEditor => "## heading",
        UiGalleryComponentId::PromptList => "search prompts / tmp: / saved:",
        UiGalleryComponentId::PromptToolbar => "[≡] [←] [→]      [◉] [*] ●",
    };
    let line = Line::from(Span::styled(label, style::text()));
    frame.render_widget(Paragraph::new(line), inner);
}

/// 底边快捷键提示：贴弹窗内容区底行左端。
fn render_hint(frame: &mut Frame<'_>, layout: &UiGalleryLayout) {
    let y = layout.shell.inner.bottom().saturating_sub(1);
    let buf = frame.buffer_mut();
    put_text(
        buf,
        layout.left.x,
        y,
        text::UI_GALLERY_HINT,
        style::muted(),
        layout.shell.inner.right(),
    );
}

/// 样例框高度（行，含边框）。
const SAMPLE_HEIGHT: u16 = 5;

/// 写入单格文本；越界忽略。
fn put_cell(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    symbol: &str,
    style: ratatui::style::Style,
) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.reset();
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
}

/// 逐字符写入并返回结束列；越过 `right` 边界停止。
fn put_text(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    style: ratatui::style::Style,
    right: u16,
) -> u16 {
    let mut column = x;
    for symbol in text.chars() {
        if column >= right {
            break;
        }
        if let Some(cell) = buf.cell_mut((column, y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style);
        }
        column = column.saturating_add(1);
    }
    column
}

#[cfg(test)]
mod tests;
