//! 详情 composer 的胶囊、底栏提示与 `/` 面板锚点。

use super::*;
use crate::app::state::{SlashPanel, TaskPage, TaskTab};

/// Agents composer 的 Lx：一个已结束/运行中的子 agent + 编辑态。
fn agent_composer_lx(status: &str) -> Lx {
    let mut lx = lx_with_messages();
    let mut run = tree_agent("a", "/root/turing", status, None);
    run.nickname = Some("Turing".into());
    lx.agents.runs.push(run);
    lx.agents.selected_id = Some("a".into());
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        agent_edit: Some("a".into()),
        ..TaskPage::default()
    });
    lx.agents.ensure_composer("a");
    lx
}

/// BTW composer 的 Lx：当前 QA 有线程与标题。
fn btw_composer_lx() -> Lx {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..TaskPage::default()
    });
    lx.btw.qa.push(crate::app::state::lx::btw::BtwQaSummary {
        qa_id: "qa-1".into(),
        title: "question one".into(),
        has_thread: true,
        status: "idle".into(),
    });
    lx.btw.ensure_thread("qa-1");
    lx.btw.editing = true;
    lx
}

#[test]
fn agent_badge_shows_private_then_steer_then_compacting() {
    let mut lx = agent_composer_lx("completed");
    let lines = render_lines(&lx, 90, 20);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Turing [executor] · private")),
        "空闲详情显示 private：{lines:?}"
    );

    lx.agents.runs[0].status = "running".into();
    lx.agents.runs[0]
        .chat
        .pending_steers
        .push(crate::app::state::lx::chat::steer::PendingSteer {
            text: "steer me".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        });
    let lines = render_lines(&lx, 90, 20);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Turing [executor] · steer 1")),
        "运行中显示 steer：{lines:?}"
    );

    lx.agents.runs[0].status = "completed".into();
    lx.agents.runs[0].chat.pending_steers.clear();
    lx.agents.runs[0].compact.start("op-1".into(), 0);
    let lines = render_lines(&lx, 90, 20);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Turing [executor] · compacting")),
        "压缩中显示 compacting：{lines:?}"
    );
}

#[test]
fn btw_badge_appends_compacting_only_when_it_fits() {
    let mut lx = btw_composer_lx();
    lx.btw
        .active_thread_mut()
        .expect("thread")
        .compact
        .start("op-1".into(), 0);
    let wide = render_lines(&lx, 90, 20).join("\n");
    assert!(
        wide.contains("question one · compacting"),
        "宽窗格追加 compacting：{wide}"
    );
    let narrow = render_lines(&lx, 34, 20).join("\n");
    assert!(
        !narrow.contains("compacting"),
        "窄窗格整段丢弃后缀：{narrow}"
    );
    assert!(narrow.contains("QA 1"), "原胶囊仍完整：{narrow}");
}

#[test]
fn edit_footer_shows_commands_hint() {
    let lx = agent_composer_lx("completed");
    let lines = render_lines(&lx, 90, 20);
    let footer = &lines[lines.len() - 2];
    assert!(footer.contains("commands"), "底栏含 / 命令提示：{footer}");
    assert!(footer.contains("/"), "提示键位是 `/`：{footer}");
    assert!(
        !footer.contains("mode"),
        "不再显示 Shift+Tab 模式：{footer}"
    );
    assert!(
        lines.last().expect("outer").trim().is_empty(),
        "窗格底边框行保持纯净"
    );
}

#[test]
fn slash_panel_anchors_above_composer() {
    let mut lx = agent_composer_lx("completed");
    lx.agents
        .composer_mut("a")
        .expect("composer")
        .editor_mut()
        .slash = Some(SlashPanel::Commands {
        items: crate::app::state::lx::slash::composer::entries(
            crate::app::state::lx::slash::composer::DetailContext::Agent,
            "",
        ),
        window: crate::app::state::PanelWindow::default(),
        query: "/".into(),
    });
    let area = Rect::new(0, 0, 90, 24);
    let input = crate::ui::main_content::lx::input_rect(area, &lx).expect("composer rect");
    let panel = crate::ui::main_content::lx::slash_layout(area, &lx).expect("slash layout");
    assert_eq!(panel.rect.bottom(), input.y, "面板底边贴 composer 上沿");
    assert!(panel.rect.height >= 3, "至少标题 + 一条候选");
}

#[test]
fn badges_with_zwj_emoji_and_controls_never_overwrite_right_rule() {
    // Agents 昵称含 ZWJ emoji + 换行控制符：顶边最右列必须保持为 `─`。
    let mut lx = agent_composer_lx("completed");
    lx.agents.runs[0].nickname = Some("👨‍👩‍👧\nTuring".into());
    let lines = render_lines(&lx, 40, 14);
    let top_rule = lines
        .iter()
        .find(|line| line.contains(crate::ui::text::LX_TASK_AGENT_INPUT_MARK))
        .expect("agents composer top rule");
    assert!(
        top_rule.trim().ends_with('─'),
        "Agents 右侧胶囊不得覆盖最右分割线: {top_rule:?}"
    );

    // BTW 标题含 ZWJ emoji + 换行控制符：顶边最右列必须保持为 `─`。
    let mut btw = btw_composer_lx();
    btw.btw.qa[0].title = "👨‍👩‍👧\tquestion\nnext".into();
    let btw_lines = render_lines(&btw, 40, 14);
    let btw_top = btw_lines
        .iter()
        .find(|line| line.contains(crate::ui::text::LX_BTW_INPUT_MARK))
        .expect("btw composer top rule");
    assert!(
        btw_top.trim().ends_with('─'),
        "BTW 右侧胶囊不得覆盖最右分割线: {btw_top:?}"
    );
}
