use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::path::PathBuf;

use super::*;
use crate::app::overlay::Overlay;
use crate::app::prompt::PromptRowKind;
use crate::app::state::{AppState, PaneView, RightTab};
use crate::config::Config;
use crate::event::TerminalEvent;
use crate::event::session::Geometry;
use crate::layout::{self, PaneId};
use crate::ui;

pub(crate) fn geometry(state: &AppState, config: &Config, screen: Rect) -> Geometry {
    let view = ui::layout::compute(
        screen,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let mut rects = if state.prompt_left && state.panes_collapsed {
        Vec::new()
    } else {
        layout::pane_rects(
            &state.active_tab().layout,
            view.panes,
            config.min_pane_width,
            config.min_pane_height,
        )
    };
    if !state.prompt_collapsed {
        rects.push((state.prompt.id(), view.prompt));
    }
    Geometry {
        screen,
        rects,
        view,
    }
}

pub(crate) fn mouse(kind: MouseEventKind, column: u16, row: u16) -> TerminalEvent {
    TerminalEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// demo 状态并把全部窗格置为终端视图：事件交互测试的默认前置。
pub(crate) fn demo_terminal() -> AppState {
    let mut state = AppState::demo();
    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.view = PaneView::Terminal;
                }
            }
        }
    }
    state
}

/// 侧栏折叠按钮矩形：tooltip 悬停测试的固定目标。
pub(crate) fn collapse_sidebar_area(geo: &Geometry) -> Rect {
    ui::collapse_buttons(&geo.view, false)
        .into_iter()
        .find(|button| button.target == ui::CollapseTarget::Sidebar)
        .expect("sidebar collapse button visible")
        .area
}

/// 窗格视图（测试断言用）。
pub(crate) fn view_of(state: &AppState, pane: PaneId) -> PaneView {
    state.pane_anywhere(pane).expect("pane exists").view
}

/// 按状态重新计算窗格矩形（拖拽后断言用）。
pub(crate) fn pane_rects_of(
    state: &AppState,
    geo: &Geometry,
    config: &Config,
) -> Vec<(PaneId, Rect)> {
    layout::pane_rects(
        &state.active_tab().layout,
        geo.view.panes,
        config.min_pane_width,
        config.min_pane_height,
    )
}

pub(crate) fn rect_of(rects: &[(PaneId, Rect)], pane: PaneId) -> Rect {
    rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("pane rect")
}

/// 目标窗格顶边框右端的视图切换按钮矩形。
pub(crate) fn toggle_button_of(geo: &Geometry, pane: PaneId) -> Rect {
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .expect("pane rect");
    ui::main_content::toggle_button(*rect).expect("toggle button visible")
}

/// 给当前焦点窗格挂 Agent 快照；返回窗格标识。
pub(crate) fn mount_agent(
    state: &mut AppState,
    kind: crate::detect::AgentKind,
    agent_state: crate::detect::AgentState,
) -> PaneId {
    let pane = state.active_tab().layout.focus();
    update::update_pane_agent(
        state,
        pane,
        Some(crate::detect::PaneAgentSnapshot {
            kind,
            state: agent_state,
            title: None,
        }),
    );
    pane
}

/// 追加 `count` 个各带一个 Agent 的标签。
pub(crate) fn create_agent_tabs(state: &mut AppState, count: usize) {
    use crate::detect::{AgentKind, AgentState};
    for _ in 0..count {
        update::create_tab(state);
        mount_agent(state, AgentKind::Codex, AgentState::Working);
    }
}

/// 容器顶边框上指定 tab 的矩形（按几何取，避免硬编码列）。
pub(crate) fn right_tab_rect(geo: &Geometry, tab: RightTab) -> Rect {
    ui::right_panel::tab_rects(geo.view.prompt)
        .into_iter()
        .find_map(|(candidate, rect)| (candidate == tab).then_some(rect))
        .expect("right tab visible")
}

/// 目标窗格内容区矩形。
pub(crate) fn pane_inner(geo: &Geometry, pane: PaneId) -> Rect {
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .expect("pane rect");
    layout::pane_inner_rect(*rect)
}

/// 给窗格塞满回滚历史，并滚回顶部 5 行。
pub(crate) fn scrolled_pane(state: &mut AppState, pane: PaneId) {
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    for i in 0..40 {
        target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
    }
    assert!(target.terminal.scroll_display(5));
    assert!(target.terminal.display_offset() > 0);
}

// ===== lx `@` 提及面板鼠标路由 =====

/// 为焦点 lx 窗格准备提及面板：设根、键入 @、写入缓存。
pub(crate) fn seed_lx_mention(
    state: &mut AppState,
    query: &str,
    entries: Vec<crate::app::markdown::MentionEntry>,
) {
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.cwd = Some(std::path::PathBuf::from("/work/demo"));
    }
    update::sync_lx_mention_root(state, pane);
    for ch in query.chars() {
        update::apply_lx_input(state, crate::app::actions::LxInput::Char(ch));
    }
    let (generation, root) = state
        .pane_mut_anywhere(pane)
        .and_then(|p| p.lx.take_mention_scan_request())
        .expect("scan request");
    update::apply_lx_mention_entries(state, pane, generation, &root, entries);
}

// ===== ask 面板接线：点击吞掉、粘贴只进手写缓冲 =====

/// 流中窗格 + 一道两选项的挂起提问（真实 request 路径会跑串行锁，这里直接构造，
/// 但 `request` 必须是 busy，否则面板会被 sweep 当成孤儿清掉）。
pub(crate) fn busy_ask_state() -> (AppState, PaneId) {
    use crate::app::state::{AskState, RequestKey, RequestState};
    use crate::chat::tools::ask::{AskOption, AskQuestion};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: "sess-ask".into(),
        generation: 1,
    };
    let question = AskQuestion {
        id: "layout".into(),
        header: "布局".into(),
        question: "用哪种布局？".into(),
        multi_select: false,
        options: vec![
            AskOption {
                label: "two columns".into(),
                description: "left list".into(),
                preview: Some("┌──┐\n└──┘".into()),
            },
            AskOption {
                label: "stacked".into(),
                description: "top down".into(),
                preview: None,
            },
        ],
    };
    let target = state.pane_mut_anywhere(pane).expect("focused pane");
    target.lx.input = "draft".into();
    target.lx.chat.session_id = Some(key.session.clone());
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: "a1".into(),
    };
    target.lx.ask = Some(AskState::new(key, "call".into(), vec![question]));
    (state, pane)
}

/// 面板内第一处命中 `AskHit::Option(0)` 的坐标（几何由渲染层给出，事件层不重算）。
pub(crate) fn ask_option_point(state: &AppState, pane: PaneId, geo: &Geometry) -> (u16, u16) {
    let rect = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("pane rect");
    let inner = layout::pane_inner_rect(rect);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let area = ui::main_content::lx::ask_rect(inner, lx).expect("ask rect");
    for row in area.y..area.bottom() {
        for column in area.x..area.right() {
            if ui::main_content::lx::ask_hit(inner, lx, column, row)
                == Some(crate::app::state::AskHit::Option(0))
            {
                return (column, row);
            }
        }
    }
    panic!("找不到选项行");
}

/// 让 prompt 文本溢出视口：按视口尺寸 resize 后写入两倍视口高度的行。
pub(crate) fn overflow_prompt(state: &mut AppState, view: &ui::layout::ViewLayout) {
    let text = crate::layout::prompt_text_rect(view.prompt);
    state.prompt.resize(text.width, text.height);
    for _ in 0..=usize::from(text.height) {
        state.prompt.insert_str("line\n");
    }
    crate::app::update::set_prompt_scroll(state, 0);
}

/// prompt 工具栏按钮矩形（按几何取，避免硬编码列）。
pub(crate) fn toolbar_button(geo: &Geometry, button: layout::PromptToolbarButton) -> Rect {
    layout::prompt_toolbar_button_rect(geo.view.prompt, button, PromptMode::Editor)
        .expect("toolbar button visible")
}

/// 构造已进入子目录的引用文件夹搜索弹窗（Ctrl/Cmd+Z 回退的前置）。
pub(crate) fn referenced_search_entered_state() -> AppState {
    let mut state = demo_terminal();
    let root = std::path::PathBuf::from("/tmp/lx-ref-z/root");
    let sub = root.join("docs");
    state.overlay = Some(Overlay::ReferencedFolderSearch(
        crate::app::overlay::ReferencedFolderSearch {
            anchor: (0, 0),
            folder: 1,
            key: Some(std::path::PathBuf::from("/tmp/lx-ref-z")),
            root: sub,
            initial_root: root.clone(),
            stack: vec![root],
            query: crate::app::overlay::TextInput::new(""),
            entries: Vec::new(),
            items: Vec::new(),
            scan_truncated: false,
            truncated: false,
            selected: 0,
            viewport: None,
            loading: false,
            failed: false,
            request: Some(1),
        },
    ));
    state
}

/// 打开引用文件夹目录弹窗并放入两条记录。
pub(crate) fn referenced_menu_state() -> AppState {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let mut state = demo_terminal();
    state.overlay = Some(Overlay::ReferencedFolderMenu(
        crate::app::overlay::ReferencedFolderMenu {
            anchor: (0, 0),
            key: Some(std::path::PathBuf::from("/tmp/lx-ref-menu")),
            rows: vec![
                ReferencedFolderMenuRow::Folder {
                    id: 1,
                    path: std::path::PathBuf::from("/tmp/lx-ref-menu/alpha"),
                },
                ReferencedFolderMenuRow::Folder {
                    id: 2,
                    path: std::path::PathBuf::from("/tmp/lx-ref-menu/beta"),
                },
                ReferencedFolderMenuRow::Add,
            ],
            selected: 0,
            hover: None,
            viewport: None,
        },
    ));
    state
}

/// 弹窗几何（事件层与渲染层共用同一 layout）；`screen` 必须与事件几何一致。
pub(crate) fn referenced_menu_layout(
    state: &AppState,
    geo: &Geometry,
) -> crate::ui::overlay::ReferencedMenuLayout {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    crate::ui::overlay::referenced_menu_layout(geo.screen, menu)
}

/// 打开引用文件夹目录弹窗并放入 `count` 条记录（末尾恒为新建行），用于溢出滚动测试。
pub(crate) fn referenced_menu_overflow_state(count: usize) -> AppState {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let mut state = demo_terminal();
    let mut rows: Vec<ReferencedFolderMenuRow> = (0..count)
        .map(|id| ReferencedFolderMenuRow::Folder {
            id: id as i64,
            path: std::path::PathBuf::from(format!("/tmp/lx-ref-menu/f{id:02}")),
        })
        .collect();
    rows.push(ReferencedFolderMenuRow::Add);
    state.overlay = Some(Overlay::ReferencedFolderMenu(
        crate::app::overlay::ReferencedFolderMenu {
            anchor: (0, 0),
            key: Some(std::path::PathBuf::from("/tmp/lx-ref-menu")),
            rows,
            selected: 0,
            hover: None,
            viewport: None,
        },
    ));
    state
}

/// 构造 prompt 工具栏测试场景：宽右栏、已同步几何。
pub(crate) fn toolbar_geometry(state: &mut AppState) -> (Config, Geometry) {
    let config = Config::default();
    state.prompt_width = 40;
    let geo = geometry(state, &config, Rect::new(0, 0, 120, 30));
    crate::app::update::resize_panes(state, &geo.rects);
    (config, geo)
}

/// prompt 树测试场景：进入树视图并重建树，返回匹配行的点击坐标；
/// 树行自搜索框与分割线之下开始（行高 1）。
pub(crate) fn prompt_tree_click(
    state: &mut AppState,
    geo: &Geometry,
    folders: Vec<crate::app::prompt::PromptFolder>,
    items: Vec<crate::app::prompt::PromptItem>,
    kind: PromptRowKind,
) -> (u16, u16) {
    update::show_prompt_tree(state);
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(folders, items);
    let area = layout::prompt_text_rect(geo.view.prompt);
    let index = state.workspaces[0]
        .prompt_panel
        .rows()
        .iter()
        .position(|row| row.kind == kind)
        .expect("row visible");
    (area.x + 1, area.y + 2 + index as u16)
}

/// 左键单击同一坐标：一次按下紧接一次松开（点击语义在松开时生效）。
pub(crate) fn prompt_tree_click_once(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    point: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, point.0, point.1),
            state,
            &mut sessions,
            geo,
            config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
}

/// 左键双击同一坐标：连续两次「按下 + 松开」。
pub(crate) fn prompt_tree_double_click(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    point: (u16, u16),
) {
    for _ in 0..2 {
        prompt_tree_click_once(state, geo, config, point);
    }
}

/// 一次按下 → 跨行拖动 → 松开，返回松开后的 `dirty`。
pub(crate) fn prompt_tree_drag(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    from: (u16, u16),
    to: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    for (kind, point) in [
        (MouseEventKind::Down(MouseButton::Left), from),
        (MouseEventKind::Drag(MouseButton::Left), to),
        (MouseEventKind::Up(MouseButton::Left), to),
    ] {
        handle_terminal_event(
            mouse(kind, point.0, point.1),
            state,
            &mut sessions,
            geo,
            config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
}

/// 构造带 add 模板块的宽屏场景，返回几何与按钮行文本区。
pub(crate) fn template_button_state() -> (AppState, Config, Geometry, Rect) {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    // 模板块含标题空行与底部空行共 14 行，prompt 需足够高以完整显示起始行按钮。
    state.prompt.resize(78, 16);
    crate::app::update::focus_prompt(&mut state);
    let (content, _) =
        crate::app::markdown::slash_template_content(crate::app::markdown::SlashCommandId::Add);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText(content.into()),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 220, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    (state, config, geo, area)
}

// ===== 分页鼠标命中 =====

/// 预置一个两页条目并进入编辑视图的 demo 状态。
pub(crate) fn paged_prompt_state() -> AppState {
    use crate::app::prompt::{PromptItem, PromptPage};
    let mut state = demo_terminal();
    let mut item = PromptItem::single(7, None, "paged", "alpha");
    item.pages.push(PromptPage::saved(70, "Page 2", "beta"));
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    state
}

// ===== 粘贴入口与结果路由 =====

/// 独立临时目录（粘贴测试文件隔离）。
pub(crate) fn paste_temp_dir(label: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("lx-tui-event-paste-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}
