//! 状态测试：进度条填充/空余、无消耗不显示百分比、降级（丢进度条 → 停止控件 → 供应商）、
//! 模型名硬切不省略号、底边线保留、供应商用 prompt 条目标题色（均绘制在窗格底边框上）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;
use crate::app::state::Lx;
use crate::app::state::lx::chat::{ChatStatus, context_percent};

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
        context_limit: context,
        input_tokens: input,
        context_percent: context_percent(input, context),
        streaming,
        omitted_turns: None,
    }
}

fn lx_with_status(status: ChatStatus) -> Lx {
    let mut lx = Lx::new();
    lx.chat.status = status;
    lx
}

/// 真实窗格几何：窗格矩形 `rect` 的边框内缩 1 得内容区。
fn pane(rect: Rect) -> Rect {
    crate::layout::pane_inner_rect(rect)
}

/// 在内容区上模拟窗格边框：底边框行铺 `─` 与两角 `╰`/`╯`，再渲染状态，返回底边框整行文本。
fn render_border(lx: &Lx, width: u16) -> String {
    let rect = Rect::new(0, 0, width, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    let row = inner.bottom();
    for x in rect.x..rect.right() {
        let ch = if x == rect.x {
            '╰'
        } else if x == rect.right() - 1 {
            '╯'
        } else {
            '─'
        };
        if let Some(cell) = buf.cell_mut((x, row)) {
            cell.set_char(ch);
        }
    }
    render(&mut buf, inner, lx);
    (rect.x..rect.right())
        .map(|x| buf.cell((x, row)).map(|c| c.symbol()).unwrap_or(" "))
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn context_percent_is_ratio_clamped() {
    assert_eq!(context_percent(None, Some(100)), None);
    assert_eq!(context_percent(Some(10), None), None);
    assert_eq!(context_percent(Some(10), Some(0)), None);
    assert_eq!(context_percent(Some(0), Some(100)), Some(0));
    assert_eq!(context_percent(Some(42), Some(100)), Some(42));
    assert_eq!(context_percent(Some(200), Some(100)), Some(100));
}

#[test]
fn context_bar_fills_proportionally() {
    let st = status(Some("M"), Some("P"), Some(100), Some(50), false);
    assert_eq!(context_bar_text(&st, 10), "█████░░░░░");
    let st = status(Some("M"), Some("P"), Some(100), Some(0), false);
    assert_eq!(context_bar_text(&st, 10), "░░░░░░░░░░", "0% 全空余");
    let st = status(Some("M"), Some("P"), Some(100), Some(100), false);
    assert_eq!(context_bar_text(&st, 10), "██████████", "满格");
    let st = status(Some("M"), Some("P"), Some(100), None, false);
    assert_eq!(context_bar_text(&st, 10), "░░░░░░░░░░", "未知整条空余");
}

#[test]
fn context_bar_rounds_nearest_cell() {
    let st = status(Some("M"), Some("P"), Some(100), Some(42), false);
    assert_eq!(context_bar_text(&st, 10), "████░░░░░░");
    let st = status(Some("M"), Some("P"), Some(100), Some(78), false);
    assert_eq!(context_bar_text(&st, 10), "████████░░");
}

#[test]
fn context_zero_usage_hides_percent_text() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(0), false));
    let line = render_border(&lx, 60);
    assert!(!line.contains('%'), "无消耗不显示百分比：{line}");
    assert!(line.contains('░'), "仍显示空进度条：{line}");
}

#[test]
fn context_usage_shows_percent_after_bar() {
    let lx = lx_with_status(status(
        Some("M"),
        Some("P"),
        Some(1_000_000),
        Some(720_000),
        false,
    ));
    let line = render_border(&lx, 60);
    assert!(line.contains("72%"), "占用显示百分比：{line}");
    let bar_at = line.find(text::LX_CHAT_CONTEXT_BAR_FREE).expect("进度条");
    assert!(line.contains('█'), "有已用段：{line}");
    assert!(line[bar_at..].contains("72%"), "百分比在进度条右侧：{line}");
}

#[test]
fn unknown_usage_shows_empty_bar_without_percent() {
    let lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let line = render_border(&lx, 60);
    assert!(!line.contains('%'), "未知不显示百分比：{line}");
    // 无上下文上限时不显示进度条。
    assert!(!line.contains('░'), "无上限不显示进度条：{line}");
}

#[test]
fn bar_used_white_and_free_gray() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), false));
    let rect = Rect::new(0, 0, 60, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, inner, &lx);
    let lay = layout(inner, &lx.chat.status);
    let used = &buf[(lay.bar.x, lay.bar.y)];
    assert_eq!(used.symbol(), "█");
    assert_eq!(used.fg, Color::Indexed(252), "已用段亮白");
    let free = &buf[(lay.bar.x + lay.bar.width - 1, lay.bar.y)];
    assert_eq!(free.symbol(), "░");
    assert_eq!(free.fg, Color::DarkGray, "空余段中性灰");
}

#[test]
fn provider_uses_prompt_item_title_style() {
    let lx = lx_with_status(status(
        Some("deepseek-v4.1-flash"),
        Some("9router-lx-t"),
        Some(1_000_000),
        Some(720_000),
        false,
    ));
    let rect = Rect::new(0, 0, 80, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, inner, &lx);
    let row = inner.bottom();
    let provider_x = rect.x + 1 + "deepseek-v4.1-flash · ".width() as u16;
    let cell = &buf[(provider_x, row)];
    // prompt 条目标题 / 供应商：普通文字色（Reset），无 dim、无加粗。
    assert_eq!(cell.fg, Color::Reset, "供应商用普通文字色");
    assert_eq!(cell.modifier, Modifier::empty(), "供应商不加粗、不 dim");
}

#[test]
fn border_line_stays_visible_between_segments() {
    // 未显示各控件时，两者之间的底边线保留 `─`（不被整段清成空格）。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), false));
    let line = render_border(&lx, 60);
    assert!(line.contains('─'), "底边线保留：{line}");
    assert!(
        line.starts_with("╰ M · P─"),
        "模型 · 供应商与进度条之间为底边线：{line}"
    );
}

#[test]
fn narrow_drops_bar_then_stop_keeps_model_and_percent() {
    // 够「条 + 百分比」：三者中无流中，故为 条+百分比。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), true));
    let inner = Rect::new(0, 0, 30, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.bar.width > 0, "30 列仍有进度条");
    assert!(lay.stop.is_some(), "30 列仍有停止控件");
    // 收窄到放不下进度条：丢进度条、丢供应商，保留模型名与百分比（+停止控件）。
    let inner = Rect::new(0, 0, 20, 3);
    let lay = layout(inner, &lx.chat.status);
    assert_eq!(lay.bar.width, 0, "窄窗格丢弃进度条");
    assert!(lay.percent.width > 0, "保留百分比");
    assert!(lay.label.width > 0, "保留模型名区");
    // 更窄：丢停止控件，仍保留模型名与百分比。
    let inner = Rect::new(0, 0, 12, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.stop.is_none(), "更窄时丢弃停止控件");
    assert!(lay.percent.width > 0, "仍保留百分比");
    assert!(lay.label.width > 0, "仍保留模型名区");
}

#[test]
fn narrow_hides_provider_without_ellipsis() {
    let lx = lx_with_status(status(
        Some("Very Long Model Name"),
        Some("Very Long Provider"),
        Some(100),
        Some(50),
        false,
    ));
    let line = render_border(&lx, 24);
    assert!(!line.contains('…'), "不使用省略号：{line}");
    assert!(!line.contains("Provider"), "供应商被整体隐藏：{line}");
    assert!(line.contains("50%"), "保留百分比：{line}");
}

#[test]
fn wide_status_on_border_with_stop_button() {
    let lx = lx_with_status(status(
        Some("deepseek-v4.1-flash"),
        Some("9router-lx-t"),
        Some(1_000_000),
        Some(720_000),
        true,
    ));
    let inner = Rect::new(0, 0, 80, 3);
    let lay = layout(inner, &lx.chat.status);
    assert_eq!(lay.area.height, 1);
    assert_eq!(lay.area.y, inner.bottom(), "绘制在底边框行");
    assert!(lay.stop.is_some(), "流中显示停止按钮");
    let line = render_border(&lx, 80);
    assert!(
        line.starts_with("╰ deepseek-v4.1-flash · 9router-lx-t"),
        "左角后模型 · 供应商：{line}"
    );
    assert!(line.ends_with("[■] ╯"), "停止控件贴右端、保留角：{line}");
    assert!(line.contains('█'), "含已用进度段：{line}");
    assert!(line.contains("72%"), "占用显示百分比：{line}");
}

#[test]
fn idle_status_hides_stop_button() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1000), None, false));
    let inner = Rect::new(0, 0, 80, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.stop.is_none(), "空闲不显示停止按钮");
}

#[test]
fn tiny_inner_yields_no_status() {
    let lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let lay = layout(Rect::new(0, 0, 7, 3), &lx.chat.status);
    assert_eq!(lay.area.height, 0);
    assert!(lay.stop.is_none());
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
    let line = render_border(&lx, 34);
    assert!(!line.contains('…'), "模型名不使用省略号：{line}");
    assert!(
        line.contains("very-long-model-name"),
        "模型名尽量保留：{line}"
    );
}
