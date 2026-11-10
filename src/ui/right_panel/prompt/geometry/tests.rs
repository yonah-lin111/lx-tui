//! prompt 几何单元测试。

use ratatui::layout::Rect;

use super::*;

#[test]
fn prompt_text_rect_reserves_scrollbar_column() {
    let panel = Rect::new(10, 5, 10, 6);
    assert_eq!(prompt_text_rect(panel), Rect::new(11, 8, 7, 2));
    assert_eq!(prompt_scrollbar_rect(panel), Some(Rect::new(18, 8, 1, 2)));
    assert_eq!(prompt_inner_size(panel), (7, 2));
}

#[test]
fn prompt_geometry_degrades_when_too_narrow() {
    let panel = Rect::new(0, 0, 3, 4);
    assert_eq!(prompt_text_rect(panel), Rect::new(1, 1, 1, 2));
    assert_eq!(prompt_scrollbar_rect(panel), None);
    assert_eq!(prompt_inner_size(panel), (1, 2));
}

#[test]
fn prompt_tree_scrollbar_starts_below_search_and_divider() {
    let panel = Rect::new(10, 5, 20, 8);
    assert_eq!(
        prompt_tree_scrollbar_rect(panel),
        Some(Rect::new(28, 10, 1, 2)),
        "槽从搜索框与分割线之下开始"
    );
    // 正文区过矮（≤ 搜索框 + 分割线）时不显示。
    let short = Rect::new(0, 0, 20, 6);
    assert_eq!(prompt_tree_scrollbar_rect(short), None);
}

#[test]
fn prompt_header_shifts_body_and_scrollbar() {
    let panel = Rect::new(10, 5, 20, 8);
    assert_eq!(prompt_header_rect(panel), Some(Rect::new(11, 6, 18, 2)));
    assert_eq!(prompt_toolbar_rect(panel), Some(Rect::new(11, 6, 18, 1)));
    assert_eq!(
        prompt_divider_rect(panel),
        Some(Rect::new(11, 7, 18, PROMPT_DIVIDER_HEIGHT))
    );
    assert_eq!(prompt_text_rect(panel), Rect::new(11, 8, 17, 4));
    assert_eq!(prompt_scrollbar_rect(panel), Some(Rect::new(28, 8, 1, 4)));
    assert_eq!(prompt_inner_size(panel), (17, 4));
}

#[test]
fn prompt_header_hidden_when_body_too_short() {
    // 内容区高度 == 1：隐藏表头，全部留给文本编辑区。
    let panel = Rect::new(0, 0, 20, 3);
    assert_eq!(prompt_header_rect(panel), None);
    assert_eq!(prompt_toolbar_rect(panel), None);
    assert_eq!(prompt_text_rect(panel), Rect::new(1, 1, 17, 1));
    assert_eq!(prompt_scrollbar_rect(panel), Some(Rect::new(18, 1, 1, 1)));
}

#[test]
fn prompt_toolbar_buttons_hit_expected_cells() {
    let panel = Rect::new(0, 0, 40, 8);
    let bar = prompt_toolbar_rect(panel).expect("toolbar visible");
    let counter = Some("1/2");
    let hit = |column: u16, row: u16| {
        prompt_toolbar_button_at(panel, column, row, PromptMode::Editor, counter)
    };
    assert_eq!(hit(bar.x, bar.y), Some(PromptToolbarButton::Mode));
    assert_eq!(hit(bar.x + 4, bar.y), Some(PromptToolbarButton::Recent));
    assert_eq!(hit(bar.x + 10, bar.y), Some(PromptToolbarButton::Undo));
    assert_eq!(hit(bar.x + 14, bar.y), Some(PromptToolbarButton::Redo));
    let save = prompt_toolbar_button_rect(
        panel,
        PromptToolbarButton::Save,
        PromptMode::Editor,
        counter,
    )
    .expect("save visible");
    assert_eq!(hit(save.x, bar.y), Some(PromptToolbarButton::Save));
    assert_eq!(hit(bar.x + 3, bar.y), None, "按钮空档不命中");
    assert_eq!(
        hit(bar.x + 1, bar.y + PROMPT_TOOLBAR_HEIGHT),
        None,
        "分割线行不命中按钮"
    );

    // 页控件位于保存点左侧：`[refDir] ‹ 1/2 › ●`。
    let controls = toolbar_page_controls(bar, "1/2", true).expect("page controls visible");
    assert_eq!(
        prompt_toolbar_button_at(panel, controls.prev.x, bar.y, PromptMode::Editor, counter),
        None,
        "页控件不是工具栏按钮"
    );
    assert_eq!(controls.next.x + 2, save.x, "计数与保存点间隔 1 列");
}

#[test]
fn prompt_toolbar_tree_mode_shows_recent_and_temp_prompt_only() {
    let panel = Rect::new(0, 0, 40, 8);
    let bar = prompt_toolbar_rect(panel).expect("toolbar visible");
    let mode = prompt_toolbar_button_rect(panel, PromptToolbarButton::Mode, PromptMode::Tree, None)
        .expect("mode visible in tree");
    assert_eq!(mode.x, bar.x);
    let recent =
        prompt_toolbar_button_rect(panel, PromptToolbarButton::Recent, PromptMode::Tree, None)
            .expect("recent visible in tree");
    assert_eq!(recent.x, bar.x + 4);
    assert_eq!(recent.width, 5, "[rec] 按钮宽度");
    let temp = prompt_toolbar_button_rect(
        panel,
        PromptToolbarButton::TempPrompt,
        PromptMode::Tree,
        None,
    )
    .expect("temp prompt visible in tree");
    assert_eq!(temp.right(), bar.right() - 1, "右对齐、与边框留 1 列");
    assert_eq!(temp.width, 5, "[tmp] 按钮宽度");
    assert_eq!(
        prompt_toolbar_button_at(panel, temp.x, bar.y, PromptMode::Tree, None),
        Some(PromptToolbarButton::TempPrompt)
    );
    assert_eq!(
        prompt_toolbar_button_rect(panel, PromptToolbarButton::Undo, PromptMode::Tree, None),
        None
    );
    assert_eq!(
        prompt_toolbar_button_rect(
            panel,
            PromptToolbarButton::TempPrompt,
            PromptMode::Editor,
            None
        ),
        None
    );
}

#[test]
fn prompt_toolbar_folders_sits_left_of_page_controls_in_editor_only() {
    let panel = Rect::new(0, 0, 40, 8);
    let bar = prompt_toolbar_rect(panel).expect("toolbar visible");
    let counter = Some("1/2");
    let folders = prompt_toolbar_button_rect(
        panel,
        PromptToolbarButton::Folders,
        PromptMode::Editor,
        counter,
    )
    .expect("folders visible");
    let save = prompt_toolbar_button_rect(
        panel,
        PromptToolbarButton::Save,
        PromptMode::Editor,
        counter,
    )
    .expect("save visible");
    let controls = toolbar_page_controls(bar, "1/2", true).expect("controls");
    assert_eq!(folders.y, bar.y);
    assert_eq!(folders.width, 8);
    assert_eq!(folders.right() + 1, controls.prev.x, "紧邻页控件左侧");
    assert_eq!(controls.next.x + 2, save.x, "页控件与保存点间隔 1 列");
    assert_eq!(
        prompt_toolbar_button_at(panel, folders.x, bar.y, PromptMode::Editor, counter),
        Some(PromptToolbarButton::Folders)
    );
    // 树视图不显示引用文件夹按钮。
    assert_eq!(
        prompt_toolbar_button_rect(panel, PromptToolbarButton::Folders, PromptMode::Tree, None),
        None
    );
    // 窄屏优先隐藏 [refDir]，保存点与页控件仍在。
    let narrow = Rect::new(0, 0, 30, 8);
    assert_eq!(
        prompt_toolbar_button_rect(
            narrow,
            PromptToolbarButton::Folders,
            PromptMode::Editor,
            counter
        ),
        None
    );
    assert!(
        prompt_toolbar_button_rect(
            narrow,
            PromptToolbarButton::Save,
            PromptMode::Editor,
            counter
        )
        .is_some(),
        "窄屏时保存点仍在"
    );
}

#[test]
fn prompt_toolbar_degrades_recent_before_undo_redo() {
    // 内宽 16：放不下 `[rec]`（需 19），撤销/重做仍在（需 13）。
    let narrow = Rect::new(0, 0, 18, 8);
    assert_eq!(
        prompt_toolbar_button_rect(
            narrow,
            PromptToolbarButton::Recent,
            PromptMode::Editor,
            None
        ),
        None
    );
    assert!(
        prompt_toolbar_button_rect(narrow, PromptToolbarButton::Undo, PromptMode::Editor, None)
            .is_some()
    );
    // 内宽 10：撤销/重做也隐藏。
    let tiny = Rect::new(0, 0, 12, 8);
    assert_eq!(
        prompt_toolbar_button_rect(tiny, PromptToolbarButton::Undo, PromptMode::Editor, None),
        None
    );
}

#[test]
fn prompt_border_rect_covers_top_border_row() {
    let panel = Rect::new(10, 5, 20, 8);
    assert_eq!(
        prompt_border_rect(panel),
        Some(Rect::new(10, 5, 20, 1)),
        "顶边框整行（含左右圆角）都是控件区"
    );
    assert_eq!(
        prompt_border_rect(Rect::new(0, 0, 4, 8)),
        None,
        "过窄无边框行"
    );
    assert_eq!(
        prompt_border_rect(Rect::new(0, 0, 20, 0)),
        None,
        "零高度无边框行"
    );
}
