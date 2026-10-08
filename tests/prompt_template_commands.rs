//! 集成测试：prompt 模板斜杠命令全链路。
//!
//! 覆盖：输入 `/add` 弹出 Templates 面板 → 回车整行插入标准模板块 → markdown 高亮
//! 呈现（命令分色、标题下划线）→ 点击 `[todo]`/`[clean]`/`[del]` 按钮操作 → 单步撤销。

use lx_tui::app::actions::EditorCommand;
use lx_tui::app::markdown::{SlashCommandId, slash_template_content};
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

const WIDTH: u16 = 140;
const HEIGHT: u16 = 30;

/// 构造已同步几何的宽屏 prompt 场景；prompt 加宽保证模板块按钮可见。
fn ready_state() -> (AppState, Config, ui::layout::ViewLayout) {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    let screen = Rect::new(0, 0, WIDTH, HEIGHT);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    rects.push((state.prompt.id(), view.prompt));
    update::resize_panes(&mut state, &rects);
    (state, config, view)
}

fn draw(state: &AppState, config: &Config) -> Terminal<TestBackend> {
    let mut terminal =
        Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
    terminal
}

/// 屏幕行文本。
fn line_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
    let buffer = terminal.backend().buffer();
    (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect()
}

/// 行内子串起始单元格列：`str::find` 返回字节偏移，需换算为字符（单元格）数。
fn cell_x(row: &str, needle: &str) -> u16 {
    let byte = row.find(needle).expect("needle in row");
    row[..byte].chars().count() as u16
}

#[test]
fn slash_command_inserts_template_with_highlight_and_buttons() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    let area = layout::prompt_text_rect(view.prompt);

    for ch in "/add".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }
    let panel = ui::prompt::slash_layout(&state.prompt, area).expect("slash panel opens");
    assert_eq!(panel.rect.height, 3, "查询 add 只留 1 个候选");
    assert_eq!(
        state.prompt.slash_panel().map(|panel| panel.items().len()),
        Some(1)
    );
    let terminal = draw(&state, &config);
    assert!(
        line_text(&terminal, panel.rect.y).contains("Templates"),
        "面板标题"
    );

    update::apply_editor(&mut state, EditorCommand::Newline);
    let (content, _) = slash_template_content(SlashCommandId::Add);
    assert_eq!(state.prompt.text(), content);
    assert!(state.prompt.slash_panel().is_none());

    // 高亮：命令名绿色加粗；标题独立成行 Yellow 下划线；块边框 todo 靛蓝。
    let terminal = draw(&state, &config);
    let start_row = area.y;
    let row = line_text(&terminal, start_row);
    assert_eq!(
        terminal.backend().buffer()[(area.x, start_row)].symbol(),
        "╭"
    );
    assert_eq!(
        terminal.backend().buffer()[(area.x, start_row)].fg,
        Color::LightBlue,
        "todo 边框为靛蓝"
    );
    let command_x = cell_x(&row, "addTemplate");
    let command = &terminal.backend().buffer()[(command_x, start_row)];
    assert_eq!(command.fg, Color::Green, "row={row:?}");
    assert!(command.modifier.contains(Modifier::BOLD));
    assert!(!row.contains('「'), "标题不在起始行: {row}");

    let title_row = line_text(&terminal, start_row + 1);
    assert!(title_row.contains("「 title: 」"), "{title_row}");
    let title_x = cell_x(&title_row, "「");
    let title = &terminal.backend().buffer()[(title_x, start_row + 1)];
    assert_eq!(title.fg, Color::Cyan);
    assert!(title.modifier.contains(Modifier::UNDERLINED));

    // 按钮组常显。
    let buttons = ui::prompt::template_buttons(&state.prompt, area);
    assert_eq!(buttons.len(), 4, "宽屏下 [todo]/[copy]/[clean]/[del] 可见");
    let row = line_text(&terminal, area.y);
    assert!(row.contains("[todo]"), "{row}");
    assert!(row.contains("[copy]"), "{row}");
    assert!(row.contains("[clean]"), "{row}");
    assert!(row.contains("[del]"), "{row}");
    assert!(row.contains('╮'), "右上角闭合: {row}");
}

#[test]
fn narrow_prompt_keeps_buttons_and_ellipsizes_header() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 40;
    let screen = Rect::new(0, 0, WIDTH, HEIGHT);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    rects.push((state.prompt.id(), view.prompt));
    update::resize_panes(&mut state, &rects);
    update::focus_prompt(&mut state);
    let (content, _) = slash_template_content(SlashCommandId::Add);
    update::apply_editor(&mut state, EditorCommand::InsertText(content.into()));

    let area = layout::prompt_text_rect(view.prompt);
    let terminal = draw(&state, &config);
    let row = line_text(&terminal, area.y);
    assert!(row.contains("[todo]"), "窄屏按钮不隐藏: {row}");
    assert!(row.contains("[del]"), "{row}");
    assert!(row.contains('…'), "左上内容省略号截断: {row}");
    assert!(!row.contains("addTemplate"), "正文让位按钮: {row}");
}

#[test]
fn slash_panel_shows_all_candidates_for_bare_slash() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    let area = layout::prompt_text_rect(view.prompt);
    update::apply_editor(&mut state, EditorCommand::InsertChar('/'));
    let panel = ui::prompt::slash_layout(&state.prompt, area).expect("slash panel opens");
    assert_eq!(panel.rect.height, 7, "空查询展示全部 5 个候选 + 上下边框");
    let terminal = draw(&state, &config);
    let text: String = (panel.rect.y..panel.rect.bottom())
        .map(|y| line_text(&terminal, y))
        .collect::<Vec<_>>()
        .join("\n");
    for label in ["/add", "/bug", "/common", "/refactor", "/style"] {
        assert!(text.contains(label), "缺少候选 {label}: {text}");
    }
}

#[test]
fn template_buttons_drive_clean_status_delete_with_single_undo() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    let area = layout::prompt_text_rect(view.prompt);
    let (content, _) = slash_template_content(SlashCommandId::Add);
    update::apply_editor(&mut state, EditorCommand::InsertText(content.into()));

    // 复制正文不修改文本（标题占位行保留）。
    let body = update::copy_template_block(&mut state, 0).expect("body");
    assert!(body.starts_with("# Add Requirement"), "{body}");
    assert!(!body.contains("&&&"));

    // 状态按钮 todo → in_progress，渲染出 [run]。
    let buttons = ui::prompt::template_buttons(&state.prompt, area);
    let status = buttons
        .iter()
        .find(|hit| hit.button == ui::prompt::TemplateBlockButton::Status)
        .copied()
        .expect("status button");
    update::toggle_template_status(&mut state, status.line);
    assert!(state.prompt.text().contains("--end in_progress"));
    let terminal = draw(&state, &config);
    assert!(line_text(&terminal, area.y).contains("[run]"));

    // 清理按钮：移除未填写的空项，保留正文标题与结束行。
    let clean = buttons
        .iter()
        .find(|hit| hit.button == ui::prompt::TemplateBlockButton::Clean)
        .copied()
        .expect("clean button");
    update::clean_template_block(&mut state, clean.line);
    assert!(state.prompt.text().contains("# Add Requirement"));
    assert!(!state.prompt.text().contains("- Reference: "));
    assert!(state.prompt.text().contains("--end in_progress"));

    // 单步撤销恢复清理前内容。
    update::apply_editor(&mut state, EditorCommand::Undo);
    assert!(state.prompt.text().contains("- Reference: "));

    // 删除按钮：整块移除。
    let buttons = ui::prompt::template_buttons(&state.prompt, area);
    let delete = buttons
        .iter()
        .find(|hit| hit.button == ui::prompt::TemplateBlockButton::Delete)
        .copied()
        .expect("delete button");
    update::delete_template_block(&mut state, delete.line);
    assert_eq!(state.prompt.text(), "");
}

#[test]
fn file_mention_renders_yellow_underline() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertText("@src/app.rs".into()));

    let terminal = draw(&state, &config);
    let area = layout::prompt_text_rect(view.prompt);
    let cell = &terminal.backend().buffer()[(area.x, area.y)];
    assert_eq!(cell.symbol(), "@");
    assert_eq!(cell.fg, Color::Yellow);
    assert!(cell.modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn slash_trigger_suppressed_inside_existing_template() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    for ch in "/bug".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }
    update::apply_editor(&mut state, EditorCommand::Newline);
    assert!(state.prompt.text().starts_with("&&& bugTemplate --start"));

    // 在起始行末尾换行后继续输入：位于模板块内部，不再弹面板。
    update::apply_editor(&mut state, EditorCommand::LineEnd);
    update::apply_editor(&mut state, EditorCommand::InsertChar('\n'));
    for ch in "/add".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }
    assert!(
        state.prompt.slash_panel().is_none(),
        "模板块内不弹出斜杠命令面板: {:?}",
        state.prompt.text()
    );
}
