//! 状态行测试：未知上下文显示 `--`、模型 · 供应商截断、上下文组件、停止按钮几何。

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

/// 渲染状态行并返回该行文本（去掉行尾空格）。
fn render_line(lx: &Lx, width: u16) -> String {
    let area = Rect::new(0, 0, width, 1);
    let mut grid = super::super::test_grid(area);
    render(&mut grid, area, lx);
    let mut buf = Buffer::empty(area);
    super::super::blit_grid(area, &mut buf, &grid);
    (0..width)
        .map(|x| buf.cell((x, 0)).map(|cell| cell.symbol()).unwrap_or(" "))
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
fn wide_status_single_row_with_stop_button() {
    let lx = lx_with_status(status(
        Some("deepseek-v4.1-flash"),
        Some("9router-lx-t"),
        Some(1_000_000),
        Some(2_000),
        true,
    ));
    let area = Rect::new(0, 0, 80, 1);
    let lay = layout(area, &lx.chat.status, rows(area, &lx.chat.status));
    assert_eq!(lay.area.height, 1);
    assert!(lay.stop.is_some(), "流中显示停止按钮");
    let line = render_line(&lx, 80);
    assert!(
        line.starts_with("deepseek-v4.1-flash · 9router-lx-t"),
        "行首模型 · 供应商：{line}"
    );
    assert!(line.contains("context 0%"), "含上下文组件：{line}");
}

#[test]
fn idle_status_hides_stop_button() {
    let lx = lx_with_status(status(Some("M"), Some("P"), Some(1000), None, false));
    let area = Rect::new(0, 0, 80, 1);
    let lay = layout(area, &lx.chat.status, rows(area, &lx.chat.status));
    assert!(lay.stop.is_none(), "空闲不显示停止按钮");
}

#[test]
fn narrow_status_uses_two_rows() {
    let lx = lx_with_status(status(
        Some("Very Long Model"),
        Some("Very Long Provider"),
        Some(128_000),
        Some(42_000),
        false,
    ));
    let area = Rect::new(0, 0, 30, 2);
    assert_eq!(rows(area, &lx.chat.status), 2, "窄窗格两行");
    let lay = layout(area, &lx.chat.status, 2);
    assert_eq!(lay.area.height, 2);
    assert_eq!(lay.label.height, 2);
}

#[test]
fn zero_height_area_yields_no_status() {
    let lx = lx_with_status(status(Some("M"), Some("P"), None, None, false));
    let lay = layout(Rect::new(0, 0, 80, 0), &lx.chat.status, 0);
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
    let line = render_line(&lx, 30);
    assert!(line.contains("prov"), "供应商保留：{line}");
    assert!(line.contains('…'), "模型名截断：{line}");
}
