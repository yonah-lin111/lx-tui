//! 状态测试：`context:NN%` 显示与隐藏（无消耗整段隐藏）、窄宽度降级（丢上下文占用）、
//! 模型名硬切不省略号、底边线保留、供应商用统计行同款次要信息色（均绘制在窗格底边框上）。

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
fn context_text_is_prefixed_percent() {
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(720_000), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("context:72%"));
    let st = status(Some("M"), Some("P"), Some(100), Some(100), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("context:100%"));
}

#[test]
fn tiny_usage_still_shows_zero_percent() {
    // 百万级上下限下，小用量四舍五入到 0% 仍要显示（否则历史会话看起来「上下文为空」）。
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(2_500), false);
    assert_eq!(context_text_for_test(&st).as_deref(), Some("context:0%"));
}

#[test]
fn context_without_usage_is_hidden_entirely() {
    // 未知（无上限/无用量）与 0% 都整段隐藏。
    for value in [
        status(Some("M"), Some("P"), None, None, false),
        status(Some("M"), Some("P"), Some(1_000), None, false),
        status(Some("M"), Some("P"), Some(1_000), Some(0), false),
    ] {
        assert_eq!(context_text_for_test(&value), None, "无消耗整段隐藏");
    }
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(0), false));
    let line = render_border(&lx, 60);
    assert!(!line.contains('%'), "0% 不显示百分比：{line}");
    assert!(!line.contains("context"), "0% 整段隐藏：{line}");
}

#[test]
fn context_usage_shows_prefixed_percent_on_border() {
    let lx = lx_with_status(status(
        Some("M"),
        Some("P"),
        Some(1_000_000),
        Some(720_000),
        false,
    ));
    let line = render_border(&lx, 60);
    assert!(
        line.contains("context:72%"),
        "占用显示 `context:72%`：{line}"
    );
}

#[test]
fn context_style_is_white() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), false));
    let rect = Rect::new(0, 0, 60, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, inner, &lx);
    let lay = layout(inner, &lx.chat.status);
    let cell = &buf[(lay.context.x, lay.context.y)];
    assert_eq!(cell.symbol(), "c");
    assert_eq!(cell.fg, Color::Indexed(252), "占用文字亮白");
}

#[test]
fn provider_uses_stats_line_style() {
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
    // 供应商用本轮统计行同款次要信息色（显式 Reset 前景 + dim），不加粗。
    assert_eq!(cell.fg, Color::Reset, "供应商用默认前景");
    assert_eq!(cell.modifier, Modifier::DIM, "供应商与统计行同款（dim）");
}

#[test]
fn border_line_stays_visible_between_segments() {
    // 未显示各控件时，两者之间的底边线保留 `─`（不被整段清成空格）。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), false));
    let line = render_border(&lx, 60);
    assert!(line.contains('─'), "底边线保留：{line}");
    assert!(
        line.starts_with("╰ M · P─"),
        "模型 · 供应商与上下文占用之间为底边线：{line}"
    );
}

#[test]
fn narrow_drops_context_keeps_model() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), true));
    // 宽：上下文占用 + 供应商。
    let inner = Rect::new(0, 0, 40, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.context.width > 0, "40 列显示上下文占用");
    let inner = Rect::new(0, 0, 20, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.context.width > 0, "20 列仍保留上下文占用");
    assert!(lay.label.width > 0, "保留模型名区");
    // 更窄：连上下文占用也放不下时只留模型名（仍硬切保留）。
    let inner = Rect::new(0, 0, 16, 3);
    let lay = layout(inner, &lx.chat.status);
    assert_eq!(lay.context.width, 0, "更窄时丢弃上下文占用");
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
    assert!(line.contains("50%"), "保留上下文占用：{line}");
}

#[test]
fn wide_status_on_border_with_provider_and_context() {
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
    assert_eq!(lay.area.y, inner.bottom(), "绘制在边框行");
    let line = render_border(&lx, 80);
    assert!(
        line.starts_with("╰ deepseek-v4.1-flash · 9router-lx-t"),
        "左角后模型 · 供应商：{line}"
    );
    assert!(
        line.ends_with("context:72% ╯"),
        "上下文占用贴右端、保留角：{line}"
    );
}

#[test]
fn no_stop_button_on_border() {
    // 停止交互改为 Esc 双击，底边框上不再有任何按钮文案。
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1_000), Some(500), true));
    let line = render_border(&lx, 80);
    assert!(!line.contains("[■]"), "不再渲染停止按钮：{line}");
}

#[test]
fn tiny_inner_yields_no_status() {
    let lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let lay = layout(Rect::new(0, 0, 7, 3), &lx.chat.status);
    assert_eq!(lay.area.height, 0);
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
