//! UI 组件查看全屏覆盖：几何、左树行渲染、右侧居中真实外观预览与命中。
//!
//! 全屏覆盖（蓝色外框、无底色）内部左区固定 24 列展示组件树，右区占剩余宽度，
//! 按选中组件渲染其**真实外观**（复用组件自身样式与尺寸，不额外包裹样例框），
//! 并在右区**水平居中**。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::app::ui_gallery::{UiGallery, UiGalleryComponentId, UiGalleryNode};
use crate::ui::widgets::fullscreen::{self, FullscreenShell};
use crate::ui::widgets::modal;
use crate::ui::{style, text};

/// 左侧组件树区宽度（列）。
const GALLERY_LEFT_WIDTH: u16 = 24;
/// 左侧树与右侧预览之间的分隔列（1 列）。
const GALLERY_DIVIDER_WIDTH: u16 = 1;

/// 全屏覆盖几何：整体、左侧树区与右侧预览区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGalleryLayout {
    pub shell: FullscreenShell,
    /// 左侧组件树区。
    pub left: Rect,
    /// 右侧预览区。
    pub right: Rect,
}

/// 全屏覆盖几何；屏幕过小时返回 None（由 `fullscreen::layout` 判定）。
pub fn ui_gallery_shell(screen: Rect) -> Option<FullscreenShell> {
    fullscreen::layout(screen)
}

/// 分区几何：左侧固定 24 列、右侧占剩余，中间 1 列分隔线。
pub fn ui_gallery_layout(screen: Rect) -> Option<UiGalleryLayout> {
    let shell = ui_gallery_shell(screen)?;
    let content = shell.content;
    if content.width < 3 || content.height == 0 {
        return None;
    }
    let left_width = GALLERY_LEFT_WIDTH.min(content.width.saturating_sub(2));
    let left = Rect::new(content.x, content.y, left_width, content.height);
    let right = Rect::new(
        content.x + left_width + GALLERY_DIVIDER_WIDTH,
        content.y,
        content
            .width
            .saturating_sub(left_width + GALLERY_DIVIDER_WIDTH),
        content.height,
    );
    Some(UiGalleryLayout { shell, left, right })
}

/// 命中组件树的可见行下标；未命中（分隔线、预览区或越界）返回 None。
pub fn ui_gallery_row_at(
    layout: &UiGalleryLayout,
    rows: usize,
    column: u16,
    row: u16,
) -> Option<usize> {
    if !layout.left.contains((column, row).into()) {
        return None;
    }
    let index = usize::from(row - layout.left.y);
    (index < usize::from(layout.left.height) && index < rows).then_some(index)
}

/// 渲染全屏覆盖：蓝色外框、标题栏/提示栏、左侧树、中间分隔线与右侧预览。
pub(crate) fn render_ui_gallery(frame: &mut Frame<'_>, screen: Rect, gallery: &UiGallery) {
    let Some(layout) = ui_gallery_layout(screen) else {
        return;
    };
    fullscreen::render(
        frame,
        &layout.shell,
        text::UI_GALLERY_TITLE,
        text::UI_GALLERY_HINT,
    );
    render_divider(frame, &layout);
    render_tree(frame, &layout, gallery);
    render_preview(frame, &layout, gallery);
}

/// 左侧树与右侧预览之间的竖直分隔线（1 列，失焦态边框色）。
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
    let list_height = usize::from(layout.left.height);
    let buf = frame.buffer_mut();
    for (index, row) in rows.iter().enumerate().take(list_height) {
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

/// 右侧预览：组件真实外观**水平居中**，其下留 1 行后渲染 muted 说明。
fn render_preview(frame: &mut Frame<'_>, layout: &UiGalleryLayout, gallery: &UiGallery) {
    let right = layout.right;
    if right.width == 0 || right.height == 0 {
        return;
    }
    let Some(component) = gallery.selected_component() else {
        // 未选中组件（高亮在文件夹行）：提示在右区水平 + 竖直居中。
        let hint = text::UI_GALLERY_PREVIEW_HINT;
        let hint_width = (hint.chars().count() as u16).min(right.width);
        let x = right.x + right.width.saturating_sub(hint_width) / 2;
        let y = right.y + right.height.saturating_sub(1) / 2;
        let buf = frame.buffer_mut();
        put_text(buf, x, y, hint, style::muted(), right.right());
        return;
    };
    let (width, height) = appearance_size(component.id, right);
    let x = right.x + right.width.saturating_sub(width) / 2;
    let rect = Rect::new(x, right.y, width, height);
    render_appearance(frame, rect, component.id);
    // 说明文字：组件下方空 1 行，每行在右区水平居中。
    let desc_y = right.y.saturating_add(height).saturating_add(1);
    let buf = frame.buffer_mut();
    for (offset, line) in component.description.iter().enumerate() {
        let y = desc_y + offset as u16;
        if y >= right.bottom() {
            break;
        }
        let line_width = (line.chars().count() as u16).min(right.width);
        let desc_x = right.x + right.width.saturating_sub(line_width) / 2;
        put_text(buf, desc_x, y, line, style::muted(), right.right());
    }
}

/// 组件的自然尺寸（宽、高），按可用区域夹取。
fn appearance_size(id: UiGalleryComponentId, available: Rect) -> (u16, u16) {
    let text_width = |s: &str| s.chars().count() as u16;
    let (width, height) = match id {
        UiGalleryComponentId::Toast => (text_width(text::UI_GALLERY_TOAST_SAMPLE) + 4, 3),
        UiGalleryComponentId::Tooltip => (text_width(text::UI_GALLERY_TOOLTIP_SAMPLE) + 4, 3),
        UiGalleryComponentId::Modal => (40, 7),
        UiGalleryComponentId::Menu => {
            let max = text::UI_GALLERY_MENU_SAMPLE
                .iter()
                .map(|item| text_width(item))
                .max()
                .unwrap_or(0);
            (
                (max + 4).max(14),
                text::UI_GALLERY_MENU_SAMPLE.len() as u16 + 2,
            )
        }
        UiGalleryComponentId::Button => (text_width(text::UI_GALLERY_BUTTON_SAMPLE), 1),
        UiGalleryComponentId::Fullscreen => (40, 8),
        UiGalleryComponentId::PromptEditor => (46, 8),
        UiGalleryComponentId::PromptList => (34, 8),
        UiGalleryComponentId::PromptToolbar => (
            text_width(text::UI_GALLERY_PROMPT_TOOLBAR_LEFT)
                + text_width(text::UI_GALLERY_PROMPT_TOOLBAR_RIGHT)
                + 6,
            1,
        ),
        UiGalleryComponentId::ReferencedFolders => (48, 7),
        UiGalleryComponentId::FolderSearch => (48, 9),
    };
    (width.min(available.width), height.min(available.height))
}

/// 在已定尺寸的 `rect` 内渲染组件真实外观。
fn render_appearance(frame: &mut Frame<'_>, rect: Rect, id: UiGalleryComponentId) {
    match id {
        UiGalleryComponentId::Toast => {
            render_toast_like(frame, rect, text::UI_GALLERY_TOAST_SAMPLE)
        }
        UiGalleryComponentId::Tooltip => {
            render_toast_like(frame, rect, text::UI_GALLERY_TOOLTIP_SAMPLE)
        }
        UiGalleryComponentId::Modal => render_modal_like(frame, rect),
        UiGalleryComponentId::Menu => render_menu_like(frame, rect),
        UiGalleryComponentId::Button => render_button_like(frame, rect),
        UiGalleryComponentId::Fullscreen => render_fullscreen_like(frame, rect),
        UiGalleryComponentId::PromptEditor => render_prompt_editor_like(frame, rect),
        UiGalleryComponentId::PromptList => render_prompt_list_like(frame, rect),
        UiGalleryComponentId::PromptToolbar => render_prompt_toolbar_like(frame, rect),
        UiGalleryComponentId::ReferencedFolders => render_referenced_folders_like(frame, rect),
        UiGalleryComponentId::FolderSearch => render_folder_search_like(frame, rect),
    }
}

/// toast / tooltip：圆角边框 + 面板底色 + 右上角 `[✕]`，正文居中；宽度贴合内容。
fn render_toast_like(frame: &mut Frame<'_>, rect: Rect, message: &str) {
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if let Some(close) = modal::close_button_rect(rect) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            close,
        );
    }
    frame.render_widget(
        Paragraph::new(Span::styled(message, style::text())).alignment(Alignment::Center),
        inner,
    );
}

/// modal：圆角强调色边框 + 标题 + `[✕]` + 问题行 + 底部按钮行。
fn render_modal_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title(Span::styled(
            format!(" {} ", text::UI_GALLERY_MODAL_SAMPLE_TITLE),
            style::border_title(),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if let Some(close) = modal::close_button_rect(rect) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            close,
        );
    }
    if inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::UI_GALLERY_MODAL_SAMPLE_BODY,
            style::text(),
        )),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let buttons = modal::button_row(
        inner,
        &[text::BUTTON_CONFIRM, text::BUTTON_CANCEL],
        2,
        inner.height.saturating_sub(1),
    );
    for (index, button) in buttons.iter().enumerate() {
        let label = if index == 0 {
            text::BUTTON_CONFIRM
        } else {
            text::BUTTON_CANCEL
        };
        frame.render_widget(
            Paragraph::new(Span::styled(label, style::accent())),
            *button,
        );
    }
}

/// menu：圆角强调色边框 + 标题 + 条目行，首个条目 `overlay_selection` 高亮。
fn render_menu_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title(Span::styled(
            format!(" {} ", text::UI_GALLERY_MENU_SAMPLE_TITLE),
            style::border_title(),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    for (index, item) in text::UI_GALLERY_MENU_SAMPLE.iter().enumerate() {
        let y = inner.y.saturating_add(index as u16);
        if y >= inner.bottom() {
            break;
        }
        let item_style = if index == 0 {
            style::overlay_selection()
        } else {
            style::text()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(*item, item_style))).style(item_style),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

/// button：只有强调色标签本身，无边框、无底色。
fn render_button_like(frame: &mut Frame<'_>, rect: Rect) {
    let buf = frame.buffer_mut();
    put_text(
        buf,
        rect.x,
        rect.y,
        text::UI_GALLERY_BUTTON_SAMPLE,
        style::accent(),
        rect.right(),
    );
}

/// fullscreen：真实全屏覆盖外观——蓝色圆角边框 + 标题/提示内嵌边框线 + 右上角 `[✕]`。
fn render_fullscreen_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .title_top(
            Line::from(Span::styled(
                format!(" {} ", text::UI_GALLERY_FULLSCREEN_SAMPLE_TITLE),
                style::border_title(),
            ))
            .alignment(Alignment::Left),
        )
        .title_bottom(
            Line::from(Span::styled(
                format!(" {} ", text::UI_GALLERY_FULLSCREEN_SAMPLE_HINT),
                style::muted(),
            ))
            .alignment(Alignment::Left),
        );
    frame.render_widget(block, rect);
    if let Some(close) = modal::close_button_rect(rect) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            close,
        );
    }
}

/// prompt editor：真实 prompt 面板外观——圆角边框 + `Prompt Manager` 标题 + 工具栏行 + 分割线 + 文本行。
fn render_prompt_editor_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 4 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::border_title(),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::UI_GALLERY_PROMPT_EDITOR_SAMPLE,
            style::text(),
        )),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if inner.height >= 2 {
        paint_hline(frame, inner.x, inner.y + 1, inner.right());
    }
    let buf = frame.buffer_mut();
    for (offset, line) in text::UI_GALLERY_PROMPT_EDITOR_BODY.iter().enumerate() {
        let y = inner.y.saturating_add(2 + offset as u16);
        if y >= inner.bottom() {
            break;
        }
        put_text(buf, inner.x, y, line, style::text(), inner.right());
    }
}

/// prompt list：真实树视图外观——搜索框 + 分割线 + `tmp:` 分区与树行。
fn render_prompt_list_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 4 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::border_title(),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::UI_GALLERY_PROMPT_LIST_SAMPLE,
            style::muted(),
        )),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if inner.height >= 2 {
        paint_hline(frame, inner.x, inner.y + 1, inner.right());
    }
    let buf = frame.buffer_mut();
    for (offset, line) in text::UI_GALLERY_PROMPT_LIST_BODY.iter().enumerate() {
        let y = inner.y.saturating_add(2 + offset as u16);
        if y >= inner.bottom() {
            break;
        }
        put_text(buf, inner.x, y, line, style::text(), inner.right());
    }
}

/// prompt toolbar：真实工具栏行——强调色按钮 + 右对齐 `[◉] [*] ●`。
fn render_prompt_toolbar_like(frame: &mut Frame<'_>, rect: Rect) {
    let left = text::UI_GALLERY_PROMPT_TOOLBAR_LEFT;
    let right = text::UI_GALLERY_PROMPT_TOOLBAR_RIGHT;
    let buf = frame.buffer_mut();
    let end = put_text(buf, rect.x, rect.y, left, style::accent(), rect.right());
    let right_start = rect.right().saturating_sub(right.chars().count() as u16);
    if right_start > end {
        put_text(
            buf,
            right_start,
            rect.y,
            right,
            style::accent(),
            rect.right(),
        );
    }
}

/// 引用文件夹目录菜单：蓝色圆角边框 + 标题 `Folders` + prompt 列表式行 + 底边快捷键提示。
fn render_referenced_folders_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 4 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title_top(Span::styled(
            format!(" {} ", text::MENU_TITLE_REFERENCED_FOLDERS),
            style::border_title(),
        ))
        .title_bottom(
            Line::from(Span::styled(
                format!(" {} ", text::REFERENCED_MENU_FOOTER),
                style::muted_on_border(),
            ))
            .alignment(Alignment::Left),
        );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }
    let names = text::UI_GALLERY_FOLDERS_SAMPLE;
    let mut y = inner.y;
    // 选中项：整行 overlay_selection 底色 + `❯` 标记。
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("❯ ▸ {}", names[0]),
            style::text(),
        )))
        .style(style::overlay_selection()),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y = y.saturating_add(1);
    // 其余目录项。
    for name in names.iter().skip(1) {
        if y >= inner.bottom() {
            return;
        }
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  ▸ {name}"),
                style::text(),
            )))
            .style(style::overlay_panel()),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y = y.saturating_add(1);
    }
    // 末尾动作行：强调色 `+ add folder…`。
    if y < inner.bottom() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text::REFERENCED_MENU_ADD_ROW,
                style::accent(),
            ))),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

/// 引用文件夹内容搜索：蓝色圆角边框 + 标题 `folder contents` + 右上角复制 + 搜索框 + 双行候选 + 底边提示。
fn render_folder_search_like(frame: &mut Frame<'_>, rect: Rect) {
    if rect.width < 4 || rect.height < 6 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title_top(Span::styled(
            format!(" {} ", text::REFERENCED_SEARCH_TITLE),
            style::border_title(),
        ))
        .title_bottom(
            Line::from(Span::styled(
                format!(" {} ", text::REFERENCED_SEARCH_FOOTER),
                style::muted_on_border(),
            ))
            .alignment(Alignment::Left),
        );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 3 {
        return;
    }
    // 顶部范围行：右端复制原始路径按钮。
    let copy_x = inner
        .right()
        .saturating_sub(text::TEMPLATE_BUTTON_COPY.chars().count() as u16);
    put_text(
        frame.buffer_mut(),
        copy_x,
        inner.y,
        text::TEMPLATE_BUTTON_COPY,
        style::accent(),
        inner.right(),
    );
    // 搜索框行与分割线。
    put_text(
        frame.buffer_mut(),
        inner.x,
        inner.y + 1,
        text::UI_GALLERY_FOLDER_SEARCH_QUERY,
        style::muted(),
        inner.right(),
    );
    paint_hline(frame, inner.x, inner.y + 2, inner.right());
    // 双行候选：图标 + 名称，次行 `└─ 父目录`。
    let mut y = inner.y.saturating_add(3);
    for (icon, name, parent) in text::UI_GALLERY_FOLDER_SEARCH_ROWS {
        if y + 1 >= inner.bottom() {
            break;
        }
        let buf = frame.buffer_mut();
        put_text(
            buf,
            inner.x,
            y,
            &format!("{icon} {name}"),
            style::text(),
            inner.right(),
        );
        put_text(
            buf,
            inner.x,
            y + 1,
            &format!("└─ {parent}"),
            style::muted(),
            inner.right(),
        );
        y = y.saturating_add(2);
    }
}

/// 写入单格文本；越界忽略。
fn put_cell(buf: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.reset();
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
}

/// 逐字符写入并返回结束列；越过 `right` 边界停止。
fn put_text(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style, right: u16) -> u16 {
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

/// 从 `x` 到 `right` 画一条水平分割线。
fn paint_hline(frame: &mut Frame<'_>, x: u16, y: u16, right: u16) {
    let buf = frame.buffer_mut();
    for column in x..right {
        if let Some(cell) = buf.cell_mut((column, y)) {
            cell.set_symbol(text::DIVIDER_MID);
            cell.set_style(style::border(false));
        }
    }
}

#[cfg(test)]
mod tests;
