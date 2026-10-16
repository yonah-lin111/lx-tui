//! 状态测试：未知上下文显示 `--`、模型 · 供应商截断、上下文组件、停止控件几何（绘制在窗格底边框）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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

/// 在内容区上模拟窗格边框：底边框行铺 `─` 与两角 `╰`/`╯`，再渲染状态。
///
/// 与 `render_panes` 一致：窗格矩形 `rect`、边框内缩 1 得内容区 `inner`，
/// 底边框行 = `inner.bottom()`（位于 `inner` 之外），两角在 `rect.x` / `rect.right()-1`。
fn render_border(lx: &Lx, width: u16) -> String {
    let rect = Rect::new(0, 0, width, 5);
    let inner = crate::layout::pane_inner_rect(rect);
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
fn unknown_context_shows_dash_dash() {
    let st = status(Some("Model"), Some("Prov"), None, None, false);
    assert_eq!(context_text(&st), "context --");
    let st = status(Some("Model"), Some("Prov"), Some(128_000), None, false);
    assert_eq!(context_text(&st), "context --");
}

#[test]
fn wide_status_on_border_with_stop_button() {
    let lx = lx_with_status(status(
        Some("deepseek-v4.1-flash"),
        Some("9router-lx-t"),
        Some(1_000_000),
        Some(2_000),
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
    assert!(line.contains("● context 0%"), "含上下文组件：{line}");
}

#[test]
fn idle_status_hides_stop_button() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1000), None, false));
    let inner = Rect::new(0, 0, 80, 3);
    let lay = layout(inner, &lx.chat.status);
    assert!(lay.stop.is_none(), "空闲不显示停止按钮");
}

#[test]
fn narrow_border_hides_stop_but_keeps_context() {
    let lx = lx_with_status(status(
        Some("Very Long Model"),
        Some("Very Long Provider"),
        Some(128_000),
        Some(42_000),
        true,
    ));
    // 宽度不足以容纳上下文组件 + 停止控件时不给停止控件。
    let inner = Rect::new(0, 0, 12, 3);
    let lay = layout(inner, &lx.chat.status);
    assert_eq!(lay.area.height, 1, "状态恒占底边框一行");
    assert!(lay.stop.is_none(), "窄边框不给停止控件");
    assert!(lay.pill.width > 0, "上下文组件仍占位");
}

#[test]
fn tiny_inner_yields_no_status() {
    let lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let lay = layout(Rect::new(0, 0, 7, 3), &lx.chat.status);
    assert_eq!(lay.area.height, 0);
    assert!(lay.stop.is_none());
}

#[test]
fn long_model_name_is_truncated_before_provider() {
    let lx = lx_with_status(status(
        Some("very-long-model-name-that-never-fits"),
        Some("prov"),
        None,
        None,
        false,
    ));
    let line = render_border(&lx, 34);
    assert!(line.contains("prov"), "供应商保留：{line}");
    assert!(
        !line.contains("very-long-model-name-that-never-fits"),
        "模型名被裁剪：{line}"
    );
}
