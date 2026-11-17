//! `lx` 页底部状态/提示行（`bottom_bar`）布局与纯边框回归测试：
//! - 输入框保留上下 `─` 分割线，底部分割线下方、窗格底边框上方为独立状态/提示行；
//! - 窗格底边框 `╰─…─╯` 在任何模式下都保持纯净，不内嵌状态文本或任务面板提示；
//! - `ask` 激活时隐藏状态行、`task` 打开时切换为面板提示、`area.height < 5` 时整体退化。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use super::*;
use crate::app::state::lx::btw::BtwQaSummary;
use crate::app::state::lx::chat::{ChatStatus, QueuedSubmission};
use crate::app::state::{AskState, RequestKey, TaskPage, TaskTab};
use crate::chat::tools::ask::{AskOption, AskQuestion};
use crate::tiling::{PaneId, pane_inner_rect};

fn render_bordered_pane(pane_rect: Rect, lx: &Lx) -> (Buffer, Rect, Option<(u16, u16)>) {
    let mut buf = Buffer::empty(pane_rect);
    let block = Block::bordered();
    let inner = block.inner(pane_rect);
    block.render(pane_rect, &mut buf);
    let cursor = render(inner, &mut buf, lx, true, None, &mut PaneCache::default());
    (buf, inner, cursor)
}

fn row_string(buf: &Buffer, rect: Rect, y: u16) -> String {
    (rect.x..rect.right())
        .map(|x| buf[(x, y)].symbol())
        .collect()
}

fn expected_clean_bottom_border(width: u16) -> String {
    format!("└{}┘", "─".repeat(usize::from(width.saturating_sub(2))))
}

fn sample_status() -> ChatStatus {
    ChatStatus {
        model_name: Some("deepseek-v4.1".into()),
        provider_name: Some("provider".into()),
        variant: Some("high".into()),
        context_limit: Some(1_000_000),
        input_tokens: Some(258_700),
        cache_hit_percent: Some(70),
        streaming: false,
        omitted_turns: None,
        blocked_subagents: 0,
    }
}

fn sample_ask() -> AskState {
    AskState::new(
        RequestKey {
            pane: PaneId::alloc(),
            session: "s".into(),
            generation: 1,
        },
        "call-1".into(),
        vec![AskQuestion {
            id: "q1".into(),
            header: "layout".into(),
            question: "Pick a layout?".into(),
            multi_select: false,
            options: vec![
                AskOption {
                    label: "A".into(),
                    description: "first".into(),
                    preview: None,
                },
                AskOption {
                    label: "B".into(),
                    description: "second".into(),
                    preview: None,
                },
            ],
        }],
    )
}

#[test]
fn normal_input_places_status_below_bottom_rule_and_keeps_pane_border_clean() {
    let pane_rect = Rect::new(0, 0, 72, 16);
    let mut lx = Lx::new();
    lx.insert_str("hello");
    lx.chat.status = sample_status();

    let (buf, inner, cursor) = render_bordered_pane(pane_rect, &lx);
    assert_eq!(inner, pane_inner_rect(pane_rect));
    let lay = layout(inner, &lx);
    let input = lay.input.expect("input rect");
    let bar = lay.bottom_bar.expect("bottom_bar rect");
    assert_eq!(input.bottom(), bar.y, "输入框底分割线紧接状态行上方");
    assert_eq!(bar.y, inner.bottom() - 1, "状态行在内容区最末行");
    assert_eq!(bar.x, input.x, "状态行左缘与输入框分割线对齐");
    assert_eq!(bar.width, input.width, "状态行与输入框同宽");

    let top_rule = row_string(&buf, inner, input.y);
    let body_line = row_string(&buf, inner, input.y + 1);
    let bottom_rule = row_string(&buf, inner, input.bottom() - 1);
    let status_line = row_string(&buf, inner, bar.y);
    let outer_border = row_string(&buf, pane_rect, pane_rect.bottom() - 1);

    assert!(top_rule.trim().starts_with('─'), "顶分割线：{top_rule}");
    assert!(
        body_line.trim_start().starts_with("> hello"),
        "输入正文：{body_line}"
    );
    assert!(
        bottom_rule.trim().starts_with('─'),
        "底分割线：{bottom_rule}"
    );
    assert_eq!(
        status_line.trim(),
        "deepseek-v4.1 · high · 258.7k(26%) · 70% cached"
    );
    // 左对齐校验：状态首字符 `d` 与底分割线首字符 `─`、提示符 `>` 处于同一列。
    assert_eq!(buf[(input.x, bar.y)].symbol(), "d");
    assert_eq!(buf[(input.x, input.bottom() - 1)].symbol(), "─");
    assert_eq!(buf[(input.x, input.y + 1)].symbol(), ">");

    // 光标位于正文行而非状态行或底边框上。
    let (crow, _) = cursor.expect("cursor");
    assert_eq!(inner.y + crow, input.y + 1);

    // 窗格底边框完全干净、无内嵌文字。
    assert_eq!(outer_border, expected_clean_bottom_border(pane_rect.width));
}

#[test]
fn shell_and_queued_stay_on_bottom_rule_while_status_and_omitted_sit_below() {
    let pane_rect = Rect::new(0, 0, 80, 16);
    let mut lx = Lx::new();
    lx.insert_str("!cargo test");
    lx.chat
        .queue
        .push(QueuedSubmission::Chat { draft: "q1".into() });
    lx.chat
        .queue
        .push(QueuedSubmission::Chat { draft: "q2".into() });
    lx.chat.status = sample_status();
    lx.chat.status.omitted_turns = Some(3);

    let (buf, inner, _) = render_bordered_pane(pane_rect, &lx);
    let lay = layout(inner, &lx);
    let input = lay.input.expect("input");
    let bar = lay.bottom_bar.expect("bottom_bar");

    let bottom_rule = row_string(&buf, inner, input.bottom() - 1);
    let status_line = row_string(&buf, inner, bar.y);
    let outer_border = row_string(&buf, pane_rect, pane_rect.bottom() - 1);

    assert!(
        bottom_rule.contains(&format!("{} · 2 queued", text::LX_INPUT_SHELL_MODE)),
        "Shell mode 与队列提示留在底分割线上：{bottom_rule}"
    );
    assert!(
        status_line.contains("deepseek-v4.1") && status_line.contains("omitted turns: 3"),
        "状态与省略提示留在下方状态行：{status_line}"
    );
    assert!(
        !status_line.contains(text::LX_INPUT_SHELL_MODE),
        "Shell mode 不进入下方状态行：{status_line}"
    );
    assert_eq!(outer_border, expected_clean_bottom_border(pane_rect.width));
}

#[test]
fn narrow_width_below_24_aligns_status_flush_with_input_zero_margin() {
    // 含外边框宽 22 → 内容区宽 20（< 24），MARGIN 退回 0。
    let pane_rect = Rect::new(0, 0, 22, 12);
    let mut lx = Lx::new();
    lx.chat.status = sample_status();

    let (buf, inner, _) = render_bordered_pane(pane_rect, &lx);
    assert_eq!(inner.width, 20);
    let lay = layout(inner, &lx);
    let input = lay.input.expect("input");
    let bar = lay.bottom_bar.expect("bottom_bar");

    assert_eq!(input.x, inner.x);
    assert_eq!(bar.x, inner.x);
    assert_eq!(bar.width, inner.width);
    assert_eq!(buf[(inner.x, bar.y)].symbol(), "d");
    assert_eq!(
        row_string(&buf, pane_rect, pane_rect.bottom() - 1),
        expected_clean_bottom_border(pane_rect.width)
    );
}

#[test]
fn tiny_height_four_degrades_to_wordmark_and_five_renders_status_above_border() {
    // 内容区高 4（总高 6）：退化为居中 `lx` 字标，不画输入框也不写状态或边框。
    let tiny_pane = Rect::new(0, 0, 42, 6);
    let mut lx = Lx::new();
    lx.chat.status = sample_status();

    let tiny_inner = pane_inner_rect(tiny_pane);
    assert_eq!(tiny_inner.height, 4);
    let tiny_lay = layout(tiny_inner, &lx);
    assert!(tiny_lay.fallback, "内容区高 4 必须退化");
    assert!(tiny_lay.input.is_none());
    assert!(tiny_lay.bottom_bar.is_none());

    let (tiny_buf, _, cursor) = render_bordered_pane(tiny_pane, &lx);
    assert!(cursor.is_none());
    let tiny_text = buffer_text(&tiny_buf);
    assert!(tiny_text.contains("lx"), "退化显示字标：{tiny_text}");
    assert!(
        !tiny_text.contains("deepseek"),
        "退化时不绘制状态：{tiny_text}"
    );
    assert_eq!(
        row_string(&tiny_buf, tiny_pane, tiny_pane.bottom() - 1),
        expected_clean_bottom_border(tiny_pane.width)
    );

    // 内容区高 5（总高 7）：正常绘制（上方留 1 行 + 3 行输入框 + 1 行状态）。
    let min_pane = Rect::new(0, 0, 42, 7);
    let min_inner = pane_inner_rect(min_pane);
    assert_eq!(min_inner.height, 5);
    let min_lay = layout(min_inner, &lx);
    assert!(!min_lay.fallback);
    let input = min_lay.input.expect("input");
    let bar = min_lay.bottom_bar.expect("bottom_bar");
    assert_eq!(input.y, min_inner.y + 1, "上方仍留 1 行");
    assert_eq!(input.bottom(), bar.y);
    assert_eq!(bar.y, min_inner.bottom() - 1);

    let (min_buf, _, min_cursor) = render_bordered_pane(min_pane, &lx);
    assert!(min_cursor.is_some());
    assert!(
        row_string(&min_buf, min_inner, bar.y).contains("deepseek"),
        "高 5 正常展示状态行"
    );
    assert_eq!(
        row_string(&min_buf, min_pane, min_pane.bottom() - 1),
        expected_clean_bottom_border(min_pane.width)
    );
}

#[test]
fn ask_panel_hides_status_and_keeps_pane_border_clean() {
    let pane_rect = Rect::new(0, 0, 72, 16);
    let mut lx = Lx::new();
    lx.chat.status = sample_status();
    lx.ask = Some(sample_ask());

    let (buf, inner, _) = render_bordered_pane(pane_rect, &lx);
    let lay = layout(inner, &lx);
    assert!(lay.bottom_bar.is_none(), "ask 激活时隐藏底部状态栏");
    let ask_rect = lay.ask.expect("ask rect");
    assert_eq!(ask_rect.bottom(), inner.bottom(), "ask 底栏贴内容区底边");

    let bottom_inner = row_string(&buf, inner, inner.bottom() - 1);
    let outer_border = row_string(&buf, pane_rect, pane_rect.bottom() - 1);
    assert!(
        bottom_inner.contains("select") && bottom_inner.contains("esc"),
        "保留 ask 自带底栏：{bottom_inner}"
    );
    assert!(
        !bottom_inner.contains("deepseek"),
        "状态行不得覆盖 ask 底栏：{bottom_inner}"
    );
    assert_eq!(outer_border, expected_clean_bottom_border(pane_rect.width));

    // 关闭 ask 后状态行重新回到窗格底边框上方。
    lx.ask = None;
    let (restored, _, _) = render_bordered_pane(pane_rect, &lx);
    assert!(
        row_string(&restored, inner, inner.bottom() - 1).contains("deepseek-v4.1"),
        "关闭 ask 后恢复状态行"
    );
    assert_eq!(
        row_string(&restored, pane_rect, pane_rect.bottom() - 1),
        expected_clean_bottom_border(pane_rect.width)
    );
}

#[test]
fn task_panel_list_and_btw_edit_render_footer_above_clean_pane_border() {
    let pane_rect = Rect::new(0, 0, 72, 16);
    let mut lx = Lx::new();
    lx.chat.status = sample_status();
    lx.task = Some(TaskPage::default());

    let (buf, inner, _) = render_bordered_pane(pane_rect, &lx);
    let lay = layout(inner, &lx);
    let list = lay.task_list.expect("task list");
    let bar = lay.bottom_bar.expect("bottom_bar");
    assert_eq!(list.bottom(), bar.y, "任务列表贴在提示行上方");
    assert_eq!(bar.y, inner.bottom() - 1);

    let footer_line = row_string(&buf, inner, bar.y);
    assert!(
        footer_line.contains(&text::LX_TASK_HINTS_PROCESSES[0].label()),
        "任务面板提示在底部提示行：{footer_line}"
    );
    assert_eq!(
        buf[(bar.x, bar.y)].symbol(),
        "[",
        "提示左缘与面板内容对齐：{footer_line}"
    );
    assert!(
        !footer_line.contains("deepseek"),
        "任务面板期间不显示模型名：{footer_line}"
    );
    assert_eq!(
        row_string(&buf, pane_rect, pane_rect.bottom() - 1),
        expected_clean_bottom_border(pane_rect.width)
    );

    // 较矮窗格（内容区高 6）：task_list 自动夹在底部提示行之上，不越界也不覆盖提示行。
    let cramped_pane = Rect::new(0, 0, 72, 8);
    let (cramped_buf, cramped_inner, _) = render_bordered_pane(cramped_pane, &lx);
    let cramped_lay = layout(cramped_inner, &lx);
    let cramped_list = cramped_lay.task_list.expect("cramped list");
    let cramped_bar = cramped_lay.bottom_bar.expect("cramped bar");
    assert_eq!(cramped_list.y, cramped_inner.y);
    assert_eq!(cramped_list.bottom(), cramped_bar.y);
    assert!(
        row_string(&cramped_buf, cramped_inner, cramped_bar.y)
            .contains(&text::LX_TASK_HINTS_PROCESSES[0].label())
    );
    assert_eq!(
        row_string(&cramped_buf, cramped_pane, cramped_pane.bottom() - 1),
        expected_clean_bottom_border(cramped_pane.width)
    );

    // BTW 编辑态：composer 上下分割线之上/之中绘制编辑框，下方紧接提示行，底边框纯净。
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..TaskPage::default()
    });
    lx.btw.qa.push(BtwQaSummary {
        qa_id: "qa-1".into(),
        title: "Why?".into(),
        has_thread: true,
        status: "idle".into(),
    });
    lx.btw
        .ensure_thread("qa-1")
        .composer
        .editor_mut()
        .insert_str("follow-up");
    lx.btw.editing = true;

    let (btw_buf, _, btw_cursor) = render_bordered_pane(pane_rect, &lx);
    let btw_lay = layout(inner, &lx);
    let composer = btw_lay.task_list.expect("composer");
    let btw_bar = btw_lay.bottom_bar.expect("btw bar");
    assert_eq!(composer.bottom(), btw_bar.y);
    assert!(btw_cursor.is_some());
    assert!(
        row_string(&btw_buf, inner, composer.bottom() - 1)
            .trim()
            .starts_with('─'),
        "BTW 编辑框底分割线仍在 composer 最末行"
    );
    let btw_footer = row_string(&btw_buf, inner, btw_bar.y);
    assert!(
        btw_footer.contains(&text::LX_BTW_HINTS_EDIT[0].label()),
        "BTW 编辑态提示在底部提示行：{btw_footer}"
    );
    assert_eq!(
        row_string(&btw_buf, pane_rect, pane_rect.bottom() - 1),
        expected_clean_bottom_border(pane_rect.width)
    );
}

#[test]
fn tiny_height_with_history_and_task_or_ask_degrades_to_centered_wordmark() {
    use crate::app::state::{AssistantStepView, ChatMessageView, ChatPartView, LxChat};

    let tiny_pane = Rect::new(0, 0, 42, 6);
    let tiny_inner = pane_inner_rect(tiny_pane);
    assert_eq!(tiny_inner.height, 4);

    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "u0".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: "hello".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });

    // 1. 有历史且 task 打开在内容高 4 行时：必须退化为居中 lx 字标，不得渲染出整块空白。
    let mut lx = Lx::new();
    lx.chat = chat;
    lx.task = Some(TaskPage::default());
    assert!(layout(tiny_inner, &lx).fallback);
    assert!(history_rect(tiny_inner, &lx).is_none());
    assert!(history_wheel_region(tiny_inner, &lx).is_none());
    let (buf, _, _) = render_bordered_pane(tiny_pane, &lx);
    let text = buffer_text(&buf);
    assert!(
        text.contains("lx"),
        "高 4 含历史+task 必须显示居中字标而非空白：{text}"
    );
    assert_eq!(
        row_string(&buf, tiny_pane, tiny_pane.bottom() - 1),
        expected_clean_bottom_border(tiny_pane.width)
    );

    // 2. ask 与 task 同时存在在内容高 4 行时：同样一致退化（task 优先级更高，不放行 3 行无列表的残缺面板）。
    lx.ask = Some(sample_ask());
    assert!(
        layout(tiny_inner, &lx).fallback,
        "ask 与 task 共存高 4 时必须一致退化"
    );
    let (both_buf, _, _) = render_bordered_pane(tiny_pane, &lx);
    assert!(buffer_text(&both_buf).contains("lx"));
}

#[test]
fn agents_editing_renders_footer_above_clean_pane_border() {
    let pane_rect = Rect::new(0, 0, 72, 16);
    let mut lx = Lx::new();
    lx.chat.status = sample_status();
    lx.agents
        .runs
        .push(crate::app::state::SubagentRun::from_record(
            &crate::db::chat::SubagentRecord {
                agent_id: "ag-1".into(),
                child_session_id: "child-1".into(),
                parent_session_id: "parent".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                status: "completed".into(),
                title: "Explore".into(),
                created_at_ms: 0,
                completed_at_ms: Some(1),
                report_json: None,
                report_delivered: true,
                nickname: Some("Turing".into()),
                agent_path: "/root/turing".into(),
                model_provider: String::new(),
                model_id: String::new(),
                model_variant: None,
            },
        ));
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        agent_edit: Some("ag-1".into()),
        ..TaskPage::default()
    });
    lx.agents.ensure_composer("ag-1");

    let (buf, inner, cursor) = render_bordered_pane(pane_rect, &lx);
    let lay = layout(inner, &lx);
    let composer = lay.task_list.expect("agents composer");
    let bar = lay.bottom_bar.expect("agents bottom bar");
    assert_eq!(composer.bottom(), bar.y);
    assert!(cursor.is_some());
    assert!(
        row_string(&buf, inner, bar.y).contains(&text::LX_TASK_HINTS_AGENTS_EDIT[0].label()),
        "Agents 编辑态底部提示在 bottom_bar"
    );
    assert_eq!(
        row_string(&buf, pane_rect, pane_rect.bottom() - 1),
        expected_clean_bottom_border(pane_rect.width)
    );
}
