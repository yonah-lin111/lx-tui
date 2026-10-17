//! 集成测试：prompt 编辑区里 `@ref/短名` 引用格式的插入、高亮与整块删除。

use lx_tui::app::actions::EditorCommand;
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// 已同步几何的 prompt 场景（120x24，右栏够宽）。
fn ready_state() -> (AppState, Config, ui::layout::ViewLayout) {
    let config = Config::default();
    let mut state = AppState::demo();
    let screen = Rect::new(0, 0, 120, 24);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
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

fn type_text(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        update::apply_editor(state, EditorCommand::InsertChar(ch));
    }
}

/// 文档里先有一个模板块文件提及（长路径），用户随后输入 `design` 并用面板确认。
#[test]
fn selecting_candidate_inserts_last_two_segments() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    state
        .prompt
        .set_text("需求见 @docs/feature/lx-at-mention/design.md 这份文档\n");
    type_text(&mut state, "design");

    let panel = state.prompt.refer_panel().expect("reference 面板打开");
    assert_eq!(
        panel.items()[0].text,
        "@docs/feature/lx-at-mention/design.md",
        "候选来自模板块里已有的长路径"
    );

    update::apply_editor(&mut state, EditorCommand::Newline);
    assert!(
        state
            .prompt
            .text()
            .ends_with("@ref/lx-at-mention/design.md "),
        "确认后插入末两段短名: {}",
        state.prompt.text()
    );
}

/// 引用根形式的目录候选：短名带尾斜杠，绝不出现根目录歧义。
#[test]
fn directory_candidate_inserts_slash_suffixed_short_name() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    state
        .prompt
        .set_text("@[refer-folder](/Users/yonah/projects/agent/lx-agent) 参考仓库\n");
    type_text(&mut state, "agent");

    assert!(state.prompt.refer_panel().is_some(), "目录候选可匹配");
    update::apply_editor(&mut state, EditorCommand::Newline);
    assert!(
        state.prompt.text().ends_with("@ref/agent/lx-agent/ "),
        "目录短名带尾斜杠: {}",
        state.prompt.text()
    );
    assert!(
        !state.prompt.text().contains("@/agent"),
        "短名不以 / 开头（不会被读成根目录）"
    );
}

/// 短引用有独立高亮：柔紫加粗，与 @ 提及黄、refer-* 三色都不同。
#[test]
fn short_ref_is_highlighted_distinctly() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    state.prompt.set_text("@ref/a/b.md @src/main.rs");
    let mut terminal =
        Terminal::new(TestBackend::new(120, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, &state, &config))
        .expect("draw succeeds");

    let buffer = terminal.backend().buffer();
    let text = layout::prompt_text_rect(view.prompt);
    let short = &buffer[(text.x, text.y)];
    assert_eq!(short.symbol(), "@");
    assert_eq!(short.fg, Color::Indexed(141), "短引用柔紫");
    assert!(short.modifier.contains(Modifier::BOLD), "短引用加粗");
    assert!(!short.modifier.contains(Modifier::UNDERLINED));

    let mention = &buffer[(text.x + "@ref/a/b.md ".len() as u16, text.y)];
    assert_eq!(mention.symbol(), "@");
    assert_eq!(mention.fg, Color::Yellow, "普通提及仍是黄");
    assert!(mention.modifier.contains(Modifier::UNDERLINED));
    assert_ne!(short.fg, mention.fg, "两种 @ 一眼可分");
}

/// 再输入同名片段时，已有的短引用也能被面板重新选中。
#[test]
fn existing_short_ref_is_offered_again() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    state.prompt.set_text("@ref/lx-at-mention/design.md 需求\n");
    type_text(&mut state, "design");

    let panel = state.prompt.refer_panel().expect("panel");
    assert_eq!(panel.items()[0].text, "@ref/lx-at-mention/design.md");
    assert_eq!(panel.items()[0].tag.label(), "ref-file");
    update::apply_editor(&mut state, EditorCommand::Newline);
    assert!(
        state
            .prompt
            .text()
            .ends_with("@ref/lx-at-mention/design.md "),
        "重复确认同一短引用不会变成别的形状: {}",
        state.prompt.text()
    );
}

/// Backspace 把整段短引用（含尾随空白）一次删净。
#[test]
fn backspace_deletes_short_ref_as_one_block() {
    let (mut state, _config, _view) = ready_state();
    update::focus_prompt(&mut state);
    state.prompt.set_text("见 @ref/agent/lx-agent/ ");
    type_text(&mut state, "");
    update::apply_editor(&mut state, EditorCommand::Backspace);
    assert_eq!(state.prompt.text(), "见 ", "整块删除");
}
