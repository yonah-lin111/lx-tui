//! 状态测试：`context:当前用量(百分比)` 显示与隐藏（无消耗整段隐藏）、窄宽度降级（丢上下文占用）、
//! 模型名硬切不省略号、底边线保留、供应商用统计行同款次要信息色（均绘制在窗格底边框上）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

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
        context_limit: context,
        input_tokens: input,
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
fn context_text_is_usage_with_percent() {
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(258_700), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("context:258.7k(26%)"),
        "一位小数 + 四舍五入百分比"
    );
    let st = status(Some("M"), Some("P"), Some(200_000), Some(45_200), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("context:45.2k(23%)")
    );
    // 整数值不带 `.0`；用量超过上限时百分比夹到 100%。
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(720_000), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("context:720k(72%)")
    );
    let st = status(Some("M"), Some("P"), Some(100), Some(200), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("context:200(100%)"),
        "超过上限夹到 100%"
    );
}

#[test]
fn tiny_usage_shows_exact_count_with_zero_percent() {
    // 小用量按一位小数原样显示（百分比取整到 0% 也照常给出）。
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(2_500), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("context:2.5k(0%)")
    );
    let st = status(Some("M"), Some("P"), Some(1_000_000), Some(999), false);
    assert_eq!(
        context_text_for_test(&st).as_deref(),
        Some("context:999(0%)")
    );
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
    let line = render_border(&lx, 60);
    assert!(!line.contains("context"), "整段隐藏：{line}");
}

#[test]
fn context_usage_shows_usage_with_percent_on_border() {
    let lx = lx_with_status(status(
        Some("M"),
        Some("P"),
        Some(1_000_000),
        Some(258_700),
        false,
    ));
    let line = render_border(&lx, 60);
    assert!(
        line.contains("context:258.7k(26%)"),
        "占用显示 `context:258.7k(26%)`：{line}"
    );
}

#[test]
fn context_uses_provider_color() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(100), Some(50), false));
    let rect = Rect::new(0, 0, 60, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, inner, &lx);
    let lay = layout(inner, &lx.chat.status);
    let cell = &buf[(lay.context.x, lay.context.y)];
    assert_eq!(cell.symbol(), "c");
    assert_eq!(cell.fg, Color::Reset, "占用文字用默认前景（供应商灰）");
    assert_eq!(
        cell.modifier,
        style::chat_provider().add_modifier,
        "占用文字与供应商同款（dim、不加粗）"
    );
}

#[test]
fn model_name_uses_entry_label_gradient() {
    let lx = lx_with_status(status(Some("model"), Some("prov"), None, None, false));
    let rect = Rect::new(0, 0, 60, 5);
    let inner = pane(rect);
    let mut buf = Buffer::empty(rect);
    render(&mut buf, inner, &lx);
    let lay = layout(inner, &lx.chat.status);
    let row = lay.label.y;
    let first = &buf[(lay.label.x, row)];
    assert_eq!(first.symbol(), "m");
    assert_eq!(first.fg, Color::Indexed(218), "模型名首字符 = 品牌粉");
    let last = &buf[(lay.label.x + 4, row)];
    assert_eq!(last.symbol(), "l");
    assert_eq!(last.fg, Color::Indexed(117), "模型名末字符 = 品牌蓝");
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
    let inner = Rect::new(0, 0, 23, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.context.width > 0, "23 列仍保留上下文占用");
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
    // 内宽 24（`render_border` 的 width - 2）：上下文占用 15 列 + 模型名区 6 列刚好放得下。
    let line = render_border(&lx, 26);
    assert!(!line.contains('…'), "不使用省略号：{line}");
    assert!(!line.contains("Provider"), "供应商被整体隐藏：{line}");
    assert!(line.contains("context:50(50%)"), "保留上下文占用：{line}");
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
        line.ends_with("context:720k(72%) ╯"),
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
