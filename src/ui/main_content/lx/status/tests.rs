//! 状态测试：上下文右侧运行指标（后台任务 / 子 agent，圆点分隔）、
//! `模型名 · 当前用量(百分比)`（**不带 `context:` 前缀、不显示供应商**）、
//! 显示与隐藏（无消耗整段隐藏）、窄宽度降级（丢上下文占用）、
//! 模型名硬切不省略号、窗格底边框保持纯净（均绘制在输入框下方、窗格底边框上方）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::*;
use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;

fn status(
    model: Option<&str>,
    provider: Option<&str>,
    context: Option<u64>,
    input: Option<u64>,
    streaming: bool,
) -> ChatStatus {
    ChatStatus {
        model_name: model.map(str::to_string),
        provider_name: provider.map(str::to_string),
        variant: None,
        context_limit: context,
        input_tokens: input,
        cache_hit_percent: None,
        streaming,
        omitted_turns: None,
        blocked_subagents: 0,
    }
}

fn lx_with_status(status: ChatStatus) -> Lx {
    let mut lx = Lx::new();
    lx.chat.status = status;
    lx
}

/// 真实窗格几何：窗格矩形 `rect` 的边框内缩 1 得内容区。
fn pane(rect: Rect) -> Rect {
    crate::tiling::pane_inner_rect(rect)
}

/// 模拟 `lx::layout` 给底部状态行分配的矩形（位于内容区最底行，与输入框同宽对齐）。
fn bar_rect(inner: Rect) -> Rect {
    let margin = if inner.width >= 24 { 1 } else { 0 };
    Rect::new(
        inner.x + margin,
        inner.bottom().saturating_sub(1),
        inner.width.saturating_sub(margin * 2),
        1,
    )
}

/// 模拟带底边框的窗格：底边框行 `inner.bottom()` 铺 `╰─…─╯`，状态画在 `inner.bottom() - 1`；
/// 同时断言底边框行未被污染，最后返回状态行文本。
fn render_bar(lx: &Lx, width: u16) -> String {
    let rect = Rect::new(0, 0, width, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    let border_row = inner.bottom();
    for x in rect.x..rect.right() {
        let ch = if x == rect.x {
            '╰'
        } else if x == rect.right() - 1 {
            '╯'
        } else {
            '─'
        };
        if let Some(cell) = buf.cell_mut((x, border_row)) {
            cell.set_char(ch);
        }
    }
    let bar = bar_rect(inner);
    render(&mut buf, bar, lx);
    let border: String = (rect.x..rect.right())
        .map(|x| buf.cell((x, border_row)).map(|c| c.symbol()).unwrap_or(" "))
        .collect();
    assert_eq!(
        border,
        format!("╰{}╯", "─".repeat(usize::from(width.saturating_sub(2)))),
        "窗格底边框必须保持纯净，不得内嵌任何状态文本"
    );
    (bar.x..bar.right())
        .map(|x| buf.cell((x, bar.y)).map(|c| c.symbol()).unwrap_or(" "))
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn context_text_is_usage_with_percent() {
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(258_700), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("258.7k(26%)"),
        "一位小数 + 四舍五入百分比"
    );
    let st = status(Some("M"), Some("P"), Some(200_000), Some(45_200), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("45.2k(23%)"));
    // 整数值不带 `.0`；用量超过上限时百分比夹到 100%。
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(720_000), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("720k(72%)"));
    let st = status(Some("M"), Some("P"), Some(100), Some(200), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("200(100%)"),
        "超过上限夹到 100%"
    );
}

#[test]
fn tiny_usage_shows_exact_count_with_zero_percent() {
    // 小用量按一位小数原样显示（百分比取整到 0% 也照常给出）。
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(2_500), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("2.5k(0%)"));
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(999), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("999(0%)"));
}

#[test]
fn context_without_usage_is_hidden_entirely() {
    // 未知上限 / 未知用量 / 零消耗 / 零上限 都整段隐藏。
    for value in [
        status(Some("M"), Some("P"), None, None, false),
        status(Some("M"), Some("P"), None, Some(10), false),
        status(Some("M"), Some("P"), Some(1_000), None, false),
        status(Some("M"), Some("P"), Some(1_000), Some(0), false),
        status(Some("M"), Some("P"), Some(0), Some(10), false),
    ] {
        assert_eq!(context_text_for_test(&value), None, "无消耗整段隐藏");
    }
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(0), false));
    let line = render_bar(&lx, 60);
    assert!(!line.contains("(0%)"), "整段隐藏：{line}");
}

#[test]
fn context_usage_shows_usage_with_percent_on_status_bar() {
    let lx = lx_with_status(status(
        Some("M"),
        Some("P"),
        Some(1_000_000),
        Some(258_700),
        false,
    ));
    let line = render_bar(&lx, 60);
    assert!(
        line.contains("258.7k(26%)"),
        "占用显示 `258.7k(26%)`（无 `context:` 前缀）：{line}"
    );
    assert!(!line.contains("context:"), "不再带 context: 前缀：{line}");
}

#[test]
fn context_follows_model_name_with_secondary_style() {
    // 上下文占用紧跟模型名（`M · 50(50%)`），用次要信息色。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), false));
    let rect = Rect::new(0, 0, 60, 5);
    let inner = pane(rect);
    let bar = bar_rect(inner);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, bar, &lx);
    // `M · 50(50%)`：模型名 1 列 + 分隔 3 列 → 占用从第 4 列起。
    let row = bar.y;
    let usage_x = bar.x + "M · ".width() as u16;
    let cell = &buf[(usage_x, row)];
    assert_eq!(cell.symbol(), "5", "占用紧跟在模型名之后");
    assert_eq!(cell.fg, Color::Reset, "占用文字用默认前景");
    assert_eq!(
        cell.modifier,
        style::chat_provider().add_modifier,
        "占用文字用次要信息色（dim、不加粗）"
    );
}

#[test]
fn model_name_uses_entry_label_gradient() {
    let lx = lx_with_status(status(Some("model"), Some("prov"), None, None, false));
    let rect = Rect::new(0, 0, 60, 5);
    let inner = pane(rect);
    let bar = bar_rect(inner);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, bar, &lx);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    let row = lay.label.y;
    let first = &buf[(lay.label.x, row)];
    assert_eq!(first.symbol(), "m");
    assert_eq!(first.fg, Color::Indexed(218), "模型名首字符 = 品牌粉");
    let last = &buf[(lay.label.x + 4, row)];
    assert_eq!(last.symbol(), "l");
    assert_eq!(last.fg, Color::Indexed(117), "模型名末字符 = 品牌蓝");
}

#[test]
fn provider_name_is_not_rendered() {
    // 供应商已移除：状态行只显示模型名与上下文占用。
    let lx = lx_with_status(status(
        Some("deepseek-v4.1-flash"),
        Some("9router-lx-t"),
        Some(1_000_000),
        Some(720_000),
        false,
    ));
    let line = render_bar(&lx, 80);
    assert!(!line.contains("9router"), "不显示供应商：{line}");
    assert!(line.contains("deepseek-v4.1-flash"), "显示模型名：{line}");
    assert!(line.contains("720k(72%)"), "显示上下文占用：{line}");
}

#[test]
fn pane_bottom_border_stays_clean_and_status_has_no_border_rules() {
    // 状态行独立于边框之外，不带 `╰` 或 `─` 边框符。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), false));
    let line = render_bar(&lx, 60);
    assert!(!line.contains('─'), "状态行本身不画边框线：{line}");
    assert_eq!(line, "M · 500(50%)");
}

#[test]
fn narrow_drops_context_keeps_model() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), true));
    // 宽：上下文占用。
    let bar = Rect::new(0, 0, 38, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert!(lay.context.width > 0, "38 列显示上下文占用");
    let bar = Rect::new(0, 0, 21, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert!(lay.context.width > 0, "21 列仍保留上下文占用");
    assert!(lay.label.width > 0, "保留模型名区");
    // 更窄：连上下文占用（7 列 `50(50%)`）也放不下时只留模型名（仍硬切保留）。
    let bar = Rect::new(0, 0, 8, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert_eq!(lay.context.width, 0, "更窄时丢弃上下文占用");
    assert!(lay.label.width > 0, "仍保留模型名区");
}

#[test]
fn narrow_keeps_context_next_to_hard_cut_model() {
    let lx = lx_with_status(status(
        Some("Very Long Model Name"),
        Some("Very Long Provider"),
        Some(100),
        Some(50),
        false,
    ));
    let line = render_bar(&lx, 34);
    assert!(!line.contains('…'), "不使用省略号：{line}");
    assert!(!line.contains("Provider"), "供应商不显示：{line}");
    assert!(line.contains("50(50%)"), "保留上下文占用：{line}");
}

#[test]
fn wide_status_renders_above_pane_bottom_border() {
    let lx = lx_with_status(status(
        Some("deepseek-v4.1-flash"),
        Some("9router-lx-t"),
        Some(1_000_000),
        Some(720_000),
        true,
    ));
    let bar = Rect::new(1, 2, 78, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert_eq!(lay.area.height, 1);
    assert_eq!(lay.area.y, bar.y, "绘制在底栏矩形行");
    let line = render_bar(&lx, 80);
    assert_eq!(line, "deepseek-v4.1-flash · 720k(72%)");
}

#[test]
fn no_stop_button_on_status_bar() {
    // 停止交互改为 Esc 双击，状态行不再有任何按钮文案。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), true));
    let line = render_bar(&lx, 80);
    assert!(!line.contains("[■]"), "不再渲染停止按钮：{line}");
}

#[test]
fn tiny_bar_yields_no_status() {
    let lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let lay = layout(
        Rect::new(0, 0, 5, 1),
        &lx.chat.status,
        lx.background_tasks,
        0,
    );
    assert_eq!(lay.area.height, 0);
    let zero_h = layout(
        Rect::new(0, 0, 40, 0),
        &lx.chat.status,
        lx.background_tasks,
        0,
    );
    assert_eq!(zero_h.area.height, 0);
}

#[test]
fn long_model_name_is_hard_cut_without_ellipsis() {
    let lx = lx_with_status(status(
        Some("very-long-model-name-that-never-fits"),
        Some("prov"),
        None,
        None,
        false,
    ));
    let line = render_bar(&lx, 34);
    assert!(!line.contains('…'), "模型名不使用省略号：{line}");
    assert!(
        line.contains("very-long-model-name"),
        "模型名尽量保留：{line}"
    );
}

// ===== 运行指标（输入框下方状态行：后台任务 / 子 agent，贴上下文右侧）=====

/// 设置后台任务计数后取底部状态行文本。
fn bar_with_tasks(count: usize, width: u16) -> String {
    bar_with_runs(count, 0, width)
}

/// 一个 running 子 agent 的展示记录。
fn running_subagent(id: &str) -> crate::app::state::SubagentRun {
    crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: id.into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: format!("sess-{id}"),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "running".into(),
        title: "investigate".into(),
        created_at_ms: 0,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    })
}

/// 设置后台任务与子 agent 计数后取底部状态行文本。
fn bar_with_runs(tasks: usize, subagents: usize, width: u16) -> String {
    let mut lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), false));
    lx.background_tasks = tasks;
    for index in 0..subagents {
        lx.agents
            .runs
            .push(running_subagent(&format!("sub-{index}")));
    }
    render_bar(&lx, width)
}

#[test]
fn task_count_is_hidden_when_zero() {
    // 为 0 时整段不渲染：不留空占位（对齐 opencode 的 `N shell`）。
    let line = bar_with_tasks(0, 60);
    assert!(!line.contains("process"), "0 个不显示指标：{line}");
    assert_eq!(line, "M · 500(50%)");
}

#[test]
fn task_count_uses_singular_and_plural() {
    assert_eq!(text::lx_task_count_label(1), "1 process");
    assert_eq!(text::lx_task_count_label(2), "2 processes");
    assert_eq!(text::lx_task_count_label(11), "11 processes");
}

#[test]
fn task_count_sits_right_of_context() {
    // 指标贴在上下文占用右侧。
    let line = bar_with_tasks(3, 60);
    assert_eq!(
        line, "M · 500(50%) · 3 processes",
        "指标贴上下文右侧：{line}"
    );
    let tasks = line.find("3 processes").expect("指标");
    let used = line.find("500(50%)").expect("用量");
    assert!(used < tasks, "指标在用量之后：{line}");
}

#[test]
fn subagent_count_sits_next_to_task_count_right_of_context() {
    let line = bar_with_runs(2, 1, 80);
    assert_eq!(
        line, "M · 500(50%) · 2 processes · 1 subagent",
        "后台任务与子 agent 用圆点分隔贴上下文右侧：{line}"
    );
}

#[test]
fn subagent_count_shows_without_background_tasks() {
    let line = bar_with_runs(0, 2, 80);
    assert_eq!(
        line, "M · 500(50%) · 2 subagents",
        "没有后台任务时也显示子 agent：{line}"
    );
}

#[test]
fn task_count_hides_when_too_narrow() {
    // 极窄时指标整段让位，不与模型名抢位置。
    let line = bar_with_tasks(3, 9);
    assert!(!line.contains("process"), "窄到放不下时不画：{line}");
}

/// 思考等级显示在模型名与上下文占用之间，用 ` · ` 分隔。
#[test]
fn variant_sits_between_model_and_context() {
    let mut st = status(Some("M"), Some("P"), Some(1_000), Some(500), false);
    st.variant = Some("high".into());
    let lx = lx_with_status(st);
    let line = render_bar(&lx, 60);
    assert_eq!(
        line, "M · high · 500(50%)",
        "顺序为 模型 · 思考等级 · 上下文：{line}"
    );
}

/// 思考等级用加粗黄字（对齐 opencode 的 warning + BOLD），分隔符仍是次要信息色。
#[test]
fn variant_is_bold_yellow_while_separator_is_muted() {
    let mut st = status(Some("M"), Some("P"), None, None, false);
    st.variant = Some("high".into());
    let lx = lx_with_status(st);
    let rect = Rect::new(0, 0, 40, 5);
    let inner = pane(rect);
    let bar = bar_rect(inner);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, bar, &lx);
    let row = bar.y;
    let cells: Vec<(u16, char, ratatui::style::Style)> = (bar.x..bar.right())
        .filter_map(|x| {
            buf.cell((x, row))
                .map(|cell| (x, cell.symbol().chars().next().unwrap_or(' '), cell.style()))
        })
        .collect();
    let variant_cell = cells
        .iter()
        .find(|(_, ch, _)| *ch == 'h')
        .expect("思考等级首字符");
    // cell 样式带 bg/underline_color 的 Reset 默认值，只比较前景与加粗。
    assert_eq!(
        variant_cell.2.fg,
        style::chat_variant().fg,
        "思考等级走专用前景色"
    );
    assert_eq!(
        variant_cell.2.fg,
        Some(ratatui::style::Color::Yellow),
        "黄色"
    );
    assert!(
        variant_cell
            .2
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "加粗"
    );
    // `·` 用次要信息色（与上下文占用的分隔符一致）。
    let sep_cell = cells.iter().find(|(_, ch, _)| *ch == '·').expect("分隔符");
    // `muted()` 只有 DIM、无前景色；单元格 fg 的 Reset 来自普通空白底色。
    assert!(
        sep_cell
            .2
            .add_modifier
            .contains(ratatui::style::Modifier::DIM),
        "分隔符走 muted（DIM）"
    );
    assert_ne!(
        sep_cell.2.fg,
        Some(ratatui::style::Color::Yellow),
        "分隔符不是黄色"
    );
}

/// 无 variants 时整段不显示（模型名与上下文照常，不出现空占位）。
#[test]
fn no_variant_leaves_no_gap() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), false));
    let line = render_bar(&lx, 60);
    assert_eq!(line, "M · 500(50%)", "无 variant 时无空洞：{line}");
    assert!(!line.contains("·  ·"), "不留下双分隔符：{line}");
}

/// 宽度不足时先丢上下文占用，再丢思考等级，模型名始终保留。
#[test]
fn blocked_subagents_show_above_border() {
    let mut lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let mut blocked = running_subagent("blocked");
    blocked.status = "blocked".into();
    lx.agents.runs.push(blocked);
    lx.chat.status.blocked_subagents = 1;
    let line = render_bar(&lx, 80);
    assert!(
        line.contains(text::lx_subagent_blocked_label(1).as_str()),
        "阻塞子 agent 显示 `1 blocked`：{line}"
    );

    // 同时有运行中的子 agent：两段用圆点分隔。
    lx.agents.runs.push(running_subagent("running"));
    let line = render_bar(&lx, 80);
    assert!(
        line.contains("1 subagent · 1 blocked"),
        "运行与阻塞指标并列：{line}"
    );
}

#[test]
fn narrow_drops_context_before_variant() {
    let mut st = status(Some("M"), Some("P"), Some(1_000), Some(500), false);
    st.variant = Some("high".into());
    let lx = lx_with_status(st);
    // 12 列状态行：`M · high` 放得下，` · 500(50%)` 放不下。
    let bar = Rect::new(0, 0, 12, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert!(lay.variant.width > 0, "思考等级保留");
    assert_eq!(lay.context.width, 0, "上下文占用先被丢弃");
    assert!(lay.label.width > 0, "模型名仍在");

    // 更窄：连思考等级也放不下（`M · high` 需 8 列，可用只有 7）。
    let bar = Rect::new(0, 0, 7, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert_eq!(lay.variant.width, 0, "更窄时丢弃思考等级");
    assert!(lay.label.width > 0, "模型名仍在");
}

#[test]
fn cache_hit_renders_after_context_and_drops_first() {
    let mut st = status(Some("M"), Some("P"), Some(1_000), Some(500), false);
    st.variant = Some("high".into());
    st.cache_hit_percent = Some(70);
    assert_eq!(cache_text_for_test(&st).as_deref(), Some("70% cached"));
    let lx = lx_with_status(st);
    let line = render_bar(&lx, 80);
    assert!(line.contains("500(50%)"), "上下文占用仍在：{line}");
    assert!(
        line.contains("70% cached"),
        "缓存命中率跟在上下文右侧：{line}"
    );

    // 宽度不足时 cached 最先丢，上下文与 variant 保留。
    let bar = Rect::new(0, 0, 19, 1);
    let lay = layout(bar, &lx.chat.status, lx.background_tasks, 0);
    assert_eq!(lay.cache.width, 0, "cached 先被丢弃");
    assert!(lay.context.width > 0, "上下文保留");
    assert!(lay.variant.width > 0, "variant 保留");
}

/// 右侧 `omitted turns` 提示必须先占位：窄窗格里不得覆盖已画出的上下文/缓存字段。
#[test]
fn omitted_turns_notice_does_not_overwrite_context_or_cache() {
    let mut st = status(Some("M"), Some("P"), Some(1_000), Some(500), false);
    st.cache_hit_percent = Some(70);
    st.omitted_turns = Some(3);
    let lx = lx_with_status(st);
    // 30 列状态行：不预留提示时布局会保留 `70% cached`，提示右对齐后正好压在它上面。
    let line = render_bar(&lx, 34);
    assert!(line.contains("500(50%)"), "上下文占用保留：{line}");
    assert!(line.contains("omitted turns: 3"), "省略提示显示：{line}");
    assert!(!line.contains("70% cached"), "缓存让位给省略提示：{line}");
    let context_end = line
        .find("500(50%)")
        .map(|index| index + "500(50%)".len())
        .unwrap_or(0);
    let notice_start = line.find("omitted turns: 3").unwrap_or(0);
    assert!(notice_start > context_end, "提示在上下文之后：{line}");
    assert!(
        line[context_end..notice_start].chars().all(|ch| ch == ' '),
        "提示与上下文之间为空白间距，不得覆盖字段：{line}"
    );
}
